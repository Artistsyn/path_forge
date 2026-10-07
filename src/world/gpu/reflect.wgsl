// Mirror reflections on glossy floors (render::reflect_pass). Two kernels:
// - colsum_main, one thread per column: the column's running sums of running sums of the frame
//   (render::ColumnBlur) in double-float arithmetic (hi + lo, so the sums of f32 values are exact,
//   as the CPU's f64 ones are), and the nearest depth in each block of 8 and 64 rows;
// - reflect_main, one thread per pixel: the march up the column and the triangle blur.

@group(0) @binding(1) var<storage, read> gbuf: array<GPix>;
@group(0) @binding(2) var<storage, read_write> hdr: array<f32>;
@group(0) @binding(3) var<storage, read> refl: array<f32>;
// Row r (0..=h) by column x: 3 double-floats at ((r * w) + x) * 3 (row-major, so a row of
// threads writes one stretch of memory).
@group(0) @binding(4) var<storage, read_write> q2s: array<vec2<f32>>;
// min8 (w * nb8) then min64 (w * nb64), column by column.
@group(0) @binding(5) var<storage, read_write> mins: array<f32>;
// Per column: q1 at the end (3 double-floats), the first row, the last row.
@group(0) @binding(6) var<storage, read_write> ends: array<f32>;
// Per column and block of 64 rows, per channel: two double-floats (see colsum_a, colsum_b).
@group(0) @binding(7) var<storage, read_write> blocks: array<vec2<f32>>;

fn nb8() -> u32 { return (W.height + 7u) / 8u; }
fn nb64() -> u32 { return (W.height + 63u) / 64u; }

// Phase A, one thread per (column, block of 64 rows): within the block, the running sums of
// running sums (lq2, stored at q2 index i for the block's rows i - 1), the block's total and its
// lq2 at the end (in `blocks`), and the nearest depth in each 8 and the 64 rows.
@compute @workgroup_size(64)
fn colsum_a(@builtin(global_invocation_id) gid: vec3<u32>) {
    let x = gid.x; let b = gid.y;
    if x >= W.width || b >= nb64() { return; }
    let w = W.width; let h = W.height;
    let n8 = nb8(); let n64 = nb64();
    var q1 = array<vec2<f32>, 3>(vec2<f32>(0.0), vec2<f32>(0.0), vec2<f32>(0.0));
    var acc = array<vec2<f32>, 3>(vec2<f32>(0.0), vec2<f32>(0.0), vec2<f32>(0.0));
    let r0 = b * 64u; let r1 = min(r0 + 64u, h);
    var m8 = bitcast<f32>(0x7F800000u); var m64 = m8;
    for (var r = r0; r < r1; r++) {
        let i = r * w + x;
        for (var k = 0u; k < 3u; k++) {
            acc[k] = df_add(acc[k], q1[k]);
            q1[k] = df_add(q1[k], vec2<f32>(hdr[i * 3u + k], 0.0));
            q2s[((r + 1u) * w + x) * 3u + k] = acc[k];
        }
        let z = gbuf[i].depth;
        if (r % 8u) == 0u { m8 = z; } else if z < m8 { m8 = z; }
        if z < m64 || r == r0 { m64 = z; }
        if (r % 8u) == 7u || r + 1u == h { mins[x * n8 + r / 8u] = m8; }
    }
    mins[w * n8 + x * n64 + b] = m64;
    let o = ((x * n64 + b) * 3u) * 2u;
    for (var k = 0u; k < 3u; k++) { blocks[o + 2u * k] = q1[k]; blocks[o + 2u * k + 1u] = acc[k]; }
}

// Phase B, one thread per column: each block's (sum of the rows before it, q2 at its start), in
// place of its (total, end lq2); then the column's ends.
@compute @workgroup_size(64)
fn colsum_b(@builtin(global_invocation_id) gid: vec3<u32>) {
    let x = gid.x;
    if x >= W.width { return; }
    let w = W.width; let h = W.height;
    let n64 = nb64();
    var q1 = array<vec2<f32>, 3>(vec2<f32>(0.0), vec2<f32>(0.0), vec2<f32>(0.0));
    var q2 = array<vec2<f32>, 3>(vec2<f32>(0.0), vec2<f32>(0.0), vec2<f32>(0.0));
    for (var b = 0u; b < n64; b++) {
        let n = f32(min(64u, h - b * 64u));
        let o = ((x * n64 + b) * 3u) * 2u;
        for (var k = 0u; k < 3u; k++) {
            let tot = blocks[o + 2u * k]; let end = blocks[o + 2u * k + 1u];
            blocks[o + 2u * k] = q1[k]; blocks[o + 2u * k + 1u] = q2[k];
            q2[k] = df_add(df_add(q2[k], df_mul_f(q1[k], n)), end);
            q1[k] = df_add(q1[k], tot);
        }
    }
    let e = x * 12u;
    for (var k = 0u; k < 3u; k++) {
        ends[e + 2u * k] = q1[k].x; ends[e + 2u * k + 1u] = q1[k].y;
        ends[e + 6u + k] = hdr[x * 3u + k];
        ends[e + 9u + k] = hdr[((h - 1u) * w + x) * 3u + k];
    }
}

