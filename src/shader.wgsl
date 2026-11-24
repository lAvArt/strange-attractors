// The attractor's sheets, drawn four ways from one ring of points: lines along the flow, lines across it, dots, and
// an unseen surface between neighbouring strands that hides what lies behind it.
//
// Every line is one triangle strip whose vertex stage reads the ring straight from a storage buffer (the CPU only
// ever uploads the newest points) and widens each point sideways with a continuous tangent, so there are no joints
// to gap, overlap or bead. Anti-aliasing is analytic: the fragment's coverage is the exact overlap of a pixel-wide
// box with the line, so a hairline thinner than a pixel dims instead of breaking into dots, and depth of field is
// the same formula with a wider box. Nothing is multisampled and nothing is blurred afterwards.
struct U {
    view: mat4x4<f32>,
    proj: mat4x4<f32>,
    c0: vec4<f32>,    // tone in shade; w: flow-line half width, px
    c1: vec4<f32>,    // tone in light; w: cross-line half width, px
    c2: vec4<f32>,    // highlight / accent; w: dot radius, px
    fog: vec4<f32>,   // the ground colour far things sink into; w: how much
    acc: vec4<f32>,   // sparkles
    ids: vec4<u32>,   // n strands, ring length, head slot, strands per ribbon
    p1: vec4<f32>,    // fade, points drawn one by one before every other is skipped, time, tail power
    p2: vec4<f32>,    // camera distance, view units per pixel at depth 1, target height px, stride / spacing of this pass
    st: vec4<f32>,    // light's say in the tone (0..1), depth of field (px of blur per unit of depth), dissolve age, scatter
    st2: vec4<f32>,   // sparkle share, focus offset, dot mode (0 dust, 1 halftone, 2 dissolve), opacity
    st3: vec4<f32>,   // the sheet's own tint (0: unseen, 1: solid; in the dot pass: how faint a covered dot gets), scanlines, each ribbon its own colour (0/1), sheet strip width
};
@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var<storage, read> pts: array<vec4<f32>>;
@group(0) @binding(2) var veil: texture_2d<f32>; // view depth and strength of the nearest sheet (soft looks)
@group(0) @binding(3) var veil_samp: sampler;

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) d: vec3<f32>,     // signed distance from the axis in px, half width px, blur px
    @location(1) col: vec4<f32>,   // rgb, opacity
};

fn hash1(x: u32) -> f32 {
    var h = x * 747796405u + 2891336453u;
    h = ((h >> ((h >> 28u) + 4u)) ^ h) * 277803737u;
    h = (h >> 22u) ^ h;
    return f32(h) / 4294967295.0;
}

// the dark gaps between the lines of a picture tube, two logical pixels apart
fn scan(y: f32) -> f32 {
    return 1.0 - u.st3.y * (0.5 - 0.5 * cos(y * 1.5707963));
}

// a colour for a whole ribbon, for the looks that paint each one differently
fn ribbon_tone(g: u32, l: f32) -> vec3<f32> {
    let h = hash1(g * 2u + 11u);
    var base = u.c0.rgb;
    if (h > 0.34) { base = u.c1.rgb; }
    if (h > 0.67) { base = u.c2.rgb; }
    // flat-shaded plastic: a dark side, the colour, and a hard white highlight
    return mix(base * (0.34 + 0.66 * l), vec3<f32>(1.0), smoothstep(0.86, 0.9, l) * 0.85);
}

fn slot_of(j: u32) -> u32 {
    // j < len, so one conditional subtraction does what a (slow, on integrated GPUs) modulo would
    let s = u.ids.z + u.ids.y - j;
    return select(s, s - u.ids.y, s >= u.ids.y);
}

fn at(j: u32, t: u32) -> vec4<f32> {
    return pts[slot_of(j) * u.ids.x + t];
}

// the ribbon's surface normal at strand t, point j (view space, facing the eye; zero if there is none)
fn sheet_normal(j: u32, t: u32, tw: vec3<f32>, pv: vec3<f32>) -> vec3<f32> {
    let m = u.ids.w;
    if (m < 2u) { return vec3<f32>(0.0); }
    let k = t % m;
    let tl = select(t, t - 1u, k > 0u);
    let tr = select(t, t + 1u, k + 1u < m);
    let c = cross(tw, at(j, tr).xyz - at(j, tl).xyz);
    let cl = length(c);
    if (cl < 1e-9) { return vec3<f32>(0.0); }
    let nv = (u.view * vec4<f32>(c / cl, 0.0)).xyz;
    return nv * sign(dot(nv, -pv));
}

