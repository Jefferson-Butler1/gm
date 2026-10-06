//! The game loop and the `UniFFI` surface Swift talks to.
//!
//! Swift owns the lifecycle, the `CADisplayLink` and touch. Each display-link callback
//! calls [`Game::frame`], which steps the sim at a fixed [`TICK_HZ`] toward the display
//! time, renders `prev` -> `current` interpolated, and returns [`HudData`] for `SwiftUI`.

mod audio;
mod camera;
mod controls;
mod haptics;
mod layout;
mod settings;
mod stats;

use audio::{Audio, Mood, Sound};
use camera::{CameraLook, CameraSettings};
use controls::{Controls, FireMode, GamepadState, Scheme, Viewport};
use haptics::{Haptic, Haptics};
use layout::ControlLayout;
use render::Renderer;
use settings::RunSettings;
use sim::{Run, SimState, TICK_HZ, TickInputs};
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
    /// The room the party is in.
    pub room: String,
    /// The room's fight: wave `wave` of `waves`, 1-based. `wave` is 0 while no fight is
    /// on; `waves` is 0 in a room without enemies.
    pub wave: u8,
    pub waves: u8,
    /// A line telling the player what to do next; empty for none.
    pub hint: String,
    /// Slot 0's phase pistol: charges left of `max_charges` (for pips).
    pub charges: u8,
    pub max_charges: u8,
    /// Venting: can't fire until every charge is back. `vent_progress` runs 0 -> 1 over
    /// the vent (0 when not venting).
    pub venting: bool,
    pub vent_progress: f32,
    /// This frame's sounds, each at most once. Per frame: play them whether or not `seq`
    /// changed.
    pub sounds: Vec<Sound>,
    /// The music to play. Per frame, like `sounds`.
    pub mood: Mood,
    /// This frame's haptics, strongest first. Per frame: play them whether or not `seq`
    /// changed.
    pub haptics: Vec<Haptic>,
}

/// The run as the HUD needs it.
#[derive(uniffi::Enum, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RunState {
    #[default]
    Playing,
    /// `can_restart` flips once the death pause is over; a tap then restarts.
    Dead { can_restart: bool },
    /// Escaped through an airlock: the derelict is cleared. A tap starts a new run.
    Won,
}

