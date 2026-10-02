//! Hull templates (issues #27, #31): a ship class drawn in ASCII, one character per
//! floor cell, bow east.
//!
//! - `#` hull: the walls around the corridors.
//! - `=` corridor floor. Each 4-connected stretch of it is one region of the ship.
//! - A letter: a slot, a rectangle exactly its size class, filled from the room pool.
//! - Space: outside the ship.
//!
//! Each slot's legend ([`Slot`]) gives its size class, its zone, whether it's optional,
//! which of its standard hatch points open onto a corridor, and for an airlock, which
//! opens onto space. The party boards through a random airlock, and once the bridge (the
//! boss slot) is clear, leaves through any. Every room of a class
//! has the same hatch points (see [`Size::hatches`]), so filling can't fail: any pool room
//! of the slot's class and zone fits it. [`Ship::generate`](crate::Ship::generate) fills
//! a template from a seed.
//!
//! Templates are validated against the pool when the crate builds (see
//! [`Template::validate`]); that the regions then always join up is proved by a seed
//! sweep in the tests.

use crate::pool::POOL;
use crate::rng::Rng;
use crate::room::{Category, Dir, PrototypeRoom, Size, Theme, nth};

/// Where along the ship a slot is. Position gives function (issue #31).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Zone {
    /// On the hull's edge: airlocks.
    Hull,
    /// Bow: command, the bridge.
    Fore,
    /// Midship: crew and cargo.
    Mid,
    /// Stern: engineering.
    Aft,
    /// Behind an access panel off a compartment: a maintenance crawlspace, the secret
    /// room. Its hatches start as panels.
    Crawlspace,
    /// The ship's stores: the reward room.
    Stores,
}

impl Zone {
    /// The zone rooms of `theme` belong in; corridors run through them all.
    #[must_use]
    pub const fn of(theme: Theme) -> Option<Self> {
        match theme {
            Theme::Corridor => None,
            Theme::Airlock => Some(Self::Hull),
            Theme::Bridge => Some(Self::Fore),
            Theme::Cargo | Theme::Crew => Some(Self::Mid),
            Theme::Engineering => Some(Self::Aft),
            Theme::Crawlspace => Some(Self::Crawlspace),
            Theme::Stores => Some(Self::Stores),
        }
    }
}

/// One slot's legend.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Slot {
    /// Its cells' character in the template.
    pub letter: u8,
    pub size: Size,
    pub zone: Zone,
    /// A coin flip per ship fills it or leaves it out. Left out, its cells are outside the
    /// ship, except those that wall in a corridor.
    pub optional: bool,
    /// The sides whose hatch points open onto a corridor; the rest are walled.
    pub hatches: &'static [Dir],
    /// An airlock: the side whose hatch point is its outer hatch, onto space.
    pub airlock: Option<Dir>,
}

impl Slot {
    /// Whether `room` can fill this slot: it's the slot's size class, and its theme
    /// belongs in the slot's zone.
    #[must_use]
    pub const fn fits(&self, room: &PrototypeRoom) -> bool {
        let size = matches!(
            (room.size(), self.size),
            (Some(Size::S), Size::S) | (Some(Size::M), Size::M) | (Some(Size::L), Size::L)
        );
        let zone = matches!(
            (Zone::of(room.theme), self.zone),
            (Some(Zone::Hull), Zone::Hull)
                | (Some(Zone::Fore), Zone::Fore)
                | (Some(Zone::Mid), Zone::Mid)
                | (Some(Zone::Aft), Zone::Aft)
                | (Some(Zone::Crawlspace), Zone::Crawlspace)
                | (Some(Zone::Stores), Zone::Stores)
        );
        size && zone
    }
}

/// A ship class: its hull drawn in ASCII (see the module docs) and its slots' legends.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Template {
    pub name: &'static str,
    /// Row-major, one character per floor cell.
    pub rows: &'static [&'static str],
    pub slots: &'static [Slot],
}

/// Why a template failed validation. Cells are `(x, y)`; `slot` indexes the legend, and
/// `room` the pool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemplateError {
    Empty,
    RaggedRow {
        row: usize,
    },
    /// Not `#`, `=`, space, or a letter in the legend.
    UnknownCell {
        x: usize,
        y: usize,
    },
    /// Two legends for one letter, or one that isn't a letter.
    BadLetter {
        slot: usize,
    },
    /// The slot's cells aren't exactly one rectangle of its size class.
    SlotShape {
        slot: usize,
    },
    /// A slot must open at least one hatch, each side at most once, and an airlock's outer
    /// hatch on none of them.
    BadHatches {
        slot: usize,
    },
    /// Just outside each of a slot's open hatch points must be corridor.
    HatchOffCorridor {
        slot: usize,
    },
    /// Just outside an airlock's outer hatch must be outside the ship.
    AirlockOffHull {
        slot: usize,
    },
    /// Corridor must be walled in: by hull, or by a slot's side.
    OpenCorridor {
        x: usize,
        y: usize,
    },
    /// Hull must touch corridor (diagonals count): it's that corridor's wall.
    StrayHull {
        x: usize,
        y: usize,
    },
    /// Pool rooms must be exactly a size class, with its hatch points as their exits.
    RoomOffClass {
        room: usize,
    },
    /// No pool room fits the slot.
    EmptySlot {
        slot: usize,
    },
    /// An airlock is boarded through: it must never be left out, and only take entrance
    /// rooms with floor at their center.
    BadAirlock {
        slot: usize,
    },
    /// There must be an airlock: somewhere to board, and to leave.
    NoAirlock,
    /// Some slot that's never left out must only take boss rooms: the bridge, whose
    /// clearing unlocks the airlocks.
    NoBoss,
}

