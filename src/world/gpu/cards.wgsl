// Billboards (render::Card::draw_rows), one thread per pixel: each pixel walks its tile's cards
// far to near, the order the CPU draws them in, so depth tests and blends happen identically.
// A card's fields are read straight from the storage buffer (`cards[ci].x`), never copied out as
// a `Card`: the copy is ~500 bytes per card per pixel, spilled, and it cost a Mali-G615 ~60 ms a
// frame where the reads take what the path through the card actually uses.

struct Card {
    sx: f32, wp: f32, hp: f32, sy_top: f32, sway: f32, z: f32,
    x0: i32, x1: i32, y0: i32, y1: i32,
    light_r: f32, light_g: f32, light_b: f32,
    snow_on: u32, snow_cap: f32, snow_r: f32, snow_g: f32, snow_b: f32,
    nearest: u32, flip: u32, plain: u32, fog_t: f32,
    fog_r: f32, fog_g: f32, fog_b: f32, fog_mul: f32, add_r: f32, add_g: f32, add_b: f32,
    emissive: f32,
    sw: u32, sh: u32, soff: u32, gw: u32, gh: u32, goff: u32,
    id: u32, realm: u32, wx: f32, wy: f32, wd: f32, source: u32,
    w_amp: f32, w_v: f32, w_lean: f32, w_t1: f32, w_t2: f32, w_c0: f32, w_c1: f32, w_trunk: f32, w_foot: f32, w_kv: f32,
    snow_dust: f32,
    pan: u32, ph: array<f32, 9>, pq0: f32, pq1: f32, pq2: f32, strip: f32, mp: array<f32, 6>,
    tube: u32, ta0: f32, ta1: f32, ta2: f32, tu0: f32, tu1: f32, tu2: f32, tlen: f32, trad: f32,
    tb0: f32, tb1: f32, tb2: f32, te0: f32, te1: f32, te2: f32, cube: array<f32, 18>, tkf: f32, tcx: f32, tcy: f32,
    fade: f32,
}

struct Pass { tile_cols: u32, pick: u32, n_cards: u32, pad: u32 }

@group(0) @binding(1) var<storage, read_write> gbuf: array<GPix>;
@group(0) @binding(2) var<storage, read_write> hdr: array<f32>;
@group(0) @binding(3) var<storage, read> texels: array<vec4<f32>>;
@group(0) @binding(4) var<storage, read> cards: array<Card>;
@group(0) @binding(5) var<storage, read> bins: array<u32>;
@group(0) @binding(6) var<storage, read_write> pick: array<u32>;
@group(0) @binding(7) var<uniform> P: Pass;

// render::snow_noise.
fn snow_h(i: i32, j: i32) -> f32 {
    var h = (bitcast<u32>(i) * 0x8da6b343u) ^ (bitcast<u32>(j) * 0xd8163841u) ^ 0x525de6dfu;
    h ^= h >> 15u;
    h = h * 0x2c1b3c6du;
    h ^= h >> 12u;
    return f32(h & 0xffffu) / 65535.0;
}
fn snow_noise(x: f32, y: f32) -> f32 {
    let xf = floor(x); let yf = floor(y);
    let fx = x - xf; let fy = y - yf;
    let sx = fx * fx * (3.0 - 2.0 * fx); let sy = fy * fy * (3.0 - 2.0 * fy);
    let i = i32(xf); let j = i32(yf);
    let a = snow_h(i, j) + (snow_h(i + 1, j) - snow_h(i, j)) * sx;
    let b = snow_h(i, j + 1) + (snow_h(i + 1, j + 1) - snow_h(i, j + 1)) * sx;
    return a + (b - a) * sy;
}

