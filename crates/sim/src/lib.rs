//! Headless, deterministic simulation (issue #5).
//!
//! Rollback-ready by construction: all state lives in one [`SimState`] that is `Clone`,
//! serializable and checksummable; math is fixed-point ([`Fx`]); time is counted in ticks
//! at [`TICK_HZ`]; and [`step`] is a pure function of the state and the tick's inputs.
//! The static floor ([`Ship`]) is shared behind an `Arc`, so a snapshot copies only live
//! state. Pause, menus and wall-clock time live outside the sim.

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
mod encounter;
mod gun;
pub mod hull;
mod input;
mod path;
mod player;
mod pool;
mod rng;
pub mod room;
pub mod ship;
pub mod trig;

pub use arena::{Arena, Id};
pub use combat::{
    Arrival, Awareness, BULLET_RADIUS, Behavior, Bullet, DEATH_TICKS, ENEMY_BULLET_RADIUS,
    ENEMY_RADIUS, Enemy, EnemyId, Patrol, Pattern, RUSHER_HP, SHOOTER_HP, SPREAD_SHOOTER_HP, chase,
};
pub use config::{Difficulty, RunConfig, Tuning, VentStyle, per_tick};
pub use gun::PhasePistol;
pub use hull::CORVETTE;
pub use input::{Buttons, MOVE_BUCKETS, PlayerInput, TickInputs};
pub use path::FlowField;
pub use player::{ASSIST_CONE, MAX_HP, PLAYER_RADIUS, Player};
pub use pool::POOL;
pub use rng::Rng;
pub use ship::{HatchId, HatchState, Ship};

use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Fixed simulation rate. Speeds and timers derive from this; render interpolates up to
/// the display rate.
pub const TICK_HZ: u32 = 60;

/// Player slots. Single-player is slot 0.
pub const MAX_PLAYERS: usize = 4;

/// The sim's only numeric type for world math: ±2^31 range, 2^-32 resolution.
pub type Fx = fixed::types::I32F32;

/// World-space vector in floor points: the origin is the floor grid's top-left corner and
/// +y points down (see [`room`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FxVec2 {
    pub x: Fx,
    pub y: Fx,
}

/// Index into the ship's rooms: a region of the floor.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct RoomId(pub u16);

/// The run's state machine. Each variant owns its data; transitions happen only in [`step`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Run {
    /// Exploring: no fight on.
    Boarding,
    /// Fighting `room`'s waves; its hatches are sealed until the last one dies.
    Encounter {
        room: RoomId,
        wave: u8,
    },
    Dead {
        ticks_until_restart: u32,
    },
    /// Extracted: the world stays frozen as it was.
    Won,
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
    /// The static floor, shared: cloning the state copies the pointer. It hashes as its
    /// own build-time checksum.
    pub ship: Arc<Ship>,
    /// Live state of each of the ship's hatches, by [`HatchId`].
    pub hatches: Vec<HatchState>,
    /// `None` = empty slot. An occupied slot with 0 HP is a dead player.
    pub players: [Option<Player>; MAX_PLAYERS],
    /// The fighting room's enemies: they spawn on entry and never leave it.
    pub enemies: Arena<Enemy>,
    /// The players' bullets.
    pub bullets: Arena<Bullet>,
    pub enemy_bullets: Arena<Bullet>,
    /// Bit `i` set = room `i` is revealed (ETG fog): a player started in it or opened a
    /// hatch into it. The future minimap's explored set.
    pub visited: u64,
    /// Bit `i` set = room `i` is cleared, so re-entering it starts no encounter.
    pub cleared: u64,
    /// Difficulty and tunables. The host may swap them mid-run ([`Self::set_config`]);
    /// restarts keep them.
    pub config: RunConfig,
}

