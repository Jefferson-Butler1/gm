//! Enemy awareness: room enemies stand unaware until they see (in plain view inside their
//! sight cone, at any range), hear or are hit by a player, then hunt it, turning at a
//! finite rate, and give up where they lost it. Unaware, they patrol near where they
//! spawned, and go to look when a hatch bangs open or shut nearby.
//! All in the cargo hold, 32 x 16 cells: an L (void top-right) with a pillar at cells
//! (5..=6, 3..=4) and a pit strip at (10..=13, 10..=11). Cells here are the hold's own.

use sim::room::cell_center;
use sim::{
    Awareness, Behavior, Buttons, ENEMY_RADIUS, Enemy, EnemyId, Event, Fx, FxVec2, HatchId,
    PlayerInput, RoomId, Run, RunConfig, SPAWN_TELEGRAPH_TICKS, SimState, TickInputs, Tuning, step,
};

const SEED: u64 = 5;
const CARGO_HOLD: RoomId = RoomId(1);
/// Facings, `u16` turns.
const RIGHT: u16 = 0;
const DOWN: u16 = 16384;
const LEFT: u16 = 32768;
/// The hatch down from the airlock into the hold, in the middle of its 2-cell gap (the
/// hold's cells (9..=10, 0)).
const INTO_HOLD: HatchId = HatchId(0);
const HATCH: FxVec2 = FxVec2 {
    x: Fx::from_bits(480 << 32),
    y: Fx::from_bits(304 << 32),
};
/// The airlock's bottom-left floor cell: out of sight of the hold even through the open
/// hatch.
const AIRLOCK_CORNER: FxVec2 = FxVec2 {
    x: Fx::from_bits(304 << 32),
    y: Fx::from_bits(272 << 32),
};

/// The center of the hold's cell (`x`, `y`): its cell (0, 0) is floor cell (5, 9).
fn at((x, y): (usize, usize)) -> FxVec2 {
    cell_center(x.saturating_add(5), y.saturating_add(9))
}

/// The party (slot 0) standing in the cargo hold at cell `at`, no enemies. The hold
/// counts as cleared, so no fight starts.
fn hold(at: (usize, usize)) -> SimState {
    let mut state = SimState::new(SEED, RunConfig::default());
    state.cleared = 1 << CARGO_HOLD.0;
    move_player(&mut state, at);
    state
}

fn move_player(state: &mut SimState, cell: (usize, usize)) {
    put_player(state, at(cell));
}

const fn put_player(state: &mut SimState, pos: FxVec2) {
    if let Some(player) = &mut state.players[0] {
        player.pos = pos;
        player.solid = pos;
    }
}

/// An unaware rusher at cell `cell` looking toward `facing`, past its spawn telegraph.
fn rusher(state: &mut SimState, cell: (usize, usize), facing: u16) -> EnemyId {
    state.enemies.insert(Enemy {
        spawn_ticks: 0,
        facing,
        ..Enemy::rusher(at(cell))
    })
}

/// Slot 0 stands still for `ticks`.
fn idle(state: &mut SimState, ticks: usize) -> Vec<Event> {
    let inputs = TickInputs::default();
    (0..ticks)
        .flat_map(|_| step(state, &inputs).events)
        .collect()
}

fn enemy(state: &SimState, id: EnemyId) -> Option<Enemy> {
    state.enemies.get(id).copied()
}

fn player_pos(state: &SimState) -> FxVec2 {
    state.players[0].map(|p| p.pos).unwrap_or_default()
}

fn cells_apart(a: FxVec2, b: FxVec2) -> Fx {
    let (dx, dy) = (a.x.saturating_sub(b.x), a.y.saturating_sub(b.y));
    dx.saturating_mul(dx)
        .saturating_add(dy.saturating_mul(dy))
        .sqrt()
        .saturating_div(Fx::from_num(32))
}

