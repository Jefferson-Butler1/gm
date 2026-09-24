//! Combat rules through the public `step`: gun vs rusher, contact damage, i-frames, and
//! death -> restart.

use sim::{
    Buttons, DEATH_TICKS, Enemy, Event, Fx, FxVec2, MAX_HP, PlayerInput, RUSHER_HP, RUSHER_RADIUS,
    Run, SPAWN_TELEGRAPH_TICKS, SimState, TickInputs, step,
};

const SEED: u64 = 7;

const DOWN: u16 = 16384;

fn point(x: i32, y: i32) -> FxVec2 {
    FxVec2 {
        x: Fx::from_num(x),
        y: Fx::from_num(y),
    }
}

/// A fresh run with the spawner held off.
fn empty_arena() -> SimState {
    let mut state = SimState::new(SEED);
    state.spawn_cooldown = u16::MAX;
    state
}

/// An empty arena plus one rusher at (`x`, 0), already past its spawn telegraph.
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
    let mut state = arena_with_rusher(250);
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
            ticks_until_restart: DEATH_TICKS
        }
    );

    // Restart is ignored during the death pause.
    let restart = press(Buttons::RESTART);
    let events = run(&mut state, usize::try_from(DEATH_TICKS).unwrap(), &restart);
    assert!(events.is_empty(), "{events:?}");

    assert_eq!(run(&mut state, 1, &restart), [Event::Restarted]);
    let mut fresh = SimState::new(SEED);
    fresh.tick = state.tick;
    assert_eq!(state, fresh);
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
    // A column of 8 at x = 200, 30 pt apart.
    let column = (-105..).step_by(30).take(8).map(|y| point(200, y));
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
