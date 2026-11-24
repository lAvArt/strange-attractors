//! The living scene: trails on the current attractor, the orbiting camera, and the slow rotation through systems.

use crate::config::{Config, Look};
use crate::logo::{self, Logo};
use crate::systems;
use crate::trails::Trails;
use glam::{Mat4, Vec3, Vec4};

/// Where a tracker's data box may stand: slots in the empty space left and right of the form, between the caption
/// blocks, as fractions of the screen (left, top). A box is `SLOT_SIZE` of the screen.
pub const SLOTS: [(f32, f32); 8] = [(0.045, 0.30), (0.045, 0.42), (0.045, 0.54), (0.885, 0.22), (0.885, 0.34), (0.885, 0.46), (0.885, 0.58), (0.885, 0.70)];
pub const SLOT_SIZE: (f32, f32) = (0.076, 0.105);

/// A tracker: a small anchor mark on a point of the form, and out in the empty space a box that reads it, joined to
/// it by a leader line. It locks on, stays a while, and lets go.
pub struct Tracker {
    /// where its anchor is (world coordinates), eased toward what it follows
    pub pos: Vec3,
    /// 0 unseen .. 1 fully there
    pub alpha: f32,
    /// its name and what it reads (x, y, z, speed), refreshed a few times a second
    pub tag: String,
    pub lines: [String; 4],
    /// which slot its box stands in for this appearance: chosen once when it appears and kept, so a box never hops
    pub slot: Option<usize>,
    /// 1 while that slot is clear of the form, easing to 0 if the form comes to pass through it
    clear: f32,
    /// what it is locked onto: one of the form's marks (they stand still), or, for `mean`, the slowly wandering
    /// middle of all the strands' heads
    mean: bool,
    mark: usize,
    what: &'static str,
    clock: f32,
    period: f32,
    cycle: u32,
}

enum Phase {
    Run,
    /// seconds into fading the old attractor out
    Out(f32),
    /// seconds into fading the new one in
    In(f32),
}

const FADE: f32 = 2.4;
/// scenes written by another version are not taken over
const SCENE_VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "-1");

/// where the wallpaper publishes its scene
pub fn scene_path() -> String {
    let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| format!("{}/.cache", crate::config::home()));
    format!("{dir}/attractor/scene.json")
}
/// attractor steps per second at speed 1
const STEPS_PER_SEC: f32 = 100.0;

pub struct World {
    pub cfg: Config,
    pub look: Look,
    /// a running crossfade between two looks (a theme change), 1 = finished
    from_look: Look,
    target_look: Look,
    blend: f32,
    theme_stamp: String,
    theme_poll: f32,
    pub trails: Trails,
    pub logo: Option<Logo>,
    pub t: f32,
    pub fade: f32,
    /// bumps whenever the trails are replaced, so renderers know to re-upload everything
    pub epoch: u32,
    order: Vec<usize>,
    idx: usize,
    since: f32,
    phase: Phase,
    acc: f32,
    seed: u64,
    cycling: bool,
    /// where on its path the camera starts: different each run
    cam_phase: f32,
    /// hold the camera still (the flicker test: with the camera held, nothing that is drawn should change at all
    /// from one frame to the next except the growing heads and fading tails)
    pub hold_camera: bool,
    /// how far round the camera has gone (radians), and the clock its lean and distance breathe by: kept as state
    /// rather than worked out from the time, so a screensaver can take the wallpaper's camera over and speed it up
    cam_angle: f32,
    cam_clock: f32,
    /// waking from a wallpaper's scene: 0 at its slow pace .. 1 at this world's own; and what that pace was
    wake: f32,
    from_speed: f32,
    from_orbit: f32,
    /// publish the scene for a screensaver to pick up (the wallpaper does)
    pub publish: bool,
    publish_poll: f32,
    /// how far the camera has stepped back to keep the whole form in the picture (1: not at all), and where that is heading
    reach: f32,
    reach_goal: f32,
    reach_poll: f32,
    pub trackers: Vec<Tracker>,
    track_poll: f32,
    slot_busy: [bool; SLOTS.len()],
    /// the next attractor, being prepared on a thread of its own while the current one fades out
    pending: Option<std::thread::JoinHandle<Trails>>,
    /// which way round the camera goes
    turn: f32,
    /// wait for the next attractor rather than hold an empty frame for it (offscreen recording: every run of a seed
    /// must then give the same frames)
    pub blocking: bool,
}

