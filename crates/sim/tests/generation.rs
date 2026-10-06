//! Generated ships (issue #35): hull templates are checked when the crate builds, and a
//! seed sweep checks the stamped grid itself, independently of how it was built.

use serde::Deserialize;
use serde::de::value::{Error, SeqDeserializer};
use sim::hull::{Slot, Template, TemplateError, Zone};
use sim::room::{Category, Cell, Dir, MAX_ROOMS, Size, cell_of};
use sim::ship::{Hatch, HatchKind, Spot};
use sim::{CLASSES, POOL, RoomId, Ship};
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

/// The airlocks' outer hatches.
fn outer_hatches(ship: &Ship) -> impl Iterator<Item = &Hatch> {
    ship.hatches()
        .iter()
        .filter(|h| h.kind == HatchKind::Airlock)
}

/// Everything wrong with `ship`, checked on its grid alone: it is walled in but for the
/// airlocks' outer hatches, every room is walkable to from the start (hatches passable,
/// shot-out panels too, and pits, as rolls cross them), the party starts on floor in an
/// airlock, there's a bridge (the boss room), panels lead exactly into crawlspaces, and
/// every chest sits on reachable floor.
fn problems(ship: &Ship) -> Vec<String> {
    let mut problems = Vec::new();
    let open = |(x, y)| !matches!(ship.cell(x, y), Cell::Wall | Cell::Void);
    let outer = |(x, y)| match ship.spot(x, y) {
        Spot::Hatch(id) => ship
            .hatches()
            .get(usize::from(id.0))
            .is_some_and(|h| h.kind == HatchKind::Airlock),
        Spot::Void | Spot::Room { .. } => false,
    };
    if ship.rooms().len() > MAX_ROOMS {
        problems.push(format!("{} rooms", ship.rooms().len()));
    }
    for at in cells(ship) {
        let to_space = sides(at)
            .iter()
            .filter(|&&(x, y)| ship.spot(x, y) == Spot::Void)
            .count();
        // An outer hatch opens onto space on exactly one side: its outward one.
        let expected = usize::from(outer(at));
        if open(at) && to_space != expected {
            problems.push(format!(
                "{at:?} is open to outside the ship {to_space} ways"
            ));
        }
    }
    let (start_room, start) = ship.start();
    let at = (cell_of(start.x), cell_of(start.y));
    let airlock_rooms: BTreeSet<RoomId> = outer_hatches(ship).map(|h| h.rooms[0]).collect();
    if ship.room(start_room).map(|r| r.room.category) != Some(Category::Entrance)
        || !airlock_rooms.contains(&start_room)
        || ship.room_at(start) != Some(start_room)
        || ship.cell(at.0, at.1) != Cell::Floor
    {
        problems.push("the start is not on an airlock's floor".into());
    }
    for hatch in outer_hatches(ship) {
        let [a, b] = hatch.rooms;
        if a != b || ship.room(a).map(|r| r.room.category) != Some(Category::Entrance) {
            problems.push(format!("outer hatch {hatch:?} is not an airlock's"));
        }
    }
    if !ship
        .rooms()
        .iter()
        .any(|r| r.room.category == Category::Boss)
    {
        problems.push("no bridge".into());
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
        if (a == b) != (hatch.kind == HatchKind::Airlock)
            || ship.room(a).is_none()
            || ship.room(b).is_none()
        {
            problems.push(format!("hatch {i} joins {a:?} to {b:?}"));
        }
        // Panels (walkable in the flood above, as once shot) lead into crawlspaces.
        let secret = ship.room(a).map(|r| r.room.category) == Some(Category::Secret);
        if (hatch.kind == HatchKind::Panel) != secret {
            problems.push(format!("hatch {i} is {:?} into {a:?}", hatch.kind));
        }
    }
    for id in (0..).map(RoomId).take(ship.rooms().len()) {
        let chest = ship
            .chest(id)
            .and_then(|(x, y)| Some((i32::try_from(x).ok()?, i32::try_from(y).ok()?)));
        if let Some(at) = chest
            && (ship.cell(at.0, at.1) != Cell::Floor || !seen.contains(&at))
        {
            problems.push(format!("room {}'s chest at {at:?} is out of reach", id.0));
        }
    }
    problems
}

/// The name of the room filling `slot` of `class` in `ship`, if it's filled.
fn filling(class: &Template, ship: &Ship, slot: &Slot) -> Option<&'static str> {
    let at = class.origin(slot.letter)?;
    ship.rooms()
        .iter()
        .find(|r| r.at == at && r.room.category != Category::Connector)
        .map(|r| r.room.name)
}

/// For each ship class, 10,000 seeds: every ship is walled in and joins up, with its 3
/// airlocks, its bridge, its crawlspace (behind a panel) and stores reachable, and a chest
/// in each of those last two; and the sweep sees every slot take every room that fits it,
/// every optional slot both filled and left out, and the party board through each
/// airlock. A ship's checksum covers all its static data, so each distinct one is checked
/// once. About 15 s in debug, a couple in release.
#[test]
fn every_seed_yields_a_fully_connected_ship_of_each_class_and_every_fill_turns_up() {
    for class in CLASSES {
        sweep(class);
    }
}

