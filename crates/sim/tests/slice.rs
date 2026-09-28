//! The whole slice: the bridge's captain, the airlocks it unlocks -> `Run::Won`, restart
//! from Won, and a scripted player that walks whole generated ships, fighting through every
//! room to the bridge and out an airlock.

use sim::room::{CELL, Category, Cell, cell_center, cell_of};
use sim::ship::{Body, HatchKind, Tiles};
use sim::{
    Arrival, Buttons, Difficulty, Event, FxVec2, HatchState, MAX_HP, MOVE_BUCKETS, Pattern,
    PlayerInput, Rng, RoomId, Run, RunConfig, Ship, SimState, TickInputs, step, trig,
};
use std::collections::VecDeque;

const SEED: u64 = 11;

/// The Corvette's boss room: the bridge.
fn bridge(ship: &Ship) -> RoomId {
    let index = ship
        .rooms()
        .iter()
        .position(|r| r.room.category == Category::Boss)
        .unwrap_or_default();
    RoomId(u16::try_from(index).unwrap_or_default())
}

/// The cells of the airlocks' outer hatches, and their live states.
fn outer_hatches(state: &SimState) -> Vec<((usize, usize), HatchState)> {
    (state.ship.hatches().iter().zip(&state.hatches))
        .filter(|(h, _)| h.kind == HatchKind::Airlock)
        .map(|(h, &live)| (h.gap.cell(0), live))
        .collect()
}

/// A fresh run, slot 0 standing in the boarding airlock's outer hatch.
fn in_an_outer_hatch() -> SimState {
    let mut state = SimState::new(SEED, RunConfig::default());
    let (start, _) = state.ship.start();
    let hatch = (state.ship.hatches().iter())
        .find(|h| h.kind == HatchKind::Airlock && h.rooms[0] == start)
        .map(|h| h.gap.cell(0))
        .unwrap_or_default();
    if let Some(player) = &mut state.players[0] {
        player.pos = cell_center(hatch.0, hatch.1);
    }
    state
}

#[test]
fn every_airlock_starts_locked_and_the_bridges_last_wave_unlocks_them_all() {
    let state = SimState::new(SEED, RunConfig::default());
    let hatches = outer_hatches(&state);
    assert_eq!(hatches.len(), 3);
    assert!(hatches.iter().all(|&(_, s)| s == HatchState::AirlockLocked));
    assert!(!state.airlocks_unlocked());

    // Another room's last wave dying unlocks nothing.
    let mut other = state.clone();
    let midship = RoomId(4);
    let room = other.ship.room(midship).map(|r| r.room);
    assert_eq!(room.map(|r| r.category), Some(Category::Normal));
    let waves = room.map_or(0, |r| r.reinforcements.len());
    other.run = Run::Encounter {
        room: midship,
        wave: u8::try_from(waves).unwrap(),
    };
    let events = step(&mut other, &TickInputs::default()).events;
    assert!(
        events.contains(&Event::RoomCleared { room: midship }),
        "{events:?}"
    );
    assert!(!other.airlocks_unlocked());

    let mut cleared = state;
    let room = bridge(&cleared.ship);
    let waves = cleared
        .ship
        .room(room)
        .map_or(0, |r| r.room.reinforcements.len());
    cleared.run = Run::Encounter {
        room,
        wave: u8::try_from(waves).unwrap(),
    };
    let events = step(&mut cleared, &TickInputs::default()).events;
    assert!(events.contains(&Event::RoomCleared { room }), "{events:?}");
    assert!(
        outer_hatches(&cleared)
            .iter()
            .all(|&(_, s)| s == HatchState::Closed)
    );
    assert!(cleared.airlocks_unlocked());
}

#[test]
fn stepping_into_an_airlocks_outer_hatch_wins_only_once_it_is_unlocked() {
    let mut locked = in_an_outer_hatch();
    for _ in 0..30 {
        let events = step(&mut locked, &TickInputs::default()).events;
        assert!(!events.contains(&Event::Won), "{events:?}");
    }
    assert_eq!(locked.run, Run::Boarding);

    let mut unlocked = in_an_outer_hatch();
    for s in &mut unlocked.hatches {
        if *s == HatchState::AirlockLocked {
            *s = HatchState::Closed;
        }
    }
    let events = step(&mut unlocked, &TickInputs::default()).events;
    assert_eq!(events, [Event::Won]);
    assert_eq!(unlocked.run, Run::Won);

    // The world freezes until restart.
    let frozen = unlocked.clone();
    let mut walk = TickInputs::default();
    walk.players[0].move_mag = u8::MAX;
    assert!(step(&mut unlocked, &walk).events.is_empty());
    assert_eq!(unlocked.players, frozen.players);
}

