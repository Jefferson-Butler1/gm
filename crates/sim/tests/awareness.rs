//! Enemy awareness: room enemies stand unaware until they see (in plain view, at any
//! range), hear or are hit by a player, then hunt it, and give up where they lost it.
//! All in the cargo hold, 32 x 16 cells: an L (void top-right) with a pillar at cells
//! (5..=6, 3..=4) and a pit strip at (10..=13, 10..=11).

use sim::room::cell_center;
use sim::{
    Awareness, Buttons, Enemy, EnemyId, Event, Fx, FxVec2, PlayerInput, RoomId, Run, RunConfig,
    SimState, TickInputs, Tuning, step,
};

const SEED: u64 = 5;
const CARGO_HOLD: RoomId = RoomId(1);

/// The party (slot 0) standing in the cargo hold at cell `at`, no enemies, doors open.
fn hold(at: (usize, usize)) -> SimState {
    let mut state = SimState::new(SEED, RunConfig::default());
    state.run = Run::Boarding { room: CARGO_HOLD };
    move_player(&mut state, at);
    state
}

fn move_player(state: &mut SimState, (x, y): (usize, usize)) {
    if let Some(player) = &mut state.players[0] {
        player.pos = cell_center(x, y);
        player.solid = player.pos;
    }
}

/// An unaware rusher at cell (`x`, `y`), past its spawn telegraph.
fn rusher(state: &mut SimState, (x, y): (usize, usize)) -> EnemyId {
    state.enemies.insert(Enemy {
        spawn_ticks: 0,
        ..Enemy::rusher(cell_center(x, y))
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
    // 4 cells down, behind the pillar.
    let id = rusher(&mut state, (6, 5));
    let before = state.enemies.clone();
    let events = idle(&mut state, 600);
    assert!(events.is_empty(), "{events:?}");
    assert_eq!(state.enemies, before, "nobody moved");
    assert_eq!(enemy(&state, id).unwrap().awareness, Awareness::Unaware);
}

#[test]
fn an_enemy_notices_a_player_in_plain_view_at_any_range_across_a_pit() {
    // 29 cells apart along row 11, past the far side of any phone screen, the pit strip
    // in between: pits don't block sight.
    let mut state = hold((1, 11));
    let id = rusher(&mut state, (30, 11));
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
fn a_shot_alerts_an_enemy_in_earshot_behind_a_wall() {
    let mut state = hold((6, 1));
    // Behind the pillar but within earshot (4 cells).
    let id = rusher(&mut state, (6, 5));
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
    let id = rusher(&mut state, (28, 12));
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
    // Unaware again: it stays where it gave up.
    let events = idle(&mut state, 300);
    assert!(events.is_empty(), "{events:?}");
    assert_eq!(enemy(&state, id).unwrap().pos, spot);
}
