//! The floor as the sim plays it (issue #26): one grid of cells, each in a room or a
//! hatch.
//!
//! A [`Ship`] is generated once per run, from a hull template and the run seed (see
//! [`Ship::generate`]), then shared read-only behind an `Arc` and checksummed once:
//! [`SimState`](crate::SimState) holds the pointer, and snapshots copy only the live
//! hatch states beside it. Tests also hand-place ships from a [`Derelict`].
//!
//! Hatches are hard boundaries. Only players cross them, and bullets and sight only
//! through an open one; enemies never leave their room (see [`Tiles`]).

use crate::checksum;
use crate::hull::{AROUND, CORVETTE, Slot, Template, Zone, toward};
use crate::pool::POOL;
use crate::room::{
    Category, Cell, Derelict, Dir, Exit, Placed, PrototypeRoom, Theme, cell_center, cell_of,
    cell_start,
};
use crate::{Fx, FxVec2, Rng, RoomId};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::hash::{Hash, Hasher};

/// Index into the ship's hatches; also the index of the live [`HatchState`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HatchId(pub u16);

/// A hatch's live state; each starts as its [`HatchKind::initial`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HatchState {
    /// Shut: stops bullets and sight. A player stepping into it opens it, revealing the
    /// room behind; stepping into an airlock's outer hatch wins instead.
    Closed,
    /// Open for good: players, bullets and sight pass.
    Open,
    /// Combat lockdown: stops everything until the room is cleared.
    Sealed,
    /// An airlock's outer hatch, before the bridge is clear: stops everything. Clearing
    /// the bridge turns every one Closed.
    AirlockLocked,
    /// A crawlspace's access panel, drawn as wall: stops everything. Shooting it, or
    /// anything else that calls [`crate::reveal`] (future EMPs), turns it Closed.
    Panel,
}

/// What occupies a floor cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Spot {
    Void,
    /// A cell of `room`. Wall two rooms share belongs to the first placed.
    Room {
        room: RoomId,
        cell: Cell,
    },
    Hatch(HatchId),
}

/// What a hatch is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HatchKind {
    /// Between a room and a corridor.
    Plain,
    /// An airlock's outer hatch, onto space: stepping into it once the bridge unlocks it
    /// leaves the ship, winning the run.
    Airlock,
    /// Into a crawlspace: hidden until revealed, then plain.
    Panel,
}

impl HatchKind {
    /// The state a hatch of this kind starts a run in.
    #[must_use]
    pub const fn initial(self) -> HatchState {
        match self {
            Self::Plain => HatchState::Closed,
            Self::Airlock => HatchState::AirlockLocked,
            Self::Panel => HatchState::Panel,
        }
    }
}

/// Where a hatch is: the rooms on either side and its gap, in floor cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Hatch {
    /// An airlock's outer hatch has the airlock on both sides: its far side is space.
    pub rooms: [RoomId; 2],
    pub gap: Exit,
    pub kind: HatchKind,
}

/// The static floor: never changes during a run.
#[derive(Clone, Debug)]
pub struct Ship {
    width: usize,
    height: usize,
    /// Row-major.
    spots: Vec<Spot>,
    /// By [`RoomId`]: the template's filled slots in legend order, then its corridors.
    rooms: Vec<Placed>,
    /// By [`HatchId`].
    hatches: Vec<Hatch>,
    start: (RoomId, FxVec2),
    /// The run seed the ship was generated from (0 when hand-placed); serde rebuilds it
    /// from this.
    seed: u64,
    checksum: u64,
}

/// A corridor's region. Its cells are the template's, so they live only in the ship's
/// grid: this room has none, nor anything else.
const CORRIDOR: PrototypeRoom = PrototypeRoom {
    name: "corridor",
    category: Category::Connector,
    theme: Theme::Corridor,
    cells: &[],
    exits: &[],
    base: &[],
    reinforcements: &[],
    events: &[],
};

/// Salts the run seed for the floor's draws, so they aren't the run RNG's own stream.
const FLOOR_STREAM: u64 = 0x5419_F100_2A3D_0001;

