//! Camera look: how far the camera leans off the player, ETG's "aim look". ETG's
//! controller camera leads along the right stick by how far it's pushed; the mouse camera
//! sits partway toward the cursor. Settings apply live (presentation only, not the sim).

use std::f32::consts::TAU;

/// What the camera leans toward.
#[derive(uniffi::Enum, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LookMode {
    /// The player dead center.
    Centered,
    /// ETG controller: along the aim, by how far the aim stick is pushed; centered again
    /// once it's let go.
    #[default]
    Aim,
    /// Along the player's facing at all times: ahead of the walk, or along the aim while
    /// shooting. Stays leaning when standing still.
    Facing,
    /// ETG mouse-style, for auto-aim: halfway toward the nearest enemy in the player's
    /// room (what auto-aim shoots at), capped at the lead; centered once the room is clear.
    Enemy,
}

#[derive(uniffi::Record, Clone, Copy, Debug, PartialEq)]
pub struct CameraSettings {
    pub look: LookMode,
    /// Furthest lean, pt.
    pub lead: f32,
    /// Time constant easing the lean toward its target, s; 0 snaps.
    pub smoothing_secs: f32,
    /// How far the view shifts down so the player sits above center, pt: the thumbs on
    /// the bottom-corner controls hide less of what's below. Added to any look.
    pub thumb_clearance: f32,
    /// Tilt peek: tipping the phone leans the view, pt per g of tilt; 0 is off. Raise an
    /// edge to look toward it.
    /// Only quick tilts count: the rest pose re-centers over [`TILT_REST_SECS`], so a
    /// grip that settles doesn't hold the view off. Added to any look.
    pub tilt_peek: f32,
}

/// How fast the tilt peek's rest pose follows the phone.
const TILT_REST_SECS: f32 = 1.5;

#[uniffi::export]
#[must_use]
pub const fn default_camera_settings() -> CameraSettings {
    CameraSettings {
        look: LookMode::Aim,
        lead: 80.0,
        smoothing_secs: 0.2,
        thumb_clearance: 40.0,
        tilt_peek: 0.0,
    }
}

pub struct CameraLook {
    settings: CameraSettings,
    /// The current lean, pt.
    offset: [f32; 2],
    /// Display time of the last update, for frame-rate-independent easing.
    last: Option<f64>,
    /// Gravity in screen axes (+x right, +y down), in g, while tilt peek runs.
    tilt: Option<[f32; 2]>,
    /// The tilt peek's rest pose: `tilt`, slowly followed.
    rest: Option<[f32; 2]>,
}

impl CameraLook {
    pub const fn new() -> Self {
        Self {
            settings: default_camera_settings(),
            offset: [0.0, 0.0],
            last: None,
            tilt: None,
            rest: None,
        }
    }

    pub const fn set(&mut self, settings: CameraSettings) {
        self.settings = settings;
    }

    /// This frame's gravity in screen axes, or `None` with tilt peek off.
    pub const fn set_tilt(&mut self, gravity: Option<[f32; 2]>) {
        self.tilt = gravity;
    }

    /// Eases the lean toward this frame's target and returns it. `facing` is the player's
    /// (one turn = 65536), `aim` how far the aim is pushed, `0..=1`, and `enemy` the
    /// offset from the player to the nearest enemy in its room.
    pub fn update(
        &mut self,
        now: f64,
        facing: Option<u16>,
        aim: f32,
        enemy: Option<[f32; 2]>,
    ) -> [f32; 2] {
        let lead = self.settings.lead;
        let along = |weight: f32| {
            facing.map_or([0.0, 0.0], |f| {
                let (sin, cos) = (f32::from(f) / 65536.0 * TAU).sin_cos();
                [cos * lead * weight, sin * lead * weight]
            })
        };
        let toward = match self.settings.look {
            LookMode::Centered => [0.0, 0.0],
            LookMode::Aim => along(aim),
            LookMode::Facing => along(1.0),
            LookMode::Enemy => enemy.map_or([0.0, 0.0], |[x, y]| {
                let k = (lead / x.hypot(y).max(f32::EPSILON)).min(0.5);
                [x * k, y * k]
            }),
        };
        // No From<f64> for f32; a frame's duration fits.
        #[allow(clippy::as_conversions, clippy::cast_possible_truncation)]
        let dt = self
            .last
            .map_or(0.0, |last| (now - last).clamp(0.0, 0.1) as f32);
        self.last = Some(now);
        let peek = self.peek(dt);
        let target = [
            toward[0] + peek[0],
            toward[1] + self.settings.thumb_clearance + peek[1],
        ];
        let tau = self.settings.smoothing_secs;
        let k = if tau <= 0.0 {
            1.0
        } else {
            1.0 - (-dt / tau).exp()
        };
        for (o, t) in self.offset.iter_mut().zip(target) {
            *o = (t - *o).mul_add(k, *o);
        }
        self.offset
    }

    /// The tilt peek, pt: how far gravity has swung from the rest pose, which then eases
    /// after it.
    fn peek(&mut self, dt: f32) -> [f32; 2] {
        let Some(tilt) = self.tilt.filter(|_| self.settings.tilt_peek > 0.0) else {
            self.rest = None;
            return [0.0, 0.0];
        };
        let rest = self.rest.get_or_insert(tilt);
        // Raising an edge (gravity swinging away from it) looks toward it.
        let peek = [
            (rest[0] - tilt[0]) * self.settings.tilt_peek,
            (rest[1] - tilt[1]) * self.settings.tilt_peek,
        ];
        let k = 1.0 - (-dt / TILT_REST_SECS).exp();
        for (r, t) in rest.iter_mut().zip(tilt) {
            *r = (t - *r).mul_add(k, *r);
        }
        peek
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quick_tilt_peeks_that_way_then_the_rest_pose_catches_up() {
        let mut camera = CameraLook::new();
        camera.set(CameraSettings {
            look: LookMode::Centered,
            smoothing_secs: 0.0,
            thumb_clearance: 0.0,
            tilt_peek: 400.0,
            ..default_camera_settings()
        });
        camera.set_tilt(Some([0.0, 0.5]));
        let [x, y] = camera.update(0.0, None, 0.0, None);
        assert!(x.abs() < 0.01 && y.abs() < 0.01, "the rest pose");
        camera.set_tilt(Some([-0.1, 0.5])); // right edge raised
        let [x, _] = camera.update(0.01, None, 0.0, None);
        assert!((x - 40.0).abs() < 1.0, "{x}");
        // Held for 10 s of 60 Hz frames.
        let x = (1..=600)
            .map(|frame| camera.update(0.01 + f64::from(frame) / 60.0, None, 0.0, None)[0])
            .last()
            .unwrap_or(f32::NAN);
        assert!(x.abs() < 1.0, "held, it re-centers: {x}");
    }
}
