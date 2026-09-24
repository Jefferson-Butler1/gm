//! Touch -> quantized [`PlayerInput`] for the four control schemes (issue #7), ported from
//! the feel spike. Schemes only change how touches become input; aim assist and auto-aim
//! are sim rules.

use crate::TouchPhase;
use render::{DodgeView, Overlay, StickView};
use sim::{Buttons, MOVE_BUCKETS, PlayerInput};
use std::f32::consts::TAU;
use std::time::{Duration, Instant};

/// Stick travel in points.
const STICK_RADIUS: f32 = 60.0;
/// Scheme B: a touch this close to a fixed base grabs it.
const FIXED_GRAB_RADIUS: f32 = 130.0;
/// Fixed stick bases sit this far in from the safe-area corner, in points.
const BASE_INSET: f32 = 100.0;
const DODGE_RADIUS: f32 = 34.0;
/// Touches register a little outside the drawn button.
const DODGE_HIT_RADIUS: f32 = DODGE_RADIUS * 1.3;
/// Dodge button offset from the right stick base: up and toward the edge, clear of the
/// stick's travel.
const DODGE_OFFSET: [f32; 2] = [56.0, -110.0];
const MOVE_DEADZONE: f32 = 0.1;
const AIM_DEADZONE: f32 = 0.2;
/// Scheme C: a right-half swipe this far within [`FLICK_TIME`] rolls in its direction.
const FLICK_DISTANCE: f32 = 45.0;
const FLICK_TIME: Duration = Duration::from_millis(200);

/// Touch control schemes, chosen in the Controls setting.
#[derive(uniffi::Enum, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scheme {
    /// A: floating twin sticks (left half moves, right half aims and fires); dodge button.
    FloatingSticks,
    /// B: stick bases anchored near the bottom corners; dodge button.
    #[default]
    FixedSticks,
    /// C: floating move stick; holding the right half fires at the nearest target and a
    /// flick there dodges.
    AutoAim,
    /// D: like A, with aim bent toward targets by the assist strength.
    AimAssist,
}

/// View size and safe-area insets, in points.
#[derive(uniffi::Record, Clone, Copy, Debug, Default, PartialEq)]
pub struct Viewport {
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub point_width: f32,
    pub point_height: f32,
    pub safe_top: f32,
    pub safe_left: f32,
    pub safe_bottom: f32,
    pub safe_right: f32,
}

/// Where the on-screen controls sit for a viewport.
struct Layout {
    width: f32,
    bases: [[f32; 2]; 2],
    dodge: [f32; 2],
}

impl Layout {
    fn new(v: &Viewport) -> Self {
        let y = v.point_height - v.safe_bottom - BASE_INSET;
        let right = [v.point_width - v.safe_right - BASE_INSET, y];
        Self {
            width: v.point_width,
            bases: [[v.safe_left + BASE_INSET, y], right],
            dodge: [right[0] + DODGE_OFFSET[0], right[1] + DODGE_OFFSET[1]],
        }
    }
}

struct Stick {
    id: u64,
    origin: [f32; 2],
    cur: [f32; 2],
    /// Fixed sticks keep their base; floating ones drag it along past the radius.
    fixed: bool,
    start: [f32; 2],
    started: Instant,
    flicked: bool,
}

impl Stick {
    fn new(id: u64, origin: [f32; 2], p: [f32; 2], fixed: bool) -> Self {
        Self {
            id,
            origin,
            cur: p,
            fixed,
            start: p,
            started: Instant::now(),
            flicked: false,
        }
    }

    fn drag(&mut self, p: [f32; 2]) {
        self.cur = p;
        let [dx, dy] = sub(p, self.origin);
        let len = dx.hypot(dy);
        if !self.fixed && len > STICK_RADIUS {
            let k = (len - STICK_RADIUS) / len;
            self.origin = [dx.mul_add(k, self.origin[0]), dy.mul_add(k, self.origin[1])];
        }
    }

    /// Deflection as (angle in turns `0..1` from +x toward +y, magnitude `0..=1`).
    fn polar(&self) -> (f32, f32) {
        let [dx, dy] = sub(self.cur, self.origin);
        (turns(dx, dy), (dx.hypot(dy) / STICK_RADIUS).min(1.0))
    }

