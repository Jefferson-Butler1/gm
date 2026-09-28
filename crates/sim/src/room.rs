//! Room format (issue #10), mirroring Enter the Gungeon's `PrototypeDungeonRoom`.
//!
//! Rooms are plain typed `'static` values. The cell grid is a text block (one string per
//! row, row-major); everything else is typed fields. Validation is a `const fn`: room,
//! derelict and template constants are wrapped in `.valid()`, so a malformed room fails
//! the build and the sim only ever sees validated data.
//!
//! Coordinates are integer cells in the data, room-local: (0, 0) is the room's top-left
//! cell. A hull template ([`crate::hull`]) or a hand-placed [`Derelict`] places each room
//! in one floor-wide grid (see [`crate::ship`]); in the sim, world space is floor points:
//! the origin is floor cell (0, 0)'s top-left corner and +y points down.

use crate::{Fx, FxVec2};

/// Side of one cell in world units (points).
pub const CELL: Fx = Fx::from_bits(32 << 32);
const HALF_CELL: Fx = Fx::from_bits(16 << 32);
/// `Fx` bits -> cell index is a floor shift: 32 fractional bits + log2(32).
const CELL_SHIFT: u32 = 37;
/// Grid size cap per side; bounds the const flood fill's scratch arrays.
pub const MAX_SIDE: usize = 64;
const MAX_CELLS: usize = MAX_SIDE * MAX_SIDE;
/// Rooms per derelict: the sim keeps the visited and cleared sets as `u64` bitmasks.
pub const MAX_ROOMS: usize = 64;

/// One grid cell. Text: `.` floor, `#` wall, `o` pit, space = void (outside the room).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Cell {
    Floor,
    Wall,
    /// Blocks enemies, not bullets; players walking onto one fall in.
    Pit,
    Void,
}

impl Cell {
    const fn parse(byte: u8) -> Option<Self> {
        match byte {
            b'.' => Some(Self::Floor),
            b'#' => Some(Self::Wall),
            b'o' => Some(Self::Pit),
            b' ' => Some(Self::Void),
            _ => None,
        }
    }
}

/// Which room edge an exit sits on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Dir {
    North,
    East,
    South,
    West,
}

impl Dir {
    /// `==` for const fns.
    const fn is(self, other: Self) -> bool {
        matches!(
            (self, other),
            (Self::North, Self::North)
                | (Self::East, Self::East)
                | (Self::South, Self::South)
                | (Self::West, Self::West)
        )
    }

    /// Whether exits on these edges can link: they must face each other.
    const fn faces(self, other: Self) -> bool {
        matches!(
            (self, other),
            (Self::North, Self::South)
                | (Self::South, Self::North)
                | (Self::East, Self::West)
                | (Self::West, Self::East)
        )
    }
}

/// Which end of a [`Connection`] an exit may be. Constrains the layout graph, not which
/// way players may walk through it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ExitKind {
    Entrance,
    Exit,
    Either,
}

impl ExitKind {
    const fn can_be_from(self) -> bool {
        matches!(self, Self::Exit | Self::Either)
    }

    const fn can_be_to(self) -> bool {
        matches!(self, Self::Entrance | Self::Either)
    }
}

/// What a room is aboard (issue #28): which template slots may take it (see
/// [`Zone`](crate::hull::Zone)), and later, what loot it hands out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Theme {
    /// A stretch of passage between rooms: wherever the template draws one.
    Corridor,
    Airlock,
    Bridge,
    Cargo,
    Crew,
    Engineering,
}

/// Room size classes (issue #28), in cells including the walls. Every room of a class
/// has the same [`Self::hatches`], so it fits any template slot of that class.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Size {
    S,
    M,
    L,
}

const S_HATCHES: [Exit; 4] = Size::S.hatch_points();
const M_HATCHES: [Exit; 4] = Size::M.hatch_points();
const L_HATCHES: [Exit; 4] = Size::L.hatch_points();

impl Size {
    /// (width, height).
    #[must_use]
    pub const fn dims(self) -> (usize, usize) {
        match self {
            Self::S => (12, 10),
            Self::M => (18, 12),
            Self::L => (24, 14),
        }
    }

    /// The class of a `width` x `height` room, if it is one.
    #[must_use]
    pub const fn of(width: usize, height: usize) -> Option<Self> {
        match (width, height) {
            (12, 10) => Some(Self::S),
            (18, 12) => Some(Self::M),
            (24, 14) => Some(Self::L),
            _ => None,
        }
    }