// how lit the sheet is here: a key from the upper left of the camera, as if the form turned on a stand
fn light_of(nv: vec3<f32>) -> f32 {
    if (dot(nv, nv) < 0.5) { return 0.6; }
    let l = clamp(dot(nv, vec3<f32>(-0.46, 0.62, 0.635)) * 0.62 + 0.38, 0.0, 1.0);
    return l * l;
}

// tone from light: shade -> lit -> highlight on the ridges that face the light squarely
fn tone_of(l: f32) -> vec3<f32> {
    let k = mix(0.6, l, u.st.x);
    return mix(mix(u.c0.rgb, u.c1.rgb, smoothstep(0.0, 0.62, k)), u.c2.rgb, smoothstep(0.55, 1.0, k));
}

// blur radius in px at depth w: zero in the focal plane
fn coc_of(w: f32) -> f32 {
    return u.st.y * abs(w - (u.p2.x + u.st2.y)) / w * (u.p2.x * u.p2.z / 1080.0);
}

fn fog_of(w: f32) -> f32 {
    return clamp((w - u.p2.x + 0.8) / 2.0, 0.0, 1.0) * u.fog.w;
}

// widen a point of a polyline (neighbours a and b before and after it) into the strip
fn widen(p: vec3<f32>, a: vec3<f32>, b: vec3<f32>, side: f32, rpx: f32, col: vec3<f32>, alpha: f32) -> VOut {
    var o: VOut;
    let pv = (u.view * vec4<f32>(p, 1.0)).xyz;
    let w = -pv.z;
    let tv = (u.view * vec4<f32>(a - b, 0.0)).xyz;
    let sx = cross(tv, -pv);
    let sl = length(sx);
    // out of focus a line widens a little and then fades, rather than spreading into a wide haze
    let coc = coc_of(w);
    let blur = 0.5 + min(coc, 2.0);
    let ok = step(1e-12, sl) * step(0.05, w) * (1.0 - 0.75 * smoothstep(1.5, 4.5, coc));
    let reach = (rpx + blur) * ok;
    o.pos = u.proj * vec4<f32>(pv + sx / max(sl, 1e-12) * (side * reach * w * u.p2.y), 1.0);
    o.d = vec3<f32>(side * reach, rpx, blur);
    let fg = fog_of(w);
    o.col = vec4<f32>(mix(col, u.fog.rgb, fg), alpha * ok * (1.0 - 0.55 * fg));
    return o;
}

fn age_fade(j: u32) -> f32 {
    let age = f32(j) / f32(u.ids.y - 1u);
    return pow(max(1.0 - age, 0.0), u.p1.w) * min(1.0, f32(j) / 8.0);
}

// lines along the flow: one strip per strand, two vertices per `stride` points
@vertex
fn vs_flow(@builtin(vertex_index) vi: u32, @builtin(instance_index) t: u32) -> VOut {
    let len = u.ids.y;
    // level of detail: the newest `fresh` points one by one, the older, fainter rest every `stride`-th
    let fresh = u32(u.p1.y);
    let v = vi >> 1u;
    let stride = select(u32(u.p2.w), 1u, v < fresh);
    // the thinned part keeps the same points from frame to frame: it is chosen by ring slot, not by age, or the
    // line would be redrawn through other points every time the head moves on
    let first = fresh + ((u.ids.z + fresh) % stride);
    let j = min(select(first + (v - fresh) * stride, v, v < fresh), len - 1u);
    let p = at(j, t);
    let a = at(max(j, stride) - stride, t).xyz;
    let b = at(min(j + stride, len - 1u), t).xyz;
    let side = f32(vi & 1u) * 2.0 - 1.0;
    let tw = normalize(a - b + vec3<f32>(1e-20));
    let pv = (u.view * vec4<f32>(p.xyz, 1.0)).xyz;
    let l = light_of(sheet_normal(j, t, tw, pv));
    let wt = abs(p.w);
    let age = f32(j) / f32(len - 1u);
    // past the dissolve age the lines give way to dots
    let keep = 1.0 - smoothstep(u.st.z - 0.12, u.st.z + 0.06, age);
    let alpha = wt * age_fade(j) * u.p1.x * keep * mix(1.0, 0.62 + 0.38 * l, u.st.x) * u.st2.w;
    return widen(p.xyz, a, b, side, u.c0.w * (0.5 + 0.75 * wt), tone_of(l), alpha);
}

