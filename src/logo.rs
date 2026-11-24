//! The optional logo: any SVG, rasterised once into a premultiplied RGBA mask that the composite pass draws behind
//! the trails. `"twinise"` and `"omarchy"` are built in; anything else is a path. Text must be converted to paths.

use resvg::{tiny_skia, usvg};
use std::path::PathBuf;

pub struct Logo {
    /// premultiplied RGBA8
    pub rgba: Vec<u8>,
    pub w: u32,
    pub h: u32,
}

const TWINISE: &str = include_str!("../assets/twinise.svg");

fn expand(s: &str) -> PathBuf {
    match s.strip_prefix("~/") {
        Some(r) => PathBuf::from(crate::config::home()).join(r),
        None => PathBuf::from(s),
    }
}

pub fn load(spec: &str) -> Option<Logo> {
    let svg = match spec.trim() {
        "" | "none" => return None,
        "twinise" => TWINISE.to_string(),
        "omarchy" => std::fs::read_to_string("/usr/share/omarchy/logo.svg").map_err(|e| eprintln!("attractor: logo: {e}")).ok()?,
        p => std::fs::read_to_string(expand(p)).map_err(|e| eprintln!("attractor: logo {p}: {e}")).ok()?,
    };
    let tree = usvg::Tree::from_str(&svg, &usvg::Options::default()).map_err(|e| eprintln!("attractor: logo: {e}")).ok()?;
    let size = tree.size();
    let k = 1536.0 / size.width().max(size.height());
    let (w, h) = (((size.width() * k).ceil() as u32).max(1), ((size.height() * k).ceil() as u32).max(1));
    let mut pm = tiny_skia::Pixmap::new(w, h)?;
    resvg::render(&tree, tiny_skia::Transform::from_scale(k, k), &mut pm.as_mut());
    Some(Logo { rgba: pm.take(), w, h })
}
