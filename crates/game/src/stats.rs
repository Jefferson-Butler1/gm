//! Frame pacing stats, published to [`HudData`] and logged (visible via `run.sh --console`).

use crate::HudData;

/// How often the HUD and log refresh, in seconds.
const WINDOW_SECS: f64 = 0.5;
/// Intervals longer than this are pauses, not frames.
const MAX_INTERVAL_SECS: f64 = 0.25;

#[derive(Default)]
pub struct Stats {
    hud: HudData,
    last_timestamp: Option<f64>,
    window_start: f64,
    frames: u32,
    interval_sum: f64,
    interval_max: f64,
    rust_sum: f64,
}

impl Stats {
    pub fn hud(&self) -> HudData {
        self.hud.clone()
    }

    pub fn record(
        &mut self,
        timestamp: f64,
        rust_secs: f64,
        presented: bool,
        tick: u64,
    ) -> HudData {
        let interval = self.last_timestamp.map(|last| timestamp - last);
        self.last_timestamp = Some(timestamp);
        if let Some(interval) = interval.filter(|&i| i > 0.0 && i < MAX_INTERVAL_SECS)
            && presented
        {
            self.frames = self.frames.saturating_add(1);
            self.interval_sum += interval;
            self.interval_max = self.interval_max.max(interval);
            self.rust_sum += rust_secs;
        }

        if timestamp - self.window_start >= WINDOW_SECS {
            if self.frames > 0 {
                let n = f64::from(self.frames);
                let h = &mut self.hud;
                h.seq = h.seq.wrapping_add(1);
                h.fps = n / self.interval_sum;
                h.frame_ms_avg = self.interval_sum / n * 1e3;
                h.frame_ms_max = self.interval_max * 1e3;
                h.rust_ms_avg = self.rust_sum / n * 1e3;
                h.tick = tick;
                eprintln!(
                    "[gm] fps={:.1} frame_ms avg={:.2} max={:.2} rust_ms avg={:.3} tick={}",
                    h.fps, h.frame_ms_avg, h.frame_ms_max, h.rust_ms_avg, h.tick
                );
            }
            *self = Self {
                hud: std::mem::take(&mut self.hud),
                last_timestamp: self.last_timestamp,
                window_start: timestamp,
                ..Self::default()
            };
        }
        self.hud()
    }
}
