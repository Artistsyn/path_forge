//! Sprites: procedural props and fixtures painted once into images, or image files, all drawn
//! the same way as camera-facing billboards.

use super::texture::{rgb_lin, srgb_to_lin};
use crate::scene::{FixtureKind, PropKind, Rgb, SetPieceKind, Shape, ShapePart};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A linear-colour RGBA image with mip levels. The anchor is the bottom centre.
pub struct Sprite {
    pub levels: Vec<(usize, usize, Vec<[f32; 4]>)>,
    /// Width / height.
    pub aspect: f32,
    /// How much each texel lights itself, in the red channel of a sprite of the same size (drawn
    /// props whose parts glow).
    pub glow: Option<Arc<Sprite>>,
}

impl Sprite {
    fn from_rgba_lin(w: usize, h: usize, px: Vec<[f32; 4]>) -> Sprite {
        let mut levels = vec![(w, h, px)];
        loop {
            let (lw, lh, prev) = levels.last().unwrap();
            if *lw <= 2 || *lh <= 2 { break; }
            let (nw, nh) = (lw / 2, lh / 2);
            let mut next = vec![[0.0f32; 4]; nw * nh];
            for y in 0..nh {
                for x in 0..nw {
                    let mut acc = [0.0f32; 4];
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let p = prev[(y * 2 + dy) * lw + x * 2 + dx];
                        // Premultiply while averaging so transparent texels add no colour.
                        acc[0] += p[0] * p[3]; acc[1] += p[1] * p[3]; acc[2] += p[2] * p[3]; acc[3] += p[3];
                    }
                    let a = acc[3] * 0.25;
                    next[y * nw + x] = if acc[3] > 1e-6 { [acc[0] / acc[3], acc[1] / acc[3], acc[2] / acc[3], a] } else { [0.0; 4] };
                }
            }
            levels.push((nw, nh, next));
        }
        Sprite { aspect: w as f32 / h as f32, levels, glow: None }
    }

    pub fn load(path: &Path) -> Option<Sprite> { Sprite::load_opt(path, false) }

    /// Load an image; with `trim`, cut away fully transparent rows and columns around it, keeping
    /// it centred on the middle of what is visible (the anchor is the bottom centre).
    pub fn load_opt(path: &Path, trim: bool) -> Option<Sprite> {
        let img = image::ImageReader::open(path).ok()?.decode().ok()?.to_rgba8();
        let (w, h) = (img.width() as usize, img.height() as usize);
        if w == 0 || h == 0 { return None; }
        let (mut x0, mut y0, mut x1, mut y1) = (0, 0, w, h);
        if trim {
            let (mut a, mut b, mut c, mut d) = (w, h, 0, 0);
            for (x, y, p) in img.enumerate_pixels() {
                if p[3] > 0 { a = a.min(x as usize); b = b.min(y as usize); c = c.max(x as usize + 1); d = d.max(y as usize + 1); }
            }
            if c <= a || d <= b { return None; }
            (x0, y0, x1, y1) = (a, b, c, d);
        }
        let mut px = Vec::with_capacity((x1 - x0) * (y1 - y0));
        for y in y0..y1 {
            for x in x0..x1 {
                let p = img.get_pixel(x as u32, y as u32);
                px.push([srgb_to_lin(p[0]), srgb_to_lin(p[1]), srgb_to_lin(p[2]), p[3] as f32 / 255.0]);
            }
        }
        Some(Sprite::from_rgba_lin(x1 - x0, y1 - y0, px))
    }

    /// Sample at (u, v) in 0..1 (v = 0 at the top) for a sprite drawn `screen_h` pixels tall.
    #[inline]
    pub fn sample(&self, u: f32, v: f32, screen_h: f32, nearest: bool) -> [f32; 4] {
        let ratio = self.levels[0].1 as f32 / screen_h.max(1e-3);
        let lod = (ratio.log2().max(0.0) as usize).min(self.levels.len() - 1);
        let (w, h, data) = &self.levels[lod];
        let (w, h) = (*w, *h);
        if nearest {
            let x = ((u * w as f32) as usize).min(w - 1);
            let y = ((v * h as f32) as usize).min(h - 1);
            return data[y * w + x];
        }
        let fx = (u * w as f32 - 0.5).clamp(0.0, w as f32 - 1.0);
        let fy = (v * h as f32 - 0.5).clamp(0.0, h as f32 - 1.0);
        let (x0, y0) = (fx as usize, fy as usize);
        let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
        let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
        let p = [data[y0 * w + x0], data[y0 * w + x1], data[y1 * w + x0], data[y1 * w + x1]];
        let wts = [(1.0 - tx) * (1.0 - ty), tx * (1.0 - ty), (1.0 - tx) * ty, tx * ty];
        let mut acc = [0.0f32; 4];
        for k in 0..4 {
            let a = p[k][3] * wts[k];
            acc[0] += p[k][0] * a; acc[1] += p[k][1] * a; acc[2] += p[k][2] * a; acc[3] += a;
        }
        if acc[3] > 1e-6 { [acc[0] / acc[3], acc[1] / acc[3], acc[2] / acc[3], acc[3]] } else { [0.0; 4] }
    }
}

// ── A tiny painter for procedural sprites ──────────────────────────────────

/// Paints in nominal units at `SS`× resolution; `finish` filters down to 2× nominal, so edges are
/// anti-aliased and close-ups keep detail.
struct Canvas {
    w: usize,
    h: usize,
    k: f32,
    px: Vec<[f32; 4]>,
    /// While set, shapes cut transparent holes instead of painting.
    erase: bool,
}

const SS: usize = 4;

fn mul(c: [f32; 3], k: f32) -> [f32; 3] { [c[0] * k, c[1] * k, c[2] * k] }
fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] { [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t] }

impl Canvas {
    fn new(w: usize, h: usize) -> Canvas { Canvas { w: w * SS, h: h * SS, k: SS as f32, px: vec![[0.0; 4]; w * h * SS * SS], erase: false } }

    fn put(&mut self, x: i32, y: i32, c: [f32; 3]) {
        if x < 0 || y < 0 || x as usize >= self.w || y as usize >= self.h { return; }
        self.px[y as usize * self.w + x as usize] = if self.erase { [0.0; 4] } else { [c[0], c[1], c[2], 1.0] };
    }

    /// Run `f` with shapes cutting holes.
    fn cut(&mut self, f: impl FnOnce(&mut Canvas)) {
        self.erase = true;
        f(self);
        self.erase = false;
    }