impl TemplateError {
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::Empty => "template has no cells",
            Self::RaggedRow { .. } => "template rows differ in length",
            Self::UnknownCell { .. } => "unknown template cell (use # = space or a slot letter)",
            Self::BadLetter { .. } => "slot letters must be letters, each with one legend",
            Self::SlotShape { .. } => "a slot's cells must be one rectangle of its size class",
            Self::BadHatches { .. } => {
                "a slot must open at least one hatch, each side once, none its outer hatch"
            }
            Self::HatchOffCorridor { .. } => "an open hatch point must face corridor",
            Self::AirlockOffHull { .. } => "an airlock's outer hatch must face outside the ship",
            Self::OpenCorridor { .. } => "corridor must be walled in by hull or slots",
            Self::StrayHull { .. } => "hull must touch corridor",
            Self::RoomOffClass { .. } => "a pool room is not exactly a size class",
            Self::EmptySlot { .. } => "no pool room fits a slot",
            Self::BadAirlock { .. } => {
                "an airlock must always hold an entrance, floor at its center"
            }
            Self::NoAirlock => "a template needs an airlock",
            Self::NoBoss => "some always-filled slot must only take boss rooms",
        }
    }
}

/// The cell one step from (`x`, `y`) toward side `dir`; `None` off the top or left.
pub(crate) const fn toward((x, y): (usize, usize), dir: Dir) -> Option<(usize, usize)> {
    let (dx, dy) = match dir {
        Dir::North => (0, -1),
        Dir::East => (1, 0),
        Dir::South => (0, 1),
        Dir::West => (-1, 0),
    };
    match (x.checked_add_signed(dx), y.checked_add_signed(dy)) {
        (Some(x), Some(y)) => Some((x, y)),
        _ => None,
    }
}

/// The 8 neighbors' offsets.
pub(crate) const AROUND: [(isize, isize); 8] = [
    (-1, -1),
    (0, -1),
    (1, -1),
    (-1, 0),
    (1, 0),
    (-1, 1),
    (0, 1),
    (1, 1),
];

/// Whether a room's four center cells, where its hatch points line up, are floor.
const fn centered(room: &PrototypeRoom) -> bool {
    let (x, y) = (room.width() / 2, room.height() / 2);
    let (left, top) = (x.saturating_sub(1), y.saturating_sub(1));
    room.floor_at(left, top)
        && room.floor_at(x, top)
        && room.floor_at(left, y)
        && room.floor_at(x, y)
}

impl Template {
    /// (width, height) in cells.
    #[must_use]
    pub const fn size(&self) -> (usize, usize) {
        match self.rows.first() {
            Some(row) => (row.len(), self.rows.len()),
            None => (0, 0),
        }
    }

    /// The character at (`x`, `y`); off the grid is outside the ship.
    #[must_use]
    pub const fn at(&self, x: usize, y: usize) -> u8 {
        match nth(self.rows, y) {
            Some(row) => match nth(row.as_bytes(), x) {
                Some(&c) => c,
                None => b' ',
            },
            None => b' ',
        }
    }

    /// The character `(dx, dy)` from (`x`, `y`).
    const fn near(&self, (x, y): (usize, usize), (dx, dy): (isize, isize)) -> u8 {
        match (x.checked_add_signed(dx), y.checked_add_signed(dy)) {
            (Some(x), Some(y)) => self.at(x, y),
            _ => b' ',
        }
    }

    /// Index of `letter`'s legend.
    const fn slot(&self, letter: u8) -> Option<usize> {
        let mut s = 0;
        while let Some(slot) = nth(self.slots, s) {
            if slot.letter == letter {
                return Some(s);
            }
            s = s.saturating_add(1);
        }
        None
    }

    /// The top-left cell of `letter`'s slot: its first cell in row-major order.
    #[must_use]
    pub const fn origin(&self, letter: u8) -> Option<(usize, usize)> {
        let mut y = 0;
        while let Some(row) = nth(self.rows, y) {
            let (mut x, mut rest) = (0, row.as_bytes());
            while let [c, tail @ ..] = rest {
                if *c == letter {
                    return Some((x, y));
                }
                (x, rest) = (x.saturating_add(1), tail);
            }
            y = y.saturating_add(1);
        }
        None
    }

