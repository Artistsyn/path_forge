// The G-buffer, its shadow masks and the lamps each tile can see, for a frame of one world:
// - raster_main (then small_z_main, small_pick_main, resolve_main): raster::rasterize, with the
//   same edge and span arithmetic on triangles set up on the CPU as `raster::prepare` sets them up;
// - shadow_main: render::prop_shadows (contact darkening and sun shadows of billboards) and
//   weather::cloud_shade, into the sun and ambient-occlusion masks (also, once per world, in a
//   frame of several, its G-buffer drawn on the CPU: `Geom::shadow_mode`);
// - tiles_main: render::tile_lights, one workgroup per 16 x 16 tile: the camera-space box of what
//   it shows, and every lamp whose reach touches it, in index order.

// A prepared triangle (raster::Prepared): a, b, c on screen; 1/z at each; world position / z at
// each; the box min_x, max_x, min_y, max_y (as i32 bits); 1 / area; id | realm << 8; and per edge
// (a-b, b-c, c-a) its (ex, ey, s_opp).
struct Tri { f: array<f32, 36> }

// A shadow caster (render::prop_shadows' Caster): foot x, d, width axis x, d, ra, shadow, rb,
// flip; contact box on (bits), x0, y0, x1, y1; sun box on, x0, y0, x1, y1; strip (or below 0);
// sprite level off, w, h; then the picture's plane: c0 (3), across (3), down (3), 1/|across|^2,
// 1/|down|^2, normal (3), normal . sun. A fence's run (37: 1) has its shape where the sprite
// level would be: style | posts << 2 | pickets << 12, length, height; and 38: its posts' width.
// With strip -2, 40..46 map the face onto part of its picture (render::Panel::map).
struct Caster { f: array<f32, 48> }

struct Geom {
    tile_cols: u32, cast_bins: u32, sun_on: u32, n_lights: u32,
    sun_x: f32, sun_y: f32, sun_z: f32, clouds: f32,
    cloud_travel: f32, cloud_seed: u32, tiles_on: u32, cap: u32,
    small_off: u32, n_small: u32, fade_k: f32, fade_near: f32,
    fade_cut: f32,
    // A frame of several worlds runs shadow_main once per world (its casters, at W's dynamic
    // offset): bit 0, ground of another world is placed across W's path from the worlds table in
    // `lights` (render::prop_shadows' `across`); bit 1, it goes on from the masks so far; and
    // from bit 2, 0 for props and clouds, 1 for props only, 2 for the clouds of W's own pixels only.
    shadow_mode: u32, gx_off: u32, pad2: u32,
    pad3: vec4<u32>,
}

// raster::Dissolve::hides: a swinging camera dissolving the ceiling, the wall on its side, and
// fences and railings past the cut on that side.
fn hides(p: GPix, x: i32, y: i32) -> bool {
    if G.fade_k <= 0.0 { return false; }
    let id = p.idr & 0xFFu;
    let on = id == ID_CEILING || (id == ID_WALL_L && G.fade_near < 0.0) || (id == ID_WALL_R && G.fade_near > 0.0)
        || (id >= ID_RAIL && id < ID_RAIL + 16u && p.x * G.fade_near > G.fade_cut);
    return on && dissolved(x, y, G.fade_k);
}

@group(0) @binding(1) var<storage, read_write> gbuf: array<GPix>;
@group(0) @binding(2) var<storage, read> tris: array<Tri>;
// Per-tile lists: the triangles', then (from cast_bins) the casters'.
@group(0) @binding(3) var<storage, read> bins: array<u32>;
@group(0) @binding(4) var<storage, read_write> masks: array<f32>;
@group(0) @binding(5) var<storage, read> casters: array<Caster>;
@group(0) @binding(6) var<storage, read> texels: array<vec4<f32>>;
// Per tile: a count, then up to `cap` lamp indices.
@group(0) @binding(7) var<storage, read_write> tile_lights: array<u32>;
// (A frame of several worlds, for mix_main and shadow_main: render::Ctx::gpu_worlds' table, then
// render::Bounds::gpu_words.)
@group(0) @binding(8) var<storage, read> lights: array<f32>;
@group(0) @binding(9) var<uniform> G: Geom;
// raster_main's meeting place for big and small triangles: 3 words per pixel (see raster_main).
@group(0) @binding(10) var<storage, read_write> racc: array<atomic<u32>>;

const NONE: u32 = 0xFFFFFFFFu;
const INF_BITS: u32 = 0x7F800000u;

fn tf(t: u32, k: u32) -> f32 { return tris[t].f[k]; }
fn tint(t: u32, k: u32) -> i32 { return bitcast<i32>(tris[t].f[k]); }

