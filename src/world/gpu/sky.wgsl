// The sky over every pixel where nothing is drawn, in the CPU's order: stars, aurora, sun, moon,
// clouds, rainbow (render::draw_sky_bodies), lightning (draw_lightning), the veil of a heavy fall
// (weather::veil_sky), and the fog banks in front. Each pixel applies the same steps in the same
// order the CPU applies them to it; stars and cloud blobs come in per-tile lists kept in order.

struct Sky {
    craters: array<vec4<f32>, 22>,
    hy: f32, ox: f32, oy: f32, fw: f32,
    fhy: f32, tile_cols: u32, pad0: u32, pad1: u32,
    stars_on: u32, star_bins: u32, star_prims: u32, md_on: u32,
    md_x: f32, md_y: f32, md_r: f32,
    au_on: u32, au_a: f32, au_m: f32, au_cx: f32,
    au_unit: f32, au_lo_r: f32, au_lo_g: f32, au_lo_b: f32, au_hi_r: f32, au_hi_g: f32, au_hi_b: f32,
    au_acc_r: f32, au_acc_g: f32, au_acc_b: f32,
    au_foot: f32, au_arc: f32, au_tall: f32, au_rays: f32, au_waves: f32,
    au_pres_lo: f32, au_pres_hi: f32, au_edge: f32, au_k: f32, au_so: f32,
    sun_on: u32, s_x: f32, s_y: f32, s_r: f32, s_c_r: f32, s_c_g: f32, s_c_b: f32, s_spread: f32, s_focal: f32,
    moon_on: u32, m_c_r: f32, m_c_g: f32, m_c_b: f32, m_gr: f32, m_sx: f32, m_sy: f32, m_litf: f32, m_op: f32, m_craters: u32,
    cl_on: u32, cl_r: f32, cl_g: f32, cl_b: f32, cl_sun: u32, cl_sx: f32, cl_sy: f32, cl_sc_r: f32, cl_sc_g: f32, cl_sc_b: f32,
    cl_op: f32, cl_bins: u32, cl_prims: u32,
    rb_on: u32, rb_cx: f32, rb_cy: f32, rb_rad: f32, rb_band: f32, rb_rad2: f32, rb_band2: f32, rb_k: f32, rb_double: u32,
    fl_on: u32, fl_r: f32, fl_g: f32, fl_b: f32,
    bolt_on: u32, b_gain: f32, b_core_r: f32, b_core_g: f32, b_core_b: f32, b_tint_r: f32, b_tint_g: f32, b_tint_b: f32,
    b_glow: f32, b_scale: f32, b_xa: u32, b_xb: u32, b_yb: u32, b_nsegs: u32, b_prims: u32,
    veil: f32, bank_on: u32, bank_t: f32, bank_r: f32, bank_g: f32, bank_b: f32,
    sp_on: u32, sp_below: u32, sp_scale: f32, sp_stars: f32, sp_sb: f32, sp_cx: f32, sp_unit: f32, sp_top: f32,
    sp_st_r: f32, sp_st_g: f32, sp_st_b: f32, sp_sh_r: f32, sp_sh_g: f32, sp_sh_b: f32,
    sp_neb: f32, sp_n1_r: f32, sp_n1_g: f32, sp_n1_b: f32, sp_n2_r: f32, sp_n2_g: f32, sp_n2_b: f32, sp_neb_f: f32,
    sp_gal: f32, sp_gc_r: f32, sp_gc_g: f32, sp_gc_b: f32, sp_gal_c: f32, sp_gal_s: f32, sp_gal_w: f32, sp_gal_h: f32, sp_gal_core: f32,
    sp_so: f32, pl_n: u32, pl_prims: u32, s_air: f32, bh_on: u32,
    tn_on: u32, tn_kind: f32, tn_vx: f32, tn_vy: f32, tn_focal: f32, tn_radius: f32, tn_travel: f32,
    tn_spin: f32, tn_twist: f32, tn_loop: f32, tn_lanes: f32, tn_period: f32, tn_nx: f32, tn_ny: f32,
    tn_c0_r: f32, tn_c0_g: f32, tn_c0_b: f32, tn_c1_r: f32, tn_c1_g: f32, tn_c1_b: f32, tn_c2_r: f32, tn_c2_g: f32, tn_c2_b: f32,
    tn_k: f32, tn_core: f32, tn_scale: f32, tn_fade: f32, bh_prims: u32,
    // 0: one world (adds to the sky shading left in hdr); 1, 2: one of several worlds, its sky
    // weighted by SkyW::at and seen as sky_seen, written (1, the first) or added (2).
    mode: u32, w_fork: u32, w_base: f32,
    w_center: f32, w_focal: f32, w_split: f32, w_near: f32, w_keep0: f32, w_keep1: f32, sp_star_var: f32, pad6: u32,
}

@group(0) @binding(1) var<uniform> S: Sky;
@group(0) @binding(2) var<storage, read> gbuf: array<GPix>;
@group(0) @binding(3) var<storage, read_write> hdr: array<f32>;
// Stars (x, y, r, ri, b, 0), cloud blobs (x, y, rx, ry, life, 0), bolt segments (ax, ay, bx, by, weight, 0).
@group(0) @binding(4) var<storage, read> prims: array<f32>;
// Per-tile lists: for stars, then for blobs, (tiles + 1) offsets followed by indices.
@group(0) @binding(5) var<storage, read> bins: array<u32>;

fn prim(base: u32, k: u32, f: u32) -> f32 { return prims[base + k * 6u + f]; }

// SkyW::at for this world (W.realm) at column x.
fn sky_weight(x: u32) -> f32 {
    if S.w_fork == 0u { return S.w_base; }
    let a = (f32(x) + 0.5 - S.w_center) / S.w_focal;
    let wl = 1.0 - smoothstep_r(S.w_split - 0.07, S.w_split + 0.07, a);
    let l = wl * S.w_keep0; let rr = (1.0 - wl) * S.w_keep1;
    let tot = l + rr;
    var left: f32;
    if tot > 1e-4 { left = l / tot; } else { left = select(0.0, 1.0, S.w_keep0 >= S.w_keep1); }
    if W.realm == 0u { return 1.0 - S.w_near; }
    if W.realm == 1u { return S.w_near * left; }
    return S.w_near * (1.0 - left);
}

