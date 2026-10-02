//! Enemy awareness: room enemies stand unaware until they see (in plain view inside their
//! sight cone, at any range), hear or are hit by a player, then hunt it, turning at a
//! finite rate, and give up where they lost it. Unaware, they patrol near where they
//! spawned, and go to look when a hatch bangs open or shut nearby.
//! All in the cargo hold (the Corvette's midship room at `SEED`), 24 x 14 cells: an L
//! (void top-right) with a pillar at cells (5..=6, 3..=4), a pit strip at
//! (10..=13, 9..=10), and hatches north (11..=12, 0), south (11..=12, 13), west (0, 6..=7)
//! and east (23, 6..=7). Cells here are the hold's own.

use sim::room::{Dir, cell_center};
use sim::{
    Awareness, Behavior, Buttons, ENEMY_RADIUS, Enemy, EnemyId, Event, Fx, FxVec2, HatchId,
    PlayerInput, RoomId, Run, RunConfig, SimState, TickInputs, Tuning, step,
};

const SEED: u64 = 3229;
const CARGO_HOLD: RoomId = RoomId(4);
/// The hold's cell (0, 0) is this floor cell.
const HOLD_AT: (usize, usize) = (46, 13);
/// Facings, `u16` turns.
const RIGHT: u16 = 0;
const DOWN: u16 = 16384;
const LEFT: u16 = 32768;

/// The center of the hold's cell (`x`, `y`).
fn at((x, y): (usize, usize)) -> FxVec2 {
    cell_center(x.saturating_add(HOLD_AT.0), y.saturating_add(HOLD_AT.1))
}

/// A fresh run at `SEED`, checked to have the hold these tests are laid out for.
fn fresh() -> SimState {
    let state = SimState::new(SEED, RunConfig::default());
    let hold = state.ship.room(CARGO_HOLD).map(|r| (r.room.name, r.at));
    assert_eq!(hold, Some(("cargo hold", HOLD_AT)), "SEED's midship room");
    state
}

/// The hold's hatch on its `side`, and the middle of its gap: where its bang is heard
/// from, and where investigators go.
fn hatch(state: &SimState, side: Dir) -> Option<(HatchId, FxVec2)> {
    let ship = &state.ship;
    let found = ship.hatches_of(CARGO_HOLD).find_map(|id| {
        let gap = ship.hatches().get(usize::from(id.0))?.gap;
        (gap.dir == side).then_some((id, gap))
    });
    let (id, gap) = found?;
    let (first, last) = (gap.cell(0), gap.cell(gap.width.saturating_sub(1)));
    let (a, b) = (cell_center(first.0, first.1), cell_center(last.0, last.1));
    let two = Fx::from_num(2);
    let middle = FxVec2 {
        x: a.x.saturating_add(b.x).saturating_div(two),
        y: a.y.saturating_add(b.y).saturating_div(two),
    };
    Some((id, middle))
}

/// The party (slot 0) standing in the cargo hold at cell `at`, no enemies. The hold
/// counts as cleared, so no fight starts.
fn hold(at: (usize, usize)) -> SimState {
    let mut state = fresh();
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
    // 21 cells apart along row 10, far out of earshot, the pit strip in between: pits
    // don't block sight.
    let mut state = hold((1, 10));
    let id = rusher(&mut state, (22, 10), LEFT);
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
    let mut state = hold((1, 10));
    state.config.tuning.patrol_speed = 0;
    let id = rusher(&mut state, (22, 10), RIGHT);
    let events = idle(&mut state, 600);
    assert!(events.is_empty(), "{events:?}");
    assert_eq!(enemy(&state, id).unwrap().awareness, Awareness::Unaware);
}

#[test]
fn a_full_circle_sight_cone_sees_behind_too() {
    let mut state = hold((1, 10));
    state.config.tuning.sight_half_angle = 180;
    let id = rusher(&mut state, (22, 10), RIGHT);
    let events = idle(&mut state, 1);
    assert_eq!(events, [Event::EnemyAlerted { enemy: id }]);
}

#[test]
fn a_hit_from_behind_alerts_an_enemy_and_turns_it_toward_the_shooter() {
    // 7 cells apart, out of earshot (5): only the bullet can alert it.
    let mut state = hold((1, 10));
    let id = rusher(&mut state, (8, 10), RIGHT);
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
    let mut state = hold((20, 10));
    state.config.tuning.rusher_speed = 30;
    state.config.tuning.turn_rate = turn_rate;
    let id = rusher(&mut state, (18, 10), RIGHT);
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
    let start = player_pos(&fresh());
    let mut state = hold((16, 12));
    let id = rusher(&mut state, (22, 12), LEFT);
    idle(&mut state, 1);
    let last_seen = player_pos(&state);
    // Then out of sight, for good: back where it started, in the airlock behind its
    // closed hatch (anywhere in the small hold, the rusher could patrol into view).
    put_player(&mut state, start);

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

/// Where [`patrolled`] puts its rusher: a couple of cells up and left of the pit strip,
/// in patrol range of it.
const PATROL_HOME: (usize, usize) = (9, 7);

/// An unaware rusher at [`PATROL_HOME`] under `patrol_speed`; the party stays in the
/// start room, out of sight behind closed hatches. Returns the rusher as of each tick
/// for 20 s, first checking it stayed unaware and wholly in the hold, off the pits.
fn patrolled(patrol_speed: u16) -> Option<Vec<Enemy>> {
    let mut state = fresh();
    state.config.tuning.patrol_speed = patrol_speed;
    let id = rusher(&mut state, PATROL_HOME, RIGHT);
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
    let spawn = at(PATROL_HOME);
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
            .all(|e| e.pos == at(PATROL_HOME) && e.facing == RIGHT)
    );
}

