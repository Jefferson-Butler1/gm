//! Player movement, dodge roll and aim. Tuning is in ticks at [`TICK_HZ`](crate::TICK_HZ)
//! and world units (points); values come from the feel spike (issue #7).

use crate::input::{Buttons, MOVE_BUCKETS, PlayerInput};
use crate::{Fx, FxVec2, trig};
use serde::{Deserialize, Serialize};

/// Full-deflection run speed: 7 pt/tick = 420 pt/s.
const RUN_SPEED: Fx = Fx::from_bits(7 << 32);
/// Roll length: 12 ticks = 0.2 s, all of it i-frames.
pub const ROLL_TICKS: u8 = 12;
/// Roll speed: 170 pt over [`ROLL_TICKS`].
const ROLL_SPEED: Fx = Fx::from_bits((170 << 32) / 12);
/// Ticks from one roll's start until the next may start: 24 = 0.4 s. A dodge pressed
/// sooner is dropped (whether to buffer it is a combat-tuning question, issue #15).
pub const ROLL_COOLDOWN_TICKS: u8 = 24;
/// Aim assist only bends toward targets within this half-angle of the aim: ±20°.
pub const ASSIST_CONE: i16 = 3641;
/// Half-size of the placeholder square player.
pub const PLAYER_HALF: Fx = Fx::from_bits(14 << 32);
/// Placeholder room until rooms land: 800 x 360 pt centered on the origin.
pub const ROOM_HALF: FxVec2 = FxVec2 {
    x: Fx::from_bits(400 << 32),
    y: Fx::from_bits(180 << 32),
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Player {
    pub pos: FxVec2,
    /// Move direction while moving, overridden by the resolved aim while firing; one
    /// full turn = 65536.
    pub facing: u16,
    /// Ticks of the current roll left, including this tick; nonzero = rolling (i-frames).
    pub roll_ticks: u8,
    pub roll_dir: u16,
    /// Ticks until the next roll may start.
    pub roll_cooldown: u8,
}

impl Player {
    #[must_use]
    pub const fn rolling(&self) -> bool {
        self.roll_ticks > 0
    }

    /// Damage is ignored while this is set. Nothing deals damage yet (Combat step).
    #[must_use]
    pub const fn invulnerable(&self) -> bool {
        self.rolling()
    }

    #[must_use]
    pub const fn can_roll(&self) -> bool {
        self.roll_cooldown == 0
    }

    /// One tick of this player's input. `targets` are what aim assist and auto-aim may
    /// lock onto.
    pub fn update(&mut self, input: PlayerInput, targets: &[FxVec2]) {
        let move_angle = (input.move_mag > 0).then(|| bucket_angle(input.move_dir));

        self.roll_ticks = self.roll_ticks.saturating_sub(1);
        if input.buttons.contains(Buttons::DODGE) && self.can_roll() {
            self.roll_dir = move_angle.unwrap_or(self.facing);
            self.roll_ticks = ROLL_TICKS;
            self.roll_cooldown = ROLL_COOLDOWN_TICKS;
        }

        let velocity = if self.rolling() {
            scale(trig::unit(self.roll_dir), ROLL_SPEED)
        } else if let Some(angle) = move_angle {
            self.facing = angle;
            let speed = RUN_SPEED
                .saturating_mul(Fx::from_num(input.move_mag))
                .checked_div(Fx::from_num(u8::MAX))
                .unwrap_or(RUN_SPEED);
            scale(trig::unit(angle), speed)
        } else {
            FxVec2::default()
        };
        let bound = FxVec2 {
            x: ROOM_HALF.x.saturating_sub(PLAYER_HALF),
            y: ROOM_HALF.y.saturating_sub(PLAYER_HALF),
        };
        self.pos = FxVec2 {
            x: clamp(self.pos.x.saturating_add(velocity.x), bound.x),
            y: clamp(self.pos.y.saturating_add(velocity.y), bound.y),
        };

        // No aiming mid-roll (Gungeon-style); the roll owns the facing.
        if !self.rolling() && input.buttons.contains(Buttons::FIRE) {
            self.facing = self.resolve_aim(input, targets);
        }

        self.roll_cooldown = self.roll_cooldown.saturating_sub(1);
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
        targets.min_by_key(|t| {
            let d = |a: Fx, b: Fx| i128::from(a.to_bits()).saturating_sub(i128::from(b.to_bits()));
            let (dx, dy) = (d(t.x, self.pos.x), d(t.y, self.pos.y));
            dx.saturating_mul(dx).saturating_add(dy.saturating_mul(dy))
        })
    }
}

/// Angle of the center of a move bucket.
fn bucket_angle(bucket: u8) -> u16 {
    let per_bucket = (1_u32 << 16)
        .checked_div(u32::from(MOVE_BUCKETS))
        .unwrap_or(0);
    let turns = u32::from(bucket).saturating_mul(per_bucket) & 0xFFFF;
    u16::try_from(turns).unwrap_or(0)
}

const fn scale(v: FxVec2, k: Fx) -> FxVec2 {
    FxVec2 {
        x: v.x.saturating_mul(k),
        y: v.y.saturating_mul(k),
    }
}

fn clamp(v: Fx, bound: Fx) -> Fx {
    v.max(bound.saturating_neg()).min(bound)
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// Position rounded to whole points.
    fn at(p: &Player) -> (i64, i64) {
        (p.pos.x.round().to_num(), p.pos.y.round().to_num())
    }

    fn point(x: i32, y: i32) -> FxVec2 {
        FxVec2 {
            x: Fx::from_num(x),
            y: Fx::from_num(y),
        }
    }

    #[test]
    fn full_stick_walks_420_pt_per_second_and_faces_the_move() {
        let mut p = Player::default();
        for _ in 0..30 {
            p.update(walk(16), &[]); // bucket 16 of 32 = straight left
        }
        assert_eq!(at(&p), (-210, 0));
        assert_eq!(p.facing, LEFT);
    }

    #[test]
    fn walking_stops_at_the_room_edge() {
        let mut p = Player::default();
        for _ in 0..120 {
            p.update(walk(0), &[]);
        }
        assert_eq!(p.pos.x, ROOM_HALF.x.saturating_sub(PLAYER_HALF));
    }

    #[test]
    fn roll_is_12_ticks_of_iframes_covering_170_pt() {
        let mut p = Player::default();
        p.update(dodge(), &[]);
        for tick in 1..ROLL_TICKS {
            assert!(p.invulnerable(), "tick {tick}");
            p.update(PlayerInput::default(), &[]);
        }
        assert!(p.invulnerable());
        assert_eq!(at(&p), (170, 0), "rolls along the facing when idle");
        p.update(PlayerInput::default(), &[]);
        assert!(!p.rolling() && !p.invulnerable());
        assert_eq!(at(&p), (170, 0));
    }

    #[test]
    fn roll_goes_where_the_stick_points_and_ignores_it_mid_roll() {
        let mut p = Player::default();
        p.update(with(walk(16), Buttons::DODGE), &[]);
        for _ in 1..ROLL_TICKS {
            p.update(walk(8), &[]);
        }
        assert_eq!(at(&p), (-170, 0));
    }

    #[test]
    fn held_dodge_rolls_again_exactly_when_the_cooldown_ends() {
        let mut p = Player::default();
        let starts: Vec<u32> = (0..60)
            .filter(|_| {
                p.update(dodge(), &[]);
                p.roll_ticks == ROLL_TICKS
            })
            .collect();
        assert_eq!(starts, [0, 24, 48]);
    }

    #[test]
    fn dodge_during_cooldown_is_dropped() {
        let mut p = Player::default();
        p.update(dodge(), &[]);
        for tick in 1..ROLL_COOLDOWN_TICKS {
            assert!(!p.can_roll(), "tick {tick}");
            p.update(dodge(), &[]);
            assert_ne!(p.roll_ticks, ROLL_TICKS, "tick {tick}");
        }
        assert!(p.can_roll(), "ready once the cooldown has fully elapsed");
    }

    #[test]
    fn fire_faces_the_raw_aim_without_targets_even_with_assist() {
        let mut p = Player::default();
        p.update(fire(1234, 255), &[]);
        assert_eq!(p.facing, 1234);
    }

    #[test]
    fn assist_bends_toward_targets_in_the_cone_by_strength() {
        let target = [point(100, 100)]; // at DOWN_RIGHT
        // 5192 is ~16.5 degrees off the target (inside the cone); 51/255 bends 1/5 of it.
        for (assist, expected) in [(0, 5192), (51, 5792), (255, DOWN_RIGHT)] {
            let mut p = Player::default();
            p.update(fire(5192, assist), &target);
            assert_eq!(p.facing, expected, "assist {assist}");
        }
        let mut p = Player::default();
        p.update(fire(4192, 255), &target); // ~22 degrees off: outside the cone
        assert_eq!(p.facing, 4192);
    }

    #[test]
    fn auto_aim_locks_the_nearest_target_else_keeps_facing() {
        let auto = with(fire(RIGHT, 0), Buttons::AUTO_AIM);
        let mut p = Player {
            facing: DOWN,
            ..Player::default()
        };
        p.update(auto, &[]);
        assert_eq!(p.facing, DOWN);
        p.update(auto, &[point(-300, 0), point(0, -50)]);
        assert_eq!(p.facing, UP);
    }

    #[test]
    fn no_aiming_mid_roll() {
        let mut p = Player::default();
        p.update(with(fire(DOWN, 0), Buttons::DODGE), &[]);
        assert_eq!((p.roll_dir, p.facing), (RIGHT, RIGHT));
    }
}
