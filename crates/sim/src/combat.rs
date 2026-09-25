//! Gun, bullets, and the enemies (melee rusher, ranged shooter in two patterns), colliding
//! with the
//! room's tiles. Systems run in a fixed order over arenas iterated in slot order, so every
//! tie resolves the same way on every machine. Speeds and timings come from the run's
//! [`RunConfig`](crate::RunConfig) (issue #15).

use crate::arena::{Arena, Id};
use crate::config::{RunConfig, per_tick};
use crate::player::{PLAYER_RADIUS, Player, dist_sq, scale};
use crate::room::{Body, Tiles};
use crate::{Event, Fx, FxVec2, Run, SimState, TickEvents, TickInputs, encounter, trig};
use serde::{Deserialize, Serialize};

pub type EnemyId = Id<Enemy>;

/// Bullets vanish after 60 ticks = 1 s if they hit nothing.
const BULLET_TICKS: u8 = 60;
/// Hitbox radius.
pub const BULLET_RADIUS: Fx = Fx::from_bits(4 << 32);
/// Bullets leave the gun this far ahead of the player's center.
const MUZZLE: Fx = Fx::from_bits(22 << 32);

/// Enemy bullets vanish after 240 ticks = 4 s (1040 pt at Normal speed) if they hit
/// nothing.
const ENEMY_BULLET_TICKS: u8 = 240;
/// Hitbox radius; bigger than the player's bullets so they read as a threat.
pub const ENEMY_BULLET_RADIUS: Fx = Fx::from_bits(5 << 32);

/// Hitbox radius of every enemy; also its half-extent against tiles.
pub const ENEMY_RADIUS: Fx = Fx::from_bits(13 << 32);
/// Enemy HP is in pistol-damage units: Normal's 5 per hit kills a rusher in 2 hits.
pub const RUSHER_HP: u8 = 10;
/// Ticks after a rusher lands a contact hit before it can land another: 0.5 s.
const CONTACT_COOLDOWN: u8 = 30;

/// 2 hits at Normal damage.
pub const SHOOTER_HP: u8 = 10;
/// 3 hits: the spread shooter is the heavier threat (ETG's Shotgun Kin has 2x the HP).
pub const SPREAD_SHOOTER_HP: u8 = 15;
/// Angle between neighboring spread pellets: 12 degrees.
const PELLET_SPACING: i16 = 2185;
/// Shooter walk speed: 2 pt/tick = 120 pt/s.
const SHOOTER_SPEED: Fx = Fx::from_bits(2 << 32);
/// Shooters back off inside this range of their target...
const SHOOTER_NEAR: Fx = Fx::from_bits(120 << 32);
/// ...close in beyond this one, and strafe in between.
const SHOOTER_FAR: Fx = Fx::from_bits(200 << 32);
/// Spawned shooters wait up to this many extra ticks before their first shot, so a wave
/// doesn't fire in unison.
pub const SHOOTER_STAGGER: u32 = 48;

/// Active enemies' centers are kept this far apart: they touch but never stack.
const ENEMY_SPACING: Fx = Fx::from_bits(ENEMY_RADIUS.to_bits().saturating_mul(2));
/// ...and this far from a living player's: pressed 3 pt into contact reach, so contact
/// still lands but a crowd rings the player instead of piling onto it.
const PLAYER_SPACING: Fx = Fx::from_bits(
    PLAYER_RADIUS
        .to_bits()
        .saturating_add(ENEMY_RADIUS.to_bits())
        .saturating_sub(3 << 32),
);
/// Separation passes per tick; each pass shrinks what a crowd's pressure leaves over.
const SEPARATION_PASSES: u8 = 8;
const HALF: Fx = Fx::from_bits(1 << 31);
/// Turns in `u16` angle units.
const EIGHTH_TURN: i16 = 8192;
const QUARTER_TURN: u16 = 16384;
const HALF_TURN: u16 = 32768;
/// Line-of-fire checks sample the line this often: under a bullet radius, so no wall
/// corner a bullet would clip slips between samples.
const LINE_STEP: Fx = Fx::from_bits(4 << 32);

