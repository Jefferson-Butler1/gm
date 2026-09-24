//! wgpu renderer drawing into a Swift-owned `CAMetalLayer`.
//!
//! Reads the previous and current [`SimState`] and interpolates between them; it never
//! mutates the sim. Hit flashes come from sim [`Event`]s. Placeholder art is flat colored
//! squares, circles and rings, converted to NDC on the CPU so there are no bind groups.
//! On-screen controls arrive as an [`Overlay`] in view points, since their layout belongs
//! to `game`.

use bytemuck::{Pod, Zeroable};
use sim::{EnemyId, Event, Fx, FxVec2, Player, SimState};
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
const ROOM_COLOR: [f32; 4] = [0.08, 0.09, 0.13, 1.0];
const PLAYER_COLOR: [f32; 4] = [0.3, 0.9, 1.0, 1.0];
/// Rolling (i-frames): shrunk and white, so dodge timing reads at a glance.
const ROLLING_COLOR: [f32; 4] = [1.0, 1.0, 1.0, 0.9];
const ROLLING_SCALE: f32 = 0.6;
const DEAD_COLOR: [f32; 4] = [0.35, 0.35, 0.4, 1.0];
const HURT_COLOR: [f32; 4] = [1.0, 0.25, 0.25, 1.0];
/// Post-hit invulnerability blinks the player: half alpha every other `BLINK_TICKS`.
const BLINK_TICKS: u8 = 4;
const RUSHER_COLOR: [f32; 4] = [0.95, 0.35, 0.3, 1.0];
const HIT_COLOR: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const BULLET_COLOR: [f32; 4] = [1.0, 0.9, 0.35, 1.0];
/// How long a hit flash lasts, in sim ticks.
const FLASH_TICKS: u64 = 6;
/// Facing nub: half-size and distance ahead of the player's center, in world units.
const NUB_HALF: f32 = 4.0;
const NUB_OFFSET: f32 = 22.0;
const STICK_KNOB_R: f32 = 22.0;
const DODGE_COLOR: [f32; 3] = [0.3, 0.9, 1.0];
const DODGE_READY_ALPHA: f32 = 0.35;
const DODGE_COOLDOWN_ALPHA: f32 = 0.1;

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
    pub dodge: Option<DodgeView>,
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
pub struct DodgeView {
    pub center: [f32; 2],
    pub radius: f32,
    /// Dimmed while the roll is on cooldown.
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
    quads: Vec<Quad>,
    /// Active hit flashes and the tick they started.
    flashes: Vec<(Flash, u64)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Flash {
    Enemy(EnemyId),
    Player(usize),
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
                Event::PlayerHit { slot } => Flash::Player(slot),
                Event::ShotFired { .. }
                | Event::EnemyKilled { .. }
                | Event::PlayerDied { .. }
                | Event::Restarted => continue,
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
            .retain(|&(_, tick)| current.tick < tick.saturating_add(FLASH_TICKS));
        self.push_world([0.0, 0.0], room_half(), ROOM_COLOR, SQUARE);
        let rusher = sim::RUSHER_HALF.to_num::<f32>();
        for (id, e) in current.enemies.iter() {
            let from = prev.enemies.get(id).map_or(e.pos, |p| p.pos);
            let color = if self.flashing(Flash::Enemy(id)) {
                HIT_COLOR
            } else {
                RUSHER_COLOR
            };
            self.push_world(lerp(from, e.pos, alpha), [rusher, rusher], color, CIRCLE);
        }
        for (slot, (a, b)) in prev.players.iter().zip(&current.players).enumerate() {
            if let (Some(a), Some(b)) = (a, b) {
                let hurt = self.flashing(Flash::Player(slot));
                self.push_player(lerp(a.pos, b.pos, alpha), b, hurt);
            }
        }
        let bullet = sim::BULLET_HALF.to_num::<f32>();
        for (id, b) in current.bullets.iter() {
            let from = prev.bullets.get(id).map_or(b.pos, |p| p.pos);
            self.push_world(
                lerp(from, b.pos, alpha),
                [bullet, bullet],
                BULLET_COLOR,
                CIRCLE,
            );
        }
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

    fn push_player(&mut self, [x, y]: [f32; 2], p: &Player, hurt: bool) {
        let half = sim::PLAYER_HALF.to_num::<f32>();
        let (half, mut color) = if !p.alive() {
            (half, DEAD_COLOR)
        } else if p.rolling() {
            (half * ROLLING_SCALE, ROLLING_COLOR)
        } else if hurt {
            (half, HURT_COLOR)
        } else {
            (half, PLAYER_COLOR)
        };
        if p.alive() && (p.hurt_ticks / BLINK_TICKS) % 2 == 1 {
            color[3] *= 0.4;
        }
        self.push_world([x, y], [half, half], color, SQUARE);
        let (sin, cos) = (f32::from(p.facing) / 65536.0 * TAU).sin_cos();
        let nub = [cos.mul_add(NUB_OFFSET, x), sin.mul_add(NUB_OFFSET, y)];
        self.push_world(nub, [NUB_HALF, NUB_HALF], color, SQUARE);
    }

    fn push_overlay(&mut self, overlay: &Overlay) {
        for s in overlay.sticks.iter().flatten() {
            let a = if s.active { 1.0 } else { 0.6 };
            self.push_screen(s.base, s.radius, [1.0, 1.0, 1.0, 0.25 * a], RING);
            self.push_screen(s.knob, STICK_KNOB_R, [1.0, 1.0, 1.0, 0.35 * a], CIRCLE);
        }
        if let Some(d) = overlay.dodge {
            let [r, g, b] = DODGE_COLOR;
            let a = if d.ready {
                DODGE_READY_ALPHA
            } else {
                DODGE_COOLDOWN_ALPHA
            };
            self.push_screen(d.center, d.radius, [r, g, b, a], CIRCLE);
        }
    }

    /// World origin is the screen center, +y down. One world unit is one point, scaled
    /// down only when the room does not fit (portrait).
    fn push_world(&mut self, [x, y]: [f32; 2], [hx, hy]: [f32; 2], color: [f32; 4], shape: f32) {
        let [w, h] = self.size_pt;
        let [rx, ry] = room_half();
        let scale = (w / (2.0 * rx)).min(h / (2.0 * ry)).min(1.0);
        let [sx, sy] = [2.0 * scale / w, 2.0 * scale / h];
        self.quads.push(Quad {
            center: [x * sx, -y * sy],
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

fn room_half() -> [f32; 2] {
    [sim::ROOM_HALF.x.to_num(), sim::ROOM_HALF.y.to_num()]
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