// q2 at index 1 <= i <= h from the tables: the block's start, plus offset times the rows before it,
// plus the in-block part.
fn q2_at(x: u32, i: u32, k: u32) -> vec2<f32> {
    let b = (i - 1u) / 64u;
    let o = ((x * nb64() + b) * 3u) * 2u;
    let q1b = blocks[o + 2u * k]; let q2b = blocks[o + 2u * k + 1u];
    return df_add(df_add(q2b, df_mul_f(q1b, f32(i - b * 64u))), q2s[(i * W.width + x) * 3u + k]);
}

// ColumnBlur::q2 for channel k.
fn q2(x: u32, i: i32, k: u32) -> vec2<f32> {
    let h = i32(W.height);
    let e = x * 12u;
    if i <= 0 {
        let m = f32(-i);
        return two_prod(ends[e + 6u + k], m * (m + 1.0) * 0.5);
    } else if i <= h {
        return q2_at(x, u32(i), k);
    }
    let m = f32(i - h);
    let q1h = vec2<f32>(ends[e + 2u * k], ends[e + 2u * k + 1u]);
    return df_add(df_add(q2_at(x, W.height, k), df_mul_f(q1h, m)), two_prod(ends[e + 9u + k], m * (m - 1.0) * 0.5));
}

// ColumnBlur::triangle.
fn triangle(x: u32, r0: i32, half_in: i32) -> vec3<f32> {
    let half = max(half_in, 0);
    let n = f32((half + 1) * (half + 1));
    var out: vec3<f32>;
    for (var k = 0u; k < 3u; k++) {
        let t = df_add(df_add(q2(x, r0 + half + 2, k), df_neg(df_mul_f(q2(x, r0 + 1, k), 2.0))), q2(x, r0 - half, k));
        out[k] = df_div_f(t, n);
    }
    return out;
}

// "z nearer than the ray at row r" (see reflect_pass).
fn near_ray(z: f32, r: u32, kr: f32, dw: f32) -> bool {
    let k1 = 1.0 + rnd((f32(r) + 0.5 - W.horizon_px) * kr);
    return k1 <= 0.001 || rnd(z * k1) <= 2.0 * dw;
}

fn tap(xs: u32, yc: f32, r0: f32, rough: f32) -> vec3<f32> {
    let spread = rough * max(yc - r0, 0.0) * 0.25;
    return triangle(xs, i32(rround(r0)), i32(rround(spread)));
}

@compute @workgroup_size(8, 8)
fn reflect_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= W.width || gid.y >= W.height { return; }
    let w = W.width; let h = W.height;
    let x = gid.x; let y = gid.y;
    let i = y * w + x;
    let rr = refl[i * 6u];
    if rr < 0.002 { return; }
    let g = gbuf[i];
    let gid_ = g.idr & 0xFFu;
    if gid_ != ID_GROUND && gid_ != ID_CHASM { return; }
    let env = vec3<f32>(refl[i * 6u + 1u], refl[i * 6u + 2u], refl[i * 6u + 3u]);
    let rough = refl[i * 6u + 4u]; let sx = refl[i * 6u + 5u];
    let hz = W.horizon_px; let f = W.focal_px;
    let p = to_cam(g.x, g.y, g.d);
    let eye = max(-p.y, 0.05);
    let dw = g.depth;
    let yc = f32(y) + 0.5;
    let xs = u32(clamp(rround(f32(x) + rnd(rnd(sx * (yc - hz)) * 0.5)), 0.0, f32(w) - 1.0));
    let kr = div(dw, rnd(f * eye));
    let r_inf = 2.0 * hz - yc;
    let r_min = i32(ceil(max(r_inf, 0.0)));
    let n8 = nb8(); let n64 = nb64();
    var hit = -1;
    var r = i32(y) - 1;
    loop {
        if r < r_min { break; }
        let ru = u32(r);
        let top64 = ru / 64u * 64u;
        if i32(top64) >= r_min && !near_ray(mins[w * n8 + xs * n64 + ru / 64u], top64, kr, dw) { r = i32(top64) - 1; continue; }
        let top8 = ru / 8u * 8u;
        if i32(top8) >= r_min && !near_ray(mins[xs * n8 + ru / 8u], top8, kr, dw) { r = i32(top8) - 1; continue; }
        let z = gbuf[ru * w + xs].depth;
        if bitcast<u32>(z) != 0x7F800000u && near_ray(z, ru, kr, dw) { hit = r; break; }
        r -= 1;
    }
    var col: vec3<f32>;
    if hit >= 0 {
        col = tap(xs, yc, f32(hit), rough);
    } else if r_inf >= 0.0 && (gbuf[min(u32(r_inf), h - 1u) * w + xs].idr & 0xFFu) == ID_NONE {
        let k = smoothstep_r(0.0, 16.0, r_inf);
        col = mix3(env, tap(xs, yc, r_inf, rough), k);
    } else {
        col = env;
    }
    let c = vec3<f32>(hdr[i * 3u], hdr[i * 3u + 1u], hdr[i * 3u + 2u]) + (col - env) * rr;
    hdr[i * 3u] = c.x; hdr[i * 3u + 1u] = c.y; hdr[i * 3u + 2u] = c.z;
}
