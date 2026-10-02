use crate::MAX_PLAYERS;
use serde::{Deserialize, Serialize};

/// Number of movement direction buckets in a full turn.
pub const MOVE_BUCKETS: u8 = 32;

/// One player's quantized input for one tick. Touch is converted to this before it reaches
/// the sim, so single-player exercises exactly what co-op would send. `Default` = no input.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PlayerInput {
    /// Direction bucket in `0..MOVE_BUCKETS`, measured from +x toward +y.
    pub move_dir: u8,
    /// Stick deflection: 0 = none, 255 = full.
    pub move_mag: u8,
    /// Aim angle from +x toward +y; one full turn = 65536. Read under `FIRE` (unless
    /// `AUTO_AIM`) or `AIM`.
    pub aim: u16,
    /// Aim-assist strength: 0 = raw aim, 255 = snap to the target in the assist cone.
    pub assist: u8,
    pub buttons: Buttons,
}

/// Button bitflags.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Buttons(pub u8);

impl Buttons {
    pub const FIRE: Self = Self(1);
    pub const DODGE: Self = Self(1 << 1);
    pub const INTERACT: Self = Self(1 << 2);
    pub const RESTART: Self = Self(1 << 3);
    /// With `FIRE`: aim at the nearest target in line of fire (else the nearest at all)
    /// instead of along `aim`.
    pub const AUTO_AIM: Self = Self(1 << 4);
    /// Vent the phase pistol now, refilling every charge (see `PhasePistol`).
    pub const VENT: Self = Self(1 << 5);
    /// The aim stick is held but not firing (tap and release fire modes): face `aim`
    /// as is, without assist or a shot. `FIRE` takes precedence.
    pub const AIM: Self = Self(1 << 6);

    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl std::ops::BitOr for Buttons {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for Buttons {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

/// Every slot's input for one tick.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TickInputs {
    pub players: [PlayerInput; MAX_PLAYERS],
}
