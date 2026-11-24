// Post-processing: bright-pass/downsample, separable blur, then composite onto the gradient ground + tonemap.
struct P {
    v: vec4<f32>,   // passes: xy texel step, z bright threshold. composite: x alpha-blend mode, y bloom, z vignette, w time
    bg0: vec4<f32>, // ground colour at the top
    bg1: vec4<f32>, // ground colour at the bottom
    ex: vec4<f32>,  // x: exposure
};
@group(0) @binding(0) var t0: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var<uniform> p: P;
@group(0) @binding(3) var t1: texture_2d<f32>;
@group(0) @binding(4) var t2: texture_2d<f32>;

struct FOut { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> };

@vertex
fn vs_full(@builtin(vertex_index) vi: u32) -> FOut {
    var q = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    var o: FOut;
    o.pos = vec4<f32>(q[vi], 0.0, 1.0);
    o.uv = vec2<f32>(q[vi].x * 0.5 + 0.5, 0.5 - q[vi].y * 0.5);
    return o;
}

fn bright(c: vec3<f32>, thr: f32) -> vec3<f32> {
    let m = max(max(c.r, c.g), c.b);
    return c * (max(m - thr, 0.0) / max(m, 1e-4));
}

@fragment
fn fs_down(i: FOut) -> @location(0) vec4<f32> {
    let s = p.v.xy;
    var c = textureSampleLevel(t0, samp, i.uv + s * vec2<f32>(-1.0, -1.0), 0.0).rgb;
    c += textureSampleLevel(t0, samp, i.uv + s * vec2<f32>(1.0, -1.0), 0.0).rgb;
    c += textureSampleLevel(t0, samp, i.uv + s * vec2<f32>(-1.0, 1.0), 0.0).rgb;
    c += textureSampleLevel(t0, samp, i.uv + s * vec2<f32>(1.0, 1.0), 0.0).rgb;
    c *= 0.25;
    return vec4<f32>(select(c, bright(c, p.v.z), p.v.z > 0.0), 1.0);
}

@fragment
fn fs_blur(i: FOut) -> @location(0) vec4<f32> {
    let d = p.v.xy;
    var c = textureSampleLevel(t0, samp, i.uv, 0.0).rgb * 0.2270270270;
    c += textureSampleLevel(t0, samp, i.uv + d * 1.3846153846, 0.0).rgb * 0.3162162162;
    c += textureSampleLevel(t0, samp, i.uv - d * 1.3846153846, 0.0).rgb * 0.3162162162;
    c += textureSampleLevel(t0, samp, i.uv + d * 3.2307692308, 0.0).rgb * 0.0702702703;
    c += textureSampleLevel(t0, samp, i.uv - d * 3.2307692308, 0.0).rgb * 0.0702702703;
    return vec4<f32>(c, 1.0);
}

fn hash12(q: vec2<f32>) -> f32 {
    var x = fract(q * vec2<f32>(123.34, 456.21));
    x += dot(x, x + 45.32);
    return fract(x.x * x.y);
}

@fragment
fn fs_comp(i: FOut) -> @location(0) vec4<f32> {
    let scene = textureSampleLevel(t0, samp, i.uv, 0.0);
    let r = i.uv * 2.0 - vec2<f32>(1.0);
    let vig = 1.0 - p.v.z * smoothstep(0.25, 1.7, dot(r, r));
    let overlay = p.ex.y > 0.5;
    // on its own ground: the gradient; as an overlay the desktop wallpaper shows through instead
    let bg = select(mix(p.bg0.rgb, p.bg1.rgb, i.uv.y) * vig, vec3<f32>(0.0), overlay);
    var c: vec3<f32>;
    var a = 1.0;
    if (p.v.x > 0.5) {
        c = scene.rgb + bg * (1.0 - scene.a);
        if (overlay) { a = scene.a; }
    } else {
        let bl = (textureSampleLevel(t1, samp, i.uv, 0.0).rgb * 0.26 + textureSampleLevel(t2, samp, i.uv, 0.0).rgb * 0.14) * p.v.y;
        let t = vec3<f32>(1.0) - exp(-(scene.rgb + bl) * p.ex.x);
        c = bg + t * (vec3<f32>(1.0) - bg);
        // premultiplied: the light's own brightness is its opacity
        if (overlay) { a = clamp(max(max(c.r, c.g), c.b), 0.0, 1.0); }
    }
    // dither by one 8-bit sRGB step (their size in linear light grows with brightness), per channel so it stays neutral
    let q = i.pos.xy + vec2<f32>(p.v.w * 61.0, 0.0);
    let nz = vec3<f32>(hash12(q), hash12(q + vec2<f32>(17.3, 5.1)), hash12(q + vec2<f32>(91.7, 33.9))) - vec3<f32>(0.5);
    let step = (1.0 / 255.0) * 2.3 * pow(max(c, vec3<f32>(0.0)) + vec3<f32>(0.003), vec3<f32>(0.545));
    c = max(c + nz * step * 1.3, vec3<f32>(0.0));
    if (overlay) { a = max(a, max(max(c.r, c.g), c.b)); }
    return vec4<f32>(c, a);
}
