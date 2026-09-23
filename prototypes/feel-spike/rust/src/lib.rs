//! FFI surface for the feel spike. Swift owns the display link + touches and calls `frame()`.

mod render;
mod sim;

use render::{Renderer, CIRCLE, RING, SQUARE};
use sim::{Aim, Sim, SimInput, BULLET_R, DT, ENEMY_FLASH_TICKS, ENEMY_R, PLAYER_SIZE, V2};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Instant;

uniffi::setup_scaffolding!();

const STICK_RADIUS: f32 = 60.0;
const AIM_DEADZONE: f32 = 0.2;
const DODGE_BTN_R: f32 = 34.0;
/// Scheme C: a right-half swipe this far within FLICK_SECS rolls in the swipe direction.
const FLICK_DIST: f32 = 45.0;
const FLICK_SECS: f32 = 0.2;

/// Touch control schemes, cycled from the SwiftUI shell.
#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq)]
pub enum Scheme {
    /// A: floating twin sticks, dodge button.
    FloatingSticks,
    /// B: fixed stick bases near the bottom corners, dodge button.
    FixedSticks,
    /// C: floating move stick; hold right half = auto-aim fire, flick right half = dodge.
    AutoAim,
    /// D: floating twin sticks with aim bent toward targets in a cone.
    AimAssist,
}

#[derive(uniffi::Enum, Clone, Copy)]
pub enum TouchPhase {
    Began,
    Moved,
    Ended,
}

/// Stats over the trailing ~1s. Recomputed ~4x/sec; `seq` bumps when it changes.
#[derive(uniffi::Record, Clone, Default)]
pub struct HudSnapshot {
    pub seq: u64,
    pub frames_total: u64,
    pub display_fps: f64,
    pub frame_ms_avg: f64,
    pub frame_ms_p99: f64,
    pub frame_ms_max: f64,
    /// Whole `frame()` call (sim + render + drawable acquire).
    pub rust_ms_avg: f64,
    pub rust_ms_p99: f64,
    /// Time blocked in `get_current_texture` (nextDrawable).
    pub acquire_ms_avg: f64,
    /// Single sim step.
    pub sim_step_ms_avg: f64,
    pub sim_steps_per_sec: u32,
    pub sprites: u32,
    pub bullets: u32,
    pub hits: u32,
    pub shots: u32,
    pub drawable_w: u32,
    pub drawable_h: u32,
}

struct Stick {
    id: u64,
    origin: V2,
    cur: V2,
    /// Fixed sticks keep their base; floating ones drag it along.
    fixed: bool,
    start: V2,
    t0: Instant,
    flicked: bool,
}

impl Stick {
    fn new(id: u64, origin: V2, p: V2, fixed: bool) -> Self {
        Self { id, origin, cur: p, fixed, start: p, t0: Instant::now(), flicked: false }
    }
    /// Floating stick: origin follows the finger once it passes the radius.
    fn drag(&mut self, p: V2) {
        self.cur = p;
        let d = p.sub(self.origin);
        let l = d.len();
        if !self.fixed && l > STICK_RADIUS {
            let k = (l - STICK_RADIUS) / l;
            self.origin.x += d.x * k;
            self.origin.y += d.y * k;
        }
    }
    /// Deflection, magnitude <= 1.
    fn vec(&self) -> V2 {
        let d = self.cur.sub(self.origin);
        let k = 1.0 / d.len().max(STICK_RADIUS);
        V2::new(d.x * k, d.y * k)
    }
}

/// Screen-space layout for the on-screen controls (depends on current bounds).
struct Layout {
    dodge_btn: V2,
    left_base: V2,
    right_base: V2,
}

fn layout(b: V2) -> Layout {
    let inset = (b.x * 0.25).min(150.0);
    let y = b.y - 160.0;
    Layout {
        dodge_btn: V2::new(b.x - 64.0, b.y - 64.0),
        left_base: V2::new(inset, y),
        right_base: V2::new(b.x - inset, y),
    }
}

struct Sample {
    t: f64,
    interval: f64,
    rust: f64,
    acquire: f64,
    steps: u32,
    step_time: f64,
}

struct Inner {
    renderer: Renderer,
    sim: Sim,
    left: Option<Stick>,
    right: Option<Stick>,
    scheme: Scheme,
    assist: f32,
    /// Roll requested by touch, consumed by the next sim step.
    dodge_pending: Option<V2>,
    sim_clock: Option<f64>,
    last_ts: Option<f64>,
    samples: VecDeque<Sample>,
    hud: HudSnapshot,
    last_hud_t: f64,
    last_log_t: f64,
}

