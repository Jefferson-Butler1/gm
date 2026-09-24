//! Determinism checks (issue #5): replay, rollback-every-tick, and a committed golden
//! checksum that CI verifies on both `x86_64` and `aarch64`.

use sim::{Buttons, Event, Player, PlayerInput, Rng, Run, SimState, TickEvents, TickInputs, step};

const SEED: u64 = 0x5EED;
const TICKS: u64 = 1200;
/// Both players stand idle through this range so the rushers kill them; random input
/// (which includes RESTART) resumes afterwards.
const STAND_STILL: std::ops::Range<u64> = 600..1000;

/// Update when a deliberate sim change alters results; never to paper over a mismatch
/// between machines.
const GOLDEN_TRACE: u64 = 0x4139_a2ca_03e7_530d;

/// A reproducible input script: pseudo-random sticks, assist and buttons for two players.
/// Dodge is pressed on ~1 tick in 8 so rolls, cooldown drops and walking all show up;
/// FIRE is held about half the time. See [`STAND_STILL`] for the scripted death.
fn script() -> Vec<TickInputs> {
    let mut rng = Rng::from_seed(0x1A7);
    (0..TICKS)
        .map(|tick| {
            let mut inputs = TickInputs::default();
            for input in &mut inputs.players[..2] {
                let bits = rng.next_u64().to_le_bytes();
                *input = PlayerInput {
                    move_dir: bits[0] % sim::MOVE_BUCKETS,
                    move_mag: bits[1],
                    aim: u16::from_le_bytes([bits[2], bits[3]]),
                    assist: bits[5],
                    buttons: Buttons(bits[4] & !Buttons::DODGE.0 & 0b1_1111)
                        | if bits[6] < 32 {
                            Buttons::DODGE
                        } else {
                            Buttons::default()
                        },
                };
            }
            if STAND_STILL.contains(&tick) {
                inputs = TickInputs::default();
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

/// Guards the script's purpose: the golden trace must cover walking, rolling, shooting,
/// kills, player deaths and restarts.
#[test]
fn script_exercises_movement_dodge_and_combat() {
    let mut state = start();
    let (mut rolls, mut moved) = (0, false);
    let mut events = Vec::new();
    for i in script() {
        events.extend(step(&mut state, &i).events);
        if let Some(p) = state.players[0] {
            rolls += usize::from(p.roll_ticks == sim::ROLL_TICKS);
            moved |= p.pos != Player::default().pos;
        }
    }
    let count = |f: fn(&Event) -> bool| events.iter().filter(|e| f(e)).count();
    let shots = count(|e| matches!(e, Event::ShotFired { .. }));
    let kills = count(|e| matches!(e, Event::EnemyKilled { .. }));
    let deaths = count(|e| matches!(e, Event::PlayerDied { .. }));
    let restarts = count(|e| matches!(e, Event::Restarted));
    println!("rolls={rolls} shots={shots} kills={kills} deaths={deaths} restarts={restarts}");
    assert!(moved && rolls >= 10, "moved={moved} rolls={rolls}");
    // Restarts: the scripted start plus at least one after a full-party death.
    assert!(
        shots >= 50 && kills >= 5 && deaths >= 2 && restarts >= 2,
        "shots={shots} kills={kills} deaths={deaths} restarts={restarts}"
    );
}