fn sweep(class: &Template) {
    let name = class.name;
    let mut seen: BTreeMap<u8, BTreeSet<Option<&str>>> = BTreeMap::new();
    let mut boarded = BTreeSet::new();
    let mut layouts = BTreeSet::new();
    for seed in 0..10_000 {
        let ship = Ship::generate(class, seed);
        if !layouts.insert(ship.checksum()) {
            continue;
        }
        let problems = problems(&ship);
        assert!(
            problems.is_empty(),
            "{name} seed {seed}: {problems:?}\n{}",
            draw(&ship)
        );
        assert_eq!(outer_hatches(&ship).count(), 3, "{name} seed {seed}");
        let panels = ship.hatches().iter().filter(|h| h.kind == HatchKind::Panel);
        assert_eq!(panels.count(), 1, "{name} seed {seed}: one crawlspace");
        let chests = (0..).map(RoomId).take(ship.rooms().len());
        assert_eq!(
            chests.filter_map(|id| ship.chest(id)).count(),
            2,
            "{name} seed {seed}"
        );
        boarded.insert(ship.room(ship.start().0).map(|r| r.at));
        for slot in class.slots {
            seen.entry(slot.letter)
                .or_default()
                .insert(filling(class, &ship, slot));
        }
    }
    for slot in class.slots {
        let mut expected: BTreeSet<Option<&str>> = POOL
            .iter()
            .filter(|room| slot.fits(room))
            .map(|room| Some(room.name))
            .collect();
        if slot.optional {
            expected.insert(None);
        }
        assert_eq!(
            seen.get(&slot.letter),
            Some(&expected),
            "{name} slot {}",
            char::from(slot.letter)
        );
    }
    let airlocks: BTreeSet<_> = (class.slots.iter())
        .filter(|slot| slot.airlock.is_some())
        .map(|slot| class.origin(slot.letter))
        .collect();
    assert_eq!(boarded, airlocks, "{name}: boarded through every airlock");
    println!("{} different {name}s", layouts.len());
}

#[test]
fn runs_board_every_ship_class() {
    let boarded: BTreeSet<_> = (0..100).map(|seed| Template::for_seed(seed).name).collect();
    let classes: BTreeSet<_> = CLASSES.iter().map(|class| class.name).collect();
    assert_eq!(boarded, classes);
}

#[test]
fn a_seed_always_generates_the_same_ship_and_a_peer_rebuilds_it_from_seed_and_checksum() {
    let ship = Ship::generate(Template::for_seed(42), 42);
    assert_eq!(
        ship.checksum(),
        Ship::generate(Template::for_seed(42), 42).checksum()
    );
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

/// An airlock (`A`, its outer hatch west) and a bridge (`B`) joined by a walled corridor:
/// about the smallest valid ship.
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
    airlock: Some(Dir::West),
};

const BRIDGE: Slot = Slot {
    letter: b'B',
    size: Size::L,
    zone: Zone::Fore,
    optional: false,
    hatches: &[Dir::West],
    airlock: None,
};

/// [`TWO_ROOMS`], with row `y` replaced if `edit` is `Some((y, row))`, and the airlock's
/// and bridge's legends.
fn two_rooms(edit: Option<(usize, &'static str)>, airlock: Slot, bridge: Slot) -> Template {
    let mut rows = TWO_ROOMS;
    if let Some((y, row)) = edit
        && let Some(old) = rows.get_mut(y)
    {
        *old = row;
    }
    Template {
        name: "two rooms",
        rows: Box::leak(Box::new(rows)),
        slots: Box::leak(Box::new([airlock, bridge])),
    }
}

#[test]
fn templates_are_rejected_when_slots_hatches_or_walls_are_malformed() {
    let valid = two_rooms(None, AIRLOCK, BRIDGE);
    assert_eq!(valid.validate(), Ok(()));
    let ship = Ship::generate(&valid, 0);
    assert!(problems(&ship).is_empty(), "{}", draw(&ship));

    let with = |airlock| two_rooms(None, airlock, BRIDGE).validate();
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
    let optional_airlock = with(Slot {
        optional: true,
        ..AIRLOCK
    });
    assert_eq!(optional_airlock, Err(TemplateError::BadAirlock { slot: 0 }));
    let outer_on_corridor = with(Slot {
        airlock: Some(Dir::East),
        ..AIRLOCK
    });
    assert_eq!(
        outer_on_corridor,
        Err(TemplateError::BadHatches { slot: 0 })
    );
    let no_airlock = with(Slot {
        airlock: None,
        ..AIRLOCK
    });
    assert_eq!(no_airlock, Err(TemplateError::NoAirlock));
    let north = Slot {
        airlock: Some(Dir::North),
        ..AIRLOCK
    };
    assert_eq!(two_rooms(None, north, BRIDGE).validate(), Ok(()));
    let hulled = two_rooms(
        Some((1, "     ##       BBBBBBBBBBBBBBBBBBBBBBBB")),
        north,
        BRIDGE,
    );
    assert_eq!(
        hulled.validate(),
        Err(TemplateError::AirlockOffHull { slot: 0 }),
        "hull beyond the outer hatch"
    );

    let open = two_rooms(
        Some((5, "AAAAAAAAAAAA  BBBBBBBBBBBBBBBBBBBBBBBB")),
        AIRLOCK,
        BRIDGE,
    );
    assert_eq!(
        open.validate(),
        Err(TemplateError::OpenCorridor { x: 12, y: 6 })
    );
    let stray = two_rooms(
        Some((0, "#             BBBBBBBBBBBBBBBBBBBBBBBB")),
        AIRLOCK,
        BRIDGE,
    );
    assert_eq!(
        stray.validate(),
        Err(TemplateError::StrayHull { x: 0, y: 0 })
    );
}
