//! The run config (issue #15): difficulty only touches enemies, tunables override it, and
//! the config is run state (checksummed, kept across restarts unless replaced).

use sim::{
    Buttons, Difficulty, Enemy, Fx, FxVec2, PlayerInput, RunConfig, SimState, TickInputs, Tuning,
    step,
};

const SEED: u64 = 3;

const fn config(difficulty: Difficulty) -> RunConfig {
    RunConfig {
        difficulty,
        tuning: Tuning::NORMAL,
    }
}

/// A shooter 160 pt right of the player, past its spawn telegraph. Returns the tick its
/// first bullet appears and that bullet's speed in pt/s, rounded; `None` if it never fires.
fn first_shot(config: RunConfig) -> Option<(u64, i64)> {
    let mut state = SimState::new(SEED, config);
    let at = state.players[0].map(|p| p.pos).unwrap_or_default();
    state.enemies.insert(Enemy {
        spawn_ticks: 0,
        ..Enemy::shooter(
            FxVec2 {
                x: at.x.saturating_add(Fx::from_num(160)),
                y: at.y,
            },
            state.config.shooter_interval(),
            0,
        )
    });
    for _ in 0..600 {
        step(&mut state, &TickInputs::default());
        if let Some((_, bullet)) = state.enemy_bullets.iter().next() {
            let (x, y) = (bullet.vel.x, bullet.vel.y);
            let per_tick = x
                .saturating_mul(x)
                .saturating_add(y.saturating_mul(y))
                .sqrt();
            let speed = per_tick.saturating_mul_int(60);
            return Some((state.tick, speed.round().to_num()));
        }
    }
    None
}

/// Where the player is after walking right for a second.
fn walked(config: RunConfig) -> FxVec2 {
    let mut state = SimState::new(SEED, config);
    let mut inputs = TickInputs::default();
    inputs.players[0] = PlayerInput {
        move_mag: u8::MAX,
        ..PlayerInput::default()
    };
    for _ in 0..60 {
        step(&mut state, &inputs);
    }
    state.players[0].map(|p| p.pos).unwrap_or_default()
}

#[test]
fn difficulty_changes_enemy_values_only() {
    let shots = [Difficulty::Easy, Difficulty::Normal, Difficulty::Hard]
        .map(|d| first_shot(config(d)).unwrap());
    // Bullets at 200 / 260 / 320 pt/s; a shot every 120 / 96 / 72 ticks.
    assert_eq!(shots, [(120, 200), (96, 260), (72, 320)]);
    let walks = [Difficulty::Easy, Difficulty::Hard].map(|d| walked(config(d)));
    assert_eq!(
        walks,
        [walked(config(Difficulty::Normal)); 2],
        "player untouched"
    );
}

#[test]
fn tunables_override_the_difficulty() {
    let mut hard = config(Difficulty::Hard);
    hard.tuning.enemy_bullet_speed = Some(150);
    hard.tuning.shooter_interval = Some(60);
    assert_eq!(first_shot(hard), Some((60, 150)));
}

#[test]
fn config_is_part_of_the_checksum() {
    let normal = SimState::new(SEED, config(Difficulty::Normal));
    assert_eq!(
        normal.checksum(),
        SimState::new(SEED, config(Difficulty::Normal)).checksum()
    );
    let hard = SimState::new(SEED, config(Difficulty::Hard));
    assert_ne!(normal.checksum(), hard.checksum());
    let mut slower = config(Difficulty::Normal);
    slower.tuning.move_speed = 200;
    assert_ne!(normal.checksum(), SimState::new(SEED, slower).checksum());
}

fn restart(state: &mut SimState) {
    let mut inputs = TickInputs::default();
    inputs.players[0].buttons = Buttons::RESTART;
    step(state, &inputs);
}

#[test]
fn restart_keeps_the_config_unless_a_new_one_was_supplied() {
    let mut state = SimState::new(SEED, config(Difficulty::Hard));
    restart(&mut state);
    assert_eq!(state.config, config(Difficulty::Hard));

    state.next_config = Some(config(Difficulty::Easy));
    assert_eq!(state.config, config(Difficulty::Hard), "not mid-run");
    restart(&mut state);
    assert_eq!(
        (state.config, state.next_config),
        (config(Difficulty::Easy), None)
    );
}
