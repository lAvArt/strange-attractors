//! The GPU side. One render pass straight into the output: the ground, the annotations, the sheets (which write
//! depth, and colour only in the looks that want it), then lines and dots blended over them. Knows nothing about windows: it draws into whatever view it is given.

use crate::config::Dots;
use crate::text::{Annot, TextVertex};
use crate::world::World;
use bytemuck::{Pod, Zeroable};

const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth16Unorm;
/// the veil: view depth and strength of the nearest sheet, at a quarter of the scene's size so it can be read smoothly
const VEIL: wgpu::TextureFormat = wgpu::TextureFormat::Rg16Float;
const MAX_TEXT_VERTS: usize = 6 * 4096;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Frame {
    view: [[f32; 4]; 4],
    proj: [[f32; 4]; 4],
    c0: [f32; 4],
    c1: [f32; 4],
    c2: [f32; 4],
    fog: [f32; 4],
    acc: [f32; 4],
    ids: [u32; 4],
    p1: [f32; 4],
    p2: [f32; 4],
    st: [f32; 4],
    st2: [f32; 4],
    st3: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Pass {
    v: [f32; 4],
    bg0: [f32; 4],
    bg1: [f32; 4],
    ex: [f32; 4],
    lg: [f32; 4],
    lt: [f32; 4],
    g0: [f32; 4],
    g1: [f32; 4],
    q: [f32; 4],
}

/// What the dots of a soft look read to know how much of them a nearer sheet covers, and the bind group that hands
/// it to them.
struct Veil {
    view: wgpu::TextureView,
    depth: wgpu::TextureView,
    dot_bind: wgpu::BindGroup,
}

struct Targets {
    veil: Option<Veil>,
    /// the whole-number upscale of a pixelated look (0: none)
    k: u32,
    depth: wgpu::TextureView,
    /// the scene, when it is rendered smaller than the output, and the group that draws it onto the output
    mid: Option<(wgpu::TextureView, wgpu::BindGroup)>,
    w: u32,
    h: u32,
}

/// one of the strand passes: its own copy of the frame uniforms (each pass has its own stride) and its bind group
struct Layer {
    buf: wgpu::Buffer,
    bind: wgpu::BindGroup,
    pipe: wgpu::RenderPipeline,
}

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    format: wgpu::TextureFormat,
    scale: f32,
    ring: wgpu::Buffer,
    sheet: Layer,
    /// the same sheet with colour writes off, for looks whose sheet only hides
    sheet_unseen: wgpu::RenderPipeline,
    veil_pipe: wgpu::RenderPipeline,
    bgl: wgpu::BindGroupLayout,
    /// a stand-in for the veil in the passes that do not read one (kept alive for their bind groups)
    _blank: wgpu::TextureView,
    soft: bool,
    flow: Layer,
    cross: Layer,
    dot: Layer,
    mote: Layer,
    pp_bgl: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    nearest: wgpu::Sampler,
    /// the look draws into a coarse picture of about this many lines, scaled up by a whole number (0: no)
    pixel: u32,
    chunk: u32,
    pass_buf: wgpu::Buffer,
    ground: wgpu::RenderPipeline,
    ground_bind: wgpu::BindGroup,
    blit: wgpu::RenderPipeline,
    text: wgpu::RenderPipeline,
    text_bind: Option<wgpu::BindGroup>,
    text_buf: wgpu::Buffer,
    text_count: u32,
    /// how many of those are the margins' text (laid out a few times a second); the trackers follow, every frame
    text_static: u32,
    text_tick: i64,
    annot: Option<Annot>,
    /// the logo's aspect ratio (w/h), 0 without a logo
    logo_aspect: f32,
    t: Targets,
    out: (u32, u32),
    n: usize,
    len: usize,
    m: usize,
    seen_steps: u64,
    seen_epoch: u32,
}

