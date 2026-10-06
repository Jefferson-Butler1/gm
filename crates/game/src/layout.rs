//! Where the on-screen controls sit: the player's arrangement from the layout editor, or
//! the defaults. A [`ControlLayout`] places each control as fractions of the safe area, so
//! it survives rotation and other screens, and scales its default radius.

use crate::controls::{Scheme, Viewport};

/// Stick travel in points, at size 1.
pub const STICK_RADIUS: f32 = 60.0;
const STICK_KNOB_RADIUS: f32 = 22.0;
/// Fixed sticks: a touch this close to the base grabs it, at size 1.
const FIXED_GRAB_RADIUS: f32 = 130.0;
/// Fixed stick bases sit this far in from the safe-area corner, in points.
const BASE_INSET: f32 = 100.0;
const DODGE_RADIUS: f32 = 34.0;
/// Dodge button offset from the right stick base: up and toward the edge, clear of the
/// stick's travel.
const DODGE_OFFSET: [f32; 2] = [56.0, -110.0];
const VENT_RADIUS: f32 = 26.0;
/// Vent button offset from the right stick base: up and inward, clear of the dodge
/// button and the stick's travel.
const VENT_OFFSET: [f32; 2] = [-50.0, -130.0];
const EMP_RADIUS: f32 = 26.0;
/// EMP button offset from the right stick base: up and inward past the vent button,
/// clear of the stick's grab and the minimap.
const EMP_OFFSET: [f32; 2] = [-130.0, -100.0];
/// Claw: the EMP button, below the left index finger's dodge button.
const CLAW_EMP_INSET: [f32; 2] = [90.0, 240.0];
/// Claw: the right index finger's fire button, in from the top-right safe corner and clear
/// of the settings button.
const CLAW_FIRE_INSET: [f32; 2] = [90.0, 130.0];
const CLAW_FIRE_RADIUS: f32 = 44.0;
/// Claw: the left index finger's dodge button, in from the top-left safe corner and below
/// the HUD.
const CLAW_DODGE_INSET: [f32; 2] = [90.0, 150.0];
/// Touches register a little outside a drawn button.
const HIT_SLOP: f32 = 1.3;

/// One control's place.
#[derive(uniffi::Record, Clone, Copy, Debug, PartialEq)]
pub struct ControlPlacement {
    /// Center, as fractions of the safe area: 0 at its left (top) edge, 1 at its right
    /// (bottom).
    pub x: f32,
    pub y: f32,
    /// Scale on the control's default radius; a stick's travel and grab scale together.
    pub size: f32,
}

/// Every touch control's place. Schemes that don't use a control ignore it.
#[derive(uniffi::Record, Clone, Copy, Debug, PartialEq)]
pub struct ControlLayout {
    pub move_stick: ControlPlacement,
    /// The aim stick, or scheme F's fire button.
    pub aim_stick: ControlPlacement,
    pub dodge: ControlPlacement,
    pub vent: ControlPlacement,
    /// The claw's fire button.
    pub fire: ControlPlacement,
    pub emp: ControlPlacement,
}

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlKind {
    MoveStick,
    AimStick,
    Dodge,
    Vent,
    Fire,
    Emp,
}

impl ControlKind {
    const ALL: [Self; 6] = [
        Self::MoveStick,
        Self::AimStick,
        Self::Dodge,
        Self::Vent,
        Self::Fire,
        Self::Emp,
    ];

    /// (drawn radius, touch radius) at size 1.
    const fn radii(self) -> (f32, f32) {
        match self {
            Self::MoveStick | Self::AimStick => (STICK_RADIUS, FIXED_GRAB_RADIUS),
            Self::Dodge => (DODGE_RADIUS, DODGE_RADIUS * HIT_SLOP),
            Self::Vent => (VENT_RADIUS, VENT_RADIUS * HIT_SLOP),
            Self::Fire => (CLAW_FIRE_RADIUS, CLAW_FIRE_RADIUS * HIT_SLOP),
            Self::Emp => (EMP_RADIUS, EMP_RADIUS * HIT_SLOP),
        }
    }
}

impl ControlLayout {
    const fn get(&self, kind: ControlKind) -> ControlPlacement {
        match kind {
            ControlKind::MoveStick => self.move_stick,
            ControlKind::AimStick => self.aim_stick,
            ControlKind::Dodge => self.dodge,
            ControlKind::Vent => self.vent,
            ControlKind::Fire => self.fire,
            ControlKind::Emp => self.emp,
        }
    }
}

impl Scheme {
    /// Whether this scheme draws and hit-tests `kind`: anchored sticks only (floating ones
    /// appear under the thumb), dodge without flick-to-dodge, fire for the claw.
    pub(crate) const fn uses(self, kind: ControlKind) -> bool {
        match kind {
            ControlKind::MoveStick => self.fixed(0),
            ControlKind::AimStick => self.fixed(1),
            ControlKind::Dodge => !self.auto_aim(),
            ControlKind::Vent | ControlKind::Emp => true,
            ControlKind::Fire => matches!(self, Self::Claw),
        }
    }
}

