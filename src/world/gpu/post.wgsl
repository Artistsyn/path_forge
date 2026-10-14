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
    th: f32, tw: f32, aspect: f32, o_q2: u32,
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
    t_cube: u32, t_drops: u32, lens_tphase: f32, fl_rays: u32,
    sh_t0: f32, sh_s1: f32, sh_s2: f32, sh_a1: f32, sh_a2: f32, sh_b1: f32, sh_b2: f32, sh_t1: f32, sh_t2: f32,
    // The lens flare (post::FlareK).
    fl_ghosts: u32, fl_amt: f32, ln_diag: i32,
    // Rain on the lens (render::lens::LensRain).
    ln_cell0: f32, ln_cell1: f32, ln_tt: f32, ln_cyc: u32,
    ln_fill: f32, ln_haze: f32, ln_blur: i32, ln_reach: f32,
    // The bloom threshold of the sky (post::sky_threshold).
    bl_sky: f32, bl_pad0: f32, bl_pad1: f32, bl_pad2: f32,
}

@group(0) @binding(1) var<uniform> P: Post;
@group(0) @binding(2) var<storage, read> fhdr: array<f32>;
@group(0) @binding(3) var<storage, read> fgbuf: array<GPix>;
@group(0) @binding(4) var<storage, read> fpick: array<u32>;
@group(0) @binding(5) var<storage, read_write> scratch: array<f32>;
// sRGB buckets (4096, one per word), sRGB starts (257 floats), then the palette LUT (bytes
// packed four to a word), palette colours (0xRRGGBB), the .cube. (The lens drops, 8 floats each,
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
    // Each pixel wavers with its own world's haze.
    let realm = (bitcast<u32>(scratch[P.o_pg + i * 5u + 4u]) >> 8u) & 0xFFu;
    var st = P.sh_strength; var c1 = P.sh_c1; var c2 = P.sh_c2; var tp = P.sh_t0;
    if realm == 1u { st = P.sh_s1; c1 = P.sh_a1; c2 = P.sh_b1; tp = P.sh_t1; }
    else if realm == 2u { st = P.sh_s2; c1 = P.sh_a2; c2 = P.sh_b2; tp = P.sh_t2; }
    let amp = st * 2.5 / P.unit * band * far;
    if amp >= 0.03 {
        let t = TAU * tp;
        let fx = f32(x) * P.unit; let fy = yf * P.unit;
        let dx = amp * (0.6 * sin(0.31 * fy + 0.05 * fx + c1 * t) + 0.4 * sin(0.77 * fy - 0.09 * fx - c2 * t + 1.3));
        let dy = amp * 0.3 * sin(0.21 * fx + 0.45 * fy + c2 * t);
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

fn lens_tap(x: u32, y: u32, dx: i32, dy: i32) -> vec3<f32> {
    return rd3(P.h_lsrc, u32(clamp(i32(y) + dy, 0, i32(P.h) - 1)) * P.w + u32(clamp(i32(x) + dx, 0, i32(P.w) - 1)));
}
// render::lens::bead: the drop that covers a pixel most, and where in the scene it looks.
struct LensHit { cov: f32, ox: f32, oy: f32, rim: f32 }
fn lens_bead(dx: f32, dy: f32, r: f32, alpha: f32, reach: f32, best: LensHit) -> LensHit {
    let q = dx * dx + dy * dy;
    if q >= 1.0 { return best; }
    let d = sqrt(q);
    let e = clamp(1.5 / r, 0.12, 0.9);
    let cov = alpha * (1.0 - smoothstep_r(1.0 - e, 1.0, d));
    if cov > best.cov {
        let k = (0.35 + 0.65 * q) * reach;
        return LensHit(cov, -dx * k, -dy * k, smoothstep_r(0.6, 1.0, d));
    }
    return best;
}

@compute @workgroup_size(8, 8)
fn post_lens(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= P.w || gid.y >= P.h { return; }
    let x = gid.x; let y = gid.y;
    let i = y * P.w + x;
    var c = rd3(P.h_lsrc, i);
    let w = f32(P.w); let h = f32(P.h);
    if P.lens_kind == 1u {
        // render::lens::rain_px.
        let px = f32(x) + 0.5; let py = f32(y) + 0.5;
        let wk = TAU / (0.22 * h);
        var best = LensHit(0.0, 0.0, 0.0, 0.0);
        var clear = 0.0;
        // Only the heavy drops whose tracks cross this pixel's strip of the glass (LensRain::strips).
        let strip_drops = u32(scratch[P.t_drops + P.n_drops * 8u + min(u32(px * 64.0 / w), 63u)]);
        for (var k = 0u; k < P.n_drops; k++) {
            if (strip_drops & (1u << k)) == 0u { continue; }
            let o = P.t_drops + k * 8u;
            let x0 = scratch[o]; let rad = scratch[o + 3u]; let wa = scratch[o + 5u];
            if abs(px - x0) > rad * 1.5 + wa { continue; }
            let y0 = scratch[o + 1u]; let yn = scratch[o + 2u];
            if py < y0 - rad * 1.5 || py > yn + rad { continue; }
            let alpha = scratch[o + 4u]; let wp = scratch[o + 6u]; let id = scratch[o + 7u];
            let dy = py - yn;
            best = lens_bead((px - (x0 + wa * sin(wk * yn + wp))) / rad, dy / select(0.95 * rad, 1.35 * rad, dy < 0.0), rad, alpha, P.ln_reach, best);
            if py < yn && py > y0 {
                let behind = yn - py;
                let fade = 1.0 - smoothstep_r(0.0, 0.45 * h, behind);
                let tw = rad * (0.7 - 0.3 * smoothstep_r(0.0, 10.0 * rad, behind));
                let cx = (px - (x0 + wa * sin(wk * py + wp))) / tw;
                let wet = (1.0 - smoothstep_r(0.5, 1.0, abs(cx))) * fade * alpha;
                clear = max(clear, wet);
                if 0.85 * wet > best.cov { best = LensHit(0.85 * wet, -cx * 0.35 * P.ln_reach, 0.0, 0.6 * smoothstep_r(0.4, 1.0, abs(cx))); }
                let s = 2.4 * rad;
                let bi = floor(div(py, s));
                let key = i32(id) * 7919 + i32(bi);
                let br = rad * (0.16 + 0.2 * hf(P.lens_seed ^ 0x4Au, key)) * fade;
                let by = (bi + 0.3 + 0.4 * hf(P.lens_seed ^ 0x4Bu, key)) * s;
                if br > 0.4 && by < yn - rad && by > y0 {
                    let bx = x0 + wa * sin(wk * by + wp) + (hf(P.lens_seed ^ 0x4Cu, key) - 0.5) * tw;
                    best = lens_bead((px - bx) / br, (py - by) / (br * 1.15), br, alpha, P.ln_reach * 0.6, best);
                }
            }
        }
        for (var layer = 0u; layer < 2u; layer++) {
            let cell = select(P.ln_cell1, P.ln_cell0, layer == 0u);
            let ci = i32(floor(div(px, cell))); let cj = i32(floor(div(py, cell)));
            let key0 = ci * 7919 + cj * 104729 + i32(layer) * 15485863;
            let tt = hf(P.lens_seed ^ 0x51u, key0) + P.ln_tt;
            let fl = floor(tt);
            let a = tt - fl;
            let key = key0 * 31 + (i32(fl) % i32(max(P.ln_cyc, 1u)));
            if hf(P.lens_seed ^ 0x52u, key) >= P.ln_fill * select(1.0, 0.5, layer == 0u) { continue; }
            let m = 0.14 + 0.26 * hf(P.lens_seed ^ 0x53u, key);
            let rb = cell * m * (0.6 + 0.4 * smoothstep_r(0.0, 0.5, a));
            let bcx = (f32(ci) + m + (1.0 - 2.0 * m) * hf(P.lens_seed ^ 0x54u, key)) * cell;
            let bcy = (f32(cj) + m + (1.0 - 2.0 * m) * hf(P.lens_seed ^ 0x55u, key)) * cell;
            let alpha = smoothstep_r(0.0, 0.08, a) * (1.0 - smoothstep_r(0.75, 1.0, a)) * (1.0 - clear) * P.lens_weight;
            best = lens_bead((px - bcx) / rb, (py - bcy) / (rb * (0.85 + 0.15 * hf(P.lens_seed ^ 0x56u, key))), rb, alpha, P.ln_reach * (0.35 + 1.6 * m), best);
        }
        let fx = f32(x); let fy = f32(y);
        let hz = P.ln_haze * (1.0 - clear);
        if hz > 0.0 {
            let r = P.ln_blur; let d = P.ln_diag;
            let ring = (lens_tap(x, y, -r, 0) + lens_tap(x, y, r, 0)) + (lens_tap(x, y, 0, -r) + lens_tap(x, y, 0, r));
            let diag = (lens_tap(x, y, -d, -d) + lens_tap(x, y, d, -d)) + (lens_tap(x, y, -d, d) + lens_tap(x, y, d, d));
            c = mix3(c, (ring + diag) * 0.125, hz);
        }
        if best.cov > 0.0 {
            let refr = bilinear(P.h_lsrc, fx + best.ox, fy + best.oy);
            c = mix3(c, refr * (1.08 * (1.0 - 0.35 * best.rim)), best.cov);
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
                let s = max(sin(TAU * P.tw * P.lens_tphase + ph), 0.0);
                let s2 = s * s; let s4 = s2 * s2;
                ice += frost * (1.5 * s4 * s4);
            }
            c = mix3(c, ice, f * 0.92 * P.lens_weight);
        }
    }
    wr3(P.h_ldst, i, c);
}