    /// The standard hatch points: a 2-wide gap centered on each side, in the order north,
    /// east, south, west. Rooms of the class leave all four open, as their exits; a
    /// template walls off the ones its slot doesn't use.
    #[must_use]
    pub const fn hatches(self) -> &'static [Exit; 4] {
        match self {
            Self::S => &S_HATCHES,
            Self::M => &M_HATCHES,
            Self::L => &L_HATCHES,
        }
    }

    /// The hatch point on side `dir`.
    #[must_use]
    pub const fn hatch(self, dir: Dir) -> Exit {
        let [north, east, south, west] = *self.hatches();
        match dir {
            Dir::North => north,
            Dir::East => east,
            Dir::South => south,
            Dir::West => west,
        }
    }

    const fn hatch_points(self) -> [Exit; 4] {
        let (width, height) = self.dims();
        // Even sides: the gap's two cells straddle the middle.
        let (x, y) = (
            (width / 2).saturating_sub(1),
            (height / 2).saturating_sub(1),
        );
        let (right, bottom) = (width.saturating_sub(1), height.saturating_sub(1));
        [
            hatch_point(Dir::North, x, 0),
            hatch_point(Dir::East, right, y),
            hatch_point(Dir::South, x, bottom),
            hatch_point(Dir::West, 0, y),
        ]
    }
}

