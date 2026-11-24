// The ground (gradient, vignette, logo, dither), the annotation layer (glyph quads from an atlas), and the upscale
// used only when the scene is rendered below the output size.
struct P {
    v: vec4<f32>,   // x: overlay (transparent ground), y: output width px, z: vignette, w: time
    bg0: vec4<f32>, // ground colour at the top; w: 1 = the logo keeps its SVG colours
    bg1: vec4<f32>, // ground colour at the bottom
    ex: vec4<f32>,  // x: ground (0 gradient, 1 sunset over a grid, 2 copper bars), y: canvas (how much the ground is paper), z: logo style (0 none, 1 outline, 2 fill, 3 both), w: output height px
    lg: vec4<f32>,  // logo: centre x, centre y (screen fractions), height (fraction of screen height), aspect w/h
    lt: vec4<f32>,  // logo: tint rgb, opacity
    g0: vec4<f32>,  // the stylised grounds' two extra colours (sun and grid; the copper bars)
    g1: vec4<f32>,
    q: vec4<f32>,   // x: colour levels per channel to dither down to (0: off), y: whole-number upscale (0: filtered), z: scanlines, w: grain
};
@group(0) @binding(0) var t0: texture_2d<f32>; // the rasterised logo, premultiplied (ground) / the scene (blit) / the glyph atlas (text)
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var<uniform> p: P;

struct FOut { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> };

@vertex
fn vs_full(@builtin(vertex_index) vi: u32) -> FOut {
    var q = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    var o: FOut;
    o.pos = vec4<f32>(q[vi], 0.0, 1.0);
    o.uv = vec2<f32>(q[vi].x * 0.5 + 0.5, 0.5 - q[vi].y * 0.5);
    return o;
}


fn pnoise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let s = f * f * (3.0 - 2.0 * f);
    let a = fract(sin(dot(i, vec2<f32>(127.1, 311.7))) * 43758.5453);
    let b = fract(sin(dot(i + vec2<f32>(1.0, 0.0), vec2<f32>(127.1, 311.7))) * 43758.5453);
    let c = fract(sin(dot(i + vec2<f32>(0.0, 1.0), vec2<f32>(127.1, 311.7))) * 43758.5453);
    let d = fract(sin(dot(i + vec2<f32>(1.0, 1.0), vec2<f32>(127.1, 311.7))) * 43758.5453);
    return mix(mix(a, b, s.x), mix(c, d, s.x), s.y);
}

// the paper's tooth at a pixel, 0 (a hollow) .. 1 (a peak): long fibres lying across, and a finer felt. It is a
// function of the place on the screen alone, so like real paper it never moves; marks slide over it.
fn paper(xy: vec2<f32>, k: f32) -> f32 {
    let q = xy / k;
    return 0.6 * pnoise(q * vec2<f32>(0.17, 0.4)) + 0.4 * pnoise(q * 0.55 + vec2<f32>(31.0, 17.0));
}

fn hash12(q: vec2<f32>) -> f32 {
    var x = fract(q * vec2<f32>(123.34, 456.21));
    x += dot(x, x + 45.32);
    return fract(x.x * x.y);
}

// the logo mask at a screen position (premultiplied), zero outside the logo's box
fn logo_sample(uv: vec2<f32>, sasp: f32) -> vec4<f32> {
    let x = (uv.x - p.lg.x) * sasp;
    let y = uv.y - p.lg.y;
    let lu = x / (p.lg.z * p.lg.w) + 0.5;
    let lv = y / p.lg.z + 0.5;
    if (lu < 0.0 || lu > 1.0 || lv < 0.0 || lv > 1.0) { return vec4<f32>(0.0); }
    return textureSampleLevel(t0, samp, vec2<f32>(lu, lv), 0.0);
}

