//! Combat rules through the public `step`: gun vs rusher, contact damage, i-frames, and
//! death -> restart.

use sim::room::{Body, Tiles, cell_center, cell_of};
use sim::{
    Buttons, DEATH_TICKS, Enemy, Event, Fx, FxVec2, MAX_HP, PlayerInput, RUSHER_HP, RUSHER_RADIUS,
    Rng, RoomId, Run, SPAWN_TELEGRAPH_TICKS, SimState, TickInputs, step,
};

const SEED: u64 = 7;

const DOWN: u16 = 16384;

/// A fresh run: the party alone in the empty start room.
fn empty_arena() -> SimState {
    SimState::new(SEED)
}

/// A point (`x`, `y`) from where the player starts.
fn point(x: i32, y: i32) -> FxVec2 {
    let start = empty_arena().players[0].map(|p| p.pos).unwrap_or_default();
    FxVec2 {
        x: start.x.saturating_add(Fx::from_num(x)),
        y: start.y.saturating_add(Fx::from_num(y)),
    }
}

/// An empty arena plus one rusher `x` points right of the player, already past its
/// spawn telegraph.
fn arena_with_rusher(x: i32) -> SimState {
    let mut state = empty_arena();
    state.enemies.insert(Enemy {
        spawn_ticks: 0,
        ..Enemy::rusher(point(x, 0))
    });
    state
}

fn press(buttons: Buttons) -> TickInputs {
    let mut inputs = TickInputs::default();
    inputs.players[0] = PlayerInput {
        buttons,
        ..PlayerInput::default()
    };
    inputs
}

fn run(state: &mut SimState, ticks: usize, inputs: &TickInputs) -> Vec<Event> {
    (0..ticks)
        .flat_map(|_| step(state, inputs).events)
        .collect()
}

fn count(events: &[Event], f: impl Fn(&Event) -> bool) -> usize {
    events.iter().filter(|e| f(e)).count()
}

#[test]
fn held_fire_kills_a_rusher() {
    let mut state = arena_with_rusher(200);
    let events = run(&mut state, 40, &press(Buttons::FIRE)); // aim 0 = straight right
    let hits = count(&events, |e| matches!(e, Event::EnemyHit { .. }));
    let kills = count(&events, |e| matches!(e, Event::EnemyKilled { .. }));
    assert_eq!((hits, kills), (usize::from(RUSHER_HP), 1), "{events:?}");
    assert!(state.enemies.is_empty());
    assert_eq!(
        state.players[0].unwrap().hp,
        MAX_HP,
        "killed before it arrived"
    );
}

#[test]
fn rusher_contact_hurts_then_post_hit_invulnerability_protects() {
    let mut state = arena_with_rusher(10);
    let events = run(&mut state, 1, &TickInputs::default());
    assert_eq!(events, [Event::PlayerHit { slot: 0 }]);
    // Still in contact, but invulnerable for the rest of the hurt window.
    let events = run(&mut state, 30, &TickInputs::default());
    assert!(events.is_empty(), "{events:?}");
    assert_eq!(state.players[0].unwrap().hp, MAX_HP - 1);
}

#[test]
fn dodge_iframes_block_contact_damage() {
    let mut state = arena_with_rusher(30); // in the roll's path (facing right)
    let mut events = run(&mut state, 1, &press(Buttons::DODGE));
    events.extend(run(
        &mut state,
        usize::from(sim::ROLL_TICKS) - 1,
        &TickInputs::default(),
    ));
    assert!(events.is_empty(), "{events:?}");
    assert_eq!(state.players[0].unwrap().hp, MAX_HP);
}

