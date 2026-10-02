//! The room pool (issue #28): every room a hull template can fill a slot with. Each room
//! is exactly one size class, with that class's four standard hatch points open, and a
//! theme that says which slots take it (see [`crate::hull`]).
//!
//! | Room          | Size | Theme       | Fight                                            |
//! |---------------|------|-------------|--------------------------------------------------|
//! | airlock       | S    | Airlock     | none: the entrance, to get your hands on things  |
//! | galley        | S    | Crew        | rushers and a shooter among the tables           |
//! | crew quarters | S    | Crew        | a shooter and rushers between the bunks          |
//! | engine room   | M    | Engineering | shooter-heavy, pillars to hide behind            |
//! | cargo hold    | L    | Cargo       | rushers with a shooter; a pillar and a pit strip |
//! | shuttle bay   | L    | Cargo       | a pit trench the shooters fire across            |
//! | bridge        | L    | Bridge      | two thick waves, then the captain; the boss room |
//! | crawlspace    | S    | Crawlspace  | none: behind an access panel, a chest            |
//! | stores        | S    | Stores      | none: the reward room, a chest                   |
//! | medbay        | S    | Crew        | circle-strafe the operating table; 2 waves       |
//! | hydroponics   | S    | Crew        | shooters behind pit troughs                      |
//! | cargo lift    | S    | Cargo       | a small ring round an open lift shaft            |
//! | reactor       | M    | Engineering | an arena ring round the core and its pit moat    |
//! | machine shop  | M    | Engineering | a chokepoint: one door into the shooters' bay    |
//! | coolant plant | M    | Engineering | pit channels and bridges; 3 waves                |
//! | sensor gallery | M    | Engineering | a long gallery for shooters, flank lanes         |
//! | service gantry | L    | Cargo       | the rare guarded hallway, corridor-shaped        |
//! | mess hall     | L    | Crew        | a pillar field; 3 waves from new directions      |
//! | container stacks | L    | Cargo       | staggered stacks: lanes, chokepoints, flanks     |
//! | war room      | M    | Bridge      | seat pillars; the Gunship's, before its bridge   |
//!
//! Spread shooters are the pattern experiment (issue #15); with it off they're shooters.
//!
//! Every room is validated when the crate builds, and the pool against every template.

use crate::room::{
    Category, EnemyKind, LayerTrigger, Placement, PrototypeRoom, Reinforcement, RoomAction,
    RoomTrigger, Size, Theme,
};

/// Seal on entry, unseal once the last wave dies: the standard combat room.
const LOCKDOWN: &[(RoomTrigger, RoomAction)] = &[
    (RoomTrigger::OnEnterWithEnemies, RoomAction::Seal),
    (RoomTrigger::OnEnemiesCleared, RoomAction::Unseal),
];

const fn rusher(x: usize, y: usize) -> Placement {
    Placement {
        kind: EnemyKind::Rusher,
        x,
        y,
    }
}

const fn shooter(x: usize, y: usize) -> Placement {
    Placement {
        kind: EnemyKind::Shooter,
        x,
        y,
    }
}

/// The pattern experiment's placements (issue #15): a plain shooter when it's off.
const fn spread_shooter(x: usize, y: usize) -> Placement {
    Placement {
        kind: EnemyKind::SpreadShooter,
        x,
        y,
    }
}

/// The bridge captain, a placeholder elite: it always arrives with the boss telegraph.
const fn captain(x: usize, y: usize) -> Placement {
    Placement {
        kind: EnemyKind::Captain,
        x,
        y,
    }
}

/// The next wave once the current one is dead.
const fn then(placements: &'static [Placement]) -> Reinforcement {
    Reinforcement {
        trigger: LayerTrigger::OnEnemiesCleared,
        placements,
    }
}

/// Empty: the party boards here.
const AIRLOCK: PrototypeRoom = PrototypeRoom {
    name: "airlock",
    category: Category::Entrance,
    theme: Theme::Airlock,
    cells: &[
        "#####..#####",
        "#..........#",
        "#..........#",
        "#..........#",
        "............",
        "............",
        "#..........#",
        "#..........#",
        "#..........#",
        "#####..#####",
    ],
    exits: Size::S.hatches(),
    base: &[],
    reinforcements: &[],
    events: &[],
}
.valid();