// ── bloom and flare (post::bloom_buffer, blur, flare_rays, flare_ghosts) ───
// post::bright_pass
fn bright_pass(l: f32, th: f32) -> f32 {
    let t = clamp((l - th) / 1.2, 0.0, 1.0);
    return t * sqrt(t);
}
@compute @workgroup_size(8, 8)
fn post_bloom_down(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= P.qw || gid.y >= P.qh { return; }
    var acc = vec3<f32>(0.0); var n = 0.0;
    for (var y = gid.y * 4u; y < min(gid.y * 4u + 4u, P.h); y++) {
        for (var x = gid.x * 4u; x < min(gid.x * 4u + 4u, P.w); x++) {
            let c = rd3(P.h_fin, y * P.w + x);
            var th = 0.8;
            if f32(y) < P.hz && pg_id(y * P.w + x) == ID_NONE { th = P.bl_sky; }
            acc += c * bright_pass(rnd((0.3 * c.x + 0.55 * c.y + 0.15 * c.z) * P.exposure), th); n += 1.0;
        }
    }
    wr3(P.o_q0, gid.y * P.qw + gid.x, acc / n);
}
const BK = array<f32, 5>(0.0625, 0.25, 0.375, 0.25, 0.0625);
// post::BLOOM_W and BLOOM_GAIN
const BLOOM_W = array<f32, 5>(0.30, 0.25, 0.20, 0.15, 0.10);
const BLOOM_GAIN: f32 = 1.5;
fn blur_at(src: u32, x: u32, y: u32, horizontal: bool, stride: i32) -> vec3<f32> {
    var acc = vec3<f32>(0.0);
    for (var t = 0; t < 5; t++) {
        let o = (t - 2) * stride;
        var sx = i32(x); var sy = i32(y);
        if horizontal { sx = clamp(sx + o, 0, i32(P.qw) - 1); } else { sy = clamp(sy + o, 0, i32(P.qh) - 1); }
        acc += rd3(src, u32(sy) * P.qw + u32(sx)) * BK[t];
    }
    return acc;
}
fn bloom_h(gid: vec3<u32>, k: u32) {
    if gid.x >= P.qw || gid.y >= P.qh { return; }
    wr3(P.o_q1, gid.y * P.qw + gid.x, blur_at(P.o_q0, gid.x, gid.y, true, i32(1u << k)));
}
// The second half of level k's blur, and its share added to the sum in q2.
fn bloom_v(gid: vec3<u32>, k: u32) {
    if gid.x >= P.qw || gid.y >= P.qh { return; }
    let i = gid.y * P.qw + gid.x;
    let c = blur_at(P.o_q1, gid.x, gid.y, false, i32(1u << k));
    wr3(P.o_q0, i, c);
    var sum = vec3<f32>(0.0);
    if k > 0u || P.fl_rays != 0u { sum = rd3(P.o_q2, i); }
    wr3(P.o_q2, i, sum + c * (BLOOM_W[k] * P.bloom * BLOOM_GAIN));
}
@compute @workgroup_size(8, 8) fn post_bh0(@builtin(global_invocation_id) gid: vec3<u32>) { bloom_h(gid, 0u); }
@compute @workgroup_size(8, 8) fn post_bv0(@builtin(global_invocation_id) gid: vec3<u32>) { bloom_v(gid, 0u); }
@compute @workgroup_size(8, 8) fn post_bh1(@builtin(global_invocation_id) gid: vec3<u32>) { bloom_h(gid, 1u); }
@compute @workgroup_size(8, 8) fn post_bv1(@builtin(global_invocation_id) gid: vec3<u32>) { bloom_v(gid, 1u); }
@compute @workgroup_size(8, 8) fn post_bh2(@builtin(global_invocation_id) gid: vec3<u32>) { bloom_h(gid, 2u); }
@compute @workgroup_size(8, 8) fn post_bv2(@builtin(global_invocation_id) gid: vec3<u32>) { bloom_v(gid, 2u); }
@compute @workgroup_size(8, 8) fn post_bh3(@builtin(global_invocation_id) gid: vec3<u32>) { bloom_h(gid, 3u); }
@compute @workgroup_size(8, 8) fn post_bv3(@builtin(global_invocation_id) gid: vec3<u32>) { bloom_v(gid, 3u); }
@compute @workgroup_size(8, 8) fn post_bh4(@builtin(global_invocation_id) gid: vec3<u32>) { bloom_h(gid, 4u); }
@compute @workgroup_size(8, 8) fn post_bv4(@builtin(global_invocation_id) gid: vec3<u32>) { bloom_v(gid, 4u); }

