//! The floor as the sim plays it (issue #26): one grid of cells, each in a room or a
//! hatch.
//!
//! A [`Ship`] is built once from a [`Derelict`], then shared read-only behind an `Arc`
//! and checksummed once: [`SimState`](crate::SimState) holds the pointer, and snapshots
//! copy only the live hatch states beside it.
//!
//! Hatches are hard boundaries. Only players cross them, and bullets and sight only
//! through an open one; enemies never leave their room (see [`Tiles`]).

use crate::checksum;
use crate::derelict::DERELICT;
use crate::room::{Cell, Derelict, Exit, Placed, cell_center, cell_of, cell_start};
use crate::{Fx, FxVec2, RoomId};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::hash::{Hash, Hasher};

/// Index into the ship's hatches; also the index of the live [`HatchState`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HatchId(pub u16);

/// A hatch's live state. Later steps add a shoot-open access panel and an airlock that
/// the bridge unlocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HatchState {
    /// Shut: stops bullets and sight. A player stepping into it opens it, revealing the
    /// room behind.
    Closed,
    /// Open for good: players, bullets and sight pass.
    Open,
    /// Combat lockdown: stops everything until the room is cleared.
    Sealed,
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

/// Where a hatch is: the rooms on either side and its gap, in floor cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Hatch {
    pub rooms: [RoomId; 2],
    pub gap: Exit,
}

/// The static floor: never changes during a run.
#[derive(Clone, Debug)]
pub struct Ship {
    width: usize,
    height: usize,
    /// Row-major.
    spots: Vec<Spot>,
    /// By [`RoomId`].
    rooms: Vec<Placed>,
    /// By [`HatchId`].
    hatches: Vec<Hatch>,
    start: (RoomId, FxVec2),
    checksum: u64,
}

impl Ship {
    /// Stamps every placed room into one grid and a hatch over each connection. The
    /// derelict is const-validated, so rooms only share wall and linked exits.
    #[must_use]
    pub fn new(derelict: &Derelict) -> Self {
        let size = |f: fn(&Placed) -> usize| derelict.rooms.iter().map(f).max().unwrap_or(0);
        let width = size(|p| p.at.0.saturating_add(p.room.width()));
        let height = size(|p| p.at.1.saturating_add(p.room.height()));
        let mut spots = vec![Spot::Void; width.saturating_mul(height)];
        for (room, placed) in (0..).map(RoomId).zip(derelict.rooms) {
            for (y, row) in placed.room.cells.iter().enumerate() {
                for x in 0..row.len() {
                    let (Ok(lx), Ok(ly)) = (i32::try_from(x), i32::try_from(y)) else {
                        continue;
                    };
                    let cell = placed.room.cell(lx, ly);
                    let at = (x.saturating_add(placed.at.0), y.saturating_add(placed.at.1));
                    if let Some(spot) = index(width, at).and_then(|i| spots.get_mut(i))
                        && *spot == Spot::Void
                        && cell != Cell::Void
                    {
                        *spot = Spot::Room { room, cell };
                    }
                }
            }
        }
        let mut hatches = Vec::new();
        for (id, link) in (0..).map(HatchId).zip(derelict.connections) {
            let Some(gap) = derelict.gap(link.from) else {
                continue;
            };
            for i in 0..gap.width {
                if let Some(spot) = index(width, gap.cell(i)).and_then(|i| spots.get_mut(i)) {
                    *spot = Spot::Hatch(id);
                }
            }
            let room = |r: usize| RoomId(u16::try_from(r).unwrap_or(u16::MAX));
            hatches.push(Hatch {
                rooms: [room(link.from.room), room(link.to.room)],
                gap,
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
        let checksum = checksum::of(&(width, height, &spots, &rooms, &hatches, start));
        Self {
            width,
            height,
            spots,
            rooms,
            hatches,
            start,
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

    /// The hatches a box of half-size `half` at `p` overlaps.
    pub fn hatches_under(&self, p: FxVec2, half: Fx) -> impl Iterator<Item = HatchId> {
        cells_under(p, half).filter_map(|(x, y)| match self.spot(x, y) {
            Spot::Hatch(id) => Some(id),
            Spot::Void | Spot::Room { .. } => None,
        })
    }

    /// Whether a box of half-size `half` at `p` overlaps `room`'s extraction pad.
    #[must_use]
    pub fn on_extraction(&self, room: RoomId, p: FxVec2, half: Fx) -> bool {
        let Some(pad) = self.room(room).and_then(|r| {
            let (x, y) = r.room.extraction?;
            let cell = |c: usize, at: usize| i32::try_from(c.saturating_add(at)).ok();
            Some((cell(x, r.at.0)?, cell(y, r.at.1)?))
        }) else {
            return false;
        };
        cells_under(p, half).any(|c| c == pad)
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

/// Serialized as its checksum: a peer rebuilds the ship and verifies it.
impl Serialize for Ship {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.checksum.serialize(serializer)
    }
}

/// Rebuilds the only ship there is so far, [`DERELICT`], and checks it is the one sent.
impl<'de> Deserialize<'de> for Ship {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let checksum = u64::deserialize(deserializer)?;
        let ship = Self::new(&DERELICT);
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
                    (HatchState::Closed | HatchState::Open | HatchState::Sealed, Body::Walker)
                    | (HatchState::Sealed, Body::Player | Body::Shot)
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
