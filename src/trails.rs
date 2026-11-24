//! A field of trails: many points on one attractor, each remembering its last `len` positions in a ring.
//! The ring is slot-major (`slot * n + trail`) so a frame's new points are one contiguous upload.

use crate::systems::{self, Calib, System, Vec3};

pub struct Trails {
    pub n: usize,
    pub len: usize,
    pub sys: System,
    pub cal: Calib,
    pos: Vec<Vec3>,
    /// slot of the newest point
    pub head: usize,
    /// total steps taken: lets several renderers upload only what they have not yet seen
    pub steps: u64,
    pub ring: Vec<[f32; 4]>,
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
}

impl Trails {
    /// `spread`: start the trails all over the attractor, with history already drawn.
    /// Otherwise they start as one tiny cloud and fan out as the chaos stretches it.
    pub fn new(sys: System, n: usize, len: usize, seed: u64, spread: bool) -> Self {
        let cal = systems::calibrate(&sys);
        let mut rng = Rng(seed | 1);
        for _ in 0..8 {
            rng.next();
        }
        let origin = cal.orbit[(rng.next() * cal.orbit.len() as f64) as usize % cal.orbit.len()];
        let pos = (0..n)
            .map(|i| {
                let base = if spread { cal.orbit[(i * 7919 + (rng.next() * 64.0) as usize) % cal.orbit.len()] } else { origin };
                let j = cal.radius * if spread { 2e-3 } else { 4e-4 };
                [base[0] + (rng.next() - 0.5) * j, base[1] + (rng.next() - 0.5) * j, base[2] + (rng.next() - 0.5) * j]
            })
            .collect();
        let mut t = Trails { n, len, sys, cal, pos, head: 0, steps: 0, ring: vec![[0.0; 4]; n * len] };
        // fill every slot with the starting points so unborn history is a zero-length (invisible) segment
        for slot in 0..len {
            for k in 0..n {
                t.ring[slot * n + k] = t.norm(t.pos[k], 0.0);
            }
        }
        if spread {
            for _ in 0..len {
                t.step();
            }
        }
        t
    }

    /// world coordinates: centred, unit radius, the attractor's z axis pointing up
    fn norm(&self, p: Vec3, speed: f64) -> [f32; 4] {
        let c = &self.cal;
        let r = c.radius;
        [((p[0] - c.center[0]) / r) as f32, ((p[2] - c.center[2]) / r) as f32, (-(p[1] - c.center[1]) / r) as f32, (speed / c.mean_speed) as f32]
    }

    pub fn step(&mut self) {
        self.head = (self.head + 1) % self.len;
        let dt = self.cal.dt;
        for k in 0..self.n {
            let p = self.pos[k];
            let q = systems::rk4(self.sys.f, p, dt);
            let v = ((q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2) + (q[2] - p[2]).powi(2)).sqrt() / dt;
            // a wild orbit (numerical blow-up) is pulled back onto the attractor
            let q = if q.iter().any(|x| !x.is_finite()) || (q[0] - self.cal.center[0]).abs() > self.cal.radius * 40.0 { self.cal.orbit[k % self.cal.orbit.len()] } else { q };
            self.pos[k] = q;
            self.ring[self.head * self.n + k] = self.norm(q, v);
        }
        self.steps += 1;
    }
}
