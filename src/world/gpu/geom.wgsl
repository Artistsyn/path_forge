// The G-buffer, its shadow masks and the lamps each tile can see, for a frame of one world:
// - raster_main (then small_z_main, small_pick_main, resolve_main): raster::rasterize, with the
//   same edge and span arithmetic on triangles set up on the CPU as `raster::prepare` sets them up;
// - shadow_main: render::prop_shadows (contact darkening and sun shadows of billboards) and
//   weather::cloud_shade, into the sun and ambient-occlusion masks;
// - tiles_main: render::tile_lights, one workgroup per 16 x 16 tile: the camera-space box of what
//   it shows, and every lamp whose reach touches it, in index order.

// A prepared triangle (raster::Prepared): a, b, c on screen; 1/z at each; world position / z at
// each; the box min_x, max_x, min_y, max_y (as i32 bits); 1 / area; id | realm << 8; and per edge
// (a-b, b-c, c-a) its (ex, ey, s_opp).
struct Tri { f: array<f32, 36> }

// A shadow caster (render::prop_shadows' Caster): bx, bd, rx, rd, half_w, shadow, height, flip;
// contact box on (bits), x0, y0, x1, y1; sun box on, x0, y0, x1, y1, scale; sprite level off, w, h.
struct Caster { f: array<f32, 24> }

struct Geom {
    tile_cols: u32, cast_bins: u32, sun_on: u32, n_lights: u32,
    sun_x: f32, sun_y: f32, sun_z: f32, clouds: f32,
    cloud_travel: f32, cloud_seed: u32, tiles_on: u32, cap: u32,
    small_off: u32, n_small: u32, pad0: u32, pad1: u32,
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
        g = tri_pix(t, b);
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
            let z = bary(t, f32(x) + 0.5, f32(y) + 0.5).w;
            if z < bitcast<f32>(INF_BITS) { atomicMin(&racc[u32(y) * W.width + u32(x)], bitcast<u32>(z)); }
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
            let z = bary(t, f32(x) + 0.5, f32(y) + 0.5).w;
            let p = u32(y) * W.width + u32(x);
            if z < bitcast<f32>(INF_BITS) && bitcast<u32>(z) == atomicLoad(&racc[p]) { atomicMin(&racc[n + p], t); }
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
fn cu(c: u32, k: u32) -> u32 { return bitcast<u32>(casters[c].f[k]); }

@compute @workgroup_size(8, 8)
fn shadow_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= W.width || gid.y >= W.height { return; }
    let n = W.width * W.height;
    let i = gid.y * W.width + gid.x;
    let x = gid.x; let y = gid.y;
    let g = gbuf[i];
    let id = g.idr & 0xFFu;
    var sun = 1.0; var ao = 1.0;
    let tile = (y / 16u) * G.tile_cols + x / 16u;
    let ntiles = G.tile_cols * ((W.height + 15u) / 16u);
    let b = G.cast_bins;
    if id == ID_GROUND {
        for (var q = bins[b + tile]; q < bins[b + tile + 1u]; q++) {
            let c = bins[b + ntiles + 1u + q];
            let bx = cf(c, 0u); let bd = cf(c, 1u); let shadow = cf(c, 5u);
            if cu(c, 8u) != 0u && x >= cu(c, 9u) && x <= cu(c, 11u) && y >= cu(c, 10u) && y <= cu(c, 12u) {
                let nx = div(g.x - bx, cf(c, 2u)); let nd = div(g.d - bd, cf(c, 3u));
                let r2 = rnd(nx * nx) + rnd(nd * nd);
                if r2 < 1.0 { ao = rnd(ao * rnd(1.0 - rnd(rnd(0.5 * shadow) * (1.0 - r2)))); }
            }
            if G.sun_on != 0u && cu(c, 13u) != 0u && x >= cu(c, 14u) && x <= cu(c, 16u) && y >= cu(c, 15u) && y <= cu(c, 17u) {
                let scale = cf(c, 18u); let height = cf(c, 6u); let half_w = cf(c, 4u);
                let hgt = div(div(rnd((bd - g.d) * G.sun_y), G.sun_z), scale);
                if hgt < 0.0 || hgt > height { continue; }
                let off = (g.x - bx) + rnd(div(rnd(hgt * G.sun_x), G.sun_y) * scale);
                if abs(off) > half_w { continue; }
                var u = div(off, 2.0 * half_w) + 0.5;
                if cu(c, 7u) != 0u { u = 1.0 - u; }
                let a = sprite_alpha(cu(c, 19u), cu(c, 20u), cu(c, 21u), u, 1.0 - div(hgt, height));
                if a > 0.01 { sun = min(sun, 1.0 - rnd(shadow * min(a, 1.0))); }
            }
        }
    }
    // Cloud shadows drifting over the ground and walls (weather::cloud_shade).
    if G.clouds > 0.0 && id != ID_NONE {
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
