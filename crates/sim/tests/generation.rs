//! Generated ships (issue #35): hull templates are checked when the crate builds, and a
//! seed sweep checks the stamped grid itself, independently of how it was built.

use serde::Deserialize;
use serde::de::value::{Error, SeqDeserializer};
use sim::hull::{Slot, Template, TemplateError, Zone};
use sim::room::{Category, Cell, Dir, MAX_ROOMS, Size, cell_of};
use sim::ship::Spot;
use sim::{CORVETTE, POOL, RoomId, Ship};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Every cell of `ship`'s grid, row-major.
fn cells(ship: &Ship) -> impl Iterator<Item = (i32, i32)> {
    let (width, height) = ship.size();
    let side = |n: usize| i32::try_from(n).unwrap_or(i32::MAX);
    let (width, height) = (side(width), side(height));
    (0..height).flat_map(move |y| (0..width).map(move |x| (x, y)))
}

const fn sides((x, y): (i32, i32)) -> [(i32, i32); 4] {
    [
        (x.saturating_add(1), y),
        (x.saturating_sub(1), y),
        (x, y.saturating_add(1)),
        (x, y.saturating_sub(1)),
    ]
}

/// The ship as text, for failure messages: `#` wall, `.` floor, `o` pit, `+` hatch.
fn draw(ship: &Ship) -> String {
    let mut text = String::new();
    for (x, y) in cells(ship) {
        if x == 0 {
            text.push('\n');
        }
        text.push(match ship.spot(x, y) {
            Spot::Void => ' ',
            Spot::Hatch(_) => '+',
            Spot::Room { cell, .. } => match cell {
                Cell::Floor => '.',
                Cell::Wall => '#',
                Cell::Pit => 'o',
                Cell::Void => '?',
            },
        });
    }
    text
}

/// Everything wrong with `ship`, checked on its grid alone: it is walled in, every room
/// is walkable to from the start (hatches passable, pits too, as rolls cross them), the
/// party starts on floor in an entrance, and an exit room has a pad.
fn problems(ship: &Ship) -> Vec<String> {
    let mut problems = Vec::new();
    let open = |(x, y)| !matches!(ship.cell(x, y), Cell::Wall | Cell::Void);
    if ship.rooms().len() > MAX_ROOMS {
        problems.push(format!("{} rooms", ship.rooms().len()));
    }
    for at in cells(ship) {
        if open(at)
            && sides(at)
                .iter()
                .any(|&(x, y)| ship.spot(x, y) == Spot::Void)
        {
            problems.push(format!("{at:?} is open to outside the ship"));
        }
    }
    let (start_room, start) = ship.start();
    let at = (cell_of(start.x), cell_of(start.y));
    if ship.room(start_room).map(|r| r.room.category) != Some(Category::Entrance)
        || ship.room_at(start) != Some(start_room)
        || ship.cell(at.0, at.1) != Cell::Floor
    {
        problems.push("the start is not on an entrance's floor".into());
    }
    if !ship
        .rooms()
        .iter()
        .any(|r| r.room.category == Category::Exit && r.room.extraction.is_some())
    {
        problems.push("no exit room with a pad".into());
    }
    let mut seen = BTreeSet::from([at]);
    let mut queue = VecDeque::from([at]);
    while let Some(at) = queue.pop_front() {
        for next in sides(at) {
            if open(next) && seen.insert(next) {
                queue.push_back(next);
            }
        }
    }
    let mut floored = BTreeSet::new();
    for at in cells(ship) {
        if let Spot::Room { room, .. } = ship.spot(at.0, at.1)
            && open(at)
        {
            floored.insert(room);
            if !seen.contains(&at) {
                problems.push(format!("{at:?} is unreachable"));
            }
        }
    }
    for id in (0..).map(RoomId).take(ship.rooms().len()) {
        if !floored.contains(&id) {
            problems.push(format!("room {} has no floor", id.0));
        }
    }
    for (i, hatch) in ship.hatches().iter().enumerate() {
        let [a, b] = hatch.rooms;
        if a == b || ship.room(a).is_none() || ship.room(b).is_none() {
            problems.push(format!("hatch {i} joins {a:?} to {b:?}"));
        }
    }
    problems
}

/// The name of the room filling `slot` in `ship`, if it's filled.
fn filling(ship: &Ship, slot: &Slot) -> Option<&'static str> {
    let at = CORVETTE.origin(slot.letter)?;
    ship.rooms()
        .iter()
        .find(|r| r.at == at && r.room.category != Category::Connector)
        .map(|r| r.room.name)
}

/// 10,000 seeds: every ship is walled in and joins up, and the sweep sees every slot
/// take every room that fits it, and every optional slot both filled and left out. A
/// ship's checksum covers all its static data, so each distinct one is checked once.
/// About 6 s in debug, under 1 s in release.
#[test]
fn every_seed_yields_a_fully_connected_corvette_and_every_fill_turns_up() {
    let mut seen: BTreeMap<u8, BTreeSet<Option<&str>>> = BTreeMap::new();
    let mut layouts = BTreeSet::new();
    for seed in 0..10_000 {
        let ship = Ship::generate(&CORVETTE, seed);
        if !layouts.insert(ship.checksum()) {
            continue;
        }
        let problems = problems(&ship);
        assert!(
            problems.is_empty(),
            "seed {seed}: {problems:?}\n{}",
            draw(&ship)
        );
        for slot in CORVETTE.slots {
            seen.entry(slot.letter)
                .or_default()
                .insert(filling(&ship, slot));
        }
    }
    for slot in CORVETTE.slots {
        let mut expected: BTreeSet<Option<&str>> = POOL
            .iter()
            .filter(|room| slot.fits(room))
            .map(|room| Some(room.name))
            .collect();
        if slot.optional {
            expected.insert(None);
        }
        assert_eq!(
            seen[&slot.letter],
            expected,
            "slot {}",
            char::from(slot.letter)
        );
    }
    println!("{} different Corvettes", layouts.len());
}

