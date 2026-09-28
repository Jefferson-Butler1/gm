//! Rooms through the public API: format validation, placing rooms in one floor grid,
//! walls and pits, walking through hatches with no cut, fog, seal on entry, waves, open
//! on clear, and enemies kept in their room.

use sim::room::{
    CELL, Category, Connection, Derelict, DerelictError, Dir, EnemyKind, Exit, ExitKind, ExitRef,
    Placed, Placement, PrototypeRoom, RoomAction, RoomError, RoomTrigger, cell_center,
};
use sim::{
    Awareness, Buttons, DERELICT, ENEMY_RADIUS, Enemy, Event, Fx, FxVec2, HatchId, HatchState,
    MAX_HP, PLAYER_RADIUS, PlayerInput, RoomId, Run, RunConfig, SimState, TickInputs, Tuning,
    per_tick, step,
};

const SEED: u64 = 3;
const LEFT: u16 = 32768;
const UP: u16 = 49152;
/// Move buckets (32 per turn).
const SOUTH: u8 = 8;
const NORTH: u8 = 24;

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
fn floor_must_be_walled_in_except_at_exits() {
    let on_edge = PrototypeRoom {
        cells: &["#####", "#...#", "#....", "#....", "#####"],
        ..BOX
    };
    assert_eq!(
        on_edge.validate(),
        Err(RoomError::Unenclosed { x: 4, y: 3 })
    );
    let into_void = PrototypeRoom {
        cells: &["#####", "#... ", "#....", "#...#", "#####"],
        ..BOX
    };
    assert_eq!(
        into_void.validate(),
        Err(RoomError::Unenclosed { x: 3, y: 1 })
    );
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
fn connections_must_link_facing_exits_on_the_same_cells_exactly_once() {
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
    const fn at(room: PrototypeRoom, x: usize) -> Placed {
        Placed { room, at: (x, 0) }
    }
    let derelict = |rooms: [Placed; 2], connections| Derelict {
        rooms: Box::leak(Box::new(rooms)),
        connections,
        start_room: 0,
        start_cell: (1, 1),
    };

    // Sharing BOX's east wall: the exits are the same cells.
    assert_eq!(
        derelict([at(BOX, 0), at(WEST_BOX, 4)], BOX_TO_WEST_BOX).validate(),
        Ok(())
    );
    assert_eq!(
        derelict([at(BOX, 0), at(WEST_BOX, 5)], BOX_TO_WEST_BOX).validate(),
        Err(DerelictError::MismatchedExits { connection: 0 }),
        "a cell apart"
    );
    assert_eq!(
        derelict([at(BOX, 0), at(BOX, 4)], BOX_TO_WEST_BOX).validate(),
        Err(DerelictError::MismatchedExits { connection: 0 }),
        "east to east"
    );
    assert_eq!(
        derelict([at(BOX, 0), at(WEST_BOX, 4)], &[]).validate(),
        Err(DerelictError::ExitLinks { room: 0, exit: 0 })
    );
    assert_eq!(
        derelict([at(BOX, 0), at(WEST_BOX, 4)], TO_MISSING_ROOM).validate(),
        Err(DerelictError::BadExitRef { connection: 0 })
    );
    assert_eq!(
        derelict([at(RAGGED_BOX, 0), at(WEST_BOX, 4)], BOX_TO_WEST_BOX).validate(),
        Err(DerelictError::Room {
            room: 0,
            error: RoomError::RaggedRow { row: 2 }
        })
    );
    assert_eq!(
        derelict([at(BOX, 0), at(NORMAL_WEST_BOX, 4)], BOX_TO_WEST_BOX).validate(),
        Err(DerelictError::NoExitRoom),
        "nowhere to win"
    );
}

#[test]
fn placed_rooms_share_only_walls_and_hatches_and_all_join_the_start() {
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
    /// No exits: joins nothing.
    const CLOSET: PrototypeRoom = PrototypeRoom {
        cells: &["###", "#.#", "###"],
        exits: &[],
        base: &[],
        events: &[],
        ..BOX
    };
    const LINK: &[Connection] = &[Connection {
        from: ExitRef { room: 0, exit: 0 },
        to: ExitRef { room: 1, exit: 0 },
    }];
    let with_closet = |x| Derelict {
        rooms: Box::leak(Box::new([
            Placed {
                room: BOX,
                at: (0, 0),
            },
            Placed {
                room: WEST_BOX,
                at: (4, 0),
            },
            Placed {
                room: CLOSET,
                at: (x, 1),
            },
        ])),
        connections: LINK,
        start_room: 0,
        start_cell: (1, 1),
    };
    assert_eq!(
        with_closet(1).validate(),
        Err(DerelictError::Overlap { x: 1, y: 1 }),
        "the closet's walls on the box's floor"
    );
    assert_eq!(
        with_closet(20).validate(),
        Err(DerelictError::UnreachableRoom { room: 2 })
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

const AIRLOCK: RoomId = RoomId(0);
const CARGO_HOLD: RoomId = RoomId(1);
const ENGINE_ROOM: RoomId = RoomId(2);
/// The cargo hold's hatches: to the airlock above it (floor cells 14..=15 of row 9), and
/// to the engine room below it.
const INTO_HOLD: HatchId = HatchId(0);
const OUT_OF_HOLD: HatchId = HatchId(1);
/// The hold's first row: its top edge is the hatch's bottom edge.
const HOLD_TOP: Fx = CELL.saturating_mul_int(10);

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

fn pos(state: &SimState) -> FxVec2 {
    state.players[0].map(|p| p.pos).unwrap_or_default()
}

const fn place(state: &mut SimState, at: FxVec2) {
    if let Some(player) = &mut state.players[0] {
        player.pos = at;
        player.solid = at;
    }
}

/// A fresh run with slot 0 in the airlock, lined up over the hatch down into the cargo
/// hold, two cells above it.
fn above_the_hold() -> SimState {
    let mut state = SimState::new(SEED, RunConfig::default());
    place(
        &mut state,
        FxVec2 {
            x: CELL.saturating_mul_int(15), // centered on the 2-cell gap
            y: cell_center(0, 7).y,
        },
    );
    state
}

/// Walks slot 0 south from [`above_the_hold`] until the hold's fight starts (at most
/// 2 s), checking every step is an ordinary walk: straight down (to within trig
/// rounding), never faster than a run.
fn walk_into_the_hold() -> (SimState, Vec<Event>) {
    let mut state = above_the_hold();
    let rounding = Fx::from_bits(1 << 20);
    let speed = per_tick(Tuning::NORMAL.move_speed).saturating_add(rounding);
    let mut events = Vec::new();
    for _ in 0..120 {
        let before = pos(&state);
        events.extend(step(&mut state, &walk(SOUTH)).events);
        let after = pos(&state);
        let moved = after.y.saturating_sub(before.y);
        assert!(
            after.x.abs_diff(before.x) < rounding && moved > Fx::ZERO && moved <= speed,
            "no cut: {before:?} -> {after:?}"
        );
        if matches!(state.run, Run::Encounter { .. }) {
            break;
        }
    }
    (state, events)
}

#[test]
fn walking_into_a_closed_hatch_opens_it_and_reveals_the_room_behind() {
    let state = above_the_hold();
    assert_eq!(state.hatches[usize::from(INTO_HOLD.0)], HatchState::Closed);
    assert!(state.visited(AIRLOCK));
    assert!(!state.visited(CARGO_HOLD), "fogged until the hatch opens");

    let (state, events) = walk_into_the_hold();
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, Event::HatchOpened { .. }))
            .collect::<Vec<_>>(),
        [&Event::HatchOpened { hatch: INTO_HOLD }]
    );
    assert!(state.visited(CARGO_HOLD));
    assert!(!state.visited(ENGINE_ROOM), "still behind a closed hatch");
}