/// Two rows of tables.
const GALLEY: PrototypeRoom = PrototypeRoom {
    name: "galley",
    category: Category::Normal,
    theme: Theme::Crew,
    cells: &[
        "#####..#####",
        "#..........#",
        "#..........#",
        "#..##..##..#",
        "............",
        "............",
        "#..##..##..#",
        "#..........#",
        "#..........#",
        "#####..#####",
    ],
    exits: Size::S.hatches(),
    base: &[rusher(2, 1), rusher(9, 8), shooter(9, 1)],
    reinforcements: &[],
    events: LOCKDOWN,
}
.valid();

/// Bunks along the walls.
const CREW_QUARTERS: PrototypeRoom = PrototypeRoom {
    name: "crew quarters",
    category: Category::Normal,
    theme: Theme::Crew,
    cells: &[
        "#####..#####",
        "#..........#",
        "#.##....##.#",
        "#.##....##.#",
        "............",
        "............",
        "#.##....##.#",
        "#.##....##.#",
        "#..........#",
        "#####..#####",
    ],
    exits: Size::S.hatches(),
    base: &[shooter(10, 1), rusher(1, 8), rusher(10, 8)],
    reinforcements: &[],
    events: LOCKDOWN,
}
.valid();

/// Five pillars: cover from the shooters, and something for rushers to steer around. The
/// shooters wait along the south wall.
const ENGINE_ROOM: PrototypeRoom = PrototypeRoom {
    name: "engine room",
    category: Category::Normal,
    theme: Theme::Engineering,
    cells: &[
        "########..########",
        "#................#",
        "#................#",
        "#..##........##..#",
        "#..##........##..#",
        "..................",
        "..................",
        "#..##...##...##..#",
        "#..##...##...##..#",
        "#................#",
        "#................#",
        "########..########",
    ],
    exits: Size::M.hatches(),
    base: &[shooter(2, 10), shooter(15, 10), rusher(8, 5)],
    reinforcements: &[then(&[
        shooter(5, 10),
        shooter(12, 10),
        rusher(3, 5),
        rusher(14, 5),
    ])],
    events: LOCKDOWN,
}
.valid();

/// Bigger than a phone screen, L-shaped (void top-right), with a pillar and a pit strip.
const CARGO_HOLD: PrototypeRoom = PrototypeRoom {
    name: "cargo hold",
    category: Category::Normal,
    theme: Theme::Cargo,
    cells: &[
        "###########..###        ",
        "#..............#        ",
        "#..............#        ",
        "#....##........#        ",
        "#....##........#        ",
        "#..............#########",
        "........................",
        "........................",
        "#......................#",
        "#.........oooo.........#",
        "#.........oooo.........#",
        "#......................#",
        "#......................#",
        "###########..###########",
    ],
    exits: Size::L.hatches(),
    base: &[rusher(20, 8), rusher(20, 11), shooter(3, 2)],
    reinforcements: &[then(&[
        rusher(22, 10),
        rusher(22, 12),
        rusher(8, 12),
        shooter(14, 3),
    ])],
    events: LOCKDOWN,
}
.valid();

/// A pit trench splits it (shots fly over, enemies go round, players go round or roll
/// over), and pillars guard the far side.
const SHUTTLE_BAY: PrototypeRoom = PrototypeRoom {
    name: "shuttle bay",
    category: Category::Normal,
    theme: Theme::Cargo,
    cells: &[
        "###########..###########",
        "#......................#",
        "#......................#",
        "#......................#",
        "#.......oo.....##......#",
        "#.......oo.....##......#",
        "........oo..............",
        "........oo..............",
        "#.......oo.....##......#",
        "#.......oo.....##......#",
        "#......................#",
        "#......................#",
        "#......................#",
        "###########..###########",
    ],
    exits: Size::L.hatches(),
    base: &[
        shooter(15, 1),
        shooter(15, 12),
        rusher(20, 2),
        rusher(20, 11),
    ],
    reinforcements: &[then(&[
        spread_shooter(18, 6),
        shooter(4, 1),
        shooter(4, 12),
        rusher(21, 1),
        rusher(21, 12),
        rusher(12, 7),
    ])],
    events: LOCKDOWN,
}
.valid();

