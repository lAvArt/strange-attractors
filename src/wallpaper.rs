//! Live wallpaper frontend: one wlr-layer-shell surface per monitor on the *bottom* layer (above the shell's own
//! background layer, below every window), with an empty input region so clicks reach the desktop underneath.
//! It pauses itself when it cannot be seen: a fullscreen window, a locked session or a sleeping monitor.

use crate::config::{Config, Mode};
use crate::{gpu, render::Renderer, world::World};
use raw_window_handle::{RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, Region},
    delegate_compositor, delegate_layer, delegate_output, delegate_registry,
    output::{OutputHandler, OutputState},
    reexports::{
        calloop::{
            EventLoop,
            channel::{self, Channel},
            timer::{TimeoutAction, Timer},
        },
        calloop_wayland_source::WaylandSource,
        client::{
            Connection, Proxy, QueueHandle,
            globals::registry_queue_init,
            protocol::{wl_output, wl_surface},
        },
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        WaylandSurface,
        wlr_layer::{Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure},
    },
};
use std::collections::HashSet;
use std::ffi::c_void;
use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixStream;
use std::ptr::NonNull;
use std::time::{Duration, Instant};

/// What the watcher thread knows about the desktop.
#[derive(Clone, PartialEq, Default)]
struct Desk {
    /// the session is locked
    locked: bool,
    /// monitors whose wallpaper is hidden (fullscreen window on top, or the monitor asleep)
    covered: HashSet<String>,
}

fn hypr_json(what: &str) -> Vec<serde_json::Value> {
    std::process::Command::new("hyprctl")
        .args(["-j", what])
        .output()
        .ok()
        .and_then(|o| serde_json::from_slice::<serde_json::Value>(&o.stdout).ok())
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
}

fn query_covered() -> HashSet<String> {
    let monitors = hypr_json("monitors");
    let workspaces = hypr_json("workspaces");
    let fullscreen = |id: i64| workspaces.iter().any(|w| w["id"].as_i64() == Some(id) && w["hasfullscreen"].as_bool() == Some(true));
    monitors
        .iter()
        .filter(|m| {
            let active = m["activeWorkspace"]["id"].as_i64().unwrap_or(0);
            let special = m["specialWorkspace"]["id"].as_i64().unwrap_or(0);
            m["dpmsStatus"].as_bool() == Some(false) || fullscreen(active) || (special != 0 && fullscreen(special))
        })
        .filter_map(|m| m["name"].as_str().map(String::from))
        .collect()
}

fn query_locked() -> bool {
    std::process::Command::new("omarchy-shell").args(["lock", "isLocked"]).output().map(|o| String::from_utf8_lossy(&o.stdout).trim() == "true").unwrap_or(false)
}

