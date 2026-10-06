//! The minimap: the revealed ship (ETG fog) in a small translucent panel.
//!
//! It shows the hatches by state and slot 0's position. Once the bridge falls, the
//! panel's border glows on the side toward the nearest unlocked airlock, pulsing with the
//! airlock hatches. Where the panel goes belongs to `game`; it arrives as [`Bounds`].

use crate::{
    AIRLOCK_LOCKED_COLOR, AIRLOCK_OPEN_COLOR, AIRLOCK_PULSE_TICKS, CIRCLE, HATCH_CLOSED_COLOR,
    HATCH_OPEN_COLOR, HATCH_SEALED_COLOR, PLAYER_COLOR, Quad, Renderer, SQUARE, unlocked_airlocks,
};
use sim::room::{Cell, Dir};
use sim::ship::Spot;
use sim::{HatchKind, HatchState, SimState};
use std::f32::consts::TAU;

const PANEL_COLOR: [f32; 4] = [0.0, 0.0, 0.0, 0.35];
const BORDER_COLOR: [f32; 4] = [1.0, 1.0, 1.0, 0.15];
/// Revealed floor, and brighter walls so rooms read as outlined shapes.
const FLOOR_COLOR: [f32; 4] = [0.55, 0.65, 0.8, 0.3];
const WALL_COLOR: [f32; 4] = [0.7, 0.8, 0.95, 0.6];
/// Ship-to-border padding, in view points.
const PAD: f32 = 4.0;
const PLAYER_RADIUS: f32 = 2.5;
/// The airlock glow: samples along the border within this many radians of the airlock's
/// direction, each a square this half-size, fading toward the ends.
const GLOW_SPREAD: f32 = 0.5;
const GLOW_SAMPLES: u8 = 8;
const GLOW_HALF: f32 = 2.0;

/// The box the minimap fits in, in view points (origin top-left). The ship is scaled to
/// fit it, keeping its shape, and centered.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub at: [f32; 2],
    pub size: [f32; 2],
}

/// What a revealed cell draws as; hatches are drawn by state on top.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tile {
    Floor,
    Wall,
}

impl Renderer {
    /// The minimap, if [`Self::set_minimap`] placed one. `alpha` smooths the glow's pulse.
    pub(crate) fn push_minimap(&mut self, state: &SimState, alpha: f32) {
        let Some(bounds) = self.minimap else {
            return;
        };
        let (width, height) = state.ship.size();
        let (Ok(width), Ok(height)) = (u16::try_from(width), u16::try_from(height)) else {
            return;
        };
        if width == 0 || height == 0 {
            return;
        }
        // View points per cell, and the ship's top-left.
        let scale = (bounds.size[0] / f32::from(width)).min(bounds.size[1] / f32::from(height));
        let half = [
            f32::from(width) * scale / 2.0,
            f32::from(height) * scale / 2.0,
        ];
        let center = [
            bounds.at[0] + bounds.size[0] / 2.0,
            bounds.at[1] + bounds.size[1] / 2.0,
        ];
        let origin = [center[0] - half[0], center[1] - half[1]];
        let at = |x: f32, y: f32| [x.mul_add(scale, origin[0]), y.mul_add(scale, origin[1])];
        let panel = [half[0] + PAD, half[1] + PAD];
        self.push_box(center, panel, PANEL_COLOR);
        // The border, as four thin edges: (which side, half-size).
        for ([sx, sy], edge) in [
            ([0.0_f32, -1.0_f32], [panel[0], 0.5]),
            ([0.0, 1.0], [panel[0], 0.5]),
            ([-1.0, 0.0], [0.5, panel[1]]),
            ([1.0, 0.0], [0.5, panel[1]]),
        ] {
            let mid = [
                sx.mul_add(panel[0], center[0]),
                sy.mul_add(panel[1], center[1]),
            ];
            self.push_box(mid, edge, BORDER_COLOR);
        }

        // Each row's revealed cells as runs of one tile, a quad per run.
        for y in 0..height {
            let mut run: Option<(u16, Tile)> = None;
            for x in 0..=width {
                let next = if x < width {
                    tile(state, i32::from(x), i32::from(y))
                } else {
                    None
                };
                if run.map(|(_, t)| t) == next {
                    continue;
                }
                if let Some((start, t)) = run {
                    let color = match t {
                        Tile::Floor => FLOOR_COLOR,
                        Tile::Wall => WALL_COLOR,
                    };
                    let [x0, y0] = at(f32::from(start), f32::from(y));
                    let [x1, y1] = at(f32::from(x), f32::from(y) + 1.0);
                    self.push_rect([x0, y0], [x1, y1], color);
                }
                run = next.map(|t| (x, t));
            }
        }

        let pulse = pulse(state.tick, alpha);
        self.push_minimap_hatches(state, &at, pulse);

        let Some(player) = state.players[0] else {
            return;
        };
        let cell = sim::room::CELL.to_num::<f32>();
        let pos = [player.pos.x.to_num::<f32>(), player.pos.y.to_num::<f32>()];
        let dot = at(pos[0] / cell, pos[1] / cell);
        self.push_screen(dot, PLAYER_RADIUS, PLAYER_COLOR, CIRCLE);

        let dist_sq = |[x, y]: [f32; 2]| (x - pos[0]).mul_add(x - pos[0], (y - pos[1]).powi(2));
        let nearest = unlocked_airlocks(state)
            .into_iter()
            .min_by(|a, b| dist_sq(*a).total_cmp(&dist_sq(*b)));
        if let Some([x, y]) = nearest {
            let [r, g, b, _] = AIRLOCK_OPEN_COLOR;
            let strength = pulse.mul_add(0.5, 0.5);
            for (offset, weight) in glow(panel, [x - pos[0], y - pos[1]]) {
                let p = [center[0] + offset[0], center[1] + offset[1]];
                self.push_box(p, [GLOW_HALF, GLOW_HALF], [r, g, b, weight * strength]);
            }
        }
    }

