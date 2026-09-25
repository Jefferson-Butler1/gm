//! Rooms through the public API: format validation, walls and pits, walking through
//! exits, seal on entry (or while enemies hunt), waves, unseal on clear, and walking out
//! of a fight and back.

use sim::room::{
    CELL, Category, Connection, Derelict, DerelictError, Dir, EnemyKind, Exit, ExitKind, ExitRef,
    Placement, PrototypeRoom, RoomAction, RoomError, RoomTrigger, cell_center,
};
use sim::{
    Awareness, Behavior, Buttons, DERELICT, Event, Fx, FxVec2, PLAYER_RADIUS, PlayerInput, RoomId,
    Run, RunConfig, SimState, TickInputs, step,
};

const SEED: u64 = 3;
const LEFT: u16 = 32768;
/// Move buckets (32 per turn).
const EAST: u8 = 0;
const WEST: u8 = 16;

// --- validation ---------------------------------------------------------------------

/// A valid 5 x 5 room with an east exit and one rusher; tests break one thing at a time.
const BOX: PrototypeRoom = PrototypeRoom {
    name: "box",
    category: Category::Normal,
    cells: &["#####", "#...#", "#....", "#...#", "#####"],
    exits: &[Exit {
        dir: Dir::East,
        x: 4,
        y: 2,
        width: 1,
        kind: ExitKind::Either,
    }],
    base: &[Placement {
        kind: EnemyKind::Rusher,
        x: 2,
        y: 2,
    }],
    reinforcements: &[],
    events: &[
        (RoomTrigger::OnEnterWithEnemies, RoomAction::Seal),
        (RoomTrigger::OnEnemiesCleared, RoomAction::Unseal),
    ],
    extraction: None,
};

fn exit(dir: Dir, x: usize, y: usize, width: usize) -> &'static [Exit] {
    Box::leak(Box::new([Exit {
        dir,
        x,
        y,
        width,
        kind: ExitKind::Either,
    }]))
}

#[test]
fn shipped_rooms_and_the_test_box_are_valid() {
    assert_eq!(DERELICT.validate(), Ok(()));
    assert_eq!(BOX.validate(), Ok(()));
}

#[test]
fn ragged_rows_and_unknown_cells_are_rejected() {
    let ragged = PrototypeRoom {
        cells: &["#####", "#...#", "#...", "#...#", "#####"],
        ..BOX
    };
    assert_eq!(ragged.validate(), Err(RoomError::RaggedRow { row: 2 }));
    let unknown = PrototypeRoom {
        cells: &["#####", "#.x.#", "#....", "#...#", "#####"],
        ..BOX
    };
    assert_eq!(
        unknown.validate(),
        Err(RoomError::UnknownCell { x: 2, y: 1 })
    );
}

#[test]
fn exits_must_be_floor_gaps_on_their_edge() {
    let off_edge = |exits| PrototypeRoom { exits, ..BOX }.validate();
    let error = Err(RoomError::ExitOffEdge { exit: 0 });
    assert_eq!(off_edge(exit(Dir::East, 3, 2, 1)), error, "not on the edge");
    assert_eq!(off_edge(exit(Dir::West, 4, 2, 1)), error, "wrong edge");
    assert_eq!(
        off_edge(exit(Dir::East, 4, 4, 2)),
        error,
        "runs off the grid"
    );
    assert_eq!(off_edge(exit(Dir::East, 4, 2, 0)), error, "empty");
    assert_eq!(
        off_edge(exit(Dir::East, 4, 1, 2)),
        Err(RoomError::ExitNotFloor { exit: 0 }),
        "covers wall"
    );
    let blocked = PrototypeRoom {
        cells: &["#####", "#...#", "#..#.", "#...#", "#####"],
        ..BOX
    };
    assert_eq!(blocked.validate(), Err(RoomError::ExitBlocked { exit: 0 }));
}

#[test]
fn placements_must_be_on_floor() {
    let on = |x, y| {
        let base = Box::leak(Box::new([Placement {
            kind: EnemyKind::Rusher,
            x,
            y,
        }]));
        PrototypeRoom { base, ..BOX }.validate()
    };
    assert_eq!(on(0, 0), Err(RoomError::PlacementOffFloor { x: 0, y: 0 }));
    assert_eq!(on(9, 9), Err(RoomError::PlacementOffFloor { x: 9, y: 9 }));
    let pit = PrototypeRoom {
        cells: &["#####", "#...#", "#.o..", "#...#", "#####"],
        base: &[Placement {
            kind: EnemyKind::Rusher,
            x: 2,
            y: 2,
        }],
        ..BOX
    };
    assert_eq!(
        pit.validate(),
        Err(RoomError::PlacementOffFloor { x: 2, y: 2 })
    );
}

