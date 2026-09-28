//! The phase pistol, the starter gun (issue #15): a few charges, a fire-rate cap, and a
//! vent that refills it. ETG's reload rhythm is burst -> reposition -> burst; venting
//! keeps ticking through a roll, so "vent, then roll" is the natural combo.

use crate::config::{Tuning, VentStyle};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PhasePistol {
    pub charges: u8,
    /// Ticks until the fire-rate cap allows another shot.
    pub cooldown: u16,
    /// Vent left; nonzero = venting: no firing, and every charge returns when it ends.
    pub vent_ticks: u16,
    /// The current (or last) vent's full length, for progress display.
    pub vent_length: u16,
    /// Vent style B: ticks until the next charge regenerates.
    pub regen_ticks: u16,
}

impl PhasePistol {
    /// Fully charged.
    #[must_use]
    pub const fn new(tuning: &Tuning) -> Self {
        Self {
            charges: tuning.charges,
            cooldown: 0,
            vent_ticks: 0,
            vent_length: 0,
            regen_ticks: 0,
        }
    }

    #[must_use]
    pub const fn venting(&self) -> bool {
        self.vent_ticks > 0
    }

    /// One tick. `trigger`: fire is held and allowed (not mid-roll); `vent`: the vent
    /// button. Returns whether a shot fires this tick.
    ///
    /// Firing the last charge vents automatically: for `vent_ticks` in style A, half as
    /// long again in style B. A manual vent (style A or B) empties the gun and takes
    /// `vent_ticks`; it does nothing while venting or at full charge.
    pub fn tick(&mut self, tuning: &Tuning, trigger: bool, vent: bool) -> bool {
        if self.venting() {
            self.vent_ticks = self.vent_ticks.saturating_sub(1);
            if !self.venting() {
                self.charges = tuning.charges;
            }
        }
        if vent && !self.venting() && self.charges < tuning.charges {
            self.start_vent(tuning.vent_ticks);
        }
        let fired = trigger && self.cooldown == 0 && self.charges > 0 && !self.venting();
        if fired {
            self.charges = self.charges.saturating_sub(1);
            self.cooldown = tuning.fire_interval;
            self.regen_ticks = tuning.regen_delay_ticks;
            if self.charges == 0 {
                self.start_vent(match tuning.vent_style {
                    VentStyle::Clip => tuning.vent_ticks,
                    VentStyle::Regen => tuning.vent_ticks.saturating_mul(3) / 2,
                });
            }
        } else if tuning.vent_style == VentStyle::Regen
            && !self.venting()
            && self.charges < tuning.charges
        {
            self.regen_ticks = self.regen_ticks.saturating_sub(1);
            if self.regen_ticks == 0 {
                self.charges = self.charges.saturating_add(1);
                self.regen_ticks = tuning.regen_charge_ticks;
            }
        }
        self.cooldown = self.cooldown.saturating_sub(1);
        fired
    }