#[test]
fn death_goes_to_dead_and_restart_starts_a_fresh_run() {
    let mut state = arena_with_rusher(0);
    state.players[0].as_mut().unwrap().hp = 1;
    let events = run(&mut state, 1, &TickInputs::default());
    assert_eq!(
        events,
        [Event::PlayerHit { slot: 0 }, Event::PlayerDied { slot: 0 }]
    );
    assert_eq!(
        state.run,
        Run::Dead {
            room: RoomId(0),
            ticks_until_restart: DEATH_TICKS
        }
    );

    // Restart is ignored during the death pause.
    let restart = press(Buttons::RESTART);
    let events = run(&mut state, usize::try_from(DEATH_TICKS).unwrap(), &restart);
    assert!(events.is_empty(), "{events:?}");

    assert_eq!(run(&mut state, 1, &restart), [Event::Restarted]);
    let mut fresh = SimState::new(Rng::next_seed(SEED));
    fresh.tick = state.tick;
    assert_eq!(state, fresh);
}

fn restart_now(state: &mut SimState) {
    state.run = Run::Dead {
        room: state.run.room().unwrap_or_default(),
        ticks_until_restart: 0,
    };
    assert_eq!(run(state, 1, &press(Buttons::RESTART)), [Event::Restarted]);
}

#[test]
fn restart_abandons_a_live_run_at_once() {
    let mut state = arena_with_rusher(40);
    state.cleared = 0b1;
    assert_eq!(run(&mut state, 1, &press(Buttons::RESTART)), [Event::Restarted]);
    let mut fresh = SimState::new(Rng::next_seed(SEED));
    fresh.tick = state.tick;
    assert_eq!(state, fresh);
}

#[test]
fn each_restart_resets_the_rooms_and_derives_the_next_run_seed() {
    let mut state = SimState::new(SEED);
    // As if the party had cleared the airlock and died in the cargo hold.
    state.cleared = 0b1;
    state.run = Run::Dead {
        room: RoomId(1),
        ticks_until_restart: 0,
    };
    restart_now(&mut state);
    let second_seed = state.seed;
    assert_eq!(second_seed, Rng::next_seed(SEED));
    let mut fresh = SimState::new(second_seed);
    fresh.tick = state.tick;
    assert_eq!(state, fresh, "back in the start room, nothing cleared");
    restart_now(&mut state);
    assert_eq!(state.seed, Rng::next_seed(second_seed));
}

#[test]
fn spawn_telegraph_is_inert_then_the_rusher_engages() {
    let mut state = empty_arena();
    // `near` overlaps the player and would be the auto-aim target; `far` sits in the line
    // of fire to the right (the default facing).
    let near = state.enemies.insert(Enemy::rusher(point(0, 10)));
    let far = state.enemies.insert(Enemy::rusher(point(60, 0)));
    let auto_fire = press(Buttons::FIRE | Buttons::AUTO_AIM);

    let events = run(&mut state, usize::from(SPAWN_TELEGRAPH_TICKS), &auto_fire);
    assert!(
        events.iter().all(|e| matches!(e, Event::ShotFired { .. })),
        "no hits either way while telegraphing: {events:?}"
    );
    assert_eq!(
        state.enemies.get(near).unwrap().pos,
        point(0, 10),
        "didn't move"
    );
    assert_eq!(
        state.enemies.get(far).unwrap().hp,
        RUSHER_HP,
        "bullets passed through"
    );
    let player = state.players[0].unwrap();
    assert_eq!(
        (player.facing, player.hp),
        (0, MAX_HP),
        "not an auto-aim target"
    );
    assert!(state.enemies.iter().all(|(_, e)| e.active()));

    let events = run(&mut state, 1, &auto_fire);
    assert!(events.contains(&Event::PlayerHit { slot: 0 }), "{events:?}");
    assert_eq!(state.players[0].unwrap().facing, DOWN, "auto-aim locks on");
}

/// Rushers touch at twice their radius; allow this much overlap while a crowd pushes.
const OVERLAP_TOLERANCE: i32 = 1;

