//! Screensaver frontend: a fullscreen winit window per instance. Uses Omarchy's screensaver app-id so its idle
//! service still locks on time and treats dismissing the saver as activity. Any key, click or real mouse movement quits.

use crate::config::{Config, Mode};
use crate::{gpu, render::Renderer, world::World};
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Fullscreen, Window, WindowId};

const APP_ID: &str = "org.omarchy.screensaver";

struct Gfx {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    renderer: Renderer,
}

struct App {
    cfg: Config,
    world: World,
    windowed: bool,
    /// ignore input (for testing: the saver normally quits on any key, click or real mouse movement)
    keep: bool,
    gfx: Option<Gfx>,
    last: Instant,
    next_frame: Instant,
    frame_dt: Duration,
    grace_until: Instant,
    pointer0: Option<(f64, f64)>,
    frames: (u32, Instant),
}

impl Gfx {
    fn new(window: Arc<Window>, cfg: &Config, world: &World) -> Gfx {
        let instance = gpu::instance();
        let surface = instance.create_surface(window.clone()).expect("surface");
        let adapter = gpu::adapter(&instance, Some(&surface), cfg);
        let (device, queue) = gpu::device(&adapter);
        let size = window.inner_size();
        let caps = surface.get_capabilities(&adapter);
        let format = gpu::surface_format(&caps);
        let mut config = surface.get_default_config(&adapter, size.width.max(1), size.height.max(1)).expect("config");
        config.format = format;
        config.present_mode = wgpu::PresentMode::Fifo;
        config.desired_maximum_frame_latency = 1;
        surface.configure(&device, &config);
        let scale = gpu::render_scale(cfg, Mode::Saver, config.width, config.height);
        if gpu::debug() {
            eprintln!("attractor: {:?} {}x{}, scene at {:.0}%", format, config.width, config.height, scale * 100.0);
        }
        let renderer = Renderer::new(&device, &queue, format, (config.width, config.height), scale, world);
        Gfx { window, surface, device, queue, config, renderer }
    }

    fn resize(&mut self, w: u32, h: u32) {
        if w > 0 && h > 0 {
            self.config.width = w;
            self.config.height = h;
            self.surface.configure(&self.device, &self.config);
            self.renderer.resize((w, h));
        }
    }
}

impl App {
    fn pace(&mut self) {
        // on battery a screensaver shouldn't drain the laptop
        let fps = if gpu::on_mains() { self.cfg.fps } else { self.cfg.fps.min(30.0) };
        self.frame_dt = Duration::from_secs_f64(1.0 / fps.clamp(10.0, 240.0) as f64);
    }

    fn render(&mut self) {
        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f32();
        self.last = now;
        self.world.update(dt);
        let Some(g) = self.gfx.as_mut() else { return };
        let frame = match g.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                g.surface.configure(&g.device, &g.config);
                return;
            }
            _ => return,
        };
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
        g.renderer.draw(&self.world, &view);
        g.window.pre_present_notify();
        g.queue.present(frame);
        if gpu::debug() {
            self.frames.0 += 1;
            if now.duration_since(self.frames.1) >= Duration::from_secs(2) {
                eprintln!("attractor: {:.1} fps ({})", self.frames.0 as f32 / now.duration_since(self.frames.1).as_secs_f32(), self.world.system_name());
                self.frames = (0, now);
            }
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.gfx.is_some() {
            return;
        }
        let mut attrs = Window::default_attributes().with_title(APP_ID).with_decorations(false);
        {
            use winit::platform::wayland::WindowAttributesExtWayland;
            attrs = attrs.with_name(APP_ID, APP_ID);
        }
        attrs = if self.windowed { attrs.with_inner_size(winit::dpi::LogicalSize::new(1280.0, 800.0)) } else { attrs.with_fullscreen(Some(Fullscreen::Borderless(None))) };
        let window = Arc::new(el.create_window(attrs).expect("window"));
        if !self.windowed {
            window.set_cursor_visible(false);
        }
        self.gfx = Some(Gfx::new(window, &self.cfg, &self.world));
        self.pace();
        let now = Instant::now();
        self.last = now;
        self.next_frame = now;
        self.grace_until = now + Duration::from_millis(600);
        self.frames = (0, now);
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, ev: WindowEvent) {
        let live = !self.windowed && !self.keep && Instant::now() > self.grace_until;
        match ev {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(s) => {
                if let Some(g) = self.gfx.as_mut() {
                    g.resize(s.width, s.height);
                }
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                if self.windowed {
                    if matches!(event.logical_key, winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape)) {
                        el.exit();
                    }
                } else if live {
                    el.exit();
                }
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, .. } | WindowEvent::Touch(_) if live => el.exit(),
            WindowEvent::CursorMoved { position, .. } if live => match self.pointer0 {
                None => self.pointer0 = Some((position.x, position.y)),
                Some((x, y)) => {
                    if (position.x - x).abs() > 18.0 || (position.y - y).abs() > 18.0 {
                        el.exit();
                    }
                }
            },
            WindowEvent::RedrawRequested => self.render(),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        let now = Instant::now();
        if now >= self.next_frame {
            self.next_frame = (self.next_frame + self.frame_dt).max(now);
            if let Some(g) = &self.gfx {
                g.window.request_redraw();
            }
        }
        el.set_control_flow(ControlFlow::WaitUntil(self.next_frame));
    }
}

pub fn run(cfg: Config, seed: u64, windowed: bool, keep: bool) {
    let el = EventLoop::new().expect("event loop");
    let now = Instant::now();
    let mut app = App {
        world: World::new(&cfg, seed),
        cfg,
        windowed,
        keep,
        gfx: None,
        last: now,
        next_frame: now,
        frame_dt: Duration::from_millis(16),
        grace_until: now,
        pointer0: None,
        frames: (0, now),
    };
    el.run_app(&mut app).expect("run");
}
