//! What a player's screen shows. Render draws through [`center`]; an unaware enemy notices
//! a player that has it on screen ([`Screen`]). Sharing the math keeps the two in step.

use crate::room::{CELL, PrototypeRoom};
use crate::{Fx, FxVec2};
use serde::{Deserialize, Serialize};

/// A player's viewport size in points; one world unit is one point. It varies by device
/// and orientation, so it reaches the sim as input.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct View {
    pub width: u16,
    pub height: u16,
}

/// The view assumed for input that carries none (tests, bots, scripts): a landscape
/// iPhone, 852 x 393 pt.
pub const DEFAULT_VIEW: View = View {
    width: 852,
    height: 393,
};

impl View {
    /// This view, or [`DEFAULT_VIEW`] if it is zero on either axis.
    #[must_use]
    pub const fn or_default(self) -> Self {
        if self.width == 0 || self.height == 0 {
            DEFAULT_VIEW
        } else {
            self
        }
    }
}

/// Where the view's center sits in room space: on `focus` (the player), clamped so the
/// view stays inside the room. An axis where the room fits in the view centers the room.
#[must_use]
pub fn center(room: &PrototypeRoom, focus: FxVec2, view: View) -> FxVec2 {
    let extent = |cells: usize| CELL.saturating_mul(Fx::saturating_from_num(cells));
    let axis = |room: Fx, screen: u16, focus: Fx| {
        let half = Fx::from_num(screen).wrapping_div_int(2);
        if room <= Fx::from_num(screen) {
            room.wrapping_div_int(2)
        } else {
            focus.max(half).min(room.saturating_sub(half))
        }
    };
    FxVec2 {
        x: axis(extent(room.width()), view.width, focus.x),
        y: axis(extent(room.height()), view.height, focus.y),
    }
}

/// The room-space rect a player's screen shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Screen {
    min: FxVec2,
    max: FxVec2,
}

impl Screen {
    /// The screen of a player at `focus` with `view` ([`DEFAULT_VIEW`] if zero).
    #[must_use]
    pub fn of(room: &PrototypeRoom, focus: FxVec2, view: View) -> Self {
        let view = view.or_default();
        let c = center(room, focus, view);
        let half_w = Fx::from_num(view.width).wrapping_div_int(2);
        let half_h = Fx::from_num(view.height).wrapping_div_int(2);
        Self {
            min: FxVec2 {
                x: c.x.saturating_sub(half_w),
                y: c.y.saturating_sub(half_h),
            },
            max: FxVec2 {
                x: c.x.saturating_add(half_w),
                y: c.y.saturating_add(half_h),
            },
        }
    }

    /// Whether `point` is on screen, edges included.
    #[must_use]
    pub fn contains(&self, point: FxVec2) -> bool {
        (self.min.x..=self.max.x).contains(&point.x) && (self.min.y..=self.max.y).contains(&point.y)
    }
}
