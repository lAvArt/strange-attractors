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

/// `--flicker N`: render N consecutive frames with the real pacing (`fps`, the configured speed, so the number of
/// simulation steps per frame alternates as it does live) and measure how much the picture changes from one frame to
/// the next. With `hold` the camera stands still: every drawn point is then fixed in space, so apart from heads
/// growing and tails fading nothing should change at all, and whatever does is flicker. Six consecutive frames are
/// saved as `<prefix>-0.png` .. `<prefix>-5.png`.
pub fn flicker(cfg: &Config, mode: Mode, prefix: &str, size: (u32, u32), frames: u32, fps: f32, hold: bool, seed: u64) {
    let instance = gpu::instance();
    let adapter = gpu::adapter(&instance, None, cfg);
    let (device, queue) = gpu::device(&adapter);
    let mut world = World::new(cfg, seed);
    for _ in 0..(20.0 * fps) as usize {
        world.update(1.0 / fps);
    }
    world.hold_camera = hold;
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let scale = gpu::render_scale(cfg, mode, size.0, size.1);
    let mut r = Renderer::new(&device, &queue, format, size, scale, &world);
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("flicker"),
        size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let row = (size.0 * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buf = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: (row * size.1) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
    let grab = |world: &World, r: &mut Renderer| -> Vec<u8> {
        r.draw(world, &view);
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        enc.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(size.1) } },
            wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
        );
        queue.submit(Some(enc.finish()));
        buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        let _ = device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
        let mut px = Vec::with_capacity((size.0 * size.1 * 4) as usize);
        {
            let data = buf.slice(..).get_mapped_range().expect("map");
            for y in 0..size.1 as usize {
                px.extend_from_slice(&data[y * row as usize..y * row as usize + size.0 as usize * 4]);
            }
        }
        buf.unmap();
        px
    };
    // the middle of the picture, where the form is: the margins hold the (changing) numbers
    let (x0, x1, y0, y1) = (size.0 as usize / 4, size.0 as usize * 3 / 4, size.1 as usize / 6, size.1 as usize * 5 / 6);
    let lum = |p: &[u8], i: usize| (p[i] as i32 * 2 + p[i + 1] as i32 * 5 + p[i + 2] as i32) / 8;
    let mut prev = grab(&world, &mut r);
    let ground = lum(&prev, (y0 * size.0 as usize + x0) * 4);
    let (mut sum_mean, mut sum_pop, mut worst_pop, mut ink) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for f in 0..frames {
        world.update(1.0 / fps);
        let cur = grab(&world, &mut r);
        let (mut total, mut drawn, mut pops) = (0u64, 0u64, 0u64);
        for y in y0..y1 {
            for x in x0..x1 {
                let i = (y * size.0 as usize + x) * 4;
                let (a, b) = (lum(&prev, i), lum(&cur, i));
                if (a - ground).abs() > 10 || (b - ground).abs() > 10 {
                    drawn += 1;
                    let d = (a - b).unsigned_abs() as u64;
                    total += d;
                    if d > 48 {
                        pops += 1;
                    }
                }
            }
        }
        let drawn = drawn.max(1) as f64;
        sum_mean += total as f64 / drawn;
        sum_pop += pops as f64 / drawn;
        worst_pop = worst_pop.max(pops as f64 / drawn);
        ink += drawn;
        if (30..36).contains(&f) {
            image::save_buffer(format!("{prefix}-{}.png", f - 30), &cur, size.0, size.1, image::ColorType::Rgba8).expect("save png");
        }
        prev = cur;
    }
    let n = frames as f64;
    println!(
        "{:>9} {:>8} {}x{} {:.0} fps {}: drawn pixels {:.0}, mean change {:.2} levels/frame, popping pixels (>48 levels) {:.3}% (worst frame {:.3}%)",
        world.system_name(),
        world.look.name,
        size.0,
        size.1,
        fps,
        if hold { "camera held" } else { "camera moving" },
        ink / n,
        sum_mean / n,
        100.0 * sum_pop / n,
        100.0 * worst_pop
    );
}

/// `--dark-check`: draw one frame, then the same frame with the form faded out (the bare ground), and count the
/// pixels in the middle of the picture where the form made the ground darker (on a dark ground; lighter, on a light
/// one). Marks should only ever add light to a dark ground or ink to a light one.
pub fn dark_check(cfg: &Config, mode: Mode, size: (u32, u32), secs: f32, seed: u64) {
    let instance = gpu::instance();
    let adapter = gpu::adapter(&instance, None, cfg);
    let (device, queue) = gpu::device(&adapter);
    let mut world = World::new(cfg, seed);
    for _ in 0..(secs * 30.0) as usize {
        world.update(1.0 / 30.0);
    }
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let mut r = Renderer::new(&device, &queue, format, size, gpu::render_scale(cfg, mode, size.0, size.1), &world);
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("dark"),
        size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let row = (size.0 * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buf = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: (row * size.1) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
    let grab = |world: &World, r: &mut Renderer| -> Vec<u8> {
        r.draw(world, &view);
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        enc.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(size.1) } },
            wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
        );
        queue.submit(Some(enc.finish()));
        buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        let _ = device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
        let mut px = Vec::with_capacity((size.0 * size.1 * 4) as usize);
        {
            let data = buf.slice(..).get_mapped_range().expect("map");
            for y in 0..size.1 as usize {
                px.extend_from_slice(&data[y * row as usize..y * row as usize + size.0 as usize * 4]);
            }
        }
        buf.unmap();
        px
    };
    let with = grab(&world, &mut r);
    world.fade = 0.0;
    let bare = grab(&world, &mut r);
    let lum = |p: &[u8], i: usize| (p[i] as i32 * 2 + p[i + 1] as i32 * 5 + p[i + 2] as i32) / 8;
    let (x0, x1, y0, y1) = (size.0 as usize / 5, size.0 as usize * 4 / 5, size.1 as usize / 8, size.1 as usize * 7 / 8);
    let light_ground = lum(&bare, (y0 * size.0 as usize + x0) * 4) > 128;
    let (mut drawn, mut wrong, mut worst) = (0u64, 0u64, 0i32);
    for y in y0..y1 {
        for x in x0..x1 {
            let i = (y * size.0 as usize + x) * 4;
            let d = lum(&with, i) - lum(&bare, i);
            if d.abs() > 3 {
                drawn += 1;
            }
            // the ground is dithered by a step or so: anything beyond that is a mark of the wrong sign
            let bad = if light_ground { d } else { -d };
            if bad > 3 {
                wrong += 1;
                worst = worst.max(bad);
            }
        }
    }
    println!(
        "{:>9} {:>8} on a {} ground: {} pixels drawn, {} of them {} than the ground (worst by {} levels)",
        world.system_name(),
        world.look.name,
        if light_ground { "light" } else { "dark" },
        drawn,
        wrong,
        if light_ground { "lighter" } else { "darker" },
        worst
    );
}
