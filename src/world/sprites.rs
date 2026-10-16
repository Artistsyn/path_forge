//! Sprites: procedural props and fixtures painted once into images, or image files, all drawn
//! the same way as camera-facing billboards.

use super::texture::{rgb_lin, srgb_to_lin};
use crate::scene::{FixtureKind, LampPost, PropKind, Rgb, SetPieceKind, Shape, ShapePart};
use std::f32::consts::PI;
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
    /// The image came with a painted mask (`<name>.mask.png`): the glow map's green channel then
    /// says which texels cast a shadow.
    pub casts: bool,
    /// The light its glowing texels give off, worked out from the mask.
    pub lamp: Option<Lamp>,
}

/// The light an image's glowing part gives off: where its middle is (across from the left and down
/// from the top, as fractions of the image), its colour at full brightness, and how much of the
/// image glows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lamp {
    pub u: f32,
    pub v: f32,
    pub color: [f32; 3],
    pub cover: f32,
}

/// Where the mask painted for an image is kept: `torch.png` has `torch.mask.png` beside it. Red is
/// how much a texel glows, green whether it casts a shadow.
pub fn mask_path(image: &Path) -> PathBuf { image.with_extension("mask.png") }
/// What a texel of a mask says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaskPart {
    /// Lights itself and what is round it, and casts no shadow: a flame, a lamp's glass, a rune.
    Glow,
    /// Takes the scene's light and casts a shadow: what an image is without a mask.
    Solid,
    /// Neither: smoke, glass, a thin chain that should not throw a hard shadow.
    Clear,
}

/// The mask painted for an image: for each texel how much it glows and whether it casts a shadow.
#[derive(Clone, Debug, PartialEq)]
pub struct Mask {
    pub w: usize,
    pub h: usize,
    /// (glow, solid), each 0..255.
    pub px: Vec<[u8; 2]>,
}

impl Mask {
    /// Nothing glows and everything casts: the same as having no mask.
    pub fn solid(w: usize, h: usize) -> Mask { Mask { w, h, px: vec![[0, 255]; w * h] } }

    /// The mask saved beside `image`, or a plain one of the image's size.
    pub fn load(image: &Path, w: usize, h: usize) -> Mask {
        match image::ImageReader::open(mask_path(image)).ok().and_then(|r| r.decode().ok()).map(|m| m.to_rgba8()) {
            Some(m) if m.width() as usize == w && m.height() as usize == h => Mask { w, h, px: m.pixels().map(|p| [p[0], p[1]]).collect() },
            _ => Mask::solid(w, h),
        }
    }

    pub fn save(&self, image: &Path) -> Result<PathBuf, String> {
        let file = mask_path(image);
        let bytes: Vec<u8> = self.px.iter().flat_map(|p| [p[0], p[1], 0, 255]).collect();
        image::save_buffer(&file, &bytes, self.w as u32, self.h as u32, image::ExtendedColorType::Rgba8).map_err(|e| format!("{}: {e}", file.display()))?;
        Ok(file)
    }

    /// Texels of an sRGB image (four bytes each) at least `from` light (0..1, as the eye sees it)
    /// glow; the rest are solid.
    pub fn from_light(rgba: &[u8], w: usize, h: usize, from: f32) -> Mask {
        let px = rgba.chunks_exact(4).map(|p| {
            let light = (0.2126 * srgb_to_lin(p[0]) + 0.7152 * srgb_to_lin(p[1]) + 0.0722 * srgb_to_lin(p[2])).max(0.0).powf(1.0 / 2.2);
            let t = ((light - from) / 0.08).clamp(0.0, 1.0);
            let g = (t * t * (3.0 - 2.0 * t) * 255.0) as u8;
            if p[3] == 0 { [0, 255] } else { [g, 255 - g] }
        }).collect();
        Mask { w, h, px }
    }

    /// Paint a round dab of `part`, `r` texels in radius, at (cx, cy).
    pub fn paint(&mut self, cx: f32, cy: f32, r: f32, part: MaskPart) {
        let v = match part { MaskPart::Glow => [255, 0], MaskPart::Solid => [0, 255], MaskPart::Clear => [0, 0] };
        let r = r.max(0.5);
        for y in ((cy - r).floor().max(0.0) as usize)..=((cy + r).ceil() as usize).min(self.h.saturating_sub(1)) {
            for x in ((cx - r).floor().max(0.0) as usize)..=((cx + r).ceil() as usize).min(self.w.saturating_sub(1)) {
                let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
                if dx * dx + dy * dy <= r * r { self.px[y * self.w + x] = v; }
            }
        }
    }

    /// The share of the image's visible texels that glow (`alpha`: one byte a texel).
    pub fn glowing(&self, alpha: &[u8]) -> f32 {
        let (mut g, mut n) = (0.0f32, 0.0f32);
        for (p, a) in self.px.iter().zip(alpha) { if *a > 0 { n += 1.0; g += p[0] as f32 / 255.0; } }
        if n > 0.0 { g / n } else { 0.0 }
    }

    pub fn is_plain(&self) -> bool { self.px.iter().all(|p| *p == [0, 255]) }
}

/// Whether a file is a mask rather than an image to draw.
pub fn is_mask(file: &Path) -> bool { file.file_name().is_some_and(|n| n.to_string_lossy().to_lowercase().ends_with(".mask.png")) }

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
        Sprite { aspect: w as f32 / h as f32, levels, glow: None, casts: false, lamp: None }
    }

    pub fn load(path: &Path) -> Option<Sprite> { Sprite::load_opt(path, false) }

    /// A copy of the image alone.
    pub fn clone_levels(s: &Sprite) -> Sprite { Sprite { levels: s.levels.clone(), aspect: s.aspect, glow: None, casts: false, lamp: None } }

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
        // A mask painted for it: what glows (and so gives light), and what casts a shadow.
        let (mw, mh) = (x1 - x0, y1 - y0);
        let mask = image::ImageReader::open(mask_path(path)).ok().and_then(|r| r.decode().ok()).map(|m| m.to_rgba8())
            .filter(|m| m.width() as usize == w && m.height() as usize == h);
        let mut s = Sprite::from_rgba_lin(mw, mh, px);
        if let Some(mask) = mask {
            let src = &s.levels[0].2;
            let mut g = Vec::with_capacity(mw * mh);
            let (mut su, mut sv, mut col, mut n) = (0.0f32, 0.0f32, [0.0f32; 3], 0.0f32);
            for y in 0..mh {
                for x in 0..mw {
                    let m = mask.get_pixel((x0 + x) as u32, (y0 + y) as u32);
                    let p = src[y * mw + x];
                    let glow = m[0] as f32 / 255.0;
                    g.push([glow * MASK_GLOW, m[1] as f32 / 255.0, 0.0, p[3]]);
                    let wgt = glow * p[3];
                    if wgt > 0.0 {
                        su += wgt * (x as f32 + 0.5) / mw as f32;
                        sv += wgt * (y as f32 + 0.5) / mh as f32;
                        for i in 0..3 { col[i] += wgt * p[i]; }
                        n += wgt;
                    }
                }
            }
            s.glow = Some(Arc::new(Sprite::from_rgba_lin(mw, mh, g)));
            s.casts = true;
            if n >= 1.0 {
                let top = col[0].max(col[1]).max(col[2]).max(1e-4);
                s.lamp = Some(Lamp { u: su / n, v: sv / n, color: [col[0] / top, col[1] / top, col[2] / top], cover: n / (mw * mh) as f32 });
            }
        }
        Some(s)
    }

    /// The mip level `sample` reads for a sprite drawn `screen_h` pixels tall.
    #[inline]
    pub fn lod(&self, screen_h: f32) -> usize {
        let ratio = self.levels[0].1 as f32 / screen_h.max(1e-3);
        (ratio.log2().max(0.0) as usize).min(self.levels.len() - 1)
    }

    /// Sample at (u, v) in 0..1 (v = 0 at the top) for a sprite drawn `screen_h` pixels tall.
    #[inline]
    pub fn sample(&self, u: f32, v: f32, screen_h: f32, nearest: bool) -> [f32; 4] {
        let (w, h, data) = &self.levels[self.lod(screen_h)];
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
/// How strongly a fully masked texel lights itself.
pub const MASK_GLOW: f32 = 1.5;

fn mul(c: [f32; 3], k: f32) -> [f32; 3] { [c[0] * k, c[1] * k, c[2] * k] }
fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] { [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t] }