/// Spawn telegraph: a new enemy spends 30 ticks = 0.5 s as a warning marker. Meanwhile
/// it is inert: it doesn't move, hurt, push or get pushed, and it can't be targeted or
/// hit (bullets pass through).
pub const SPAWN_TELEGRAPH_TICKS: u8 = 30;

/// Ticks from all players dying until restart is accepted: 0.75 s, so a panicked tap
/// doesn't skip the death.
pub const DEATH_TICKS: u32 = 45;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Bullet {
    pub pos: FxVec2,
    pub vel: FxVec2,
    pub ticks_left: u8,
}

/// One enemy of any type; [`Behavior`] holds what differs by type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Enemy {
    pub pos: FxVec2,
    pub hp: u8,
    /// Spawn telegraph left; nonzero = inert (see [`SPAWN_TELEGRAPH_TICKS`]).
    pub spawn_ticks: u8,
    /// Which way it is detouring around something in its path: +1 turns clockwise
    /// (toward +y from +x), -1 counter-clockwise, 0 = heading straight.
    pub steer: i8,
    pub behavior: Behavior,
}

/// What a shooter fires.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Pattern {
    /// One bullet at the target, every `RunConfig::shooter_interval`.
    Aimed,
    /// The pattern experiment: a fan of slow pellets centered on the target (5, or 7 on
    /// Hard), every `RunConfig::spread_interval`.
    Spread,
}

impl Pattern {
    fn interval(self, config: &RunConfig) -> u16 {
        match self {
            Self::Aimed => config.shooter_interval(),
            Self::Spread => config.spread_interval(),
        }
    }
}

/// Each enemy type's own state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Behavior {
    /// Chases the nearest living player and hurts on contact.
    Rusher { contact_cooldown: u8 },
    /// Keeps its distance, strafing, and shoots at the nearest living player.
    Shooter {
        pattern: Pattern,
        /// Ticks until the next shot. The run's interval for the pattern restarts it;
        /// its last `shooter_telegraph` ticks are the shot telegraph: standing still,
        /// aiming. The telegraph only starts with a clear line of fire, and once started
        /// always ends in a shot (at wherever the target is by then).
        shot_timer: u16,
        /// Strafe direction, +1 or -1 turns (as [`Enemy::steer`]); flips after each shot
        /// and when blocked.
        strafe: i8,
    },
}

impl Enemy {
    /// A rusher arriving at `pos`. Every spawn starts in the telegraph, so any spawner
    /// just inserts this.
    #[must_use]
    pub const fn rusher(pos: FxVec2) -> Self {
        Self {
            pos,
            hp: RUSHER_HP,
            spawn_ticks: SPAWN_TELEGRAPH_TICKS,
            steer: 0,
            behavior: Behavior::Rusher {
                contact_cooldown: 0,
            },
        }
    }

    /// A shooter arriving at `pos`, which fires its first shot a full interval (for its
    /// pattern, under `config`) plus `delay` ticks after its spawn telegraph.
    #[must_use]
    pub fn shooter(pos: FxVec2, pattern: Pattern, config: &RunConfig, delay: u16) -> Self {
        Self {
            pos,
            hp: match pattern {
                Pattern::Aimed => SHOOTER_HP,
                Pattern::Spread => SPREAD_SHOOTER_HP,
            },
            spawn_ticks: SPAWN_TELEGRAPH_TICKS,
            steer: 0,
            behavior: Behavior::Shooter {
                pattern,
                shot_timer: pattern.interval(config).saturating_add(delay),
                strafe: 1,
            },
        }
    }

    /// Past the spawn telegraph: moves, hurts, and can be targeted and hit.
    #[must_use]
    pub const fn active(&self) -> bool {
        self.spawn_ticks == 0
    }

    /// Shot telegraph left, `telegraph..=1` (it fires at 0); `None` when not aiming.
    /// `telegraph` is the run's `shooter_telegraph`.
    #[must_use]
    pub const fn aiming(&self, telegraph: u16) -> Option<u16> {
        match self.behavior {
            Behavior::Shooter { shot_timer, .. } if shot_timer <= telegraph => Some(shot_timer),
            Behavior::Shooter { .. } | Behavior::Rusher { .. } => None,
        }
    }
}