    /// Filled ellipse lit from the upper left (`shade` 0 = flat, 1 = strong dome).
    fn ellipse(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, c: [f32; 3], shade: f32) {
        let (cx, cy, rx, ry) = (cx * self.k, cy * self.k, rx * self.k, ry * self.k);
        if rx <= 0.0 || ry <= 0.0 { return; }
        for y in (cy - ry).floor() as i32..=(cy + ry).ceil() as i32 {
            for x in (cx - rx).floor() as i32..=(cx + rx).ceil() as i32 {
                let nx = (x as f32 + 0.5 - cx) / rx;
                let ny = (y as f32 + 0.5 - cy) / ry;
                let r2 = nx * nx + ny * ny;
                if r2 > 1.0 { continue; }
                let nz = (1.0 - r2).sqrt();
                let lit = (-0.45 * nx - 0.6 * ny + 0.66 * nz).max(0.0);
                self.put(x, y, mul(c, 1.0 - shade * 0.55 + shade * 0.75 * lit));
            }
        }
    }

    fn rect(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, c: [f32; 3], shade: f32) {
        let (x0, y0, x1, y1) = (x0 * self.k, y0 * self.k, x1 * self.k, y1 * self.k);
        let (xa, xb) = (x0.min(x1), x0.max(x1));
        for y in y0.min(y1).floor() as i32..y0.max(y1).ceil() as i32 {
            for x in xa.floor() as i32..xb.ceil() as i32 {
                let t = ((x as f32 + 0.5 - xa) / (xb - xa).max(1e-3)).clamp(0.0, 1.0);
                self.put(x, y, mul(c, 1.0 + shade * (0.35 - 0.7 * t)));
            }
        }
    }

    /// Filled polygon (even-odd), horizontally shaded.
    fn poly(&mut self, pts: &[[f32; 2]], c: [f32; 3], shade: f32) {
        let pts: Vec<[f32; 2]> = pts.iter().map(|p| [p[0] * self.k, p[1] * self.k]).collect();
        let pts = &pts[..];
        let (mut x0, mut x1, mut y0, mut y1) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
        for p in pts { x0 = x0.min(p[0]); x1 = x1.max(p[0]); y0 = y0.min(p[1]); y1 = y1.max(p[1]); }
        for y in y0.floor() as i32..=y1.ceil() as i32 {
            let py = y as f32 + 0.5;
            let mut xs: Vec<f32> = Vec::new();
            for i in 0..pts.len() {
                let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
                if (a[1] <= py) != (b[1] <= py) {
                    xs.push(a[0] + (py - a[1]) / (b[1] - a[1]) * (b[0] - a[0]));
                }
            }
            xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
            for pair in xs.chunks_exact(2) {
                for x in (pair[0] - 0.5).ceil() as i32..=(pair[1] - 0.5).floor() as i32 {
                    let t = ((x as f32 - x0) / (x1 - x0).max(1e-3)).clamp(0.0, 1.0);
                    self.put(x, y, mul(c, 1.0 + shade * (0.3 - 0.6 * t)));
                }
            }
        }
    }

    fn line(&mut self, a: [f32; 2], b: [f32; 2], thick: f32, c: [f32; 3]) {
        let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt().max(1e-3);
        let (nx, ny) = (-(b[1] - a[1]) / len * thick * 0.5, (b[0] - a[0]) / len * thick * 0.5);
        self.poly(&[[a[0] + nx, a[1] + ny], [b[0] + nx, b[1] + ny], [b[0] - nx, b[1] - ny], [a[0] - nx, a[1] - ny]], c, 0.4);
    }

    /// Alpha-weighted box filter from SS× down to 2× nominal size.
    fn finish(self) -> Sprite {
        let f = SS / 2;
        let (w, h) = (self.w / f, self.h / f);
        let mut out = vec![[0.0f32; 4]; w * h];
        for y in 0..h {
            for x in 0..w {
                let mut acc = [0.0f32; 4];
                for dy in 0..f { for dx in 0..f {
                    let p = self.px[(y * f + dy) * self.w + x * f + dx];
                    acc[0] += p[0] * p[3]; acc[1] += p[1] * p[3]; acc[2] += p[2] * p[3]; acc[3] += p[3];
                }}
                out[y * w + x] = if acc[3] > 1e-6 { [acc[0] / acc[3], acc[1] / acc[3], acc[2] / acc[3], acc[3] / (f * f) as f32] } else { [0.0; 4] };
            }
        }
        Sprite::from_rgba_lin(w, h, out)
    }
}

struct Rng(u32);
impl Rng {
    fn f(&mut self) -> f32 {
        self.0 ^= self.0 << 13; self.0 ^= self.0 >> 17; self.0 ^= self.0 << 5;
        (self.0 >> 8) as f32 / 16_777_216.0
    }
    fn range(&mut self, a: f32, b: f32) -> f32 { a + (b - a) * self.f() }
}

