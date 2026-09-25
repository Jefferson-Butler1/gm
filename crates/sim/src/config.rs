//! The run's configuration (issue #15): difficulty plus every combat tunable. Fixed when
//! a run is created and part of [`SimState`](crate::SimState), so it is serialized,
//! checksummed and replayed like everything else. Normal values are the tuning pass's
//! starting table; the debug Tuning settings override them for playtesting.
//!
//! Units are integers the sim can use without floats: speeds in points per second,
//! distances in points, times in ticks at [`TICK_HZ`], fractions in percent.

use crate::{Fx, TICK_HZ};
use serde::{Deserialize, Serialize};

/// Enemy-side strength. The player's values never depend on it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Difficulty {
    Easy,
    #[default]
    Normal,
    Hard,
}

impl Difficulty {
    /// Aimed enemy bullet speed, pt/s.
    #[must_use]
    pub const fn enemy_bullet_speed(self) -> u16 {
        match self {
            Self::Easy => 200,
            Self::Normal => 260,
            Self::Hard => 320,
        }
    }

    /// Ticks from one aimed shot to the next: 2.0 / 1.6 / 1.2 s.
    #[must_use]
    pub const fn shooter_interval(self) -> u16 {
        match self {
            Self::Easy => 120,
            Self::Normal => 96,
            Self::Hard => 72,
        }
    }

    /// Pellets per spread-shooter volley: Hard gets a denser pattern.
    #[must_use]
    pub const fn spread_pellets(self) -> u8 {
        match self {
            Self::Easy | Self::Normal => 5,
            Self::Hard => 7,
        }
    }
}

/// How the phase pistol recovers charges.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum VentStyle {
    /// A: clip-style. Firing the last charge (or pressing vent) vents; the vent refills
    /// every charge at once.
    #[default]
    Clip,
    /// B: charges regenerate one at a time once the gun hasn't fired for a moment;
    /// emptying it forces a longer vent.
    Regen,
}

/// Every combat tunable. Enemy values that difficulty sets are `Option`s: `None` takes
/// the difficulty's value, `Some` overrides it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Tuning {
    /// Full-deflection walk speed, pt/s.
    pub move_speed: u16,
    /// Roll length, pt, covered fast-then-slow.
    pub roll_distance: u16,
    pub roll_ticks: u16,
    /// Share of the roll, from its start, that has i-frames; the rest is a vulnerable
    /// landing.
    pub roll_iframe_percent: u8,
    /// Phase pistol shots between vents.
    pub charges: u8,
    /// Fire-rate cap: minimum ticks between shots.
    pub fire_interval: u16,
    /// A full vent (style A, or a manual vent in style B).
    pub vent_ticks: u16,
    /// Damage per pistol hit (enemy HP is in the same units).
    pub damage: u8,
    /// Player bullet speed, pt/s.
    pub bullet_speed: u16,
    /// Aimed enemy bullet speed, pt/s; spread pellets fly at 5/8 of it.
    pub enemy_bullet_speed: Option<u16>,
    /// Ticks from one aimed shot to the next; spread shooters take 35/16 of it.
    pub shooter_interval: Option<u16>,
    /// The last ticks of each shooter interval, spent standing still aiming.
    pub shooter_telegraph: u16,
    /// Invulnerability after taking a hit, and after respawning from a pit.
    pub hurt_ticks: u16,
    /// A fall into a pit: the player can't act or be hit, then respawns.
    pub fall_ticks: u16,
    /// Rusher chase speed, pt/s.
    pub rusher_speed: u16,
    pub vent_style: VentStyle,
    /// Vent style B: ticks without a shot before the first charge regenerates.
    pub regen_delay_ticks: u16,
    /// Vent style B: ticks per further regenerated charge.
    pub regen_charge_ticks: u16,
    /// The pattern experiment: some waves field spread shooters instead of shooters.
    pub spread_shooter: bool,
    /// An unaware enemy notices a player this close, pt, with a clear line of sight
    /// (walls block it, pits don't).
    pub sight_radius: u16,
    /// A player's shot alerts every enemy this close, pt, walls or not.
    pub hearing_radius: u16,
    /// An enemy hunting a player alerts unaware allies this close, pt, that it can see.
    pub alert_radius: u16,
    /// An enemy that reaches where it last saw a player and finds nobody gives up after
    /// this many ticks.
    pub forget_ticks: u16,
}