/// Follows Hyprland's event socket and re-reads the desktop state when something relevant changes; also polls
/// every few seconds, since a lock or a monitor going to sleep may not produce an event of its own.
fn spawn_watcher(tx: channel::Sender<Desk>) {
    std::thread::spawn(move || {
        let sock = match (std::env::var("XDG_RUNTIME_DIR"), std::env::var("HYPRLAND_INSTANCE_SIGNATURE")) {
            (Ok(d), Ok(s)) => format!("{d}/hypr/{s}/.socket2.sock"),
            _ => String::new(),
        };
        let mut last = Desk::default();
        let send = |d: Desk, last: &mut Desk| -> bool {
            if d != *last {
                *last = d.clone();
                return tx.send(d).is_ok();
            }
            true
        };
        // the first answer straight away, not after the first poll interval
        if !send(Desk { locked: query_locked(), covered: query_covered() }, &mut last) {
            return;
        }
        loop {
            if let Ok(stream) = UnixStream::connect(&sock) {
                let slow = Some(Duration::from_secs(4));
                let quick = Some(Duration::from_millis(120));
                stream.set_read_timeout(slow).ok();
                let mut r = BufReader::new(stream);
                let mut line = String::new();
                let mut pending = false;
                loop {
                    let mut periodic = false;
                    match r.read_line(&mut line) {
                        Ok(0) => break,
                        Ok(_) if line.ends_with('\n') => {
                            const RELEVANT: [&str; 9] = ["fullscreen>>", "workspace", "focusedmon", "activespecial", "openwindow", "closewindow", "movewindow", "monitor", "dpms"];
                            if RELEVANT.iter().any(|k| line.starts_with(k)) && !pending {
                                // events arrive in bursts: settle for a moment, then read the state once
                                pending = true;
                                r.get_ref().set_read_timeout(quick).ok();
                            }
                            line.clear();
                        }
                        Ok(_) => {}
                        Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                            periodic = !pending;
                            if pending {
                                pending = false;
                                r.get_ref().set_read_timeout(slow).ok();
                                let d = Desk { locked: last.locked, covered: query_covered() };
                                if !send(d, &mut last) {
                                    return;
                                }
                            }
                        }
                        Err(_) => break,
                    }
                    if periodic {
                        let d = Desk { locked: query_locked(), covered: query_covered() };
                        if !send(d, &mut last) {
                            return;
                        }
                    }
                }
            } else {
                // not under Hyprland (or its socket is not up yet): still watch the lock
                let d = Desk { locked: query_locked(), covered: HashSet::new() };
                if !send(d, &mut last) {
                    return;
                }
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    });
}

struct OutGfx {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    renderer: Renderer,
}

struct Out {
    // dropped first: the GPU surface must go before the Wayland surface it draws to
    gfx: Option<OutGfx>,
    layer: LayerSurface,
    output: wl_output::WlOutput,
    name: String,
    /// logical size from the last configure
    size: (u32, u32),
    scale: i32,
}

struct Gpu {
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

struct App {
    registry_state: RegistryState,
    output_state: OutputState,
    compositor: CompositorState,
    layer_shell: LayerShell,
    conn: Connection,
    qh: QueueHandle<App>,
    cfg: Config,
    world: World,
    instance: wgpu::Instance,
    gpu: Option<Gpu>,
    outs: Vec<Out>,
    desk: Desk,
    last: Instant,
    ticks: u32,
    mains: bool,
    frames: (u32, Instant),
}

impl App {
    fn add_output(&mut self, output: wl_output::WlOutput) {
        if self.outs.iter().any(|o| o.output == output) {
            return;
        }
        let name = self.output_state.info(&output).and_then(|i| i.name).unwrap_or_default();
        let surface = self.compositor.create_surface(&self.qh);
        let layer = self.layer_shell.create_layer_surface(&self.qh, surface, Layer::Bottom, Some("attractor-wallpaper"), Some(&output));
        layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.set_size(0, 0);
        // an empty input region: pointer and touch go straight through to whatever is below
        if let Ok(region) = Region::new(&self.compositor) {
            layer.set_input_region(Some(region.wl_region()));
        }
        layer.commit();
        if gpu::debug() {
            eprintln!("attractor: output {name} added");
        }
        self.outs.push(Out { gfx: None, layer, output, name, size: (0, 0), scale: 1 });
    }

    /// (re)build the GPU side of output `i` for its current size and scale
    fn setup_gfx(&mut self, i: usize) {
        let (w, h, scale) = {
            let o = &self.outs[i];
            (o.size.0 * o.scale as u32, o.size.1 * o.scale as u32, o.scale)
        };
        if w == 0 || h == 0 {
            return;
        }
        self.outs[i].layer.wl_surface().set_buffer_scale(scale);
        if let Some(g) = self.outs[i].gfx.as_mut() {
            g.config.width = w;
            g.config.height = h;
            let dev = &self.gpu.as_ref().unwrap().device;
            g.surface.configure(dev, &g.config);
            g.renderer.resize((w, h));
            return;
        }
        let display = NonNull::new(self.conn.backend().display_ptr() as *mut c_void).expect("wl_display");
        let surf = NonNull::new(self.outs[i].layer.wl_surface().id().as_ptr() as *mut c_void).expect("wl_surface");
        let target = wgpu::SurfaceTargetUnsafe::RawHandle {
            raw_display_handle: Some(RawDisplayHandle::Wayland(WaylandDisplayHandle::new(display))),
            raw_window_handle: RawWindowHandle::Wayland(WaylandWindowHandle::new(surf)),
        };
        // SAFETY: both pointers stay valid as long as the connection and the layer surface, which outlive `gfx`
        let surface = unsafe { self.instance.create_surface_unsafe(target) }.expect("surface");
        if self.gpu.is_none() {
            let adapter = gpu::adapter(&self.instance, Some(&surface), &self.cfg);
            let (device, queue) = gpu::device(&adapter);
            self.gpu = Some(Gpu { adapter, device, queue });
        }
        let g = self.gpu.as_ref().unwrap();
        let caps = surface.get_capabilities(&g.adapter);
        let format = gpu::surface_format(&caps);
        // not vsync-blocking: this loop paces itself, and a sleeping monitor must never stall it in an acquire
        let present_mode = if caps.present_modes.contains(&wgpu::PresentMode::Mailbox) { wgpu::PresentMode::Mailbox } else { wgpu::PresentMode::Fifo };
        let alpha_mode = if self.cfg.overlay && caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::PreMultiplied) {
            wgpu::CompositeAlphaMode::PreMultiplied
        } else if caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::Opaque) {
            wgpu::CompositeAlphaMode::Opaque
        } else {
            caps.alpha_modes[0]
        };
        let mut config = surface.get_default_config(&g.adapter, w, h).expect("surface config");
        config.format = format;
        config.present_mode = present_mode;
        config.desired_maximum_frame_latency = 1;
        config.alpha_mode = alpha_mode;
        surface.configure(&g.device, &config);
        let rscale = gpu::render_scale(&self.cfg, Mode::Wallpaper, w, h);
        if gpu::debug() {
            eprintln!("attractor: {} {:?} {}x{} (scale {}), {:?}, {:?}, scene at {:.0}%", self.outs[i].name, format, w, h, scale, present_mode, alpha_mode, rscale * 100.0);
        }
        let renderer = Renderer::new(&g.device, &g.queue, format, (w, h), rscale, &self.world);
        self.outs[i].gfx = Some(OutGfx { surface, config, renderer });
    }

    fn visible(&self, name: &str) -> bool {
        !self.desk.locked && !self.desk.covered.contains(name)
    }

    /// advance and draw one frame; returns how long to wait for the next
    fn tick(&mut self) -> Duration {
        self.ticks += 1;
        if self.ticks % 40 == 1 {
            self.mains = gpu::on_mains();
        }
        // slower on battery: a wallpaper is never worth the charge
        let fps = if self.mains { self.cfg.fps } else { self.cfg.fps.min(20.0) }.clamp(5.0, 120.0);
        let interval = Duration::from_secs_f32(1.0 / fps);
        let now = Instant::now();
        if !self.outs.iter().any(|o| o.gfx.is_some() && self.visible(&o.name)) {
            // nothing to show: keep time frozen and check back twice a second
            self.last = now;
            return Duration::from_millis(500);
        }
        self.world.update(now.duration_since(self.last).as_secs_f32());
        self.last = now;
        let g = self.gpu.as_ref().unwrap();
        for o in self.outs.iter_mut() {
            let visible = !self.desk.locked && !self.desk.covered.contains(&o.name);
            let Some(gfx) = o.gfx.as_mut() else { continue };
            if !visible {
                continue;
            }
            let frame = match gfx.surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
                wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                    gfx.surface.configure(&g.device, &gfx.config);
                    continue;
                }
                _ => continue,
            };
            let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
            gfx.renderer.draw(&self.world, &view);
            g.queue.present(frame);
        }
        if gpu::debug() {
            self.frames.0 += 1;
            if now.duration_since(self.frames.1) >= Duration::from_secs(3) {
                eprintln!("attractor: {:.1} fps ({}), {} output(s) visible", self.frames.0 as f32 / now.duration_since(self.frames.1).as_secs_f32(), self.world.system_name(), self.outs.iter().filter(|o| self.visible(&o.name)).count());
                self.frames = (0, now);
            }
        }
        // the work itself took part of the interval
        interval.saturating_sub(now.elapsed())
    }
}

