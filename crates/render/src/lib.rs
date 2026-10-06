//! wgpu renderer drawing into a Swift-owned `CAMetalLayer`.
//!
//! Reads the previous and current [`SimState`] and interpolates between them; it never
//! mutates the sim. The whole floor is drawn, but only the rooms the party has revealed
//! (ETG fog); the camera follows the player. Hit flashes, muzzle flashes, death puffs and the "!" over an enemy
//! that notices the party come from sim [`Event`]s. Placeholder art is flat colored
//! squares, circles, rings, carets and sectors (enemy sight cones), converted to NDC on
//! the CPU so there are no bind groups.
//! On-screen controls arrive as an [`Overlay`] in view points, since their layout belongs
//! to `game`, as does where the [`minimap`] sits.

pub mod minimap;

use bytemuck::{Pod, Zeroable};
use sim::room::{Cell, Dir};
use sim::ship::Spot;
use sim::{
    Behavior, Enemy, EnemyId, Event, Fx, FxVec2, HatchKind, HatchState, Pattern, PickupKind,
    Player, RoomId, SimState,
};
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
/// Brick lip along pit edges: the edge course, then the staggered inner course.
const PIT_LIP_COLOR: [f32; 4] = [0.42, 0.33, 0.27, 1.0];
const PIT_LIP_DARK_COLOR: [f32; 4] = [0.31, 0.24, 0.2, 1.0];
/// Each lip course is this fraction of a cell deep, two courses in all.
const PIT_LIP_COURSE: f32 = 0.125;
/// Hatches by state: closed reads as a door in the wall, open as floor with a green
/// tint, sealed as a warning.
const HATCH_CLOSED_COLOR: [f32; 4] = [0.35, 0.42, 0.55, 1.0];
const HATCH_OPEN_COLOR: [f32; 4] = [0.1, 0.3, 0.2, 1.0];
const HATCH_SEALED_COLOR: [f32; 4] = [0.95, 0.45, 0.1, 1.0];
/// An airlock's outer hatch: red while locked; once the bridge falls, green, its alpha
/// pulsing over [`AIRLOCK_PULSE_TICKS`] (the way out).
const AIRLOCK_LOCKED_COLOR: [f32; 4] = [0.75, 0.1, 0.12, 1.0];
const AIRLOCK_OPEN_COLOR: [f32; 4] = [0.3, 1.0, 0.6, 1.0];
const AIRLOCK_PULSE_TICKS: u16 = 60;
/// An access panel is wall with this faint seam through it: findable if you look.
const PANEL_SEAM_COLOR: [f32; 4] = [0.17, 0.19, 0.26, 1.0];
/// The placeholder chest: a gold box with a dark lid line; opened, dull brown.
const CHEST_COLOR: [f32; 4] = [0.9, 0.65, 0.2, 1.0];
const CHEST_LID_COLOR: [f32; 4] = [0.45, 0.3, 0.1, 1.0];
const CHEST_OPEN_COLOR: [f32; 4] = [0.35, 0.25, 0.12, 1.0];
/// Its half-size, in world pt.
const CHEST_HALF: [f32; 2] = [11.0, 8.0];
/// Scrap pickups: small squares, copper for 1s and a bigger pale blue for 5s, each glinting
/// (a white core) for `SCRAP_GLINT_TICKS` of every `SCRAP_GLINT_PERIOD`, staggered by
/// slot so a pile twinkles rather than blinks.
const SCRAP_ONE_COLOR: [f32; 4] = [0.85, 0.5, 0.25, 1.0];
const SCRAP_FIVE_COLOR: [f32; 4] = [0.6, 0.85, 1.0, 1.0];
const SCRAP_ONE_HALF: f32 = 3.0;
const SCRAP_FIVE_HALF: f32 = 4.5;
const SCRAP_GLINT_PERIOD: u64 = 45;
const SCRAP_GLINT_TICKS: u64 = 5;
const SCRAP_GLINT_STAGGER: u64 = 17;
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
/// The bridge captain: gold, and drawn this much bigger than its hitbox (a placeholder
/// elite that should read as the boss at a glance).
const CAPTAIN_COLOR: [f32; 4] = [1.0, 0.8, 0.2, 1.0];
const CAPTAIN_SCALE: f32 = 1.5;
/// A shooter's aim telegraph: a white core swelling to this fraction of its body.
const AIM_CORE: f32 = 0.7;
/// The "!" over an enemy that just noticed the party: a bar over a dot, their centers
/// this far above the body's top edge, in world pt.
const ALERT_COLOR: [f32; 4] = [1.0, 0.85, 0.2, 1.0];
const ALERT_HALF_WIDTH: f32 = 2.5;
const ALERT_BAR_HALF_HEIGHT: f32 = 6.0;
const ALERT_DOT_RISE: f32 = 5.0;
const ALERT_BAR_RISE: f32 = 16.0;
/// The "?" over an enemy that heard a noise: dot, stem, the hook's right side and its
/// cap, each (x offset, rise above the body, half width, half height).
const QUERY_COLOR: [f32; 4] = [0.7, 0.85, 1.0, 1.0];
const QUERY_PARTS: [[f32; 4]; 4] = [
    [0.0, 5.0, 2.5, 2.5],
    [0.0, 13.0, 2.5, 2.5],
    [3.5, 19.0, 2.5, 4.0],
    [1.0, 23.5, 5.0, 2.5],
];
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
const DODGE_COLOR: [f32; 3] = [0.3, 0.9, 1.0];
const VENT_COLOR: [f32; 3] = [1.0, 0.75, 0.25];
/// EMPs: the button, the thrown one, its blast and stunned enemies' sparks.
const EMP_COLOR: [f32; 3] = [0.55, 0.7, 1.0];
/// A thrown EMP's drawn radius.
const EMP_R: f32 = 7.0;
/// A stunned enemy's spark ring blinks on for this many ticks in every
/// [`STUN_BLINK_PERIOD`].
const STUN_BLINK_TICKS: u64 = 4;
const STUN_BLINK_PERIOD: u64 = 8;
const FIRE_COLOR: [f32; 3] = [1.0, 0.35, 0.3];
const BUTTON_READY_ALPHA: f32 = 0.35;
const BUTTON_UNREADY_ALPHA: f32 = 0.1;