/// Paint one variant of a procedural prop in `tint`.
pub fn paint_prop(kind: PropKind, tint: Rgb, variant: u32) -> Sprite {
    let t = rgb_lin(tint);
    let mut r = Rng(variant.wrapping_mul(2_654_435_761).wrapping_add(0x9e37) | 1);
    let bark = mix([0.05, 0.035, 0.02], t, 0.15);
    match kind {
        PropKind::Tree => {
            let mut c = Canvas::new(208, 256);
            let trunk_w = r.range(14.0, 22.0);
            c.rect(104.0 - trunk_w / 2.0, 120.0, 104.0 + trunk_w / 2.0, 256.0, bark, 0.8);
            c.line([104.0, 170.0], [r.range(60.0, 80.0), 120.0], 8.0, bark);
            c.line([104.0, 160.0], [r.range(128.0, 150.0), 112.0], 7.0, bark);
            let blobs = 6 + (variant % 3) as usize;
            for i in 0..blobs {
                let a = i as f32 / blobs as f32 * std::f32::consts::TAU;
                let (bx, by) = (104.0 + a.cos() * r.range(30.0, 58.0), 92.0 + a.sin() * r.range(22.0, 44.0));
                c.ellipse(bx, by + 12.0, r.range(36.0, 50.0), r.range(30.0, 40.0), mul(t, 0.55), 0.6);
            }
            for _ in 0..5 {
                c.ellipse(104.0 + r.range(-46.0, 40.0), r.range(52.0, 104.0), r.range(28.0, 44.0), r.range(24.0, 34.0), t, 0.9);
            }
            c.ellipse(96.0 + r.range(-10.0, 10.0), 52.0, 34.0, 26.0, mul(t, 1.25), 0.9);
            c.finish()
        }
        PropKind::Pine => {
            let mut c = Canvas::new(160, 256);
            c.rect(73.0, 200.0, 87.0, 256.0, bark, 0.8);
            let tiers = 4 + (variant % 2) as usize;
            for i in 0..tiers {
                let f = i as f32 / tiers as f32;
                let base_y = 214.0 - f * 170.0;
                let half = 74.0 * (1.0 - f * 0.7) * r.range(0.9, 1.05);
                let hgt = 72.0 * (1.0 - f * 0.3);
                c.poly(&[[80.0, base_y - hgt], [80.0 + half, base_y], [80.0 - half, base_y]], mul(t, 0.7 + f * 0.45), 0.9);
            }
            c.finish()
        }
        PropKind::Bush => {
            let mut c = Canvas::new(192, 128);
            for _ in 0..7 {
                c.ellipse(96.0 + r.range(-54.0, 54.0), r.range(60.0, 92.0), r.range(28.0, 42.0), r.range(26.0, 36.0), mul(t, 0.6), 0.5);
            }
            for _ in 0..6 {
                c.ellipse(96.0 + r.range(-46.0, 46.0), r.range(36.0, 76.0), r.range(22.0, 36.0), r.range(20.0, 30.0), t, 0.9);
            }
            c.finish()
        }
        PropKind::Rock | PropKind::Boulder => {
            let mut c = Canvas::new(192, 128);
            let n = 9;
            let pts: Vec<[f32; 2]> = (0..n).map(|i| {
                let a = std::f32::consts::PI + i as f32 / (n - 1) as f32 * std::f32::consts::PI;
                let rad = r.range(0.75, 1.0);
                [96.0 + a.cos() * 92.0 * rad, 127.0 + a.sin() * 118.0 * rad.min(0.95)]
            }).collect();
            c.poly(&pts, mul(t, 0.75), 1.0);
            c.ellipse(96.0 - r.range(10.0, 30.0), r.range(48.0, 64.0), r.range(26.0, 40.0), r.range(16.0, 24.0), mul(t, 1.15), 0.5);
            for _ in 0..3 {
                let x = r.range(50.0, 140.0);
                let y = r.range(50.0, 110.0);
                c.line([x, y], [x + r.range(-24.0, 24.0), y + r.range(8.0, 20.0)], 2.5, mul(t, 0.4));
            }
            c.finish()
        }
        PropKind::Cactus => {
            let mut c = Canvas::new(144, 256);
            c.rect(58.0, 24.0, 86.0, 256.0, t, 1.0);
            c.ellipse(72.0, 26.0, 14.0, 14.0, t, 0.8);
            let ay = r.range(100.0, 140.0);
            c.rect(22.0, ay, 58.0, ay + 18.0, t, 0.6);
            c.rect(22.0, ay - 60.0, 40.0, ay + 18.0, t, 1.0);
            c.ellipse(31.0, ay - 60.0, 9.0, 9.0, t, 0.8);
            let by = r.range(120.0, 170.0);
            c.rect(86.0, by, 120.0, by + 16.0, t, 0.6);
            c.rect(104.0, by - 44.0, 120.0, by + 16.0, t, 1.0);
            c.ellipse(112.0, by - 44.0, 8.0, 8.0, t, 0.8);
            c.finish()
        }
        PropKind::DeadTree => {
            let mut c = Canvas::new(208, 256);
            c.poly(&[[92.0, 256.0], [116.0, 256.0], [110.0, 60.0], [100.0, 60.0]], t, 1.0);
            let branch = |c: &mut Canvas, x: f32, y: f32, dx: f32, dy: f32, w: f32| {
                c.line([x, y], [x + dx, y + dy], w, t);
                c.line([x + dx, y + dy], [x + dx * 1.5 + dy * 0.2, y + dy * 1.6], w * 0.6, t);
            };
            branch(&mut c, 104.0, 150.0, -r.range(40.0, 60.0), -r.range(40.0, 60.0), 9.0);
            branch(&mut c, 106.0, 120.0, r.range(40.0, 64.0), -r.range(36.0, 56.0), 8.0);
            branch(&mut c, 104.0, 90.0, -r.range(26.0, 44.0), -r.range(30.0, 46.0), 6.0);
            branch(&mut c, 105.0, 70.0, r.range(20.0, 36.0), -r.range(30.0, 44.0), 5.0);
            c.finish()
        }
        PropKind::Mushroom => {
            let mut c = Canvas::new(160, 160);
            c.rect(66.0, 70.0, 94.0, 160.0, [0.75, 0.70, 0.6], 0.8);
            c.ellipse(80.0, 66.0, 76.0, 44.0, mul(t, 0.7), 0.3);
            c.ellipse(80.0, 58.0, 72.0, 40.0, t, 0.9);
            for _ in 0..5 {
                c.ellipse(80.0 + r.range(-50.0, 50.0), r.range(36.0, 66.0), r.range(5.0, 10.0), r.range(4.0, 7.0), [0.9, 0.9, 0.88], 0.2);
            }
            c.finish()
        }
        PropKind::Pillar => {
            let mut c = Canvas::new(96, 256);
            c.rect(20.0, 24.0, 76.0, 236.0, t, 1.0);
            c.rect(8.0, 4.0, 88.0, 24.0, mul(t, 1.1), 0.6);
            c.rect(10.0, 236.0, 86.0, 256.0, mul(t, 0.9), 0.6);
            for i in 0..4 { let x = 28.0 + i as f32 * 13.0; c.rect(x, 26.0, x + 3.0, 234.0, mul(t, 0.7), 0.0); }
            if variant % 2 == 1 {
                // Broken top.
                c.poly(&[[0.0, 0.0], [96.0, 0.0], [96.0, r.range(40.0, 70.0)], [50.0, r.range(20.0, 40.0)], [0.0, r.range(60.0, 90.0)]], [0.0; 3], 0.0);
                for p in c.px.iter_mut() { if p[0] == 0.0 && p[1] == 0.0 && p[2] == 0.0 { p[3] = 0.0; } }
            }
            c.finish()
        }
        PropKind::Gravestone => {
            let mut c = Canvas::new(128, 160);
            c.rect(20.0, 50.0, 108.0, 160.0, t, 1.0);
            c.ellipse(64.0, 52.0, 44.0, 42.0, t, 0.8);
            c.rect(58.0, 50.0, 70.0, 116.0, mul(t, 0.6), 0.0);
            c.rect(42.0, 66.0, 86.0, 76.0, mul(t, 0.6), 0.0);
            c.finish()
        }
        PropKind::Crystal => {
            let mut c = Canvas::new(128, 192);
            for i in 0..4 {
                let x = 64.0 + (i as f32 - 1.5) * r.range(14.0, 22.0);
                let hgt = r.range(90.0, 180.0) * if i == 1 || i == 2 { 1.0 } else { 0.7 };
                let lean = r.range(-16.0, 16.0);
                c.poly(&[[x - 14.0, 192.0], [x + lean - 4.0, 192.0 - hgt], [x + lean + 6.0, 192.0 - hgt + 10.0], [x + 14.0, 192.0]], mul(t, r.range(0.8, 1.3)), 1.0);
            }
            c.finish()
        }
        PropKind::Reeds => {
            // A clump of blades bending away from the centre, a few cattails standing above them.
            let mut c = Canvas::new(160, 192);
            let blades = 18 + (variant % 7) as usize;
            for i in 0..blades {
                let x0 = 80.0 + r.range(-46.0, 46.0);
                let top = r.range(30.0, 150.0);
                let lean = (x0 - 80.0) * r.range(0.3, 0.8) + r.range(-12.0, 12.0);
                let col = mul(t, r.range(0.55, 1.15));
                let mid = [x0 + lean * 0.35, (192.0 + top) * 0.5];
                c.line([x0, 192.0], mid, r.range(3.0, 5.0), col);
                c.line(mid, [x0 + lean, top], r.range(1.5, 3.0), col);
                if i % 6 == 0 {
                    let head = [x0 + lean * 0.9, top + 6.0];
                    c.ellipse(head[0], head[1] + 10.0, 5.0, 14.0, [0.11, 0.06, 0.03], 0.8);
                }
            }
            c.finish()
        }
        PropKind::Willow => {
            // A weeping tree: a gnarled trunk under a dome of fine streamers, gathered in clumps of
            // different lengths, dark ones behind and lighter ones in front, tapering to their tips.
            let mut c = Canvas::new(240, 256);
            c.poly(&[[106.0, 256.0], [134.0, 256.0], [127.0, 112.0], [115.0, 112.0]], bark, 1.0);
            c.line([120.0, 150.0], [r.range(70.0, 90.0), 98.0], 8.0, bark);
            c.line([122.0, 136.0], [r.range(150.0, 172.0), 94.0], 7.0, bark);
            c.ellipse(120.0, 88.0, 100.0, 56.0, mul(t, 0.42), 0.5);
            let crown_y = |x: f32| 88.0 - 56.0 * (1.0 - ((x - 120.0) / 100.0).powi(2)).max(0.0).sqrt();
            for layer in 0..2 {
                let (count, shade) = if layer == 0 { (70, 0.55) } else { (110, 1.0) };
                for _ in 0..count / 6 {
                    let cx = 120.0 + r.range(-100.0, 100.0);
                    let clump_len = r.range(80.0, 170.0) * (1.0 - ((cx - 120.0) / 120.0).abs() * 0.35);
                    let sway = r.range(-10.0, 10.0);
                    for _ in 0..6 {
                        let x = cx + r.range(-9.0, 9.0);
                        let y0 = crown_y(x) + r.range(0.0, 18.0);
                        let len = clump_len * r.range(0.75, 1.0);
                        let col = mul(t, shade * r.range(0.75, 1.15));
                        let mut p = [x, y0];
                        for k in 0..4 {
                            let f = (k + 1) as f32 / 4.0;
                            let q = [x + sway * f * f, (y0 + len * f).min(254.0)];
                            c.line(p, q, 2.4 * (1.0 - f * 0.6), mix(col, mul(t, 1.35), f * 0.35));
                            p = q;
                        }
                    }
                }
            }
            c.finish()
        }
        PropKind::Palm => {
            // A leaning ringed trunk and a crown of drooping fronds.
            let mut c = Canvas::new(224, 256);
            let lean = r.range(-30.0, 30.0);
            let (bx, tx) = (112.0, 112.0 + lean);
            let segs = 12;
            for i in 0..segs {
                let (f0, f1) = (i as f32 / segs as f32, (i + 1) as f32 / segs as f32);
                let p0 = [bx + (tx - bx) * f0 * f0, 256.0 - f0 * 190.0];
                let p1 = [bx + (tx - bx) * f1 * f1, 256.0 - f1 * 190.0];
                let wdt = 18.0 - f0 * 8.0;
                c.line(p0, p1, wdt, mix([0.22, 0.15, 0.09], bark, 0.4));
                c.line([p1[0] - wdt * 0.5, p1[1]], [p1[0] + wdt * 0.5, p1[1] + 2.0], 2.0, [0.09, 0.06, 0.035]);
            }
            let crown = [tx, 66.0];
            let fronds = 7 + (variant % 3) as usize;
            for i in 0..fronds {
                let a = std::f32::consts::PI * (0.08 + 0.84 * i as f32 / (fronds - 1) as f32) + r.range(-0.12, 0.12);
                let len = r.range(80.0, 104.0);
                let col = mul(t, r.range(0.75, 1.2));
                let mut p = crown;
                for k in 0..8 {
                    let f = k as f32 / 8.0;
                    let dir = [-a.cos(), -a.sin() * (1.0 - f * 1.6)];
                    let q = [p[0] + dir[0] * len / 8.0, p[1] + dir[1] * len / 8.0 + f * 6.0];
                    c.line(p, q, 7.0 * (1.0 - f * 0.7), col);
                    // Leaflets hang off each frond.
                    c.line(q, [q[0] + r.range(-4.0, 4.0), q[1] + 14.0 * (1.0 - f * 0.5)], 2.5, mul(col, 0.85));
                    p = q;
                }
            }
            c.ellipse(crown[0], crown[1] + 4.0, 12.0, 9.0, [0.16, 0.11, 0.05], 0.6);
            c.finish()
        }
        PropKind::Obelisk => {
            // A tapering stone shaft with a pyramid cap and carved bands; some have lost their tops.
            let mut c = Canvas::new(112, 256);
            c.poly(&[[24.0, 256.0], [88.0, 256.0], [76.0, 34.0], [36.0, 34.0]], t, 1.0);
            c.poly(&[[36.0, 34.0], [76.0, 34.0], [56.0, 4.0]], mul(t, 1.15), 1.0);
            c.poly(&[[56.0, 34.0], [76.0, 34.0], [88.0, 256.0], [62.0, 256.0]], mul(t, 0.82), 0.0);
            for i in 0..6 {
                let y = 60.0 + i as f32 * 30.0 + r.range(-3.0, 3.0);
                let half = 20.0 + (y - 34.0) / 222.0 * 12.0;
                c.rect(56.0 - half * 0.5, y, 56.0 + half * 0.5, y + 3.0, mul(t, 0.6), 0.0);
                c.rect(56.0 - 3.0, y + 8.0, 56.0 + 3.0, y + 16.0, mul(t, 0.6), 0.0);
            }
            if variant % 3 == 2 {
                c.erase = true;
                c.poly(&[[0.0, 0.0], [112.0, 0.0], [112.0, r.range(70.0, 100.0)], [50.0, r.range(56.0, 80.0)], [0.0, r.range(90.0, 120.0)]], [0.0; 3], 0.0);
                c.erase = false;
            }
            c.finish()
        }
        PropKind::Icicle => {
            // Hangs: the top of the sprite is the ceiling. A row of cones of different lengths.
            let mut c = Canvas::new(128, 192);
            let n = 3 + (variant % 4) as usize;
            for i in 0..n {
                let x = 18.0 + i as f32 * (92.0 / n.max(1) as f32) + r.range(-6.0, 6.0);
                let len = r.range(60.0, 188.0) * if i == n / 2 { 1.0 } else { 0.7 };
                let wdt = r.range(9.0, 16.0);
                c.poly(&[[x - wdt, 0.0], [x + wdt, 0.0], [x + r.range(-2.0, 2.0), len]], mul(t, r.range(0.85, 1.1)), 1.0);
                c.line([x - wdt * 0.3, 2.0], [x, len * 0.8], 2.0, [0.95, 0.98, 1.0]);
            }
            c.rect(0.0, 0.0, 128.0, 6.0, mul(t, 0.9), 0.5);
            c.finish()
        }
        PropKind::IceSpike => {
            // Clear ice shards with frosted edges and a bright face catching the light.
            let mut c = Canvas::new(128, 192);
            for i in 0..5 {
                let x = 64.0 + (i as f32 - 2.0) * r.range(12.0, 20.0);
                let hgt = r.range(80.0, 186.0) * if i == 2 { 1.0 } else { 0.65 };
                let lean = r.range(-20.0, 20.0);
                let tip = [x + lean, 192.0 - hgt];
                c.poly(&[[x - 13.0, 192.0], tip, [x + 13.0, 192.0]], mul(t, r.range(0.75, 1.0)), 1.0);
                c.poly(&[[x - 2.0, 192.0], tip, [x + 13.0, 192.0]], mul(t, 1.2), 0.0);
                c.line([x - 13.0, 192.0], tip, 2.0, [0.92, 0.97, 1.0]);
            }
            c.finish()
        }
        PropKind::Stalagmite => {
            let mut c = Canvas::new(112, 224);
            c.poly(&[[6.0, 224.0], [50.0 + r.range(-8.0, 8.0), 0.0], [64.0, 10.0], [106.0, 224.0]], t, 1.0);
            for i in 1..5 { let y = 224.0 - i as f32 * 42.0; c.line([20.0 + i as f32 * 8.0, y], [92.0 - i as f32 * 8.0, y + 6.0], 3.0, mul(t, 0.7)); }
            c.finish()
        }
    }
}

