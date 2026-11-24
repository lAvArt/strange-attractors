// Trails as screen-space ribbons. One instance per segment (age rank × trail); the vertex stage reads the ring of
// positions straight from a storage buffer, so the CPU only ever uploads the newest points.
struct U {
    vp: mat4x4<f32>,
    c0: vec4<f32>,
    c1: vec4<f32>,
    c2: vec4<f32>,
    ids: vec4<u32>, // n trails, ring length, head slot, mode (0 additive, 1 alpha)
    p1: vec4<f32>,  // width px, intensity, fade, age power
    res: vec4<f32>, // target size px, 1/size
    p2: vec4<f32>,  // x: reference depth (the camera distance)
};
@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var<storage, read> pts: array<vec4<f32>>;

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) side: f32,
    @location(1) col: vec3<f32>,
    @location(2) a: f32,
};

fn hash1(x: u32) -> f32 {
    var h = x * 747796405u + 2891336453u;
    h = ((h >> ((h >> 28u) + 4u)) ^ h) * 277803737u;
    h = (h >> 22u) ^ h;
    return f32(h) / 4294967295.0;
}

fn pal(m: f32) -> vec3<f32> {
    let a = smoothstep(0.0, 0.55, m);
    let b = smoothstep(0.45, 1.0, m);
    return mix(mix(u.c0.rgb, u.c1.rgb, a), u.c2.rgb, b);
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VOut {
    var cend = array<u32, 6>(0u, 0u, 1u, 1u, 0u, 1u);
    var cside = array<f32, 6>(-1.0, 1.0, -1.0, -1.0, 1.0, 1.0);
    let n = u.ids.x;
    let len = u.ids.y;
    let head = u.ids.z;
    // oldest segment first, so alpha blending stacks the newest on top
    let s = (len - 2u) - ii / n;
    let t = ii % n;
    let sa = (head + len - s) % len;
    let sb = (head + len - s - 1u) % len;
    let pa = pts[sa * n + t];
    let pb = pts[sb * n + t];
    let ca = u.vp * vec4<f32>(pa.xyz, 1.0);
    let cb = u.vp * vec4<f32>(pb.xyz, 1.0);
    var o: VOut;
    o.pos = vec4<f32>(2.0, 2.0, 2.0, 1.0);
    o.side = 0.0;
    o.col = vec3<f32>(0.0);
    o.a = 0.0;
    if (ca.w < 0.3 || cb.w < 0.3) { return o; }
    let na = ca.xy / ca.w;
    let nb = cb.xy / cb.w;
    let dpx = (nb - na) * u.res.xy * 0.5;
    let l = length(dpx);
    if (l < 0.01) { return o; }
    let dir = dpx / l;
    let nrm = vec2<f32>(-dir.y, dir.x);
    let e = cend[vi];
    let sd = cside[vi];
    let fl = f32(len - 1u);
    let age_e = (f32(s) + f32(e)) / fl;
    let w_end = select(ca.w, cb.w, e == 1u);
    let base = select(na, nb, e == 1u);
    let clipc = select(ca, cb, e == 1u);
    let wpx = u.p1.x * mix(1.0, 0.22, age_e) * u.p2.x / w_end;
    let ext = select(-dir, dir, e == 1u) * (wpx * 0.35);
    let off = (nrm * sd * wpx * 0.5 + ext) * 2.0 / u.res.xy;
    o.pos = vec4<f32>((base + off) * clipc.w, clipc.z, clipc.w);
    o.side = sd;

    let sp = clamp(mix(pa.w, pb.w, 0.5) * 0.5, 0.0, 1.0);
    let age_m = (f32(s) + 0.5) / fl;
    let glint = exp(-f32(s) * 0.045);
    var col = pal(clamp(sp * 0.85 + (1.0 - age_m) * 0.15, 0.0, 1.0));
    col = mix(col, u.c2.rgb, glint * 0.6);
    let tv = 0.72 + 0.28 * hash1(t);
    let dep = clamp(1.25 - 0.22 * (w_end - u.p2.x), 0.55, 1.3);
    o.col = col;
    o.a = pow(1.0 - age_m, u.p1.w) * tv * dep * u.p1.z * (1.0 + 1.6 * glint);
    return o;
}

@fragment
fn fs_main(i: VOut) -> @location(0) vec4<f32> {
    let g = exp(-i.side * i.side * 2.8);
    if (u.ids.w == 0u) {
        let a = i.a * g;
        return vec4<f32>(i.col * a * u.p1.y, a);
    }
    let a = clamp(i.a * g * u.p1.y, 0.0, 1.0);
    return vec4<f32>(i.col * a, a);
}
