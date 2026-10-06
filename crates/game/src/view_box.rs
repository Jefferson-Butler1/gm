//! The view box: the part of the screen the player always stays in, one per orientation.
//! The camera rests with the player at its center, and every lean (look, tilt peek) is
//! clamped so the player never leaves it. Jeff sets it in the view box editor; it's in
//! fractions of the safe area (0 = left/top edge, 1 = right/bottom), so a box carries
//! across screens.

use crate::controls::Viewport;

#[derive(uniffi::Record, Clone, Copy, Debug, PartialEq)]
pub struct ViewBox {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

/// Portrait keeps the player above the bottom-corner sticks and below the HUD; landscape
/// centers it, the thumbs off to the sides.
#[uniffi::export]
#[must_use]
pub const fn default_view_box(portrait: bool) -> ViewBox {
    if portrait {
        ViewBox {
            left: 0.15,
            top: 0.15,
            right: 0.85,
            bottom: 0.6,
        }
    } else {
        ViewBox {
            left: 0.3,
            top: 0.2,
            right: 0.7,
            bottom: 0.8,
        }
    }
}

/// Where the camera's lean may go, in view points. The player is drawn at the screen's
/// center less the lean, so `rest` puts it at the box's center and `min..=max` keeps it
/// inside the box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub rest: [f32; 2],
    pub min: [f32; 2],
    pub max: [f32; 2],
}

impl Frame {
    /// No box: the player dead center, leaning freely.
    pub const FREE: Self = Self {
        rest: [0.0, 0.0],
        min: [f32::NEG_INFINITY, f32::NEG_INFINITY],
        max: [f32::INFINITY, f32::INFINITY],
    };

    /// `b` on `v`.
    pub fn new(b: ViewBox, v: &Viewport) -> Self {
        let width = (v.point_width - v.safe_left - v.safe_right).max(0.0);
        let height = (v.point_height - v.safe_top - v.safe_bottom).max(0.0);
        let [cx, cy] = [v.point_width / 2.0, v.point_height / 2.0];
        let span = |origin: f32, size: f32, a: f32, b: f32| {
            let at = |f: f32| f.clamp(0.0, 1.0).mul_add(size, origin);
            (at(a.min(b)), at(a.max(b)))
        };
        let (x0, x1) = span(v.safe_left, width, b.left, b.right);
        let (y0, y1) = span(v.safe_top, height, b.top, b.bottom);
        Self {
            rest: [cx - f32::midpoint(x0, x1), cy - f32::midpoint(y0, y1)],
            min: [cx - x1, cy - y1],
            max: [cx - x0, cy - y0],
        }
    }

    /// `look`, kept inside the box.
    pub const fn clamp(self, look: [f32; 2]) -> [f32; 2] {
        [
            look[0].clamp(self.min[0], self.max[0]),
            look[1].clamp(self.min[1], self.max[1]),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_player_rests_at_the_box_center_and_never_leaves_it() {
        let v = Viewport {
            point_width: 400.0,
            point_height: 800.0,
            ..Viewport::default()
        };
        let box_ = ViewBox {
            left: 0.25,
            top: 0.25,
            right: 0.75,
            bottom: 0.5,
        };
        let frame = Frame::new(box_, &v);
        let near =
            |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).abs() < 0.01 && (a[1] - b[1]).abs() < 0.01;
        // The box spans x 100..300, y 200..400; the screen center is (200, 400).
        assert!(near(frame.rest, [0.0, 100.0]), "{:?}", frame.rest);
        // A big lean down-right still leaves the player at the box's top-left corner.
        assert!(near(frame.clamp([1000.0, 1000.0]), [100.0, 200.0]));
        assert!(near(frame.clamp([-1000.0, -1000.0]), [-100.0, 0.0]));
    }
}
