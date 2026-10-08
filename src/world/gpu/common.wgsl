// Shared by every pass: the world's parameters, exact ports of the CPU renderer's helpers (hashes,
// noise, the view, fog), and the pixel buffers' layouts. Each function names the Rust function it
// mirrors; a change to one must be made to the other (pf parity --engine gpu checks them).

struct World {
    // View.
    width: u32, height: u32, horizon_px: f32, center_px: f32,
    focal_px: f32, eye_height: f32, bend: f32, hill: f32,
    half_width: f32, flare: f32, near_ground: f32, scroll: f32,
    top: f32, left: f32,
    // Stairs (StairProfile).
    stairs_on: u32, st_period: f32, st_run: f32, st_rise: f32, st_steps: f32, st_offset: f32,
    // Ctx.
    loop_len: f32, tphase: f32, far: f32, gain: f32,
    fog_on: u32, fog_dist: f32, fog_r: f32, fog_g: f32, fog_b: f32,
    void_r: f32, void_g: f32, void_b: f32,
    amb_r: f32, amb_g: f32, amb_b: f32,
    sky_top_r: f32, sky_top_g: f32, sky_top_b: f32,
    sky_hor_r: f32, sky_hor_g: f32, sky_hor_b: f32,
    // layers.sky && sky.enabled; fog matched to the sky; fog or fog banks (for the room's reflection).
    sky_on: u32, match_sky: u32, env_fog: u32,
    // Scene.
    edge_noise: f32, edge_dark: f32, verge_on: u32, walls_on: u32,
    walls_gap: f32, base_shadow: f32, wall_top: f32, ceiling_on: u32,
    bands: u32,
    // Materials: tile sizes; rotate bits (1 path, 2 verge, 4 walls, 8 ceiling, 16 deck); gloss and ripples.
    path_tile: f32, verge_tile: f32, wall_tile: f32, ceil_tile: f32, deck_tile: f32, bottom_tile: f32,
    rotate: u32,
    path_gloss: f32, path_rip: f32, verge_gloss: f32, verge_rip: f32, deck_gloss: f32, deck_rip: f32,
    // Bridge (Spans): bottom 0 ground, 1 water, 2 void.
    bridge_on: u32, br_period: f32, br_len: f32, br_offset: f32, br_floor: f32, br_bottom: u32,
    bottom_r: f32, bottom_g: f32, bottom_b: f32,
    deck_base_r: f32, deck_base_g: f32, deck_base_b: f32,
    rail_r: f32, rail_g: f32, rail_b: f32,
    // Forks: side 0 left, 1 right, 2 alternate.
    fork_on: u32, fk_period: f32, fk_offset: f32, fk_side: u32, fk_tan: f32, fk_hw: f32, fk_per_loop: u32,
    // Fog banks.
    fb_on: u32, fb_density: f32, fb_spacing: f32, fb_length: f32, fb_offset: f32,
    // Weather (Wx) and precipitation.
    wx_wet: f32, wx_puddles: f32, wx_snow: f32, wx_track: f32, wx_rings: f32,
    precip_seed: u32, loop_seconds: f32,
    // Lights: counts, and whether per-tile lists are used (cols of 16 x 16 tiles).
    n_lights: u32, n_sky: u32, tiles_on: u32, tile_cols: u32,
    zero: u32, fk_style: u32, fk_noise: f32, tile_cap: u32,
    // A frame of several worlds (WorldParams).
    realm: u32, tex_base: u32, light_base: u32, multi: u32,
    sp_on: u32, sp_zb: f32, sp_hw_b: f32, sp_flare_b: f32, sp_taper: f32,
    sa_on: u32, sa_period: f32, sa_run: f32, sa_rise: f32, sa_steps: f32, sa_offset: f32, sa_scroll: f32,
    sb_on: u32, sb_period: f32, sb_run: f32, sb_rise: f32, sb_steps: f32, sb_offset: f32, sb_scroll: f32,
    sh_z0_0: f32, sh_z0_1: f32, sh_z0_2: f32, sh_k_0: f32, sh_k_1: f32, sh_k_2: f32,
    fa_on: u32, fa_zb: f32, fa_fog_on: u32, fa_fog_dist: f32, fa_r: f32, fa_g: f32, fa_b: f32, fa_gain: f32,
    pt_on: u32, pt_zb: f32, pt_front: u32, pt_r: f32, pt_g: f32, pt_b: f32,
    pt0x: f32, pt0y: f32, pt0z: f32, pt1x: f32, pt1y: f32, pt1z: f32, pt2x: f32, pt2y: f32, pt2z: f32, pt3x: f32, pt3y: f32, pt3z: f32,
    op_on: u32, op_hw: f32, op_top: f32, op_arch: f32, op_rim: f32, op_seed: u32, op_outer_hw: f32, op_outer_top: f32, op_hill: f32,
    fac_tile: f32,
    vb_on: u32, vb_z: f32,
    wg0a: f32, wg0b: f32, wg0c: f32, wg1a: f32, wg1b: f32, wg1c: f32, wg2a: f32, wg2b: f32, wg2c: f32, wg3a: f32, wg3b: f32, wg3c: f32,
    el_on: u32, el_r: f32, el_g: f32, el_b: f32, el_hw: f32, el_inset: f32, el_period: f32, el_phase: f32,
    // The path as a waterway (render::water::WaterK).
    ww_on: u32, ww_shift: f32, ww_foam: f32, ww_foam_r: f32, ww_foam_g: f32, ww_foam_b: f32, ww_foam_w: f32,
    ww_lap_ph: f32, ww_lap_k: f32, ww_k1: f32, ww_k2: f32, ww_wet: f32,
    ww_kind: u32, ww_dens: f32, ww_fr: f32, ww_fg: f32, ww_fb: f32, ww_fsize: f32, ww_period: f32, ww_cs: f32, ww_seed: u32,
}

