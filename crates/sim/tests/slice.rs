//! The whole slice: extraction -> `Run::Won`, restart from Won, and a scripted player that
//! walks whole generated ships, fighting through every room to the win.

use sim::room::{CELL, Category, Cell, cell_center, cell_of};
use sim::ship::{Body, Tiles};
use sim::{
    Buttons, Difficulty, Event, FxVec2, MAX_HP, MOVE_BUCKETS, PlayerInput, Rng, RoomId, Run,
    RunConfig, Ship, SimState, TickInputs, step, trig,
};
use std::collections::VecDeque;

const SEED: u64 = 11;

/// The Corvette's exit room: the bridge.
fn exit_room(ship: &Ship) -> RoomId {
    let index = ship
        .rooms()
        .iter()
        .position(|r| r.room.category == Category::Exit)
        .unwrap_or_default();
    RoomId(u16::try_from(index).unwrap_or_default())
}

/// The exit room's extraction pad, in floor cells.
fn pad(ship: &Ship) -> (usize, usize) {
    let placed = ship.room(exit_room(ship));
    let (x, y) = placed.and_then(|p| p.room.extraction).unwrap_or_default();
    let (ox, oy) = placed.map(|p| p.at).unwrap_or_default();
    (x.saturating_add(ox), y.saturating_add(oy))
}

/// A fresh run, slot 0 standing on the exit room's extraction pad.
fn on_the_pad(run: Run) -> SimState {
    let mut state = SimState::new(SEED, RunConfig::default());
    state.run = run;
    let (x, y) = pad(&state.ship);
    if let Some(player) = &mut state.players[0] {
        player.pos = cell_center(x, y);
    }
    state
}

#[test]
fn the_extraction_pad_wins_only_once_the_exit_room_is_clear() {
    let room = exit_room(&SimState::new(SEED, RunConfig::default()).ship);
    // Mid-fight (a wave still telegraphing in), the pad does nothing.
    let mut fighting = on_the_pad(Run::Encounter { room, wave: 1 });
    fighting
        .enemies
        .insert(sim::Enemy::rusher(cell_center(2, 2)));
    let events = step(&mut fighting, &TickInputs::default()).events;
    assert!(!events.contains(&Event::Won), "{events:?}");
    assert!(matches!(fighting.run, Run::Encounter { .. }));

    let mut clear = on_the_pad(Run::Boarding);
    clear.cleared = 1 << room.0;
    let events = step(&mut clear, &TickInputs::default()).events;
    assert_eq!(events, [Event::Won]);
    assert_eq!(clear.run, Run::Won);

    // The world freezes until restart.
    let frozen = clear.clone();
    let mut walk = TickInputs::default();
    walk.players[0].move_mag = u8::MAX;
    assert!(step(&mut clear, &walk).events.is_empty());
    assert_eq!(clear.players, frozen.players);
}

/// Walks the party east from the passage fore of the hold (every other room cleared)
/// through its hatch into the exit room's encounter.
fn enter_the_exit_room() -> SimState {
    let mut state = SimState::new(SEED, RunConfig::default());
    state.cleared = !(1 << exit_room(&state.ship).0);
    if let Some(player) = &mut state.players[0] {
        // In the passage's floor cells (56..=59, 19..=20), centered on its rows.
        player.pos = FxVec2 {
            x: cell_center(57, 19).x,
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
fn the_pad_stays_dead_while_the_exit_rooms_last_wave_lives() {
    let mut state = enter_the_exit_room();
    let room = exit_room(&state.ship);
    assert!(matches!(state.run, Run::Encounter { room: r, wave: 0, .. } if r == room));
    // The base wave dies: the last wave spawns.
    state.enemies.retain(|_, _| false);
    let events = step(&mut state, &TickInputs::default()).events;
    assert!(
        events.contains(&Event::WaveStarted { wave: 1 }),
        "{events:?}"
    );

    // Stand on the pad through the whole telegraph and beyond, unhurt.
    let (x, y) = pad(&state.ship);
    for _ in 0..120 {
        if let Some(player) = &mut state.players[0] {
            player.pos = cell_center(x, y);
            player.hp = MAX_HP;
        }
        let events = step(&mut state, &TickInputs::default()).events;
        assert!(!state.enemies.is_empty());
        assert!(!events.contains(&Event::Won), "{events:?}");
        assert!(
            matches!(state.run, Run::Encounter { wave: 1, .. }),
            "{:?}",
            state.run
        );
        assert!(!state.extraction_live(room));
    }
}

#[test]
fn restart_from_won_starts_a_fresh_run() {
    let mut state = on_the_pad(Run::Won);
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
/// enemies (its first placement), the exit room last, then to the extraction pad.
fn next_goal(state: &SimState) -> Option<(i32, i32)> {
    let next = (0..)
        .map(RoomId)
        .zip(state.ship.rooms())
        .filter(|&(id, r)| r.room.has_enemies() && !state.cleared(id))
        .min_by_key(|(_, r)| r.room.category == Category::Exit)
        .and_then(|(_, r)| {
            let first = r.room.base.first()?;
            Some((
                first.x.saturating_add(r.at.0),
                first.y.saturating_add(r.at.1),
            ))
        });
    let (x, y) = next.unwrap_or_else(|| pad(&state.ship));
    Some((i32::try_from(x).ok()?, i32::try_from(y).ok()?))
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
        Run::Boarding => next_goal(state),
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
/// room in the pool between them.
#[test]
fn a_scripted_player_clears_every_room_and_extracts() {
    for seed in [1, 5] {
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
        assert_eq!(cleared.last(), Some(&exit_room(ship)), "the bridge last");
        let placed: usize = ship
            .rooms()
            .iter()
            .map(|r| r.room)
            .flat_map(|r| {
                std::iter::once(r.base).chain(r.reinforcements.iter().map(|l| l.placements))
            })
            .map(<[_]>::len)
            .sum();
        // Every room and corridor was revealed on the way.
        assert!((0..ship.rooms().len()).all(|i| state.visited(RoomId(u16::try_from(i).unwrap()))));
        assert_eq!(kills, placed, "every placed enemy died");
        assert_eq!(events.last(), Some(&Event::Won));
    }
}
