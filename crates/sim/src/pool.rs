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

/// Empty: the party boards here. Its pit is the first one you meet.
const AIRLOCK: PrototypeRoom = PrototypeRoom {
    name: "airlock",
    category: Category::Entrance,
    theme: Theme::Airlock,
    cells: &[
        "#####..#####",
        "#..........#",
        "#..........#",
        "#..oo......#",
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
        shooter(6, 10),
        shooter(11, 10),
        rusher(1, 5),
        rusher(16, 5),
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
        rusher(22, 8),
        rusher(22, 12),
        rusher(9, 12),
        shooter(14, 1),
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
        shooter(12, 1),
        shooter(12, 12),
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
    base: &[rusher(3, 1), rusher(20, 1), shooter(11, 2), shooter(20, 11)],
    reinforcements: &[
        then(&[
            rusher(3, 6),
            rusher(20, 7),
            rusher(11, 1),
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
];