const fn hatch_point(dir: Dir, x: usize, y: usize) -> Exit {
    Exit {
        dir,
        x,
        y,
        width: 2,
        kind: ExitKind::Either,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Category {
    Normal,
    Connector,
    Hub,
    Reward,
    Boss,
    Secret,
    Entrance,
    Exit,
}

/// A hatch's gap in the room's edge: `width` floor cells starting at (`x`, `y`) and running
/// along the edge (rightward on north/south edges, downward on east/west ones).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Exit {
    pub dir: Dir,
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub kind: ExitKind,
}

impl Exit {
    /// The `i`th gap cell.
    #[must_use]
    pub const fn cell(&self, i: usize) -> (usize, usize) {
        match self.dir {
            Dir::North | Dir::South => (self.x.saturating_add(i), self.y),
            Dir::East | Dir::West => (self.x, self.y.saturating_add(i)),
        }
    }

    /// The cell one step into the room from (`x`, `y`); `None` off the grid's top/left.
    const fn inward(&self, (x, y): (usize, usize)) -> Option<(usize, usize)> {
        match self.dir {
            Dir::North => Some((x, y.saturating_add(1))),
            Dir::West => Some((x.saturating_add(1), y)),
            Dir::South => match y.checked_sub(1) {
                Some(y) => Some((x, y)),
                None => None,
            },
            Dir::East => match x.checked_sub(1) {
                Some(x) => Some((x, y)),
                None => None,
            },
        }
    }

    const fn covers(&self, x: usize, y: usize) -> bool {
        let (along, across, start, line) = match self.dir {
            Dir::North | Dir::South => (x, y, self.x, self.y),
            Dir::East | Dir::West => (y, x, self.y, self.x),
        };
        across == line && along >= start && along < start.saturating_add(self.width)
    }

    /// The same gap `dx`, `dy` cells over: a room's exit in floor cells.
    #[must_use]
    pub const fn shifted(self, (dx, dy): (usize, usize)) -> Self {
        Self {
            x: self.x.saturating_add(dx),
            y: self.y.saturating_add(dy),
            ..self
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EnemyKind {
    Rusher,
    Shooter,
    /// A spread shooter while the run's `spread_shooter` experiment is on, else a shooter.
    SpreadShooter,
}

/// An enemy spawned at the center of cell (`x`, `y`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Placement {
    pub kind: EnemyKind,
    pub x: usize,
    pub y: usize,
}

/// When a reinforcement layer spawns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LayerTrigger {
    /// Every enemy of the previous wave is dead.
    OnEnemiesCleared,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Reinforcement {
    pub trigger: LayerTrigger,
    pub placements: &'static [Placement],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RoomTrigger {
    /// The party enters a room that still has enemies to fight.
    OnEnterWithEnemies,
    /// The room's last wave is dead.
    OnEnemiesCleared,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RoomAction {
    Seal,
    Unseal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PrototypeRoom {
    pub name: &'static str,
    pub category: Category,
    pub theme: Theme,
    /// Row-major text grid; see [`Cell`] for the characters.
    pub cells: &'static [&'static str],
    pub exits: &'static [Exit],
    /// Object layer spawned on entry: wave 0.
    pub base: &'static [Placement],
    /// Later waves in order: reinforcement `i` is wave `i + 1`.
    pub reinforcements: &'static [Reinforcement],
    pub events: &'static [(RoomTrigger, RoomAction)],
    /// The extraction pad's cell; `Some` in exactly the [`Category::Exit`] rooms. A living
    /// player touching it once the room is clear wins the run. Mirrors ETG's exit-room
    /// elevator: a placeable, not a cell type.
    pub extraction: Option<(usize, usize)>,
}

/// Why a room failed validation. Cells are `(x, y)`; `exit` indexes the room's exits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoomError {
    EmptyGrid,
    TooLarge,
    RaggedRow {
        row: usize,
    },
    UnknownCell {
        x: usize,
        y: usize,
    },
    /// An exit cell is outside the grid or not on the exit's edge, or the exit is empty.
    ExitOffEdge {
        exit: usize,
    },
    /// Exit cells are the hatch's gap, so they must be floor.
    ExitNotFloor {
        exit: usize,
    },
    /// The cell just inside each exit cell must be floor: the way in from the hatch.
    ExitBlocked {
        exit: usize,
    },
    PlacementOffFloor {
        x: usize,
        y: usize,
    },
    /// Floor must be one 4-connected region without crossing pits: enemies can't cross
    /// them, and players only by falling in or rolling over.
    UnreachableFloor,
    /// Floor and pits must be walled in: only an exit cell may border void or the grid's
    /// edge, and only on its exit's side. Placed rooms then meet only through hatches.
    Unenclosed {
        x: usize,
        y: usize,
    },
    /// A room that can seal must end its `OnEnemiesCleared` actions unsealed.
    SealsForever,
    /// Exit rooms, and only exit rooms, have an extraction pad.
    ExtractionMismatch,
    ExtractionOffFloor,
}

impl RoomError {
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::EmptyGrid => "room grid has no floor",
            Self::TooLarge => "room grid exceeds MAX_SIDE",
            Self::RaggedRow { .. } => "room grid rows differ in length",
            Self::UnknownCell { .. } => "unknown cell character (use . # o or space)",
            Self::ExitOffEdge { .. } => "exit cells are not on the exit's edge",
            Self::ExitNotFloor { .. } => "exit cells must be floor",
            Self::ExitBlocked { .. } => "the cells just inside an exit must be floor",
            Self::PlacementOffFloor { .. } => "placement is not on a floor cell",
            Self::UnreachableFloor => "some floor is unreachable",
            Self::Unenclosed { .. } => "floor must be walled in except at exits",
            Self::SealsForever => "room seals but never unseals on OnEnemiesCleared",
            Self::ExtractionMismatch => "exit rooms, and only exit rooms, need an extraction pad",
            Self::ExtractionOffFloor => "extraction pad is not on a floor cell",
        }
    }
}

/// `items[i]` for const fns (`<[T]>::get` is not const yet).
pub(crate) const fn nth<T>(items: &[T], i: usize) -> Option<&T> {
    match items.split_at_checked(i) {
        Some((_, rest)) => rest.first(),
        None => None,
    }
}

const fn nth_mut<T>(items: &mut [T], i: usize) -> Option<&mut T> {
    match items.split_at_mut_checked(i) {
        Some((_, rest)) => rest.first_mut(),
        None => None,
    }
}

impl PrototypeRoom {
    #[must_use]
    pub const fn width(&self) -> usize {
        match self.cells.first() {
            Some(row) => row.len(),
            None => 0,
        }
    }

    #[must_use]
    pub const fn height(&self) -> usize {
        self.cells.len()
    }

    /// Off-grid (and, before validation, unknown) cells read as void.
    const fn cell_at(&self, x: usize, y: usize) -> Cell {
        let Some(row) = nth(self.cells, y) else {
            return Cell::Void;
        };
        match nth(row.as_bytes(), x) {
            Some(&byte) => match Cell::parse(byte) {
                Some(cell) => cell,
                None => Cell::Void,
            },
            None => Cell::Void,
        }
    }

    pub(crate) const fn floor_at(&self, x: usize, y: usize) -> bool {
        matches!(self.cell_at(x, y), Cell::Floor)
    }

    /// The cell at signed coordinates; off-grid is void.
    #[must_use]
    pub fn cell(&self, x: i32, y: i32) -> Cell {
        match (usize::try_from(x), usize::try_from(y)) {
            (Ok(x), Ok(y)) => self.cell_at(x, y),
            _ => Cell::Void,
        }
    }

    /// Index of the exit whose gap includes cell (`x`, `y`).
    const fn exit_at(&self, x: usize, y: usize) -> Option<usize> {
        let mut i = 0;
        while let Some(exit) = nth(self.exits, i) {
            if exit.covers(x, y) {
                return Some(i);
            }
            i = i.saturating_add(1);
        }
        None
    }

    /// The room's size class: `Some` when it is exactly a class's size and its exits are
    /// exactly that class's hatch points, in order.
    #[must_use]
    pub const fn size(&self) -> Option<Size> {
        let Some(size) = Size::of(self.width(), self.height()) else {
            return None;
        };
        let hatches = size.hatches();
        if self.exits.len() != hatches.len() {
            return None;
        }
        let mut i = 0;
        while let (Some(exit), Some(hatch)) = (nth(self.exits, i), nth(hatches, i)) {
            let same = exit.dir.is(hatch.dir)
                && exit.x == hatch.x
                && exit.y == hatch.y
                && exit.width == hatch.width
                && matches!(exit.kind, ExitKind::Either);
            if !same {
                return None;
            }
            i = i.saturating_add(1);
        }
        Some(size)
    }

    /// Whether any wave has enemies.
    #[must_use]
    pub fn has_enemies(&self) -> bool {
        !self.base.is_empty() || self.reinforcements.iter().any(|r| !r.placements.is_empty())
    }

    /// Checks everything issue #10 requires of a room. Runs at compile time via
    /// [`Self::valid`]; public so tests can exercise it directly.
    ///
    /// # Errors
    /// The first problem found.
    pub const fn validate(&self) -> Result<(), RoomError> {
        let (width, height) = (self.width(), self.height());
        if width == 0 || height == 0 {
            return Err(RoomError::EmptyGrid);
        }
        if width > MAX_SIDE || height > MAX_SIDE {
            return Err(RoomError::TooLarge);
        }
        let mut y = 0;
        while let Some(row) = nth(self.cells, y) {
            if row.len() != width {
                return Err(RoomError::RaggedRow { row: y });
            }
            let mut x = 0;
            while let Some(&byte) = nth(row.as_bytes(), x) {
                if Cell::parse(byte).is_none() {
                    return Err(RoomError::UnknownCell { x, y });
                }
                x = x.saturating_add(1);
            }
            y = y.saturating_add(1);
        }
        let mut index = 0;
        while let Some(exit) = nth(self.exits, index) {
            if let Err(error) = self.check_exit(exit, index) {
                return Err(error);
            }
            index = index.saturating_add(1);
        }
        if let Err(error) = self.check_placements(self.base) {
            return Err(error);
        }
        let mut index = 0;
        while let Some(layer) = nth(self.reinforcements, index) {
            if let Err(error) = self.check_placements(layer.placements) {
                return Err(error);
            }
            index = index.saturating_add(1);
        }
        if let Err(error) = self.check_enclosed() {
            return Err(error);
        }
        if let Err(error) = self.check_unseals() {
            return Err(error);
        }
        if matches!(self.category, Category::Exit) != self.extraction.is_some() {
            return Err(RoomError::ExtractionMismatch);
        }
        if let Some((x, y)) = self.extraction
            && !self.floor_at(x, y)
        {
            return Err(RoomError::ExtractionOffFloor);
        }
        self.check_connected()
    }

    /// Compile-time validation: use in `const` items only.
    ///
    /// # Panics
    /// When invalid, which in a `const` item is a build error naming the problem.
    // The panic is the build error; every caller is a const item, so it never runs live.
    #[allow(clippy::panic)]
    #[must_use]
    pub const fn valid(self) -> Self {
        match self.validate() {
            Ok(()) => self,
            Err(error) => panic!("{}", error.message()),
        }
    }

    const fn check_exit(&self, exit: &Exit, index: usize) -> Result<(), RoomError> {
        let (width, height) = (self.width(), self.height());
        if exit.width == 0 {
            return Err(RoomError::ExitOffEdge { exit: index });
        }
        let mut i = 0;
        while i < exit.width {
            let (x, y) = exit.cell(i);
            let on_edge = x < width
                && y < height
                && match exit.dir {
                    Dir::North => y == 0,
                    Dir::South => y == height.saturating_sub(1),
                    Dir::West => x == 0,
                    Dir::East => x == width.saturating_sub(1),
                };
            if !on_edge {
                return Err(RoomError::ExitOffEdge { exit: index });
            }
            i = i.saturating_add(1);
        }
        let mut i = 0;
        while i < exit.width {
            let (x, y) = exit.cell(i);
            if !self.floor_at(x, y) {
                return Err(RoomError::ExitNotFloor { exit: index });
            }
            match exit.inward((x, y)) {
                Some((x, y)) if self.floor_at(x, y) => {}
                _ => return Err(RoomError::ExitBlocked { exit: index }),
            }
            i = i.saturating_add(1);
        }
        Ok(())
    }

    const fn check_enclosed(&self) -> Result<(), RoomError> {
        let mut y = 0;
        while y < self.height() {
            let mut x = 0;
            while x < self.width() {
                if matches!(self.cell_at(x, y), Cell::Floor | Cell::Pit) {
                    let outward = match self.exit_at(x, y) {
                        Some(i) => match nth(self.exits, i) {
                            Some(exit) => Some(exit.dir),
                            None => None,
                        },
                        None => None,
                    };
                    let sides = [
                        (Dir::North, Some(x), y.checked_sub(1)),
                        (Dir::South, Some(x), y.checked_add(1)),
                        (Dir::West, x.checked_sub(1), Some(y)),
                        (Dir::East, x.checked_add(1), Some(y)),
                    ];
                    let mut s = 0;
                    while let Some(&(dir, nx, ny)) = nth(&sides, s) {
                        let open = match (nx, ny) {
                            (Some(nx), Some(ny)) => matches!(self.cell_at(nx, ny), Cell::Void),
                            _ => true,
                        };
                        let exit_side = match outward {
                            Some(out) => out.is(dir),
                            None => false,
                        };
                        if open && !exit_side {
                            return Err(RoomError::Unenclosed { x, y });
                        }
                        s = s.saturating_add(1);
                    }
                }
                x = x.saturating_add(1);
            }
            y = y.saturating_add(1);
        }
        Ok(())
    }

    const fn check_unseals(&self) -> Result<(), RoomError> {
        let (mut seals, mut locked_after_clear) = (false, true);
        let mut i = 0;
        while let Some(&(trigger, action)) = nth(self.events, i) {
            let seal = matches!(action, RoomAction::Seal);
            seals |= seal;
            if matches!(trigger, RoomTrigger::OnEnemiesCleared) {
                locked_after_clear = seal;
            }
            i = i.saturating_add(1);
        }
        if seals && locked_after_clear {
            Err(RoomError::SealsForever)
        } else {
            Ok(())
        }
    }

    const fn check_placements(&self, placements: &[Placement]) -> Result<(), RoomError> {
        let mut i = 0;
        while let Some(&Placement { x, y, .. }) = nth(placements, i) {
            if !self.floor_at(x, y) {
                return Err(RoomError::PlacementOffFloor { x, y });
            }
            i = i.saturating_add(1);
        }
        Ok(())
    }

    /// Every floor cell must be 4-connected to the first one. Floods by repeated sweeps
    /// over a `seen` grid: no work stack to size in const eval, and handmade rooms
    /// settle in a few passes.
    const fn check_connected(&self) -> Result<(), RoomError> {
        let floors = self.count_floor();
        if floors == 0 {
            return Err(RoomError::EmptyGrid);
        }
        let mut seen = [false; MAX_CELLS];
        let mut reached: usize = 0;
        let mut grew = true;
        while grew {
            grew = false;
            let mut y = 0;
            while y < self.height() {
                let mut x = 0;
                while x < self.width() {
                    // The first floor cell found seeds the fill.
                    let touches = reached == 0
                        || seen_at(&seen, x.saturating_add(1), y)
                        || seen_at(&seen, x, y.saturating_add(1))
                        || matches!(x.checked_sub(1), Some(left) if seen_at(&seen, left, y))
                        || matches!(y.checked_sub(1), Some(up) if seen_at(&seen, x, up));
                    let index = y.saturating_mul(MAX_SIDE).saturating_add(x);
                    if touches
                        && self.floor_at(x, y)
                        && let Some(cell) = nth_mut(&mut seen, index)
                        && !*cell
                    {
                        *cell = true;
                        reached = reached.saturating_add(1);
                        grew = true;
                    }
                    x = x.saturating_add(1);
                }
                y = y.saturating_add(1);
            }
        }
        if reached == floors {
            Ok(())
        } else {
            Err(RoomError::UnreachableFloor)
        }
    }

    const fn count_floor(&self) -> usize {
        let mut count: usize = 0;
        let mut y = 0;
        while y < self.height() {
            let mut x = 0;
            while x < self.width() {
                if self.floor_at(x, y) {
                    count = count.saturating_add(1);
                }
                x = x.saturating_add(1);
            }
            y = y.saturating_add(1);
        }
        count
    }
}

const fn max(a: usize, b: usize) -> usize {
    if a > b { a } else { b }
}

const fn min(a: usize, b: usize) -> usize {
    if a < b { a } else { b }
}

/// Whether cell (`x`, `y`) is marked in a [`MAX_SIDE`]-strided grid; off-grid is not.
const fn seen_at(seen: &[bool; MAX_CELLS], x: usize, y: usize) -> bool {
    x < MAX_SIDE
        && matches!(
            nth(seen, y.saturating_mul(MAX_SIDE).saturating_add(x)),
            Some(true)
        )
}

/// One end of a [`Connection`]: exit `exit` of room `room` (indices).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ExitRef {
    pub room: usize,
    pub exit: usize,
}

