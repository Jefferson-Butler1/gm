//! Headless, deterministic simulation (issue #5).
//!
//! Rollback-ready by construction: all state lives in one [`SimState`] that is `Clone`,
//! serializable and checksummable; math is fixed-point ([`Fx`]); time is counted in ticks
//! at [`TICK_HZ`]; and [`step`] is a pure function of the state and the tick's inputs.
//! Pause, menus and wall-clock time live outside the sim.

#![forbid(unsafe_code)]
#![deny(
    clippy::float_arithmetic,
    clippy::disallowed_types,
    clippy::wildcard_enum_match_arm
)]

mod arena;
mod checksum;
mod combat;
mod config;
mod derelict;
mod encounter;
mod gun;
mod input;
mod path;
mod player;
mod rng;
pub mod room;
pub mod trig;

pub use arena::{Arena, Id};
pub use combat::{
    Awareness, BULLET_RADIUS, Behavior, Bullet, DEATH_TICKS, ENEMY_BULLET_RADIUS, ENEMY_RADIUS,
    Enemy, EnemyId, Pattern, RUSHER_HP, SHOOTER_HP, SPAWN_TELEGRAPH_TICKS, SPREAD_SHOOTER_HP,
    chase,
};
pub use config::{Difficulty, RunConfig, Tuning, VentStyle, per_tick};
pub use derelict::DERELICT;
pub use gun::PhasePistol;
pub use input::{Buttons, MOVE_BUCKETS, PlayerInput, TickInputs};
pub use path::FlowField;
pub use player::{ASSIST_CONE, MAX_HP, PLAYER_RADIUS, Player};
pub use rng::Rng;

use serde::{Deserialize, Serialize};

/// Fixed simulation rate. Speeds and timers derive from this; render interpolates up to
/// the display rate.
pub const TICK_HZ: u32 = 60;

/// Player slots. Single-player is slot 0.
pub const MAX_PLAYERS: usize = 4;

/// The sim's only numeric type for world math: ±2^31 range, 2^-32 resolution.
pub type Fx = fixed::types::I32F32;

/// World-space vector in room-local points: the origin is the room grid's top-left
/// corner and +y points down (see [`room`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FxVec2 {
    pub x: Fx,
    pub y: Fx,
}

/// Index into [`DERELICT`]'s rooms.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RoomId(pub u16);

/// The run's state machine. Each variant owns its data; transitions happen only in [`step`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Run {
    Boarding {
        room: RoomId,
    },
    Encounter {
        room: RoomId,
        wave: u8,
        doors_locked: bool,
    },
    Dead {
        /// Where the party fell; presentation keeps drawing it.
        room: RoomId,
        ticks_until_restart: u32,
    },
    /// Extracted from `room` (the exit room), which presentation keeps drawing.
    Won {
        room: RoomId,
    },
}

impl Run {
    /// The room the party is in (or ended the run in).
    #[must_use]
    pub const fn room(self) -> RoomId {
        match self {
            Self::Boarding { room }
            | Self::Encounter { room, .. }
            | Self::Dead { room, .. }
            | Self::Won { room } => room,
        }
    }

    #[must_use]
    pub const fn doors_locked(self) -> bool {
        matches!(
            self,
            Self::Encounter {
                doors_locked: true,
                ..
            }
        )
    }
}

/// Everything that affects simulation results. Nothing outside this struct may.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SimState {
    pub tick: u64,
    /// The run seed. The first comes from the host (random per session); each restart
    /// derives the next with [`Rng::next_seed`], so replays stay deterministic.
    pub seed: u64,
    pub rng: Rng,
    pub run: Run,
    /// `None` = empty slot. An occupied slot with 0 HP is a dead player.
    pub players: [Option<Player>; MAX_PLAYERS],
    /// Enemies and bullets in the party's current room; leaving a room drops them.
    pub enemies: Arena<Enemy>,
    /// The players' bullets.
    pub bullets: Arena<Bullet>,
    pub enemy_bullets: Arena<Bullet>,
    /// Bit `i` set = room `i` is cleared, so re-entering it starts no encounter.
    pub cleared: u64,
    /// Difficulty and tunables, fixed for the run.
    pub config: RunConfig,
    /// The config the next restart uses; `None` keeps [`Self::config`]. The host sets it
    /// (settings changed mid-run); it takes effect only when a new run starts.
    pub next_config: Option<RunConfig>,
}