// One edge's limit on the row's span (raster::rasterize): xl, xr narrowed, or empty (x > y).
fn edge_span(span: vec2<f32>, py: f32, e0x: f32, e0y: f32, ex: f32, ey: f32, s_opp: f32) -> vec2<f32> {
    let k = rnd(ex * (py - e0y)) + rnd(ey * e0x);
    if abs(ey) < 1e-12 {
        if rnd(k * s_opp) < 0.0 { return vec2<f32>(1.0, 0.0); }
        return span;
    }
    let x_cross = div(k, ey);
    if (ey > 0.0) == (s_opp > 0.0) { return vec2<f32>(span.x, min(span.y, x_cross)); }
    return vec2<f32>(max(span.x, x_cross), span.y);
}

// Row y's covered pixels xs..=xe of triangle t (raster_prepared's span); empty when xs > xe.
fn row_span(t: u32, y: i32) -> vec2<i32> {
    let min_x = tint(t, 18u); let max_x = tint(t, 19u);
    let py = f32(y) + 0.5;
    var span = vec2<f32>(f32(min_x), f32(max_x) + 1.0);
    span = edge_span(span, py, tf(t, 0u), tf(t, 1u), tf(t, 24u), tf(t, 25u), tf(t, 26u));
    if span.x > span.y { return vec2<i32>(1, 0); }
    span = edge_span(span, py, tf(t, 2u), tf(t, 3u), tf(t, 27u), tf(t, 28u), tf(t, 29u));
    if span.x > span.y { return vec2<i32>(1, 0); }
    span = edge_span(span, py, tf(t, 4u), tf(t, 5u), tf(t, 30u), tf(t, 31u), tf(t, 32u));
    if !(span.x < span.y) { return vec2<i32>(1, 0); }
    return vec2<i32>(i32(max(ceil(span.x - 0.5), f32(min_x))), min(i32(floor(span.y - 0.5)), max_x));
}

// Barycentrics b0, b1, b2 and the depth at pixel centre (px, py); depth +inf where it is behind.
fn bary(t: u32, px: f32, py: f32) -> vec4<f32> {
    let ax = tf(t, 0u); let ay = tf(t, 1u); let bx = tf(t, 2u); let by = tf(t, 3u); let cx = tf(t, 4u); let cy = tf(t, 5u);
    let inv_area = tf(t, 22u);
    let b0 = rnd(rnd(rnd((bx - px) * (cy - py)) - rnd((cx - px) * (by - py))) * inv_area);
    let b1 = rnd(rnd(rnd((cx - px) * (ay - py)) - rnd((ax - px) * (cy - py))) * inv_area);
    let b2 = rnd(rnd(1.0 - b0) - b1);
    let iz = rnd(rnd(rnd(b0 * tf(t, 6u)) + rnd(b1 * tf(t, 7u))) + rnd(b2 * tf(t, 8u)));
    if iz <= 0.0 { return vec4<f32>(0.0, 0.0, 0.0, bitcast<f32>(INF_BITS)); }
    return vec4<f32>(b0, b1, b2, div(1.0, iz));
}

fn tri_pix(t: u32, b: vec4<f32>) -> GPix {
    let z = b.w;
    let wx = rnd(rnd(rnd(rnd(b.x * tf(t, 9u)) + rnd(b.y * tf(t, 12u))) + rnd(b.z * tf(t, 15u))) * z);
    let wy = rnd(rnd(rnd(rnd(b.x * tf(t, 10u)) + rnd(b.y * tf(t, 13u))) + rnd(b.z * tf(t, 16u))) * z);
    let wd = rnd(rnd(rnd(rnd(b.x * tf(t, 11u)) + rnd(b.y * tf(t, 14u))) + rnd(b.z * tf(t, 17u))) * z);
    return GPix(z, wx, wy, wd, bitcast<u32>(tf(t, 23u)));
}

// The nearest triangle wins and, at equal depth, the earliest: raster_prepared's order. Big
// triangles are walked per pixel from the tile lists; small ones (G.n_small, listed at
// G.small_off in bins) each run as one thread over their own spans, so a few tiles holding
// thousands of far railing bars no longer make each of their pixels walk all of them. The two
// meet in `racc`: per pixel the least depth bits (positive f32 bits order as the floats do),
// the earliest small triangle at that depth, and the big pass's winner.
@compute @workgroup_size(8, 8)
fn raster_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= W.width || gid.y >= W.height { return; }
    let n = W.width * W.height;
    let i = gid.y * W.width + gid.x;
    let x = i32(gid.x); let y = i32(gid.y);
    let tile = (gid.y / 16u) * G.tile_cols + gid.x / 16u;
    let ntiles = G.tile_cols * ((W.height + 15u) / 16u);
    var g = GPix(bitcast<f32>(INF_BITS), 0.0, 0.0, 0.0, 0u);
    var win = NONE;
    let py = f32(y) + 0.5;
    let px = f32(x) + 0.5;
    for (var q = bins[tile]; q < bins[tile + 1u]; q++) {
        let t = bins[ntiles + 1u + q];
        if y < tint(t, 20u) || y > tint(t, 21u) || x < tint(t, 18u) || x > tint(t, 19u) { continue; }
        let s = row_span(t, y);
        if x < s.x || x > s.y { continue; }
        let b = bary(t, px, py);
        if b.w >= g.depth { continue; }
        let tp = tri_pix(t, b);
        if hides(tp, x, y) { continue; }
        g = tp;
        win = t;
    }
    gbuf[i] = g;
    if G.n_small > 0u {
        atomicStore(&racc[i], bitcast<u32>(g.depth));
        atomicStore(&racc[n + i], NONE);
        atomicStore(&racc[2u * n + i], win);
    }
}

