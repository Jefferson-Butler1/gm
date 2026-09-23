//! FFI surface for the feel spike. Swift owns the display link + touches and calls `frame()`.

mod render;
mod sim;

use render::{Renderer, CIRCLE, RING, SQUARE};
use sim::{Sim, SimInput, DT, PLAYER_SIZE, V2};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Instant;

uniffi::setup_scaffolding!();

const STICK_RADIUS: f32 = 60.0;
const AIM_DEADZONE: f32 = 0.2;

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
    pub drawable_w: u32,
    pub drawable_h: u32,
}

struct Stick {
    id: u64,
    origin: V2,
    cur: V2,
}

impl Stick {
    /// Floating stick: origin follows the finger once it passes the radius.
    fn drag(&mut self, p: V2) {
        self.cur = p;
        let d = V2::new(p.x - self.origin.x, p.y - self.origin.y);
        let l = d.len();
        if l > STICK_RADIUS {
            let k = (l - STICK_RADIUS) / l;
            self.origin.x += d.x * k;
            self.origin.y += d.y * k;
        }
    }
    fn vec(&self) -> V2 {
        V2::new((self.cur.x - self.origin.x) / STICK_RADIUS, (self.cur.y - self.origin.y) / STICK_RADIUS)
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
        g.hud.drawable_w = width_px;
        g.hud.drawable_h = height_px;
    }

    pub fn set_stress(&self, count: u32) {
        self.inner.lock().unwrap().sim.set_stress(count as usize);
    }

    /// Touch in view points. Left half spawns the move stick, right half the aim stick.
    pub fn touch(&self, id: u64, phase: TouchPhase, x: f32, y: f32) {
        let mut guard = self.inner.lock().unwrap();
        let g = &mut *guard;
        let p = V2::new(x, y);
        match phase {
            TouchPhase::Began => {
                let slot = if x < g.sim.bounds.x * 0.5 { &mut g.left } else { &mut g.right };
                if slot.is_none() {
                    *slot = Some(Stick { id, origin: p, cur: p });
                }
            }
            TouchPhase::Moved => {
                for s in [&mut g.left, &mut g.right].into_iter().flatten() {
                    if s.id == id {
                        s.drag(p);
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
        let input = SimInput {
            move_dir: g.left.as_ref().map(|s| s.vec()).unwrap_or_default(),
            aim_dir: g.right.as_ref().map(|s| s.vec()).filter(|v| v.len() > AIM_DEADZONE),
        };
        let mut steps = 0;
        let t_sim = Instant::now();
        while clock + dt <= target_timestamp {
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
            r.push(p.x, p.y, 3.5, 3.5, [1.0, 0.85, 0.3, 1.0], CIRCLE);
        }
        let p = V2::lerp(g.sim.player.prev, g.sim.player.pos, alpha);
        let h = PLAYER_SIZE * 0.5;
        r.push(p.x, p.y, h, h, [0.3, 0.9, 1.0, 1.0], SQUARE);
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
                    "[spike] frames={} fps={:.1} frame_ms avg={:.2} p99={:.2} max={:.2} rust_ms avg={:.3} p99={:.3} acquire_ms={:.3} sim_step_ms={:.4} steps/s={} sprites={} bullets={} drawable={}x{}",
                    h.frames_total, h.display_fps, h.frame_ms_avg, h.frame_ms_p99, h.frame_ms_max,
                    h.rust_ms_avg, h.rust_ms_p99, h.acquire_ms_avg, h.sim_step_ms_avg, h.sim_steps_per_sec,
                    h.sprites, h.bullets, h.drawable_w, h.drawable_h
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
