//! Enemy pathfinding: flow fields over the room's cells toward where enemies are headed.
//!
//! Dijkstra from every goal cell at once over the cells a walker may enter, 8-way, with
//! diagonal steps only where both orthogonal cells are open too (no cutting a wall's
//! corner). Each cell then holds its path cost to the nearest goal, and an enemy walks
//! to whichever neighbor is cheapest. Rooms are at most `MAX_SIDE` squared cells, so
//! fields are rebuilt every tick (one per goal, when some enemy needs it) rather than
//! cached: they are derived state, never stored.
//! Costs, heap order and neighbor order are all integer and fixed, so every machine
//! builds the same field and picks the same steps.

use crate::room::{Body, Tiles, cell_center, cell_of};
use crate::{Fx, FxVec2};
use std::cell::OnceCell;
use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};

/// Step costs: 2 orthogonal, 3 diagonal (about sqrt 2 : 1), so paths hug corners
/// instead of zigzagging.
const STRAIGHT: u32 = 2;
const DIAGONAL: u32 = 3;
/// [`walk_clear`]'s sample spacing along the major axis: far under a cell, so no
/// blocking cell's corner slips between two samples of the box.
const SAMPLE: Fx = Fx::from_bits(8 << 32);
/// Neighbor offsets in tie-break order: orthogonals first, then diagonals.
const NEIGHBORS: [(i32, i32); 8] = [
    (1, 0),
    (0, 1),
    (-1, 0),
    (0, -1),
    (1, 1),
    (-1, 1),
    (-1, -1),
    (1, -1),
];

/// The field toward a set of goals, built on first use: most ticks every enemy has a
/// clear line to its target and never asks.
#[derive(Clone, Debug)]
pub struct FlowField {
    tiles: Tiles,
    goals: Vec<FxVec2>,
    grid: OnceCell<Grid>,
}

impl FlowField {
    /// The field toward whichever of `goals` is nearest by path. A goal's own cell is
    /// seeded even if walkers can't enter it (a player rolling over a pit), so enemies
    /// still close in on its edge.
    #[must_use]
    pub fn toward(tiles: Tiles, goals: &[FxVec2]) -> Self {
        Self {
            tiles,
            goals: goals.to_vec(),
            grid: OnceCell::new(),
        }
    }

    /// Where an enemy at `pos` should head next: the center of its cheapest neighbor
    /// cell. `None` when it is already in a goal cell, off the field, or nothing is
    /// cheaper (walled off from every goal).
    #[must_use]
    pub fn next(&self, pos: FxVec2) -> Option<FxVec2> {
        self.grid
            .get_or_init(|| Grid::build(self.tiles, &self.goals))
            .next(pos)
    }
}

/// One [`FlowField`] per goal cell, made the first time an enemy heads there: each
/// enemy chases its own target (a player it sees, or where it last saw one). Keyed by
/// cell in a `BTreeMap`, so nothing depends on the order enemies ask.
#[derive(Clone, Debug)]
pub struct FlowFields {
    tiles: Tiles,
    fields: BTreeMap<(i32, i32), FlowField>,
}

impl FlowFields {
    #[must_use]
    pub const fn new(tiles: Tiles) -> Self {
        Self {
            tiles,
            fields: BTreeMap::new(),
        }
    }

    /// The field toward `goal`'s cell.
    pub fn toward(&mut self, goal: FxVec2) -> &FlowField {
        let tiles = self.tiles;
        self.fields
            .entry((cell_of(goal.x), cell_of(goal.y)))
            .or_insert_with(|| FlowField::toward(tiles, &[goal]))
    }
}

/// Per-cell grids, row-major.
#[derive(Clone, Debug)]
struct Grid {
    width: usize,
    /// Whether a walker may enter the cell.
    open: Vec<bool>,
    /// Path cost to the nearest goal; `None` = unreachable or blocked.
    cost: Vec<Option<u32>>,
}

