// Phase 5 — GPU rendering via wgpu 29.
// Background quads + glyph atlas textured quads.  Damage tracking gates passes.

pub mod atlas;
pub mod damage;

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use thiserror::Error;
use winit::window::Window;

use fonts::FontSystem;
use vt::Terminal;

use atlas::GlyphAtlas;
use damage::DamageTracker;

use crate::colors;

// ─── error ────────────────────────────────────────────────────────────────────

#[derive(Error, Debug)]
pub enum RendererError {
    #[error("surface creation failed: {0}")]
    Surface(#[from] wgpu::CreateSurfaceError),
    #[error("no suitable GPU adapter found")]
    NoAdapter,
    #[error("device request failed: {0}")]
    Device(#[from] wgpu::RequestDeviceError),
}

// ─── vertex types ─────────────────────────────────────────────────────────────

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct BgInstance {
    pos:  [f32; 2],
    size: [f32; 2],
    col:  [f32; 4],
}

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct GlyphInstance {
    pos:     [f32; 2],
    size:    [f32; 2],
    uv_rect: [f32; 4],
    col:     [f32; 4],
}

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct ScreenUniform {
    size: [f32; 2],
    _pad: [f32; 2],
}

// ─── WGSL ─────────────────────────────────────────────────────────────────────

const SHADER_SRC: &str = r#"
struct Screen { size: vec2<f32>, _pad: vec2<f32> }
@group(0) @binding(0) var<uniform> screen: Screen;

const QUAD: array<vec2<f32>, 6> = array(
    vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
    vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0),
);
const QUAD_UV: array<vec2<f32>, 6> = array(
    vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
    vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0),
);

fn to_ndc(sp: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(
         sp.x / screen.size.x * 2.0 - 1.0,
        1.0 - sp.y / screen.size.y * 2.0,
    );
}

// --- Background pipeline ---
struct BgIn {
    @location(0) pos:  vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) col:  vec4<f32>,
}
struct BgOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) col: vec4<f32>,
}
@vertex fn vs_bg(@builtin(vertex_index) vi: u32, inst: BgIn) -> BgOut {
    var out: BgOut;
    out.clip = vec4<f32>(to_ndc(inst.pos + QUAD[vi] * inst.size), 0.0, 1.0);
    out.col  = inst.col;
    return out;
}
@fragment fn fs_bg(in: BgOut) -> @location(0) vec4<f32> { return in.col; }

// --- Glyph pipeline ---
struct GlIn {
    @location(0) pos:     vec2<f32>,
    @location(1) size:    vec2<f32>,
    @location(2) uv_rect: vec4<f32>,
    @location(3) col:     vec4<f32>,
}
struct GlOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv:  vec2<f32>,
    @location(1) col: vec4<f32>,
}
@group(0) @binding(1) var atlas_tex:  texture_2d<f32>;
@group(0) @binding(2) var atlas_samp: sampler;

@vertex fn vs_glyph(@builtin(vertex_index) vi: u32, inst: GlIn) -> GlOut {
    let lp  = QUAD[vi];
    let uv  = inst.uv_rect.xy + QUAD_UV[vi] * (inst.uv_rect.zw - inst.uv_rect.xy);
    var out: GlOut;
    out.clip = vec4<f32>(to_ndc(inst.pos + lp * inst.size), 0.0, 1.0);
    out.uv   = uv;
    out.col  = inst.col;
    return out;
}
@fragment fn fs_glyph(in: GlOut) -> @location(0) vec4<f32> {
    let cov = textureSample(atlas_tex, atlas_samp, in.uv).r;
    return vec4<f32>(in.col.rgb, cov);
}
"#;

// ─── Renderer ─────────────────────────────────────────────────────────────────

pub struct Renderer {
    surface:        wgpu::Surface<'static>,
    device:         wgpu::Device,
    queue:          wgpu::Queue,
    config:         wgpu::SurfaceConfiguration,
    bg_pipeline:    wgpu::RenderPipeline,
    glyph_pipeline: wgpu::RenderPipeline,
    bind_group:     wgpu::BindGroup,
    screen_buf:     wgpu::Buffer,
    atlas:          GlyphAtlas,
    damage:         DamageTracker,
    pub present_mode_name: String,
}

impl Renderer {
    pub async fn new(
        window: Arc<Window>,
        width:  u32,
        height: u32,
    ) -> Result<Self, RendererError> {
        let instance = wgpu::Instance::default();
        let surface: wgpu::Surface<'static> = instance.create_surface(window)?;

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference:       wgpu::PowerPreference::HighPerformance,
                compatible_surface:     Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .map_err(|_| RendererError::NoAdapter)?;

        let (device, queue): (wgpu::Device, wgpu::Queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label:             Some("termd"),
                required_features: wgpu::Features::empty(),
                required_limits:   wgpu::Limits::default(),
                memory_hints:      wgpu::MemoryHints::Performance,
                ..Default::default() // experimental_features, trace
            })
            .await?;