// render::moon_albedo
fn moon_albedo(n: vec3<f32>, px: f32) -> f32 {
    let x = n.x; let y = n.y; let z = n.z;
    let m = 0.55 * value_noise(x * 2.2 + 3.1, y * 2.2 + 7.4) + 0.3 * value_noise(x * 4.7 + 1.3, y * 4.7 + 5.2) + 0.15 * value_noise(x * 9.0 + 4.0, y * 9.0);
    let soft = max(2.0 * px, 0.04);
    let maria = smoothstep_r(0.55 - soft, 0.6 + soft, m - 0.1 * y - 0.06 * x);
    var a = 1.0 - 0.42 * maria;
    for (var k = 0; k < 22; k++) {
        let c = S.craters[k];
        if c.w <= 0.0 { continue; }
        let d = sqrt((x - c.x) * (x - c.x) + (y - c.y) * (y - c.y) + (z - c.z) * (z - c.z)) / c.w;
        if d > 1.4 { continue; }
        let e = max(px / c.w, 0.06);
        let fl = 1.0 - smoothstep_r(0.78 - e, 0.82 + e, d);
        let rim = smoothstep_r(0.72 - e, 0.92, d) * (1.0 - smoothstep_r(1.0, 1.2 + e, d));
        a *= 1.0 - 0.2 * fl + 0.14 * rim;
    }
    let grain = (1.0 - smoothstep_r(0.03, 0.08, px)) * (value_noise(x * 24.0 + 9.0, y * 24.0) - 0.5);
    return a * (1.0 + 0.16 * grain);
}
fn spectrum(t0: f32) -> vec3<f32> {
    let t = clamp(t0, 0.0, 1.0);
    return vec3<f32>(smoothstep_r(0.45, 0.85, t) + 0.35 * (1.0 - smoothstep_r(0.0, 0.15, t)), smoothstep_r(0.2, 0.5, t) * (1.0 - smoothstep_r(0.7, 0.9, t)), 1.0 - smoothstep_r(0.25, 0.55, t));
}

// weather::AuroraK, aurora_col and aurora_px, step for step.
fn au_l(r: f32, ph: f32) -> vec2<f32> { let q = S.au_a + ph; return S.au_m * r * vec2<f32>(cos(q), sin(q)); }
fn au_n(x: f32, y: f32, r: f32, ph: f32) -> f32 { let l = au_l(r, ph); return value_noise(x + l.x, y + l.y); }
fn au_fbm(x0: f32, y0: f32, r: f32, ph: f32) -> f32 {
    let l = au_l(r, ph);
    var x = x0 + l.x; var y = y0 + l.y; var amp = 0.5; var s = 0.0;
    for (var i = 0; i < 4; i++) {
        s += amp * value_noise(x, y);
        x = x * 2.03 + 7.1; y = y * 2.03 + 7.1; amp *= 0.5;
    }
    return s;
}
fn au_base_wave(x: f32) -> f32 {
    let xs = x * 1.8 + 9.0;
    let sheet = 0.6 * au_n(xs * 0.9, 0.0, 0.5, 0.0) + 0.4 * au_n(xs * 2.3 + 5.0, 0.0, 0.6, 1.7);
    return S.au_waves * (0.22 * (sheet - 0.5) + 0.06 * (au_n(x * 5.0, 0.0, 0.6, 3.0) - 0.5));
}
struct AuCol { fx: f32, base: f32, pres: f32, edge: f32, wid: f32, veil: f32, knot: f32 }
fn aurora_col(x: f32) -> AuCol {
    let fx = x + 0.35 * S.au_m * sin(S.au_a) + 5.7 + S.au_so;
    let bw = au_base_wave(fx);
    let pres = smoothstep_r(S.au_pres_lo, S.au_pres_hi, au_fbm(fx * 1.2 + 3.0, 0.0, 0.9, 0.0));
    let slope = (au_base_wave(fx + 0.012) - bw) / 0.012;
    let along = 0.35 + 0.9 * smoothstep_r(0.25, 0.8, au_fbm(fx * 1.6, 7.0, 1.0, 0.0));
    return AuCol(
        fx,
        S.au_foot - S.au_arc * 0.05 * x * x - bw,
        pres,
        along * (0.15 + 0.85 * pres) * (1.0 + 0.9 * min(abs(slope), 1.5)),
        0.008 + 0.016 * au_n(fx * 2.4, 5.0, 0.9, 1.0),
        S.au_tall * (0.35 + 0.25 * au_fbm(fx * 1.1, 3.0, 0.7, 1.5)),
        smoothstep_r(0.62, 0.82, au_fbm(fx * 2.2, 11.0, 1.1, 4.0)),
    );
}
fn aurora_px(c: AuCol, x: f32, y: f32) -> vec3<f32> {
    let hb = y - c.base;
    if hb < -0.01 { return vec3<f32>(0.0); }
    let hp = max(hb, 0.0);
    let hn = hp / c.veil;
    if hn > 1.6 { return vec3<f32>(0.0); }
    let below = smoothstep_r(-0.004, 0.002, hb);
    let on = smoothstep_r(-0.02, 0.02, hb);
    let fxr = c.fx - hp * x * 0.25;
    let arc = exp(-hp / (c.wid * 1.3)) * c.edge;
    let sh = exp(-hp / 0.045);
    let ray_s = au_n(fxr * 48.0, 0.0, 1.3, 0.0);
    let ray_m = au_n(fxr * 14.0, hn * 2.0, 1.0, 2.0);
    let rays = max(1.0 + S.au_rays * (0.9 * pow(max(ray_s, 1e-6), 1.6) * (0.5 + 0.5 * ray_m) - 0.45), 0.0);
    let life = smoothstep_r(0.25, 0.75, au_n(fxr * 9.0, hn * 3.0, 1.5, 5.0));
    let green = exp(-hn * 3.2) * rays * (0.35 + 0.65 * c.pres) * (0.5 + 0.5 * life);
    let pv = smoothstep_r(0.3, 0.7, au_fbm(c.fx * 0.9 + 17.0, hn * 1.4, 0.8, 2.0));
    let violet = smoothstep_r(0.08, 0.5, hn) * (1.0 - smoothstep_r(0.7, 1.4, hn)) * pv * (0.5 + 0.5 * rays) * (0.4 + 0.6 * c.pres);
    let n3 = au_n(fxr * 90.0, 0.0, 1.4, 4.0);
    let n6 = n3 * n3 * n3;
    let spike = n6 * n6 * 3.0 * exp(-hp / 0.09) * on * c.pres * min(S.au_rays, 1.0);
    let knot = c.knot * exp(-hp / 0.07) * on;
    let lo = vec3<f32>(S.au_lo_r, S.au_lo_g, S.au_lo_b);
    let hi = vec3<f32>(S.au_hi_r, S.au_hi_g, S.au_hi_b);
    let acc = vec3<f32>(S.au_acc_r, S.au_acc_g, S.au_acc_b);
    let white = vec3<f32>(1.0);
    var col = mix3(lo, white, 0.45) * (arc * 1.5 * S.au_edge);
    col += lo * (sh * 0.55 * (0.15 + 0.85 * c.pres) + green * 0.9);
    col += hi * (violet * 0.9);
    col += mix3(hi, acc, 0.5) * (violet * 0.35 * life);
    col += mix3(lo, white, 0.75) * (spike * 0.5);
    col += acc * (knot * 0.6);
    return col * (S.au_k * below);
}

