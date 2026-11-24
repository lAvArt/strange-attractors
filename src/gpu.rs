//! GPU selection shared by every mode. Like Gridfall, this stays on the integrated GPU by default: waking a
//! hybrid laptop's NVIDIA card for a background effect is what knocks some of them off the bus.

use crate::config::{Config, Mode};

pub fn debug() -> bool {
    std::env::var_os("ATTRACTOR_DEBUG").is_some()
}

fn want_fast_gpu(cfg: &Config) -> bool {
    cfg.gpu == "high"
}

/// A GPU other than an NVIDIA one is present.
fn has_other_gpu() -> bool {
    std::fs::read_dir("/sys/class/drm").into_iter().flatten().flatten().any(|e| {
        let vendor = std::fs::read_to_string(e.path().join("device/vendor")).unwrap_or_default();
        let v = vendor.trim();
        !v.is_empty() && v != "0x10de"
    })
}

/// Call first thing in `main`, before any thread starts: the Vulkan loader then never even loads the NVIDIA driver.
pub fn prepare_env(cfg: &Config) {
    if !want_fast_gpu(cfg) && has_other_gpu() && std::env::var_os("VK_LOADER_DRIVERS_DISABLE").is_none() {
        // SAFETY: single-threaded at this point
        unsafe { std::env::set_var("VK_LOADER_DRIVERS_DISABLE", "*nvidia*") };
    }
}

pub fn instance() -> wgpu::Instance {
    let mut d = wgpu::InstanceDescriptor::new_without_display_handle();
    d.backends = wgpu::Backends::VULKAN;
    wgpu::Instance::new(d)
}

pub fn adapter(instance: &wgpu::Instance, surface: Option<&wgpu::Surface<'_>>, cfg: &Config) -> wgpu::Adapter {
    let a = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: if want_fast_gpu(cfg) { wgpu::PowerPreference::HighPerformance } else { wgpu::PowerPreference::LowPower },
        compatible_surface: surface,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))
    .expect("no GPU adapter");
    if debug() {
        let i = a.get_info();
        eprintln!("attractor: adapter {} ({:?}, {:?})", i.name, i.device_type, i.backend);
    }
    a
}

pub fn device(adapter: &wgpu::Adapter) -> (wgpu::Device, wgpu::Queue) {
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("device")
}

/// Prefer an 8-bit sRGB surface: the shaders output linear light and the GPU encodes it for the panel.
pub fn surface_format(caps: &wgpu::SurfaceCapabilities) -> wgpu::TextureFormat {
    caps.formats.iter().copied().find(|f| matches!(f, wgpu::TextureFormat::Bgra8UnormSrgb | wgpu::TextureFormat::Rgba8UnormSrgb)).unwrap_or(caps.formats[0])
}

/// How big the scene renders relative to its output: full size, since fine lines do not survive being scaled up,
/// unless `render_scale` says otherwise.
pub fn render_scale(cfg: &Config, mode: Mode, w: u32, h: u32) -> f32 {
    if let Some(s) = cfg.render_scale {
        return s.clamp(0.3, 1.0);
    }
    let _ = (mode, w, h);
    1.0
}

/// Plugged in (or a machine without a battery).
pub fn on_mains() -> bool {
    let Ok(dir) = std::fs::read_dir("/sys/class/power_supply") else { return true };
    let mut battery = false;
    for e in dir.flatten() {
        let p = e.path();
        let kind = std::fs::read_to_string(p.join("type")).unwrap_or_default();
        match kind.trim() {
            "Mains" if std::fs::read_to_string(p.join("online")).unwrap_or_default().trim() == "1" => return true,
            "Battery" => battery = true,
            _ => {}
        }
    }
    !battery
}