#[test]
fn unreachable_floor_is_rejected() {
    let split = PrototypeRoom {
        cells: &["#####", "#.#.#", "#.#..", "#.#.#", "#####"],
        base: &[],
        ..BOX
    };
    assert_eq!(split.validate(), Err(RoomError::UnreachableFloor));
    // Pits cut floor apart too: walkers can't cross them.
    let moat = PrototypeRoom {
        cells: &["#####", "#.o.#", "#.o..", "#.o.#", "#####"],
        base: &[],
        ..BOX
    };
    assert_eq!(moat.validate(), Err(RoomError::UnreachableFloor));
}

#[test]
fn a_room_that_seals_must_unseal_on_clear() {
    let trap = PrototypeRoom {
        events: &[(RoomTrigger::OnEnterWithEnemies, RoomAction::Seal)],
        ..BOX
    };
    assert_eq!(trap.validate(), Err(RoomError::SealsForever));
}

#[test]
fn connections_must_link_facing_exits_exactly_once() {
    const WEST_BOX: PrototypeRoom = PrototypeRoom {
        cells: &["#####", "#...#", "....#", "#...#", "#####"],
        exits: &[Exit {
            dir: Dir::West,
            x: 0,
            y: 2,
            width: 1,
            kind: ExitKind::Either,
        }],
        category: Category::Exit,
        extraction: Some((2, 2)),
        ..BOX
    };
    const NORMAL_WEST_BOX: PrototypeRoom = PrototypeRoom {
        category: Category::Normal,
        extraction: None,
        ..WEST_BOX
    };
    const RAGGED_BOX: PrototypeRoom = PrototypeRoom {
        cells: &["#####", "#...#", "#...", "#...#", "#####"],
        ..BOX
    };
    const fn link(from: usize, to: usize) -> Connection {
        Connection {
            from: ExitRef {
                room: from,
                exit: 0,
            },
            to: ExitRef { room: to, exit: 0 },
        }
    }
    const BOX_TO_WEST_BOX: &[Connection] = &[link(0, 1)];
    const TO_MISSING_ROOM: &[Connection] = &[link(0, 2)];
    let derelict = |rooms, connections| Derelict {
        rooms,
        connections,
        start_room: 0,
        start_cell: (1, 1),
    };

    assert_eq!(
        derelict(&[BOX, WEST_BOX], BOX_TO_WEST_BOX).validate(),
        Ok(())
    );
    assert_eq!(
        derelict(&[BOX, BOX], BOX_TO_WEST_BOX).validate(),
        Err(DerelictError::MismatchedExits { connection: 0 }),
        "east to east"
    );
    assert_eq!(
        derelict(&[BOX, WEST_BOX], &[]).validate(),
        Err(DerelictError::ExitLinks { room: 0, exit: 0 })
    );
    assert_eq!(
        derelict(&[BOX, WEST_BOX], TO_MISSING_ROOM).validate(),
        Err(DerelictError::BadExitRef { connection: 0 })
    );
    assert_eq!(
        derelict(&[RAGGED_BOX, WEST_BOX], BOX_TO_WEST_BOX).validate(),
        Err(DerelictError::Room {
            room: 0,
            error: RoomError::RaggedRow { row: 2 }
        })
    );
    assert_eq!(
        derelict(&[BOX, NORMAL_WEST_BOX], BOX_TO_WEST_BOX).validate(),
        Err(DerelictError::NoExitRoom),
        "nowhere to win"
    );
}

#[test]
fn exit_rooms_and_only_exit_rooms_have_an_extraction_pad_on_floor() {
    let exit_room = |extraction| PrototypeRoom {
        category: Category::Exit,
        extraction,
        ..BOX
    };
    assert_eq!(exit_room(Some((1, 1))).validate(), Ok(()));
    assert_eq!(
        exit_room(None).validate(),
        Err(RoomError::ExtractionMismatch)
    );
    assert_eq!(
        exit_room(Some((0, 0))).validate(),
        Err(RoomError::ExtractionOffFloor)
    );
    let stray = PrototypeRoom {
        extraction: Some((1, 1)),
        ..BOX
    };
    assert_eq!(stray.validate(), Err(RoomError::ExtractionMismatch));
}

// --- play ---------------------------------------------------------------------------

fn walk(bucket: u8) -> TickInputs {
    let mut inputs = TickInputs::default();
    inputs.players[0] = PlayerInput {
        move_dir: bucket,
        move_mag: u8::MAX,
        ..PlayerInput::default()
    };
    inputs
}

