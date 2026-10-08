//! A black hole in the space sky (`sky.space.black_hole`): its shadow, the accretion disk seen
//! through its own lensing (the far side over the top and under the bottom, the photon ring), the
//! starfield and nebulae bent round it, and matter falling in from both sides.
//!
//! Light is bent as a non-spinning (Schwarzschild) hole bends it. Lengths are in horizon radii
//! (Rs = 1). A ray from the camera passing at impact parameter `b` stays in the plane holding the
//! line of sight and its offset from the hole, and its orbit there, u = 1/r against the angle
//! turned, solves u'' + u = 1.5 u^2. That orbit depends on `b` alone, so it is solved once
//! (`table`, in f64) and both renderers read the same f32 table: where the orbit crosses the
//! disk's plane is a lookup, not a march. `sky.wgsl` repeats every function here step for step,
//! from the numbers `BhK::pack` hands it.

use super::*;
use super::space::{SpaceK, features, fbm_wrap};
use std::f32::consts::PI;
use std::sync::OnceLock;

/// The shadow's edge: rays passing closer than this fall in (3 sqrt(3) / 2).
const B_C: f32 = 2.598_076_2;
/// Beyond this the weak-field series bends the light (it agrees with the table there to 1e-6).
const B_MAX: f32 = 40.0;
/// Table rows: impact parameters inside the shadow (even), then outside (dense near the edge).
const NI: usize = 48;
const NO: usize = 112;
const NB: usize = NI + NO;
/// Samples along each orbit, over this much turned angle.
const NP: usize = 192;
const PSI_MAX: f32 = 3.0 * PI;
/// Disk bands, each turning a whole number of times per loop (the inner faster, as orbits do).
const BANDS: usize = 16;
pub(super) const HEAD: usize = 64;
pub(super) const DEBRIS_FLOATS: usize = 16;
pub(super) const MAX_DEBRIS: usize = 48;
/// The table's place in the packed block: the bending of each outside row, then the orbits.
pub(super) const ALPHA_AT: usize = HEAD;
pub(super) const U_AT: usize = HEAD + NO;
/// Where the streaks start (sky.wgsl's BH_DEBRIS; pinned by a test).
#[allow(dead_code)]
pub(super) const DEBRIS_AT: usize = U_AT + NB * NP;
/// Sentinels past an orbit's end: it has left (escaped) or fallen in.
const GONE: f32 = -1.0;
const FALLEN: f32 = 2.0;
/// Peak of the disk's flux profile (r_in / r)^3 (1 - sqrt(r_in / r)), at r = 49/36 r_in.
const FLUX_MAX: f32 = 0.056_710_3;

fn row_b(i: usize) -> f64 {
    let bc = B_C as f64;
    if i < NI { bc * (i as f64 + 0.5) / NI as f64 } else {
        let s = (i - NI + 1) as f64 / NO as f64;
        bc + (B_MAX as f64 - bc) * s * s
    }
}

/// The orbits: NO bendings (outside rows), then NB rows of NP samples of u.
fn table() -> &'static [f32] {
    static T: OnceLock<Vec<f32>> = OnceLock::new();
    T.get_or_init(|| {
        let mut alpha = vec![0f32; NO];
        let mut u_tab = vec![0f32; NB * NP];
        let dpsi = PI as f64 * 3.0 / (NP - 1) as f64;
        const SUB: usize = 64;
        let h = dpsi / SUB as f64;
        let acc = |u: f64| 1.5 * u * u - u;
        for i in 0..NB {
            let b = row_b(i);
            let (mut u, mut v) = (0.0f64, 1.0 / b);
            let mut end: Option<f32> = None;
            let mut fallen = false;
            let row = &mut u_tab[i * NP..(i + 1) * NP];
            row[0] = 0.0;
            'orbit: for j in 1..NP {
                for s in 0..SUB {
                    let u0 = u;
                    let (k1u, k1v) = (v, acc(u));
                    let (k2u, k2v) = (v + 0.5 * h * k1v, acc(u + 0.5 * h * k1u));
                    let (k3u, k3v) = (v + 0.5 * h * k2v, acc(u + 0.5 * h * k2u));
                    let (k4u, k4v) = (v + h * k3v, acc(u + h * k3u));
                    u += h / 6.0 * (k1u + 2.0 * k2u + 2.0 * k3u + k4u);
                    v += h / 6.0 * (k1v + 2.0 * k2v + 2.0 * k3v + k4v);
                    if u >= 1.0 { fallen = true; }
                    if u <= 0.0 && v < 0.0 {
                        let at = ((j - 1) * SUB + s) as f64 * h + h * u0 / (u0 - u);
                        end = Some(at as f32);
                    }
                    if fallen || end.is_some() {
                        for r in row.iter_mut().skip(j) { *r = if fallen { FALLEN } else { GONE }; }
                        break 'orbit;
                    }
                }
                row[j] = u as f32;
            }
            if i >= NI {
                // Still circling at the table's end (right at the edge): call it bent that far.
                alpha[i - NI] = end.unwrap_or(PSI_MAX) - PI;
            }
        }
        let mut t = alpha;
        t.extend(u_tab);
        t
    })
}