impl ExitRef {
    /// `==` for const fns.
    const fn is(self, other: Self) -> bool {
        self.room == other.room && self.exit == other.exit
    }
}

/// A hatch between two rooms: their linked exits, which must be the same floor cells.
/// Walkable both ways; `from`/`to` only give the layout a direction for [`ExitKind`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Connection {
    pub from: ExitRef,
    pub to: ExitRef,
}

/// A room placed in the floor grid: its cell (0, 0) is floor cell `at`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Placed {
    pub room: PrototypeRoom,
    pub at: (usize, usize),
}

impl Placed {
    /// One past the room's last floor column.
    const fn right(&self) -> usize {
        self.at.0.saturating_add(self.room.width())
    }

    /// One past the room's last floor row.
    const fn bottom(&self) -> usize {
        self.at.1.saturating_add(self.room.height())
    }

    /// The room's cell at floor cell (`x`, `y`) and its room-local coordinates; void
    /// outside the room.
    const fn floor_cell(&self, x: usize, y: usize) -> (Cell, usize, usize) {
        match (x.checked_sub(self.at.0), y.checked_sub(self.at.1)) {
            (Some(x), Some(y)) => (self.room.cell_at(x, y), x, y),
            _ => (Cell::Void, 0, 0),
        }
    }
}

