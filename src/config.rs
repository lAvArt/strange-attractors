//! Settings (~/.config/attractor/config.json, ATTRACTOR_* variables, command-line flags) and the colour looks.

use serde_json::Value;

#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    Saver,
    Wallpaper,
}

#[derive(Clone)]
pub struct Config {
    /// loaded for the wallpaper (else the screensaver)
    pub wall: bool,
    pub preset: String,
    /// an attractor's name, or "cycle" to rotate through all of them
    pub system: String,
    pub gpu: String,
    pub fps: f32,
    pub render_scale: Option<f32>,
    /// strands in all, and the points each one remembers: `None` lets the look decide
    pub trails: Option<usize>,
    pub length: Option<usize>,
    /// strands per ribbon
    pub strands: Option<usize>,
    /// "theme" (the system theme's colours), "original" (the look's own colours), or "auto": the theme, except for
    /// the period looks whose colours are the point
    pub palette: String,
    /// the attractor's name, equations and live numbers, set quietly around the edges
    pub annotations: bool,
    /// `false` drops the hairlines from a look that has them (the dust then is grains alone)
    pub lines: Option<bool>,
    pub speed: f32,
    pub width: Option<f32>,
    pub zoom: f32,
    pub cycle: f32,
    pub intensity: Option<f32>,
    /// wallpaper only: draw the trails with transparency over the desktop wallpaper instead of on their own ground
    pub overlay: bool,
    /// "none", "twinise", "omarchy", or the path of an SVG
    pub logo: String,
    /// "outline", "fill" or "both"
    pub logo_style: String,
    /// "auto" (the look's own colour), "accent" / "foreground" (your theme), "svg" (the SVG's colours) or "#rrggbb"
    pub logo_color: String,
    /// the longer side of the logo, as a fraction of the screen height
    pub logo_size: f32,
    /// the logo's centre, as fractions of the screen width and height
    pub logo_x: f32,
    pub logo_y: f32,
    pub logo_opacity: f32,
    /// slides the attractor sideways (and up) in screen space, in units of half the screen: `None` makes room for a logo
    pub shift_x: Option<f32>,
    pub shift_y: f32,
    /// wallpaper only: render at the monitor's native pixels (the default) instead of logical ones
    pub hidpi: bool,
}

pub fn home() -> String {
    std::env::var("HOME").unwrap_or_default()
}

