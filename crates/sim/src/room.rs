//! Room format (issue #10), mirroring Enter the Gungeon's `PrototypeDungeonRoom`.
//!
//! Rooms are plain typed `'static` values. The cell grid is a text block (one string per
//! row, row-major); everything else is typed fields. Validation is a `const fn`: room and
//! derelict constants are wrapped in `.valid()`, so a malformed room fails the build and
//! the sim only ever sees validated data.
//!
//! Coordinates are integer cells in the data. In the sim, a room's world space is
//! room-local points: the origin is cell (0, 0)'s top-left corner and +y points down.

use crate::{Fx, FxVec2, RoomId};

/// Side of one cell in world units (points).
pub const CELL: Fx = Fx::from_bits(32 << 32);
const HALF_CELL: Fx = Fx::from_bits(16 << 32);
/// `Fx` bits -> cell index is a floor shift: 32 fractional bits + log2(32).
const CELL_SHIFT: u32 = 37;
/// Grid size cap per side; bounds the const flood fill's scratch arrays.
pub const MAX_SIDE: usize = 64;
const MAX_CELLS: usize = MAX_SIDE * MAX_SIDE;
/// Rooms per derelict: the sim keeps the cleared set as a `u64` bitmask.
pub const MAX_ROOMS: usize = 64;

/// One grid cell. Text: `.` floor, `#` wall, `o` pit, space = void (outside the room).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    North,
    East,
    South,
    West,
}

impl Dir {
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

/// A door gap in the room's edge: `width` floor cells starting at (`x`, `y`) and running
/// along the edge (rightward on north/south edges, downward on east/west ones).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Exit {
    pub dir: Dir,
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub kind: ExitKind,
}