// Every pass binds the world's parameters here.
@group(0) @binding(0) var<uniform> W: World;
// Whether the pipeline draws frames of several worlds; false compiles out what only those need
// (boundaries, branches' shear, the air in front of a boundary, light through a threshold).
override MULTI: bool = true;

// One G-buffer pixel, as raster::GPixel: depth, world x, y, d, and id | realm << 8.
struct GPix { depth: f32, x: f32, y: f32, d: f32, idr: u32 }

// raster::id
const ID_NONE: u32 = 0u;
const ID_GROUND: u32 = 1u;
const ID_WALL_L: u32 = 2u;
const ID_WALL_R: u32 = 3u;
const ID_CEILING: u32 = 4u;
const ID_RISER: u32 = 5u;
const ID_CHASM: u32 = 6u;
const ID_CLIFF: u32 = 7u;
const ID_PROP: u32 = 10u;
const ID_FIXTURE: u32 = 11u;
const ID_TUFT: u32 = 12u;
const ID_FACADE: u32 = 13u;
const ID_RAIL: u32 = 20u;
const RAIL_STONE: u32 = 0u;
const RAIL_WOOD: u32 = 1u;
const RAIL_PAINT: u32 = 2u;
const FACE_INNER: u32 = 0u;
const FACE_OUTER: u32 = 1u;
const FACE_TOP: u32 = 2u;
const FACE_FRONT: u32 = 3u;

fn is_rail(id: u32) -> bool { return id >= ID_RAIL && id < ID_RAIL + 12u; }

const PI: f32 = 3.14159265358979;
const TAU: f32 = 6.28318530717959;

// ── Rust's float semantics ───────────────────────────────────────────────

// x / y rounded as the CPU rounds it. Metal compiles shaders with fast math, so `/` can be an ulp
// or two off, which moves a texture lookup to the next texel at texel edges. One fma step from
// the fast quotient recovers the correctly rounded one.
fn div(x: f32, y: f32) -> f32 {
    let q = x / y;
    let r = fma(-q, y, x);
    return fma(r, 1.0 / y, q);
}

// f32::round: halves away from zero (WGSL's round goes to even).
fn rround(x: f32) -> f32 { return sign(x) * floor(abs(x) + 0.5); }
// f32::rem_euclid.
fn rem_e(x: f32, y: f32) -> f32 { let r = fma(-y, trunc(div(x, y)), x); return select(r, r + abs(y), r < 0.0); }
fn rem_ei(x: i32, n: i32) -> i32 { let r = x % n; return select(r, r + abs(n), r < 0); }
fn smoothstep_r(e0: f32, e1: f32, x: f32) -> f32 { let t = clamp((x - e0) / (e1 - e0), 0.0, 1.0); return t * t * (3.0 - 2.0 * t); }
// `x` rounded to f32 right here: the uniform `zero` (always 0) is opaque to the compiler, so a
// product passed through this cannot be fused into a following add (Metal's fast math contracts
// a * b + c into an fma; the CPU rounds the product first). Use it where a threshold follows.
fn rnd(x: f32) -> f32 { return bitcast<f32>(bitcast<u32>(x) | W.zero); }
fn mix3(a: vec3<f32>, b: vec3<f32>, t: f32) -> vec3<f32> { return a + (b - a) * t; }

