//! Frame pacing stats, published to [`HudData`] and logged (visible via `run.sh --console`).
//! Also carries the HUD's game status, since this owns the published [`HudData`].

use crate::{HudData, RunState};
use sim::{MAX_HP, PhasePistol, Run, SimState};

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

    /// Reads slot 0's HP and gun, the party's Scrap, the run state, and the room and wave
    /// from `state`: the fight's room, else the one slot 0 stands in (the last one while
    /// it crosses a hatch); and the hint: in a room whose chest is open, or once the airlocks unlock.
    /// Publishes immediately (bumps `seq`) when anything changed.
    pub fn set_status(&mut self, state: &SimState) {
        let id = match state.run {
            Run::Encounter { room, .. } => Some(room),
            Run::Boarding | Run::Dead { .. } | Run::Won => {
                state.players[0].and_then(|p| state.ship.room_at(p.pos))
            }
        };
        let room = id.and_then(|id| state.ship.room(id)).map(|p| &p.room);
        let hp = state.players[0].map_or(0, |p| p.hp);
        let run = RunState::of(state.run);
        let name = room.map_or(self.hud.room.as_str(), |r| r.name).to_owned();
        // Base layer plus reinforcements; a room without enemies has no waves.
        let waves = room.filter(|r| r.has_enemies()).map_or(0, |r| {
            u8::try_from(r.reinforcements.len())
                .unwrap_or(u8::MAX)
                .saturating_add(1)
        });
        let wave = if let Run::Encounter { wave, .. } = state.run {
            wave.saturating_add(1)
        } else {
            0
        };
        let hint = if id.is_some_and(|id| state.chest_opened(id)) {
            "Chest opened (items coming soon)"
        } else if state.airlocks_unlocked() {
            "Bridge clear: escape through any airlock"
        } else {
            ""
        };
        let gun = state.players[0].map(|p| p.gun);
        let charges = gun.map_or(0, |g| g.charges);
        let max_charges = state.config.tuning.charges;
        let venting = gun.is_some_and(|g| g.venting());
        let vent_progress = gun.filter(PhasePistol::venting).map_or(0.0, |g| {
            1.0 - f32::from(g.vent_ticks) / f32::from(g.vent_length.max(1))
        });
        let scrap = state.scrap;
        let h = &mut self.hud;
        let gun_changed = (h.charges, h.max_charges, h.venting) != (charges, max_charges, venting)
            || h.vent_progress.to_bits() != vent_progress.to_bits();
        if (h.hp, h.max_hp, h.run, h.wave, h.waves) != (hp, MAX_HP, run, wave, waves)
            || h.room != name
            || h.hint != hint
            || gun_changed
            || h.scrap != scrap
        {
            (h.hp, h.max_hp, h.run, h.wave, h.waves) = (hp, MAX_HP, run, wave, waves);
            (h.charges, h.max_charges, h.venting, h.vent_progress) =
                (charges, max_charges, venting, vent_progress);
            h.scrap = scrap;
            h.room = name;
            hint.clone_into(&mut h.hint);
            h.seq = h.seq.wrapping_add(1);
        }
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
