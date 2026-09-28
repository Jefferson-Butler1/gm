//! Rooms in play (ETG's lifecycle): players open hatches by stepping into them, a room's
//! enemies spawn the first time a player is wholly inside it, room events seal and unseal
//! its hatches, waves come from object layers, and a clear room's extraction pad wins.
//! Drives `Run::Boarding` <-> `Run::Encounter` -> `Run::Won`.

use crate::combat::{Arrival, Awareness, Enemy, Pattern, SHOOTER_STAGGER};
use crate::player::{PLAYER_RADIUS, dist_sq};
use crate::room::{
    EnemyKind, LayerTrigger, Placed, Placement, RoomAction, RoomTrigger, cell_center,
};
use crate::{Event, HatchId, HatchState, RoomId, Run, SimState, TickEvents, bit};

/// After combat: advance the fight's waves, open the closed hatches living players step
/// into, then (no fight on) start the first uncleared room with enemies a living player
/// is wholly inside, or win if one touches a clear room's extraction pad.
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
            if state.hatches.get(usize::from(hatch.0)) == Some(&HatchState::Closed) {
                open(state, hatch);
                events.events.push(Event::HatchOpened { hatch });
            }
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
        react(state, id, &room, RoomTrigger::OnEnterWithEnemies);
        return;
    }
    let won = (0..).map(RoomId).take(ship.rooms().len()).any(|id| {
        state.extraction_live(id)
            && living
                .iter()
                .any(|&p| ship.on_extraction(id, p, PLAYER_RADIUS))
    });
    if won {
        state.run = Run::Won;
        events.events.push(Event::Won);
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
    react(state, id, &room, RoomTrigger::OnEnemiesCleared);
    state.cleared |= bit(id);
    events.events.push(Event::RoomCleared { room: id });
    state.run = Run::Boarding;
}

/// Applies `room`'s actions for `trigger`, in order, to the hatches in its walls: seal
/// them all, or open them all (revealing what's behind).
fn react(state: &mut SimState, id: RoomId, room: &Placed, trigger: RoomTrigger) {
    let ship = std::sync::Arc::clone(&state.ship);
    for &(_, action) in room.room.events.iter().filter(|&&(t, _)| t == trigger) {
        for hatch in ship.hatches_of(id) {
            match action {
                RoomAction::Seal => {
                    if let Some(s) = state.hatches.get_mut(usize::from(hatch.0)) {
                        *s = HatchState::Sealed;
                    }
                }
                RoomAction::Unseal => unseal(state, hatch),
            }
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

/// Spawns `placements` (cells of `room`) with `arrival`'s telegraph. Prespawns start
/// unaware; reinforcements join a fight in progress, already knowing where the nearest
/// living player is.
fn spawn(state: &mut SimState, room: &Placed, placements: &[Placement], arrival: Arrival) {
    for placement in placements {
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
        let awareness = match nearest {
            Some(last_seen) if arrival == Arrival::Reinforcement => Awareness::Alert {
                last_seen,
                searching: 0,
            },
            Some(_) | None => Awareness::Unaware,
        };
        let enemy = match placement.kind {
            EnemyKind::Rusher => Enemy::rusher(pos),
            EnemyKind::Shooter | EnemyKind::SpreadShooter => {
                // Spread placements field plain shooters unless the experiment is on.
                let pattern = if placement.kind == EnemyKind::SpreadShooter
                    && state.config.tuning.spread_shooter
                {
                    Pattern::Spread
                } else {
                    Pattern::Aimed
                };
                let delay = state.rng.below(SHOOTER_STAGGER);
                let delay = u16::try_from(delay).unwrap_or(0);
                Enemy::shooter(pos, pattern, &state.config, delay)
            }
        };
        state.enemies.insert(Enemy {
            awareness,
            arrival,
            spawn_ticks: arrival.telegraph_ticks(),
            ..enemy
        });
    }
}
