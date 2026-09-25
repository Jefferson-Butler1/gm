//! Gun, bullets and the melee rusher, colliding with the room's tiles. Systems run in a
//! fixed order over arenas iterated in slot order, so every tie resolves the same way on
//! every machine. Tuning values are first guesses for combat tuning (issue #15).

use crate::arena::Id;
use crate::player::{PLAYER_RADIUS, Player, dist_sq, scale};
use crate::room::{Body, Tiles};
use crate::{Event, Fx, FxVec2, Run, SimState, TickEvents, TickInputs, encounter, trig};
use serde::{Deserialize, Serialize};

pub type EnemyId = Id<Enemy>;

/// Bullet speed: 15 pt/tick = 900 pt/s.
const BULLET_SPEED: Fx = Fx::from_bits(15 << 32);
/// Bullets vanish after 60 ticks = 1 s if they hit nothing.
const BULLET_TICKS: u8 = 60;
/// Hitbox radius.
pub const BULLET_RADIUS: Fx = Fx::from_bits(4 << 32);
/// Bullets leave the gun this far ahead of the player's center.
const MUZZLE: Fx = Fx::from_bits(22 << 32);

/// Hitbox radius; also its half-extent against tiles.
pub const RUSHER_RADIUS: Fx = Fx::from_bits(13 << 32);
pub const RUSHER_HP: u8 = 3;
/// Rusher chase speed: 3 pt/tick = 180 pt/s (the player runs 420).
const RUSHER_SPEED: Fx = Fx::from_bits(3 << 32);
/// Ticks after a rusher lands a contact hit before it can land another: 0.5 s.
const CONTACT_COOLDOWN: u8 = 30;

/// Active enemies' centers are kept this far apart: they touch but never stack.
const ENEMY_SPACING: Fx = Fx::from_bits(RUSHER_RADIUS.to_bits().saturating_mul(2));
/// ...and this far from a living player's: pressed 3 pt into contact reach, so contact
/// still lands but a crowd rings the player instead of piling onto it.
const PLAYER_SPACING: Fx = Fx::from_bits(
    PLAYER_RADIUS
        .to_bits()
        .saturating_add(RUSHER_RADIUS.to_bits())
        .saturating_sub(3 << 32),
);
/// Separation passes per tick; each pass shrinks what a crowd's pressure leaves over.
const SEPARATION_PASSES: u8 = 8;
const HALF: Fx = Fx::from_bits(1 << 31);

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

/// The melee rusher: chases the nearest living player and hurts on contact.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Enemy {
    pub pos: FxVec2,
    pub hp: u8,
    pub contact_cooldown: u8,
    /// Spawn telegraph left; nonzero = inert (see [`SPAWN_TELEGRAPH_TICKS`]).
    pub spawn_ticks: u8,
}

impl Enemy {
    /// A rusher arriving at `pos`. Every spawn starts in the telegraph, so any spawner
    /// just inserts this.
    #[must_use]
    pub const fn rusher(pos: FxVec2) -> Self {
        Self {
            pos,
            hp: RUSHER_HP,
            contact_cooldown: 0,
            spawn_ticks: SPAWN_TELEGRAPH_TICKS,
        }
    }

    /// Past the spawn telegraph: moves, hurts, and can be targeted and hit.
    #[must_use]
    pub const fn active(&self) -> bool {
        self.spawn_ticks == 0
    }
}

/// One live tick, in order: players (move, fire), bullets, enemies (chase, separate,
/// contact, telegraph countdown), the death check, then the room (waves, exits).
pub fn tick(state: &mut SimState, inputs: &TickInputs, events: &mut TickEvents) {
    let Some(tiles) = state.tiles() else {
        return;
    };
    players(state, inputs, tiles, events);
    bullets(state, tiles, events);
    enemies(state, tiles, events);
    if !state.players.iter().flatten().any(Player::alive) {
        state.run = Run::Dead {
            room: state.run.room().unwrap_or_default(),
            ticks_until_restart: DEATH_TICKS,
        };
        return;
    }
    encounter::tick(state, events);
}

