//! Rooms in play (ETG's lifecycle): players open hatches by stepping into them, a room's
//! enemies spawn the first time a player is wholly inside it, room events seal and unseal
//! its hatches, and waves come from object layers. Clearing the bridge (the boss room)
//! unlocks every airlock's outer hatch, and stepping into an unlocked one wins. Access
//! panels hide crawlspaces until [`reveal`]ed, and reward and secret rooms hold a chest.
//! Drives `Run::Boarding` <-> `Run::Encounter` -> `Run::Won`. A hatch banging open or
//! sealed is a noise that unaware enemies nearby come to investigate.

use crate::combat::{Arrival, Awareness, Enemy, Pattern, SHOOTER_STAGGER, stand_ticks};
use crate::player::{PLAYER_RADIUS, dist_sq};
use crate::room::{
    Category, EnemyKind, LayerTrigger, Placed, Placement, RoomAction, RoomTrigger, cell_center,
};
use crate::{
    Event, Fx, FxVec2, HatchId, HatchKind, HatchState, RoomId, Run, SimState, TickEvents, bit, trig,
};

/// Reveals `hatch` if it's an access panel, turning it Closed. Returns whether it was one.
///
/// Revealed, it's an ordinary hatch a player opens by stepping into it. This is the one
/// way panels open: a player's bullet hitting one calls it, and so will EMPs.
pub fn reveal(state: &mut SimState, hatch: HatchId) -> bool {
    match state.hatches.get_mut(usize::from(hatch.0)) {
        Some(s) if *s == HatchState::Panel => {
            *s = HatchState::Closed;
            true
        }
        Some(_) | None => false,
    }
}

/// `u16` turns.
const EIGHTH_TURN: u16 = 8192;

/// After combat: advance the fight's waves, then open the closed hatches living players
/// step into, or win if one is an airlock's outer hatch (unlocked, so the bridge is
/// clear), and open the closed chests they touch. Then (no fight on) start the first
/// uncleared room with enemies a living player is wholly inside.
pub fn tick(state: &mut SimState, events: &mut TickEvents) {
    if let Run::Encounter { room: id, wave } = state.run
        && state.enemies.is_empty()
    {
        next_wave(state, id, wave, events);
    }

    let ship = std::sync::Arc::clone(&state.ship);
    let living: Vec<_> = state
        .players
        .iter()
        .flatten()
        .filter(|p| p.alive())
        .map(|p| p.pos)
        .collect();
    for &pos in &living {
        for hatch in ship.hatches_under(pos, PLAYER_RADIUS) {
            if state.hatches.get(usize::from(hatch.0)) != Some(&HatchState::Closed) {
                continue;
            }
            if ship
                .hatches()
                .get(usize::from(hatch.0))
                .is_some_and(|h| h.kind == HatchKind::Airlock)
            {
                state.run = Run::Won;
                events.events.push(Event::Won);
                return;
            }
            open(state, hatch);
            events.events.push(Event::HatchOpened { hatch });
            bang(state, &[hatch], events);
        }
        if let Some(room) = ship.on_chest(pos, PLAYER_RADIUS)
            && !state.chest_opened(room)
        {
            state.chests |= bit(room);
            events.events.push(Event::ChestOpened { room });
        }
    }

    if state.run != Run::Boarding {
        return;
    }
    let entered = living.iter().find_map(|&pos| {
        let id = ship.inside(pos, PLAYER_RADIUS)?;
        let room = ship.room(id)?;
        (!state.cleared(id) && room.room.has_enemies()).then_some((id, *room))
    });
    if let Some((id, room)) = entered {
        spawn(state, &room, room.room.base, Arrival::Prespawn);
        state.run = Run::Encounter { room: id, wave: 0 };
        react(state, id, &room, RoomTrigger::OnEnterWithEnemies, events);
    }
}

/// The current wave of room `id` is dead: spawn the next reinforcement layer, or fire
/// the cleared events and end the encounter.
fn next_wave(state: &mut SimState, id: RoomId, wave: u8, events: &mut TickEvents) {
    let Some(room) = state.ship.room(id).copied() else {
        return;
    };
    // Reinforcement `i` is wave `i + 1`.
    if let Some(layer) = room.room.reinforcements.get(usize::from(wave)) {
        match layer.trigger {
            LayerTrigger::OnEnemiesCleared => {
                let wave = wave.saturating_add(1);
                spawn(state, &room, layer.placements, Arrival::Reinforcement);
                state.run = Run::Encounter { room: id, wave };
                events.events.push(Event::WaveStarted { wave });
            }
        }
        return;
    }
    react(state, id, &room, RoomTrigger::OnEnemiesCleared, events);
    if room.room.category == Category::Boss {
        for s in &mut state.hatches {
            if *s == HatchState::AirlockLocked {
                *s = HatchState::Closed;
            }
        }
    }
    state.cleared |= bit(id);
    events.events.push(Event::RoomCleared { room: id });
    state.run = Run::Boarding;
}

/// Applies `room`'s actions for `trigger`, in order, to the hatches in its walls: seal
/// them all (slamming shut: a [`bang`]), or open them all (revealing what's behind).
fn react(
    state: &mut SimState,
    id: RoomId,
    room: &Placed,
    trigger: RoomTrigger,
    events: &mut TickEvents,
) {
    let hatches: Vec<HatchId> = state.ship.hatches_of(id).collect();
    for &(_, action) in room.room.events.iter().filter(|&&(t, _)| t == trigger) {
        match action {
            RoomAction::Seal => {
                for hatch in &hatches {
                    if let Some(s) = state.hatches.get_mut(usize::from(hatch.0)) {
                        *s = HatchState::Sealed;
                    }
                }
                bang(state, &hatches, events);
            }
            RoomAction::Unseal => {
                for &hatch in &hatches {
                    unseal(state, hatch);
                }
            }
        }
    }
}