// render::snow_on_card.
fn sample_above(ci: u32, u: f32, v: f32, dv: f32) -> vec4<f32> {
    if v - dv < 0.0 { return vec4<f32>(0.0); }
    return sprite_sample(cards[ci].soff, cards[ci].sw, cards[ci].sh, u, v - dv, true);
}
fn luma4(c: vec4<f32>) -> f32 { return 0.3 * c.x + 0.59 * c.y + 0.11 * c.z; }
fn snow_on_card(ci: u32, u: f32, v: f32, lit: vec3<f32>) -> vec3<f32> {
    let snow = vec3<f32>(cards[ci].snow_r, cards[ci].snow_g, cards[ci].snow_b);
    if sample_above(ci, u, v, cards[ci].snow_cap).w < 0.5 { return snow; }
    if cards[ci].snow_dust <= 0.0 { return lit; }
    let step = max(1.5 / max(cards[ci].hp, 1.0), 0.012);
    let here = sprite_sample(cards[ci].soff, cards[ci].sw, cards[ci].sh, u, v, true);
    let a1 = sample_above(ci, u, v, step);
    let a2 = sample_above(ci, u, v, 2.5 * step);
    var open = 0.0;
    if a1.w < 0.5 { open = 1.0; } else if a2.w < 0.5 { open = 0.7; }
    var ledge = 0.0;
    if a1.w >= 0.5 { ledge = clamp((luma4(here) - luma4(a1)) * 5.0, 0.0, 1.0); }
    let patchy = pow(0.5 + 0.5 * sin(37.0 * u + 23.0 * v + 3.1 * sin(13.0 * v)), 0.7);
    let d = cards[ci].snow_dust;
    let blob = smoothstep_r(0.5, 0.7, 0.65 * snow_noise(5.0 * u, 8.0 * v) + 0.35 * snow_noise(11.0 * u + 3.1, 17.0 * v + 1.7));
    let k = min(d * max(open, ledge) * (0.55 + 0.45 * patchy) + d * d * (0.75 * blob + 0.2 * patchy) * (1.0 - 0.5 * v), 1.0);
    return mix3(lit, snow, k);
}

// render::dot3, its products rounded one by one as the CPU rounds them.
fn dot3r(a: vec3<f32>, b: vec3<f32>) -> f32 { return rnd(rnd(rnd(a.x * b.x) + rnd(a.y * b.y)) + rnd(a.z * b.z)); }
fn sc3(a: vec3<f32>, k: f32) -> vec3<f32> { return vec3<f32>(rnd(a.x * k), rnd(a.y * k), rnd(a.z * k)); }

// render::log_shade.
fn log_shade(v0: f32) -> f32 {
    let v = clamp(v0, 0.0, 1.0);
    return rnd(rnd(0.25 + rnd(0.75 * sqrt(max(sin(rnd(3.14159274 * v)), 0.0)))) * rnd(0.8 + rnd(0.4 * v)));
}

// render::tube_light.
fn tube_light(ci: u32, n: vec3<f32>) -> vec3<f32> {
    var l = vec3<f32>(0.0);
    for (var i = 0u; i < 3u; i++) {
        let c = n[i];
        var o = i * 3u;
        if c < 0.0 { o = (i + 3u) * 3u; }
        let w = rnd(c * c);
        l = vec3<f32>(rnd(l.x + rnd(w * cards[ci].cube[o])), rnd(l.y + rnd(w * cards[ci].cube[o + 1u])), rnd(l.z + rnd(w * cards[ci].cube[o + 2u])));
    }
    return l;
}

