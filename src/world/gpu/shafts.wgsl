// Light made visible by the air (weather::light_shafts), at half resolution: the open-sky mask
// round the sun, the march toward the sun and the lamp haloes per cell, then a bilinear add over
// the frame.

struct Shafts {
    mw: u32, mh: u32, sun_on: u32, lamps_on: u32,
    sx: f32, sy: f32, glow_r: f32, k_sun: f32,
    sun_r: f32, sun_g: f32, sun_b: f32, k_lamp: f32,
    tc: u32, n_lights: u32, hy: f32, pad: u32,
}

@group(0) @binding(1) var<storage, read> gbuf: array<GPix>;
@group(0) @binding(2) var<storage, read_write> hdr: array<f32>;
@group(0) @binding(3) var<storage, read> lights: array<f32>;
@group(0) @binding(4) var<storage, read> lists: array<u32>;
// mask (mw * mh), then add (mw * mh * 3).
@group(0) @binding(5) var<storage, read_write> cells: array<f32>;
@group(0) @binding(6) var<uniform> S: Shafts;

const Q: u32 = 2u;

@compute @workgroup_size(8, 8)
fn shaft_mask(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= S.mw || gid.y >= S.mh { return; }
    let c = gid.y * S.mw + gid.x;
    if S.sun_on == 0u { cells[c] = 0.0; return; }
    let dx = f32(gid.x) + 0.5 - S.sx; let dy = f32(gid.y) + 0.5 - S.sy;
    let near_sun = 0.1 + 12.0 * exp(-(rnd(dx * dx) + rnd(dy * dy)) / rnd(S.glow_r * S.glow_r));
    var open = 0u;
    let cx = gid.x * Q; let cy = gid.y * Q;
    for (var y = cy; y < min(cy + Q, W.height); y++) {
        if f32(y) >= S.hy { continue; }
        for (var x = cx; x < min(cx + Q, W.width); x++) { if (gbuf[y * W.width + x].idr & 0xFFu) == ID_NONE { open += 1u; } }
    }
    cells[c] = near_sun * f32(open) / f32(Q * Q);
}

fn cell_px(cx: u32, cy: u32) -> vec2<u32> { return vec2<u32>(min(cx * Q + Q / 2u, W.width - 1u), min(cy * Q + Q / 2u, W.height - 1u)); }

@compute @workgroup_size(8, 8)
fn shaft_add(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= S.mw || gid.y >= S.mh { return; }
    let mw = S.mw; let mh = S.mh;
    let c = gid.y * mw + gid.x;
    var o = vec3<f32>(0.0);
    if S.sun_on != 0u {
        let px = f32(gid.x) + 0.5; let py = f32(gid.y) + 0.5;
        let steps = 48.0;
        let dx = div(S.sx - px, steps); let dy = div(S.sy - py, steps);
        var x = px; var y = py; var wgt = 1.0; var acc = 0.0;
        for (var k = 0; k < 48; k++) {
            x = rnd(x + dx); y = rnd(y + dy);
            // The mask read between cells: read cell by cell, the long steps far from the sun
            // gathered the same cells for whole blocks of pixels, and smooth surfaces showed them.
            let fx = x - 0.5; let fy = y - 0.5;
            let x0 = floor(fx); let y0 = floor(fy);
            let tx = fx - x0; let ty = fy - y0;
            let ix = i32(x0); let iy = i32(y0);
            let c00 = mask_at(ix, iy); let c10 = mask_at(ix + 1, iy); let c01 = mask_at(ix, iy + 1); let c11 = mask_at(ix + 1, iy + 1);
            let top = rnd(c00 + rnd((c10 - c00) * tx)); let bot = rnd(c01 + rnd((c11 - c01) * tx));
            let m = rnd(top + rnd((bot - top) * ty));
            acc = rnd(acc + rnd(m * wgt));
            wgt = rnd(wgt * 0.975);
        }
        o += vec3<f32>(S.sun_r, S.sun_g, S.sun_b) * ((1.0 - exp(-acc * 0.04)) * S.k_sun);
    }
    if S.lamps_on != 0u {
        let p = cell_px(gid.x, gid.y);
        let dir0 = vec3<f32>((f32(p.x) + 0.5 - W.center_px) / W.focal_px, (W.horizon_px - f32(p.y) - 0.5) / W.focal_px, 1.0);
        let dl = sqrt(dot(dir0, dir0));
        let dir = dir0 / dl;
        let depth = min(gbuf[p.y * W.width + p.x].depth, 80.0);
        let reach = depth * dl;
        var glow = vec3<f32>(0.0);
        let t = (gid.y / 16u) * S.tc + gid.x / 16u;
        let ntiles = S.tc * ((mh + 15u) / 16u);
        for (var q = lists[t]; q < lists[t + 1u]; q++) {
            let li = lists[ntiles + 1u + q];
            let lp = vec3<f32>(lights[li * 8u], lights[li * 8u + 1u], lights[li * 8u + 2u]);
            let rad = lights[li * 8u + 3u];
            let b = dot(dir, lp);
            let h2 = max(dot(lp, lp) - b * b, 0.0);
            if h2 > rad * rad { continue; }
            let hh = sqrt(h2 + 0.02);
            let integ = (atan((reach - b) / hh) - atan(-b / hh)) / hh;
            let fl = 1.0 - sqrt(h2) / rad;
            glow += vec3<f32>(lights[li * 8u + 4u], lights[li * 8u + 5u], lights[li * 8u + 6u]) * (integ * fl * fl);
        }
        o += glow * S.k_lamp;
    }
    let a = mw * mh + c * 3u;
    cells[a] = o.x; cells[a + 1u] = o.y; cells[a + 2u] = o.z;
}

fn mask_at(ix: i32, iy: i32) -> f32 {
    if ix < 0 || iy < 0 || u32(ix) >= S.mw || u32(iy) >= S.mh { return 0.0; }
    return cells[u32(iy) * S.mw + u32(ix)];
}
fn cell_add(x: u32, y: u32) -> vec3<f32> { let a = S.mw * S.mh + (y * S.mw + x) * 3u; return vec3<f32>(cells[a], cells[a + 1u], cells[a + 2u]); }

@compute @workgroup_size(8, 8)
fn shaft_apply(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= W.width || gid.y >= W.height { return; }
    let i = gid.y * W.width + gid.x;
    let fx = (f32(gid.x) + 0.5) / f32(Q) - 0.5;
    let fy = (f32(gid.y) + 0.5) / f32(Q) - 0.5;
    let x0 = u32(max(floor(fx), 0.0)); let y0 = u32(max(floor(fy), 0.0));
    let x1 = min(x0 + 1u, S.mw - 1u); let y1 = min(y0 + 1u, S.mh - 1u);
    let tx = clamp(fx - f32(x0), 0.0, 1.0); let ty = clamp(fy - f32(y0), 0.0, 1.0);
    let top = mix3(cell_add(x0, y0), cell_add(x1, y0), tx);
    let bot = mix3(cell_add(x0, y1), cell_add(x1, y1), tx);
    let c = vec3<f32>(hdr[i * 3u], hdr[i * 3u + 1u], hdr[i * 3u + 2u]) + mix3(top, bot, ty);
    hdr[i * 3u] = c.x; hdr[i * 3u + 1u] = c.y; hdr[i * 3u + 2u] = c.z;
}
