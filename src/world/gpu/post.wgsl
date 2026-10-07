// Post and style (render.rs after the air: crop, pick, heat shimmer, lens; post::finish with
// bloom; looks::kuwahara and grade; post::outline, lens_warp, quantize; frame stats; upscale and
// looks::surface). Every image between stages lives in `scratch` at offsets the CPU picks for the
// frame (it knows which stages run), so all kernels share one layout.

struct Post {
    // Frame (guarded) size is W.width x W.height; the post size, its crop offset, the output.
    w: u32, h: u32, gl: u32, gt: u32,
    out_w: u32, out_h: u32, px: u32, pick_on: u32,
    // Scratch offsets (in words): cropped G-buffer (5 words a pixel), pick, bloom (two quarter
    // images), Kuwahara sums (14 words a pixel), reductions.
    o_pg: u32, o_pk: u32, o_q0: u32, o_q1: u32,
    o_ks: u32, o_red: u32, qw: u32, qh: u32,
    // hdr images (3 words a pixel): crop writes h_crop; shimmer h_crop -> h_shim; lens h_lsrc ->
    // h_ldst; finish reads h_fin.
    h_crop: u32, h_shim: u32, h_lsrc: u32, h_ldst: u32,
    h_fin: u32, r_fin: u32, r_kuw: u32, r_mid: u32,
    // rgb images: finish writes r_fin; Kuwahara r_fin -> r_kuw; grade and outline on r_mid;
    // warp r_mid -> r_wdst; quantize, stats and the output read r_last.
    r_wdst: u32, r_last: u32, hz: f32, unit: f32,
    // Heat shimmer.
    sh_on: u32, sh_strength: f32, sh_c1: f32, sh_c2: f32,
    // Lens: kind 0 none, 1 drops, 2 frost.
    lens_kind: u32, lens_weight: f32, n_drops: u32, lens_seed: u32,
    th: f32, tw: f32, aspect: f32, pad0: u32,
    // finish.
    post_on: u32, bloom: f32, exposure: f32, grain_seed: u32,
    tint_r: f32, tint_g: f32, tint_b: f32, saturation: f32,
    contrast: f32, vignette: f32, grain: f32, kuw_r: i32,
    // Grade: 0 none, 1..7 built in (looks::BUILTIN order), 8 a .cube of size grade_n.
    grade: u32, grade_n: u32, grade_s: f32, ol_on: u32,
    ol_objects_only: u32, ol_r: f32, ol_g: f32, ol_b: f32,
    // Lens warp.
    lw_on: u32, lw_amp: f32, lw_den: f32, lw_blend_top: f32,
    lw_span: f32, lw_nearest: u32, ramp_on: u32, ramp_lo: f32,
    ramp_k: f32, q_on: u32, q_bits: u32, q_amp: f32,
    dither: u32, paper: f32, scan: f32, stats_on: u32,
    sky_enabled: u32, verge_enabled: u32, t_lut: u32, t_col: u32,
    t_cube: u32, t_drops: u32, pad1: u32, pad2: u32,
}

@group(0) @binding(1) var<uniform> P: Post;
@group(0) @binding(2) var<storage, read> fhdr: array<f32>;
@group(0) @binding(3) var<storage, read> fgbuf: array<GPix>;
@group(0) @binding(4) var<storage, read> fpick: array<u32>;
@group(0) @binding(5) var<storage, read_write> scratch: array<f32>;
// sRGB buckets (4096, one per word), sRGB starts (257 floats), then the palette LUT (bytes
// packed four to a word), palette colours (0xRRGGBB), the .cube. (The lens drops, 5 floats each,
// are in scratch at t_drops.)
@group(0) @binding(6) var<storage, read> tables: array<u32>;
@group(0) @binding(7) var<storage, read_write> outp: array<u32>;

fn rd3(o: u32, i: u32) -> vec3<f32> { return vec3<f32>(scratch[o + i * 3u], scratch[o + i * 3u + 1u], scratch[o + i * 3u + 2u]); }
fn wr3(o: u32, i: u32, c: vec3<f32>) { scratch[o + i * 3u] = c.x; scratch[o + i * 3u + 1u] = c.y; scratch[o + i * 3u + 2u] = c.z; }
fn pg(i: u32) -> GPix {
    let o = P.o_pg + i * 5u;
    return GPix(scratch[o], scratch[o + 1u], scratch[o + 2u], scratch[o + 3u], bitcast<u32>(scratch[o + 4u]));
}
fn pg_id(i: u32) -> u32 { return bitcast<u32>(scratch[P.o_pg + i * 5u + 4u]) & 0xFFu; }
fn pg_depth(i: u32) -> f32 { return scratch[P.o_pg + i * 5u]; }