/// Distance between the closest pair of enemy centers.
fn closest_pair(state: &SimState) -> Fx {
    let pos: Vec<FxVec2> = state.enemies.iter().map(|(_, e)| e.pos).collect();
    let mut closest = Fx::MAX;
    let mut rest = pos.as_slice();
    while let Some((a, tail)) = rest.split_first() {
        for b in tail {
            let (dx, dy) = (a.x.saturating_sub(b.x), a.y.saturating_sub(b.y));
            let d = dx.saturating_mul(dx).saturating_add(dy.saturating_mul(dy));
            closest = closest.min(d.sqrt());
        }
        rest = tail;
    }
    closest
}

/// Active rushers at `spots`, and a player who can't die during the test.
fn rushers_at(spots: impl IntoIterator<Item = FxVec2>) -> SimState {
    let mut state = empty_arena();
    for player in state.players.iter_mut().flatten() {
        player.hp = u8::MAX;
    }
    for spot in spots {
        state.enemies.insert(Enemy {
            spawn_ticks: 0,
            ..Enemy::rusher(spot)
        });
    }
    state
}

#[test]
fn two_stacked_rushers_part_and_never_overlap() {
    let touching = RUSHER_RADIUS.saturating_mul_int(2);
    let min = touching.saturating_sub(Fx::from_num(OVERLAP_TOLERANCE));
    let mut state = rushers_at([point(200, 0); 2]);
    // They walk onto the player at ~60 ticks and press on it for the rest.
    for tick in 0..240 {
        step(&mut state, &TickInputs::default());
        let closest = closest_pair(&state);
        assert!(closest >= min, "tick {tick}: {closest}");
    }
}

#[test]
fn a_crowd_pressing_on_the_player_rings_it_and_holds_still() {
    let touching = RUSHER_RADIUS.saturating_mul_int(2);
    let min = touching.saturating_sub(Fx::from_num(OVERLAP_TOLERANCE));
    // A column of 8, 30 pt apart, 200 pt right of the player and spanning the airlock's
    // height (the player starts in its lower half).
    let column = (-150..).step_by(30).take(8).map(|y| point(200, y));
    let mut state = rushers_at(column);
    let mut last: Vec<FxVec2> = Vec::new();
    for tick in 0..300 {
        step(&mut state, &TickInputs::default());
        let closest = closest_pair(&state);
        assert!(closest >= min, "tick {tick}: {closest}");
        let now: Vec<FxVec2> = state.enemies.iter().map(|(_, e)| e.pos).collect();
        // Settled by 240: nobody moves 1/32 pt a tick (sub-pixel; no visible jitter).
        if tick >= 240 {
            for (a, b) in now.iter().zip(&last) {
                let moved = a.x.abs_diff(b.x).saturating_add(a.y.abs_diff(b.y));
                assert!(moved < Fx::from_bits(1 << 27), "tick {tick}: moved {moved}");
            }
        }
        last = now;
    }
}

/// Whether an enemy's box overlaps any cell that stops walkers.
fn in_blocking_tiles(tiles: Tiles, pos: FxVec2) -> bool {
    let cells = |c: Fx| {
        cell_of(c.saturating_sub(RUSHER_RADIUS))
            ..=cell_of(c.saturating_add(RUSHER_RADIUS).saturating_sub(Fx::DELTA))
    };
    cells(pos.y).any(|y| cells(pos.x).any(|x| tiles.blocks(x, y, Body::Walker)))
}

#[test]
fn separation_never_pushes_a_rusher_into_walls_or_pits() {
    let column = (-150..).step_by(30).take(8).map(|y| point(200, y));
    let mut state = rushers_at(column);
    // The player stands just below the airlock's pit, near its west wall, so the ring the
    // crowd forms around it overlaps both.
    state.players[0].as_mut().unwrap().pos = cell_center(3, 4);
    for tick in 0..300 {
        step(&mut state, &TickInputs::default());
        let tiles = state.tiles().unwrap();
        for (_, enemy) in state.enemies.iter() {
            assert!(
                !in_blocking_tiles(tiles, enemy.pos),
                "tick {tick}: {enemy:?}"
            );
        }
    }
}
