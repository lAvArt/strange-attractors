//! A field of strands on one attractor, each remembering its last `len` positions in a ring.
//! The ring is slot-major (`slot * n + strand`) so a frame's new points are one contiguous upload.
//!
//! Strands come in ribbons of `m` neighbours that start a hair apart, in order, along a short curve. Left alone,
//! chaos would pull them apart exponentially. Instead every ribbon is treated as what it is, a material curve being
//! stretched along the attractor's unstable direction, and is held to a constant length the way a Lyapunov vector is
//! renormalised: each step its strands are slid a little along that curve (so they stay on the sheet) toward even
//! spacing around the middle, and slid along their own orbits so the ribbon's leading edge stays square to the flow.
//! Strand k and strand k+1 therefore stay neighbours for good, and together they sweep out a band of the folded
//! sheet: something a mesh, a contour field or a halftone can be drawn on.
//!
//! Where the flow tears a ribbon in two (the Lorenz saddle sends one half to each wing), the smaller half lets go,
//! thins out to nothing and grows again beside the half that stayed.

use crate::systems::{self, Calib, System, Vec3};

/// steps over which a strand thins away before it is moved, and thickens again afterwards
const RAMP: i16 = 36;

pub struct Trails {
    pub n: usize,
    pub len: usize,
    /// strands per ribbon
    pub m: usize,
    pub sys: System,
    pub cal: Calib,
    pos: Vec<Vec3>,
    /// each strand's place across its ribbon, in gaps: its index, nudged irregularly so that neighbouring lines are
    /// never exactly evenly spaced (even spacing beats against the pixel grid and against other layers)
    coef: Vec<f64>,
    /// each strand's own weight
    thick: Vec<f32>,
    /// 0 settled, >0 steps into growing, <0 steps into thinning away (detached from its ribbon)
    state: Vec<i16>,
    /// each ribbon's gap between neighbouring strands (in the attractor's own units), and the way it last lay
    pitch: Vec<f64>,
    dir: Vec<Vec3>,
    /// slot of the newest point
    pub head: usize,
    /// total steps taken: lets several renderers upload only what they have not yet seen
    pub steps: u64,
    /// a reference orbit and a companion a hair away, renormalised every step (Benettin's method): the running sum
    /// of ln(separation after a step / before) over the elapsed time is the largest Lyapunov exponent
    probe: (Vec3, Vec3),
    stretch: f64,
    stretch_time: f64,
    /// Places worth pointing at, in world coordinates, each with what it is: the system's equilibria, and the
    /// orbit's farthest reaches along the form's own axes. They stand still, so a box locked onto one shows the
    /// camera's parallax and nothing else.
    pub marks: Vec<(&'static str, [f32; 3])>,
    /// xyz in world coordinates; |w| is the strand's weight there (0..1), negative while detached from its ribbon
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

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn axpy(a: Vec3, b: Vec3, s: f64) -> Vec3 {
    [a[0] + b[0] * s, a[1] + b[1] * s, a[2] + b[2] * s]
}
fn dot(a: Vec3, b: Vec3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn norm(a: Vec3) -> f64 {
    dot(a, a).sqrt()
}
fn unit(a: Vec3) -> Vec3 {
    let l = norm(a).max(1e-300);
    [a[0] / l, a[1] / l, a[2] / l]
}

impl Trails {
    /// `n` strands in ribbons of `m`; `pitch` is the gap between neighbouring strands in world units (unit ball).
    /// `spread`: start with the history already drawn. Otherwise the strands grow out of their starting points.
    pub fn new(sys: System, n: usize, len: usize, m: usize, pitch: f64, seed: u64, spread: bool) -> Self {
        let cal = systems::calibrate(&sys);
        let m = m.clamp(1, n.max(1));
        let n = (n / m).max(1) * m;
        // the renderer thins points by ring slot (every 2nd, 3rd, 4th): a ring whose length they all divide has no seam
        let len = len.div_ceil(12).max(2) * 12;
        let groups = n / m;
        let mut rng = Rng(seed | 1);
        for _ in 0..8 {
            rng.next();
        }
        let mut pos = Vec::with_capacity(n);
        let mut thick = Vec::with_capacity(n);
        let mut coef = Vec::with_capacity(n);
        let mut pitches = Vec::with_capacity(groups);
        let mut dir = Vec::with_capacity(groups);
        let o0 = (rng.next() * cal.orbit.len() as f64) as usize;
        for g in 0..groups {
            let base = cal.orbit[(o0 + g * cal.orbit.len() / groups + (rng.next() * 3.0) as usize) % cal.orbit.len()];
            let p = pitch * cal.radius * (0.8 + 0.4 * rng.next());
            let d = unit([rng.next() - 0.5, rng.next() - 0.5, rng.next() - 0.5]);
            let tone = 0.85 + 0.15 * rng.next();
            for k in 0..m {
                let a = k as f64 + (rng.next() - 0.5) * 0.7;
                coef.push(a);
                pos.push(axpy(base, d, (a - (m as f64 - 1.0) * 0.5) * p));
                // every fifth strand is drawn heavier, like the index lines of a contour map
                let w = if m >= 10 && k % 5 == 2 { 1.0 } else if m >= 10 { 0.6 + 0.12 * rng.next() } else { 0.75 + 0.25 * rng.next() };
                thick.push((w * tone) as f32);
            }
            pitches.push(p);
            dir.push(d);
        }
        let cal_probe = cal.orbit[0];
        let probe_d = cal.radius * 1e-8;
        let mut t = Trails { n, len, m, sys, cal, pos, coef, thick, state: vec![0; n], pitch: pitches, dir, probe: (cal_probe, [cal_probe[0] + probe_d, cal_probe[1], cal_probe[2]]), marks: Vec::new(), stretch: 0.0, stretch_time: 0.0, head: 0, steps: 0, ring: vec![[0.0; 4]; n * len] };
        // let every ribbon turn into the direction the flow stretches before anything is drawn
        let mut seen: Vec<Vec3> = Vec::new();
        let skip = (n / 96).max(1);
        for i in 0..900 {
            t.advance();
            if i >= 300 && i % 6 == 0 {
                for k in (0..n).step_by(skip) {
                    let q = sub(t.pos[k], t.cal.center);
                    seen.push([dot(q, t.cal.basis[0]), dot(q, t.cal.basis[1]), dot(q, t.cal.basis[2])]);
                }
            }
        }
        // Ribbons have width, so where a system pulls its orbits onto the attractor only slowly (Chen-Lee) their
        // strands ride wider loops than the single orbit the calibration followed. Frame what is actually drawn.
        let wide = systems::fit(&seen);
        // (where they ride much wider, they are still drifting outward when this is measured)
        t.cal.radius = t.cal.radius.max(if wide > t.cal.radius * 1.3 { wide * 1.12 } else { wide });
        t.state.iter_mut().for_each(|s| *s = 0);
        // fill every slot with the starting points so unborn history is a zero-length (invisible) segment
        for slot in 0..len {
            for k in 0..n {
                let mut p = t.world(t.pos[k]);
                p[3] = 0.0;
                t.ring[slot * n + k] = p;
            }
        }
        // (after the last refit of the radius: marks are stored in world coordinates)
        let mut marks: Vec<(&'static str, [f32; 3])> = Vec::new();
        for e in systems::equilibria(&t.sys, &t.cal) {
            let w = t.world(e);
            marks.push(("FIXED POINT", [w[0], w[1], w[2]]));
        }
        for axis in 0..2 {
            let key = |p: &Vec3| dot(sub(*p, t.cal.center), t.cal.basis[axis]);
            let lo = t.cal.orbit.iter().copied().min_by(|a, b| key(a).partial_cmp(&key(b)).unwrap());
            let hi = t.cal.orbit.iter().copied().max_by(|a, b| key(a).partial_cmp(&key(b)).unwrap());
            for e in [lo, hi].into_iter().flatten() {
                let w = t.world(e);
                marks.push(("EXTENT", [w[0], w[1], w[2]]));
            }
        }
        t.marks = marks;
        if spread {
            for _ in 0..len {
                t.step();
            }
        }
        t
    }

    /// world coordinates: centred, scaled, and turned into the form's own frame (x across the screen, y up, z along
    /// the direction the form is thinnest in)
    fn world(&self, p: Vec3) -> [f32; 4] {
        let c = &self.cal;
        let q = sub(p, c.center);
        [(dot(q, c.basis[0]) / c.radius) as f32, (dot(q, c.basis[1]) / c.radius) as f32, (dot(q, c.basis[2]) / c.radius) as f32, 1.0]
    }

    /// one integration step for every strand, then hold each ribbon together
    fn advance(&mut self) {
        let dt = self.cal.dt;
        let f = self.sys.f;
        // The strands run on a rescaled clock: the field is slowed where it is fast and hurried where it is slow, which
        // leaves every orbit exactly the curve it was but makes the steps along it nearly equal in length. Without
        // this the fast outer loops of some systems would be drawn with a few long straight pieces.
        let v0 = self.cal.mean_speed;
        let paced = |x: Vec3| {
            let v = f(x);
            // (v0 / |v|) ^ 0.75, as two square roots
            let r = (v0 * v0 / dot(v, v).max(v0 * v0 * 1e-6)).sqrt().sqrt();
            let k = (r * r.sqrt()).min(6.0);
            [v[0] * k, v[1] * k, v[2] * k]
        };
        for k in 0..self.n {
            let q = systems::rk4(paced, self.pos[k], dt);
            // a wild orbit (numerical blow-up) rejoins its ribbon
            if q.iter().any(|x| !x.is_finite()) || norm(sub(q, self.cal.center)) > self.cal.radius * 40.0 {
                self.pos[k] = self.cal.orbit[k % self.cal.orbit.len()];
                self.state[k] = -RAMP;
            } else {
                self.pos[k] = q;
                // nudged off the attractor and leaving for good (the fit puts all of the orbit within 2 radii):
                // let go, thin out, and come back beside the others
                if self.state[k] >= 0 && norm(sub(q, self.cal.center)) > self.cal.radius * 2.4 {
                    self.state[k] = -1;
                }
            }
        }
        let m = self.m;
        {
            let d0 = self.cal.radius * 1e-8;
            let (r, p) = (systems::rk4(f, self.probe.0, dt), systems::rk4(f, self.probe.1, dt));
            let d = norm(sub(p, r));
            if d.is_finite() && d > 0.0 && r.iter().all(|x| x.is_finite()) {
                self.stretch += (d / d0).ln();
                self.stretch_time += dt;
                self.probe = (r, axpy(r, sub(p, r), d0 / d));
            } else {
                self.probe = (self.cal.orbit[0], axpy(self.cal.orbit[0], [1.0, 0.0, 0.0], d0));
            }
        }
        if m < 2 {
            for k in 0..self.n {
                let s = self.state[k];
                self.state[k] = if s <= -RAMP {
                    self.pos[k] = self.cal.orbit[(k * 7919 + self.steps as usize) % self.cal.orbit.len()];
                    1
                } else if s < 0 {
                    s - 1
                } else if s >= RAMP {
                    0
                } else if s > 0 {
                    s + 1
                } else {
                    0
                };
            }
            return;
        }
        let mut chain: Vec<usize> = Vec::with_capacity(m);
        let mut arc: Vec<f64> = Vec::with_capacity(m);
        let mut old: Vec<Vec3> = Vec::with_capacity(m);
        for g in 0..self.n / m {
            let r = g * m..(g + 1) * m;
            let p = self.pitch[g];
            chain.clear();
            chain.extend(r.clone().filter(|&k| self.state[k] >= 0));
            if chain.is_empty() {
                // nothing left to hold on to: once the last strand has thinned away, the whole ribbon starts again
                // somewhere on the orbit
                if r.clone().all(|k| self.state[k] <= -RAMP) {
                    let base = self.cal.orbit[(g * 37 + self.steps as usize) % self.cal.orbit.len()];
                    let mid = self.coef[r.start + m / 2];
                    for k in r {
                        self.pos[k] = axpy(base, self.dir[g], (self.coef[k] - mid) * p);
                        self.state[k] = 1;
                    }
                } else {
                    for k in r {
                        self.state[k] = (self.state[k] - 1).max(-RAMP);
                    }
                }
                continue;
            }
            if chain.len() >= 2 {
                // keep the leading edge square to the flow: slide each strand along its own orbit (which changes
                // nothing about the curve it draws) until it is level with the middle one
                let xc = self.pos[chain[chain.len() / 2]];
                for &k in &chain {
                    let v = f(self.pos[k]);
                    let sp = norm(v).max(1e-300);
                    let th = [v[0] / sp, v[1] / sp, v[2] / sp];
                    let lim = 0.25 * sp * dt;
                    let d = (0.2 * dot(sub(self.pos[k], xc), th)).clamp(-lim, lim);
                    self.pos[k] = axpy(self.pos[k], th, -d);
                }
                // a gap far wider than the rest is a tear: the smaller side lets go
                let span = self.coef[chain[chain.len() - 1]] - self.coef[chain[0]];
                let dc = |i: usize| self.coef[chain[i + 1]] - self.coef[chain[i]];
                let gap = |i: usize| norm(sub(self.pos[chain[i + 1]], self.pos[chain[i]])) / dc(i);
                let total: f64 = (0..chain.len() - 1).map(|i| gap(i) * dc(i)).sum();
                let (mut worst, mut wi) = (0.0, 0);
                for i in 0..chain.len() - 1 {
                    let x = gap(i);
                    if x > worst {
                        worst = x;
                        wi = i;
                    }
                }
                let rest = if chain.len() > 2 { (total - worst * dc(wi)) / (span - dc(wi)) } else { worst };
                if worst > 3.5 * p && worst > 3.0 * rest {
                    let keep_low = wi + 1 >= chain.len() - wi - 1;
                    let (lo, hi) = if keep_low { (wi + 1, chain.len()) } else { (0, wi + 1) };
                    for &k in &chain[lo..hi] {
                        self.state[k] = -1;
                    }
                    if keep_low { chain.truncate(wi + 1) } else { chain.drain(0..wi + 1); }
                }
            }
            let q = chain.len();
            let (mut e0, mut e1) = ([-self.dir[g][0], -self.dir[g][1], -self.dir[g][2]], self.dir[g]);
            if q >= 2 {
                // slide the strands along the curve they form: toward the target length (this is the renormalisation
                // that stands in for the stretching) and toward even spacing
                arc.clear();
                old.clear();
                arc.push(0.0);
                old.push(self.pos[chain[0]]);
                for i in 1..q {
                    arc.push(arc[i - 1] + norm(sub(self.pos[chain[i]], self.pos[chain[i - 1]])));
                    old.push(self.pos[chain[i]]);
                }
                let total = arc[q - 1].max(1e-300);
                let span = self.coef[chain[q - 1]] - self.coef[chain[0]];
                let s = (p * span / total).clamp(0.955, 1.035);
                let mut seg = 0;
                for i in 0..q {
                    let even = ((self.coef[chain[i]] - self.coef[chain[0]]) / span - 0.5) * total;
                    let cur = arc[i] - total * 0.5;
                    let want = (cur + 0.10 * (even - cur)) * s + total * 0.5;
                    self.pos[chain[i]] = if want <= 0.0 {
                        axpy(old[0], unit(sub(old[0], old[1])), -want)
                    } else if want >= total {
                        axpy(old[q - 1], unit(sub(old[q - 1], old[q - 2])), want - total)
                    } else {
                        while seg + 2 < q && arc[seg + 1] < want {
                            seg += 1;
                        }
                        while seg > 0 && arc[seg] > want {
                            seg -= 1;
                        }
                        let u = (want - arc[seg]) / (arc[seg + 1] - arc[seg]).max(1e-300);
                        axpy(old[seg], sub(old[seg + 1], old[seg]), u)
                    };
                }
                e0 = unit(sub(self.pos[chain[0]], self.pos[chain[1]]));
                e1 = unit(sub(self.pos[chain[q - 1]], self.pos[chain[q - 2]]));
                self.dir[g] = e1;
            }
            let (first, last) = (chain[0], chain[q - 1]);
            let (xf, xl) = (self.pos[first], self.pos[last]);
            for k in r {
                let st = self.state[k];
                if st > 0 {
                    self.state[k] = if st >= RAMP { 0 } else { st + 1 };
                } else if st < 0 {
                    if st <= -RAMP {
                        // grow again beside the strands that stayed, in order
                        self.pos[k] = if k < first {
                            axpy(xf, e0, p * (self.coef[first] - self.coef[k]))
                        } else if k > last {
                            axpy(xl, e1, p * (self.coef[k] - self.coef[last]))
                        } else {
                            let i = chain.iter().position(|&c| c > k).unwrap_or(q - 1).max(1);
                            let (a, b) = (chain[i - 1], chain[i]);
                            axpy(self.pos[a], sub(self.pos[b], self.pos[a]), (self.coef[k] - self.coef[a]) / (self.coef[b] - self.coef[a]).max(1e-9))
                        };
                        self.state[k] = 1;
                    } else {
                        self.state[k] = st - 1;
                    }
                }
            }
        }
    }

    /// the running estimate of the largest Lyapunov exponent, per unit of the system's own time
    pub fn lyapunov(&self) -> f64 {
        if self.stretch_time > 0.0 { self.stretch / self.stretch_time } else { 0.0 }
    }

    /// simulated time, in the system's own units
    pub fn time(&self) -> f64 {
        self.steps as f64 * self.cal.dt
    }

    /// a point of the picture (world coordinates) in the system's own coordinates
    pub fn system_of(&self, w: [f32; 3]) -> Vec3 {
        let c = &self.cal;
        let mut p = c.center;
        for i in 0..3 {
            p = axpy(p, c.basis[i], w[i] as f64 * c.radius);
        }
        p
    }

    /// how fast the flow runs at a point (the system's own coordinates and time)
    pub fn speed_at(&self, p: Vec3) -> f64 {
        norm((self.sys.f)(p))
    }

    /// one strand to report on: where it is (the system's own coordinates) and how fast it moves
    pub fn lead(&self) -> (Vec3, f64) {
        let p = self.pos[self.m / 2];
        (p, norm((self.sys.f)(p)))
    }

    pub fn step(&mut self) {
        self.head = (self.head + 1) % self.len;
        self.advance();
        for k in 0..self.n {
            let st = self.state[k] as f32;
            let ramp = RAMP as f32 - 2.0;
            // thinning: reaches zero a step before the strand is moved; growing: starts from zero where it lands
            let f = if st < 0.0 { 1.0 + st / ramp } else if st > 0.0 { (st - 1.0) / ramp } else { 1.0 };
            let f = f.clamp(0.0, 1.0);
            let mut p = self.world(self.pos[k]);
            p[3] = self.thick[k] * f * f * (3.0 - 2.0 * f) * if st < 0.0 { -1.0 } else { 1.0 };
            self.ring[self.head * self.n + k] = p;
        }
        self.steps += 1;
    }
}