/// `hatches` banged open or shut: each unaware enemy in a room beside one, within the
/// run's `hatch_hearing_radius` of its middle, investigates the nearest such.
fn bang(state: &mut SimState, hatches: &[HatchId], events: &mut TickEvents) {
    let ship = std::sync::Arc::clone(&state.ship);
    let noises: Vec<(FxVec2, [RoomId; 2])> = hatches
        .iter()
        .filter_map(|id| ship.hatches().get(usize::from(id.0)))
        .map(|h| {
            let (first, last) = (h.gap.cell(0), h.gap.cell(h.gap.width.saturating_sub(1)));
            let (a, b) = (cell_center(first.0, first.1), cell_center(last.0, last.1));
            let mid = |a: Fx, b: Fx| {
                a.saturating_add(b.saturating_sub(a).checked_div_int(2).unwrap_or_default())
            };
            let at = FxVec2 {
                x: mid(a.x, b.x),
                y: mid(a.y, b.y),
            };
            (at, h.rooms)
        })
        .collect();
    let radius = i128::from(Fx::from_num(state.config.tuning.hatch_hearing_radius).to_bits());
    let earshot = radius.saturating_mul(radius);
    for (id, enemy) in state.enemies.iter_mut() {
        let Some(room) = ship.room_at(enemy.pos) else {
            continue;
        };
        let heard = noises
            .iter()
            .filter(|(at, rooms)| rooms.contains(&room) && dist_sq(*at, enemy.pos) < earshot)
            .map(|&(at, _)| at)
            .min_by_key(|&at| dist_sq(at, enemy.pos));
        if let Some(spot) = heard {
            enemy.investigate(id, spot, events);
        }
    }
}

/// Opens `hatch` for good and reveals the rooms on both sides.
fn open(state: &mut SimState, hatch: HatchId) {
    if let Some(s) = state.hatches.get_mut(usize::from(hatch.0)) {
        *s = HatchState::Open;
    }
    if let Some(h) = state.ship.hatches().get(usize::from(hatch.0)) {
        state.visited |= h.rooms.iter().fold(0, |bits, &r| bits | bit(r));
    }
}

/// Lifts `hatch`'s seal: Open if both sides are already revealed, else Closed, so the
/// room behind stays fogged until a player touches the hatch (ETG).
fn unseal(state: &mut SimState, hatch: HatchId) {
    let Some(h) = state.ship.hatches().get(usize::from(hatch.0)) else {
        return;
    };
    let revealed = h.rooms.iter().all(|&r| state.visited(r));
    if let Some(s) = state.hatches.get_mut(usize::from(hatch.0)) {
        *s = if revealed {
            HatchState::Open
        } else {
            HatchState::Closed
        };
    }
}

/// Spawns `placements` (cells of `room`) with `arrival`'s telegraph, except a captain,
/// which gets its own. Prespawns start unaware, facing one of 8 directions at random, and
/// stand a random while before their first patrol walk, so a room doesn't set off in
/// step. Reinforcements and bosses join a fight in progress, already knowing where the
/// nearest living player is and facing it.
fn spawn(state: &mut SimState, room: &Placed, placements: &[Placement], arrival: Arrival) {
    for placement in placements {
        let arrival = if placement.kind == EnemyKind::Captain {
            Arrival::Boss
        } else {
            arrival
        };
        let pos = cell_center(
            placement.x.saturating_add(room.at.0),
            placement.y.saturating_add(room.at.1),
        );
        let nearest = state
            .players
            .iter()
            .flatten()
            .filter(|p| p.alive())
            .map(|p| p.pos)
            .min_by_key(|&p| dist_sq(p, pos));
        let idle = u16::try_from(state.rng.below(8)).unwrap_or(0);
        let idle = idle.saturating_mul(EIGHTH_TURN);
        let (awareness, facing) = match nearest {
            Some(last_seen) if arrival != Arrival::Prespawn => (
                Awareness::Alert {
                    last_seen,
                    searching: 0,
                },
                trig::angle_of(FxVec2 {
                    x: last_seen.x.saturating_sub(pos.x),
                    y: last_seen.y.saturating_sub(pos.y),
                })
                .unwrap_or(idle),
            ),
            Some(_) | None => (Awareness::Unaware, idle),
        };
        let enemy = match placement.kind {
            EnemyKind::Rusher => Enemy::rusher(pos),
            EnemyKind::Shooter | EnemyKind::SpreadShooter | EnemyKind::Captain => {
                // Spread placements field plain shooters unless the experiment is on.
                let pattern = match placement.kind {
                    EnemyKind::Captain => Pattern::Captain,
                    EnemyKind::SpreadShooter if state.config.tuning.spread_shooter => {
                        Pattern::Spread
                    }
                    EnemyKind::Rusher | EnemyKind::Shooter | EnemyKind::SpreadShooter => {
                        Pattern::Aimed
                    }
                };
                let delay = state.rng.below(SHOOTER_STAGGER);
                let delay = u16::try_from(delay).unwrap_or(0);
                Enemy::shooter(pos, pattern, &state.config, delay)
            }
        };
        let patrol = crate::Patrol {
            ticks: stand_ticks(&mut state.rng),
            ..enemy.patrol
        };
        state.enemies.insert(Enemy {
            facing,
            awareness,
            patrol,
            arrival,
            spawn_ticks: arrival.telegraph_ticks(),
            ..enemy
        });
    }
}