/// Walks the party east from the passage fore of the hold (every other room cleared)
/// through its hatch into the bridge's encounter.
fn enter_the_bridge() -> SimState {
    let mut state = SimState::new(SEED, RunConfig::default());
    state.cleared = !(1 << bridge(&state.ship).0);
    if let Some(player) = &mut state.players[0] {
        // In the passage's floor cells (70..=73, 19..=20), centered on its rows.
        player.pos = FxVec2 {
            x: cell_center(71, 19).x,
            y: CELL.saturating_mul_int(20),
        };
        player.solid = player.pos;
    }
    let mut east = TickInputs::default();
    east.players[0].move_mag = u8::MAX;
    for _ in 0..40 {
        step(&mut state, &east);
    }
    state
}

#[test]
fn the_captain_warps_in_last_with_the_boss_telegraph_and_the_airlocks_open_when_it_dies() {
    let mut state = enter_the_bridge();
    let room = bridge(&state.ship);
    assert!(matches!(state.run, Run::Encounter { room: r, wave: 0, .. } if r == room));
    // The first two waves die: the captain's wave spawns.
    for wave in [1, 2] {
        state.enemies.retain(|_, _| false);
        let events = step(&mut state, &TickInputs::default()).events;
        assert!(events.contains(&Event::WaveStarted { wave }), "{events:?}");
    }
    let captain = |state: &SimState| {
        state.enemies.iter().map(|(_, e)| *e).find(|e| {
            matches!(
                e.behavior,
                sim::Behavior::Shooter {
                    pattern: Pattern::Captain,
                    ..
                }
            )
        })
    };
    let boss = captain(&state).unwrap();
    assert_eq!((boss.arrival, boss.hp), (Arrival::Boss, sim::CAPTAIN_HP));
    assert!(boss.hunting(), "arrives knowing where the party is");
    assert_eq!(boss.spawn_ticks, Arrival::Boss.telegraph_ticks());

    // Its wave lives: the airlocks stay locked.
    for _ in 0..120 {
        if let Some(player) = &mut state.players[0] {
            player.hp = MAX_HP;
        }
        step(&mut state, &TickInputs::default());
        assert!(!state.airlocks_unlocked());
    }
    assert!(captain(&state).is_some());
    state.enemies.retain(|_, _| false);
    let events = step(&mut state, &TickInputs::default()).events;
    assert!(events.contains(&Event::RoomCleared { room }), "{events:?}");
    assert!(state.airlocks_unlocked());
}

#[test]
fn restart_from_won_starts_a_fresh_run() {
    let mut state = in_an_outer_hatch();
    state.run = Run::Won;
    state.cleared = 0b11110;
    let mut restart = TickInputs::default();
    restart.players[0].buttons = Buttons::RESTART;
    assert_eq!(step(&mut state, &restart).events, [Event::Restarted]);
    let mut fresh = SimState::new(Rng::next_seed(SEED), RunConfig::default());
    fresh.tick = state.tick;
    assert_eq!(state, fresh);
}

// --- scripted end-to-end run --------------------------------------------------------

/// Cell (`x`, `y`) of the floor, as a grid index.
fn index(tiles: Tiles<'_>, (x, y): (i32, i32)) -> Option<usize> {
    let (x, y) = (usize::try_from(x).ok()?, usize::try_from(y).ok()?);
    let (w, h) = tiles.ship.size();
    (x < w && y < h).then(|| y.saturating_mul(w).saturating_add(x))
}

const fn neighbors((x, y): (i32, i32)) -> [(i32, i32); 4] {
    [
        (x.saturating_add(1), y),
        (x.saturating_sub(1), y),
        (x, y.saturating_add(1)),
        (x, y.saturating_sub(1)),
    ]
}

/// Where the scripted player may step: anywhere a player can go (through closed and open
/// hatches) except pits.
fn passable(tiles: Tiles<'_>, (x, y): (i32, i32)) -> bool {
    !tiles.blocks(x, y, Body::Player) && tiles.ship.cell(x, y) != Cell::Pit
}

/// The first step of a shortest 4-connected walk from `from` to `to` across the floor
/// (`to` itself once there). A breadth-first fill out from `to`, then downhill.
fn next_cell(tiles: Tiles<'_>, from: (i32, i32), to: (i32, i32)) -> Option<(i32, i32)> {
    let (w, h) = tiles.ship.size();
    let mut dist: Vec<Option<u32>> = vec![None; w.saturating_mul(h)];
    *dist.get_mut(index(tiles, to)?)? = Some(0);
    let mut queue = VecDeque::from([(to, 0_u32)]);
    while let Some((cell, d)) = queue.pop_front() {
        for n in neighbors(cell) {
            if !passable(tiles, n) {
                continue;
            }
            if let Some(slot) = index(tiles, n).and_then(|i| dist.get_mut(i))
                && slot.is_none()
            {
                *slot = Some(d.saturating_add(1));
                queue.push_back((n, d.saturating_add(1)));
            }
        }
    }
    let at = |c| index(tiles, c).and_then(|i| dist.get(i).copied().flatten());
    if from == to {
        return Some(to);
    }
    neighbors(from)
        .into_iter()
        .filter_map(|n| at(n).map(|d| (d, n)))
        .min()
        .map(|(_, n)| n)
}