/// the size the scene is drawn at, and the whole number it is scaled up by afterwards (0: by filtering, or not at all)
fn scene_size(out: (u32, u32), scale: f32, pixel: u32) -> ((u32, u32), u32) {
    if pixel > 0 {
        let k = ((out.1 as f32 / pixel as f32).round() as u32).max(1);
        return ((out.0.div_ceil(k), out.1.div_ceil(k)), k);
    }
    if scale >= 0.999 { (out, 0) } else { ((((out.0 as f32 * scale) as u32).max(1), ((out.1 as f32 * scale) as u32).max(1)), 0) }
}

fn tex_group(device: &wgpu::Device, bgl: &wgpu::BindGroupLayout, sampler: &wgpu::Sampler, view: &wgpu::TextureView, buf: &wgpu::Buffer) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: bgl,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(view) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
            wgpu::BindGroupEntry { binding: 2, resource: buf.as_entire_binding() },
        ],
    })
}

fn strand_entries<'a>(buf: &'a wgpu::Buffer, ring: &'a wgpu::Buffer, veil: &'a wgpu::TextureView, sampler: &'a wgpu::Sampler) -> [wgpu::BindGroupEntry<'a>; 4] {
    [
        wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() },
        wgpu::BindGroupEntry { binding: 1, resource: ring.as_entire_binding() },
        wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(veil) },
        wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(sampler) },
    ]
}

fn tex2d(device: &wgpu::Device, w: u32, h: u32, format: wgpu::TextureFormat, usage: wgpu::TextureUsages) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