/// Fractional table row for impact parameter b (clamped to the side of the edge it is on).
fn row_of(b: f32) -> (usize, f32) {
    if b < B_C {
        let f = (b / B_C * NI as f32 - 0.5).clamp(0.0, (NI - 1) as f32);
        let i0 = (f.floor() as usize).min(NI - 2);
        (i0, (f - i0 as f32).clamp(0.0, 1.0))
    } else {
        let g = (((b - B_C) / (B_MAX - B_C)).sqrt() * NO as f32 - 1.0).clamp(0.0, (NO - 1) as f32);
        let j0 = (g.floor() as usize).min(NO - 2);
        (NI + j0, (g - j0 as f32).clamp(0.0, 1.0))
    }
}

/// How far a ray passing at b is bent (radians); negative inside the shadow, where it falls in.
fn alpha_at(t: &[f32], b: f32) -> f32 {
    if b < B_C { return -1.0; }
    if b >= B_MAX {
        let ib = 1.0 / b;
        return ib * (2.0 + ib * (2.945_243_1 + ib * 5.333_333_5));
    }
    let (r0, w) = row_of(b);
    let a0 = t[ALPHA_AT - HEAD + r0 - NI];
    a0 + (t[ALPHA_AT - HEAD + r0 - NI + 1] - a0) * w
}

/// u = 1/r on the orbit at impact parameter b after turning `d`; GONE or FALLEN past its end.
fn u_at(t: &[f32], b: f32, d: f32) -> f32 {
    let (r0, w) = row_of(b);
    let q = d / (PSI_MAX / (NP - 1) as f32);
    let j0 = (q.floor().max(0.0) as usize).min(NP - 2);
    let s = (q - j0 as f32).clamp(0.0, 1.0);
    let at = |r: usize, j: usize| t[U_AT - HEAD + r * NP + j];
    let (a, b2, c, e) = (at(r0, j0), at(r0, j0 + 1), at(r0 + 1, j0), at(r0 + 1, j0 + 1));
    if a < 0.0 || b2 < 0.0 || c < 0.0 || e < 0.0 { return GONE; }
    if a > 1.5 || b2 > 1.5 || c > 1.5 || e > 1.5 { return FALLEN; }
    let u0 = a + (b2 - a) * s;
    let u1 = c + (e - c) * s;
    u0 + (u1 - u0) * w
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] { [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]] }
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 { a[0] * b[0] + a[1] * b[1] + a[2] * b[2] }

/// A black hole for one frame, as both renderers draw it.
#[derive(Clone, Debug)]
pub(super) struct BhK {
    /// Centre (pixels), pixels per horizon radius, and the lensing's reach (pixels per radian).
    pub cx: f32,
    pub cy: f32,
    pub ppr: f32,
    pub lens: f32,
    pub r_in: f32,
    pub r_out: f32,
    /// Pixels from the centre inside which the disk can show.
    pub reach: f32,
    /// The way the disk turns (+1 / -1), its Doppler and brightness.
    pub dir: f32,
    pub doppler: f32,
    pub bright: f32,
    /// The disk's normal (towards the side seen) and two axes in its plane.
    pub n: [f32; 3],
    pub d1: [f32; 3],
    pub d2: [f32; 3],
    pub hot: [f32; 3],
    pub mid: [f32; 3],
    pub cool: [f32; 3],
    /// Each band's turn so far this loop (0..1).
    pub turn: [f32; BANDS],
    /// The thin photon ring: its width (pixels), the shadow's radius, and the side of the disk
    /// coming towards you on screen (x right, y up).
    pub ring_w: f32,
    pub rpx: f32,
    pub towards: [f32; 2],
    /// The falling nodes' body, and their two glows.
    pub node_cols: [[f32; 3]; 3],
    /// What falls in, each `DEBRIS_FLOATS` long, kind first. A streak (0): a, b (pixels), width,
    /// colour, strength. A node (1): centre, radius, stretch along its way (cos, sin), its two
    /// rings' axes, strength, redness.
    pub debris: Vec<[f32; DEBRIS_FLOATS]>,
}

