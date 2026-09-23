//! Thin instanced-quad renderer. CPU converts points -> NDC, so there are no bind groups.

use bytemuck::{Pod, Zeroable};
use std::ffi::c_void;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Quad {
    center: [f32; 2],
    half: [f32; 2],
    color: [f32; 4],
    /// 0 = square, 1 = filled circle, 2 = ring
    shape: f32,
}

pub const SQUARE: f32 = 0.0;
pub const CIRCLE: f32 = 1.0;
pub const RING: f32 = 2.0;

const SHADER: &str = r#"
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
@fragment fn fs(v: VOut) -> @location(0) vec4f {
    if (v.shape > 0.5) {
        let d = length(v.uv);
        if (d > 1.0) { discard; }
        if (v.shape > 1.5 && d < 0.88) { discard; }
    }
    return v.color;
}
"#;

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
    pub quads: Vec<Quad>,
}

impl Renderer {
    /// # Safety
    /// `layer` must be a live CAMetalLayer that outlives the renderer.
    pub unsafe fn new(layer: *mut c_void, width_px: u32, height_px: u32, size_pt: [f32; 2]) -> Self {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let surface = unsafe {
            instance
                .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::CoreAnimationLayer(layer))
                .expect("create surface")
        };
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .expect("adapter");
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("device");

        let caps = surface.get_capabilities(&adapter);
        // Prefer a non-sRGB format so hard-coded colors look as written.
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .unwrap_or(caps.formats[0]);
        eprintln!("[spike] adapter={:?} formats={:?} chosen={:?} present_modes={:?}",
            adapter.get_info().name, caps.formats, format, caps.present_modes);
        let mut config = surface
            .get_default_config(&adapter, width_px.max(1), height_px.max(1))
            .expect("surface config");
        config.format = format;
        config.present_mode = wgpu::PresentMode::Fifo;
        // 2 => maximumDrawableCount 3. With 1 (2 drawables) nextDrawable() blocked ~14ms per
        // display-link callback (one drawable on screen, one queued), halving 120 Hz to 60.
        config.desired_maximum_frame_latency = 2;
        config.alpha_mode = wgpu::CompositeAlphaMode::Opaque;
        surface.configure(&device, &config);

        let pipeline = make_pipeline(&device, format);
        let capacity = 4096;
        let instances = Self::make_buffer(&device, capacity);
        Self { surface, device, queue, config, pipeline, instances, capacity, size_pt, quads: Vec::new() }
    }

    fn make_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("instances"),
            size: (capacity * std::mem::size_of::<Quad>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
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

    /// Push a quad in screen points (center, half-size).
    pub fn push(&mut self, cx: f32, cy: f32, hw: f32, hh: f32, color: [f32; 4], shape: f32) {
        let [w, h] = self.size_pt;
        self.quads.push(Quad {
            center: [cx / w * 2.0 - 1.0, 1.0 - cy / h * 2.0],
            half: [hw / w * 2.0, hh / h * 2.0],
            color,
            shape,
        });
    }

    /// Returns seconds spent blocked acquiring the drawable, or None if no frame was presented.
    pub fn draw(&mut self) -> Option<f64> {
        let t0 = std::time::Instant::now();
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
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
            self.instances = Self::make_buffer(&self.device, self.capacity);
        }
        self.queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(&self.quads));

        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.035, g: 0.04, b: 0.06, a: 1.0 }),
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
            pass.draw(0..6, 0..self.quads.len() as u32);
        }
        self.queue.submit([enc.finish()]);
        self.queue.present(frame);
        self.quads.clear();
        Some(acquire)
    }
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
        let attrs = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4, 3 => Float32];
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("quad"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Quad>() as u64,
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
                compilation_options: Default::default(),
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
    /// Host (macOS Metal) check that the WGSL + pipeline validate; the device is otherwise a black box.
    #[test]
    fn pipeline_validates() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
        let (device, _q) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        super::make_pipeline(&device, wgpu::TextureFormat::Bgra8Unorm);
        assert!(pollster::block_on(scope.pop()).is_none());
    }
}