// render/space.rs, step for step.
fn cell_hash(x: i32, y: i32, k: u32) -> f32 {
    var h = (bitcast<u32>(x) * 0x8da6b343u) ^ (bitcast<u32>(y) * 0xd8163841u) ^ (k * 0x9e3779b9u);
    h ^= h >> 15u; h = h * 0x2c1b3c6du; h ^= h >> 12u;
    return f32(h & 0xffffu) / 65535.0;
}
fn fbm4(x0: f32, y0: f32) -> f32 {
    var x = x0; var y = y0; var amp = 0.5; var s = 0.0;
    for (var i = 0; i < 4; i++) {
        s += amp * value_noise(x, y);
        x = x * 2.03 + 7.1; y = y * 2.03 + 7.1; amp *= 0.5;
    }
    return s;
}
fn noise_wrap(x: f32, y: f32, n: i32) -> f32 {
    let xf = floor(x); let yf = floor(y);
    let fx = x - xf; let fy = y - yf;
    let sx = fx * fx * (3.0 - 2.0 * fx); let sy = fy * fy * (3.0 - 2.0 * fy);
    let x0 = rem_ei(i32(xf), n); let x1 = rem_ei(x0 + 1, n); let yi = i32(yf);
    let a = cell_hash(x0, yi, 0x51u) + (cell_hash(x1, yi, 0x51u) - cell_hash(x0, yi, 0x51u)) * sx;
    let b = cell_hash(x0, yi + 1, 0x51u) + (cell_hash(x1, yi + 1, 0x51u) - cell_hash(x0, yi + 1, 0x51u)) * sx;
    return a + (b - a) * sy;
}
fn fbm_wrap(u: f32, v: f32, n0: i32, vs: f32) -> f32 {
    var n = n0; var y = v * vs; var amp = 0.5; var s = 0.0;
    for (var k = 0; k < 4; k++) {
        s += amp * noise_wrap(u * f32(n), y + f32(k) * 7.1, n);
        n *= 2; y *= 2.0; amp *= 0.5;
    }
    return s;
}
fn star_layer(px: f32, py: f32, cell: f32, prob: f32, sigma0: f32, gain0: f32, salt: u32, cvar: f32) -> vec3<f32> {
    let sigma = max(sigma0, 0.5);
    let gain = gain0 * (sigma0 / sigma) * (sigma0 / sigma);
    let ix = i32(floor(div(px, cell))); let iy = i32(floor(div(py, cell)));
    if cell_hash(ix, iy, salt) >= prob { return vec3<f32>(0.0); }
    let m = min(2.5 * sigma / cell, 0.45);
    let sx = (f32(ix) + m + (1.0 - 2.0 * m) * cell_hash(ix, iy, salt + 1u)) * cell;
    let sy = (f32(iy) + m + (1.0 - 2.0 * m) * cell_hash(ix, iy, salt + 2u)) * cell;
    let d2 = (px - sx) * (px - sx) + (py - sy) * (py - sy);
    let b = cell_hash(ix, iy, salt + 3u);
    let k = gain * (0.25 + b * b) * exp(-d2 / (2.0 * sigma * sigma));
    let col = mix3(vec3<f32>(0.72, 0.84, 1.0), vec3<f32>(1.0, 0.82, 0.6), cell_hash(ix, iy, salt + 4u));
    if cvar < 1.0 { return mix3(vec3<f32>(0.86, 0.88, 1.0), col, cvar) * k; }
    return col * k;
}
fn space_backdrop(x: u32, y: u32, c0: vec3<f32>) -> vec3<f32> {
    let fy = f32(y);
    let above = fy < S.hy;
    if !above && S.sp_below == 0u { return c0; }
    var c = c0;
    if !above {
        let t = pow(clamp((2.0 * S.hy - fy - S.sp_top) / max(S.hy - S.sp_top, 1.0), 0.0, 1.0), 1.3);
        c = mix3(vec3<f32>(S.sp_st_r, S.sp_st_g, S.sp_st_b), vec3<f32>(S.sp_sh_r, S.sp_sh_g, S.sp_sh_b), t);
    }
    if S.bh_on != 0u { return bh_lensed(x, y, c); }
    return space_features(f32(x) + 0.5, fy + 0.5, c);
}
fn space_features(px: f32, py: f32, c0: vec3<f32>) -> vec3<f32> {
    var c = c0;
    let sx = (px - S.sp_cx) / S.sp_unit; let sy = (S.hy - py) / S.sp_unit;
    var dark = 0.0;
    if S.sp_neb > 0.0 {
        let nx = sx * S.sp_neb_f + S.sp_so; let ny = sy * S.sp_neb_f;
        let n1 = fbm4(nx, ny);
        let n2 = fbm4(nx * 1.7 + 13.1, ny * 1.7 + 5.3);
        let d = smoothstep_r(0.42, 0.8, n1);
        let f1 = 1.0 - abs(2.0 * value_noise(nx * 5.0, ny * 5.0) - 1.0);
        let fil = f1 * f1 * f1 * f1 * smoothstep_r(0.35, 0.6, n1);
        let col = mix3(vec3<f32>(S.sp_n1_r, S.sp_n1_g, S.sp_n1_b), vec3<f32>(S.sp_n2_r, S.sp_n2_g, S.sp_n2_b), smoothstep_r(0.3, 0.7, n2));
        c += col * (S.sp_neb * (0.16 * d + 0.1 * fil));
        dark = smoothstep_r(0.55, 0.75, fbm4(nx * 2.0 + 31.0, ny * 2.0)) * 0.6 * min(S.sp_neb, 1.0);
    }
    var band = 0.0;
    if S.sp_gal > 0.0 {
        let dy = sy - S.sp_gal_h;
        let along = sx * S.sp_gal_c + dy * S.sp_gal_s;
        let across0 = -sx * S.sp_gal_s + dy * S.sp_gal_c;
        let across = across0 + 0.25 * S.sp_gal_w * (value_noise(along * 1.3 + S.sp_so, 3.7) - 0.5);
        let q = across / S.sp_gal_w;
        if abs(q) < 2.6 {
            band = exp(-q * q);
            let ca = (along - S.sp_gal_core) / 0.22;
            let cq = across / (1.1 * S.sp_gal_w);
            let core = exp(-ca * ca - cq * cq);
            let lq = across / (0.45 * S.sp_gal_w);
            let dust = smoothstep_r(0.45, 0.75, fbm4(along * 3.0 + S.sp_so, q * 1.2 + 11.0)) * exp(-lq * lq);
            let glow = 0.1 * band * (0.6 + 0.8 * fbm4(along * 4.0 + 3.0, q * 2.0)) + 0.22 * core;
            c += vec3<f32>(S.sp_gc_r, S.sp_gc_g, S.sp_gc_b) * (S.sp_gal * glow * (1.0 - 0.85 * dust));
            dark = max(dark, 0.8 * dust);
        }
    }
    if S.sp_stars > 0.0 {
        let dens = S.sp_stars * (1.0 + 2.5 * min(S.sp_gal, 1.0) * band) * (1.0 - dark);
        let k = S.sp_scale;
        var st = star_layer(px, py, 4.5 * k, min(0.3 * dens, 0.95), 0.45 * k, 0.5, 0xA0u, S.sp_star_var);
        st += star_layer(px, py, 11.0 * k, min(0.35 * dens, 0.95), 0.6 * k, 1.0, 0xB0u, S.sp_star_var);
        st += star_layer(px, py, 31.0 * k, min(0.3 * S.sp_stars, 0.95), 0.85 * k, 2.2, 0xC0u, S.sp_star_var);
        c += st * (S.sp_sb * (1.0 - 0.7 * dark));
    }
    return c;
}

