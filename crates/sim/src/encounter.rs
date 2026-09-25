//! Rooms in play: walking through exits, room events (seal/unseal), waves from object
//! layers, and extraction. Drives `Run::Boarding` <-> `Run::Encounter` -> `Run::Won`.

use crate::combat::{Awareness, Enemy, Pattern, SHOOTER_STAGGER};
use crate::derelict::DERELICT;
use crate::player::{PLAYER_RADIUS, dist_sq};
use crate::room::{
    EnemyKind, LayerTrigger, Placement, PrototypeRoom, RoomAction, RoomTrigger, cell_center,
    cell_of,
};
use crate::{Event, FxVec2, RoomId, Run, SimState, TickEvents};

/// Moves the whole party to `at` in room `id` and starts that room: an Encounter (base
/// layer spawned, entry events fired) if it still has enemies, else Boarding. Bullets
/// and enemies belong to the room being left, so they are dropped.
pub fn enter(state: &mut SimState, id: RoomId, at: FxVec2, events: &mut TickEvents) {
    state.enemies.retain(|_, _| false);
    state.bullets.retain(|_, _| false);
    state.enemy_bullets.retain(|_, _| false);
    for player in state.players.iter_mut().flatten() {
        player.pos = at;
        player.solid = at;
    }
    events.events.push(Event::RoomEntered { room: id });
    let Some(room) = DERELICT.room(id) else {
        state.run = Run::Boarding { room: id };
        return;
    };
    state.run = if state.cleared(id) || !room.has_enemies() {
        Run::Boarding { room: id }
    } else {
        spawn(state, room.base, false);
        Run::Encounter {
            room: id,
            wave: 0,
            doors_locked: react(room, RoomTrigger::OnEnterWithEnemies, false),
        }
    };
}

/// After combat: advance waves or finish the room, then win if a living player touches
/// a clear room's extraction pad, else take any open exit a living player stands in.
pub fn tick(state: &mut SimState, events: &mut TickEvents) {
    if let Run::Encounter {
        room: id,
        wave,
        doors_locked,
    } = state.run
        && state.enemies.is_empty()
        && let Some(room) = DERELICT.room(id)
    {
        next_wave(state, room, id, wave, doors_locked, events);
    }

    let Some(tiles) = state.tiles() else {
        return;
    };
    if tiles.sealed {
        return;
    }
    let mut living = state.players.iter().flatten().filter(|p| p.alive());
    if let Run::Boarding { room } = state.run
        && living.any(|p| tiles.room.on_extraction(p.pos, PLAYER_RADIUS))
    {
        state.run = Run::Won { room };
        events.events.push(Event::Won);
        return;
    }
    let exit = state
        .players
        .iter()
        .flatten()
        .filter(|p| p.alive())
        .find_map(|p| tiles.room.exit_at(cell_of(p.pos.x), cell_of(p.pos.y)));
    let from = state.run.room();
    if let Some(exit) = exit
        && let Some((to, to_exit)) = DERELICT.link(from, exit)
        && let Some(arrival) = DERELICT
            .room(to)
            .and_then(|r| r.exits.get(to_exit))
            .map(crate::room::Exit::arrival)
    {
        enter(state, to, arrival, events);
    }
}

/// The current wave is dead: spawn the next reinforcement layer, or fire the cleared
/// events. The encounter ends once the last wave is dead and the doors are unlocked.
fn next_wave(
    state: &mut SimState,
    room: &PrototypeRoom,
    id: RoomId,
    wave: u8,
    doors_locked: bool,
    events: &mut TickEvents,
) {
    // Reinforcement `i` is wave `i + 1`.
    if let Some(layer) = room.reinforcements.get(usize::from(wave)) {
        match layer.trigger {
            LayerTrigger::OnEnemiesCleared => {
                let wave = wave.saturating_add(1);
                spawn(state, layer.placements, true);
                state.run = Run::Encounter {
                    room: id,
                    wave,
                    doors_locked,
                };
                events.events.push(Event::WaveStarted { wave });
            }
        }
        return;
    }
    let doors_locked = react(room, RoomTrigger::OnEnemiesCleared, doors_locked);
    // Gungeon-style: clearing the room clears its enemy fire too.
    state.enemy_bullets.retain(|_, _| false);
    state.set_cleared(id);
    events.events.push(Event::RoomCleared { room: id });
    state.run = if doors_locked {
        // Unreachable for valid rooms (validation requires Seal to pair with an Unseal).
        Run::Encounter {
            room: id,
            wave,
            doors_locked,
        }
    } else {
        Run::Boarding { room: id }
    };
}

/// Applies `room`'s actions for `trigger`, in order, to the doors' locked state.
fn react(room: &PrototypeRoom, trigger: RoomTrigger, doors_locked: bool) -> bool {
    room.events
        .iter()
        .filter(|&&(t, _)| t == trigger)
        .fold(doors_locked, |_, &(_, action)| match action {
            RoomAction::Seal => true,
            RoomAction::Unseal => false,
        })
}

/// Spawns `placements`, unaware unless `hunting`: then each already knows where the
/// nearest living player is (reinforcements join a fight in progress).
fn spawn(state: &mut SimState, placements: &[Placement], hunting: bool) {
    for placement in placements {
        let pos = cell_center(placement.x, placement.y);
        let nearest = state
            .players
            .iter()
            .flatten()
            .filter(|p| p.alive())
            .map(|p| p.pos)
            .min_by_key(|&p| dist_sq(p, pos));
        let awareness = match nearest {
            Some(last_seen) if hunting => Awareness::Alert {
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
        state.enemies.insert(Enemy { awareness, ..enemy });
    }
}