// ── 64-bit integer arithmetic from 32-bit halves (x = lo, y = hi) ───────────

// The full 64-bit product of two u32.
fn mul32(a: u32, b: u32) -> vec2<u32> {
    let a0 = a & 0xFFFFu; let a1 = a >> 16u;
    let b0 = b & 0xFFFFu; let b1 = b >> 16u;
    let p00 = a0 * b0; let p01 = a0 * b1; let p10 = a1 * b0; let p11 = a1 * b1;
    let mid = (p00 >> 16u) + (p01 & 0xFFFFu) + (p10 & 0xFFFFu);
    let lo = (p00 & 0xFFFFu) | (mid << 16u);
    let hi = p11 + (p01 >> 16u) + (p10 >> 16u) + (mid >> 16u);
    return vec2<u32>(lo, hi);
}
// a * b mod 2^64.
fn mul64(a: vec2<u32>, b: vec2<u32>) -> vec2<u32> {
    let p = mul32(a.x, b.x);
    return vec2<u32>(p.x, p.y + a.x * b.y + a.y * b.x);
}
fn xor64(a: vec2<u32>, b: vec2<u32>) -> vec2<u32> { return a ^ b; }
// a >> s for 0 < s < 32.
fn shr64(a: vec2<u32>, s: u32) -> vec2<u32> { return vec2<u32>((a.x >> s) | (a.y << (32u - s)), a.y >> s); }
fn i64_of(k: i32) -> vec2<u32> { return vec2<u32>(bitcast<u32>(k), select(0u, 0xFFFFFFFFu, k < 0)); }

// render::hash
fn hash(seed: u32, k: i32) -> u32 {
    var h = xor64(mul64(i64_of(k), vec2<u32>(0x7F4A7C15u, 0x9E3779B9u)), mul64(vec2<u32>(seed, 0u), vec2<u32>(0x27D4EB4Fu, 0xC2B2AE3Du)));
    h = xor64(h, shr64(h, 31u));
    h = mul64(h, vec2<u32>(0x1CE4E5B9u, 0xBF58476Du));
    h = xor64(h, shr64(h, 29u));
    // (h >> 16) as u32
    return (h.x >> 16u) | (h.y << 16u);
}
// render::hf
fn hf(seed: u32, k: i32) -> f32 { return f32(hash(seed, k) >> 8u) / 16777216.0; }

// render::snap_to_loop
fn snap_to_loop(len: f32, loop_len: f32) -> f32 { let n = max(rround(div(loop_len, max(len, 0.01))), 1.0); return div(loop_len, n); }
// render::cycles
fn cycles(per_second: f32, loop_seconds: f32) -> f32 { return max(rround(per_second * loop_seconds), 1.0); }

// render::path_noise
fn path_noise(dw: f32, loop_len: f32, cell: f32, seed: u32) -> f32 {
    let c = snap_to_loop(cell, loop_len);
    let n = i32(rround(div(loop_len, c)));
    let t = div(dw, c);
    let i = i32(floor(t));
    var f = t - f32(i);
    f = f * f * (3.0 - 2.0 * f);
    let a = hf(seed, rem_ei(i, n));
    let b = hf(seed, rem_ei(i + 1, n));
    return a + (b - a) * f;
}

// weather::noise2
fn noise2(x: f32, w: f32, cell: f32, loop_len: f32, seed: u32) -> f32 {
    let c = snap_to_loop(cell, loop_len);
    let n = i32(max(rround(div(loop_len, c)), 1.0));
    let tx = div(x, cell); let tw = div(w, c);
    let ix = i32(floor(tx)); let iw = i32(floor(tw));
    let sx = tx - f32(ix); let sw = tw - f32(iw);
    let fx = sx * sx * (3.0 - 2.0 * sx); let fw = sw * sw * (3.0 - 2.0 * sw);
    let w0 = rem_ei(iw, n);
    let w1 = select(w0 + 1, 0, w0 + 1 == n);
    let s0 = seed ^ (bitcast<u32>(ix) * 0x9E3779B1u);
    let s1 = seed ^ (bitcast<u32>(ix + 1) * 0x9E3779B1u);
    let a = hf(s0, w0); let b = hf(s1, w0);
    let c0 = hf(s0, w1); let d0 = hf(s1, w1);
    let top = a + (b - a) * fx;
    let bot = c0 + (d0 - c0) * fx;
    return top + (bot - top) * fw;
}

