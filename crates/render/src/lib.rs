//! wgpu renderer drawing into a Swift-owned `CAMetalLayer`.
//!
//! Reads the previous and current [`SimState`] and interpolates between them; it never
//! mutates the sim. Hit flashes, muzzle flashes, death puffs and the "!" over an enemy
//! that notices the party come from sim [`Event`]s. Placeholder art is flat colored
//! squares, circles and rings, converted to NDC on the CPU so there are no bind groups.
//! On-screen controls arrive as an [`Overlay`] in view points, since their layout belongs
//! to `game`.

use bytemuck::{Pod, Zeroable};
use sim::room::{Cell, PrototypeRoom};
use sim::{Behavior, Enemy, EnemyId, Event, Fx, FxVec2, Pattern, Player, SimState};
use std::f32::consts::TAU;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::time::Instant;

const CLEAR: wgpu::Color = wgpu::Color {
    r: 0.035,
    g: 0.04,
    b: 0.06,
    a: 1.0,
};
const FLOOR_COLOR: [f32; 4] = [0.08, 0.09, 0.13, 1.0];
const WALL_COLOR: [f32; 4] = [0.22, 0.25, 0.33, 1.0];
/// Darker than the clear color, so pits read as holes in the floor.
const PIT_COLOR: [f32; 4] = [0.0, 0.0, 0.01, 1.0];
const DOOR_OPEN_COLOR: [f32; 4] = [0.1, 0.3, 0.2, 1.0];
const DOOR_SEALED_COLOR: [f32; 4] = [0.95, 0.45, 0.1, 1.0];
const PAD_COLOR: [f32; 4] = [0.3, 1.0, 0.6, 1.0];
const PLAYER_COLOR: [f32; 4] = [0.3, 0.9, 1.0, 1.0];
/// Rolling (i-frames): shrunk and white, so dodge timing reads at a glance.
const ROLLING_COLOR: [f32; 4] = [1.0, 1.0, 1.0, 0.9];
const ROLLING_SCALE: f32 = 0.6;
const DEAD_COLOR: [f32; 4] = [0.35, 0.35, 0.4, 1.0];
const HURT_COLOR: [f32; 4] = [1.0, 0.25, 0.25, 1.0];
/// Post-hit invulnerability blinks the player: half alpha every other `BLINK_TICKS`.
const BLINK_TICKS: u16 = 4;
const RUSHER_COLOR: [f32; 4] = [0.95, 0.35, 0.3, 1.0];
const SHOOTER_COLOR: [f32; 4] = [0.7, 0.4, 1.0, 1.0];
const SPREAD_SHOOTER_COLOR: [f32; 4] = [1.0, 0.35, 0.75, 1.0];
/// A shooter's aim telegraph: a white core swelling to this fraction of its body.
const AIM_CORE: f32 = 0.7;
/// The "!" over an enemy that just noticed the party: a bar over a dot, their centers
/// this far above the body's top edge, in world pt.
const ALERT_COLOR: [f32; 4] = [1.0, 0.85, 0.2, 1.0];
const ALERT_HALF_WIDTH: f32 = 2.5;
const ALERT_BAR_HALF_HEIGHT: f32 = 6.0;
const ALERT_DOT_RISE: f32 = 5.0;
const ALERT_BAR_RISE: f32 = 16.0;
const ENEMY_BULLET_COLOR: [f32; 4] = [1.0, 0.3, 0.85, 1.0];
/// The spawn telegraph's ring starts this many radii beyond the body.
const TELEGRAPH_RING_GROWTH: f32 = 1.5;
const HIT_COLOR: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const BULLET_COLOR: [f32; 4] = [1.0, 0.9, 0.35, 1.0];
const MUZZLE_COLOR: [f32; 4] = [1.0, 0.95, 0.7, 1.0];
/// Muzzle flash: radius, and distance ahead of the player's center (where bullets spawn).
const MUZZLE_R: f32 = 9.0;
const MUZZLE_OFFSET: f32 = 24.0;
/// A death puff's ring grows to this many radii of the body as it fades.
const PUFF_GROWTH: f32 = 1.5;
/// Facing nub: half-size and distance ahead of the player's center, in world units.
const NUB_HALF: f32 = 4.0;
const NUB_OFFSET: f32 = 22.0;
const STICK_KNOB_R: f32 = 22.0;
const DODGE_COLOR: [f32; 3] = [0.3, 0.9, 1.0];
const VENT_COLOR: [f32; 3] = [1.0, 0.75, 0.25];
const BUTTON_READY_ALPHA: f32 = 0.35;
const BUTTON_UNREADY_ALPHA: f32 = 0.1;