impl Tuning {
    /// Normal: the tuning pass's starting table (issue #15).
    pub const NORMAL: Self = Self {
        move_speed: 230,
        roll_distance: 160,
        roll_ticks: 36,
        roll_iframe_percent: 55,
        charges: 6,
        // 17 ticks = 3.53 shots/s, the closest tick count to the 3.5 cap.
        fire_interval: 17,
        vent_ticks: 66,
        damage: 5,
        bullet_speed: 800,
        enemy_bullet_speed: None,
        shooter_interval: None,
        shooter_telegraph: 36,
        hurt_ticks: 60,
        fall_ticks: 30,
        rusher_speed: 150,
        vent_style: VentStyle::Clip,
        // 0.6 s pause, then a charge per 0.4 s: a full refill takes 2.6 s. (0.35 s + 0.15 s
        // per charge was far too fast in Jeff's hands.)
        regen_delay_ticks: 36,
        regen_charge_ticks: 24,
        spread_shooter: true,
        // 7, 5 and 4 cells of 32 pt; 3 s.
        sight_radius: 224,
        hearing_radius: 160,
        alert_radius: 128,
        forget_ticks: 180,
    };
}

impl Default for Tuning {
    fn default() -> Self {
        Self::NORMAL
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RunConfig {
    pub difficulty: Difficulty,
    pub tuning: Tuning,
}

impl RunConfig {
    /// Clamped into ranges the sim handles: every tick's movement stays under a cell (as
    /// `Tiles::slide` needs), the telegraph fits inside the shot interval, and nothing
    /// that divides is zero.
    #[must_use]
    pub fn sanitized(self) -> Self {
        let t = self.tuning;
        let shooter_interval = t.shooter_interval.map(|i| i.clamp(12, 600));
        let mut sane = Self {
            difficulty: self.difficulty,
            tuning: Tuning {
                move_speed: t.move_speed.clamp(30, 900),
                roll_distance: t.roll_distance.min(240),
                roll_ticks: t.roll_ticks.clamp(12, 120),
                roll_iframe_percent: t.roll_iframe_percent.min(100),
                charges: t.charges.clamp(1, 60),
                fire_interval: t.fire_interval.clamp(1, 120),
                vent_ticks: t.vent_ticks.clamp(1, 600),
                damage: t.damage.max(1),
                bullet_speed: t.bullet_speed.clamp(60, 1800),
                enemy_bullet_speed: t.enemy_bullet_speed.map(|s| s.clamp(30, 900)),
                shooter_interval,
                shooter_telegraph: t.shooter_telegraph,
                hurt_ticks: t.hurt_ticks.min(600),
                fall_ticks: t.fall_ticks.clamp(1, 600),
                rusher_speed: t.rusher_speed.clamp(30, 900),
                vent_style: t.vent_style,
                regen_delay_ticks: t.regen_delay_ticks.clamp(1, 600),
                regen_charge_ticks: t.regen_charge_ticks.clamp(1, 600),
                spread_shooter: t.spread_shooter,
                sight_radius: t.sight_radius.min(2048),
                hearing_radius: t.hearing_radius.min(2048),
                alert_radius: t.alert_radius.min(2048),
                forget_ticks: t.forget_ticks.clamp(1, 3600),
            },
        };
        let interval = sane.shooter_interval();
        sane.tuning.shooter_telegraph = t.shooter_telegraph.min(interval.saturating_sub(1));
        sane
    }

    /// Aimed enemy bullet speed, pt/s: the override, else the difficulty's.
    #[must_use]
    pub fn enemy_bullet_speed(&self) -> u16 {
        self.tuning
            .enemy_bullet_speed
            .unwrap_or_else(|| self.difficulty.enemy_bullet_speed())
    }

    /// Ticks between aimed shots: the override, else the difficulty's.
    #[must_use]
    pub fn shooter_interval(&self) -> u16 {
        self.tuning
            .shooter_interval
            .unwrap_or_else(|| self.difficulty.shooter_interval())
    }

    /// Ticks between spread volleys: 35/16 of the aimed interval (3.5 s on Normal).
    #[must_use]
    pub fn spread_interval(&self) -> u16 {
        let scaled = u32::from(self.shooter_interval()).saturating_mul(35) / 16;
        u16::try_from(scaled).unwrap_or(u16::MAX)
    }

    /// Spread pellet speed, pt/s: 5/8 of the aimed bullets', as ETG's shotgun pellets.
    #[must_use]
    pub fn pellet_speed(&self) -> u16 {
        let scaled = u32::from(self.enemy_bullet_speed()).saturating_mul(5) / 8;
        u16::try_from(scaled).unwrap_or(u16::MAX)
    }
}

/// A speed in pt/s as the distance covered per tick.
#[must_use]
pub fn per_tick(points_per_second: u16) -> Fx {
    Fx::from_num(points_per_second)
        .checked_div_int(i64::from(TICK_HZ))
        .unwrap_or(Fx::ZERO)
}