/// A painting with the glow map painted alongside it (the same shapes, glowing ones in light, the
/// rest in black over them).
fn with_glow(mut s: Sprite, g: Canvas) -> Sprite {
    s.glow = Some(Arc::new(g.finish()));
    s
}

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

    /// A soft glow laid over what is there: an ellipse turned by `ang`, strongest at its middle and
    /// gone at its edge. Where nothing is painted yet it leaves a see-through haze.
    fn haze(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, ang: f32, c: [f32; 3], strength: f32) { self.wash(cx, cy, rx, ry, ang, c, strength, false) }

    /// The same glow, but only over what is already painted: a stain, moss, a sheen.
    fn stain(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, ang: f32, c: [f32; 3], strength: f32) { self.wash(cx, cy, rx, ry, ang, c, strength, true) }

    #[allow(clippy::too_many_arguments)]
    fn wash(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, ang: f32, c: [f32; 3], strength: f32, painted_only: bool) {
        let (cx, cy, rx, ry) = (cx * self.k, cy * self.k, rx * self.k, ry * self.k);
        if rx <= 0.0 || ry <= 0.0 { return; }
        let (sn, cs) = ang.sin_cos();
        let e = rx.max(ry);
        for y in ((cy - e).floor() as i32).max(0)..=((cy + e).ceil() as i32).min(self.h as i32 - 1) {
            for x in ((cx - e).floor() as i32).max(0)..=((cx + e).ceil() as i32).min(self.w as i32 - 1) {
                let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
                let (u, v) = ((dx * cs + dy * sn) / rx, (-dx * sn + dy * cs) / ry);
                let q = 1.0 - (u * u + v * v);
                if q <= 0.0 { continue; }
                let a = (strength * q * q).min(1.0);
                let p = &mut self.px[y as usize * self.w + x as usize];
                if painted_only && p[3] < 0.99 { continue; }
                let out = a + p[3] * (1.0 - a);
                for i in 0..3 { p[i] = (c[i] * a + p[i] * p[3] * (1.0 - a)) / out.max(1e-6); }
                p[3] = out;
            }
        }
    }

    /// A flat ellipse turned by `ang` radians: a leaf, or a dab of foliage.
    fn dab(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, ang: f32, c: [f32; 3]) {
        let (cx, cy, rx, ry) = (cx * self.k, cy * self.k, rx * self.k, ry * self.k);
        if rx <= 0.0 || ry <= 0.0 { return; }
        let (sn, cs) = ang.sin_cos();
        let e = rx.max(ry);
        for y in (cy - e).floor() as i32..=(cy + e).ceil() as i32 {
            for x in (cx - e).floor() as i32..=(cx + e).ceil() as i32 {
                let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
                let (u, v) = ((dx * cs + dy * sn) / rx, (-dx * sn + dy * cs) / ry);
                if u * u + v * v <= 1.0 { self.put(x, y, c); }
            }
        }
    }

    /// A limb from `a` (`wa` wide) to `b` (`wb` wide), rounded where it starts: shaded round on
    /// the side away from the upper-left light, with bark furrows running along it.
    fn limb(&mut self, a: [f32; 2], b: [f32; 2], wa: f32, wb: f32, c: [f32; 3], seed: f32) { self.bar(a, b, wa, wb, c, seed, 0.14) }

    /// A round column from `a` to `b`, rounded at both ends, with `ribs` ridges showing down its
    /// length (a cactus) or as many flutes cut into it (a stone column, `fluted`); `spines` dots the
    /// ridges with pale thorns.
    #[allow(clippy::too_many_arguments)]
    fn ribbed(&mut self, a: [f32; 2], b: [f32; 2], w: f32, c: [f32; 3], ribs: f32, fluted: bool, spines: bool) {
        let k = self.k;
        let (ax, ay, bx, by) = (a[0] * k, a[1] * k, b[0] * k, b[1] * k);
        let (dx, dy) = (bx - ax, by - ay);
        let len = (dx * dx + dy * dy).sqrt().max(1e-3);
        let (ux, uy) = (dx / len, dy / len);
        let (nx, ny) = (-uy, ux);
        let side = if -0.6 * nx - 0.8 * ny >= 0.0 { 1.0 } else { -1.0 };
        let hw = w * k * 0.5;
        let m = hw + 1.0;
        for y in (ay.min(by) - m).floor() as i32..=(ay.max(by) + m).ceil() as i32 {
            for x in (ax.min(bx) - m).floor() as i32..=(ax.max(bx) + m).ceil() as i32 {
                let (px, py) = (x as f32 + 0.5 - ax, y as f32 + 0.5 - ay);
                let along = px * ux + py * uy;
                let across = px * nx + py * ny;
                // Past either end: a dome (a square end for a fluted column, which a capital covers).
                let over = if along < 0.0 { -along } else if along > len { along - len } else { 0.0 };
                if over > 0.0 && (fluted || over * over + across * across > hw * hw) { continue; }
                if across.abs() > hw { continue; }
                let s = (across / hw).clamp(-1.0, 1.0) * side;
                let turn = s.asin();
                let ridge = (ribs * turn).cos();
                let round = (1.0 - s * s).max(0.0).sqrt() * (1.0 - 0.5 * (over / hw).powi(2));
                let relief = if fluted { 0.9 + 0.1 * ridge.abs().powf(0.6) - 0.16 * (1.0 - ridge.abs()).powi(3) } else { 0.8 + 0.2 * ridge };
                let mut col = mul(c, (0.62 + 0.28 * round + 0.3 * s) * relief);
                if spines && ridge > 0.93 && ((along / k) % 9.0) < 1.1 { col = mix(col, [0.8, 0.78, 0.55], 0.45); }
                self.put(x, y, col);
            }
        }
    }

    /// A smooth round bar: turned iron, a candle, a pole.
    fn rod(&mut self, a: [f32; 2], b: [f32; 2], wa: f32, wb: f32, c: [f32; 3]) { self.bar(a, b, wa, wb, c, 0.0, 0.0) }

    fn bar(&mut self, a: [f32; 2], b: [f32; 2], wa: f32, wb: f32, c: [f32; 3], seed: f32, furrows: f32) {
        let k = self.k;
        let (ax, ay, bx, by) = (a[0] * k, a[1] * k, b[0] * k, b[1] * k);
        let (dx, dy) = (bx - ax, by - ay);
        let len2 = (dx * dx + dy * dy).max(1e-6);
        let len = len2.sqrt();
        let (nx, ny) = (-dy / len, dx / len);
        let side = if -0.6 * nx - 0.8 * ny >= 0.0 { 1.0 } else { -1.0 };
        let (ha, hb) = (wa * k * 0.5, wb * k * 0.5);
        let m = ha.max(hb) + 1.0;
        for y in (ay.min(by) - m).floor() as i32..=(ay.max(by) + m).ceil() as i32 {
            for x in (ax.min(bx) - m).floor() as i32..=(ax.max(bx) + m).ceil() as i32 {
                let (px, py) = (x as f32 + 0.5 - ax, y as f32 + 0.5 - ay);
                let t = (px * dx + py * dy) / len2;
                if t > 1.0 { continue; }
                let hw = ha + (hb - ha) * t.max(0.0);
                let across = px * nx + py * ny;
                if (t < 0.0 && px * px + py * py > ha * ha) || (t >= 0.0 && across.abs() > hw) { continue; }
                let s = (across / hw.max(1e-3)).clamp(-1.0, 1.0) * side;
                let round = (1.0 - s * s).max(0.0).sqrt();
                let furrow = 1.0 - furrows + furrows * ((across / k) * 1.7 + seed + 0.8 * ((t * len / k) * 0.3 + seed).sin()).sin();
                self.put(x, y, mul(c, (0.7 + 0.25 * round + 0.3 * s) * furrow));
            }
        }
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
/// A lumpy rock all round, lit from the upper left, with a few craters.
/// What a lump of rock is like: out in space (craters, a black night side) or lying on the ground
/// (cut flat where it meets it, broken into faces, darker towards its foot).
struct Lump { space: bool, floor: f32, faces: f32 }

/// A lump of rock lit from the upper left, `ax` by `ay` about (`cx`, `cy`): an uneven outline, its
/// surface rising and falling, and what `kind` asks for.
fn lump(c: &mut Canvas, r: &mut Rng, t: [f32; 3], centre: [f32; 2], radii: [f32; 2], kind: Lump) {
    let ([cx, cy], [ax, ay]) = (centre, radii);
    let outline: Vec<(f32, f32)> = (2..7).map(|k| (r.range(0.03, 0.11) / (k as f32 * 0.5), r.range(0.0, 6.2832))).collect();
    let swell: Vec<(f32, f32, f32, f32)> = (0..5).map(|_| (r.range(1.5, 5.0), r.range(1.5, 5.0), r.range(0.0, 6.2832), r.range(0.1, 0.22))).collect();
    let craters: Vec<(f32, f32, f32)> = if kind.space { (0..r.range(4.0, 8.0) as usize).map(|_| (r.range(-0.6, 0.6), r.range(-0.6, 0.6), r.range(0.07, 0.22))).collect() } else { Vec::new() };
    let k = c.k;
    for y in 0..c.h as i32 {
        for x in 0..c.w as i32 {
            let (fx, fy) = ((x as f32 + 0.5) / k, (y as f32 + 0.5) / k);
            if fy > kind.floor { continue; }
            let (p, q) = ((fx - cx) / ax, (fy - cy) / ay);
            let (ang, rr) = (q.atan2(p), (p * p + q * q).sqrt());
            let edge = 1.0 + outline.iter().enumerate().map(|(i, (a, ph))| a * ((i + 2) as f32 * ang + ph).cos()).sum::<f32>();
            let s = rr / edge;
            if s >= 1.0 { continue; }
            // The round of the lump, then its swells and hollows leaning the surface this way and that.
            let nz = (1.0 - s * s).sqrt();
            let (mut nx, mut ny) = (s * ang.cos(), s * ang.sin());
            for (wx, wy, ph, amp) in &swell {
                // (Broken rock: the swells are squared off into faces with creases between.)
                let wave = (wx * p + wy * q + ph).cos();
                let g = amp * (wave + kind.faces * ((3.0 * wave).clamp(-1.0, 1.0) - wave));
                let lean = 0.25 + 0.3 * kind.faces;
                nx += g * wx * lean;
                ny += g * wy * lean;
            }
            let mut dust = 1.0;
            for (ccx, ccy, rad) in &craters {
                let (dx, dy) = (p - ccx, q - ccy);
                let d = (dx * dx + dy * dy).sqrt() / rad;
                if d < 1.0 {
                    // The bowl: its floor level, its wall steep.
                    let wall = d * d * d;
                    nx -= dx / rad * 0.9 * wall / d.max(0.05);
                    ny -= dy / rad * 0.9 * wall / d.max(0.05);
                    dust *= 0.86 + 0.14 * d;
                } else if d < 1.3 {
                    let rim = 1.0 - (d - 1.0) / 0.3;
                    nx += dx / rad * 0.5 * rim / d;
                    ny += dy / rad * 0.5 * rim / d;
                }
            }
            let len = (nx * nx + ny * ny + nz * nz).sqrt();
            let lit = ((-0.52 * nx - 0.6 * ny + 0.6 * nz) / len).max(0.0);
            let h = ((x / 3) as u32).wrapping_mul(0x9E37_79B1) ^ ((y / 3) as u32).wrapping_mul(0x85EB_CA77);
            let grain = 0.9 + 0.2 * ((h ^ (h >> 15)).wrapping_mul(0x2C1B_3C6D) >> 24) as f32 / 255.0;
            let shade = if kind.space { 0.035 + 1.25 * lit.powf(0.85) } else {
                // On the ground the sky fills its shadows, and it darkens where it meets the earth.
                (0.4 + 0.8 * lit.powf(0.9)) * (0.62 + 0.38 * smooth01((kind.floor - fy) / (0.3 * ay)))
            };
            c.put(x, y, mul(t, shade * dust * grain));
        }
    }
}

fn smooth01(x: f32) -> f32 { let t = x.clamp(0.0, 1.0); t * t * (3.0 - 2.0 * t) }

fn paint_asteroid(tint: Rgb, variant: u32) -> Sprite {
    // A lump of rock in hard sunlight: an uneven outline, its surface rising and falling, craters
    // with a lit far wall and a shadowed near one, and a night side that is nearly black.
    let mut r = Rng(variant.wrapping_mul(2_654_435_761).wrapping_add(0xA57) | 1);
    let mut c = Canvas::new(160, 128);
    let radii = [r.range(60.0, 72.0), r.range(44.0, 54.0)];
    lump(&mut c, &mut r, rgb_lin(tint), [80.0, 64.0], radii, Lump { space: true, floor: 1e9, faces: 0.0 });
    c.finish()
}

/// A crystal standing from `base` to `tip`, `w` wide: three long faces (one to the light, one to
/// the eye, one in shade) that turn in to a point from `shoulder` of the way up, light pooling
/// inside it towards the tip, and a glint down the edge that faces the light.
fn shard(c: &mut Canvas, base: [f32; 2], tip: [f32; 2], w: f32, shoulder: f32, t: [f32; 3]) {
    let (dx, dy) = (tip[0] - base[0], tip[1] - base[1]);
    let len = (dx * dx + dy * dy).sqrt().max(1e-3);
    let (nx, ny) = (-dy / len, dx / len);
    // (Across is towards the light when negative: the light is at the upper left.)
    let side = if nx > 0.0 { -1.0 } else { 1.0 };
    let at = |along: f32, across: f32| [base[0] + dx * along + nx * side * across * w, base[1] + dy * along + ny * side * across * w];
    let (a, b) = (-0.14, 0.2);
    c.poly(&[at(0.0, -0.5), at(shoulder, -0.46), tip, at(shoulder, a), at(0.0, a)], mul(t, 1.3), 0.15);
    c.poly(&[at(0.0, b), at(shoulder, b), tip, at(shoulder, 0.46), at(0.0, 0.5)], mul(t, 0.6), 0.15);
    c.poly(&[at(0.0, a), at(shoulder, a), tip, at(shoulder, b), at(0.0, b)], t, 0.1);
    let mid = at(0.62, 0.0);
    c.stain(mid[0], mid[1], 0.3 * len, 0.42 * w, dy.atan2(dx), [0.95, 0.98, 1.0], 0.3);
    let foot = at(0.06, 0.0);
    c.stain(foot[0], foot[1], 0.16 * len, 0.6 * w, dy.atan2(dx), mul(t, 0.35), 0.5);
    c.line(at(0.08, a), at(shoulder * 0.96, a), 1.1, [0.95, 0.98, 1.0]);
}

/// How far a cactus flower sits above the tip it grows from, by where across the tip it is.
fn w_tip(d: f32) -> f32 { 10.0 - 0.4 * d.abs() }

/// Foliage colour for a light level from 0 (deep shade) to 1 (full sun): dark and cool in the
/// shade, light and warm in the sun, in a few soft steps the way a painter lays foliage in.
fn leaf_tone(t: [f32; 3], lit: f32) -> [f32; 3] {
    let l = lit.clamp(0.0, 1.0);
    let q = 0.6 * ((l * 3.0).round() / 3.0) + 0.4 * l;
    let shade = mix(mul(t, 0.28), [0.010, 0.026, 0.042], 0.3);
    let sun = [t[0] * 1.5 + 0.03, t[1] * 1.32 + 0.022, t[2] * 0.85];
    if q < 0.5 { mix(shade, t, q * 2.0) } else { mix(t, sun, (q - 0.5) * 2.0) }
}

/// The outward normal of a dome over the ellipse (cx, cy, rx, ry) at (x, y), facing the viewer.
fn dome_normal(x: f32, y: f32, cx: f32, cy: f32, rx: f32, ry: f32) -> [f32; 3] {
    let (nx, ny) = ((x - cx) / rx, (y - cy) / ry);
    let r2 = nx * nx + ny * ny;
    if r2 >= 1.0 { let r = r2.sqrt(); [nx / r, ny / r, 0.0] } else { [nx, ny, (1.0 - r2).sqrt()] }
}

/// Sun on foliage whose crown faces `crown` and whose own clump faces `clump`, lit from the
/// upper left; `below` (0 at the crown's top, 1 at its bottom) takes light away underneath.
fn foliage_light(crown: [f32; 3], clump: [f32; 3], below: f32) -> f32 {
    let n = [0.55 * crown[0] + 0.45 * clump[0], 0.55 * crown[1] + 0.45 * clump[1], 0.55 * crown[2] + 0.45 * clump[2]];
    let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-4);
    let d = ((-0.45 * n[0] - 0.7 * n[1] + 0.55 * n[2]) / l).max(0.0);
    (0.12 + 0.88 * d) * (1.0 - 0.45 * below.clamp(0.0, 1.0))
}

/// The crown of a broadleaf tree, as one dome: (cx, cy, rx, ry).
type Crown = (f32, f32, f32, f32);

/// One clump of leaves round `p`, `rad` across, `depth` from -1 (at the back of the crown) to 1
/// (at the front): leaf dabs laid back to front, each lit as part of the clump and of the crown,
/// with a few strays past the clump's edge to break its outline.
fn leaf_clump(c: &mut Canvas, r: &mut Rng, t: [f32; 3], cr: Crown, p: [f32; 2], rad: f32, depth: f32) {
    let n = (rad * rad * 0.55) as usize;
    let mut dabs: Vec<(f32, f32, f32, bool)> = (0..n).map(|_| {
        let a = r.range(0.0, std::f32::consts::TAU);
        let stray = r.f() < 0.06;
        let rr = if stray { r.range(0.95, 1.2) } else { r.f().sqrt() * 0.95 };
        let (lx, ly) = (a.cos() * rr, a.sin() * rr * 0.85);
        (lx, ly, (1.0 - lx * lx - ly * ly).max(0.0).sqrt(), stray)
    }).collect();
    dabs.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap());
    let back = 0.72 + 0.28 * (depth + 1.0) * 0.5;
    for (lx, ly, lz, stray) in dabs {
        let (x, y) = (p[0] + lx * rad, p[1] + ly * rad);
        let below = (y - (cr.1 - cr.3)) / (2.0 * cr.3);
        let lit = foliage_light(dome_normal(x, y, cr.0, cr.1, cr.2, cr.3), [lx, ly, lz], below) * back * r.range(0.9, 1.08);
        let size = if stray { r.range(1.5, 2.4) } else { r.range(2.4, 4.4) };
        c.dab(x, y, size * r.range(1.0, 1.5), size, r.range(0.0, std::f32::consts::PI), leaf_tone(t, lit));
    }
}

/// A bare limb from `a` heading `ang` (radians from the right, a quarter turn = straight up),
/// `len` long and `w` wide at its start: it bends along its length, then forks into two or three
/// thinner ones that do the same, down to twigs; now and then one has snapped off short.
#[allow(clippy::too_many_arguments)]
fn bare_limb(c: &mut Canvas, r: &mut Rng, wood: [f32; 3], a: [f32; 2], ang: f32, len: f32, w: f32, depth: u32) {
    use std::f32::consts::PI;
    let segs = if depth == 0 { 3 } else { 2 };
    let (mut p, mut an, mut ww) = (a, ang, w);
    for sgm in 0..segs {
        let l = len / segs as f32;
        let q = [p[0] + an.cos() * l, p[1] - an.sin() * l];
        let w1 = ww * if sgm == segs - 1 { 0.66 } else { 0.86 };
        c.limb(p, q, ww, w1, wood, r.f() * 6.0);
        (p, ww) = (q, w1);
        // Now and then a lesser bough leaves the side of a trunk or a main limb.
        if depth <= 1 && sgm < segs - 1 && r.f() < 0.55 {
            let side = if r.f() < 0.5 { -1.0 } else { 1.0 };
            let sa = (an + side * r.range(0.6, 1.05)).clamp(0.12, PI - 0.12);
            let sl = len * r.range(0.3, 0.45);
            bare_limb(c, r, wood, p, sa, sl, ww * 0.42, depth + 2);
        }
        // A trunk stands nearly true; its boughs wander.
        an += if depth == 0 { r.range(-0.09, 0.09) } else { r.range(-0.28, 0.28) };
    }
    if depth >= 4 || ww < 1.3 || (depth >= 2 && r.f() < 0.12) { return; }
    let kids = if depth == 0 { 3 } else if r.f() < 0.35 { 3 } else { 2 };
    let spread = r.range(0.45, 0.8);
    for k in 0..kids {
        let f = k as f32 / (kids - 1) as f32 * 2.0 - 1.0;
        // Boughs reach up and out, never far below level.
        let ca = (an + f * spread + r.range(-0.15, 0.15)).clamp(0.12, PI - 0.12);
        let kw = ww * r.range(0.6, 0.8) * if f == 0.0 { 1.1 } else { 1.0 };
        let kl = len * r.range(0.55, 0.72);
        bare_limb(c, r, wood, p, ca, kl, kw, depth + 1);
    }
}