impl Ship {
    /// Fills `template` from the room pool, drawing only on `seed`, so a peer rebuilds the
    /// same ship from the same seed (issue #26):
    ///
    /// 1. Each slot, in legend order: an optional one is left out on a coin flip. Else it
    ///    takes a random pool room that fits it, preferring rooms not yet placed, stamped
    ///    at the slot with its unused hatch points walled.
    /// 2. Each stretch of corridor becomes a region, walled by the hull and by any outside
    ///    cell touching it (a left-out slot, a room's void).
    /// 3. A hatch joins each slot's open hatch points to the corridor outside (a panel, for
    ///    a crawlspace), and each airlock gets its outer hatch, onto space.
    /// 4. The party boards through a random airlock.
    ///
    /// It can't fail: templates are validated against the pool when the crate builds, and
    /// a seed sweep in the tests checks every ship joins up.
    #[must_use]
    pub fn generate(template: &Template, seed: u64) -> Self {
        let (width, height) = template.size();
        let mut spots = vec![Spot::Void; width.saturating_mul(height)];
        let mut rng = Rng::from_seed(seed ^ FLOOR_STREAM);
        let filled = fill(template, &mut rng);
        for (id, &(slot, placed)) in (0..).map(RoomId).zip(&filled) {
            stamp(&mut spots, width, id, &placed);
            let walled = |dir: &Dir| !slot.hatches.contains(dir) && slot.airlock != Some(*dir);
            for dir in DIRS.into_iter().filter(walled) {
                let gap = slot.size.hatch(dir).shifted(placed.at);
                for i in 0..gap.width {
                    let wall = Spot::Room {
                        room: id,
                        cell: Cell::Wall,
                    };
                    set(&mut spots, width, gap.cell(i), wall);
                }
            }
        }
        let mut rooms: Vec<Placed> = filled.iter().map(|&(_, placed)| placed).collect();
        add_corridors(template, &mut spots, &mut rooms);

        let mut hatches = Vec::new();
        for (id, &(slot, placed)) in (0..).map(RoomId).zip(&filled) {
            for &dir in slot.hatches {
                let gap = slot.size.hatch(dir).shifted(placed.at);
                let outside = toward(gap.cell(0), dir).map(|at| get(&spots, width, at));
                let Some(Spot::Room { room: corridor, .. }) = outside else {
                    continue;
                };
                let hatch = HatchId(u16::try_from(hatches.len()).unwrap_or(u16::MAX));
                for i in 0..gap.width {
                    set(&mut spots, width, gap.cell(i), Spot::Hatch(hatch));
                }
                let kind = if slot.zone == Zone::Crawlspace {
                    HatchKind::Panel
                } else {
                    HatchKind::Plain
                };
                hatches.push(Hatch {
                    rooms: [id, corridor],
                    gap,
                    kind,
                });
            }
            if let Some(dir) = slot.airlock {
                let gap = slot.size.hatch(dir).shifted(placed.at);
                let hatch = HatchId(u16::try_from(hatches.len()).unwrap_or(u16::MAX));
                for i in 0..gap.width {
                    set(&mut spots, width, gap.cell(i), Spot::Hatch(hatch));
                }
                hatches.push(Hatch {
                    rooms: [id, id],
                    gap,
                    kind: HatchKind::Airlock,
                });
            }
        }

        // Airlock slots are never left out. The party starts at a random one's center,
        // where the room's hatch points line up.
        let airlocks: Vec<_> = (0..)
            .map(RoomId)
            .zip(&filled)
            .filter(|(_, (slot, _))| slot.airlock.is_some())
            .collect();
        let pick = rng.below(u32::try_from(airlocks.len()).unwrap_or(u32::MAX));
        let start = usize::try_from(pick)
            .ok()
            .and_then(|i| airlocks.get(i))
            .map(|&(id, &(slot, placed))| {
                let (w, h) = slot.size.dims();
                let mid = |at: usize, side: usize| {
                    cell_start(i32::try_from(at.saturating_add(side / 2)).unwrap_or(0))
                };
                let (x, y) = placed.at;
                (
                    id,
                    FxVec2 {
                        x: mid(x, w),
                        y: mid(y, h),
                    },
                )
            })
            .unwrap_or_default();
        Self::from_parts(width, height, spots, rooms, hatches, start, seed)
    }

