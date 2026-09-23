//! wgpu renderer drawing into a Swift-owned `CAMetalLayer`.
//!
//! Reads the previous and current [`SimState`] and interpolates between them; it never
//! mutates the sim. Placeholder art is flat colored quads, converted to NDC on the CPU so
//! there are no bind groups.

use bytemuck::{Pod, Zeroable};
use sim::{Fx, FxVec2, SimState};
use std::ffi::c_void;
use std::ptr::NonNull;
use std::time::Instant;

const CLEAR: wgpu::Color = wgpu::Color {
    r: 0.035,
    g: 0.04,
    b: 0.06,
    a: 1.0,
};
const PLAYER_COLOR: [f32; 4] = [0.3, 0.9, 1.0, 1.0];
/// Placeholder player half-size, in points.
const PLAYER_HALF: f32 = 14.0;

const SHADER: &str = r"
struct Inst {
    @location(0) center: vec2f,
    @location(1) half_size: vec2f,
    @location(2) color: vec4f,
};
struct VOut {
    @builtin(position) pos: vec4f,
    @location(0) color: vec4f,
};
@vertex fn vs(@builtin(vertex_index) vi: u32, i: Inst) -> VOut {
    var corners = array<vec2f, 6>(
        vec2f(-1.0, -1.0), vec2f(1.0, -1.0), vec2f(1.0, 1.0),
        vec2f(-1.0, -1.0), vec2f(1.0, 1.0), vec2f(-1.0, 1.0));
    var o: VOut;
    o.pos = vec4f(i.center + corners[vi] * i.half_size, 0.0, 1.0);
    o.color = i.color;
    return o;
}
@fragment fn fs(v: VOut) -> @location(0) vec4f {
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
        })
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
    pub fn draw(&mut self, prev: &SimState, current: &SimState, alpha: f32) -> Option<f64> {
        for (a, b) in prev.players.iter().zip(&current.players) {
            if let (Some(a), Some(b)) = (a, b) {
                let [x, y] = lerp(a.pos, b.pos, alpha);
                self.push(x, y, PLAYER_HALF, PLAYER_COLOR);
            }
        }

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

    /// Queues a square centered at world point (`x`, `y`). The world origin is the screen
    /// center and one world unit is one point.
    fn push(&mut self, x: f32, y: f32, half: f32, color: [f32; 4]) {
        let [w, h] = self.size_pt;
        self.quads.push(Quad {
            center: [x / w * 2.0, -y / h * 2.0],
            half: [half / w * 2.0, half / h * 2.0],
            color,
        });
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
    let attrs = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4];
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