impl World {
    pub fn new(cfg: &Config, seed: u64) -> World {
        World::build(cfg, seed, None)
    }

    /// `given`: a system and its strands already made (a scene taken over), instead of a fresh instance
    fn build(cfg: &Config, seed: u64, given: Option<(usize, Trails)>) -> World {
        let cycling = systems::find(&cfg.system).is_none();
        // a different starting attractor each run, rather than always Lorenz
        let first = given.as_ref().map(|g| g.0).or(systems::find(&cfg.system)).unwrap_or((seed as usize >> 3) % systems::ALL.len());
        // the rotation: every system once, in an order of this run's own, starting with `first`
        let mut order: Vec<usize> = (0..systems::ALL.len()).collect();
        let mut h = seed.wrapping_mul(0x9E3779B97F4A7C15) | 1;
        for i in (1..order.len()).rev() {
            h ^= h << 13;
            h ^= h >> 7;
            h ^= h << 17;
            order.swap(i, (h >> 20) as usize % (i + 1));
        }
        let at = order.iter().position(|&s| s == first).unwrap_or(0);
        order.rotate_left(at);
        let look = Look::resolve(cfg);
        let trails = match given {
            Some((_, t)) => t,
            None => World::make_trails(&look, cfg, order[0], seed, true),
        };
        let turn = if (seed >> 3) & 1 == 0 { 1.0 } else { -1.0 };
        let cam_phase = (seed % 1000) as f32;
        World { cfg: cfg.clone(), from_look: look.clone(), target_look: look.clone(), blend: 1.0, theme_stamp: crate::config::theme_stamp(), theme_poll: 0.0, look, trails, logo: logo::load(&cfg.logo), t: 0.0, fade: 1.0, epoch: 0, order, idx: 0, since: 0.0, phase: Phase::Run, acc: 0.0, seed, cycling, cam_phase, hold_camera: false, cam_angle: turn * cam_phase * std::f32::consts::TAU / 80.0, cam_clock: cam_phase, wake: 1.0, from_speed: cfg.speed, from_orbit: cfg.orbit, publish: false, publish_poll: 0.0, reach: 1.0, reach_goal: 1.0, reach_poll: 0.0, trackers: World::make_trackers(cfg, seed), track_poll: 0.0, slot_busy: [false; SLOTS.len()], pending: None, blocking: false, turn }
    }

    /// A new instance of system `sys`: parameters of its own (drawn and tested by `systems::draw`), fitted and framed
    /// afresh, its strands seeded from `seed`. Takes some tens of milliseconds.
    fn make_trails(look: &Look, cfg: &Config, sys: usize, seed: u64, spread: bool) -> Trails {
        let (n, len) = if cfg.wall { (look.trails.0, look.length.0) } else { (look.trails.1, look.length.1) };
        let instance = if cfg.fixed_params { systems::ALL[sys] } else { systems::draw(&systems::ALL[sys], seed).0 };
        Trails::new(instance, n, len, look.strands, look.pitch as f64, seed, spread)
    }

    /// the seed of the instance now running: `--seed N --system NAME` brings it back
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// With the `theme` look, notice when the system theme changes (checked once a second, a single stat call) and
    /// crossfade to the new colours over a moment. Call it on every tick, even while nothing is drawn.
    pub fn poll_theme(&mut self, dt: f32) {
        if self.blend < 1.0 {
            self.blend = (self.blend + dt / 1.6).min(1.0);
            let t = self.blend * self.blend * (3.0 - 2.0 * self.blend);
            self.look = Look::lerp(&self.from_look, &self.target_look, t);
        }
        self.theme_poll += dt;
        if self.theme_poll < 1.0 {
            return;
        }
        self.theme_poll = 0.0;
        let stamp = crate::config::theme_stamp();
        if stamp != self.theme_stamp {
            self.theme_stamp = stamp;
            self.from_look = self.look.clone();
            self.target_look = Look::resolve(&self.cfg);
            self.blend = 0.0;
            if crate::gpu::debug() {
                eprintln!("attractor: theme changed, blending to the new colours");
            }
        }
    }