impl RunState {
    const fn of(run: Run) -> Self {
        match run {
            Run::Boarding | Run::Encounter { .. } => Self::Playing,
            Run::Dead {
                ticks_until_restart,
                ..
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
    look: CameraLook,
    /// Display-link time that `current` corresponds to; `None` resyncs on the next frame.
    sim_clock: Option<f64>,
    paused: bool,
    stats: Stats,
    audio: Audio,
    haptics: Haptics,
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
    /// `seed` seeds the session's first run (Swift picks it at random); restarts derive
    /// the next run's seed inside the sim. `settings` configure the first run.
    ///
    /// # Errors
    /// If the pointer is null or wgpu cannot set up rendering on the layer.
    #[uniffi::constructor]
    pub fn new(
        layer_ptr: u64,
        viewport: Viewport,
        seed: u64,
        settings: RunSettings,
    ) -> Result<Arc<Self>, GameError> {
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
        eprintln!("[gm] Game::new {viewport:?} seed={seed:#018x}");
        let state = SimState::new(seed, settings.run_config());
        Ok(Arc::new(Self {
            inner: Mutex::new(Inner {
                renderer,
                prev: state.clone(),
                current: state,
                viewport,
                controls: Controls::new(&viewport),
                look: CameraLook::new(),
                sim_clock: None,
                paused: false,
                stats: Stats::default(),
                audio: Audio::default(),
                haptics: Haptics::default(),
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

    pub fn set_fire_mode(&self, mode: FireMode) {
        eprintln!("[gm] fire_mode={mode:?}");
        self.lock().controls.set_fire_mode(mode);
    }

    pub fn set_scheme(&self, scheme: Scheme) {
        eprintln!("[gm] scheme={scheme:?}");
        self.lock().controls.set_scheme(scheme);
    }

    /// Where the touch controls sit, from the layout editor; `None` for the scheme's
    /// default. Applies live (the editor previews every drag, so no log here); Swift
    /// sends it after each scheme change.
    pub fn set_layout(&self, layout: Option<ControlLayout>) {
        self.lock().controls.set_layout(layout);
    }

    /// The connected game controller, polled by Swift every frame; `None` without one.
    /// While connected it replaces touch.
    pub fn set_gamepad(&self, pad: Option<GamepadState>) {
        self.lock().controls.set_gamepad(pad);
    }

    /// Aim assist strength for [`Scheme::AimAssist`], `0..=1`.
    pub fn set_assist_strength(&self, strength: f32) {
        eprintln!("[gm] assist={strength}");
        self.lock().controls.set_assist(strength);
    }

    /// Difficulty and tunables, live: they apply from the next tick and carry into
    /// restarts.
    pub fn set_run_settings(&self, settings: RunSettings) {
        eprintln!("[gm] run settings: {settings:?}");
        self.lock().current.set_config(settings.run_config());
    }

    /// Tilt peek: gravity in screen axes (+x right, +y down), in g, polled each frame;
    /// `None` while the peek is off.
    pub fn set_tilt(&self, gravity: Option<Vec<f32>>) {
        let gravity = gravity.and_then(|g| Some([*g.first()?, *g.get(1)?]));
        self.lock().look.set_tilt(gravity);
    }

    pub fn set_camera(&self, settings: CameraSettings) {
        eprintln!("[gm] camera: {settings:?}");
        self.lock().look.set(settings);
    }

    /// Restart: sends RESTART on the next tick. A live run restarts at once; after a
    /// death the sim ignores it until the death pause has elapsed.
    pub fn restart(&self) {
        self.lock().controls.request_restart();
    }

    /// Touch in view points.
    pub fn touch(&self, id: u64, phase: TouchPhase, x: f32, y: f32) {
        self.lock().controls.touch(id, phase, x, y);
    }

    /// Stop stepping the sim (settings open, app inactive); frames keep drawing the frozen
    /// state. Pause lives outside the sim. Held sticks and a pending dodge are dropped:
    /// their touches may never report an end.
    pub fn pause(&self) {
        let mut g = self.lock();
        g.paused = true;
        g.controls.release();
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

        let dt = 1.0 / f64::from(TICK_HZ);
        let alpha = if g.paused {
            // Frozen on `current`; `resume` resyncs the clock so nothing fast-forwards.
            g.prev.clone_from(&g.current);
            1.0
        } else {
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
                g.audio.note(&g.prev, &g.current, &events.events);
                g.haptics.note(&g.prev, &g.current, &events.events);
                clock += dt;
            }
            g.sim_clock = Some(clock);
            // No From<f64> for f32; precision loss is fine for an interpolation factor.
            #[allow(clippy::as_conversions, clippy::cast_possible_truncation)]
            let alpha = ((target_timestamp - clock) / dt).clamp(0.0, 1.0) as f32;
            alpha
        };
        let player = g.current.players[0];
        let roll_ready = player.is_none_or(|p| p.can_roll());
        let max_charges = g.current.config.tuning.charges;
        let vent_ready = player.is_some_and(|p| !p.gun.venting() && p.gun.charges < max_charges);
        let overlay = g.controls.overlay(roll_ready, vent_ready);
        // The nearest enemy in slot 0's room, as an offset from it: what auto-aim shoots.
        let enemy = player.and_then(|p| {
            let ship = &g.current.ship;
            let room = ship.room_at(p.pos)?;
            g.current
                .enemies
                .iter()
                .map(|(_, e)| e)
                .filter(|e| e.active() && ship.room_at(e.pos) == Some(room))
                .map(|e| {
                    let (ex, ey) = (e.pos.x.to_num::<f32>(), e.pos.y.to_num::<f32>());
                    [ex - p.pos.x.to_num::<f32>(), ey - p.pos.y.to_num::<f32>()]
                })
                .min_by(|a, b| a[0].hypot(a[1]).total_cmp(&b[0].hypot(b[1])))
        });
        let look = g.look.update(
            target_timestamp,
            player.map(|p| p.facing),
            g.controls.aim_push(),
            enemy,
        );
        g.renderer.set_look(look);
        let presented = g
            .renderer
            .draw(&g.prev, &g.current, alpha, &overlay)
            .is_some();
        // Tap-to-fire aims from where the player was just drawn.
        let player_view =
            player.map(|p| g.renderer.view_point([p.pos.x.to_num(), p.pos.y.to_num()]));
        g.controls.set_player_view(player_view);
        g.stats.set_status(&g.current);
        HudData {
            sounds: g.audio.take(),
            mood: Mood::of(g.current.run),
            haptics: g.haptics.take(timestamp),
            ..g.stats.record(
                timestamp,
                started.elapsed().as_secs_f64(),
                presented,
                g.current.tick,
            )
        }
    }
}