/// A whole boarding target placed by hand: rooms placed in one floor grid, the hatches
/// joining them, and where the party starts. Tests use these; runs generate their ships
/// from hull templates.
///
/// Rooms may share wall cells; floor cells only meet where two linked exits coincide,
/// which is the hatch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Derelict {
    pub rooms: &'static [Placed],
    pub connections: &'static [Connection],
    pub start_room: usize,
    /// The party starts at this cell's center, in the start room's cells.
    pub start_cell: (usize, usize),
}

/// Why a derelict failed validation. `room`, `exit` and `connection` are indices; `x`
/// and `y` are floor cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DerelictError {
    Room {
        room: usize,
        error: RoomError,
    },
    NoRooms,
    TooManyRooms,
    StartOffFloor,
    /// There must be somewhere to win: at least one [`Category::Exit`] room.
    NoExitRoom,
    BadExitRef {
        connection: usize,
    },
    /// Linked exits must face each other, be equally wide, and sit on the same cells.
    MismatchedExits {
        connection: usize,
    },
    /// `from` must be an Exit/Either exit and `to` an Entrance/Either one.
    WrongExitKind {
        connection: usize,
    },
    /// Every exit must be linked exactly once.
    ExitLinks {
        room: usize,
        exit: usize,
    },
    /// Two rooms overlap other than wall on wall or at a hatch.
    Overlap {
        x: usize,
        y: usize,
    },
    /// No chain of hatches leads to this room from the start room.
    UnreachableRoom {
        room: usize,
    },
}