// render/blackhole.rs, step for step, from the block BhK::pack writes at S.bh_prims: the header,
// the orbit table (each outside row's bending, then every row's u = 1/r), then the streaks.
const BH_BC: f32 = 2.5980762;
const BH_BMAX: f32 = 40.0;
const BH_NI: u32 = 48u;
const BH_NO: u32 = 112u;
const BH_NP: u32 = 192u;
const BH_BANDS: u32 = 16u;
const BH_ALPHA: u32 = 64u;
const BH_U: u32 = 176u;
const BH_DEBRIS: u32 = 30896u;
const BH_STRIDE: u32 = 16u;
const BH_FLUX_MAX: f32 = 0.0567103;
fn bh(k: u32) -> f32 { return prims[S.bh_prims + k]; }
fn bh3(k: u32) -> vec3<f32> { return vec3<f32>(bh(k), bh(k + 1u), bh(k + 2u)); }
// The fractional table row for impact parameter b: (first row, weight).
fn bh_row(b: f32) -> vec2<f32> {
    if b < BH_BC {
        let f = clamp(div(b, BH_BC) * f32(BH_NI) - 0.5, 0.0, f32(BH_NI - 1u));
        let i0 = min(u32(floor(f)), BH_NI - 2u);
        return vec2<f32>(f32(i0), clamp(f - f32(i0), 0.0, 1.0));
    }
    let g = clamp(sqrt(div(b - BH_BC, BH_BMAX - BH_BC)) * f32(BH_NO) - 1.0, 0.0, f32(BH_NO - 1u));
    let j0 = min(u32(floor(g)), BH_NO - 2u);
    return vec2<f32>(f32(BH_NI + j0), clamp(g - f32(j0), 0.0, 1.0));
}
fn bh_alpha(b: f32) -> f32 {
    if b < BH_BC { return -1.0; }
    if b >= BH_BMAX {
        let ib = 1.0 / b;
        return ib * (2.0 + ib * (2.9452431 + ib * 5.3333335));
    }
    let rw = bh_row(b);
    let r0 = u32(rw.x) - BH_NI;
    let a0 = bh(BH_ALPHA + r0);
    return a0 + (bh(BH_ALPHA + r0 + 1u) - a0) * rw.y;
}
fn bh_u(b: f32, d: f32) -> f32 {
    let rw = bh_row(b);
    let r0 = u32(rw.x);
    let q = div(d, 3.0 * PI / f32(BH_NP - 1u));
    let j0 = min(u32(max(floor(q), 0.0)), BH_NP - 2u);
    let s = clamp(q - f32(j0), 0.0, 1.0);
    let k0 = BH_U + r0 * BH_NP + j0;
    let a = bh(k0); let b2 = bh(k0 + 1u); let c = bh(k0 + BH_NP); let e = bh(k0 + BH_NP + 1u);
    if a < 0.0 || b2 < 0.0 || c < 0.0 || e < 0.0 { return -1.0; }
    if a > 1.5 || b2 > 1.5 || c > 1.5 || e > 1.5 { return 2.0; }
    let u0 = a + (b2 - a) * s;
    let u1 = c + (e - c) * s;
    return u0 + (u1 - u0) * rw.y;
}
fn bh_band(j: u32, u0: f32, r: f32) -> f32 {
    let u1 = u0 - bh(30u + j);
    let u = u1 - floor(u1);
    return 0.55 * fbm_wrap(u, r * 0.55, 6, 1.0) + 0.45 * fbm_wrap(u, r * 3.0, 20, 1.0);
}
// The disk's light (rgb) and opacity (w) where a ray crosses it.
fn bh_disk(r: f32, phi: f32, g: f32) -> vec4<f32> {
    let r_in = bh(4u); let r_out = bh(5u);
    let x = r_in / r;
    let f = x * x * x * (1.0 - sqrt(x)) / BH_FLUX_MAX;
    let fade = smoothstep_r(r_in, r_in * 1.15, r) * (1.0 - smoothstep_r(r_in + 0.6 * (r_out - r_in), r_out, r));
    if fade <= 0.0 { return vec4<f32>(0.0); }
    let u0 = phi / (2.0 * PI) + 0.5;
    let bf = div(r - r_in, r_out - r_in) * f32(BH_BANDS) - 0.5;
    let j0 = min(u32(max(floor(bf), 0.0)), BH_BANDS - 1u);
    let j1 = min(j0 + 1u, BH_BANDS - 1u);
    let w = smoothstep_r(0.0, 1.0, bf - f32(j0));
    let n0 = bh_band(j0, u0, r);
    let nz = n0 + (bh_band(j1, u0, r) - n0) * w;
    let t = pow(max(f, 0.0), 0.25) * g;
    let col = mix3(mix3(bh3(25u), bh3(22u), smoothstep_r(0.25, 0.65, t)), bh3(19u), smoothstep_r(0.6, 1.0, t));
    let i = bh(9u) * f * g * g * g * (0.35 + 1.3 * nz) * fade;
    return vec4<f32>(col * i, clamp(fade * (0.35 + 0.7 * nz), 0.0, 0.92));
}
fn bh_lensed(x: u32, y: u32, c: vec3<f32>) -> vec3<f32> {
    let px = f32(x) + 0.5; let py = f32(y) + 0.5;
    let ox = px - bh(0u); let oy = bh(1u) - py;
    let d = max(sqrt(ox * ox + oy * oy), 1e-3);
    let b = d / bh(2u);
    let ux = ox / d; let uy = oy / d;
    var out = vec3<f32>(0.0);
    var tr = 1.0;
    if d < bh(6u) {
        let n = bh3(10u); let d1 = bh3(13u); let d2 = bh3(16u);
        let r_in = bh(4u); let r_out = bh(5u);
        let a = dot(n, vec3<f32>(ux, uy, 0.0));
        let d0 = atan(-a / n.z) + 0.5 * PI;
        let alpha = bh_alpha(b);
        var peri = 1e9;
        if alpha >= 0.0 { peri = 0.5 * (PI + alpha); }
        let ib2 = 1.0 / (b * b);
        for (var kk = 0; kk < 3; kk++) {
            let dl = d0 + f32(kk) * PI;
            if dl > 3.0 * PI { break; }
            let u = bh_u(b, dl);
            if u == -1.0 || u == 2.0 { break; }
            if !(u > 0.0 && u < 1.0) { continue; }
            let r = 1.0 / u;
            if r < r_in || r > r_out { continue; }
            let psi = dl - 0.5 * PI;
            let cs = cos(psi); let sn = sin(psi);
            let rh = vec3<f32>(cs * ux, cs * uy, sn);
            let ph = vec3<f32>(-sn * ux, -sn * uy, cs);
            let p = rh * r;
            let phi = atan2(dot(p, d2), dot(p, d1));
            let up = sqrt(max(ib2 - u * u + u * u * u, 0.0)) * select(-1.0, 1.0, dl < peri);
            let drp = -up / (u * u);
            let tv = rh * drp + ph * r;
            let kv = -tv / max(sqrt(dot(tv, tv)), 1e-6);
            let vv = cross(n, rh);
            let beta = min(sqrt(0.5 / (r - 1.0)), 0.95);
            let cos_t = bh(7u) * dot(vv, kv);
            let g0 = sqrt(1.0 - 1.0 / r) * sqrt(1.0 - beta * beta) / (1.0 - beta * cos_t);
            let g = 1.0 + bh(8u) * (g0 - 1.0);
            let e = bh_disk(r, phi, g);
            out += e.xyz * tr;
            tr *= 1.0 - e.w;
        }
    }
    if b >= BH_BC {
        let shift = bh(3u) * bh_alpha(b);
        out += space_features(px - ux * shift, py + uy * shift, c) * tr;
        let q = (d - bh(48u) * 1.018) / bh(29u);
        if abs(q) < 4.0 {
            let side = 0.5 + 0.5 * (ux * bh(46u) + uy * bh(47u));
            let gain = 1.0 + bh(8u) * (0.15 + 1.6 * side - 1.0);
            out += bh3(19u) * (0.45 * bh(9u) * gain * exp(-q * q) * tr);
        }
    }
    return out;
}
fn bh_dim(v: vec3<f32>) -> vec3<f32> {
    let l = 0.3 * v.x + 0.59 * v.y + 0.11 * v.z;
    return mix3(v, vec3<f32>(0.62 * l, 0.66 * l, 0.8 * l), 0.3) * 0.75;
}
fn bh_redden(v: vec3<f32>, r: f32) -> vec3<f32> { return vec3<f32>(v.x * (1.0 + 0.6 * r), v.y * (1.0 - 0.7 * r), v.z * (1.0 - 0.8 * r)); }
fn bh_node(o: u32, px: f32, py: f32, c0: vec3<f32>) -> vec3<f32> {
    let dx = px - bh(o + 1u); let dy = py - bh(o + 2u);
    let rad = bh(o + 3u); let st = bh(o + 4u); let dc = bh(o + 5u); let ds = bh(o + 6u);
    let lx = (dx * dc + dy * ds) / st; let ly = -dx * ds + dy * dc;
    let q = sqrt(lx * lx + ly * ly) / rad;
    if q > 2.2 { return c0; }
    let body = bh3(50u); let g1 = bh3(53u); let g2 = bh3(56u);
    let red = bh(o + 14u); let kk = bh(o + 13u);
    var c = c0;
    var cover = 0.0;
    if q < 1.0 {
        let nx = (lx * dc - ly * ds) / rad; let ny = (lx * ds + ly * dc) / rad;
        let nz = sqrt(max(1.0 - q * q, 0.0));
        let lit = max(-0.45 * nx - 0.55 * ny + 0.70 * nz, 0.0);
        var col = body * (0.25 + 0.75 * lit);
        col += g1 * (0.5 * pow(lit, 12.0));
        let r1 = (nx * bh(o + 7u) + ny * bh(o + 8u) + nz * bh(o + 9u)) / 0.07;
        let r2 = (nx * bh(o + 10u) + ny * bh(o + 11u) + nz * bh(o + 12u)) / 0.07;
        col += g1 * (1.2 * exp(-r1 * r1)) + g2 * (1.2 * exp(-r2 * r2));
        let e = 1.0 - nz;
        col += g2 * (0.4 * e * e * e);
        cover = clamp((1.0 - q) * rad + 0.5, 0.0, 1.0) * kk;
        c = mix3(c, bh_dim(bh_redden(col, red)), cover);
    }
    let halo = exp(-max(q - 1.0, 0.0) / 0.35) * 0.25 * kk * (1.0 - cover);
    return c + bh_dim(bh_redden(g2, red)) * halo;
}
fn bh_infall(x: u32, y: u32, c0: vec3<f32>) -> vec3<f32> {
    let px = f32(x) + 0.5; let py = f32(y) + 0.5;
    var c = c0;
    let n = u32(bh(28u));
    for (var i = 0u; i < n; i++) {
        let o = BH_DEBRIS + i * BH_STRIDE;
        if bh(o) > 0.5 {
            let e = bh(o + 3u) * bh(o + 4u) * 2.2 + 2.0;
            if abs(px - bh(o + 1u)) > e || abs(py - bh(o + 2u)) > e { continue; }
            c = bh_node(o, px, py, c);
            continue;
        }
        let ax = bh(o + 1u); let ay = bh(o + 2u); let bx = bh(o + 3u); let by = bh(o + 4u); let w = bh(o + 5u);
        let e = 10.0 * w + 2.0;
        if px < min(ax, bx) - e || px > max(ax, bx) + e || py < min(ay, by) - e || py > max(ay, by) + e { continue; }
        let abx = bx - ax; let aby = by - ay;
        let tt = clamp(((px - ax) * abx + (py - ay) * aby) / max(abx * abx + aby * aby, 1e-6), 0.0, 1.0);
        let ex = px - ax - abx * tt; let ey = py - ay - aby * tt;
        let dd = sqrt(ex * ex + ey * ey);
        if dd > e { continue; }
        let core = 1.0 - smoothstep_r(w, w + 1.0, dd);
        let glow = exp(-dd / (2.5 * w));
        c += bh3(o + 6u) * (bh(o + 9u) * (1.4 * core + 0.45 * glow));
    }
    return c;
}

