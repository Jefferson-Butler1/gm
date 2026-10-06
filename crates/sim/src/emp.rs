//! EMPs (issue #55), our blanks: swiped off the EMP button one flies about 3 cells and
//! goes off where it lands, or on the first wall; tapped it goes off at the player's feet.
//!
//! Going off, it clears every enemy bullet in its room and stuns every enemy there for
//! [`STUN_TICKS`]. Within [`BLAST_RADIUS`] it also hits enemies for [`DAMAGE`], shoves
//! them [`SHOVE`] out, and reveals access panels. Outside a room (in a hatch's gap) only
//! the blast acts.

use crate::combat::{ENEMY_RADIUS, overlaps};
use crate::player::{dist_sq, scale};
use crate::room::cell_center;
use crate::ship::{Body, HatchKind, Tiles};
use crate::{Event, Fx, FxVec2, HatchId, SimState, TickEvents, pickup, trig};
use serde::{Deserialize, Serialize};

/// EMPs a player boards with; there's no cap.
pub const EMP_START: u8 = 2;
/// A throw flies this many ticks (0.2 s)...
pub const THROW_TICKS: u8 = 12;
/// ...at 8 pt per tick: 96 pt, 3 cells.
const THROW_SPEED: Fx = Fx::from_bits(8 << 32);
/// The blast: 2 cells.
pub const BLAST_RADIUS: Fx = Fx::from_bits(64 << 32);
/// What the blast does to an enemy: a pistol hit at Normal.
const DAMAGE: u8 = 5;
/// Enemies in the room are stunned this long: 2 s.
pub const STUN_TICKS: u8 = 120;
/// The blast shoves enemies this far straight out from it: a cell, in quarter steps.
const SHOVE: Fx = Fx::from_bits(32 << 32);

/// An EMP in flight.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Emp {
    pub pos: FxVec2,
    pub vel: FxVec2,
    /// Flight left; it goes off at 0.
    pub ticks_left: u8,
}

/// Slot `slot`'s EMP press: `throw` is the swipe's move bucket, `None` a tap. Spends one
/// if the player has any; a tap goes off at once.
pub fn press(
    state: &mut SimState,
    slot: usize,
    throw: Option<u8>,
    tiles: Tiles<'_>,
    events: &mut TickEvents,
) -> Vec<HatchId> {
    let Some(player) = state.players.get_mut(slot).and_then(Option::as_mut) else {
        return Vec::new();
    };
    if !player.targetable() || player.emps == 0 {
        return Vec::new();
    }
    player.emps = player.emps.saturating_sub(1);
    let pos = player.pos;
    events.events.push(Event::EmpThrown { slot });
    match throw {
        Some(bucket) => {
            let dir = trig::unit(crate::player::bucket_angle(bucket));
            state.emps.insert(Emp {
                pos,
                vel: scale(dir, THROW_SPEED),
                ticks_left: THROW_TICKS,
            });
            Vec::new()
        }
        None => detonate(state, pos, tiles, events),
    }
}

/// Flies every EMP a tick, setting off those that land or meet a wall (or shut hatch).
/// Returns the access panels the blasts reach.
pub fn tick(state: &mut SimState, tiles: Tiles<'_>, events: &mut TickEvents) -> Vec<HatchId> {
    let mut landed = Vec::new();
    state.emps.retain(|_, emp| {
        let next = FxVec2 {
            x: emp.pos.x.saturating_add(emp.vel.x),
            y: emp.pos.y.saturating_add(emp.vel.y),
        };
        emp.ticks_left = emp.ticks_left.saturating_sub(1);
        if tiles.blocks_point(next, Body::Shot) {
            landed.push(emp.pos);
            return false;
        }
        emp.pos = next;
        if emp.ticks_left == 0 {
            landed.push(emp.pos);
            return false;
        }
        true
    });
    landed
        .into_iter()
        .flat_map(|at| detonate(state, at, tiles, events))
        .collect()
}

/// Sets off an EMP at `at`. Returns the access panels in its blast.
fn detonate(
    state: &mut SimState,
    at: FxVec2,
    tiles: Tiles<'_>,
    events: &mut TickEvents,
) -> Vec<HatchId> {
    let ship = tiles.ship;
    let room = ship.room_at(at);
    let in_room = |p: FxVec2| room.is_some() && ship.room_at(p) == room;
    let blast = |p: FxVec2, radius: Fx| overlaps(at, p, BLAST_RADIUS.saturating_add(radius));
    state
        .enemy_bullets
        .retain(|_, b| !in_room(b.pos) && !blast(b.pos, Fx::ZERO));
    let (enemies, pickups, rng) = (&mut state.enemies, &mut state.pickups, &mut state.rng);
    for (id, enemy) in enemies.iter_mut().filter(|(_, e)| e.active()) {
        let hit = blast(enemy.pos, ENEMY_RADIUS);
        if !hit && !in_room(enemy.pos) {
            continue;
        }
        enemy.stun_ticks = STUN_TICKS;
        if !hit {
            continue;
        }
        enemy.hp = enemy.hp.saturating_sub(DAMAGE);
        events.events.push(Event::EnemyHit { enemy: id });
        if enemy.hp == 0 {
            events.events.push(Event::EnemyKilled {
                enemy: id,
                pos: enemy.pos,
            });
            pickup::drop_scrap(pickups, rng, tiles, enemy.pos, enemy.scrap);
            continue;
        }
        // Straight out from the blast (along +x, if dead on it), sliding so it stays in
        // its room.
        let away = trig::angle_of(FxVec2 {
            x: enemy.pos.x.saturating_sub(at.x),
            y: enemy.pos.y.saturating_sub(at.y),
        })
        .unwrap_or(0);
        let step = scale(trig::unit(away), SHOVE.saturating_div(Fx::from_num(4)));
        let walker = tiles.walker_at(enemy.pos);
        for _ in 0..4 {
            enemy.pos = walker.slide(enemy.pos, ENEMY_RADIUS, step, Body::Walker);
        }
    }
    enemies.retain(|_, e| e.hp > 0);
    events.events.push(Event::EmpDetonated { pos: at });
    let reach = i128::from(BLAST_RADIUS.to_bits());
    (0..)
        .map(HatchId)
        .zip(ship.hatches())
        .filter(|(_, h)| h.kind == HatchKind::Panel)
        .filter(|(_, h)| {
            (0..h.gap.width).any(|i| {
                let (x, y) = h.gap.cell(i);
                dist_sq(cell_center(x, y), at) < reach.saturating_mul(reach)
            })
        })
        .map(|(id, _)| id)
        .collect()
}