// weather::drifting_noise
fn drifting_noise(x: f32, w: f32, cell: f32, travel: f32, seed: u32) -> f32 {
    var n = 0.0;
    for (var j = 0u; j < 2u; j++) {
        let ph = rem_e(W.tphase + f32(j) * 0.5, 1.0);
        let sn = sin(PI * ph);
        let wt = sn * sn;
        let xo = x - rnd(travel * (ph - 0.5)) + f32(j) * 53.0;
        n += wt * (0.7 * noise2(xo, w, cell, W.loop_len, seed ^ 0x51u ^ j) + 0.3 * noise2(xo, w, rnd(cell * 0.37), W.loop_len, seed ^ 0x53u ^ j));
    }
    return n;
}

// ── The view (view.rs) ──────────────────────────────────────────────────

// StairProfile (period, run, rise, steps, offset) for any of a frame's stairs.
struct Stairs { period: f32, run: f32, rise: f32, steps: f32, offset: f32 }
fn stp_split(s: Stairs, w: f32) -> vec2<f32> {
    let y = w - s.offset;
    let k = floor(div(y, s.period));
    return vec2<f32>(k, y - k * s.period);
}
fn stp_ground(s: Stairs, w: f32) -> f32 {
    let ku = stp_split(s, w);
    return ku.x * s.steps * s.rise + s.rise * min(floor(div(ku.y, s.run)) + 1.0, s.steps);
}
fn stp_ramp(s: Stairs, w: f32) -> f32 {
    let ku = stp_split(s, w);
    return ku.x * s.steps * s.rise + s.steps * s.rise * clamp(ku.y / (s.steps * s.run), 0.0, 1.0);
}
fn own_stairs() -> Stairs { return Stairs(W.st_period, W.st_run, W.st_rise, W.st_steps, W.st_offset); }
fn st_split(w: f32) -> vec2<f32> { return stp_split(own_stairs(), w); }
fn st_ground(w: f32) -> f32 { return stp_ground(own_stairs(), w); }
fn st_ramp(w: f32) -> f32 { return stp_ramp(own_stairs(), w); }
fn st_since_riser(w: f32) -> f32 {
    let u = st_split(w).y;
    let flight = W.st_steps * W.st_run;
    return select(u - flight + W.st_run, rem_e(u, W.st_run), u < flight);
}
fn st_to_next_riser(w: f32) -> f32 {
    let u = st_split(w).y;
    let flight = W.st_steps * W.st_run;
    return select(W.st_period - u, W.st_run - rem_e(u, W.st_run), u < flight - W.st_run);
}
// Split: the ground of the worlds either side of a boundary, joined (Split::lift).
fn split_ga(w: f32) -> f32 { if W.sa_on == 0u { return 0.0; } return stp_ground(Stairs(W.sa_period, W.sa_run, W.sa_rise, W.sa_steps, W.sa_offset), w); }
fn split_gb(w: f32) -> f32 { if W.sb_on == 0u { return 0.0; } return stp_ground(Stairs(W.sb_period, W.sb_run, W.sb_rise, W.sb_steps, W.sb_offset), w); }
fn split_lift(d: f32) -> f32 {
    let c = split_ga(W.sa_scroll + W.sp_zb) - split_gb(W.sb_scroll + W.sp_zb);
    var g: f32;
    if d < W.sp_zb { g = split_ga(W.sa_scroll + d); } else { g = split_gb(W.sb_scroll + d) + c; }
    var eye = 0.0;
    if W.sp_zb > 0.0 {
        if W.sa_on != 0u { eye = stp_ramp(Stairs(W.sa_period, W.sa_run, W.sa_rise, W.sa_steps, W.sa_offset), W.sa_scroll); }
    } else {
        if W.sb_on != 0u { eye = stp_ramp(Stairs(W.sb_period, W.sb_run, W.sb_rise, W.sb_steps, W.sb_offset), W.sb_scroll); }
        eye += c;
    }
    return g - eye;
}
fn lift(d: f32) -> f32 {
    if MULTI && W.sp_on != 0u { return split_lift(d); }
    if W.stairs_on == 0u { return 0.0; }
    return st_ground(W.scroll + d) - st_ramp(W.scroll);
}
fn bend_x(d: f32) -> f32 { return W.bend * 0.012 * d * d; }
fn hill_y(d: f32) -> f32 { return W.hill * 0.006 * d * d; }
// View::shear_x for world r of the frame.
fn shear_of(r: u32, d: f32) -> f32 {
    var z0 = W.sh_z0_0; var k = W.sh_k_0;
    if r == 1u { z0 = W.sh_z0_1; k = W.sh_k_1; } else if r == 2u { z0 = W.sh_z0_2; k = W.sh_k_2; }
    return k * max(d - z0, 0.0);
}
fn shear_x(d: f32) -> f32 { if !MULTI { return 0.0; } return shear_of(W.realm, d); }
fn flared(hw: f32, flare: f32, d: f32) -> f32 {
    if flare <= 0.0 { return hw; }
    return hw * pow(max(d, W.near_ground) / W.near_ground, flare);
}
fn path_half_width(d: f32) -> f32 {
    let a = flared(W.half_width, W.flare, d);
    if !MULTI || W.sp_on == 0u { return a; }
    var t: f32;
    if W.sp_taper > 0.0 {
        let u = clamp((d - W.sp_zb + W.sp_taper) / (2.0 * W.sp_taper), 0.0, 1.0);
        t = u * u * (3.0 - 2.0 * u);
    } else { t = select(1.0, 0.0, d < W.sp_zb); }
    if t <= 0.0 { return a; }
    return a + (flared(W.sp_hw_b, W.sp_flare_b, d) - a) * t;
}
fn to_cam(x: f32, y: f32, d: f32) -> vec3<f32> {
    return vec3<f32>(x + bend_x(d) + shear_x(d), y - W.eye_height + hill_y(d) + lift(d), d);
}