fn pl(b: u32, f: u32) -> f32 { return prims[b + f]; }
fn pl3(b: u32, f: u32) -> vec3<f32> { return vec3<f32>(prims[b + f], prims[b + f + 1u], prims[b + f + 2u]); }
struct Surf { alb: vec3<f32>, emit: vec3<f32>, gloss: f32 }
fn planet_surface(b: u32, u: f32, v: f32, ndl: f32) -> Surf {
    let white = vec3<f32>(0.92, 0.94, 0.97);
    let col = pl3(b, 4u); let col2 = pl3(b, 7u);
    let clouds = pl(b, 20u); let lights = pl(b, 21u);
    let uo = u + pl(b, 22u) * 0.013;
    let kind = i32(pl(b, 0u));
    if kind == 1 {
        let turb = fbm_wrap(uo, v, 8, 10.0);
        let band = 0.5 + 0.5 * sin(TAU * (v * 4.5 + 0.35 * turb));
        let fine = 0.5 + 0.5 * sin(TAU * (v * 13.0 + 0.6 * turb));
        var a = mix3(col, col2, band * 0.75 + fine * 0.25);
        let du = rem_e(u - 0.3, 1.0) - 0.5;
        let dv = v - 0.62;
        let spot = exp(-(du * 9.0) * (du * 9.0) - (dv * 22.0) * (dv * 22.0));
        a = mix3(a, mix3(col2, col, 0.3), 0.7 * spot);
        a = mix3(a, white, clouds * 0.25 * fine);
        return Surf(a, vec3<f32>(0.0), 0.0);
    } else if kind == 2 {
        let h = fbm_wrap(uo, v, 5, 2.5);
        let land = smoothstep_r(0.5, 0.53, h);
        var a = mix3(col, col2 * (0.75 + 0.5 * fbm_wrap(uo, v, 20, 10.0)), land);
        let ice = smoothstep_r(0.78, 0.86, abs(v - 0.5) * 2.0 + 0.08 * (h - 0.5));
        a = mix3(a, white, ice);
        let cl = min(smoothstep_r(0.5, 0.72, fbm_wrap(uo + 0.13, v, 7, 3.5)) * clouds * 1.6, 1.0);
        a = mix3(a, white, cl);
        let night = 1.0 - smoothstep_r(-0.1, 0.15, ndl);
        let city = land * (1.0 - ice) * (1.0 - 0.7 * cl) * smoothstep_r(0.62, 0.8, fbm_wrap(uo, v, 160, 80.0));
        return Surf(a, vec3<f32>(1.0, 0.7, 0.35) * (lights * 0.6 * city * night), (1.0 - land) * (1.0 - cl) * (1.0 - ice));
    } else if kind == 3 {
        let h = fbm_wrap(uo, v, 6, 3.0);
        let r = 1.0 - abs(2.0 * fbm_wrap(uo, v, 12, 6.0) - 1.0);
        let cr = r * r * r;
        return Surf(mix3(col, col2, cr * cr) * (0.85 + 0.3 * h), vec3<f32>(0.0), 0.0);
    } else if kind == 4 {
        let h = fbm_wrap(uo, v, 8, 4.0);
        let r = 1.0 - abs(2.0 * fbm_wrap(uo + 0.21, v, 10, 5.0) - 1.0);
        let crack = smoothstep_r(0.82, 0.97, r);
        return Surf(col * (0.7 + 0.6 * h), col2 * (crack * 1.6 * (0.7 + 0.3 * h)), 0.0);
    }
    let h = fbm_wrap(uo, v, 6, 3.0);
    let h2 = fbm_wrap(uo + 0.37, v, 24, 12.0);
    let pits = smoothstep_r(0.6, 0.8, fbm_wrap(uo, v, 16, 8.0));
    return Surf(mix3(col, col2, smoothstep_r(0.35, 0.65, h)) * ((0.8 + 0.4 * h2) * (1.0 - 0.25 * pits)), vec3<f32>(0.0), 0.0);
}
fn planet_px(b: u32, x: u32, y: u32, c: vec3<f32>) -> vec3<f32> {
    let cx = pl(b, 1u); let cy = pl(b, 2u); let r = pl(b, 3u);
    let dx = (f32(x) + 0.5 - cx) / r;
    let dy = (f32(y) + 0.5 - cy) / r;
    let aa = 1.0 / r;
    let rr = sqrt(dx * dx + dy * dy);
    let s = pl3(b, 14u);
    let atm = pl3(b, 10u); let atm_k = pl(b, 13u);
    var out = c;
    var ring_a = 0.0; var ring_front = false; var ring_col = vec3<f32>(0.0);
    if pl(b, 23u) > 0.0 {
        let r_in = pl(b, 24u); let r_out = pl(b, 25u); let r_open = pl(b, 26u); let r_c = pl(b, 27u); let r_s = pl(b, 28u);
        let a = dx * r_c + dy * r_s;
        let bb = -dx * r_s + dy * r_c;
        let bo = bb / r_open;
        let rho = sqrt(a * a + bo * bo);
        let w = aa / max(min(r_open, 1.0), 0.15);
        if rho > r_in - w && rho < r_out + w {
            let t = clamp((rho - r_in) / (r_out - r_in), 0.0, 1.0);
            let bands = 0.55 + 0.45 * value_noise(t * 38.0 + pl(b, 22u), 0.5);
            let gq = (t - 0.62) / 0.025;
            let gap = 1.0 - 0.85 * exp(-gq * gq);
            let edges = smoothstep_r(r_in - w, r_in + w, rho) * (1.0 - smoothstep_r(r_out - w, r_out + w, rho));
            ring_a = pl(b, 32u) * bands * gap * edges;
            let sa = s.x * r_c - s.y * r_s;
            let sb = s.x * r_s + s.y * r_c;
            let q = vec3<f32>(a, -bb, bo * sqrt(1.0 - r_open * r_open));
            let qs = q.x * sa + q.y * sb + q.z * s.z;
            let shadow = qs < 0.0 && q.x * q.x + q.y * q.y + q.z * q.z - qs * qs < 1.0;
            ring_col = pl3(b, 29u) * select(1.0, 0.08, shadow);
            ring_front = bb > 0.0;
        }
    }
    if ring_a > 0.0 && !ring_front { out = mix3(out, ring_col, ring_a); }
    let cov = clamp((1.0 - rr) / aa + 0.5, 0.0, 1.0);
    if atm_k > 0.0 && rr >= 1.0 - aa && rr < 1.25 {
        let e = max(rr - 1.0, 0.0);
        let lit = smoothstep_r(-0.35, 0.45, (dx * s.x - dy * s.y) / max(rr, 1e-4));
        out += atm * (atm_k * exp(-e / 0.045) * lit * (1.0 - cov));
    }
    if cov > 0.0 {
        let rc = max(rr, 1.0);
        let nx = dx / rc; let ny = -dy / rc;
        let nz = sqrt(max(1.0 - nx * nx - ny * ny, 0.0));
        let ndl = nx * s.x + ny * s.y + nz * s.z;
        let diff = smoothstep_r(-0.05, 0.05, ndl) * (0.12 + 0.88 * clamp(ndl, 0.0, 1.0));
        let tc = pl(b, 17u); let ts = pl(b, 18u);
        let xt = nx * tc + ny * ts;
        let yt = -nx * ts + ny * tc;
        let lat = asin(clamp(yt, -1.0, 1.0));
        let lon = atan2(xt, nz) + pl(b, 19u);
        let u = rem_e(lon / TAU, 1.0); let v = lat / PI + 0.5;
        let sf = planet_surface(b, u, v, ndl);
        var col = sf.alb * (1.1 * diff + 0.012) + sf.emit;
        if sf.gloss > 0.0 && ndl > 0.0 {
            let h = vec3<f32>(s.x, s.y, s.z + 1.0);
            let hl = max(length(h), 1e-5);
            let nh = max((nx * h.x + ny * h.y + nz * h.z) / hl, 0.0);
            let nh2 = nh * nh; let nh8 = nh2 * nh2 * nh2 * nh2;
            let nh40 = nh8 * nh8 * nh8 * nh8 * nh8;
            col += vec3<f32>(1.0, 0.95, 0.85) * (0.6 * sf.gloss * nh40);
        }
        if atm_k > 0.0 {
            let rim = 1.0 - nz;
            col += atm * (atm_k * (rim * rim * rim * smoothstep_r(-0.3, 0.3, ndl) + 0.15 * diff));
        }
        out = mix3(out, col, cov);
    }
    if ring_a > 0.0 && ring_front { out = mix3(out, ring_col, ring_a); }
    return out;
}
fn planet_reach(b: u32) -> f32 {
    var e = select(1.02, 1.25, pl(b, 13u) > 0.0);
    if pl(b, 23u) > 0.0 { e = max(e, pl(b, 25u) + 0.02); }
    return e * pl(b, 3u) + 1.0;
}