// the logo as premultiplied colour + alpha: a hairline outline, a faint fill, or both
fn logo_layer(uv: vec2<f32>) -> vec4<f32> {
    let style = p.ex.z;
    if (style < 0.5) { return vec4<f32>(0.0); }
    let sasp = p.v.y / p.ex.w;
    let m0 = logo_sample(uv, sasp);
    let a0 = m0.a;
    // edges: the gradient of the mask over about a pixel and a half of the output
    let px = vec2<f32>(1.4 / (p.ex.w * sasp), 0.0);
    let py = vec2<f32>(0.0, 1.4 / p.ex.w);
    let gx = logo_sample(uv + px, sasp).a - logo_sample(uv - px, sasp).a;
    let gy = logo_sample(uv + py, sasp).a - logo_sample(uv - py, sasp).a;
    let edge = clamp(length(vec2<f32>(gx, gy)), 0.0, 1.0);
    var a = 0.0;
    if (style < 1.5) { a = edge * 1.6; }
    else if (style < 2.5) { a = a0 * 0.9; }
    else { a = a0 * 0.045 + edge * 1.5; }
    // the 2.4 s heartbeat of the Twinise desktop scene, barely there
    let pulse = 0.93 + 0.07 * sin(p.v.w * 2.6179939);
    a = clamp(a * p.lt.w * pulse, 0.0, 1.0);
    var col = p.lt.rgb;
    if (p.bg0.w > 0.5 && a0 > 0.001) { col = m0.rgb / a0; }
    return vec4<f32>(col * a, a);
}

// 1983: a banded sun going down behind a grid that runs to the horizon
fn sunset(uv: vec2<f32>) -> vec3<f32> {
    let asp = p.v.y / p.ex.w;
    let hz = 0.66;
    var c = mix(p.bg0.rgb, p.bg1.rgb, smoothstep(0.0, hz, uv.y));
    let px = 1.0 / p.ex.w;
    if (uv.y < hz) {
        let d = length(vec2<f32>((uv.x - 0.5) * asp, uv.y - (hz - 0.07)));
        let disc = 1.0 - smoothstep(0.27 - px, 0.27 + px, d);
        // the lower half is cut by bars that widen toward the horizon
        let t = clamp((uv.y - (hz - 0.34)) / 0.34, 0.0, 1.0);
        let bar = step(0.5, t) * step(fract(t * t * 22.0), 0.18 + 0.5 * (t - 0.5));
        c = mix(c, mix(p.g0.rgb, p.g1.rgb, t), disc * (1.0 - bar) * 0.7);
        c += p.g1.rgb * 0.10 * smoothstep(hz - 0.2, hz, uv.y);
    } else {
        let z = 1.0 / max(uv.y - hz, 1e-4);
        let gx = (uv.x - 0.5) * asp * z * 0.9;
        let gz = z * 0.5 + p.v.w * 0.22;
        let wx = fwidth(gx) * 1.2;
        let wz = fwidth(gz) * 1.2;
        let lx = 1.0 - smoothstep(0.0, wx, abs(fract(gx + 0.5) - 0.5));
        let lz = 1.0 - smoothstep(0.0, wz, abs(fract(gz + 0.5) - 0.5));
        let far = smoothstep(0.0, 0.07, uv.y - hz);
        c = p.bg1.rgb * 0.5 + p.g1.rgb * max(lx, lz) * far * 0.6;
    }
    return c;
}

// 1993: a stepped gradient with raster bars drifting through it
fn copper(uv: vec2<f32>) -> vec3<f32> {
    var c = mix(p.bg0.rgb, p.bg1.rgb, floor(uv.y * 24.0) / 23.0);
    for (var k = 0; k < 3; k++) {
        let yc = 0.5 + 0.36 * sin(p.v.w * (0.21 + 0.07 * f32(k)) + f32(k) * 2.1);
        let d = abs(uv.y - yc) / 0.035;
        let b = max(1.0 - d, 0.0);
        c = mix(c, mix(p.g0.rgb, p.g1.rgb, f32(k) * 0.5) * (0.25 + 0.75 * b), step(d, 1.0) * 0.32);
    }
    return c;
}

fn scan(y: f32) -> f32 {
    return 1.0 - p.q.z * (0.5 - 0.5 * cos(y * 1.5707963));
}