/// A willow branchlet springing from `start` on a limb: it arches up and over to `apex`, then
/// hangs from there to `y1`, drawn as narrow leaves all along it.
#[allow(clippy::too_many_arguments)]
fn arching_branchlet(c: &mut Canvas, r: &mut Rng, t: [f32; 3], start: [f32; 2], apex: [f32; 2], y1: f32, out: f32, front: f32, lit_at: &dyn Fn(f32, f32) -> f32) {
    // The rise: a curve that climbs from the limb and levels off at the apex, heading outward.
    let ctrl = [start[0] + (apex[0] - start[0]) * 0.35, apex[1] - r.range(3.0, 9.0)];
    let len = ((apex[0] - start[0]).powi(2) + (apex[1] - start[1]).powi(2)).sqrt() * 1.15;
    let tone = r.range(0.7, 1.12);
    let mut s = 0.0;
    let mut alt = 1.0;
    while s < len {
        let u = s / len;
        let p = [
            (1.0 - u) * (1.0 - u) * start[0] + 2.0 * (1.0 - u) * u * ctrl[0] + u * u * apex[0],
            (1.0 - u) * (1.0 - u) * start[1] + 2.0 * (1.0 - u) * u * ctrl[1] + u * u * apex[1],
        ];
        let d = [2.0 * (1.0 - u) * (ctrl[0] - start[0]) + 2.0 * u * (apex[0] - ctrl[0]), 2.0 * (1.0 - u) * (ctrl[1] - start[1]) + 2.0 * u * (apex[1] - ctrl[1])];
        // Leaves along the shoot, drooping.
        let ang = d[1].atan2(d[0]) + alt * r.range(0.5, 1.0);
        c.dab(p[0], p[1], 2.4, 0.8, ang, leaf_tone(t, lit_at(p[0], p[1]) * front * tone * r.range(0.88, 1.1)));
        if r.f() < 0.5 { alt = -alt; }
        s += r.range(1.0, 1.8);
    }
    branchlet(c, r, t, apex[0], apex[1], y1, out, front, lit_at);
}

/// A willow branchlet hanging from (x, y0) to y1: it leaves its arch outward by `out` and falls
/// nearly straight, drawn as narrow leaves along it, smaller towards the tip. `lit_at` gives
/// the light at a point of it; `front` dims the ones behind.
fn branchlet(c: &mut Canvas, r: &mut Rng, t: [f32; 3], x: f32, y0: f32, y1: f32, out: f32, front: f32, lit_at: &dyn Fn(f32, f32) -> f32) {
    let len = (y1 - y0).max(4.0);
    let sway = r.range(-5.0, 5.0);
    let tone = r.range(0.65, 1.15);
    let mut s = 0.0;
    let mut alt = 1.0;
    while s < len {
        let f = s / len;
        let px = x + out * (1.0 - (-s / 10.0).exp()) + sway * f * f;
        let py = y0 + s;
        let size = 1.0 - 0.45 * f;
        let lit = lit_at(px, py) * front * tone;
        let ang = std::f32::consts::FRAC_PI_2 + alt * r.range(0.1, 0.7);
        c.dab(px + alt * r.range(0.0, 0.7) * size, py, 2.5 * size, 0.8 * size, ang, leaf_tone(t, lit * r.range(0.88, 1.1)));
        if r.f() < 0.5 { alt = -alt; }
        s += r.range(0.9, 1.7);
    }
}