    /// The fit is made before anything is drawn, from where the strands were then. Ribbons can ride wider later (and a
    /// rare far loop can come round), so twice a second look at where the strands' heads are now, and let the camera
    /// ease back when they would leave the picture and, slowly, forward again when they no longer would.
    fn keep_in_frame(&mut self, dt: f32) {
        self.reach_poll += dt;
        if self.reach_poll >= 0.5 {
            self.reach_poll = 0.0;
            let tr = &self.trails;
            let mut need = 1.0f32;
            for p in &tr.ring[tr.head * tr.n..(tr.head + 1) * tr.n] {
                if p[3] != 0.0 {
                    // (the picture turns, so any direction in the form's plane comes to stand upright on the screen)
                    need = need.max((p[0] * p[0] + p[1] * p[1]).sqrt() / 1.42).max((p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt() / 1.75);
                }
            }
            // what was far half a minute ago still counts: its trail is on screen for a while, and it will be back
            self.reach_goal = need.min(2.5).max(1.0 + (self.reach_goal - 1.0) * 0.985);
        }
        if !self.hold_camera {
            let rate = if self.reach_goal > self.reach { 0.9 } else { 0.08 };
            self.reach += (self.reach_goal - self.reach) * (1.0 - (-dt * rate).exp());
        }
    }

    fn make_trackers(cfg: &Config, seed: u64) -> Vec<Tracker> {
        if !cfg.trackers {
            return Vec::new();
        }
        // fewer on the wallpaper; staggered so they never come or go together
        let count = if cfg.wall { 4 } else { 6 };
        (0..count)
            .map(|i| {
                let period = 19.0 + 4.3 * i as f32;
                Tracker { pos: Vec3::ZERO, alpha: 0.0, tag: String::new(), lines: Default::default(), slot: None, clear: 1.0, mean: i == 2, mark: i, what: "", clock: period * (0.37 * i as f32 + (seed % 97) as f32 / 97.0).fract(), period, cycle: (seed as u32).wrapping_add(i as u32 * 7919) }
            })
            .collect()
    }

    /// Move the trackers. Each anchor eases toward the point it follows; each box takes, when it appears, the free
    /// slot nearest its anchor and keeps it; boxes fade in over a few seconds, stay, fade out and rest, never all at
    /// once. Slots the form passes through are left alone (and a box whose slot the form enters fades until it has
    /// passed).
    fn track(&mut self, dt: f32) {
        if self.trackers.is_empty() {
            return;
        }
        self.track_poll += dt;
        let refresh = self.track_poll >= 0.25;
        if refresh {
            self.track_poll = 0.0;
        }
        // (laid out for a 16:10 screen: on another shape the slots are only a little off)
        let (view, proj, _) = self.camera(1.6);
        let vp = proj * view;
        let screen = |p: Vec3| {
            let c = vp * p.extend(1.0);
            (c.x / c.w * 0.5 + 0.5, 0.5 - c.y / c.w * 0.5)
        };
        let tr = &self.trails;
        let fade_all = self.fade;
        let heads = &tr.ring[tr.head * tr.n..(tr.head + 1) * tr.n];
        // which slots the form is in: a few hundred of its points, a few times a second
        let mut busy = [false; SLOTS.len()];
        if refresh {
            let step = (tr.ring.len() / 600).max(1);
            for p in tr.ring.iter().step_by(step) {
                if p[3] == 0.0 {
                    continue;
                }
                let (x, y) = screen(Vec3::new(p[0], p[1], p[2]));
                for (i, s) in SLOTS.iter().enumerate() {
                    if x > s.0 - 0.012 && x < s.0 + SLOT_SIZE.0 + 0.012 && y > s.1 - 0.015 && y < s.1 + SLOT_SIZE.1 + 0.015 {
                        busy[i] = true;
                    }
                }
            }
            self.slot_busy = busy;
        }
        let busy = self.slot_busy;
        let hash = |x: u32| {
            let mut h = x.wrapping_mul(747796405).wrapping_add(2891336453);
            h = ((h >> ((h >> 28) + 4)) ^ h).wrapping_mul(277803737);
            (h >> 22) ^ h
        };
        for i in 0..self.trackers.len() {
            let taken: Vec<usize> = self.trackers.iter().enumerate().filter(|(k, t)| *k != i && t.alpha > 0.0).filter_map(|(_, t)| t.slot).collect();
            let marks_taken: Vec<usize> = self.trackers.iter().enumerate().filter(|(k, t)| *k != i && !t.mean).map(|(_, t)| t.mark).collect();
            let t = &mut self.trackers[i];
            t.clock += dt;
            let fresh = t.clock >= t.period || t.tag.is_empty();
            let at = |p: &[f32; 4]| Vec3::new(p[0], p[1], p[2]);
            if fresh {
                if t.clock >= t.period {
                    t.clock = 0.0;
                }
                t.cycle = t.cycle.wrapping_add(1);
                // a mark no other tracker is on
                if !tr.marks.is_empty() {
                    let start = hash(t.cycle) as usize % tr.marks.len();
                    t.mark = (0..tr.marks.len()).map(|k| (start + k) % tr.marks.len()).find(|m| !marks_taken.contains(m)).unwrap_or(start);
                }
            }
            let (goal, what) = if t.mean || tr.marks.is_empty() {
                (heads.iter().map(at).sum::<Vec3>() / heads.len() as f32, "MEAN")
            } else {
                let (name, p) = tr.marks[t.mark % tr.marks.len()];
                (Vec3::from(p), name)
            };
            t.what = what;
            if goal.is_finite() && goal.length() < 4.0 {
                let k = if fresh || t.alpha <= 0.0 { 1.0 } else { 1.0 - (-dt / 1.5).exp() };
                t.pos += (goal - t.pos) * k;
            }
            if fresh {
                // the nearest slot that is free and clear of the form
                let (ax, ay) = screen(t.pos);
                t.slot = (0..SLOTS.len())
                    .filter(|s| !taken.contains(s) && !busy[*s])
                    .min_by(|a, b| {
                        let d = |s: usize| (SLOTS[s].0 + SLOT_SIZE.0 * 0.5 - ax).powi(2) + ((SLOTS[s].1 + SLOT_SIZE.1 * 0.5 - ay) * 0.625).powi(2);
                        d(*a).partial_cmp(&d(*b)).unwrap()
                    });
                t.clear = 1.0;
            }
            // seen for a little over half its time
            let seen = t.period * 0.58;
            let fade = 3.0;
            let a = (t.clock / fade).min((seen - t.clock) / fade).clamp(0.0, 1.0);
            let want = if t.slot.is_some_and(|s| !busy[s]) { 1.0 } else { 0.0 };
            t.clear += (want - t.clear) * (1.0 - (-dt / 0.8).exp());
            t.alpha = a * a * (3.0 - 2.0 * a) * fade_all * t.clear * t.slot.is_some() as u8 as f32;
            if refresh || fresh {
                let p = tr.system_of([t.pos.x, t.pos.y, t.pos.z]);
                t.tag = format!("{:02} {}", i + 1, t.what);
                t.lines = [format!("x {:>+8.2}", p[0]), format!("y {:>+8.2}", p[1]), format!("z {:>+8.2}", p[2]), format!("v {:>8.2}", tr.speed_at(p))];
            }
        }
    }

    /// a fingerprint of where every strand's head is
    fn checksum(&self) -> String {
        let tr = &self.trails;
        let mut h = 0xcbf29ce484222325u64;
        for p in &tr.ring[tr.head * tr.n..(tr.head + 1) * tr.n] {
            for v in p {
                h = (h ^ v.to_bits() as u64).wrapping_mul(0x100000001b3);
            }
        }
        format!("{h:016x}")
    }

    /// Write down the scene (which system, which seed, how many steps in, where the camera is) so that a screensaver
    /// starting now can rebuild exactly this picture and carry on from it. Written beside and renamed, so a reader
    /// never sees half a file.
    pub fn publish_scene(&self, path: &str) {
        if !matches!(self.phase, Phase::Run) {
            // nothing to take over in the middle of a change: let the old file go stale
            return;
        }
        let tr = &self.trails;
        let v = serde_json::json!({
            "version": SCENE_VERSION, "system": tr.sys.name, "params": tr.sys.params(), "seed": self.seed, "classic": self.cfg.fixed_params,
            "steps": tr.steps, "n": tr.n, "len": tr.len, "m": tr.m, "pitch": self.look.pitch, "look": self.look.name, "palette": self.cfg.palette,
            "cam_angle": self.cam_angle, "cam_clock": self.cam_clock, "reach": self.reach, "reach_goal": self.reach_goal, "turn": self.turn,
            "t": self.t, "speed": self.cfg.speed, "orbit": self.cfg.orbit, "zoom": self.cfg.zoom, "checksum": self.checksum(),
            "at": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0),
        });
        let p = std::path::Path::new(path);
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = format!("{path}.tmp");
        if std::fs::write(&tmp, v.to_string()).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }

