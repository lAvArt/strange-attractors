//! The GPU side: trails as instanced ribbons into an HDR target, a two-level bloom, and a composite onto the
//! gradient ground. Knows nothing about windows: it draws into whatever view it is given.

use crate::world::World;
use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

pub const HDR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Frame {
    vp: [[f32; 4]; 4],
    c0: [f32; 4],
    c1: [f32; 4],
    c2: [f32; 4],
    ids: [u32; 4],
    p1: [f32; 4],
    res: [f32; 4],
    p2: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Pass {
    v: [f32; 4],
    bg0: [f32; 4],
    bg1: [f32; 4],
    ex: [f32; 4],
}

struct Targets {
    scene: wgpu::TextureView,
    l1a: wgpu::TextureView,
    l1b: wgpu::TextureView,
    l2a: wgpu::TextureView,
    l2b: wgpu::TextureView,
    /// down1, blurH1, blurV1, down2, blurH2, blurV2, composite
    groups: Vec<wgpu::BindGroup>,
    comp_buf: wgpu::Buffer,
    w: u32,
    h: u32,
}

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    scale: f32,
    pipe: wgpu::RenderPipeline,
    bind: wgpu::BindGroup,
    frame_buf: wgpu::Buffer,
    ring: wgpu::Buffer,
    pp_bgl: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    down: wgpu::RenderPipeline,
    blur: wgpu::RenderPipeline,
    comp: wgpu::RenderPipeline,
    t: Targets,
    out: (u32, u32),
    n: usize,
    len: usize,
    seen_steps: u64,
    seen_epoch: u32,
}

fn tex(device: &wgpu::Device, w: u32, h: u32) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HDR,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

fn pass_buf(device: &wgpu::Device, p: Pass) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::bytes_of(&p),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    })
}

fn build_targets(device: &wgpu::Device, bgl: &wgpu::BindGroupLayout, sampler: &wgpu::Sampler, w: u32, h: u32) -> Targets {
    let (w1, h1) = ((w / 4).max(1), (h / 4).max(1));
    let (w2, h2) = ((w1 / 4).max(1), (h1 / 4).max(1));
    let scene = tex(device, w, h);
    let (l1a, l1b, l2a, l2b) = (tex(device, w1, h1), tex(device, w1, h1), tex(device, w2, h2), tex(device, w2, h2));
    let z = [0.0; 4];
    let p = |v: [f32; 4]| pass_buf(device, Pass { v, bg0: z, bg1: z, ex: z });
    let u_down1 = p([1.0 / w as f32, 1.0 / h as f32, 0.55, 0.0]);
    let u_bh1 = p([1.0 / w1 as f32, 0.0, 0.0, 0.0]);
    let u_bv1 = p([0.0, 1.0 / h1 as f32, 0.0, 0.0]);
    let u_down2 = p([1.0 / w1 as f32, 1.0 / h1 as f32, 0.0, 0.0]);
    let u_bh2 = p([1.0 / w2 as f32, 0.0, 0.0, 0.0]);
    let u_bv2 = p([0.0, 1.0 / h2 as f32, 0.0, 0.0]);
    let comp_buf = pass_buf(device, Pass { v: z, bg0: z, bg1: z, ex: z });
    let bg = |src: &wgpu::TextureView, u: &wgpu::Buffer, t1: &wgpu::TextureView, t2: &wgpu::TextureView| {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(src) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: u.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(t1) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(t2) },
            ],
        })
    };
    let groups = vec![
        bg(&scene, &u_down1, &scene, &scene),
        bg(&l1a, &u_bh1, &scene, &scene),
        bg(&l1b, &u_bv1, &scene, &scene),
        bg(&l1a, &u_down2, &scene, &scene),
        bg(&l2a, &u_bh2, &scene, &scene),
        bg(&l2b, &u_bv2, &scene, &scene),
        bg(&scene, &comp_buf, &l1a, &l2a),
    ];
    Targets { scene, l1a, l1b, l2a, l2b, groups, comp_buf, w, h }
}

fn render_pass<'a>(enc: &'a mut wgpu::CommandEncoder, view: &'a wgpu::TextureView, clear: wgpu::Color) -> wgpu::RenderPass<'a> {
    enc.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: None,
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations { load: wgpu::LoadOp::Clear(clear), store: wgpu::StoreOp::Store },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    })
}

