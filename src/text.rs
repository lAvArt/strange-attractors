//! The annotation layer: the running attractor's name, its equations and parameters, and numbers read from the
//! simulation, set small around the edges like the margins of a plate. Glyphs are rasterised once into an atlas at
//! the exact pixel size they are drawn at (so they stay crisp), and laid out a few times a second into one small
//! vertex buffer.

use crate::systems;
use crate::world::World;
use bytemuck::{Pod, Zeroable};
use std::collections::HashMap;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct TextVertex {
    pub pos: [f32; 2],
    pub uv: [f32; 2],
    pub col: [f32; 4],
}

#[derive(Clone, Copy)]
struct Glyph {
    /// atlas rectangle, px
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    /// offset of the bitmap's top-left corner from the pen position on the baseline
    dx: f32,
    dy: f32,
}

pub struct Annot {
    pub atlas: Vec<u8>,
    pub atlas_w: u32,
    pub atlas_h: u32,
    /// (character, size index) → glyph
    glyphs: HashMap<(char, u8), Glyph>,
    /// pixel sizes: body, title; and the body's advance
    px: [f32; 2],
    adv: [f32; 2],
    /// the font has the dotted letters of Newton's notation
    dots: bool,
    /// chunky text: glyphs are rasterised this many times smaller, hardened to on/off, and drawn that much larger
    chunk: f32,
    /// hard-edged text cannot be faint: it is either there or not
    hard: bool,
}

const CHARSET: &str = " !\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefghijklmnopqrstuvwxyz{|}~αβγδεζλρσΔ−—·×²³°≈ẋẏż₁";

fn font_bytes() -> Option<Vec<u8>> {
    if let Ok(p) = std::env::var("ATTRACTOR_FONT") {
        if let Ok(b) = std::fs::read(p) {
            return Some(b);
        }
    }
    // the system's monospace font (on Omarchy: the one the theme and the terminal use)
    let asked = std::process::Command::new("fc-match").args(["-f", "%{file}", "monospace"]).output().ok().and_then(|o| String::from_utf8(o.stdout).ok());
    let fallbacks = ["/usr/share/fonts/TTF/JetBrainsMonoNerdFont-Regular.ttf", "/usr/share/fonts/TTF/DejaVuSansMono.ttf", "/usr/share/fonts/liberation/LiberationMono-Regular.ttf"];
    asked.iter().map(|s| s.trim()).chain(fallbacks).filter(|p| p.ends_with(".ttf") || p.ends_with(".otf")).find_map(|p| std::fs::read(p).ok())
}