fn small_tri(gid: vec3<u32>, nwg: vec3<u32>) -> u32 {
    let k = gid.y * nwg.x * 64u + gid.x;
    if k >= G.n_small { return NONE; }
    return bins[G.small_off + k];
}

// Each small triangle's least depth into racc[pixel].
@compute @workgroup_size(64)
fn small_z_main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) nwg: vec3<u32>) {
    let t = small_tri(gid, nwg);
    if t == NONE { return; }
    for (var y = tint(t, 20u); y <= tint(t, 21u); y++) {
        let s = row_span(t, y);
        for (var x = s.x; x <= s.y; x++) {
            let b = bary(t, f32(x) + 0.5, f32(y) + 0.5);
            let z = b.w;
            if z < bitcast<f32>(INF_BITS) && !hides(tri_pix(t, b), x, y) { atomicMin(&racc[u32(y) * W.width + u32(x)], bitcast<u32>(z)); }
        }
    }
}

// The earliest small triangle at each pixel's least depth.
@compute @workgroup_size(64)
fn small_pick_main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) nwg: vec3<u32>) {
    let t = small_tri(gid, nwg);
    if t == NONE { return; }
    let n = W.width * W.height;
    for (var y = tint(t, 20u); y <= tint(t, 21u); y++) {
        let s = row_span(t, y);
        for (var x = s.x; x <= s.y; x++) {
            let b = bary(t, f32(x) + 0.5, f32(y) + 0.5);
            let z = b.w;
            let p = u32(y) * W.width + u32(x);
            if z < bitcast<f32>(INF_BITS) && bitcast<u32>(z) == atomicLoad(&racc[p]) && !hides(tri_pix(t, b), x, y) { atomicMin(&racc[n + p], t); }
        }
    }
}

// Where a small triangle won, its pixel, recomputed as raster_main computes one.
@compute @workgroup_size(8, 8)
fn resolve_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= W.width || gid.y >= W.height { return; }
    let n = W.width * W.height;
    let i = gid.y * W.width + gid.x;
    let ws = atomicLoad(&racc[n + i]);
    if ws == NONE { return; }
    let wb = atomicLoad(&racc[2u * n + i]);
    if wb < ws && bitcast<u32>(gbuf[i].depth) == atomicLoad(&racc[i]) { return; }
    gbuf[i] = tri_pix(ws, bary(ws, f32(gid.x) + 0.5, f32(gid.y) + 0.5));
}

// render::Sprite::sample's alpha (bilinear, weighted by alpha), at a level picked on the CPU.
fn sprite_alpha(off: u32, w: u32, h: u32, u: f32, v: f32) -> f32 {
    let fx = clamp(rnd(u * f32(w)) - 0.5, 0.0, f32(w) - 1.0);
    let fy = clamp(rnd(v * f32(h)) - 0.5, 0.0, f32(h) - 1.0);
    let x0 = u32(fx); let y0 = u32(fy);
    let x1 = min(x0 + 1u, w - 1u); let y1 = min(y0 + 1u, h - 1u);
    let tx = fx - f32(x0); let ty = fy - f32(y0);
    var acc = rnd(texels[off + y0 * w + x0].w * rnd((1.0 - tx) * (1.0 - ty)));
    acc = rnd(acc + rnd(texels[off + y0 * w + x1].w * rnd(tx * (1.0 - ty))));
    acc = rnd(acc + rnd(texels[off + y1 * w + x0].w * rnd((1.0 - tx) * ty)));
    acc = rnd(acc + rnd(texels[off + y1 * w + x1].w * rnd(tx * ty)));
    if acc > 1e-6 { return acc; }
    return 0.0;
}

fn cf(c: u32, k: u32) -> f32 { return casters[c].f[k]; }
// render::dot3, its products rounded one by one as the CPU rounds them.
fn dot3g(a: vec3<f32>, b: vec3<f32>) -> f32 { return rnd(rnd(rnd(a.x * b.x) + rnd(a.y * b.y)) + rnd(a.z * b.z)); }
fn cu(c: u32, k: u32) -> u32 { return bitcast<u32>(casters[c].f[k]); }

// render::fence_box: how much of [x - h, x + h] lies in [lo, hi].
fn fence_box(lo: f32, hi: f32, x: f32, h: f32) -> f32 {
    if h < 1e-5 { return select(0.0, 1.0, x >= lo && x < hi); }
    return div(max(min(x + h, hi) - max(x - h, lo), 0.0), rnd(2.0 * h));
}