    /// The hatches of revealed rooms by state, through `at` (cells to view points). Unlocked
    /// airlocks pulse by `pulse`, as in the scene.
    fn push_minimap_hatches(
        &mut self,
        state: &SimState,
        at: &impl Fn(f32, f32) -> [f32; 2],
        pulse: f32,
    ) {
        for (hatch, live) in state.ship.hatches().iter().zip(&state.hatches) {
            if !hatch.rooms.iter().any(|&r| state.visited(r)) {
                continue;
            }
            let color = match live {
                HatchState::Closed if hatch.kind == HatchKind::Airlock => {
                    let [r, g, b, _] = AIRLOCK_OPEN_COLOR;
                    [r, g, b, pulse.mul_add(0.6, 0.4)]
                }
                HatchState::Closed => HATCH_CLOSED_COLOR,
                HatchState::Open => HATCH_OPEN_COLOR,
                HatchState::Sealed => HATCH_SEALED_COLOR,
                HatchState::AirlockLocked => AIRLOCK_LOCKED_COLOR,
                // Hidden: it draws as wall.
                HatchState::Panel => continue,
            };
            let gap = hatch.gap;
            let cell = |n: usize| u16::try_from(n).map_or(0.0, f32::from);
            let (x, y, w) = (cell(gap.x), cell(gap.y), cell(gap.width));
            let (w, h) = if matches!(gap.dir, Dir::North | Dir::South) {
                (w, 1.0)
            } else {
                (1.0, w)
            };
            self.push_rect(at(x, y), at(x + w, y + h), color);
        }
    }

    /// An axis-aligned box from its top-left to its bottom-right corner, in view points.
    fn push_rect(&mut self, [x0, y0]: [f32; 2], [x1, y1]: [f32; 2], color: [f32; 4]) {
        let half = [(x1 - x0) / 2.0, (y1 - y0) / 2.0];
        self.push_box([x0 + half[0], y0 + half[1]], half, color);
    }

    /// An axis-aligned box by center and half-size, in view points.
    fn push_box(&mut self, [x, y]: [f32; 2], [hx, hy]: [f32; 2], color: [f32; 4]) {
        let [w, h] = self.size_pt;
        self.quads.push(Quad {
            center: [(x / w).mul_add(2.0, -1.0), (y / h).mul_add(-2.0, 1.0)],
            half: [hx / w * 2.0, hy / h * 2.0],
            color,
            shape: SQUARE,
            dir: [1.0, 0.0],
            param: 0.0,
        });
    }
}

/// Cell (`x`, `y`) as the minimap shows it: floor and wall of revealed rooms and corridors
/// only (ETG fog). Pits are holes; hatches are drawn by state.
fn tile(state: &SimState, x: i32, y: i32) -> Option<Tile> {
    match state.ship.spot(x, y) {
        Spot::Room { room, cell } if state.visited(room) => match cell {
            Cell::Floor => Some(Tile::Floor),
            Cell::Wall => Some(Tile::Wall),
            Cell::Pit | Cell::Void => None,
        },
        Spot::Room { .. } | Spot::Hatch(_) | Spot::Void => None,
    }
}

