//! Camera look: how far the camera leans off the player, ETG's "aim look". ETG's
//! controller camera leads along the right stick by how far it's pushed; the mouse camera
//! sits partway toward the cursor. Settings apply live (presentation only, not the sim).

use crate::view_box::Frame;
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
    /// Tilt peek: tipping the phone leans the view up to this far, pt; 0 is off. Raise an
    /// edge to see more that way. Past a dead zone of [`TILT_DEADZONE`] the lean grows
    /// with the tilt, reaching all of it [`TILT_FULL`] further on. The rest pose follows
    /// the grip over [`TILT_REST_SECS`], so a held tilt lasts but a new grip settles.
    /// Capped at [`TILT_PEEK_MAX`]. Added to any look.
    pub tilt_peek: f32,
}

/// How fast the tilt peek's rest pose follows the phone.
const TILT_REST_SECS: f32 = 5.0;
/// Tilt the peek ignores, so a hand's wobble doesn't jitter the view: about 3°, radians.
const TILT_DEADZONE: f32 = 0.05;
/// Tilt past the dead zone that peeks the whole way: about 20°, radians.
const TILT_FULL: f32 = 0.35;
/// The furthest any tilt peek setting leans, pt.
pub const TILT_PEEK_MAX: f32 = 300.0;

#[uniffi::export]
#[must_use]
pub const fn default_camera_settings() -> CameraSettings {
    CameraSettings {
        look: LookMode::Aim,
        lead: 80.0,
        smoothing_secs: 0.2,
        tilt_peek: 0.0,
    }
}

pub struct CameraLook {
    settings: CameraSettings,
    /// The current lean, pt.
    offset: [f32; 2],
    /// Display time of the last update, for frame-rate-independent easing.
    last: Option<f64>,
    /// The phone's [roll, pitch] in radians while tilt peek runs: roll rises as the right
    /// edge dips, pitch as the bottom edge does.
    tilt: Option<[f32; 2]>,
    /// The tilt peek's rest pose: `tilt`, slowly followed.
    rest: Option<[f32; 2]>,
    /// The view box on the current screen.
    frame: Frame,
}

impl CameraLook {
    pub const fn new() -> Self {
        Self {
            settings: default_camera_settings(),
            offset: [0.0, 0.0],
            last: None,
            tilt: None,
            rest: None,
            frame: Frame::FREE,
        }
    }

    pub const fn set(&mut self, settings: CameraSettings) {
        self.settings = settings;
    }

    /// The view box for the current screen and orientation.
    pub const fn set_frame(&mut self, frame: Frame) {
        self.frame = frame;
    }

    /// This frame's [roll, pitch], or `None` with tilt peek off.
    pub const fn set_tilt(&mut self, angles: Option<[f32; 2]>) {
        self.tilt = angles;
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
        // Every lean starts from the view box's center and stays inside the box.
        let rest = self.frame.rest;
        let target = self
            .frame
            .clamp([rest[0] + toward[0] + peek[0], rest[1] + toward[1] + peek[1]]);
        let tau = self.settings.smoothing_secs;
        let k = if tau <= 0.0 {
            1.0
        } else {
            1.0 - (-dt / tau).exp()
        };
        for (o, t) in self.offset.iter_mut().zip(target) {
            *o = (t - *o).mul_add(k, *o);
        }
        // Easing toward a box that just moved (a turn of the phone) can't leave it either.
        self.offset = self.frame.clamp(self.offset);
        self.offset
    }

    /// The tilt peek, pt: how far the phone has tipped from the rest pose, which then
    /// eases after it.
    fn peek(&mut self, dt: f32) -> [f32; 2] {
        let Some(tilt) = self.tilt.filter(|_| self.settings.tilt_peek > 0.0) else {
            self.rest = None;
            return [0.0, 0.0];
        };
        let rest = self.rest.get_or_insert(tilt);
        // Raising an edge looks toward it: raising the right one lowers the roll, raising
        // the top one tips the phone toward upright, raising the pitch.
        let swing = [rest[0] - tilt[0], rest[1] - tilt[1]];
        let angle = swing[0].hypot(swing[1]);
        let reach = ((angle - TILT_DEADZONE) / TILT_FULL).clamp(0.0, 1.0)
            * self.settings.tilt_peek.min(TILT_PEEK_MAX);
        let k = reach / angle.max(f32::EPSILON);
        let peek = [swing[0] * k, swing[1] * k];
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

    fn peeking(tilt_peek: f32) -> CameraLook {
        let mut camera = CameraLook::new();
        camera.set(CameraSettings {
            look: LookMode::Centered,
            smoothing_secs: 0.0,
            tilt_peek,
            ..default_camera_settings()
        });
        camera.set_tilt(Some([0.0, 0.8]));
        let [x, y] = camera.update(0.0, None, 0.0, None);
        assert!(x.abs() < 0.01 && y.abs() < 0.01, "the rest pose");
        camera
    }

    #[test]
    fn raising_an_edge_peeks_toward_it_past_a_dead_zone_up_to_the_setting() {
        let mut camera = peeking(200.0);
        camera.set_tilt(Some([0.03, 0.8])); // a wobble
        assert!(camera.update(0.01, None, 0.0, None)[0].abs() < 0.01);
        camera.set_tilt(Some([-0.05 - 0.35 / 2.0, 0.8])); // right edge raised ~13°
        let [x, _] = camera.update(0.02, None, 0.0, None);
        assert!((x - 100.0).abs() < 2.0, "halfway: {x}");
        camera.set_tilt(Some([0.0, 1.3])); // top edge raised ~29°, toward upright
        let [_, y] = camera.update(0.03, None, 0.0, None);
        assert!((y + 200.0).abs() < 2.0, "all the way up: {y}");
    }

    #[test]
    fn a_held_tilt_lasts_a_while_then_the_rest_pose_catches_up() {
        let mut camera = peeking(200.0);
        camera.set_tilt(Some([-0.4, 0.8]));
        let at = |camera: &mut CameraLook, secs: f64| camera.update(secs, None, 0.0, None)[0];
        let x = (1..=60)
            .map(|f| at(&mut camera, f64::from(f) / 60.0))
            .last();
        assert!(
            x.is_some_and(|x| x > 150.0),
            "still peeking after 1 s: {x:?}"
        );
        let x = (61..=1800)
            .map(|f| at(&mut camera, f64::from(f) / 60.0))
            .last();
        assert!(
            x.is_some_and(|x| x.abs() < 1.0),
            "re-centered by 30 s: {x:?}"
        );
    }
}
