//! Player movement, dodge roll, pits, aim, gun and HP. Speeds and times come from the
//! run's [`Tuning`] (issue #15); the rest are fixed here.
//!
//! Pits (ETG rules): walking onto one, judged by the player's center cell, is a fall. The
//! whole roll is airborne, so only where it lands counts. A fall always costs 1 HP (even
//! mid post-hit invulnerability), leaves the player unable to act or be hit for
//! `fall_ticks`, then respawns them on the last safe spot they stood on, with the
//! post-hit invulnerability. A safe spot is grounded (not mid-roll) with no pit within
//! [`PIT_CLEARANCE`] (one cell) of the center along either axis, so a respawn never
//! lands on a pit's lip.

use crate::config::{Tuning, per_tick};
use crate::gun::PhasePistol;
use crate::input::{Buttons, MOVE_BUCKETS, PlayerInput};
use crate::room::{Body, CELL, Tiles};
use crate::{Fx, FxVec2, trig};
use serde::{Deserialize, Serialize};

/// Aim assist only bends toward targets within this half-angle of the aim: ±20°.
pub const ASSIST_CONE: i16 = 3641;
/// Hits a fresh player can take.
pub const MAX_HP: u8 = 5;
/// Hitbox radius of the placeholder player; also its half-extent against tiles.
pub const PLAYER_RADIUS: Fx = Fx::from_bits(14 << 32);
/// A respawn spot keeps every pit at least this far from the player's center: one cell,
/// over twice the player's radius.
pub const PIT_CLEARANCE: Fx = CELL;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Player {
    pub pos: FxVec2,
    /// Move direction while moving, overridden by the resolved aim while firing or the
    /// raw aim while aiming (`Buttons::AIM`); one full turn = 65536.
    pub facing: u16,
    /// Ticks of the current roll left, including this tick; nonzero = rolling: locked
    /// direction, no firing, no new roll.
    pub roll_ticks: u16,
    /// I-frames left of the current roll. They cover its start; the rest of the roll is
    /// a vulnerable landing.
    pub roll_iframes: u16,
    pub roll_dir: u16,
    /// 0 = dead. A dead player keeps its slot but no longer acts or takes hits.
    pub hp: u8,
    /// Post-hit invulnerability left.
    pub hurt_ticks: u16,
    /// Fall left; nonzero = falling into a pit: no acting, no hits. Ends in a respawn at
    /// [`Self::solid`].
    pub fall_ticks: u16,
    /// Where the player last stood grounded (not mid-roll) at least [`PIT_CLEARANCE`]
    /// from any pit; entering a room resets it to the arrival spot.
    pub solid: FxVec2,
    pub gun: PhasePistol,
}

impl Player {
    /// A fresh player at the origin: full HP, gun fully charged.
    #[must_use]
    pub const fn new(tuning: &Tuning) -> Self {
        Self {
            pos: FxVec2 {
                x: Fx::ZERO,
                y: Fx::ZERO,
            },
            facing: 0,
            roll_ticks: 0,
            roll_iframes: 0,
            roll_dir: 0,
            hp: MAX_HP,
            hurt_ticks: 0,
            fall_ticks: 0,
            solid: FxVec2 {
                x: Fx::ZERO,
                y: Fx::ZERO,
            },
            gun: PhasePistol::new(tuning),
        }
    }

    #[must_use]
    pub const fn rolling(&self) -> bool {
        self.roll_ticks > 0
    }

    #[must_use]
    pub const fn alive(&self) -> bool {
        self.hp > 0
    }

    #[must_use]
    pub const fn falling(&self) -> bool {
        self.fall_ticks > 0
    }

    /// What enemies chase, aim at and crowd: alive and not falling. A falling player is
    /// out of the fight until it respawns.
    #[must_use]
    pub const fn targetable(&self) -> bool {
        self.alive() && !self.falling()
    }

