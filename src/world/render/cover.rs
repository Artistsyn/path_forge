//! Ground cover (`ground_cover`): grass, tall grass and wildflowers standing on the ground, and
//! moss lying on it, over the verge, the path or both. Nothing is drawn as a sprite: each ground
//! pixel looks back along its line of sight through the height the cover stands to, and takes the
//! first tuft it meets (tufts stand one to a cell of a grid on the ground, each a clump of blades
//! of its own height, leaning with the wind). Far off, where a line of sight crosses many tufts,
//! the ground simply takes their colour. `shade.wgsl` (`cover_at`) repeats `at` step for step.

use super::*;
use crate::scene::{CoverKind, CoverOn};

/// Layers the GPU has room for.
pub(super) const MAX: usize = 3;
/// Looks back along the line of sight.
const STEPS: usize = 8;

/// One layer for one frame. `kind` 1 grass, 2 tall grass, 3 wildflowers, 4 moss; `on` 1 the
/// verge, 2 the path, 3 both.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct CoverK {
    pub kind: u32,
    pub on: u32,
    pub density: f32,
    pub height: f32,
    pub col: [f32; 3],
    pub patch: f32,
    pub col2: [f32; 3],
    pub seed: u32,
    /// The grid: a cell's side, how many make the loop, and a tuft's radius at the ground.
    pub cs: f32,
    pub ncell: f32,
    pub r0: f32,
}

impl CoverK {
    /// The layer as the GPU takes it (`World.gc`).
    pub(super) fn rec(&self) -> [[f32; 4]; 4] {
        [
            [self.kind as f32, self.on as f32, self.density, self.height],
            [self.col[0], self.col[1], self.col[2], self.patch],
            [self.col2[0], self.col2[1], self.col2[2], self.seed as f32],
            [self.cs, self.ncell, self.r0, 0.0],
        ]
    }
}

pub(super) fn plan(scene: &Scene, loop_len: f32) -> Vec<CoverK> {
    scene.ground_cover.iter().filter(|c| c.enabled && c.density > 0.0).take(MAX).map(|c| {
        let height = c.height.clamp(0.02, 1.5);
        let cs0 = match c.kind {
            CoverKind::Grass | CoverKind::TallGrass => (0.45 * height).clamp(0.04, 0.2),
            CoverKind::Wildflowers => (0.5 * height).clamp(0.12, 0.3),
            CoverKind::Moss => 0.5,
        };
        let ncell = (loop_len / cs0).round().max(1.0);
        let cs = loop_len / ncell;
        CoverK {
            kind: match c.kind { CoverKind::Grass => 1, CoverKind::TallGrass => 2, CoverKind::Wildflowers => 3, CoverKind::Moss => 4 },
            on: match c.on { CoverOn::Verge => 1, CoverOn::Path => 2, CoverOn::Both => 3 },
            density: c.density.clamp(0.0, 1.0), height,
            col: rgb_lin(c.color), patch: c.patchiness.clamp(0.0, 1.0), col2: rgb_lin(c.color2),
            // (Small enough to be a whole number in a float.)
            seed: c.seed % 60_000,
            cs, ncell, r0: 0.42 * cs,
        }
    }).collect()
}

/// Smooth noise over the ground that repeats with the loop, in cells about `cell` metres across.
/// A 32-bit mix of a seed and a key. The march asks for several numbers a step, and the scene's 64-bit
/// `hash` has to be built from 32-bit halves on a GPU: one of those a step cost more than the rest of the tuft.
fn h32(seed: u32, key: i32) -> u32 {
    let mut x = (key as u32) ^ seed.wrapping_mul(0x9E37_79B9);
    x ^= x >> 16;
    x = x.wrapping_mul(0x7FEB_352D);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846C_A68B);
    x ^ (x >> 16)
}
/// `h32` as 0..1.
fn h32f(seed: u32, key: i32) -> f32 { (h32(seed, key) >> 8) as f32 / 16_777_216.0 }
/// Where between two march steps pixel `i` starts, so the steps do not show as bands.
pub(super) fn jitter(i: usize) -> f32 { h32f(0x6C, i as i32) }

fn patch_noise(x: f32, w: f32, loop_len: f32, cell: f32, seed: u32) -> f32 {
    let c = snap_to_loop(cell, loop_len);
    let n = (loop_len / c).round() as i32;
    let (tx, tw) = (x / c, w / c);
    let (ix, iw) = (tx.floor(), tw.floor());
    let (fx, fw) = (tx - ix, tw - iw);
    let (sx, sw) = (fx * fx * (3.0 - 2.0 * fx), fw * fw * (3.0 - 2.0 * fw));
    let h = |i: i32, j: i32| h32f(seed, i.wrapping_mul(7919).wrapping_add(j.rem_euclid(n).wrapping_mul(104_729)));
    let (i, j) = (ix as i32, iw as i32);
    let a = h(i, j) + (h(i + 1, j) - h(i, j)) * sx;
    let b = h(i, j + 1) + (h(i + 1, j + 1) - h(i, j + 1)) * sx;
    a + (b - a) * sw
}