/// Paint a fixture body (the flame or orb is drawn separately). The wall side is on the left.
pub fn paint_fixture(kind: FixtureKind) -> Option<Sprite> {
    let iron = [0.05, 0.045, 0.04];
    let wood = [0.12, 0.06, 0.025];
    match kind {
        FixtureKind::Torch | FixtureKind::GreenFire | FixtureKind::Magic | FixtureKind::IceWisp => {
            let mut c = Canvas::new(64, 160);
            c.rect(0.0, 70.0, 10.0, 110.0, iron, 0.5);
            c.line([6.0, 90.0], [34.0, 60.0], 6.0, iron);
            c.line([24.0, 158.0], [36.0, 26.0], 10.0, wood);
            c.rect(22.0, 22.0, 48.0, 40.0, iron, 0.6);
            Some(c.finish())
        }
        FixtureKind::Lantern => {
            let mut c = Canvas::new(96, 200);
            c.rect(0.0, 6.0, 8.0, 40.0, iron, 0.5);
            c.line([4.0, 14.0], [64.0, 14.0], 5.0, iron);
            c.rect(62.0, 14.0, 66.0, 70.0, iron, 0.0);
            c.rect(44.0, 70.0, 84.0, 80.0, iron, 0.6);
            c.rect(46.0, 80.0, 82.0, 140.0, [0.9, 0.62, 0.25], 0.3);
            for x in [46.0, 63.0, 80.0] { c.rect(x, 80.0, x + 3.0, 140.0, iron, 0.0); }
            c.rect(42.0, 140.0, 86.0, 152.0, iron, 0.6);
            Some(c.finish())
        }
        FixtureKind::Candle => {
            let mut c = Canvas::new(48, 120);
            c.rect(6.0, 104.0, 42.0, 120.0, iron, 0.5);
            c.rect(16.0, 30.0, 32.0, 106.0, [0.78, 0.72, 0.58], 0.8);
            c.ellipse(26.0, 34.0, 4.0, 6.0, [0.85, 0.8, 0.66], 0.5);
            Some(c.finish())
        }
        FixtureKind::Brazier => {
            let mut c = Canvas::new(128, 160);
            c.line([30.0, 160.0], [60.0, 70.0], 6.0, iron);
            c.line([98.0, 160.0], [68.0, 70.0], 6.0, iron);
            c.line([64.0, 160.0], [64.0, 70.0], 6.0, iron);
            c.poly(&[[6.0, 30.0], [122.0, 30.0], [100.0, 74.0], [28.0, 74.0]], [0.09, 0.07, 0.06], 0.8);
            c.rect(10.0, 26.0, 118.0, 34.0, [0.3, 0.12, 0.04], 0.3);
            Some(c.finish())
        }
        FixtureKind::Crystal => Some(paint_prop(PropKind::Crystal, [140, 210, 255], 3)),
        FixtureKind::Firefly => None,
    }
}