fn players(state: &mut SimState, inputs: &TickInputs, tiles: Tiles, events: &mut TickEvents) {
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
        if let Some(angle) = player.update(*input, &targets, tiles) {
            let dir = trig::unit(angle);
            state.bullets.insert(Bullet {
                pos: add(player.pos, scale(dir, MUZZLE)),
                vel: scale(dir, BULLET_SPEED),
                ticks_left: BULLET_TICKS,
            });
            events.events.push(Event::ShotFired { slot });
        }
    }
}

fn bullets(state: &mut SimState, tiles: Tiles, events: &mut TickEvents) {
    // Each bullet hits at most the first live enemy it overlaps, in slot order. Walls,
    // void and sealed doors stop it; it flies over pits.
    let enemies = &mut state.enemies;
    state.bullets.retain(|_, bullet| {
        bullet.pos = add(bullet.pos, bullet.vel);
        bullet.ticks_left = bullet.ticks_left.saturating_sub(1);
        if bullet.ticks_left == 0 || tiles.blocks_point(bullet.pos, Body::Shot) {
            return false;
        }
        let reach = BULLET_RADIUS.saturating_add(RUSHER_RADIUS);
        let Some((enemy, e)) = enemies
            .iter_mut()
            .find(|(_, e)| e.hp > 0 && e.active() && overlaps(bullet.pos, e.pos, reach))
        else {
            return true;
        };
        e.hp = e.hp.saturating_sub(1);
        events.events.push(Event::EnemyHit { enemy });
        if e.hp == 0 {
            events.events.push(Event::EnemyKilled { enemy, pos: e.pos });
        }
        false
    });
    enemies.retain(|_, e| e.hp > 0);
}

fn enemies(state: &mut SimState, tiles: Tiles, events: &mut TickEvents) {
    for (_, enemy) in state.enemies.iter_mut().filter(|(_, e)| e.active()) {
        enemy.contact_cooldown = enemy.contact_cooldown.saturating_sub(1);
        // Nearest living player; `min_by_key` keeps the first, so ties go to the lower slot.
        let target = state
            .players
            .iter()
            .flatten()
            .filter(|p| p.alive())
            .min_by_key(|p| dist_sq(p.pos, enemy.pos));
        // No direction when exactly on the target: stay put (contact still applies).
        if let Some(step) = target
            .and_then(|p| trig::angle_of(sub(p.pos, enemy.pos)))
            .map(|a| scale(trig::unit(a), RUSHER_SPEED))
        {
            // No pathfinding yet: it slides along whatever is in the way.
            enemy.pos = tiles.slide(enemy.pos, RUSHER_RADIUS, step, Body::Walker);
        }
    }

    separate(state, tiles);

    let reach = PLAYER_RADIUS.saturating_add(RUSHER_RADIUS);
    for (_, enemy) in state.enemies.iter_mut() {
        if !enemy.active() {
            // Counted down last, so a telegraph of N ticks is inert for exactly N.
            enemy.spawn_ticks = enemy.spawn_ticks.saturating_sub(1);
            continue;
        }
        if enemy.contact_cooldown > 0 {
            continue;
        }
        for (slot, player) in state.players.iter_mut().enumerate() {
            if let Some(player) = player
                && overlaps(player.pos, enemy.pos, reach)
                && player.hurt()
            {
                enemy.contact_cooldown = CONTACT_COOLDOWN;
                events.events.push(Event::PlayerHit { slot });
                if !player.alive() {
                    events.events.push(Event::PlayerDied { slot });
                }
                break;
            }
        }
    }
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
    let slide = |pos: FxVec2, push: FxVec2| tiles.slide(pos, RUSHER_RADIUS, push, Body::Walker);
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
    let gap_sq = dist_sq(a, b);
    if gap_sq >= spacing_bits.saturating_mul(spacing_bits) {
        return FxVec2::default();
    }
    // `dist_sq` is in squared raw bits, so its root is in raw bits.
    let gap = Fx::from_bits(i64::try_from(gap_sq.unsigned_abs().isqrt()).unwrap_or(i64::MAX));
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