impl Renderer {
    /// `out` is the target size in pixels, `scale` the fraction of it the scene renders at (1: straight into the target).
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat, out: (u32, u32), scale: f32, world: &World) -> Self {
        let (n, len, m) = (world.trails.n, world.trails.len, world.trails.m);
        let shader = device.create_shader_module(wgpu::include_wgsl!("shader.wgsl"));
        let ring = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ring"),
            size: (n * len * 16) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniform = |min: Option<u64>| wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: min.and_then(std::num::NonZeroU64::new) };
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::VERTEX_FRAGMENT, ty: uniform(None), count: None },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 3, visibility: wgpu::ShaderStages::VERTEX, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            ],
        });
        let blank = tex2d(device, 1, 1, VEIL, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING).create_view(&Default::default());
        let veil_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(&bgl)], immediate_size: 0 });
        let over = wgpu::BlendState { color: wgpu::BlendComponent::OVER, alpha: wgpu::BlendComponent::OVER };
        let depth = |write: bool, compare: wgpu::CompareFunction| wgpu::DepthStencilState {
            format: DEPTH,
            depth_write_enabled: Some(write),
            depth_compare: Some(compare),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        };
        let layer = |vs: &str, fs: &str, sheet: bool, unseen: bool| {
            let targets = [Some(wgpu::ColorTargetState { format, blend: if unseen { None } else { Some(over) }, write_mask: if unseen { wgpu::ColorWrites::empty() } else { wgpu::ColorWrites::ALL } })];
            let buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(vs),
                size: std::mem::size_of::<Frame>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &bgl,
                entries: &strand_entries(&buf, &ring, &blank, &veil_sampler),
            });
            let pipe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(vs),
                layout: Some(&layout),
                vertex: wgpu::VertexState { module: &shader, entry_point: Some(vs), compilation_options: Default::default(), buffers: &[] },
                fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some(fs), compilation_options: Default::default(), targets: &targets }),
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, ..Default::default() },
                depth_stencil: Some(depth(sheet, wgpu::CompareFunction::LessEqual)),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });
            Layer { buf, bind, pipe }
        };
        let (sheet, flow, cross, dot) = (layer("vs_sheet", "fs_sheet", true, false), layer("vs_flow", "fs_line", false, false), layer("vs_cross", "fs_line", false, false), layer("vs_dot", "fs_dot", false, false));
        let sheet_unseen = layer("vs_sheet", "fs_sheet", true, true).pipe;
        let mote = layer("vs_mote", "fs_dot", false, false);
        let veil_pipe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("veil"),
            layout: Some(&layout),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_sheet"), compilation_options: Default::default(), buffers: &[] },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_veil"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: VEIL, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, ..Default::default() },
            depth_stencil: Some(depth(true, wgpu::CompareFunction::LessEqual)),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let pp_shader = device.create_shader_module(wgpu::include_wgsl!("pp.wgsl"));
        let pp_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::VERTEX_FRAGMENT, ty: uniform(None), count: None },
            ],
        });
        let pp_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(&pp_bgl)], immediate_size: 0 });
        let text_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<TextVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4],
        };
        let pp_make = |vs: &str, fs: &str, blend: Option<wgpu::BlendState>, with_depth: bool, buffers: &[Option<wgpu::VertexBufferLayout>]| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(fs),
                layout: Some(&pp_layout),
                vertex: wgpu::VertexState { module: &pp_shader, entry_point: Some(vs), compilation_options: Default::default(), buffers },
                fragment: Some(wgpu::FragmentState {
                    module: &pp_shader,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: with_depth.then(|| depth(false, wgpu::CompareFunction::Always)),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let nearest = device.create_sampler(&wgpu::SamplerDescriptor { address_mode_u: wgpu::AddressMode::ClampToEdge, address_mode_v: wgpu::AddressMode::ClampToEdge, ..Default::default() });
        let ground = pp_make("vs_full", "fs_ground", None, true, &[]);
        let blit = pp_make("vs_full", "fs_blit", None, false, &[]);
        let text = pp_make("vs_text", "fs_text", Some(over), true, &[Some(text_layout)]);
        let sampler = &veil_sampler;
        // the logo's mask: a 1x1 transparent texel when there is none, so the ground always binds the same layout
        let (lw, lh, ldata): (u32, u32, &[u8]) = match &world.logo {
            Some(l) => (l.w, l.h, &l.rgba),
            None => (1, 1, &[0, 0, 0, 0]),
        };
        let upload = |w: u32, h: u32, fmt: wgpu::TextureFormat, data: &[u8], bpp: u32| {
            let t = tex2d(device, w, h, fmt, wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST);
            queue.write_texture(
                t.as_image_copy(),
                data,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * bpp), rows_per_image: Some(h) },
                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            );
            t.create_view(&wgpu::TextureViewDescriptor::default())
        };
        let logo_view = upload(lw, lh, wgpu::TextureFormat::Rgba8UnormSrgb, ldata, 4);
        let logo_aspect = world.logo.as_ref().map(|l| l.w as f32 / l.h as f32).unwrap_or(0.0);
        let pass_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pass"),
            size: std::mem::size_of::<Pass>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let ground_bind = tex_group(device, &pp_bgl, sampler, &logo_view, &pass_buf);
        let text_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("text"),
            size: (MAX_TEXT_VERTS * std::mem::size_of::<TextVertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut r = Renderer {
            device: device.clone(),
            queue: queue.clone(),
            format,
            scale,
            ring,
            sheet,
            sheet_unseen,
            veil_pipe,
            bgl,
            _blank: blank,
            soft: world.look.soft > 0.0,
            flow,
            cross,
            dot,
            mote,
            pp_bgl,
            sampler: veil_sampler,
            nearest,
            pixel: world.look.pixel,
            chunk: world.look.chunk.max(1),
            pass_buf,
            ground,
            ground_bind,
            blit,
            text,
            text_bind: None,
            text_buf,
            text_count: 0,
            text_static: 0,
            text_tick: -1,
            annot: None,
            logo_aspect,
            t: Targets { veil: None, k: 0, depth: tex2d(device, 1, 1, DEPTH, wgpu::TextureUsages::RENDER_ATTACHMENT).create_view(&Default::default()), mid: None, w: 0, h: 0 },
            out: (0, 0),
            n,
            len,
            m,
            seen_steps: 0,
            seen_epoch: u32::MAX,
        };
        r.resize(out);
        if world.cfg.annotations || world.cfg.trackers {
            r.build_annot();
        }
        r
    }

    fn build_annot(&mut self) {
        let Some(a) = Annot::new(self.t.h, self.chunk) else { return };
        let t = tex2d(&self.device, a.atlas_w, a.atlas_h, wgpu::TextureFormat::R8Unorm, wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST);
        self.queue.write_texture(
            t.as_image_copy(),
            &a.atlas,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(a.atlas_w), rows_per_image: Some(a.atlas_h) },
            wgpu::Extent3d { width: a.atlas_w, height: a.atlas_h, depth_or_array_layers: 1 },
        );
        let view = t.create_view(&wgpu::TextureViewDescriptor::default());
        self.text_bind = Some(tex_group(&self.device, &self.pp_bgl, if self.chunk > 1 { &self.nearest } else { &self.sampler }, &view, &self.pass_buf));
        self.annot = Some(a);
        self.text_tick = -1;
    }

    pub fn resize(&mut self, out: (u32, u32)) {
        if out.0 == 0 || out.1 == 0 || out == self.out {
            return;
        }
        self.out = out;
        let ((w, h), k) = scene_size(out, self.scale, self.pixel);
        let depth = tex2d(&self.device, w, h, DEPTH, wgpu::TextureUsages::RENDER_ATTACHMENT).create_view(&Default::default());
        let mid = ((w, h) != out).then(|| {
            let v = tex2d(&self.device, w, h, self.format, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING).create_view(&Default::default());
            let g = tex_group(&self.device, &self.pp_bgl, &self.sampler, &v, &self.pass_buf);
            (v, g)
        });
        let rebuilt = self.annot.is_some() && self.t.h != h;
        let veil = self.soft.then(|| {
            let (vw, vh) = (w.div_ceil(4), h.div_ceil(4));
            let view = tex2d(&self.device, vw, vh, VEIL, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING).create_view(&Default::default());
            let depth = tex2d(&self.device, vw, vh, DEPTH, wgpu::TextureUsages::RENDER_ATTACHMENT).create_view(&Default::default());
            let dot_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &self.bgl, entries: &strand_entries(&self.dot.buf, &self.ring, &view, &self.sampler) });
            Veil { view, depth, dot_bind }
        });
        self.t = Targets { veil, k, depth, mid, w, h };
        if rebuilt {
            self.build_annot();
        }
    }

    /// copy the points this renderer has not seen yet into the GPU ring
    fn upload(&mut self, world: &World) {
        let tr = &world.trails;
        let new = tr.steps.saturating_sub(self.seen_steps);
        if world.epoch != self.seen_epoch || new as usize >= self.len || tr.steps < self.seen_steps {
            self.queue.write_buffer(&self.ring, 0, bytemuck::cast_slice(&tr.ring));
        } else if new > 0 {
            let count = new as usize;
            let start = (tr.head + 1 + self.len - count) % self.len;
            let first = count.min(self.len - start);
            let row = |slot: usize, k: usize| &tr.ring[slot * self.n..(slot + k) * self.n];
            self.queue.write_buffer(&self.ring, (start * self.n * 16) as u64, bytemuck::cast_slice(row(start, first)));
            if first < count {
                self.queue.write_buffer(&self.ring, 0, bytemuck::cast_slice(row(0, count - first)));
            }
        }
        self.seen_steps = tr.steps;
        self.seen_epoch = world.epoch;
    }

    /// draw the world into `target` (which must be `format` and `out`-sized) and submit
    pub fn draw(&mut self, world: &World, target: &wgpu::TextureView) {
        self.upload(world);
        let look = &world.look;
        let (view, proj, dist) = world.camera(self.out.0 as f32 / self.out.1 as f32);
        let (sw, sh) = (self.t.w as f32, self.t.h as f32);
        let px = sh / 1080.0;
        let fogc = [(look.bg_top[0] + look.bg_bot[0]) * 0.5, (look.bg_top[1] + look.bg_bot[1]) * 0.5, (look.bg_top[2] + look.bg_bot[2]) * 0.5];
        let b4 = |c: [f32; 3], w: f32| [c[0], c[1], c[2], w];
        let (n, len, m) = (self.n as u32, self.len as u32, self.m as u32);
        let dot_mode = match look.dots {
            Dots::Dust => 0.0,
            Dots::Halftone => 1.0,
            Dots::Dissolve => 2.0,
        };
        // lines: the newest 45% of every strand point by point, the fading rest every other point
        let fresh = len * 2 / 5;
        // nothing is drawn past the age at which a look's lines have come apart
        let live = (((look.dissolve + 0.07).min(1.0) * len as f32) as u32).clamp(2, len);
        let flow_verts = 2 * (fresh.min(live) + live.saturating_sub(fresh).div_ceil(2));
        let sheet_wide = 4u32.min(m.max(2) - 1);
        let frame = |stride: u32| Frame {
            view: view.to_cols_array_2d(),
            proj: proj.to_cols_array_2d(),
            c0: b4(look.c[0], look.line * px * 0.5),
            c1: b4(look.c[1], look.cross * px * 0.5),
            c2: b4(look.c[2], look.dot * px),
            fog: b4(fogc, look.fog),
            acc: b4(look.accent, look.canvas),
            ids: [n, len, world.trails.head as u32, m],
            p1: [world.fade, fresh as f32, world.t, look.age_pow],
            // view units per pixel at depth 1: 2 / (height * proj[1][1])
            p2: [dist, 2.0 / (sh * proj.y_axis.y), sh, stride as f32],
            st: [look.shade, look.dof, look.dissolve, look.scatter],
            st2: [look.sparkle, look.focus, dot_mode, look.opacity],
            st3: [look.wash, look.scan, look.ribbon_colours as u32 as f32, sheet_wide as f32],
        };
        let sheet_stride = 3u32;
        let mut lf = frame(2);
        lf.st2[3] = look.opacity * look.line_alpha;
        self.queue.write_buffer(&self.flow.buf, 0, bytemuck::bytes_of(&lf));
        self.queue.write_buffer(&self.sheet.buf, 0, bytemuck::bytes_of(&frame(sheet_stride)));
        let mut cf = frame(look.cross_every.max(1));
        cf.st2[3] = look.opacity * look.line_alpha;
        self.queue.write_buffer(&self.cross.buf, 0, bytemuck::bytes_of(&cf));
        let mut mf = frame(1);
        mf.st2[3] = look.motes * world.cfg.overlay.then_some(0.0).unwrap_or(1.0);
        self.queue.write_buffer(&self.mote.buf, 0, bytemuck::bytes_of(&mf));
        let mut df = frame(look.dot_every.max(1));
        // in the dot pass this slot says how faint a fully covered dot becomes (0: nothing covers dots)
        df.st3[0] = if self.t.veil.is_some() { look.soft } else { 0.0 };
        self.queue.write_buffer(&self.dot.buf, 0, bytemuck::bytes_of(&df));

        let cfg = &world.cfg;
        let style = if self.logo_aspect <= 0.0 {
            0.0
        } else {
            match cfg.logo_style.as_str() {
                "outline" => 1.0,
                "fill" => 2.0,
                _ => 3.0,
            }
        };
        let tint = look.logo_tint.unwrap_or([1.0; 3]);
        let pass = Pass {
            v: [cfg.overlay as u32 as f32, sw, look.vignette, world.t],
            // bg0.w: the logo keeps its own SVG colours
            bg0: b4(look.bg_top, look.logo_tint.is_none() as u32 as f32),
            bg1: b4(look.bg_bot, 0.0),
            ex: [look.ground as f32, look.canvas, style, sh],
            // a wide logo is sized by its width, so it never outgrows the screen
            lg: [cfg.logo_x, cfg.logo_y, cfg.logo_size / self.logo_aspect.max(1.0), self.logo_aspect.max(0.01)],
            lt: [tint[0], tint[1], tint[2], cfg.logo_opacity],
            g0: b4(look.g0, 0.0),
            g1: b4(look.g1, 0.0),
            q: [look.quant, self.t.k as f32, look.scan, look.grain],
        };
        self.queue.write_buffer(&self.pass_buf, 0, bytemuck::bytes_of(&pass));

        // the annotations change a few times a second, and on every frame of a fade; the tracking boxes move, so they
        // are laid out every frame, after them in the same buffer
        if let Some(a) = &self.annot {
            let tick = if world.settled() { (world.t * 6.0) as i64 } else { -2 - (world.t * 1000.0) as i64 };
            if tick != self.text_tick {
                self.text_tick = tick;
                let v = if world.cfg.annotations { a.layout(world, (sw, sh)) } else { Vec::new() };
                let v = &v[..v.len().min(MAX_TEXT_VERTS / 2)];
                self.text_static = v.len() as u32;
                self.queue.write_buffer(&self.text_buf, 0, bytemuck::cast_slice(v));
            }
            let v = a.trackers(world, (sw, sh));
            let v = &v[..v.len().min(MAX_TEXT_VERTS / 2)];
            if !v.is_empty() {
                self.queue.write_buffer(&self.text_buf, self.text_static as u64 * std::mem::size_of::<TextVertex>() as u64, bytemuck::cast_slice(v));
            }
            self.text_count = self.text_static + v.len() as u32;
        }

        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let groups = n / m;
        let sheets = look.sheet && m > 1 && self.t.veil.is_none();
        let sheet_verts = 2 * (live / sheet_stride + 2);
        if let (Some(v), true) = (&self.t.veil, m > 1) {
            // the veil: how deep the nearest sheet lies at each place and how much it covers, for the dots to read
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &v.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: 1.0e4, g: 0.0, b: 0.0, a: 0.0 }), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &v.depth,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rp.set_pipeline(&self.veil_pipe);
            rp.set_bind_group(0, &self.sheet.bind, &[]);
            rp.draw(0..sheet_verts, 0..groups * ((m - 2) / sheet_wide + 1));
        }
        {
            let color = self.t.mid.as_ref().map(|m| &m.0).unwrap_or(target);
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: color,
                    depth_slice: None,
                    resolve_target: None,
                    // the ground covers every pixel, so there is nothing to clear first
                    // SAFETY: every pixel of the attachment is written by the ground's full-screen triangle
                    ops: wgpu::Operations { load: wgpu::LoadOp::DontCare(unsafe { wgpu::LoadOpDontCare::enabled() }), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.t.depth,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rp.set_pipeline(&self.ground);
            rp.set_bind_group(0, &self.ground_bind, &[]);
            rp.draw(0..3, 0..1);
            if let (Some(bind), true) = (&self.text_bind, self.text_count > 0) {
                rp.set_pipeline(&self.text);
                rp.set_bind_group(0, bind, &[]);
                rp.set_vertex_buffer(0, self.text_buf.slice(..));
                rp.draw(0..self.text_count, 0..1);
            }
            if look.motes > 0.0 {
                rp.set_pipeline(&self.mote.pipe);
                rp.set_bind_group(0, &self.mote.bind, &[]);
                rp.draw(0..3, 0..320);
            }
            if sheets {
                rp.set_pipeline(if look.wash > 0.0 { &self.sheet.pipe } else { &self.sheet_unseen });
                rp.set_bind_group(0, &self.sheet.bind, &[]);
                rp.draw(0..sheet_verts, 0..groups * ((m - 2) / sheet_wide + 1));
            }
            if look.line > 0.0 {
                rp.set_pipeline(&self.flow.pipe);
                rp.set_bind_group(0, &self.flow.bind, &[]);
                rp.draw(0..flow_verts, 0..n);
            }
            if look.cross > 0.0 && m > 1 {
                rp.set_pipeline(&self.cross.pipe);
                rp.set_bind_group(0, &self.cross.bind, &[]);
                rp.draw(0..2 * m, 0..groups * (live / look.cross_every.max(1)));
            }
            if look.dot > 0.0 {
                let every = look.dot_every.max(1);
                let rows = len / every;
                // dots that only appear where lines dissolve need only the tail's rows
                let first = if look.dots == Dots::Dissolve { (((look.dissolve - 0.14).max(0.0) * len as f32) as u32 / every).min(rows) } else { 0 };
                rp.set_pipeline(&self.dot.pipe);
                rp.set_bind_group(0, self.t.veil.as_ref().map(|v| &v.dot_bind).unwrap_or(&self.dot.bind), &[]);
                rp.draw(0..3, first * n..rows * n);
            }
        }
        if let Some((_, group)) = &self.t.mid {
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rp.set_pipeline(&self.blit);
            rp.set_bind_group(0, group, &[]);
            rp.draw(0..3, 0..1);
        }
        self.queue.submit(Some(enc.finish()));
    }
}
