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

mod checksum;
mod input;
mod rng;

pub use input::{Buttons, MOVE_BUCKETS, PlayerInput, TickInputs};
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Player {
    pub pos: FxVec2,
    /// Last aim angle while firing; one full turn = 65536.
    pub facing: u16,
}

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
    pub rng: Rng,
    pub run: Run,
    pub players: [Option<Player>; MAX_PLAYERS],
}

impl SimState {
    /// A fresh single-player run.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self {
            tick: 0,
            rng: Rng::from_seed(seed),
            run: Run::START,
            players: [Some(Player::default()), None, None, None],
        }
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
    Restarted,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TickEvents {
    pub events: Vec<Event>,
}

/// Advance the sim by exactly one tick. Pure: no clock, no I/O, no state outside `state`.
pub fn step(state: &mut SimState, inputs: &TickInputs) -> TickEvents {
    let mut events = TickEvents::default();

    for (player, input) in state.players.iter_mut().zip(&inputs.players) {
        if let Some(player) = player
            && input.buttons.contains(Buttons::FIRE)
        {
            player.facing = input.aim;
        }
    }

    let restart_pressed = inputs
        .players
        .iter()
        .any(|input| input.buttons.contains(Buttons::RESTART));
    match &mut state.run {
        Run::Dead {
            ticks_until_restart,
        } if *ticks_until_restart > 0 => {
            *ticks_until_restart = ticks_until_restart.saturating_sub(1);
        }
        Run::Dead { .. } | Run::Won if restart_pressed => {
            state.run = Run::START;
            events.events.push(Event::Restarted);
        }
        Run::Boarding { .. } | Run::Encounter { .. } | Run::Dead { .. } | Run::Won => {}
    }

    state.tick = state.tick.wrapping_add(1);
    events
}
