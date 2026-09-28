//! Determinism checks (issue #5): replay, rollback-every-tick, and a committed golden
//! checksum that CI verifies on both `x86_64` and `aarch64`.

use sim::{
    Buttons, Event, HatchState, PlayerInput, Rng, Run, RunConfig, SimState, TickEvents, TickInputs,
    Tuning, step,
};
use std::ops::Range;
use std::sync::Arc;

const SEED: u64 = 0x5EED;
const TICKS: u64 = 2700;
/// Ticks when the party holds only RESTART (the scripted start, then after the death).
/// RESTART also abandons a live run, so each window presses it on its last tick only:
/// once the death pause is over, not every tick after the run has restarted.
const RESTARTS: [Range<u64>; 2] = [0..40, 1200..1250];
/// Ticks when the party walks from the start cell through the airlock's hatch down into
/// the cargo hold's encounter: [`WALK_EAST`] ticks right, lining up with the hatch, then
/// down.
const WALK_IN: [Range<u64>; 2] = [40..125, 1250..1335];
const WALK_EAST: u64 = 16;
/// Both players stand idle from just after entering the cargo hold, so its shooter lives
/// long enough to fire and the rushers kill them; they only roll every
/// [`STAND_STILL_ROLL`] ticks, alternately down and right, so some rolls land in the hold's
/// pit strip and some hits land on a roll's vulnerable landing.
const STAND_STILL: Range<u64> = 130..1200;
const STAND_STILL_ROLL: u64 = 72;
/// Random movement, but fire held with auto-aim, so the second visit clears waves.
const AUTO_FIGHT: Range<u64> = 1335..TICKS;

/// Update when a deliberate sim change alters results; never to paper over a mismatch
/// between machines.
const GOLDEN_TRACE: u64 = 0x38c0_7211_87d9_b0c8;

