//! Gun, bullets, the melee rusher, and the temporary spawner. Systems run in a fixed
//! order over arenas iterated in slot order, so every tie resolves the same way on every
//! machine. Tuning values are first guesses for combat tuning (issue #15).

use crate::arena::Id;
use crate::player::{PLAYER_RADIUS, Player, ROOM_HALF, clamp, dist_sq, scale};
use crate::{Event, Fx, FxVec2, Rng, Run, SimState, TickEvents, TickInputs, trig};
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

/// Hitbox radius.
pub const RUSHER_RADIUS: Fx = Fx::from_bits(13 << 32);
pub const RUSHER_HP: u8 = 3;
/// Rusher chase speed: 3 pt/tick = 180 pt/s (the player runs 420).
const RUSHER_SPEED: Fx = Fx::from_bits(3 << 32);
/// Ticks after a rusher lands a contact hit before it can land another: 0.5 s.
const CONTACT_COOLDOWN: u8 = 30;

/// Spawn telegraph: a new enemy spends 30 ticks = 0.5 s as a warning marker. Meanwhile
/// it is inert: it doesn't move, hurt, push or get pushed, and it can't be targeted or
/// hit (bullets pass through).
pub const SPAWN_TELEGRAPH_TICKS: u8 = 30;

/// Ticks from all players dying until restart is accepted: 0.75 s, so a panicked tap
/// doesn't skip the death.
pub const DEATH_TICKS: u32 = 45;

/// TEMPORARY spawner until rooms and waves land (Rooms step): keep this many rushers
/// alive in the placeholder room, adding one every [`SPAWN_INTERVAL`] ticks.
const RUSHERS_ALIVE: usize = 4;
const SPAWN_INTERVAL: u16 = 40;
/// Grace period at run start before the first spawn: 1 s.
pub const FIRST_SPAWN_TICKS: u16 = 60;
/// A spawn point this close to a living player flips to the far side of the room.
const SPAWN_CLEARANCE: Fx = Fx::from_bits(160 << 32);

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

/// One live tick of combat, in order: players (move, fire), bullets, enemies, death check,
/// spawner.
pub fn tick(state: &mut SimState, inputs: &TickInputs, events: &mut TickEvents) {
    players(state, inputs, events);
    bullets(state, events);
    enemies(state, events);
    if !state.players.iter().flatten().any(Player::alive) {
        state.run = Run::Dead {
            ticks_until_restart: DEATH_TICKS,
        };
        return;
    }
    spawner(state);
}

fn players(state: &mut SimState, inputs: &TickInputs, events: &mut TickEvents) {
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
        if let Some(angle) = player.update(*input, &targets) {
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

fn bullets(state: &mut SimState, events: &mut TickEvents) {
    // Each bullet hits at most the first live enemy it overlaps, in slot order.
    let enemies = &mut state.enemies;
    state.bullets.retain(|_, bullet| {
        bullet.pos = add(bullet.pos, bullet.vel);
        bullet.ticks_left = bullet.ticks_left.saturating_sub(1);
        if bullet.ticks_left == 0 || !inside_room(bullet.pos) {
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

fn enemies(state: &mut SimState, events: &mut TickEvents) {
    for (_, enemy) in state.enemies.iter_mut() {
        if !enemy.active() {
            enemy.spawn_ticks = enemy.spawn_ticks.saturating_sub(1);
            continue;
        }
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
            enemy.pos = FxVec2 {
                x: clamp(
                    enemy.pos.x.saturating_add(step.x),
                    ROOM_HALF.x.saturating_sub(RUSHER_RADIUS),
                ),
                y: clamp(
                    enemy.pos.y.saturating_add(step.y),
                    ROOM_HALF.y.saturating_sub(RUSHER_RADIUS),
                ),
            };
        }

        if enemy.contact_cooldown > 0 {
            continue;
        }
        let reach = PLAYER_RADIUS.saturating_add(RUSHER_RADIUS);
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

/// TEMPORARY: see [`RUSHERS_ALIVE`].
fn spawner(state: &mut SimState) {
    state.spawn_cooldown = state.spawn_cooldown.saturating_sub(1);
    if state.spawn_cooldown == 0 && state.enemies.len() < RUSHERS_ALIVE {
        state.spawn_cooldown = SPAWN_INTERVAL;
        let mut pos = edge_point(&mut state.rng);
        let crowded = |pos: FxVec2| {
            let clear = i128::from(SPAWN_CLEARANCE.to_bits());
            let clear_sq = clear.saturating_mul(clear);
            state
                .players
                .iter()
                .flatten()
                .any(|p| p.alive() && dist_sq(p.pos, pos) < clear_sq)
        };
        if crowded(pos) {
            pos = FxVec2 {
                x: pos.x.saturating_neg(),
                y: pos.y.saturating_neg(),
            };
        }
        state.enemies.insert(Enemy::rusher(pos));
    }
}

/// A random point on the placeholder room's edge, inset so the rusher fits.
fn edge_point(rng: &mut Rng) -> FxVec2 {
    let half = FxVec2 {
        x: ROOM_HALF.x.saturating_sub(RUSHER_RADIUS),
        y: ROOM_HALF.y.saturating_sub(RUSHER_RADIUS),
    };
    let edge = rng.below(4);
    let mut along = |half: Fx| {
        let span = half.saturating_mul_int(2).saturating_to_num::<u32>();
        Fx::from_num(rng.below(span)).saturating_sub(half)
    };
    match edge {
        0 => FxVec2 {
            x: along(half.x),
            y: half.y.saturating_neg(),
        },
        1 => FxVec2 {
            x: along(half.x),
            y: half.y,
        },
        2 => FxVec2 {
            x: half.x.saturating_neg(),
            y: along(half.y),
        },
        _ => FxVec2 {
            x: half.x,
            y: along(half.y),
        },
    }
}

fn inside_room(p: FxVec2) -> bool {
    p.x.saturating_abs() <= ROOM_HALF.x && p.y.saturating_abs() <= ROOM_HALF.y
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