    fn knob(&self) -> [f32; 2] {
        let [dx, dy] = sub(self.cur, self.origin);
        let k = STICK_RADIUS / dx.hypot(dy).max(STICK_RADIUS);
        [dx.mul_add(k, self.origin[0]), dy.mul_add(k, self.origin[1])]
    }
}

/// A dodge waiting for the next sim tick.
#[derive(Clone, Copy)]
enum Dodge {
    Button,
    /// Scheme C: roll along the flick, whatever the move stick says.
    Flick {
        move_dir: u8,
    },
}

pub struct Controls {
    scheme: Scheme,
    /// Aim assist for scheme D, `0..=1`.
    assist: f32,
    layout: Layout,
    sticks: [Option<Stick>; 2],
    dodge: Option<Dodge>,
}

impl Controls {
    pub fn new(viewport: &Viewport) -> Self {
        Self {
            scheme: Scheme::default(),
            assist: 0.5,
            layout: Layout::new(viewport),
            sticks: [None, None],
            dodge: None,
        }
    }

    /// Stick origins belong to the old layout; rotation cancels the touches anyway.
    pub fn set_viewport(&mut self, viewport: &Viewport) {
        self.layout = Layout::new(viewport);
        self.sticks = [None, None];
    }

    pub const fn set_scheme(&mut self, scheme: Scheme) {
        self.scheme = scheme;
        self.sticks = [None, None];
    }

    pub const fn set_assist(&mut self, strength: f32) {
        self.assist = strength.clamp(0.0, 1.0);
    }

    pub fn touch(&mut self, id: u64, phase: TouchPhase, x: f32, y: f32) {
        let p = [x, y];
        match phase {
            TouchPhase::Began => self.begin(id, p),
            TouchPhase::Moved => {
                for stick in self.sticks.iter_mut().flatten() {
                    if stick.id == id {
                        stick.drag(p);
                    }
                }
                if self.scheme == Scheme::AutoAim
                    && let [_, Some(right)] = &mut self.sticks
                    && right.id == id
                    && !right.flicked
                {
                    let [dx, dy] = sub(p, right.start);
                    if dx.hypot(dy) > FLICK_DISTANCE && right.started.elapsed() < FLICK_TIME {
                        right.flicked = true;
                        self.dodge = Some(Dodge::Flick {
                            move_dir: move_bucket(turns(dx, dy)),
                        });
                    }
                }
            }
            TouchPhase::Ended => {
                for slot in &mut self.sticks {
                    if slot.as_ref().is_some_and(|s| s.id == id) {
                        *slot = None;
                    }
                }
            }
        }
    }

    fn begin(&mut self, id: u64, p: [f32; 2]) {
        if self.scheme != Scheme::AutoAim && dist(p, self.layout.dodge) < DODGE_HIT_RADIUS {
            self.dodge = Some(Dodge::Button);
            return;
        }
        let side = usize::from(p[0] >= self.layout.width * 0.5);
        let (Some(slot), Some(&base)) = (self.sticks.get_mut(side), self.layout.bases.get(side))
        else {
            return;
        };
        if slot.is_some() {
            return;
        }
        if self.scheme == Scheme::FixedSticks {
            if dist(p, base) < FIXED_GRAB_RADIUS {
                let mut stick = Stick::new(id, base, p, true);
                stick.drag(p);
                *slot = Some(stick);
            }
        } else {
            *slot = Some(Stick::new(id, p, p, false));
        }
    }

    /// Input for the next sim tick. A pending dodge goes out once, on the first tick that
    /// actually runs.
    pub fn next_input(&mut self) -> PlayerInput {
        let mut input = PlayerInput::default();
        let [left, right] = &self.sticks;
        if let Some((t, mag)) = left.as_ref().map(Stick::polar)
            && mag > MOVE_DEADZONE
        {
            input.move_dir = move_bucket(t);
            input.move_mag = u8::try_from(quantize(mag, 255)).unwrap_or(u8::MAX);
        }
        let aim = right
            .as_ref()
            .map(Stick::polar)
            .filter(|&(_, mag)| mag > AIM_DEADZONE);
        match self.scheme {
            Scheme::FloatingSticks | Scheme::FixedSticks | Scheme::AimAssist => {
                if let Some((t, _)) = aim {
                    input.aim = u16::try_from(quantize(t, 1 << 16) & 0xFFFF).unwrap_or(0);
                    input.buttons |= Buttons::FIRE;
                }
                if self.scheme == Scheme::AimAssist {
                    input.assist = u8::try_from(quantize(self.assist, 255)).unwrap_or(u8::MAX);
                }
            }
            Scheme::AutoAim => {
                if right.is_some() {
                    input.buttons |= Buttons::FIRE | Buttons::AUTO_AIM;
                }
            }
        }
        match self.dodge.take() {
            Some(Dodge::Button) => input.buttons |= Buttons::DODGE,
            Some(Dodge::Flick { move_dir }) => {
                input.buttons |= Buttons::DODGE;
                input.move_dir = move_dir;
                input.move_mag = u8::MAX;
            }
            None => {}
        }
        input
    }