// ── Ctx helpers (render.rs) ─────────────────────────────────────────────

fn wall_x(d: f32) -> f32 { return path_half_width(d) + max(W.walls_gap, 0.0); }
fn path_edge(d: f32) -> f32 {
    let dw = d + W.scroll;
    let n = path_noise(dw, W.loop_len, 0.9, 101u) * 0.65 + path_noise(dw, W.loop_len, 0.3, 202u) * 0.35;
    return path_half_width(d) + W.edge_noise * (n * 2.0 - 1.0);
}
fn on_bridge(d: f32) -> bool {
    return W.bridge_on != 0u && rem_e(W.scroll + d - W.br_offset, W.br_period) < W.br_len;
}
fn fork_side_of(k: i32) -> f32 {
    if W.fk_side == 0u || W.fk_side == 3u { return -1.0; }
    if W.fk_side == 1u { return 1.0; }
    return select(1.0, -1.0, rem_ei(k, i32(W.fk_per_loop)) % 2 == 0);
}
// Forks::sides: the second is 0 when a fork leaves on one side only.
fn fork_sides(k: i32) -> vec2<f32> {
    if W.fk_side == 3u { return vec2<f32>(-1.0, 1.0); }
    return vec2<f32>(fork_side_of(k), 0.0);
}
// Forks::on_branch (the windowed form).
fn on_branch(x: f32, w: f32, main_hw: f32) -> bool {
    let reach = 300.0 / W.fk_tan;
    let norm = sqrt(1.0 + W.fk_tan * W.fk_tan);
    let k0 = i32(floor(div(w - reach - W.fk_offset, W.fk_period)));
    let k1 = i32(floor(div(w + W.fk_hw * 2.0 - W.fk_offset, W.fk_period)));
    let ts = (abs(x) - main_hw * 0.5) / W.fk_tan;
    let spread = W.fk_hw * norm / W.fk_tan;
    let lo = array<f32, 2>(w - ts - spread, w - W.fk_hw);
    let hi = array<f32, 2>(w - ts + spread, w + W.fk_hw);
    for (var win = 0; win < 2; win++) {
        let a = max(i32(floor(div(lo[win] - W.fk_offset, W.fk_period))) - 1, k0);
        let b = min(i32(floor(div(hi[win] - W.fk_offset, W.fk_period))) + 1, k1);
        for (var k = a; k <= b; k++) {
            let j = W.fk_offset + f32(k) * W.fk_period;
            let sides = fork_sides(k);
            for (var si = 0; si < 2; si++) {
                let side = sides[si];
                let t = w - j;
                if side == 0.0 || t < -W.fk_hw || x * side <= 0.0 { continue; }
                let x0 = side * main_hw * 0.5;
                var dist: f32;
                if t >= 0.0 { dist = abs((x - x0) - side * W.fk_tan * t) / norm; } else { dist = sqrt((x - x0) * (x - x0) + t * t); }
                if dist < W.fk_hw { return true; }
            }
        }
    }
    return false;
}
fn smin(a: f32, b: f32, k: f32) -> f32 {
    let h = max(k - abs(a - b), 0.0) / k;
    return min(a, b) - h * h * k * 0.25;
}
// Forks::branch_sd (the split style).
fn branch_sd(x: f32, w: f32, main_hw: f32) -> f32 {
    let r = div(main_hw + W.fk_hw, W.fk_tan);
    let norm = sqrt(1.0 + W.fk_tan * W.fk_tan);
    let cap = W.fk_hw + W.fk_noise + W.fk_hw;
    let e = cap * norm;
    let reach = 300.0 / W.fk_tan;
    let v_lo = abs(x) - e; let v_hi = abs(x) + e;
    var t_lo = 0.0;
    if v_lo > 0.0 { let a = div(v_lo, W.fk_tan) + r; t_lo = sqrt(a * a - r * r); }
    let a_hi = div(v_hi, W.fk_tan) + r;
    let t_hi = min(sqrt(a_hi * a_hi - r * r), reach);
    let lo = w - t_hi;
    let hi = w - t_lo + select(0.0, cap, t_lo == 0.0);
    var best = bitcast<f32>(0x7F800000u);
    let k0 = i32(floor(div(lo - W.fk_offset, W.fk_period))) - 1;
    let k1 = i32(floor(div(hi - W.fk_offset, W.fk_period))) + 1;
    for (var kk = k0; kk <= k1; kk++) {
        let t = w - (W.fk_offset + f32(kk) * W.fk_period);
        if t < -cap || t > reach { continue; }
        let sides = fork_sides(kk);
        for (var si = 0; si < 2; si++) {
            let side = sides[si];
            if side == 0.0 || x * side <= 0.0 { continue; }
            var dist: f32;
            if t >= 0.0 {
                let q = sqrt(t * t + r * r);
                let xc = side * W.fk_tan * (q - r);
                let slope = div(W.fk_tan * t, q);
                dist = abs(x - xc) / sqrt(1.0 + slope * slope);
            } else {
                dist = sqrt(x * x + t * t);
            }
            if dist - W.fk_hw - W.fk_noise >= best { continue; }
            let seed = 303u + 7u * u32(rem_ei(kk, i32(W.fk_per_loop))) + select(0u, 3u, side > 0.0);
            let tn = max(t, 0.0);
            let n = path_noise(tn, 1024.0, 0.9, seed) * 0.65 + path_noise(tn, 1024.0, 0.3, seed + 1u) * 0.35;
            best = min(best, dist - (W.fk_hw + W.fk_noise * (n * 2.0 - 1.0)));
        }
    }
    return best;
}
// Ctx::split_field: (main road, branch roads, both together).
fn split_field(x: f32, d: f32) -> vec3<f32> {
    let m = abs(x) - path_edge(d);
    let b = branch_sd(x, W.scroll + d, path_half_width(d));
    return vec3<f32>(m, b, smin(m, b, W.fk_hw));
}
// Ctx::split_edge_dark.
fn split_edge_dark(x: f32, d: f32) -> f32 {
    let f = split_field(x, d);
    let edge = max(path_edge(d), 0.05);
    let rm = 0.5 * edge; let rb = 0.5 * W.fk_hw;
    let n = smin(f.x / rm, f.y / rb, W.fk_hw / min(rm, rb));
    return 1.0 - clamp(W.edge_dark, 0.0, 1.0) * (1.0 - smoothstep_r(0.0, 1.0, -n));
}
fn on_fork(x: f32, d: f32) -> bool {
    if W.fork_on == 0u { return false; }
    if W.walls_on != 0u { return abs(x) > wall_x(d) - 1e-3; }
    if W.fk_style != 0u { let f = split_field(x, d); return f.z < 0.0 && f.z < f.x; }
    return on_branch(x, W.scroll + d, path_half_width(d));
}
fn passage_dark(x: f32, d: f32) -> f32 {
    if W.fork_on == 0u || W.walls_on == 0u || W.ceiling_on == 0u { return 1.0; }
    let beyond = abs(x) - wall_x(d);
    return select(1.0, 0.15 + 0.85 * exp(-beyond / 1.2), beyond > 0.0);
}