impl CompositorHandler for App {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, surface: &wl_surface::WlSurface, new_factor: i32) {
        if let Some(i) = self.outs.iter().position(|o| o.layer.wl_surface() == surface) {
            self.outs[i].scale = new_factor.max(1);
            self.setup_gfx(i);
        }
    }
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
}

impl OutputHandler for App {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, output: wl_output::WlOutput) {
        self.add_output(output);
    }
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, output: wl_output::WlOutput) {
        // a monitor's name only arrives with its first full description
        let info = self.output_state.info(&output);
        if let Some(i) = self.outs.iter().position(|o| o.output == output) {
            if let Some(n) = info.as_ref().and_then(|i| i.name.clone()) {
                self.outs[i].name = n;
            }
            if let Some(s) = info.map(|i| i.scale_factor) {
                if s != self.outs[i].scale && s > 0 {
                    self.outs[i].scale = s;
                    self.setup_gfx(i);
                }
            }
        }
    }
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, output: wl_output::WlOutput) {
        self.outs.retain(|o| o.output != output);
    }
}

impl LayerShellHandler for App {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        self.outs.retain(|o| &o.layer != layer);
    }
    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface, configure: LayerSurfaceConfigure, _serial: u32) {
        let Some(i) = self.outs.iter().position(|o| &o.layer == layer) else { return };
        let info = self.output_state.info(&self.outs[i].output);
        let (mut w, mut h) = configure.new_size;
        if (w == 0 || h == 0) && let Some(ls) = info.as_ref().and_then(|i| i.logical_size) {
            (w, h) = (ls.0 as u32, ls.1 as u32);
        }
        let o = &mut self.outs[i];
        o.size = (w, h);
        if let Some(n) = info.as_ref().and_then(|i| i.name.clone()) {
            o.name = n;
        }
        o.scale = info.map(|i| i.scale_factor).unwrap_or(1).max(1);
        self.setup_gfx(i);
    }
}