    pub fn overlay(&self, roll_ready: bool) -> Overlay {
        let mut overlay = Overlay::default();
        for ((view, stick), &base) in overlay
            .sticks
            .iter_mut()
            .zip(&self.sticks)
            .zip(&self.layout.bases)
        {
            *view = match stick {
                Some(s) => Some(StickView {
                    base: s.origin,
                    radius: STICK_RADIUS,
                    knob: s.knob(),
                    active: true,
                }),
                None if self.scheme == Scheme::FixedSticks => Some(StickView {
                    base,
                    radius: STICK_RADIUS,
                    knob: base,
                    active: false,
                }),
                None => None,
            };
        }
        overlay.dodge = (self.scheme != Scheme::AutoAim).then_some(DodgeView {
            center: self.layout.dodge,
            radius: DODGE_RADIUS,
            ready: roll_ready,
        });
        overlay
    }
}

fn sub(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [a[0] - b[0], a[1] - b[1]]
}

fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    let [dx, dy] = sub(a, b);
    dx.hypot(dy)
}

/// Angle of (`dx`, `dy`) in turns `0..1` from +x toward +y.
fn turns(dx: f32, dy: f32) -> f32 {
    (dy.atan2(dx) / TAU).rem_euclid(1.0)
}

fn move_bucket(turns: f32) -> u8 {
    let buckets = u32::from(MOVE_BUCKETS);
    // Rounding up to `buckets` wraps to bucket 0.
    quantize(turns, buckets)
        .checked_rem(buckets)
        .and_then(|b| u8::try_from(b).ok())
        .unwrap_or(0)
}

/// Rounds `x` in `0..=1` to the nearest step in `0..=max`.
// Float -> int has no TryFrom; `x` is clamped so the result is always in range.
#[allow(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
fn quantize(x: f32, max: u32) -> u32 {
    (x.clamp(0.0, 1.0) * max as f32).round() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn controls(scheme: Scheme) -> Controls {
        let mut c = Controls::new(&Viewport {
            point_width: 852.0,
            point_height: 393.0,
            safe_left: 59.0,
            safe_right: 59.0,
            safe_bottom: 21.0,
            ..Viewport::default()
        });
        c.set_scheme(scheme);
        c
    }

    #[test]
    fn dodge_button_sends_one_dodge() {
        let mut c = controls(Scheme::FixedSticks);
        let [x, y] = c.layout.dodge;
        c.touch(1, TouchPhase::Began, x, y);
        assert!(c.next_input().buttons.contains(Buttons::DODGE));
        assert!(!c.next_input().buttons.contains(Buttons::DODGE));
    }

    #[test]
    fn fixed_sticks_only_grab_near_their_base() {
        let mut c = controls(Scheme::FixedSticks);
        let [x, y] = c.layout.bases[0];
        c.touch(1, TouchPhase::Began, x, 20.0); // far above the left base
        c.touch(2, TouchPhase::Began, x + STICK_RADIUS, y); // full right
        let input = c.next_input();
        assert_eq!((input.move_dir, input.move_mag), (0, 255));
    }

    #[test]
    fn flick_dodges_along_the_flick() {
        let mut c = controls(Scheme::AutoAim);
        c.touch(1, TouchPhase::Began, 600.0, 200.0);
        c.touch(1, TouchPhase::Moved, 600.0, 150.0); // 50 pt straight up
        let input = c.next_input();
        assert!(
            input
                .buttons
                .contains(Buttons::DODGE | Buttons::FIRE | Buttons::AUTO_AIM)
        );
        assert_eq!((input.move_dir, input.move_mag), (24, 255));
    }
}