// ── crop, and the pick ids of what no card covers ─────────────────────────
@compute @workgroup_size(8, 8)
fn post_crop(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= P.w || gid.y >= P.h { return; }
    let i = gid.y * P.w + gid.x;
    let j = (gid.y + P.gt) * W.width + gid.x + P.gl;
    scratch[P.h_crop + i * 3u] = fhdr[j * 3u]; scratch[P.h_crop + i * 3u + 1u] = fhdr[j * 3u + 1u]; scratch[P.h_crop + i * 3u + 2u] = fhdr[j * 3u + 2u];
    let g = fgbuf[j];
    let o = P.o_pg + i * 5u;
    scratch[o] = g.depth; scratch[o + 1u] = g.x; scratch[o + 2u] = g.y; scratch[o + 3u] = g.d; scratch[o + 4u] = bitcast<f32>(g.idr);
    if P.pick_on != 0u {
        var p = fpick[j];
        if p == 0u {
            let id = g.idr & 0xFFu;
            if id == ID_NONE { p = 5u; }
            else if id == ID_WALL_L || id == ID_WALL_R { p = 3u; }
            else if id == ID_CEILING { p = 4u; }
            else if id == ID_GROUND && P.verge_enabled != 0u && abs(g.x) > path_edge(g.d) { p = 2u; }
            else { p = 1u; }
        }
        scratch[P.o_pk + i] = bitcast<f32>(p);
    }
}

// weather::bilinear over a w x h image at offset o.
fn bilinear(o: u32, x_in: f32, y_in: f32) -> vec3<f32> {
    let x = clamp(x_in, 0.0, f32(P.w) - 1.0); let y = clamp(y_in, 0.0, f32(P.h) - 1.0);
    let x0 = u32(floor(x)); let y0 = u32(floor(y));
    let x1 = min(x0 + 1u, P.w - 1u); let y1 = min(y0 + 1u, P.h - 1u);
    let tx = x - f32(x0); let ty = y - f32(y0);
    let top = mix3(rd3(o, y0 * P.w + x0), rd3(o, y0 * P.w + x1), tx);
    let bot = mix3(rd3(o, y1 * P.w + x0), rd3(o, y1 * P.w + x1), tx);
    return mix3(top, bot, ty);
}

// ── heat shimmer (weather::heat_shimmer) ──────────────────────────────────
@compute @workgroup_size(8, 8)
fn post_shimmer(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= P.w || gid.y >= P.h { return; }
    let x = gid.x; let y = gid.y;
    let i = y * P.w + x;
    var c = rd3(P.h_crop, i);
    let hy = P.hz; let hf_ = f32(P.h);
    let yf = f32(y) + 0.5;
    let band = smoothstep_r(hy - 0.07 * hf_, hy - 0.01 * hf_, yf) * (1.0 - smoothstep_r(hy + 0.02 * hf_, hy + 0.3 * hf_, yf));
    var far = 1.0;
    if pg_id(i) != ID_NONE { far = smoothstep_r(5.0, 35.0, pg_depth(i)); }
    let amp = P.sh_strength * 2.5 / P.unit * band * far;
    if amp >= 0.03 {
        let t = TAU * W.tphase;
        let fx = f32(x) * P.unit; let fy = yf * P.unit;
        let dx = amp * (0.6 * sin(0.31 * fy + 0.05 * fx + P.sh_c1 * t) + 0.4 * sin(0.77 * fy - 0.09 * fx - P.sh_c2 * t + 1.3));
        let dy = amp * 0.3 * sin(0.21 * fx + 0.45 * fy + P.sh_c2 * t);
        c = bilinear(P.h_crop, f32(x) + dx, f32(y) + dy);
    }
    wr3(P.h_shim, i, c);
}

// ── lens (weather::lens) ──────────────────────────────────────────────────
fn wlum(c: vec3<f32>) -> f32 { return 0.2126 * c.x + 0.7152 * c.y + 0.0722 * c.z; }