    /// Stamps every placed room into one grid and a hatch over each connection. The
    /// derelict is const-validated, so rooms only share wall and linked exits.
    #[must_use]
    pub fn new(derelict: &Derelict) -> Self {
        let size = |f: fn(&Placed) -> usize| derelict.rooms.iter().map(f).max().unwrap_or(0);
        let width = size(|p| p.at.0.saturating_add(p.room.width()));
        let height = size(|p| p.at.1.saturating_add(p.room.height()));
        let mut spots = vec![Spot::Void; width.saturating_mul(height)];
        for (room, placed) in (0..).map(RoomId).zip(derelict.rooms) {
            stamp(&mut spots, width, room, placed);
        }
        let mut hatches = Vec::new();
        for (id, link) in (0..).map(HatchId).zip(derelict.connections) {
            let Some(gap) = derelict.gap(link.from) else {
                continue;
            };
            for i in 0..gap.width {
                set(&mut spots, width, gap.cell(i), Spot::Hatch(id));
            }
            let room = |r: usize| RoomId(u16::try_from(r).unwrap_or(u16::MAX));
            hatches.push(Hatch {
                rooms: [room(link.from.room), room(link.to.room)],
                gap,
                kind: HatchKind::Plain,
            });
        }
        let start_room = RoomId(u16::try_from(derelict.start_room).unwrap_or(0));
        let (x, y) = derelict.start_cell;
        let at = derelict
            .rooms
            .get(derelict.start_room)
            .map_or((0, 0), |p| p.at);
        let start = (
            start_room,
            cell_center(x.saturating_add(at.0), y.saturating_add(at.1)),
        );
        let rooms = derelict.rooms.to_vec();
        Self::from_parts(width, height, spots, rooms, hatches, start, 0)
    }

    /// The ship, checksummed.
    fn from_parts(
        width: usize,
        height: usize,
        spots: Vec<Spot>,
        rooms: Vec<Placed>,
        hatches: Vec<Hatch>,
        start: (RoomId, FxVec2),
        seed: u64,
    ) -> Self {
        let checksum = checksum::of(&(width, height, &spots, &rooms, &hatches, start));
        Self {
            width,
            height,
            spots,
            rooms,
            hatches,
            start,
            seed,
            checksum,
        }
    }