impl BhK {
    /// The block `sky.wgsl` reads: the header, the orbit table, then the streaks.
    pub(super) fn pack(&self) -> Vec<f32> {
        let mut h = [0f32; HEAD];
        h[..10].copy_from_slice(&[self.cx, self.cy, self.ppr, self.lens, self.r_in, self.r_out, self.reach, self.dir, self.doppler, self.bright]);
        h[10..13].copy_from_slice(&self.n);
        h[13..16].copy_from_slice(&self.d1);
        h[16..19].copy_from_slice(&self.d2);
        h[19..22].copy_from_slice(&self.hot);
        h[22..25].copy_from_slice(&self.mid);
        h[25..28].copy_from_slice(&self.cool);
        h[28] = self.debris.len() as f32;
        h[29] = self.ring_w;
        h[30..30 + BANDS].copy_from_slice(&self.turn);
        h[46..48].copy_from_slice(&self.towards);
        h[48] = self.rpx;
        for (j, c) in self.node_cols.iter().enumerate() { h[50 + 3 * j..53 + 3 * j].copy_from_slice(c); }
        let mut out = h.to_vec();
        out.extend_from_slice(table());
        for d in &self.debris { out.extend_from_slice(d); }
        out
    }
}

pub(super) fn bh_k(ctx: &Ctx) -> Option<BhK> {
    let sky = &ctx.scene.sky;
    let bh = &sky.space.black_hole;
    if !sky.enabled || !sky.space.enabled || !bh.enabled || bh.size <= 0.0 { return None; }
    let v = &ctx.view;
    let hy = v.horizon_px.max(2.0);
    let (ox, oy) = (v.left as f32, v.top as f32);
    let (fw, fhy) = (v.width as f32 - 2.0 * ox, (hy - oy).max(2.0));
    let (cx, cy) = (ox + bh.pos[0] * fw, oy + bh.pos[1] * fhy);
    let rpx = bh.size * fhy;
    let ppr = rpx / B_C;
    let r_in = bh.disk_inner.clamp(1.6, 20.0);
    let r_out = bh.disk_outer.clamp(r_in + 0.5, 30.0);
    let (ti, ro) = (bh.tilt.clamp(0.0, 89.0).to_radians(), bh.roll.to_radians());
    let n = [-ro.sin() * ti.sin(), ro.cos() * ti.sin(), -ti.cos()];
    let d1 = [ro.cos(), ro.sin(), 0.0];
    let d2 = cross(n, d1);
    let dir = if bh.spin < 0 { -1.0 } else { 1.0 };
    let mut turn = [0f32; BANDS];
    for (j, t) in turn.iter_mut().enumerate() {
        let r = r_in + (j as f32 + 0.5) / BANDS as f32 * (r_out - r_in);
        let k = (bh.spin.unsigned_abs() as f32 * (r_in / r).powf(1.5)).round().max(if bh.spin != 0 { 1.0 } else { 0.0 });
        *t = (dir * k * ctx.tphase).rem_euclid(1.0);
    }
    let scale = ((v.height as f32 - oy) / 854.0).max(0.25);
    let q = ti.cos().max(0.08);
    let ic = rgb_lin(bh.infall_color);
    let red = [1.0, 0.06, 0.02];
    let rh = rpx * 0.92;
    // Every object's trips: 1-2 (nodes) or 2-4 (gas) a loop, times `infall_speed`, and each trip
    // rolls its own numbers and the pauses either side of it, so nothing comes in to a beat; the
    // loop still closes, as the same trips come round again. Gives the trip's progress (outside
    // 0..1 between trips) and its own random numbers.
    let slot = |i: usize, salt: u32, base: f32, extra: f32| {
        let hi = |k: i64| hf(bh.seed ^ salt, i as i64 * 8 + k);
        let m = bh.infall_speed.max(1) as f32 * (base + (hi(0) * (extra + 1.0)).floor());
        let x = ctx.tphase * m + hi(1);
        let j = (x.floor() as i64).rem_euclid(m as i64);
        let u = x - x.floor();
        let hh = move |k: i64| hf(bh.seed ^ salt ^ 0x7121, (i as i64 * 64 + j) * 16 + k);
        let a0 = 0.3 * hh(5);
        let len = (1.0 - a0 - 0.25 * hh(6)).max(0.35);
        ((u - a0) / len, hh)
    };
    // Gone at the shadow's rim, wherever on screen it gets there.
    let rim_fade = |p: [f32; 2]| smoothstep(rpx * 1.05, rpx * 1.5, ((p[0] - cx) * (p[0] - cx) + (p[1] - cy) * (p[1] - cy)).sqrt());
    let mut debris = Vec::new();
    // Gas at the hole's own scale: from beyond the disk's edge, spiralling in its plane, slowing
    // and reddening as it nears the shadow.
    let disk_px = ppr * r_out;
    for i in 0..(bh.infall as usize).min(MAX_DEBRIS) {
        let (f, hh) = slot(i, 0xBD0, 2.0, 2.0);
        if !(0.0..1.0).contains(&f) { continue; }
        let r0 = disk_px * (1.1 + 0.6 * hh(2));
        let th0 = TAU * hh(3);
        let (swirl, pace) = (0.5 + 0.8 * hh(8), 1.0 + 0.7 * hh(9));
        let at = |f: f32| -> ([f32; 2], f32) {
            let e = smoothstep(0.0, 1.0, f).powf(pace);
            let r = rh + (r0 - rh) * (1.0 - e);
            let th = th0 - dir * swirl * ((r0 / r).powf(0.8) - 1.0);
            let (lx, ly) = (r * th.cos(), r * th.sin() * q);
            ([cx + lx * ro.cos() - ly * ro.sin(), cy - (lx * ro.sin() + ly * ro.cos())], r)
        };
        let (a, _) = at((f - 0.03).max(0.0));
        let (b, r) = at(f);
        let w = scale * (0.5 + 0.8 * hh(4)) * (r / r0).powf(0.3);
        let heat = smoothstep(0.2, 0.9, f);
        let col = mix3(ic, red, smoothstep(0.7, 1.0, f));
        let k = (0.35 + 1.4 * heat) * smoothstep(0.0, 0.1, f) * (1.0 - smoothstep(0.9, 1.0, f)) * (0.6 + 0.8 * hh(10)) * rim_fade(b);
        if k <= 0.0 { continue; }
        let mut d = [0f32; DEBRIS_FLOATS];
        d[..10].copy_from_slice(&[0.0, a[0], a[1], b[0], b[1], w, col[0], col[1], col[2], k]);
        debris.push(d);
    }
    // Nodes like the game's hook nodes, at play depth: each floats in slowly from its side, level,
    // untouched until it nears the middle; there the hole takes it and pulls it back, away from
    // the camera, towards the hole `distance` times further off. In perspective it shrinks to a
    // speck and, though falling faster and faster, crawls on screen as it goes: that is what says
    // the hole is far away. Near the end it swirls into the disk's plane, hazes and reddens, and
    // fades at the rim.
    let depth = bh.distance.max(2.0);
    let vx0 = v.width as f32 * 0.5;
    // The disk's long axis on screen (y down).
    let e1 = [d1[0], -d1[1]];
    let nodes = (bh.nodes as usize).min(MAX_DEBRIS - debris.len());
    for i in 0..nodes {
        let (f, hh) = slot(i, 0x40DE, 1.0, 1.0);
        if !(0.0..1.0).contains(&f) { continue; }
        let side = if hh(7) < 0.5 { 1.0 } else { -1.0 };
        let rad0 = 0.5 * bh.node_size.max(0.0) * fw * (0.8 + 0.4 * hh(4));
        let p = 0.55 + 0.15 * hh(8);
        let x_start = if side > 0.0 { ox + fw } else { ox } + side * 2.0 * rad0;
        let x_end = vx0 + side * (0.06 + 0.22 * hh(3)) * fw;
        let y0 = oy + (0.12 + 0.76 * hh(2)) * (v.height as f32 - oy);
        let bob = 0.15 * rad0;
        let ph = TAU * hh(11);
        // Phase one: level and slow, at depth 1 (play depth).
        let drift = |t: f32| [x_start + (x_end - x_start) * t, y0 + bob * (TAU * 1.5 * t + ph).sin()];
        // The pull. A straight line in depth from the node to the hole, seen in perspective,
        // closes on the hole's place on screen as (1 - e) / z: always inwards, fast at first and
        // crawling as it recedes. Its sideways drift carries on inwards a moment as the pull takes
        // over, and its height above or below the disk's plane dies away faster, so it settles
        // into the disk as it nears.
        let e0 = drift(1.0);
        let off0 = [e0[0] - cx, e0[1] - cy];
        let ax = { let l = (e1[0] * e1[0] + e1[1] * e1[1]).sqrt().max(1e-4); [e1[0] / l, e1[1] / l] };
        let dv = ((x_end - x_start) * (1.0 - p) / p).abs();
        let pos = |f: f32| -> ([f32; 2], f32, f32) {
            if f < p { return (drift(f / p), 1.0, 0.0); }
            let s = (f - p) / (1.0 - p);
            let e = s.powf(2.2);
            let z = 1.0 + (depth - 1.0) * e;
            let shrink = (1.0 - e) / z;
            let c = (0.5 * dv * (1.0 - (1.0 - s) * (1.0 - s))).min(0.8 * off0[0].abs());
            let ox = off0[0] - off0[0].signum() * c;
            let (u, w) = (ox * ax[0] + off0[1] * ax[1], -ox * ax[1] + off0[1] * ax[0]);
            let w = w * (1.0 - 0.85 * smoothstep(0.35, 0.95, s));
            ([cx + (u * ax[0] - w * ax[1]) * shrink, cy + (u * ax[1] + w * ax[0]) * shrink], z, s)
        };
        let (b, z, s2) = pos(f);
        let (a, _, _) = pos((f - 0.004).max(0.0));
        let (vx, vy) = (b[0] - a[0], b[1] - a[1]);
        let vl = (vx * vx + vy * vy).sqrt();
        let (dc, ds) = if vl > 1e-4 { (vx / vl, vy / vl) } else { (1.0, 0.0) };
        let rad = (rad0 / z).max(0.6);
        let stretch = 1.0 + 0.8 * smoothstep(0.75, 1.0, s2);
        let haze = 1.0 - 0.5 * smoothstep(0.0, 1.0, s2);
        let k = smoothstep(0.0, 0.03, f) * (1.0 - smoothstep(0.94, 1.0, f)) * rim_fade(b) * haze * (rad0 / z / rad).min(1.0).sqrt();
        if k <= 0.0 { continue; }
        let spin = TAU * (1.5 * f + hh(9));
        let (c1, s1) = (spin.cos(), spin.sin());
        let (c2, s2r) = ((1.3 * spin + 1.0).cos(), (1.3 * spin + 1.0).sin());
        let a1 = [0.5 * s1, 0.866, 0.5 * c1];
        let a2 = [0.866 * c2, -0.5 * s2r, 0.5];
        let mut d = [0f32; DEBRIS_FLOATS];
        d[..15].copy_from_slice(&[1.0, b[0], b[1], rad, stretch, dc, ds, a1[0], a1[1], a1[2], a2[0], a2[1], a2[2], k, smoothstep(0.5, 1.0, s2)]);
        debris.push(d);
    }
    let zc = n[0] * d1[1] - n[1] * d1[0];
    let sgn = if dir * zc < 0.0 { 1.0 } else { -1.0 };
    Some(BhK {
        cx, cy, ppr,
        lens: bh.lensing.max(0.0) * rpx,
        r_in, r_out,
        reach: ppr * (r_out * 1.15 + 1.0),
        dir, doppler: bh.doppler.clamp(0.0, 1.0), bright: bh.brightness.max(0.0),
        n, d1, d2,
        hot: rgb_lin(bh.disk_colors[0]), mid: rgb_lin(bh.disk_colors[1]), cool: rgb_lin(bh.disk_colors[2]),
        turn,
        ring_w: (0.012 * rpx).max(0.6),
        rpx,
        towards: [sgn * d1[0], sgn * d1[1]],
        node_cols: [rgb_lin(bh.node_colors[0]), rgb_lin(bh.node_colors[1]), rgb_lin(bh.node_colors[2])],
        debris,
    })
}