#[derive(uniffi::Object)]
pub struct Game {
    inner: Mutex<Inner>,
}

#[uniffi::export]
impl Game {
    /// `layer_ptr` is a CAMetalLayer* that Swift keeps alive for the Game's lifetime.
    #[uniffi::constructor]
    pub fn new(layer_ptr: u64, width_px: u32, height_px: u32, width_pt: f32, height_pt: f32) -> Arc<Self> {
        let renderer = unsafe {
            Renderer::new(layer_ptr as usize as *mut std::ffi::c_void, width_px, height_px, [width_pt, height_pt])
        };
        eprintln!("[spike] Game::new {width_px}x{height_px}px {width_pt}x{height_pt}pt");
        Arc::new(Self {
            inner: Mutex::new(Inner {
                renderer,
                sim: Sim::new(V2::new(width_pt, height_pt)),
                left: None,
                right: None,
                scheme: Scheme::FloatingSticks,
                assist: 0.5,
                dodge_pending: None,
                sim_clock: None,
                last_ts: None,
                samples: VecDeque::new(),
                hud: HudSnapshot { drawable_w: width_px, drawable_h: height_px, ..Default::default() },
                last_hud_t: 0.0,
                last_log_t: 0.0,
            }),
        })
    }

    pub fn resize(&self, width_px: u32, height_px: u32, width_pt: f32, height_pt: f32) {
        let mut g = self.inner.lock().unwrap();
        g.renderer.resize(width_px, height_px, [width_pt, height_pt]);
        g.sim.bounds = V2::new(width_pt, height_pt);
        // Stick origins are in the old layout; drop them (rotation cancels the touches anyway).
        g.left = None;
        g.right = None;
        eprintln!("[spike] resize {width_px}x{height_px}px {width_pt}x{height_pt}pt");
        g.hud.drawable_w = width_px;
        g.hud.drawable_h = height_px;
    }

    pub fn set_stress(&self, count: u32) {
        self.inner.lock().unwrap().sim.set_stress(count as usize);
    }

    pub fn set_scheme(&self, scheme: Scheme) {
        let mut g = self.inner.lock().unwrap();
        g.scheme = scheme;
        g.left = None;
        g.right = None;
        eprintln!("[spike] scheme={scheme:?} assist={}", g.assist);
    }

    /// Aim assist strength for `Scheme::AimAssist`, 0..=1.
    pub fn set_assist(&self, strength: f32) {
        let mut g = self.inner.lock().unwrap();
        g.assist = strength;
        eprintln!("[spike] scheme={:?} assist={strength}", g.scheme);
    }

    /// Touch in view points. Left half = move stick, right half = aim/fire; routing depends on scheme.
    pub fn touch(&self, id: u64, phase: TouchPhase, x: f32, y: f32) {
        let mut guard = self.inner.lock().unwrap();
        let g = &mut *guard;
        let p = V2::new(x, y);
        let l = layout(g.sim.bounds);
        match phase {
            TouchPhase::Began => {
                if g.scheme != Scheme::AutoAim && p.sub(l.dodge_btn).len() < DODGE_BTN_R * 1.3 {
                    g.dodge_pending = Some(V2::default());
                    return;
                }
                let is_left = x < g.sim.bounds.x * 0.5;
                let slot = if is_left { &mut g.left } else { &mut g.right };
                if slot.is_some() {
                    return;
                }
                if g.scheme == Scheme::FixedSticks {
                    // Register only near the drawn base (generous radius).
                    let base = if is_left { l.left_base } else { l.right_base };
                    if p.sub(base).len() < STICK_RADIUS * 2.2 {
                        *slot = Some(Stick::new(id, base, p, true));
                    }
                } else {
                    *slot = Some(Stick::new(id, p, p, false));
                }
            }
            TouchPhase::Moved => {
                for s in [&mut g.left, &mut g.right].into_iter().flatten() {
                    if s.id == id {
                        s.drag(p);
                    }
                }
                if g.scheme == Scheme::AutoAim {
                    if let Some(s) = g.right.as_mut().filter(|s| s.id == id && !s.flicked) {
                        let d = p.sub(s.start);
                        if d.len() > FLICK_DIST && s.t0.elapsed().as_secs_f32() < FLICK_SECS {
                            s.flicked = true;
                            g.dodge_pending = Some(d);
                        }
                    }
                }
            }
            TouchPhase::Ended => {
                for slot in [&mut g.left, &mut g.right] {
                    if slot.as_ref().is_some_and(|s| s.id == id) {
                        *slot = None;
                    }
                }
            }
        }
    }