impl Renderer {
    /// `out` is the target size in pixels, `scale` the fraction of it the scene (and its bloom) renders at.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat, out: (u32, u32), scale: f32, world: &World) -> Self {
        let (n, len) = (world.trails.n, world.trails.len);
        let shader = device.create_shader_module(wgpu::include_wgsl!("shader.wgsl"));
        let ring = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ring"),
            size: (n * len * 16) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let frame_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("frame"),
            size: std::mem::size_of::<Frame>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
            ],
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: frame_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: ring.as_entire_binding() },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(&bgl)], immediate_size: 0 });
        let blend = if world.look.over {
            wgpu::BlendState { color: wgpu::BlendComponent::OVER, alpha: wgpu::BlendComponent::OVER }
        } else {
            wgpu::BlendState {
                color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
                alpha: wgpu::BlendComponent::OVER,
            }
        };
        let pipe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("trails"),
            layout: Some(&layout),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_main"), compilation_options: Default::default(), buffers: &[] },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: HDR, blend: Some(blend), write_mask: wgpu::ColorWrites::ALL })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let pp_shader = device.create_shader_module(wgpu::include_wgsl!("pp.wgsl"));
        let tex_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
            count: None,
        };
        let pp_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                tex_entry(0),
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                tex_entry(3),
                tex_entry(4),
            ],
        });
        let pp_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(&pp_bgl)], immediate_size: 0 });
        let pp_make = |fs: &str, fmt: wgpu::TextureFormat| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(fs),
                layout: Some(&pp_layout),
                vertex: wgpu::VertexState { module: &pp_shader, entry_point: Some("vs_full"), compilation_options: Default::default(), buffers: &[] },
                fragment: Some(wgpu::FragmentState {
                    module: &pp_shader,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format: fmt, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let (sw, sh) = (((out.0 as f32 * scale) as u32).max(1), ((out.1 as f32 * scale) as u32).max(1));
        let t = build_targets(device, &pp_bgl, &sampler, sw, sh);
        Renderer {
            device: device.clone(),
            queue: queue.clone(),
            scale,
            pipe,
            bind,
            frame_buf,
            ring,
            down: pp_make("fs_down", HDR),
            blur: pp_make("fs_blur", HDR),
            comp: pp_make("fs_comp", format),
            pp_bgl,
            sampler,
            t,
            out,
            n,
            len,
            seen_steps: 0,
            seen_epoch: u32::MAX,
        }
    }

    pub fn resize(&mut self, out: (u32, u32)) {
        if out.0 == 0 || out.1 == 0 || out == self.out {
            return;
        }
        self.out = out;
        let (sw, sh) = (((out.0 as f32 * self.scale) as u32).max(1), ((out.1 as f32 * self.scale) as u32).max(1));
        self.t = build_targets(&self.device, &self.pp_bgl, &self.sampler, sw, sh);
    }

    /// copy the points this renderer has not seen yet into the GPU ring
    fn upload(&mut self, world: &World) {
        let tr = &world.trails;
        let new = tr.steps.saturating_sub(self.seen_steps);
        if world.epoch != self.seen_epoch || new as usize >= self.len {
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
        let (vp, dist) = world.camera(self.out.0 as f32 / self.out.1 as f32);
        let (sw, sh) = (self.t.w as f32, self.t.h as f32);
        let c4 = |c: [f32; 3]| [c[0], c[1], c[2], 0.0];
        let frame = Frame {
            vp: vp.to_cols_array_2d(),
            c0: c4(look.c[0]),
            c1: c4(look.c[1]),
            c2: c4(look.c[2]),
            ids: [self.n as u32, self.len as u32, world.trails.head as u32, look.over as u32],
            // line width scales with the scene height so the look is the same at any resolution
            p1: [look.width * sh / 1080.0, look.intensity, world.fade, look.age_pow],
            res: [sw, sh, 1.0 / sw, 1.0 / sh],
            p2: [dist, 0.0, 0.0, 0.0],
        };
        self.queue.write_buffer(&self.frame_buf, 0, bytemuck::bytes_of(&frame));
        let bloom = if look.over { 0.0 } else { look.bloom };
        let b4 = |c: [f32; 3]| [c[0], c[1], c[2], 1.0];
        let comp = Pass {
            v: [look.over as u32 as f32, bloom, look.vignette, world.t],
            bg0: b4(look.bg_top),
            bg1: b4(look.bg_bot),
            ex: [look.exposure, world.cfg.overlay as u32 as f32, 0.0, 0.0],
        };
        self.queue.write_buffer(&self.t.comp_buf, 0, bytemuck::bytes_of(&comp));

        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut rp = render_pass(&mut enc, &self.t.scene, wgpu::Color::TRANSPARENT);
            rp.set_pipeline(&self.pipe);
            rp.set_bind_group(0, &self.bind, &[]);
            rp.draw(0..6, 0..(self.n * (self.len - 1)) as u32);
        }
        let t = &self.t;
        let fs = |enc: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, pipe: &wgpu::RenderPipeline, bg: &wgpu::BindGroup| {
            let mut rp = render_pass(enc, view, wgpu::Color::BLACK);
            rp.set_pipeline(pipe);
            rp.set_bind_group(0, bg, &[]);
            rp.draw(0..3, 0..1);
        };
        if bloom > 0.0 {
            fs(&mut enc, &t.l1a, &self.down, &t.groups[0]);
            fs(&mut enc, &t.l1b, &self.blur, &t.groups[1]);
            fs(&mut enc, &t.l1a, &self.blur, &t.groups[2]);
            fs(&mut enc, &t.l2a, &self.down, &t.groups[3]);
            fs(&mut enc, &t.l2b, &self.blur, &t.groups[4]);
            fs(&mut enc, &t.l2a, &self.blur, &t.groups[5]);
        }
        fs(&mut enc, target, &self.comp, &t.groups[6]);
        self.queue.submit(Some(enc.finish()));
    }
}