#[test]
fn an_unaware_enemy_behind_a_wall_ignores_an_idle_player() {
    let mut state = hold((6, 1));
    state.config.tuning.patrol_speed = 0;
    // 4 cells down, behind the pillar.
    let id = rusher(&mut state, (6, 5), LEFT);
    let before = state.enemies.clone();
    let events = idle(&mut state, 600);
    assert!(events.is_empty(), "{events:?}");
    assert_eq!(state.enemies, before, "nobody moved");
    assert_eq!(enemy(&state, id).unwrap().awareness, Awareness::Unaware);
}

#[test]
fn an_enemy_notices_a_player_in_plain_view_in_its_cone_at_any_range_across_a_pit() {
    // 29 cells apart along row 11, past the far side of any phone screen, the pit strip
    // in between: pits don't block sight.
    let mut state = hold((1, 11));
    let id = rusher(&mut state, (30, 11), LEFT);
    let events = idle(&mut state, 1);
    assert_eq!(events, [Event::EnemyAlerted { enemy: id }]);
    assert_eq!(
        enemy(&state, id).unwrap().awareness,
        Awareness::Alert {
            last_seen: player_pos(&state),
            searching: 0
        }
    );
}

#[test]
fn an_unaware_enemy_facing_away_ignores_a_player_in_plain_view_behind_it() {
    let mut state = hold((1, 11));
    state.config.tuning.patrol_speed = 0;
    let id = rusher(&mut state, (30, 11), RIGHT);
    let events = idle(&mut state, 600);
    assert!(events.is_empty(), "{events:?}");
    assert_eq!(enemy(&state, id).unwrap().awareness, Awareness::Unaware);
}

#[test]
fn a_full_circle_sight_cone_sees_behind_too() {
    let mut state = hold((1, 11));
    state.config.tuning.sight_half_angle = 180;
    let id = rusher(&mut state, (30, 11), RIGHT);
    let events = idle(&mut state, 1);
    assert_eq!(events, [Event::EnemyAlerted { enemy: id }]);
}

#[test]
fn a_hit_from_behind_alerts_an_enemy_and_turns_it_toward_the_shooter() {
    // 7 cells apart, out of earshot (5): only the bullet can alert it.
    let mut state = hold((1, 11));
    let id = rusher(&mut state, (8, 11), RIGHT);
    let mut fire = TickInputs::default();
    fire.players[0] = PlayerInput {
        aim: RIGHT,
        buttons: Buttons::FIRE,
        ..PlayerInput::default()
    };
    let mut events = step(&mut state, &fire).events;
    events.extend(idle(&mut state, 30));
    assert_eq!(
        events,
        [
            Event::ShotFired { slot: 0 },
            Event::EnemyHit { enemy: id },
            Event::EnemyAlerted { enemy: id }
        ]
    );
    // A half turn at 150°/s takes 1.2 s.
    idle(&mut state, 72);
    let e = enemy(&state, id).unwrap();
    assert_eq!(e.facing, LEFT);
    assert_eq!(
        e.awareness,
        Awareness::Alert {
            last_seen: player_pos(&state),
            searching: 0
        },
        "sees the shooter"
    );
}

/// A rusher hunting a player 2 cells east of it in open floor, which the player then
/// circles counter-clockwise at 2x the default turn rate (300°/s) for 40 ticks, the
/// rusher slowed to a crawl so the circle stays in the open. `turn_rate` is the run's.
/// Returns the tick the rusher first lost sight of the player (its `last_seen` went
/// stale), if it did.
fn circled(turn_rate: u16) -> Option<usize> {
    let mut state = hold((24, 11));
    state.config.tuning.rusher_speed = 30;
    state.config.tuning.turn_rate = turn_rate;
    let id = rusher(&mut state, (22, 11), RIGHT);
    idle(&mut state, 1);
    let radius = Fx::from_num(64);
    let mut angle = RIGHT;
    for tick in 0..40 {
        // 2x the default 455 turns a tick (150°/s).
        angle = angle.wrapping_sub(910);
        let center = enemy(&state, id).map(|e| e.pos).unwrap_or_default();
        let offset = sim::trig::unit(angle);
        let spot = FxVec2 {
            x: center.x.saturating_add(offset.x.saturating_mul(radius)),
            y: center.y.saturating_add(offset.y.saturating_mul(radius)),
        };
        if let Some(player) = &mut state.players[0] {
            player.pos = spot;
            player.solid = spot;
        }
        idle(&mut state, 1);
        match enemy(&state, id).map(|e| e.awareness) {
            Some(Awareness::Alert { last_seen, .. }) if last_seen == spot => {}
            _ => return Some(tick),
        }
    }
    None
}