/// The ship's goal: four pillars, a thicker mix, then the captain (a placeholder elite)
/// warps in with two rushers. Clearing it unlocks the airlocks.
const BRIDGE: PrototypeRoom = PrototypeRoom {
    name: "bridge",
    category: Category::Boss,
    theme: Theme::Bridge,
    cells: &[
        "###########..###########",
        "#......................#",
        "#......................#",
        "#......................#",
        "#.....##.......##......#",
        "#.....##.......##......#",
        "........................",
        "........................",
        "#.....##.......##......#",
        "#.....##.......##......#",
        "#......................#",
        "#......................#",
        "#......................#",
        "###########..###########",
    ],
    exits: Size::L.hatches(),
    base: &[rusher(3, 1), rusher(20, 1), shooter(11, 3), shooter(20, 11)],
    reinforcements: &[
        then(&[
            rusher(3, 6),
            rusher(20, 7),
            rusher(8, 1),
            shooter(3, 1),
            spread_shooter(20, 2),
        ]),
        then(&[captain(20, 6), rusher(18, 2), rusher(18, 11)]),
    ],
    events: LOCKDOWN,
}
.valid();

/// Behind an access panel: a cramped maintenance crawlspace, pipes around a chest.
const CRAWLSPACE: PrototypeRoom = PrototypeRoom {
    name: "crawlspace",
    category: Category::Secret,
    theme: Theme::Crawlspace,
    cells: &[
        "#####..#####",
        "#..........#",
        "#.###..###.#",
        "#.#......#.#",
        ".....##.....",
        "............",
        "#.#......#.#",
        "#.###..###.#",
        "#..........#",
        "#####..#####",
    ],
    exits: Size::S.hatches(),
    base: &[],
    reinforcements: &[],
    events: &[],
}
.valid();

/// Shelving bays around a chest: the reward room.
const STORES: PrototypeRoom = PrototypeRoom {
    name: "stores",
    category: Category::Reward,
    theme: Theme::Stores,
    cells: &[
        "#####..#####",
        "#...#..#...#",
        "#...#..#...#",
        "#..........#",
        "............",
        "............",
        "#..........#",
        "#...#..#...#",
        "#...#..#...#",
        "#####..#####",
    ],
    exits: Size::S.hatches(),
    base: &[],
    reinforcements: &[],
    events: &[],
}
.valid();

// --- the pool proper (issue #38) ----------------------------------------------------
//
// These rooms keep enemies at least 3 cells (Chebyshev) from every hatch point, since any
// hatch may be the way in. Themes without a zone of their own yet (medical, hydroponics,
// sensors) file under the zone's nearest theme.

/// Medical: an operating table dead center to circle-strafe around, beds along the
/// walls. The patients get up once the orderlies are down.
const MEDBAY: PrototypeRoom = PrototypeRoom {
    name: "medbay",
    category: Category::Normal,
    theme: Theme::Crew,
    cells: &[
        "#####..#####",
        "#..........#",
        "#.#......#.#",
        "#.#......#.#",
        ".....##.....",
        ".....##.....",
        "#.#......#.#",
        "#.#......#.#",
        "#..........#",
        "#####..#####",
    ],
    exits: Size::S.hatches(),
    base: &[shooter(10, 1), rusher(3, 3), rusher(8, 6)],
    reinforcements: &[then(&[rusher(1, 1), rusher(10, 8)])],
    events: LOCKDOWN,
}
.valid();

/// Hydroponics: four nutrient troughs (pits). Shots fly over them and rushers go round,
/// so the shooters tucked behind them are safe until you roll across.
const HYDROPONICS: PrototypeRoom = PrototypeRoom {
    name: "hydroponics",
    category: Category::Normal,
    theme: Theme::Crew,
    cells: &[
        "#####..#####",
        "#..........#",
        "#.ooo..ooo.#",
        "#..........#",
        "............",
        "............",
        "#..........#",
        "#.ooo..ooo.#",
        "#..........#",
        "#####..#####",
    ],
    exits: Size::S.hatches(),
    base: &[shooter(1, 1), shooter(10, 8), rusher(8, 3), rusher(3, 6)],
    reinforcements: &[],
    events: LOCKDOWN,
}
.valid();

/// A small arena ring: the cargo lift's open shaft (a 4 x 4 pit) fills the middle, so
/// every fight goes round it and the shooters fire across it.
const CARGO_LIFT: PrototypeRoom = PrototypeRoom {
    name: "cargo lift",
    category: Category::Normal,
    theme: Theme::Cargo,
    cells: &[
        "#####..#####",
        "#..........#",
        "#..........#",
        "#...oooo...#",
        "....oooo....",
        "....oooo....",
        "#...oooo...#",
        "#..........#",
        "#..........#",
        "#####..#####",
    ],
    exits: Size::S.hatches(),
    base: &[shooter(10, 1), shooter(1, 8), rusher(1, 1)],
    reinforcements: &[then(&[rusher(10, 8), rusher(2, 1)])],
    events: LOCKDOWN,
}
.valid();