/// The safe area's top-left corner and size, in points. The size never reaches 0, so
/// fractions stay finite.
fn safe_area(v: &Viewport) -> ([f32; 2], [f32; 2]) {
    let width = v.point_width - v.safe_left - v.safe_right;
    let height = v.point_height - v.safe_top - v.safe_bottom;
    ([v.safe_left, v.safe_top], [width.max(1.0), height.max(1.0)])
}

/// The built-in layout for `scheme` on `viewport`: stick bases in from the bottom safe
/// corners, dodge, vent and the EMP above the right one (the claw's dodge, EMP and fire
/// up top).
#[uniffi::export]
#[must_use]
pub fn default_control_layout(scheme: Scheme, viewport: Viewport) -> ControlLayout {
    let v = &viewport;
    let (origin, size) = safe_area(v);
    let place = |[x, y]: [f32; 2]| ControlPlacement {
        x: (x - origin[0]) / size[0],
        y: (y - origin[1]) / size[1],
        size: 1.0,
    };
    let y = v.point_height - v.safe_bottom - BASE_INSET;
    let right = [v.point_width - v.safe_right - BASE_INSET, y];
    let dodge = if scheme == Scheme::Claw {
        [
            v.safe_left + CLAW_DODGE_INSET[0],
            v.safe_top + CLAW_DODGE_INSET[1],
        ]
    } else {
        [right[0] + DODGE_OFFSET[0], right[1] + DODGE_OFFSET[1]]
    };
    let emp = if scheme == Scheme::Claw {
        [
            v.safe_left + CLAW_EMP_INSET[0],
            v.safe_top + CLAW_EMP_INSET[1],
        ]
    } else {
        [right[0] + EMP_OFFSET[0], right[1] + EMP_OFFSET[1]]
    };
    ControlLayout {
        move_stick: place([v.safe_left + BASE_INSET, y]),
        aim_stick: place(right),
        dodge: place(dodge),
        vent: place([right[0] + VENT_OFFSET[0], right[1] + VENT_OFFSET[1]]),
        fire: place([
            v.point_width - v.safe_right - CLAW_FIRE_INSET[0],
            v.safe_top + CLAW_FIRE_INSET[1],
        ]),
        emp: place(emp),
    }
}

/// The controls `scheme` shows, where `layout` puts them on `viewport`: what the layout
/// editor draws, and the resolved points it saves for reading back.
#[uniffi::export]
#[must_use]
pub fn place_controls(
    scheme: Scheme,
    layout: ControlLayout,
    viewport: Viewport,
) -> Vec<PlacedControl> {
    let resolved = Layout::new(&viewport, &layout);
    ControlKind::ALL
        .into_iter()
        .filter(|&kind| scheme.uses(kind))
        .map(|kind| {
            let spot = resolved.spot(kind);
            PlacedControl {
                kind,
                x: spot.at[0],
                y: spot.at[1],
                radius: spot.radius,
                hit_radius: spot.hit,
            }
        })
        .collect()
}

/// A control in view points.
#[derive(uniffi::Record, Clone, Copy, Debug, PartialEq)]
pub struct PlacedControl {
    pub kind: ControlKind,
    pub x: f32,
    pub y: f32,
    /// As drawn: a stick's travel, a button's disc.
    pub radius: f32,
    /// Touches this close register: a fixed stick's grab, a button's disc plus slop.
    pub hit_radius: f32,
}

/// A resolved control, in view points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spot {
    pub at: [f32; 2],
    pub radius: f32,
    pub hit: f32,
}

impl Spot {
    /// A stick's knob, scaled with its travel.
    pub fn knob_radius(&self) -> f32 {
        STICK_KNOB_RADIUS * self.radius / STICK_RADIUS
    }
}

/// A [`ControlLayout`] resolved on a viewport, in view points.
pub struct Layout {
    pub width: f32,
    /// Move, aim.
    pub sticks: [Spot; 2],
    pub dodge: Spot,
    pub vent: Spot,
    pub fire: Spot,
    pub emp: Spot,
}

impl Layout {
    pub fn new(v: &Viewport, layout: &ControlLayout) -> Self {
        let (origin, size) = safe_area(v);
        let spot = |kind: ControlKind| {
            let p = layout.get(kind);
            let (radius, hit) = kind.radii();
            Spot {
                at: [
                    p.x.mul_add(size[0], origin[0]),
                    p.y.mul_add(size[1], origin[1]),
                ],
                radius: radius * p.size,
                hit: hit * p.size,
            }
        };
        Self {
            width: v.point_width,
            sticks: [spot(ControlKind::MoveStick), spot(ControlKind::AimStick)],
            dodge: spot(ControlKind::Dodge),
            vent: spot(ControlKind::Vent),
            fire: spot(ControlKind::Fire),
            emp: spot(ControlKind::Emp),
        }
    }

    const fn spot(&self, kind: ControlKind) -> Spot {
        match kind {
            ControlKind::MoveStick => self.sticks[0],
            ControlKind::AimStick => self.sticks[1],
            ControlKind::Dodge => self.dodge,
            ControlKind::Vent => self.vent,
            ControlKind::Fire => self.fire,
            ControlKind::Emp => self.emp,
        }
    }
}
