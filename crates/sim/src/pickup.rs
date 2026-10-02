//! Pickups and Scrap, the run's currency (issue #57).
//!
//! ETG's floor budget: a floor's Scrap total is rolled from the run seed when its ship
//! generates, and split across every enemy placement in its rooms ([`budget`]), so it is
//! seedable. Each spawned enemy carries its share and drops it where it dies, as 5s and
//! 1s scattered a little. Each hit an enemy lands on a player halves its share, so taking
//! hits costs Scrap. When a room's fight ends, its pickups fly to the nearest living
//! player; walking over one collects it any time, into the party's shared wallet.

use crate::arena::Arena;
use crate::combat::{dist, overlaps};
use crate::player::{PLAYER_RADIUS, Player, dist_sq, scale};
use crate::rng::Rng;
use crate::room::{EnemyKind, Placement, PrototypeRoom};
use crate::ship::{Body, Ship, Tiles};
use crate::{Event, Fx, FxVec2, RoomId, SimState, TickEvents};
use serde::{Deserialize, Serialize};

/// A floor's Scrap total: `SCRAP_MEAN` ± `SCRAP_SPREAD`, uniform, and never under
/// `SCRAP_MIN`. Starting guesses to tune on Garold.
pub const SCRAP_MEAN: u16 = 60;
pub const SCRAP_SPREAD: u16 = 15;
pub const SCRAP_MIN: u16 = 40;
/// The bridge captain's fixed share, off the top of the total.
pub const CAPTAIN_SCRAP: u8 = 15;
/// Hitbox radius; also the half-extent drops scatter against tiles with.
pub const PICKUP_RADIUS: Fx = Fx::from_bits(4 << 32);
/// Drops land up to this many points (either axis) from where the enemy died.
const SCATTER: u32 = 12;
/// A magnetized pickup flies 10 pt/tick = 600 pt/s: faster than a player walks.
const MAGNET_SPEED: Fx = Fx::from_bits(10 << 32);

/// Salts the run seed for the budget's draws, so they aren't the run RNG's own stream.
const BUDGET_STREAM: u64 = 0x5C4A_9B00_D6E7_0057;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PickupKind {
    /// Worth this much Scrap: 1 or 5.
    Scrap(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Pickup {
    pub kind: PickupKind,
    pub pos: FxVec2,
    /// Flying to the nearest living player: its room's fight is over.
    pub magnet: bool,
}

/// The floor's Scrap total for run `seed`.
#[must_use]
pub fn scrap_total(seed: u64) -> u16 {
    roll_total(&mut Rng::from_seed(seed ^ BUDGET_STREAM))
}

fn roll_total(rng: &mut Rng) -> u16 {
    let span = u32::from(SCRAP_SPREAD).saturating_mul(2).saturating_add(1);
    let roll = u16::try_from(rng.below(span)).unwrap_or(0);
    SCRAP_MEAN
        .saturating_sub(SCRAP_SPREAD)
        .saturating_add(roll)
        .max(SCRAP_MIN)
}

/// Each enemy placement's share of [`scrap_total`], in the floor's placement order (see
/// [`first_share`]).
///
/// Captains take [`CAPTAIN_SCRAP`] each; the rest splits evenly across
/// the other placements, the remainder going 1 at a time to random ones. Sums to the
/// total unless the floor has nobody to carry it.
#[must_use]
pub fn budget(seed: u64, ship: &Ship) -> Vec<u8> {
    let mut rng = Rng::from_seed(seed ^ BUDGET_STREAM);
    let total = roll_total(&mut rng);
    let captain: Vec<bool> = (ship.rooms().iter())
        .flat_map(|p| layers(&p.room))
        .flatten()
        .map(|p| p.kind == EnemyKind::Captain)
        .collect();
    let others: Vec<usize> = (0..)
        .zip(&captain)
        .filter_map(|(i, &c)| (!c).then_some(i))
        .collect();
    let captains = captain.len().saturating_sub(others.len());
    let captains = u16::try_from(captains).unwrap_or(u16::MAX);
    let rest = total.saturating_sub(captains.saturating_mul(CAPTAIN_SCRAP.into()));
    let n = u16::try_from(others.len()).unwrap_or(u16::MAX);
    let each = u8::try_from(rest.checked_div(n).unwrap_or(0)).unwrap_or(u8::MAX);
    let mut shares: Vec<u8> = (captain.iter())
        .map(|&c| if c { CAPTAIN_SCRAP } else { each })
        .collect();
    for _ in 0..rest.checked_rem(n).unwrap_or(0) {
        let pick = usize::try_from(rng.below(n.into())).ok();
        let at = pick.and_then(|i| others.get(i));
        if let Some(share) = at.and_then(|&i| shares.get_mut(i)) {
            *share = share.saturating_add(1);
        }
    }
    shares
}

/// A room's object layers in wave order: the base, then its reinforcements.
fn layers(room: &PrototypeRoom) -> impl Iterator<Item = &'static [Placement]> {
    std::iter::once(room.base).chain(room.reinforcements.iter().map(|r| r.placements))
}

/// Where room `id`'s wave `wave` starts in the floor's placement order: rooms by
/// [`RoomId`], each room's layers in wave order, each layer's placements in order.
#[must_use]
pub fn first_share(ship: &Ship, id: RoomId, wave: u8) -> usize {
    let count = |layers: &mut dyn Iterator<Item = &'static [Placement]>| -> usize {
        layers.map(<[Placement]>::len).sum()
    };
    let before: usize = (ship.rooms().iter().take(usize::from(id.0)))
        .map(|p| count(&mut layers(&p.room)))
        .sum();
    let within = ship
        .room(id)
        .map_or(0, |p| count(&mut layers(&p.room).take(usize::from(wave))));
    before.saturating_add(within)
}