// render::tube_at: depth, cover, then the colour as lit; depth 0 where it is not.
struct TubeHit { z: f32, cover: f32, lit: vec3<f32> }
fn tube_at(ci: u32, fx: f32, fy: f32) -> TubeHit {
    let d = vec3<f32>(rnd((fx - cards[ci].tcx) * cards[ci].tkf), rnd((cards[ci].tcy - fy) * cards[ci].tkf), 1.0);
    let a = vec3<f32>(cards[ci].ta0, cards[ci].ta1, cards[ci].ta2);
    let u = vec3<f32>(cards[ci].tu0, cards[ci].tu1, cards[ci].tu2);
    let r = cards[ci].trad;
    let bark = vec3<f32>(cards[ci].tb0, cards[ci].tb1, cards[ci].tb2);
    let endc = vec3<f32>(cards[ci].te0, cards[ci].te1, cards[ci].te2);
    let du = dot3r(d, u);
    let au = dot3r(a, u);
    let dp = d - sc3(u, du);
    let ap = a - sc3(u, au);
    let qa = dot3r(dp, dp);
    let qb = dot3r(dp, ap);
    let aa = dot3r(ap, ap);
    var best = TubeHit(0.0, 0.0, vec3<f32>(0.0));
    if qa > 1e-12 {
        let tc = div(qb, qa);
        let rho = sqrt(max(aa - rnd(qb * tc), 0.0));
        let pix = rnd(max(tc, 0.05) * cards[ci].tkf);
        let cover = clamp(div(r - rho, pix) + 0.5, 0.0, 1.0);
        if cover > 0.0 {
            var th = tc;
            if rho < r { th = tc - sqrt(div(rnd(r * r) - rnd(rho * rho), qa)); }
            let s = rnd(th * du) - au;
            if th > 0.05 && s >= 0.0 && s <= cards[ci].tlen {
                var n = sc3(d, th) - a - sc3(u, s);
                let nl = max(sqrt(dot3r(n, n)), 1e-9);
                n = vec3<f32>(div(n.x, nl), div(n.y, nl), div(n.z, nl));
                let k = log_shade(0.5 + rnd(0.5 * n.y));
                let hh = sqrt(rnd(rnd(n.x * n.x) + rnd(n.z * n.z)));
                var outn = vec3<f32>(0.0, 0.0, -1.0);
                if hh > 1e-3 { outn = vec3<f32>(div(n.x, hh), 0.0, div(n.z, hh)); }
                let tl = tube_light(ci, outn);
                let b = sc3(bark, k);
                best = TubeHit(th, cover, vec3<f32>(rnd(b.x * tl.x), rnd(b.y * tl.y), rnd(b.z * tl.z)));
            }
        }
    }
    if abs(du) > 1e-6 {
        for (var e = 0u; e < 2u; e++) {
            var sc = 0.0; var sg = -1.0;
            if e == 1u { sc = cards[ci].tlen; sg = 1.0; }
            let tq = div(au + sc, du);
            if tq <= 0.05 || (best.z > 0.0 && best.z <= tq) { continue; }
            let p = sc3(d, tq) - a - sc3(u, sc);
            let q = div(sqrt(dot3r(p, p)), r);
            if q > 1.0 { continue; }
            var col: vec3<f32>;
            if q > 0.86 { col = sc3(bark, 0.7); } else if q < 0.07 { col = sc3(endc, 0.62); } else { col = sc3(endc, rnd(0.86 + rnd(0.14 * cos(rnd(26.0 * q))))); }
            let tl = tube_light(ci, vec3<f32>(sg * u.x, sg * u.y, sg * u.z));
            best = TubeHit(tq, 1.0, vec3<f32>(rnd(col.x * tl.x), rnd(col.y * tl.y), rnd(col.z * tl.z)));
        }
    }
    return best;
}

// Sprite::sample at one (already chosen) level.
fn sprite_sample(off: u32, w: u32, h: u32, u: f32, v: f32, nearest: bool) -> vec4<f32> {
    if nearest {
        let x = min(u32(max(u * f32(w), 0.0)), w - 1u);
        let y = min(u32(max(v * f32(h), 0.0)), h - 1u);
        return texels[off + y * w + x];
    }
    let fx = clamp(rnd(u * f32(w)) - 0.5, 0.0, f32(w) - 1.0);
    let fy = clamp(rnd(v * f32(h)) - 0.5, 0.0, f32(h) - 1.0);
    let x0 = u32(fx); let y0 = u32(fy);
    let x1 = min(x0 + 1u, w - 1u); let y1 = min(y0 + 1u, h - 1u);
    let tx = fx - f32(x0); let ty = fy - f32(y0);
    var p: array<vec4<f32>, 4>;
    p[0] = texels[off + y0 * w + x0]; p[1] = texels[off + y0 * w + x1];
    p[2] = texels[off + y1 * w + x0]; p[3] = texels[off + y1 * w + x1];
    var wts = array<f32, 4>(rnd((1.0 - tx) * (1.0 - ty)), rnd(tx * (1.0 - ty)), rnd((1.0 - tx) * ty), rnd(tx * ty));
    var acc = vec4<f32>(0.0);
    for (var k = 0; k < 4; k++) {
        let a = rnd(p[k].w * wts[k]);
        acc = vec4<f32>(rnd(acc.x + rnd(p[k].x * a)), rnd(acc.y + rnd(p[k].y * a)), rnd(acc.z + rnd(p[k].z * a)), rnd(acc.w + a));
    }
    if acc.w > 1e-6 { return vec4<f32>(acc.x / acc.w, acc.y / acc.w, acc.z / acc.w, acc.w); }
    return vec4<f32>(0.0);
}

