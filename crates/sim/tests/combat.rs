//! Combat rules through the public `step`: gun vs rusher and shooter, contact damage,
//! enemy bullets, i-frames, steering, and death -> restart.

use sim::room::{cell_center, cell_of};
use sim::ship::{Body, Tiles};
use sim::{
    Awareness, Behavior, Bullet, Buttons, DEATH_TICKS, Difficulty, ENEMY_RADIUS, Enemy, Event, Fx,
    FxVec2, MAX_HP, Pattern, PlayerInput, RUSHER_HP, Rng, RoomId, Run, RunConfig,
    SPAWN_TELEGRAPH_TICKS, SimState, TickInputs, Tuning, step, trig,
};

const SEED: u64 = 7;

const DOWN: u16 = 16384;
const LEFT: u16 = 32768;
/// The shot telegraph.
const AIM_TICKS: u16 = Tuning::NORMAL.shooter_telegraph;

/// A fresh run: the party alone in the empty start room, the airlock.
fn empty_arena() -> SimState {
    SimState::new(SEED, RunConfig::default())
}

/// The floor cell at (`x`, `y`) of the airlock, whose cell (0, 0) is floor cell (8, 0).
fn airlock(x: usize, y: usize) -> FxVec2 {
    cell_center(x.saturating_add(8), y)
}

/// A point (`x`, `y`) from where the player starts.
fn point(x: i32, y: i32) -> FxVec2 {
    let start = empty_arena().players[0].map(|p| p.pos).unwrap_or_default();
    FxVec2 {
        x: start.x.saturating_add(Fx::from_num(x)),
        y: start.y.saturating_add(Fx::from_num(y)),
    }
}

/// Already hunting the player where it starts, so tests of the fight itself see no
/// `EnemyAlerted`.
fn hunting() -> Awareness {
    Awareness::Alert {
        last_seen: point(0, 0),
        searching: 0,
    }
}