const SQUARE: f32 = 0.0;
const CIRCLE: f32 = 1.0;
const RING: f32 = 2.0;

const SHADER: &str = r"
struct Inst {
    @location(0) center: vec2f,
    @location(1) half_size: vec2f,
    @location(2) color: vec4f,
    @location(3) shape: f32,
};
struct VOut {
    @builtin(position) pos: vec4f,
    @location(0) uv: vec2f,
    @location(1) color: vec4f,
    @location(2) shape: f32,
};
@vertex fn vs(@builtin(vertex_index) vi: u32, i: Inst) -> VOut {
    var corners = array<vec2f, 6>(
        vec2f(-1.0, -1.0), vec2f(1.0, -1.0), vec2f(1.0, 1.0),
        vec2f(-1.0, -1.0), vec2f(1.0, 1.0), vec2f(-1.0, 1.0));
    let c = corners[vi];
    var o: VOut;
    o.pos = vec4f(i.center + c * i.half_size, 0.0, 1.0);
    o.uv = c;
    o.color = i.color;
    o.shape = i.shape;
    return o;
}
// shape: 0 = square, 1 = filled circle, 2 = ring.
@fragment fn fs(v: VOut) -> @location(0) vec4f {
    if (v.shape > 0.5) {
        let d = length(v.uv);
        if (d > 1.0) { discard; }
        if (v.shape > 1.5 && d < 0.88) { discard; }
    }
    return v.color;
}
";

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Quad {
    /// NDC.
    center: [f32; 2],
    half: [f32; 2],
    color: [f32; 4],
    shape: f32,
}

/// On-screen controls, in view points (origin top-left).
#[derive(Clone, Copy, Debug, Default)]
pub struct Overlay {
    pub sticks: [Option<StickView>; 2],
    pub dodge: Option<ButtonView>,
    pub vent: Option<ButtonView>,
}

#[derive(Clone, Copy, Debug)]
pub struct StickView {
    pub base: [f32; 2],
    pub radius: f32,
    /// Knob center, already clamped to the stick's travel.
    pub knob: [f32; 2],
    /// Idle fixed sticks draw fainter.
    pub active: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct ButtonView {
    pub center: [f32; 2],
    pub radius: f32,
    /// Dimmed while pressing it would do nothing (dodge: mid-roll; vent: full or
    /// already venting).
    pub ready: bool,
}

#[derive(Debug)]
pub enum Error {
    Surface(wgpu::CreateSurfaceError),
    Adapter(wgpu::RequestAdapterError),
    Device(wgpu::RequestDeviceError),
    UnsupportedSurface,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Surface(e) => write!(f, "create surface: {e}"),
            Self::Adapter(e) => write!(f, "request adapter: {e}"),
            Self::Device(e) => write!(f, "request device: {e}"),
            Self::UnsupportedSurface => write!(f, "surface unsupported by adapter"),
        }
    }
}

impl std::error::Error for Error {}

pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    instances: wgpu::Buffer,
    capacity: usize,
    /// Screen size in points, for points -> NDC.
    size_pt: [f32; 2],
    /// Room-space point at the screen center; set each frame from the room and player.
    camera: [f32; 2],
    quads: Vec<Quad>,
    /// Active event effects and the tick they started.
    flashes: Vec<(Flash, u64)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Flash {
    /// An enemy took a hit.
    Enemy(EnemyId),
    /// An enemy noticed the party: a "!" over it.
    Alert(EnemyId),
    /// A player took a hit.
    Player(usize),
    /// A player's gun fired.
    Muzzle(usize),
    /// An enemy died here.
    Puff(FxVec2),
}

