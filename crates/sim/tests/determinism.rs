//! Determinism checks (issue #5): replay, rollback-every-tick, and a committed golden
//! checksum that CI verifies on both `x86_64` and `aarch64`.

use sim::{Buttons, Event, PlayerInput, Rng, RoomId, Run, SimState, TickEvents, TickInputs, step};
use std::ops::Range;

const SEED: u64 = 0x5EED;
const TICKS: u64 = 1500;
/// Ticks when the party holds only RESTART (the scripted start, then after the death).
const RESTARTS: [Range<u64>; 2] = [0..40, 900..950];
/// Ticks when the party walks from the start cell out of the airlock's east exit and into
/// the cargo hold's encounter: [`WALK_NORTH`] ticks up, then east.
const WALK_IN: [Range<u64>; 2] = [40..100, 950..1010];
const WALK_NORTH: u64 = 8;
/// Both players stand idle through this range so the rushers kill them.
const STAND_STILL: Range<u64> = 160..900;
/// Random movement, but fire held with auto-aim, so the second visit clears waves.
const AUTO_FIGHT: Range<u64> = 1010..TICKS;

/// Update when a deliberate sim change alters results; never to paper over a mismatch
/// between machines.
const GOLDEN_TRACE: u64 = 0x019f_d6d1_a18d_a2ab;

/// A reproducible input script for two players: scripted restarts, walks into the cargo
/// hold, and a stand-still death (see the phase constants); pseudo-random sticks, assist
/// and buttons elsewhere. Dodge is pressed on ~1 tick in 8 so rolls, cooldown drops and
/// walking all show up; FIRE is held about half the time.
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
            let scripted = if STAND_STILL.contains(&tick) {
                Some(PlayerInput::default())
            } else if RESTARTS.iter().any(|r| r.contains(&tick)) {
                Some(PlayerInput {
                    buttons: Buttons::RESTART,
                    ..PlayerInput::default()
                })
            } else {
                WALK_IN
                    .iter()
                    .find(|r| r.contains(&tick))
                    .map(|r| PlayerInput {
                        // Buckets: 24 of 32 = straight up, 0 = straight right.
                        move_dir: if tick.saturating_sub(r.start) < WALK_NORTH {
                            24
                        } else {
                            0
                        },
                        move_mag: u8::MAX,
                        ..PlayerInput::default()
                    })
            };
            if let Some(input) = scripted {
                inputs.players[..2].fill(input);
            }
            if AUTO_FIGHT.contains(&tick) {
                for input in &mut inputs.players[..2] {
                    input.buttons |= Buttons::FIRE | Buttons::AUTO_AIM;
                }
            }
            inputs
        })
        .collect()
}

/// Two players, starting dead so the script exercises the restart transition.
fn start() -> SimState {
    let mut state = SimState::new(SEED);
    state.players[1] = state.players[0];
    state.run = Run::Dead {
        room: RoomId(0),
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
/// kills, a room transition into a sealed encounter with a second wave, player deaths
/// and restarts.
#[test]
fn script_exercises_movement_dodge_combat_and_rooms() {
    let mut state = start();
    let start_pos = SimState::new(SEED).players[0].unwrap().pos;
    let (mut rolls, mut moved, mut sealed) = (0, false, false);
    let mut events = Vec::new();
    for i in script() {
        events.extend(step(&mut state, &i).events);
        if let Some(p) = state.players[0] {
            rolls += usize::from(p.roll_ticks == sim::ROLL_TICKS);
            moved |= p.pos != start_pos;
        }
        sealed |= state.run.doors_locked();
    }
    let count = |f: fn(&Event) -> bool| events.iter().filter(|e| f(e)).count();
    let shots = count(|e| matches!(e, Event::ShotFired { .. }));
    let kills = count(|e| matches!(e, Event::EnemyKilled { .. }));
    let deaths = count(|e| matches!(e, Event::PlayerDied { .. }));
    let restarts = count(|e| matches!(e, Event::Restarted));
    let entries = count(|e| matches!(e, Event::RoomEntered { .. }));
    let waves = count(|e| matches!(e, Event::WaveStarted { .. }));
    let cleared = count(|e| matches!(e, Event::RoomCleared { .. }));
    println!(
        "rolls={rolls} shots={shots} kills={kills} deaths={deaths} restarts={restarts} \
         entries={entries} waves={waves} cleared={cleared} sealed={sealed}"
    );
    assert!(moved && rolls >= 10, "moved={moved} rolls={rolls}");
    // Restarts: the scripted start plus at least one after a full-party death.
    assert!(
        shots >= 50 && kills >= 5 && deaths >= 2 && restarts >= 2,
        "shots={shots} kills={kills} deaths={deaths} restarts={restarts}"
    );
    assert!(
        entries >= 2 && waves >= 1 && cleared >= 1 && sealed,
        "entries={entries} waves={waves} cleared={cleared} sealed={sealed}"
    );
}
