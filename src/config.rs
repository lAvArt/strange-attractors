//! Settings (~/.config/attractor/config.json, ATTRACTOR_* variables, command-line flags) and the colour looks.

use serde_json::Value;

#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    Saver,
    Wallpaper,
}

#[derive(Clone)]
pub struct Config {
    pub preset: String,
    /// an attractor's name, or "cycle" to rotate through all of them
    pub system: String,
    pub gpu: String,
    pub fps: f32,
    pub render_scale: Option<f32>,
    pub trails: usize,
    pub length: usize,
    pub speed: f32,
    pub width: Option<f32>,
    pub zoom: f32,
    pub cycle: f32,
    pub intensity: Option<f32>,
    pub bloom: Option<f32>,
    /// wallpaper only: draw the trails with transparency over the desktop wallpaper instead of on their own ground
    pub overlay: bool,
}

pub fn home() -> String {
    std::env::var("HOME").unwrap_or_default()
}

impl Config {
    pub fn load(mode: Mode) -> Config {
        let wall = mode == Mode::Wallpaper;
        let mut c = Config {
            preset: "theme".into(),
            system: "cycle".into(),
            gpu: "low".into(),
            fps: if wall { 30.0 } else { 60.0 },
            render_scale: None,
            trails: if wall { 256 } else { 384 },
            length: if wall { 520 } else { 640 },
            speed: if wall { 0.7 } else { 1.0 },
            width: None,
            zoom: 1.0,
            cycle: if wall { 150.0 } else { 90.0 },
            intensity: None,
            bloom: None,
            overlay: false,
        };
        let path = std::env::var("ATTRACTOR_CONFIG").unwrap_or_else(|_| format!("{}/.config/attractor/config.json", home()));
        if let Some(v) = std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok()) {
            // a "wallpaper_options" / "saver_options" object overrides the shared top-level keys for that mode
            let sect = v.get(if wall { "wallpaper_options" } else { "saver_options" });
            let get = |k: &str| sect.and_then(|s| s.get(k)).or_else(|| v.get(k));
            let s = |k: &str| get(k).and_then(|x| x.as_str()).map(String::from);
            let f = |k: &str| get(k).and_then(|x| x.as_f64()).map(|x| x as f32);
            if let Some(x) = s("preset") { c.preset = x }
            if let Some(x) = s("system") { c.system = x }
            if let Some(x) = s("gpu") { c.gpu = x }
            if let Some(x) = f("fps") { c.fps = x }
            if let Some(x) = f("render_scale") { c.render_scale = Some(x) }
            if let Some(x) = f("trails") { c.trails = x as usize }
            if let Some(x) = f("length") { c.length = x as usize }
            if let Some(x) = f("speed") { c.speed = x }
            if let Some(x) = f("width") { c.width = Some(x) }
            if let Some(x) = f("zoom") { c.zoom = x }
            if let Some(x) = f("cycle") { c.cycle = x }
            if let Some(x) = f("intensity") { c.intensity = Some(x) }
            if let Some(x) = f("bloom") { c.bloom = Some(x) }
            if let Some(x) = get("overlay").and_then(|x| x.as_bool()) { c.overlay = x }
        }
        let e = |k: &str| std::env::var(k).ok();
        if let Some(x) = e("ATTRACTOR_PRESET") { c.preset = x }
        if let Some(x) = e("ATTRACTOR_SYSTEM") { c.system = x }
        if let Some(x) = e("ATTRACTOR_GPU") { c.gpu = x }
        c.trails = c.trails.clamp(8, 4096);
        c.length = c.length.clamp(16, 4096);
        c
    }
}