impl DerelictError {
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::Room { error, .. } => error.message(),
            Self::NoRooms => "derelict has no rooms",
            Self::TooManyRooms => "derelict exceeds MAX_ROOMS",
            Self::StartOffFloor => "start cell is not floor in the start room",
            Self::NoExitRoom => "derelict has no exit room",
            Self::BadExitRef { .. } => "connection names a missing room or exit",
            Self::MismatchedExits { .. } => {
                "linked exits must face each other, equally wide, on the same cells"
            }
            Self::WrongExitKind { .. } => "connection runs from an Entrance or into an Exit",
            Self::ExitLinks { .. } => "every exit must be linked exactly once",
            Self::Overlap { .. } => "rooms overlap other than wall on wall or at a hatch",
            Self::UnreachableRoom { .. } => "a room is unreachable from the start room",
        }
    }
}

impl Derelict {
    /// Validates every room, the start, the connections, how the rooms overlap, and that
    /// every room is reachable.
    ///
    /// # Errors
    /// The first problem found.
    pub const fn validate(&self) -> Result<(), DerelictError> {
        if self.rooms.is_empty() {
            return Err(DerelictError::NoRooms);
        }
        if self.rooms.len() > MAX_ROOMS {
            return Err(DerelictError::TooManyRooms);
        }
        let mut r = 0;
        while let Some(placed) = nth(self.rooms, r) {
            if let Err(error) = placed.room.validate() {
                return Err(DerelictError::Room { room: r, error });
            }
            r = r.saturating_add(1);
        }
        match nth(self.rooms, self.start_room) {
            Some(placed) if placed.room.floor_at(self.start_cell.0, self.start_cell.1) => {}
            _ => return Err(DerelictError::StartOffFloor),
        }
        let mut c = 0;
        while let Some(link) = nth(self.connections, c) {
            let (Some(from), Some(to)) = (self.gap(link.from), self.gap(link.to)) else {
                return Err(DerelictError::BadExitRef { connection: c });
            };
            let (a, b) = (from.cell(0), to.cell(0));
            if !from.dir.faces(to.dir) || from.width != to.width || a.0 != b.0 || a.1 != b.1 {
                return Err(DerelictError::MismatchedExits { connection: c });
            }
            if !from.kind.can_be_from() || !to.kind.can_be_to() {
                return Err(DerelictError::WrongExitKind { connection: c });
            }
            c = c.saturating_add(1);
        }
        let mut has_exit_room = false;
        let mut r = 0;
        while let Some(placed) = nth(self.rooms, r) {
            has_exit_room |= matches!(placed.room.category, Category::Exit);
            let mut e = 0;
            while e < placed.room.exits.len() {
                if self.links_of(ExitRef { room: r, exit: e }) != 1 {
                    return Err(DerelictError::ExitLinks { room: r, exit: e });
                }
                e = e.saturating_add(1);
            }
            r = r.saturating_add(1);
        }
        if !has_exit_room {
            return Err(DerelictError::NoExitRoom);
        }
        if let Err(error) = self.check_overlaps() {
            return Err(error);
        }
        self.check_reachable()
    }