impl Flash {
    /// Lifetime in sim ticks.
    const fn ticks(self) -> u16 {
        match self {
            Self::Enemy(_) | Self::Player(_) => 6,
            Self::Alert(_) => 40,
            Self::Muzzle(_) => 3,
            Self::Puff(_) => 15,
        }
    }
}

impl Renderer {
    /// # Errors
    /// If wgpu cannot create a Metal surface, adapter or device for the layer.
    ///
    /// # Safety
    /// `layer` must be a live `CAMetalLayer` that outlives the renderer.
    // The one unsafe call: wgpu takes the raw CAMetalLayer pointer handed over from Swift.
    #[allow(unsafe_code)]
    pub unsafe fn new(
        layer: NonNull<c_void>,
        width_px: u32,
        height_px: u32,
        size_pt: [f32; 2],
    ) -> Result<Self, Error> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        // SAFETY: the caller guarantees `layer` is a live CAMetalLayer outliving `self`.
        let surface = unsafe {
            instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::CoreAnimationLayer(
                layer.as_ptr(),
            ))
        }
        .map_err(Error::Surface)?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            ..wgpu::RequestAdapterOptions::default()
        }))
        .map_err(Error::Adapter)?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .map_err(Error::Device)?;

        let caps = surface.get_capabilities(&adapter);
        // Prefer a non-sRGB format so hard-coded colors look as written.
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .or_else(|| caps.formats.first().copied())
            .ok_or(Error::UnsupportedSurface)?;
        let mut config = surface
            .get_default_config(&adapter, width_px.max(1), height_px.max(1))
            .ok_or(Error::UnsupportedSurface)?;
        config.format = format;
        config.present_mode = wgpu::PresentMode::Fifo;
        // 2 => maximumDrawableCount 3. With 1 (2 drawables) nextDrawable() blocks ~a vsync
        // per display-link callback, capping 120 Hz phones at 60 (feel spike, issue #6).
        config.desired_maximum_frame_latency = 2;
        config.alpha_mode = wgpu::CompositeAlphaMode::Opaque;
        surface.configure(&device, &config);
        eprintln!(
            "[gm] adapter={:?} format={format:?} present_modes={:?}",
            adapter.get_info().name,
            caps.present_modes
        );

        let pipeline = make_pipeline(&device, format);
        let capacity = 256;
        let instances = make_instance_buffer(&device, capacity);
        Ok(Self {
            surface,
            device,
            queue,
            config,
            pipeline,
            instances,
            capacity,
            size_pt,
            camera: [0.0, 0.0],
            quads: Vec::new(),
            flashes: Vec::new(),
        })
    }

    /// Feeds one sim step's events; `tick` is the state's tick after that step. A re-run
    /// tick (rollback) repeats events, so duplicates are dropped.
    pub fn note_events(&mut self, tick: u64, events: &[Event]) {
        for event in events {
            let flash = match *event {
                Event::EnemyHit { enemy } => Flash::Enemy(enemy),
                Event::PlayerHit { slot } | Event::PlayerFell { slot } => Flash::Player(slot),
                Event::ShotFired { slot } => Flash::Muzzle(slot),
                Event::EnemyKilled { pos, .. } => Flash::Puff(pos),
                Event::EnemyAlerted { enemy } => Flash::Alert(enemy),
                Event::PlayerDied { .. }
                | Event::Restarted
                | Event::RoomEntered { .. }
                | Event::WaveStarted { .. }
                | Event::RoomCleared { .. }
                | Event::Won => continue,
            };
            if !self.flashes.contains(&(flash, tick)) {
                self.flashes.push((flash, tick));
            }
        }
    }

    fn flashing(&self, flash: Flash) -> bool {
        self.flashes.iter().any(|&(f, _)| f == flash)
    }

    pub fn resize(&mut self, width_px: u32, height_px: u32, size_pt: [f32; 2]) {
        self.size_pt = size_pt;
        if width_px == 0 || height_px == 0 {
            return;
        }
        if (self.config.width, self.config.height) != (width_px, height_px) {
            self.config.width = width_px;
            self.config.height = height_px;
            self.surface.configure(&self.device, &self.config);
        }
    }

    /// Draws `prev` -> `current` interpolated by `alpha` in `0..=1`. Returns seconds spent
    /// blocked acquiring the drawable, or `None` if nothing was presented.
    pub fn draw(
        &mut self,
        prev: &SimState,
        current: &SimState,
        alpha: f32,
        overlay: &Overlay,
    ) -> Option<f64> {
        self.flashes
            .retain(|&(flash, tick)| current.tick < tick.saturating_add(u64::from(flash.ticks())));
        self.push_scene(prev, current, alpha);
        self.push_overlay(overlay);

        let t0 = Instant::now();
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                self.quads.clear();
                return None;
            }
            _ => {
                self.quads.clear();
                return None;
            }
        };
        let acquire = t0.elapsed().as_secs_f64();

        if self.quads.len() > self.capacity {
            self.capacity = self.quads.len().next_power_of_two();
            self.instances = make_instance_buffer(&self.device, self.capacity);
        }
        self.queue
            .write_buffer(&self.instances, 0, bytemuck::cast_slice(&self.quads));

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(CLEAR),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_vertex_buffer(0, self.instances.slice(..));
            pass.draw(0..6, 0..u32::try_from(self.quads.len()).unwrap_or(u32::MAX));
        }
        self.queue.submit([enc.finish()]);
        self.queue.present(frame);
        self.quads.clear();
        Some(acquire)
    }

    /// The world through the camera: room tiles, enemies, players, bullets.
    fn push_scene(&mut self, prev: &SimState, current: &SimState, alpha: f32) {
        // Across a room change (or restart into another room) positions jump; don't
        // smear them between two rooms' coordinates.
        let lerp_from = |a: FxVec2, b: FxVec2| {
            if prev.run.room() == current.run.room() {
                a
            } else {
                b
            }
        };
        let players: Vec<_> = prev
            .players
            .iter()
            .zip(&current.players)
            .map(|(a, b)| {
                a.zip(*b).map(|(a, b)| {
                    // A pit respawn is a jump, not a slide.
                    let from = if a.falling() && !b.falling() {
                        b.pos
                    } else {
                        lerp_from(a.pos, b.pos)
                    };
                    (lerp(from, b.pos, alpha), b)
                })
            })
            .collect();
        if let Some(room) = sim::DERELICT.room(current.run.room()) {
            let focus = players.iter().flatten().next().map_or([0.0, 0.0], |p| p.0);
            self.camera = self.camera_for(room, focus);
            let pad_live = !matches!(current.run, sim::Run::Encounter { .. });
            self.push_room(room, current.run.doors_locked(), pad_live);
        }
        let radius = sim::ENEMY_RADIUS.to_num::<f32>();
        let telegraph = current.config.tuning.shooter_telegraph;
        for (id, e) in current.enemies.iter() {
            if !e.active() {
                self.push_telegraph(e, radius, alpha);
                continue;
            }
            let from = prev.enemies.get(id).map_or(e.pos, |p| p.pos);
            let pos = lerp(from, e.pos, alpha);
            let color = if self.flashing(Flash::Enemy(id)) {
                HIT_COLOR
            } else {
                enemy_color(e)
            };
            self.push_world(pos, [radius, radius], color, CIRCLE);
            if self.flashing(Flash::Alert(id)) {
                let [x, top] = [pos[0], pos[1] - radius];
                let dot = [x, top - ALERT_DOT_RISE];
                let bar = [x, top - ALERT_BAR_RISE];
                let half = ALERT_HALF_WIDTH;
                self.push_world(dot, [half, half], ALERT_COLOR, SQUARE);
                self.push_world(bar, [half, ALERT_BAR_HALF_HEIGHT], ALERT_COLOR, SQUARE);
            }
            if let Some(left) = e.aiming(telegraph) {
                // 0 -> 1 over the telegraph, interpolated like `push_telegraph`.
                let aimed =
                    (1.0 - (f32::from(left) - alpha) / f32::from(telegraph)).clamp(0.0, 1.0);
                let core = radius * AIM_CORE * aimed;
                self.push_world(pos, [core, core], HIT_COLOR, CIRCLE);
            }
        }
        let fall_ticks = f32::from(current.config.tuning.fall_ticks.max(1));
        for (slot, player) in players.iter().enumerate() {
            if let Some((pos, p)) = player {
                let hurt = self.flashing(Flash::Player(slot));
                // Fall left, 1 -> 0, interpolated like positions (`fall_ticks` drops 1/tick).
                let fall = p
                    .falling()
                    .then(|| ((f32::from(p.fall_ticks) - alpha) / fall_ticks).clamp(0.0, 1.0));
                self.push_player(*pos, p, hurt, fall);
            }
        }
        let bullet = sim::BULLET_RADIUS.to_num::<f32>();
        for (id, b) in current.bullets.iter() {
            let from = prev.bullets.get(id).map_or(b.pos, |p| p.pos);
            self.push_world(
                lerp(from, b.pos, alpha),
                [bullet, bullet],
                BULLET_COLOR,
                CIRCLE,
            );
        }
        let enemy_bullet = sim::ENEMY_BULLET_RADIUS.to_num::<f32>();
        for (id, b) in current.enemy_bullets.iter() {
            let from = prev.enemy_bullets.get(id).map_or(b.pos, |p| p.pos);
            let pos = lerp(from, b.pos, alpha);
            self.push_world(
                pos,
                [enemy_bullet, enemy_bullet],
                ENEMY_BULLET_COLOR,
                CIRCLE,
            );
            let core = enemy_bullet * 0.45;
            self.push_world(pos, [core, core], HIT_COLOR, CIRCLE);
        }
        self.push_effects(&players, current.tick, alpha);
    }

    /// Muzzle flashes and death puffs, fading over their lifetimes. `players` are the
    /// frame's interpolated positions (as [`Self::push_scene`] draws them) and states.
    fn push_effects(&mut self, players: &[Option<([f32; 2], Player)>], tick: u64, alpha: f32) {
        let rusher = sim::ENEMY_RADIUS.to_num::<f32>();
        let effects = std::mem::take(&mut self.flashes);
        for &(flash, start) in &effects {
            // Life left, 1 -> 0. The event happened during the `prev` -> `current` step.
            let age = u16::try_from(tick.saturating_sub(start)).map_or(f32::MAX, f32::from);
            let left = (1.0 - (age + alpha) / f32::from(flash.ticks())).clamp(0.0, 1.0);
            match flash {
                Flash::Muzzle(slot) => {
                    let Some(&Some(([x, y], player))) = players.get(slot) else {
                        continue;
                    };
                    let (sin, cos) = (f32::from(player.facing) / 65536.0 * TAU).sin_cos();
                    let at = [cos.mul_add(MUZZLE_OFFSET, x), sin.mul_add(MUZZLE_OFFSET, y)];
                    let radius = MUZZLE_R * left.mul_add(0.5, 0.5);
                    self.push_world(at, [radius, radius], MUZZLE_COLOR, CIRCLE);
                }
                Flash::Puff(pos) => {
                    let at = [pos.x.to_num(), pos.y.to_num()];
                    let [r, g, b, _] = RUSHER_COLOR;
                    let ring = rusher * (1.0 - left).mul_add(PUFF_GROWTH, 1.0);
                    self.push_world(at, [ring, ring], [r, g, b, left], RING);
                    let core = rusher * left;
                    self.push_world(at, [core, core], [1.0, 1.0, 1.0, 0.6 * left], CIRCLE);
                }
                Flash::Enemy(_) | Flash::Player(_) | Flash::Alert(_) => {}
            }
        }
        self.flashes = effects;
    }

    /// Spawn warning: a ring closing in on the spot while the body fades in.
    fn push_telegraph(&mut self, e: &Enemy, radius: f32, alpha: f32) {
        // Telegraph left, 1 -> 0, interpolated like positions (`spawn_ticks` drops 1/tick).
        let left = ((f32::from(e.spawn_ticks) + 1.0 - alpha)
            / f32::from(sim::SPAWN_TELEGRAPH_TICKS))
        .clamp(0.0, 1.0);
        let pos = [e.pos.x.to_num(), e.pos.y.to_num()];
        let ring = radius * left.mul_add(TELEGRAPH_RING_GROWTH, 1.0);
        let [r, g, b, _] = enemy_color(e);
        self.push_world(pos, [ring, ring], [r, g, b, 0.9], RING);
        self.push_world(pos, [radius, radius], [r, g, b, 0.3 * (1.0 - left)], CIRCLE);
    }

    /// `fall` is how much of a fall into a pit is left (1 -> 0): the player shrinks and
    /// fades into it. A fatal fall freezes the run, so a dead faller isn't drawn at all.
    fn push_player(&mut self, [x, y]: [f32; 2], p: &Player, hurt: bool, fall: Option<f32>) {
        if fall.is_some() && !p.alive() {
            return;
        }
        let radius = sim::PLAYER_RADIUS.to_num::<f32>();
        let (radius, mut color) = if !p.alive() {
            (radius, DEAD_COLOR)
        } else if p.roll_iframes > 0 {
            // Only the roll's i-frames look like a roll: the landing is vulnerable.
            (radius * ROLLING_SCALE, ROLLING_COLOR)
        } else if hurt {
            (radius, HURT_COLOR)
        } else {
            (radius, PLAYER_COLOR)
        };
        if p.alive() && (p.hurt_ticks / BLINK_TICKS) % 2 == 1 {
            color[3] *= 0.4;
        }
        let shrink = fall.unwrap_or(1.0);
        color[3] *= shrink;
        let radius = radius * shrink;
        self.push_world([x, y], [radius, radius], color, CIRCLE);
        let (sin, cos) = (f32::from(p.facing) / 65536.0 * TAU).sin_cos();
        let (offset, half) = (NUB_OFFSET * shrink, NUB_HALF * shrink);
        let nub = [cos.mul_add(offset, x), sin.mul_add(offset, y)];
        self.push_world(nub, [half, half], color, SQUARE);
    }

    fn push_overlay(&mut self, overlay: &Overlay) {
        for s in overlay.sticks.iter().flatten() {
            let a = if s.active { 1.0 } else { 0.6 };
            self.push_screen(s.base, s.radius, [1.0, 1.0, 1.0, 0.25 * a], RING);
            self.push_screen(s.knob, STICK_KNOB_R, [1.0, 1.0, 1.0, 0.35 * a], CIRCLE);
        }
        let buttons = [(overlay.dodge, DODGE_COLOR), (overlay.vent, VENT_COLOR)];
        for (button, [r, g, b]) in buttons {
            let Some(d) = button else {
                continue;
            };
            let a = if d.ready {
                BUTTON_READY_ALPHA
            } else {
                BUTTON_UNREADY_ALPHA
            };
            self.push_screen(d.center, d.radius, [r, g, b, a], CIRCLE);
        }
    }

    /// Where the screen center sits in room space: on the focus (the player), clamped so
    /// the view stays inside the room. An axis where the room fits on screen centers it.
    fn camera_for(&self, room: &PrototypeRoom, focus: [f32; 2]) -> [f32; 2] {
        let cell = sim::room::CELL.to_num::<f32>();
        let extent = |cells: usize| f32::from(u16::try_from(cells).unwrap_or(u16::MAX)) * cell;
        let axis = |room: f32, screen: f32, focus: f32| {
            if room <= screen {
                room / 2.0
            } else {
                focus.clamp(screen / 2.0, room - screen / 2.0)
            }
        };
        let [w, h] = self.size_pt;
        [
            axis(extent(room.width()), w, focus[0]),
            axis(extent(room.height()), h, focus[1]),
        ]
    }

    /// One quad per non-void cell. Exit gaps draw as doors: open or sealed. The extraction
    /// pad, if any, is dim until `pad_live`.
    fn push_room(&mut self, room: &PrototypeRoom, sealed: bool, pad_live: bool) {
        let half = sim::room::CELL.to_num::<f32>() / 2.0;
        for y in 0..room.height() {
            for x in 0..room.width() {
                let (Ok(cx), Ok(cy)) = (i32::try_from(x), i32::try_from(y)) else {
                    continue;
                };
                // Tiles are inset a hair so the grid reads; pits a bit more, as holes.
                let (color, inset) = match (room.cell(cx, cy), room.exit_at(cx, cy), sealed) {
                    (_, Some(_), true) => (DOOR_SEALED_COLOR, 0.5),
                    (_, Some(_), false) => (DOOR_OPEN_COLOR, 0.5),
                    (Cell::Floor, None, _) => (FLOOR_COLOR, 0.5),
                    (Cell::Wall, None, _) => (WALL_COLOR, 0.5),
                    (Cell::Pit, None, _) => (PIT_COLOR, 3.0),
                    (Cell::Void, None, _) => continue,
                };
                let center = sim::room::cell_center(x, y);
                self.push_world(
                    [center.x.to_num(), center.y.to_num()],
                    [half - inset, half - inset],
                    color,
                    SQUARE,
                );
            }
        }
        if let Some((x, y)) = room.extraction {
            // The ring marks the pad's reach: touching the cell with any part of the body.
            let center = sim::room::cell_center(x, y);
            let at = [center.x.to_num(), center.y.to_num()];
            let [r, g, b, _] = PAD_COLOR;
            let alpha = if pad_live { 1.0 } else { 0.3 };
            self.push_world(at, [half, half], [r, g, b, alpha], SQUARE);
            let reach = half + sim::PLAYER_RADIUS.to_num::<f32>();
            self.push_world(at, [reach, reach], [r, g, b, alpha * 0.8], RING);
        }
    }

    /// A room-space point in view points (origin top-left), through the camera of the
    /// last drawn frame.
    #[must_use]
    pub fn view_point(&self, [x, y]: [f32; 2]) -> [f32; 2] {
        let [w, h] = self.size_pt;
        let [cx, cy] = self.camera;
        [x - cx + w / 2.0, y - cy + h / 2.0]
    }

    /// Room space (points, +y down) through the camera: one world unit is one view point.
    fn push_world(&mut self, [x, y]: [f32; 2], [hx, hy]: [f32; 2], color: [f32; 4], shape: f32) {
        let [w, h] = self.size_pt;
        let [cx, cy] = self.camera;
        let [sx, sy] = [2.0 / w, 2.0 / h];
        self.quads.push(Quad {
            center: [(x - cx) * sx, -(y - cy) * sy],
            half: [hx * sx, hy * sy],
            color,
            shape,
        });
    }

    /// View points, origin top-left.
    fn push_screen(&mut self, [x, y]: [f32; 2], radius: f32, color: [f32; 4], shape: f32) {
        let [w, h] = self.size_pt;
        self.quads.push(Quad {
            center: [(x / w).mul_add(2.0, -1.0), (y / h).mul_add(-2.0, 1.0)],
            half: [radius / w * 2.0, radius / h * 2.0],
            color,
            shape,
        });
    }
}