    /// A world that starts as the scene written at `path`: the same system, parameters and seed, the simulation run
    /// forward the same number of steps, the camera where it was. `None` (start afresh) if there is no such file, it
    /// is older than `max_age` seconds, it is another version's or another look's, or the rebuilt strands do not
    /// match its fingerprint.
    pub fn from_scene(cfg: &Config, path: &str, max_age: f64) -> Option<World> {
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_secs_f64();
        if v["version"].as_str()? != SCENE_VERSION || (now - v["at"].as_f64()?).abs() > max_age {
            return None;
        }
        let look = Look::resolve(cfg);
        if v["look"].as_str()? != look.name || v["palette"].as_str()? != cfg.palette || (v["zoom"].as_f64()? as f32 - cfg.zoom).abs() > 1e-6 {
            return None;
        }
        let idx = systems::find(v["system"].as_str()?)?;
        let seed = v["seed"].as_u64()?;
        let instance = if v["classic"].as_bool()? { systems::ALL[idx] } else { systems::draw(&systems::ALL[idx], seed).0 };
        if instance.params() != v["params"].as_str()? {
            return None;
        }
        let steps = v["steps"].as_u64()?;
        let mut trails = Trails::new(instance, v["n"].as_u64()? as usize, v["len"].as_u64()? as usize, v["m"].as_u64()? as usize, v["pitch"].as_f64()?, seed, true);
        if steps < trails.steps || steps - trails.steps > 60_000 {
            return None;
        }
        while trails.steps < steps {
            trails.step();
        }
        let mut w = World::build(cfg, seed, Some((idx, trails)));
        if w.checksum() != v["checksum"].as_str()? {
            if crate::gpu::debug() {
                eprintln!("attractor: the wallpaper's scene could not be rebuilt exactly, starting afresh");
            }
            return None;
        }
        w.cam_angle = v["cam_angle"].as_f64()? as f32;
        w.cam_clock = v["cam_clock"].as_f64()? as f32;
        w.reach = v["reach"].as_f64()? as f32;
        w.reach_goal = v["reach_goal"].as_f64()? as f32;
        w.turn = v["turn"].as_f64()? as f32;
        w.from_speed = v["speed"].as_f64()? as f32;
        w.from_orbit = v["orbit"].as_f64()? as f32;
        // (the dither and the margins' clocks run on this)
        w.t = v["t"].as_f64()? as f32;
        w.wake = 0.0;
        Some(w)
    }