impl SimState {
    /// A fresh single-player run: a Corvette generated from `seed`, the party standing in
    /// its start room with every hatch closed. `config` is clamped to what the sim handles (see [`RunConfig::sanitized`]).
    #[must_use]
    pub fn new(seed: u64, config: RunConfig) -> Self {
        let config = config.sanitized();
        let ship = Ship::generate(&CORVETTE, seed);
        let (room, at) = ship.start();
        let player = Player {
            pos: at,
            solid: at,
            ..Player::new(&config.tuning)
        };
        Self {
            tick: 0,
            seed,
            rng: Rng::from_seed(seed),
            run: Run::Boarding,
            hatches: vec![HatchState::Closed; ship.hatches().len()],
            ship: Arc::new(ship),
            players: [Some(player), None, None, None],
            enemies: Arena::default(),
            bullets: Arena::default(),
            enemy_bullets: Arena::default(),
            visited: bit(room),
            cleared: 0,
            config,
        }
    }

    /// A fresh run from the next run seed, with the same occupied slots and config. The
    /// tick keeps counting: it is session time, and presentation keys effects by it.
    #[must_use]
    pub fn restarted(&self) -> Self {
        let mut fresh = Self::new(Rng::next_seed(self.seed), self.config);
        fresh.tick = self.tick;
        let start = fresh.players[0];
        for (slot, old) in fresh.players.iter_mut().zip(&self.players) {
            *slot = old.and(start);
        }
        fresh
    }

    /// Swaps in `config` (sanitized) from the next tick: settings changed mid-run. Guns
    /// holding more charges than the new cap drop to it.
    pub fn set_config(&mut self, config: RunConfig) {
        self.config = config.sanitized();
        let cap = self.config.tuning.charges;
        for player in self.players.iter_mut().flatten() {
            player.gun.charges = player.gun.charges.min(cap);
        }
    }

    /// The floor's collision view this tick.
    #[must_use]
    pub fn tiles(&self) -> ship::Tiles<'_> {
        ship::Tiles::new(&self.ship, &self.hatches)
    }

    #[must_use]
    pub fn visited(&self, room: RoomId) -> bool {
        self.visited & bit(room) != 0
    }

    #[must_use]
    pub fn cleared(&self, room: RoomId) -> bool {
        self.cleared & bit(room) != 0
    }

    /// Whether `room` has nothing left to fight: every wave cleared, or none to begin
    /// with. Its extraction pad, if any, wins only then; presentation lights it from this
    /// too.
    #[must_use]
    pub fn extraction_live(&self, room: RoomId) -> bool {
        self.cleared(room) || self.ship.room(room).is_some_and(|r| !r.room.has_enemies())
    }

    /// Endianness-pinned hash of the whole state, comparable across machines.
    #[must_use]
    pub fn checksum(&self) -> u64 {
        checksum::of(self)
    }
}

/// Room `room`'s bit in the visited and cleared sets; none past [`room::MAX_ROOMS`].
fn bit(room: RoomId) -> u64 {
    1_u64.checked_shl(u32::from(room.0)).unwrap_or(0)
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
    /// A shooter fired (one event per volley, however many pellets).
    EnemyFired {
        enemy: EnemyId,
    },
    /// An unaware enemy heard a hatch and went to look (see [`Awareness`]).
    EnemyInvestigating {
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
    /// A player stepped into a closed hatch, opening it.
    HatchOpened {
        hatch: HatchId,
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
        Run::Boarding | Run::Encounter { .. } if restart_pressed => {
            *state = state.restarted();
            events.events.push(Event::Restarted);
        }
        Run::Boarding | Run::Encounter { .. } => combat::tick(state, inputs, &mut events),
        Run::Dead {
            ticks_until_restart,
        } if *ticks_until_restart > 0 => {
            *ticks_until_restart = ticks_until_restart.saturating_sub(1);
        }
        Run::Dead { .. } | Run::Won if restart_pressed => {
            *state = state.restarted();
            events.events.push(Event::Restarted);
        }
        Run::Dead { .. } | Run::Won => {}
    }

    state.tick = state.tick.wrapping_add(1);
    events
}