#[test]
fn a_seed_always_generates_the_same_ship_and_a_peer_rebuilds_it_from_seed_and_checksum() {
    let ship = Ship::generate(&CORVETTE, 42);
    assert_eq!(ship.checksum(), Ship::generate(&CORVETTE, 42).checksum());
    let load = |seed: u64, checksum: u64| {
        Ship::deserialize(SeqDeserializer::<_, Error>::new(
            [seed, checksum].into_iter(),
        ))
    };
    let loaded = load(42, ship.checksum()).unwrap();
    assert_eq!(draw(&loaded), draw(&ship));
    assert_eq!(loaded.rooms(), ship.rooms());
    assert!(load(43, ship.checksum()).is_err(), "another seed's ship");
}

// --- template validation ------------------------------------------------------------

/// An airlock (`A`) and a bridge (`B`) joined by a walled corridor: about the smallest
/// valid ship.
const TWO_ROOMS: [&str; 14] = [
    "              BBBBBBBBBBBBBBBBBBBBBBBB",
    "              BBBBBBBBBBBBBBBBBBBBBBBB",
    "AAAAAAAAAAAA  BBBBBBBBBBBBBBBBBBBBBBBB",
    "AAAAAAAAAAAA  BBBBBBBBBBBBBBBBBBBBBBBB",
    "AAAAAAAAAAAA  BBBBBBBBBBBBBBBBBBBBBBBB",
    "AAAAAAAAAAAA##BBBBBBBBBBBBBBBBBBBBBBBB",
    "AAAAAAAAAAAA==BBBBBBBBBBBBBBBBBBBBBBBB",
    "AAAAAAAAAAAA==BBBBBBBBBBBBBBBBBBBBBBBB",
    "AAAAAAAAAAAA##BBBBBBBBBBBBBBBBBBBBBBBB",
    "AAAAAAAAAAAA  BBBBBBBBBBBBBBBBBBBBBBBB",
    "AAAAAAAAAAAA  BBBBBBBBBBBBBBBBBBBBBBBB",
    "AAAAAAAAAAAA  BBBBBBBBBBBBBBBBBBBBBBBB",
    "              BBBBBBBBBBBBBBBBBBBBBBBB",
    "              BBBBBBBBBBBBBBBBBBBBBBBB",
];

const AIRLOCK: Slot = Slot {
    letter: b'A',
    size: Size::S,
    zone: Zone::Hull,
    optional: false,
    hatches: &[Dir::East],
};

const BRIDGE: Slot = Slot {
    letter: b'B',
    size: Size::L,
    zone: Zone::Fore,
    optional: false,
    hatches: &[Dir::West],
};

/// [`TWO_ROOMS`], with row `y` replaced if `edit` is `Some((y, row))`, and the airlock's
/// legend.
fn two_rooms(edit: Option<(usize, &'static str)>, airlock: Slot) -> Template {
    let mut rows = TWO_ROOMS;
    if let Some((y, row)) = edit
        && let Some(old) = rows.get_mut(y)
    {
        *old = row;
    }
    Template {
        name: "two rooms",
        rows: Box::leak(Box::new(rows)),
        slots: Box::leak(Box::new([airlock, BRIDGE])),
        start: b'A',
    }
}

#[test]
fn templates_are_rejected_when_slots_hatches_or_walls_are_malformed() {
    let valid = two_rooms(None, AIRLOCK);
    assert_eq!(valid.validate(), Ok(()));
    let ship = Ship::generate(&valid, 0);
    assert!(problems(&ship).is_empty(), "{}", draw(&ship));

    let with = |airlock| two_rooms(None, airlock).validate();
    let shape = with(Slot {
        size: Size::M,
        ..AIRLOCK
    });
    assert_eq!(shape, Err(TemplateError::SlotShape { slot: 0 }));
    let off_corridor = with(Slot {
        hatches: &[Dir::North],
        ..AIRLOCK
    });
    assert_eq!(
        off_corridor,
        Err(TemplateError::HatchOffCorridor { slot: 0 })
    );
    let twice = with(Slot {
        hatches: &[Dir::East, Dir::East],
        ..AIRLOCK
    });
    assert_eq!(twice, Err(TemplateError::BadHatches { slot: 0 }));
    let no_room = with(Slot {
        zone: Zone::Fore,
        ..AIRLOCK
    });
    assert_eq!(no_room, Err(TemplateError::EmptySlot { slot: 0 }));
    let optional_start = with(Slot {
        optional: true,
        ..AIRLOCK
    });
    assert_eq!(optional_start, Err(TemplateError::BadStart));

    let open = two_rooms(Some((5, "AAAAAAAAAAAA  BBBBBBBBBBBBBBBBBBBBBBBB")), AIRLOCK);
    assert_eq!(
        open.validate(),
        Err(TemplateError::OpenCorridor { x: 12, y: 6 })
    );
    let stray = two_rooms(Some((0, "#             BBBBBBBBBBBBBBBBBBBBBBBB")), AIRLOCK);
    assert_eq!(
        stray.validate(),
        Err(TemplateError::StrayHull { x: 0, y: 0 })
    );
}