// One thread per row: the row's luminance summed (frost's mean), then one thread sums the rows.
@compute @workgroup_size(64)
fn post_frost_rows(@builtin(global_invocation_id) gid: vec3<u32>) {
    let y = gid.x;
    if y >= P.h { return; }
    var s = 0.0;
    for (var x = 0u; x < P.w; x++) { s += wlum(rd3(P.h_lsrc, y * P.w + x)); }
    scratch[P.o_red + 1u + y] = s;
}
@compute @workgroup_size(1)
fn post_frost_mean() {
    var s = 0.0;
    for (var y = 0u; y < P.h; y++) { s += scratch[P.o_red + 1u + y]; }
    scratch[P.o_red] = s / f32(max(P.w * P.h, 1u));
}

@compute @workgroup_size(8, 8)
fn post_lens(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= P.w || gid.y >= P.h { return; }
    let x = gid.x; let y = gid.y;
    let i = y * P.w + x;
    var c = rd3(P.h_lsrc, i);
    let w = f32(P.w); let h = f32(P.h);
    if P.lens_kind == 1u {
        for (var k = 0u; k < P.n_drops; k++) {
            let o = P.t_drops + k * 5u;
            let cx = scratch[o]; let cy = scratch[o + 1u]; let r = scratch[o + 2u];
            let ry = scratch[o + 3u]; let alpha = scratch[o + 4u];
            let ya = u32(max(floor(cy - ry), 0.0)); let yb = min(u32(ceil(cy + ry)), P.h - 1u);
            let xa = u32(max(floor(cx - r), 0.0)); let xb = min(u32(ceil(cx + r)), P.w - 1u);
            if y < ya || y > yb || x < xa || x > xb { continue; }
            let dx = div(f32(x) + 0.5 - cx, r); let dy = div(f32(y) + 0.5 - cy, ry);
            let q = rnd(dx * dx) + rnd(dy * dy);
            if q >= 1.0 { continue; }
            let sx = cx - dx * r * 2.4; let sy = cy - dy * ry * 2.4;
            let refr = (bilinear(P.h_lsrc, sx - 1.0, sy) + bilinear(P.h_lsrc, sx + 1.0, sy) + bilinear(P.h_lsrc, sx, sy + 1.0)) * (1.0 / 3.0);
            let rim = smoothstep_r(0.55, 1.0, sqrt(q));
            let gx = dx + 0.35; let gy = dy + 0.4;
            let glint = exp(-(gx * gx + gy * gy) / 0.02);
            let col = refr * (1.0 - 0.55 * rim) + (refr + vec3<f32>(0.25)) * (glint * 0.8);
            c = mix3(c, col, alpha * (1.0 - 0.3 * rim));
        }
    } else if P.lens_kind == 2u {
        let mean = scratch[P.o_red];
        let frost = vec3<f32>(0.82, 0.88, 0.95) * (0.15 + 0.85 * min(mean, 1.5));
        let u = (f32(x) + 0.5) / w; let v = (f32(y) + 0.5) / h;
        let eu = abs(2.0 * u - 1.0); let ev = abs(2.0 * v - 1.0);
        let e = max(max(eu, ev), (sqrt(eu * eu + ev * ev) / 1.41) * 0.95);
        let seed = f32(P.lens_seed);
        let n = 0.6 * value_noise(u * 9.0 * P.aspect + seed, v * 9.0) + 0.4 * value_noise(u * 23.0 * P.aspect, v * 23.0 + seed);
        let fine = value_noise(u * 70.0 * P.aspect, v * 70.0);
        let f = smoothstep_r(P.th - 0.06, P.th + 0.18, e + (n - 0.5) * 0.35) * (0.45 + 0.55 * smoothstep_r(0.3, 0.7, fine + (e - P.th) * 1.5));
        if f > 0.0 {
            let fx = f32(x); let fy = f32(y);
            let blur = (bilinear(P.h_lsrc, fx - 2.0, fy) + bilinear(P.h_lsrc, fx + 2.0, fy) + (bilinear(P.h_lsrc, fx, fy - 2.0) + bilinear(P.h_lsrc, fx, fy + 2.0))) * 0.25;
            var ice = blur * 0.7 + frost * (0.35 + 0.45 * fine);
            if fine > 0.9 {
                let ph = hf(P.lens_seed ^ 0x49u, i32(x) * 7919 + i32(y)) * TAU;
                let s = max(sin(TAU * P.tw * W.tphase + ph), 0.0);
                let s2 = s * s; let s4 = s2 * s2;
                ice += frost * (1.5 * s4 * s4);
            }
            c = mix3(c, ice, f * 0.92 * P.lens_weight);
        }
    }
    wr3(P.h_ldst, i, c);
}