delegate_compositor!(App);
delegate_output!(App);
delegate_layer!(App);
delegate_registry!(App);

impl ProvidesRegistryState for App {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState];
}

pub fn run(cfg: Config, seed: u64) {
    let conn = Connection::connect_to_env().expect("no Wayland display");
    let (globals, event_queue) = registry_queue_init::<App>(&conn).expect("registry");
    let qh = event_queue.handle();
    let compositor = CompositorState::bind(&globals, &qh).expect("wl_compositor");
    let layer_shell = LayerShell::bind(&globals, &qh).expect("this compositor has no wlr-layer-shell");
    let now = Instant::now();
    let mut app = App {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        compositor,
        layer_shell,
        conn: conn.clone(),
        qh: qh.clone(),
        world: World::new(&cfg, seed),
        cfg,
        instance: gpu::instance(),
        gpu: None,
        outs: Vec::new(),
        desk: Desk::default(),
        last: now,
        ticks: 0,
        mains: true,
        frames: (0, now),
    };
    let mut event_loop: EventLoop<App> = EventLoop::try_new().expect("event loop");
    let handle = event_loop.handle();
    WaylandSource::new(conn, event_queue).insert(handle.clone()).expect("wayland source");
    let (tx, rx): (channel::Sender<Desk>, Channel<Desk>) = channel::channel();
    handle
        .insert_source(rx, |ev, _, app: &mut App| {
            if let channel::Event::Msg(d) = ev {
                app.desk = d;
            }
        })
        .expect("watcher channel");
    handle.insert_source(Timer::from_duration(Duration::from_millis(300)), |_, _, app: &mut App| TimeoutAction::ToDuration(app.tick())).expect("timer");
    spawn_watcher(tx);
    event_loop.run(None, &mut app, |_| {}).expect("event loop");
}
