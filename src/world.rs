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
        let trails = Trails::new(systems::ALL[order[0]], cfg.trails, cfg.length, seed, true);
        World { cfg: cfg.clone(), from_look: look.clone(), target_look: look.clone(), blend: 1.0, theme_stamp: crate::config::theme_stamp(), theme_poll: 0.0, look, trails, logo: logo::load(&cfg.logo), t: 0.0, fade: 1.0, epoch: 0, order, idx: 0, since: 0.0, phase: Phase::Run, acc: 0.0, seed, cycling }
    }

    /// With the `theme` look, notice when the system theme changes (checked once a second, a single stat call) and
    /// crossfade to the new colours over a moment. Call it on every tick, even while nothing is drawn.
    pub fn poll_theme(&mut self, dt: f32) {
        if self.cfg.preset != "theme" {
            return;
        }
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
                    self.trails = Trails::new(systems::ALL[self.order[self.idx]], self.cfg.trails, self.cfg.length, self.seed, false);
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

    /// view-projection of the slowly orbiting camera
    pub fn camera(&self, aspect: f32) -> (Mat4, f32) {
        let t = self.t;
        let yaw = t * 0.055;
        let pitch = 0.30 + 0.20 * (t * 0.037).sin();
        let dist = 4.1 / self.cfg.zoom.max(0.2) * (1.0 + 0.035 * (t * 0.021).sin());
        let eye = Vec3::new(pitch.cos() * yaw.sin(), pitch.sin(), pitch.cos() * yaw.cos()) * dist;
        let view = Mat4::look_at_rh(eye, Vec3::ZERO, Vec3::Y);
        let fov = 38f32.to_radians();
        let proj = Mat4::perspective_rh(fov, aspect, 0.2, 40.0);
        // slide the picture in screen space (added to clip x/y in proportion to w), to leave room for the logo
        let dx = self.cfg.shift_x.unwrap_or(if self.logo.is_some() { -0.3 } else { 0.0 });
        let shift = Mat4::from_cols(Vec4::X, Vec4::Y, Vec4::Z, Vec4::new(dx, self.cfg.shift_y, 0.0, 1.0));
        (shift * proj * view, dist)
    }
}