// ── bloom (post::bloom_buffer, blur) ──────────────────────────────────────
@compute @workgroup_size(8, 8)
fn post_bloom_down(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= P.qw || gid.y >= P.qh { return; }
    var acc = vec3<f32>(0.0); var n = 0.0;
    for (var y = gid.y * 4u; y < min(gid.y * 4u + 4u, P.h); y++) {
        for (var x = gid.x * 4u; x < min(gid.x * 4u + 4u, P.w); x++) {
            let c = rd3(P.h_fin, y * P.w + x);
            let l = 0.3 * c.x + 0.55 * c.y + 0.15 * c.z;
            let k = max((l - 0.7) / max(l, 1e-4), 0.0);
            acc += c * k; n += 1.0;
        }
    }
    wr3(P.o_q0, gid.y * P.qw + gid.x, acc / n);
}
const BK = array<f32, 7>(0.03, 0.1, 0.2, 0.34, 0.2, 0.1, 0.03);
fn blur_at(src: u32, x: u32, y: u32, horizontal: bool) -> vec3<f32> {
    var acc = vec3<f32>(0.0);
    for (var t = 0; t < 7; t++) {
        let o = t - 3;
        var sx = i32(x); var sy = i32(y);
        if horizontal { sx = clamp(sx + o * 2, 0, i32(P.qw) - 1); } else { sy = clamp(sy + o * 2, 0, i32(P.qh) - 1); }
        acc += rd3(src, u32(sy) * P.qw + u32(sx)) * BK[t];
    }
    return acc;
}
@compute @workgroup_size(8, 8)
fn post_blur_h(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= P.qw || gid.y >= P.qh { return; }
    wr3(P.o_q1, gid.y * P.qw + gid.x, blur_at(P.o_q0, gid.x, gid.y, true));
}
@compute @workgroup_size(8, 8)
fn post_blur_v(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= P.qw || gid.y >= P.qh { return; }
    wr3(P.o_q0, gid.y * P.qw + gid.x, blur_at(P.o_q1, gid.x, gid.y, false));
}

// texture::lin_to_srgb, from its own tables (so bytes match exactly).
fn lin_to_srgb(t: f32) -> f32 {
    let v = clamp(t, 0.0, 1.0);
    var b = tables[min(u32(v * 4096.0), 4095u)];
    loop {
        if b >= 255u || v < bitcast<f32>(tables[4096u + b + 1u]) { break; }
        b += 1u;
    }
    return f32(b);
}