    /// Size in cells.
    #[must_use]
    pub const fn size(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    /// Hash of all the static data, taken once at build.
    #[must_use]
    pub const fn checksum(&self) -> u64 {
        self.checksum
    }

    #[must_use]
    pub fn rooms(&self) -> &[Placed] {
        &self.rooms
    }

    #[must_use]
    pub fn room(&self, id: RoomId) -> Option<&Placed> {
        self.rooms.get(usize::from(id.0))
    }

    /// The region graph: each hatch joins two rooms.
    #[must_use]
    pub fn hatches(&self) -> &[Hatch] {
        &self.hatches
    }

    /// The hatches in `room`'s walls.
    pub fn hatches_of(&self, room: RoomId) -> impl Iterator<Item = HatchId> {
        (0..)
            .map(HatchId)
            .zip(&self.hatches)
            .filter(move |(_, h)| h.rooms.contains(&room))
            .map(|(id, _)| id)
    }

    /// The start room and the party's starting point in it.
    #[must_use]
    pub const fn start(&self) -> (RoomId, FxVec2) {
        self.start
    }

    /// What fills cell (`x`, `y`); off the grid is void.
    #[must_use]
    pub fn spot(&self, x: i32, y: i32) -> Spot {
        let (Ok(x), Ok(y)) = (usize::try_from(x), usize::try_from(y)) else {
            return Spot::Void;
        };
        index(self.width, (x, y))
            .filter(|_| y < self.height)
            .and_then(|i| self.spots.get(i).copied())
            .unwrap_or(Spot::Void)
    }

    /// The cell at (`x`, `y`): a hatch's gap is floor.
    #[must_use]
    pub fn cell(&self, x: i32, y: i32) -> Cell {
        match self.spot(x, y) {
            Spot::Void => Cell::Void,
            Spot::Room { cell, .. } => cell,
            Spot::Hatch(_) => Cell::Floor,
        }
    }

    /// The room whose cell `p` is in; `None` in a hatch or outside the ship.
    #[must_use]
    pub fn room_at(&self, p: FxVec2) -> Option<RoomId> {
        match self.spot(cell_of(p.x), cell_of(p.y)) {
            Spot::Room { room, .. } => Some(room),
            Spot::Void | Spot::Hatch(_) => None,
        }
    }

    /// The room a box of half-size `half` at `p` is wholly inside: its center's room,
    /// with no part of it in a hatch.
    #[must_use]
    pub fn inside(&self, p: FxVec2, half: Fx) -> Option<RoomId> {
        let in_hatch = cells_under(p, half).any(|(x, y)| matches!(self.spot(x, y), Spot::Hatch(_)));
        if in_hatch { None } else { self.room_at(p) }
    }

    /// The floor cell of `room`'s chest, if it has one (see [`PrototypeRoom::chest`]).
    #[must_use]
    pub fn chest(&self, room: RoomId) -> Option<(usize, usize)> {
        let placed = self.room(room)?;
        let (x, y) = placed.room.chest()?;
        Some((x.saturating_add(placed.at.0), y.saturating_add(placed.at.1)))
    }

    /// The room whose chest a box of half-size `half` at `p` (its center in that room)
    /// overlaps, if any.
    #[must_use]
    pub fn on_chest(&self, p: FxVec2, half: Fx) -> Option<RoomId> {
        let room = self.room_at(p)?;
        let (x, y) = self.chest(room)?;
        let chest = (i32::try_from(x).ok()?, i32::try_from(y).ok()?);
        cells_under(p, half).any(|c| c == chest).then_some(room)
    }

    /// The hatches a box of half-size `half` at `p` overlaps.
    pub fn hatches_under(&self, p: FxVec2, half: Fx) -> impl Iterator<Item = HatchId> {
        cells_under(p, half).filter_map(|(x, y)| match self.spot(x, y) {
            Spot::Hatch(id) => Some(id),
            Spot::Void | Spot::Room { .. } => None,
        })
    }
}

/// The ship is static, so it hashes (for state checksums) as its build-time checksum.
impl Hash for Ship {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.checksum.hash(state);
    }
}

impl PartialEq for Ship {
    fn eq(&self, other: &Self) -> bool {
        self.checksum == other.checksum
    }
}

impl Eq for Ship {}

/// Serialized as its seed and checksum: a peer regenerates the ship and verifies it.
impl Serialize for Ship {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        (self.seed, self.checksum).serialize(serializer)
    }
}

/// Regenerates the only ship class there is so far, [`CORVETTE`], from the seed sent,
/// and checks it is the ship sent. Hand-placed ships (tests) don't round-trip.
impl<'de> Deserialize<'de> for Ship {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let (seed, checksum) = <(u64, u64)>::deserialize(deserializer)?;
        let ship = Self::generate(&CORVETTE, seed);
        if ship.checksum == checksum {
            Ok(ship)
        } else {
            Err(serde::de::Error::custom("ship checksum mismatch"))
        }
    }
}

/// Row-major index of on-grid cell (`x`, `y`) in a `width`-wide grid.
fn index(width: usize, (x, y): (usize, usize)) -> Option<usize> {
    (x < width).then(|| y.checked_mul(width)?.checked_add(x))?
}

/// What fills cell `at` of a `width`-wide grid; off the grid is void.
fn get(spots: &[Spot], width: usize, at: (usize, usize)) -> Spot {
    index(width, at)
        .and_then(|i| spots.get(i).copied())
        .unwrap_or(Spot::Void)
}

/// Fills cell `at` of a `width`-wide grid, if it's on it.
fn set(spots: &mut [Spot], width: usize, at: (usize, usize), spot: Spot) {
    if let Some(cell) = index(width, at).and_then(|i| spots.get_mut(i)) {
        *cell = spot;
    }
}