/// Fixture body size: (height in metres at size 1, where the flame sits as a fraction from the top).
pub fn fixture_body(kind: FixtureKind) -> (f32, f32) {
    match kind {
        FixtureKind::Torch | FixtureKind::GreenFire | FixtureKind::Magic | FixtureKind::IceWisp => (0.65, 0.12),
        FixtureKind::Lantern => (0.8, 0.55),
        FixtureKind::Candle => (0.3, 0.2),
        FixtureKind::Brazier => (1.0, 0.16),
        FixtureKind::Crystal => (0.9, 0.3),
        FixtureKind::Firefly => (0.0, 0.0),
    }
}

/// Nominal pixels per metre for set-pieces (the sprite holds twice this).
const SP_PX: f32 = 40.0;

/// Paint a set-piece around an opening `ow` x `oh` metres. Returns the sprite and its size in metres;
/// the opening is centred at the bottom.
pub fn paint_set_piece(kind: SetPieceKind, tint: Rgb, accent: Rgb, ow: f32, oh: f32, variant: u32) -> (Sprite, f32, f32) {
    let t = rgb_lin(tint);
    let acc = rgb_lin(accent);
    let mortar = mul(t, 0.55);
    let mut r = Rng(variant.wrapping_mul(2_654_435_761).wrapping_add(0x51) | 1);
    let m = |v: f32| v * SP_PX;
    let ow = ow.clamp(0.5, 30.0);
    let oh = oh.clamp(1.0, 20.0);
    match kind {
        SetPieceKind::Archway | SetPieceKind::RuinedArch => {
            let (pw, ring, cap) = (0.9f32, 0.7f32, 0.5f32);
            let rad = ow * 0.5;
            let spring = (oh - rad).max(0.3); // height where the arch starts
            let (wm, hm) = (ow + 2.0 * pw, oh + ring + cap);
            let mut c = Canvas::new(m(wm) as usize + 1, m(hm) as usize + 1);
            let ground = m(hm);
            // Masonry block, then courses and joints.
            c.rect(0.0, 0.0, m(wm), m(hm), t, 0.35);
            let course = 0.45;
            let mut y = 0.0;
            let mut row = 0;
            while y < hm {
                let yy = ground - m(y);
                c.rect(0.0, yy - 1.2, m(wm), yy + 1.2, mortar, 0.0);
                let off = if row % 2 == 0 { 0.0 } else { 0.35 };
                let mut x = -off;
                while x < wm {
                    c.rect(m(x) - 1.0, yy - m(course), m(x) + 1.0, yy, mortar, 0.0);
                    // A few stones a little lighter or darker.
                    let k = 0.85 + r.f() * 0.3;
                    c.rect(m(x) + 1.5, yy - m(course) + 1.5, m(x + 0.7) - 1.5, yy - 1.5, mul(t, k), 0.25);
                    x += 0.7;
                }
                y += course;
                row += 1;
            }
            // Arch ring: voussoirs fanning around the opening, and a keystone.
            let (cx, cy) = (m(wm * 0.5), ground - m(spring));
            c.ellipse(cx, cy, m(rad + ring), m(rad + ring), mul(t, 1.08), 0.15);
            let n = ((rad * std::f32::consts::PI) / 0.45).round().max(5.0) as i32;
            for i in 0..=n {
                let a = std::f32::consts::PI * (i as f32 / n as f32);
                let (ca, sa) = (a.cos(), a.sin());
                c.line([cx + ca * m(rad), cy - sa * m(rad)], [cx + ca * m(rad + ring), cy - sa * m(rad + ring)], 2.0, mortar);
            }
            c.poly(&[[cx - m(0.28), cy - m(rad + ring + 0.12)], [cx + m(0.28), cy - m(rad + ring + 0.12)], [cx + m(0.18), cy - m(rad - 0.05)], [cx - m(0.18), cy - m(rad - 0.05)]], mul(t, 1.2), 0.5);
            // Cornice along the top.
            c.rect(0.0, 0.0, m(wm), m(0.18), mul(t, 1.15), 0.3);
            // The opening.
            c.cut(|c| {
                c.rect(m(pw), cy, m(pw + ow), ground + 2.0, t, 0.0);
                c.ellipse(cx, cy, m(rad), m(rad), t, 0.0);
            });
            if kind == SetPieceKind::RuinedArch {
                // The crown has fallen: a jagged bite out of the top, off-centre.
                let bx = cx + m(r.range(-0.3, 0.3) * rad);
                let pts = [[bx - m(rad * 0.9), 0.0], [bx + m(rad * 0.7), 0.0], [bx + m(rad * 0.5), m(r.range(0.6, 1.0))],
                    [bx + m(0.2), m(ring + cap + 0.4)], [bx - m(0.3), m(r.range(0.8, 1.2))], [bx - m(rad * 0.7), m(r.range(0.3, 0.6))]];
                c.cut(|c| c.poly(&pts, t, 0.0));
                // One pier lost its top course.
                let side = if variant % 2 == 0 { 0.0 } else { m(wm - pw) };
                c.cut(|c| c.poly(&[[side, 0.0], [side + m(pw), 0.0], [side + m(pw), m(r.range(0.4, 0.9))], [side, m(r.range(0.9, 1.4))]], t, 0.0));
            }
            (c.finish(), wm, hm)
        }
        SetPieceKind::Gate => {
            let (pw, top) = (1.25f32, 1.7f32);
            let (wm, hm) = (ow + 2.0 * pw, oh + top);
            let mut c = Canvas::new(m(wm) as usize + 1, m(hm) as usize + 1);
            let ground = m(hm);
            let iron = [0.05, 0.05, 0.055];
            // Gatehouse wall with crenellations.
            c.rect(0.0, m(0.5), m(wm), ground, t, 0.4);
            let merlon = 0.6;
            let mut x = 0.0;
            while x < wm { c.rect(m(x), 0.0, m(x + merlon * 0.6), m(0.55), mul(t, 1.05), 0.4); x += merlon; }
            // Piers stand proud of the wall; then courses and staggered joints over everything.
            for px0 in [0.0, wm - pw] { c.rect(m(px0), m(0.5), m(px0 + pw), ground, mul(t, 1.08), 0.8); }
            for i in 0..((hm / 0.5) as i32) {
                let yy = ground - m(i as f32 * 0.5);
                c.rect(0.0, yy - 1.0, m(wm), yy + 1.0, mortar, 0.0);
                let off = if i % 2 == 0 { 0.0 } else { 0.4 };
                let mut x = off;
                while x < wm { c.rect(m(x) - 1.0, yy - m(0.5), m(x) + 1.0, yy, mortar, 0.0); x += 0.8; }
            }
            for px0 in [0.0, wm - pw] {
                c.rect(m(px0), m(0.5), m(px0) + 2.0, ground, mul(t, 0.6), 0.0);
                c.rect(m(px0 + pw) - 2.0, m(0.5), m(px0 + pw), ground, mul(t, 0.6), 0.0);
            }
            // Timber lintel.
            let wood = mul(mix([0.16, 0.08, 0.035], t, 0.2), 1.0);
            c.rect(m(pw - 0.2), ground - m(oh + 0.45), m(pw + ow + 0.2), ground - m(oh), wood, 0.5);
            c.cut(|c| c.rect(m(pw), ground - m(oh), m(pw + ow), ground + 2.0, t, 0.0));
            // Raised portcullis: bars hanging below the lintel with spiked ends.
            let bars = ((ow / 0.42).round() as i32).max(3);
            for i in 0..=bars {
                let bx = m(pw) + m(ow) * i as f32 / bars as f32;
                let y0 = ground - m(oh);
                let y1 = y0 + m(0.75);
                c.rect(bx - 2.5, y0, bx + 2.5, y1, iron, 0.3);
                c.poly(&[[bx - 4.0, y1], [bx + 4.0, y1], [bx, y1 + 10.0]], iron, 0.0);
            }
            c.rect(m(pw), ground - m(oh) + m(0.3), m(pw + ow), ground - m(oh) + m(0.36), iron, 0.0);
            // Heraldic banners on the piers.
            for px0 in [0.0, wm - pw] {
                let (bx0, bx1) = (m(px0 + 0.2), m(px0 + pw - 0.2));
                let (by0, by1) = (m(0.9), m(0.9 + oh * 0.55));
                c.poly(&[[bx0, by0], [bx1, by0], [bx1, by1], [(bx0 + bx1) * 0.5, by1 - m(0.3)], [bx0, by1]], acc, 0.5);
                c.poly(&[[(bx0 + bx1) * 0.5, by0 + m(0.4)], [bx1 - m(0.15), (by0 + by1) * 0.5], [(bx0 + bx1) * 0.5, by1 - m(0.6)], [bx0 + m(0.15), (by0 + by1) * 0.5]], mix(acc, [0.9, 0.75, 0.3], 0.7), 0.2);
            }
            (c.finish(), wm, hm)
        }
        SetPieceKind::Banners => {
            let post = 0.22f32;
            let (wm, hm) = (ow + 2.0 * post, oh + 0.35);
            let mut c = Canvas::new(m(wm) as usize + 1, m(hm) as usize + 1);
            let ground = m(hm);
            let wood = mix([0.16, 0.08, 0.035], t, 0.3);
            for px0 in [0.0, wm - post] { c.rect(m(px0), 0.0, m(px0 + post), ground, wood, 0.7); }
            c.rect(0.0, m(0.05), m(wm), m(0.3), wood, 0.4);
            let n = ((ow / 1.4).round() as i32).max(2);
            let bw = (ow / n as f32) * 0.62;
            let drop = (oh * 0.42).clamp(0.8, 2.2);
            for i in 0..n {
                let cxm = post + ow * (i as f32 + 0.5) / n as f32;
                let (x0, x1) = (m(cxm - bw * 0.5), m(cxm + bw * 0.5));
                let (y0, y1) = (m(0.3), m(0.3 + drop));
                let sway = r.range(-0.06, 0.06);
                c.poly(&[[x0, y0], [x1, y0], [x1 + m(sway), y1], [(x0 + x1) * 0.5 + m(sway), y1 - m(0.28)], [x0 + m(sway), y1]], acc, 0.6);
                c.rect(x0, y0 + m(0.08), x1, y0 + m(0.16), mix(acc, [0.9, 0.75, 0.3], 0.6), 0.0);
                let (ex, ey) = ((x0 + x1) * 0.5 + m(sway * 0.5), (y0 + y1) * 0.5);
                c.poly(&[[ex, ey - m(bw * 0.3)], [ex + m(bw * 0.22), ey], [ex, ey + m(bw * 0.3)], [ex - m(bw * 0.22), ey]], mix(acc, [0.95, 0.85, 0.5], 0.75), 0.2);
            }
            (c.finish(), wm, hm)
        }
        SetPieceKind::Portal => {
            let band = 0.32f32;
            let (wm, hm) = (ow + 2.0 * band, oh + 2.0 * band);
            let mut c = Canvas::new(m(wm) as usize + 1, m(hm) as usize + 1);
            let (cx, cy) = (m(wm * 0.5), m(hm * 0.5));
            c.ellipse(cx, cy, m(wm * 0.5), m(hm * 0.5), acc, 0.3);
            c.ellipse(cx, cy, m(wm * 0.5 - band * 0.35), m(hm * 0.5 - band * 0.35), mix(acc, [1.0, 1.0, 1.0], 0.45), 0.0);
            c.cut(|c| c.ellipse(cx, cy, m(ow * 0.5), m(oh * 0.5), acc, 0.0));
            // Runes around the ring.
            let n = 14;
            for i in 0..n {
                let a = std::f32::consts::TAU * i as f32 / n as f32;
                let (rx, ry) = (m(wm * 0.5 - band * 0.5), m(hm * 0.5 - band * 0.5));
                c.ellipse(cx + a.cos() * rx, cy + a.sin() * ry, m(0.05), m(0.07), [1.0, 0.98, 0.9], 0.0);
            }
            (c.finish(), wm, hm)
        }
    }
}

