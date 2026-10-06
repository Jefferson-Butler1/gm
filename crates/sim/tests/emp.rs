//! EMPs: thrown off a swipe or set off at the feet; clearing their room's enemy bullets,
//! stunning its enemies, and hitting and shoving those in the blast. Laid out in the
//! cargo hold at `SEED`, as in the awareness tests: hold cell (0, 0) is floor cell
//! `HOLD_AT`, with a pillar at hold cells (5..=6, 3..=4).

use sim::room::cell_center;
use sim::{
    BLAST_RADIUS, Bullet, Buttons, EMP_START, Enemy, Event, FxVec2, PlayerInput, RUSHER_HP, RoomId,
    RunConfig, STUN_TICKS, SimState, THROW_TICKS, TickInputs, step,
};

const SEED: u64 = 3229;
const CARGO_HOLD: RoomId = RoomId(4);
const HOLD_AT: (usize, usize) = (46, 12);
/// Move bucket for straight right (of 32).
const RIGHT: u8 = 0;

fn at((x, y): (usize, usize)) -> FxVec2 {
    cell_center(x.saturating_add(HOLD_AT.0), y.saturating_add(HOLD_AT.1))
}

/// Slot 0 alone in the cleared hold at hold cell `cell`, so no fight starts.
fn hold(cell: (usize, usize)) -> SimState {
    let mut state = SimState::new(SEED, RunConfig::default());
    assert_eq!(
        state.ship.room(CARGO_HOLD).map(|r| (r.room.name, r.at)),
        Some(("cargo hold", HOLD_AT))
    );
    state.cleared |= 1 << CARGO_HOLD.0;
    if let Some(player) = &mut state.players[0] {
        player.pos = at(cell);
        player.solid = player.pos;
    }
    state
}

/// An active rusher at hold cell `cell`, hunting slot 0.
fn rusher(state: &mut SimState, cell: (usize, usize)) -> sim::EnemyId {
    let last_seen = state.players[0].map(|p| p.pos).unwrap_or_default();
    state.enemies.insert(Enemy {
        spawn_ticks: 0,
        awareness: sim::Awareness::Alert {
            last_seen,
            searching: 0,
        },
        ..Enemy::rusher(at(cell))
    })
}

fn press(throw: Option<u8>) -> TickInputs {
    let mut inputs = TickInputs::default();
    inputs.players[0] = PlayerInput {
        throw,
        buttons: Buttons::EMP,
        ..PlayerInput::default()
    };
    inputs
}

fn emps(state: &SimState) -> u8 {
    state.players[0].map_or(0, |p| p.emps)
}

#[test]
fn a_tap_goes_off_at_the_feet_clearing_the_rooms_bullets_and_stunning_its_enemies() {
    let mut state = hold((2, 10));
    // In the blast: 1 cell right. Across the hold, out of it: stunned only.
    let near = rusher(&mut state, (3, 10));
    let far = rusher(&mut state, (20, 10));
    let bullet = |cell| Bullet {
        pos: at(cell),
        vel: FxVec2::default(),
        ticks_left: 100,
        from: None,
    };
    state.enemy_bullets.insert(bullet((18, 2)));
    state.enemy_bullets.insert(bullet((10, 6)));
    let far_at = state.enemies.get(far).unwrap().pos;

    let events = step(&mut state, &press(None)).events;
    assert!(events.contains(&Event::EmpThrown { slot: 0 }));
    assert!(events.contains(&Event::EmpDetonated { pos: at((2, 10)) }));
    assert_eq!(emps(&state), EMP_START - 1);
    assert!(state.enemy_bullets.is_empty(), "the whole room's bullets");
    let near = state.enemies.get(near).unwrap();
    assert!(near.hp < RUSHER_HP, "hit by the blast");
    assert!(near.pos.x > at((3, 10)).x, "shoved out from the blast");
    let far_e = state.enemies.get(far).unwrap();
    assert_eq!(far_e.hp, RUSHER_HP, "out of the blast");
    assert!(far_e.stun_ticks > 0);

    // Stunned, it stands still and can't hurt; then it hunts again.
    let idle = TickInputs::default();
    for _ in 1..STUN_TICKS {
        step(&mut state, &idle);
        assert_eq!(state.enemies.get(far).unwrap().pos, far_at);
    }
    for _ in 0..30 {
        step(&mut state, &idle);
    }
    assert_ne!(state.enemies.get(far).unwrap().pos, far_at, "hunting again");
}

#[test]
fn a_swipe_throws_it_three_cells_where_it_goes_off_and_none_left_does_nothing() {
    let mut state = hold((1, 10));
    let events = step(&mut state, &press(Some(RIGHT))).events;
    assert!(events.contains(&Event::EmpThrown { slot: 0 }));
    assert_eq!(state.emps.len(), 1, "in flight");
    let mut landed = None;
    for _ in 0..THROW_TICKS {
        let events = step(&mut state, &TickInputs::default()).events;
        landed = landed.or_else(|| {
            events.iter().find_map(|e| match *e {
                Event::EmpDetonated { pos } => Some(pos),
                _ => None,
            })
        });
    }
    let landed = landed.expect("went off");
    // To within trig rounding.
    let close = |a: sim::Fx, b: sim::Fx| (a - b).abs() < sim::Fx::from_num(0.01);
    assert!(close(landed.y, at((1, 10)).y), "{landed:?}");
    assert!(close(landed.x, at((4, 10)).x), "3 cells on: {landed:?}");
    assert!(state.emps.is_empty());

    step(&mut state, &press(None));
    assert_eq!(emps(&state), 0);
    let events = step(&mut state, &press(None)).events;
    assert!(events.is_empty(), "{events:?}");
}

#[test]
fn a_throw_stops_and_goes_off_at_the_first_wall() {
    // Two cells left of the pillar (hold cells 5..=6, 3..=4), throwing right.
    let mut state = hold((3, 3));
    step(&mut state, &press(Some(RIGHT)));
    let mut landed = None;
    for _ in 0..THROW_TICKS {
        let events = step(&mut state, &TickInputs::default()).events;
        landed = landed.or_else(|| {
            events.iter().find_map(|e| match *e {
                Event::EmpDetonated { pos } => Some(pos),
                _ => None,
            })
        });
    }
    let landed = landed.expect("went off");
    assert!(
        landed.x < at((5, 3)).x - BLAST_RADIUS / 4,
        "short of the pillar"
    );
    assert!(landed.x > at((4, 3)).x - BLAST_RADIUS / 4);
}
