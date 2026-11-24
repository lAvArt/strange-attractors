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

/// Where the orbit lives and how fast it moves.
#[derive(Clone)]
pub struct Calib {
    pub center: Vec3,
    /// frames the orbit: positions are divided by this, and nearly all of it then lies within about 1.1 of the centre
    pub radius: f64,
    /// integration step giving a normalised step length of `STEP_LEN`
    pub dt: f64,
    /// mean speed in units of the system's own time, to normalise colour
    pub mean_speed: f64,
    /// samples spread along the orbit, to seed trails on the attractor
    pub orbit: Vec<Vec3>,
}

pub const STEP_LEN: f64 = 0.012;

pub fn calibrate(s: &System) -> Calib {
    let mut p = s.start;
    for _ in 0..6000 {
        p = rk4(s.f, p, s.dt0);
    }
    let n = 40_000;
    let mut pts = Vec::with_capacity(n);
    let mut path = 0.0;
    let mut prev = p;
    for _ in 0..n {
        p = rk4(s.f, p, s.dt0);
        path += ((p[0] - prev[0]).powi(2) + (p[1] - prev[1]).powi(2) + (p[2] - prev[2]).powi(2)).sqrt();
        prev = p;
        pts.push(p);
    }
    // centre: halfway between the centroid (where the orbit spends its time) and the middle of its extent
    let mut c = [0.0; 3];
    for q in &pts {
        for i in 0..3 {
            c[i] += q[i] / n as f64;
        }
    }
    for i in 0..3 {
        let (lo, hi) = pts.iter().fold((f64::MAX, f64::MIN), |(lo, hi), q| (lo.min(q[i]), hi.max(q[i])));
        c[i] = 0.5 * c[i] + 0.25 * (lo + hi);
    }
    let mut d: Vec<f64> = pts.iter().map(|q| ((q[0] - c[0]).powi(2) + (q[1] - c[1]).powi(2) + (q[2] - c[2]).powi(2)).sqrt()).collect();
    d.sort_by(|a, b| a.partial_cmp(b).unwrap());
    // the whole orbit has to fit on the screen from every side, so the far reaches decide the size
    let radius = (d[(n as f64 * 0.97) as usize] / 1.12).max(d[(n as f64 * 0.999) as usize] / 1.35).max(d[n - 1] / 1.38).max(1e-6);
    let mean_speed = path / (n as f64 * s.dt0);
    let dt = (STEP_LEN * radius / mean_speed).clamp(s.dt0 * 0.25, s.dt0 * 3.0);
    let orbit = (0..512).map(|i| pts[i * (n / 512)]).collect();
    Calib { center: c, radius, dt, mean_speed, orbit }
}
