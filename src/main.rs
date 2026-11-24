//! Attractor: strange attractors as a GPU screensaver and a live wallpaper for Omarchy. By lAvArt Studio.

mod config;
mod gpu;
mod logo;
mod render;
mod saver;
mod shot;
mod systems;
mod text;
mod trails;
mod wallpaper;
mod world;

use config::{Config, Mode};

fn arg<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(|s| s.as_str())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!(
            "attractor [--wallpaper | --windowed | --keep] [--system NAME|cycle] [--preset theme|weave|contour|stipple|dust|80s|90s] [--palette auto|theme|original]\n\
             \x20         [--logo none|twinise|omarchy|FILE.svg] [--fps N] [--scale F] [--shot out.png [--size WxH] [--secs N]] [--list]"
        );
        return;
    }
    if args.iter().any(|a| a == "--looks") {
        for s in config::STYLES {
            println!("{s}");
        }
        return;
    }
    if args.iter().any(|a| a == "--list") {
        for s in systems::ALL {
            println!("{}", s.name);
        }
        return;
    }
    let mode = if args.iter().any(|a| a == "--wallpaper") { Mode::Wallpaper } else { Mode::Saver };
    let mut cfg = Config::load(mode);
    if let Some(v) = arg(&args, "--system") { cfg.system = v.into() }
    if let Some(v) = arg(&args, "--preset") { cfg.preset = v.into() }
    if let Some(v) = arg(&args, "--palette") { cfg.palette = v.into() }
    if args.iter().any(|a| a == "--no-annotations") { cfg.annotations = false }
    if args.iter().any(|a| a == "--no-lines") { cfg.lines = Some(false) }
    if args.iter().any(|a| a == "--no-trackers") { cfg.trackers = false }
    if let Some(v) = arg(&args, "--canvas").and_then(|v| v.parse().ok()) { cfg.canvas = v }
    if let Some(v) = arg(&args, "--orbit").and_then(|v| v.parse().ok()) { cfg.orbit = v }
    if let Some(v) = arg(&args, "--logo") { cfg.logo = v.into() }
    if let Some(v) = arg(&args, "--fps").and_then(|v| v.parse().ok()) { cfg.fps = v }
    if let Some(v) = arg(&args, "--scale").and_then(|v| v.parse().ok()) { cfg.render_scale = Some(v) }
    gpu::prepare_env(&cfg);
    let seed = arg(&args, "--seed").and_then(|v| v.parse().ok()).unwrap_or_else(|| std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(7));

    if let Some(n) = arg(&args, "--bench").and_then(|v| v.parse::<u32>().ok()) {
        let size = arg(&args, "--size").and_then(|v| v.split_once('x')).and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?))).unwrap_or((3200, 2000));
        shot::bench(&cfg, mode, size, n, seed);
        return;
    }
    if args.iter().any(|a| a == "--dark-check") {
        let size = arg(&args, "--size").and_then(|v| v.split_once('x')).and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?))).unwrap_or((3200, 2000));
        let secs = arg(&args, "--secs").and_then(|v| v.parse().ok()).unwrap_or(30.0);
        shot::dark_check(&cfg, mode, size, secs, seed);
        return;
    }
    if let Some(n) = arg(&args, "--flicker").and_then(|v| v.parse::<u32>().ok()) {
        let size = arg(&args, "--size").and_then(|v| v.split_once('x')).and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?))).unwrap_or((3200, 2000));
        let prefix = arg(&args, "--out").unwrap_or("flicker");
        shot::flicker(&cfg, mode, prefix, size, n, cfg.fps, args.iter().any(|a| a == "--hold"), seed);
        return;
    }
    if let Some(path) = arg(&args, "--shot") {
        let size = arg(&args, "--size").and_then(|v| v.split_once('x')).and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?))).unwrap_or((1920, 1080));
        let secs = arg(&args, "--secs").and_then(|v| v.parse().ok()).unwrap_or(30.0);
        shot::run(&cfg, mode, path, size, secs, seed);
        return;
    }
    match mode {
        Mode::Saver => saver::run(cfg, seed, args.iter().any(|a| a == "--windowed"), args.iter().any(|a| a == "--keep")),
        Mode::Wallpaper => wallpaper::run(cfg, seed),
    }
}