// ── post::finish ──────────────────────────────────────────────────────────
@compute @workgroup_size(8, 8)
fn post_finish(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= P.w || gid.y >= P.h { return; }
    let x = gid.x; let y = gid.y;
    let i = y * P.w + x;
    var c = rd3(P.h_fin, i);
    if P.bloom > 0.0 {
        let fx = clamp((f32(x) + 0.5) / 4.0 - 0.5, 0.0, f32(P.qw) - 1.0);
        let fy = clamp((f32(y) + 0.5) / 4.0 - 0.5, 0.0, f32(P.qh) - 1.0);
        let x0 = u32(fx); let y0 = u32(fy);
        let x1 = min(x0 + 1u, P.qw - 1u); let y1 = min(y0 + 1u, P.qh - 1u);
        let tx = fx - f32(x0); let ty = fy - f32(y0);
        let a = rd3(P.o_q0, y0 * P.qw + x0) * (1.0 - tx) + rd3(P.o_q0, y0 * P.qw + x1) * tx;
        let b = rd3(P.o_q0, y1 * P.qw + x0) * (1.0 - tx) + rd3(P.o_q0, y1 * P.qw + x1) * tx;
        c += (a * (1.0 - ty) + b * ty) * P.bloom * 0.9;
    }
    var rgb: vec3<f32>;
    for (var k = 0; k < 3; k++) {
        let v = rnd(rnd(c[k] * P.exposure) * 1.15);
        let t = div(rnd(v * (rnd(2.51 * v) + 0.03)), rnd(v * (rnd(2.43 * v) + 0.59)) + 0.14);
        rgb[k] = lin_to_srgb(t);
    }
    if P.post_on != 0u {
        let lum = 0.299 * rgb.x + 0.587 * rgb.y + 0.114 * rgb.z;
        let tint = vec3<f32>(P.tint_r, P.tint_g, P.tint_b);
        for (var k = 0; k < 3; k++) {
            var v = lum + (rgb[k] - lum) * P.saturation;
            v = (v - 128.0) * P.contrast + 128.0;
            rgb[k] = v * tint[k];
        }
        if P.vignette > 0.001 {
            let cx = f32(P.w) * 0.5; let cy = f32(P.h) * 0.5;
            let dx = f32(x) - cx; let dy = f32(y) - cy;
            let f = max(1.0 - pow((dx * dx + dy * dy) / (cx * cx + cy * cy), 1.4) * P.vignette, 0.0);
            rgb *= f;
        }
        if P.grain > 0.001 {
            var hs = (x * 0x8da6b343u) ^ (y * 0xd8163841u) ^ (P.grain_seed * 0xcb1ab31fu);
            hs ^= hs >> 15u; hs = hs * 0x2c1b3c6du; hs ^= hs >> 12u;
            let g = (f32(hs & 0xffffu) / 32768.0 - 1.0) * P.grain * 40.0;
            rgb += vec3<f32>(g);
        }
    }
    wr3(P.r_fin, i, clamp(rgb, vec3<f32>(0.0), vec3<f32>(255.0)));
}

// ── looks::kuwahara: window sums along rows, then down columns ────────────
fn luma(c: vec3<f32>) -> f32 { return 0.299 * c.x + 0.587 * c.y + 0.114 * c.z; }

// Per pixel, two windows along the row ([x - r, x] and [x, x + r], clipped): r, g, b sums, then
// the luma and luma-squared sums as double-floats (7 words each).
@compute @workgroup_size(8, 8)
fn post_kuw_h(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= P.w || gid.y >= P.h { return; }
    let r = P.kuw_r; let x = i32(gid.x); let y = gid.y;
    let i = y * P.w + gid.x;
    for (var side = 0; side < 2; side++) {
        let xa = select(x, max(x - r, 0), side == 0);
        let xb = select(min(x + r, i32(P.w) - 1), x, side == 0);
        var s = vec3<f32>(0.0); var l = vec2<f32>(0.0); var l2 = vec2<f32>(0.0);
        for (var xx = xa; xx <= xb; xx++) {
            let c = rd3(P.r_fin, y * P.w + u32(xx));
            let lu = luma(c);
            s += c;
            l = df_add(l, vec2<f32>(lu, 0.0));
            l2 = df_add(l2, two_prod(lu, lu));
        }
        let o = P.o_ks + (i * 2u + u32(side)) * 7u;
        scratch[o] = s.x; scratch[o + 1u] = s.y; scratch[o + 2u] = s.z;
        scratch[o + 3u] = l.x; scratch[o + 4u] = l.y; scratch[o + 5u] = l2.x; scratch[o + 6u] = l2.y;
    }
}
@compute @workgroup_size(8, 8)
fn post_kuw_v(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= P.w || gid.y >= P.h { return; }
    let r = P.kuw_r; let x = i32(gid.x); let y = i32(gid.y);
    let i = gid.y * P.w + gid.x;
    var best_v = 3.4e38; var best = vec3<f32>(0.0);
    // (dx, dy) in [(-r, -r), (0, -r), (-r, 0), (0, 0)]: side 0 is the window left of x.
    for (var k = 0; k < 4; k++) {
        let side = u32(k % 2);
        let up = k < 2;
        let ya = select(y, max(y - r, 0), up);
        let yb = select(min(y + r, i32(P.h) - 1), y, up);
        let xa = select(x, max(x - r, 0), side == 0u);
        let xb = select(min(x + r, i32(P.w) - 1), x, side == 0u);
        let n = f32((xb - xa + 1) * (yb - ya + 1));
        var s = vec3<f32>(0.0); var l = vec2<f32>(0.0); var l2 = vec2<f32>(0.0);
        for (var yy = ya; yy <= yb; yy++) {
            let o = P.o_ks + ((u32(yy) * P.w + gid.x) * 2u + side) * 7u;
            s += vec3<f32>(scratch[o], scratch[o + 1u], scratch[o + 2u]);
            l = df_add(l, vec2<f32>(scratch[o + 3u], scratch[o + 4u]));
            l2 = df_add(l2, vec2<f32>(scratch[o + 5u], scratch[o + 6u]));
        }
        let var_ = df_to_f(df_add(df_div(l2, n), df_neg(df_sq(df_div(l, n)))));
        if var_ < best_v { best_v = var_; best = vec3<f32>(div(s.x, n), div(s.y, n), div(s.z, n)); }
    }
    wr3(P.r_kuw, i, best);
}