/// Below the hold's north hatch (whose gap is hold cells (11..=12, 0)): 8 cells down,
/// within its 10-cell earshot.
const NEAR_HATCH: (usize, usize) = (11, 8);

/// The party steps into the hold's closed north hatch from the passage above it for a
/// tick, banging it open, then slips back to where it started in the airlock, out of
/// sight behind the airlock's own closed hatch. The hold counts as cleared, so no fight
/// starts. Returns that tick's events.
fn bang_the_hatch_open(state: &mut SimState) -> Vec<Event> {
    state.cleared = 1 << CARGO_HOLD.0;
    let start = player_pos(state);
    let spot = hatch(state, Dir::North).map(|(_, spot)| spot);
    put_player(state, spot.unwrap_or_default());
    let events = idle(state, 1);
    put_player(state, start);
    events
}

#[test]
fn a_hatch_banging_open_draws_an_unaware_enemy_in_earshot_to_look_without_alerting_it() {
    let mut state = fresh();
    let (into_hold, spot) = hatch(&state, Dir::North).unwrap();
    // Looking away from the hatch.
    let id = rusher(&mut state, NEAR_HATCH, DOWN);
    let events = bang_the_hatch_open(&mut state);
    assert_eq!(
        events,
        [
            Event::HatchOpened { hatch: into_hold },
            Event::EnemyInvestigating { enemy: id }
        ]
    );
    // Walking there, till it starts looking around.
    for _ in 0..600 {
        let events = idle(&mut state, 1);
        assert!(events.is_empty(), "never alerted: {events:?}");
        let e = enemy(&state, id).unwrap();
        match e.awareness {
            Awareness::Investigating { spot: to, looking } => {
                assert_eq!(to, spot);
                if looking > 0 {
                    assert!(cells_apart(e.pos, spot) < 2, "went to the hatch");
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
    let mut state = fresh();
    let home = at(NEAR_HATCH);
    let id = rusher(&mut state, NEAR_HATCH, DOWN);
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
    let mut state = fresh();
    let id = rusher(&mut state, NEAR_HATCH, DOWN);
    bang_the_hatch_open(&mut state);
    // The player drops into the hold just inside the hatch: behind the rusher as it
    // stands, but where it is headed.
    move_player(&mut state, (12, 2));
    let mut events = Vec::new();
    for _ in 0..120 {
        events.extend(idle(&mut state, 1));
        if enemy(&state, id).unwrap().hunting() {
            break;
        }
    }
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
    let mut state = fresh();
    state.config.tuning.patrol_speed = 0;
    let near = rusher(&mut state, NEAR_HATCH, DOWN);
    // 11 cells from the hatch, and 3 from its ally in plain view (across the pits): close
    // enough to be alerted by a hunter, but an investigator isn't one.
    let far = rusher(&mut state, (11, 11), DOWN);
    let events = bang_the_hatch_open(&mut state);
    assert!(events.contains(&Event::EnemyInvestigating { enemy: near }));
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
fn entering_a_room_seals_it_with_a_bang_that_draws_spawns_near_the_hatches() {
    // Two cells above the north hatch in the passage from the airlock, walking down into
    // the hold's fight.
    let mut state = fresh();
    let (_, north) = hatch(&state, Dir::North).unwrap();
    let (_, west) = hatch(&state, Dir::West).unwrap();
    let (_, east) = hatch(&state, Dir::East).unwrap();
    put_player(
        &mut state,
        FxVec2 {
            y: north.y.saturating_sub(Fx::from_num(64)),
            ..north
        },
    );
    let mut south = TickInputs::default();
    south.players[0] = PlayerInput {
        move_dir: 8,
        move_mag: u8::MAX,
        ..PlayerInput::default()
    };
    let mut sealed = Vec::new();
    for _ in 0..120 {
        if state.run != Run::Boarding {
            break;
        }
        sealed = step(&mut state, &south).events;
    }
    assert!(matches!(state.run, Run::Encounter { .. }));
    // Sealing bangs all three of the hold's hatches, and each spawn heard the nearest:
    // the shooter, top-left, the west one; the rushers, on the right, the east one.
    let heard: Vec<_> = state
        .enemies
        .iter()
        .map(|(_, e)| match e.behavior {
            Behavior::Shooter { .. } => (e.awareness, west),
            Behavior::Rusher { .. } => (e.awareness, east),
        })
        .collect();
    assert_eq!(heard.len(), 3);
    for (awareness, spot) in heard {
        assert_eq!(awareness, Awareness::Investigating { spot, looking: 0 });
    }
    // First-wave spawns have no telegraph: their "?"s show on the sealing tick itself.
    let investigating: Vec<_> = state
        .enemies
        .iter()
        .map(|(enemy, _)| Event::EnemyInvestigating { enemy })
        .collect();
    assert_eq!(sealed, investigating);
}
