//! Determinism checks (issue #5): replay, rollback-every-tick, and a committed golden
//! checksum that CI verifies on both `x86_64` and `aarch64`.

use sim::{Buttons, Player, PlayerInput, Rng, Run, SimState, TickEvents, TickInputs, step};

const SEED: u64 = 0x5EED;
const TICKS: u64 = 600;

/// Update when a deliberate sim change alters results; never to paper over a mismatch
/// between machines.
const GOLDEN_TRACE: u64 = 0xbb21_bd70_33a3_05f7;

/// A reproducible input script: pseudo-random sticks and buttons for two players.
fn script() -> Vec<TickInputs> {
    let mut rng = Rng::from_seed(0x1A7);
    (0..TICKS)
        .map(|_| {
            let mut inputs = TickInputs::default();
            for input in &mut inputs.players[..2] {
                let bits = rng.next_u64().to_le_bytes();
                *input = PlayerInput {
                    move_dir: bits[0] % sim::MOVE_BUCKETS,
                    move_mag: bits[1],
                    aim: u16::from_le_bytes([bits[2], bits[3]]),
                    buttons: Buttons(bits[4] & 0b1111),
                };
            }
            inputs
        })
        .collect()
}

/// Starts dead so the script exercises the restart transition.
fn start() -> SimState {
    let mut state = SimState::new(SEED);
    state.players[1] = Some(Player::default());
    state.run = Run::Dead {
        ticks_until_restart: 30,
    };
    state
}

fn run(inputs: &[TickInputs]) -> Vec<(u64, TickEvents)> {
    let mut state = start();
    inputs
        .iter()
        .map(|i| {
            let events = step(&mut state, i);
            (state.checksum(), events)
        })
        .collect()
}

#[test]
fn replay_matches_every_tick() {
    let inputs = script();
    assert_eq!(run(&inputs), run(&inputs));
}

/// Save, step, restore, re-step: catches any state living outside `SimState`.
#[test]
fn rollback_every_tick_matches_straight_run() {
    let inputs = script();
    let reference = run(&inputs);
    let mut state = start();
    let mut snapshot = state.clone();
    for (tick, (i, expected)) in inputs.iter().zip(&reference).enumerate() {
        snapshot.clone_from(&state);
        let first = step(&mut state, i);
        state.clone_from(&snapshot);
        let second = step(&mut state, i);
        assert_eq!(first, second, "events diverged at tick {tick}");
        assert_eq!(
            &(state.checksum(), second),
            expected,
            "state diverged at tick {tick}"
        );
    }
}

#[test]
fn golden_trace_matches_committed_value() {
    let trace = run(&script())
        .iter()
        .fold(0_u64, |acc, (checksum, _)| acc.rotate_left(5) ^ checksum);
    println!("golden trace = {trace:#018x}");
    assert_eq!(trace, GOLDEN_TRACE, "got {trace:#018x}");
}
