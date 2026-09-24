//! The game loop and the `UniFFI` surface Swift talks to.
//!
//! Swift owns the lifecycle, the `CADisplayLink` and touch. Each display-link callback
//! calls [`Game::frame`], which steps the sim at a fixed [`TICK_HZ`] toward the display
//! time, renders `prev` -> `current` interpolated, and returns [`HudData`] for `SwiftUI`.

mod controls;
mod stats;

use controls::{Controls, Scheme, Viewport};
use render::Renderer;
use sim::{MAX_HP, Run, SimState, TICK_HZ, TickInputs};
use stats::Stats;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

uniffi::setup_scaffolding!();

/// If the sim falls further behind the display than this (backgrounding, a debugger
/// pause, a huge hitch), resync instead of spiralling through catch-up steps.
const MAX_CATCH_UP_SECS: f64 = 0.25;

#[derive(uniffi::Enum, Clone, Copy)]
pub enum TouchPhase {
    Began,
    Moved,
    Ended,
}

/// What `SwiftUI` renders over the game. Perf numbers refresh a few times a second; HP
/// and run state publish the frame they change. `seq` bumps on any change so Swift only
/// touches view state when needed.
#[derive(uniffi::Record, Clone, Default)]
pub struct HudData {
    pub seq: u64,
    pub fps: f64,
    pub frame_ms_avg: f64,
    pub frame_ms_max: f64,
    /// Whole `frame()` call: sim steps, render and drawable acquire.
    pub rust_ms_avg: f64,
    pub tick: u64,
    /// Slot 0's HP.
    pub hp: u8,
    pub max_hp: u8,
    pub run: RunState,
}

/// The run as the HUD needs it.
#[derive(uniffi::Enum, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RunState {
    #[default]
    Playing,
    /// `can_restart` flips once the death pause is over; a tap then restarts.
    Dead {
        can_restart: bool,
    },
    Won,
}

impl RunState {
    const fn of(run: Run) -> Self {
        match run {
            Run::Boarding { .. } | Run::Encounter { .. } => Self::Playing,
            Run::Dead {
                ticks_until_restart,
            } => Self::Dead {
                can_restart: ticks_until_restart == 0,
            },
            Run::Won => Self::Won,
        }
    }
}

#[derive(Debug, uniffi::Error)]
#[uniffi(flat_error)]
pub enum GameError {
    NullLayer,
    Render(render::Error),
}