/// Drops `scrap` at `at` as pickups, 5s then 1s, each scattered up to [`SCATTER`] pt off
/// (sliding through `tiles` like a walker, so none lands in a wall or pit).
pub(crate) fn drop_scrap(
    pickups: &mut Arena<Pickup>,
    rng: &mut Rng,
    tiles: Tiles<'_>,
    at: FxVec2,
    scrap: u8,
) {
    let tiles = tiles.walker_at(at);
    let pieces = std::iter::repeat_n(5, usize::from(scrap / 5))
        .chain(std::iter::repeat_n(1, usize::from(scrap % 5)));
    let range = i32::try_from(SCATTER).unwrap_or(0);
    for value in pieces {
        let mut offset = || {
            let d = i32::try_from(rng.below(SCATTER.saturating_mul(2).saturating_add(1)))
                .unwrap_or(range);
            Fx::from_num(d.saturating_sub(range))
        };
        let offset = FxVec2 {
            x: offset(),
            y: offset(),
        };
        pickups.insert(Pickup {
            kind: PickupKind::Scrap(value),
            pos: tiles.slide(at, PICKUP_RADIUS, offset, Body::Walker),
            magnet: false,
        });
    }
}

/// Room `room`'s fight is over: its pickups start flying to the party.
pub(crate) fn magnetize(state: &mut SimState, room: RoomId) {
    let ship = &state.ship;
    for (_, pickup) in state.pickups.iter_mut() {
        pickup.magnet |= ship.room_at(pickup.pos) == Some(room);
    }
}

/// Magnetized pickups fly toward the nearest living player (through anything); then every
/// pickup a living player touches is collected, into the party's wallet.
pub(crate) fn tick(state: &mut SimState, events: &mut TickEvents) {
    let living: Vec<(usize, FxVec2)> = (state.players.iter().enumerate())
        .filter_map(|(slot, p)| p.filter(Player::alive).map(|p| (slot, p.pos)))
        .collect();
    let reach = PLAYER_RADIUS.saturating_add(PICKUP_RADIUS);
    let scrap = &mut state.scrap;
    state.pickups.retain(|_, pickup| {
        let nearest = living.iter().min_by_key(|(_, p)| dist_sq(*p, pickup.pos));
        if pickup.magnet
            && let Some(&(_, to)) = nearest
        {
            pickup.pos = approach(pickup.pos, to, MAGNET_SPEED);
        }
        let Some(&(slot, _)) = living.iter().find(|(_, p)| overlaps(*p, pickup.pos, reach)) else {
            return true;
        };
        let PickupKind::Scrap(value) = pickup.kind;
        *scrap = scrap.saturating_add(value.into());
        events.events.push(Event::ScrapCollected { slot, value });
        false
    });
}

/// A step from `from` up to `step` straight toward `to`, stopping on it.
fn approach(from: FxVec2, to: FxVec2, step: Fx) -> FxVec2 {
    let gap = dist(from, to);
    if gap <= step {
        return to;
    }
    let k = step.checked_div(gap).unwrap_or(Fx::ZERO);
    let delta = scale(
        FxVec2 {
            x: to.x.saturating_sub(from.x),
            y: to.y.saturating_sub(from.y),
        },
        k,
    );
    FxVec2 {
        x: from.x.saturating_add(delta.x),
        y: from.y.saturating_add(delta.y),
    }
}
