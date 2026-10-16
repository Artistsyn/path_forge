// Billboards (render::Card::draw_rows), one thread per pixel: each pixel walks its tile's cards
// far to near, the order the CPU draws them in, so depth tests and blends happen identically.

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
fn sample_above(cd: Card, u: f32, v: f32, dv: f32) -> vec4<f32> {
    if v - dv < 0.0 { return vec4<f32>(0.0); }
    return sprite_sample(cd.soff, cd.sw, cd.sh, u, v - dv, true);
}
fn luma4(c: vec4<f32>) -> f32 { return 0.3 * c.x + 0.59 * c.y + 0.11 * c.z; }
fn snow_on_card(cd: Card, u: f32, v: f32, lit: vec3<f32>) -> vec3<f32> {
    let snow = vec3<f32>(cd.snow_r, cd.snow_g, cd.snow_b);
    if sample_above(cd, u, v, cd.snow_cap).w < 0.5 { return snow; }
    if cd.snow_dust <= 0.0 { return lit; }
    let step = max(1.5 / max(cd.hp, 1.0), 0.012);
    let here = sprite_sample(cd.soff, cd.sw, cd.sh, u, v, true);
    let a1 = sample_above(cd, u, v, step);
    let a2 = sample_above(cd, u, v, 2.5 * step);
    var open = 0.0;
    if a1.w < 0.5 { open = 1.0; } else if a2.w < 0.5 { open = 0.7; }
    var ledge = 0.0;
    if a1.w >= 0.5 { ledge = clamp((luma4(here) - luma4(a1)) * 5.0, 0.0, 1.0); }
    let patchy = pow(0.5 + 0.5 * sin(37.0 * u + 23.0 * v + 3.1 * sin(13.0 * v)), 0.7);
    let d = cd.snow_dust;
    let blob = smoothstep_r(0.5, 0.7, 0.65 * snow_noise(5.0 * u, 8.0 * v) + 0.35 * snow_noise(11.0 * u + 3.1, 17.0 * v + 1.7));
    let k = min(d * max(open, ledge) * (0.55 + 0.45 * patchy) + d * d * (0.75 * blob + 0.2 * patchy) * (1.0 - 0.5 * v), 1.0);
    return mix3(lit, snow, k);
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
        let cd = cards[bins[ntiles + 1u + k]];
        if px < cd.x0 || px > cd.x1 || py < cd.y0 || py > cd.y1 { continue; }
        let row = div(f32(py) + 0.5 - cd.sy_top, cd.hp);
        if !(row >= 0.0 && row < 1.0) { continue; }
        var vv = row;
        if cd.z >= g.depth { continue; }
        let bend = rnd(rnd(cd.sway * (1.0 - vv)) * (1.0 - vv));
        var u = div(rnd(rnd(f32(px) + 0.5 - rnd(cd.sx - cd.wp * 0.5)) - bend), cd.wp);
        if cd.w_amp != 0.0 {
            // render::CardWind::at.
            var wt = smoothstep_r(cd.w_c1, cd.w_c0, row);
            if cd.w_trunk > 0.0 { wt *= 1.0 - (1.0 - smoothstep_r(cd.w_trunk, 4.0 * cd.w_trunk, abs(u - 0.5))) * smoothstep_r(cd.w_foot - 0.45, cd.w_foot, row); }
            let wh = cd.w_kv * row;
            let wa = TAU * (1.6 * u + 1.1 * wh) - cd.w_t1;
            let wb = TAU * (4.3 * u - 2.7 * wh) - cd.w_t2;
            let wc = TAU * (11.0 * u + 7.0 * wh) - cd.w_t1 - cd.w_t2;
            u -= wt * (cd.w_lean + cd.w_amp * (0.55 * sin(wa) + 0.3 * sin(wb) + 0.15 * sin(wc)));
            vv = row - wt * cd.w_v * sin(wb + 1.3);
            if !(vv >= 0.0 && vv < 1.0) { continue; }
        }
        if !(u >= 0.0 && u < 1.0) { continue; }
        if cd.flip != 0u { u = 1.0 - u; }
        let nearest = cd.nearest != 0u;
        let s = sprite_sample(cd.soff, cd.sw, cd.sh, u, vv, nearest);
        var a = s.w;
        if nearest { a = select(0.0, 1.0, s.w >= 0.5); }
        if a < 0.02 { continue; }
        var glow = cd.emissive;
        if cd.goff != 0xFFFFFFFFu { glow += sprite_sample(cd.goff, cd.gw, cd.gh, u, vv, nearest).x; }
        let light = vec3<f32>(cd.light_r, cd.light_g, cd.light_b);
        var lit = s.xyz * light + s.xyz * glow;
        if cd.snow_on != 0u && a >= 0.5 {
            lit = snow_on_card(cd, u, vv, lit);
        }
        var col: vec3<f32>;
        if cd.plain != 0u { col = mix3(vec3<f32>(cd.fog_r, cd.fog_g, cd.fog_b), lit, cd.fog_t); }
        else { col = lit * cd.fog_mul + vec3<f32>(cd.add_r, cd.add_g, cd.add_b); }
        c = mix3(c, col, a);
        touched = true;
        if a >= 0.5 {
            g = GPix(cd.z, cd.wx, cd.wy, cd.wd, cd.id | (cd.realm << 8u));
            pk = cd.source; wrote_pick = true;
        }
    }
    if touched {
        hdr[i * 3u] = c.x; hdr[i * 3u + 1u] = c.y; hdr[i * 3u + 2u] = c.z;
        gbuf[i] = g;
        if P.pick != 0u && wrote_pick { pick[i] = pk; }
    }
}