/// The four sides, in hatch-point order.
const DIRS: [Dir; 4] = [Dir::North, Dir::East, Dir::South, Dir::West];

/// Step 1 of [`Ship::generate`]: each filled slot and its room, placed at the slot.
fn fill(template: &Template, rng: &mut Rng) -> Vec<(Slot, Placed)> {
    let mut used = vec![false; POOL.len()];
    let mut filled = Vec::new();
    for &slot in template.slots {
        if slot.optional && rng.below(2) == 0 {
            continue;
        }
        let fits: Vec<usize> = (0..POOL.len())
            .filter(|&i| POOL.get(i).is_some_and(|room| slot.fits(room)))
            .collect();
        let fresh: Vec<usize> = fits
            .iter()
            .copied()
            .filter(|&i| used.get(i) == Some(&false))
            .collect();
        let choices = if fresh.is_empty() { fits } else { fresh };
        let pick = rng.below(u32::try_from(choices.len()).unwrap_or(u32::MAX));
        let Some(pick) = usize::try_from(pick)
            .ok()
            .and_then(|i| choices.get(i).copied())
        else {
            continue;
        };
        let at = template.origin(slot.letter);
        if let (Some(&room), Some(at), Some(taken)) = (POOL.get(pick), at, used.get_mut(pick)) {
            *taken = true;
            filled.push((slot, Placed { room, at }));
        }
    }
    filled
}

/// Step 2 of [`Ship::generate`]: each 4-connected stretch of the template's corridor
/// becomes a region of floor, then every outside cell touching one (diagonals count)
/// becomes its wall: the hull, a left-out slot, a room's void.
fn add_corridors(template: &Template, spots: &mut [Spot], rooms: &mut Vec<Placed>) {
    let (width, _) = template.size();
    let corridor = |(x, y): (usize, usize)| template.at(x, y) == b'=';
    let floor: Vec<(usize, usize)> = (template.rows.iter().enumerate())
        .flat_map(|(y, row)| row.bytes().enumerate().map(move |(x, c)| (x, y, c)))
        .filter_map(|(x, y, c)| (c == b'=').then_some((x, y)))
        .collect();
    for &at in &floor {
        if get(spots, width, at) != Spot::Void {
            continue;
        }
        let id = RoomId(u16::try_from(rooms.len()).unwrap_or(u16::MAX));
        rooms.push(Placed { room: CORRIDOR, at });
        let mut stack = vec![at];
        while let Some(at) = stack.pop() {
            if corridor(at) && get(spots, width, at) == Spot::Void {
                let floor = Spot::Room {
                    room: id,
                    cell: Cell::Floor,
                };
                set(spots, width, at, floor);
                stack.extend(DIRS.into_iter().filter_map(|dir| toward(at, dir)));
            }
        }
    }
    for &(x, y) in &floor {
        let Spot::Room { room, .. } = get(spots, width, (x, y)) else {
            continue;
        };
        for &(dx, dy) in &AROUND {
            let (Some(nx), Some(ny)) = (x.checked_add_signed(dx), y.checked_add_signed(dy)) else {
                continue;
            };
            if get(spots, width, (nx, ny)) == Spot::Void {
                let wall = Spot::Room {
                    room,
                    cell: Cell::Wall,
                };
                set(spots, width, (nx, ny), wall);
            }
        }
    }
}

/// Stamps `placed`'s non-void cells into `spots` as `room`'s, except cells another room
/// already has: wall two rooms share belongs to the first placed.
fn stamp(spots: &mut [Spot], width: usize, room: RoomId, placed: &Placed) {
    for (y, row) in placed.room.cells.iter().enumerate() {
        for x in 0..row.len() {
            let (Ok(lx), Ok(ly)) = (i32::try_from(x), i32::try_from(y)) else {
                continue;
            };
            let cell = placed.room.cell(lx, ly);
            let at = (x.saturating_add(placed.at.0), y.saturating_add(placed.at.1));
            if get(spots, width, at) == Spot::Void && cell != Cell::Void {
                set(spots, width, at, Spot::Room { room, cell });
            }
        }
    }
}