// lines across the flow: every `spacing`-th moment of a ribbon, joined from strand to strand
@vertex
fn vs_cross(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VOut {
    let n = u.ids.x;
    let len = u.ids.y;
    let m = u.ids.w;
    let every = u32(u.p2.w);
    let groups = n / m;
    let g = ii % groups;
    // crosses belong to moments, not to ages: they are born at the head and stay where they were drawn
    let j = (u.ids.z % every) + (ii / groups) * every;
    let k = min(vi >> 1u, m - 1u);
    let t = g * m + k;
    let side = f32(vi & 1u) * 2.0 - 1.0;
    var o: VOut;
    if (j + 1u >= len) {
        o.pos = vec4<f32>(2.0, 2.0, 2.0, 1.0);
        return o;
    }
    let p = at(j, t);
    let a = at(j, g * m + max(k, 1u) - 1u);
    let b = at(j, g * m + min(k + 1u, m - 1u));
    let tw = normalize(at(max(j, 1u) - 1u, t).xyz - at(j + 1u, t).xyz + vec3<f32>(1e-20));
    let pv = (u.view * vec4<f32>(p.xyz, 1.0)).xyz;
    let l = light_of(sheet_normal(j, t, tw, pv));
    // a strand that has let go of its ribbon takes its cross-lines with it
    let held = step(0.0, p.w) * step(0.0, a.w) * step(0.0, b.w) * smoothstep(0.1, 0.5, min(abs(p.w), min(abs(a.w), abs(b.w))));
    let age = f32(j) / f32(len - 1u);
    let keep = 1.0 - smoothstep(u.st.z - 0.12, u.st.z + 0.06, age);
    let alpha = held * age_fade(j) * u.p1.x * keep * mix(1.0, 0.62 + 0.38 * l, u.st.x) * u.st2.w * 0.8;
    return widen(p.xyz, b.xyz, a.xyz, side, u.c1.w, tone_of(l), alpha);
}

@fragment
fn fs_line(i: VOut) -> @location(0) vec4<f32> {
    // a line of half width r under a box of half width b: full where the box fits inside, a ramp across the edge
    let cov = clamp((i.d.y + i.d.z - abs(i.d.x)) / (2.0 * i.d.z), 0.0, min(1.0, i.d.y / i.d.z));
    let a = cov * i.col.a * scan(i.pos.y);
    return vec4<f32>(i.col.rgb * a, a);
}

struct DOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) q: vec2<f32>,     // offset from the centre, px
    @location(1) rb: vec4<f32>,    // radius px, blur px, kind, edge softness
    @location(2) col: vec4<f32>,
};