pub fn paint_prop(kind: PropKind, tint: Rgb, variant: u32) -> Sprite {
    let t = rgb_lin(tint);
    let mut r = Rng(variant.wrapping_mul(2_654_435_761).wrapping_add(0x9e37) | 1);
    let bark = mix([0.05, 0.035, 0.02], t, 0.15);
    match kind {
        PropKind::Tree => {
            // A broadleaf tree as a painter lays one in: a tapering trunk flaring at the root and
            // forking into limbs, and a crown of separate leaf clumps with sky and branches showing
            // between them, each clump lit as part of the whole crown from the upper left (light
            // and warm on top, dark and cool underneath), its edge broken by stray leaves.
            let mut c = Canvas::new(208, 256);
            let cr: Crown = (104.0 + r.range(-6.0, 6.0), 94.0, r.range(80.0, 90.0), r.range(70.0, 78.0));
            let n = 13 + (variant % 4) as usize;
            let mut clumps: Vec<([f32; 2], f32, f32)> = Vec::new();
            for _ in 0..600 {
                if clumps.len() >= n { break; }
                let a = r.range(0.0, std::f32::consts::TAU);
                let rr = r.f().sqrt() * 0.76;
                let p = [cr.0 + a.cos() * rr * cr.2, cr.1 + a.sin() * rr * cr.3];
                // Bigger clumps up top where the light is.
                let rad = r.range(19.0, 28.0) * (1.12 - 0.22 * ((p[1] - cr.1) / cr.3 + 1.0) * 0.5);
                if clumps.iter().any(|(q, qr, _)| ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt() < 0.6 * (rad + qr)) { continue; }
                clumps.push((p, rad, r.range(-1.0, 1.0)));
            }
            clumps.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap());
            let tw = r.range(15.0, 20.0);
            let fork = [cr.0 + r.range(-4.0, 4.0), cr.1 + cr.3 * 0.6];
            let bark = mul(bark, 1.6);
            // Three main limbs, and a branch from the nearest one out to every clump.
            let ends: Vec<[f32; 2]> = [-0.5f32, 0.05, 0.5].iter().map(|&f| [cr.0 + f * cr.2 + r.range(-8.0, 8.0), cr.1 - (0.42 - f.abs() * 0.5) * cr.3 + r.range(-6.0, 6.0)]).collect();
            for (p, rad, z) in clumps.iter().filter(|k| k.2 < 0.0) { leaf_clump(&mut c, &mut r, t, cr, *p, *rad, *z); }
            c.poly(&[[104.0 - tw * 1.0, 256.0], [104.0 + tw * 1.0, 256.0], [104.0 + tw * 0.5, 240.0], [104.0 - tw * 0.5, 240.0]], mul(bark, 0.75), 0.6);
            c.limb([104.0, 256.0], fork, tw, tw * 0.72, bark, 0.4);
            for (i, e) in ends.iter().enumerate() {
                let mid = [fork[0] + (e[0] - fork[0]) * 0.45 + r.range(-6.0, 6.0), fork[1] + (e[1] - fork[1]) * 0.55];
                c.limb(fork, mid, tw * 0.55, tw * 0.4, bark, i as f32 * 2.1);
                c.limb(mid, *e, tw * 0.4, tw * 0.2, bark, i as f32 * 2.1 + 1.0);
            }
            for (p, _, _) in &clumps {
                let e = ends.iter().min_by(|a, b| ((a[0] - p[0]).powi(2) + (a[1] - p[1]).powi(2)).partial_cmp(&((b[0] - p[0]).powi(2) + (b[1] - p[1]).powi(2))).unwrap()).unwrap();
                c.limb(*e, *p, tw * 0.2, 1.6, bark, p[0]);
            }
            for (p, rad, z) in clumps.iter().filter(|k| k.2 >= 0.0) { leaf_clump(&mut c, &mut r, t, cr, *p, *rad, *z); }
            c.finish()
        }
        PropKind::Pine => {
            // A conifer: a straight tapering trunk bare at the foot, and whorls of boughs that sweep
            // out and down, shortest at the top. Each bough is sprays of needles, lit along its upper
            // side and dark beneath; trunk and shadow show between the tiers, and the outline is
            // ragged, never a triangle.
            let mut c = Canvas::new(176, 256);
            let cx = 88.0;
            let top = r.range(8.0, 16.0);
            let skirt = r.range(194.0, 216.0);
            let reach = r.range(58.0, 74.0);
            let lean = r.range(-5.0, 5.0);
            let axis = |y: f32| cx + lean * ((256.0 - y) / 256.0).powi(2);
            let cr: Crown = (cx, 0.5 * (top + skirt) + 10.0, reach * 1.05, 0.5 * (skirt - top) + 24.0);
            // One bough from the trunk at height y: `side` -1 left, 1 right, 0 hanging towards the
            // viewer over the trunk; `depth` dims the ones round the back.
            let bough = |c: &mut Canvas, r: &mut Rng, y: f32, len: f32, side: f32, depth: f32| {
                let droop = r.range(0.26, 0.42);
                let skew = r.range(-0.3, 0.3);
                let n = ((len / 2.0) as i32).max(2);
                for i in 0..=n {
                    let u = i as f32 / n as f32;
                    let (bx, by, ang) = if side == 0.0 {
                        (axis(y) + skew * len * u, y + 0.55 * len * u, std::f32::consts::FRAC_PI_2 + skew)
                    } else {
                        // Out and down, the tip lifting a little.
                        (axis(y) + side * len * u, y + droop * len * u.powf(1.4) - 0.1 * len * u * u * u, if side > 0.0 { droop } else { std::f32::consts::PI - droop })
                    };
                    let th = (0.15 * len + 3.0) * (1.0 - 0.72 * u);
                    for _ in 0..3 {
                        // More of the spray hangs below the bough than stands above it.
                        let off = r.range(-0.45, 1.0);
                        let (x, yy) = (bx + r.range(-1.6, 1.6), by + off * th);
                        let below = (yy - top) / (skirt + 30.0 - top);
                        let lit = foliage_light(dome_normal(x, yy, cr.0, cr.1, cr.2, cr.3), [side * 0.4, off * 0.9, 0.45], below) * depth * r.range(0.88, 1.1) * (0.9 + 0.22 * u + 0.2 * (-off).max(0.0));
                        c.dab(x, yy, r.range(3.4, 5.6), r.range(1.2, 1.9), ang + r.range(-0.5, 0.5), leaf_tone(t, lit));
                    }
                }
            };
            // The tiers, from the lowest up: closer together and shorter towards the top.
            let mut tiers: Vec<(f32, f32)> = Vec::new();
            let mut y = skirt;
            while y > top + 8.0 {
                let f = ((y - top) / (skirt - top)).clamp(0.0, 1.0);
                tiers.push((y, reach * f.powf(0.85) * r.range(0.82, 1.1) + 4.0));
                y -= r.range(9.0, 13.0) * (0.5 + 0.5 * f);
            }
            for (y, len) in &tiers {
                for side in [-1.0, 1.0] {
                    let l = len * r.range(0.75, 0.95);
                    bough(&mut c, &mut r, y + 5.0, l, side, 0.55);
                }
            }
            let tw = r.range(11.0, 15.0);
            let bark = mul(bark, 1.6);
            c.poly(&[[cx - tw * 0.9, 256.0], [cx + tw * 0.9, 256.0], [cx + tw * 0.5, 244.0], [cx - tw * 0.5, 244.0]], mul(bark, 0.75), 0.6);
            c.limb([cx, 256.0], [axis(top + 4.0), top + 4.0], tw, 1.4, bark, 0.7);
            for (y, len) in &tiers {
                for side in [-1.0, 1.0] { bough(&mut c, &mut r, *y, *len, side, 1.0); }
                if r.f() < 0.75 { bough(&mut c, &mut r, *y, len * 0.5, 0.0, 0.95); }
            }
            // The leader at the very top.
            for k in 0..6 { c.dab(axis(top) + r.range(-1.5, 1.5), top - 2.0 + k as f32 * 2.2, 1.6, 3.2, r.range(-0.2, 0.2), leaf_tone(t, 0.75)); }
            c.finish()
        }
        PropKind::Bush => {
            // A shrub as a mound of leaf clumps sitting on the ground, lit as one dome like a tree's
            // crown, with a few twigs showing low down between the clumps.
            let mut c = Canvas::new(192, 128);
            let cr: Crown = (96.0 + r.range(-4.0, 4.0), 84.0, r.range(78.0, 88.0), r.range(44.0, 50.0));
            for _ in 0..5 {
                let x = cr.0 + r.range(-40.0, 40.0);
                c.limb([cr.0 + r.range(-6.0, 6.0), 128.0], [x, r.range(84.0, 104.0)], 4.0, 1.2, mul(bark, 1.6), x);
            }
            let n = 10 + (variant % 3) as usize;
            let mut clumps: Vec<([f32; 2], f32, f32)> = Vec::new();
            for _ in 0..400 {
                if clumps.len() >= n { break; }
                let a = r.range(0.0, std::f32::consts::TAU);
                let rr = r.f().sqrt() * 0.75;
                let rad = r.range(17.0, 24.0);
                // The lowest clumps sit on the ground.
                let p = [cr.0 + a.cos() * rr * cr.2, (cr.1 + a.sin() * rr * cr.3).min(128.0 - rad * 0.8)];
                if clumps.iter().any(|(q, qr, _)| ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt() < 0.6 * (rad + qr)) { continue; }
                clumps.push((p, rad, r.range(-1.0, 1.0)));
            }
            clumps.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap());
            for (p, rad, z) in &clumps { leaf_clump(&mut c, &mut r, t, cr, *p, *rad, *z); }
            c.finish()
        }
        PropKind::Rock | PropKind::Boulder => {
            // Broken stone lying where it fell: faces catching the light differently, a crack or
            // two, lichen on its upper side, dark where it sits in the earth. A boulder stands
            // taller; a rock lies low.
            let mut c = Canvas::new(192, 128);
            let (cy, ay) = if kind == PropKind::Boulder { (r.range(72.0, 80.0), r.range(70.0, 78.0)) } else { (r.range(96.0, 104.0), r.range(50.0, 60.0)) };
            let ax = r.range(70.0, 80.0);
            lump(&mut c, &mut r, t, [96.0, cy], [ax, ay], Lump { space: false, floor: 127.0, faces: 0.85 });
            for _ in 0..2 {
                let (x, y) = (r.range(50.0, 140.0), r.range(cy - 0.4 * ay, 104.0));
                let (dx, dy) = (r.range(-22.0, 22.0), r.range(10.0, 22.0));
                c.stain(x + dx * 0.5, y + dy * 0.5, 0.6 * (dx * dx + dy * dy).sqrt(), 1.3, dy.atan2(dx), mul(t, 0.25), 0.9);
            }
            for _ in 0..3 { c.stain(r.range(50.0, 140.0), cy - ay * r.range(0.3, 0.8), r.range(10.0, 22.0), r.range(5.0, 10.0), r.range(-0.4, 0.4), [0.42, 0.46, 0.24], 0.35); }
            c.stain(96.0, 126.0, 90.0, 9.0, 0.0, [0.1, 0.14, 0.05], 0.5);
            c.finish()
        }
        PropKind::Cactus => {
            // A saguaro: a ribbed column with arms that leave it sideways and turn up, thorns down
            // every rib, sometimes in flower.
            let mut c = Canvas::new(144, 256);
            let green = mul(t, 0.8);
            let arms = [(-1.0f32, r.range(104.0, 150.0), r.range(48.0, 78.0), r.range(34.0, 44.0), 19.0), (1.0, r.range(120.0, 176.0), r.range(34.0, 60.0), r.range(30.0, 40.0), 17.0)];
            let top = r.range(20.0, 34.0);
            let mut tips = vec![[72.0, top]];
            for (side, y, up, out, w) in arms {
                if side > 0.0 && variant % 3 == 2 { continue; }
                let x = 72.0 + side * out;
                c.ribbed([72.0, y], [x, y], w, green, 5.0, false, true);
                c.ribbed([x, y], [x, y - up], w, green, 5.0, false, true);
                tips.push([x, y - up]);
            }
            c.ribbed([72.0, 262.0], [72.0, top], 30.0, green, 7.0, false, true);
            if variant % 2 == 0 {
                for [x, y] in tips { for d in [-5.0f32, 0.0, 5.0] { c.ellipse(x + d, y - 0.5 * w_tip(d), 4.0, 3.2, [0.95, 0.9, 0.82], 0.5); c.ellipse(x + d, y - 0.5 * w_tip(d), 1.4, 1.2, [0.95, 0.75, 0.2], 0.0); } }
            }
            c.stain(72.0, 254.0, 30.0, 12.0, 0.0, [0.25, 0.2, 0.1], 0.5);
            c.finish()
        }
        PropKind::DeadTree => {
            // A dead tree: a tapering trunk, a little off true, forking again and again into crooked
            // boughs and bare twigs, some snapped short; its wood furrowed and lit from the upper left.
            let mut c = Canvas::new(208, 256);
            let x0 = 104.0 + r.range(-6.0, 6.0);
            let tw = r.range(15.0, 21.0);
            c.poly(&[[x0 - tw * 0.95, 256.0], [x0 + tw * 0.95, 256.0], [x0 + tw * 0.5, 243.0], [x0 - tw * 0.5, 243.0]], mul(t, 0.8), 0.6);
            let trunk_len = r.range(86.0, 102.0);
            let ang = std::f32::consts::FRAC_PI_2 + r.range(-0.1, 0.1);
            bare_limb(&mut c, &mut r, t, [x0, 256.0], ang, trunk_len, tw, 0);
            c.finish()
        }
        PropKind::Mushroom => {
            // A toadstool: a domed cap with warts, pale gills under its lip, a stem with a ring.
            let mut c = Canvas::new(160, 160);
            let (stem, gill) = ([0.78, 0.74, 0.62], [0.5, 0.45, 0.36]);
            let lean = r.range(-8.0, 8.0);
            let (cx, cy, rx, ry) = (80.0 + lean, r.range(60.0, 68.0), r.range(64.0, 74.0), r.range(40.0, 48.0));
            c.ellipse(cx, cy, rx, ry, mul(t, 0.85), 1.0);
            c.cut(|c| c.rect(0.0, cy + 0.22 * ry, 160.0, 160.0, [0.0; 3], 0.0));
            c.ellipse(cx, cy + 0.22 * ry, rx * 0.96, 7.0, gill, 0.5);
            for i in 0..15 { let x = cx - 0.9 * rx + 1.8 * rx * i as f32 / 14.0; c.line([cx + (x - cx) * 0.3, cy + 0.22 * ry + 4.0], [x, cy + 0.22 * ry - 3.0], 0.9, mul(gill, 0.6)); }
            c.rod([80.0, 164.0], [cx, cy + 0.22 * ry + 3.0], 34.0, 24.0, stem);
            c.ellipse((80.0 + cx) * 0.5, cy + 0.22 * ry + 28.0, 17.0, 5.0, mul(stem, 0.9), 0.6);
            c.stain(cx, cy + 0.22 * ry + 10.0, 22.0, 12.0, 0.0, [0.2, 0.17, 0.12], 0.6);
            for _ in 0..r.range(6.0, 10.0) as usize {
                // Warts sit on the dome: smaller and flatter towards its edge.
                let (u, v) = (r.range(-0.85, 0.85), r.range(-0.8, 0.1));
                if u * u + v * v > 0.8 { continue; }
                let size = r.range(4.0, 9.0) * (1.0 - 0.5 * u.abs());
                c.ellipse(cx + u * rx, cy + v * ry, size * (1.0 - 0.4 * u.abs()), size * 0.75, [0.93, 0.9, 0.82], 0.6);
            }
            c.stain(cx - 0.35 * rx, cy - 0.55 * ry, 0.3 * rx, 0.16 * ry, -0.3, [1.0, 1.0, 1.0], 0.22);
            c.stain(80.0, 158.0, 30.0, 8.0, 0.0, [0.12, 0.1, 0.06], 0.6);
            c.finish()
        }
        PropKind::Pillar => {
            // A stone column: a fluted shaft on a moulded base under a square capital; some have
            // lost their tops.
            let mut c = Canvas::new(96, 256);
            c.ribbed([48.0, 238.0], [48.0, 22.0], 54.0, t, 9.0, true, false);
            c.rect(8.0, 4.0, 88.0, 20.0, mul(t, 1.08), 0.7);
            c.rect(8.0, 4.0, 88.0, 7.0, mul(t, 1.3), 0.3);
            c.ellipse(48.0, 24.0, 35.0, 6.0, mul(t, 0.95), 0.9);
            c.ellipse(48.0, 238.0, 36.0, 7.0, mul(t, 0.95), 0.9);
            c.rect(8.0, 242.0, 88.0, 256.0, mul(t, 0.92), 0.7);
            for _ in 0..4 { c.stain(r.range(24.0, 72.0), r.range(40.0, 230.0), r.range(4.0, 9.0), r.range(16.0, 40.0), 0.0, mul(t, 0.45), 0.3); }
            c.stain(48.0, 250.0, 44.0, 14.0, 0.0, [0.14, 0.18, 0.08], 0.4);
            if variant % 2 == 1 {
                // Broken top.
                let pts = [[0.0, 0.0], [96.0, 0.0], [96.0, r.range(40.0, 70.0)], [50.0, r.range(20.0, 40.0)], [0.0, r.range(60.0, 90.0)]];
                c.cut(|c| c.poly(&pts, [0.0; 3], 0.0));
            }
            c.finish()
        }
        PropKind::Gravestone => {
            // A weathered headstone, leaning a little: a bevelled slab (round-topped, pointed, or a
            // cross), its carving worn shallow, streaked by rain, lichen and moss creeping up it.
            let mut c = Canvas::new(128, 160);
            let lean = r.range(-0.07, 0.07);
            let (sn, cs) = lean.sin_cos();
            let at = |x: f32, y: f32| [64.0 + (x - 64.0) * cs - (y - 160.0) * sn, 160.0 + (x - 64.0) * sn + (y - 160.0) * cs];
            let shape: Vec<[f32; 2]> = match variant % 3 {
                0 => {
                    let mut p = vec![[22.0, 164.0], [22.0, 62.0]];
                    for i in 0..=12 { let a = PI * (1.0 - i as f32 / 12.0); p.push([64.0 + 42.0 * a.cos(), 62.0 - 40.0 * a.sin()]); }
                    p.push([106.0, 164.0]);
                    p
                }
                1 => vec![[48.0, 164.0], [48.0, 80.0], [20.0, 80.0], [20.0, 52.0], [48.0, 52.0], [48.0, 18.0], [80.0, 18.0], [80.0, 52.0], [108.0, 52.0], [108.0, 80.0], [80.0, 80.0], [80.0, 164.0]],
                _ => vec![[26.0, 164.0], [26.0, 60.0], [64.0, 26.0], [102.0, 60.0], [102.0, 164.0]],
            };
            let turned = |dx: f32, dy: f32| shape.iter().map(|p| at(p[0] + dx, p[1] + dy)).collect::<Vec<_>>();
            // Its thickness shows down the side away from the light, and a lit edge towards it.
            c.poly(&turned(4.0, 3.0), mul(t, 0.45), 0.0);
            c.poly(&turned(-1.5, -1.5), mul(t, 1.25), 0.0);
            c.poly(&turned(0.0, 0.0), t, 0.5);
            // Carving: cut lines dark, their lower lips catching light.
            let carve = |c: &mut Canvas, a: [f32; 2], b: [f32; 2], w: f32| { c.line(at(a[0] + 1.0, a[1] + 1.0), at(b[0] + 1.0, b[1] + 1.0), w, mul(t, 1.2)); c.line(at(a[0], a[1]), at(b[0], b[1]), w, mul(t, 0.5)); };
            if variant % 3 != 1 {
                carve(&mut c, [64.0, 50.0], [64.0, 86.0], 4.0);
                carve(&mut c, [50.0, 62.0], [78.0, 62.0], 4.0);
            }
            for (i, y) in [100.0f32, 110.0, 120.0].into_iter().enumerate() { let half = if variant % 3 == 1 { 9.0 } else { 26.0 - 5.0 * i as f32 }; carve(&mut c, [64.0 - half, y], [64.0 + half, y], 1.6); }
            for _ in 0..5 { c.stain(r.range(30.0, 98.0), r.range(40.0, 150.0), r.range(3.0, 7.0), r.range(14.0, 36.0), lean, mul(t, 0.4), 0.3); }
            for _ in 0..4 { c.stain(r.range(30.0, 98.0), r.range(30.0, 130.0), r.range(5.0, 11.0), r.range(4.0, 8.0), r.range(0.0, 3.0), [0.5, 0.52, 0.3], 0.4); }
            c.stain(64.0, 158.0, 56.0, 20.0, 0.0, [0.12, 0.2, 0.07], 0.75);
            c.finish()
        }
        PropKind::Crystal => {
            // A cluster of crystal points: the tall ones in the middle, short ones leaning out at
            // the sides, each with its faces lit differently.
            let mut c = Canvas::new(128, 192);
            for i in [0usize, 4, 1, 3, 2] {
                let off = i as f32 - 2.0;
                let x = 64.0 + off * r.range(13.0, 19.0);
                let hgt = r.range(110.0, 182.0) * (1.0 - 0.2 * off.abs());
                let lean = off * r.range(6.0, 13.0) + r.range(-5.0, 5.0);
                shard(&mut c, [x, 196.0], [x + lean, 192.0 - hgt], r.range(20.0, 28.0), r.range(0.78, 0.88), mul(t, r.range(0.85, 1.15)));
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
            // A weeping willow is all branchlets: a short stout trunk divides low into a few big
            // limbs that rise and spread, and from their tops long fine branchlets arch up, over
            // and down, hanging in curtains nearly to the ground. The crown's rounded top is just
            // where the branchlets arch over, so it is made of them too, never of leafy clumps.
            // Lit from the upper left: the arches on top and the sunny side's outer curtains light
            // and yellow-green, the curtains behind deep and cool, sky and limbs between curtains.
            let mut c = Canvas::new(300, 256);
            let lean = r.range(-10.0, 10.0);
            let cr: Crown = (150.0 + lean * 0.6, 88.0, r.range(124.0, 136.0), r.range(64.0, 72.0));
            let top_at = |x: f32| cr.1 - cr.3 * (1.0 - ((x - cr.0) / cr.2).powi(2)).max(0.0).sqrt();
            let lit_at = |x: f32, y: f32| {
                let nx = ((x - cr.0) / cr.2).clamp(-1.0, 1.0);
                let n = [nx, -0.25, (1.0 - nx * nx).max(0.0).sqrt()];
                let d = (-0.45 * n[0] - 0.7 * n[1] + 0.55 * n[2]).max(0.0) / 1.03;
                // Darker deeper under the crown; the tips near the ground see some sky again.
                let under = ((y - cr.1) / (240.0 - cr.1)).clamp(0.0, 1.0);
                let arch = foliage_light(dome_normal(x, y, cr.0, cr.1, cr.2, cr.3), dome_normal(x, y, cr.0, cr.1, cr.2, cr.3), (y - (cr.1 - cr.3)) / (2.0 * cr.3));
                let hang = (0.1 + 0.9 * d) * (1.0 - 0.4 * under * (1.0 - under) * 4.0 * 0.6 - 0.15 * under);
                // Up in the arches the light is the dome's; below them, the curtains'.
                let k = ((y - top_at(x)) / 30.0).clamp(0.0, 1.0);
                arch + (hang - arch) * k
            };
            // The limbs: where they fork from the trunk, and the tops the branchlets spring from.
            let tw = r.range(22.0, 28.0);
            let fork = [150.0 + lean, r.range(178.0, 196.0)];
            let nl = 3 + (variant % 3) as usize;
            let limbs: Vec<([f32; 2], [f32; 2])> = (0..nl).map(|i| {
                let f = (i as f32 + 0.5) / nl as f32 * 2.0 - 1.0 + r.range(-0.12, 0.12);
                let tx = cr.0 + f * cr.2 * 0.6;
                let top = [tx, top_at(tx) + r.range(22.0, 32.0)];
                ([fork[0] + (top[0] - fork[0]) * 0.3, fork[1] + (top[1] - fork[1]) * 0.55], top)
            }).collect();
            let curtain = |c: &mut Canvas, r: &mut Rng, front: f32, count: usize, from: f32| {
                for _ in 0..count {
                    let fx = r.range(-0.97, 0.97);
                    let x0 = cr.0 + fx * cr.2;
                    let base = 236.0 + r.range(-30.0, 16.0) - 22.0 * fx.abs().powi(3);
                    let strands = 6 + (r.f() * 8.0) as usize;
                    let spread = r.range(5.0, 13.0);
                    // The limb this curtain springs from: the one whose top is nearest.
                    let lt = limbs.iter().map(|l| l.1).min_by(|a, b| (a[0] - x0).abs().partial_cmp(&(b[0] - x0).abs()).unwrap()).unwrap();
                    for _ in 0..strands {
                        let x = x0 + r.range(-spread, spread);
                        let apex = [x, top_at(x) + from + r.range(-5.0, 9.0)];
                        let start = [lt[0] + r.range(-10.0, 10.0), lt[1] + r.range(-4.0, 10.0)];
                        let out = (x - cr.0) / cr.2 * r.range(5.0, 13.0);
                        let y1 = (base + r.range(-12.0, 10.0)).min(254.0);
                        arching_branchlet(c, r, t, start, apex, y1, out, front, &lit_at);
                    }
                }
            };
            // The curtains behind, deep in the crown's shade.
            curtain(&mut c, &mut r, 0.45, 28, 6.0);
            let bark = mul(bark, 1.5);
            c.poly(&[[150.0 - tw * 1.05, 256.0], [150.0 + tw * 1.05, 256.0], [150.0 + tw * 0.55, 242.0], [150.0 - tw * 0.55, 242.0]], mul(bark, 0.75), 0.6);
            c.limb([150.0, 256.0], fork, tw, tw * 0.85, bark, 0.3);
            for (i, (mid, top)) in limbs.iter().enumerate() {
                c.limb(fork, *mid, tw * 0.5, tw * 0.36, bark, i as f32 * 1.7);
                c.limb(*mid, *top, tw * 0.36, tw * 0.16, bark, i as f32 * 1.7 + 0.9);
            }
            // The curtains in front, arching over the top of the crown.
            curtain(&mut c, &mut r, 1.0, 48, 0.0);
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
            // Hangs: the top of the sprite is the ceiling. A row of icicles of different lengths,
            // round and tapering, light running down each.
            let mut c = Canvas::new(128, 192);
            let n = 3 + (variant % 4) as usize;
            c.rect(0.0, 0.0, 128.0, 7.0, mul(t, 0.85), 0.5);
            for i in 0..n {
                let x = 18.0 + i as f32 * (92.0 / n.max(1) as f32) + r.range(-6.0, 6.0);
                let len = r.range(60.0, 186.0) * if i == n / 2 { 1.0 } else { 0.7 };
                let wdt = r.range(16.0, 28.0);
                let tip = [x + r.range(-3.0, 3.0), len];
                c.rod([x, -6.0], tip, wdt, 1.2, mul(t, r.range(0.85, 1.1)));
                c.stain(x - 0.16 * wdt, 0.4 * len, 0.09 * wdt, 0.38 * len, 0.0, [1.0, 1.0, 1.0], 0.55);
                c.stain(tip[0], tip[1] - 5.0, 3.0, 5.0, 0.0, [1.0, 1.0, 1.0], 0.5);
            }
            c.finish()
        }
        PropKind::IceSpike => {
            // Shards of clear ice thrust up from the floor: thin, sharp, frosted at the foot.
            let mut c = Canvas::new(128, 192);
            for i in [0usize, 4, 1, 3, 2] {
                let off = i as f32 - 2.0;
                let x = 64.0 + off * r.range(12.0, 20.0);
                let hgt = r.range(100.0, 186.0) * if i == 2 { 1.0 } else { 0.65 };
                let lean = off * r.range(4.0, 12.0) + r.range(-8.0, 8.0);
                shard(&mut c, [x, 196.0], [x + lean, 192.0 - hgt], r.range(18.0, 26.0), r.range(0.25, 0.45), mul(t, r.range(0.8, 1.05)));
            }
            c.stain(64.0, 190.0, 60.0, 12.0, 0.0, [0.95, 0.98, 1.0], 0.5);
            c.finish()
        }
        PropKind::Asteroid => paint_asteroid(tint, variant),
        PropKind::Satellite => {
            // A foil-wrapped body between two wings of solar cells, a dish on a boom: turned a
            // different way each time, the cells catching the sun on one wing more than the other.
            let mut c = Canvas::new(224, 160);
            let ang = r.range(-0.6, 0.6);
            let (sn, cs) = ang.sin_cos();
            let at = |x: f32, y: f32| [112.0 + x * cs - y * sn, 80.0 + x * sn + y * cs];
            let quad = |c: &mut Canvas, x0: f32, y0: f32, x1: f32, y1: f32, col: [f32; 3], shade: f32| c.poly(&[at(x0, y0), at(x1, y0), at(x1, y1), at(x0, y1)], col, shade);
            let (cell, frame, grey) = ([0.01, 0.022, 0.085], [0.2, 0.2, 0.215], [0.3, 0.31, 0.33]);
            for (side, lit) in [(-1.0f32, r.range(0.7, 1.0)), (1.0, r.range(1.3, 2.6))] {
                c.rod(at(side * 18.0, 0.0), at(side * 34.0, 0.0), 3.0, 3.0, grey);
                quad(&mut c, side * 33.0, -20.0, side * 100.0, 20.0, frame, 0.3);
                for i in 0..4 { for j in 0..2 {
                    let x0 = side * (34.5 + 16.3 * i as f32);
                    let y0 = -19.0 + 19.5 * j as f32;
                    quad(&mut c, x0, y0, x0 + side * 14.5, y0 + 18.0, mul(cell, lit * (0.85 + 0.3 * r.f())), 0.25);
                } }
            }
            quad(&mut c, -18.0, -16.0, 18.0, 16.0, t, 0.9);
            for i in 0..9 { let x = -16.0 + 4.0 * i as f32; c.line(at(x, -16.0), at(x + r.range(-4.0, 4.0), 16.0), r.range(0.8, 2.2), mul(t, r.range(0.45, 1.3))); }
            quad(&mut c, -18.0, -19.0, 18.0, -15.0, grey, 0.5);
            c.rod(at(0.0, -17.0), at(6.0, -38.0), 2.6, 2.2, grey);
            let d = at(8.0, -44.0);
            c.dab(d[0], d[1], 19.0, 7.5, ang - 0.3, [0.5, 0.51, 0.54]);
            c.dab(d[0] + 1.5, d[1] + 1.5, 15.0, 5.0, ang - 0.3, [0.24, 0.25, 0.28]);
            c.rod(d, at(13.0, -58.0), 1.6, 1.2, grey);
            c.rod(at(-6.0, 16.0), at(-12.0, 40.0), 1.6, 1.0, grey);
            c.finish()
        }
        PropKind::Debris => {
            // Torn hull plate: a few buckled sheets with ragged edges, ribs showing behind, a spar
            // sticking out, scorched where it broke.
            let mut c = Canvas::new(176, 152);
            let scorch = [0.03, 0.028, 0.026];
            let a0 = r.range(0.0, PI);
            c.rod([88.0 - 66.0 * a0.cos(), 76.0 - 50.0 * a0.sin()], [88.0 + 70.0 * a0.cos(), 76.0 + 52.0 * a0.sin()], 5.0, 3.0, mul(t, 0.5));
            for i in 0..3 {
                let (cx, cy) = (88.0 + r.range(-26.0, 26.0), 76.0 + r.range(-20.0, 20.0));
                let (rx, ry) = (r.range(34.0, 52.0), r.range(24.0, 38.0));
                let n = 9;
                let turn = r.range(0.0, 6.28);
                let pts: Vec<[f32; 2]> = (0..n).map(|j| {
                    let a = turn + j as f32 / n as f32 * 6.2832;
                    let rad = if j % 2 == 0 { r.range(0.75, 1.0) } else { r.range(0.4, 0.8) };
                    [cx + a.cos() * rx * rad, cy + a.sin() * ry * rad]
                }).collect();
                // A dark rim a little larger, then the plate: the torn edge shows its thickness.
                let rim: Vec<[f32; 2]> = pts.iter().map(|p| [cx + (p[0] - cx) * 1.07 + 1.5, cy + (p[1] - cy) * 1.07 + 2.0]).collect();
                c.poly(&rim, scorch, 0.0);
                c.poly(&pts, mul(t, 0.75 + 0.2 * i as f32), 0.9);
                let (la, lb) = (turn + 0.4, turn + 0.4 + PI);
                for k in -1..=1 {
                    let off = 0.3 * k as f32;
                    c.line([cx + (la + off).cos() * rx * 0.45, cy + (la + off).sin() * ry * 0.45], [cx + (lb - off).cos() * rx * 0.45, cy + (lb - off).sin() * ry * 0.45], 1.3, mul(t, 0.5));
                }
                for _ in 0..5 { let (px, py) = (cx + r.range(-0.4, 0.4) * rx, cy + r.range(-0.4, 0.4) * ry); c.ellipse(px, py, 1.6, 1.6, mul(t, 1.3), 0.6); }
            }
            c.finish()
        }
        PropKind::CargoPod => {
            // A pressure can with domed ends, hoops round it and a hazard band, tumbling end over end.
            let mut c = Canvas::new(200, 140);
            let ang = r.range(-0.7, 0.7);
            let (sn, cs) = ang.sin_cos();
            let at = |x: f32, y: f32| [100.0 + x * cs - y * sn, 70.0 + x * sn + y * cs];
            let (steel, dark) = ([0.3, 0.31, 0.34], [0.05, 0.05, 0.055]);
            for side in [-1.0f32, 1.0] { let e = at(side * 54.0, 0.0); c.dab(e[0], e[1], 22.0, 30.0, ang, mul(steel, if side < 0.0 { 1.0 } else { 0.6 })); }
            // The barrel, lit along its upper side.
            for i in 0..12 {
                let (y0, y1) = (-30.0 + 5.0 * i as f32, -25.0 + 5.0 * i as f32);
                let lit = 1.15 - 1.0 * ((i as f32 - 2.5) / 9.5).abs().min(1.0).powf(0.8);
                c.poly(&[at(-54.0, y0), at(54.0, y0), at(54.0, y1), at(-54.0, y1)], mul(t, lit), 0.0);
            }
            for x in [-46.0f32, -16.0, 16.0, 46.0] { c.poly(&[at(x - 2.5, -31.0), at(x + 2.5, -31.0), at(x + 2.5, 31.0), at(x - 2.5, 31.0)], mul(steel, 0.7), 0.0); }
            for i in 0..5 { let x = -12.0 + 5.0 * i as f32; c.poly(&[at(x, -30.0), at(x + 2.5, -30.0), at(x + 8.5, 30.0), at(x + 6.0, 30.0)], if i % 2 == 0 { [0.55, 0.42, 0.06] } else { dark }, 0.0); }
            let h = at(-32.0, -6.0);
            c.dab(h[0], h[1], 9.0, 11.0, ang, dark);
            c.dab(h[0], h[1], 6.5, 8.5, ang, mul(steel, 0.8));
            c.finish()
        }
        PropKind::Comet => {
            // A lump of dirty ice in its own glow, gas and dust streaming away behind it.
            let mut c = Canvas::new(256, 128);
            let (hx, hy) = (58.0, 64.0 + r.range(-8.0, 8.0));
            let lean = r.range(-0.12, 0.12);
            c.haze(hx + 84.0, hy + 84.0 * lean, 108.0, 26.0, lean, mul(t, 0.7), 0.3);
            c.haze(hx + 66.0, hy + 66.0 * (lean + 0.1), 92.0, 12.0, lean + 0.1, [0.9, 0.85, 0.7], 0.22);
            c.haze(hx + 56.0, hy + 56.0 * (lean - 0.07), 84.0, 7.0, lean - 0.07, [0.6, 0.8, 1.0], 0.4);
            c.haze(hx, hy, 40.0, 36.0, 0.0, t, 0.6);
            let n = 10;
            let pts: Vec<[f32; 2]> = (0..n).map(|i| { let a = i as f32 / n as f32 * 6.2832; let rad = r.range(0.72, 1.0); [hx + a.cos() * 17.0 * rad, hy + a.sin() * 14.0 * rad] }).collect();
            c.poly(&pts, [0.62, 0.68, 0.74], 0.9);
            c.ellipse(hx - 4.0, hy - 3.0, 9.0, 7.0, [0.9, 0.95, 1.0], 0.5);
            c.ellipse(hx + 6.0, hy + 5.0, 4.0, 3.0, [0.35, 0.38, 0.42], 0.7);
            c.haze(hx - 2.0, hy, 22.0, 20.0, 0.0, [0.9, 0.97, 1.0], 0.35);
            c.finish()
        }
        PropKind::Stalagmite => {
            // Stone built up drip by drip: one tall spire with smaller ones round its foot, ridged
            // down their length, wet where the light catches.
            let mut c = Canvas::new(112, 224);
            let tip = 50.0 + r.range(-10.0, 10.0);
            c.limb([24.0 + r.range(-4.0, 4.0), 232.0], [16.0 + r.range(-4.0, 6.0), r.range(120.0, 160.0)], 42.0, 2.0, mul(t, 0.8), r.range(0.0, 6.0));
            c.limb([90.0 + r.range(-4.0, 4.0), 232.0], [96.0 + r.range(-6.0, 4.0), r.range(140.0, 176.0)], 38.0, 2.0, mul(t, 0.85), r.range(0.0, 6.0));
            c.limb([56.0, 236.0], [tip, 4.0], 88.0, 3.0, t, r.range(0.0, 6.0));
            for i in 1..6 { let y = 224.0 - i as f32 * 36.0; let half = 44.0 * (y / 224.0) * 0.8; c.stain(56.0 + (tip - 56.0) * (1.0 - y / 224.0), y, half, 2.2, 0.08, mul(t, 0.4), 0.35); }
            c.stain(46.0 + 0.3 * (tip - 56.0), 110.0, 5.0, 70.0, -0.05, [0.9, 0.92, 1.0], 0.2);
            c.stain(56.0, 222.0, 54.0, 10.0, 0.0, mul(t, 0.25), 0.6);
            c.finish()
        }
    }
}

