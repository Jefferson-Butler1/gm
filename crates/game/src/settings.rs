//! The run config as the settings screen sees it (issue #15): difficulty plus the key
//! tunables in human units (pt/s, seconds, shots/s). Converted to the sim's integer
//! [`sim::RunConfig`] when a run starts; tunables without a slider keep Normal's values.

use sim::{RunConfig, TICK_HZ, Tuning};

#[derive(uniffi::Enum, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Difficulty {
    Easy,
    #[default]
    Normal,
    Hard,
}

/// How the phase pistol recovers: A clip-style vents, B regenerating charges.
#[derive(uniffi::Enum, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VentStyle {
    #[default]
    Clip,
    Regen,
}

#[derive(uniffi::Record, Clone, Copy, Debug, PartialEq)]
pub struct RunSettings {
    pub difficulty: Difficulty,
    /// pt/s.
    pub move_speed: f32,
    /// pt.
    pub roll_distance: f32,
    pub roll_secs: f32,
    /// Share of the roll, from its start, with i-frames: `0..=1`.
    pub roll_iframe_fraction: f32,
    pub charges: u8,
    /// Fire-rate cap, shots/s.
    pub fire_rate: f32,
    pub vent_secs: f32,
    /// pt/s; `None` = the difficulty's.
    pub enemy_bullet_speed: Option<f32>,
    /// `None` = the difficulty's.
    pub shooter_interval_secs: Option<f32>,
    pub vent_style: VentStyle,
    /// Vent style B: pause after a shot before charges regenerate.
    pub regen_delay_secs: f32,
    /// Vent style B: time per regenerated charge.
    pub regen_charge_secs: f32,
    pub spread_shooter: bool,
    /// A fall into a pit, before the respawn.
    pub fall_secs: f32,
}

/// What a difficulty sets for the enemy tunables, for sliders without an override.
#[derive(uniffi::Record, Clone, Copy, Debug, PartialEq)]
pub struct EnemyDefaults {
    pub enemy_bullet_speed: f32,
    pub shooter_interval_secs: f32,
}

/// Normal's settings.
#[uniffi::export]
#[must_use]
pub fn default_run_settings() -> RunSettings {
    let t = Tuning::NORMAL;
    RunSettings {
        difficulty: Difficulty::Normal,
        move_speed: f32::from(t.move_speed),
        roll_distance: f32::from(t.roll_distance),
        roll_secs: secs(t.roll_ticks),
        roll_iframe_fraction: f32::from(t.roll_iframe_percent) / 100.0,
        charges: t.charges,
        fire_rate: tick_hz() / f32::from(t.fire_interval),
        vent_secs: secs(t.vent_ticks),
        enemy_bullet_speed: None,
        shooter_interval_secs: None,
        vent_style: VentStyle::Clip,
        regen_delay_secs: secs(t.regen_delay_ticks),
        regen_charge_secs: secs(t.regen_charge_ticks),
        spread_shooter: t.spread_shooter,
        fall_secs: secs(t.fall_ticks),
    }
}

#[uniffi::export]
#[must_use]
pub fn enemy_defaults(difficulty: Difficulty) -> EnemyDefaults {
    let d = sim_difficulty(difficulty);
    EnemyDefaults {
        enemy_bullet_speed: f32::from(d.enemy_bullet_speed()),
        shooter_interval_secs: secs(d.shooter_interval()),
    }
}

impl RunSettings {
    /// The sim config: values round to whole units and ticks (the fire cap to the nearest
    /// whole-tick interval); the sim clamps anything out of range.
    #[must_use]
    pub fn run_config(&self) -> RunConfig {
        RunConfig {
            difficulty: sim_difficulty(self.difficulty),
            tuning: Tuning {
                move_speed: whole(self.move_speed),
                roll_distance: whole(self.roll_distance),
                roll_ticks: ticks(self.roll_secs),
                roll_iframe_percent: u8::try_from(whole(self.roll_iframe_fraction * 100.0))
                    .unwrap_or(u8::MAX),
                charges: self.charges,
                fire_interval: whole(tick_hz() / self.fire_rate.max(0.1)),
                vent_ticks: ticks(self.vent_secs),
                enemy_bullet_speed: self.enemy_bullet_speed.map(whole),
                shooter_interval: self.shooter_interval_secs.map(ticks),
                vent_style: match self.vent_style {
                    VentStyle::Clip => sim::VentStyle::Clip,
                    VentStyle::Regen => sim::VentStyle::Regen,
                },
                regen_delay_ticks: ticks(self.regen_delay_secs),
                regen_charge_ticks: ticks(self.regen_charge_secs),
                spread_shooter: self.spread_shooter,
                fall_ticks: ticks(self.fall_secs),
                ..Tuning::NORMAL
            },
        }
    }
}

const fn sim_difficulty(difficulty: Difficulty) -> sim::Difficulty {
    match difficulty {
        Difficulty::Easy => sim::Difficulty::Easy,
        Difficulty::Normal => sim::Difficulty::Normal,
        Difficulty::Hard => sim::Difficulty::Hard,
    }
}

fn tick_hz() -> f32 {
    f32::from(u16::try_from(TICK_HZ).unwrap_or(u16::MAX))
}

fn secs(ticks: u16) -> f32 {
    f32::from(ticks) / tick_hz()
}

fn ticks(secs: f32) -> u16 {
    whole(secs * tick_hz())
}

/// Nearest whole number, clamped into `u16`.
// Float -> int has no TryFrom; the value is clamped so the cast is always in range.
#[allow(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn whole(x: f32) -> u16 {
    x.round().clamp(0.0, f32::from(u16::MAX)) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings_round_trip_to_normal() {
        assert_eq!(default_run_settings().run_config(), RunConfig::default());
    }

    #[test]
    fn overrides_and_units_convert() {
        let settings = RunSettings {
            difficulty: Difficulty::Hard,
            roll_secs: 0.5,
            fire_rate: 4.0,
            enemy_bullet_speed: Some(210.4),
            shooter_interval_secs: Some(2.5),
            vent_style: VentStyle::Regen,
            ..default_run_settings()
        };
        let config = settings.run_config();
        assert_eq!(config.difficulty, sim::Difficulty::Hard);
        let t = config.tuning;
        assert_eq!(
            (
                t.roll_ticks,
                t.fire_interval,
                t.enemy_bullet_speed,
                t.shooter_interval
            ),
            (30, 15, Some(210), Some(150))
        );
        assert_eq!(t.vent_style, sim::VentStyle::Regen);
    }
}