/// A reproducible input script for two players: scripted restarts, walks into the cargo
/// hold, and a stand-still death (see the phase constants); pseudo-random sticks, assist
/// and buttons elsewhere (never RESTART, which would abandon the live run). Dodge is
/// pressed on ~1 tick in 32 so rolls, dropped dodges and walking all show up; FIRE is held
/// about half the time, so the pistol empties and auto-vents; VENT is pressed on ~1 tick
/// in 256 for manual vents.
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
                    buttons: Buttons(bits[4] & !Buttons::DODGE.0 & !Buttons::RESTART.0 & 0b1_1111)
                        | if bits[6] < 8 {
                            Buttons::DODGE
                        } else {
                            Buttons::default()
                        }
                        | if bits[7] == 0 {
                            Buttons::VENT
                        } else {
                            Buttons::default()
                        },
                };
            }
            let scripted = if STAND_STILL.contains(&tick) {
                Some(if tick % STAND_STILL_ROLL == 0 {
                    // Only the roll moves: alternately straight down (bucket 8 of 32) and
                    // straight right (bucket 0).
                    PlayerInput {
                        move_dir: if (tick / STAND_STILL_ROLL).is_multiple_of(2) {
                            8
                        } else {
                            0
                        },
                        move_mag: u8::MAX,
                        buttons: Buttons::DODGE,
                        ..PlayerInput::default()
                    }
                } else {
                    PlayerInput::default()
                })
            } else if let Some(window) = RESTARTS.iter().find(|r| r.contains(&tick)) {
                Some(PlayerInput {
                    buttons: if tick == window.end.saturating_sub(1) {
                        Buttons::RESTART
                    } else {
                        Buttons::default()
                    },
                    ..PlayerInput::default()
                })
            } else {
                WALK_IN
                    .iter()
                    .find(|r| r.contains(&tick))
                    .map(|r| PlayerInput {
                        // Buckets: 0 of 32 = straight right, 8 = straight down.
                        move_dir: if tick.saturating_sub(r.start) < WALK_EAST {
                            0
                        } else {
                            8
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
    let mut state = SimState::new(SEED, RunConfig::default());
    state.players[1] = state.players[0];
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

/// A snapshot shares the static ship and copies only live state, which the checksum
/// covers: the ship itself counts once, as its build-time checksum.
#[test]
fn snapshots_share_the_ship_and_checksum_the_live_hatches() {
    let state = SimState::new(SEED, RunConfig::default());
    let snapshot = state.clone();
    assert!(Arc::ptr_eq(&state.ship, &snapshot.ship));
    let mut opened = snapshot.clone();
    opened.hatches[0] = HatchState::Open;
    assert_ne!(opened.checksum(), state.checksum());
    assert_eq!(snapshot.checksum(), state.checksum());
}

/// Guards the script's purpose: the golden trace must cover walking, rolling (dropped
/// mid-roll dodges, a hit on a vulnerable landing), falling into a pit, shooting, venting
/// (auto and manual, and the refills), kills, opening a hatch into a sealed encounter
/// with a second wave, shooters firing, player deaths and restarts.
#[test]
fn script_exercises_movement_dodge_combat_and_rooms() {
    let mut state = start();
    let start_pos = SimState::new(SEED, RunConfig::default()).players[0]
        .unwrap()
        .pos;
    let (mut rolls, mut moved, mut sealed) = (0, false, false);
    let (mut dropped_dodges, mut landing_hits) = (0, 0);
    let (mut auto_vents, mut manual_vents, mut refills) = (0, 0, 0);
    let (mut enemy_shots, mut enemy_bullets) = (0, 0);
    let mut events = Vec::new();
    for i in script() {
        let was_rolling = state.players[0].is_some_and(|p| p.rolling());
        let was_venting = state.players[0].is_some_and(|p| p.gun.venting());
        let stepped = step(&mut state, &i).events;
        if let Some(p) = state.players[0] {
            let started = !was_venting && p.gun.venting();
            if started && stepped.contains(&Event::ShotFired { slot: 0 }) {
                auto_vents += 1;
            } else if started {
                manual_vents += 1;
            }
            refills += usize::from(was_venting && !p.gun.venting());
        }
        if let Some(p) = state.players[0] {
            rolls += usize::from(p.roll_ticks == Tuning::NORMAL.roll_ticks);
            moved |= p.pos != start_pos;
            // ETG roll: dodges mid-roll are dropped, and the landing can be hit.
            dropped_dodges += usize::from(
                was_rolling && p.rolling() && i.players[0].buttons.contains(Buttons::DODGE),
            );
        }
        landing_hits += stepped
            .iter()
            .filter(|e| match e {
                Event::PlayerHit { slot } => state.players[*slot].is_some_and(|p| p.rolling()),
                _ => false,
            })
            .count();
        events.extend(stepped);
        sealed |= state.hatches.contains(&HatchState::Sealed);
        enemy_shots += usize::from(state.enemy_bullets.len() > enemy_bullets);
        enemy_bullets = state.enemy_bullets.len();
    }
    let count = |f: fn(&Event) -> bool| events.iter().filter(|e| f(e)).count();
    let shots = count(|e| matches!(e, Event::ShotFired { .. }));
    let kills = count(|e| matches!(e, Event::EnemyKilled { .. }));
    let deaths = count(|e| matches!(e, Event::PlayerDied { .. }));
    let falls = count(|e| matches!(e, Event::PlayerFell { .. }));
    let restarts = count(|e| matches!(e, Event::Restarted));
    let entries = count(|e| matches!(e, Event::HatchOpened { .. }));
    let waves = count(|e| matches!(e, Event::WaveStarted { .. }));
    let cleared = count(|e| matches!(e, Event::RoomCleared { .. }));
    println!(
        "rolls={rolls} dropped_dodges={dropped_dodges} landing_hits={landing_hits} \
         shots={shots} kills={kills} deaths={deaths} restarts={restarts} \
         entries={entries} waves={waves} cleared={cleared} sealed={sealed} \
         enemy_shots={enemy_shots} falls={falls}"
    );
    println!("auto_vents={auto_vents} manual_vents={manual_vents} refills={refills}");
    assert!(
        auto_vents >= 3 && manual_vents >= 1 && refills >= 5,
        "auto_vents={auto_vents} manual_vents={manual_vents} refills={refills}"
    );
    assert!(
        moved && rolls >= 10 && dropped_dodges >= 5 && landing_hits >= 1,
        "moved={moved} rolls={rolls} dropped_dodges={dropped_dodges} \
         landing_hits={landing_hits}"
    );
    // Restarts: the scripted start plus at least one after a full-party death.
    assert!(
        shots >= 50 && kills >= 5 && deaths >= 2 && restarts >= 2,
        "shots={shots} kills={kills} deaths={deaths} restarts={restarts}"
    );
    assert!(
        entries >= 2 && waves >= 1 && cleared >= 1 && sealed,
        "entries={entries} waves={waves} cleared={cleared} sealed={sealed}"
    );
    // The cargo hold's second wave brings a shooter.
    assert!(enemy_shots >= 2, "enemy_shots={enemy_shots}");
    // Rolls landing in the cargo hold's pit: falls, respawns, and a fatal fall.
    assert!(falls >= 2, "falls={falls}");
}
