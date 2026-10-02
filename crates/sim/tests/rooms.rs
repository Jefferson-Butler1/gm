//! Rooms through the public API: format validation, placing rooms in one floor grid,
//! walls and pits, walking through hatches with no cut, fog, seal on entry, waves, open
//! on clear (its Scrap flying to the party), enemies kept in their room, access panels,
//! and chests.

use sim::pickup::first_share;
use sim::room::{
    CELL, Category, Connection, Derelict, DerelictError, Dir, EnemyKind, Exit, ExitKind, ExitRef,
    Placed, Placement, PrototypeRoom, RoomAction, RoomError, RoomTrigger, Theme, cell_center,
};
use sim::{
    Awareness, Buttons, CORVETTE, ENEMY_RADIUS, Enemy, Event, Fx, FxVec2, HatchId, HatchKind,
    HatchState, MAX_HP, PLAYER_RADIUS, POOL, Pickup, PickupKind, PlayerInput, RoomId, Run,
    RunConfig, SimState, TickInputs, Tuning, per_tick, reveal, step,
};

const SEED: u64 = 57;
const LEFT: u16 = 32768;
const UP: u16 = 49152;
/// Move buckets (32 per turn).
const EAST: u8 = 0;
const SOUTH: u8 = 8;
const NORTH: u8 = 24;

// --- validation ---------------------------------------------------------------------

