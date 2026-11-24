//! The attractors: their equations, and an automatic fit of each one to the unit ball.
//! Every system is integrated in f64 with RK4; `calibrate` measures where the orbit lives and how fast it
//! moves so the renderer never needs hand-tuned scales.

pub type Vec3 = [f64; 3];

#[derive(Clone, Copy)]
pub struct System {
    pub name: &'static str,
    pub f: fn(Vec3) -> Vec3,
    pub start: Vec3,
    /// a step size that is safely stable for RK4
    pub dt0: f64,
    /// the right-hand sides of dx/dt, dy/dt, dz/dt as they are set in print, and the parameter values used above
    pub eq: [&'static str; 3],
    pub params: &'static str,
}

fn lorenz(p: Vec3) -> Vec3 {
    let (s, r, b) = (10.0, 28.0, 8.0 / 3.0);
    [s * (p[1] - p[0]), p[0] * (r - p[2]) - p[1], p[0] * p[1] - b * p[2]]
}

fn aizawa(p: Vec3) -> Vec3 {
    let (a, b, c, d, e, f) = (0.95, 0.7, 0.6, 3.5, 0.25, 0.1);
    let [x, y, z] = p;
    [(z - b) * x - d * y, d * x + (z - b) * y, c + a * z - z * z * z / 3.0 - (x * x + y * y) * (1.0 + e * z) + f * z * x * x * x]
}

fn halvorsen(p: Vec3) -> Vec3 {
    let a = 1.27;
    let [x, y, z] = p;
    [-a * x - 4.0 * y - 4.0 * z - y * y, -a * y - 4.0 * z - 4.0 * x - z * z, -a * z - 4.0 * x - 4.0 * y - x * x]
}

fn thomas(p: Vec3) -> Vec3 {
    let b = 0.19;
    [p[1].sin() - b * p[0], p[2].sin() - b * p[1], p[0].sin() - b * p[2]]
}

fn rossler(p: Vec3) -> Vec3 {
    let (a, b, c) = (0.2, 0.2, 5.7);
    [-p[1] - p[2], p[0] + a * p[1], b + p[2] * (p[0] - c)]
}

fn chen_lee(p: Vec3) -> Vec3 {
    let (a, b, c) = (5.0, -10.0, -0.38);
    [a * p[0] - p[1] * p[2], b * p[1] + p[0] * p[2], c * p[2] + p[0] * p[1] / 3.0]
}

fn four_wing(p: Vec3) -> Vec3 {
    let (a, b, c) = (0.2, 0.01, -0.4);
    [a * p[0] + p[1] * p[2], b * p[0] + c * p[1] - p[0] * p[2], -p[2] - p[0] * p[1]]
}

fn dadras(p: Vec3) -> Vec3 {
    let (a, b, c, d, e) = (3.0, 2.7, 1.7, 2.0, 9.0);
    [p[1] - a * p[0] + b * p[1] * p[2], c * p[1] - p[0] * p[2] + p[2], d * p[0] * p[1] - e * p[2]]
}

pub const ALL: &[System] = &[
    System { name: "lorenz", f: lorenz, start: [0.1, 0.0, 0.0], dt0: 0.005,
        eq: ["σ(y − x)", "x(ρ − z) − y", "xy − βz"],
        params: "σ = 10   ρ = 28   β = 8/3",
    },
    System { name: "aizawa", f: aizawa, start: [0.1, 0.0, 0.0], dt0: 0.01,
        eq: ["(z − β)x − δy", "δx + (z − β)y", "γ + αz − z³/3 − (x² + y²)(1 + εz) + ζzx³"],
        params: "α = 0.95  β = 0.7  γ = 0.6  δ = 3.5  ε = 0.25  ζ = 0.1",
    },
    System { name: "halvorsen", f: halvorsen, start: [-1.48, -1.51, 2.04], dt0: 0.004,
        eq: ["−ax − 4y − 4z − y²", "−ay − 4z − 4x − z²", "−az − 4x − 4y − x²"],
        params: "a = 1.27",
    },
    System { name: "thomas", f: thomas, start: [1.1, 1.1, -0.01], dt0: 0.05,
        eq: ["sin y − bx", "sin z − by", "sin x − bz"],
        params: "b = 0.19",
    },
    System { name: "rossler", f: rossler, start: [0.1, 0.0, 0.0], dt0: 0.02,
        eq: ["−y − z", "x + ay", "b + z(x − c)"],
        params: "a = 0.2   b = 0.2   c = 5.7",
    },
    System { name: "chen-lee", f: chen_lee, start: [5.0, 10.0, 10.0], dt0: 0.003,
        eq: ["αx − yz", "βy + xz", "δz + xy/3"],
        params: "α = 5   β = −10   δ = −0.38",
    },
    System { name: "four-wing", f: four_wing, start: [1.0, 1.0, 1.0], dt0: 0.05,
        eq: ["ax + yz", "bx + cy − xz", "−z − xy"],
        params: "a = 0.2   b = 0.01   c = −0.4",
    },
    System { name: "dadras", f: dadras, start: [1.1, 2.1, -2.0], dt0: 0.005,
        eq: ["y − ax + byz", "cy − xz + z", "dxy − ez"],
        params: "a = 3   b = 2.7   c = 1.7   d = 2   e = 9",
    },
];

pub fn find(name: &str) -> Option<usize> {
    ALL.iter().position(|s| s.name == name)
}

pub fn rk4(f: impl Fn(Vec3) -> Vec3, p: Vec3, dt: f64) -> Vec3 {
    let add = |a: Vec3, b: Vec3, s: f64| [a[0] + b[0] * s, a[1] + b[1] * s, a[2] + b[2] * s];
    let k1 = f(p);
    let k2 = f(add(p, k1, dt * 0.5));
    let k3 = f(add(p, k2, dt * 0.5));
    let k4 = f(add(p, k3, dt));
    [
        p[0] + dt / 6.0 * (k1[0] + 2.0 * k2[0] + 2.0 * k3[0] + k4[0]),
        p[1] + dt / 6.0 * (k1[1] + 2.0 * k2[1] + 2.0 * k3[1] + k4[1]),
        p[2] + dt / 6.0 * (k1[2] + 2.0 * k2[2] + 2.0 * k3[2] + k4[2]),
    ]
}

/// Where the orbit lives, how it lies, and how fast it moves.
#[derive(Clone)]
pub struct Calib {
    pub center: Vec3,
    /// frames the orbit: positions are divided by this, and seen face-on the form is then about 2 units tall
    pub radius: f64,
    /// the form's own frame: screen-right, screen-up, and the direction it is thinnest in (toward the viewer).
    /// Looking along the third axis shows the form open; looking across it would show it edge-on.
    pub basis: [Vec3; 3],
    /// integration step giving a normalised step length of `STEP_LEN`
    pub dt: f64,
    /// mean speed in units of the system's own time
    pub mean_speed: f64,
    /// samples spread along the orbit, to seed trails on the attractor
    pub orbit: Vec<Vec3>,
}

pub const STEP_LEN: f64 = 0.012;

fn dot3(a: Vec3, b: Vec3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross3(a: Vec3, b: Vec3) -> Vec3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn unit3(a: Vec3) -> Vec3 {
    let l = dot3(a, a).sqrt().max(1e-300);
    [a[0] / l, a[1] / l, a[2] / l]
}

/// eigenvectors of a symmetric 3x3 matrix (as columns of the result) and its eigenvalues, by Jacobi rotations
fn eigen(mut a: [[f64; 3]; 3]) -> ([Vec3; 3], Vec3) {
    let mut v = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    for _ in 0..24 {
        for (p, q) in [(0, 1), (0, 2), (1, 2)] {
            if a[p][q].abs() < 1e-300 {
                continue;
            }
            let th = 0.5 * (2.0 * a[p][q]).atan2(a[q][q] - a[p][p]);
            let (c, sn) = (th.cos(), th.sin());
            for k in 0..3 {
                let (x, y) = (a[k][p], a[k][q]);
                a[k][p] = c * x - sn * y;
                a[k][q] = sn * x + c * y;
            }
            for k in 0..3 {
                let (x, y) = (a[p][k], a[q][k]);
                a[p][k] = c * x - sn * y;
                a[q][k] = sn * x + c * y;
            }
            for k in 0..3 {
                let (x, y) = (v[k][p], v[k][q]);
                v[k][p] = c * x - sn * y;
                v[k][q] = sn * x + c * y;
            }
        }
    }
    ([[v[0][0], v[1][0], v[2][0]], [v[0][1], v[1][1], v[2][1]], [v[0][2], v[1][2], v[2][2]]], [a[0][0], a[1][1], a[2][2]])
}

/// The scale at which a cloud of points (in the form's own frame, centred) fills the picture without leaving it:
/// the camera's own path is tried (it leans up to 45 degrees off the face-on axis, in every direction) and the form,
/// at its most awkward, must still stand within 80% of the screen's height and 68% of its width (which also keeps
/// the margins free for the annotations).
pub fn fit(local: &[Vec3]) -> f64 {
    let stride = (local.len() / 8000).max(1);
    let mut buf: Vec<f64> = Vec::with_capacity(local.len() / stride + 1);
    let mut worst = |r: f64| {
        let (dist, p11, aspect) = (4.1, 1.0 / (19f64.to_radians()).tan(), 1.6);
        let mut most: f64 = 0.0;
        for lean in [0.30f64, 0.54, 0.78] {
            for k in 0..8 {
                let a = k as f64 * std::f64::consts::FRAC_PI_4;
                let e = [lean.sin() * a.cos(), lean.sin() * a.sin(), lean.cos()];
                let rt = unit3(cross3([-e[0], -e[1], -e[2]], [0.0, 1.0, 0.0]));
                let u = cross3(rt, [-e[0], -e[1], -e[2]]);
                buf.clear();
                buf.extend(local.iter().step_by(stride).map(|q| {
                    let w = [q[0] / r, q[1] / r, q[2] / r];
                    let depth = (dist - dot3(w, e)).max(0.3);
                    (dot3(w, u).abs() * p11 / depth / 0.80).max(dot3(w, rt).abs() * p11 / aspect / depth / 0.68)
                }));
                // the body of the form (97.5% of it) inside the frame; its rare far reaches (a Rössler spike) may run
                // a little past the screen's edge at the worst moment rather than shrink everything else
                let k = ((buf.len() as f64 * 0.975) as usize).min(buf.len() - 1);
                let (_, body, rest) = buf.select_nth_unstable_by(k, |a, b| a.partial_cmp(b).unwrap());
                let reach = rest.iter().fold(*body, |m, x| m.max(*x));
                most = most.max(*body).max(reach / 1.45);
            }
        }
        most
    };
    let far = local.iter().map(|q| dot3(*q, *q).sqrt()).fold(0.0, f64::max).max(1e-9);
    let (mut lo, mut hi) = (far * 0.2, far * 4.0);
    for _ in 0..22 {
        let mid = 0.5 * (lo + hi);
        if worst(mid) > 1.0 { lo = mid } else { hi = mid }
    }
    hi
}

pub fn calibrate(s: &System) -> Calib {
    let mut p = s.start;
    for _ in 0..6000 {
        p = rk4(s.f, p, s.dt0);
    }
    let n = 200_000;
    let mut pts = Vec::with_capacity(n);
    let mut path = 0.0;
    let mut prev = p;
    for _ in 0..n {
        p = rk4(s.f, p, s.dt0);
        path += ((p[0] - prev[0]).powi(2) + (p[1] - prev[1]).powi(2) + (p[2] - prev[2]).powi(2)).sqrt();
        prev = p;
        pts.push(p);
    }
    let mut mean = [0.0; 3];
    for q in &pts {
        for i in 0..3 {
            mean[i] += q[i] / n as f64;
        }
    }
    // how the form lies: the direction in which the orbit varies least is the one to look along
    let mut cov = [[0.0; 3]; 3];
    for q in &pts {
        for i in 0..3 {
            for j in 0..3 {
                cov[i][j] += (q[i] - mean[i]) * (q[j] - mean[j]) / n as f64;
            }
        }
    }
    let (ev, lam) = eigen(cov);
    let mut order = [0, 1, 2];
    order.sort_by(|&a, &b| lam[b].partial_cmp(&lam[a]).unwrap());
    let normal = unit3(ev[order[2]]);
    // up: the system's own z axis where that makes sense (the Lorenz butterfly stands upright), else the form's
    // second-longest direction, so the longest lies across the screen
    let z = [0.0, 0.0, 1.0];
    let zn = dot3(z, normal);
    let mut up = if zn.abs() < 0.75 { unit3([z[0] - normal[0] * zn, z[1] - normal[1] * zn, z[2] - normal[2] * zn]) } else { unit3(ev[order[1]]) };
    let mut right = unit3(cross3(up, normal));
    // a form taller than it is wide is laid on its side: the screen is a landscape
    let ext = |axis: Vec3| {
        let mut d: Vec<f64> = pts.iter().map(|q| dot3([q[0] - mean[0], q[1] - mean[1], q[2] - mean[2]], axis)).collect();
        d.sort_by(|a, b| a.partial_cmp(b).unwrap());
        (d[(n as f64 * 0.001) as usize], d[(n as f64 * 0.999) as usize])
    };
    let (mut eh, mut evv) = (ext(right), ext(up));
    if (evv.1 - evv.0) > 1.25 * (eh.1 - eh.0) && zn.abs() >= 0.75 {
        std::mem::swap(&mut up, &mut right);
        right = unit3(cross3(up, normal));
        eh = ext(right);
        evv = ext(up);
    }
    // centre: the middle of the picture across and up, the centroid in depth
    let (mh, mv) = (0.5 * (eh.0 + eh.1), 0.5 * (evv.0 + evv.1));
    let c = [mean[0] + right[0] * mh + up[0] * mv, mean[1] + right[1] * mh + up[1] * mv, mean[2] + right[2] * mh + up[2] * mv];
    // size: see `fit`
    let local: Vec<Vec3> = pts.iter().step_by(16).map(|q| { let w = [q[0] - c[0], q[1] - c[1], q[2] - c[2]]; [dot3(w, right), dot3(w, up), dot3(w, normal)] }).collect();
    let radius = fit(&local);
    let mean_speed = path / (n as f64 * s.dt0);
    let dt = (STEP_LEN * radius / mean_speed).clamp(s.dt0 * 0.25, s.dt0 * 3.0);
    let orbit = (0..512).map(|i| pts[i * (n / 512)]).collect();
    Calib { center: c, radius, basis: [right, up, normal], dt, mean_speed, orbit }
}