    /// Called from CADisplayLink. `target_timestamp` is when this frame will be shown.
    pub fn frame(&self, timestamp: f64, target_timestamp: f64) -> HudSnapshot {
        let t_start = Instant::now();
        let mut g = self.inner.lock().unwrap();
        let g = &mut *g;

        // Fixed-step sim toward the display time; render interpolates prev->pos.
        let dt = DT as f64;
        let mut clock = g.sim_clock.unwrap_or(target_timestamp);
        if target_timestamp - clock > 0.25 {
            clock = target_timestamp; // resumed from pause / huge hitch: don't spiral
        }
        let aim_stick = g.right.as_ref().map(|s| s.vec()).filter(|v| v.len() > AIM_DEADZONE);
        let aim = match g.scheme {
            Scheme::FloatingSticks | Scheme::FixedSticks => aim_stick.map(Aim::Stick),
            Scheme::AutoAim => g.right.as_ref().map(|_| Aim::Auto),
            Scheme::AimAssist => aim_stick.map(|v| Aim::Assist(v, g.assist)),
        }
        .unwrap_or_default();
        let mut input = SimInput {
            move_dir: g.left.as_ref().map(|s| s.vec()).unwrap_or_default(),
            aim,
            dodge: None,
        };
        let mut steps = 0;
        let t_sim = Instant::now();
        while clock + dt <= target_timestamp {
            // A pending roll goes to the first step only; if no step runs it waits for the next frame.
            input.dodge = g.dodge_pending.take();
            g.sim.step(input);
            clock += dt;
            steps += 1;
        }
        let step_time = t_sim.elapsed().as_secs_f64();
        g.sim_clock = Some(clock);
        let alpha = ((target_timestamp - clock) / dt).clamp(0.0, 1.0) as f32;

        // Build draw list.
        let r = &mut g.renderer;
        for m in &g.sim.stress {
            let p = V2::lerp(m.prev, m.pos, alpha);
            r.push(p.x, p.y, 3.0, 3.0, [0.45, 0.35, 0.9, 0.9], SQUARE);
        }
        for m in &g.sim.bullets {
            let p = V2::lerp(m.prev, m.pos, alpha);
            r.push(p.x, p.y, BULLET_R, BULLET_R, [1.0, 0.85, 0.3, 1.0], CIRCLE);
        }
        for (i, e) in g.sim.enemies.iter().enumerate() {
            let p = V2::lerp(e.m.prev, e.m.pos, alpha);
            if e.alive() {
                r.push(p.x, p.y, ENEMY_R, ENEMY_R, [0.95, 0.3, 0.3, 1.0], CIRCLE);
                if g.sim.aim_target == Some(i) {
                    let rr = ENEMY_R + 7.0;
                    r.push(p.x, p.y, rr, rr, [1.0, 0.9, 0.3, 0.8], RING);
                }
            } else if e.down_ticks <= ENEMY_FLASH_TICKS {
                // Hit flash: white, growing.
                let rr = ENEMY_R * (1.0 + e.down_ticks as f32 * 0.06);
                r.push(p.x, p.y, rr, rr, [1.0, 1.0, 1.0, 1.0], CIRCLE);
            }
        }
        let p = V2::lerp(g.sim.player.prev, g.sim.player.pos, alpha);
        let rolling = g.sim.roll_ticks > 0;
        // Rolling (i-frames): shrunk + white.
        let h = PLAYER_SIZE * if rolling { 0.3 } else { 0.5 };
        let color = if rolling { [1.0, 1.0, 1.0, 0.9] } else { [0.3, 0.9, 1.0, 1.0] };
        r.push(p.x, p.y, h, h, color, SQUARE);

        let l = layout(g.sim.bounds);
        if g.scheme != Scheme::AutoAim {
            let a = if g.sim.roll_ready() { 0.35 } else { 0.1 };
            r.push(l.dodge_btn.x, l.dodge_btn.y, DODGE_BTN_R, DODGE_BTN_R, [0.3, 0.9, 1.0, a], CIRCLE);
        }
        if g.scheme == Scheme::FixedSticks {
            // Idle bases so the thumbs know where to go; active sticks draw over them below.
            for (base, s) in [(l.left_base, &g.left), (l.right_base, &g.right)] {
                if s.is_none() {
                    r.push(base.x, base.y, STICK_RADIUS, STICK_RADIUS, [1.0, 1.0, 1.0, 0.15], RING);
                    r.push(base.x, base.y, 22.0, 22.0, [1.0, 1.0, 1.0, 0.15], CIRCLE);
                }
            }
        }
        for s in [&g.left, &g.right].into_iter().flatten() {
            r.push(s.origin.x, s.origin.y, STICK_RADIUS, STICK_RADIUS, [1.0, 1.0, 1.0, 0.25], RING);
            let v = s.vec();
            let (kx, ky) = (s.origin.x + v.x * STICK_RADIUS, s.origin.y + v.y * STICK_RADIUS);
            r.push(kx, ky, 22.0, 22.0, [1.0, 1.0, 1.0, 0.35], CIRCLE);
        }
        let acquire = r.draw();

        // Stats.
        g.hud.frames_total += acquire.is_some() as u64;
        let interval = g.last_ts.map(|l| timestamp - l).unwrap_or(0.0);
        g.last_ts = Some(timestamp);
        g.samples.push_back(Sample {
            t: timestamp,
            interval,
            rust: t_start.elapsed().as_secs_f64(),
            acquire: acquire.unwrap_or(0.0),
            steps,
            step_time,
        });
        while g.samples.front().is_some_and(|s| s.t < timestamp - 1.0) {
            g.samples.pop_front();
        }
        if timestamp - g.last_hud_t >= 0.25 {
            g.last_hud_t = timestamp;
            refresh_hud(g);
            if timestamp - g.last_log_t >= 1.0 {
                g.last_log_t = timestamp;
                let h = &g.hud;
                eprintln!(
                    "[spike] frames={} fps={:.1} frame_ms avg={:.2} p99={:.2} max={:.2} rust_ms avg={:.3} p99={:.3} acquire_ms={:.3} sim_step_ms={:.4} steps/s={} sprites={} bullets={} drawable={}x{} scheme={:?} assist={} hits={} shots={}",
                    h.frames_total, h.display_fps, h.frame_ms_avg, h.frame_ms_p99, h.frame_ms_max,
                    h.rust_ms_avg, h.rust_ms_p99, h.acquire_ms_avg, h.sim_step_ms_avg, h.sim_steps_per_sec,
                    h.sprites, h.bullets, h.drawable_w, h.drawable_h, g.scheme, g.assist, h.hits, h.shots
                );
            }
        }
        g.hud.clone()
    }
}