/// Whether a shot from `from` to `to` clears every wall (sampled every 4 pt).
fn clear_shot(tiles: Tiles<'_>, from: FxVec2, to: FxVec2) -> bool {
    let steps = 64;
    (1..steps).all(|i| {
        let at = |a: sim::Fx, b: sim::Fx| {
            a.saturating_add(
                b.saturating_sub(a)
                    .saturating_mul_int(i)
                    .checked_div_int(steps)
                    .unwrap_or_default(),
            )
        };
        let p = FxVec2 {
            x: at(from.x, to.x),
            y: at(from.y, to.y),
        };
        !tiles.blocks_point(p, Body::Shot)
    })
}

fn cell(p: FxVec2) -> (i32, i32) {
    (cell_of(p.x), cell_of(p.y))
}

/// Manhattan distance in cells.
const fn cells_apart(a: (i32, i32), b: (i32, i32)) -> u32 {
    a.0.abs_diff(b.0).saturating_add(a.1.abs_diff(b.1))
}

/// Stick input toward `to`: full deflection, eased near it so a run step (7 pt at full
/// deflection) doesn't overshoot.
fn stick_toward(from: FxVec2, to: FxVec2) -> PlayerInput {
    let offset = FxVec2 {
        x: to.x.saturating_sub(from.x),
        y: to.y.saturating_sub(from.y),
    };
    let Some(angle) = trig::angle_of(offset) else {
        return PlayerInput::default();
    };
    // Nearest of the 32 move buckets (2048 angle units each).
    let bucket = (u32::from(angle).saturating_add(1024) >> 11) & u32::from(MOVE_BUCKETS - 1);
    let reach = offset.x.abs().max(offset.y.abs());
    let mag = reach
        .saturating_mul_int(255)
        .checked_div_int(7)
        .map_or(255, |m| m.saturating_to_num::<u32>().clamp(1, 255));
    PlayerInput {
        move_dir: u8::try_from(bucket).unwrap_or(0),
        move_mag: u8::try_from(mag).unwrap_or(u8::MAX),
        ..PlayerInput::default()
    }
}

/// Where to head to step from cell `here` into its neighbor `next`: first line up with
/// `here`'s center across the step (a 28 pt body in a 32 pt cell has 2 pt of slack, and
/// the quantized stick can't creep that little sideways), then `next`'s center.
fn waypoint(pos: FxVec2, here: (i32, i32), next: (i32, i32)) -> Option<FxVec2> {
    let center = |(x, y): (i32, i32)| {
        Some(cell_center(
            usize::try_from(x).ok()?,
            usize::try_from(y).ok()?,
        ))
    };
    let (mid, to) = (center(here)?, center(next)?);
    let off = |a: sim::Fx, b: sim::Fx| a.saturating_sub(b).abs() > sim::Fx::ONE;
    Some(if next.0 != here.0 && off(pos.y, mid.y) {
        FxVec2 { x: pos.x, y: mid.y }
    } else if next.1 != here.1 && off(pos.x, mid.x) {
        FxVec2 { x: mid.x, y: pos.y }
    } else {
        to
    })
}

/// Where the scripted player heads between fights: into the first uncleared room with
/// enemies (its first placement), the bridge last, then out the nearest airlock.
fn next_goal(state: &SimState, here: (i32, i32)) -> Option<(i32, i32)> {
    let next = (0..)
        .map(RoomId)
        .zip(state.ship.rooms())
        .filter(|&(id, r)| r.room.has_enemies() && !state.cleared(id))
        .min_by_key(|(_, r)| r.room.category == Category::Boss)
        .and_then(|(_, r)| {
            let first = r.room.base.first()?;
            Some((
                first.x.saturating_add(r.at.0),
                first.y.saturating_add(r.at.1),
            ))
        });
    let cell = |(x, y): (usize, usize)| Some((i32::try_from(x).ok()?, i32::try_from(y).ok()?));
    next.map_or_else(
        || {
            outer_hatches(state)
                .into_iter()
                .filter_map(|(at, _)| cell(at))
                .min_by_key(|&at| cells_apart(at, here))
        },
        cell,
    )
}