/// The light its disk sheds on the path, added to the ambient.
pub(super) fn bh_glow(sky: &crate::scene::Sky) -> [f32; 3] {
    let bh = &sky.space.black_hole;
    if !sky.enabled || !sky.space.enabled || !bh.enabled { return [0.0; 3]; }
    scale3(rgb_lin(bh.disk_colors[1]), 0.06 * bh.light.max(0.0) * bh.brightness.max(0.0))
}

/// The disk's light where a ray crosses it at radius r, angle phi round it, Doppler factor g.
fn disk_light(k: &BhK, r: f32, phi: f32, g: f32) -> ([f32; 3], f32) {
    let x = k.r_in / r;
    let f = x * x * x * (1.0 - x.sqrt()) / FLUX_MAX;
    let fade = smoothstep(k.r_in, k.r_in * 1.15, r) * (1.0 - smoothstep(k.r_in + 0.6 * (k.r_out - k.r_in), k.r_out, r));
    if fade <= 0.0 { return ([0.0; 3], 0.0); }
    let u0 = phi / (2.0 * PI) + 0.5;
    let bf = (r - k.r_in) / (k.r_out - k.r_in) * BANDS as f32 - 0.5;
    let j0 = (bf.floor().max(0.0) as usize).min(BANDS - 1);
    let j1 = (j0 + 1).min(BANDS - 1);
    let w = smoothstep(0.0, 1.0, bf - j0 as f32);
    let band = |j: usize| {
        let u = u0 - k.turn[j];
        let u = u - u.floor();
        0.55 * fbm_wrap(u, r * 0.55, 6, 1.0) + 0.45 * fbm_wrap(u, r * 3.0, 20, 1.0)
    };
    let nz = band(j0) + (band(j1) - band(j0)) * w;
    let t = f.max(0.0).powf(0.25) * g;
    let col = mix3(mix3(k.cool, k.mid, smoothstep(0.25, 0.65, t)), k.hot, smoothstep(0.6, 1.0, t));
    let i = k.bright * f * g * g * g * (0.35 + 1.3 * nz) * fade;
    (scale3(col, i), (fade * (0.35 + 0.7 * nz)).clamp(0.0, 0.92))
}