// ── looks::grade (built in, or a .cube) ───────────────────────────────────
fn mixv(a: vec3<f32>, b: vec3<f32>, t: f32) -> vec3<f32> { return a + (b - a) * t; }
fn grade_of(c: vec3<f32>) -> vec3<f32> {
    switch P.grade {
        case 1u: { return vec3<f32>(c.x * 1.08 + 6.0, c.y * 1.0 + 2.0, c.z * 0.86); }
        case 2u: { return vec3<f32>(c.x * 0.88, c.y * 0.98 + 2.0, c.z * 1.1 + 8.0); }
        case 3u: {
            let t = clamp(luma(c) / 255.0, 0.0, 1.0);
            let sh = vec3<f32>(c.x * 0.82, c.y * 1.0 + 6.0, c.z * 1.08 + 10.0);
            let hi = vec3<f32>(c.x * 1.1 + 8.0, c.y * 1.0 + 2.0, c.z * 0.82);
            return mixv(sh, hi, t * t * (3.0 - 2.0 * t));
        }
        case 4u: { let l = luma(c); let d = mixv(c, vec3<f32>(l), 0.25); return vec3<f32>(d.x * 0.82 + 28.0, d.y * 0.82 + 26.0, d.z * 0.82 + 30.0); }
        case 5u: { let l = luma(c); let d = mixv(c, vec3<f32>(l), 0.45); return vec3<f32>(d.x * 0.7, d.y * 0.82, d.z * 1.05 + 6.0); }
        case 6u: { return vec3<f32>(0.393 * c.x + 0.769 * c.y + 0.189 * c.z, 0.349 * c.x + 0.686 * c.y + 0.168 * c.z, 0.272 * c.x + 0.534 * c.y + 0.131 * c.z); }
        case 7u: { let l = luma(c); let s = mixv(vec3<f32>(l), c, 1.35); return (s - vec3<f32>(128.0)) * 1.08 + vec3<f32>(128.0); }
        case 8u: {
            let n = P.grade_n;
            var i0: vec3<u32>; var fr: vec3<f32>;
            for (var k = 0; k < 3; k++) {
                let f = clamp(c[k] / 255.0, 0.0, 1.0) * f32(n - 1u);
                let ii = min(u32(f), n - 2u);
                i0[k] = ii; fr[k] = f - f32(ii);
            }
            var out = vec3<f32>(0.0);
            for (var dr = 0u; dr < 2u; dr++) { for (var dg = 0u; dg < 2u; dg++) { for (var db = 0u; db < 2u; db++) {
                let wr = select(1.0 - fr.x, fr.x, dr == 1u); let wg = select(1.0 - fr.y, fr.y, dg == 1u); let wb = select(1.0 - fr.z, fr.z, db == 1u);
                let e = P.t_cube + (((i0.z + db) * n + i0.y + dg) * n + i0.x + dr) * 3u;
                out += vec3<f32>(bitcast<f32>(tables[e]), bitcast<f32>(tables[e + 1u]), bitcast<f32>(tables[e + 2u])) * (wr * wg * wb);
            }}}
            return out;
        }
        default: { return c; }
    }
}

