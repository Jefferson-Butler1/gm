//! Touch -> quantized [`PlayerInput`]: floating twin sticks (left half moves, right half
//! aims and fires), ported from the feel spike.

use crate::TouchPhase;
use sim::{Buttons, MOVE_BUCKETS, PlayerInput};
use std::f32::consts::TAU;

/// Stick travel in points.
const STICK_RADIUS: f32 = 60.0;
const MOVE_DEADZONE: f32 = 0.1;
const AIM_DEADZONE: f32 = 0.2;

struct Stick {
    id: u64,
    origin: [f32; 2],
    cur: [f32; 2],
}

impl Stick {
    /// Floating stick: the origin follows the finger once it passes the radius.
    fn drag(&mut self, p: [f32; 2]) {
        self.cur = p;
        let [dx, dy] = [p[0] - self.origin[0], p[1] - self.origin[1]];
        let len = dx.hypot(dy);
        if len > STICK_RADIUS {
            let k = (len - STICK_RADIUS) / len;
            self.origin = [dx.mul_add(k, self.origin[0]), dy.mul_add(k, self.origin[1])];
        }
    }

    /// Deflection as (angle in turns `0..1` from +x toward +y, magnitude `0..=1`).
    fn polar(&self) -> (f32, f32) {
        let [dx, dy] = [self.cur[0] - self.origin[0], self.cur[1] - self.origin[1]];
        let turns = (dy.atan2(dx) / TAU).rem_euclid(1.0);
        (turns, (dx.hypot(dy) / STICK_RADIUS).min(1.0))
    }
}

pub struct Controls {
    point_width: f32,
    left: Option<Stick>,
    right: Option<Stick>,
}

impl Controls {
    pub const fn new(point_width: f32) -> Self {
        Self {
            point_width,
            left: None,
            right: None,
        }
    }

    pub fn touch(&mut self, id: u64, phase: TouchPhase, x: f32, y: f32) {
        let p = [x, y];
        match phase {
            TouchPhase::Began => {
                let slot = if x < self.point_width * 0.5 {
                    &mut self.left
                } else {
                    &mut self.right
                };
                if slot.is_none() {
                    *slot = Some(Stick {
                        id,
                        origin: p,
                        cur: p,
                    });
                }
            }
            TouchPhase::Moved => {
                for stick in [&mut self.left, &mut self.right].into_iter().flatten() {
                    if stick.id == id {
                        stick.drag(p);
                    }
                }
            }
            TouchPhase::Ended => {
                for slot in [&mut self.left, &mut self.right] {
                    if slot.as_ref().is_some_and(|s| s.id == id) {
                        *slot = None;
                    }
                }
            }
        }
    }

    pub fn input(&self) -> PlayerInput {
        let mut input = PlayerInput::default();
        if let Some((turns, mag)) = self.left.as_ref().map(Stick::polar)
            && mag > MOVE_DEADZONE
        {
            let buckets = u32::from(MOVE_BUCKETS);
            // Rounding up to `buckets` wraps to bucket 0.
            input.move_dir = quantize(turns, buckets)
                .checked_rem(buckets)
                .and_then(|b| u8::try_from(b).ok())
                .unwrap_or(0);
            input.move_mag = u8::try_from(quantize(mag, 255)).unwrap_or(u8::MAX);
        }
        if let Some((turns, mag)) = self.right.as_ref().map(Stick::polar)
            && mag > AIM_DEADZONE
        {
            input.aim = u16::try_from(quantize(turns, 1 << 16) & 0xFFFF).unwrap_or(0);
            input.buttons |= Buttons::FIRE;
        }
        input
    }
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