    /// Compile-time validation: use in `const` items only.
    ///
    /// # Panics
    /// When invalid, which in a `const` item is a build error naming the problem.
    // The panic is the build error; every caller is a const item, so it never runs live.
    #[allow(clippy::panic)]
    #[must_use]
    pub const fn valid(self) -> Self {
        match self.validate() {
            Ok(()) => self,
            Err(error) => panic!("{}", error.message()),
        }
    }

    /// The exit `at` names, in floor cells.
    #[must_use]
    pub const fn gap(&self, at: ExitRef) -> Option<Exit> {
        match nth(self.rooms, at.room) {
            Some(placed) => match nth(placed.room.exits, at.exit) {
                Some(exit) => Some(exit.shifted(placed.at)),
                None => None,
            },
            None => None,
        }
    }

    const fn links_of(&self, at: ExitRef) -> usize {
        let mut count: usize = 0;
        let mut c = 0;
        while let Some(link) = nth(self.connections, c) {
            let ends = [link.from, link.to];
            let mut i = 0;
            while let Some(end) = nth(&ends, i) {
                if end.is(at) {
                    count = count.saturating_add(1);
                }
                i = i.saturating_add(1);
            }
            c = c.saturating_add(1);
        }
        count
    }

    /// Whether a connection joins these two exits, either way round.
    const fn linked(&self, one: ExitRef, other: ExitRef) -> bool {
        let mut c = 0;
        while let Some(link) = nth(self.connections, c) {
            let (from, to) = (link.from, link.to);
            if (from.is(one) && to.is(other)) || (to.is(one) && from.is(other)) {
                return true;
            }
            c = c.saturating_add(1);
        }
        false
    }

