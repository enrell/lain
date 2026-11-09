//! mpv inside the iced window.
//!
//! Path: `mpv-engine`'s exported-frame backend renders mpv (vo=libmpv,
//! OpenGL on a hidden WGL context) into shared D3D11 textures through
//! `WGL_NV_DX_interop2`; this module opens each pool texture's shared NT
//! handle on iced's own D3D12 device (`ID3D12Device::OpenSharedHandle`),
//! wraps it as a `wgpu::Texture` through wgpu-hal, and samples it in an
//! iced `shader` widget primitive. No pixel copies on the CPU; one
//! GPU-local `CopyResource` inside mpv-engine per frame.
//!
//! mpv-engine's own `into_wgpu_texture` is not used: it is built on
//! wgpu 30, and iced 0.14 is on wgpu 27, so the import is redone here
//! against wgpu 27 (about 40 lines). Each pool buffer is opened once and
//! cached by its handle; the frame handles themselves are held for two
//! more frames before they go back to mpv, so mpv never renders into a
//! texture the GPU may still be sampling.
//!
//! If the open fails (GL and wgpu landed on different GPUs, or wgpu is not
//! on DX12), it falls back to a CPU readback (`copy_pixels`) plus
//! `queue.write_texture` — correct but slow; `import_mode` says which.

use std::collections::VecDeque;
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use iced::widget::shader::{self, Viewport};
use iced::{Rectangle, mouse, wgpu};
use mpv_engine::{Engine, ExportedFrame};

/// One playback session's engine plus what the render side reports back.
pub struct VideoShared {
    pub id: u64,
    pub engine: Arc<Engine>,
    size: Mutex<(u32, u32)>,
    pub frames_shown: AtomicU64,
    pub import_mode: Mutex<&'static str>,
    pub import_error: Mutex<Option<String>>,
    /// Coalescing flags for the callbacks → iced messages.
    pub frame_pending: AtomicBool,
    pub wake_pending: AtomicBool,
}

impl fmt::Debug for VideoShared {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "VideoShared({})", self.id)
    }
}

impl VideoShared {
    pub fn new(id: u64, engine: Arc<Engine>, w: u32, h: u32) -> Self {
        VideoShared {
            id,
            engine,
            size: Mutex::new((w, h)),
            frames_shown: AtomicU64::new(0),
            import_mode: Mutex::new("pending"),
            import_error: Mutex::new(None),
            frame_pending: AtomicBool::new(false),
            wake_pending: AtomicBool::new(false),
        }
    }
}

/// The `shader` program: every draw hands the render side the session.
pub struct VideoView(pub Arc<VideoShared>);

impl<Message> shader::Program<Message> for VideoView {
    type State = ();
    type Primitive = VideoPrimitive;

    fn draw(&self, _state: &(), _cursor: mouse::Cursor, _bounds: Rectangle) -> VideoPrimitive {
        VideoPrimitive(self.0.clone())
    }
}

#[derive(Debug)]
pub struct VideoPrimitive(Arc<VideoShared>);

struct Imported {
    key: (usize, u32, u32),
    bind: wgpu::BindGroup,
    _texture: wgpu::Texture,
}

pub struct VideoPipeline {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    session: u64,
    cache: Vec<Imported>,
    current: Option<usize>,
    /// Frames still possibly read by in-flight GPU work.
    held: VecDeque<ExportedFrame>,
    /// CPU fallback target.
    cpu: Option<Imported>,
    use_cpu: bool,
}

const SHADER: &str = r#"
struct VsOut { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> };

@vertex
fn vs(@builtin(vertex_index) i: u32) -> VsOut {
    var p = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    let xy = p[i];
    var o: VsOut;
    o.pos = vec4(xy, 0.0, 1.0);
    o.uv = vec2((xy.x + 1.0) * 0.5, (1.0 - xy.y) * 0.5);
    return o;
}

@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var smp: sampler;

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + 0.055) / 1.055, vec3(2.4));
    return select(hi, lo, c <= vec3(0.04045));
}

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
    let c = textureSample(tex, smp, in.uv).rgb;
    return vec4(DECODE(c), 1.0);
}
"#;

impl shader::Pipeline for VideoPipeline {
    fn new(device: &wgpu::Device, _queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        // The shared texture is BGRA8 *UNORM* holding sRGB-encoded bytes;
        // if iced's target is an sRGB format, decode so the hardware
        // re-encode lands on the original values.
        let src = SHADER.replace("DECODE(c)", if format.is_srgb() { "to_linear(c)" } else { "c" });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("lain video"),
            source: wgpu::ShaderSource::Wgsl(src.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lain video"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("lain video"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("lain video"),
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("lain video"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        VideoPipeline {
            pipeline,
            layout,
            sampler,
            session: 0,
            cache: Vec::new(),
            current: None,
            held: VecDeque::new(),
            cpu: None,
            use_cpu: false,
        }
    }
}

impl VideoPipeline {
    fn reset(&mut self, session: u64) {
        self.session = session;
        self.cache.clear();
        self.current = None;
        self.held.clear();
        self.cpu = None;
        self.use_cpu = false;
    }

    fn bind(&self, device: &wgpu::Device, texture: &wgpu::Texture) -> wgpu::BindGroup {
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lain video frame"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        })
    }

    fn show_cpu(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, frame: &ExportedFrame) {
        let (w, h) = (frame.width(), frame.height());
        if self.cpu.as_ref().map(|c| (c.key.1, c.key.2)) != Some((w, h)) {
            let texture = device.create_texture(&texture_desc(w, h, wgpu::TextureUsages::COPY_DST));
            let bind = self.bind(device, &texture);
            self.cpu = Some(Imported { key: (0, w, h), bind, _texture: texture });
        }
        let pixels = frame.copy_pixels();
        if let Some(c) = &self.cpu {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &c._texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &pixels,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(h) },
                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            );
        }
    }
}