// render::fence_comb: how much of [x - h, x + h] (within [lo, hi]) bars `w` wide every `p`,
// one centred on `c`, cover.
fn fence_cum(x: f32, p: f32, w: f32, c: f32) -> f32 {
    let r = (x - c) + rnd(0.5 * p);
    let k = floor(div(r, p));
    return rnd(k * w) + clamp((r - rnd(k * p)) - rnd(0.5 * (p - w)), 0.0, w);
}
fn fence_comb(x: f32, h: f32, p: f32, w: f32, c: f32, lo: f32, hi: f32) -> f32 {
    if h < 1e-5 {
        if x < lo || x >= hi { return 0.0; }
        let r0 = (x - c) + rnd(0.5 * p);
        let r = r0 - rnd(p * floor(div(r0, p)));
        return select(0.0, 1.0, r >= rnd(0.5 * (p - w)) && r < rnd(0.5 * (p + w)));
    }
    let a = max(x - h, lo); let b = min(x + h, hi);
    if b <= a { return 0.0; }
    return div(fence_cum(b, p, w, c) - fence_cum(a, p, w, c), rnd(2.0 * h));
}

// render::FenceCut::alpha: how much of caster c's run of fence stands at (u, v), averaged over a
// box fu x fv wide.
fn fence_alpha(c: u32, u: f32, v: f32, fu: f32, fv: f32) -> f32 {
    let shape = cu(c, 19u);
    let style = shape & 3u;
    let posts = max((shape >> 2u) & 1023u, 1u);
    let n = max(shape >> 12u, 1u);
    let len = cf(c, 20u); let ht = cf(c, 21u); let pw = cf(c, 38u);
    let top = ht + 0.06;
    let span = len + pw;
    let s = rnd(u * span) - rnd(0.5 * pw);
    let y = rnd((1.0 - v) * top);
    let hs = rnd(rnd(0.5 * fu) * span);
    let hy = rnd(rnd(0.5 * fv) * top);
    let post = rnd(fence_comb(s, hs, div(len, f32(posts)), pw, 0.0, -rnd(0.5 * pw), len + rnd(0.5 * pw)) * fence_box(0.0, ht + 0.06, y, hy));
    let along = fence_box(0.0, len, s, hs);
    var upright = 0.0; var across = 0.0;
    if style == 0u {
        let p = div(len, f32(n));
        let body = rnd(fence_comb(s, hs, p, 0.074, rnd(0.5 * p), 0.0, len) * fence_box(0.04, ht - 0.05, y, hy));
        let tip = rnd(fence_comb(s, hs, p, 0.036, rnd(0.5 * p), 0.0, len) * fence_box(ht - 0.05, ht, y, hy));
        upright = body + tip;
        across = rnd(along * (fence_box(rnd(0.22 * ht), rnd(0.22 * ht) + 0.07, y, hy) + fence_box(rnd(0.72 * ht), rnd(0.72 * ht) + 0.07, y, hy)));
    } else if style == 1u {
        across = rnd(along * ((fence_box(rnd(0.28 * ht) - 0.09, rnd(0.28 * ht), y, hy) + fence_box(rnd(0.6 * ht) - 0.09, rnd(0.6 * ht), y, hy)) + fence_box(rnd(0.9 * ht) - 0.09, rnd(0.9 * ht), y, hy)));
    } else {
        across = rnd(along * fence_box(0.02, ht + 0.02, y, hy));
    }
    let up = 1.0 - rnd((1.0 - min(post, 1.0)) * (1.0 - min(upright, 1.0)));
    return 1.0 - rnd((1.0 - up) * (1.0 - min(across, 1.0)));
}

// render::ground_step's neighbour: how far the ground moves to pixel j (s = 1) or from it (s = -1);
// x > 1e29 when j is not ground.
fn ground_to(g: GPix, j: u32, s: f32) -> vec2<f32> {
    let o = gbuf[j];
    if (o.idr & 0xFFu) != ID_GROUND { return vec2<f32>(1e30, 0.0); }
    return vec2<f32>(rnd(s * (o.x - g.x)), rnd(s * (o.d - g.d)));
}

// Caster c's plane k (22: u, 25: v, 28: t; 31, 34: the contact oval) at ground (gx, gd):
// render::Caster::on_pic and on_oval.
fn plane_at(c: u32, k: u32, gx: f32, gd: f32) -> f32 { return rnd(rnd(rnd(cf(c, k) * gx) + rnd(cf(c, k + 1u) * gd)) + cf(c, k + 2u)); }

// The (t, u, v) where the sun's ray from ground (gx, gd) meets caster c's picture.
fn on_pic(c: u32, gx: f32, gd: f32) -> vec3<f32> {
    return vec3<f32>(plane_at(c, 28u, gx, gd), plane_at(c, 22u, gx, gd), plane_at(c, 25u, gx, gd));
}