// Grade and outline, on r_mid in place (each reads only its own pixel's colour).
@compute @workgroup_size(8, 8)
fn post_grade_outline(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= P.w || gid.y >= P.h { return; }
    let x = gid.x; let y = gid.y;
    let i = y * P.w + x;
    var c = rd3(P.r_mid, i);
    if P.grade != 0u {
        let s = clamp(P.grade_s, 0.0, 1.0);
        c = clamp(mixv(c, grade_of(c), s), vec3<f32>(0.0), vec3<f32>(255.0));
    }
    if P.ol_on != 0u {
        let g = pg(i);
        let gid_ = g.idr & 0xFFu;
        let g_obj = gid_ >= ID_PROP && !is_rail(gid_) && g.depth < 14.0;
        var hit = false;
        for (var k = 0; k < 4; k++) {
            var nx = i32(x); var ny = i32(y);
            if k == 0 { nx -= 1; } else if k == 1 { nx += 1; } else if k == 2 { ny -= 1; } else { ny += 1; }
            if nx < 0 || ny < 0 || nx >= i32(P.w) || ny >= i32(P.h) { continue; }
            let n = pg(u32(ny) * P.w + u32(nx));
            let nid = n.idr & 0xFFu;
            let n_obj = nid >= ID_PROP && !is_rail(nid) && n.depth < 14.0;
            if n_obj && !g_obj && n.depth < g.depth { hit = true; break; }
            if n_obj && g_obj && n.depth < g.depth * 0.85 { hit = true; break; }
            if P.ol_objects_only == 0u && nid != gid_ && n.depth < g.depth && gid_ != ID_NONE && nid != ID_NONE
                && !(nid == ID_GROUND && (gid_ == ID_WALL_L || gid_ == ID_WALL_R)) { hit = true; break; }
        }
        if hit { c = vec3<f32>(P.ol_r, P.ol_g, P.ol_b); }
    }
    wr3(P.r_mid, i, c);
}

// ── post::lens_warp ───────────────────────────────────────────────────────
@compute @workgroup_size(8, 8)
fn post_warp(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= P.w || gid.y >= P.h { return; }
    let x = gid.x; let y = gid.y;
    let t = clamp((f32(y) - P.lw_blend_top) / P.lw_span, 0.0, 1.0);
    let weight = t * t * (3.0 - 2.0 * t);
    let nx = f32(x) / P.lw_den * 2.0 - 1.0;
    let sy = clamp(f32(y) - P.lw_amp * nx * nx * weight, 0.0, f32(P.h - 1u));
    var c: vec3<f32>;
    if P.lw_nearest != 0u {
        c = rd3(P.r_mid, min(u32(rround(sy)), P.h - 1u) * P.w + x);
    } else {
        let y0 = u32(floor(sy)); let y1 = min(y0 + 1u, P.h - 1u);
        let f = sy - f32(y0);
        c = mixv(rd3(P.r_mid, y0 * P.w + x), rd3(P.r_mid, y1 * P.w + x), f);
    }
    wr3(P.r_wdst, y * P.w + x, c);
}

// ── ramp levels and post::quantize, on r_last in place ────────────────────
fn bayer(x: u32, y: u32) -> f32 {
    switch P.dither {
        case 1u: { let b = array<u32, 4>(0u, 2u, 3u, 1u); return (f32(b[(y & 1u) * 2u + (x & 1u)]) + 0.5) / 4.0 - 0.5; }
        case 2u: { let b = array<u32, 16>(0u, 8u, 2u, 10u, 12u, 4u, 14u, 6u, 3u, 11u, 1u, 9u, 15u, 7u, 13u, 5u); return (f32(b[(y & 3u) * 4u + (x & 3u)]) + 0.5) / 16.0 - 0.5; }
        case 3u: {
            let b = array<u32, 64>(0u, 32u, 8u, 40u, 2u, 34u, 10u, 42u, 48u, 16u, 56u, 24u, 50u, 18u, 58u, 26u, 12u, 44u, 4u, 36u, 14u, 46u, 6u, 38u, 60u, 28u, 52u, 20u, 62u, 30u, 54u, 22u,
                3u, 35u, 11u, 43u, 1u, 33u, 9u, 41u, 51u, 19u, 59u, 27u, 49u, 17u, 57u, 25u, 15u, 47u, 7u, 39u, 13u, 45u, 5u, 37u, 63u, 31u, 55u, 23u, 61u, 29u, 53u, 21u);
            return (f32(b[(y & 7u) * 8u + (x & 7u)]) + 0.5) / 64.0 - 0.5;
        }
        default: { return 0.0; }
    }
}
@compute @workgroup_size(8, 8)
fn post_quant(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= P.w || gid.y >= P.h { return; }
    let i = gid.y * P.w + gid.x;
    var c = rd3(P.r_last, i);
    if P.ramp_on != 0u { c = clamp((c - vec3<f32>(P.ramp_lo)) * P.ramp_k, vec3<f32>(0.0), vec3<f32>(255.0)); }
    if P.q_on != 0u {
        let t = bayer(gid.x, gid.y) * P.q_amp;
        let sh = 8u - P.q_bits;
        var q: vec3<u32>;
        for (var k = 0; k < 3; k++) { q[k] = u32(clamp(c[k] + t, 0.0, 255.0)) >> sh; }
        let idx = (q.x << (2u * P.q_bits)) | (q.y << P.q_bits) | q.z;
        let e = (tables[P.t_lut + idx / 4u] >> ((idx % 4u) * 8u)) & 0xFFu;
        let col = tables[P.t_col + e];
        c = vec3<f32>(f32((col >> 16u) & 0xFFu), f32((col >> 8u) & 0xFFu), f32(col & 0xFFu));
    }
    wr3(P.r_last, i, c);
}