/// A valid 5 x 5 room with an east exit and one rusher; tests break one thing at a time.
const BOX: PrototypeRoom = PrototypeRoom {
    name: "box",
    category: Category::Normal,
    theme: Theme::Cargo,
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
fn the_pool_the_corvette_and_the_test_box_are_valid() {
    assert!(POOL.iter().all(|room| room.validate().is_ok()));
    assert_eq!(CORVETTE.validate(), Ok(()));
    assert_eq!(BOX.validate(), Ok(()));
}

/// Entering a room, nothing spawns on top of you: every pool room's spawns, base and
/// reinforcements, are at least 3 cells (either axis) from each of its hatch points.
#[test]
fn no_pool_room_spawns_an_enemy_closer_than_3_cells_to_a_hatch() {
    let near: Vec<_> = POOL
        .iter()
        .flat_map(|room| {
            let layers = room.reinforcements.iter().map(|layer| layer.placements);
            let spawns = std::iter::once(room.base).chain(layers).flatten();
            spawns.filter_map(move |p| {
                let close = room.exits.iter().any(|exit| {
                    (0..exit.width).any(|i| {
                        let (x, y) = exit.cell(i);
                        p.x.abs_diff(x) < 3 && p.y.abs_diff(y) < 3
                    })
                });
                close.then_some((room.name, p.x, p.y))
            })
        })
        .collect();
    assert!(near.is_empty(), "spawns by a hatch: {near:?}");
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
        ..BOX
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

// --- play ---------------------------------------------------------------------------

// The Corvette at `SEED`, boarded through the port airlock. Rooms are its filled slots in
// legend order, then corridors; hatches go by slot, then side (north, east, south, west),
// then the airlocks' outer hatches.
const AIRLOCK: RoomId = RoomId(0);
const ENGINE_ROOM: RoomId = RoomId(3);
/// The midship slot, whose room at `SEED` is the cargo hold, at floor cell (46, 12).
const CARGO_HOLD: RoomId = RoomId(4);
/// The cargo hold's hatches: up to the passage from the port airlock (floor cells 57..=58
/// of row 12), and east to the passage to the bridge.
const INTO_HOLD: HatchId = HatchId(10);
const OUT_OF_HOLD: HatchId = HatchId(11);
/// The hold's first row: its top edge is the hatch's bottom edge.
const HOLD_TOP: Fx = CELL.saturating_mul_int(13);

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

/// A fresh run with slot 0 in the passage down from the airlock, lined up over the hatch
/// into the cargo hold, two cells above it.
fn above_the_hold() -> SimState {
    let mut state = SimState::new(SEED, RunConfig::default());
    let hold = state.ship.room(CARGO_HOLD).map(|r| r.room.name);
    assert_eq!(hold, Some("cargo hold"), "SEED's midship room");
    assert_eq!(state.ship.start().0, AIRLOCK, "SEED's boarding airlock");
    place(
        &mut state,
        FxVec2 {
            x: CELL.saturating_mul_int(58), // centered on the 2-cell gap
            y: cell_center(0, 10).y,
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
        state.enemies.iter().all(|(_, e)| e.active()),
        "prespawns skip the telegraph"
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

    // Open: walk back up the passage into the airlock, then down into a quiet hold.
    run(&mut state, 60, &walk(NORTH));
    assert_eq!(state.ship.room_at(pos(&state)), Some(AIRLOCK));
    run(&mut state, 60, &walk(SOUTH));
    assert_eq!(state.ship.room_at(pos(&state)), Some(CARGO_HOLD));
    assert_eq!(state.run, Run::Boarding);
    assert!(state.enemies.is_empty());
}

#[test]
fn spawns_carry_their_scrap_share_and_it_flies_to_the_party_once_the_room_unseals() {
    let (mut state, _) = walk_into_the_hold();
    let first = first_share(&state.ship, CARGO_HOLD, 0);
    let carried: Vec<u8> = state.enemies.iter().map(|(_, e)| e.scrap).collect();
    assert_eq!(
        state.scrap_shares.get(first..first + carried.len()),
        Some(carried.as_slice())
    );

    // Scrap in the hold's far corner from the player, as if dropped in the fight.
    let corner = state.ship.room(CARGO_HOLD).map(|r| r.at).unwrap();
    let at = cell_center(corner.0 + 2, corner.1 + 11);
    state.pickups.insert(Pickup {
        kind: PickupKind::Scrap(5),
        pos: at,
        magnet: false,
    });
    state.enemies.retain(|_, _| false);
    run(&mut state, 1, &TickInputs::default());
    assert_eq!(
        state.run,
        Run::Encounter {
            room: CARGO_HOLD,
            wave: 1
        }
    );
    let lying = |state: &SimState| state.pickups.iter().all(|(_, p)| p.pos == at && !p.magnet);
    assert!(lying(&state), "it waits out the fight");

    state.enemies.retain(|_, _| false);
    let events = run(&mut state, 1, &TickInputs::default());
    assert!(events.contains(&Event::RoomCleared { room: CARGO_HOLD }));
    assert!(state.pickups.iter().all(|(_, p)| p.magnet));
    let events = run(&mut state, 90, &TickInputs::default());
    assert_eq!(
        events,
        [Event::ScrapCollected { slot: 0, value: 5 }],
        "it came to the player standing still"
    );
    assert_eq!((state.scrap, state.pickups.len()), (5, 0));
}

#[test]
fn an_enemy_never_sees_through_a_closed_hatch_nor_leaves_its_room_through_an_open_one() {
    // Cleared, so standing in the hold starts no fight; the player waits in the passage
    // two cells above the hatch, a hunting-range rusher four cells below it, standing
    // still (no patrol) looking straight at the hatch.
    let mut state = above_the_hold();
    state.cleared = 1 << CARGO_HOLD.0;
    state.config.tuning.patrol_speed = 0;
    let player = pos(&state);
    let rusher = state.enemies.insert(Enemy {
        spawn_ticks: 0,
        facing: UP,
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
fn auto_aim_prefers_an_enemy_in_line_of_fire_over_a_nearer_one_behind_a_pillar() {
    // The cargo hold's pillar covers its cells (5..=6, 3..=4).
    let mut state = SimState::new(SEED, RunConfig::default());
    let (hx, hy) = state.ship.room(CARGO_HOLD).map(|r| r.at).unwrap();
    let cell = |x: usize, y: usize| cell_center(hx.saturating_add(x), hy.saturating_add(y));
    state.cleared |= 1 << CARGO_HOLD.0;
    place(&mut state, cell(3, 3));
    for (x, y) in [(8, 3), (3, 10)] {
        state.enemies.insert(Enemy {
            spawn_ticks: 0,
            ..Enemy::rusher(cell(x, y))
        });
    }
    let mut auto = TickInputs::default();
    auto.players[0] = PlayerInput {
        buttons: Buttons::FIRE | Buttons::AUTO_AIM,
        ..PlayerInput::default()
    };
    run(&mut state, 1, &auto);
    // The one 5 cells right is behind the pillar: it shoots the one 7 cells down.
    assert_eq!(state.players[0].unwrap().facing, 16384);
}

#[test]
fn bullets_fly_over_pits_and_stop_at_walls() {
    // The cargo hold's pit strip is at its cells (10..=13, 9..=10), its west wall at x = 0.
    let mut state = SimState::new(SEED, RunConfig::default());
    let (hx, hy) = state.ship.room(CARGO_HOLD).map(|r| r.at).unwrap();
    state.cleared |= 1 << CARGO_HOLD.0;
    place(
        &mut state,
        cell_center(hx.saturating_add(16), hy.saturating_add(9)),
    );
    let mut fire_left = TickInputs::default();
    fire_left.players[0] = PlayerInput {
        aim: LEFT,
        buttons: Buttons::FIRE,
        ..PlayerInput::default()
    };
    run(&mut state, 1, &fire_left);
    let wall = CELL.saturating_mul_int(i64::try_from(hx.saturating_add(1)).unwrap());
    let pit_start = CELL.saturating_mul_int(i64::try_from(hx.saturating_add(10)).unwrap());
    let mut over_pit = false;
    for _ in 0..60 {
        run(&mut state, 1, &TickInputs::default());
        let Some((_, b)) = state.bullets.iter().next() else {
            break;
        };
        assert!(b.pos.x > wall, "through the wall: {:?}", b.pos.x);
        over_pit |= b.pos.x < pit_start;
    }
    assert!(over_pit, "flew on past the pit");
    assert!(
        state.bullets.is_empty(),
        "stopped by the wall, not lifetime"
    );
}

/// The crawlspace's access panel, and the middle of the dead-end passage below it (floor
/// cells 22..=23, 11..=12, off the engine room's north hatch).
fn at_the_panel() -> (SimState, HatchId) {
    let mut state = SimState::new(SEED, RunConfig::default());
    let panel = (state.ship.hatches().iter())
        .position(|h| h.kind == HatchKind::Panel)
        .and_then(|i| u16::try_from(i).ok())
        .map_or(HatchId(u16::MAX), HatchId);
    place(
        &mut state,
        FxVec2 {
            x: CELL.saturating_mul_int(23), // centered on the 2-cell gap
            y: cell_center(0, 12).y,
        },
    );
    (state, panel)
}

#[test]
fn an_access_panel_stops_everything_until_a_shot_reveals_it_then_it_opens_like_a_hatch() {
    let (mut state, panel) = at_the_panel();
    let crawlspace = state.ship.hatches()[usize::from(panel.0)].rooms[0];
    assert_eq!(
        state.ship.room(crawlspace).map(|r| r.room.category),
        Some(Category::Secret)
    );
    assert_eq!(state.hatches[usize::from(panel.0)], HatchState::Panel);
    let before = pos(&state);
    run(&mut state, 30, &walk(NORTH));
    assert_eq!(
        pos(&state).y,
        CELL.saturating_mul_int(11).saturating_add(PLAYER_RADIUS)
    );
    assert!(pos(&state).y < before.y, "walked up to it, and no further");

    let mut fire_up = TickInputs::default();
    fire_up.players[0] = PlayerInput {
        aim: UP,
        buttons: Buttons::FIRE,
        ..PlayerInput::default()
    };
    run(&mut state, 1, &fire_up);
    run(&mut state, 3, &TickInputs::default());
    assert!(state.bullets.is_empty());
    assert_eq!(state.hatches[usize::from(panel.0)], HatchState::Closed);
    assert!(!state.visited(crawlspace), "still fogged until touched");

    let events = run(&mut state, 30, &walk(NORTH));
    assert!(
        events.contains(&Event::HatchOpened { hatch: panel }),
        "{events:?}"
    );
    assert!(state.visited(crawlspace));
}

#[test]
fn reveal_turns_only_a_panel_closed() {
    let (mut state, panel) = at_the_panel();
    assert!(reveal(&mut state, panel));
    assert_eq!(state.hatches[usize::from(panel.0)], HatchState::Closed);
    assert!(!reveal(&mut state, panel), "already revealed");
    let plain = HatchId(0);
    assert!(!reveal(&mut state, plain));
    assert_eq!(state.hatches[usize::from(plain.0)], HatchState::Closed);
}

#[test]
fn touching_a_chest_opens_it_once() {
    let mut state = SimState::new(SEED, RunConfig::default());
    let stores = (0..)
        .map(RoomId)
        .zip(state.ship.rooms())
        .find(|(_, r)| r.room.category == Category::Reward)
        .map(|(id, _)| id)
        .unwrap();
    let (x, y) = state.ship.chest(stores).unwrap();
    // Standing just west of it, then walking onto it.
    place(&mut state, cell_center(x.saturating_sub(1), y));
    assert!(run(&mut state, 1, &TickInputs::default()).is_empty());
    let events = run(&mut state, 30, &walk(EAST));
    assert_eq!(events, [Event::ChestOpened { room: stores }]);
    assert!(state.chest_opened(stores));
    place(&mut state, cell_center(x, y));
    assert!(run(&mut state, 10, &TickInputs::default()).is_empty());
}