        // Choose the lowest-latency present mode available on this backend.
        let caps = surface.get_capabilities(&adapter);
        let present_mode = [
            wgpu::PresentMode::Mailbox,
            wgpu::PresentMode::Immediate,
            wgpu::PresentMode::FifoRelaxed,
            wgpu::PresentMode::Fifo,
        ]
        .into_iter()
        .find(|m| caps.present_modes.contains(m))
        .unwrap_or(wgpu::PresentMode::Fifo);
        let present_mode_name = format!("{present_mode:?}");
        eprintln!("[render] GPU adapter: {}  present mode: {present_mode_name}",
                  adapter.get_info().name);

        let surface_format = caps
            .formats.iter()
            .find(|f| f.is_srgb())
            .copied()
            .unwrap_or(caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage:    wgpu::TextureUsages::RENDER_ATTACHMENT,
            format:   surface_format,
            width:    width.max(1),
            height:   height.max(1),
            present_mode,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
            desired_maximum_frame_latency: 1,
        };
        surface.configure(&device, &config);

        // Screen-size uniform.
        let screen_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("screen_uniform"),
            size:  std::mem::size_of::<ScreenUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&screen_buf, 0, bytemuck::bytes_of(&ScreenUniform {
            size: [width as f32, height as f32],
            _pad: [0.0; 2],
        }));

        let atlas = GlyphAtlas::new(&device);

        // Bind group layout: uniform (binding 0) + texture (1) + sampler (2).
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label:   Some("bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding:    0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty:                 wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size:   None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding:    1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type:    wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled:   false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding:    2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label:   Some("bind_group"),
            layout:  &bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: screen_buf.as_entire_binding() },
                wgpu::BindGroupEntry {
                    binding:  1,
                    resource: wgpu::BindingResource::TextureView(&atlas.view),
                },
                wgpu::BindGroupEntry {
                    binding:  2,
                    resource: wgpu::BindingResource::Sampler(&atlas.sampler),
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label:              Some("pipeline_layout"),
            bind_group_layouts: &[Some(&bgl)],
            ..Default::default() // immediate_size = 0
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label:  Some("termd_shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_SRC.into()),
        });

        let bg_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label:  Some("bg_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module:      &shader,
                entry_point: Some("vs_bg"),
                buffers:     &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<BgInstance>() as u64,
                    step_mode:    wgpu::VertexStepMode::Instance,
                    attributes:   &[
                        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 0,  shader_location: 0 },
                        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 8,  shader_location: 1 },
                        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 16, shader_location: 2 },
                    ],
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module:      &shader,
                entry_point: Some("fs_bg"),
                targets:     &[Some(wgpu::ColorTargetState {
                    format:     surface_format,
                    blend:      Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive:     wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample:   wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache:          None,
        });

        let glyph_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label:  Some("glyph_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module:      &shader,
                entry_point: Some("vs_glyph"),
                buffers:     &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GlyphInstance>() as u64,
                    step_mode:    wgpu::VertexStepMode::Instance,
                    attributes:   &[
                        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 0,  shader_location: 0 },
                        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 8,  shader_location: 1 },
                        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 16, shader_location: 2 },
                        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 32, shader_location: 3 },
                    ],
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module:      &shader,
                entry_point: Some("fs_glyph"),
                targets:     &[Some(wgpu::ColorTargetState {
                    format:     surface_format,
                    blend:      Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive:     wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample:   wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache:          None,
        });

        Ok(Self {
            surface, device, queue, config,
            bg_pipeline, glyph_pipeline,
            bind_group, screen_buf,
            atlas,
            damage: DamageTracker::new(),
            present_mode_name,
        })
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 { return; }
        self.config.width  = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        self.queue.write_buffer(&self.screen_buf, 0, bytemuck::bytes_of(&ScreenUniform {
            size: [width as f32, height as f32],
            _pad: [0.0; 2],
        }));
        self.damage.reset(); // force full redraw after resize
    }

    /// Render one frame.  Returns false when nothing changed (damage gate skipped the pass).
    ///
    /// `scroll_offset` is the number of lines scrolled back (0 = live view).
    /// When non-zero the damage gate is bypassed and no cursor is drawn.
    pub fn render(
        &mut self,
        term:         &Terminal,
        fonts:        &FontSystem,
        cell_w:       u32,
        cell_h:       u32,
        ascent:       u32,
        scroll_offset: usize,
    ) -> bool {
        // Damage gate — only skip when in live view and grid is unchanged.
        if scroll_offset == 0 && !self.damage.diff(term) {
            self.damage.skipped += 1;
            return false;
        }
        self.damage.rendered += 1;

        let rows = term.screen.rows();
        let cols = term.screen.cols();
        let cur  = term.screen.cursor();
        let cw   = cell_w as f32;
        let ch   = cell_h as f32;
        let asc  = ascent as i32;

        let mut bg_insts:    Vec<BgInstance>    = Vec::with_capacity(rows * cols + 8);
        let mut glyph_insts: Vec<GlyphInstance> = Vec::with_capacity(rows * cols);

        for row in 0..rows {
            for col in 0..cols {
                // Resolve cell: either from live grid or scrollback when scrolled.
                let cell = if scroll_offset == 0 {
                    term.screen.cell(row, col).clone()
                } else {
                    term.screen.display_cell(row, col, scroll_offset)
                        .cloned()
                        .unwrap_or_default()
                };

                let (fg_col, bg_col) = if cell.attrs.inverse {
                    (cell.attrs.bg, cell.attrs.fg)
                } else {
                    (cell.attrs.fg, cell.attrs.bg)
                };
                let fg = argb_to_rgbaf(colors::to_argb(fg_col, true));
                let bg = argb_to_rgbaf(colors::to_argb(bg_col, false));

                let px = (col as f32) * cw;
                let py = (row as f32) * ch;

                bg_insts.push(BgInstance { pos: [px, py], size: [cw, ch], col: bg });

                // Only draw cursor in live view.
                let is_cursor = scroll_offset == 0 && cur.visible && cur.row == row && cur.col == col;
                if is_cursor {
                    let inv = [1.0 - bg[0], 1.0 - bg[1], 1.0 - bg[2], 1.0f32];
                    bg_insts.push(BgInstance { pos: [px, py], size: [2.0, ch], col: inv });
                }

                if cell.ch != ' ' {
                    let rg_opt = fonts.rasterize(cell.ch);
                    let rg_ref = rg_opt.as_ref();
                    let q      = &self.queue;
                    if let Some(e) = self.atlas.get_or_insert(cell.ch, rg_ref, q) {
                        let gx = px + e.bearing_x as f32;
                        let gy = py + (asc - e.bearing_y) as f32;
                        glyph_insts.push(GlyphInstance {
                            pos:     [gx, gy],
                            size:    [e.width as f32, e.height as f32],
                            uv_rect: e.uv,
                            col:     fg,
                        });
                    }
                }
            }
        }

        // Upload instance data to GPU.
        let bg_data    = bytemuck::cast_slice::<BgInstance,    u8>(&bg_insts);
        let glyph_data = bytemuck::cast_slice::<GlyphInstance, u8>(&glyph_insts);

        let bg_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bg_inst"),
            size:  (bg_data.len() as u64).max(4),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&bg_buf, 0, bg_data);

        let gl_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("glyph_inst"),
            size:  (glyph_data.len() as u64).max(4),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&gl_buf, 0, glyph_data);

        // Acquire frame.
        let surface_tex = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(st) | wgpu::CurrentSurfaceTexture::Suboptimal(st) => st,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                self.damage.reset();
                return false;
            }
            _ => return false,
        };
        let view = surface_tex.texture.create_view(&wgpu::TextureViewDescriptor::default());

        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("frame_enc"),
        });

        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view:           &view,
                    depth_slice:    None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load:  wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes:         None,
                occlusion_query_set:      None,
                multiview_mask:           None,
            });

            pass.set_bind_group(0, &self.bind_group, &[]);

            if !bg_insts.is_empty() {
                pass.set_pipeline(&self.bg_pipeline);
                pass.set_vertex_buffer(0, bg_buf.slice(..));
                pass.draw(0..6, 0..bg_insts.len() as u32);
            }
            if !glyph_insts.is_empty() {
                pass.set_pipeline(&self.glyph_pipeline);
                pass.set_vertex_buffer(0, gl_buf.slice(..));
                pass.draw(0..6, 0..glyph_insts.len() as u32);
            }
        }

        self.queue.submit(std::iter::once(enc.finish()));
        surface_tex.present();
        true
    }

    pub fn damage_stats(&self) -> (u64, u64) {
        (self.damage.rendered, self.damage.skipped)
    }
}

fn argb_to_rgbaf(argb: u32) -> [f32; 4] {
    let r = ((argb >> 16) & 0xFF) as f32 / 255.0;
    let g = ((argb >>  8) & 0xFF) as f32 / 255.0;
    let b = ( argb        & 0xFF) as f32 / 255.0;
    [r, g, b, 1.0]
}