    const fn start_vent(&mut self, length: u16) {
        self.charges = 0;
        self.vent_ticks = length;
        self.vent_length = length;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ticks (0-based) on which a shot fires over `ticks` ticks of `input(tick)`.
    fn shots(
        gun: &mut PhasePistol,
        tuning: &Tuning,
        ticks: u16,
        input: impl Fn(u16) -> (bool, bool),
    ) -> Vec<u16> {
        (0..ticks)
            .filter(|&t| {
                let (trigger, vent) = input(t);
                gun.tick(tuning, trigger, vent)
            })
            .collect()
    }

    fn held(_: u16) -> (bool, bool) {
        (true, false)
    }

    #[test]
    fn held_fire_empties_the_charges_at_the_cap_then_vents_and_refills() {
        let tuning = Tuning::NORMAL;
        let mut gun = PhasePistol::new(&tuning);
        // 6 charges 17 ticks apart; the 6th (tick 85) starts the 66-tick vent, and the
        // refill lands in time for a shot on tick 85 + 66.
        assert_eq!(
            shots(&mut gun, &tuning, 170, held),
            [0, 17, 34, 51, 68, 85, 151, 168]
        );
    }

    #[test]
    fn venting_blocks_fire_and_reports_progress() {
        let tuning = Tuning::NORMAL;
        let mut gun = PhasePistol::new(&tuning);
        shots(&mut gun, &tuning, 86, held);
        assert_eq!((gun.charges, gun.vent_ticks, gun.vent_length), (0, 66, 66));
        assert!(gun.venting());
        assert!(shots(&mut gun, &tuning, 65, held).is_empty());
        assert!(
            gun.tick(&tuning, true, false),
            "refilled on the vent's last tick"
        );
        assert_eq!(gun.charges, 5);
    }

    #[test]
    fn manual_vent_empties_then_refills_and_is_ignored_when_full_or_venting() {
        let tuning = Tuning::NORMAL;
        let mut gun = PhasePistol::new(&tuning);
        gun.tick(&tuning, false, true);
        assert!(!gun.venting(), "full: nothing to vent");
        shots(&mut gun, &tuning, 18, held); // 2 shots
        assert_eq!(gun.charges, 4);
        gun.tick(&tuning, false, true);
        assert_eq!((gun.charges, gun.vent_ticks), (0, 66));
        gun.tick(&tuning, false, true);
        assert_eq!(gun.vent_ticks, 65, "a second press doesn't restart it");
        // Held fire through the vent: the first shot lands as it refills.
        assert_eq!(shots(&mut gun, &tuning, 70, held), [64]);
        assert_eq!(gun.charges, 5);
    }

    #[test]
    fn fire_cap_holds_however_fast_the_trigger_is_tapped() {
        let tuning = Tuning::NORMAL;
        let mut gun = PhasePistol::new(&tuning);
        // A tap every other tick still fires at most every 17 ticks.
        let fired = shots(&mut gun, &tuning, 60, |t| (t % 2 == 0, false));
        assert_eq!(fired, [0, 18, 36, 54]);
    }

    #[test]
    fn style_b_regenerates_one_charge_at_a_time_after_a_pause() {
        let tuning = Tuning {
            vent_style: VentStyle::Regen,
            ..Tuning::NORMAL
        };
        let mut gun = PhasePistol::new(&tuning);
        shots(&mut gun, &tuning, 35, held); // 3 shots: ticks 0, 17, 34
        assert_eq!(gun.charges, 3);
        let mut counts = Vec::new();
        for _ in 0..120 {
            gun.tick(&tuning, false, false);
            counts.push(gun.charges);
        }
        // The first charge `regen_delay_ticks` after the last shot, then one per
        // `regen_charge_ticks`.
        let (delay, per) = (tuning.regen_delay_ticks, tuning.regen_charge_ticks);
        let at = |tick: u16| counts[usize::from(tick) - 1];
        assert_eq!(at(delay - 1), 3);
        assert_eq!(at(delay), 4);
        assert_eq!(at(delay + per), 5);
        assert_eq!(at(delay + 2 * per), 6);
        assert_eq!(counts.last(), Some(&6), "stops at full");
    }

    #[test]
    fn style_b_emptying_forces_a_longer_vent() {
        let tuning = Tuning {
            vent_style: VentStyle::Regen,
            ..Tuning::NORMAL
        };
        let mut gun = PhasePistol::new(&tuning);
        // Held fire never pauses long enough to regenerate, so it empties at tick 85
        // into a 99-tick vent (1.5x the 66-tick A vent).
        assert_eq!(
            shots(&mut gun, &tuning, 190, held),
            [0, 17, 34, 51, 68, 85, 184]
        );
        let mut manual = PhasePistol::new(&tuning);
        manual.tick(&tuning, true, false);
        manual.tick(&tuning, false, true);
        assert_eq!(manual.vent_ticks, 66, "a manual vent is the normal length");
    }
}