impl Grid {
    fn build(tiles: Tiles, goals: &[FxVec2]) -> Self {
        let (width, height) = (tiles.room.width(), tiles.room.height());
        let cells = (0..height).flat_map(|y| (0..width).map(move |x| (x, y)));
        let open = cells
            .map(|(x, y)| match (i32::try_from(x), i32::try_from(y)) {
                (Ok(x), Ok(y)) => !tiles.blocks(x, y, Body::Walker),
                _ => false,
            })
            .collect();
        let mut field = Self {
            width,
            open,
            cost: vec![None; width.saturating_mul(height)],
        };
        let mut open = BinaryHeap::new();
        for &goal in goals {
            let cell = (cell_of(goal.x), cell_of(goal.y));
            if let Some(i) = field.index(cell) {
                open.push(Reverse((0, i, cell)));
            }
        }
        while let Some(Reverse((cost, i, cell))) = open.pop() {
            match field.cost.get_mut(i) {
                Some(Some(_)) | None => continue,
                Some(slot) => *slot = Some(cost),
            }
            for (to, step) in field.steps(cell) {
                if let Some(j) = field.index(to)
                    && field.cost.get(j) == Some(&None)
                {
                    open.push(Reverse((cost.saturating_add(step), j, to)));
                }
            }
        }
        field
    }

    /// See [`FlowField::next`].
    fn next(&self, pos: FxVec2) -> Option<FxVec2> {
        let here = (cell_of(pos.x), cell_of(pos.y));
        let own = self.cost_at(here).filter(|&c| c > 0)?;
        let (best, cost) = self
            .steps(here)
            .filter_map(|(to, _)| Some((to, self.cost_at(to)?)))
            // `min_by_key` keeps the first of equals: `NEIGHBORS` order breaks ties.
            .min_by_key(|&(_, cost)| cost)?;
        if cost >= own {
            return None;
        }
        let x = usize::try_from(best.0).ok()?;
        let y = usize::try_from(best.1).ok()?;
        Some(cell_center(x, y))
    }

    fn cost_at(&self, cell: (i32, i32)) -> Option<u32> {
        self.cost.get(self.index(cell)?).copied().flatten()
    }

    /// Row-major index of an on-grid cell.
    fn index(&self, (x, y): (i32, i32)) -> Option<usize> {
        let (x, y) = (usize::try_from(x).ok()?, usize::try_from(y).ok()?);
        if x >= self.width {
            return None;
        }
        y.checked_mul(self.width)?
            .checked_add(x)
            .filter(|&i| i < self.open.len())
    }

    /// The walkable neighbors of `cell` and the cost of stepping to each, in
    /// [`NEIGHBORS`] order. A diagonal needs both cells it passes between open.
    fn steps(&self, (x, y): (i32, i32)) -> impl Iterator<Item = ((i32, i32), u32)> {
        let open = move |dx: i32, dy: i32| {
            let cell = (x.saturating_add(dx), y.saturating_add(dy));
            self.index(cell)
                .and_then(|i| self.open.get(i))
                .is_some_and(|&open| open)
        };
        NEIGHBORS.into_iter().filter_map(move |(dx, dy)| {
            let to = (x.saturating_add(dx), y.saturating_add(dy));
            if dx == 0 || dy == 0 {
                open(dx, dy).then_some((to, STRAIGHT))
            } else {
                (open(dx, dy) && open(dx, 0) && open(0, dy)).then_some((to, DIAGONAL))
            }
        })
    }
}

/// Whether a walker's box of half-size `half` can travel straight from `from` to `to`
/// without touching a cell that blocks it. Samples the line every [`SAMPLE`] or less.
#[must_use]
pub fn walk_clear(tiles: Tiles, from: FxVec2, to: FxVec2, half: Fx) -> bool {
    let (dx, dy) = (to.x.saturating_sub(from.x), to.y.saturating_sub(from.y));
    let span = dx.abs().max(dy.abs());
    let steps = span
        .checked_div(SAMPLE)
        .map_or(1, |n| n.saturating_to_num::<i64>().saturating_add(1));
    let (Some(sx), Some(sy)) = (dx.checked_div_int(steps), dy.checked_div_int(steps)) else {
        return false;
    };
    let mut at = from;
    (0..=steps).all(|_| {
        let clear = box_clear(tiles, at, half);
        at = FxVec2 {
            x: at.x.saturating_add(sx),
            y: at.y.saturating_add(sy),
        };
        clear
    })
}

/// Whether no cell under the box of half-size `half` around `p` blocks a walker.
fn box_clear(tiles: Tiles, p: FxVec2, half: Fx) -> bool {
    let span = |v: Fx| {
        cell_of(v.saturating_sub(half))..=cell_of(v.saturating_add(half).saturating_sub(Fx::DELTA))
    };
    span(p.y).all(|y| span(p.x).all(|x| !tiles.blocks(x, y, Body::Walker)))
}