/// Paint a prop drawn from parts in metres (x from the centre, y up from the ground). Returns the
/// sprite and its height in metres; None when the parts cover nothing.
pub fn paint_shape(parts: &[ShapePart]) -> Option<(Sprite, f32)> {
    let (mut half_w, mut top) = (0.0f32, 0.0f32);
    for p in parts.iter().filter(|p| !p.cut) {
        let (xs, ys): (Vec<f32>, Vec<f32>) = match &p.shape {
            Shape::Ellipse { center, radius } => (vec![center[0] - radius[0].abs(), center[0] + radius[0].abs()], vec![center[1] + radius[1].abs()]),
            Shape::Rect { min, max } => (vec![min[0], max[0]], vec![min[1], max[1]]),
            Shape::Poly { points } => (points.iter().map(|q| q[0]).collect(), points.iter().map(|q| q[1]).collect()),
            Shape::Line { from, to, width } => {
                let r = width.abs() * 0.5;
                (vec![from[0] - r, from[0] + r, to[0] - r, to[0] + r], vec![from[1] + r, to[1] + r])
            }
        };
        for x in xs { if x.is_finite() { half_w = half_w.max(x.abs()); } }
        for y in ys { if y.is_finite() { top = top.max(y); } }
    }
    if half_w <= 1e-3 || top <= 1e-3 { return None; }
    // About 128 painted pixels along the longer side (256 once finished at 2x).
    let ppm = (128.0 / (2.0 * half_w).max(top)).clamp(4.0, 400.0);
    let (w, h) = (((2.0 * half_w * ppm).ceil() as usize).max(2), ((top * ppm).ceil() as usize).max(2));
    let at = |q: [f32; 2]| [q[0] * ppm + w as f32 * 0.5, (top - q[1]) * ppm];
    let paint = |c: &mut Canvas, p: &ShapePart, col: [f32; 3], shade: f32| {
        let draw = |c: &mut Canvas| match &p.shape {
            Shape::Ellipse { center, radius } => { let q = at(*center); c.ellipse(q[0], q[1], radius[0].abs() * ppm, radius[1].abs() * ppm, col, shade) }
            Shape::Rect { min, max } => { let (a, b) = (at(*min), at(*max)); c.rect(a[0], a[1], b[0], b[1], col, shade) }
            Shape::Poly { points } => { let pts: Vec<[f32; 2]> = points.iter().map(|q| at(*q)).collect(); if pts.len() >= 3 { c.poly(&pts, col, shade) } }
            Shape::Line { from, to, width } => c.line(at(*from), at(*to), width.abs() * ppm, col),
        };
        if p.cut { c.cut(draw) } else { draw(c) }
    };
    let mut c = Canvas::new(w, h);
    for p in parts { paint(&mut c, p, rgb_lin(p.color), p.shade.clamp(0.0, 1.0)); }
    let mut sprite = c.finish();
    // Glowing parts: the same parts painted again with their glow as the colour, so a part painted
    // over a glowing one covers its glow too.
    if parts.iter().any(|p| p.glow > 0.0 && !p.cut) {
        let mut g = Canvas::new(w, h);
        for p in parts { let k = p.glow.max(0.0); paint(&mut g, p, [k, k, k], 0.0); }
        sprite.glow = Some(Arc::new(g.finish()));
    }
    Some((sprite, top))
}