    /// Checks the drawing, every slot's shape and hatches, and the pool against every
    /// slot. Runs at compile time via [`Self::valid`].
    ///
    /// # Errors
    /// The first problem found.
    pub const fn validate(&self) -> Result<(), TemplateError> {
        let (width, _) = self.size();
        if width == 0 {
            return Err(TemplateError::Empty);
        }
        let mut y = 0;
        while let Some(row) = nth(self.rows, y) {
            if row.len() != width {
                return Err(TemplateError::RaggedRow { row: y });
            }
            let mut x = 0;
            while let Some(&c) = nth(row.as_bytes(), x) {
                if !matches!(c, b'#' | b'=' | b' ') && self.slot(c).is_none() {
                    return Err(TemplateError::UnknownCell { x, y });
                }
                x = x.saturating_add(1);
            }
            y = y.saturating_add(1);
        }
        let mut s = 0;
        while let Some(slot) = nth(self.slots, s) {
            if let Err(error) = self.check_slot(s, slot) {
                return Err(error);
            }
            s = s.saturating_add(1);
        }
        let mut y = 0;
        while let Some(row) = nth(self.rows, y) {
            let (mut x, mut rest) = (0, row.as_bytes());
            while let [c, tail @ ..] = rest {
                if matches!(c, b'#' | b'=')
                    && let Err(error) = self.check_walls((x, y))
                {
                    return Err(error);
                }
                (x, rest) = (x.saturating_add(1), tail);
            }
            y = y.saturating_add(1);
        }
        self.check_pool()
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

    const fn check_slot(&self, s: usize, slot: &Slot) -> Result<(), TemplateError> {
        let one_legend = matches!(self.slot(slot.letter), Some(first) if first == s);
        if !slot.letter.is_ascii_alphabetic() || !one_legend {
            return Err(TemplateError::BadLetter { slot: s });
        }
        let shape = Err(TemplateError::SlotShape { slot: s });
        let Some((left, top)) = self.origin(slot.letter) else {
            return shape;
        };
        let (width, height) = slot.size.dims();
        let (mut inside, mut all) = (0_usize, 0_usize);
        // Walks each row's bytes as a slice pattern: looking up cell by cell would outlast
        // the const-eval budget on a whole ship.
        let mut y = 0;
        while let Some(row) = nth(self.rows, y) {
            let (mut x, mut rest) = (0, row.as_bytes());
            while let [c, tail @ ..] = rest {
                if *c == slot.letter {
                    all = all.saturating_add(1);
                    let in_rect = x >= left
                        && y >= top
                        && x < left.saturating_add(width)
                        && y < top.saturating_add(height);
                    if in_rect {
                        inside = inside.saturating_add(1);
                    }
                }
                (x, rest) = (x.saturating_add(1), tail);
            }
            y = y.saturating_add(1);
        }
        if inside != all || all != width.saturating_mul(height) {
            return shape;
        }
        if slot.hatches.is_empty() {
            return Err(TemplateError::BadHatches { slot: s });
        }
        let mut side = 0;
        while let Some(&dir) = nth(slot.hatches, side) {
            let mut earlier = 0;
            while earlier < side {
                if matches!(nth(slot.hatches, earlier), Some(&other) if same_side(other, dir)) {
                    return Err(TemplateError::BadHatches { slot: s });
                }
                earlier = earlier.saturating_add(1);
            }
            let gap = slot.size.hatch(dir).shifted((left, top));
            let mut i = 0;
            while i < gap.width {
                let corridor = match toward(gap.cell(i), dir) {
                    Some((x, y)) => self.at(x, y) == b'=',
                    None => false,
                };
                if !corridor {
                    return Err(TemplateError::HatchOffCorridor { slot: s });
                }
                i = i.saturating_add(1);
            }
            side = side.saturating_add(1);
        }
        let Some(outer) = slot.airlock else {
            return Ok(());
        };
        let mut side = 0;
        while let Some(&dir) = nth(slot.hatches, side) {
            if same_side(dir, outer) {
                return Err(TemplateError::BadHatches { slot: s });
            }
            side = side.saturating_add(1);
        }
        let gap = slot.size.hatch(outer).shifted((left, top));
        let mut i = 0;
        while i < gap.width {
            let space = match toward(gap.cell(i), outer) {
                Some((x, y)) => self.at(x, y) == b' ',
                None => true,
            };
            if !space {
                return Err(TemplateError::AirlockOffHull { slot: s });
            }
            i = i.saturating_add(1);
        }
        Ok(())
    }

    /// Corridor has no side open to outside the ship, and hull touches corridor.
    const fn check_walls(&self, (x, y): (usize, usize)) -> Result<(), TemplateError> {
        match self.at(x, y) {
            b'=' => {
                let sides = [(0, -1), (1, 0), (0, 1), (-1, 0)];
                let mut i = 0;
                while let Some(&side) = nth(&sides, i) {
                    if self.near((x, y), side) == b' ' {
                        return Err(TemplateError::OpenCorridor { x, y });
                    }
                    i = i.saturating_add(1);
                }
                Ok(())
            }
            b'#' => {
                let mut i = 0;
                while let Some(&offset) = nth(&AROUND, i) {
                    if self.near((x, y), offset) == b'=' {
                        return Ok(());
                    }
                    i = i.saturating_add(1);
                }
                Err(TemplateError::StrayHull { x, y })
            }
            _ => Ok(()),
        }
    }

    const fn check_pool(&self) -> Result<(), TemplateError> {
        let mut r = 0;
        while let Some(room) = nth(POOL, r) {
            if room.size().is_none() {
                return Err(TemplateError::RoomOffClass { room: r });
            }
            r = r.saturating_add(1);
        }
        let (mut has_airlock, mut has_boss) = (false, false);
        let mut s = 0;
        while let Some(slot) = nth(self.slots, s) {
            let (mut any, mut all_bosses, mut all_entrances) = (false, true, true);
            let mut r = 0;
            while let Some(room) = nth(POOL, r) {
                if slot.fits(room) {
                    any = true;
                    all_bosses &= matches!(room.category, Category::Boss);
                    all_entrances &= matches!(room.category, Category::Entrance) && centered(room);
                }
                r = r.saturating_add(1);
            }
            if !any {
                return Err(TemplateError::EmptySlot { slot: s });
            }
            if slot.airlock.is_some() {
                if slot.optional || !all_entrances {
                    return Err(TemplateError::BadAirlock { slot: s });
                }
                has_airlock = true;
            }
            has_boss |= !slot.optional && all_bosses;
            s = s.saturating_add(1);
        }
        if !has_airlock {
            return Err(TemplateError::NoAirlock);
        }
        if !has_boss {
            return Err(TemplateError::NoBoss);
        }
        Ok(())
    }
}

/// `==` for const fns.
const fn same_side(a: Dir, b: Dir) -> bool {
    matches!(
        (a, b),
        (Dir::North, Dir::North)
            | (Dir::East, Dir::East)
            | (Dir::South, Dir::South)
            | (Dir::West, Dir::West)
    )
}

const fn slot(letter: u8, size: Size, zone: Zone, hatches: &'static [Dir]) -> Slot {
    Slot {
        letter,
        size,
        zone,
        optional: false,
        hatches,
        airlock: None,
    }
}

const fn optional(slot: Slot) -> Slot {
    Slot {
        optional: true,
        ..slot
    }
}

/// `slot` as an airlock whose outer hatch is on side `outer`.
const fn airlock(slot: Slot, outer: Dir) -> Slot {
    Slot {
        airlock: Some(outer),
        ..slot
    }
}

/// The Corvette (issue #27, Rocinante-like): the first hand-built derelict, made a
/// template.
///
/// Three airlocks on the hull, port `A`, starboard `Z` and aft `X`: you board through a
/// random one. The midship hold `H` joins the port and starboard airlocks to the keel.
/// Aft down the keel are two optional side compartments, port `P` and starboard `S`, and
/// engineering `E` at the stern, with the aft airlock beyond it. Off engineering are the
/// stores `R` (the reward room, with a chest) and, behind an access panel at the end of a
/// short passage, the crawlspace `C` (with another). Fore through a bulkhead passage is
/// the bridge `B`, whose captain guards the airlocks' locks.
pub const CORVETTE: Template = Template {
    name: "corvette",
    rows: &[
        "                                                    AAAAAAAAAAAA                                  ",
        "                                                    AAAAAAAAAAAA                                  ",
        "                 CCCCCCCCCCCC                       AAAAAAAAAAAA                                  ",
        "                 CCCCCCCCCCCC                       AAAAAAAAAAAA                                  ",
        "                 CCCCCCCCCCCC                       AAAAAAAAAAAA                                  ",
        "                 CCCCCCCCCCCC                       AAAAAAAAAAAA                                  ",
        "                 CCCCCCCCCCCC                       AAAAAAAAAAAA                                  ",
        "                 CCCCCCCCCCCC                       AAAAAAAAAAAA                                  ",
        "                 CCCCCCCCCCCC                       AAAAAAAAAAAA                                  ",
        "                 CCCCCCCCCCCC    PPPPPPPPPPPP       AAAAAAAAAAAA                                  ",
        "                 CCCCCCCCCCCC    PPPPPPPPPPPP           #==#                                      ",
        "                 CCCCCCCCCCCC    PPPPPPPPPPPP           #==#                                      ",
        "                     #==#        PPPPPPPPPPPP           #==#                                      ",
        "                     #==#        PPPPPPPPPPPP HHHHHHHHHHHHHHHHHHHHHHHH    BBBBBBBBBBBBBBBBBBBBBBBB",
        "              EEEEEEEEEEEEEEEEEE PPPPPPPPPPPP HHHHHHHHHHHHHHHHHHHHHHHH    BBBBBBBBBBBBBBBBBBBBBBBB",
        "XXXXXXXXXXXX  EEEEEEEEEEEEEEEEEE PPPPPPPPPPPP HHHHHHHHHHHHHHHHHHHHHHHH    BBBBBBBBBBBBBBBBBBBBBBBB",
        "XXXXXXXXXXXX  EEEEEEEEEEEEEEEEEE PPPPPPPPPPPP HHHHHHHHHHHHHHHHHHHHHHHH    BBBBBBBBBBBBBBBBBBBBBBBB",
        "XXXXXXXXXXXX  EEEEEEEEEEEEEEEEEE PPPPPPPPPPPP HHHHHHHHHHHHHHHHHHHHHHHH    BBBBBBBBBBBBBBBBBBBBBBBB",
        "XXXXXXXXXXXX##EEEEEEEEEEEEEEEEEE#PPPPPPPPPPPP#HHHHHHHHHHHHHHHHHHHHHHHH####BBBBBBBBBBBBBBBBBBBBBBBB",
        "XXXXXXXXXXXX==EEEEEEEEEEEEEEEEEE==============HHHHHHHHHHHHHHHHHHHHHHHH====BBBBBBBBBBBBBBBBBBBBBBBB",
        "XXXXXXXXXXXX==EEEEEEEEEEEEEEEEEE==============HHHHHHHHHHHHHHHHHHHHHHHH====BBBBBBBBBBBBBBBBBBBBBBBB",
        "XXXXXXXXXXXX##EEEEEEEEEEEEEEEEEE#SSSSSSSSSSSS#HHHHHHHHHHHHHHHHHHHHHHHH####BBBBBBBBBBBBBBBBBBBBBBBB",
        "XXXXXXXXXXXX  EEEEEEEEEEEEEEEEEE SSSSSSSSSSSS HHHHHHHHHHHHHHHHHHHHHHHH    BBBBBBBBBBBBBBBBBBBBBBBB",
        "XXXXXXXXXXXX  EEEEEEEEEEEEEEEEEE SSSSSSSSSSSS HHHHHHHHHHHHHHHHHHHHHHHH    BBBBBBBBBBBBBBBBBBBBBBBB",
        "XXXXXXXXXXXX  EEEEEEEEEEEEEEEEEE SSSSSSSSSSSS HHHHHHHHHHHHHHHHHHHHHHHH    BBBBBBBBBBBBBBBBBBBBBBBB",
        "              EEEEEEEEEEEEEEEEEE SSSSSSSSSSSS HHHHHHHHHHHHHHHHHHHHHHHH    BBBBBBBBBBBBBBBBBBBBBBBB",
        "                     #==#        SSSSSSSSSSSS HHHHHHHHHHHHHHHHHHHHHHHH    BBBBBBBBBBBBBBBBBBBBBBBB",
        "                     #==#        SSSSSSSSSSSS           #==#                                      ",
        "                 RRRRRRRRRRRR    SSSSSSSSSSSS           #==#                                      ",
        "                 RRRRRRRRRRRR    SSSSSSSSSSSS           #==#                                      ",
        "                 RRRRRRRRRRRR    SSSSSSSSSSSS       ZZZZZZZZZZZZ                                  ",
        "                 RRRRRRRRRRRR                       ZZZZZZZZZZZZ                                  ",
        "                 RRRRRRRRRRRR                       ZZZZZZZZZZZZ                                  ",
        "                 RRRRRRRRRRRR                       ZZZZZZZZZZZZ                                  ",
        "                 RRRRRRRRRRRR                       ZZZZZZZZZZZZ                                  ",
        "                 RRRRRRRRRRRR                       ZZZZZZZZZZZZ                                  ",
        "                 RRRRRRRRRRRR                       ZZZZZZZZZZZZ                                  ",
        "                 RRRRRRRRRRRR                       ZZZZZZZZZZZZ                                  ",
        "                                                    ZZZZZZZZZZZZ                                  ",
        "                                                    ZZZZZZZZZZZZ                                  ",
    ],
    slots: &[
        airlock(slot(b'A', Size::S, Zone::Hull, &[Dir::South]), Dir::North),
        airlock(slot(b'Z', Size::S, Zone::Hull, &[Dir::North]), Dir::South),
        airlock(slot(b'X', Size::S, Zone::Hull, &[Dir::East]), Dir::West),
        slot(
            b'E',
            Size::M,
            Zone::Aft,
            &[Dir::North, Dir::East, Dir::South, Dir::West],
        ),
        slot(
            b'H',
            Size::L,
            Zone::Mid,
            &[Dir::North, Dir::East, Dir::South, Dir::West],
        ),
        slot(b'B', Size::L, Zone::Fore, &[Dir::West]),
        optional(slot(b'P', Size::S, Zone::Mid, &[Dir::South])),
        optional(slot(b'S', Size::S, Zone::Mid, &[Dir::North])),
        slot(b'C', Size::S, Zone::Crawlspace, &[Dir::South]),
        slot(b'R', Size::S, Zone::Stores, &[Dir::North]),
    ],
}
.valid();

/// The Freighter (issue #27): long and cargo-heavy. Three holds bulkhead the keel, so
/// you fight through each in turn: aft hold `H`, midship hold `G` and fore hold `K`.
///
/// Engineering `E` at the stern has the aft airlock `X` beyond it, the crawlspace `C`
/// behind an access panel above it, and the stores `R` below. The port `A` and
/// starboard `Z` airlocks open off the midship hold. Optional side compartments hang off
/// the aft hold (`P`, `S`) and the fore hold (`T`). The bridge `B` is past the fore hold.
pub const FREIGHTER: Template = Template {
    name: "freighter",
    rows: &[
        "                                        PPPPPPPPPPPP              AAAAAAAAAAAA              TTTTTTTTTTTT                                ",
        "                 CCCCCCCCCCCC           PPPPPPPPPPPP              AAAAAAAAAAAA              TTTTTTTTTTTT                                ",
        "                 CCCCCCCCCCCC           PPPPPPPPPPPP              AAAAAAAAAAAA              TTTTTTTTTTTT                                ",
        "                 CCCCCCCCCCCC           PPPPPPPPPPPP              AAAAAAAAAAAA              TTTTTTTTTTTT                                ",
        "                 CCCCCCCCCCCC           PPPPPPPPPPPP              AAAAAAAAAAAA              TTTTTTTTTTTT                                ",
        "                 CCCCCCCCCCCC           PPPPPPPPPPPP              AAAAAAAAAAAA              TTTTTTTTTTTT                                ",
        "                 CCCCCCCCCCCC           PPPPPPPPPPPP              AAAAAAAAAAAA              TTTTTTTTTTTT                                ",
        "                 CCCCCCCCCCCC           PPPPPPPPPPPP              AAAAAAAAAAAA              TTTTTTTTTTTT                                ",
        "                 CCCCCCCCCCCC           PPPPPPPPPPPP              AAAAAAAAAAAA              TTTTTTTTTTTT                                ",
        "                 CCCCCCCCCCCC           PPPPPPPPPPPP              AAAAAAAAAAAA              TTTTTTTTTTTT                                ",
        "                 CCCCCCCCCCCC               #==#                      #==#                      #==#                                    ",
        "                     #==#                   #==#                      #==#                      #==#                                    ",
        "                     #==#         HHHHHHHHHHHHHHHHHHHHHHHH  GGGGGGGGGGGGGGGGGGGGGGGG  KKKKKKKKKKKKKKKKKKKKKKKK  BBBBBBBBBBBBBBBBBBBBBBBB",
        "              EEEEEEEEEEEEEEEEEE  HHHHHHHHHHHHHHHHHHHHHHHH  GGGGGGGGGGGGGGGGGGGGGGGG  KKKKKKKKKKKKKKKKKKKKKKKK  BBBBBBBBBBBBBBBBBBBBBBBB",
        "XXXXXXXXXXXX  EEEEEEEEEEEEEEEEEE  HHHHHHHHHHHHHHHHHHHHHHHH  GGGGGGGGGGGGGGGGGGGGGGGG  KKKKKKKKKKKKKKKKKKKKKKKK  BBBBBBBBBBBBBBBBBBBBBBBB",
        "XXXXXXXXXXXX  EEEEEEEEEEEEEEEEEE  HHHHHHHHHHHHHHHHHHHHHHHH  GGGGGGGGGGGGGGGGGGGGGGGG  KKKKKKKKKKKKKKKKKKKKKKKK  BBBBBBBBBBBBBBBBBBBBBBBB",
        "XXXXXXXXXXXX  EEEEEEEEEEEEEEEEEE  HHHHHHHHHHHHHHHHHHHHHHHH  GGGGGGGGGGGGGGGGGGGGGGGG  KKKKKKKKKKKKKKKKKKKKKKKK  BBBBBBBBBBBBBBBBBBBBBBBB",
        "XXXXXXXXXXXX##EEEEEEEEEEEEEEEEEE##HHHHHHHHHHHHHHHHHHHHHHHH##GGGGGGGGGGGGGGGGGGGGGGGG##KKKKKKKKKKKKKKKKKKKKKKKK##BBBBBBBBBBBBBBBBBBBBBBBB",
        "XXXXXXXXXXXX==EEEEEEEEEEEEEEEEEE==HHHHHHHHHHHHHHHHHHHHHHHH==GGGGGGGGGGGGGGGGGGGGGGGG==KKKKKKKKKKKKKKKKKKKKKKKK==BBBBBBBBBBBBBBBBBBBBBBBB",
        "XXXXXXXXXXXX==EEEEEEEEEEEEEEEEEE==HHHHHHHHHHHHHHHHHHHHHHHH==GGGGGGGGGGGGGGGGGGGGGGGG==KKKKKKKKKKKKKKKKKKKKKKKK==BBBBBBBBBBBBBBBBBBBBBBBB",
        "XXXXXXXXXXXX##EEEEEEEEEEEEEEEEEE##HHHHHHHHHHHHHHHHHHHHHHHH##GGGGGGGGGGGGGGGGGGGGGGGG##KKKKKKKKKKKKKKKKKKKKKKKK##BBBBBBBBBBBBBBBBBBBBBBBB",
        "XXXXXXXXXXXX  EEEEEEEEEEEEEEEEEE  HHHHHHHHHHHHHHHHHHHHHHHH  GGGGGGGGGGGGGGGGGGGGGGGG  KKKKKKKKKKKKKKKKKKKKKKKK  BBBBBBBBBBBBBBBBBBBBBBBB",
        "XXXXXXXXXXXX  EEEEEEEEEEEEEEEEEE  HHHHHHHHHHHHHHHHHHHHHHHH  GGGGGGGGGGGGGGGGGGGGGGGG  KKKKKKKKKKKKKKKKKKKKKKKK  BBBBBBBBBBBBBBBBBBBBBBBB",
        "XXXXXXXXXXXX  EEEEEEEEEEEEEEEEEE  HHHHHHHHHHHHHHHHHHHHHHHH  GGGGGGGGGGGGGGGGGGGGGGGG  KKKKKKKKKKKKKKKKKKKKKKKK  BBBBBBBBBBBBBBBBBBBBBBBB",
        "              EEEEEEEEEEEEEEEEEE  HHHHHHHHHHHHHHHHHHHHHHHH  GGGGGGGGGGGGGGGGGGGGGGGG  KKKKKKKKKKKKKKKKKKKKKKKK  BBBBBBBBBBBBBBBBBBBBBBBB",
        "                     #==#         HHHHHHHHHHHHHHHHHHHHHHHH  GGGGGGGGGGGGGGGGGGGGGGGG  KKKKKKKKKKKKKKKKKKKKKKKK  BBBBBBBBBBBBBBBBBBBBBBBB",
        "                     #==#                   #==#                      #==#                                                              ",
        "                 RRRRRRRRRRRR               #==#                      #==#                                                              ",
        "                 RRRRRRRRRRRR           SSSSSSSSSSSS              ZZZZZZZZZZZZ                                                          ",
        "                 RRRRRRRRRRRR           SSSSSSSSSSSS              ZZZZZZZZZZZZ                                                          ",
        "                 RRRRRRRRRRRR           SSSSSSSSSSSS              ZZZZZZZZZZZZ                                                          ",
        "                 RRRRRRRRRRRR           SSSSSSSSSSSS              ZZZZZZZZZZZZ                                                          ",
        "                 RRRRRRRRRRRR           SSSSSSSSSSSS              ZZZZZZZZZZZZ                                                          ",
        "                 RRRRRRRRRRRR           SSSSSSSSSSSS              ZZZZZZZZZZZZ                                                          ",
        "                 RRRRRRRRRRRR           SSSSSSSSSSSS              ZZZZZZZZZZZZ                                                          ",
        "                 RRRRRRRRRRRR           SSSSSSSSSSSS              ZZZZZZZZZZZZ                                                          ",
        "                 RRRRRRRRRRRR           SSSSSSSSSSSS              ZZZZZZZZZZZZ                                                          ",
        "                                        SSSSSSSSSSSS              ZZZZZZZZZZZZ                                                          ",
    ],
    slots: &[
        airlock(slot(b'A', Size::S, Zone::Hull, &[Dir::South]), Dir::North),
        airlock(slot(b'Z', Size::S, Zone::Hull, &[Dir::North]), Dir::South),
        airlock(slot(b'X', Size::S, Zone::Hull, &[Dir::East]), Dir::West),
        slot(
            b'E',
            Size::M,
            Zone::Aft,
            &[Dir::North, Dir::East, Dir::South, Dir::West],
        ),
        slot(
            b'H',
            Size::L,
            Zone::Mid,
            &[Dir::North, Dir::East, Dir::South, Dir::West],
        ),
        slot(
            b'G',
            Size::L,
            Zone::Mid,
            &[Dir::North, Dir::East, Dir::South, Dir::West],
        ),
        slot(b'K', Size::L, Zone::Mid, &[Dir::North, Dir::East, Dir::West]),
        slot(b'B', Size::L, Zone::Fore, &[Dir::West]),
        optional(slot(b'P', Size::S, Zone::Mid, &[Dir::South])),
        optional(slot(b'S', Size::S, Zone::Mid, &[Dir::North])),
        optional(slot(b'T', Size::S, Zone::Mid, &[Dir::South])),
        slot(b'C', Size::S, Zone::Crawlspace, &[Dir::South]),
        slot(b'R', Size::S, Zone::Stores, &[Dir::North]),
    ],
}
.valid();

/// The Gunship (issue #27): short and dense.
///
/// From engineering `E` at the stern, one keel
/// runs fore past four small compartments packed along it: the port airlock `A`, the
/// starboard airlock `Z`, and two crew or cargo rooms, `S` and the optional `P`. Then
/// comes the war room `W`, which bulkheads the way to the bridge `B`. As on the Corvette,
/// the aft airlock `X` is past engineering, with the crawlspace `C` (behind an access
/// panel) above it and the stores `R` below.
pub const GUNSHIP: Template = Template {
    name: "gunship",
    rows: &[
        "                 CCCCCCCCCCCC                                                                            ",
        "                 CCCCCCCCCCCC                                                                            ",
        "                 CCCCCCCCCCCC                                                                            ",
        "                 CCCCCCCCCCCC                                                                            ",
        "                 CCCCCCCCCCCC                                                                            ",
        "                 CCCCCCCCCCCC                                                                            ",
        "                 CCCCCCCCCCCC                                                                            ",
        "                 CCCCCCCCCCCC     AAAAAAAAAAAA PPPPPPPPPPPP                                              ",
        "                 CCCCCCCCCCCC     AAAAAAAAAAAA PPPPPPPPPPPP                                              ",
        "                 CCCCCCCCCCCC     AAAAAAAAAAAA PPPPPPPPPPPP                                              ",
        "                     #==#         AAAAAAAAAAAA PPPPPPPPPPPP                                              ",
        "                     #==#         AAAAAAAAAAAA PPPPPPPPPPPP                      BBBBBBBBBBBBBBBBBBBBBBBB",
        "              EEEEEEEEEEEEEEEEEE  AAAAAAAAAAAA PPPPPPPPPPPP  WWWWWWWWWWWWWWWWWW  BBBBBBBBBBBBBBBBBBBBBBBB",
        "XXXXXXXXXXXX  EEEEEEEEEEEEEEEEEE  AAAAAAAAAAAA PPPPPPPPPPPP  WWWWWWWWWWWWWWWWWW  BBBBBBBBBBBBBBBBBBBBBBBB",
        "XXXXXXXXXXXX  EEEEEEEEEEEEEEEEEE  AAAAAAAAAAAA PPPPPPPPPPPP  WWWWWWWWWWWWWWWWWW  BBBBBBBBBBBBBBBBBBBBBBBB",
        "XXXXXXXXXXXX  EEEEEEEEEEEEEEEEEE  AAAAAAAAAAAA PPPPPPPPPPPP  WWWWWWWWWWWWWWWWWW  BBBBBBBBBBBBBBBBBBBBBBBB",
        "XXXXXXXXXXXX##EEEEEEEEEEEEEEEEEE##AAAAAAAAAAAA#PPPPPPPPPPPP##WWWWWWWWWWWWWWWWWW##BBBBBBBBBBBBBBBBBBBBBBBB",
        "XXXXXXXXXXXX==EEEEEEEEEEEEEEEEEE=============================WWWWWWWWWWWWWWWWWW==BBBBBBBBBBBBBBBBBBBBBBBB",
        "XXXXXXXXXXXX==EEEEEEEEEEEEEEEEEE=============================WWWWWWWWWWWWWWWWWW==BBBBBBBBBBBBBBBBBBBBBBBB",
        "XXXXXXXXXXXX##EEEEEEEEEEEEEEEEEE##SSSSSSSSSSSS#ZZZZZZZZZZZZ##WWWWWWWWWWWWWWWWWW##BBBBBBBBBBBBBBBBBBBBBBBB",
        "XXXXXXXXXXXX  EEEEEEEEEEEEEEEEEE  SSSSSSSSSSSS ZZZZZZZZZZZZ  WWWWWWWWWWWWWWWWWW  BBBBBBBBBBBBBBBBBBBBBBBB",
        "XXXXXXXXXXXX  EEEEEEEEEEEEEEEEEE  SSSSSSSSSSSS ZZZZZZZZZZZZ  WWWWWWWWWWWWWWWWWW  BBBBBBBBBBBBBBBBBBBBBBBB",
        "XXXXXXXXXXXX  EEEEEEEEEEEEEEEEEE  SSSSSSSSSSSS ZZZZZZZZZZZZ  WWWWWWWWWWWWWWWWWW  BBBBBBBBBBBBBBBBBBBBBBBB",
        "              EEEEEEEEEEEEEEEEEE  SSSSSSSSSSSS ZZZZZZZZZZZZ  WWWWWWWWWWWWWWWWWW  BBBBBBBBBBBBBBBBBBBBBBBB",
        "                     #==#         SSSSSSSSSSSS ZZZZZZZZZZZZ                      BBBBBBBBBBBBBBBBBBBBBBBB",
        "                     #==#         SSSSSSSSSSSS ZZZZZZZZZZZZ                                              ",
        "                 RRRRRRRRRRRR     SSSSSSSSSSSS ZZZZZZZZZZZZ                                              ",
        "                 RRRRRRRRRRRR     SSSSSSSSSSSS ZZZZZZZZZZZZ                                              ",
        "                 RRRRRRRRRRRR     SSSSSSSSSSSS ZZZZZZZZZZZZ                                              ",
        "                 RRRRRRRRRRRR                                                                            ",
        "                 RRRRRRRRRRRR                                                                            ",
        "                 RRRRRRRRRRRR                                                                            ",
        "                 RRRRRRRRRRRR                                                                            ",
        "                 RRRRRRRRRRRR                                                                            ",
        "                 RRRRRRRRRRRR                                                                            ",
        "                 RRRRRRRRRRRR                                                                            ",
    ],
    slots: &[
        airlock(slot(b'A', Size::S, Zone::Hull, &[Dir::South]), Dir::North),
        airlock(slot(b'Z', Size::S, Zone::Hull, &[Dir::North]), Dir::South),
        airlock(slot(b'X', Size::S, Zone::Hull, &[Dir::East]), Dir::West),
        slot(
            b'E',
            Size::M,
            Zone::Aft,
            &[Dir::North, Dir::East, Dir::South, Dir::West],
        ),
        slot(b'S', Size::S, Zone::Mid, &[Dir::North]),
        optional(slot(b'P', Size::S, Zone::Mid, &[Dir::South])),
        slot(b'W', Size::M, Zone::Fore, &[Dir::East, Dir::West]),
        slot(b'B', Size::L, Zone::Fore, &[Dir::West]),
        slot(b'C', Size::S, Zone::Crawlspace, &[Dir::South]),
        slot(b'R', Size::S, Zone::Stores, &[Dir::North]),
    ],
}
.valid();

/// Every ship class. A run's comes from its seed: see [`Template::for_seed`].
pub const CLASSES: &[Template] = &[CORVETTE, FREIGHTER, GUNSHIP];

/// Salts the run seed for the class draw, so it isn't the run RNG's own stream.
const CLASS_STREAM: u64 = 0x5419_F100_2A3D_0002;

impl Template {
    /// The ship class a run on `seed` boards: one of [`CLASSES`], drawn from the seed
    /// alone so a peer picks the same one.
    #[must_use]
    pub fn for_seed(seed: u64) -> &'static Self {
        let count = u32::try_from(CLASSES.len()).unwrap_or(1);
        let pick = Rng::from_seed(seed ^ CLASS_STREAM).below(count);
        usize::try_from(pick)
            .ok()
            .and_then(|i| CLASSES.get(i))
            .unwrap_or(&CORVETTE)
    }
}