@compute @workgroup_size(8, 8)
fn shadow_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= W.width || gid.y >= W.height { return; }
    let n = W.width * W.height;
    let i = gid.y * W.width + gid.x;
    let x = gid.x; let y = gid.y;
    let g = gbuf[i];
    let id = g.idr & 0xFFu;
    var sun = 1.0; var ao = 1.0;
    if (G.shadow_mode & 2u) != 0u { sun = masks[i]; ao = masks[n + i]; }
    let part = G.shadow_mode >> 2u;
    // Across this world's path (another world's ground brought into it, as prop_shadows does).
    var gx = g.x;
    if (G.shadow_mode & 1u) != 0u && id == ID_GROUND && (g.idr >> 8u) != W.realm {
        gx = rnd(rnd(cam_xz_of((g.idr >> 8u) & 0xFFu, g.x, g.d).x - bend_of(W.realm, g.d)) - shear_mix(W.realm, g.d));
    }
    let tile = (y / 16u) * G.tile_cols + x / 16u;
    let ntiles = G.tile_cols * ((W.height + 15u) / 16u);
    let b = G.cast_bins;
    if id == ID_GROUND && part != 2u {
        for (var q = bins[b + tile]; q < bins[b + tile + 1u]; q++) {
            let c = bins[b + ntiles + 1u + q];
            let shadow = cf(c, 5u);
            if cu(c, 8u) != 0u && x >= cu(c, 9u) && x <= cu(c, 11u) && y >= cu(c, 10u) && y <= cu(c, 12u) {
                let na = plane_at(c, 31u, gx, g.d);
                let nb = plane_at(c, 34u, gx, g.d);
                // Under a prop a soft oval; along a fence's foot a soft strip.
                var r2 = rnd(na * na) + rnd(nb * nb);
                if cu(c, 37u) == 1u { r2 = select(1.0, rnd(nb * nb), abs(na) <= 1.0); }
                if r2 < 1.0 { ao = rnd(ao * rnd(1.0 - rnd(rnd(0.5 * shadow) * (1.0 - r2)))); }
            }
            if G.sun_on != 0u && cu(c, 13u) != 0u && x >= cu(c, 14u) && x <= cu(c, 16u) && y >= cu(c, 15u) && y <= cu(c, 17u) {
                // Toward the sun from this bit of ground: where the ray meets the picture's plane.
                // u first, then v, then t: most rays miss the picture to one side.
                var u = plane_at(c, 22u, gx, g.d);
                if !(u >= 0.0 && u < 1.0) { continue; }
                let vv = plane_at(c, 25u, gx, g.d);
                if !(vv >= 0.0 && vv < 1.0) || plane_at(c, 28u, gx, g.d) <= 0.0 { continue; }
                var a = 0.0;
                if cu(c, 37u) == 1u {
                    // Averaged over the ground this pixel covers, so pickets far off do not shimmer.
                    var fu = 0.0; var fv = 0.0;
                    var ax = vec2<f32>(1e30, 0.0); var dn = vec2<f32>(1e30, 0.0);
                    if x + 1u < W.width { ax = ground_to(g, i + 1u, 1.0); }
                    if ax.x > 1e29 && x > 0u { ax = ground_to(g, i - 1u, -1.0); }
                    if y + 1u < W.height { dn = ground_to(g, i + W.width, 1.0); }
                    if dn.x > 1e29 && y > 0u { dn = ground_to(g, i - W.width, -1.0); }
                    if ax.x < 1e29 && dn.x < 1e29 {
                        let pa = on_pic(c, gx + ax.x, g.d + ax.y);
                        let pb = on_pic(c, gx + dn.x, g.d + dn.y);
                        fu = abs(pa.y - u) + abs(pb.y - u);
                        fv = abs(pa.z - vv) + abs(pb.z - vv);
                    }
                    a = fence_alpha(c, u, vv, min(fu, 1.0), min(fv, 1.0));
                } else {
                    let strip = cf(c, 18u);
                    var pv = vv;
                    if strip >= 0.0 { u = strip; }
                    else if strip < -1.5 {
                        let mu = cf(c, 40u) + rnd(cf(c, 41u) * u) + rnd(cf(c, 42u) * vv);
                        pv = cf(c, 43u) + rnd(cf(c, 44u) * u) + rnd(cf(c, 45u) * vv);
                        u = mu;
                    }
                    else if cu(c, 7u) != 0u { u = 1.0 - u; }
                    a = sprite_alpha(cu(c, 19u), cu(c, 20u), cu(c, 21u), u, pv);
                }
                if a > 0.01 { sun = min(sun, 1.0 - rnd(shadow * min(a, 1.0))); }
            }
        }
    }
    // Cloud shadows drifting over the ground and walls (weather::cloud_shade).
    if G.clouds > 0.0 && id != ID_NONE && part != 1u && (part != 2u || (g.idr >> 8u) == W.realm) {
        let nz = drifting_noise(g.x, W.scroll + g.d, 11.0, G.cloud_travel, G.cloud_seed ^ 0xC1u);
        sun *= 1.0 - G.clouds * 0.85 * smoothstep_r(0.4, 0.58, nz);
    }
    masks[i] = sun;
    masks[n + i] = ao;
}