// ── frame stats: one thread per row (classes as in render::frame_stats) ───
@compute @workgroup_size(64)
fn post_stats(@builtin(global_invocation_id) gid: vec3<u32>) {
    let y = gid.x;
    if y >= P.h { return; }
    var n = array<f32, 11>(0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    var l = array<f32, 11>(0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    var total = 0.0;
    for (var x = 0u; x < P.w; x++) {
        let i = y * P.w + x;
        let g = pg(i);
        let id = g.idr & 0xFFu;
        var k = 10u;
        if id == ID_NONE { k = select(1u, 0u, P.sky_enabled != 0u); }
        else if id == ID_CHASM || id == ID_CLIFF { k = 2u; }
        else if is_rail(id) { k = 3u; }
        else if id == ID_GROUND || id == ID_RISER { k = select(5u, 4u, P.verge_enabled == 0u || abs(g.x) < path_edge(g.d) || on_fork(g.x, g.d)); }
        else if id == ID_WALL_L || id == ID_WALL_R { k = 6u; }
        else if id == ID_CEILING { k = 7u; }
        else if id == ID_PROP { k = 8u; }
        else if id == ID_FIXTURE { k = 9u; }
        let c = rd3(P.r_last, i);
        let lu = 0.299 * c.x + 0.587 * c.y + 0.114 * c.z;
        n[k] += 1.0; l[k] += lu; total += lu;
    }
    let o = P.o_red + y * 23u;
    for (var k = 0u; k < 11u; k++) { scratch[o + k] = n[k]; scratch[o + 11u + k] = l[k]; }
    scratch[o + 22u] = total;
}

// ── post::upscale and looks::surface, to RGBA bytes ───────────────────────
@compute @workgroup_size(8, 8)
fn post_out(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= P.out_w || gid.y >= P.out_h { return; }
    let x = gid.x; let y = gid.y;
    let sy = min(y / P.px, P.h - 1u); let sx = min(x / P.px, P.w - 1u);
    let c = rd3(P.r_last, sy * P.w + sx);
    var b = vec3<u32>(u32(c.x), u32(c.y), u32(c.z));
    let paper = clamp(P.paper, 0.0, 1.0); let scan = clamp(P.scan, 0.0, 1.0);
    if paper >= 0.001 || scan >= 0.001 {
        let period = max(P.px, 2u);
        var f = 1.0;
        if scan > 0.0 && y % period == period - 1u { f = 1.0 - 0.55 * scan; }
        if paper > 0.0 {
            let fine = value_noise(f32(x) * 0.9, f32(y) * 0.35) - 0.5;
            let blot = value_noise(f32(x) * 0.035, f32(y) * 0.035) - 0.5;
            f *= 1.0 + (fine * 0.22 + blot * 0.16) * paper;
        }
        for (var k = 0; k < 3; k++) { b[k] = u32(clamp(f32(b[k]) * f, 0.0, 255.0)); }
    }
    outp[y * P.out_w + x] = b.x | (b.y << 8u) | (b.z << 16u) | (255u << 24u);
}
