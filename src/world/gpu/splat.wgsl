// Splats (render::splat::Splat::draw), one thread per pixel: each pixel walks its tile's splats in
// the order the passes emitted them, so tests and blends happen as on the CPU. Only blades write
// the G-buffer; they are drawn in a dispatch of their own before the rest, which only read it
// (splash gates look at another pixel's G-buffer).

struct Splat {
    kind: u32, z: f32, gx: i32, gy: i32, gcz: f32,
    f: array<f32, 19>,
}

struct Pass { tile_cols: u32, n: u32, pad0: u32, pad1: u32 }

@group(0) @binding(1) var<storage, read_write> gbuf: array<GPix>;
@group(0) @binding(2) var<storage, read_write> hdr: array<f32>;
@group(0) @binding(3) var<storage, read> splats: array<Splat>;
@group(0) @binding(4) var<storage, read> bins: array<u32>;
@group(0) @binding(5) var<uniform> P: Pass;

const K_BLADE: u32 = 0u;
const K_HALO: u32 = 1u;
const K_ORB: u32 = 2u;
const K_BODY: u32 = 3u;
const K_WISP: u32 = 4u;
const K_MOTE: u32 = 5u;
const K_STREAK: u32 = 6u;
const K_DOT: u32 = 7u;
const K_CURTAIN: u32 = 8u;

fn fi(s: Splat, k: u32) -> i32 { return bitcast<i32>(s.f[k]); }
fn fu(s: Splat, k: u32) -> u32 { return bitcast<u32>(s.f[k]); }
fn f3(s: Splat, k: u32) -> vec3<f32> { return vec3<f32>(s.f[k], s.f[k + 1u], s.f[k + 2u]); }
fn sq(x: f32) -> f32 { return rnd(x * x); }

