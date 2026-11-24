//! The living scene: trails on the current attractor, the orbiting camera, and the slow rotation through systems.

use crate::config::{Config, Look};
use crate::logo::{self, Logo};
use crate::systems;
use crate::trails::Trails;
use glam::{Mat4, Vec3, Vec4};

/// A tracking box: it locks onto a point of the form, follows it for a while, and lets go.
pub struct Tracker {
    /// where it is (world coordinates), eased toward what it follows
    pub pos: Vec3,
    /// 0 unseen .. 1 fully there
    pub alpha: f32,
    /// its name and what it reads, refreshed a few times a second
    pub tag: String,
    pub label: String,
    /// box size, relative
    pub size: f32,
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
    held: f32,
    /// how far the camera has stepped back to keep the whole form in the picture (1: not at all), and where that is heading
    reach: f32,
    reach_goal: f32,
    reach_poll: f32,
    pub trackers: Vec<Tracker>,
    track_poll: f32,
}

impl World {
    pub fn new(cfg: &Config, seed: u64) -> World {
        let cycling = systems::find(&cfg.system).is_none();
        // a different starting attractor each run, rather than always Lorenz
        let first = systems::find(&cfg.system).unwrap_or((seed as usize >> 3) % systems::ALL.len());
        let order: Vec<usize> = (0..systems::ALL.len()).map(|i| (first + i) % systems::ALL.len()).collect();
        let look = Look::resolve(cfg);
        let trails = World::make_trails(&look, cfg, order[0], seed, true);
        World { cfg: cfg.clone(), from_look: look.clone(), target_look: look.clone(), blend: 1.0, theme_stamp: crate::config::theme_stamp(), theme_poll: 0.0, look, trails, logo: logo::load(&cfg.logo), t: 0.0, fade: 1.0, epoch: 0, order, idx: 0, since: 0.0, phase: Phase::Run, acc: 0.0, seed, cycling, cam_phase: (seed % 1000) as f32, hold_camera: false, held: 0.0, reach: 1.0, reach_goal: 1.0, reach_poll: 0.0, trackers: World::make_trackers(cfg, seed), track_poll: 0.0 }
    }

    fn make_trails(look: &Look, cfg: &Config, sys: usize, seed: u64, spread: bool) -> Trails {
        let (n, len) = if cfg.wall { (look.trails.0, look.length.0) } else { (look.trails.1, look.length.1) };
        Trails::new(systems::ALL[sys], n, len, look.strands, look.pitch as f64, seed, spread)
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
                Tracker { pos: Vec3::ZERO, alpha: 0.0, tag: String::new(), label: String::new(), size: 1.0, mean: i == 2, mark: 0, what: "", clock: period * (0.37 * i as f32 + (seed % 97) as f32 / 97.0).fract(), period, cycle: (seed as u32).wrapping_add(i as u32 * 7919) }
            })
            .collect()
    }

    /// move the tracking boxes: each eases toward the point it follows, fades in over a few seconds, stays, fades
    /// out, rests, and then picks something else
    fn track(&mut self, dt: f32) {
        if self.trackers.is_empty() {
            return;
        }
        let tr = &self.trails;
        let fade_all = self.fade;
        let heads = &tr.ring[tr.head * tr.n..(tr.head + 1) * tr.n];
        self.track_poll += dt;
        let refresh = self.track_poll >= 0.25;
        if refresh {
            self.track_poll = 0.0;
        }
        let hash = |x: u32| {
            let mut h = x.wrapping_mul(747796405).wrapping_add(2891336453);
            h = ((h >> ((h >> 28) + 4)) ^ h).wrapping_mul(277803737);
            (h >> 22) ^ h
        };
        for (i, t) in self.trackers.iter_mut().enumerate() {
            t.clock += dt;
            let fresh = t.clock >= t.period || t.tag.is_empty();
            if fresh {
                if t.clock >= t.period {
                    t.clock = 0.0;
                }
                t.cycle = t.cycle.wrapping_add(1);
                t.size = 0.8 + 0.7 * (hash(t.cycle ^ 0x9e37) % 1000) as f32 / 1000.0;
                // the next mark that no other box is on
                if !tr.marks.is_empty() {
                    t.mark = (t.mark + 1 + hash(t.cycle) as usize % tr.marks.len()) % tr.marks.len();
                }
            }
            // seen for a little over half its time
            let seen = t.period * 0.58;
            let fade = 3.0;
            t.alpha = (t.clock / fade).min((seen - t.clock) / fade).clamp(0.0, 1.0);
            t.alpha = t.alpha * t.alpha * (3.0 - 2.0 * t.alpha) * fade_all;
            let at = |p: &[f32; 4]| Vec3::new(p[0], p[1], p[2]);
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
            if refresh || fresh {
                let p = tr.system_of([t.pos.x, t.pos.y, t.pos.z]);
                t.tag = format!("{:02} {}", i + 1, t.what);
                t.label = format!("{:+.2} {:+.2} {:+.2}  |v| {:.2}", p[0], p[1], p[2], tr.speed_at(p));
            }
        }
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
        if !self.hold_camera {
            self.held = self.t;
        }
        self.since += dt;
        self.acc += dt * STEPS_PER_SEC * self.cfg.speed;
        let n = (self.acc as usize).min(64);
        self.acc -= self.acc.floor();
        for _ in 0..n {
            self.trails.step();
        }
        match self.phase {
            Phase::Run => {
                if self.cycling && self.since > self.cfg.cycle {
                    self.phase = Phase::Out(0.0);
                }
            }
            Phase::Out(s) => {
                let s = s + dt;
                self.fade = (1.0 - s / FADE).clamp(0.0, 1.0);
                if s >= FADE {
                    self.idx = (self.idx + 1) % self.order.len();
                    self.seed = self.seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                    // the next attractor is born as a point and grows out of it
                    self.trails = World::make_trails(&self.look, &self.cfg, self.order[self.idx], self.seed, false);
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
        let t = if self.hold_camera { self.held } else { self.t } + self.cam_phase;
        let rev = self.cfg.orbit.max(10.0);
        (t * std::f32::consts::TAU / rev, 0.62 + 0.20 * (t * 0.045).sin(), 4.1 * self.reach / self.cfg.zoom.max(0.2) * (1.0 + 0.03 * (t * 0.017).sin()))
    }

    /// how far the picture is slid sideways and up, in units of half the screen
    pub fn shift(&self) -> (f32, f32) {
        (self.cfg.shift_x.unwrap_or(if self.logo.is_some() { -0.3 } else { 0.0 }), self.cfg.shift_y)
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