#[test]
fn a_combat_room_seals_once_a_player_is_wholly_inside_and_spawns_its_base_wave() {
    let (state, _) = walk_into_the_hold();
    assert_eq!(
        state.run,
        Run::Encounter {
            room: CARGO_HOLD,
            wave: 0
        }
    );
    // Sealed on the first tick the body cleared the hatch.
    let top = pos(&state).y.saturating_sub(PLAYER_RADIUS);
    assert!(top >= HOLD_TOP && top < HOLD_TOP.saturating_add(Fx::from_num(4)));
    for hatch in [INTO_HOLD, OUT_OF_HOLD] {
        assert_eq!(state.hatches[usize::from(hatch.0)], HatchState::Sealed);
    }
    assert_eq!(state.enemies.len(), 3, "base layer");
    assert!(
        state.enemies.iter().all(|(_, e)| !e.active()),
        "telegraphing"
    );
}

#[test]
fn sealed_hatches_stop_players_and_bullets() {
    let (mut state, _) = walk_into_the_hold();
    run(&mut state, 30, &walk(NORTH));
    assert_eq!(
        pos(&state).y,
        HOLD_TOP.saturating_add(PLAYER_RADIUS),
        "flush with the sealed hatch"
    );

    let mut fire_up = TickInputs::default();
    fire_up.players[0] = PlayerInput {
        aim: UP,
        buttons: Buttons::FIRE,
        ..PlayerInput::default()
    };
    let events = run(&mut state, 1, &fire_up);
    assert!(events.contains(&Event::ShotFired { slot: 0 }));
    run(&mut state, 2, &TickInputs::default());
    assert!(state.bullets.is_empty(), "the sealed hatch ate it");
}

