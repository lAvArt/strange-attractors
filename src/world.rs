//! The living scene: trails on the current attractor, the orbiting camera, and the slow rotation through systems.

use crate::config::{Config, Look};
use crate::logo::{self, Logo};
use crate::systems;
use crate::trails::Trails;
use glam::{Mat4, Vec3, Vec4};

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
}

impl World {
    pub fn new(cfg: &Config, seed: u64) -> World {
        let cycling = systems::find(&cfg.system).is_none();
        // a different starting attractor each run, rather than always Lorenz
        let first = systems::find(&cfg.system).unwrap_or((seed as usize >> 3) % systems::ALL.len());
        let order: Vec<usize> = (0..systems::ALL.len()).map(|i| (first + i) % systems::ALL.len()).collect();
        let look = Look::resolve(cfg);
        let trails = World::make_trails(&look, cfg, order[0], seed, true);
        World { cfg: cfg.clone(), from_look: look.clone(), target_look: look.clone(), blend: 1.0, theme_stamp: crate::config::theme_stamp(), theme_poll: 0.0, look, trails, logo: logo::load(&cfg.logo), t: 0.0, fade: 1.0, epoch: 0, order, idx: 0, since: 0.0, phase: Phase::Run, acc: 0.0, seed, cycling }
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

    /// the slowly orbiting camera's angles (radians) and distance
    pub fn orbit(&self) -> (f32, f32, f32) {
        let t = self.t;
        (t * 0.055, 0.30 + 0.20 * (t * 0.037).sin(), 4.1 / self.cfg.zoom.max(0.2) * (1.0 + 0.035 * (t * 0.021).sin()))
    }

    /// how far the picture is slid sideways and up, in units of half the screen
    pub fn shift(&self) -> (f32, f32) {
        (self.cfg.shift_x.unwrap_or(if self.logo.is_some() { -0.3 } else { 0.0 }), self.cfg.shift_y)
    }

    /// view and projection of the camera, and its distance
    pub fn camera(&self, aspect: f32) -> (Mat4, Mat4, f32) {
        let (yaw, pitch, dist) = self.orbit();
        let eye = Vec3::new(pitch.cos() * yaw.sin(), pitch.sin(), pitch.cos() * yaw.cos()) * dist;
        let view = Mat4::look_at_rh(eye, Vec3::ZERO, Vec3::Y);
        let fov = 38f32.to_radians();
        // a tight depth range around the form, so a 16-bit depth buffer is plenty
        let proj = Mat4::perspective_rh(fov, aspect, (dist - 3.2).max(0.2), dist + 3.2);
        // slide the picture in screen space (added to clip x/y in proportion to w), to leave room for the logo
        let (dx, dy) = self.shift();
        let shift = Mat4::from_cols(Vec4::X, Vec4::Y, Vec4::Z, Vec4::new(dx, dy, 0.0, 1.0));
        (view, shift * proj, dist)
    }
}
