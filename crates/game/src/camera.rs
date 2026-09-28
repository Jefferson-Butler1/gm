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
}

#[derive(uniffi::Record, Clone, Copy, Debug, PartialEq)]
pub struct CameraSettings {
    pub look: LookMode,
    /// Furthest lean, pt.
    pub lead: f32,
    /// Time constant easing the lean toward its target, s; 0 snaps.
    pub smoothing_secs: f32,
}

#[uniffi::export]
#[must_use]
pub const fn default_camera_settings() -> CameraSettings {
    CameraSettings {
        look: LookMode::Aim,
        lead: 80.0,
        smoothing_secs: 0.2,
    }
}

pub struct CameraLook {
    settings: CameraSettings,
    /// The current lean, pt.
    offset: [f32; 2],
    /// Display time of the last update, for frame-rate-independent easing.
    last: Option<f64>,
}

impl CameraLook {
    pub const fn new() -> Self {
        Self {
            settings: default_camera_settings(),
            offset: [0.0, 0.0],
            last: None,
        }
    }

    pub const fn set(&mut self, settings: CameraSettings) {
        self.settings = settings;
    }

    /// Eases the lean toward this frame's target and returns it. `facing` is the player's
    /// (one turn = 65536), `aim` how far the aim is pushed, `0..=1`.
    pub fn update(&mut self, now: f64, facing: Option<u16>, aim: f32) -> [f32; 2] {
        let weight = match self.settings.look {
            LookMode::Centered => 0.0,
            LookMode::Aim => aim,
            LookMode::Facing => 1.0,
        };
        let target = facing.map_or([0.0, 0.0], |f| {
            let (sin, cos) = (f32::from(f) / 65536.0 * TAU).sin_cos();
            let reach = self.settings.lead * weight;
            [cos * reach, sin * reach]
        });
        // No From<f64> for f32; a frame's duration fits.
        #[allow(clippy::as_conversions, clippy::cast_possible_truncation)]
        let dt = self
            .last
            .map_or(0.0, |last| (now - last).clamp(0.0, 0.1) as f32);
        self.last = Some(now);
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
}