@compute @workgroup_size(8, 8)
fn cards_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= W.width || gid.y >= W.height { return; }
    let i = gid.y * W.width + gid.x;
    let px = i32(gid.x); let py = i32(gid.y);
    let tile = (gid.y / 16u) * P.tile_cols + gid.x / 16u;
    let ntiles = P.tile_cols * ((W.height + 15u) / 16u);
    let a0 = bins[tile]; let a1 = bins[tile + 1u];
    if a0 == a1 { return; }
    var g = gbuf[i];
    var c = vec3<f32>(hdr[i * 3u], hdr[i * 3u + 1u], hdr[i * 3u + 2u]);
    var pk = 0u;
    var wrote_pick = false;
    var touched = false;
    for (var k = a0; k < a1; k++) {
        let ci = bins[ntiles + 1u + k];
        if px < cards[ci].x0 || px > cards[ci].x1 || py < cards[ci].y0 || py > cards[ci].y1 { continue; }
        if dissolved(px, py, cards[ci].fade) { continue; }
        if cards[ci].tube != 0u {
            // A cylinder (render::Card::draw_rows, its tube).
            let th = tube_at(ci, f32(px) + 0.5, f32(py) + 0.5);
            if th.z <= 0.0 || th.z < 0.05 || th.z >= g.depth { continue; }
            var tcol: vec3<f32>;
            if cards[ci].plain != 0u { tcol = mix3(vec3<f32>(cards[ci].fog_r, cards[ci].fog_g, cards[ci].fog_b), th.lit, cards[ci].fog_t); }
            else { tcol = th.lit * cards[ci].fog_mul + vec3<f32>(cards[ci].add_r, cards[ci].add_g, cards[ci].add_b); }
            c = mix3(c, tcol, th.cover);
            touched = true;
            if th.cover >= 0.5 {
                g = GPix(th.z, cards[ci].wx, cards[ci].wy, cards[ci].wd, cards[ci].id | (cards[ci].realm << 8u));
                pk = cards[ci].source; wrote_pick = true;
            }
            continue;
        }
        if cards[ci].pan != 0u {
            // A card standing in the world (render::Card::draw_rows, its panel).
            let fx = f32(px) + 0.5; let fy = f32(py) + 0.5;
            let iz = rnd(cards[ci].pq0 * fx) + rnd(cards[ci].pq1 * fy) + cards[ci].pq2;
            if iz <= 0.0 { continue; }
            // (div: Metal's fast reciprocal is a bit off the CPU's 1/iz, and depth must match exactly.)
            let pz = div(1.0, iz);
            if pz < 0.05 || pz >= g.depth { continue; }
            let wd = rnd(cards[ci].ph[6] * fx) + rnd(cards[ci].ph[7] * fy) + cards[ci].ph[8];
            if abs(wd) < 1e-12 { continue; }
            var pu = div(rnd(cards[ci].ph[0] * fx) + rnd(cards[ci].ph[1] * fy) + cards[ci].ph[2], wd);
            var pv = div(rnd(cards[ci].ph[3] * fx) + rnd(cards[ci].ph[4] * fy) + cards[ci].ph[5], wd);
            if !(pu >= 0.0 && pu < 1.0) || !(pv >= 0.0 && pv < 1.0) { continue; }
            if cards[ci].strip >= 0.0 { pu = cards[ci].strip; }
            else if cards[ci].strip < -1.5 {
                // (render::map_uv.)
                let mu = cards[ci].mp[0] + rnd(cards[ci].mp[1] * pu) + rnd(cards[ci].mp[2] * pv);
                pv = cards[ci].mp[3] + rnd(cards[ci].mp[4] * pu) + rnd(cards[ci].mp[5] * pv);
                pu = mu;
            }
            else if cards[ci].flip != 0u { pu = 1.0 - pu; }
            let pn = cards[ci].nearest != 0u;
            let ps = sprite_sample(cards[ci].soff, cards[ci].sw, cards[ci].sh, pu, pv, pn);
            var pa = ps.w;
            if pn { pa = select(0.0, 1.0, ps.w >= 0.5); }
            if cards[ci].strip < -2.5 { pa = 1.0; }
            if pa < 0.02 { continue; }
            var pglow = cards[ci].emissive;
            if cards[ci].goff != 0xFFFFFFFFu { pglow += sprite_sample(cards[ci].goff, cards[ci].gw, cards[ci].gh, pu, pv, pn).x; }
            var plit = ps.xyz * vec3<f32>(cards[ci].light_r, cards[ci].light_g, cards[ci].light_b) + ps.xyz * pglow;
            if cards[ci].snow_on != 0u && pa >= 0.5 { plit = snow_on_card(ci, pu, pv, plit); }
            var pcol: vec3<f32>;
            if cards[ci].plain != 0u { pcol = mix3(vec3<f32>(cards[ci].fog_r, cards[ci].fog_g, cards[ci].fog_b), plit, cards[ci].fog_t); }
            else { pcol = plit * cards[ci].fog_mul + vec3<f32>(cards[ci].add_r, cards[ci].add_g, cards[ci].add_b); }
            c = mix3(c, pcol, pa);
            touched = true;
            if pa >= 0.5 {
                g = GPix(pz, cards[ci].wx, cards[ci].wy, cards[ci].wd, cards[ci].id | (cards[ci].realm << 8u));
                pk = cards[ci].source; wrote_pick = true;
            }
            continue;
        }
        let row = div(f32(py) + 0.5 - cards[ci].sy_top, cards[ci].hp);
        if !(row >= 0.0 && row < 1.0) { continue; }
        var vv = row;
        if cards[ci].z >= g.depth { continue; }
        let bend = rnd(rnd(cards[ci].sway * (1.0 - vv)) * (1.0 - vv));
        var u = div(rnd(rnd(f32(px) + 0.5 - rnd(cards[ci].sx - cards[ci].wp * 0.5)) - bend), cards[ci].wp);
        if cards[ci].w_amp != 0.0 {
            // render::CardWind::at.
            var wt = smoothstep_r(cards[ci].w_c1, cards[ci].w_c0, row);
            if cards[ci].w_trunk > 0.0 { wt *= 1.0 - (1.0 - smoothstep_r(cards[ci].w_trunk, 4.0 * cards[ci].w_trunk, abs(u - 0.5))) * smoothstep_r(cards[ci].w_foot - 0.45, cards[ci].w_foot, row); }
            let wh = cards[ci].w_kv * row;
            let wa = TAU * (1.6 * u + 1.1 * wh) - cards[ci].w_t1;
            let wb = TAU * (4.3 * u - 2.7 * wh) - cards[ci].w_t2;
            let wc = TAU * (11.0 * u + 7.0 * wh) - cards[ci].w_t1 - cards[ci].w_t2;
            u -= wt * (cards[ci].w_lean + cards[ci].w_amp * (0.55 * sin(wa) + 0.3 * sin(wb) + 0.15 * sin(wc)));
            vv = row - wt * cards[ci].w_v * sin(wb + 1.3);
            if !(vv >= 0.0 && vv < 1.0) { continue; }
        }
        if !(u >= 0.0 && u < 1.0) { continue; }
        if cards[ci].flip != 0u { u = 1.0 - u; }
        let nearest = cards[ci].nearest != 0u;
        let s = sprite_sample(cards[ci].soff, cards[ci].sw, cards[ci].sh, u, vv, nearest);
        var a = s.w;
        if nearest { a = select(0.0, 1.0, s.w >= 0.5); }
        if a < 0.02 { continue; }
        var glow = cards[ci].emissive;
        if cards[ci].goff != 0xFFFFFFFFu { glow += sprite_sample(cards[ci].goff, cards[ci].gw, cards[ci].gh, u, vv, nearest).x; }
        let light = vec3<f32>(cards[ci].light_r, cards[ci].light_g, cards[ci].light_b);
        var lit = s.xyz * light + s.xyz * glow;
        if cards[ci].snow_on != 0u && a >= 0.5 {
            lit = snow_on_card(ci, u, vv, lit);
        }
        var col: vec3<f32>;
        if cards[ci].plain != 0u { col = mix3(vec3<f32>(cards[ci].fog_r, cards[ci].fog_g, cards[ci].fog_b), lit, cards[ci].fog_t); }
        else { col = lit * cards[ci].fog_mul + vec3<f32>(cards[ci].add_r, cards[ci].add_g, cards[ci].add_b); }
        c = mix3(c, col, a);
        touched = true;
        if a >= 0.5 {
            g = GPix(cards[ci].z, cards[ci].wx, cards[ci].wy, cards[ci].wd, cards[ci].id | (cards[ci].realm << 8u));
            pk = cards[ci].source; wrote_pick = true;
        }
    }
    if touched {
        hdr[i * 3u] = c.x; hdr[i * 3u + 1u] = c.y; hdr[i * 3u + 2u] = c.z;
        gbuf[i] = g;
        if P.pick != 0u && wrote_pick { pick[i] = pk; }
    }
}
