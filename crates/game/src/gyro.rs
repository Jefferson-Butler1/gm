//! Gyro aim (prototype setting): turning the phone nudges the aim angle while aiming or
//! firing, on top of the stick's aim or the auto-aim target, and recenters as soon as
//! neither is held. Swift reports the turn rate; this integrates it once per sim tick.

use crate::controls::{aim_angle, turns};
use sim::{Buttons, PlayerInput, TICK_HZ};
use std::f32::consts::TAU;

/// The nudge stops this far off the aim, in turns (45 degrees).
const MAX_NUDGE: f32 = 0.125;

#[derive(Default)]
pub struct GyroAim {
    /// Aim turn per phone turn; 0 = off.
    sensitivity: f32,
    /// The phone's latest turn rate in radians per second, clockwise on screen.
    rate: f32,
    /// The current nudge, in turns, clockwise.
    nudge: f32,
    /// Direction to the nearest target, in turns: what an auto-aimed shot is nudged off.
    target: Option<f32>,
}

impl GyroAim {
    pub const fn set_sensitivity(&mut self, sensitivity: f32) {
        self.sensitivity = sensitivity.max(0.0);
    }

    pub const fn set_rate(&mut self, radians_per_sec: f32) {
        self.rate = radians_per_sec;
    }

    /// The nearest target as an offset from the player, in world units (+y down).
    pub fn set_target(&mut self, offset: Option<[f32; 2]>) {
        self.target = offset.map(|[dx, dy]| turns(dx, dy));
    }

    /// Nudges one tick's `input`. An auto-aimed shot becomes a manual one at the target
    /// plus the nudge; without a known target it stays auto-aimed.
    pub fn apply(&mut self, input: &mut PlayerInput) {
        let aiming = input.buttons.contains(Buttons::FIRE) || input.buttons.contains(Buttons::AIM);
        let nudge = self.tick(aiming);
        if nudge.abs() < f32::EPSILON {
            return;
        }
        let from = if input.buttons.contains(Buttons::FIRE | Buttons::AUTO_AIM) {
            let Some(target) = self.target else { return };
            input.buttons = Buttons(input.buttons.0 & !Buttons::AUTO_AIM.0);
            target
        } else {
            f32::from(input.aim) / 65536.0
        };
        input.aim = aim_angle((from + nudge).rem_euclid(1.0));
    }

    /// One sim tick: grows the nudge by this tick's turn while `aiming`, else recenters.
    /// Returns the nudge in turns.
    fn tick(&mut self, aiming: bool) -> f32 {
        // No From<u32> for f32; TICK_HZ is small and exact.
        #[allow(clippy::as_conversions, clippy::cast_precision_loss)]
        let dt = 1.0 / TICK_HZ as f32;
        self.nudge = if aiming {
            (self.rate / TAU)
                .mul_add(self.sensitivity * dt, self.nudge)
                .clamp(-MAX_NUDGE, MAX_NUDGE)
        } else {
            0.0
        };
        self.nudge
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tick_for(gyro: &mut GyroAim, ticks: u32) -> f32 {
        (0..ticks).fold(0.0, |_, _| gyro.tick(true))
    }

    #[test]
    fn turning_nudges_by_the_turn_times_sensitivity_capped_and_recenters() {
        let mut gyro = GyroAim::default();
        gyro.set_sensitivity(0.5);
        gyro.set_rate(TAU / 16.0); // a sixteenth of a turn per second
        let nudge = tick_for(&mut gyro, TICK_HZ);
        assert!((nudge - 1.0 / 32.0).abs() < 1e-4, "{nudge}");
        gyro.set_rate(-TAU);
        assert!(
            (tick_for(&mut gyro, TICK_HZ) + MAX_NUDGE).abs() < 1e-6,
            "capped"
        );
        assert!(
            gyro.tick(false).abs() < f32::EPSILON,
            "recenters when not aiming"
        );
    }

    #[test]
    fn nudges_manual_aim_and_steers_auto_aim_off_the_target() {
        let mut gyro = GyroAim::default();
        gyro.set_sensitivity(1.0);
        gyro.set_rate(-TAU * 60.0); // counterclockwise, capped on the first tick
        let mut input = PlayerInput {
            aim: 0,
            buttons: Buttons::AIM,
            ..PlayerInput::default()
        };
        gyro.apply(&mut input);
        assert_eq!(
            input.aim, 57344,
            "an eighth of a turn counterclockwise, wrapped"
        );

        gyro.set_target(Some([0.0, 5.0])); // straight down
        let mut input = PlayerInput {
            buttons: Buttons::FIRE | Buttons::AUTO_AIM,
            ..PlayerInput::default()
        };
        gyro.apply(&mut input);
        assert_eq!(input.buttons, Buttons::FIRE);
        assert_eq!(input.aim, 8192, "a quarter turn less an eighth");

        let mut idle = PlayerInput::default();
        gyro.apply(&mut idle);
        assert_eq!(idle, PlayerInput::default(), "not aiming: untouched");
    }
}