fn texture_desc(w: u32, h: u32, extra: wgpu::TextureUsages) -> wgpu::TextureDescriptor<'static> {
    wgpu::TextureDescriptor {
        label: Some("mpv frame"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Bgra8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | extra,
        view_formats: &[],
    }
}

/// Open the frame's shared D3D11 texture on wgpu's D3D12 device.
#[cfg(windows)]
fn import(device: &wgpu::Device, frame: &ExportedFrame) -> Result<wgpu::Texture, String> {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Graphics::Direct3D12::ID3D12Resource;
    let (w, h) = (frame.width(), frame.height());
    let resource = {
        // SAFETY: read-only access to the hal device; the guard is dropped
        // before `create_texture_from_hal` below.
        let hal = unsafe { device.as_hal::<wgpu::hal::api::Dx12>() };
        let Some(hal) = hal else { return Err("wgpu is not on DX12".into()) };
        let mut res: Option<ID3D12Resource> = None;
        // SAFETY: the handle is a live shared NT handle while `frame` lives;
        // OpenSharedHandle duplicates it.
        unsafe { hal.raw_device().OpenSharedHandle(HANDLE(frame.shared_handle()), &mut res) }
            .map_err(|e| format!("OpenSharedHandle: {e}"))?;
        res.ok_or("OpenSharedHandle returned nothing")?
    };
    // SAFETY: resource is a 2D, 1-mip, 1-sample BGRA8 texture, fully
    // written by mpv before the frame was published.
    let hal_tex = unsafe {
        wgpu::hal::dx12::Device::texture_from_raw(
            resource,
            wgpu::TextureFormat::Bgra8Unorm,
            wgpu::TextureDimension::D2,
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            1,
            1,
        )
    };
    let desc = texture_desc(w, h, wgpu::TextureUsages::empty());
    Ok(unsafe { device.create_texture_from_hal::<wgpu::hal::api::Dx12>(hal_tex, &desc) })
}

#[cfg(not(windows))]
fn import(_device: &wgpu::Device, _frame: &ExportedFrame) -> Result<wgpu::Texture, String> {
    Err("zero-copy import is only wired for Windows in this prototype".into())
}

impl shader::Primitive for VideoPrimitive {
    type Pipeline = VideoPipeline;

    fn prepare(
        &self,
        pipe: &mut VideoPipeline,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bounds: &Rectangle,
        viewport: &Viewport,
    ) {
        let s = &self.0;
        if pipe.session != s.id {
            pipe.reset(s.id);
        }
        let scale = viewport.scale_factor();
        let want = ((bounds.width * scale).round().max(1.0) as u32, (bounds.height * scale).round().max(1.0) as u32);
        {
            let mut size = s.size.lock().unwrap_or_else(|e| e.into_inner());
            if *size != want {
                *size = want;
                let _ = s.engine.set_export_size(want.0, want.1);
            }
        }
        let frame = match s.engine.acquire_frame() {
            Ok(Some(f)) => f,
            _ => return,
        };
        if pipe.use_cpu {
            pipe.show_cpu(device, queue, &frame);
        } else {
            let key = (frame.shared_handle() as usize, frame.width(), frame.height());
            if let Some(i) = pipe.cache.iter().position(|c| c.key == key) {
                pipe.current = Some(i);
            } else {
                // A resize brings a new set of buffers: drop the old ones.
                pipe.cache.retain(|c| (c.key.1, c.key.2) == (key.1, key.2));
                match import(device, &frame) {
                    Ok(texture) => {
                        let bind = pipe.bind(device, &texture);
                        pipe.cache.push(Imported { key, bind, _texture: texture });
                        pipe.current = Some(pipe.cache.len() - 1);
                        *s.import_mode.lock().unwrap_or_else(|e| e.into_inner()) = "zero-copy (D3D11 shared handle → D3D12)";
                    }
                    Err(e) => {
                        *s.import_error.lock().unwrap_or_else(|e| e.into_inner()) = Some(e);
                        *s.import_mode.lock().unwrap_or_else(|e| e.into_inner()) = "CPU readback fallback";
                        pipe.use_cpu = true;
                        pipe.show_cpu(device, queue, &frame);
                    }
                }
            }
        }
        s.frames_shown.fetch_add(1, Ordering::Relaxed);
        pipe.held.push_back(frame);
        while pipe.held.len() > 2 {
            if let Some(old) = pipe.held.pop_front() {
                old.presented();
            }
        }
    }

    fn draw(&self, pipe: &VideoPipeline, pass: &mut wgpu::RenderPass<'_>) -> bool {
        if pipe.session != self.0.id {
            return true;
        }
        let bind = if pipe.use_cpu {
            pipe.cpu.as_ref().map(|c| &c.bind)
        } else {
            pipe.current.and_then(|i| pipe.cache.get(i)).map(|c| &c.bind)
        };
        if let Some(bind) = bind {
            pass.set_pipeline(&pipe.pipeline);
            pass.set_bind_group(0, bind, &[]);
            pass.draw(0..3, 0..1);
        }
        true
    }
}
