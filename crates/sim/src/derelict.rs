//! The handmade derelict: a corvette, bow east, its rooms placed in one floor grid and
//! joined by hatches, walked in a line:
//!
//! 1. airlock (entrance): empty, to get your hands on the controls. On the dorsal hull.
//! 2. cargo hold: rushers with a shooter in each wave; a pillar and a pit strip.
//! 3. engine room: aft; shooter-heavy, with six pillars to hide behind.
//! 4. bridge: fore, down the service passage; four pillars, a thicker mix, and a second
//!    wave that brings a spread shooter.
//! 5. shuttle bay (exit): at the bow, through the hangar passage. A pit trench the
//!    shooters fire across, two last waves (the second led by a spread shooter), then
//!    the extraction pad wins the run.
//!
//! ```text
//!         +-airlock-+
//!    +----=-cargo hold-------+
//!    |                  +----+
//! +--=-engine-+   +-bridge-+   +--shuttle bay--+
//! |           =---=        =---=               |
//! +-----------+   +--------+   +---------------+
//! ```
//!
//! Spread shooters are the pattern experiment (issue #15); with it off they're shooters.
//!
//! Every room and the derelict are validated when the crate builds.

use crate::room::{
    Category, Connection, Derelict, Dir, EnemyKind, Exit, ExitKind, ExitRef, LayerTrigger, Placed,
    Placement, PrototypeRoom, Reinforcement, RoomAction, RoomTrigger, Theme,
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

/// The next wave once the current one is dead.
const fn then(placements: &'static [Placement]) -> Reinforcement {
    Reinforcement {
        trigger: LayerTrigger::OnEnemiesCleared,
        placements,
    }
}

const fn exit(dir: Dir, x: usize, y: usize, kind: ExitKind) -> Exit {
    Exit {
        dir,
        x,
        y,
        width: 2,
        kind,
    }
}

const AIRLOCK: PrototypeRoom = PrototypeRoom {
    name: "airlock",
    theme: Theme::Airlock,
    category: Category::Entrance,
    cells: &[
        "##############",
        "#............#",
        "#............#",
        "#..oo........#",
        "#............#",
        "#............#",
        "#............#",
        "#............#",
        "#............#",
        "######..######",
    ],
    exits: &[exit(Dir::South, 6, 9, ExitKind::Exit)],
    base: &[],
    reinforcements: &[],
    events: &[],
    extraction: None,
}
.valid();

/// Bigger than a phone screen, L-shaped (void top-right), with a pillar and a pit.
const CARGO_HOLD: PrototypeRoom = PrototypeRoom {
    name: "cargo hold",
    theme: Theme::Cargo,
    category: Category::Normal,
    cells: &[
        "#########..#########            ",
        "#..................#            ",
        "#..................#            ",
        "#....##............#            ",
        "#....##............#            ",
        "#..................#            ",
        "#..................#            ",
        "#..................#############",
        "#..............................#",
        "#..............................#",
        "#.........oooo.................#",
        "#.........oooo.................#",
        "#..............................#",
        "#..............................#",
        "#..............................#",
        "####..##########################",
    ],
    exits: &[
        exit(Dir::North, 9, 0, ExitKind::Entrance),
        exit(Dir::South, 4, 15, ExitKind::Exit),
    ],
    base: &[rusher(27, 9), rusher(27, 13), shooter(16, 2)],
    reinforcements: &[then(&[
        rusher(29, 8),
        rusher(29, 14),
        rusher(12, 13),
        shooter(3, 1),
    ])],
    events: LOCKDOWN,
    extraction: None,
}
.valid();

/// Six pillars: cover from the shooters, and something for rushers to steer around. The
/// shooters wait at the far (south) end from the hatch in.
const ENGINE_ROOM: PrototypeRoom = PrototypeRoom {
    name: "engine room",
    theme: Theme::Engineering,
    category: Category::Normal,
    cells: &[
        "#########..###########",
        "#....................#",
        "#....................#",
        "#....................#",
        "#...##..........##...#",
        "#...##..........##...#",
        "#.....................",
        "#.....................",
        "#...##....##....##...#",
        "#...##....##....##...#",
        "#....................#",
        "#....................#",
        "#....................#",
        "######################",
    ],
    exits: &[
        exit(Dir::North, 9, 0, ExitKind::Entrance),
        exit(Dir::East, 21, 6, ExitKind::Exit),
    ],
    base: &[shooter(2, 11), shooter(19, 11), rusher(10, 6)],
    reinforcements: &[then(&[
        shooter(7, 12),
        shooter(14, 12),
        rusher(1, 6),
        rusher(20, 6),
    ])],
    events: LOCKDOWN,
    extraction: None,
}
.valid();

const BRIDGE: PrototypeRoom = PrototypeRoom {
    name: "bridge",
    theme: Theme::Bridge,
    category: Category::Normal,
    cells: &[
        "################",
        "#..............#",
        "#..............#",
        "#...##....##...#",
        "#...##....##...#",
        "................",
        "................",
        "#...##....##...#",
        "#...##....##...#",
        "#..............#",
        "#..............#",
        "################",
    ],
    exits: &[
        exit(Dir::West, 0, 5, ExitKind::Entrance),
        exit(Dir::East, 15, 5, ExitKind::Exit),
    ],
    base: &[rusher(2, 1), rusher(13, 1), shooter(7, 1), shooter(13, 9)],
    reinforcements: &[then(&[
        rusher(2, 5),
        rusher(13, 5),
        rusher(7, 1),
        shooter(2, 1),
        spread_shooter(13, 1),
    ])],
    events: LOCKDOWN,
    extraction: None,
}
.valid();

/// The exit room: a pit trench splits it (shots fly over, enemies go round, players go
/// round or roll over), pillars guard the far side, and the extraction pad sits at the
/// far end.
const SHUTTLE_BAY: PrototypeRoom = PrototypeRoom {
    name: "shuttle bay",
    theme: Theme::Cargo,
    category: Category::Exit,
    cells: &[
        "########################",
        "#......................#",
        "#......................#",
        "#.......oo.....##......#",
        "#.......oo.....##......#",
        "........oo.............#",
        "........oo.............#",
        "#.......oo.....##......#",
        "#.......oo.....##......#",
        "#......................#",
        "#......................#",
        "########################",
    ],
    exits: &[exit(Dir::West, 0, 5, ExitKind::Entrance)],
    base: &[
        shooter(12, 1),
        shooter(12, 10),
        rusher(20, 2),
        rusher(20, 9),
    ],
    reinforcements: &[then(&[
        spread_shooter(18, 5),
        shooter(4, 1),
        shooter(4, 10),
        rusher(21, 1),
        rusher(21, 10),
        rusher(12, 6),
    ])],
    events: LOCKDOWN,
    extraction: Some((21, 5)),
}
.valid();

/// Engine room to bridge, under the cargo hold.
const SERVICE_PASSAGE: PrototypeRoom = PrototypeRoom {
    name: "service passage",
    theme: Theme::Corridor,
    category: Category::Connector,
    cells: &["########", "........", "........", "########"],
    exits: &[
        exit(Dir::West, 0, 1, ExitKind::Entrance),
        exit(Dir::East, 7, 1, ExitKind::Exit),
    ],
    base: &[],
    reinforcements: &[],
    events: &[],
    extraction: None,
}
.valid();

/// Bridge to shuttle bay.
const HANGAR_PASSAGE: PrototypeRoom = PrototypeRoom {
    name: "hangar passage",
    cells: &["####", "....", "....", "####"],
    exits: &[
        exit(Dir::West, 0, 1, ExitKind::Entrance),
        exit(Dir::East, 3, 1, ExitKind::Exit),
    ],
    ..SERVICE_PASSAGE
}
.valid();

/// A hatch from (room, exit) to (room, exit), as indices.
const fn link(from: (usize, usize), to: (usize, usize)) -> Connection {
    Connection {
        from: ExitRef {
            room: from.0,
            exit: from.1,
        },
        to: ExitRef {
            room: to.0,
            exit: to.1,
        },
    }
}

const fn at(room: PrototypeRoom, x: usize, y: usize) -> Placed {
    Placed { room, at: (x, y) }
}

/// Rooms keep their slice order (0-4), so the passages come last.
pub const DERELICT: Derelict = Derelict {
    rooms: &[
        at(AIRLOCK, 8, 0),
        at(CARGO_HOLD, 5, 9),
        at(ENGINE_ROOM, 0, 24),
        at(BRIDGE, 28, 25),
        at(SHUTTLE_BAY, 46, 25),
        at(SERVICE_PASSAGE, 21, 29),
        at(HANGAR_PASSAGE, 43, 29),
    ],
    connections: &[
        link((0, 0), (1, 0)),
        link((1, 1), (2, 0)),
        link((2, 1), (5, 0)),
        link((5, 1), (3, 0)),
        link((3, 1), (6, 0)),
        link((6, 1), (4, 0)),
    ],
    start_room: 0,
    start_cell: (5, 6),
}
.valid();