/// Sprites by key: procedural variants and loaded image files.
#[derive(Default)]
pub struct SpriteCache {
    procedural: HashMap<(u8, Rgb, u32), Arc<Sprite>>,
    /// Set-pieces by kind, colours, opening size in centimetres and variant; with their size in metres.
    pieces: HashMap<(u8, Rgb, Rgb, u32, u32, u32), (Arc<Sprite>, f32, f32)>,
    fixtures: HashMap<u8, Option<Arc<Sprite>>>,
    /// By path and trim, with the file's modification time when it was read.
    files: HashMap<(PathBuf, bool), (Option<std::time::SystemTime>, Option<Arc<Sprite>>)>,
    /// Images with a glow layer from their lightest texels, with the image they were made from.
    glowing: HashMap<(PathBuf, bool, u32, u32), (Arc<Sprite>, Arc<Sprite>)>,
    /// Drawn props by the text of their parts, with their height in metres.
    shapes: HashMap<String, Option<(Arc<Sprite>, f32)>>,
}

pub const VARIANTS: u32 = 4;

impl SpriteCache {
    pub fn prop(&mut self, kind: PropKind, tint: Rgb, variant: u32) -> Arc<Sprite> {
        let key = (kind as u8, tint, variant % VARIANTS);
        if self.procedural.len() > 256 { self.procedural.clear(); }
        self.procedural.entry(key).or_insert_with(|| Arc::new(paint_prop(kind, tint, variant % VARIANTS))).clone()
    }