fn run(state: &mut SimState, ticks: usize, inputs: &TickInputs) -> Vec<Event> {
    (0..ticks)
        .flat_map(|_| step(state, inputs).events)
        .collect()
}

/// Walks one way until the party changes rooms (at most 2 s); returns the new room.
fn walk_to_next_room(state: &mut SimState, bucket: u8) -> Option<RoomId> {
    (0..120).find_map(|_| {
        step(state, &walk(bucket))
            .events
            .iter()
            .find_map(|e| match e {
                Event::RoomEntered { room } => Some(*room),
                _ => None,
            })
    })
}

/// A fresh run with slot 0 moved to `pos` in the airlock.
fn airlock_at(pos: FxVec2) -> SimState {
    let mut state = SimState::new(SEED, RunConfig::default());
    if let Some(player) = &mut state.players[0] {
        player.pos = pos;
    }
    state
}

/// Walks from just inside the airlock's east exit into the cargo hold, under the run's
/// `doors_lock_on_aggro` set to `aggro` (off: seal on entry, unseal on clear).
fn enter_cargo_hold(aggro: bool) -> (SimState, Vec<Event>) {
    let mut state = airlock_at(FxVec2 {
        x: cell_center(12, 4).x,
        y: CELL.saturating_mul_int(5), // centered on the 2-cell gap (rows 4 and 5)
    });
    state.config.tuning.doors_lock_on_aggro = aggro;
    let events = run(&mut state, 10, &walk(EAST));
    (state, events)
}

#[test]
fn walking_through_an_exit_enters_the_linked_room_and_seals_it() {
    let (state, events) = enter_cargo_hold(false);
    assert!(events.contains(&Event::RoomEntered { room: RoomId(1) }));
    assert_eq!(
        state.run,
        Run::Encounter {
            room: RoomId(1),
            wave: 0,
            doors_locked: true
        }
    );
    assert_eq!(state.enemies.len(), 3, "base layer");
    let hold_west_exit = DERELICT.rooms[1].exits[0];
    assert_eq!(
        state.players[0].unwrap().pos.y.round(),
        hold_west_exit.arrival().y,
        "lands at the exit (then keeps walking east)"
    );
}

#[test]
fn sealed_doors_stop_players_and_bullets() {
    let (mut state, _) = enter_cargo_hold(false);
    run(&mut state, 30, &walk(WEST));
    assert_eq!(state.run.room(), RoomId(1), "still inside");
    assert_eq!(
        state.players[0].unwrap().pos.x,
        CELL.saturating_add(PLAYER_RADIUS),
        "flush with the sealed gap"
    );

    let mut fire_left = TickInputs::default();
    fire_left.players[0] = PlayerInput {
        aim: LEFT,
        buttons: Buttons::FIRE,
        ..PlayerInput::default()
    };
    let events = run(&mut state, 1, &fire_left);
    assert!(events.contains(&Event::ShotFired { slot: 0 }));
    run(&mut state, 2, &TickInputs::default());
    assert!(state.bullets.is_empty(), "the sealed door ate it");
}

#[test]
fn waves_advance_on_clear_then_the_room_unseals_and_stays_cleared() {
    let (mut state, _) = enter_cargo_hold(false);

    state.enemies.retain(|_, _| false);
    let events = run(&mut state, 1, &TickInputs::default());
    assert!(
        events.contains(&Event::WaveStarted { wave: 1 }),
        "{events:?}"
    );
    assert_eq!(
        state.run,
        Run::Encounter {
            room: RoomId(1),
            wave: 1,
            doors_locked: true
        }
    );
    assert_eq!(state.enemies.len(), 4, "reinforcement layer");
    assert!(
        state.enemies.iter().all(|(_, e)| !e.active()),
        "waves arrive in the spawn telegraph"
    );

    state.enemies.retain(|_, _| false);
    let events = run(&mut state, 1, &TickInputs::default());
    assert!(events.contains(&Event::RoomCleared { room: RoomId(1) }));
    assert_eq!(state.run, Run::Boarding { room: RoomId(1) });

    // Unsealed: walk back out west into the airlock, then back in to a quiet room.
    assert_eq!(walk_to_next_room(&mut state, WEST), Some(RoomId(0)));
    assert_eq!(walk_to_next_room(&mut state, EAST), Some(RoomId(1)));
    assert_eq!(state.run, Run::Boarding { room: RoomId(1) });
    assert!(state.enemies.is_empty());
}

fn locked(state: &SimState) -> bool {
    state.tiles().is_some_and(|t| t.sealed)
}