impl Exit {
    /// The `i`th edge cell.
    const fn cell(&self, i: usize) -> (usize, usize) {
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

    /// Where a party arriving through this exit stands: centered on the gap, one cell in.
    #[must_use]
    pub fn arrival(&self) -> FxVec2 {
        let (x, y) = self.inward(self.cell(0)).unwrap_or((self.x, self.y));
        let first = cell_center(x, y);
        // From the first cell's center to the gap's center: (width - 1) half-cells.
        let shift = HALF_CELL.saturating_mul(Fx::from_num(self.width.saturating_sub(1)));
        match self.dir {
            Dir::North | Dir::South => FxVec2 {
                x: first.x.saturating_add(shift),
                y: first.y,
            },
            Dir::East | Dir::West => FxVec2 {
                x: first.x,
                y: first.y.saturating_add(shift),
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnemyKind {
    Rusher,
    Shooter,
    /// A spread shooter while the run's `spread_shooter` experiment is on, else a shooter.
    SpreadShooter,
}

/// An enemy spawned at the center of cell (`x`, `y`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    pub kind: EnemyKind,
    pub x: usize,
    pub y: usize,
}

/// When a reinforcement layer spawns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerTrigger {
    /// Every enemy of the previous wave is dead.
    OnEnemiesCleared,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reinforcement {
    pub trigger: LayerTrigger,
    pub placements: &'static [Placement],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoomTrigger {
    /// The party enters a room that still has enemies to fight.
    OnEnterWithEnemies,
    /// The room's last wave is dead.
    OnEnemiesCleared,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoomAction {
    Seal,
    Unseal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrototypeRoom {
    pub name: &'static str,
    pub category: Category,
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
    /// Exit cells are the door gap, so they must be floor.
    ExitNotFloor {
        exit: usize,
    },
    /// The cell just inside each exit cell must be floor: arrivals land there.
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
            Self::SealsForever => "room seals but never unseals on OnEnemiesCleared",
            Self::ExtractionMismatch => "exit rooms, and only exit rooms, need an extraction pad",
            Self::ExtractionOffFloor => "extraction pad is not on a floor cell",
        }
    }
}

/// `items[i]` for const fns (`<[T]>::get` is not const yet).
const fn nth<T>(items: &[T], i: usize) -> Option<&T> {
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

    const fn floor_at(&self, x: usize, y: usize) -> bool {
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
    #[must_use]
    pub fn exit_at(&self, x: i32, y: i32) -> Option<usize> {
        let (x, y) = (usize::try_from(x).ok()?, usize::try_from(y).ok()?);
        self.exits.iter().position(|e| e.covers(x, y))
    }

    /// Whether a box of half-size `half` centered at `p` overlaps the extraction pad.
    #[must_use]
    pub fn on_extraction(&self, p: FxVec2, half: Fx) -> bool {
        let Some((x, y)) = self.extraction else {
            return false;
        };
        let spans = |c: Fx, cell: usize| {
            let cell = i32::try_from(cell).unwrap_or(i32::MAX);
            let (lo, hi) = (c.saturating_sub(half), c.saturating_add(half));
            (cell_of(lo)..=cell_of(hi.saturating_sub(Fx::DELTA))).contains(&cell)
        };
        spans(p.x, x) && spans(p.y, y)
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

/// Whether cell (`x`, `y`) is marked in a [`MAX_SIDE`]-strided grid; off-grid is not.
const fn seen_at(seen: &[bool; MAX_CELLS], x: usize, y: usize) -> bool {
    x < MAX_SIDE
        && matches!(
            nth(seen, y.saturating_mul(MAX_SIDE).saturating_add(x)),
            Some(true)
        )
}

/// One end of a [`Connection`]: exit `exit` of room `room` (indices).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExitRef {
    pub room: usize,
    pub exit: usize,
}

/// A hand-written door between two rooms. Walkable both ways; `from`/`to` only give the
/// layout a direction for [`ExitKind`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Connection {
    pub from: ExitRef,
    pub to: ExitRef,
}

/// A whole boarding target: rooms, how their exits link, and where the party starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Derelict {
    pub rooms: &'static [PrototypeRoom],
    pub connections: &'static [Connection],
    pub start_room: usize,
    /// The party starts at this cell's center.
    pub start_cell: (usize, usize),
}

/// Why a derelict failed validation. `room`, `exit` and `connection` are indices.
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
    /// Linked exits must face each other and be equally wide.
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
            Self::MismatchedExits { .. } => "linked exits must face each other, equally wide",
            Self::WrongExitKind { .. } => "connection runs from an Entrance or into an Exit",
            Self::ExitLinks { .. } => "every exit must be linked exactly once",
        }
    }
}

impl Derelict {
    /// Validates every room, the start, and the connections.
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
        while let Some(room) = nth(self.rooms, r) {
            if let Err(error) = room.validate() {
                return Err(DerelictError::Room { room: r, error });
            }
            r = r.saturating_add(1);
        }
        match nth(self.rooms, self.start_room) {
            Some(room) if room.floor_at(self.start_cell.0, self.start_cell.1) => {}
            _ => return Err(DerelictError::StartOffFloor),
        }
        let mut c = 0;
        while let Some(link) = nth(self.connections, c) {
            let (Some(from), Some(to)) = (self.exit(link.from), self.exit(link.to)) else {
                return Err(DerelictError::BadExitRef { connection: c });
            };
            if !from.dir.faces(to.dir) || from.width != to.width {
                return Err(DerelictError::MismatchedExits { connection: c });
            }
            if !from.kind.can_be_from() || !to.kind.can_be_to() {
                return Err(DerelictError::WrongExitKind { connection: c });
            }
            c = c.saturating_add(1);
        }
        let mut has_exit_room = false;
        let mut r = 0;
        while let Some(room) = nth(self.rooms, r) {
            has_exit_room |= matches!(room.category, Category::Exit);
            let mut e = 0;
            while e < room.exits.len() {
                if self.links_of(ExitRef { room: r, exit: e }) != 1 {
                    return Err(DerelictError::ExitLinks { room: r, exit: e });
                }
                e = e.saturating_add(1);
            }
            r = r.saturating_add(1);
        }
        if has_exit_room {
            Ok(())
        } else {
            Err(DerelictError::NoExitRoom)
        }
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

    const fn exit(&self, at: ExitRef) -> Option<&Exit> {
        match nth(self.rooms, at.room) {
            Some(room) => nth(room.exits, at.exit),
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
                if end.room == at.room && end.exit == at.exit {
                    count = count.saturating_add(1);
                }
                i = i.saturating_add(1);
            }
            c = c.saturating_add(1);
        }
        count
    }

    #[must_use]
    pub fn room(&self, id: RoomId) -> Option<&'static PrototypeRoom> {
        self.rooms.get(usize::from(id.0))
    }

    /// Where the party starts: the start room and a point in it.
    #[must_use]
    pub fn start(&self) -> (RoomId, FxVec2) {
        let room = RoomId(u16::try_from(self.start_room).unwrap_or(0));
        (room, cell_center(self.start_cell.0, self.start_cell.1))
    }

    /// The far side of `room`'s exit `exit`: the linked room and its exit index.
    #[must_use]
    pub fn link(&self, room: RoomId, exit: usize) -> Option<(RoomId, usize)> {
        let here = ExitRef {
            room: usize::from(room.0),
            exit,
        };
        let there = self.connections.iter().find_map(|c| {
            if c.from == here {
                Some(c.to)
            } else if c.to == here {
                Some(c.from)
            } else {
                None
            }
        })?;
        Some((RoomId(u16::try_from(there.room).ok()?), there.exit))
    }
}

