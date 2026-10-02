//! Where the minimap sits: top right of the safe area, just left of the settings button,
//! clear of both thumbs.
//!
//! No default control sits there: the sticks and buttons hug the bottom corners, the HUD
//! takes the top left, and the claw's index-finger buttons sit below it.

use crate::controls::Viewport;
use render::minimap::Bounds;

/// The most room the minimap takes, in points; the renderer fits the ship inside.
const MAX_SIZE: [f32; 2] = [160.0, 64.0];
/// Gap below the safe area's top edge, in points (the panel's border sits a few points
/// outside the box).
const TOP_INSET: f32 = 10.0;
/// Kept free at the safe area's right edge for Swift's settings button (a 38 pt circle,
/// 8 pt in), in points.
const RIGHT_INSET: f32 = 54.0;

/// The minimap's box on `v`.
pub fn bounds(v: &Viewport) -> Bounds {
    let safe_width = (v.point_width - v.safe_left - v.safe_right).max(0.0);
    let size = [
        MAX_SIZE[0].min((safe_width - RIGHT_INSET).max(0.0)),
        MAX_SIZE[1],
    ];
    let right = v.point_width - v.safe_right - RIGHT_INSET;
    Bounds {
        at: [right - size[0], v.safe_top + TOP_INSET],
        size,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controls::Scheme;
    use crate::layout::{default_control_layout, place_controls};

    #[test]
    fn no_default_control_overlaps_the_minimap() {
        let landscape = Viewport {
            point_width: 852.0,
            point_height: 393.0,
            safe_left: 59.0,
            safe_right: 59.0,
            safe_bottom: 21.0,
            ..Viewport::default()
        };
        let portrait = Viewport {
            point_width: 393.0,
            point_height: 852.0,
            safe_top: 59.0,
            safe_bottom: 34.0,
            ..Viewport::default()
        };
        let schemes = [
            Scheme::FloatingSticks,
            Scheme::FixedSticks,
            Scheme::AutoAim,
            Scheme::AimAssist,
            Scheme::FixedAutoAim,
            Scheme::Claw,
            Scheme::FireButton,
        ];
        for v in [landscape, portrait] {
            let Bounds { at, size } = bounds(&v);
            for scheme in schemes {
                for c in place_controls(scheme, default_control_layout(scheme, v), v) {
                    // The nearest point of the box to the control's center.
                    let near = [
                        c.x.clamp(at[0], at[0] + size[0]),
                        c.y.clamp(at[1], at[1] + size[1]),
                    ];
                    let gap = (c.x - near[0]).hypot(c.y - near[1]);
                    // The minimap takes no touches, so only the drawn control must stay clear.
                    assert!(gap > c.radius, "{scheme:?} {:?} at {v:?}", c.kind);
                }
            }
        }
    }
}