/// How thick a layer grows at a place: all of it where it is even, in patches where it is not.
fn thick(k: &CoverK, x: f32, w: f32, loop_len: f32) -> f32 {
    if k.patch <= 0.0 { return 1.0; }
    1.0 - k.patch * (1.0 - smoothstep(0.35, 0.65, patch_noise(x, w, loop_len, 1.2, k.seed ^ 0xB1)))
}

/// Byte `i` of a hash as 0..1: one hash gives a tuft four of its numbers.
fn part(h: u32, i: u32) -> f32 { ((h >> (8 * i)) & 0xFF) as f32 / 256.0 }

/// The tuft of layer `k` standing where (x, w) is, if the point `h` metres up is inside it: its colour.
fn tuft(k: &CoverK, wx: (f32, f32, f32), x: f32, w: f32, h: f32, grown: f32) -> Option<[f32; 3]> {
    let (fx, fz) = (x / k.cs, w / k.cs);
    let (ci, cj) = (fx.floor(), fz.floor());
    let key = (ci as i32).wrapping_mul(7919).wrapping_add((cj as i32).rem_euclid(k.ncell as i32).wrapping_mul(104_729));
    // Whether a tuft stands in this cell, and how tall.
    let h1 = h32(k.seed ^ 0xC1, key);
    if (h1 & 0xFFFF) as f32 / 65536.0 >= k.density * grown { return None; }
    let hb = k.height * (0.55 + 0.45 * (h1 >> 16) as f32 / 65536.0);
    if h >= hb { return None; }
    let t = h / hb;
    // Its lean, its place in the wind's sway, and where in the cell it stands.
    let h2 = h32(k.seed ^ 0xC3, key);
    // It leans its own way, and with the wind, more towards its tips.
    let mut lean = (part(h2, 0) - 0.5) * 0.5;
    if wx.0 != 0.0 { lean += wx.0 * (wx.1 + 0.35 * (wx.2 + TAU * part(h2, 1)).sin()) * 2.5; }
    let dx = (fx - ci - 0.3 - 0.4 * part(h2, 2)) * k.cs - lean * h * t;
    let dz = (fz - cj - 0.3 - 0.4 * part(h2, 3)) * k.cs;
    let q = dx * dx + dz * dz;
    if q >= k.r0 * k.r0 { return None; }
    // Its blades' offset, its tone, and for a flower its colour and shade.
    let h3 = h32(k.seed ^ 0xC7, key);
    let tone = 0.8 + 0.4 * part(h3, 1);
    if k.kind == 3 {
        // A flower: a thin stem and a head, some heads one colour, some another, a few white.
        let head = t > 0.7;
        let rr = if head { 0.5 * k.r0 * (1.0 - ((t - 0.85) / 0.15).powi(2)).max(0.0).sqrt() } else { 0.14 * k.r0 };
        if q >= rr * rr { return None; }
        if !head { return Some(scale3(k.col, 0.55 * tone)); }
        let pick = part(h3, 2);
        let petal = if pick < 0.55 { k.col2 } else if pick < 0.8 { [k.col2[2], k.col2[0], k.col2[1]] } else { [0.9, 0.9, 0.82] };
        return Some(scale3(petal, 0.85 + 0.3 * part(h3, 3)));
    }
    let rr = k.r0 * (1.0 - 0.7 * t);
    if q >= rr * rr { return None; }
    // The clump is blades side by side, each its own height, with air between them.
    let sl = dx / (0.4 * k.r0) + 7.0 * part(h3, 0);
    let si = sl.floor();
    let bh = hb * (0.6 + 0.4 * h32f(k.seed ^ 0xC8, key.wrapping_mul(16).wrapping_add(si as i32)));
    if h >= bh || sl - si > 0.62 { return None; }
    let tb = h / bh;
    // Dark down among the stems, light and (for the second colour) sun-bleached at the tips.
    Some(scale3(mix3(k.col, k.col2, tb * tb), tone * (0.45 + 0.55 * tb)))
}

fn over(col: &mut [f32; 3], a: &mut f32, c: [f32; 3], m: f32) {
    let na = m + *a * (1.0 - m);
    if na > 0.0 { *col = scale3(add3(scale3(c, m), scale3(*col, *a * (1.0 - m))), 1.0 / na); }
    *a = na;
}