#[test]
fn a_player_circling_a_hunter_faster_than_it_turns_slips_out_of_its_cone() {
    // Its cone's edge is 60° off its facing, and the player gains 150°/s on it: gone
    // within about 0.4 s (24 ticks).
    let lost = circled(Tuning::NORMAL.turn_rate).expect("loses the player");
    assert!(lost <= 26, "{lost}");
    // A hunter that turns faster than the player circles keeps it in view.
    assert_eq!(circled(720), None);
}

#[test]
fn a_shot_alerts_an_enemy_in_earshot_behind_a_wall() {
    let mut state = hold((6, 1));
    // Behind the pillar but within earshot (4 cells).
    let id = rusher(&mut state, (6, 5), LEFT);
    let mut fire = TickInputs::default();
    fire.players[0] = PlayerInput {
        aim: 0, // straight right
        buttons: Buttons::FIRE,
        ..PlayerInput::default()
    };
    let events = step(&mut state, &fire).events;
    assert_eq!(
        events,
        [
            Event::ShotFired { slot: 0 },
            Event::EnemyAlerted { enemy: id }
        ]
    );
}

#[test]
fn a_hunter_that_loses_the_player_searches_where_it_last_saw_it_then_gives_up() {
    let forget = Tuning::NORMAL.forget_ticks;
    // 6 cells west of the rusher, in plain view along the bottom strip.
    let mut state = hold((22, 12));
    let id = rusher(&mut state, (28, 12), LEFT);
    idle(&mut state, 1);
    let last_seen = player_pos(&state);
    // Then out of sight: round the corner of the L, behind the void.
    move_player(&mut state, (18, 1));

    let mut arrived = None;
    let mut gave_up = None;
    for tick in 1..600 {
        idle(&mut state, 1);
        let e = enemy(&state, id).unwrap();
        match e.awareness {
            Awareness::Alert { searching, .. } => {
                assert_eq!(
                    e.awareness,
                    Awareness::Alert {
                        last_seen,
                        searching
                    },
                    "never sees the player again"
                );
                if searching == 1 {
                    arrived = Some(tick);
                }
            }
            Awareness::Unaware => {
                gave_up = Some(tick);
                break;
            }
            Awareness::Investigating { .. } => panic!("no hatch banged"),
        }
    }
    let (arrived, gave_up) = (arrived.unwrap(), gave_up.unwrap());
    // Searching from the arrival tick through the one it gives up on.
    assert_eq!(gave_up - arrived + 1, usize::from(forget));
    let spot = enemy(&state, id).unwrap().pos;
    assert!(
        cells_apart(spot, last_seen) < 2,
        "gave up at the last-known spot"
    );
    // Unaware again: it patrols around where it gave up.
    let events = idle(&mut state, 600);
    assert!(events.is_empty(), "{events:?}");
    let now = enemy(&state, id).unwrap().pos;
    assert_ne!(now, spot);
    assert!(cells_apart(now, spot) < 5, "patrols near where it gave up");
}

