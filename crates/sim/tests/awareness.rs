//! Enemy awareness: room enemies stand unaware until they see (on the player's screen,
//! in plain view), hear or are hit by a player, then hunt it, and give up where they lost
//! it. All in the cargo hold, 32 x 16 cells: an L (void top-right) with a pillar at cells
//! (5..=6, 3..=4) and a pit strip at (10..=13, 10..=11).

use sim::camera::View;
use sim::room::cell_center;
use sim::{
    Awareness, Buttons, Enemy, EnemyId, Event, Fx, FxVec2, Pattern, PlayerInput, RoomId, Run,
    RunConfig, SimState, TickInputs, Tuning, step,
};

const SEED: u64 = 5;
const CARGO_HOLD: RoomId = RoomId(1);
/// A phone held upright: the hold's whole height fits on screen, but only 12 cells across.
/// (No view = the default landscape one, 852 x 393 pt.)
const PORTRAIT: View = View {
    width: 393,
    height: 852,
};

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

/// Slot 0 stands still for `ticks`, looking through `view`.
fn watch(state: &mut SimState, view: View, ticks: usize) -> Vec<Event> {
    let mut inputs = TickInputs::default();
    inputs.players[0].view = view;
    (0..ticks)
        .flat_map(|_| step(state, &inputs).events)
        .collect()
}

fn idle(state: &mut SimState, ticks: usize) -> Vec<Event> {
    watch(state, View::default(), ticks)
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
fn unaware_enemies_ignore_an_idle_player_off_screen_or_behind_a_wall() {
    let mut state = hold((6, 1));
    // On screen, 4 cells down, but behind the pillar.
    let hidden = rusher(&mut state, (6, 5));
    // In plain view (past the pillar, across the pit), but below the bottom of a
    // landscape screen.
    let below = state.enemies.insert(Enemy {
        spawn_ticks: 0,
        ..Enemy::shooter(cell_center(12, 14), Pattern::Aimed, &state.config, 0)
    });
    let before = state.enemies.clone();
    let events = idle(&mut state, 600);
    assert!(events.is_empty(), "{events:?}");
    assert_eq!(state.enemies, before, "nobody moved");
    assert_eq!(enemy(&state, hidden).unwrap().awareness, Awareness::Unaware);
    assert_eq!(enemy(&state, below).unwrap().awareness, Awareness::Unaware);
    assert!(
        state.enemy_bullets.is_empty(),
        "shooters only fire when aware"
    );
    // Turned upright, the screen shows the whole height: the shooter is on it now.
    let events = watch(&mut state, PORTRAIT, 1);
    assert_eq!(events, [Event::EnemyAlerted { enemy: below }]);
    assert_eq!(enemy(&state, hidden).unwrap().awareness, Awareness::Unaware);
}

#[test]
fn an_enemy_on_screen_notices_a_player_in_plain_view_across_a_pit() {
    let mut state = hold((11, 14));
    // 9 cells straight up, the pit strip in between: pits don't block sight.
    let id = rusher(&mut state, (11, 5));
    let events = watch(&mut state, PORTRAIT, 1);
    assert_eq!(events, [Event::EnemyAlerted { enemy: id }]);
    assert_eq!(
        enemy(&state, id).unwrap().awareness,
        Awareness::Alert {
            last_seen: player_pos(&state),
            searching: 0
        }
    );
    // It comes round the pit for the player.
    let start = cells_apart(enemy(&state, id).unwrap().pos, player_pos(&state));
    watch(&mut state, PORTRAIT, 60);
    let now = cells_apart(enemy(&state, id).unwrap().pos, player_pos(&state));
    assert!(now < start, "{now} cells, from {start}");
}

#[test]
fn a_shot_alerts_enemies_in_earshot_and_the_enemy_it_hits() {
    let mut state = hold((6, 1));
    // Behind the pillar but within earshot (4 cells).
    let heard = rusher(&mut state, (6, 5));
    // Straight down the line of fire, 10 cells off: out of earshot, and off the side of
    // a portrait screen.
    let hit = rusher(&mut state, (16, 1));
    let mut fire = TickInputs::default();
    fire.players[0] = PlayerInput {
        aim: 0, // straight right
        buttons: Buttons::FIRE,
        view: PORTRAIT,
        ..PlayerInput::default()
    };
    let events = step(&mut state, &fire).events;
    assert_eq!(
        events,
        [
            Event::ShotFired { slot: 0 },
            Event::EnemyAlerted { enemy: heard }
        ]
    );
    let events = watch(&mut state, PORTRAIT, 30);
    assert_eq!(
        events,
        [
            Event::EnemyHit { enemy: hit },
            Event::EnemyAlerted { enemy: hit }
        ]
    );
    for id in [heard, hit] {
        assert!(matches!(
            enemy(&state, id).unwrap().awareness,
            Awareness::Alert { .. }
        ));
    }
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