/// The cells a box of half-size `half` at `p` overlaps.
fn cells_under(p: FxVec2, half: Fx) -> impl Iterator<Item = (i32, i32)> {
    let span = |v: Fx| {
        cell_of(v.saturating_sub(half))..=cell_of(v.saturating_add(half).saturating_sub(Fx::DELTA))
    };
    let xs = span(p.x);
    span(p.y).flat_map(move |y| xs.clone().map(move |x| (x, y)))
}

/// What is moving through the tiles. Walls, void and sealed hatches stop everything;
/// pits and other hatches differ by body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Body {
    /// Enemies: pits and every hatch stop them.
    Walker,
    /// Players walk onto pits, and fall in (see `Player`), and through closed hatches,
    /// which opens them.
    Player,
    /// Flies over pits and through open hatches.
    Shot,
}

/// The floor's collision view this tick: the ship, its hatches' states, and for walkers
/// the room they are kept in.
#[derive(Clone, Copy, Debug)]
pub struct Tiles<'a> {
    pub ship: &'a Ship,
    /// By [`HatchId`]; a missing state reads as sealed.
    pub hatches: &'a [HatchState],
    /// Walkers can't leave this room: other rooms' cells block them. `None` = anywhere.
    pub room: Option<RoomId>,
}

impl<'a> Tiles<'a> {
    #[must_use]
    pub const fn new(ship: &'a Ship, hatches: &'a [HatchState]) -> Self {
        Self {
            ship,
            hatches,
            room: None,
        }
    }

    /// This view for a walker at `p`: kept in the room it stands in.
    #[must_use]
    pub fn walker_at(self, p: FxVec2) -> Self {
        Self {
            room: self.ship.room_at(p),
            ..self
        }
    }

    #[must_use]
    pub fn blocks(self, x: i32, y: i32, body: Body) -> bool {
        match self.ship.spot(x, y) {
            Spot::Void => true,
            Spot::Hatch(id) => {
                let state = self.hatches.get(usize::from(id.0)).copied();
                match (state.unwrap_or(HatchState::Sealed), body) {
                    (HatchState::Closed | HatchState::Open, Body::Player)
                    | (HatchState::Open, Body::Shot) => false,
                    (
                        HatchState::Closed
                        | HatchState::Open
                        | HatchState::Sealed
                        | HatchState::AirlockLocked
                        | HatchState::Panel,
                        Body::Walker,
                    )
                    | (
                        HatchState::Sealed | HatchState::AirlockLocked | HatchState::Panel,
                        Body::Player | Body::Shot,
                    )
                    | (HatchState::Closed, Body::Shot) => true,
                }
            }
            Spot::Room { room, cell } => match (cell, body) {
                (Cell::Wall | Cell::Void, _) | (Cell::Pit, Body::Walker) => true,
                (Cell::Floor, Body::Walker) => self.room.is_some_and(|r| r != room),
                (Cell::Floor | Cell::Pit, Body::Player | Body::Shot) => false,
            },
        }
    }

    /// The cells a flow field for this view covers, as (left, top, width, height): the
    /// walker's room, else the whole ship.
    #[must_use]
    pub fn bounds(self) -> (usize, usize, usize, usize) {
        self.room
            .and_then(|r| self.ship.room(r))
            .map_or((0, 0, self.ship.width, self.ship.height), |p| {
                (p.at.0, p.at.1, p.room.width(), p.room.height())
            })
    }

    /// Whether `p` is over a pit.
    #[must_use]
    pub fn pit_at(self, p: FxVec2) -> bool {
        self.ship.cell(cell_of(p.x), cell_of(p.y)) == Cell::Pit
    }

    /// Whether any pit cell overlaps the square of half-size `reach` around `p`: some pit
    /// lies closer than `reach` to `p` along both axes.
    #[must_use]
    pub fn pit_within(self, p: FxVec2, reach: Fx) -> bool {
        cells_under(p, reach).any(|(x, y)| self.ship.cell(x, y) == Cell::Pit)
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
