//! Player movement, dodge roll, aim, gun and HP. Speeds and times come from the run's
//! [`Tuning`] (issue #15); the rest are fixed here.

use crate::config::{Tuning, per_tick};
use crate::input::{Buttons, MOVE_BUCKETS, PlayerInput};
use crate::room::{Body, Tiles};
use crate::{Fx, FxVec2, trig};
use serde::{Deserialize, Serialize};

/// Ticks from one roll's start until the next may start: 24 = 0.4 s. A dodge pressed
/// sooner is dropped (whether to buffer it is a combat-tuning question, issue #15).
pub const ROLL_COOLDOWN_TICKS: u16 = 24;
/// Aim assist only bends toward targets within this half-angle of the aim: ±20°.
pub const ASSIST_CONE: i16 = 3641;
/// Hits a fresh player can take.
pub const MAX_HP: u8 = 5;
/// Hitbox radius of the placeholder player; also its half-extent against tiles.
pub const PLAYER_RADIUS: Fx = Fx::from_bits(14 << 32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Player {
    pub pos: FxVec2,
    /// Move direction while moving, overridden by the resolved aim while firing; one
    /// full turn = 65536.
    pub facing: u16,
    /// Ticks of the current roll left, including this tick; nonzero = rolling (i-frames).
    pub roll_ticks: u16,
    pub roll_dir: u16,
    /// Ticks until the next roll may start.
    pub roll_cooldown: u16,
    /// 0 = dead. A dead player keeps its slot but no longer acts or takes hits.
    pub hp: u8,
    /// Post-hit invulnerability left.
    pub hurt_ticks: u16,
    /// Ticks until the gun may fire again.
    pub fire_cooldown: u16,
}

impl Default for Player {
    fn default() -> Self {
        Self {
            pos: FxVec2::default(),
            facing: 0,
            roll_ticks: 0,
            roll_dir: 0,
            roll_cooldown: 0,
            hp: MAX_HP,
            hurt_ticks: 0,
            fire_cooldown: 0,
        }
    }
}

impl Player {
    #[must_use]
    pub const fn rolling(&self) -> bool {
        self.roll_ticks > 0
    }

    #[must_use]
    pub const fn alive(&self) -> bool {
        self.hp > 0
    }

    /// Damage is ignored while this is set: dodge i-frames and post-hit invulnerability.
    #[must_use]
    pub const fn invulnerable(&self) -> bool {
        self.rolling() || self.hurt_ticks > 0
    }

    /// Takes one hit unless invulnerable or already dead, then stays invulnerable for
    /// `hurt_ticks`. Returns whether it landed.
    pub const fn hurt(&mut self, hurt_ticks: u16) -> bool {
        if !self.alive() || self.invulnerable() {
            return false;
        }
        self.hp = self.hp.saturating_sub(1);
        self.hurt_ticks = hurt_ticks;
        true
    }

    #[must_use]
    pub const fn can_roll(&self) -> bool {
        self.roll_cooldown == 0
    }

    /// One tick of this player's input, moving through `tiles`. `targets` are what aim
    /// assist and auto-aim may lock onto. Returns the angle of a shot fired this tick.
    pub fn update(
        &mut self,
        input: PlayerInput,
        targets: &[FxVec2],
        tiles: Tiles,
        tuning: &Tuning,
    ) -> Option<u16> {
        let move_angle = (input.move_mag > 0).then(|| bucket_angle(input.move_dir));

        self.roll_ticks = self.roll_ticks.saturating_sub(1);
        if input.buttons.contains(Buttons::DODGE) && self.can_roll() {
            self.roll_dir = move_angle.unwrap_or(self.facing);
            self.roll_ticks = tuning.roll_ticks;
            self.roll_cooldown = ROLL_COOLDOWN_TICKS;
        }

        let velocity = if self.rolling() {
            let speed = Fx::from_num(tuning.roll_distance)
                .checked_div_int(i64::from(tuning.roll_ticks))
                .unwrap_or(Fx::ZERO);
            scale(trig::unit(self.roll_dir), speed)
        } else if let Some(angle) = move_angle {
            self.facing = angle;
            let run = per_tick(tuning.move_speed);
            let speed = run
                .saturating_mul(Fx::from_num(input.move_mag))
                .checked_div(Fx::from_num(u8::MAX))
                .unwrap_or(run);
            scale(trig::unit(angle), speed)
        } else {
            FxVec2::default()
        };
        self.pos = tiles.slide(self.pos, PLAYER_RADIUS, velocity, Body::Walker);

        // No aiming or firing mid-roll (Gungeon-style); the roll owns the facing.
        let mut shot = None;
        if !self.rolling() && input.buttons.contains(Buttons::FIRE) {
            self.facing = self.resolve_aim(input, targets);
            if self.fire_cooldown == 0 {
                self.fire_cooldown = tuning.fire_interval;
                shot = Some(self.facing);
            }
        }

        self.roll_cooldown = self.roll_cooldown.saturating_sub(1);
        self.fire_cooldown = self.fire_cooldown.saturating_sub(1);
        self.hurt_ticks = self.hurt_ticks.saturating_sub(1);
        shot
    }

    /// Where a shot fired this tick would go. Assist and auto-aim fall back to the raw aim
    /// (auto-aim: the current facing) when there is nothing to lock onto.
    fn resolve_aim(&self, input: PlayerInput, targets: &[FxVec2]) -> u16 {
        let angle_to = |t: FxVec2| {
            trig::angle_of(FxVec2 {
                x: t.x.saturating_sub(self.pos.x),
                y: t.y.saturating_sub(self.pos.y),
            })
        };
        if input.buttons.contains(Buttons::AUTO_AIM) {
            let target = self.nearest(targets.iter().copied());
            return target.and_then(angle_to).unwrap_or(self.facing);
        }
        if input.assist == 0 {
            return input.aim;
        }
        let in_cone = targets.iter().copied().filter(|&t| {
            angle_to(t).is_some_and(|a| {
                trig::angle_diff(input.aim, a).unsigned_abs() <= ASSIST_CONE.unsigned_abs()
            })
        });
        let Some(target) = self.nearest(in_cone).and_then(angle_to) else {
            return input.aim;
        };
        let bend = i32::from(trig::angle_diff(input.aim, target))
            .saturating_mul(i32::from(input.assist))
            .checked_div(i32::from(u8::MAX))
            .and_then(|b| i16::try_from(b).ok())
            .unwrap_or(0);
        input.aim.wrapping_add_signed(bend)
    }

    /// Closest target; ties go to the earliest, so order must be deterministic.
    fn nearest(&self, targets: impl Iterator<Item = FxVec2>) -> Option<FxVec2> {
        targets.min_by_key(|&t| dist_sq(t, self.pos))
    }
}

/// Squared distance in raw `Fx` bits; exact, for comparisons only.
pub fn dist_sq(a: FxVec2, b: FxVec2) -> i128 {
    let d = |a: Fx, b: Fx| i128::from(a.to_bits()).saturating_sub(i128::from(b.to_bits()));
    let (dx, dy) = (d(a.x, b.x), d(a.y, b.y));
    dx.saturating_mul(dx).saturating_add(dy.saturating_mul(dy))
}

/// Angle of the center of a move bucket.
fn bucket_angle(bucket: u8) -> u16 {
    let per_bucket = (1_u32 << 16)
        .checked_div(u32::from(MOVE_BUCKETS))
        .unwrap_or(0);
    let turns = u32::from(bucket).saturating_mul(per_bucket) & 0xFFFF;
    u16::try_from(turns).unwrap_or(0)
}

pub const fn scale(v: FxVec2, k: Fx) -> FxVec2 {
    FxVec2 {
        x: v.x.saturating_mul(k),
        y: v.y.saturating_mul(k),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::room::{CELL, Category, PrototypeRoom, cell_center};

    const ROLL_TICKS: u16 = Tuning::NORMAL.roll_ticks;

    const RIGHT: u16 = 0;
    const DOWN: u16 = 16384;
    const LEFT: u16 = 32768;
    const UP: u16 = 49152;
    const DOWN_RIGHT: u16 = 8192;

    fn walk(dir: u8) -> PlayerInput {
        PlayerInput {
            move_dir: dir,
            move_mag: u8::MAX,
            ..PlayerInput::default()
        }
    }

    fn with(mut input: PlayerInput, buttons: Buttons) -> PlayerInput {
        input.buttons |= buttons;
        input
    }

    fn dodge() -> PlayerInput {
        with(PlayerInput::default(), Buttons::DODGE)
    }

    fn fire(aim: u16, assist: u8) -> PlayerInput {
        PlayerInput {
            aim,
            assist,
            buttons: Buttons::FIRE,
            ..PlayerInput::default()
        }
    }

    /// 30 x 20 cells of floor inside walls, with a pit strip at x = 2..=3, rows 8..=11.
    const HALL: PrototypeRoom = PrototypeRoom {
        name: "test hall",
        category: Category::Normal,
        cells: &[
            "##############################",
            "#............................#",
            "#............................#",
            "#............................#",
            "#............................#",
            "#............................#",
            "#............................#",
            "#............................#",
            "#.oo.........................#",
            "#.oo.........................#",
            "#.oo.........................#",
            "#.oo.........................#",
            "#............................#",
            "#............................#",
            "#............................#",
            "#............................#",
            "#............................#",
            "#............................#",
            "#............................#",
            "##############################",
        ],
        exits: &[],
        base: &[],
        reinforcements: &[],
        events: &[],
        extraction: None,
    }
    .valid();

    fn tiles() -> Tiles {
        Tiles {
            room: &HALL,
            sealed: false,
        }
    }

    /// Mid-hall, at the pit strip's height.
    fn start() -> FxVec2 {
        cell_center(15, 10)
    }

    fn player() -> Player {
        Player {
            pos: start(),
            ..Player::default()
        }
    }

    /// Position relative to [`start`], rounded to whole points.
    fn at(p: &Player) -> (i64, i64) {
        let s = start();
        let rel = |v: Fx, s: Fx| v.saturating_sub(s).round().to_num();
        (rel(p.pos.x, s.x), rel(p.pos.y, s.y))
    }

    /// A point relative to [`start`].
    fn point(x: i32, y: i32) -> FxVec2 {
        let s = start();
        FxVec2 {
            x: s.x.saturating_add(Fx::from_num(x)),
            y: s.y.saturating_add(Fx::from_num(y)),
        }
    }

    #[test]
    fn full_stick_walks_230_pt_per_second_and_faces_the_move() {
        let mut p = player();
        for _ in 0..30 {
            p.update(walk(16), &[], tiles(), &Tuning::NORMAL); // bucket 16 of 32 = straight left
        }
        assert_eq!(at(&p), (-115, 0));
        assert_eq!(p.facing, LEFT);
    }

    #[test]
    fn walking_stops_flush_against_a_wall_and_slides_along_it() {
        let mut p = player();
        for _ in 0..120 {
            p.update(walk(0), &[], tiles(), &Tuning::NORMAL);
        }
        let wall = CELL.saturating_mul_int(29);
        assert_eq!(p.pos.x, wall.saturating_sub(PLAYER_RADIUS));
        // Pushing diagonally into the wall still slides down it.
        for _ in 0..20 {
            p.update(walk(4), &[], tiles(), &Tuning::NORMAL); // 45 degrees, down-right
        }
        assert_eq!(p.pos.x, wall.saturating_sub(PLAYER_RADIUS));
        assert!(at(&p).1 > 40, "{:?}", at(&p));
    }

    #[test]
    fn pits_stop_walkers() {
        let mut p = player();
        for _ in 0..120 {
            p.update(walk(16), &[], tiles(), &Tuning::NORMAL);
        }
        assert_eq!(
            p.pos.x,
            CELL.saturating_mul_int(4).saturating_add(PLAYER_RADIUS),
            "flush with the pit's edge"
        );
    }

    #[test]
    fn roll_is_36_ticks_of_iframes_covering_160_pt() {
        let mut p = player();
        p.update(dodge(), &[], tiles(), &Tuning::NORMAL);
        for tick in 1..ROLL_TICKS {
            assert!(p.invulnerable(), "tick {tick}");
            p.update(PlayerInput::default(), &[], tiles(), &Tuning::NORMAL);
        }
        assert!(p.invulnerable());
        assert_eq!(at(&p), (160, 0), "rolls along the facing when idle");
        p.update(PlayerInput::default(), &[], tiles(), &Tuning::NORMAL);
        assert!(!p.rolling() && !p.invulnerable());
        assert_eq!(at(&p), (160, 0));
    }

    #[test]
    fn roll_goes_where_the_stick_points_and_ignores_it_mid_roll() {
        let mut p = player();
        p.update(
            with(walk(16), Buttons::DODGE),
            &[],
            tiles(),
            &Tuning::NORMAL,
        );
        for _ in 1..ROLL_TICKS {
            p.update(walk(8), &[], tiles(), &Tuning::NORMAL);
        }
        assert_eq!(at(&p), (-160, 0));
    }

    #[test]
    fn held_dodge_rolls_again_exactly_when_the_cooldown_ends() {
        let mut p = player();
        let starts: Vec<u32> = (0..60)
            .filter(|_| {
                p.update(dodge(), &[], tiles(), &Tuning::NORMAL);
                p.roll_ticks == ROLL_TICKS
            })
            .collect();
        assert_eq!(starts, [0, 24, 48]);
    }

    #[test]
    fn dodge_during_cooldown_is_dropped() {
        let mut p = player();
        p.update(dodge(), &[], tiles(), &Tuning::NORMAL);
        for tick in 1..ROLL_COOLDOWN_TICKS {
            assert!(!p.can_roll(), "tick {tick}");
            p.update(dodge(), &[], tiles(), &Tuning::NORMAL);
            assert_ne!(p.roll_ticks, ROLL_TICKS, "tick {tick}");
        }
        assert!(p.can_roll(), "ready once the cooldown has fully elapsed");
    }

    #[test]
    fn fire_faces_the_raw_aim_without_targets_even_with_assist() {
        let mut p = player();
        p.update(fire(1234, 255), &[], tiles(), &Tuning::NORMAL);
        assert_eq!(p.facing, 1234);
    }

    #[test]
    fn assist_bends_toward_targets_in_the_cone_by_strength() {
        let target = [point(100, 100)]; // at DOWN_RIGHT
        // 5192 is ~16.5 degrees off the target (inside the cone); 51/255 bends 1/5 of it.
        for (assist, expected) in [(0, 5192), (51, 5792), (255, DOWN_RIGHT)] {
            let mut p = player();
            p.update(fire(5192, assist), &target, tiles(), &Tuning::NORMAL);
            assert_eq!(p.facing, expected, "assist {assist}");
        }
        let mut p = player();
        p.update(fire(4192, 255), &target, tiles(), &Tuning::NORMAL); // ~22 degrees off: outside the cone
        assert_eq!(p.facing, 4192);
    }

    #[test]
    fn auto_aim_locks_the_nearest_target_else_keeps_facing() {
        let auto = with(fire(RIGHT, 0), Buttons::AUTO_AIM);
        let mut p = Player {
            facing: DOWN,
            ..player()
        };
        p.update(auto, &[], tiles(), &Tuning::NORMAL);
        assert_eq!(p.facing, DOWN);
        p.update(
            auto,
            &[point(-300, 0), point(0, -50)],
            tiles(),
            &Tuning::NORMAL,
        );
        assert_eq!(p.facing, UP);
    }

    #[test]
    fn no_aiming_or_firing_mid_roll() {
        let mut p = player();
        let shot = p.update(
            with(fire(DOWN, 0), Buttons::DODGE),
            &[],
            tiles(),
            &Tuning::NORMAL,
        );
        assert_eq!((p.roll_dir, p.facing, shot), (RIGHT, RIGHT, None));
    }

    #[test]
    fn held_fire_shoots_every_fire_interval() {
        let mut p = player();
        let shots: Vec<u32> = (0..40)
            .filter(|_| p.update(fire(DOWN, 0), &[], tiles(), &Tuning::NORMAL) == Some(DOWN))
            .collect();
        assert_eq!(shots, [0, 17, 34]);
    }
}