fn noise_wrap2(x: f32, y: f32, nx: i32, ny: i32) -> f32 {
    let xf = floor(x); let yf = floor(y);
    let fx = x - xf; let fy = y - yf;
    let sx = fx * fx * (3.0 - 2.0 * fx); let sy = fy * fy * (3.0 - 2.0 * fy);
    let x0 = rem_ei(i32(xf), nx); let y0 = rem_ei(i32(yf), ny);
    let x1 = rem_ei(x0 + 1, nx); let y1 = rem_ei(y0 + 1, ny);
    let a = cell_hash(x0, y0, 0x52u) + (cell_hash(x1, y0, 0x52u) - cell_hash(x0, y0, 0x52u)) * sx;
    let b = cell_hash(x0, y1, 0x52u) + (cell_hash(x1, y1, 0x52u) - cell_hash(x0, y1, 0x52u)) * sx;
    return a + (b - a) * sy;
}
fn tunnel_px(x: u32, y: u32) -> vec3<f32> {
    let c0 = vec3<f32>(S.tn_c0_r, S.tn_c0_g, S.tn_c0_b);
    let c1 = vec3<f32>(S.tn_c1_r, S.tn_c1_g, S.tn_c1_b);
    let c2 = vec3<f32>(S.tn_c2_r, S.tn_c2_g, S.tn_c2_b);
    let dx = f32(x) + 0.5 - S.tn_vx; let dy = f32(y) + 0.5 - S.tn_vy;
    let rs = max(sqrt(dx * dx + dy * dy), 0.5);
    let z = S.tn_radius * S.tn_focal / rs;
    let w = S.tn_travel + z;
    var a = atan2(dy, dx) / TAU + 0.5 + S.tn_spin + S.tn_twist * w;
    let far = 1.0 - exp(-z / S.tn_fade);
    var tube: vec3<f32>;
    if S.tn_kind < 0.5 {
        let lane = rem_e(a, 1.0) * S.tn_lanes;
        let li = floor(lane);
        let fl = lane - li - 0.5;
        let ln = rem_ei(i32(li), i32(S.tn_lanes));
        let spacing = TAU * rs / S.tn_lanes;
        let width = 0.9 * S.tn_scale;
        var v = 0.0;
        if cell_hash(ln, 7, 0x71u) < 0.7 {
            let len = 0.25 + 0.5 * cell_hash(ln, 2, 0x73u);
            let s = rem_e(w / S.tn_period + cell_hash(ln, 1, 0x72u), 1.0);
            let along = smoothstep_r(0.0, 0.04, s) * (1.0 - smoothstep_r(len * 0.6, len, s));
            v = along * smoothstep_r(width, 0.0, abs(fl) * spacing);
        }
        let mean = 0.7 * 0.4 * min(2.0 * width / spacing, 1.0);
        v += (mean - v) * smoothstep_r(3.0, 1.0, spacing / width);
        let walls = 0.5 + 0.5 * noise_wrap2(rem_e(a, 1.0) * 12.0, w / S.tn_loop * S.tn_ny * 2.0, 12, i32(S.tn_ny * 2.0));
        let hue = mix3(c1, c2, 0.5 * cell_hash(ln, 3, 0x74u));
        tube = c0 * walls + hue * (1.6 * v * (1.0 - far));
    } else {
        a += 0.25 * log(1.0 + z / S.tn_radius);
        let au = rem_e(a, 1.0);
        let yw = w / S.tn_loop * S.tn_ny;
        let nx = i32(S.tn_nx); let ny = i32(S.tn_ny);
        let val = 0.55 * noise_wrap2(au * S.tn_nx, yw, nx, ny) + 0.3 * noise_wrap2(au * S.tn_nx * 2.0, yw * 2.0, nx * 2, ny * 2)
            + 0.15 * noise_wrap2(au * S.tn_nx * 4.0, yw * 4.0, nx * 4, ny * 4);
        let bands = smoothstep_r(0.35, 0.8, val);
        let f = 1.0 - abs(2.0 * noise_wrap2(au * S.tn_nx * 3.0 + 0.5 * val, yw * 3.0, nx * 3, ny * 3) - 1.0);
        let f2 = f * f;
        let fil = f2 * f2 * f2;
        tube = c0 * (0.4 + 0.6 * val) + c1 * ((0.9 * bands + 0.7 * fil) * (1.0 - 0.6 * far));
    }
    let q = rs / S.tn_focal;
    tube = mix3(tube, c2 * (0.6 * S.tn_core), far);
    tube += c2 * (1.2 * S.tn_core * exp(-q / 0.03));
    return (tube + c1 * (0.15 * S.tn_core * exp(-q / 0.15))) * S.tn_k;
}