@fragment
fn fs_ground(i: FOut) -> @location(0) vec4<f32> {
    let r = i.uv * 2.0 - vec2<f32>(1.0);
    let vig = 1.0 - p.v.z * smoothstep(0.25, 1.7, dot(r, r));
    let overlay = p.v.x > 0.5;
    var base = mix(p.bg0.rgb, p.bg1.rgb, i.uv.y);
    if (p.ex.x > 1.5) { base = copper(i.uv); } else if (p.ex.x > 0.5) { base = sunset(i.uv); }
    base *= scan(i.pos.y);
    // on its own ground: the gradient; as an overlay the desktop wallpaper shows through instead
    let ground = select(base * vig, vec3<f32>(0.0), overlay);
    let lg = logo_layer(i.uv);
    var c = ground * (1.0 - lg.a) + lg.rgb;
    // dither by about one 8-bit sRGB step (their size in linear light grows with brightness); one noise value for all
    // channels keeps it neutral and is cheap enough for a full-resolution pass
    let nz = hash12(i.pos.xy + vec2<f32>(p.v.w * 61.0, 0.0)) - 0.5;
    let step = (1.0 / 255.0) * 2.4 * sqrt(max(c, vec3<f32>(0.0)) + vec3<f32>(0.004));
    // grain: fixed to the pixels (it never crawls), about a level either way, so the ground has the tooth of paper
    // rather than the cleanness of a gradient; and a breath of lightness in the middle, behind the form
    let grain = (hash12(floor(i.pos.xy) * 1.37 + vec2<f32>(11.0, 5.0)) + hash12(floor(i.pos.xy * 0.5) + vec2<f32>(3.0, 17.0)) - 1.0) * p.q.w;
    let lift = 1.0 + 0.5 * p.q.w * 0.12 * exp(-dot(r, r) * 1.6);
    // paper: its tooth lit by a low light from the upper left (the slope of the height, which the rasteriser's own
    // derivatives give for nothing), and a ground laid on by hand, a level or two uneven from place to place. All
    // of it a function of the place on the screen alone: it cannot crawl.
    var sheet = 0.0;
    if (p.ex.y > 0.0) {
        let hgt = paper(i.pos.xy, p.ex.w / 2000.0);
        let rake = dpdx(hgt) + dpdy(hgt);
        let wash = sin(i.uv.x * 3.1 + 1.3) * sin(i.uv.y * 2.3 + 0.7) + 0.5 * sin(i.uv.x * 7.3 + i.uv.y * 5.1) + 0.35 * sin(i.uv.x * 13.0 - i.uv.y * 17.0);
        sheet = p.ex.y * (rake * 22.0 + (hgt - 0.5) * 2.2 + wash * 1.4);
    }
    c = max(c * lift + step * (nz * 1.3 + grain + sheet) * select(1.0, 0.0, overlay), vec3<f32>(0.0));
    return vec4<f32>(c, select(1.0, lg.a, overlay));
}

@fragment
fn fs_blit(i: FOut) -> @location(0) vec4<f32> {
    if (p.q.y < 0.5) { return textureSampleLevel(t0, samp, i.uv, 0.0); }
    // pixels: each coarse texel becomes a crisp square of whole output pixels
    let cell = vec2<i32>(i.pos.xy / p.q.y);
    var c = textureLoad(t0, cell, 0);
    if (p.q.x > 1.5) {
        // a 4x4 ordered dither down to a few levels per channel, in display (not linear) values as a VGA card would
        var bayer = array<f32, 16>(0.0, 8.0, 2.0, 10.0, 12.0, 4.0, 14.0, 6.0, 3.0, 11.0, 1.0, 9.0, 15.0, 7.0, 13.0, 5.0);
        let th = (bayer[(cell.y & 3) * 4 + (cell.x & 3)] + 0.5) / 16.0;
        let s = pow(max(c.rgb, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2));
        let n = p.q.x - 1.0;
        let d = floor(s * n + vec3<f32>(th)) / n;
        c = vec4<f32>(pow(d, vec3<f32>(2.2)), c.a);
    }
    return c;
}

// annotations: quads in output pixels, each sampling one glyph (or the atlas's solid texel, for rules and ticks)
struct TOut { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32>, @location(1) col: vec4<f32> };

@vertex
fn vs_text(@location(0) pos: vec2<f32>, @location(1) uv: vec2<f32>, @location(2) col: vec4<f32>) -> TOut {
    var o: TOut;
    o.pos = vec4<f32>(pos.x / p.v.y * 2.0 - 1.0, 1.0 - pos.y / p.ex.w * 2.0, 0.0, 1.0);
    o.uv = uv;
    o.col = col;
    return o;
}

@fragment
fn fs_text(i: TOut) -> @location(0) vec4<f32> {
    let a = textureSampleLevel(t0, samp, i.uv, 0.0).r * i.col.a * scan(i.pos.y);
    return vec4<f32>(i.col.rgb * a, a);
}