impl SimState {
    /// A fresh single-player run, standing in the derelict's start room. `config` is
    /// clamped to what the sim handles (see [`RunConfig::sanitized`]).
    #[must_use]
    pub fn new(seed: u64, config: RunConfig) -> Self {
        let config = config.sanitized();
        let (room, at) = DERELICT.start();
        let mut state = Self {
            tick: 0,
            seed,
            rng: Rng::from_seed(seed),
            run: Run::Boarding { room },
            players: [Some(Player::new(&config.tuning)), None, None, None],
            enemies: Arena::default(),
            bullets: Arena::default(),
            enemy_bullets: Arena::default(),
            cleared: 0,
            config,
            next_config: None,
        };
        encounter::enter(&mut state, room, at, &mut TickEvents::default());
        state
    }

    /// A fresh run from the next run seed, with the same occupied slots, under
    /// [`Self::next_config`] if one was supplied, else the same config. The tick keeps
    /// counting: it is session time, and presentation keys effects by it.
    #[must_use]
    pub fn restarted(&self) -> Self {
        let config = self.next_config.unwrap_or(self.config);
        let mut fresh = Self::new(Rng::next_seed(self.seed), config);
        fresh.tick = self.tick;
        let start = fresh.players[0];
        for (slot, old) in fresh.players.iter_mut().zip(&self.players) {
            *slot = old.and(start);
        }
        fresh
    }

    /// Collision for the party's current room.
    #[must_use]
    pub fn tiles(&self) -> Option<room::Tiles> {
        let room = DERELICT.room(self.run.room())?;
        Some(room::Tiles {
            room,
            sealed: self.run.doors_locked(),
        })
    }

    #[must_use]
    pub fn cleared(&self, room: RoomId) -> bool {
        self.cleared
            .checked_shr(u32::from(room.0))
            .is_some_and(|bits| bits & 1 == 1)
    }

    fn set_cleared(&mut self, room: RoomId) {
        self.cleared |= 1_u64.checked_shl(u32::from(room.0)).unwrap_or(0);
    }

    /// Endianness-pinned hash of the whole state, comparable across machines.
    #[must_use]
    pub fn checksum(&self) -> u64 {
        checksum::of(self)
    }
}

/// One-way sim -> presentation notifications. Presentation may see a tick re-run on
/// rollback, so effects must be keyed by tick, not assumed unique.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    ShotFired {
        slot: usize,
    },
    EnemyHit {
        enemy: EnemyId,
    },
    /// An unaware enemy noticed the party (see [`Awareness`]).
    EnemyAlerted {
        enemy: EnemyId,
    },
    /// Also preceded by the killing `EnemyHit`. `pos` is where it died; the ID is stale.
    EnemyKilled {
        enemy: EnemyId,
        pos: FxVec2,
    },
    PlayerHit {
        slot: usize,
    },
    /// A player walked (or rolled) into a pit, losing a hit point; respawns after the
    /// run's `fall_ticks`.
    PlayerFell {
        slot: usize,
    },
    /// Also preceded by the killing `PlayerHit` or `PlayerFell`.
    PlayerDied {
        slot: usize,
    },
    Restarted,
    /// The party walked through an exit into `room`.
    RoomEntered {
        room: RoomId,
    },
    /// A reinforcement layer spawned; the base layer is wave 0.
    WaveStarted {
        wave: u8,
    },
    /// The room's last wave died.
    RoomCleared {
        room: RoomId,
    },
    /// A player reached the extraction pad: the run is won.
    Won,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TickEvents {
    pub events: Vec<Event>,
}

/// Advance the sim by exactly one tick. Pure: no clock, no I/O, no state outside `state`.
pub fn step(state: &mut SimState, inputs: &TickInputs) -> TickEvents {
    let mut events = TickEvents::default();

    let restart_pressed = inputs
        .players
        .iter()
        .any(|input| input.buttons.contains(Buttons::RESTART));
    // Only a live run simulates; the world stays frozen while Dead or Won. RESTART
    // abandons a live run at once (the settings sheet's Restart button); after a death it
    // waits out the death pause.
    match &mut state.run {
        Run::Boarding { .. } | Run::Encounter { .. } if restart_pressed => {
            *state = state.restarted();
            events.events.push(Event::Restarted);
        }
        Run::Boarding { .. } | Run::Encounter { .. } => combat::tick(state, inputs, &mut events),
        Run::Dead {
            ticks_until_restart,
            ..
        } if *ticks_until_restart > 0 => {
            *ticks_until_restart = ticks_until_restart.saturating_sub(1);
        }
        Run::Dead { .. } | Run::Won { .. } if restart_pressed => {
            *state = state.restarted();
            events.events.push(Event::Restarted);
        }
        Run::Dead { .. } | Run::Won { .. } => {}
    }

    state.tick = state.tick.wrapping_add(1);
    events
}