impl Annot {
    /// `h`: the height in pixels of the picture the text is drawn into; `chunk` > 1 makes it a blocky bitmap face
    pub fn new(h: u32, chunk: u32) -> Option<Annot> {
        let font = fontdue::Font::from_bytes(font_bytes()?, fontdue::FontSettings::default()).ok()?;
        let chunk = chunk.max(1) as f32;
        // a coarse picture (a pixelated look) needs its text hard-edged too
        let hard = chunk > 1.0 || h < 700;
        let body = (h as f32 * if chunk > 1.0 { 0.0135 } else { 0.0125 } / chunk).round().clamp(if hard { 11.0 } else { 9.0 }, 40.0);
        let px = [body, (body * if hard { 2.0 } else { 1.9 }).round()];
        let atlas_w = 1024u32;
        let mut rows: Vec<u8> = vec![0; (atlas_w * 4) as usize];
        // a solid block in the corner, for rules and ticks
        for y in 0..4 {
            for x in 0..4 {
                rows[(y * atlas_w + x) as usize] = 255;
            }
        }
        let (mut cx, mut cy, mut rowh) = (6u32, 0u32, 4u32);
        let mut glyphs = HashMap::new();
        for (si, &size) in px.iter().enumerate() {
            for ch in CHARSET.chars() {
                if font.lookup_glyph_index(ch) == 0 && ch != ' ' {
                    continue;
                }
                let (m, mut bmp) = font.rasterize(ch, size);
                if hard {
                    bmp.iter_mut().for_each(|b| *b = if *b > 104 { 255 } else { 0 });
                }
                let (w, hh) = (m.width as u32, m.height as u32);
                if cx + w + 1 > atlas_w {
                    cx = 0;
                    cy += rowh + 1;
                    rowh = 0;
                }
                rowh = rowh.max(hh);
                let need = ((cy + rowh + 1) * atlas_w) as usize;
                if rows.len() < need {
                    rows.resize(need, 0);
                }
                for y in 0..hh {
                    let d = ((cy + y) * atlas_w + cx) as usize;
                    rows[d..d + w as usize].copy_from_slice(&bmp[(y * w) as usize..((y + 1) * w) as usize]);
                }
                glyphs.insert((ch, si as u8), Glyph { x: cx, y: cy, w, h: hh, dx: m.xmin as f32, dy: -(m.ymin as f32) - hh as f32 });
                cx += w + 1;
            }
        }
        let atlas_h = (rows.len() as u32).div_ceil(atlas_w).max(4);
        rows.resize((atlas_w * atlas_h) as usize, 0);
        let adv = [font.metrics('0', px[0]).advance_width, font.metrics('0', px[1]).advance_width];
        let dots = ['ẋ', 'ẏ', 'ż'].iter().all(|&c| glyphs.contains_key(&(c, 0)));
        Some(Annot { atlas: rows, atlas_w, atlas_h, glyphs, px: [px[0] * chunk, px[1] * chunk], adv: [adv[0] * chunk, adv[1] * chunk], dots, chunk, hard })
    }

    fn quad(&self, v: &mut Vec<TextVertex>, x: f32, y: f32, w: f32, h: f32, uv: [f32; 4], col: [f32; 4]) {
        let p = |px: f32, py: f32, u: f32, t: f32| TextVertex { pos: [px, py], uv: [u, t], col };
        let (a, b, c, d) = (p(x, y, uv[0], uv[1]), p(x + w, y, uv[2], uv[1]), p(x, y + h, uv[0], uv[3]), p(x + w, y + h, uv[2], uv[3]));
        v.extend_from_slice(&[a, b, c, c, b, d]);
    }

    /// a filled rectangle (a rule, a tick, a dot)
    fn rect(&self, v: &mut Vec<TextVertex>, x: f32, y: f32, w: f32, h: f32, col: [f32; 4]) {
        let (u, t) = (2.0 / self.atlas_w as f32, 2.0 / self.atlas_h as f32);
        self.quad(v, x.round(), y.round(), w.round().max(1.0), h.round().max(1.0), [u, t, u, t], col);
    }

    /// text with its baseline at `y`, starting at `x` (or ending there, if `right`); `track` adds letter-spacing
    /// in units of the advance. Returns the width drawn.
    #[allow(clippy::too_many_arguments)]
    fn text(&self, v: &mut Vec<TextVertex>, s: &str, size: u8, x: f32, y: f32, right: bool, track: f32, col: [f32; 4]) -> f32 {
        let adv = self.adv[size as usize] * (1.0 + track);
        let n = s.chars().count() as f32;
        let width = n * adv;
        let mut pen = (if right { x - width } else { x }).round();
        let y = y.round();
        for ch in s.chars() {
            let ch = if self.glyphs.contains_key(&(ch, size)) { ch } else { fallback(ch) };
            if let Some(g) = self.glyphs.get(&(ch, size)) {
                if g.w > 0 {
                    let (aw, ah) = (self.atlas_w as f32, self.atlas_h as f32);
                    let uv = [g.x as f32 / aw, g.y as f32 / ah, (g.x + g.w) as f32 / aw, (g.y + g.h) as f32 / ah];
                    let k = self.chunk;
                    self.quad(v, (pen + g.dx * k).round(), y + g.dy * k, g.w as f32 * k, g.h as f32 * k, uv, col);
                }
            }
            pen += adv;
        }
        width
    }