var<workgroup> hits: array<atomic<u32>, 64>;
// The tile's camera-space box: min x, y, z then max x, y, z, as order-preserving bits.
var<workgroup> box: array<atomic<u32>, 6>;

// f32 bits that order as the floats do.
fn ord(f: f32) -> u32 { let u = bitcast<u32>(f); return select(u | 0x80000000u, ~u, (u & 0x80000000u) != 0u); }
fn unord(u: u32) -> f32 { return bitcast<f32>(select(~u, u & 0x7FFFFFFFu, (u & 0x80000000u) != 0u)); }

@compute @workgroup_size(8, 8)
fn tiles_main(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_index) li: u32, @builtin(local_invocation_id) lid: vec3<u32>) {
    if li < 3u { atomicStore(&box[li], 0xFFFFFFFFu); atomicStore(&box[li + 3u], 0u); }
    workgroupBarrier();
    // Each thread a 2 x 2 block of the tile.
    let inf = bitcast<f32>(0x7F800000u);
    var l = vec3<f32>(inf); var h = vec3<f32>(-inf);
    for (var k = 0u; k < 4u; k++) {
        let x = wg.x * 16u + lid.x * 2u + (k & 1u); let y = wg.y * 16u + lid.y * 2u + (k >> 1u);
        if x < W.width && y < W.height {
            let g = gbuf[y * W.width + x];
            if (g.idr & 0xFFu) != ID_NONE { let p = to_cam(g.x, g.y, g.d); l = min(l, p); h = max(h, p); }
        }
    }
    if l.x <= h.x {
        atomicMin(&box[0], ord(l.x)); atomicMin(&box[1], ord(l.y)); atomicMin(&box[2], ord(l.z));
        atomicMax(&box[3], ord(h.x)); atomicMax(&box[4], ord(h.y)); atomicMax(&box[5], ord(h.z));
    }
    workgroupBarrier();
    let any = atomicLoad(&box[0]) != 0xFFFFFFFFu;
    let a = vec3<f32>(unord(atomicLoad(&box[0])), unord(atomicLoad(&box[1])), unord(atomicLoad(&box[2])));
    let b = vec3<f32>(unord(atomicLoad(&box[3])), unord(atomicLoad(&box[4])), unord(atomicLoad(&box[5])));
    let t = wg.y * G.tile_cols + wg.x;
    let base = t * (G.cap + 1u);
    // Every thread tests its share of the lamps (2048 at a time, one bit each); the first gathers
    // the ones that reach, in index order.
    var count = 0u;
    for (var k0 = 0u; k0 < G.n_lights; k0 += 2048u) {
        atomicStore(&hits[li], 0u);
        workgroupBarrier();
        if any {
            for (var k = k0 + li; k < min(k0 + 2048u, G.n_lights); k += 64u) {
                let lp = vec3<f32>(lights[k * 8u], lights[k * 8u + 1u], lights[k * 8u + 2u]);
                let r = lights[k * 8u + 3u];
                let e = max(max(a - lp, lp - b), vec3<f32>(0.0));
                // A wider margin than the CPU's: a lamp that cannot reach adds nothing either way.
                if dot(e, e) < r * r * 1.001 + 1e-3 { atomicOr(&hits[(k - k0) / 32u], 1u << ((k - k0) % 32u)); }
            }
        }
        workgroupBarrier();
        if li == 0u {
            for (var wd = 0u; wd < 64u; wd++) {
                var bitsw = atomicLoad(&hits[wd]);
                while bitsw != 0u {
                    let bit = firstTrailingBit(bitsw);
                    bitsw &= bitsw - 1u;
                    if count < G.cap { tile_lights[base + 1u + count] = k0 + wd * 32u + bit; count += 1u; }
                }
            }
        }
        workgroupBarrier();
    }
    if li == 0u { tile_lights[base] = count; }
}

// The worlds of a frame meeting in patches (render::RealmMix, render::Bounds::realm_mix), read
// from `lights`: MIX_STRIDE words a world (render::Ctx::gpu_worlds: view::Bend::gpu_words, has a
// ceiling, has walls, scroll, is there), then from MIX_HEAD render::Bounds::gpu_words' head
// (anchor, seed, fork, strips on + 5, seams on + 4, restyle on + 4, -, where the regions, the calm
// pieces and the edges start) and its lists.
const MIX_STRIDE: u32 = 40u;
const MIX_HEAD: u32 = 120u;
const SEAM: f32 = 1.2;
// render::SAME_X.
const SAME_X: u32 = 0x80u;

fn mw(k: u32) -> f32 { return lights[k]; }
fn mn(k: u32) -> u32 { return u32(lights[k]); }