/// The pixel (x, y) of the space sky with the hole in it: the disk's crossings front to back, then
/// the backdrop bent round it (or the shadow). `c` is the sky's base there (`backdrop_px`'s).
pub(super) fn lensed_px(k: &BhK, sk: &SpaceK, x: usize, y: usize, c: [f32; 3]) -> [f32; 3] {
    let t = table();
    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
    let (ox, oy) = (px - k.cx, k.cy - py);
    let d = (ox * ox + oy * oy).sqrt().max(1e-3);
    let b = d / k.ppr;
    let (ux, uy) = (ox / d, oy / d);
    let mut out = [0.0f32; 3];
    let mut tr = 1.0f32;
    if d < k.reach {
        let e1 = [ux, uy, 0.0];
        let a = dot(k.n, e1);
        let d0 = (-a / k.n[2]).atan() + 0.5 * PI;
        let alpha = alpha_at(t, b);
        let peri = if alpha < 0.0 { 1e9 } else { 0.5 * (PI + alpha) };
        let ib2 = 1.0 / (b * b);
        for kk in 0..3 {
            let dl = d0 + kk as f32 * PI;
            if dl > PSI_MAX { break; }
            let u = u_at(t, b, dl);
            if u == GONE || u == FALLEN { break; }
            if !(u > 0.0 && u < 1.0) { continue; }
            let r = 1.0 / u;
            if r < k.r_in || r > k.r_out { continue; }
            let psi = dl - 0.5 * PI;
            let (cs, sn) = (psi.cos(), psi.sin());
            let rh = [cs * ux, cs * uy, sn];
            let ph = [-sn * ux, -sn * uy, cs];
            let p = [r * rh[0], r * rh[1], r * rh[2]];
            let phi = dot(p, k.d2).atan2(dot(p, k.d1));
            // The photon's way at the crossing (towards the camera), and the gas's.
            let up = (ib2 - u * u + u * u * u).max(0.0).sqrt() * if dl < peri { 1.0 } else { -1.0 };
            let drp = -up / (u * u);
            let tv = [drp * rh[0] + r * ph[0], drp * rh[1] + r * ph[1], drp * rh[2] + r * ph[2]];
            let tl = dot(tv, tv).sqrt().max(1e-6);
            let kv = [-tv[0] / tl, -tv[1] / tl, -tv[2] / tl];
            let vv = cross(k.n, rh);
            let beta = (0.5 / (r - 1.0)).sqrt().min(0.95);
            let cos_t = k.dir * dot(vv, kv);
            let g = (1.0 - 1.0 / r).sqrt() * (1.0 - beta * beta).sqrt() / (1.0 - beta * cos_t);
            let g = 1.0 + k.doppler * (g - 1.0);
            let (e, op) = disk_light(k, r, phi, g);
            out = add3(out, scale3(e, tr));
            tr *= 1.0 - op;
        }
    }
    if b >= B_C {
        let shift = k.lens * alpha_at(t, b);
        let bg = features(sk, px - ux * shift, py + uy * shift, c);
        out = add3(out, scale3(bg, tr));
        // The photon ring: light that circled the hole, a hairline just outside the shadow, kept
        // at least a pixel wide and brighter on the side coming towards you.
        let q = (d - k.rpx * 1.018) / k.ring_w;
        if q.abs() < 4.0 {
            let side = 0.5 + 0.5 * (ux * k.towards[0] + uy * k.towards[1]);
            let gain = 1.0 + k.doppler * (0.15 + 1.6 * side - 1.0);
            out = add3(out, scale3(k.hot, 0.45 * k.bright * gain * (-q * q).exp() * tr));
        }
    }
    out
}