/// An empty arena plus one hunting rusher `x` points right of the player, already past
/// its spawn telegraph.
fn arena_with_rusher(x: i32) -> SimState {
    let mut state = empty_arena();
    state.enemies.insert(Enemy {
        spawn_ticks: 0,
        awareness: hunting(),
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
    assert_eq!((hits, kills), (2, 1), "rushers die in 2 hits: {events:?}");
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
fn roll_iframes_block_contact_damage_but_the_landing_is_vulnerable() {
    let mut state = arena_with_rusher(30); // in the roll's path (facing right)
    let mut events = run(&mut state, 1, &press(Buttons::DODGE));
    // The i-frames: 55% of the 36-tick roll, rounded = 20 ticks.
    events.extend(run(&mut state, 19, &TickInputs::default()));
    assert!(events.is_empty(), "{events:?}");
    // The rusher turns and catches the slowing landing.
    let events = run(&mut state, 16, &TickInputs::default());
    assert_eq!(events, [Event::PlayerHit { slot: 0 }]);
    assert!(
        state.players[0].unwrap().rolling(),
        "hit before the roll ended"
    );
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
    let mut fresh = SimState::new(Rng::next_seed(SEED), RunConfig::default());
    fresh.tick = state.tick;
    assert_eq!(state, fresh);
}

fn restart_now(state: &mut SimState) {
    state.run = Run::Dead {
        ticks_until_restart: 0,
    };
    assert_eq!(run(state, 1, &press(Buttons::RESTART)), [Event::Restarted]);
}

#[test]
fn restart_abandons_a_live_run_at_once() {
    let mut state = arena_with_rusher(40);
    state.cleared = 0b1;
    assert_eq!(
        run(&mut state, 1, &press(Buttons::RESTART)),
        [Event::Restarted]
    );
    let mut fresh = SimState::new(Rng::next_seed(SEED), RunConfig::default());
    fresh.tick = state.tick;
    assert_eq!(state, fresh);
}

#[test]
fn each_restart_resets_the_rooms_and_derives_the_next_run_seed() {
    let mut state = SimState::new(SEED, RunConfig::default());
    // As if the party had cleared the airlock and died in the cargo hold.
    state.cleared = 0b1;
    state.visited = 0b11;
    state.hatches[0] = sim::HatchState::Sealed;
    state.run = Run::Dead {
        ticks_until_restart: 0,
    };
    restart_now(&mut state);
    let second_seed = state.seed;
    assert_eq!(second_seed, Rng::next_seed(SEED));
    let mut fresh = SimState::new(second_seed, RunConfig::default());
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
    let touching = ENEMY_RADIUS.saturating_mul_int(2);
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
    let touching = ENEMY_RADIUS.saturating_mul_int(2);
    let min = touching.saturating_sub(Fx::from_num(OVERLAP_TOLERANCE));
    // A column of 8, 30 pt apart, 200 pt right of the player and spanning the airlock's
    // height (the player starts in its lower half).
    let column = (-150..).step_by(30).take(8).map(|y| point(200, y));
    let mut state = rushers_at(column);
    let mut last: Vec<FxVec2> = Vec::new();
    for tick in 0..360 {
        step(&mut state, &TickInputs::default());
        let closest = closest_pair(&state);
        assert!(closest >= min, "tick {tick}: {closest}");
        let now: Vec<FxVec2> = state.enemies.iter().map(|(_, e)| e.pos).collect();
        // Settled by 300: nobody moves 1/16 pt a tick (sub-pixel; no visible jitter). At
        // the 150 pt/s rusher speed the ring creeps ~0.04 pt a tick instead of freezing.
        if tick >= 300 {
            for (a, b) in now.iter().zip(&last) {
                let moved = a.x.abs_diff(b.x).saturating_add(a.y.abs_diff(b.y));
                assert!(moved < Fx::from_bits(1 << 28), "tick {tick}: moved {moved}");
            }
        }
        last = now;
    }
}

/// Whether an enemy's box overlaps any cell that stops walkers.
fn in_blocking_tiles(tiles: Tiles<'_>, pos: FxVec2) -> bool {
    let cells = |c: Fx| {
        cell_of(c.saturating_sub(ENEMY_RADIUS))
            ..=cell_of(c.saturating_add(ENEMY_RADIUS).saturating_sub(Fx::DELTA))
    };
    cells(pos.y).any(|y| cells(pos.x).any(|x| tiles.blocks(x, y, Body::Walker)))
}

#[test]
fn separation_never_pushes_a_rusher_into_walls_or_pits() {
    let column = (-150..).step_by(30).take(8).map(|y| point(200, y));
    let mut state = rushers_at(column);
    // The player stands just below the airlock's pit, near its west wall, so the ring the
    // crowd forms around it overlaps both.
    state.players[0].as_mut().unwrap().pos = airlock(3, 4);
    for tick in 0..300 {
        step(&mut state, &TickInputs::default());
        let tiles = state.tiles();
        for (_, enemy) in state.enemies.iter() {
            assert!(
                !in_blocking_tiles(tiles, enemy.pos),
                "tick {tick}: {enemy:?}"
            );
        }
    }
}

// --- shooter, enemy bullets, steering -------------------------------------------------

/// An empty arena plus one active, hunting shooter at `at`, `ticks` from starting to aim.
fn arena_with_shooter(at: FxVec2, ticks: u16) -> SimState {
    let mut state = empty_arena();
    state.enemies.insert(Enemy {
        spawn_ticks: 0,
        awareness: hunting(),
        behavior: Behavior::Shooter {
            pattern: Pattern::Aimed,
            shot_timer: AIM_TICKS.saturating_add(ticks),
            strafe: 1,
        },
        ..Enemy::shooter(at, Pattern::Aimed, &RunConfig::default(), 0)
    });
    state
}

fn first_enemy(state: &SimState) -> Option<Enemy> {
    state.enemies.iter().next().map(|(_, e)| *e)
}

#[test]
fn shooter_telegraphs_stands_still_then_fires_a_bullet_that_hurts() {
    // 160 pt right: inside its preferred range, so it only strafes before aiming.
    let mut state = arena_with_shooter(point(160, 0), 1);
    run(&mut state, 1, &TickInputs::default());
    let aiming_at = first_enemy(&state).unwrap().pos;
    assert_eq!(
        first_enemy(&state).unwrap().aiming(AIM_TICKS),
        Some(AIM_TICKS)
    );

    let aim_rest = usize::from(AIM_TICKS) - 1;
    let events = run(&mut state, aim_rest, &TickInputs::default());
    assert!(events.is_empty(), "{events:?}");
    assert!(state.enemy_bullets.is_empty(), "not yet");
    assert_eq!(
        first_enemy(&state).unwrap().pos,
        aiming_at,
        "stood still while aiming"
    );

    run(&mut state, 1, &TickInputs::default());
    assert_eq!(state.enemy_bullets.len(), 1, "fired");
    assert_eq!(
        first_enemy(&state).unwrap().aiming(AIM_TICKS),
        None,
        "reloading"
    );

    let events = run(&mut state, 40, &TickInputs::default());
    assert_eq!(events, [Event::PlayerHit { slot: 0 }]);
    assert_eq!(state.players[0].unwrap().hp, MAX_HP - 1);
    assert!(state.enemy_bullets.is_empty(), "spent on the hit");
}

#[test]
fn shooter_backs_off_to_its_range_and_never_touches() {
    let mut state = arena_with_shooter(point(40, 0), 90);
    let events = run(&mut state, 60, &TickInputs::default());
    assert!(events.is_empty(), "no contact damage: {events:?}");
    // Backs off to 120 pt, then strafes around the player at about that range.
    let (at, from) = (first_enemy(&state).unwrap().pos, point(0, 0));
    let (dx, dy) = (at.x.saturating_sub(from.x), at.y.saturating_sub(from.y));
    let gap = dx
        .saturating_mul(dx)
        .saturating_add(dy.saturating_mul(dy))
        .sqrt();
    assert!(gap >= Fx::from_num(118), "{gap}");
}

/// An enemy bullet flying left from `from`.
fn bullet_flying_left(state: &mut SimState, from: FxVec2) {
    state.enemy_bullets.insert(Bullet {
        pos: from,
        vel: FxVec2 {
            x: Fx::from_num(-5),
            y: Fx::ZERO,
        },
        ticks_left: 100,
    });
}

#[test]
fn dodge_iframes_let_enemy_bullets_pass_through() {
    let mut standing = empty_arena();
    bullet_flying_left(&mut standing, point(40, 0));
    let events = run(&mut standing, 30, &TickInputs::default());
    assert_eq!(events, [Event::PlayerHit { slot: 0 }], "control: it hits");

    // Roll right (the default facing), through the bullet.
    let mut rolling = empty_arena();
    bullet_flying_left(&mut rolling, point(40, 0));
    let mut events = run(&mut rolling, 1, &press(Buttons::DODGE));
    events.extend(run(&mut rolling, 29, &TickInputs::default()));
    assert!(events.is_empty(), "{events:?}");
    assert_eq!(rolling.players[0].unwrap().hp, MAX_HP);
    assert_eq!(rolling.enemy_bullets.len(), 1, "flew on past");
}

#[test]
fn pillars_stop_enemy_bullets_and_block_a_shooters_aim() {
    // The cargo hold's pillar covers floor cells (10..=11, 12..=13), x 320..384; the
    // player hides west of it at its height. The hold counts as cleared, so no fight
    // starts.
    let row = |x: i32| FxVec2 {
        x: Fx::from_num(x),
        y: Fx::from_num(416),
    };
    let mut state = empty_arena();
    state.cleared = 1 << 1;
    state.players[0].as_mut().unwrap().pos = row(240);
    bullet_flying_left(&mut state, row(460));
    let events = run(&mut state, 60, &TickInputs::default());
    assert!(events.is_empty(), "{events:?}");
    assert!(state.enemy_bullets.is_empty(), "the pillar ate it");

    let mut state = arena_with_shooter(row(420), 1);
    state.cleared = 1 << 1;
    state.players[0].as_mut().unwrap().pos = row(240);
    run(&mut state, 1, &TickInputs::default());
    assert_eq!(
        first_enemy(&state).unwrap().aiming(AIM_TICKS),
        None,
        "no line of fire"
    );
}

#[test]
fn a_rusher_steers_around_a_pit_between_it_and_the_player() {
    // The airlock's pit covers floor cells (11..=12, 3): x 352..416, y 96..128. The
    // player stands right above its middle and the rusher right below, so the way is
    // blocked head-on.
    let mut state = rushers_at([FxVec2 {
        x: Fx::from_num(384),
        y: Fx::from_num(200),
    }]);
    state.players[0].as_mut().unwrap().pos = FxVec2 {
        x: Fx::from_num(384),
        y: Fx::from_num(48),
    };
    let mut events = Vec::new();
    for tick in 0..150 {
        events.extend(step(&mut state, &TickInputs::default()).events);
        // Players may walk onto pits now; enemies still can't.
        let tiles = state.tiles();
        for (_, enemy) in state.enemies.iter() {
            assert!(
                !in_blocking_tiles(tiles, enemy.pos),
                "tick {tick}: {enemy:?}"
            );
        }
    }
    assert!(events.contains(&Event::PlayerHit { slot: 0 }), "{events:?}");
}

#[test]
fn falling_into_a_pit_on_the_last_hit_point_is_a_death() {
    let mut state = empty_arena();
    let player = state.players[0].as_mut().unwrap();
    player.hp = 1;
    // Right of the airlock's pit (its cells 3..=4, 3), walking into it.
    player.pos = airlock(6, 3);
    let mut walk_left = TickInputs::default();
    walk_left.players[0] = PlayerInput {
        move_dir: 16,
        move_mag: u8::MAX,
        ..PlayerInput::default()
    };
    let events = run(&mut state, 30, &walk_left);
    assert_eq!(
        events,
        [Event::PlayerFell { slot: 0 }, Event::PlayerDied { slot: 0 }]
    );
    assert!(matches!(state.run, Run::Dead { .. }), "{:?}", state.run);
}

#[test]
fn enemies_ignore_a_falling_player_and_its_respawn_pushes_them_back() {
    let mut state = empty_arena();
    // Mid-fall into the airlock's pit (its cells 3..=4, 3), respawning at its cell (8, 6).
    let respawn = airlock(8, 6);
    let player = state.players[0].as_mut().unwrap();
    player.pos = airlock(3, 3);
    player.fall_ticks = 20;
    player.solid = respawn;
    // A rusher 20 pt from the respawn spot, and a shooter.
    let near = FxVec2 {
        x: respawn.x.saturating_add(Fx::from_num(20)),
        ..respawn
    };
    let rusher = state.enemies.insert(Enemy {
        spawn_ticks: 0,
        ..Enemy::rusher(near)
    });
    state.enemies.insert(Enemy {
        spawn_ticks: 0,
        ..Enemy::shooter(airlock(10, 2), Pattern::Aimed, &RunConfig::default(), 0)
    });
    let enemies_at = |s: &SimState| s.enemies.iter().map(|(_, e)| e.pos).collect::<Vec<_>>();
    let before = enemies_at(&state);
    run(&mut state, 19, &TickInputs::default());
    assert!(state.players[0].unwrap().falling());
    assert_eq!(
        enemies_at(&state),
        before,
        "nobody chases or crowds the pit"
    );
    assert!(state.enemy_bullets.is_empty(), "nobody fires at it");

    step(&mut state, &TickInputs::default());
    let player = state.players[0].unwrap();
    assert!(!player.falling() && player.pos == respawn);
    let gap = state
        .enemies
        .get(rusher)
        .map(|e| sub_len(e.pos, respawn))
        .unwrap();
    // Pushed to two cells (64 pt), then it took its first 2.5 pt step back in.
    assert!(gap >= Fx::from_num(61), "rusher {gap} pt from the respawn");
}

const fn sub_len(a: FxVec2, b: FxVec2) -> Fx {
    let (dx, dy) = (a.x.saturating_sub(b.x), a.y.saturating_sub(b.y));
    dx.saturating_mul(dx)
        .saturating_add(dy.saturating_mul(dy))
        .sqrt()
}

// --- spread shooter (the pattern experiment) ------------------------------------------

#[test]
fn spread_shooter_fans_slow_pellets_at_the_target() {
    for (difficulty, pellets, speed, interval) in [
        (Difficulty::Normal, 5, 162, 210),
        (Difficulty::Hard, 7, 200, 157),
    ] {
        let config = RunConfig {
            difficulty,
            tuning: Tuning::NORMAL,
        };
        let mut state = SimState::new(SEED, config);
        state.enemies.insert(Enemy {
            spawn_ticks: 0,
            behavior: Behavior::Shooter {
                pattern: Pattern::Spread,
                shot_timer: 1,
                strafe: 1,
            },
            ..Enemy::shooter(point(160, 0), Pattern::Spread, &config, 0)
        });
        run(&mut state, 1, &TickInputs::default());
        let vels: Vec<FxVec2> = state.enemy_bullets.iter().map(|(_, b)| b.vel).collect();
        assert_eq!(vels.len(), pellets, "{difficulty:?}");
        for vel in &vels {
            let per_tick = vel
                .x
                .saturating_mul(vel.x)
                .saturating_add(vel.y.saturating_mul(vel.y))
                .sqrt();
            let per_second: i64 = per_tick.saturating_mul_int(60).round().to_num();
            assert_eq!(per_second, speed, "5/8 of the aimed speed ({difficulty:?})");
        }
        // Fanned 12 degrees (~2184 units) apart, centered on the target (straight left).
        let offsets: Vec<i16> = vels
            .iter()
            .map(|&v| trig::angle_diff(LEFT, trig::angle_of(v).unwrap()))
            .collect();
        let half = i16::try_from(pellets / 2).unwrap();
        for (i, offset) in (-half..=half).zip(&offsets) {
            assert!(
                (offset - i * 2184).abs() <= 8,
                "{difficulty:?}: {offsets:?}"
            );
        }
        let Behavior::Shooter { shot_timer, .. } = first_enemy(&state).unwrap().behavior else {
            panic!("not a shooter");
        };
        assert_eq!(shot_timer, interval, "next volley ({difficulty:?})");
    }
}

#[test]
fn spread_placements_follow_the_experiment_toggle() {
    for (on, spread) in [(true, 1), (false, 0)] {
        let mut config = RunConfig::default();
        config.tuning.spread_shooter = on;
        let mut state = SimState::new(SEED, config);
        // The bridge (from floor cell (28, 25)) with its base wave dead: the next tick
        // spawns its second wave, two shooters of which one is a spread placement.
        state.run = Run::Encounter {
            room: RoomId(3),
            wave: 0,
        };
        state.players[0].as_mut().unwrap().pos = cell_center(28 + 7, 25 + 6);
        step(&mut state, &TickInputs::default());
        let patterns: Vec<Pattern> = state
            .enemies
            .iter()
            .filter_map(|(_, e)| match e.behavior {
                Behavior::Shooter { pattern, .. } => Some(pattern),
                Behavior::Rusher { .. } => None,
            })
            .collect();
        assert_eq!(patterns.len(), 2);
        let spreads = patterns.iter().filter(|&&p| p == Pattern::Spread).count();
        assert_eq!(spreads, spread, "spread_shooter = {on}");
    }
}