// dots: one quad per ring point. Dust scatters them off the strands and lets depth of field turn the near and far
// ones into discs; halftone sizes them by the light, on the lattice the strands and the steps make.
@vertex
fn vs_dot(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> DOut {
    let n = u.ids.x;
    let len = u.ids.y;
    let m = u.ids.w;
    let every = max(u32(u.p2.w), 1u);
    let t = ii % n;
    let mode = u.st2.z;
    // every `every`-th point, counted by ring slot (not by age), so a dot that is drawn stays drawn
    let j = min((ii / n) * every + (u.ids.z % every), len - 2u);
    let p = at(j, t);
    let a = at(max(j, 1u) - 1u, t).xyz;
    let b = at(j + 1u, t).xyz;
    var o: DOut;
    let d = a - b;
    let dl = length(d);
    let tw = d / max(dl, 1e-12);
    var pos = p.xyz;
    let age = f32(j) / f32(len - 1u);
    let id = slot_of(j) * n + t;
    let h0 = hash1(id * 3u + 1u);
    let h1 = hash1(id * 3u + 2u);
    let h2 = hash1(id * 3u + 3u);
    var scatter = u.st.w;
    var show = 1.0;
    if (mode > 1.5) {
        // dissolve: dots appear where the lines let go and drift further apart with age
        let x = clamp((age - u.st.z + 0.12) / max(1.0 - u.st.z + 0.12, 1e-3), 0.0, 1.0);
        show = smoothstep(0.0, 0.25, x);
        scatter = u.st.w * x * x;
    }
    let pv0 = (u.view * vec4<f32>(pos, 1.0)).xyz;
    let nv = sheet_normal(j, t, tw, pv0);
    if (mode > 0.5 && mode < 1.5) {
        // halftone: a lattice would beat against the pixel grid and against the layer behind it, so every dot sits
        // at its own fixed offset inside its cell (fixed: it belongs to the point, so nothing shimmers)
        let k = t % m;
        let tn = select(t - 1u, t + 1u, k + 1u < m);
        pos += d * (h0 - 0.5) * 0.9 + (at(j, tn).xyz - p.xyz) * (h1 - 0.5) * 0.8;
    }
    if (scatter > 0.0) {
        // along the strand by up to a step, and off it in a thin cloud that hugs the sheet
        let k = t % m;
        let tn = select(t - 1u, t + 1u, k + 1u < m);
        let acr = at(j, tn).xyz - p.xyz;
        let nw = cross(tw, normalize(acr + vec3<f32>(1e-20)));
        // most grains keep close to their strand, so the strands read as silky filaments; a few stray far and
        // fill the sheet between them
        let hs = hash1(id * 5u + 9u);
        let stray = mix(0.10, 1.0, hs * hs * hs * hs);
        pos += d * (h0 - 0.5) + (acr * (h1 - 0.5) * 2.0 + nw * (h2 - 0.5) * length(acr) * 1.6) * (scatter * stray);
    }
    let pv = (u.view * vec4<f32>(pos, 1.0)).xyz;
    let w = -pv.z;
    let l = light_of(nv);
    var r = u.c2.w;
    var col = tone_of(l);
    // a new dot swells in over a second or so: dots that appeared at full size would make the leading edge sparkle
    let born = smoothstep(0.0, 40.0, f32(j));
    var alpha = abs(p.w) * age_fade(j) * u.p1.x * show * u.st2.w * born;
    var blur = 0.5;
    var kind = 0.0;   // 0: a grain, 1: a disc of light out of focus, 2: a soft light
    var soft = 0.3;   // how much of a disc's radius its edge takes to fade
    if (mode > 0.5 && mode < 1.5) {
        // halftone: the dot's area is the tone (so the shading reads as a smooth ramp), and even in full shade a dot
        // stays a small, clearly lit mark rather than going dark
        r = u.c2.w * sqrt(mix(1.0, 0.10 + 0.95 * l, u.st.x)) * (0.75 + 0.25 * abs(p.w)) * (0.62 + 0.85 * h2 * h2);
        alpha *= step(0.0, p.w);
        if (u.st3.x > 0.0) {
            // behind a nearer sheet a dot is not cut off: it thins and dims, smoothly with how far behind it lies
            // and with how much that sheet itself shows, and never vanishes
            let cp = u.proj * vec4<f32>(pv, 1.0);
            let uv = vec2<f32>(cp.x / cp.w * 0.5 + 0.5, 0.5 - cp.y / cp.w * 0.5);
            let vl = textureSampleLevel(veil, veil_samp, uv, 0.0).rg;
            let cover = smoothstep(0.035, 0.26, w - vl.x) * clamp(vl.y, 0.0, 1.0);
            alpha *= mix(1.0, u.st3.x, cover);
            r *= mix(1.0, 0.82, cover);
        }
    } else {
        let coc = coc_of(w);
        // grains of different sizes, lit by the sheet they ride on
        // (never below a size at which a grain would only twinkle: smaller ones are dimmer, not smaller)
        r = max(u.c2.w * (0.4 + 1.3 * h2 * h2), 0.42);
        alpha *= mix(1.0, 0.3 + 0.7 * l, u.st.x);
        let pick = hash1(id * 7u + 5u);
        if (h1 < u.st2.x) {
            // one in a few hundred is a sparkle in the accent colour
            col = u.acc.rgb;
            r *= 1.7;
            alpha = min(alpha * 1.6, 1.0);
        }
        if (u.st.y > 0.0) {
            // every grain has its own aperture, drawn once from its hash: many small, fewer medium, a few large
            let hz = hash1(id * 11u + 3u);
            let size = 0.22 + 2.0 * hz * hz * hz;
            let k = u.p2.z / 1080.0;
            if (pick > 0.9993 && coc < 2.5) {
                // a few soft lights in the focal plane, no two the same size
                kind = 2.0;
                col = u.c2.rgb;
                blur = (1.3 + 5.5 * size * 0.45) * k;
                alpha = (0.3 + 0.3 * h0) * u.p1.x * age_fade(j);
            } else if (pick > 0.9925) {
                // the brightest grains swell into discs of light as they leave the focal plane: how far they swell
                // follows both the distance from that plane and the grain's own aperture
                let rad = clamp(coc * size, 0.0, 8.5 * k);
                kind = step(2.2, rad);
                blur = mix(0.5 + min(coc, 2.5), rad, kind);
                // small discs are the bright ones; and each has its own softness of edge
                alpha = mix(alpha, (0.07 + 0.20 * h2) * mix(1.25, 0.6, clamp(rad / (8.5 * k), 0.0, 1.0)) * u.p1.x * age_fade(j) * abs(p.w), kind);
                soft = mix(0.12, 0.7, hash1(id * 13u + 1u));
                col = mix(col, u.c2.rgb, 0.5 * kind);
            } else {
                // the rest blur a little and then fade out: no haze of half-seen grains
                blur = 0.5 + min(coc, 2.2);
                alpha *= 1.0 - 0.8 * smoothstep(1.5, 3.5, coc);
            }
        }
    }
    r *= 0.4 + 0.6 * born;
    let ok = step(1e-9, dl) * step(0.05, w);
    let reach = (select(r, 0.0, kind > 0.5) + blur) * ok;
    // one triangle around the dot: three vertices instead of a quad's four
    var cs = array<vec2<f32>, 3>(vec2<f32>(-1.7321, -1.0), vec2<f32>(1.7321, -1.0), vec2<f32>(0.0, 2.0));
    let c = cs[vi] * reach;
    o.pos = u.proj * vec4<f32>(pv + vec3<f32>(c * (w * u.p2.y), 0.0), 1.0);
    o.q = c;
    o.rb = vec4<f32>(r, blur, kind, soft);
    let fg = fog_of(w);
    o.col = vec4<f32>(mix(col, u.fog.rgb, fg), alpha * ok * (1.0 - 0.55 * fg));
    return o;
}

@fragment
fn fs_dot(i: DOut) -> @location(0) vec4<f32> {
    let r = i.rb.x;
    let b = i.rb.y;
    let d = length(i.q);
    var cov = 0.0;
    if (i.rb.z > 1.5) {
        // a soft light: brightest in the middle, gone by its edge
        cov = exp(-4.5 * d * d / (b * b)) * step(d, b);
    } else if (i.rb.z > 0.5) {
        // a disc out of focus: even, a touch brighter at the rim, with a crisp edge
        cov = clamp((b - d) / max(i.rb.w * b, 1.3), 0.0, 1.0) * (0.8 + 0.2 * smoothstep(0.5 * b, b, d));
    } else {
        // a disc of radius r under a blur of radius b: the same ramp as a line, with the light spread over the area
        cov = clamp((r + b - d) / (2.0 * b), 0.0, 1.0) * min(1.0, (r * r) / (b * b));
    }
    let a = min(cov * i.col.a, 1.0) * scan(i.pos.y);
    return vec4<f32>(i.col.rgb * a, a);
}

struct SOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) col: vec4<f32>,
    @location(1) vw: vec2<f32>,   // view depth, how much this sheet covers what is behind it
};