const fn enemy_color(e: &Enemy) -> [f32; 4] {
    match e.behavior {
        Behavior::Rusher { .. } => RUSHER_COLOR,
        Behavior::Shooter {
            pattern: Pattern::Aimed,
            ..
        } => SHOOTER_COLOR,
        Behavior::Shooter {
            pattern: Pattern::Spread,
            ..
        } => SPREAD_SHOOTER_COLOR,
    }
}

fn lerp(a: FxVec2, b: FxVec2, t: f32) -> [f32; 2] {
    let f = |v: Fx| v.to_num::<f32>();
    [
        (f(b.x) - f(a.x)).mul_add(t, f(a.x)),
        (f(b.y) - f(a.y)).mul_add(t, f(a.y)),
    ]
}

fn make_instance_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    let bytes = capacity.saturating_mul(size_of::<Quad>());
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("instances"),
        size: u64::try_from(bytes).unwrap_or(u64::MAX),
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn make_pipeline(device: &wgpu::Device, format: wgpu::TextureFormat) -> wgpu::RenderPipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("quad"),
        source: wgpu::ShaderSource::Wgsl(SHADER.into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[],
        immediate_size: 0,
    });
    let attrs =
        wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4, 3 => Float32];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("quad"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: u64::try_from(size_of::<Quad>()).unwrap_or(u64::MAX),
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &attrs,
            })],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

#[cfg(test)]
mod tests {
    /// Host (macOS Metal) check that the WGSL and pipeline validate; the device is
    /// otherwise a black box.
    #[test]
    fn pipeline_validates() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .unwrap();
        let (device, _queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        super::make_pipeline(&device, wgpu::TextureFormat::Bgra8Unorm);
        assert!(pollster::block_on(scope.pop()).is_none());
    }
}