// view::Bend::x of world r.
fn bend_of(r: u32, d: f32) -> f32 {
    let o = r * MIX_STRIDE;
    var x = rnd(rnd(mw(o) * d) * d);
    for (var k = 0u; k < mn(o + 1u); k++) {
        let w = o + 2u + 6u * k;
        x = x + rnd(mw(w) * rnd(rnd(sin(rnd(mw(w + 1u) * d) + mw(w + 2u)) - mw(w + 3u)) - rnd(rnd(mw(w + 5u) * d) * mw(w + 4u))));
    }
    return x;
}
// view::Bend::shear of world r.
fn shear_mix(r: u32, d: f32) -> f32 {
    let o = r * MIX_STRIDE;
    if mw(o + 26u) == 0.0 { return 0.0; }
    return rnd(mw(o + 28u) * max(d - mw(o + 27u), 0.0));
}
// view::Bend::cam_xz of world r.
fn cam_xz_of(r: u32, x: f32, d: f32) -> vec2<f32> {
    let o = r * MIX_STRIDE;
    let cx = rnd(rnd(x + bend_of(r, d)) + shear_mix(r, d));
    if mw(o + 29u) == 0.0 { return vec2<f32>(cx, d); }
    let px = cx - mw(o + 30u);
    let yc = mw(o + 31u); let ys = mw(o + 32u);
    return vec2<f32>(rnd(rnd(yc * px) - rnd(ys * d)), rnd(rnd(ys * px) + rnd(yc * d)));
}
// RealmMix::has: whether world r draws surfaces like `id`.
fn mix_has(r: u32, id: u32) -> bool {
    let o = r * MIX_STRIDE;
    if r >= 3u || mw(o + 36u) == 0.0 { return false; }
    if id == ID_GROUND { return true; }
    if id == ID_CEILING { return mw(o + 33u) != 0.0; }
    return mw(o + 34u) != 0.0;
}

// A convex piece at k (a count, then each half-plane's a, b, c): whether (x, z) is in it.
fn piece_in(k: u32, x: f32, z: f32) -> bool {
    for (var h = 0u; h < mn(k); h++) {
        let q = k + 1u + 3u * h;
        if rnd(rnd(mw(q) * x) + rnd(mw(q + 1u) * z)) + mw(q + 2u) < 0.0 { return false; }
    }
    return true;
}
fn piece_len(k: u32) -> u32 { return 1u + 3u * mn(k); }

// Bounds::region_of.
fn region_of(x: f32, z: f32) -> u32 {
    var k = mn(MIX_HEAD + 20u);
    let worlds = mn(k);
    k += 1u;
    for (var r = 0u; r < worlds; r++) {
        let pieces = mn(k);
        k += 1u;
        var inside = false;
        for (var q = 0u; q < pieces; q++) {
            if !inside && piece_in(k, x, z) { inside = true; }
            k += piece_len(k);
        }
        if inside { return r; }
    }
    return 0u;
}
fn in_calm(x: f32, z: f32) -> bool {
    var k = mn(MIX_HEAD + 21u);
    let pieces = mn(k);
    k += 1u;
    for (var q = 0u; q < pieces; q++) {
        if piece_in(k, x, z) { return true; }
        k += piece_len(k);
    }
    return false;
}
// Bounds::on_branch: 1 or 2, or 0 for neither.
fn mix_on_branch(x: f32, z: f32) -> u32 {
    if mw(MIX_HEAD + 3u) == 0.0 { return 0u; }
    let zb = mw(MIX_HEAD + 4u);
    if z <= zb { return 0u; }
    let dl = abs(x - rnd(mw(MIX_HEAD + 5u) * (z - zb)));
    let dr = abs(x - rnd(mw(MIX_HEAD + 6u) * (z - zb)));
    let hl = mw(MIX_HEAD + 7u); let hr = mw(MIX_HEAD + 8u);
    var l = dl < hl - 0.45; var r = dr < hr - 0.45;
    if !l && !r { l = dl < hl + 0.45; r = dr < hr + 0.45; }
    if l && !r { return 1u; }
    if r && !l { return 2u; }
    return 0u;
}

// Bounds::realm_mix's answer: the world shown, the other world at a soft edge, and its share.
struct Mixed { to: u32, other: u32, k: f32 }

