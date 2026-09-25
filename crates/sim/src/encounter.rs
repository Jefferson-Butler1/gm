//! Rooms in play: walking through exits, room events (seal/unseal), and waves from object
//! layers. Drives `Run::Boarding` <-> `Run::Encounter`.

use crate::combat::Enemy;
use crate::derelict::DERELICT;
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
    for player in state.players.iter_mut().flatten() {
        player.pos = at;
    }
    events.events.push(Event::RoomEntered { room: id });
    let Some(room) = DERELICT.room(id) else {
        state.run = Run::Boarding { room: id };
        return;
    };
    state.run = if state.cleared(id) || !room.has_enemies() {
        Run::Boarding { room: id }
    } else {
        spawn(state, room.base);
        Run::Encounter {
            room: id,
            wave: 0,
            doors_locked: react(room, RoomTrigger::OnEnterWithEnemies, false),
        }
    };
}

/// After combat: advance waves or finish the room, then take any open exit a living
/// player stands in.
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
    let exit = state
        .players
        .iter()
        .flatten()
        .filter(|p| p.alive())
        .find_map(|p| tiles.room.exit_at(cell_of(p.pos.x), cell_of(p.pos.y)));
    if let Some(from) = state.run.room()
        && let Some(exit) = exit
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
                spawn(state, layer.placements);
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

fn spawn(state: &mut SimState, placements: &[Placement]) {
    for placement in placements {
        let pos = cell_center(placement.x, placement.y);
        state.enemies.insert(match placement.kind {
            EnemyKind::Rusher => Enemy::rusher(pos),
        });
    }
}