/// An arena ring: the reactor's core (walls in a moat of pits) fills the middle. Shooters
/// hold the far side, rushers come round both ways, and the second wave brings a spread
/// shooter.
const REACTOR: PrototypeRoom = PrototypeRoom {
    name: "reactor",
    category: Category::Normal,
    theme: Theme::Engineering,
    cells: &[
        "########..########",
        "#................#",
        "#................#",
        "#.....oooooo.....#",
        "#....o######o....#",
        ".....o######o.....",
        ".....o######o.....",
        "#....o######o....#",
        "#.....oooooo.....#",
        "#................#",
        "#................#",
        "########..########",
    ],
    exits: Size::M.hatches(),
    base: &[shooter(3, 4), shooter(3, 7), rusher(16, 1), rusher(16, 10)],
    reinforcements: &[then(&[spread_shooter(1, 1), rusher(1, 10), rusher(13, 4)])],
    events: LOCKDOWN,
}
.valid();

/// A chokepoint: a bulkhead splits the shop, and its one door lines up with the west and
/// east hatches. The shooters hold the tool bay behind it, so you push through the door
/// or wait for the rushers to funnel out of it.
const MACHINE_SHOP: PrototypeRoom = PrototypeRoom {
    name: "machine shop",
    category: Category::Normal,
    theme: Theme::Engineering,
    cells: &[
        "########..########",
        "#......#.........#",
        "#......#.........#",
        "#......#...##....#",
        "#......#...##....#",
        "..................",
        "..................",
        "#......#...##....#",
        "#......#...##....#",
        "#......#.........#",
        "#......#.........#",
        "########..########",
    ],
    exits: Size::M.hatches(),
    base: &[shooter(3, 3), shooter(3, 8), rusher(14, 1), rusher(14, 10)],
    reinforcements: &[then(&[spread_shooter(4, 5), rusher(1, 1), rusher(1, 10)])],
    events: LOCKDOWN,
}
.valid();

/// Pit channels: two coolant channels cut the plant into three bands, crossed by three
/// short bridges. Shooters in the outer bands fire across at the middle one, where the
/// hatches are; roll a channel to reach them.
const COOLANT_PLANT: PrototypeRoom = PrototypeRoom {
    name: "coolant plant",
    category: Category::Normal,
    theme: Theme::Engineering,
    cells: &[
        "########..########",
        "#................#",
        "#................#",
        "#ooo..oooooo..ooo#",
        "#.........##.....#",
        "..................",
        "..................",
        "#.....##.........#",
        "#ooooooo..ooooooo#",
        "#................#",
        "#................#",
        "########..########",
    ],
    exits: Size::M.hatches(),
    base: &[shooter(2, 1), shooter(15, 10), rusher(7, 5), rusher(10, 6)],
    reinforcements: &[
        then(&[shooter(15, 1), shooter(2, 10)]),
        then(&[rusher(4, 4), rusher(13, 7), spread_shooter(4, 1)]),
    ],
    events: LOCKDOWN,
}
.valid();

/// Sensors: a long gallery. Consoles wall off a lane along each side, so the shooters at
/// the far end have the whole middle; the side lanes are the flank route up to them.
const SENSOR_GALLERY: PrototypeRoom = PrototypeRoom {
    name: "sensor gallery",
    category: Category::Normal,
    theme: Theme::Engineering,
    cells: &[
        "########..########",
        "#................#",
        "#................#",
        "#..#####..#####..#",
        "#................#",
        "..................",
        "..................",
        "#................#",
        "#..#####..#####..#",
        "#................#",
        "#................#",
        "########..########",
    ],
    exits: Size::M.hatches(),
    base: &[shooter(3, 4), shooter(3, 7), rusher(13, 1), rusher(13, 10)],
    reinforcements: &[then(&[
        spread_shooter(3, 5),
        shooter(1, 1),
        shooter(1, 10),
        rusher(4, 1),
    ])],
    events: LOCKDOWN,
}
.valid();