/// An unaware rusher at the hold's cell (7, 13), a few cells from its south hatch and
/// the pit strip, under `patrol_speed`; the party stays in the start room, out of sight
/// behind a closed hatch. Returns the rusher as of each tick for 20 s, first checking
/// it stayed unaware and wholly in the hold, off the pits.
fn patrolled(patrol_speed: u16) -> Option<Vec<Enemy>> {
    let mut state = SimState::new(SEED, RunConfig::default());
    state.config.tuning.patrol_speed = patrol_speed;
    let id = rusher(&mut state, (7, 13), RIGHT);
    (0..1200)
        .map(|_| {
            idle(&mut state, 1);
            let e = enemy(&state, id)?;
            assert_eq!(e.awareness, Awareness::Unaware);
            assert_eq!(state.ship.inside(e.pos, ENEMY_RADIUS), Some(CARGO_HOLD));
            assert!(!state.tiles().pit_within(e.pos, ENEMY_RADIUS));
            Some(e)
        })
        .collect()
}

#[test]
fn an_unaware_enemy_patrols_away_from_its_spawn_but_stays_in_its_room() {
    let spawn = at((7, 13));
    let ticks = patrolled(Tuning::NORMAL.patrol_speed).unwrap();
    let farthest = ticks
        .iter()
        .map(|e| cells_apart(e.pos, spawn))
        .max()
        .unwrap();
    assert!(farthest >= 1, "wandered off: {farthest}");
    assert!(ticks.iter().any(|e| e.facing != RIGHT), "looked around");
}

#[test]
fn a_patrol_speed_of_zero_keeps_an_unaware_enemy_still() {
    let ticks = patrolled(0).unwrap();
    assert!(
        ticks
            .iter()
            .all(|e| e.pos == at((7, 13)) && e.facing == RIGHT)
    );
}

/// The party steps into the closed hatch from the airlock for a tick, banging it open,
/// then slips back into the airlock's corner, out of sight. The hold counts as cleared,
/// so no fight starts. Returns that tick's events.
fn bang_the_hatch_open(state: &mut SimState) -> Vec<Event> {
    state.cleared = 1 << CARGO_HOLD.0;
    put_player(state, HATCH);
    let events = idle(state, 1);
    put_player(state, AIRLOCK_CORNER);
    events
}

#[test]
fn a_hatch_banging_open_draws_an_unaware_enemy_in_earshot_to_look_without_alerting_it() {
    let mut state = SimState::new(SEED, RunConfig::default());
    // 9 cells below the hatch, looking away from it.
    let id = rusher(&mut state, (9, 9), DOWN);
    let events = bang_the_hatch_open(&mut state);
    assert_eq!(
        events,
        [
            Event::HatchOpened { hatch: INTO_HOLD },
            Event::EnemyInvestigating { enemy: id }
        ]
    );
    // Walking there, till it starts looking around.
    for _ in 0..600 {
        let events = idle(&mut state, 1);
        assert!(events.is_empty(), "never alerted: {events:?}");
        let e = enemy(&state, id).unwrap();
        match e.awareness {
            Awareness::Investigating { spot, looking } => {
                assert_eq!(spot, HATCH);
                if looking > 0 {
                    assert!(cells_apart(e.pos, HATCH) < 2, "went to the hatch");
                    return;
                }
            }
            Awareness::Unaware | Awareness::Alert { .. } => panic!("{:?}", e.awareness),
        }
    }
    panic!("never got there");
}

#[test]
fn an_investigator_that_finds_nobody_goes_back_to_its_patrol() {
    let forget = Tuning::NORMAL.forget_ticks;
    let mut state = SimState::new(SEED, RunConfig::default());
    let home = at((9, 9));
    let id = rusher(&mut state, (9, 9), DOWN);
    bang_the_hatch_open(&mut state);
    let mut arrived = None;
    let mut gave_up = None;
    for tick in 1..1200 {
        idle(&mut state, 1);
        match enemy(&state, id).unwrap().awareness {
            Awareness::Investigating { looking: 1, .. } => arrived = Some(tick),
            Awareness::Investigating { .. } => {}
            Awareness::Unaware => {
                gave_up = Some(tick);
                break;
            }
            Awareness::Alert { .. } => panic!("nobody to see"),
        }
    }
    let (arrived, gave_up) = (arrived.unwrap(), gave_up.unwrap());
    // Looking from the arrival tick through the one it gives up on.
    assert_eq!(gave_up - arrived + 1, usize::from(forget));
    // Back on patrol: it wanders back to around where it started.
    let events = idle(&mut state, 1200);
    assert!(events.is_empty(), "{events:?}");
    let e = enemy(&state, id).unwrap();
    assert_eq!(e.awareness, Awareness::Unaware);
    assert!(cells_apart(e.pos, home) < 5, "patrols near home");
}