impl std::fmt::Display for GameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NullLayer => write!(f, "null CAMetalLayer pointer"),
            Self::Render(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for GameError {}

struct Inner {
    renderer: Renderer,
    prev: SimState,
    current: SimState,
    viewport: Viewport,
    controls: Controls,
    /// Display-link time that `current` corresponds to; `None` resyncs on the next frame.
    sim_clock: Option<f64>,
    paused: bool,
    stats: Stats,
}

#[derive(uniffi::Object)]
pub struct Game {
    inner: Mutex<Inner>,
}

impl Game {
    /// Nothing panics while holding the lock, but a poisoned lock is still usable state.
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[uniffi::export]
impl Game {
    /// `layer_ptr` is a `CAMetalLayer*` that Swift keeps alive for the Game's lifetime.
    ///
    /// # Errors
    /// If the pointer is null or wgpu cannot set up rendering on the layer.
    #[uniffi::constructor]
    pub fn new(layer_ptr: u64, viewport: Viewport) -> Result<Arc<Self>, GameError> {
        let addr = usize::try_from(layer_ptr).map_err(|_| GameError::NullLayer)?;
        let layer = NonNull::new(std::ptr::with_exposed_provenance_mut::<c_void>(addr))
            .ok_or(GameError::NullLayer)?;
        // SAFETY: Swift passes its view's CAMetalLayer and keeps it alive while the Game
        // exists (GameUIView owns both).
        #[allow(unsafe_code)]
        let renderer = unsafe {
            Renderer::new(
                layer,
                viewport.pixel_width,
                viewport.pixel_height,
                [viewport.point_width, viewport.point_height],
            )
        }
        .map_err(GameError::Render)?;
        eprintln!("[gm] Game::new {viewport:?}");
        // The session seed arrives with run setup; fixed for now, so every run spawns the
        // same rushers.
        let state = SimState::new(0);
        Ok(Arc::new(Self {
            inner: Mutex::new(Inner {
                renderer,
                prev: state.clone(),
                current: state,
                viewport,
                controls: Controls::new(&viewport),
                sim_clock: None,
                paused: false,
                stats: Stats::default(),
            }),
        }))
    }

    /// Called on every layout pass; only acts when the viewport actually changed, so
    /// held sticks survive unrelated layouts.
    pub fn resize(&self, viewport: Viewport) {
        let mut g = self.lock();
        if g.viewport == viewport {
            return;
        }
        eprintln!("[gm] resize {viewport:?}");
        g.renderer.resize(
            viewport.pixel_width,
            viewport.pixel_height,
            [viewport.point_width, viewport.point_height],
        );
        g.controls.set_viewport(&viewport);
        g.viewport = viewport;
    }

    pub fn set_scheme(&self, scheme: Scheme) {
        eprintln!("[gm] scheme={scheme:?}");
        self.lock().controls.set_scheme(scheme);
    }

    /// Aim assist strength for [`Scheme::AimAssist`], `0..=1`.
    pub fn set_assist_strength(&self, strength: f32) {
        eprintln!("[gm] assist={strength}");
        self.lock().controls.set_assist(strength);
    }

    /// Tap to restart: sends RESTART on the next tick. The sim ignores it unless the run
    /// is over and the death pause has elapsed.
    pub fn restart(&self) {
        self.lock().controls.request_restart();
    }

    /// Touch in view points.
    pub fn touch(&self, id: u64, phase: TouchPhase, x: f32, y: f32) {
        self.lock().controls.touch(id, phase, x, y);
    }

    /// Stop stepping and drawing (app backgrounded). Pause lives outside the sim.
    pub fn pause(&self) {
        self.lock().paused = true;
    }

    /// Resume without replaying the time spent paused.
    pub fn resume(&self) {
        let mut g = self.lock();
        g.paused = false;
        g.sim_clock = None;
    }

    /// Called from `CADisplayLink`. `target_timestamp` is when this frame will be shown.
    // The lock is held for the whole frame by design.
    #[allow(clippy::significant_drop_tightening)]
    pub fn frame(&self, timestamp: f64, target_timestamp: f64) -> HudData {
        let started = Instant::now();
        let mut guard = self.lock();
        let g = &mut *guard;
        if g.paused {
            return g.stats.hud();
        }

        let dt = 1.0 / f64::from(TICK_HZ);
        let mut clock = match g.sim_clock {
            Some(c) if target_timestamp - c <= MAX_CATCH_UP_SECS => c,
            _ => {
                g.prev.clone_from(&g.current);
                target_timestamp
            }
        };
        // Fixed-step accumulator; the float comparison is the point.
        #[allow(clippy::while_float)]
        while clock + dt <= target_timestamp {
            let mut inputs = TickInputs::default();
            inputs.players[0] = g.controls.next_input();
            g.prev.clone_from(&g.current);
            let events = sim::step(&mut g.current, &inputs);
            g.renderer.note_events(g.current.tick, &events.events);
            clock += dt;
        }
        g.sim_clock = Some(clock);

        // No From<f64> for f32; precision loss is fine for an interpolation factor.
        #[allow(clippy::as_conversions, clippy::cast_possible_truncation)]
        let alpha = ((target_timestamp - clock) / dt).clamp(0.0, 1.0) as f32;
        let roll_ready = g.current.players[0].is_none_or(|p| p.can_roll());
        let overlay = g.controls.overlay(roll_ready);
        let presented = g
            .renderer
            .draw(&g.prev, &g.current, alpha, &overlay)
            .is_some();
        g.stats.set_status(
            g.current.players[0].map_or(0, |p| p.hp),
            MAX_HP,
            RunState::of(g.current.run),
        );
        g.stats.record(
            timestamp,
            started.elapsed().as_secs_f64(),
            presented,
            g.current.tick,
        )
    }
}
