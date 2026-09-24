//! Combat rules through the public `step`: gun vs rusher, contact damage, i-frames, and
//! death -> restart.

use sim::{
    Buttons, DEATH_TICKS, Enemy, Event, Fx, FxVec2, MAX_HP, PlayerInput, RUSHER_HP, Run,
    SPAWN_TELEGRAPH_TICKS, SimState, TickInputs, step,
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