/// One tick of the scripted player: in a fight, walk toward the nearest active enemy
/// until within ~3 cells with a clear shot, auto-firing throughout; between fights, walk
/// to the next room to clear (see [`next_goal`]).
fn scripted_input(state: &SimState) -> Option<PlayerInput> {
    let player = state.players[0]?;
    let tiles = state.tiles();
    let here = cell(player.pos);
    let goal = match state.run {
        Run::Encounter { .. } => {
            let nearest = state
                .enemies
                .iter()
                .filter(|(_, e)| e.active())
                .map(|(_, e)| e.pos)
                .min_by_key(|&e| cells_apart(cell(e), here));
            // Pillars can block the auto-aimed shot: then close in regardless.
            nearest
                .filter(|&e| cells_apart(cell(e), here) > 3 || !clear_shot(tiles, player.pos, e))
                .map(cell)
        }
        Run::Boarding => next_goal(state, here),
        Run::Dead { .. } | Run::Won => return None,
    };
    let mut input = goal
        .and_then(|goal| next_cell(tiles, here, goal))
        .and_then(|next| waypoint(player.pos, here, next))
        .map_or_else(PlayerInput::default, |to| stick_toward(player.pos, to));
    if matches!(state.run, Run::Encounter { .. }) {
        // Held fire: the pistol auto-vents whenever it runs dry.
        input.buttons = Buttons::FIRE | Buttons::AUTO_AIM;
    } else if player.gun.charges < state.config.tuning.charges {
        // Between fights, top up before the next room.
        input.buttons = Buttons::VENT;
    }
    Some(input)
}

/// Plays whole Corvettes on Normal with the scripted player and a test-only cheat: slot 0
/// is topped back up to full HP every tick, so the run can't die. It fights through the
/// phase pistol's vents and vents manually between rooms. The seeds' ships hold every
/// room in the pool between them, and board through different airlocks.
#[test]
fn a_scripted_player_clears_every_room_kills_the_captain_and_escapes() {
    for seed in [1, 3] {
        let mut state = SimState::new(seed, RunConfig::default());
        assert_eq!(state.config.difficulty, Difficulty::Normal);
        let mut events = Vec::new();
        let (mut auto_vents, mut manual_vents) = (0, 0);
        let budget = 60 * 60 * 5; // 5 minutes of play
        for _ in 0..budget {
            if let Some(player) = &mut state.players[0] {
                player.hp = MAX_HP;
            }
            let Some(input) = scripted_input(&state) else {
                break;
            };
            let mut inputs = TickInputs::default();
            inputs.players[0] = input;
            let was_venting = state.players[0].is_some_and(|p| p.gun.venting());
            events.extend(step(&mut state, &inputs).events);
            if !was_venting && state.players[0].is_some_and(|p| p.gun.venting()) {
                if input.buttons.contains(Buttons::VENT) {
                    manual_vents += 1;
                } else {
                    auto_vents += 1;
                }
            }
        }
        let cleared: Vec<RoomId> = events
            .iter()
            .filter_map(|e| match e {
                Event::RoomCleared { room } => Some(*room),
                _ => None,
            })
            .collect();
        let kills = events
            .iter()
            .filter(|e| matches!(e, Event::EnemyKilled { .. }))
            .count();
        let hits = events
            .iter()
            .filter(|e| matches!(e, Event::PlayerHit { .. }))
            .count();
        let falls = events
            .iter()
            .filter(|e| matches!(e, Event::PlayerFell { .. }))
            .count();
        println!(
            "seed {seed}: won after {} ticks: cleared {cleared:?}, {kills} kills, took {hits} \
         hits, {falls} falls, {auto_vents} auto vents, {manual_vents} manual vents",
            state.tick
        );
        // It paths around pits (as enemies do), so the HP top-up never hides a fall.
        assert_eq!(falls, 0);
        assert!(
            auto_vents >= 5 && manual_vents >= 1,
            "{auto_vents} {manual_vents}"
        );
        assert_eq!(state.run, Run::Won, "{cleared:?}");
        let ship = &state.ship;
        let fights: Vec<RoomId> = (0..)
            .map(RoomId)
            .zip(ship.rooms())
            .filter_map(|(id, r)| r.room.has_enemies().then_some(id))
            .collect();
        let mut sorted = cleared.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, fights, "every fight, once");
        assert_eq!(cleared.last(), Some(&bridge(ship)), "the bridge last");
        let placed: usize = ship
            .rooms()
            .iter()
            .map(|r| r.room)
            .flat_map(|r| {
                std::iter::once(r.base).chain(r.reinforcements.iter().map(|l| l.placements))
            })
            .map(<[_]>::len)
            .sum();
        assert_eq!(kills, placed, "every placed enemy died");
        assert_eq!(events.last(), Some(&Event::Won));
    }
}