/// With no enemy on screen, a caret at the screen edge points to the nearest one (red),
/// or with none left and the minimap hidden, to the nearest unlocked airlock (green): its
/// half-size, and its
/// inset from the edge, in view points.
const CARET_COLOR: [f32; 4] = [1.0, 0.2, 0.2, 0.9];
const CARET_HALF: f32 = 9.0;
const CARET_MARGIN: f32 = 16.0;

const SQUARE: f32 = 0.0;
const CIRCLE: f32 = 1.0;
const RING: f32 = 2.0;
/// A triangle pointing along the quad's `dir`.
const CARET: f32 = 3.0;
/// A wedge of the disc centered on the quad's `dir`, `param` the cosine of its half-angle
/// (-1 = the whole disc), fading toward the rim.
const SECTOR: f32 = 4.0;

/// An enemy's sight cone, faint so it reads as a hint rather than a wall of color: drawn
/// this far out (sight itself has no range), dim while unaware, red once hunting.
const CONE_RADIUS: f32 = 112.0;
const CONE_UNAWARE_COLOR: [f32; 4] = [1.0, 0.95, 0.7, 0.1];
const CONE_ALERT_COLOR: [f32; 4] = [1.0, 0.3, 0.25, 0.16];

const SHADER: &str = r"
struct Inst {
    @location(0) center: vec2f,
    @location(1) half_size: vec2f,
    @location(2) color: vec4f,
    @location(3) shape: f32,
    @location(4) dir: vec2f,
    @location(5) param: f32,
};
struct VOut {
    @builtin(position) pos: vec4f,
    @location(0) uv: vec2f,
    @location(1) color: vec4f,
    @location(2) shape: f32,
    @location(3) dir: vec2f,
    @location(4) param: f32,
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
    o.dir = i.dir;
    o.param = i.param;
    return o;
}
// shape: 0 = square, 1 = filled circle, 2 = ring, 3 = caret, 4 = sector.
@fragment fn fs(v: VOut) -> @location(0) vec4f {
    if (v.shape > 3.5) {
        // Within the half-angle of dir: cos(angle to uv) >= param, without normalizing.
        let d = length(v.uv);
        if (d > 1.0 || dot(v.uv, v.dir) < v.param * d) { discard; }
        return vec4f(v.color.rgb, v.color.a * (1.0 - d * d));
    } else if (v.shape > 2.5) {
        // In the caret's frame (x along dir): a triangle inscribed in the unit circle,
        // tip at (1, 0), base at x = -0.5.
        let p = vec2f(dot(v.uv, v.dir), dot(v.uv, vec2f(-v.dir.y, v.dir.x)));
        if (p.x < -0.5 || abs(p.y) > (1.0 - p.x) * 0.57735) { discard; }
    } else if (v.shape > 0.5) {
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
    /// Unit direction a caret points or a sector faces, NDC-oriented (+y up); ignored by
    /// other shapes. True to angle only on a quad that is square in points.
    dir: [f32; 2],
    /// A sector's cosine of its half-angle; ignored by other shapes.
    param: f32,
}

/// On-screen controls, in view points (origin top-left).
#[derive(Clone, Copy, Debug, Default)]
pub struct Overlay {
    pub sticks: [Option<StickView>; 2],
    pub dodge: Option<ButtonView>,
    pub vent: Option<ButtonView>,
    /// A separate fire button (the claw grip's index finger).
    pub fire: Option<ButtonView>,
    pub emp: Option<ButtonView>,
}

#[derive(Clone, Copy, Debug)]
pub struct StickView {
    pub base: [f32; 2],
    pub radius: f32,
    /// Knob center, already clamped to the stick's travel.
    pub knob: [f32; 2],
    pub knob_radius: f32,
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
    /// Floor-space point at the screen center; set each frame from the floor and player.
    camera: [f32; 2],
    /// Where the camera leans off the player, in points (the host's aim look).
    look: [f32; 2],
    quads: Vec<Quad>,
    /// Where the minimap goes; `None` hides it.
    minimap: Option<minimap::Bounds>,
    /// Active event effects and the tick they started.
    flashes: Vec<(Flash, u64)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Flash {
    /// An enemy took a hit.
    Enemy(EnemyId),
    /// An enemy noticed the party: a "!" over it.
    Alert(EnemyId),
    /// An enemy heard a noise and went to look: a "?" over it.
    Query(EnemyId),
    /// A player took a hit.
    Player(usize),
    /// A player's gun fired.
    Muzzle(usize),
    /// An enemy died here.
    Puff(FxVec2),
    /// An EMP went off here.
    Emp(FxVec2),
}

impl Flash {
    /// Lifetime in sim ticks.
    const fn ticks(self) -> u16 {
        match self {
            Self::Enemy(_) | Self::Player(_) => 6,
            Self::Alert(_) | Self::Query(_) => 40,
            Self::Muzzle(_) => 3,
            Self::Puff(_) => 15,
            Self::Emp(_) => 24,
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
            look: [0.0, 0.0],
            quads: Vec::new(),
            minimap: None,
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
                Event::EmpDetonated { pos } => Flash::Emp(pos),
                Event::EnemyAlerted { enemy } => Flash::Alert(enemy),
                Event::EnemyInvestigating { enemy } => Flash::Query(enemy),
                Event::PlayerDied { .. }
                | Event::EnemyFired { .. }
                | Event::Restarted
                | Event::HatchOpened { .. }
                | Event::WaveStarted { .. }
                | Event::RoomCleared { .. }
                | Event::ChestOpened { .. }
                | Event::ScrapCollected { .. }
                | Event::EmpThrown { .. }
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

    /// Leans the camera `offset` points off the player (still clamped to the floor).
    pub const fn set_look(&mut self, offset: [f32; 2]) {
        self.look = offset;
    }

    /// Places the minimap; `None` hides it.
    pub const fn set_minimap(&mut self, bounds: Option<minimap::Bounds>) {
        self.minimap = bounds;
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
        self.push_minimap(current, alpha);
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

    /// The world through the camera: the revealed floor, Scrap, enemies, players, bullets.
    fn push_scene(&mut self, prev: &SimState, current: &SimState, alpha: f32) {
        // Across a restart (a new run seed) positions jump back to the start; don't smear
        // them across the floor.
        let lerp_from = |a: FxVec2, b: FxVec2| {
            if prev.seed == current.seed { a } else { b }
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
        // The screen center sits on the player plus the look, never clamped to the floor:
        // the host keeps the player inside its view box, even by the hull's edge.
        self.camera = players.iter().flatten().next().map_or([0.0, 0.0], |p| {
            [p.0[0] + self.look[0], p.0[1] + self.look[1]]
        });
        self.push_floor(current, alpha);
        self.push_pickups(prev, current, alpha);
        let telegraph = current.config.tuning.shooter_telegraph;
        let cos_half = f32::from(current.config.tuning.sight_half_angle)
            .to_radians()
            .cos();
        // Where each enemy is drawn, spawns telegraphing in included.
        let mut enemies = Vec::with_capacity(current.enemies.len());
        for (id, e) in current.enemies.iter() {
            let radius = drawn_radius(e);
            if !e.active() {
                self.push_telegraph(e, radius, alpha);
                enemies.push([e.pos.x.to_num(), e.pos.y.to_num()]);
                continue;
            }
            let from = prev.enemies.get(id).map_or(e.pos, |p| p.pos);
            let pos = lerp(from, e.pos, alpha);
            enemies.push(pos);
            let facing = prev.enemies.get(id).map_or(e.facing, |p| p.facing);
            self.push_cone(pos, e, facing, cos_half, alpha);
            let color = if self.flashing(Flash::Enemy(id)) {
                HIT_COLOR
            } else {
                enemy_color(e)
            };
            self.push_world(pos, [radius, radius], color, CIRCLE);
            self.push_stun(pos, radius, e, current.tick);
            self.push_mark(id, [pos[0], pos[1] - radius]);
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
        self.push_emps(prev, current, alpha);
        self.push_effects(&players, current.tick, alpha);
        if let Some((focus, _)) = players.iter().flatten().next() {
            // The way out is the minimap's glow; the caret stands in only with it hidden.
            let (targets, color) = if enemies.is_empty() && self.minimap.is_none() {
                (unlocked_airlocks(current), AIRLOCK_OPEN_COLOR)
            } else if enemies.is_empty() {
                (Vec::new(), AIRLOCK_OPEN_COLOR)
            } else {
                (enemies, CARET_COLOR)
            };
            self.push_caret(*focus, &targets, color);
        }
    }

    /// With `targets` about but none on screen, a caret on the screen edge points to the
    /// nearest one: where the line from the player (`focus`) to it leaves the screen,
    /// inset by [`CARET_MARGIN`]. `targets` are floor-space positions, as drawn.
    fn push_caret(&mut self, focus: [f32; 2], targets: &[[f32; 2]], color: [f32; 4]) {
        let [w, h] = self.size_pt;
        let on_screen = |[x, y]: [f32; 2]| (0.0..=w).contains(&x) && (0.0..=h).contains(&y);
        if targets.iter().any(|&e| on_screen(self.view_point(e))) {
            return;
        }
        let offset = |[x, y]: [f32; 2]| [x - focus[0], y - focus[1]];
        let dist_sq = |e: [f32; 2]| {
            let [dx, dy] = offset(e);
            dx.mul_add(dx, dy * dy)
        };
        let Some(&nearest) = targets
            .iter()
            .min_by(|a, b| dist_sq(**a).total_cmp(&dist_sq(**b)))
        else {
            return;
        };
        let [dx, dy] = offset(nearest);
        let len = dx.hypot(dy);
        if len <= f32::EPSILON {
            return;
        }
        let [px, py] = self.view_point(focus);
        // How far along (dx, dy) to the inset edge on one axis; the nearer axis wins.
        let reach = |d: f32, p: f32, size: f32| {
            if d > 0.0 {
                (size - CARET_MARGIN - p) / d
            } else if d < 0.0 {
                (CARET_MARGIN - p) / d
            } else {
                f32::INFINITY
            }
        };
        let t = reach(dx, px, w).min(reach(dy, py, h)).max(0.0);
        let at = [dx.mul_add(t, px), dy.mul_add(t, py)];
        self.push_screen(at, CARET_HALF, color, CARET);
        if let Some(caret) = self.quads.last_mut() {
            // View points are +y down; the shader's frame is +y up.
            caret.dir = [dx / len, -dy / len];
        }
    }

    /// Muzzle flashes, death puffs and EMP blasts, fading over their lifetimes. `players` are the
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
                Flash::Emp(pos) => {
                    // A ring out to the blast's edge, and a fading flash inside it.
                    let at = [pos.x.to_num(), pos.y.to_num()];
                    let [r, g, b] = EMP_COLOR;
                    let blast = sim::BLAST_RADIUS.to_num::<f32>();
                    let ring = blast * (1.0 - left).mul_add(0.7, 0.3);
                    self.push_world(at, [ring, ring], [r, g, b, left], RING);
                    self.push_world(at, [blast, blast], [r, g, b, 0.3 * left], CIRCLE);
                }
                Flash::Enemy(_) | Flash::Player(_) | Flash::Alert(_) | Flash::Query(_) => {}
            }
        }
        self.flashes = effects;
    }

    /// Enemy `id`'s "!" (just noticed the party) or "?" (just heard a noise), if either is
    /// showing, over `[x, top]`, the top of its body.
    fn push_mark(&mut self, id: EnemyId, [x, top]: [f32; 2]) {
        if self.flashing(Flash::Alert(id)) {
            let dot = [x, top - ALERT_DOT_RISE];
            let bar = [x, top - ALERT_BAR_RISE];
            let half = ALERT_HALF_WIDTH;
            self.push_world(dot, [half, half], ALERT_COLOR, SQUARE);
            self.push_world(bar, [half, ALERT_BAR_HALF_HEIGHT], ALERT_COLOR, SQUARE);
        } else if self.flashing(Flash::Query(id)) {
            for [dx, rise, hw, hh] in QUERY_PARTS {
                self.push_world([x + dx, top - rise], [hw, hh], QUERY_COLOR, SQUARE);
            }
        }
    }

    /// Spawn warning: a ring closing in on the spot while the body fades in.
    fn push_telegraph(&mut self, e: &Enemy, radius: f32, alpha: f32) {
        // Telegraph left, 1 -> 0, interpolated like positions (`spawn_ticks` drops 1/tick).
        let left = ((f32::from(e.spawn_ticks) + 1.0 - alpha)
            / f32::from(e.arrival.telegraph_ticks().max(1)))
        .clamp(0.0, 1.0);
        let pos = [e.pos.x.to_num(), e.pos.y.to_num()];
        let ring = radius * left.mul_add(TELEGRAPH_RING_GROWTH, 1.0);
        let [r, g, b, _] = enemy_color(e);
        self.push_world(pos, [ring, ring], [r, g, b, 0.9], RING);
        self.push_world(pos, [radius, radius], [r, g, b, 0.3 * (1.0 - left)], CIRCLE);
    }

    /// `e`'s sight cone at `pos`, turning from `prev_facing` by `alpha`. `cos_half` is the
    /// cosine of the run's `sight_half_angle`.
    fn push_cone(&mut self, pos: [f32; 2], e: &Enemy, prev_facing: u16, cos_half: f32, alpha: f32) {
        let turn = f32::from(sim::trig::angle_diff(prev_facing, e.facing));
        let turns = turn.mul_add(alpha, f32::from(prev_facing)) / 65536.0;
        let (sin, cos) = (turns * TAU).sin_cos();
        // Investigating isn't hunting: its cone stays the unaware color.
        let color = if e.hunting() {
            CONE_ALERT_COLOR
        } else {
            CONE_UNAWARE_COLOR
        };
        self.push_world(pos, [CONE_RADIUS, CONE_RADIUS], color, SECTOR);
        if let Some(cone) = self.quads.last_mut() {
            // Floor space is +y down; the shader's frame is +y up.
            cone.dir = [cos, -sin];
            cone.param = cos_half;
        }
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
            self.push_screen(s.knob, s.knob_radius, [1.0, 1.0, 1.0, 0.35 * a], CIRCLE);
        }
        let buttons = [
            (overlay.dodge, DODGE_COLOR),
            (overlay.vent, VENT_COLOR),
            (overlay.fire, FIRE_COLOR),
            (overlay.emp, EMP_COLOR),
        ];
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

    /// Every revealed room and corridor, the hatches in their walls by state (an airlock's
    /// outer hatch red while locked, then pulsing green over `alpha` of the tick, so the
    /// pulse runs smooth; an access panel as wall), and their chests. Unrevealed rooms stay
    /// black.
    fn push_floor(&mut self, state: &SimState, alpha: f32) {
        let ship = &state.ship;
        self.push_cells(state);
        let half = sim::room::CELL.to_num::<f32>() / 2.0;
        // 0 -> 1 -> 0 over AIRLOCK_PULSE_TICKS.
        let ticks = (state.tick.checked_rem(u64::from(AIRLOCK_PULSE_TICKS)))
            .and_then(|t| u16::try_from(t).ok())
            .map_or(0.0, f32::from)
            + alpha;
        let pulse = 0.5_f32.mul_add(-(ticks / f32::from(AIRLOCK_PULSE_TICKS) * TAU).cos(), 0.5);
        for (hatch, live) in ship.hatches().iter().zip(&state.hatches) {
            if !hatch.rooms.iter().any(|&r| state.visited(r)) {
                continue;
            }
            let color = match live {
                HatchState::Closed if hatch.kind == HatchKind::Airlock => {
                    let [r, g, b, _] = AIRLOCK_OPEN_COLOR;
                    [r, g, b, pulse.mul_add(0.6, 0.4)]
                }
                HatchState::Closed => HATCH_CLOSED_COLOR,
                HatchState::Open => HATCH_OPEN_COLOR,
                HatchState::Sealed => HATCH_SEALED_COLOR,
                HatchState::AirlockLocked => AIRLOCK_LOCKED_COLOR,
                HatchState::Panel => {
                    self.push_panel(hatch.gap);
                    continue;
                }
            };
            for i in 0..hatch.gap.width {
                let (x, y) = hatch.gap.cell(i);
                let center = sim::room::cell_center(x, y);
                let at = [center.x.to_num(), center.y.to_num()];
                self.push_world(at, [half - 0.5, half - 0.5], color, SQUARE);
            }
        }
        let rooms = (0..).map(RoomId).take(ship.rooms().len());
        for id in rooms.filter(|&id| state.visited(id)) {
            let Some((x, y)) = ship.chest(id) else {
                continue;
            };
            let center = sim::room::cell_center(x, y);
            let at = [center.x.to_num(), center.y.to_num()];
            let opened = state.chest_opened(id);
            let (color, lid) = if opened {
                (CHEST_OPEN_COLOR, CHEST_OPEN_COLOR)
            } else {
                (CHEST_COLOR, CHEST_LID_COLOR)
            };
            let [w, h] = CHEST_HALF;
            self.push_world(at, [w, h], color, SQUARE);
            self.push_world([at[0], at[1] - h / 3.0], [w, 1.0], lid, SQUARE);
        }
    }

    /// A blinking spark ring round an EMP-stunned enemy drawn at `pos`.
    fn push_stun(&mut self, pos: [f32; 2], radius: f32, e: &Enemy, tick: u64) {
        if e.stun_ticks > 0 && tick.checked_rem(STUN_BLINK_PERIOD) < Some(STUN_BLINK_TICKS) {
            let [r, g, b] = EMP_COLOR;
            let ring = radius * 1.25;
            self.push_world(pos, [ring, ring], [r, g, b, 0.9], RING);
        }
    }

    /// EMPs in flight, interpolated like bullets.
    fn push_emps(&mut self, prev: &SimState, current: &SimState, alpha: f32) {
        let [r, g, b] = EMP_COLOR;
        for (id, e) in current.emps.iter() {
            let from = prev.emps.get(id).map_or(e.pos, |p| p.pos);
            let pos = lerp(from, e.pos, alpha);
            self.push_world(pos, [EMP_R, EMP_R], [r, g, b, 1.0], CIRCLE);
            self.push_world(pos, [EMP_R * 0.45, EMP_R * 0.45], HIT_COLOR, CIRCLE);
        }
    }

    /// Scrap on the floor, interpolated like bullets.
    fn push_pickups(&mut self, prev: &SimState, current: &SimState, alpha: f32) {
        for (stagger, (id, p)) in (0_u64..).zip(current.pickups.iter()) {
            let from = prev.pickups.get(id).map_or(p.pos, |q| q.pos);
            let pos = lerp(from, p.pos, alpha);
            let (color, half) = match p.kind {
                PickupKind::Scrap(5..) => (SCRAP_FIVE_COLOR, SCRAP_FIVE_HALF),
                PickupKind::Scrap(_) => (SCRAP_ONE_COLOR, SCRAP_ONE_HALF),
            };
            self.push_world(pos, [half, half], color, SQUARE);
            let phase = current
                .tick
                .wrapping_add(stagger.wrapping_mul(SCRAP_GLINT_STAGGER))
                .checked_rem(SCRAP_GLINT_PERIOD);
            if phase.is_some_and(|t| t < SCRAP_GLINT_TICKS) {
                let core = half * 0.5;
                self.push_world(pos, [core, core], HIT_COLOR, SQUARE);
            }
        }
    }

    /// An access panel over `gap`: wall, but for a faint seam down its middle, along the
    /// wall.
    fn push_panel(&mut self, gap: sim::room::Exit) {
        let half = sim::room::CELL.to_num::<f32>() / 2.0;
        let along_x = matches!(gap.dir, Dir::North | Dir::South);
        for i in 0..gap.width {
            let (x, y) = gap.cell(i);
            let center = sim::room::cell_center(x, y);
            let at = [center.x.to_num(), center.y.to_num()];
            self.push_world(at, [half - 0.5, half - 0.5], WALL_COLOR, SQUARE);
            let seam = if along_x { [half, 0.75] } else { [0.75, half] };
            self.push_world(at, seam, PANEL_SEAM_COLOR, SQUARE);
        }
    }

    /// One quad per cell of every revealed room and corridor, as the ship's grid has it
    /// (unused hatch points walled), except hatch cells (drawn by state in
    /// [`Self::push_floor`]); then the pits' lips.
    fn push_cells(&mut self, state: &SimState) {
        let ship = &state.ship;
        let half = sim::room::CELL.to_num::<f32>() / 2.0;
        for (x, y) in revealed(state) {
            // Tiles are inset a hair so the grid reads; pits a bit more, as holes.
            let (color, inset) = match ship.cell(x, y) {
                Cell::Floor => (FLOOR_COLOR, 0.5),
                Cell::Wall => (WALL_COLOR, 0.5),
                Cell::Pit => (PIT_COLOR, 3.0),
                Cell::Void => continue,
            };
            let (Ok(fx), Ok(fy)) = (usize::try_from(x), usize::try_from(y)) else {
                continue;
            };
            let center = sim::room::cell_center(fx, fy);
            self.push_world(
                [center.x.to_num(), center.y.to_num()],
                [half - inset, half - inset],
                color,
                SQUARE,
            );
        }
        self.push_pit_lips(state);
    }

    /// A brick lip on every revealed floor edge that drops into a pit, so pits read at a
    /// glance: two staggered courses of half-tile bricks laid along the edge, on the floor
    /// side.
    fn push_pit_lips(&mut self, state: &SimState) {
        let ship = &state.ship;
        let cell = sim::room::CELL.to_num::<f32>();
        let course = cell * PIT_LIP_COURSE;
        for (cx, cy) in revealed(state) {
            if ship.cell(cx, cy) != Cell::Floor {
                continue;
            }
            let (Ok(fx), Ok(fy)) = (usize::try_from(cx), usize::try_from(cy)) else {
                continue;
            };
            let center = sim::room::cell_center(fx, fy);
            let [mx, my] = [center.x.to_num::<f32>(), center.y.to_num::<f32>()];
            // (dx, dy): the unit step toward the pit neighbor.
            for (dx, dy) in [(0, -1), (0, 1), (-1, 0), (1, 0)] {
                let (Some(nx), Some(ny)) = (cx.checked_add(dx), cy.checked_add(dy)) else {
                    continue;
                };
                if ship.cell(nx, ny) != Cell::Pit {
                    continue;
                }
                let [nx, ny] = [
                    f32::from(i8::try_from(dx).unwrap_or(0)),
                    f32::from(i8::try_from(dy).unwrap_or(0)),
                ];
                // (course, color, bricks as (center along the edge, length), in cells).
                // The outer course sits on the edge; the inner one is staggered half a brick.
                for (row, color, bricks) in [
                    (0.5, PIT_LIP_COLOR, &[(-0.25_f32, 0.5_f32), (0.25, 0.5)][..]),
                    (
                        1.5,
                        PIT_LIP_DARK_COLOR,
                        &[(-0.375, 0.25), (0.0, 0.5), (0.375, 0.25)][..],
                    ),
                ] {
                    let depth = course.mul_add(-row, cell / 2.0);
                    for &(along, len) in bricks {
                        let along = along * cell;
                        let at = [
                            ny.abs().mul_add(along, nx.mul_add(depth, mx)),
                            nx.abs().mul_add(along, ny.mul_add(depth, my)),
                        ];
                        // Shrunk a hair on each side so mortar lines show between bricks.
                        let (long, short) = (len.mul_add(cell / 2.0, -0.75), course / 2.0 - 0.5);
                        let half = if dx == 0 {
                            [long, short]
                        } else {
                            [short, long]
                        };
                        self.push_world(at, half, color, SQUARE);
                    }
                }
            }
            // Corner stones, so the lip turns corners cleanly: outside a pit's corner
            // (only the diagonal is pit) to join the two runs, and inside one (both
            // sides are pit) to cover where the runs cross.
            for (dx, dy) in [(-1, -1), (1, -1), (-1, 1), (1, 1)] {
                let pit = |x: Option<i32>, y: Option<i32>| {
                    x.zip(y).is_some_and(|(x, y)| ship.cell(x, y) == Cell::Pit)
                };
                let (nx, ny) = (cx.checked_add(dx), cy.checked_add(dy));
                let (side_x, side_y) = (pit(nx, Some(cy)), pit(Some(cx), ny));
                let outside = pit(nx, ny) && !side_x && !side_y;
                if !(outside || side_x && side_y) {
                    continue;
                }
                let [sx, sy] = [
                    f32::from(i8::try_from(dx).unwrap_or(0)),
                    f32::from(i8::try_from(dy).unwrap_or(0)),
                ];
                let inset = cell / 2.0 - course;
                let at = [sx.mul_add(inset, mx), sy.mul_add(inset, my)];
                self.push_world(at, [course - 0.5, course - 0.5], PIT_LIP_COLOR, SQUARE);
            }
        }
    }

    /// A floor-space point in view points (origin top-left), through the camera of the
    /// last drawn frame.
    #[must_use]
    pub fn view_point(&self, [x, y]: [f32; 2]) -> [f32; 2] {
        let [w, h] = self.size_pt;
        let [cx, cy] = self.camera;
        [x - cx + w / 2.0, y - cy + h / 2.0]
    }

    /// Floor space (points, +y down) through the camera: one world unit is one view point.
    fn push_world(&mut self, [x, y]: [f32; 2], [hx, hy]: [f32; 2], color: [f32; 4], shape: f32) {
        let [w, h] = self.size_pt;
        let [cx, cy] = self.camera;
        let [sx, sy] = [2.0 / w, 2.0 / h];
        self.quads.push(Quad {
            center: [(x - cx) * sx, -(y - cy) * sy],
            half: [hx * sx, hy * sy],
            color,
            shape,
            dir: [1.0, 0.0],
            param: 0.0,
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
            dir: [1.0, 0.0],
            param: 0.0,
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
        Behavior::Shooter {
            pattern: Pattern::Captain,
            ..
        } => CAPTAIN_COLOR,
    }
}

/// Where the unlocked airlocks' outer hatches are, in floor space: once the bridge falls,
/// the way out, which the caret and the minimap's glow point to.
fn unlocked_airlocks(state: &SimState) -> Vec<[f32; 2]> {
    (state.ship.hatches().iter().zip(&state.hatches))
        .filter(|&(h, s)| h.kind == HatchKind::Airlock && *s == HatchState::Closed)
        .map(|(h, _)| {
            let (x, y) = h.gap.cell(0);
            let at = sim::room::cell_center(x, y);
            [at.x.to_num(), at.y.to_num()]
        })
        .collect()
}

/// How big `e` is drawn: its hitbox, but bigger for the captain.
fn drawn_radius(e: &Enemy) -> f32 {
    let radius = sim::ENEMY_RADIUS.to_num::<f32>();
    match e.behavior {
        Behavior::Shooter {
            pattern: Pattern::Captain,
            ..
        } => radius * CAPTAIN_SCALE,
        Behavior::Rusher { .. } | Behavior::Shooter { .. } => radius,
    }
}

/// The floor cells of revealed rooms and corridors, row-major; not hatches.
fn revealed(state: &SimState) -> impl Iterator<Item = (i32, i32)> {
    let (width, height) = state.ship.size();
    let (width, height) = (
        i32::try_from(width).unwrap_or(0),
        i32::try_from(height).unwrap_or(0),
    );
    (0..height)
        .flat_map(move |y| (0..width).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            matches!(state.ship.spot(x, y), Spot::Room { room, .. } if state.visited(room))
        })
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
    let attrs = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4, 3 => Float32, 4 => Float32x2, 5 => Float32];
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