    /// Where two rooms both have a cell, it must be wall in both, or a hatch: floor in
    /// both, under exits linked to each other.
    const fn check_overlaps(&self) -> Result<(), DerelictError> {
        let mut a = 0;
        while let Some(first) = nth(self.rooms, a) {
            let mut b = a.saturating_add(1);
            while let Some(second) = nth(self.rooms, b) {
                // Only where their rectangles meet, in floor cells.
                let (left, top) = (max(first.at.0, second.at.0), max(first.at.1, second.at.1));
                let right = min(first.right(), second.right());
                let bottom = min(first.bottom(), second.bottom());
                let mut fy = top;
                while fy < bottom {
                    let mut fx = left;
                    while fx < right {
                        let (mine, x, y) = first.floor_cell(fx, fy);
                        let (theirs, sx, sy) = second.floor_cell(fx, fy);
                        let ok = match (mine, theirs) {
                            (Cell::Void, _) | (_, Cell::Void) | (Cell::Wall, Cell::Wall) => true,
                            (Cell::Floor, Cell::Floor) => {
                                match (first.room.exit_at(x, y), second.room.exit_at(sx, sy)) {
                                    (Some(ea), Some(eb)) => self.linked(
                                        ExitRef { room: a, exit: ea },
                                        ExitRef { room: b, exit: eb },
                                    ),
                                    _ => false,
                                }
                            }
                            _ => false,
                        };
                        if !ok {
                            return Err(DerelictError::Overlap { x: fx, y: fy });
                        }
                        fx = fx.saturating_add(1);
                    }
                    fy = fy.saturating_add(1);
                }
                b = b.saturating_add(1);
            }
            a = a.saturating_add(1);
        }
        Ok(())
    }

    /// Every room joins the start room through some chain of connections.
    const fn check_reachable(&self) -> Result<(), DerelictError> {
        let mut reached = [false; MAX_ROOMS];
        if let Some(start) = nth_mut(&mut reached, self.start_room) {
            *start = true;
        }
        let mut grew = true;
        while grew {
            grew = false;
            let mut c = 0;
            while let Some(link) = nth(self.connections, c) {
                let from = matches!(nth(&reached, link.from.room), Some(true));
                let to = matches!(nth(&reached, link.to.room), Some(true));
                if from != to {
                    let other = if from { link.to.room } else { link.from.room };
                    if let Some(slot) = nth_mut(&mut reached, other) {
                        *slot = true;
                        grew = true;
                    }
                }
                c = c.saturating_add(1);
            }
        }
        let mut r = 0;
        while r < self.rooms.len() {
            if !matches!(nth(&reached, r), Some(true)) {
                return Err(DerelictError::UnreachableRoom { room: r });
            }
            r = r.saturating_add(1);
        }
        Ok(())
    }
}

/// The cell index containing coordinate `v` (floor division by [`CELL`]).
#[must_use]
pub fn cell_of(v: Fx) -> i32 {
    // |bits >> 37| < 2^27, so this always fits.
    i32::try_from(v.to_bits() >> CELL_SHIFT).unwrap_or(i32::MIN)
}

/// Where cell `c` starts along an axis.
#[must_use]
pub fn cell_start(c: i32) -> Fx {
    CELL.saturating_mul_int(i64::from(c))
}

/// World position of cell (`x`, `y`)'s center.
#[must_use]
pub fn cell_center(x: usize, y: usize) -> FxVec2 {
    let at = |c: usize| {
        let c = i64::try_from(c).unwrap_or(i64::MAX);
        CELL.saturating_mul_int(c).saturating_add(HALF_CELL)
    };
    FxVec2 { x: at(x), y: at(y) }
}