// the sheet itself: strips from strand to strand. It writes depth, which is what makes a near fold hide a far one,
// and lays a thin wash of the form's own tone under the lines so the gaps between them are never holes of ground.
@vertex
fn vs_sheet(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> SOut {
    let len = u.ids.y;
    let m = u.ids.w;
    let stride = u32(u.p2.w);
    // one strip spans `wide` strands: the sheet bends little across so few, and big triangles are far cheaper to
    // fill than slivers
    let wide = u32(u.st3.w);
    let per = (m - 2u) / wide + 1u;
    let k = (ii % per) * wide;
    let g = ii / per;
    let t = g * m + min(k + (vi & 1u) * wide, m - 1u);
    // the newest point, then every `stride`-th by ring slot: the same points every frame
    let v = vi >> 1u;
    let j = min(select((v - 1u) * stride + (u.ids.z % stride), 0u, v == 0u), len - 1u);
    let p = at(j, t);
    var o: SOut;
    let pv0 = (u.view * vec4<f32>(p.xyz, 1.0)).xyz;
    // a hair behind its own lines, so they are never the ones it hides
    let pv = pv0 * 1.0025;
    o.pos = u.proj * vec4<f32>(pv, 1.0);
    let age = f32(j) / f32(len - 1u);
    let keep = 1.0 - smoothstep(u.st.z - 0.2, u.st.z - 0.05, age);
    let fade = age_fade(j);
    let a = step(0.0, p.w) * smoothstep(0.15, 0.6, abs(p.w)) * smoothstep(0.0, 0.5, fade) * u.p1.x * keep;
    let tw = normalize(at(max(j, stride) - stride, t).xyz - at(min(j + stride, len - 1u), t).xyz + vec3<f32>(1e-20));
    let l = light_of(sheet_normal(j, t, tw, pv0));
    let w = -pv0.z;
    let fg = fog_of(w);
    var tone = tone_of(l);
    if (u.st3.z > 0.5) { tone = ribbon_tone(g, l); }
    let alpha = u.st3.x * mix(fade * (1.0 - 0.6 * fg), 1.0, step(0.999, u.st3.x)) * u.p1.x;
    o.col = vec4<f32>(mix(tone, u.fog.rgb, fg * step(u.st3.x, 0.999)), alpha);
    // a sheet covers as much as it shows: where its own marks are faint (in shade, at the tail) it hides little
    o.vw = vec2<f32>(w, clamp(0.15 + 0.95 * l, 0.0, 1.0) * fade * u.p1.x);
    // where its lines have mostly faded the sheet stops hiding things: those vertices go beyond the far plane, and
    // clipping cuts the strip cleanly between them and the last solid ones
    if (a < 0.5) { o.pos.z = o.pos.w * 2.0; }
    return o;
}

@fragment
fn fs_sheet(i: SOut) -> @location(0) vec4<f32> {
    let a = i.col.a * mix(1.0, scan(i.pos.y), step(i.col.a, 0.999));
    return vec4<f32>(i.col.rgb * a, a);
}

@fragment
fn fs_veil(i: SOut) -> @location(0) vec4<f32> {
    return vec4<f32>(i.vw, 0.0, 0.0);
}

// motes: a few hundred specks of dust hanging in the air around the form, at fixed places and many depths. They
// belong to the ground, but being placed in space they slide against the form as the camera goes round.
@vertex
fn vs_mote(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> DOut {
    let a = hash1(ii * 4u + 1u) * 6.2831853;
    let z = hash1(ii * 4u + 2u) * 2.0 - 1.0;
    let rad = 1.2 + 2.6 * hash1(ii * 4u + 3u);
    let s = sqrt(max(1.0 - z * z, 0.0));
    let pos = vec3<f32>(s * cos(a), s * sin(a), z * 0.8) * rad;
    let pv = (u.view * vec4<f32>(pos, 1.0)).xyz;
    let w = -pv.z;
    let hz = hash1(ii * 4u + 4u);
    let k = u.p2.z / 1080.0;
    let r = max((0.3 + 1.1 * hz * hz * hz) * k, 0.42);
    let coc = u.st.y * abs(w - u.p2.x) / max(w, 0.2) * (u.p2.x * k) * 0.35 + 0.6 * abs(w - u.p2.x);
    let blur = 0.5 + min(coc, 4.0 * k);
    let ok = step(0.4, w);
    let reach = (r + blur) * ok;
    var cs = array<vec2<f32>, 3>(vec2<f32>(-1.7321, -1.0), vec2<f32>(1.7321, -1.0), vec2<f32>(0.0, 2.0));
    let c = cs[vi] * reach;
    var o: DOut;
    o.pos = u.proj * vec4<f32>(pv + vec3<f32>(c * (w * u.p2.y), 0.0), 1.0);
    o.q = c;
    o.rb = vec4<f32>(r, blur, 0.0, 0.3);
    o.col = vec4<f32>(u.c1.rgb, (0.25 + 0.75 * hash1(ii * 7u + 9u)) * u.st2.w * ok);
    return o;
}