fn refresh_hud(g: &mut Inner) {
    let n = g.samples.len().max(1) as f64;
    // Ignore the first frame / post-pause gaps when looking at pacing.
    let mut intervals: Vec<f64> = g.samples.iter().map(|s| s.interval).filter(|&i| i > 0.0 && i < 0.25).collect();
    let mut rust: Vec<f64> = g.samples.iter().map(|s| s.rust).collect();
    let steps: u32 = g.samples.iter().map(|s| s.steps).sum();
    let step_time: f64 = g.samples.iter().map(|s| s.step_time).sum();
    let h = &mut g.hud;
    h.seq += 1;
    h.display_fps = if intervals.is_empty() { 0.0 } else { intervals.len() as f64 / intervals.iter().sum::<f64>() };
    h.frame_ms_avg = avg(&intervals) * 1e3;
    h.frame_ms_p99 = p99(&mut intervals) * 1e3;
    h.frame_ms_max = intervals.iter().copied().fold(0.0, f64::max) * 1e3;
    h.rust_ms_avg = avg(&rust) * 1e3;
    h.rust_ms_p99 = p99(&mut rust) * 1e3;
    h.acquire_ms_avg = g.samples.iter().map(|s| s.acquire).sum::<f64>() / n * 1e3;
    h.sim_step_ms_avg = if steps == 0 { 0.0 } else { step_time / steps as f64 * 1e3 };
    h.sim_steps_per_sec = steps;
    h.sprites = g.sim.stress.len() as u32;
    h.bullets = g.sim.bullets.len() as u32;
    h.hits = g.sim.hits;
    h.shots = g.sim.shots;
}

fn avg(v: &[f64]) -> f64 {
    if v.is_empty() { 0.0 } else { v.iter().sum::<f64>() / v.len() as f64 }
}

fn p99(v: &mut [f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    v[((v.len() as f64 * 0.99).ceil() as usize).saturating_sub(1).min(v.len() - 1)]
}
