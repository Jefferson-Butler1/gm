//! The whole slice: extraction -> `Run::Won`, restart from Won, and a scripted player that
//! fights through every room of the derelict to the win.

use sim::room::{Body, Category, Tiles, cell_center, cell_of};
use sim::{
    Buttons, DERELICT, Event, FxVec2, MAX_HP, MOVE_BUCKETS, PlayerInput, Rng, RoomId, Run,
    RunConfig, SimState, TickInputs, step, trig,
};
use std::collections::VecDeque;

const SEED: u64 = 11;

fn exit_room() -> RoomId {
    let index = DERELICT
        .rooms
        .iter()
        .position(|r| r.category == Category::Exit)
        .unwrap_or_default();
    RoomId(u16::try_from(index).unwrap_or_default())
}

/// A fresh run moved to the exit room, slot 0 standing on its extraction pad.
fn on_the_pad(run: Run) -> SimState {
    let mut state = SimState::new(SEED, RunConfig::default());
    state.run = run;
    let (x, y) = DERELICT
        .room(exit_room())
        .and_then(|r| r.extraction)
        .unwrap_or_default();
    if let Some(player) = &mut state.players[0] {
        player.pos = cell_center(x, y);
    }
    state
}

#[test]
fn the_extraction_pad_wins_only_once_the_exit_room_is_clear() {
    let room = exit_room();
    // Mid-fight (a wave still telegraphing in), the pad does nothing.
    let mut fighting = on_the_pad(Run::Encounter {
        room,
        wave: 1,
        doors_locked: true,
    });
    fighting
        .enemies
        .insert(sim::Enemy::rusher(cell_center(2, 2)));
    let events = step(&mut fighting, &TickInputs::default()).events;
    assert!(!events.contains(&Event::Won), "{events:?}");
    assert!(matches!(fighting.run, Run::Encounter { .. }));

    let mut clear = on_the_pad(Run::Boarding { room });
    let events = step(&mut clear, &TickInputs::default()).events;
    assert_eq!(events, [Event::Won]);
    assert_eq!(clear.run, Run::Won { room });

    // The world freezes until restart.
    let frozen = clear.clone();
    let mut walk = TickInputs::default();
    walk.players[0].move_mag = u8::MAX;
    assert!(step(&mut clear, &walk).events.is_empty());
    assert_eq!(clear.players, frozen.players);
}

#[test]
fn restart_from_won_starts_a_fresh_run() {
    let mut state = on_the_pad(Run::Won { room: exit_room() });
    state.cleared = 0b11110;
    let mut restart = TickInputs::default();
    restart.players[0].buttons = Buttons::RESTART;
    assert_eq!(step(&mut state, &restart).events, [Event::Restarted]);
    let mut fresh = SimState::new(Rng::next_seed(SEED), RunConfig::default());
    fresh.tick = state.tick;
    assert_eq!(state, fresh);
}

// --- scripted end-to-end run --------------------------------------------------------

/// Walkable cell (`x`, `y`) of `tiles`, as a grid index.
fn index(tiles: Tiles, (x, y): (i32, i32)) -> Option<usize> {
    let (x, y) = (usize::try_from(x).ok()?, usize::try_from(y).ok()?);
    let (w, h) = (tiles.room.width(), tiles.room.height());
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

/// The first step of a shortest 4-connected walk from `from` to `to` through `tiles`
/// (`to` itself once there). A breadth-first fill out from `to`, then downhill.
fn next_cell(tiles: Tiles, from: (i32, i32), to: (i32, i32)) -> Option<(i32, i32)> {
    let size = tiles.room.width().saturating_mul(tiles.room.height());
    let mut dist: Vec<Option<u32>> = vec![None; size];
    *dist.get_mut(index(tiles, to)?)? = Some(0);
    let mut queue = VecDeque::from([(to, 0_u32)]);
    while let Some((cell, d)) = queue.pop_front() {
        for n in neighbors(cell) {
            if tiles.blocks(n.0, n.1, Body::Walker) {
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

/// One tick of the scripted player: in a fight, walk toward the nearest active enemy
/// until within ~3 cells, auto-firing throughout; once the room is clear, walk to the
/// extraction pad, else to the exit into the next room.
fn scripted_input(state: &SimState) -> Option<PlayerInput> {
    let player = state.players[0]?;
    let tiles = state.tiles()?;
    let here = cell(player.pos);
    let goal = match state.run {
        Run::Encounter { .. } => {
            let nearest = state
                .enemies
                .iter()
                .filter(|(_, e)| e.active())
                .map(|(_, e)| e.pos)
                .min_by_key(|&e| cells_apart(cell(e), here));
            nearest.map(cell).filter(|&c| cells_apart(c, here) > 3)
        }
        Run::Boarding { room } => tiles.room.extraction.map_or_else(
            || {
                let next = RoomId(room.0.saturating_add(1));
                let exit = (0..tiles.room.exits.len())
                    .find(|&e| DERELICT.link(room, e).is_some_and(|(to, _)| to == next))?;
                let gap = tiles.room.exits.get(exit)?;
                Some((i32::try_from(gap.x).ok()?, i32::try_from(gap.y).ok()?))
            },
            |(x, y)| Some((i32::try_from(x).ok()?, i32::try_from(y).ok()?)),
        ),
        Run::Dead { .. } | Run::Won { .. } => return None,
    };
    let mut input = goal
        .and_then(|goal| next_cell(tiles, here, goal))
        .and_then(|next| waypoint(player.pos, here, next))
        .map_or_else(PlayerInput::default, |to| stick_toward(player.pos, to));
    if matches!(state.run, Run::Encounter { .. }) {
        input.buttons = Buttons::FIRE | Buttons::AUTO_AIM;
    }
    Some(input)
}

/// Plays the whole derelict with the scripted player and a test-only cheat: slot 0 is
/// topped back up to full HP every tick, so the run can't die.
#[test]
fn a_scripted_player_clears_every_room_and_extracts() {
    let mut state = SimState::new(SEED, RunConfig::default());
    let mut events = Vec::new();
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
        events.extend(step(&mut state, &inputs).events);
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
    println!(
        "won after {} ticks: cleared {cleared:?}, {kills} kills, took {hits} hits",
        state.tick
    );
    assert_eq!(state.run, Run::Won { room: exit_room() }, "{cleared:?}");
    assert_eq!(cleared, [RoomId(1), RoomId(2), RoomId(3), RoomId(4)]);
    let placed: usize = DERELICT
        .rooms
        .iter()
        .flat_map(|r| std::iter::once(r.base).chain(r.reinforcements.iter().map(|l| l.placements)))
        .map(<[_]>::len)
        .sum();
    assert_eq!(kills, placed, "every placed enemy died");
    assert_eq!(events.last(), Some(&Event::Won));
}
