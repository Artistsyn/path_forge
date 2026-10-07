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
    au_on: u32, au_c1: f32, au_c2: f32, au_c3: f32, au_t: f32,
    au_lo_r: f32, au_lo_g: f32, au_lo_b: f32, au_hi_r: f32, au_hi_g: f32, au_hi_b: f32,
    au_seed: u32, au_p1: f32, au_p2: f32, au_height: f32, au_k: f32,
    sun_on: u32, s_x: f32, s_y: f32, s_r: f32, s_c_r: f32, s_c_g: f32, s_c_b: f32, s_spread: f32, s_focal: f32,
    moon_on: u32, m_c_r: f32, m_c_g: f32, m_c_b: f32, m_gr: f32, m_sx: f32, m_sy: f32, m_litf: f32, m_op: f32, m_craters: u32,
    cl_on: u32, cl_r: f32, cl_g: f32, cl_b: f32, cl_sun: u32, cl_sx: f32, cl_sy: f32, cl_sc_r: f32, cl_sc_g: f32, cl_sc_b: f32,
    cl_op: f32, cl_bins: u32, cl_prims: u32,
    rb_on: u32, rb_cx: f32, rb_cy: f32, rb_rad: f32, rb_band: f32, rb_rad2: f32, rb_band2: f32, rb_k: f32, rb_double: u32,
    fl_on: u32, fl_r: f32, fl_g: f32, fl_b: f32,
    bolt_on: u32, b_gain: f32, b_core_r: f32, b_core_g: f32, b_core_b: f32, b_tint_r: f32, b_tint_g: f32, b_tint_b: f32,
    b_glow: f32, b_scale: f32, b_xa: u32, b_xb: u32, b_yb: u32, b_nsegs: u32, b_prims: u32,
    veil: f32, bank_on: u32, bank_t: f32, bank_r: f32, bank_g: f32, bank_b: f32,
    pad2: u32, pad3: u32, pad4: u32,
}

@group(0) @binding(1) var<uniform> S: Sky;
@group(0) @binding(2) var<storage, read> gbuf: array<GPix>;
@group(0) @binding(3) var<storage, read_write> hdr: array<f32>;
// Stars (x, y, r, ri, b, 0), cloud blobs (x, y, rx, ry, life, 0), bolt segments (ax, ay, bx, by, weight, 0).
@group(0) @binding(4) var<storage, read> prims: array<f32>;
// Per-tile lists: for stars, then for blobs, (tiles + 1) offsets followed by indices.
@group(0) @binding(5) var<storage, read> bins: array<u32>;

fn prim(base: u32, k: u32, f: u32) -> f32 { return prims[base + k * 6u + f]; }

// render::noise1
fn noise1(seed: u32, x: f32) -> f32 {
    let i = i32(floor(x));
    var f = x - f32(i);
    f = f * f * (3.0 - 2.0 * f);
    let a = hf(seed, i); let b = hf(seed, i + 1);
    return (a + (b - a) * f) * 2.0 - 1.0;
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

@compute @workgroup_size(8, 8)
fn sky_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= W.width || gid.y >= W.height { return; }
    let x = gid.x; let y = gid.y;
    let i = y * W.width + x;
    if (gbuf[i].idr & 0xFFu) != ID_NONE { return; }
    var c = vec3<f32>(hdr[i * 3u], hdr[i * 3u + 1u], hdr[i * 3u + 2u]);
    let fx = f32(x); let fy = f32(y);
    let above = fy < S.hy;
    let tile = (y / 16u) * S.tile_cols + x / 16u;
    let ntiles = S.tile_cols * ((W.height + 15u) / 16u);
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
        let ny = (fy + 0.5 - S.oy) / S.fhy;
        let nx = (fx + 0.5 - S.ox) / S.fw;
        let t = S.au_t;
        let foot = 0.15 + 0.6 * S.au_height + 0.1 * sin(TAU * nx * 1.3 + S.au_c1 * t + S.au_p1) + 0.04 * sin(TAU * nx * 3.7 - S.au_c2 * t + S.au_p2);
        let up = foot - ny;
        var profile: f32;
        if up < 0.0 { let q = up / 0.02; profile = exp(-(q * q)); } else { profile = exp(-up / 0.22); }
        if profile >= 0.01 {
            let fold = 0.5 + 0.5 * sin(TAU * (nx * 2.0 + 0.3 * sin(TAU * nx * 0.7 + S.au_c1 * t)) + S.au_p2);
            let ray = 0.55 + 0.45 * noise1(S.au_seed ^ 0x83u, nx * 150.0 + 3.0 * sin(S.au_c3 * t + S.au_p1));
            let col = mix3(vec3<f32>(S.au_lo_r, S.au_lo_g, S.au_lo_b), vec3<f32>(S.au_hi_r, S.au_hi_g, S.au_hi_b), clamp(up / 0.3, 0.0, 1.0));
            c += col * (S.au_k * profile * (0.25 + 0.75 * fold) * ray);
        }
    }
    if S.sun_on != 0u && above {
        let sc = vec3<f32>(S.s_c_r, S.s_c_g, S.s_c_b);
        let dx = fx + 0.5 - S.s_x; let dy = fy + 0.5 - S.s_y;
        let dd = sqrt(dx * dx + dy * dy);
        let th = dd / S.s_focal;
        let glow = 0.45 * exp(-th / (0.05 * S.s_spread)) + 0.1 * exp(-th / (0.35 * S.s_spread));
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
    if S.bank_on != 0u { c = mix3(vec3<f32>(S.bank_r, S.bank_g, S.bank_b), c, S.bank_t); }
    hdr[i * 3u] = c.x; hdr[i * 3u + 1u] = c.y; hdr[i * 3u + 2u] = c.z;
}