/// What is moving through the tiles. Walls, void and sealed doors stop everything; pits
/// differ by body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Body {
    /// Enemies: pits stop them.
    Walker,
    /// Players walk onto pits, and fall in (see `Player`).
    Player,
    /// Flies over pits.
    Shot,
}

/// A room's collision view: its cells, with the exit gaps shut while `sealed`.
#[derive(Clone, Copy, Debug)]
pub struct Tiles {
    pub room: &'static PrototypeRoom,
    pub sealed: bool,
}

impl Tiles {
    #[must_use]
    pub fn blocks(self, x: i32, y: i32, body: Body) -> bool {
        if self.sealed && self.room.exit_at(x, y).is_some() {
            return true;
        }
        match (self.room.cell(x, y), body) {
            (Cell::Floor, _) | (Cell::Pit, Body::Player | Body::Shot) => false,
            (Cell::Wall | Cell::Void, _) | (Cell::Pit, Body::Walker) => true,
        }
    }

    /// Whether `p` is over a pit.
    #[must_use]
    pub fn pit_at(self, p: FxVec2) -> bool {
        self.room.cell(cell_of(p.x), cell_of(p.y)) == Cell::Pit
    }

    /// Whether any pit cell overlaps the square of half-size `reach` around `p`: some pit
    /// lies closer than `reach` to `p` along both axes.
    #[must_use]
    pub fn pit_within(self, p: FxVec2, reach: Fx) -> bool {
        let span = |v: Fx| {
            cell_of(v.saturating_sub(reach))
                ..=cell_of(v.saturating_add(reach).saturating_sub(Fx::DELTA))
        };
        span(p.y).any(|y| span(p.x).any(|x| self.room.cell(x, y) == Cell::Pit))
    }

    /// Whether the cell containing `p` blocks `body`.
    #[must_use]
    pub fn blocks_point(self, p: FxVec2, body: Body) -> bool {
        self.blocks(cell_of(p.x), cell_of(p.y), body)
    }

    /// Moves a box of half-size `half` by `delta`, x then y, stopping flush against
    /// blocking cells. Steps must be shorter than a cell, which all speeds are.
    #[must_use]
    pub fn slide(self, pos: FxVec2, half: Fx, delta: FxVec2, body: Body) -> FxVec2 {
        let x = self.sweep(pos.x, pos.y, half, delta.x, body, |along, across| {
            (along, across)
        });
        let y = self.sweep(pos.y, x, half, delta.y, body, |along, across| {
            (across, along)
        });
        FxVec2 { x, y }
    }

    /// One axis of [`Self::slide`]: `to_xy` maps (along, across) cell indices to (x, y).
    fn sweep(
        self,
        along: Fx,
        across: Fx,
        half: Fx,
        delta: Fx,
        body: Body,
        to_xy: impl Fn(i32, i32) -> (i32, i32),
    ) -> Fx {
        if delta == Fx::ZERO {
            return along;
        }
        let moved = along.saturating_add(delta);
        let forward = delta > Fx::ZERO;
        // The box spans [c - half, c + half): its far edge's cell is one bit short of it.
        let lead = if forward {
            cell_of(moved.saturating_add(half).saturating_sub(Fx::DELTA))
        } else {
            cell_of(moved.saturating_sub(half))
        };
        let first = cell_of(across.saturating_sub(half));
        let last = cell_of(across.saturating_add(half).saturating_sub(Fx::DELTA));
        let hit = (first..=last).any(|c| {
            let (x, y) = to_xy(lead, c);
            self.blocks(x, y, body)
        });
        match (hit, forward) {
            (false, _) => moved,
            (true, true) => cell_start(lead).saturating_sub(half),
            (true, false) => cell_start(lead.saturating_add(1)).saturating_add(half),
        }
    }
}

/// The cell index containing coordinate `v` (floor division by [`CELL`]).
#[must_use]
pub fn cell_of(v: Fx) -> i32 {
    // |bits >> 37| < 2^27, so this always fits.
    i32::try_from(v.to_bits() >> CELL_SHIFT).unwrap_or(i32::MIN)
}

fn cell_start(c: i32) -> Fx {
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