/// The cover over the ground pixel `g` (`tex` 0 the path, 1 the verge): its colour, and how much
/// of the ground it hides. `jitter` (0..1, the pixel's own) shifts where along its line of sight
/// the pixel looks, so the steps do not show as bands.
pub(super) fn at(ctx: &Ctx, layers: &[CoverK], g: &GPixel, tex: u8, jitter: f32) -> ([f32; 3], f32) {
    let v = &ctx.view;
    let w0 = ctx.scroll + g.d;
    let here = if tex == 1 { 1 } else { 2 };
    let water = ctx.water.is_some();
    let (mut col, mut a) = ([0.0f32; 3], 0.0f32);
    let mut hmax = 0.0f32;
    let mut grown = [1.0f32; MAX];
    // The sides (verge 1, path 2) any standing layer grows on.
    let mut sides = 0;
    for k in layers {
        if k.kind != 4 { hmax = hmax.max(k.height); sides |= k.on; continue; }
        if k.on & here == 0 || (water && here == 2) { continue; }
        // Moss: soft cushions, spreading with the density, greener and yellower in turn.
        let n = 0.65 * patch_noise(g.x, w0, ctx.loop_len, 0.9, k.seed) + 0.35 * patch_noise(g.x, w0, ctx.loop_len, 0.23, k.seed ^ 0x5A);
        let th = 0.95 - 0.9 * k.density;
        let m = 0.92 * smoothstep(th, th + 0.1 + 0.2 * (1.0 - k.patch), n);
        if m > 0.0 {
            let fine = 0.6 * patch_noise(g.x, w0, ctx.loop_len, 0.14, k.seed ^ 0x77) + 0.4 * patch_noise(g.x, w0, ctx.loop_len, 0.045, k.seed ^ 0x78);
            over(&mut col, &mut a, scale3(mix3(k.col, k.col2, fine * fine), 0.8 + 0.3 * fine), m);
        }
    }
    if hmax <= 0.0 { return (col, a); }
    let e = v.eye_height.max(0.2);
    let xc = g.x + v.bend_x(g.d);
    let en = ctx.scene.path.edge_noise.abs();
    let wx = ctx.wx.sway_parts();
    // (The path's width where the pixel is: near enough along the short way back to the eye.)
    let hw = v.path_half_width(g.d);
    if water { sides &= 1; }
    // Where the line of sight comes in over the tallest blades, and where it lands: with both clear of the
    // path's edge on the same side, every step between is on that side too.
    let ft = (1.0 - hmax / e).max(0.02);
    let xt = (xc * ft - v.bend_x(g.d * ft)).abs();
    let known = if xt.min(g.x.abs()) > hw + en + 0.05 { 1 } else if xt.max(g.x.abs()) < hw - en - 0.05 { 2 } else { 0 };
    // (Nothing stands on that side: nothing to march through. Most of the path's pixels end here.)
    let steps = if known != 0 && sides & known == 0 { 0 } else { STEPS };
    if steps == 0 && known == here { return (col, a); }
    // How thickly each layer grows here, in its patches.
    for (i, k) in layers.iter().enumerate() {
        if k.kind != 4 { grown[i] = thick(k, g.x, w0, ctx.loop_len); }
    }
    for j in 0..steps {
        // From the tops of the tallest blades (nearest the eye) down to the ground.
        let h = hmax * (1.0 - (j as f32 + jitter) / STEPS as f32);
        let f = 1.0 - h / e;
        if f <= 0.02 { continue; }
        let dj = g.d * f;
        if ctx.on_bridge(dj) { continue; }
        let xj = xc * f - v.bend_x(dj);
        let side = if known != 0 { known } else if xj.abs() > hw + en { 1 } else if xj.abs() < hw - en { 2 } else if xj.abs() > ctx.path_edge(dj) { 1 } else { 2 };
        if water && side == 2 { continue; }
        for (i, k) in layers.iter().enumerate() {
            if k.kind == 4 || k.on & side == 0 || h >= k.height { continue; }
            if let Some(c) = tuft(k, wx, xj, ctx.scroll + dj, h, grown[i]) { return (c, 1.0); }
        }
    }
    // Far off the steps are longer than the tufts are wide: the ground there is simply covered.
    let step = g.d * hmax / e / STEPS as f32;
    for (i, k) in layers.iter().enumerate() {
        if k.kind == 4 || k.on & here == 0 || (water && here == 2) { continue; }
        let fill = smoothstep(0.5, 2.5, step / k.cs) * (1.3 * k.density).min(1.0) * grown[i];
        if fill > 0.0 {
            let tone = 0.8 + 0.4 * patch_noise(g.x, w0, ctx.loop_len, 2.0 * k.cs, k.seed ^ 0xC9);
            over(&mut col, &mut a, scale3(mix3(k.col, k.col2, 0.45), 0.8 * tone), fill);
        }
    }
    (col, a)
}