// ── Fog (and a frame of several worlds' air in front of a boundary) ─────────────────

fn bank_transmittance(z: f32) -> f32 {
    if W.fb_on == 0u || W.fb_density <= 0.0 { return 1.0; }
    let sp = snap_to_loop(max(W.fb_spacing, 1.0), W.loop_len);
    let len = clamp(W.fb_length, 0.0, sp);
    let y1 = W.scroll + z - W.fb_offset;
    let y0 = W.scroll - W.fb_offset;
    let c1 = floor(div(y1, sp)) * len + min(rem_e(y1, sp), len);
    let c0 = floor(div(y0, sp)) * len + min(rem_e(y0, sp), len);
    return exp(-W.fb_density * (c1 - c0));
}
fn bank_t(z: f32) -> f32 { return bank_transmittance(clamp(z, 0.0, W.far)); }
// Ctx::own_fog_t
fn own_fog_t(len: f32, start: f32) -> f32 {
    var even = 1.0;
    if W.fog_on != 0u { even = exp(-min(len, 1.0e4) / W.fog_dist); }
    var banks: f32;
    if start > 0.0 { banks = bank_t(start + len) / max(bank_t(start), 1e-6); } else { banks = bank_t(len); }
    return even * banks;
}
fn front_on() -> bool { return MULTI && W.fa_on != 0u && W.fa_zb > 0.0; }
fn front_t(near: f32) -> f32 { if W.fa_fog_on != 0u { return exp(-near / W.fa_fog_dist); } return 1.0; }
fn front_col() -> vec3<f32> { return vec3<f32>(W.fa_r, W.fa_g, W.fa_b) * W.fa_gain; }
// Ctx::fog_t
fn fog_t(z: f32) -> f32 {
    if !front_on() { return own_fog_t(z, 0.0); }
    let near = min(z, W.fa_zb);
    return front_t(near) * own_fog_t(z - near, near);
}
fn fog_col() -> vec3<f32> { return vec3<f32>(W.fog_r, W.fog_g, W.fog_b); }
// Ctx::apply_fog
fn apply_fog(c: vec3<f32>, z: f32) -> vec3<f32> {
    if !front_on() {
        let t = fog_t(z);
        if t >= 1.0 { return c * W.gain; }
        return mix3(fog_col() * W.gain, c * W.gain, t);
    }
    let near = min(z, W.fa_zb);
    let own = own_fog_t(z - near, near);
    var c2 = c * W.gain;
    if own < 1.0 { c2 = mix3(fog_col() * W.gain, c * W.gain, own); }
    let t = front_t(near);
    if t >= 1.0 { return c2; }
    return mix3(front_col(), c2, t);
}
// render::sky_seen: a world's sky at its adaptation, through the first world's air.
fn sky_seen(c: vec3<f32>) -> vec3<f32> {
    let g = c * W.gain;
    if front_on() && W.fa_fog_on != 0u { return mix3(front_col(), g, exp(-W.fa_zb / W.fa_fog_dist)); }
    return g;
}