    /// the screensaver's world: the wallpaper's scene if there is one to take over, else a fresh one
    pub fn start(cfg: &Config, seed: u64) -> World {
        let taken = if cfg.handoff && !cfg.wall { World::from_scene(cfg, &scene_path(), 4.0) } else { None };
        if crate::gpu::debug() {
            eprintln!("attractor: {}", if taken.is_some() { "taking over the wallpaper's scene" } else { "a fresh scene" });
        }
        taken.unwrap_or_else(|| World::new(cfg, seed))
    }

    /// nothing is fading: neither one attractor into the next nor one theme into another
    pub fn settled(&self) -> bool {
        self.fade >= 1.0 && self.blend >= 1.0
    }

    /// which attractor of the rotation is running: (index, count)
    pub fn system_index(&self) -> (usize, usize) {
        (self.order[self.idx], systems::ALL.len())
    }

    pub fn system_name(&self) -> &'static str {
        self.trails.sys.name
    }

    pub fn update(&mut self, dt: f32) {
        let dt = dt.clamp(0.0, 0.25);
        self.t += dt;
        self.keep_in_frame(dt);
        self.track(dt);
        // a world that took a wallpaper's scene over wakes from its pace to its own over two seconds
        let mut speed = self.cfg.speed;
        let mut rate = 1.0 / self.cfg.orbit.max(10.0);
        if self.wake < 1.0 {
            self.wake = (self.wake + dt / 2.0).min(1.0);
            let s = self.wake * self.wake * (3.0 - 2.0 * self.wake);
            speed = self.from_speed + (speed - self.from_speed) * s;
            rate = 1.0 / self.from_orbit.max(10.0) + (rate - 1.0 / self.from_orbit.max(10.0)) * s;
        }
        if !self.hold_camera {
            self.cam_angle += self.turn * std::f32::consts::TAU * rate * dt;
            // (the lean and the distance breathe at the orbit's pace: a slow orbit is slow in everything)
            self.cam_clock += dt * 80.0 * rate;
        }
        if self.publish {
            self.publish_poll += dt;
            if self.publish_poll >= 1.0 {
                self.publish_poll = 0.0;
                self.publish_scene(&scene_path());
            }
        }
        self.since += dt;
        self.acc += dt * STEPS_PER_SEC * speed;
        let n = (self.acc as usize).min(64);
        self.acc -= self.acc.floor();
        for _ in 0..n {
            self.trails.step();
        }
        match self.phase {
            Phase::Run => {
                if self.cycling && self.since > self.cfg.cycle {
                    self.phase = Phase::Out(0.0);
                    // prepare the next attractor while this one fades, so the switch itself costs no frame
                    let next = self.order[(self.idx + 1) % self.order.len()];
                    let seed = (self.seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407) >> 33) % 65536;
                    let (look, cfg) = (self.look.clone(), self.cfg.clone());
                    // (the wallpaper is nearly a still: its next picture arrives fully formed; the screensaver's grows from a point)
                    self.pending = Some(std::thread::spawn(move || { let formed = cfg.wall; World::make_trails(&look, &cfg, next, seed, formed) }));
                }
            }
            Phase::Out(s) => {
                let s = s + dt;
                self.fade = (1.0 - s / FADE).clamp(0.0, 1.0);
                if s >= FADE && (self.blocking || self.pending.as_ref().is_none_or(|p| p.is_finished())) {
                    self.idx = (self.idx + 1) % self.order.len();
                    self.seed = (self.seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407) >> 33) % 65536;
                    // the next attractor is born as a point and grows out of it
                    self.trails = match self.pending.take().and_then(|p| p.join().ok()) {
                        Some(t) => t,
                        None => World::make_trails(&self.look, &self.cfg, self.order[self.idx], self.seed, self.cfg.wall),
                    };
                    self.turn = if (self.seed >> 3) & 1 == 0 { 1.0 } else { -1.0 };
                    self.epoch += 1;
                    self.phase = Phase::In(0.0);
                } else {
                    self.phase = Phase::Out(s);
                }
            }
            Phase::In(s) => {
                let s = s + dt;
                self.fade = (s / FADE).clamp(0.0, 1.0);
                if s >= FADE {
                    self.since = 0.0;
                    self.phase = Phase::Run;
                } else {
                    self.phase = Phase::In(s);
                }
            }
        }
    }

    /// The camera's place: how far round the form it has gone, its lean away from the form's own axis (both radians),
    /// and its distance. It orbits the axis the form is thinnest along, like a piece on a turntable seen from 43 to 66
    /// degrees above its plane: a full, continuous orbit in which the form is never edge-on.
    pub fn orbit(&self) -> (f32, f32, f32) {
        let t = self.cam_clock;
        (self.cam_angle, 0.62 + 0.20 * (t * 0.045).sin(), 4.1 * self.reach / self.cfg.zoom.max(0.2) * (1.0 + 0.03 * (t * 0.017).sin()))
    }

    /// how far the picture is slid sideways and up, in units of half the screen
    pub fn shift(&self) -> (f32, f32) {
        // with captions down the left, the form sits a little right of centre, as a plate would be composed
        (self.cfg.shift_x.unwrap_or(if self.logo.is_some() { -0.3 } else if self.cfg.annotations { 0.07 } else { 0.0 }), self.cfg.shift_y)
    }

    /// view and projection of the camera, and its distance
    pub fn camera(&self, aspect: f32) -> (Mat4, Mat4, f32) {
        let (around, lean, dist) = self.orbit();
        let eye = Vec3::new(lean.sin() * around.cos(), lean.sin() * around.sin(), lean.cos()) * dist;
        let view = Mat4::look_at_rh(eye, Vec3::ZERO, Vec3::Z);
        let fov = 38f32.to_radians();
        // a tight depth range around the form, so a 16-bit depth buffer is plenty
        let proj = Mat4::perspective_rh(fov, aspect, (dist - 3.2).max(0.2), dist + 3.2);
        // slide the picture in screen space (added to clip x/y in proportion to w), to leave room for the logo
        let (dx, dy) = self.shift();
        let shift = Mat4::from_cols(Vec4::X, Vec4::Y, Vec4::Z, Vec4::new(dx, dy, 0.0, 1.0));
        (view, shift * proj, dist)
    }
}
