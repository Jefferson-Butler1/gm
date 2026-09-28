//! The slice's handmade derelict, five rooms in a line:
//!
//! 1. airlock (entrance): empty, to get your hands on the controls.
//! 2. cargo hold: rushers with a shooter in each wave; a pillar and a pit strip.
//! 3. engine room: shooter-heavy, with six pillars to hide behind.
//! 4. bridge: four pillars; the mix gets thicker, and its second wave brings a spread
//!    shooter.
//! 5. shuttle bay (exit): a pit trench the shooters fire across, two last waves (the
//!    second led by a spread shooter), then the extraction pad wins the run.
//!
//! Spread shooters are the pattern experiment (issue #15); with it off they're shooters.
//!
//! Every room and the derelict are validated when the crate builds.

use crate::room::{
    Category, Connection, Derelict, Dir, EnemyKind, Exit, ExitKind, ExitRef, LayerTrigger,
    Placement, PrototypeRoom, Reinforcement, RoomAction, RoomTrigger,
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
    category: Category::Entrance,
    cells: &[
        "##############",
        "#............#",
        "#............#",
        "#..oo........#",
        "#.............",
        "#.............",
        "#............#",
        "#............#",
        "#............#",
        "##############",
    ],
    exits: &[exit(Dir::East, 13, 4, ExitKind::Exit)],
    base: &[],
    reinforcements: &[],
    events: &[],
    extraction: None,
}
.valid();

/// Bigger than a phone screen, L-shaped (void top-right), with a pillar and a pit.
const CARGO_HOLD: PrototypeRoom = PrototypeRoom {
    name: "cargo hold",
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
        "...............................#",
        "..........oooo.................#",
        "#.........oooo.................#",
        "#..............................#",
        "#..............................#",
        "#..............................#",
        "################################",
    ],
    exits: &[
        exit(Dir::West, 0, 9, ExitKind::Entrance),
        exit(Dir::North, 9, 0, ExitKind::Exit),
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

/// Six pillars: cover from the shooters, and something for rushers to steer around.
const ENGINE_ROOM: PrototypeRoom = PrototypeRoom {
    name: "engine room",
    category: Category::Normal,
    cells: &[
        "##########..##########",
        "#....................#",
        "#....................#",
        "#....................#",
        "#...##....##....##...#",
        "#...##....##....##...#",
        "#....................#",
        "#....................#",
        "#...##..........##...#",
        "#...##..........##...#",
        "#....................#",
        "#....................#",
        "#....................#",
        "#########..###########",
    ],
    exits: &[
        exit(Dir::South, 9, 13, ExitKind::Entrance),
        exit(Dir::North, 10, 0, ExitKind::Exit),
    ],
    base: &[shooter(2, 2), shooter(19, 2), rusher(10, 7)],
    reinforcements: &[then(&[
        shooter(7, 1),
        shooter(14, 1),
        rusher(1, 7),
        rusher(20, 7),
    ])],
    events: LOCKDOWN,
    extraction: None,
}
.valid();

const BRIDGE: PrototypeRoom = PrototypeRoom {
    name: "bridge",
    category: Category::Normal,
    cells: &[
        "################",
        "#..............#",
        "#..............#",
        "#...##....##...#",
        "#...##....##...#",
        "#...............",
        "#...............",
        "#...##....##...#",
        "#...##....##...#",
        "#..............#",
        "#..............#",
        "#######..#######",
    ],
    exits: &[
        exit(Dir::South, 7, 11, ExitKind::Entrance),
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

/// A door from (room, exit) to (room, exit), as indices.
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

pub const DERELICT: Derelict = Derelict {
    rooms: &[AIRLOCK, CARGO_HOLD, ENGINE_ROOM, BRIDGE, SHUTTLE_BAY],
    connections: &[
        link((0, 0), (1, 0)),
        link((1, 1), (2, 0)),
        link((2, 1), (3, 0)),
        link((3, 1), (4, 0)),
    ],
    start_room: 0,
    start_cell: (5, 6),
}
.valid();