@compute @workgroup_size(8, 8)
fn splat_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= W.width || gid.y >= W.height { return; }
    let i = gid.y * W.width + gid.x;
    let x = i32(gid.x); let y = i32(gid.y);
    let fx = f32(x); let fy = f32(y);
    let tile = (gid.y / 16u) * P.tile_cols + gid.x / 16u;
    let ntiles = P.tile_cols * ((W.height + 15u) / 16u);
    let a0 = bins[tile]; let a1 = bins[tile + 1u];
    if a0 == a1 { return; }
    var g = gbuf[i];
    var c = vec3<f32>(hdr[i * 3u], hdr[i * 3u + 1u], hdr[i * 3u + 2u]);
    let c_in = c;
    var g_written = false;
    for (var q = a0; q < a1; q++) {
        let s = splats[bins[ntiles + 1u + q]];
        let z = s.z;
        if s.gx >= 0 {
            let gp = gbuf[u32(s.gy) * W.width + u32(s.gx)];
            if (gp.idr & 0xFFu) != ID_GROUND || abs(gp.depth - s.gcz) > rnd(0.15 * s.gcz) + 0.1 { continue; }
        }
        switch s.kind {
            case 0u: { // blade
                let bx = s.f[0]; let sy = s.f[1]; let lean = s.f[2]; let len = s.f[3]; let steps = fi(s, 4u);
                // Row py = trunc(sy - len t) only for t near (sy - y) / len: a few steps, tested exactly.
                let tc = (sy - fy) / max(len, 1e-6) * f32(steps);
                let st0 = max(i32(floor(tc - f32(steps) / max(len, 1e-6))) - 2, 0);
                let st1 = min(i32(ceil(tc)) + 2, steps - 1);
                for (var st = st0; st <= st1; st++) {
                    let t = div(f32(st), f32(steps));
                    let px = i32(bx + rnd(rnd(rnd(rnd(lean * len) * t) * t) * 0.5));
                    let py = i32(sy - rnd(len * t));
                    if px != x || py != y { continue; }
                    if z >= g.depth + 0.05 { continue; }
                    c = f3(s, 5u) * (0.7 + rnd(0.5 * t));
                    g.idr = (g.idr & 0xFFFFFF00u) | ID_TUFT;
                    g.depth = z;
                    g_written = true;
                }
            }
            case 1u: { // halo
                let sx = s.f[0]; let sy = s.f[1]; let gr = s.f[2];
                if x < i32(sx - gr) || x > i32(sx + gr) || y < i32(sy - gr) || y > i32(sy + gr) { continue; }
                if g.depth < z - 0.3 { continue; }
                let dd = div(sqrt(sq(fx + 0.5 - sx) + sq(fy + 0.5 - sy)), gr);
                if dd < 1.0 { c += f3(s, 3u) * ((1.0 - dd) * (1.0 - dd)); }
            }
            case 2u: { // orb
                let sx = s.f[0]; let sy = s.f[1]; let r = s.f[2];
                if x < i32(sx - r) || x > i32(sx + r) || y < i32(sy - r) || y > i32(sy + r) { continue; }
                if g.depth < z - 0.1 { continue; }
                let dd = div(sqrt(sq(fx + 0.5 - sx) + sq(fy + 0.5 - sy)), r);
                if dd < 1.0 { c += f3(s, 3u) * (3.0 * (1.0 - dd) * s.f[6]); }
            }
            case 3u: { // flame body
                let sx = s.f[0]; let cy = s.f[1]; let rx = s.f[2]; let ry = s.f[3];
                if x < i32(sx - rx) || x > i32(sx + rx) || y < i32(cy - ry) || y > i32(cy + ry) { continue; }
                if g.depth < z - 0.1 { continue; }
                let dx = div(fx + 0.5 - sx, rx); let dy = div(fy + 0.5 - cy, ry);
                let taper = 1.0 + rnd(max(-dy, 0.0) * 0.8);
                let dd = sqrt(sq(dx * taper) + sq(dy));
                if dd >= 1.0 { continue; }
                var col: vec3<f32>; var a: f32;
                if dd < 0.35 { col = mix3(f3(s, 4u), f3(s, 7u), div(dd, 0.35)); a = 1.0; }
                else if dd < 0.7 { col = mix3(f3(s, 7u), f3(s, 10u), div(dd - 0.35, 0.35)); a = 0.85; }
                else { col = mix3(f3(s, 10u), f3(s, 13u), div(dd - 0.7, 0.3)); a = 0.85 * (1.0 - div(dd - 0.7, 0.3)); }
                c += col * (a * 2.2 * s.f[16]);
            }
            case 4u: { // wisp
                if x < fi(s, 4u) || x > fi(s, 5u) || y < fi(s, 6u) || y > fi(s, 7u) { continue; }
                if z >= g.depth + 0.5 { continue; }
                let qq = sq(div(fx + 0.5 - s.f[0], s.f[2])) + sq(div(fy + 0.5 - s.f[1], s.f[3]));
                if qq >= 1.0 { continue; }
                c = mix3(c, f3(s, 8u), s.f[11] * ((1.0 - qq) * (1.0 - qq)));
            }
            case 5u: { // mote
                let ox = x - fi(s, 0u); let oy = y - fi(s, 1u);
                let ri = fi(s, 2u); let dy_len = fi(s, 3u); let dx_len = fi(s, 4u);
                if oy < -ri || oy > ri + dy_len || ox < -ri - dx_len || ox > ri { continue; }
                if z >= g.depth { continue; }
                let fl = fu(s, 6u);
                var d2 = 0.0;
                if (fl & 1u) == 0u { d2 = f32(ox * ox); }
                if (fl & 2u) == 0u { d2 += f32(oy * oy); }
                let dd = div(sqrt(d2), s.f[5] + 0.5);
                if dd >= 1.0 { continue; }
                let k = (1.0 - dd) * s.f[10] * s.f[11];
                if (fl & 4u) != 0u { c += f3(s, 7u) * (k * select(0.6, 1.0, (fl & 8u) != 0u)); }
                else { c = mix3(c, f3(s, 7u), k); }
            }
            case 6u: { // streak
                let x0 = s.f[0]; let y0 = s.f[1];
                let dx = s.f[2] - x0; let dy = s.f[3] - y0;
                let alpha = s.f[7]; let steps = fi(s, 8u); let half = s.f[9]; let reach = fi(s, 10u);
                if z >= g.depth { continue; }
                for (var st = 0; st <= steps; st++) {
                    let t = div(f32(st), f32(steps));
                    let py = y0 + rnd(dy * t);
                    if i32(floor(py)) != y { continue; }
                    let px = x0 + rnd(dx * t);
                    let ox = x - i32(floor(px));
                    if ox < -reach || ox > reach + 1 { continue; }
                    let cover = clamp(half + 0.5 - abs(fx + 0.5 - px), 0.0, 1.0);
                    if cover <= 0.0 { continue; }
                    let k = alpha * (0.35 + rnd(0.65 * t));
                    c = mix3(c, f3(s, 4u), k * cover);
                }
            }
            case 7u: { // dot
                let sx = s.f[0]; let sy = s.f[1]; let r = s.f[2]; let ri = fi(s, 7u);
                let ox = x - i32(floor(sx)); let oy = y - i32(floor(sy));
                if ox < -ri || ox > ri || oy < -ri || oy > ri { continue; }
                if z >= g.depth { continue; }
                let dd = div(sqrt(sq(fx + 0.5 - sx) + sq(fy + 0.5 - sy)), r + 0.5);
                if dd >= 1.0 { continue; }
                c = mix3(c, f3(s, 3u), s.f[6] * (1.0 - dd));
            }
            case 8u: { // curtains
                var far = 1.0;
                if (g.idr & 0xFFu) != ID_NONE { far = smoothstep_r(25.0, 60.0, g.depth); }
                if far <= 0.0 { continue; }
                let col_id = i32(floor((fx - rnd(s.f[1] * fy)) / 2.0));
                let seed = fu(s, 3u);
                let speed = 1.0 + f32(hash(seed ^ 0x72u, col_id) % 2u);
                let ss = rem_e(rnd(div(fy, f32(W.height)) * 2.5) + hf(seed ^ 0x71u, col_id) - rnd(rnd(speed * s.f[2]) * s.f[7]), 1.0);
                let st = smoothstep_r(0.75, 1.0, ss) * (0.4 + 0.6 * hf(seed ^ 0x73u, col_id));
                c += f3(s, 4u) * (s.f[0] * st * far);
            }
            default: {}
        }
    }
    if any(c != c_in) || g_written {
        hdr[i * 3u] = c.x; hdr[i * 3u + 1u] = c.y; hdr[i * 3u + 2u] = c.z;
    }
    if g_written { gbuf[i] = g; }
}