/// One live tick, in order: players (move, fall, fire), their bullets, enemies (move, fire,
/// separate, contact, telegraph countdown), enemy bullets, the death check, then the room
/// (waves, exits, extraction).
pub fn tick(state: &mut SimState, inputs: &TickInputs, events: &mut TickEvents) {
    let Some(tiles) = state.tiles() else {
        return;
    };
    players(state, inputs, tiles, events);
    bullets(state, tiles, events);
    enemies(state, tiles, events);
    enemy_bullets(state, tiles, events);
    if !state.players.iter().flatten().any(Player::alive) {
        state.run = Run::Dead {
            room: state.run.room(),
            ticks_until_restart: DEATH_TICKS,
        };
        return;
    }
    encounter::tick(state, events);
}

fn players(state: &mut SimState, inputs: &TickInputs, tiles: Tiles, events: &mut TickEvents) {
    let tuning = state.config.tuning;
    let targets: Vec<FxVec2> = state
        .enemies
        .iter()
        .filter(|(_, e)| e.active())
        .map(|(_, e)| e.pos)
        .collect();
    for (slot, (player, input)) in state.players.iter_mut().zip(&inputs.players).enumerate() {
        let Some(player) = player.as_mut().filter(|p| p.alive()) else {
            continue;
        };
        let shot = player.update(*input, &targets, tiles, &tuning);
        // A fall starts at the full `fall_ticks` (at least 1), and only ever this way.
        if player.fall_ticks == tuning.fall_ticks {
            events.events.push(Event::PlayerFell { slot });
            if !player.alive() {
                events.events.push(Event::PlayerDied { slot });
            }
        }
        if let Some(angle) = shot {
            let dir = trig::unit(angle);
            state.bullets.insert(Bullet {
                pos: add(player.pos, scale(dir, MUZZLE)),
                vel: scale(dir, per_tick(tuning.bullet_speed)),
                ticks_left: BULLET_TICKS,
            });
            events.events.push(Event::ShotFired { slot });
        }
    }
}

/// Moves a bullet one tick. Returns whether it is still flying: lifetime left, and not
/// in a wall, void or sealed door (it flies over pits).
fn fly(bullet: &mut Bullet, tiles: Tiles) -> bool {
    bullet.pos = add(bullet.pos, bullet.vel);
    bullet.ticks_left = bullet.ticks_left.saturating_sub(1);
    bullet.ticks_left > 0 && !tiles.blocks_point(bullet.pos, Body::Shot)
}

fn bullets(state: &mut SimState, tiles: Tiles, events: &mut TickEvents) {
    // Each bullet hits at most the first live enemy it overlaps, in slot order.
    let damage = state.config.tuning.damage;
    let enemies = &mut state.enemies;
    state.bullets.retain(|_, bullet| {
        if !fly(bullet, tiles) {
            return false;
        }
        let reach = BULLET_RADIUS.saturating_add(ENEMY_RADIUS);
        let Some((enemy, e)) = enemies
            .iter_mut()
            .find(|(_, e)| e.hp > 0 && e.active() && overlaps(bullet.pos, e.pos, reach))
        else {
            return true;
        };
        e.hp = e.hp.saturating_sub(damage);
        events.events.push(Event::EnemyHit { enemy });
        if e.hp == 0 {
            events.events.push(Event::EnemyKilled { enemy, pos: e.pos });
        }
        false
    });
    enemies.retain(|_, e| e.hp > 0);
}