fn lin(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

/// "#rrggbb" in sRGB → linear RGB
pub fn hex(s: &str) -> [f32; 3] {
    let s = s.trim().trim_start_matches('#');
    let v = u32::from_str_radix(s, 16).unwrap_or(0);
    [lin(((v >> 16) & 255) as f32 / 255.0), lin(((v >> 8) & 255) as f32 / 255.0), lin((v & 255) as f32 / 255.0)]
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// How the trails are coloured and composited.
#[derive(Clone)]
pub struct Look {
    pub bg_top: [f32; 3],
    pub bg_bot: [f32; 3],
    /// slow → fast colours along the trail
    pub c: [[f32; 3]; 3],
    /// ordinary alpha blending (dark ink on a light ground) instead of additive light
    pub over: bool,
    pub bloom: f32,
    /// line width in pixels at 1080p
    pub width: f32,
    pub intensity: f32,
    pub vignette: f32,
    pub exposure: f32,
    pub age_pow: f32,
}

fn theme_colors() -> Option<Vec<(String, String)>> {
    let p = std::env::var("ATTRACTOR_THEME").unwrap_or_else(|_| format!("{}/.local/state/omarchy/current/theme/colors.toml", home()));
    let text = std::fs::read_to_string(p).ok()?;
    Some(
        text.lines()
            .filter_map(|l| {
                let (k, v) = l.split_once('=')?;
                Some((k.trim().to_string(), v.trim().trim_matches('"').to_string()))
            })
            .collect(),
    )
}

impl Look {
    pub fn resolve(cfg: &Config) -> Look {
        let mut l = match cfg.preset.as_str() {
            // the colours of the active Omarchy theme: its accent as the light, its background as the ground
            "theme" => {
                let th = theme_colors().unwrap_or_default();
                let g = |k: &str, d: &str| th.iter().find(|(a, _)| a == k).map(|(_, v)| v.clone()).unwrap_or_else(|| d.to_string());
                let bg = hex(&g("background", "#101014"));
                let dark = hex(&g("darker_background", "#08080a"));
                let accent = hex(&g("accent", "#e68e0d"));
                let bright = hex(&g("bright_foreground", "#eaeaea"));
                Look {
                    bg_top: bg,
                    bg_bot: dark,
                    c: [mix(accent, [0.0; 3], 0.7), accent, mix(accent, bright, 0.4)],
                    over: false,
                    bloom: 0.9,
                    width: 2.2,
                    intensity: 0.6,
                    vignette: 0.55,
                    exposure: 1.2,
                    age_pow: 1.5,
                }
            }
            // the look of the Lorenz Deformation renders: amber light over teal-green concrete
            "deformation" => Look {
                bg_top: hex("#0c2a2e"),
                bg_bot: hex("#4a5233"),
                c: [hex("#a8400c"), hex("#ff8c1a"), hex("#ffe27a")],
                over: false,
                bloom: 0.8,
                width: 2.6,
                intensity: 0.6,
                vignette: 0.6,
                exposure: 1.2,
                age_pow: 1.2,
            },
            "lavart" => Look {
                bg_top: hex("#05070b"),
                bg_bot: hex("#0b1118"),
                c: [hex("#0c3b4a"), hex("#22c3e8"), hex("#c9f4ff")],
                over: false,
                bloom: 0.9,
                width: 2.0,
                intensity: 0.55,
                vignette: 0.6,
                exposure: 1.35,
                age_pow: 1.5,
            },
            "ink" => Look {
                bg_top: hex("#f4f3ef"),
                bg_bot: hex("#e3e2dc"),
                c: [hex("#9aa3ab"), hex("#4f5961"), hex("#171a1e")],
                over: true,
                bloom: 0.0,
                width: 3.4,
                intensity: 1.0,
                vignette: 0.12,
                exposure: 1.0,
                age_pow: 0.8,
            },
            // "neon" and anything unknown
            _ => Look {
                bg_top: hex("#04050c"),
                bg_bot: hex("#0a0b1c"),
                c: [hex("#3a1fb0"), hex("#14d2ff"), hex("#ff6fd8")],
                over: false,
                bloom: 1.1,
                width: 2.0,
                intensity: 0.55,
                vignette: 0.6,
                exposure: 1.35,
                age_pow: 1.5,
            },
        };
        if let Some(w) = cfg.width { l.width = w }
        if let Some(i) = cfg.intensity { l.intensity = i }
        if let Some(b) = cfg.bloom { l.bloom = b }
        l
    }
}
