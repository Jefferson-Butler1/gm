//! The handmade test derelict: an empty airlock, a cargo hold with two waves, and a
//! bridge with one. Each room and the derelict are validated when the crate builds.
//! The full 3–5 room slice derelict (exit room, win) is the next build step.

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
    exits: &[Exit {
        dir: Dir::East,
        x: 13,
        y: 4,
        width: 2,
        kind: ExitKind::Exit,
    }],
    base: &[],
    reinforcements: &[],
    events: &[],
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
        Exit {
            dir: Dir::West,
            x: 0,
            y: 9,
            width: 2,
            kind: ExitKind::Entrance,
        },
        Exit {
            dir: Dir::North,
            x: 9,
            y: 0,
            width: 2,
            kind: ExitKind::Exit,
        },
    ],
    base: &[rusher(27, 9), rusher(27, 13), shooter(16, 2)],
    reinforcements: &[Reinforcement {
        trigger: LayerTrigger::OnEnemiesCleared,
        placements: &[rusher(29, 8), rusher(29, 14), rusher(12, 13), shooter(3, 1)],
    }],
    events: LOCKDOWN,
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
        "#..............#",
        "#..............#",
        "#...##....##...#",
        "#...##....##...#",
        "#..............#",
        "#..............#",
        "#######..#######",
    ],
    exits: &[Exit {
        dir: Dir::South,
        x: 7,
        y: 11,
        width: 2,
        kind: ExitKind::Entrance,
    }],
    base: &[
        rusher(2, 1),
        rusher(13, 1),
        rusher(7, 1),
        rusher(2, 5),
        rusher(13, 5),
    ],
    reinforcements: &[],
    events: LOCKDOWN,
}
.valid();

pub const DERELICT: Derelict = Derelict {
    rooms: &[AIRLOCK, CARGO_HOLD, BRIDGE],
    connections: &[
        Connection {
            from: ExitRef { room: 0, exit: 0 },
            to: ExitRef { room: 1, exit: 0 },
        },
        Connection {
            from: ExitRef { room: 1, exit: 1 },
            to: ExitRef { room: 2, exit: 0 },
        },
    ],
    start_room: 0,
    start_cell: (5, 6),
}
.valid();