/// Enemy bullets hurt the first player (in slot order) they overlap who can take the hit.
/// Invulnerable players (rolling, or just hurt) don't stop them: dodged bullets fly on.
fn enemy_bullets(state: &mut SimState, tiles: Tiles, events: &mut TickEvents) {
    let hurt_ticks = state.config.tuning.hurt_ticks;
    let players = &mut state.players;
    state.enemy_bullets.retain(|_, bullet| {
        if !fly(bullet, tiles) {
            return false;
        }
        let reach = ENEMY_BULLET_RADIUS.saturating_add(PLAYER_RADIUS);
        let hit = players.iter_mut().enumerate().find_map(|(slot, player)| {
            let player = player.as_mut()?;
            (overlaps(bullet.pos, player.pos, reach) && player.hurt(hurt_ticks))
                .then_some((slot, player))
        });
        let Some((slot, player)) = hit else {
            return true;
        };
        events.events.push(Event::PlayerHit { slot });
        if !player.alive() {
            events.events.push(Event::PlayerDied { slot });
        }
        false
    });
}

fn enemies(state: &mut SimState, tiles: Tiles, events: &mut TickEvents) {
    let config = state.config;
    let telegraph = config.tuning.shooter_telegraph;
    let rusher_speed = per_tick(config.tuning.rusher_speed);
    let players: Vec<FxVec2> = state
        .players
        .iter()
        .flatten()
        .filter(|p| p.alive())
        .map(|p| p.pos)
        .collect();
    for (_, enemy) in state.enemies.iter_mut().filter(|(_, e)| e.active()) {
        // Nearest living player; `min_by_key` keeps the first, so ties go to the lower slot.
        let Some(&target) = players.iter().min_by_key(|&&p| dist_sq(p, enemy.pos)) else {
            continue;
        };
        // No direction when exactly on the target: stay put (contact still applies).
        let Some(angle) = trig::angle_of(sub(target, enemy.pos)) else {
            continue;
        };
        match &mut enemy.behavior {
            Behavior::Rusher { contact_cooldown } => {
                *contact_cooldown = contact_cooldown.saturating_sub(1);
                enemy.pos = steer(tiles, enemy.pos, angle, rusher_speed, &mut enemy.steer);
            }
            Behavior::Shooter {
                pattern,
                shot_timer,
                strafe,
            } => {
                if *shot_timer <= telegraph {
                    // Aiming: stand still, then fire at wherever the target is now.
                    *shot_timer = shot_timer.saturating_sub(1);
                    if *shot_timer == 0 {
                        fire(
                            &mut state.enemy_bullets,
                            enemy.pos,
                            angle,
                            *pattern,
                            &config,
                        );
                        *shot_timer = pattern.interval(&config);
                        *strafe = strafe.saturating_neg();
                    }
                    continue;
                }
                let clear = line_of_fire(tiles, enemy.pos, target);
                // Only start aiming with a clear line of fire; otherwise keep repositioning.
                if *shot_timer > telegraph.saturating_add(1) || clear {
                    *shot_timer = shot_timer.saturating_sub(1);
                }
                enemy.pos = if !clear || !overlaps(enemy.pos, target, SHOOTER_FAR) {
                    steer(tiles, enemy.pos, angle, SHOOTER_SPEED, &mut enemy.steer)
                } else if overlaps(enemy.pos, target, SHOOTER_NEAR) {
                    let away = angle.wrapping_add(HALF_TURN);
                    steer(tiles, enemy.pos, away, SHOOTER_SPEED, &mut enemy.steer)
                } else {
                    let side = if *strafe < 0 {
                        angle.wrapping_sub(QUARTER_TURN)
                    } else {
                        angle.wrapping_add(QUARTER_TURN)
                    };
                    let to = walk(tiles, enemy.pos, side, SHOOTER_SPEED);
                    if !progressed(enemy.pos, to, SHOOTER_SPEED) {
                        *strafe = strafe.saturating_neg();
                    }
                    to
                };
            }
        }
    }

    separate(state, tiles);

    let reach = PLAYER_RADIUS.saturating_add(ENEMY_RADIUS);
    for (_, enemy) in state.enemies.iter_mut() {
        if !enemy.active() {
            // Counted down last, so a telegraph of N ticks is inert for exactly N.
            enemy.spawn_ticks = enemy.spawn_ticks.saturating_sub(1);
            continue;
        }
        let Behavior::Rusher { contact_cooldown } = &mut enemy.behavior else {
            continue;
        };
        if *contact_cooldown > 0 {
            continue;
        }
        for (slot, player) in state.players.iter_mut().enumerate() {
            if let Some(player) = player
                && overlaps(player.pos, enemy.pos, reach)
                && player.hurt(config.tuning.hurt_ticks)
            {
                *contact_cooldown = CONTACT_COOLDOWN;
                events.events.push(Event::PlayerHit { slot });
                if !player.alive() {
                    events.events.push(Event::PlayerDied { slot });
                }
                break;
            }
        }
    }
}