@compute @workgroup_size(8, 8)
fn sky_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= W.width || gid.y >= W.height { return; }
    let x = gid.x; let y = gid.y;
    let i = y * W.width + x;
    if (gbuf[i].idr & 0xFFu) != ID_NONE { return; }
    var c: vec3<f32>;
    if S.mode == 0u { c = vec3<f32>(hdr[i * 3u], hdr[i * 3u + 1u], hdr[i * 3u + 2u]); } else { c = sky_base(f32(y)); }
    let fx = f32(x); let fy = f32(y);
    let above = fy < S.hy;
    let tile = (y / 16u) * S.tile_cols + x / 16u;
    let ntiles = S.tile_cols * ((W.height + 15u) / 16u);
    if S.sp_on != 0u { c = space_backdrop(x, y, c); }
    if S.stars_on != 0u && above {
        let behind = S.md_on != 0u && (fx + 0.5 - S.md_x) * (fx + 0.5 - S.md_x) + (fy + 0.5 - S.md_y) * (fy + 0.5 - S.md_y) < S.md_r * S.md_r;
        if !behind {
            let a = bins[S.star_bins + tile]; let b = bins[S.star_bins + tile + 1u];
            for (var k = a; k < b; k++) {
                let s = bins[S.star_bins + ntiles + 1u + k];
                let ox = i32(x) - i32(prim(S.star_prims, s, 0u));
                let oy = i32(y) - i32(prim(S.star_prims, s, 1u));
                let ri = i32(prim(S.star_prims, s, 3u));
                if abs(ox) > ri || abs(oy) > ri { continue; }
                let r = prim(S.star_prims, s, 2u);
                let dd = sqrt(f32(ox * ox + oy * oy));
                if dd <= r { c += vec3<f32>(0.9, 0.92, 1.0) * (prim(S.star_prims, s, 4u) * (1.0 - dd / (r + 1.0))); }
            }
        }
    }
    if S.au_on != 0u && above {
        let ax = (fx + 0.5 - S.au_cx) / S.au_unit;
        c += aurora_px(aurora_col(ax), ax, (S.hy - (fy + 0.5)) / S.au_unit);
    }
    if S.sun_on != 0u && above {
        let sc = vec3<f32>(S.s_c_r, S.s_c_g, S.s_c_b);
        let dx = fx + 0.5 - S.s_x; let dy = fy + 0.5 - S.s_y;
        let dd = sqrt(dx * dx + dy * dy);
        let th = dd / S.s_focal;
        let glow = S.s_air * (0.45 * exp(-th / (0.05 * S.s_spread)) + 0.1 * exp(-th / (0.35 * S.s_spread))) + (1.0 - S.s_air) * 0.5 * exp(-th / 0.01);
        c += sc * glow;
        if dd < S.s_r + 0.5 {
            let q = min(dd / S.s_r, 1.0);
            let mu = sqrt(1.0 - q * q);
            c = mix3(c, sc * (3.0 * (0.6 + 0.4 * mu)), clamp(S.s_r + 0.5 - dd, 0.0, 1.0));
        }
    }
    if S.moon_on != 0u {
        let cx = S.md_x; let cy = S.md_y; let r = S.md_r; let gr = S.m_gr;
        let ix = i32(x); let iy = i32(y);
        if ix >= i32(cx - gr) && ix <= i32(cx + gr) && iy >= i32(cy - gr) && iy <= i32(cy + gr) && above {
            let mc = vec3<f32>(S.m_c_r, S.m_c_g, S.m_c_b);
            let dx = (fx + 0.5 - cx) / r; let dy = (fy + 0.5 - cy) / r;
            let dd = sqrt(dx * dx + dy * dy);
            if dd < 1.0 {
                let nz = sqrt(max(1.0 - dd * dd, 0.0));
                let edge = clamp(1.5 / max(r, 1.0), 0.01, 0.2);
                let lit = smoothstep_r(-edge, edge, dx * S.m_sx + nz * S.m_sy);
                var crater = 1.0;
                if S.m_craters != 0u { crater = moon_albedo(vec3<f32>(dx, dy, nz), 1.0 / max(r, 1.0)); }
                if lit > 0.0 { c = mix3(c, mc * (1.15 * crater), clamp(S.m_op * lit, 0.0, 1.0)); }
                c += mc * (0.012 * S.m_op * (1.0 - lit));
                c += mc * (0.15 * S.m_litf);
            } else if dd * r < gr {
                let q = 1.0 - (dd * r - r) / (gr - r);
                c += mc * (0.15 * S.m_litf * q * q);
            }
        }
    }
    if S.sp_on != 0u && (above || S.sp_below != 0u) {
        for (var k = 0u; k < S.pl_n; k++) {
            let b = S.pl_prims + k * 36u;
            let e = planet_reach(b);
            if abs(fx + 0.5 - pl(b, 1u)) < e + 0.5 && abs(fy + 0.5 - pl(b, 2u)) < e + 0.5 { c = planet_px(b, x, y, c); }
        }
        if S.bh_on != 0u { c = bh_infall(x, y, c); }
    }
    if S.cl_on != 0u && above {
        let a = bins[S.cl_bins + tile]; let b = bins[S.cl_bins + tile + 1u];
        let col = vec3<f32>(S.cl_r, S.cl_g, S.cl_b);
        let ix = i32(x); let iy = i32(y);
        for (var k = a; k < b; k++) {
            let bl = bins[S.cl_bins + ntiles + 1u + k];
            let bx = prim(S.cl_prims, bl, 0u); let by = prim(S.cl_prims, bl, 1u);
            let rx = prim(S.cl_prims, bl, 2u); let ry = prim(S.cl_prims, bl, 3u);
            if ix < i32(bx - rx) || ix > i32(bx + rx) || iy < i32(by - ry) || iy > i32(by + ry) { continue; }
            let dx = (fx - bx) / rx; let dy = (fy - by) / ry;
            let d2 = dx * dx + dy * dy;
            if d2 < 1.0 {
                let shade = 1.0 - 0.25 * max(dy, 0.0);
                var cc = col * shade;
                if S.cl_sun != 0u {
                    let q = 0.3 * S.fhy;
                    let near = exp(-((fx - S.cl_sx) * (fx - S.cl_sx) + (fy - S.cl_sy) * (fy - S.cl_sy)) / (q * q));
                    cc += vec3<f32>(S.cl_sc_r, S.cl_sc_g, S.cl_sc_b) * (near * (0.25 + 0.9 * d2));
                }
                c = mix3(c, cc, clamp(S.cl_op * prim(S.cl_prims, bl, 4u) * pow(1.0 - d2, 1.2) * 0.8, 0.0, 1.0));
            }
        }
    }
    if S.rb_on != 0u && above {
        let low = smoothstep_r(S.hy, S.hy - 0.08 * S.fhy, fy);
        let dx = fx + 0.5 - S.rb_cx; let dy = fy + 0.5 - S.rb_cy;
        let d = sqrt(dx * dx + dy * dy);
        var rc = vec3<f32>(0.0);
        let t1 = (d - (S.rb_rad - S.rb_band * 0.5)) / S.rb_band;
        if t1 >= 0.0 && t1 < 1.0 { rc += spectrum(t1) * sqrt(sin(PI * t1)); }
        if d < S.rb_rad - S.rb_band * 0.5 { rc += vec3<f32>(0.05); }
        if S.rb_double != 0u {
            let t2 = (d - (S.rb_rad2 - S.rb_band2 * 0.5)) / S.rb_band2;
            if t2 >= 0.0 && t2 < 1.0 { rc += spectrum(1.0 - t2) * (0.4 * sqrt(sin(PI * t2))); }
        }
        if any(rc != vec3<f32>(0.0)) { c += rc * (S.rb_k * low); }
    }
    if S.fl_on != 0u && y < min(u32(S.hy), W.height) {
        let k = 0.55 * pow(1.0 - max((fy - S.oy) / S.fhy, 0.0), 0.7) + 0.15;
        c += vec3<f32>(S.fl_r, S.fl_g, S.fl_b) * k;
    }
    if S.bolt_on != 0u && x >= S.b_xa && x <= S.b_xb && y <= S.b_yb {
        let pt = vec2<f32>(fx + 0.5, fy + 0.5);
        var cc = 0.0; var gg = 0.0;
        for (var k = 0u; k < S.b_nsegs; k++) {
            let a = vec2<f32>(prim(S.b_prims, k, 0u), prim(S.b_prims, k, 1u));
            let b = vec2<f32>(prim(S.b_prims, k, 2u), prim(S.b_prims, k, 3u));
            let wgt = prim(S.b_prims, k, 4u);
            let ab = b - a;
            let t = clamp(((pt.x - a.x) * ab.x + (pt.y - a.y) * ab.y) / max(ab.x * ab.x + ab.y * ab.y, 1e-6), 0.0, 1.0);
            let ex = pt.x - a.x - ab.x * t; let ey = pt.y - a.y - ab.y * t;
            let d = sqrt(ex * ex + ey * ey);
            let width = max(0.9 * S.b_scale * wgt, 0.5);
            cc = max(cc, wgt * (1.0 - smoothstep_r(width, width + 1.0, d)));
            gg = max(gg, wgt * exp(-d / (S.b_glow * max(wgt, 0.5))));
        }
        let add = vec3<f32>(S.b_core_r, S.b_core_g, S.b_core_b) * (6.0 * cc) + vec3<f32>(S.b_tint_r, S.b_tint_g, S.b_tint_b) * (0.9 * gg);
        c += add * S.b_gain;
    }
    if S.veil > 0.0 { c = mix3(c, fog_col(), S.veil); }
    if S.mode != 0u {
        c = sky_seen(c) * sky_weight(x);
        if S.mode == 2u { c += vec3<f32>(hdr[i * 3u], hdr[i * 3u + 1u], hdr[i * 3u + 2u]); }
    }
    if S.tn_on != 0u { c = tunnel_px(x, y); }
    if S.bank_on != 0u { c = mix3(vec3<f32>(S.bank_r, S.bank_g, S.bank_b), c, S.bank_t); }
    hdr[i * 3u] = c.x; hdr[i * 3u + 1u] = c.y; hdr[i * 3u + 2u] = c.z;
}