    pub fn set_piece(&mut self, kind: SetPieceKind, tint: Rgb, accent: Rgb, ow: f32, oh: f32, variant: u32) -> (Arc<Sprite>, f32, f32) {
        let key = (kind as u8, tint, accent, (ow * 10.0).round() as u32, (oh * 10.0).round() as u32, variant % VARIANTS);
        if self.pieces.len() > 64 { self.pieces.clear(); }
        self.pieces.entry(key).or_insert_with(|| {
            let (s, w, h) = paint_set_piece(kind, tint, accent, key.3 as f32 / 10.0, key.4 as f32 / 10.0, variant % VARIANTS);
            (Arc::new(s), w, h)
        }).clone()
    }

    pub fn fixture(&mut self, kind: FixtureKind) -> Option<Arc<Sprite>> {
        self.fixtures.entry(kind as u8).or_insert_with(|| paint_fixture(kind).map(Arc::new)).clone()
    }

    pub fn file(&mut self, base: Option<&Path>, path: &str) -> Option<Arc<Sprite>> {
        let p = Path::new(path.trim());
        let full = match base { Some(b) if p.is_relative() => b.join(p), _ => p.to_path_buf() };
        self.file_at(&full, false)
    }

    /// An image at a resolved path, optionally trimmed to what is visible.
    /// Read again when the file changes on disk.
    pub fn file_at(&mut self, full: &Path, trim: bool) -> Option<Arc<Sprite>> {
        let mtime = std::fs::metadata(full).and_then(|m| m.modified()).ok();
        let key = (full.to_path_buf(), trim);
        if let Some((t, s)) = self.files.get(&key) {
            if *t == mtime { return s.clone(); }
        }
        if self.files.len() > 512 { self.files.clear(); }
        let s = Sprite::load_opt(full, trim).map(Arc::new);
        self.files.insert(key, (mtime, s.clone()));
        s
    }

    /// An image whose texels at least `from` light (0..1, perceived) glow by `strength`.
    pub fn file_glowing(&mut self, full: &Path, trim: bool, from: f32, strength: f32) -> Option<Arc<Sprite>> {
        let base = self.file_at(full, trim)?;
        let key = (full.to_path_buf(), trim, (from * 1000.0) as u32, (strength * 1000.0) as u32);
        if let Some((b, s)) = self.glowing.get(&key) { if Arc::ptr_eq(b, &base) { return Some(s.clone()); } }
        let (w, h, px) = &base.levels[0];
        let mask: Vec<[f32; 4]> = px.iter().map(|p| {
            let light = (0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]).max(0.0).powf(1.0 / 2.2);
            let t = ((light - from) / 0.08).clamp(0.0, 1.0);
            let k = t * t * (3.0 - 2.0 * t) * strength;
            [k, k, k, p[3]]
        }).collect();
        let mut s = Sprite::from_rgba_lin(*w, *h, base.levels[0].2.clone());
        s.glow = Some(Arc::new(Sprite::from_rgba_lin(*w, *h, mask)));
        let s = Arc::new(s);
        if self.glowing.len() > 64 { self.glowing.clear(); }
        self.glowing.insert(key, (base, s.clone()));
        Some(s)
    }

    pub fn shape(&mut self, parts: &[ShapePart]) -> Option<(Arc<Sprite>, f32)> {
        let key = serde_json::to_string(parts).unwrap_or_default();
        if self.shapes.len() > 128 { self.shapes.clear(); }
        self.shapes.entry(key).or_insert_with(|| paint_shape(parts).map(|(s, h)| (Arc::new(s), h))).clone()
    }
}