/// A shooter at `from` fires its `pattern` centered on `angle`.
fn fire(
    bullets: &mut Arena<Bullet>,
    from: FxVec2,
    angle: u16,
    pattern: Pattern,
    config: &RunConfig,
) {
    let (count, speed) = match pattern {
        Pattern::Aimed => (1, config.enemy_bullet_speed()),
        Pattern::Spread => (config.difficulty.spread_pellets(), config.pellet_speed()),
    };
    for i in 0..count {
        // Offsets from the center: (2i - (count - 1)) half-spacings.
        let halves = i16::from(i)
            .saturating_mul(2)
            .saturating_sub(i16::from(count).saturating_sub(1));
        let offset = halves.saturating_mul(PELLET_SPACING / 2);
        let dir = trig::unit(angle.wrapping_add_signed(offset));
        bullets.insert(Bullet {
            pos: add(from, scale(dir, ENEMY_RADIUS)),
            vel: scale(dir, per_tick(speed)),
            ticks_left: ENEMY_BULLET_TICKS,
        });
    }
}

/// One enemy step through the tiles toward `angle`.
fn walk(tiles: Tiles, pos: FxVec2, angle: u16, speed: Fx) -> FxVec2 {
    tiles.slide(
        pos,
        ENEMY_RADIUS,
        scale(trig::unit(angle), speed),
        Body::Walker,
    )
}

/// Whether a step of `speed` covered at least half of that.
fn progressed(from: FxVec2, to: FxVec2, speed: Fx) -> bool {
    let half = i128::from(speed.to_bits()).checked_div(2).unwrap_or(0);
    dist_sq(from, to) >= half.saturating_mul(half)
}

/// A step toward `angle` that detours around whatever blocks it, without pathfinding.
/// Walking already slides along walls, so this only matters when the way is blocked
/// nearly head-on: then it tries 45° and 90° off to one side, then the other. The side
/// that works is kept in `side` while the direct way stays blocked, so a body follows a
/// pillar's face around its corner instead of dithering. The first side tried is the one
/// the blocked step drifted toward.
fn steer(tiles: Tiles, pos: FxVec2, angle: u16, speed: Fx, side: &mut i8) -> FxVec2 {
    let direct = walk(tiles, pos, angle, speed);
    if progressed(pos, direct, speed) {
        *side = 0;
        return direct;
    }
    let first = if *side == 0 {
        // The cross product's sign: which side of the heading the blocked step slid to.
        let (dir, drift) = (trig::unit(angle), sub(direct, pos));
        let cross = dir
            .x
            .saturating_mul(drift.y)
            .saturating_sub(dir.y.saturating_mul(drift.x));
        if cross < Fx::ZERO { -1 } else { 1 }
    } else {
        *side
    };
    for s in [first, first.saturating_neg()] {
        for turns in [1, 2] {
            let turn = EIGHTH_TURN
                .saturating_mul(turns)
                .saturating_mul(i16::from(s));
            let to = walk(tiles, pos, angle.wrapping_add_signed(turn), speed);
            if progressed(pos, to, speed) {
                *side = s;
                return to;
            }
        }
    }
    direct
}

/// Whether a shot from `from` to `to` would clear every wall, void and sealed door on
/// the way (pits don't block shots).
fn line_of_fire(tiles: Tiles, from: FxVec2, to: FxVec2) -> bool {
    let offset = sub(to, from);
    let steps = dist(from, to)
        .checked_div(LINE_STEP)
        .map_or(1, |n| n.saturating_to_num::<i64>().max(1));
    let (Some(dx), Some(dy)) = (
        offset.x.checked_div_int(steps),
        offset.y.checked_div_int(steps),
    ) else {
        return false;
    };
    let mut at = from;
    (0..steps).all(|_| {
        at = add(at, FxVec2 { x: dx, y: dy });
        !tiles.blocks_point(at, Body::Shot)
    })
}