impl Config {
    pub fn load(mode: Mode) -> Config {
        let wall = mode == Mode::Wallpaper;
        let mut c = Config {
            wall,
            preset: "theme".into(),
            system: "cycle".into(),
            gpu: "low".into(),
            fps: if wall { 30.0 } else { 60.0 },
            render_scale: None,
            trails: None,
            length: None,
            strands: None,
            palette: "auto".into(),
            annotations: true,
            lines: None,
            speed: if wall { 0.7 } else { 1.0 },
            width: None,
            zoom: 1.0,
            cycle: if wall { 150.0 } else { 90.0 },
            intensity: None,
            overlay: false,
            logo: "none".into(),
            logo_style: "both".into(),
            logo_color: "auto".into(),
            logo_size: 0.56,
            logo_x: 0.66,
            logo_y: 0.52,
            logo_opacity: 0.6,
            shift_x: None,
            shift_y: 0.0,
            hidpi: true,
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
            if let Some(x) = f("trails") { c.trails = Some((x as usize).clamp(8, 16384)) }
            if let Some(x) = f("length") { c.length = Some((x as usize).clamp(16, 4096)) }
            if let Some(x) = f("strands") { c.strands = Some((x as usize).clamp(1, 256)) }
            if let Some(x) = s("palette") { c.palette = x }
            if let Some(x) = get("annotations").and_then(|x| x.as_bool()) { c.annotations = x }
            if let Some(x) = get("lines").and_then(|x| x.as_bool()) { c.lines = Some(x) }
            if let Some(x) = f("speed") { c.speed = x }
            if let Some(x) = f("width") { c.width = Some(x) }
            if let Some(x) = f("zoom") { c.zoom = x }
            if let Some(x) = f("cycle") { c.cycle = x }
            if let Some(x) = f("intensity") { c.intensity = Some(x) }
            if let Some(x) = f("shift_x") { c.shift_x = Some(x) }
            if let Some(x) = f("shift_y") { c.shift_y = x }
            if let Some(x) = s("logo") { c.logo = x }
            if let Some(x) = s("logo_style") { c.logo_style = x }
            if let Some(x) = s("logo_color") { c.logo_color = x }
            if let Some(x) = f("logo_size") { c.logo_size = x }
            if let Some(x) = f("logo_x") { c.logo_x = x }
            if let Some(x) = f("logo_y") { c.logo_y = x }
            if let Some(x) = f("logo_opacity") { c.logo_opacity = x }
            if let Some(x) = get("hidpi").and_then(|x| x.as_bool()) { c.hidpi = x }
            if let Some(x) = get("overlay").and_then(|x| x.as_bool()) { c.overlay = x }
        }
        let e = |k: &str| std::env::var(k).ok();
        if let Some(x) = e("ATTRACTOR_PRESET") { c.preset = x }
        if let Some(x) = e("ATTRACTOR_SYSTEM") { c.system = x }
        if let Some(x) = e("ATTRACTOR_GPU") { c.gpu = x }
        if let Some(x) = e("ATTRACTOR_LOGO") { c.logo = x }
        if let Some(x) = e("ATTRACTOR_PALETTE") { c.palette = x }
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

/// How the dots of a look behave.
#[derive(Clone, Copy, PartialEq)]
pub enum Dots {
    /// scattered grains, with depth of field
    Dust,
    /// a lattice whose dot size is the tone
    Halftone,
    /// where the lines end, they come apart into drifting dots
    Dissolve,
}

/// How the attractor is drawn: what is drawn (lines along the flow, lines across it, dots, the hiding sheet), how
/// finely, and in which colours. Sizes are in pixels of a 1080-line picture and scale with the output.
#[derive(Clone)]
pub struct Look {
    pub name: &'static str,
    pub bg_top: [f32; 3],
    pub bg_bot: [f32; 3],
    /// the tone in shade, in light, and on the ridges that face the light
    pub c: [[f32; 3]; 3],
    /// sparkles
    pub accent: [f32; 3],
    /// annotations: the quiet text, and the live values
    pub text: [f32; 3],
    pub text_hi: [f32; 3],
    /// width of the lines along the flow (0: none), and their strength relative to the dots'
    pub line: f32,
    pub line_alpha: f32,
    /// width of the lines across the flow (0: none), and how many steps apart they are
    pub cross: f32,
    pub cross_every: u32,
    /// dot radius (0: none), their behaviour, and how many steps apart they are along a strand
    pub dot: f32,
    pub dots: Dots,
    pub dot_every: u32,
    /// near folds hide far ones
    pub sheet: bool,
    /// above 0, nothing is ever cut off behind a sheet: marks behind one thin and dim, smoothly, down to this share of
    /// their strength (for the looks made of dots, where a hard cut reads as a dark band)
    pub soft: f32,
    /// how much of its own tone the sheet lays under the lines (0: none, it only hides; 1: a solid surface)
    pub wash: f32,
    /// every ribbon in one of the three tones, flat-shaded, instead of tone following the light
    pub ribbon_colours: bool,
    /// scanlines (0: none)
    pub scan: f32,
    /// 0: a plain gradient, 1: a sunset over a grid, 2: copper bars
    pub ground: u32,
    /// the two extra colours of those grounds
    pub g0: [f32; 3],
    pub g1: [f32; 3],
    /// draw into a coarse picture of about this many lines and scale it up in whole pixels (0: no)
    pub pixel: u32,
    /// dither that picture down to this many levels per channel (0: no)
    pub quant: f32,
    /// blocky text, this many pixels to the dot
    pub chunk: u32,
    /// how much the light decides the tone (0: flat)
    pub shade: f32,
    /// depth of field: pixels of blur per unit of depth away from the focal plane
    pub dof: f32,
    /// the focal plane, in units in front of (-) or behind (+) the attractor's centre
    pub focus: f32,
    /// the age (0 head .. 1 tail) at which lines come apart into dots; above 1: never
    pub dissolve: f32,
    /// how far dots stray from their strands
    pub scatter: f32,
    /// the share of grains that sparkle in the accent colour
    pub sparkle: f32,
    /// how much far things sink into the ground colour
    pub fog: f32,
    pub vignette: f32,
    /// how the tail fades: higher fades sooner
    pub age_pow: f32,
    pub opacity: f32,
    /// strands per ribbon, the gap between neighbours (world units; the attractor has radius 1), and how many
    /// strands and points per strand: (wallpaper, screensaver)
    pub strands: usize,
    pub pitch: f32,
    pub trails: (usize, usize),
    pub length: (usize, usize),
    /// the logo's colour; `None` keeps the SVG's own colours
    pub logo_tint: Option<[f32; 3]>,
}

fn theme_path() -> String {
    std::env::var("ATTRACTOR_THEME").unwrap_or_else(|_| format!("{}/.local/state/omarchy/current/theme/colors.toml", home()))
}

fn theme_colors() -> Option<Vec<(String, String)>> {
    let text = std::fs::read_to_string(theme_path()).ok()?;
    Some(
        text.lines()
            .filter_map(|l| {
                let (k, v) = l.split_once('=')?;
                Some((k.trim().to_string(), v.trim().trim_matches('"').to_string()))
            })
            .collect(),
    )
}

/// identifies the current theme file: its resolved path and modification time, so a theme switch (a new symlink
/// target) or an edit is noticed
pub fn theme_stamp() -> String {
    let p = theme_path();
    let canon = std::fs::canonicalize(&p).map(|c| c.display().to_string()).unwrap_or_default();
    let mtime = std::fs::metadata(&p).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_nanos()).unwrap_or(0);
    format!("{canon}:{mtime}")
}

pub const STYLES: &[&str] = &["weave", "contour", "stipple", "dust", "80s", "90s"];
/// what `"preset": "theme"` draws: (wallpaper, screensaver)
const DEFAULT_STYLE: (&str, &str) = ("stipple", "dust");

/// The colours of a look: the ground, three tones for the form, an accent.
struct Pal {
    bg_top: [f32; 3],
    bg_bot: [f32; 3],
    c: [[f32; 3]; 3],
    accent: [f32; 3],
    text: [f32; 3],
    text_hi: [f32; 3],
    g0: [f32; 3],
    g1: [f32; 3],
    vignette: f32,
}

fn lum(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// the system theme's colours in the roles a look needs. `tinted`: the form is drawn in the accent; otherwise in the
/// foreground, with the accent kept for sparkles and live numbers. The shaded tone is a slightly deeper tint of the
/// same hue, never a step toward black: depth comes from light and layering, not from dark.
fn theme_palette(tinted: bool) -> Pal {
    let th = theme_colors().unwrap_or_default();
    let g = |k: &str, d: &str| th.iter().find(|(a, _)| a == k).map(|(_, v)| v.clone()).unwrap_or_else(|| d.to_string());
    let bg = hex(&g("background", "#121212"));
    let fg = hex(&g("foreground", "#bebebe"));
    let accent = hex(&g("accent", "#e68e0d"));
    let light = g("mode", "dark") == "light" || lum(bg) > 0.4;
    let white = [1.0, 1.0, 1.0];
    let black = [0.0, 0.0, 0.0];
    if light {
        // ink on the theme's paper: pale in shade, full in light
        let darker = hex(&g("darker_background", "#d8d8d8"));
        let ink = if lum(fg) < 0.25 { fg } else { mix(fg, black, 0.5) };
        // an accent too pale to read on paper is deepened
        let acc = if lum(accent) > 0.45 { mix(accent, black, 0.45) } else { accent };
        let ink = mix(ink, black, 0.25);
        let c = if tinted { [mix(acc, bg, 0.2), mix(acc, ink, 0.3), mix(acc, ink, 0.8)] } else { [mix(ink, bg, 0.3), mix(ink, bg, 0.08), ink] };
        Pal { bg_top: bg, bg_bot: mix(bg, darker, 0.6), c, accent: acc, text: ink, text_hi: mix(acc, ink, 0.25), g0: acc, g1: mix(acc, ink, 0.5), vignette: 0.10 }
    } else {
        let lighter = hex(&g("lighter_background", "#1e1e1e"));
        let darker = hex(&g("darker_background", "#0a0a0a"));
        let bright = hex(&g("bright_foreground", "#eaeaea"));
        let bright = if lum(bright) > lum(fg) { bright } else { mix(fg, white, 0.45) };
        // an accent too dark to read on the ground is lifted
        let acc = if lum(accent) < 0.12 { mix(accent, white, 0.35) } else { accent };
        let c = if tinted { [mix(acc, black, 0.16), mix(acc, white, 0.18), mix(acc, white, 0.78)] } else { [mix(fg, bg, 0.2), mix(fg, bright, 0.6), mix(bright, white, 0.7)] };
        Pal { bg_top: mix(bg, lighter, 0.5), bg_bot: darker, c, accent: acc, text: mix(fg, bg, 0.3), text_hi: acc, g0: mix(acc, white, 0.4), g1: acc, vignette: 0.22 }
    }
}

impl Look {
    /// the look `t` of the way from `a` to `b`: the colours blend, everything else is `b`'s
    pub fn lerp(a: &Look, b: &Look, t: f32) -> Look {
        let f = |x: f32, y: f32| x + (y - x) * t;
        let c3 = |x: [f32; 3], y: [f32; 3]| mix(x, y, t);
        Look {
            bg_top: c3(a.bg_top, b.bg_top),
            bg_bot: c3(a.bg_bot, b.bg_bot),
            c: [c3(a.c[0], b.c[0]), c3(a.c[1], b.c[1]), c3(a.c[2], b.c[2])],
            accent: c3(a.accent, b.accent),
            text: c3(a.text, b.text),
            text_hi: c3(a.text_hi, b.text_hi),
            g0: c3(a.g0, b.g0),
            g1: c3(a.g1, b.g1),
            vignette: f(a.vignette, b.vignette),
            logo_tint: match (a.logo_tint, b.logo_tint) {
                (Some(x), Some(y)) => Some(c3(x, y)),
                _ => b.logo_tint,
            },
            ..b.clone()
        }
    }

    /// the name of the look a preset draws. "theme" (and anything unknown) is the default: the stipple for the
    /// wallpaper, the dust for the screensaver.
    pub fn style_name(preset: &str, wall: bool) -> &'static str {
        match preset {
            "weave" | "mesh" => "weave",
            "contour" => "contour",
            "stipple" | "halftone" => "stipple",
            "dust" => "dust",
            "80s" | "vector" => "80s",
            "90s" | "vga" => "90s",
            _ => if wall { DEFAULT_STYLE.0 } else { DEFAULT_STYLE.1 },
        }
    }

    pub fn resolve(cfg: &Config) -> Look {
        let name = Look::style_name(&cfg.preset, cfg.wall);
        let period = name == "80s" || name == "90s";
        let original = cfg.palette == "original" || (period && cfg.palette != "theme");
        let pal = |tinted: bool, own: Pal| if original { own } else { theme_palette(tinted) };
        // what every look starts from
        let base = |p: Pal| Look {
            name,
            bg_top: p.bg_top, bg_bot: p.bg_bot, c: p.c, accent: p.accent, text: p.text, text_hi: p.text_hi, g0: p.g0, g1: p.g1, vignette: p.vignette,
            line: 0.0, line_alpha: 1.0, cross: 0.0, cross_every: 1, dot: 0.0, dots: Dots::Dust, dot_every: 1,
            sheet: true, soft: 0.0, wash: 0.0, ribbon_colours: false, scan: 0.0, ground: 0, pixel: 0, quant: 0.0, chunk: 1,
            shade: 0.8, dof: 0.0, focus: 0.0, dissolve: 2.0, scatter: 0.0, sparkle: 0.0, fog: 0.5, age_pow: 0.45, opacity: 1.0,
            strands: 24, pitch: 0.011, trails: (720, 1200), length: (400, 480), logo_tint: None,
        };
        let mut l = match name {
            // contour lines: fine parallel lines along the flow, brightest where the sheet faces the light
            "contour" => Look {
                line: 0.5, wash: 0.05, shade: 1.0, dof: 0.8, focus: -0.3,
                strands: 30, pitch: 0.0082, trails: (720, 1080), length: (380, 440),
                ..base(pal(true, Pal {
                    bg_top: hex("#161331"), bg_bot: hex("#0a0817"),
                    c: [hex("#2aa58a"), hex("#58e6bd"), hex("#d6fff0")],
                    accent: hex("#d6fff0"), text: hex("#5d8f8a"), text_hi: hex("#7ff0cb"), g0: hex("#58e6bd"), g1: hex("#2aa58a"), vignette: 0.35,
                }))
            },
            // a halftone: dots on the lattice of strands and steps, sized by the light
            "stipple" => Look {
                dot: 1.1, dots: Dots::Halftone, soft: 0.2, shade: 1.0, fog: 0.4, age_pow: 0.5,
                strands: 40, pitch: 0.0115, trails: (600, 1000), length: (300, 396),
                ..base(pal(false, Pal {
                    bg_top: hex("#2b2a29"), bg_bot: hex("#232221"),
                    c: [hex("#b9b6b0"), hex("#f1efe9"), hex("#ffffff")],
                    accent: hex("#ffffff"), text: hex("#a9a6a0"), text_hi: hex("#f4f2ec"), g0: hex("#ffffff"), g1: hex("#b9b6b0"), vignette: 0.12,
                }))
            },
            // grains scattered through the sheets, with a shallow depth of field
            "dust" => Look {
                dot: 0.46, dots: Dots::Dust, line: 0.32, line_alpha: 0.26, cross: 0.28, cross_every: 12, sheet: false, shade: 0.7, dof: 5.0, focus: -0.25, scatter: 1.0, sparkle: 0.0025, fog: 0.3, age_pow: 0.35, opacity: 0.9,
                strands: 16, pitch: 0.016, trails: (336, 688), length: (288, 348),
                ..base(pal(false, Pal {
                    bg_top: hex("#030304"), bg_bot: hex("#000000"),
                    c: [hex("#8f9198"), hex("#e9eaee"), hex("#ffffff")],
                    accent: hex("#7c6cff"), text: hex("#7d7f86"), text_hi: hex("#e9eaee"), g0: hex("#ffffff"), g1: hex("#7c6cff"), vignette: 0.3,
                }))
            },
            // a vector display: a hidden-line wireframe in phosphor colours over a sunset and a grid, with scanlines
            "80s" => Look {
                line: 0.8, cross: 0.7, cross_every: 4, shade: 1.0, fog: 0.35, age_pow: 0.9, scan: 0.3, ground: 1, chunk: 2, vignette: 0.45,
                strands: 12, pitch: 0.03, trails: (240, 360), length: (360, 440),
                ..base(pal(true, Pal {
                    bg_top: hex("#0b0221"), bg_bot: hex("#3a0f5c"),
                    c: [hex("#ff2bd6"), hex("#29f0ff"), hex("#f4feff")],
                    accent: hex("#ffe14a"), text: hex("#29f0ff"), text_hi: hex("#ffe14a"), g0: hex("#ffd75a"), g1: hex("#ff2f92"), vignette: 0.45,
                }))
            },
            // early computer graphics: flat-shaded ribbons in a coarse picture, dithered down to a few colours
            "90s" => Look {
                wash: 1.0, ribbon_colours: true, shade: 1.0, fog: 0.0, age_pow: 0.25, ground: 2, pixel: 400, quant: 6.0, vignette: 0.0,
                strands: 10, pitch: 0.022, trails: (200, 300), length: (300, 380),
                ..base(pal(true, Pal {
                    bg_top: hex("#10106a"), bg_bot: hex("#008080"),
                    c: [hex("#21c7b6"), hex("#a45ee5"), hex("#d5dbe3")],
                    accent: hex("#ffd166"), text: hex("#c0c0c0"), text_hi: hex("#ffff55"), g0: hex("#ffd166"), g1: hex("#ef476f"), vignette: 0.0,
                }))
            },
            // a weave of the sheet: hairlines along the flow and across it, coming apart into dust at the tail
            _ => Look {
                line: 0.42, cross: 0.38, cross_every: 4, dot: 0.7, dots: Dots::Dissolve, dot_every: 2, dissolve: 0.62, scatter: 4.0, sparkle: 0.004,
                shade: 0.8, fog: 0.55, age_pow: 0.3, opacity: 0.9,
                strands: 24, pitch: 0.0125, trails: (600, 960), length: (400, 460),
                ..base(pal(false, Pal {
                    bg_top: hex("#1b1c1e"), bg_bot: hex("#050506"),
                    c: [hex("#8d9198"), hex("#d3d6db"), hex("#ffffff")],
                    accent: hex("#9a8cff"), text: hex("#8d9096"), text_hi: hex("#e8e9ec"), g0: hex("#ffffff"), g1: hex("#9a8cff"), vignette: 0.35,
                }))
            },
        };
        if let Some(w) = cfg.width {
            l.line *= w;
            l.cross *= w;
            l.dot *= w;
        }
        if cfg.lines == Some(false) {
            l.line = 0.0;
            l.cross = 0.0;
        }
        if let Some(i) = cfg.intensity { l.opacity = (l.opacity * i).clamp(0.0, 1.0) }
        if let Some(s) = cfg.strands { l.strands = s }
        if let Some(n) = cfg.trails { l.trails = (n, n) }
        if let Some(n) = cfg.length { l.length = (n, n) }
        l.logo_tint = match cfg.logo_color.as_str() {
            "svg" => None,
            "accent" | "foreground" => {
                let th = theme_colors().unwrap_or_default();
                th.iter().find(|(k, _)| k == cfg.logo_color.as_str()).map(|(_, v)| hex(v)).or(Some(l.c[1]))
            }
            c if c.starts_with('#') => Some(hex(c)),
            _ => Some(l.c[1]),
        };
        l
    }
}