/// A falling node over (px, py): the colour, and how much it covers.
fn node_at(k: &BhK, s: &[f32; DEBRIS_FLOATS], px: f32, py: f32, c: [f32; 3]) -> [f32; 3] {
    let (dx, dy) = (px - s[1], py - s[2]);
    let (rad, st, dc, ds) = (s[3], s[4], s[5], s[6]);
    let (lx, ly) = ((dx * dc + dy * ds) / st, -dx * ds + dy * dc);
    let q = (lx * lx + ly * ly).sqrt() / rad;
    if q > 2.2 { return c; }
    let [body, g1, g2] = k.node_cols;
    let redden = |v: [f32; 3]| { let r = s[14]; [v[0] * (1.0 + 0.6 * r), v[1] * (1.0 - 0.7 * r), v[2] * (1.0 - 0.8 * r)] };
    // Dimmed and drawn towards blue-grey, so they never pass for the real thing.
    let dim = |v: [f32; 3]| { let l = 0.3 * v[0] + 0.59 * v[1] + 0.11 * v[2]; scale3(mix3(v, [0.62 * l, 0.66 * l, 0.8 * l], 0.3), 0.75) };
    let mut c = c;
    let mut cover = 0.0;
    if q < 1.0 {
        // The ball's normal on screen (y down), lit from the upper left.
        let (nx, ny) = ((lx * dc - ly * ds) / rad, (lx * ds + ly * dc) / rad);
        let nz = (1.0 - q * q).max(0.0).sqrt();
        let lit = (-0.45 * nx - 0.55 * ny + 0.70 * nz).max(0.0);
        let mut col = scale3(body, 0.25 + 0.75 * lit);
        col = add3(col, scale3(g1, 0.5 * lit.powi(12)));
        let r1 = (nx * s[7] + ny * s[8] + nz * s[9]) / 0.07;
        let r2 = (nx * s[10] + ny * s[11] + nz * s[12]) / 0.07;
        col = add3(col, add3(scale3(g1, 1.2 * (-r1 * r1).exp()), scale3(g2, 1.2 * (-r2 * r2).exp())));
        col = add3(col, scale3(g2, 0.4 * (1.0 - nz).powi(3)));
        cover = ((1.0 - q) * rad + 0.5).clamp(0.0, 1.0) * s[13];
        c = mix3(c, dim(redden(col)), cover);
    }
    let halo = (-(q - 1.0).max(0.0) / 0.35).exp() * 0.25 * s[13] * (1.0 - cover);
    add3(c, scale3(dim(redden(g2)), halo))
}