    /// everything, for a picture of `size` pixels
    pub fn layout(&self, world: &World, size: (f32, f32)) -> Vec<TextVertex> {
        let mut v = Vec::with_capacity(6 * 700);
        let (w, h) = size;
        let look = &world.look;
        let tr = &world.trails;
        let sys = &tr.sys;
        let hard = self.hard;
        let c = |rgb: [f32; 3], a: f32| [rgb[0], rgb[1], rgb[2], if hard { (a * 1.9).min(1.0) } else { a }];
        // the system's own text fades with it; the frame stays
        let f = world.fade * world.fade * (3.0 - 2.0 * world.fade);
        let quiet = c(look.text, 0.78 * f);
        let faint = c(look.text, 0.5 * f);
        let live = c(look.text_hi, 0.9 * f);
        let rule = c(look.text, 0.30);
        let lh = (self.px[0] * 1.62).round();
        let a0 = self.adv[0];
        // margins: clear of the bar along the top and the dock along the bottom
        let (mx, mt, mb) = ((w * 0.03).round(), (h * 0.062).round(), (h * 0.056).round());
        let (x0, x1, y0, y1) = (mx, w - mx, mt, h - mb);

        // crop marks at the corners, and a scale of ticks down the left edge
        let arm = (h * 0.012).round();
        for (cx, cy, sx, sy) in [(x0, y0, 1.0, 1.0), (x1, y0, -1.0, 1.0), (x0, y1, 1.0, -1.0), (x1, y1, -1.0, -1.0)] {
            self.rect(&mut v, if sx > 0.0 { cx } else { cx - arm }, cy, arm, 1.0, rule);
            self.rect(&mut v, cx, if sy > 0.0 { cy } else { cy - arm }, 1.0, arm, rule);
        }
        let ticks = 40;
        for i in 0..=ticks {
            let y = y0 + (y1 - y0) * (0.3 + 0.4 * i as f32 / ticks as f32);
            let long = i % 5 == 0;
            self.rect(&mut v, x0 - if long { arm * 0.8 } else { arm * 0.4 }, y, if long { arm * 0.8 } else { arm * 0.4 }, 1.0, c(look.text, if long { 0.34 } else { 0.2 }));
        }

        // top left: the name, the equations, the parameters
        let mut y = y0 + arm + self.px[1] * 1.05;
        let x = x0 + arm;
        self.text(&mut v, &sys.name.to_uppercase(), 1, x, y, false, 0.22, c(look.text, 0.86 * f));
        y += lh * 0.9;
        let (idx, count) = world.system_index();
        self.text(&mut v, &format!("STRANGE ATTRACTOR  {:02}/{:02}", idx + 1, count), 0, x, y, false, 0.12, faint);
        y += lh * 0.7;
        self.rect(&mut v, x, y, a0 * 30.0, 1.0, c(look.text, 0.30 * f));
        y += lh * 1.35;
        for (i, rhs) in sys.eq.iter().enumerate() {
            let lhs = if self.dots { ["ẋ", "ẏ", "ż"][i].to_string() } else { format!("d{}/dt", ["x", "y", "z"][i]) };
            let wl = self.text(&mut v, &lhs, 0, x, y, false, 0.0, quiet);
            let we = self.text(&mut v, " = ", 0, x + wl, y, false, 0.0, faint);
            self.text(&mut v, rhs, 0, x + wl + we, y, false, 0.0, quiet);
            y += lh;
        }
        y += lh * 0.35;
        self.text(&mut v, sys.params, 0, x, y, false, 0.0, live);

        // top right: the rotation, as a row of dots, and what is drawn
        let d = (self.px[0] * 0.36).round().max(2.0);
        let gap = (self.px[0] * 1.15).round();
        let ty = y0 + arm + self.px[0] * 0.9;
        for i in 0..count {
            let on = i == idx;
            let xx = x1 - arm - (count - i) as f32 * gap + gap - d;
            self.rect(&mut v, xx, ty - d, d, d, if on { c(look.text_hi, 0.95 * f.max(0.3)) } else { c(look.text, 0.36) });
        }
        let mut y = ty + lh * 1.25;
        let groups = tr.n / tr.m.max(1);
        for (label, value) in [
            ("LOOK", look.name.to_uppercase()),
            ("STRANDS", format!("{} × {}", groups, tr.m)),
            ("POINTS", group_digits((tr.n * tr.len) as u64)),
        ] {
            self.text(&mut v, &value, 0, x1 - arm, y, true, 0.0, quiet);
            self.text(&mut v, label, 0, x1 - arm - a0 * 12.0, y, true, 0.12, faint);
            y += lh;
        }

        // bottom left: numbers from the integration, as it runs
        let (p, speed) = tr.lead();
        let lam = tr.lyapunov();
        let l1 = if self.glyphs.contains_key(&('₁', 0)) { "λ₁" } else { "λ1" };
        let rows: [(&str, String); 8] = [
            ("t", format!("{:>10.3}", tr.time())),
            ("step", format!("{:>10}", tr.steps)),
            ("Δt", format!("{:>10.5}", tr.cal.dt)),
            ("x", format!("{:>+10.4}", p[0])),
            ("y", format!("{:>+10.4}", p[1])),
            ("z", format!("{:>+10.4}", p[2])),
            ("|v|", format!("{:>10.3}", speed)),
            (l1, if tr.steps > 200 { format!("{:>+10.3}", lam) } else { format!("{:>10}", "·") }),
        ];
        let mut y = y1 - arm - lh * (rows.len() as f32 - 1.0) - self.px[0] * 0.3;
        self.text(&mut v, "RK4 · f64", 0, x, y - lh * 1.5, false, 0.12, faint);
        self.rect(&mut v, x, y - lh * 1.05, a0 * 16.0, 1.0, c(look.text, 0.30 * f));
        for (label, value) in rows.iter() {
            let label = if *label == "Δt" && !self.glyphs.contains_key(&('Δ', 0)) { "dt" } else { label };
            self.text(&mut v, label, 0, x, y, false, 0.0, quiet);
            self.text(&mut v, &value.replace('-', "−"), 0, x + a0 * 16.0, y, true, 0.0, live);
            y += lh;
        }

        // bottom right: the camera, and the maker
        let (around, lean, dist) = world.orbit();
        let mut y = y1 - arm - lh * 3.0 - self.px[0] * 0.3;
        for (label, value) in [
            ("AZIMUTH", format!("{:>7.2}°", around.to_degrees().rem_euclid(360.0))),
            ("ELEVATION", format!("{:>7.2}°", 90.0 - lean.to_degrees())),
            ("DISTANCE", format!("{:>7.3} ", dist)),
        ] {
            self.text(&mut v, &value, 0, x1 - arm, y, true, 0.0, c(look.text_hi, 0.9));
            self.text(&mut v, label, 0, x1 - arm - a0 * 11.0, y, true, 0.12, c(look.text, 0.5));
            y += lh;
        }
        self.text(&mut v, "ATTRACTOR — LAVART STUDIO", 0, x1 - arm, y, true, 0.12, c(look.text, 0.5));
        let _ = systems::ALL;
        v
    }
}

/// a plainer stand-in for a character the font lacks
fn fallback(ch: char) -> char {
    match ch {
        '−' => '-',
        '·' => '.',
        '×' => 'x',
        '—' => '-',
        'σ' => 's',
        'ρ' => 'r',
        'β' => 'b',
        'α' => 'a',
        'γ' => 'c',
        'δ' => 'd',
        'ε' => 'e',
        'ζ' => 'f',
        'λ' => 'L',
        '²' => '2',
        '³' => '3',
        '°' => ' ',
        c => c,
    }
}

fn group_digits(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(' ');
        }
        out.push(ch);
    }
    out
}