/// Sets every enemy's awareness: hunting slot 0 where it stands, or not.
fn aware(state: &mut SimState, hunting: bool) {
    let at = state.players[0].map(|p| p.pos).unwrap_or_default();
    for (_, enemy) in state.enemies.iter_mut() {
        enemy.awareness = if hunting {
            Awareness::Alert {
                last_seen: at,
                searching: 0,
            }
        } else {
            Awareness::Unaware
        };
    }
}

#[test]
fn with_doors_lock_on_aggro_doors_seal_only_while_an_enemy_hunts() {
    let (mut state, _) = enter_cargo_hold(true);
    assert!(matches!(state.run, Run::Encounter { wave: 0, .. }));
    // The base wave telegraphs in and stands unaware, off screen: the doors stay open.
    run(&mut state, 60, &TickInputs::default());
    assert!(!locked(&state));

    aware(&mut state, true);
    run(&mut state, 1, &TickInputs::default());
    assert!(locked(&state), "sealed while they hunt");

    // They give up: open again.
    aware(&mut state, false);
    run(&mut state, 1, &TickInputs::default());
    assert!(!locked(&state));

    // Only the shooter (far from the rushers, so it alerts neither) hunts, then dies:
    // open again, with the rushers alive and unaware.
    let (_, shooter) = state
        .enemies
        .iter_mut()
        .find(|(_, e)| matches!(e.behavior, Behavior::Shooter { .. }))
        .unwrap();
    shooter.awareness = Awareness::Alert {
        last_seen: shooter.pos,
        searching: 0,
    };
    run(&mut state, 1, &TickInputs::default());
    assert!(locked(&state));
    state
        .enemies
        .retain(|_, e| matches!(e.behavior, Behavior::Rusher { .. }));
    run(&mut state, 1, &TickInputs::default());
    assert_eq!(state.enemies.len(), 2);
    assert!(!locked(&state));
}

#[test]
fn with_doors_lock_on_aggro_a_fight_walked_out_of_resumes_on_return() {
    let (mut state, _) = enter_cargo_hold(true);
    // The base wave dies; the reinforcements arrive hunting and seal the doors.
    state.enemies.retain(|_, _| false);
    run(&mut state, 1, &TickInputs::default());
    assert!(locked(&state));
    // They lose the party, and one dies: it walks out through the open doors.
    aware(&mut state, false);
    let (first, _) = state.enemies.iter().next().unwrap();
    state.enemies.retain(|id, _| id != first);
    assert_eq!(walk_to_next_room(&mut state, WEST), Some(RoomId(0)));
    assert!(state.enemies.is_empty());
    let left = state.suspended.first().map(|s| s.enemies.clone()).unwrap();
    assert_eq!(left.len(), 3);

    // Back in: the same wave and survivors, as they were left; not cleared.
    assert_eq!(walk_to_next_room(&mut state, EAST), Some(RoomId(1)));
    assert!(matches!(
        state.run,
        Run::Encounter {
            room: RoomId(1),
            wave: 1,
            doors_locked: false
        }
    ));
    assert_eq!(state.enemies.len(), left.len());
    assert!(state.enemies.iter().all(|(_, e)| left.contains(e)));
    assert!(state.suspended.is_empty());
    assert!(!state.cleared(RoomId(1)));

    // Killing them clears the room: no wave comes again.
    state.enemies.retain(|_, _| false);
    let events = run(&mut state, 1, &TickInputs::default());
    assert!(events.contains(&Event::RoomCleared { room: RoomId(1) }));
    assert_eq!(state.run, Run::Boarding { room: RoomId(1) });
}

#[test]
fn bullets_fly_over_pits_and_stop_at_walls() {
    // The airlock has a pit at cells (3..=4, 3) and its west wall at x = 0..32.
    let mut state = airlock_at(cell_center(6, 3));
    let mut fire_left = TickInputs::default();
    fire_left.players[0] = PlayerInput {
        aim: LEFT,
        buttons: Buttons::FIRE,
        ..PlayerInput::default()
    };
    run(&mut state, 1, &fire_left);
    run(&mut state, 7, &TickInputs::default());
    let past_pit = CELL.saturating_mul_int(3);
    let bullet = state.bullets.iter().next().map(|(_, b)| b.pos.x);
    assert!(
        bullet.is_some_and(|x| x < past_pit && x > Fx::ZERO),
        "{bullet:?}"
    );
    run(&mut state, 4, &TickInputs::default());
    assert!(
        state.bullets.is_empty(),
        "stopped by the wall, not lifetime"
    );
}