/// The rare guarded hallway: a gantry, corridor-shaped (void above and below), that
/// crosses the slot with stubs to the north and south hatches. Guards hold both ends
/// behind crates, so there is no side to hide on.
const SERVICE_GANTRY: PrototypeRoom = PrototypeRoom {
    name: "service gantry",
    category: Category::Normal,
    theme: Theme::Cargo,
    cells: &[
        "         ##..##         ",
        "         #....#         ",
        "         #....#         ",
        "##########....##########",
        "#......................#",
        "#.....##........##.....#",
        "........................",
        "........................",
        "#.....##........##.....#",
        "#......................#",
        "##########....##########",
        "         #....#         ",
        "         #....#         ",
        "         ##..##         ",
    ],
    exits: Size::L.hatches(),
    base: &[shooter(3, 4), shooter(20, 9), rusher(19, 4)],
    reinforcements: &[then(&[rusher(3, 9), rusher(20, 5)])],
    events: LOCKDOWN,
}
.valid();

/// A pillar field: rows of tables round a serving counter, and three waves, each from
/// somewhere new; the last brings a spread shooter to the counter.
const MESS_HALL: PrototypeRoom = PrototypeRoom {
    name: "mess hall",
    category: Category::Normal,
    theme: Theme::Crew,
    cells: &[
        "###########..###########",
        "#......................#",
        "#......................#",
        "#..##..##......##..##..#",
        "#..##..##......##..##..#",
        "#......................#",
        "..........####..........",
        "..........####..........",
        "#......................#",
        "#..##..##......##..##..#",
        "#..##..##......##..##..#",
        "#......................#",
        "#......................#",
        "###########..###########",
    ],
    exits: Size::L.hatches(),
    base: &[shooter(1, 1), rusher(5, 5), rusher(18, 8)],
    reinforcements: &[
        then(&[shooter(22, 1), shooter(22, 12), rusher(5, 8)]),
        then(&[spread_shooter(11, 8), rusher(3, 1), rusher(20, 12)]),
    ],
    events: LOCKDOWN,
}
.valid();

/// Staggered container stacks: short lanes and chokepoints between them, with a flank
/// route round every stack. The shooters sit in opposite corners, so one always has an
/// angle.
const CONTAINER_STACKS: PrototypeRoom = PrototypeRoom {
    name: "container stacks",
    category: Category::Normal,
    theme: Theme::Cargo,
    cells: &[
        "###########..###########",
        "#......................#",
        "#.######.......#####...#",
        "#.######.......#####...#",
        "#..........##..........#",
        "#...####...##...######.#",
        "........................",
        "........................",
        "#.######...##...####...#",
        "#..........##..........#",
        "#...#####.......######.#",
        "#...#####.......######.#",
        "#......................#",
        "###########..###########",
    ],
    exits: Size::L.hatches(),
    base: &[shooter(22, 2), shooter(1, 12), rusher(9, 9), rusher(15, 4)],
    reinforcements: &[then(&[
        rusher(22, 12),
        rusher(1, 1),
        shooter(17, 12),
        spread_shooter(5, 1),
    ])],
    events: LOCKDOWN,
}
.valid();

/// Command: a briefing room, seats (single pillars) facing a holo table. Only the Gunship
/// has a fore M slot.
const WAR_ROOM: PrototypeRoom = PrototypeRoom {
    name: "war room",
    category: Category::Normal,
    theme: Theme::Bridge,
    cells: &[
        "########..########",
        "#................#",
        "#................#",
        "#..#..#....#..#..#",
        "#................#",
        ".......####.......",
        ".......####.......",
        "#................#",
        "#..#..#....#..#..#",
        "#................#",
        "#................#",
        "########..########",
    ],
    exits: Size::M.hatches(),
    base: &[shooter(1, 1), shooter(16, 10), rusher(4, 5), rusher(13, 6)],
    reinforcements: &[then(&[spread_shooter(16, 1), rusher(1, 10), rusher(4, 1)])],
    events: LOCKDOWN,
}
.valid();

/// Every room the generator may place, checked against each template when it builds.
pub const POOL: &[PrototypeRoom] = &[
    AIRLOCK,
    GALLEY,
    CREW_QUARTERS,
    ENGINE_ROOM,
    CARGO_HOLD,
    SHUTTLE_BAY,
    BRIDGE,
    CRAWLSPACE,
    STORES,
    MEDBAY,
    HYDROPONICS,
    CARGO_LIFT,
    REACTOR,
    MACHINE_SHOP,
    COOLANT_PLANT,
    SENSOR_GALLERY,
    SERVICE_GANTRY,
    MESS_HALL,
    CONTAINER_STACKS,
    WAR_ROOM,
];