/// Paint a fixture body (the flame or orb is drawn separately). The wall side is on the left.
pub fn paint_fixture(kind: FixtureKind) -> Option<Sprite> {
    let wood = [0.12, 0.06, 0.025];
    match kind {
        FixtureKind::Torch | FixtureKind::GreenFire | FixtureKind::Magic | FixtureKind::IceWisp => {
            // A brand in an iron sconce: a plate bolted to the wall, an arm with a ring, a wooden
            // shaft through it, its head bound in pitch-soaked cloth.
            let mut c = Canvas::new(64, 160);
            let (plate, cloth) = ([0.07, 0.065, 0.06], [0.05, 0.04, 0.03]);
            c.rect(0.0, 66.0, 9.0, 114.0, plate, 0.7);
            c.rect(0.0, 66.0, 9.0, 68.5, mul(plate, 1.8), 0.0);
            for y in [72.0, 108.0] { c.ellipse(4.5, y, 2.2, 2.2, mul(plate, 2.0), 0.9); }
            c.rod([6.0, 96.0], [30.0, 66.0], 5.5, 4.5, plate);
            c.rod([8.0, 78.0], [24.0, 74.0], 3.0, 3.0, plate);
            c.limb([23.0, 160.0], [36.0, 40.0], 9.0, 11.0, wood, 2.0);
            c.rod([21.0, 68.0], [41.0, 64.0], 6.0, 6.0, mul(plate, 1.3));
            // The head: wrapped and tied, charred at the top.
            c.rod([35.0, 46.0], [38.0, 20.0], 18.0, 15.0, cloth);
            for (i, y) in [42.0f32, 36.0, 30.0, 24.0].into_iter().enumerate() { c.line([27.0 + i as f32 * 0.5, y + 2.0], [46.0, y - 3.0], 1.4, mul(cloth, 2.4)); }
            c.stain(37.0, 20.0, 9.0, 5.0, 0.0, [0.9, 0.35, 0.05], 0.7);
            // A pitch torch's burning head glows with embers; a magic one burns clean.
            if kind != FixtureKind::Torch { return Some(c.finish()); }
            for (x, y, rx) in [(31.0, 23.0, 2.6), (40.0, 22.0, 3.0), (36.0, 26.0, 2.2), (44.0, 25.0, 1.8)] { c.dab(x, y, rx, 1.2, 0.3, [1.0, 0.45, 0.1]); }
            // The burning top of the head glows.
            let mut g = Canvas::new(64, 160);
            g.ellipse(37.0, 21.0, 9.0, 4.5, [1.3; 3], 0.0);
            for (x, y, rx) in [(31.0, 23.0, 2.6), (40.0, 22.0, 3.0), (36.0, 26.0, 2.2), (44.0, 25.0, 1.8)] { g.dab(x, y, rx, 1.2, 0.3, [1.5; 3]); }
            Some(with_glow(c.finish(), g))
        }
        FixtureKind::Lantern => {
            // A glazed lantern hung by a ring from a scrolled wall bracket.
            let mut c = Canvas::new(96, 200);
            let (plate, glass) = ([0.07, 0.065, 0.06], [0.95, 0.66, 0.28]);
            c.rect(0.0, 4.0, 8.0, 44.0, plate, 0.7);
            for y in [10.0, 38.0] { c.ellipse(4.0, y, 2.0, 2.0, mul(plate, 2.0), 0.9); }
            c.rod([4.0, 14.0], [66.0, 14.0], 5.0, 4.0, plate);
            for i in 0..8 { let t = |i: i32| { let a = 0.5 * PI * i as f32 / 8.0; [8.0 + 30.0 * a.sin(), 44.0 - 28.0 * (1.0 - a.cos()) - 2.0 * a.sin()] }; c.rod(t(i), t(i + 1), 3.0, 2.8, plate); }
            for i in 0..5 { c.ellipse(64.0, 20.0 + 8.0 * i as f32, 2.6, 4.4, plate, 0.6); }
            c.ellipse(64.0, 60.0, 5.0, 5.0, plate, 0.8);
            c.poly(&[[64.0, 60.0], [92.0, 78.0], [36.0, 78.0]], plate, 0.9);
            c.rect(38.0, 77.0, 90.0, 82.0, mul(plate, 1.3), 0.7);
            c.poly(&[[44.0, 82.0], [84.0, 82.0], [80.0, 140.0], [48.0, 140.0]], glass, 0.3);
            c.poly(&[[65.0, 82.0], [84.0, 82.0], [80.0, 140.0], [64.5, 140.0]], mul(glass, 0.85), 0.2);
            c.stain(56.0, 104.0, 6.0, 24.0, 0.05, [1.0, 0.95, 0.8], 0.45);
            for (x0, x1) in [(44.0, 48.0), (84.0, 80.0), (64.5, 64.0)] { c.rod([x0, 82.0], [x1, 140.0], 3.0, 3.0, plate); }
            c.rect(44.0, 139.0, 84.0, 147.0, plate, 0.7);
            c.poly(&[[52.0, 147.0], [76.0, 147.0], [68.0, 156.0], [60.0, 156.0]], plate, 0.8);
            // The panes glow with the flame inside, brightest round it; the frame stays dark.
            let mut g = Canvas::new(96, 200);
            g.poly(&[[44.0, 82.0], [84.0, 82.0], [80.0, 140.0], [48.0, 140.0]], [0.7; 3], 0.0);
            g.haze(64.0, 112.0, 16.0, 26.0, 0.0, [1.6; 3], 0.9);
            for (x0, x1) in [(44.0, 48.0), (84.0, 80.0), (64.5, 64.0)] { g.rod([x0, 82.0], [x1, 140.0], 3.0, 3.0, [0.0; 3]); }
            Some(with_glow(c.finish(), g))
        }
        FixtureKind::Candle => {
            // A fat tallow candle on an iron pricket dish, wax run down its sides.
            let mut c = Canvas::new(48, 120);
            let (plate, wax) = ([0.07, 0.065, 0.06], [0.8, 0.74, 0.58]);
            c.rod([24.0, 120.0], [24.0, 108.0], 6.0, 5.0, plate);
            c.ellipse(24.0, 107.0, 21.0, 6.0, plate, 0.9);
            c.ellipse(24.0, 105.0, 18.0, 3.5, mul(plate, 0.6), 0.3);
            c.rod([24.0, 106.0], [24.0, 34.0], 17.0, 16.0, wax);
            for (x, y0, y1, w) in [(17.5, 36.0, 74.0, 4.0), (29.0, 34.0, 58.0, 3.4), (22.0, 35.0, 49.0, 3.0), (31.5, 60.0, 92.0, 3.0)] {
                c.rod([x, y0], [x, y1], w, w * 0.8, mul(wax, 1.08));
                c.ellipse(x, y1, w * 0.6, w * 0.8, mul(wax, 1.1), 0.6);
            }
            c.ellipse(24.0, 34.0, 8.0, 3.0, mul(wax, 1.2), 0.4);
            c.ellipse(24.0, 34.5, 4.0, 1.6, [0.95, 0.8, 0.45], 0.0);
            c.rod([24.0, 34.0], [24.6, 27.0], 1.6, 1.2, [0.03, 0.025, 0.02]);
            Some(c.finish())
        }
        FixtureKind::Brazier => {
            // A forged iron fire basket on three scrolled legs: straps round a deep bowl, the
            // coals glowing red between them and heaped over the rim, cracked with fire.
            let mut c = Canvas::new(128, 160);
            let mut g = Canvas::new(128, 160);
            let iron = [0.06, 0.055, 0.05];
            let black = [0.0; 3];
            // The back leg, then the two splayed ones, each ending in a curled foot.
            c.rod([64.0, 86.0], [64.0, 152.0], 4.5, 4.0, mul(iron, 0.6));
            for s in [-1.0f32, 1.0] {
                let p = |x: f32, y: f32| [64.0 + s * x, y];
                c.rod(p(24.0, 80.0), p(34.0, 118.0), 6.5, 5.5, iron);
                c.rod(p(34.0, 118.0), p(46.0, 150.0), 5.5, 5.0, iron);
                let f = p(50.0, 151.0);
                c.ellipse(f[0], f[1], 6.0, 5.0, iron, 0.6);
                c.cut(|c| c.ellipse(f[0] + s * 0.8, f[1] - 0.5, 2.6, 2.2, black, 0.0));
            }
            // A brace ring between the legs.
            c.ellipse(64.0, 122.0, 32.0, 5.5, iron, 0.6);
            c.cut(|c| c.ellipse(64.0, 121.0, 28.0, 3.2, black, 0.0));
            // The bowl's hot inside, seen between the straps.
            for cv in [&mut c, &mut g] {
                cv.ellipse(64.0, 40.0, 54.0, 48.0, [1.0; 3], 0.0);
                cv.cut(|c| c.rect(0.0, 0.0, 128.0, 40.0, black, 0.0));
            }
            c.ellipse(64.0, 40.0, 54.0, 48.0, [0.42, 0.07, 0.02], 0.0);
            c.cut(|c| c.rect(0.0, 0.0, 128.0, 40.0, black, 0.0));
            c.stain(64.0, 46.0, 46.0, 22.0, 0.0, [1.0, 0.42, 0.08], 0.85);
            c.stain(64.0, 44.0, 30.0, 10.0, 0.0, [1.0, 0.75, 0.3], 0.6);
            g.ellipse(64.0, 40.0, 54.0, 48.0, [0.5; 3], 0.0);
            g.cut(|c| c.rect(0.0, 0.0, 128.0, 40.0, black, 0.0));
            g.stain(64.0, 46.0, 46.0, 22.0, 0.0, [1.6; 3], 1.0);
            // Straps converging under the bowl, and two bands round it.
            for i in 0..7 {
                let t = (i as f32 - 3.0) / 3.4;
                let (a, b) = ([64.0 + 54.0 * t, 40.0], [64.0 + 14.0 * t, 86.0]);
                c.rod(a, b, 5.5, 4.5, iron);
                g.rod(a, b, 5.5, 4.5, black);
            }
            for (rx, ry, y, w) in [(47.0, 25.0, 40.0, 4.0), (24.0, 4.0, 82.0, 5.0)] {
                for j in 0..14 {
                    let (a0, a1) = (PI * j as f32 / 14.0, PI * (j + 1) as f32 / 14.0);
                    let (p0, p1) = ([64.0 + rx * a0.cos(), y + ry * a0.sin()], [64.0 + rx * a1.cos(), y + ry * a1.sin()]);
                    c.rod(p0, p1, w, w, mul(iron, 1.2));
                    g.rod(p0, p1, w, w, black);
                }
            }
            // The coals heaped above the rim: dark lumps, cracked with fire.
            let coal = [0.05, 0.04, 0.035];
            for (x, y, rx, ry) in [(34.0, 37.0, 13.0, 7.0), (52.0, 33.0, 14.0, 8.0), (72.0, 32.0, 15.0, 8.0), (92.0, 36.0, 13.0, 7.0), (62.0, 38.0, 12.0, 5.0), (44.0, 39.0, 10.0, 4.0), (82.0, 39.0, 10.0, 4.0)] {
                c.ellipse(x, y, rx, ry, coal, 0.8);
                g.ellipse(x, y, rx, ry, black, 0.0);
            }
            for (x, y, rx, ry, a) in [(40.0, 35.0, 7.0, 1.4, 0.3), (55.0, 31.0, 8.0, 1.6, -0.2), (70.0, 30.0, 6.0, 1.3, 0.5), (86.0, 34.0, 7.0, 1.4, -0.4), (63.0, 36.5, 9.0, 1.5, 0.1), (48.0, 38.0, 5.0, 1.2, 0.0), (78.0, 37.0, 6.0, 1.3, 0.2)] {
                c.dab(x, y, rx, ry, a, [1.0, 0.5, 0.12]);
                g.dab(x, y, rx, ry, a, [1.8; 3]);
            }
            // The rim's near half over the coals' foot, riveted.
            for j in 0..16 {
                let (a0, a1) = (PI * j as f32 / 16.0, PI * (j + 1) as f32 / 16.0);
                let (p0, p1) = ([64.0 + 57.0 * a0.cos(), 40.0 + 8.0 * a0.sin()], [64.0 + 57.0 * a1.cos(), 40.0 + 8.0 * a1.sin()]);
                c.rod(p0, p1, 6.0, 6.0, mul(iron, 1.6));
                g.rod(p0, p1, 6.0, 6.0, black);
            }
            for j in 1..8 { let a = PI * j as f32 / 8.0; c.ellipse(64.0 + 57.0 * a.cos(), 40.0 + 8.0 * a.sin(), 1.6, 1.6, mul(iron, 3.0), 0.9); }
            Some(with_glow(c.finish(), g))
        }
        FixtureKind::Crystal => Some(paint_prop(PropKind::Crystal, [140, 210, 255], 3)),
        FixtureKind::Firefly => None,
        FixtureKind::StreetLamp => {
            // A glazed lamp head: a peaked roof with a finial, four panes narrowing downwards, a neck to stand on.
            let (iron, glass) = ([0.085, 0.08, 0.075], [0.95, 0.8, 0.5]);
            let mut c = Canvas::new(56, 120);
            c.ellipse(28.0, 5.0, 3.0, 4.0, iron, 0.8);
            c.poly(&[[28.0, 8.0], [52.0, 30.0], [4.0, 30.0]], iron, 0.9);
            c.rect(6.0, 29.0, 50.0, 34.0, iron, 0.7);
            c.poly(&[[10.0, 34.0], [46.0, 34.0], [40.0, 92.0], [16.0, 92.0]], glass, 0.3);
            // (The pane to the right is turned from the light; the glow inside still fills it.)
            c.poly(&[[30.0, 34.0], [46.0, 34.0], [40.0, 92.0], [29.0, 92.0]], mul(glass, 0.86), 0.2);
            for (x0, x1) in [(10.0, 16.0), (46.0, 40.0), (29.0, 28.5)] { c.rod([x0, 34.0], [x1, 92.0], 3.0, 3.0, iron); }
            c.rect(13.0, 91.0, 43.0, 98.0, iron, 0.7);
            c.poly(&[[18.0, 98.0], [38.0, 98.0], [33.0, 112.0], [23.0, 112.0]], iron, 0.8);
            c.rect(22.0, 112.0, 34.0, 120.0, iron, 0.7);
            // A mantle burning bright inside the panes; the frame and roof stay iron.
            c.ellipse(28.0, 62.0, 5.0, 8.0, [1.0, 0.95, 0.85], 0.0);
            let mut g = Canvas::new(56, 120);
            g.poly(&[[10.0, 34.0], [46.0, 34.0], [40.0, 92.0], [16.0, 92.0]], [0.8; 3], 0.0);
            g.haze(28.0, 62.0, 13.0, 22.0, 0.0, [1.8; 3], 0.9);
            for (x0, x1) in [(10.0, 16.0), (46.0, 40.0), (29.0, 28.5)] { g.rod([x0, 34.0], [x1, 92.0], 3.0, 3.0, [0.0; 3]); }
            Some(with_glow(c.finish(), g))
        }
        FixtureKind::PaperLantern => {
            // A round paper lantern on its string: ribs showing through the paper, a tassel beneath.
            let (paper, wood, gold) = ([0.9, 0.17, 0.08], [0.07, 0.04, 0.03], [0.8, 0.58, 0.16]);
            let mut c = Canvas::new(64, 108);
            c.rect(31.0, 0.0, 33.0, 10.0, wood, 0.0);
            c.ellipse(32.0, 48.0, 28.0, 35.0, paper, 0.45);
            for i in 0..7 {
                let y = 21.0 + 9.0 * i as f32;
                let half = 28.0 * (1.0 - ((y - 48.0) / 35.0f32).powi(2)).max(0.0).sqrt();
                c.rect(32.0 - half, y, 32.0 + half, y + 1.4, mul(paper, 0.7), 0.3);
            }
            c.rect(20.0, 9.0, 44.0, 16.0, wood, 0.6);
            c.rect(22.0, 80.0, 42.0, 86.0, wood, 0.6);
            c.rect(30.5, 86.0, 33.5, 92.0, gold, 0.3);
            c.poly(&[[29.0, 92.0], [35.0, 92.0], [37.0, 108.0], [27.0, 108.0]], gold, 0.5);
            Some(c.finish())
        }
        FixtureKind::Campfire => {
            // Logs leaning together in a ring of stones, embers in their lap.
            let (stone, log) = ([0.2, 0.19, 0.18], [0.1, 0.055, 0.03]);
            let mut c = Canvas::new(128, 60);
            for (x, y, rx, ry) in [(30.0, 40.0, 11.0, 8.0), (64.0, 37.0, 12.0, 8.0), (98.0, 40.0, 11.0, 8.0)] { c.ellipse(x, y, rx, ry, mul(stone, 0.7), 0.8); }
            c.limb([18.0, 46.0], [92.0, 16.0], 13.0, 10.0, log, 1.0);
            c.limb([110.0, 46.0], [38.0, 14.0], 13.0, 10.0, log, 2.3);
            c.ellipse(64.0, 34.0, 20.0, 7.0, [1.0, 0.32, 0.05], 0.2);
            c.ellipse(64.0, 33.0, 10.0, 3.5, [1.0, 0.7, 0.2], 0.0);
            c.limb([64.0, 52.0], [66.0, 10.0], 11.0, 8.0, mul(log, 0.85), 4.1);
            // Embers glowing in the logs' lap and along their charred undersides.
            for (x, y, rx, a) in [(50.0, 36.0, 5.0, 0.4), (78.0, 35.0, 5.0, -0.4), (64.0, 40.0, 7.0, 0.0), (40.0, 41.0, 4.0, 0.3), (88.0, 41.0, 4.0, -0.3)] { c.dab(x, y, rx, 1.6, a, [1.0, 0.5, 0.12]); }
            let mut g = Canvas::new(128, 60);
            g.ellipse(64.0, 34.0, 20.0, 7.0, [1.5; 3], 0.0);
            g.limb([18.0, 46.0], [92.0, 16.0], 13.0, 10.0, [0.0; 3], 1.0);
            g.limb([110.0, 46.0], [38.0, 14.0], 13.0, 10.0, [0.0; 3], 2.3);
            g.limb([64.0, 52.0], [66.0, 10.0], 11.0, 8.0, [0.0; 3], 4.1);
            for (x, y, rx, a) in [(50.0, 36.0, 5.0, 0.4), (78.0, 35.0, 5.0, -0.4), (64.0, 40.0, 7.0, 0.0), (40.0, 41.0, 4.0, 0.3), (88.0, 41.0, 4.0, -0.3)] { g.dab(x, y, rx, 1.6, a, [1.6; 3]); }
            for (x, y, rx, ry) in [(12.0, 50.0, 12.0, 9.0), (38.0, 53.0, 13.0, 7.0), (66.0, 54.0, 12.0, 6.0), (92.0, 53.0, 13.0, 7.0), (116.0, 50.0, 12.0, 9.0)] { c.ellipse(x, y, rx, ry, stone, 0.9); g.ellipse(x, y, rx, ry, [0.0; 3], 0.0); }
            Some(with_glow(c.finish(), g))
        }
        FixtureKind::Glowshroom => {
            // Three caps of different ages on pale stems, spotted, lit from within.
            let (cap, stem) = ([0.22, 0.9, 0.74], [0.55, 0.78, 0.7]);
            let mut c = Canvas::new(96, 80);
            for (cx, cy, rx, ry, lean) in [(30.0, 26.0, 24.0, 15.0, -4.0), (68.0, 42.0, 19.0, 12.0, 5.0), (47.0, 58.0, 12.0, 8.0, 1.0)] {
                c.ellipse(cx, cy, rx, ry, cap, 0.55);
                c.cut(|c| c.rect(cx - rx - 1.0, cy + 0.3 * ry, cx + rx + 1.0, cy + ry + 2.0, [0.0; 3], 0.0));
                c.rect(cx - 0.82 * rx, cy + 0.3 * ry - 1.5, cx + 0.82 * rx, cy + 0.3 * ry + 1.0, mul(cap, 0.55), 0.2);
                c.rod([cx, cy + 0.3 * ry], [cx + lean, 80.0], 0.26 * rx, 0.36 * rx, stem);
                for (dx, dy, r) in [(-0.45, -0.25, 0.13), (0.2, -0.5, 0.1), (0.5, -0.1, 0.09)] { c.ellipse(cx + dx * rx, cy + dy * ry, r * rx, r * ry, [0.8, 1.0, 0.93], 0.0); }
            }
            Some(c.finish())
        }
        FixtureKind::Beacon => {
            // A lens under a wire cage on a squat housing.
            let (metal, lens) = ([0.13, 0.13, 0.14], [0.95, 0.12, 0.08]);
            let mut c = Canvas::new(48, 72);
            c.ellipse(24.0, 28.0, 15.0, 19.0, lens, 0.4);
            c.ellipse(19.0, 20.0, 4.0, 6.0, [1.0, 0.55, 0.45], 0.0);
            for x in [12.0, 24.0, 36.0] { c.rod([x, 44.0], [24.0 + (x - 24.0) * 0.5, 8.0], 1.6, 1.6, metal); }
            c.rect(9.0, 26.0, 39.0, 28.0, metal, 0.3);
            c.rect(4.0, 42.0, 44.0, 48.0, metal, 0.7);
            c.poly(&[[8.0, 48.0], [40.0, 48.0], [44.0, 72.0], [4.0, 72.0]], metal, 0.9);
            Some(c.finish())
        }
        FixtureKind::Neon => Some(paint_neon()),
        FixtureKind::Chandelier => {
            // An iron ring on three chains from one above, candles standing round it.
            let (iron, wax) = ([0.07, 0.065, 0.06], [0.8, 0.74, 0.6]);
            let mut c = Canvas::new(160, 150);
            for i in 0..7 { c.ellipse(80.0, 4.0 + 8.0 * i as f32, 2.6, 4.6, iron, 0.5); }
            c.ellipse(80.0, 60.0, 7.0, 6.0, iron, 0.8);
            for x in [14.0, 80.0, 146.0] { c.rod([80.0, 62.0], [x, 100.0], 2.4, 2.4, iron); }
            // The far side of the ring, its candles, then the near side over their feet.
            c.ellipse(80.0, 100.0, 72.0, 13.0, mul(iron, 0.7), 0.4);
            c.cut(|c| c.ellipse(80.0, 102.0, 66.0, 9.0, [0.0; 3], 0.0));
            for (x, y) in CHANDELIER_CANDLES { c.rod([x, y + 26.0], [x, y + 3.0], 7.0, 6.5, wax); c.ellipse(x, y + 26.0, 7.0, 3.0, iron, 0.6); }
            c.poly(&[[8.0, 101.0], [152.0, 101.0], [140.0, 110.0], [118.0, 114.5], [80.0, 116.0], [42.0, 114.5], [20.0, 110.0]], iron, 0.8);
            c.cut(|c| c.ellipse(80.0, 100.0, 70.0, 10.5, [0.0; 3], 0.0));
            for (x, y) in CHANDELIER_CANDLES { if y > 78.0 { c.rod([x, y + 26.0], [x, y + 3.0], 7.0, 6.5, wax); c.ellipse(x, y + 26.0, 7.0, 3.0, iron, 0.6); } }
            Some(c.finish())
        }
    }
}