#[test]
fn an_investigator_that_spots_the_player_hunts_it() {
    let mut state = SimState::new(SEED, RunConfig::default());
    let id = rusher(&mut state, (9, 9), DOWN);
    bang_the_hatch_open(&mut state);
    // The player drops into the hold just inside the hatch: behind the rusher as it
    // stands, but where it is headed.
    move_player(&mut state, (10, 2));
    let events = idle(&mut state, 120);
    assert_eq!(events, [Event::EnemyAlerted { enemy: id }]);
    assert_eq!(
        enemy(&state, id).unwrap().awareness,
        Awareness::Alert {
            last_seen: player_pos(&state),
            searching: 0
        }
    );
}

#[test]
fn an_enemy_out_of_earshot_of_a_hatch_ignores_it_and_an_investigating_ally() {
    let mut state = SimState::new(SEED, RunConfig::default());
    state.config.tuning.patrol_speed = 0;
    let near = rusher(&mut state, (9, 9), DOWN);
    // 12 cells from the hatch, and 3 from its ally in plain view: close enough to be
    // alerted by a hunter, but an investigator isn't one.
    let far = rusher(&mut state, (9, 12), DOWN);
    let events = bang_the_hatch_open(&mut state);
    assert!(!events.contains(&Event::EnemyInvestigating { enemy: far }));
    // Under the time it spends looking (it looks from where it stands at patrol speed 0).
    let events = idle(&mut state, 120);
    assert!(events.is_empty(), "{events:?}");
    assert!(matches!(
        enemy(&state, near).unwrap().awareness,
        Awareness::Investigating { .. }
    ));
    assert_eq!(enemy(&state, far).unwrap().awareness, Awareness::Unaware);
}

#[test]
fn entering_a_room_seals_it_with_a_bang_that_draws_spawns_near_the_hatch() {
    // Two cells above the hatch in the airlock, walking down into the hold's fight.
    let mut state = SimState::new(SEED, RunConfig::default());
    put_player(
        &mut state,
        FxVec2 {
            y: cell_center(0, 7).y,
            ..HATCH
        },
    );
    let mut south = TickInputs::default();
    south.players[0] = PlayerInput {
        move_dir: 8,
        move_mag: u8::MAX,
        ..PlayerInput::default()
    };
    for _ in 0..120 {
        if state.run != Run::Boarding {
            break;
        }
        step(&mut state, &south);
    }
    assert!(matches!(state.run, Run::Encounter { .. }));
    // The shooter spawns 7 cells from the hatch in; the rushers, across the hold, are
    // out of earshot of both hatches.
    let (shooter, _) = state
        .enemies
        .iter()
        .find(|(_, e)| matches!(e.behavior, Behavior::Shooter { .. }))
        .unwrap();
    for (id, e) in state.enemies.iter() {
        let expected = if id == shooter {
            Awareness::Investigating {
                spot: HATCH,
                looking: 0,
            }
        } else {
            Awareness::Unaware
        };
        assert_eq!(e.awareness, expected);
    }
    // Its "?" shows once its spawn telegraph is over.
    let telegraph = usize::from(SPAWN_TELEGRAPH_TICKS);
    assert_eq!(idle(&mut state, telegraph - 1), []);
    assert_eq!(
        idle(&mut state, 1),
        [Event::EnemyInvestigating { enemy: shooter }]
    );
}