// render::sky_base for one world.
fn sky_base(y: f32) -> vec3<f32> {
    let hy = max(W.horizon_px, 1.0);
    let st = vec3<f32>(W.sky_top_r, W.sky_top_g, W.sky_top_b);
    let sh = vec3<f32>(W.sky_hor_r, W.sky_hor_g, W.sky_hor_b);
    if W.sky_on != 0u && y < hy {
        return mix3(st, sh, pow(clamp((y - W.top) / max(hy - W.top, 1.0), 0.0, 1.0), 1.3));
    } else if W.sky_on != 0u && W.fog_on != 0u && W.match_sky != 0u {
        return sh;
    }
    return vec3<f32>(W.void_r, W.void_g, W.void_b);
}

// render::noise1
fn noise1(seed: u32, x: f32) -> f32 {
    let i = i32(floor(x));
    var f = x - f32(i);
    f = f * f * (3.0 - 2.0 * f);
    let a = hf(seed, i); let b = hf(seed, i + 1);
    return (a + (b - a) * f) * 2.0 - 1.0;
}

// Opening (a threshold's opening in its face): half_width, top_at, rim_distance.
fn op_half_width() -> f32 { return W.op_hw * (1.0 - 0.1 * W.op_rim); }
fn op_top_at(x: f32) -> f32 {
    let hw = op_half_width();
    if abs(x) >= hw { return 0.0; }
    let u = x / hw;
    let arch = pow(max(1.0 - u * u, 0.0), 0.45);
    let t = W.op_top * ((1.0 - W.op_arch) + W.op_arch * arch);
    let rough = 1.0 + W.op_rim * (0.08 * noise1(W.op_seed ^ 0x51u, x * 2.2) + 0.04 * noise1(W.op_seed ^ 0x52u, x * 7.0));
    return t * rough;
}
fn op_rim_distance(x: f32, y: f32) -> f32 {
    let hw = op_half_width();
    if abs(x) < hw { return max(y - op_top_at(x), 0.0); }
    let edge_top = op_top_at(sign(x) * (hw - 1e-3));
    let dy = max(y - edge_top, 0.0);
    return sqrt((abs(x) - hw) * (abs(x) - hw) + dy * dy);
}