// post::q_at on q0.
fn q_at(fx: f32, fy: f32) -> vec3<f32> {
    if fx < 0.0 || fy < 0.0 || fx > f32(P.qw) - 1.0 || fy > f32(P.qh) - 1.0 { return vec3<f32>(0.0); }
    let x0 = u32(fx); let y0 = u32(fy);
    let x1 = min(x0 + 1u, P.qw - 1u); let y1 = min(y0 + 1u, P.qh - 1u);
    let tx = fx - f32(x0); let ty = fy - f32(y0);
    let a = rd3(P.o_q0, y0 * P.qw + x0) * (1.0 - tx) + rd3(P.o_q0, y0 * P.qw + x1) * tx;
    let b = rd3(P.o_q0, y1 * P.qw + x0) * (1.0 - tx) + rd3(P.o_q0, y1 * P.qw + x1) * tx;
    return a * (1.0 - ty) + b * ty;
}
// post::flare_lines: (direction, reach), then (strength, colour).
fn flare_line(rays: u32, k: u32) -> vec4<f32> {
    if rays == 4u { if k == 0u { return vec4<f32>(0.9396926, -0.3420201, 0.07, 0.5); } return vec4<f32>(0.3420201, 0.9396926, 0.07, 0.5); }
    if rays == 6u {
        if k == 0u { return vec4<f32>(0.9659258, -0.258819, 0.06, 0.4); }
        if k == 1u { return vec4<f32>(0.258819, -0.9659258, 0.06, 0.4); }
        return vec4<f32>(-0.7071068, -0.7071068, 0.06, 0.4);
    }
    return vec4<f32>(1.0, 0.0, 0.3, 1.0);
}
// post::flare_rays: starts the sum, from the bright parts before any blur.
@compute @workgroup_size(8, 8)
fn post_rays(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= P.qw || gid.y >= P.qh { return; }
    let step = 1.5;
    var lines = 1u;
    if P.fl_rays == 4u { lines = 2u; } else if P.fl_rays == 6u { lines = 3u; }
    var tint = vec3<f32>(1.0, 0.96, 0.9);
    if P.fl_rays == 1u { tint = vec3<f32>(0.45, 0.65, 1.0); }
    var out = vec3<f32>(0.0);
    for (var li = 0u; li < lines; li++) {
        let ln = flare_line(P.fl_rays, li);
        var span = f32(P.qh);
        if P.fl_rays == 1u { span = f32(P.qw); }
        let len = max(ln.z * span, 2.0);
        let n = i32(ceil(3.0 * len / step));
        var acc = vec3<f32>(0.0);
        for (var t = 1; t <= n; t++) {
            let s = f32(t) * step;
            let wt = exp(-s / len);
            acc += (q_at(f32(gid.x) - s * ln.x, f32(gid.y) - s * ln.y) + q_at(f32(gid.x) + s * ln.x, f32(gid.y) + s * ln.y)) * wt;
        }
        out += acc * (ln.w * P.fl_amt * step / len) * tint;
    }
    wr3(P.o_q2, gid.y * P.qw + gid.x, out);
}
// post::GHOSTS: (scale, strength), then its colour.
fn ghost(k: u32) -> vec2<f32> {
    if k == 0u { return vec2<f32>(1.0, 0.30); }
    if k == 1u { return vec2<f32>(0.55, 0.22); }
    if k == 2u { return vec2<f32>(1.7, 0.16); }
    if k == 3u { return vec2<f32>(0.25, 0.20); }
    return vec2<f32>(-0.4, 0.14);
}
fn ghost_tint(k: u32) -> vec3<f32> {
    if k == 0u { return vec3<f32>(1.0, 0.8, 0.6); }
    if k == 1u { return vec3<f32>(0.6, 1.0, 0.7); }
    if k == 2u { return vec3<f32>(0.6, 0.7, 1.0); }
    if k == 3u { return vec3<f32>(1.0, 0.6, 0.9); }
    return vec3<f32>(0.8, 0.9, 1.0);
}
// post::flare_ghosts, added to the sum.
@compute @workgroup_size(8, 8)
fn post_ghosts(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= P.qw || gid.y >= P.qh { return; }
    let cx = 0.5 * (f32(P.qw) - 1.0); let cy = 0.5 * (f32(P.qh) - 1.0);
    let dx = cx - f32(gid.x); let dy = cy - f32(gid.y);
    let half = max(sqrt(cx * cx + cy * cy), 1.0);
    var out = vec3<f32>(0.0);
    for (var k = 0u; k < 5u; k++) {
        let g = ghost(k);
        let px = cx + dx * g.x; let py = cy + dy * g.x;
        let r = sqrt((px - cx) * (px - cx) + (py - cy) * (py - cy)) / half;
        let fade = 1.0 - clamp((r - 0.55) / 0.4, 0.0, 1.0);
        if fade <= 0.0 { continue; }
        out += q_at(px, py) * (g.y * fade * P.fl_amt) * ghost_tint(k);
    }
    let d = sqrt(dx * dx + dy * dy);
    if d > 1.0 {
        let k = 0.42 * f32(P.qh) / d;
        out += q_at(f32(gid.x) + dx * k, f32(gid.y) + dy * k) * (0.12 * P.fl_amt) * vec3<f32>(0.7, 0.8, 1.0);
    }
    let i = gid.y * P.qw + gid.x;
    wr3(P.o_q2, i, rd3(P.o_q2, i) + out);
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
    if P.bloom > 0.0 || P.fl_rays != 0u || P.fl_ghosts != 0u {
        let fx = clamp((f32(x) + 0.5) / 4.0 - 0.5, 0.0, f32(P.qw) - 1.0);
        let fy = clamp((f32(y) + 0.5) / 4.0 - 0.5, 0.0, f32(P.qh) - 1.0);
        let x0 = u32(fx); let y0 = u32(fy);
        let x1 = min(x0 + 1u, P.qw - 1u); let y1 = min(y0 + 1u, P.qh - 1u);
        let tx = fx - f32(x0); let ty = fy - f32(y0);
        let a = rd3(P.o_q2, y0 * P.qw + x0) * (1.0 - tx) + rd3(P.o_q2, y0 * P.qw + x1) * tx;
        let b = rd3(P.o_q2, y1 * P.qw + x0) * (1.0 - tx) + rd3(P.o_q2, y1 * P.qw + x1) * tx;
        c += a * (1.0 - ty) + b * ty;
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