/// A rock face 1 wide by 1.25 tall (the waterfall's height is its top): a jagged crest, ledges
/// banded across it, moss on the ledges, and a dark wet channel down its middle.
fn paint_cliff(tint: Rgb, variant: u32) -> Sprite {
    let t = rgb_lin(tint);
    let mut r = Rng(variant.wrapping_mul(2_654_435_761).wrapping_add(0xC1) | 1);
    let (w, h) = (200.0, 250.0);
    let mut c = Canvas::new(w as usize, h as usize);
    // The outline: shoulders either side of the notch the water pours through, falling away
    // raggedly to each side.
    let mut top = Vec::new();
    for i in 0..=20 {
        let x = i as f32 * w / 20.0;
        let off = (x - 100.0).abs();
        let y = if off <= 30.0 { 18.0 + r.range(-1.5, 1.5) }
            else if off <= 70.0 { 4.0 + r.range(0.0, 10.0) + 6.0 * ((off - 30.0) / 40.0 - 0.5).abs() }
            else { 12.0 + (off - 70.0).powf(1.25) * r.range(1.6, 2.6) };
        top.push([x, y.min(h - 30.0)]);
    }
    let top_at = |x: f32| -> f32 {
        let i = ((x / w * 20.0).floor() as usize).min(19);
        let f = x / w * 20.0 - i as f32;
        top[i][1] + (top[i + 1][1] - top[i][1]) * f
    };
    let mut pts = vec![[0.0, h]];
    pts.extend(top.iter().copied());
    pts.push([w, h]);
    c.poly(&pts, mul(t, 0.55), 0.2);
    // Broken slabs and blocks, of no regular size, each lit on its top and left and shadowed
    // under it, with darker cracks between.
    for _ in 0..120 {
        let (cx, cy) = (r.range(-10.0, w + 10.0), r.range(0.0, h));
        let (bw, bh) = (r.range(14.0, 46.0), r.range(10.0, 30.0));
        let (x0, x1, y0, y1) = (cx - 0.5 * bw, cx + 0.5 * bw, cy - 0.5 * bh, cy + 0.5 * bh);
        let ty = top_at(cx.clamp(0.0, w));
        if y1 < ty + 6.0 { continue; }
        let y0 = y0.max(ty + 2.0);
        let k = r.range(0.7, 1.2);
        let face = [t[0] * k, t[1] * k * r.range(0.96, 1.03), t[2] * k * r.range(0.95, 1.04)];
        let sk = r.range(-5.0, 5.0);
        let q = [[x0 + sk + r.range(0.0, 6.0), y0], [x1 + sk - r.range(0.0, 6.0), y0 + r.range(-2.0, 3.0)], [x1 + r.range(-2.0, 2.0), 0.5 * (y0 + y1)], [x1 - r.range(0.0, 5.0), y1], [x0 + r.range(0.0, 5.0), y1 + r.range(-2.0, 1.0)], [x0 + r.range(-2.0, 2.0), 0.5 * (y0 + y1)]];
        c.poly(&q, face, 0.5);
        c.line(q[0], q[1], 1.8, mul(face, 1.4));
        c.line(q[3], q[4], 2.0, mul(t, 0.3));
        if r.f() < 0.5 { c.stain(cx + r.range(-4.0, 4.0), y0 + 1.5, bw * r.range(0.18, 0.4), 2.5, 0.0, [0.08 * t[0] + 0.04, 0.13 + 0.1 * t[1], 0.035], 0.7); }
        if r.f() < 0.3 { let x = r.range(x0 + 3.0, x1 - 3.0); c.line([x, y0 + 2.0], [x + r.range(-5.0, 5.0), y1 - 2.0], 0.9, mul(t, 0.28)); }
    }
    // Grass and ferns hanging over the top, and a few down the face.
    for _ in 0..70 {
        let x = r.range(0.0, w);
        if (72.0..128.0).contains(&x) { continue; }
        let ty = top_at(x);
        let g = [0.05 + r.range(0.0, 0.06), 0.14 + r.range(0.0, 0.12), 0.03];
        let len = r.range(5.0, 16.0);
        let lean = r.range(-0.8, 0.8);
        c.line([x, ty + 2.0], [x + lean * len, ty - len * 0.7], 1.6, g);
        if r.f() < 0.3 { c.line([x, ty + 1.0], [x + lean * 6.0, ty + len], 1.2, mul(g, 0.8)); }
    }
    for _ in 0..10 { let (x, yy) = (r.range(8.0, 192.0), r.range(40.0, 230.0)); if !(66.0..134.0).contains(&x) { c.stain(x, yy, r.range(5.0, 11.0), r.range(3.0, 6.0), 0.0, [0.06, 0.16, 0.04], 0.7); } }
    // The channel the water has worn: darker, wet, and smooth.
    c.stain(100.0, 140.0, 34.0, 130.0, 0.0, mul(t, 0.22), 0.9);
    c.stain(100.0, 140.0, 26.0, 125.0, 0.0, [0.02, 0.025, 0.03], 0.6);
    for i in 0..6 { c.stain(84.0 + i as f32 * 6.0, 120.0 + r.range(-20.0, 20.0), 2.0, 70.0, 0.0, mul(t, 0.9), 0.35); }
    c.finish()
}