// Bounds::region_of(x, z) == 0 past a fork's junction: inside the wedge's four half-planes.
fn in_wedge(x: f32, z: f32) -> bool {
    return W.wg0a * x + W.wg0b * z + W.wg0c >= 0.0 && W.wg1a * x + W.wg1b * z + W.wg1c >= 0.0
        && W.wg2a * x + W.wg2b * z + W.wg2c >= 0.0 && W.wg3a * x + W.wg3b * z + W.wg3c >= 0.0;
}

// looks::hash2 / value_noise
fn hash2(x: i32, y: i32) -> f32 {
    var h = (bitcast<u32>(x) * 0x8da6b343u) ^ (bitcast<u32>(y) * 0xd8163841u) ^ 0x9e3779b9u;
    h ^= h >> 15u; h = h * 0x2c1b3c6du; h ^= h >> 12u;
    return f32(h & 0xffffu) / 65535.0;
}
fn value_noise(x: f32, y: f32) -> f32 {
    let xi = i32(floor(x)); let yi = i32(floor(y));
    let fx = x - f32(xi); let fy = y - f32(yi);
    let sx = fx * fx * (3.0 - 2.0 * fx); let sy = fy * fy * (3.0 - 2.0 * fy);
    let a = hash2(xi, yi) + (hash2(xi + 1, yi) - hash2(xi, yi)) * sx;
    let b = hash2(xi, yi + 1) + (hash2(xi + 1, yi + 1) - hash2(xi, yi + 1)) * sx;
    return a + (b - a) * sy;
}

// ── double-float arithmetic (every step rounded where written: see rnd) ─────────────────────
fn two_sum(a: f32, b: f32) -> vec2<f32> {
    let s = rnd(a + b);
    let bb = rnd(s - a);
    return vec2<f32>(s, rnd(a - rnd(s - bb)) + rnd(b - bb));
}
fn df_norm(s: f32, e: f32) -> vec2<f32> {
    let hi = rnd(s + e);
    return vec2<f32>(hi, rnd(e - rnd(hi - s)));
}
fn df_add(x: vec2<f32>, y: vec2<f32>) -> vec2<f32> {
    let s = two_sum(x.x, y.x);
    let t = two_sum(x.y, y.y);
    let a = df_norm(s.x, rnd(s.y + t.x));
    return df_norm(a.x, rnd(a.y + t.y));
}
fn df_neg(x: vec2<f32>) -> vec2<f32> { return -x; }
// a * b exactly, as hi + lo.
fn two_prod(a: f32, b: f32) -> vec2<f32> {
    let p = rnd(a * b);
    return vec2<f32>(p, fma(a, b, -p));
}
// x * b for a double-float x and an f32 b.
fn df_mul_f(x: vec2<f32>, b: f32) -> vec2<f32> {
    let p = two_prod(x.x, b);
    return df_norm(p.x, rnd(p.y + rnd(x.y * b)));
}
fn df_sq(x: vec2<f32>) -> vec2<f32> {
    let p = two_prod(x.x, x.x);
    return df_norm(p.x, rnd(p.y + rnd(2.0 * rnd(x.x * x.y))));
}
fn df_to_f(x: vec2<f32>) -> f32 { return x.x + x.y; }
// x / n, rounded to f32.
fn df_div_f(x: vec2<f32>, n: f32) -> f32 {
    let q = div(x.x, n);
    let r = df_add(x, df_neg(two_prod(q, n)));
    return q + div(r.x + r.y, n);
}

// x / n as a double-float.
fn df_div(x: vec2<f32>, n: f32) -> vec2<f32> {
    let q = div(x.x, n);
    let r = df_add(x, df_neg(two_prod(q, n)));
    return df_norm(q, div(r.x + r.y, n));
}