/// Resolves overlaps among active enemies: pushes them apart and out of living players.
/// Every push slides through `tiles` like a walk, so nobody is shoved into a wall, pit,
/// void or sealed door; a body pinned against one leaves the rest of the correction to
/// later passes and to its neighbors. Pairs resolve in slot order, each pass building on
/// the last, so the result is deterministic, and a crowd pressing on a player settles
/// into a still ring around it instead of stacking.
fn separate(state: &mut SimState, tiles: Tiles) {
    let players: Vec<FxVec2> = state
        .players
        .iter()
        .flatten()
        .filter(|p| p.alive())
        .map(|p| p.pos)
        .collect();
    let mut bodies: Vec<FxVec2> = state
        .enemies
        .iter()
        .filter(|(_, e)| e.active())
        .map(|(_, e)| e.pos)
        .collect();
    // Pushes stay under a cell, as `slide` needs: a pair's half is at most
    // ENEMY_SPACING / 2 and a player's push at most PLAYER_SPACING.
    let slide = |pos: FxVec2, push: FxVec2| tiles.slide(pos, ENEMY_RADIUS, push, Body::Walker);
    for _ in 0..SEPARATION_PASSES {
        let mut rest = bodies.as_mut_slice();
        while let Some((a, tail)) = rest.split_first_mut() {
            for b in tail.iter_mut() {
                // Each side of the pair takes half the correction.
                let push = scale(push_out(*a, *b, ENEMY_SPACING), HALF);
                *a = slide(*a, sub(FxVec2::default(), push));
                *b = slide(*b, push);
            }
            rest = tail;
        }
        for body in &mut bodies {
            // Players don't budge.
            for &player in &players {
                *body = slide(*body, push_out(player, *body, PLAYER_SPACING));
            }
        }
    }
    let active = state.enemies.iter_mut().filter(|(_, e)| e.active());
    for ((_, enemy), body) in active.zip(bodies) {
        enemy.pos = body;
    }
}

/// How far `b` must move directly away from `a` for their centers to be `spacing`
/// apart; zero if they already are. Coincident centers part along +x.
fn push_out(a: FxVec2, b: FxVec2, spacing: Fx) -> FxVec2 {
    let spacing_bits = i128::from(spacing.to_bits());
    if dist_sq(a, b) >= spacing_bits.saturating_mul(spacing_bits) {
        return FxVec2::default();
    }
    let gap = dist(a, b);
    let offset = sub(b, a);
    let dir = match (offset.x.checked_div(gap), offset.y.checked_div(gap)) {
        (Some(x), Some(y)) => FxVec2 { x, y },
        _ => FxVec2 {
            x: Fx::ONE,
            y: Fx::ZERO,
        },
    };
    scale(dir, spacing.saturating_sub(gap))
}

/// Distance between two points.
fn dist(a: FxVec2, b: FxVec2) -> Fx {
    // `dist_sq` is in squared raw bits, so its root is in raw bits.
    Fx::from_bits(i64::try_from(dist_sq(a, b).unsigned_abs().isqrt()).unwrap_or(i64::MAX))
}

/// Circles whose radii sum to `reach` overlap.
fn overlaps(a: FxVec2, b: FxVec2, reach: Fx) -> bool {
    let reach = i128::from(reach.to_bits());
    dist_sq(a, b) < reach.saturating_mul(reach)
}

const fn add(a: FxVec2, b: FxVec2) -> FxVec2 {
    FxVec2 {
        x: a.x.saturating_add(b.x),
        y: a.y.saturating_add(b.y),
    }
}

const fn sub(a: FxVec2, b: FxVec2) -> FxVec2 {
    FxVec2 {
        x: a.x.saturating_sub(b.x),
        y: a.y.saturating_sub(b.y),
    }
}