/// The matter falling in, over pixel (x, y).
pub(super) fn infall_px(k: &BhK, x: usize, y: usize, c: [f32; 3]) -> [f32; 3] {
    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
    let mut c = c;
    for s in &k.debris {
        if s[0] > 0.5 {
            let e = s[3] * s[4] * 2.2 + 2.0;
            if (px - s[1]).abs() > e || (py - s[2]).abs() > e { continue; }
            c = node_at(k, s, px, py, c);
            continue;
        }
        let (ax, ay, bx, by, w) = (s[1], s[2], s[3], s[4], s[5]);
        let e = 10.0 * w + 2.0;
        if px < ax.min(bx) - e || px > ax.max(bx) + e || py < ay.min(by) - e || py > ay.max(by) + e { continue; }
        let (abx, aby) = (bx - ax, by - ay);
        let tt = (((px - ax) * abx + (py - ay) * aby) / (abx * abx + aby * aby).max(1e-6)).clamp(0.0, 1.0);
        let (ex, ey) = (px - ax - abx * tt, py - ay - aby * tt);
        let dd = (ex * ex + ey * ey).sqrt();
        if dd > e { continue; }
        let core = 1.0 - smoothstep(w, w + 1.0, dd);
        let glow = (-dd / (2.5 * w)).exp();
        c = add3(c, scale3([s[6], s[7], s[8]], s[9] * (1.4 * core + 0.45 * glow)));
    }
    c
}