/// A neon sign: tubes bent into a border and a cocktail glass with an olive, on a dark board
/// bolted out from the wall. The tubes are white (the fixture's colour tints them) and glow;
/// the board and its brackets do not.
fn paint_neon() -> Sprite {
    let mut c = Canvas::new(160, 144);
    let mut g = Canvas::new(160, 144);
    let (board, iron, tube) = ([0.03, 0.03, 0.035], [0.08, 0.08, 0.085], [1.0, 1.0, 1.0]);
    c.rect(6.0, 8.0, 154.0, 136.0, board, 0.3);
    for (x, y) in [(14.0, 16.0), (146.0, 16.0), (14.0, 128.0), (146.0, 128.0)] { c.ellipse(x, y, 2.4, 2.4, iron, 0.8); }
    // The tubes: a glass, its stem and foot, a stick with an olive, and a border round it all.
    let tubes: Vec<([f32; 2], [f32; 2])> = vec![
        ([18.0, 22.0], [142.0, 22.0]), ([142.0, 22.0], [142.0, 122.0]), ([142.0, 122.0], [18.0, 122.0]), ([18.0, 122.0], [18.0, 22.0]),
        ([46.0, 36.0], [114.0, 36.0]), ([46.0, 36.0], [80.0, 74.0]), ([114.0, 36.0], [80.0, 74.0]),
        ([80.0, 74.0], [80.0, 104.0]), ([62.0, 108.0], [98.0, 108.0]),
        ([92.0, 30.0], [70.0, 58.0]),
    ];
    for (a, b) in &tubes { c.line(*a, *b, 3.4, tube); g.line(*a, *b, 3.4, [2.6; 3]); }
    for (a, b) in &tubes { g.line(*a, *b, 9.0, [0.35; 3]); }
    c.ellipse(76.0, 52.0, 5.0, 5.0, tube, 0.0);
    g.ellipse(76.0, 52.0, 5.0, 5.0, [2.6; 3], 0.0);
    c.rect(0.0, 66.0, 6.0, 78.0, iron, 0.6);
    with_glow(c.finish(), g)
}