#[test]
fn waves_advance_on_clear_then_the_hatches_unseal_and_the_room_stays_cleared() {
    let (mut state, _) = walk_into_the_hold();

    state.enemies.retain(|_, _| false);
    let events = run(&mut state, 1, &TickInputs::default());
    assert!(
        events.contains(&Event::WaveStarted { wave: 1 }),
        "{events:?}"
    );
    assert_eq!(
        state.run,
        Run::Encounter {
            room: CARGO_HOLD,
            wave: 1
        }
    );
    assert_eq!(state.enemies.len(), 4, "reinforcement layer");
    assert!(
        state.enemies.iter().all(|(_, e)| !e.active()),
        "waves arrive in the spawn telegraph"
    );

    state.enemies.retain(|_, _| false);
    let events = run(&mut state, 1, &TickInputs::default());
    assert!(events.contains(&Event::RoomCleared { room: CARGO_HOLD }));
    assert_eq!(state.run, Run::Boarding);
    assert_eq!(state.hatches[usize::from(INTO_HOLD.0)], HatchState::Open);
    assert_eq!(
        state.hatches[usize::from(OUT_OF_HOLD.0)],
        HatchState::Closed,
        "an unexplored side stays shut"
    );
    assert!(!state.visited(ENGINE_ROOM), "still fogged until touched");

    // Open: walk back up into the airlock, then down into a quiet hold.
    run(&mut state, 40, &walk(NORTH));
    assert_eq!(state.ship.room_at(pos(&state)), Some(AIRLOCK));
    run(&mut state, 60, &walk(SOUTH));
    assert_eq!(state.ship.room_at(pos(&state)), Some(CARGO_HOLD));
    assert_eq!(state.run, Run::Boarding);
    assert!(state.enemies.is_empty());
}

#[test]
fn an_enemy_never_sees_through_a_closed_hatch_nor_leaves_its_room_through_an_open_one() {
    // Cleared, so standing in the hold starts no fight; the player waits in the airlock
    // two cells above the hatch, a hunting-range rusher four cells below it.
    let mut state = above_the_hold();
    state.cleared = 1 << CARGO_HOLD.0;
    let player = pos(&state);
    let rusher = state.enemies.insert(Enemy {
        spawn_ticks: 0,
        ..Enemy::rusher(FxVec2 {
            y: HOLD_TOP.saturating_add(CELL.saturating_mul_int(4)),
            ..player
        })
    });
    let events = run(&mut state, 120, &TickInputs::default());
    assert!(events.is_empty(), "{events:?}");
    assert_eq!(
        state.enemies.get(rusher).unwrap().awareness,
        Awareness::Unaware
    );

    // Open, the hatch lets its sight through, and it charges, but only up to the hatch.
    state.hatches[usize::from(INTO_HOLD.0)] = HatchState::Open;
    let events = run(&mut state, 300, &TickInputs::default());
    assert_eq!(events, [Event::EnemyAlerted { enemy: rusher }]);
    let at = state.enemies.get(rusher).unwrap().pos;
    assert_eq!(state.ship.inside(at, ENEMY_RADIUS), Some(CARGO_HOLD));
    assert_eq!(
        at.y,
        HOLD_TOP.saturating_add(ENEMY_RADIUS),
        "pressed to the hatch"
    );
    assert_eq!(state.players[0].unwrap().hp, MAX_HP);
}

#[test]
fn bullets_fly_over_pits_and_stop_at_walls() {
    // The airlock's pit is at floor cells (11..=12, 3), its west wall at x = 256..288.
    let mut state = SimState::new(SEED, RunConfig::default());
    place(&mut state, cell_center(14, 3));
    let mut fire_left = TickInputs::default();
    fire_left.players[0] = PlayerInput {
        aim: LEFT,
        buttons: Buttons::FIRE,
        ..PlayerInput::default()
    };
    run(&mut state, 1, &fire_left);
    run(&mut state, 7, &TickInputs::default());
    let past_pit = CELL.saturating_mul_int(11);
    let wall = CELL.saturating_mul_int(9);
    let bullet = state.bullets.iter().next().map(|(_, b)| b.pos.x);
    assert!(
        bullet.is_some_and(|x| x < past_pit && x > wall),
        "{bullet:?}"
    );
    run(&mut state, 4, &TickInputs::default());
    assert!(
        state.bullets.is_empty(),
        "stopped by the wall, not lifetime"
    );
}
