//! `--shot out.png`: simulate some seconds offscreen and save one frame. Used to check looks without a display.

use crate::config::{Config, Mode};
use crate::{gpu, render::Renderer, world::World};

pub fn run(cfg: &Config, mode: Mode, path: &str, size: (u32, u32), secs: f32, seed: u64) {
    let instance = gpu::instance();
    let adapter = gpu::adapter(&instance, None, cfg);
    let (device, queue) = gpu::device(&adapter);
    let mut world = World::new(cfg, seed);
    let steps = (secs * 30.0) as usize;
    for _ in 0..steps {
        world.update(1.0 / 30.0);
    }
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let scale = gpu::render_scale(cfg, mode, size.0, size.1);
    let mut r = Renderer::new(&device, &queue, format, size, scale, &world);
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("shot"),
        size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    r.draw(&world, &view);

    let row = (size.0 * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buf = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: (row * size.1) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    enc.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(size.1) } },
        wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
    );
    queue.submit(Some(enc.finish()));
    buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    let _ = device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
    let data = buf.slice(..).get_mapped_range().expect("map");
    let mut px = Vec::with_capacity((size.0 * size.1 * 4) as usize);
    for y in 0..size.1 as usize {
        px.extend_from_slice(&data[y * row as usize..y * row as usize + size.0 as usize * 4]);
    }
    image::save_buffer(path, &px, size.0, size.1, image::ColorType::Rgba8).expect("save png");
    println!("{} ({}) -> {}", world.system_name(), cfg.preset, path);
}

/// `--bench N`: render N frames offscreen as fast as the GPU goes and report the time per frame.
pub fn bench(cfg: &Config, mode: Mode, size: (u32, u32), frames: u32, seed: u64) {
    let instance = gpu::instance();
    let adapter = gpu::adapter(&instance, None, cfg);
    let (device, queue) = gpu::device(&adapter);
    let mut world = World::new(cfg, seed);
    let format = wgpu::TextureFormat::Bgra8UnormSrgb;
    let scale = gpu::render_scale(cfg, mode, size.0, size.1);
    let mut r = Renderer::new(&device, &queue, format, size, scale, &world);
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("bench"),
        size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let wait = || {
        let _ = device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
    };
    for _ in 0..10 {
        world.update(1.0 / 30.0);
        r.draw(&world, &view);
    }
    wait();
    let (mut cpu_sim, mut total) = (std::time::Duration::ZERO, std::time::Duration::ZERO);
    for _ in 0..frames {
        let t0 = std::time::Instant::now();
        world.update(1.0 / 30.0);
        cpu_sim += t0.elapsed();
        r.draw(&world, &view);
        wait();
        total += t0.elapsed();
    }
    let ms = |d: std::time::Duration| d.as_secs_f64() * 1000.0 / frames as f64;
    println!(
        "{:>9} {:>9} {}x{} @{:.0}%  {:>6.2} ms/frame  (sim {:.2} ms, gpu+submit {:.2} ms)  -> {} strands x {} points, lyapunov {:.3}",
        world.system_name(),
        world.look.name,
        size.0,
        size.1,
        scale * 100.0,
        ms(total),
        ms(cpu_sim),
        ms(total) - ms(cpu_sim),
        world.trails.n,
        world.trails.len,
        world.trails.lyapunov()
    );
}
