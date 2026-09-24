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
mod input;
mod player;
mod rng;
pub mod trig;

pub use arena::{Arena, Id};
pub use combat::{
    BULLET_RADIUS, Bullet, DEATH_TICKS, Enemy, EnemyId, FIRST_SPAWN_TICKS, RUSHER_HP,
    RUSHER_RADIUS, SPAWN_TELEGRAPH_TICKS,
};
pub use input::{Buttons, MOVE_BUCKETS, PlayerInput, TickInputs};
pub use player::{
    ASSIST_CONE, FIRE_INTERVAL, HURT_TICKS, MAX_HP, PLAYER_RADIUS, Player, ROLL_COOLDOWN_TICKS,
    ROLL_TICKS, ROOM_HALF,
};
pub use rng::Rng;

use serde::{Deserialize, Serialize};

/// Fixed simulation rate. Speeds and timers derive from this; render interpolates up to
/// the display rate.
pub const TICK_HZ: u32 = 60;

/// Player slots. Single-player is slot 0.
pub const MAX_PLAYERS: usize = 4;

/// The sim's only numeric type for world math: ±2^31 range, 2^-32 resolution.
pub type Fx = fixed::types::I32F32;

/// World-space vector. World units are screen points; +y points down.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FxVec2 {
    pub x: Fx,
    pub y: Fx,
}

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
        ticks_until_restart: u32,
    },
    Won,
}

impl Run {
    const START: Self = Self::Boarding { room: RoomId(0) };
}

/// Everything that affects simulation results. Nothing outside this struct may.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SimState {
    pub tick: u64,
    /// The run seed; restart re-seeds from it.
    pub seed: u64,
    pub rng: Rng,
    pub run: Run,
    /// `None` = empty slot. An occupied slot with 0 HP is a dead player.
    pub players: [Option<Player>; MAX_PLAYERS],
    pub enemies: Arena<Enemy>,
    pub bullets: Arena<Bullet>,
    /// TEMPORARY (until the Rooms step): ticks until the placeholder spawner may add a
    /// rusher.
    pub spawn_cooldown: u16,
}

impl SimState {
    /// A fresh single-player run.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self {
            tick: 0,
            seed,
            rng: Rng::from_seed(seed),
            run: Run::START,
            players: [Some(Player::default()), None, None, None],
            enemies: Arena::default(),
            bullets: Arena::default(),
            spawn_cooldown: FIRST_SPAWN_TICKS,
        }
    }

    /// A fresh run from the same seed with the same occupied slots. The tick keeps
    /// counting: it is session time, and presentation keys effects by it.
    #[must_use]
    pub fn restarted(&self) -> Self {
        let mut fresh = Self::new(self.seed);
        fresh.tick = self.tick;
        for (slot, old) in fresh.players.iter_mut().zip(&self.players) {
            *slot = old.map(|_| Player::default());
        }
        fresh
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
    /// Also preceded by the killing `EnemyHit`. `pos` is where it died; the ID is stale.
    EnemyKilled {
        enemy: EnemyId,
        pos: FxVec2,
    },
    PlayerHit {
        slot: usize,
    },
    /// Also preceded by the killing `PlayerHit`.
    PlayerDied {
        slot: usize,
    },
    Restarted,
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
    // Only a live run simulates; the world stays frozen while Dead or Won.
    match &mut state.run {
        Run::Boarding { .. } | Run::Encounter { .. } => combat::tick(state, inputs, &mut events),
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