    /// Damage is ignored while this is set: roll i-frames, post-hit invulnerability, and
    /// falling.
    #[must_use]
    pub const fn invulnerable(&self) -> bool {
        self.roll_iframes > 0 || self.hurt_ticks > 0 || self.falling()
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

    /// No cooldown (ETG): a roll may start once the last one has landed. A dodge pressed
    /// mid-roll is dropped, not buffered.
    #[must_use]
    pub const fn can_roll(&self) -> bool {
        !self.rolling() && !self.falling()
    }

    /// One tick of this player's input, moving through `tiles`. `targets` are what aim
    /// assist and auto-aim may lock onto. Returns the angle of a shot fired this tick. A
    /// fall starting this tick leaves `fall_ticks` at the run's full `fall_ticks`.
    pub fn update(
        &mut self,
        input: PlayerInput,
        targets: &[FxVec2],
        tiles: Tiles,
        tuning: &Tuning,
    ) -> Option<u16> {
        if self.falling() {
            self.fall_ticks = self.fall_ticks.saturating_sub(1);
            if !self.falling() {
                self.pos = self.solid;
                self.hurt_ticks = tuning.hurt_ticks;
            }
            // The gun can't fire, but a vent carries on as through a roll.
            self.gun.tick(tuning, false, false);
            return None;
        }
        let move_angle = (input.move_mag > 0).then(|| bucket_angle(input.move_dir));

        self.roll_ticks = self.roll_ticks.saturating_sub(1);
        self.roll_iframes = self.roll_iframes.saturating_sub(1);
        if input.buttons.contains(Buttons::DODGE) && self.can_roll() {
            self.roll_dir = move_angle.unwrap_or(self.facing);
            self.roll_ticks = tuning.roll_ticks;
            self.roll_iframes = iframe_ticks(tuning);
        }

        let velocity = if self.rolling() {
            let elapsed = tuning.roll_ticks.saturating_sub(self.roll_ticks);
            scale(trig::unit(self.roll_dir), roll_step(tuning, elapsed))
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
        self.pos = tiles.slide(self.pos, PLAYER_RADIUS, velocity, Body::Player);

        // The whole roll is airborne: it lands (grounded again) at the end of its last tick.
        if self.roll_ticks <= 1 {
            if tiles.pit_at(self.pos) {
                self.hp = self.hp.saturating_sub(1);
                self.fall_ticks = tuning.fall_ticks;
                self.roll_ticks = 0;
                self.roll_iframes = 0;
                self.hurt_ticks = 0;
                return None;
            }
            if !tiles.pit_within(self.pos, PIT_CLEARANCE) {
                self.solid = self.pos;
            }
        }

        // No aiming or firing mid-roll (Gungeon-style); the roll owns the facing. Venting
        // carries on through a roll.
        let trigger = !self.rolling() && input.buttons.contains(Buttons::FIRE);
        if trigger {
            self.facing = self.resolve_aim(input, targets);
        } else if !self.rolling() && input.buttons.contains(Buttons::AIM) {
            self.facing = input.aim;
        }
        let vent = input.buttons.contains(Buttons::VENT);
        let shot = self.gun.tick(tuning, trigger, vent).then_some(self.facing);

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

/// The roll's i-frames: its first `roll_iframe_percent`, rounded to whole ticks.
fn iframe_ticks(tuning: &Tuning) -> u16 {
    let scaled = u32::from(tuning.roll_ticks)
        .saturating_mul(u32::from(tuning.roll_iframe_percent))
        .saturating_add(50)
        / 100;
    u16::try_from(scaled).unwrap_or(tuning.roll_ticks)
}

/// Distance the roll covers on its tick `elapsed` (0-based). ETG's roll is front-loaded,
/// so per-tick weights fall linearly, `5n - 3i` over `n` ticks: the last tick moves ~0.4x
/// the first (ETG's curve: ~10.7 -> ~4.3 tiles/s). Steps are differences of the
/// cumulative distance, so the whole roll covers exactly `roll_distance`.
fn roll_step(tuning: &Tuning, elapsed: u16) -> Fx {
    let n = i64::from(tuning.roll_ticks);
    let total = roll_weight(n, n);
    let covered = |k: i64| {
        Fx::from_num(tuning.roll_distance)
            .saturating_mul_int(roll_weight(n, k))
            .checked_div_int(total)
            .unwrap_or(Fx::ZERO)
    };
    let k = i64::from(elapsed);
    covered(k.saturating_add(1)).saturating_sub(covered(k))
}

/// Summed weight of an `n`-tick roll's first `k` ticks: 5nk - 3k(k - 1)/2.
const fn roll_weight(n: i64, k: i64) -> i64 {
    let falloff = k.saturating_mul(k.saturating_sub(1)).saturating_mul(3) / 2;
    n.saturating_mul(k)
        .saturating_mul(5)
        .saturating_sub(falloff)
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
    use crate::room::{CELL, Category, PrototypeRoom, cell_center, cell_of};

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

    /// 30 x 20 cells of floor inside walls, with a pit strip at x = 2..=3, rows 8..=11,
    /// and a 1-deep pit row at y = 4, x = 10..=20.
    const HALL: PrototypeRoom = PrototypeRoom {
        name: "test hall",
        category: Category::Normal,
        cells: &[
            "##############################",
            "#............................#",
            "#............................#",
            "#............................#",
            "#.........ooooooooooo........#",
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
        player_at(start())
    }

    /// A player placed at `pos`, as entering a room places it.
    fn player_at(pos: FxVec2) -> Player {
        Player {
            pos,
            solid: pos,
            ..Player::new(&Tuning::NORMAL)
        }
    }

    fn cell(p: FxVec2) -> (i32, i32) {
        (cell_of(p.x), cell_of(p.y))
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

    const FALL_TICKS: u16 = Tuning::NORMAL.fall_ticks;

    /// Runs `ticks` updates of `input`; returns how many fired.
    fn hold(p: &mut Player, input: PlayerInput, ticks: u16) -> usize {
        (0..ticks)
            .filter(|_| p.update(input, &[], tiles(), &Tuning::NORMAL).is_some())
            .count()
    }

    /// The pit strip's right edge (its cells end at x = 4 * 32).
    const STRIP_EDGE: Fx = CELL.saturating_mul_int(4);

    /// Walks `p` left until it falls into the pit strip; returns where it stood last.
    fn walk_into_the_strip(p: &mut Player) -> FxVec2 {
        let mut last = p.pos;
        for _ in 0..200 {
            if p.falling() {
                break;
            }
            last = p.pos;
            p.update(walk(16), &[], tiles(), &Tuning::NORMAL);
        }
        last
    }

    #[test]
    fn walking_into_a_pit_falls_costs_a_hit_and_respawns_a_cell_back_from_its_edge() {
        // Mid post-hit invulnerability: the fall costs the hit anyway.
        let mut p = Player {
            hurt_ticks: 1000,
            ..player()
        };
        let last = walk_into_the_strip(&mut p);
        assert!(p.falling());
        assert_eq!((p.hp, p.fall_ticks), (MAX_HP - 1, FALL_TICKS));
        // Judged by the center: it just crossed from floor into the pit.
        assert!(tiles().pit_at(p.pos) && !tiles().pit_at(last));
        // The respawn is the last spot a full cell clear of the pit, not its lip: the
        // first step (3.8 pt) within a cell of the edge stopped updating it.
        let clearance = p.solid.x.saturating_sub(STRIP_EDGE);
        assert!(
            clearance >= CELL && clearance < CELL.saturating_add(Fx::from_num(4)),
            "{clearance}"
        );
        let last = p.solid;

        // Falling: no moving, rolling, firing or getting hit, for the rest of the fall.
        let flail = with(walk(0), Buttons::DODGE | Buttons::FIRE);
        let pit = p.pos;
        assert_eq!(hold(&mut p, flail, FALL_TICKS - 1), 0);
        assert!(p.falling() && p.invulnerable() && !p.can_roll());
        assert!(!p.hurt(Tuning::NORMAL.hurt_ticks));
        assert_eq!((p.pos, p.hp), (pit, MAX_HP - 1));

        p.update(PlayerInput::default(), &[], tiles(), &Tuning::NORMAL);
        assert!(!p.falling());
        assert_eq!(p.pos, last, "back on the last safe spot");
        assert_eq!(
            p.hurt_ticks,
            Tuning::NORMAL.hurt_ticks,
            "with post-hit invulnerability"
        );
    }

    #[test]
    fn a_fall_from_the_pit_edge_respawns_at_least_a_cell_from_every_pit() {
        // Standing on the strip's lip (8 pt from it) since the room was entered, then
        // strolling around its corner: every spot so far is within a cell of the pit.
        let lip = FxVec2 {
            x: STRIP_EDGE.saturating_add(Fx::from_num(8)),
            y: cell_center(4, 12).y,
        };
        let mut p = player_at(lip);
        hold(&mut p, walk(24), 20); // up along the strip
        let tiles = tiles();
        assert!(tiles.pit_within(p.pos, PIT_CLEARANCE) && p.solid == lip);
        // Wander clear of it, then walk back in along its row.
        hold(&mut p, walk(0), 30); // right, out to 123 pt from the edge
        let far = p.pos;
        hold(&mut p, walk(24), 10); // up, still clear
        walk_into_the_strip(&mut p);
        assert!(p.falling());
        hold(&mut p, PlayerInput::default(), FALL_TICKS);
        assert!(!p.falling());
        assert!(!tiles.pit_within(p.pos, PIT_CLEARANCE), "{:?}", p.pos);
        let clearance = p.pos.x.saturating_sub(STRIP_EDGE);
        assert!(clearance >= CELL && p.pos.x < far.x, "{clearance}");
    }

    #[test]
    fn a_roll_flies_over_a_one_wide_pit() {
        // 160 pt up from row 6 lands in row 1, crossing the pit row at y = 4.
        let from = cell_center(15, 6);
        let mut p = player_at(from);
        p.update(
            with(walk(24), Buttons::DODGE),
            &[],
            tiles(),
            &Tuning::NORMAL,
        );
        let mut over_pit = false;
        for _ in 1..ROLL_TICKS + 10 {
            over_pit |= tiles().pit_at(p.pos);
            p.update(PlayerInput::default(), &[], tiles(), &Tuning::NORMAL);
            assert!(!p.falling());
        }
        assert!(over_pit);
        assert_eq!((p.hp, cell(p.pos)), (MAX_HP, (15, 1)));
    }

    #[test]
    fn a_roll_that_lands_on_a_pit_falls() {
        // 160 pt up from row 9's center lands mid pit row 4.
        let from = cell_center(15, 9);
        let mut p = player_at(from);
        p.update(
            with(walk(24), Buttons::DODGE),
            &[],
            tiles(),
            &Tuning::NORMAL,
        );
        hold(&mut p, PlayerInput::default(), ROLL_TICKS - 2);
        assert!(p.rolling() && !p.falling(), "airborne until it lands");
        p.update(PlayerInput::default(), &[], tiles(), &Tuning::NORMAL);
        assert_eq!(p.fall_ticks, FALL_TICKS, "falls as the roll lands");
        assert_eq!((p.hp, cell(p.pos)), (MAX_HP - 1, (15, 4)));
        hold(&mut p, PlayerInput::default(), FALL_TICKS);
        assert_eq!(p.pos, from, "respawns where the roll took off");
    }

    #[test]
    fn roll_has_iframes_for_its_first_55_percent_then_a_vulnerable_landing() {
        let mut p = player();
        let mut iframes = Vec::new();
        for tick in 0..ROLL_TICKS {
            let input = if tick == 0 {
                dodge()
            } else {
                PlayerInput::default()
            };
            p.update(input, &[], tiles(), &Tuning::NORMAL);
            assert!(p.rolling(), "tick {tick}");
            iframes.push(p.invulnerable());
        }
        // 55% of 36 ticks = 19.8, rounded to 20: ticks 0..20 dodge, 20..36 don't.
        let expected: Vec<bool> = (0..ROLL_TICKS).map(|t| t < 20).collect();
        assert_eq!(iframes, expected);
        assert_eq!(at(&p), (160, 0), "rolls along the facing when idle");
        p.update(PlayerInput::default(), &[], tiles(), &Tuning::NORMAL);
        assert!(!p.rolling());
        assert_eq!(at(&p), (160, 0));
    }

    #[test]
    fn roll_is_fast_then_slow() {
        let mut p = player();
        let mut steps = Vec::new();
        for tick in 0..ROLL_TICKS {
            let before = p.pos.x;
            let input = if tick == 0 {
                dodge()
            } else {
                PlayerInput::default()
            };
            p.update(input, &[], tiles(), &Tuning::NORMAL);
            steps.push(p.pos.x.saturating_sub(before));
        }
        assert!(steps.windows(2).all(|w| w[0] >= w[1]), "{steps:?}");
        // ~376 pt/s at the start (above the 230 walk), ~157 at the end (below it).
        let per_second = |v: Fx| v.saturating_mul_int(60).round().to_num::<i64>();
        assert_eq!(
            (per_second(steps[0]), per_second(steps[35])),
            (376, 157),
            "{steps:?}"
        );
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
    fn held_dodge_rolls_again_right_after_landing() {
        let mut p = player();
        let starts: Vec<u16> = (0..100)
            .filter(|_| {
                p.update(dodge(), &[], tiles(), &Tuning::NORMAL);
                p.roll_ticks == ROLL_TICKS
            })
            .collect();
        assert_eq!(starts, [0, 36, 72]);
    }

    #[test]
    fn dodge_mid_roll_is_dropped() {
        let mut p = player();
        p.update(dodge(), &[], tiles(), &Tuning::NORMAL);
        for tick in 1..ROLL_TICKS {
            assert!(!p.can_roll(), "tick {tick}");
            p.update(dodge(), &[], tiles(), &Tuning::NORMAL);
            assert_ne!(p.roll_ticks, ROLL_TICKS, "tick {tick}");
        }
        p.update(dodge(), &[], tiles(), &Tuning::NORMAL);
        assert_eq!(
            p.roll_ticks, ROLL_TICKS,
            "the tick after landing rolls again"
        );
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
    fn aim_without_fire_faces_the_raw_aim_over_the_move_without_shooting() {
        let target = [point(100, 100)]; // in the assist cone of 5192
        let aiming = with(walk(16), Buttons::AIM); // walking left
        let mut p = player();
        let shots = (0..40)
            .filter(|_| {
                p.update(
                    PlayerInput {
                        aim: 5192,
                        assist: 255,
                        ..aiming
                    },
                    &target,
                    tiles(),
                    &Tuning::NORMAL,
                )
                .is_some()
            })
            .count();
        assert_eq!((shots, p.facing), (0, 5192), "no shots, no assist bend");
        assert_eq!(p.gun.charges, PhasePistol::new(&Tuning::NORMAL).charges);
        // Mid-roll the roll still owns the facing.
        p.update(with(dodge(), Buttons::AIM), &[], tiles(), &Tuning::NORMAL);
        assert_eq!(p.facing, 5192);
        p.update(
            with(PlayerInput::default(), Buttons::AIM),
            &[],
            tiles(),
            &Tuning::NORMAL,
        );
        assert_eq!(p.facing, 5192);
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
        // Held fire stays silent through the landing, then fires once the roll is over.
        let shots: Vec<u16> = (1..=ROLL_TICKS)
            .filter(|_| {
                p.update(fire(DOWN, 0), &[], tiles(), &Tuning::NORMAL)
                    .is_some()
            })
            .collect();
        assert_eq!(shots, [ROLL_TICKS]);
    }

    #[test]
    fn held_fire_shoots_every_fire_interval() {
        let mut p = player();
        let shots: Vec<u32> = (0..40)
            .filter(|_| p.update(fire(DOWN, 0), &[], tiles(), &Tuning::NORMAL) == Some(DOWN))
            .collect();
        assert_eq!(shots, [0, 17, 34]);
    }

    #[test]
    fn vent_button_vents_and_the_vent_keeps_ticking_through_a_roll() {
        let mut p = player();
        p.update(fire(DOWN, 0), &[], tiles(), &Tuning::NORMAL);
        p.update(
            with(PlayerInput::default(), Buttons::VENT),
            &[],
            tiles(),
            &Tuning::NORMAL,
        );
        assert_eq!((p.gun.charges, p.gun.vent_ticks), (0, 66));
        // Roll right away: the whole roll counts toward the vent.
        for _ in 0..ROLL_TICKS {
            p.update(dodge(), &[], tiles(), &Tuning::NORMAL);
        }
        assert_eq!(p.gun.vent_ticks, 66 - ROLL_TICKS);
        let shots: Vec<u16> = (1..=40)
            .filter(|_| {
                p.update(fire(DOWN, 0), &[], tiles(), &Tuning::NORMAL)
                    .is_some()
            })
            .collect();
        assert_eq!(shots, [66 - ROLL_TICKS], "fires the tick the vent refills");
    }
}