/// Where the chandelier's candle tops are in its 160 x 150 painting.
const CHANDELIER_CANDLES: [(f32, f32); 5] = [(16.0, 72.0), (50.0, 66.0), (110.0, 66.0), (144.0, 72.0), (80.0, 84.0)];
/// The chandelier's flames, as offsets from the middle of its body in body widths and heights
/// (right and up positive).
pub fn chandelier_flames() -> [[f32; 2]; 5] {
    CHANDELIER_CANDLES.map(|(x, y)| [x / 160.0 - 0.5, (CHANDELIER_FLAME_Y - y) / 150.0])
}
const CHANDELIER_FLAME_Y: f32 = 72.0;

/// Where a crook's arm ends, in its painting: across (from the left, the post standing on the
/// right) and down from the top, as fractions.
pub const CROOK_HOOK: [f32; 2] = [22.0 / 120.0, 70.0 / 480.0];

/// What a light stands on or hangs from: a pole, a crook (its arm reaching left), or a pedestal.
pub fn paint_post(style: LampPost) -> Sprite {
    let iron = [0.04, 0.037, 0.034];
    match style {
        LampPost::Crook => {
            let mut c = Canvas::new(120, 480);
            let px = 98.0;
            // The arm: up out of the post, over, and down to a ring.
            let arc = |a: f32| [60.0 + 38.0 * a.cos(), 64.0 - 38.0 * a.sin()];
            for i in 0..16 {
                let (a0, a1) = (PI * i as f32 / 16.0, PI * (i + 1) as f32 / 16.0);
                let w = 6.5 - 2.5 * i as f32 / 16.0;
                c.rod(arc(a0), arc(a1), w, w - 0.15, iron);
            }
            // A scroll bracing it.
            for i in 0..10 {
                let t = |i: i32| { let a = 0.5 * PI * i as f32 / 10.0; [px - 30.0 * a.sin(), 122.0 - 44.0 * (1.0 - a.cos()) - 12.0 * a.sin()] };
                c.rod(t(i), t(i + 1), 3.2, 3.0, iron);
            }
            c.ellipse(22.0, 67.0, 3.6, 3.6, iron, 0.6);
            c.rod([px, 446.0], [px, 62.0], 9.0, 6.5, iron);
            c.rect(px - 7.0, 196.0, px + 7.0, 203.0, iron, 0.8);
            c.poly(&[[px - 6.0, 440.0], [px + 6.0, 440.0], [px + 10.0, 464.0], [px - 10.0, 464.0]], iron, 0.9);
            c.rect(px - 13.0, 463.0, px + 13.0, 480.0, iron, 0.8);
            c.finish()
        }
        LampPost::Pedestal => paint_prop(PropKind::Pillar, [36, 32, 30], 0),
        _ => {
            let mut c = Canvas::new(28, 480);
            c.rod([14.0, 446.0], [14.0, 4.0], 9.0, 6.5, iron);
            c.ellipse(14.0, 5.0, 5.0, 4.5, iron, 0.8);
            c.rect(7.0, 250.0, 21.0, 257.0, iron, 0.8);
            c.poly(&[[8.0, 440.0], [20.0, 440.0], [24.0, 464.0], [4.0, 464.0]], iron, 0.9);
            c.rect(1.0, 463.0, 27.0, 480.0, iron, 0.8);
            c.finish()
        }
    }
}

/// Fixture body size: (height in metres at size 1, where the flame sits as a fraction from the top).
pub fn fixture_body(kind: FixtureKind) -> (f32, f32) {
    match kind {
        FixtureKind::Torch | FixtureKind::GreenFire | FixtureKind::Magic | FixtureKind::IceWisp => (0.65, 0.12),
        FixtureKind::Lantern => (0.8, 0.55),
        FixtureKind::Candle => (0.3, 0.2),
        FixtureKind::Brazier => (1.0, 0.17),
        FixtureKind::Crystal => (0.9, 0.3),
        FixtureKind::Firefly => (0.0, 0.0),
        FixtureKind::StreetLamp => (0.62, 0.52),
        FixtureKind::PaperLantern => (0.5, 0.45),
        FixtureKind::Campfire => (0.32, 0.25),
        FixtureKind::Glowshroom => (0.4, 0.4),
        FixtureKind::Beacon => (0.32, 0.39),
        FixtureKind::Chandelier => (0.75, CHANDELIER_FLAME_Y / 150.0),
        FixtureKind::Neon => (0.9, 0.5),
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
            // The ring is a half round: it springs from the piers and never runs down over them.
            let half: Vec<[f32; 2]> = (0..=48).map(|i| { let a = std::f32::consts::PI * i as f32 / 48.0; [cx + a.cos() * m(rad + ring), cy - a.sin() * m(rad + ring)] }).collect();
            c.poly(&half, mul(t, 1.08), 0.15);
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

/// The picture of a prop's face (`PropFace::shape`): its parts painted over `width` by `height`
/// metres, `x` from the left edge and `y` up from the bottom edge.
pub fn paint_face(parts: &[ShapePart], width: f32, height: f32) -> Option<Sprite> {
    if !(width > 1e-3 && height > 1e-3) || parts.is_empty() { return None; }
    let ppm = (128.0 / width.max(height)).clamp(4.0, 400.0);
    let (w, h) = (((width * ppm).round() as usize).max(2), ((height * ppm).round() as usize).max(2));
    // (Stretched a hair so the picture fills its pixels exactly.)
    let (kx, ky) = (w as f32 / width, h as f32 / height);
    let at = |q: [f32; 2]| [q[0] * kx, (height - q[1]) * ky];
    let paint = |c: &mut Canvas, p: &ShapePart, col: [f32; 3], shade: f32| {
        let draw = |c: &mut Canvas| match &p.shape {
            Shape::Ellipse { center, radius } => { let q = at(*center); c.ellipse(q[0], q[1], radius[0].abs() * kx, radius[1].abs() * ky, col, shade) }
            Shape::Rect { min, max } => { let (a, b) = (at(*min), at(*max)); c.rect(a[0], a[1], b[0], b[1], col, shade) }
            Shape::Poly { points } => { let pts: Vec<[f32; 2]> = points.iter().map(|q| at(*q)).collect(); if pts.len() >= 3 { c.poly(&pts, col, shade) } }
            Shape::Line { from, to, width } => c.line(at(*from), at(*to), width.abs() * ky, col),
        };
        if p.cut { c.cut(draw) } else { draw(c) }
    };
    let mut c = Canvas::new(w, h);
    for p in parts { paint(&mut c, p, rgb_lin(p.color), p.shade.clamp(0.0, 1.0)); }
    let mut sprite = c.finish();
    if parts.iter().any(|p| p.glow > 0.0 && !p.cut) {
        let mut g = Canvas::new(w, h);
        for p in parts { let k = p.glow.max(0.0); paint(&mut g, p, [k, k, k], 0.0); }
        sprite.glow = Some(Arc::new(g.finish()));
    }
    Some(sprite)
}

/// Sprites by key: procedural variants and loaded image files.
#[derive(Default)]
pub struct SpriteCache {
    procedural: HashMap<(u8, Rgb, u32), Arc<Sprite>>,
    /// Set-pieces by kind, colours, opening size in centimetres and variant; with their size in metres.
    pieces: HashMap<(u8, Rgb, Rgb, u32, u32, u32), (Arc<Sprite>, f32, f32)>,
    fixtures: HashMap<u8, Option<Arc<Sprite>>>,
    posts: HashMap<u8, Arc<Sprite>>,
    /// By path and trim, with the file's modification time when it was read.
    files: HashMap<(PathBuf, bool), (Option<std::time::SystemTime>, Option<Arc<Sprite>>)>,
    /// Images with a glow layer from their lightest texels, with the image they were made from.
    glowing: HashMap<(PathBuf, bool, u32, u32), (Arc<Sprite>, Arc<Sprite>)>,
    /// Drawn props by the text of their parts, with their height in metres.
    shapes: HashMap<String, Option<(Arc<Sprite>, f32)>>,
}

pub const VARIANTS: u32 = 6;

impl SpriteCache {
    pub fn prop(&mut self, kind: PropKind, tint: Rgb, variant: u32) -> Arc<Sprite> {
        let key = (kind as u8, tint, variant % VARIANTS);
        if self.procedural.len() > 256 { self.procedural.clear(); }
        self.procedural.entry(key).or_insert_with(|| Arc::new(paint_prop(kind, tint, variant % VARIANTS))).clone()
    }

    /// A rock face for a waterfall to pour down: craggy, banded, mossy, darker and wet down its middle.
    pub fn cliff(&mut self, tint: Rgb, variant: u32) -> Arc<Sprite> {
        let key = (0xC1, tint, variant % VARIANTS);
        if self.procedural.len() > 256 { self.procedural.clear(); }
        self.procedural.entry(key).or_insert_with(|| Arc::new(paint_cliff(tint, variant % VARIANTS))).clone()
    }

    /// A whole rock for one floating in space (`PropLayer.float`): the ground rocks are domes with
/// a flat base, which reads as a cut edge in mid-air.
pub fn asteroid(&mut self, tint: Rgb, variant: u32) -> Arc<Sprite> {
    let key = (0xA5, tint, variant % VARIANTS);
    if self.procedural.len() > 256 { self.procedural.clear(); }
    self.procedural.entry(key).or_insert_with(|| Arc::new(paint_asteroid(tint, variant % VARIANTS))).clone()
}

pub fn set_piece(&mut self, kind: SetPieceKind, tint: Rgb, accent: Rgb, ow: f32, oh: f32, variant: u32) -> (Arc<Sprite>, f32, f32) {
        let key = (kind as u8, tint, accent, (ow * 10.0).round() as u32, (oh * 10.0).round() as u32, variant % VARIANTS);
        if self.pieces.len() > 64 { self.pieces.clear(); }
        self.pieces.entry(key).or_insert_with(|| {
            let (s, w, h) = paint_set_piece(kind, tint, accent, key.3 as f32 / 10.0, key.4 as f32 / 10.0, variant % VARIANTS);
            (Arc::new(s), w, h)
        }).clone()
    }

    pub fn post(&mut self, style: LampPost) -> Arc<Sprite> {
        self.posts.entry(style as u8).or_insert_with(|| Arc::new(paint_post(style))).clone()
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
        // (The later of the image's time and its mask's: painting a mask redraws the image.)
        let stamp = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
        let mtime = stamp(full).max(stamp(&mask_path(full)).map(|t| t + std::time::Duration::from_nanos(1)));
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
        // (A painted mask says what glows; the lightness rule is for images without one.)
        if base.casts { return Some(base); }
        let mut s = Sprite::from_rgba_lin(*w, *h, base.levels[0].2.clone());
        s.glow = Some(Arc::new(Sprite::from_rgba_lin(*w, *h, mask)));
        let s = Arc::new(s);
        if self.glowing.len() > 64 { self.glowing.clear(); }
        self.glowing.insert(key, (base, s.clone()));
        Some(s)
    }

    /// A prop face's picture (`paint_face`), kept by its parts and size.
    pub fn face(&mut self, parts: &[ShapePart], width: f32, height: f32) -> Option<Arc<Sprite>> {
        let key = format!("{width}x{height}:{}", serde_json::to_string(parts).unwrap_or_default());
        if self.shapes.len() > 512 { self.shapes.clear(); }
        self.shapes.entry(key).or_insert_with(|| paint_face(parts, width, height).map(|s| (Arc::new(s), height))).clone().map(|(s, _)| s)
    }

    pub fn shape(&mut self, parts: &[ShapePart]) -> Option<(Arc<Sprite>, f32)> {
        let key = serde_json::to_string(parts).unwrap_or_default();
        if self.shapes.len() > 512 { self.shapes.clear(); }
        self.shapes.entry(key).or_insert_with(|| paint_shape(parts).map(|(s, h)| (Arc::new(s), h))).clone()
    }
}