// Bounds::seam.
fn mix_seam(r: u32, x: f32, z: f32) -> Mixed {
    if mw(MIX_HEAD + 9u) == 0.0 { return Mixed(r, r, 0.0); }
    let zb = mw(MIX_HEAD + 10u); let sd = mw(MIX_HEAD + 11u); let cd = mw(MIX_HEAD + 12u); let apex = mw(MIX_HEAD + 13u);
    let split = div(x - (rnd(sd * (z - zb)) + cd), sqrt(1.0 + rnd(sd * sd)));
    if r == 0u && z > zb - SEAM && z <= zb { return Mixed(0u, select(2u, 1u, split < 0.0), rnd(0.5 * (1.0 - div(zb - z, SEAM)))); }
    if r != 0u && z >= zb && z < zb + SEAM { return Mixed(r, 0u, rnd(0.5 * (1.0 - div(z - zb, SEAM)))); }
    if r != 0u && z < apex && abs(split) < SEAM * 0.6 { return Mixed(r, 3u - r, rnd(0.5 * (1.0 - div(abs(split), SEAM * 0.6)))); }
    return Mixed(r, r, 0.0);
}
// The three octaves of patch noise `realm_mix` and `road_look` share.
fn patch_noise(x: f32, w: f32, seed: f32) -> f32 {
    return rnd(rnd(value_noise(x * 0.35 + seed * 0.37, w * 0.35) * 0.55) + rnd(value_noise(x * 1.1 + 31.0, w * 1.1 + seed) * 0.3))
        + rnd(value_noise(x * 3.7 + 7.0, w * 3.7 - 11.0) * 0.15);
}
// Bounds::realm_mix.
fn realm_mix(x: f32, z: f32) -> Mixed {
    let b = mix_on_branch(x, z);
    if b != 0u {
        let zb = mw(MIX_HEAD + 4u);
        if z < zb + SEAM { return Mixed(b, 0u, rnd(0.5 * (1.0 - div(z - zb, SEAM)))); }
        return Mixed(b, b, 0.0);
    }
    let r = region_of(x, z);
    if in_calm(x, z) { return mix_seam(r, x, z); }
    var k = mn(MIX_HEAD + 22u);
    let edges = mn(k);
    k += 1u;
    for (var e = 0u; e < edges; e++) {
        let next = k + 6u + piece_len(k + 6u);
        let a = mn(k + 4u); let bw = mn(k + 5u);
        let width = mw(k + 3u);
        let s = rnd(rnd(mw(k) * x) + rnd(mw(k + 1u) * z)) + mw(k + 2u);
        if (r == a || r == bw) && piece_in(k + 6u, x, z) && abs(s) < width {
            let n = patch_noise(x, z + mw(MIX_HEAD), mw(MIX_HEAD + 1u));
            let d = rnd(div(s, width) * 0.5) + 0.5 - n;
            return Mixed(select(a, bw, d > 0.0), select(bw, a, d > 0.0), 0.5 * (1.0 - smoothstep_r(0.0, 0.07, abs(d))));
        }
        k = next;
    }
    return Mixed(r, r, 0.0);
}
// Bounds::road_look.
fn road_look(x: f32, w: f32, z: f32) -> f32 {
    if mw(MIX_HEAD + 14u) == 0.0 { return 0.0; }
    let past = z - mw(MIX_HEAD + 15u) - mw(MIX_HEAD + 16u);
    let band = mw(MIX_HEAD + 17u);
    if past <= -band { return 1.0; }
    if past >= band * 1.1 { return 0.0; }
    return 1.0 - smoothstep_r(-0.07, 0.07, div(past, band) - patch_noise(x, w, mw(MIX_HEAD + 18u)));
}

// render::RealmMix::mix over each pixel: its world, moved into that world's path frame, and the
// soft patch edge's other world and share into the masks after the sun and contact shadow.
@compute @workgroup_size(8, 8)
fn mix_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= W.width || gid.y >= W.height { return; }
    let n = W.width * W.height;
    let i = gid.y * W.width + gid.x;
    var g = gbuf[i];
    let id = g.idr & 0xFFu;
    var soft_o = 0u; var soft_k = 0.0;
    if id == ID_GROUND || id == ID_WALL_L || id == ID_WALL_R || id == ID_CEILING {
        let own = (g.idr >> 8u) & 0xFFu;
        var realm = own;
        let cam = cam_xz_of(own, g.x, g.d);
        let m = realm_mix(cam.x, cam.y);
        // A fork's walls are never handed or blended across the split between its branches.
        let walls = mw(MIX_HEAD + 2u) != 0.0 && (id == ID_WALL_L || id == ID_WALL_R) && own > 0u;
        let to = select(m.to, own, walls && m.to > 0u && m.to != own);
        let other = select(m.other, own, walls && m.other > 0u && m.other != own);
        if to != realm && mix_has(to, id) {
            g.x = rnd(rnd(cam.x - bend_of(to, g.d)) - shear_mix(to, g.d));
            realm = to;
        }
        if m.k > 0.0 && other != realm && mix_has(other, id) { soft_o = other; soft_k = m.k; }
        // Near a fork's junction its branches' ground is drawn as the first world would draw it.
        if id == ID_GROUND && realm != 0u {
            let look = road_look(g.x, g.d + mw(realm * MIX_STRIDE + 35u), cam.y);
            if look > 0.0 && look >= soft_k { soft_o = SAME_X; soft_k = look; }
        }
        g.idr = (g.idr & ~0xFF00u) | (realm << 8u);
        gbuf[i] = g;
    }
    masks[2u * n + 2u * i] = bitcast<f32>(soft_o);
    masks[2u * n + 2u * i + 1u] = soft_k;
}