/// 0 -> 1 -> 0 over [`AIRLOCK_PULSE_TICKS`], in step with the airlock hatches' pulse.
fn pulse(tick: u64, alpha: f32) -> f32 {
    let ticks = tick
        .checked_rem(u64::from(AIRLOCK_PULSE_TICKS))
        .and_then(|t| u16::try_from(t).ok())
        .map_or(0.0, f32::from)
        + alpha;
    0.5_f32.mul_add(-(ticks / f32::from(AIRLOCK_PULSE_TICKS) * TAU).cos(), 0.5)
}

/// The glow on a border of half-size `half` toward `toward` (floor space; +y down like the
/// view): sample points as offsets from the panel's center, where rays fanned across
/// [`GLOW_SPREAD`] either side of `toward` meet the border, each with its alpha, 1 at the
/// middle fading toward the ends. Empty when `toward` has no direction.
fn glow(half: [f32; 2], toward: [f32; 2]) -> impl Iterator<Item = ([f32; 2], f32)> {
    let heading = (toward[0].hypot(toward[1]) > f32::EPSILON).then(|| toward[1].atan2(toward[0]));
    let n = f32::from(GLOW_SAMPLES);
    let steps = (0..=GLOW_SAMPLES.saturating_mul(2)).map(move |i| (f32::from(i) - n) / n);
    heading.into_iter().flat_map(move |heading| {
        steps.clone().map(move |s| {
            let (sin, cos) = s.mul_add(GLOW_SPREAD, heading).sin_cos();
            // How far along the ray to the nearer of the edges it heads for.
            let reach = |d: f32, h: f32| {
                if d.abs() > f32::EPSILON {
                    h / d.abs()
                } else {
                    f32::INFINITY
                }
            };
            let t = reach(cos, half[0]).min(reach(sin, half[1]));
            ([cos * t, sin * t], 1.0 - s.abs())
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim::{RoomId, RunConfig};

    #[test]
    fn the_glow_hugs_the_border_on_the_airlock_side() {
        let half = [80.0, 32.0];
        let on_border = |[x, y]: [f32; 2]| {
            ((x.abs() - half[0]).abs() < 1e-3 && y.abs() <= half[1] + 1e-3)
                || ((y.abs() - half[1]).abs() < 1e-3 && x.abs() <= half[0] + 1e-3)
        };
        // Due east: the right edge, brightest at its middle.
        let east: Vec<_> = glow(half, [500.0, 0.0]).collect();
        assert!(east.iter().all(|&(p, _)| on_border(p) && p[0] > 0.0));
        let brightest = east.iter().max_by(|a, b| a.1.total_cmp(&b.1));
        assert!(brightest.is_some_and(|&([x, y], w)| {
            (x - half[0]).abs() < 1e-3 && y.abs() < 1e-3 && (w - 1.0).abs() < 1e-3
        }));
        // Up and a little left (view +y is down): the top edge, brightest left of center.
        let up: Vec<_> = glow(half, [-10.0, -100.0]).collect();
        assert!(up.iter().all(|&([_, y], _)| (y + half[1]).abs() < 1e-3));
        let brightest = up.iter().max_by(|a, b| a.1.total_cmp(&b.1));
        assert!(brightest.is_some_and(|&([x, _], _)| x < 0.0));
        // On top of the airlock there is no direction to glow in.
        assert_eq!(glow(half, [0.0, 0.0]).count(), 0);
    }

    #[test]
    fn a_fresh_run_shows_only_its_start_room() {
        let state = SimState::new(7, RunConfig::default());
        let (width, height) = state.ship.size();
        let (width, height) = (
            i32::try_from(width).unwrap_or(0),
            i32::try_from(height).unwrap_or(0),
        );
        let shown: Vec<RoomId> = (0..height)
            .flat_map(|y| (0..width).map(move |x| (x, y)))
            .filter(|&(x, y)| tile(&state, x, y).is_some())
            .filter_map(|(x, y)| match state.ship.spot(x, y) {
                Spot::Room { room, .. } => Some(room),
                Spot::Hatch(_) | Spot::Void => None,
            })
            .collect();
        let (start, _) = state.ship.start();
        assert!(!shown.is_empty());
        assert!(shown.iter().all(|&room| room == start));
    }
}