/// The streaks of matter, over every empty pixel they reach.
pub(super) fn draw_infall(ctx: &Ctx, gbuf: &[GPixel], hdr: &mut [[f32; 3]]) {
    let Some(sk) = super::space::space_k(ctx) else { return };
    let Some(k) = bh_k(ctx) else { return };
    if k.debris.is_empty() { return; }
    let w = ctx.view.width;
    hdr.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        if (y as f32) >= sk.hy && !sk.below { return; }
        for (x, px) in row.iter_mut().enumerate() {
            if gbuf[y * w + x].id == id::NONE { *px = infall_px(&k, x, y, *px); }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preset(n: &str) -> Scene { crate::scene::presets::ALL.iter().find(|(m, _)| *m == n).unwrap().1() }
    fn diff(a: &Image, b: &Image) -> f32 { a.rgba.iter().zip(&b.rgba).map(|(x, y)| (*x as f32 - *y as f32).abs()).sum::<f32>() / a.rgba.len() as f32 }
    fn opts(t: f32) -> RenderOptions { RenderOptions { size: Some((180, 320)), time: Some(t), ..RenderOptions::default() } }

    #[test]
    fn the_black_hole_loops_and_its_shadow_is_dark() {
        let mut r = WorldRenderer::default();
        let s = preset("Event Horizon");
        let (len, secs) = (s.motion.loop_length, s.motion.loop_seconds());
        let a = r.render(&s, 0.0, &opts(0.0));
        assert_eq!(a.rgba, r.render(&s, len, &opts(secs)).rgba, "the loop must close");
        // Standing still, the disk turns and matter falls in.
        let e = r.render(&s, 0.0, &opts(secs * 0.37));
        assert!(diff(&a, &e) > 0.05, "the disk and the infall should move with time alone ({})", diff(&a, &e));
        // The shadow's middle is black (bar a little bloom); the disk beside it is bright.
        let bh = &s.sky.space.black_hole;
        let (w, h) = (180usize, 320usize);
        let hy = s.camera.horizon * h as f32;
        let (cx, cy) = ((bh.pos[0] * w as f32) as usize, (bh.pos[1] * hy) as usize);
        let px = |x: usize, y: usize| { let i = (y * w + x) * 4; a.rgba[i] as u32 + a.rgba[i + 1] as u32 + a.rgba[i + 2] as u32 };
        assert!(px(cx, cy) < 30, "the shadow should be black (bloom lifts it a little) ({})", px(cx, cy));
        let rpx = bh.size * hy;
        let side = (0..w).filter(|&x| (x as f32 - cx as f32).abs() > 1.3 * rpx).map(|x| px(x, cy)).max().unwrap();
        assert!(side > 400, "the disk should cross the frame bright beside the shadow ({side})");
    }

    #[test]
    fn the_shader_reads_the_block_where_pack_writes_it() {
        let wgsl = include_str!("../gpu/sky.wgsl");
        for (name, v) in [("BH_NI", NI), ("BH_NO", NO), ("BH_NP", NP), ("BH_BANDS", BANDS), ("BH_ALPHA", ALPHA_AT), ("BH_U", U_AT), ("BH_DEBRIS", DEBRIS_AT), ("BH_STRIDE", DEBRIS_FLOATS)] {
            assert!(wgsl.contains(&format!("const {name}: u32 = {v}u;")), "{name} should be {v}");
        }
        assert_eq!(30 + BANDS, 46);
        assert!(59 <= HEAD);
    }

    #[test]
    fn the_orbit_table_bends_light_as_a_black_hole_does() {
        let t = table();
        // Far out, the table's bending meets the weak-field series it hands over to.
        let near_max = alpha_at(t, B_MAX - 1e-3);
        let series = { let ib = 1.0 / B_MAX; ib * (2.0 + ib * (2.945_243_1 + ib * 5.333_333_5)) };
        assert!((near_max - series).abs() < 2e-4, "table {near_max} series {series}");
        // At b = 5 the exact bending is 0.6.. (about 2/b + 3/b^2 + ...).
        let a5 = alpha_at(t, 5.0);
        assert!(a5 > 0.55 && a5 < 0.7, "alpha(5) = {a5}");
        // Bending grows without bound towards the shadow's edge, and inside it rays fall in.
        assert!(alpha_at(t, B_C + 0.01) > 2.5);
        assert!(alpha_at(t, B_C - 0.01) < 0.0);
        // A ray inside the shadow falls in; one well outside leaves.
        assert_eq!(u_at(t, 1.0, PSI_MAX - 0.01), FALLEN);
        assert_eq!(u_at(t, 10.0, PSI_MAX - 0.01), GONE);
        // Closest approach for b = 10: 1/u solving 1/b^2 = u^2 - u^3, about 9.47.
        let peri = 0.5 * (PI + alpha_at(t, 10.0));
        let r = 1.0 / u_at(t, 10.0, peri);
        assert!((r - 9.47).abs() < 0.05, "periapsis {r}");
    }
}
