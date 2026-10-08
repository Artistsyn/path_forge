//! Futuristic floor and wall patterns: metal plating, a glowing grid, hex plates and circuit
//! traces. Each returns the tile's colours (sRGB bytes, like `tiles::gen_tile`) and how much each
//! texel glows, 0..1: the parts painted in the `mortar` colour are its lights, and the material's
//! `glow` sets how bright they shine (0: they are only inlays).

use crate::scene::Pattern;

pub const SIZE: usize = 192;

/// Colours and glow of one tile, or None for a pattern this module does not draw.
pub fn gen(pattern: Pattern, base: [u8; 3], mortar: [u8; 3], noise: u32, damage: f32, seed: u32) -> Option<(Vec<u8>, Vec<f32>)> {
    let mut t = Tile::new(base);
    match pattern {
        Pattern::Panels => panels(&mut t, base, mortar, seed),
        Pattern::Grid => grid(&mut t, base, mortar),
        Pattern::Hex => hex(&mut t, base, mortar, seed),
        Pattern::Circuit => circuit(&mut t, base, mortar, seed),
        _ => return None,
    }
    t.grain(noise, seed);
    t.scuff(damage, seed);
    Some(t.finish())
}

pub fn is_scifi(p: Pattern) -> bool { matches!(p, Pattern::Panels | Pattern::Grid | Pattern::Hex | Pattern::Circuit) }

struct Tile { col: Vec<[f32; 3]>, glow: Vec<f32> }

fn h(x: u32) -> u32 {
    let mut x = x.wrapping_mul(0x9E37_79B1) ^ 0x85EB_CA6B;
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    x = x.wrapping_mul(0x297A_2D39);
    x ^ (x >> 15)
}
fn hf(seed: u32, k: u32) -> f32 { (h(seed.wrapping_mul(0x632B_E5AB) ^ k) >> 8) as f32 / 16_777_216.0 }
fn f3(c: [u8; 3]) -> [f32; 3] { [c[0] as f32, c[1] as f32, c[2] as f32] }
fn sc(c: [f32; 3], k: f32) -> [f32; 3] { [c[0] * k, c[1] * k, c[2] * k] }
fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] { [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t] }
fn wrap(v: i32) -> usize { v.rem_euclid(SIZE as i32) as usize }
/// Coverage of a pixel at distance d from an edge of a shape (positive inside): one pixel of AA.
fn cover(d: f32) -> f32 { (d + 0.5).clamp(0.0, 1.0) }

impl Tile {
    fn new(base: [u8; 3]) -> Tile { Tile { col: vec![f3(base); SIZE * SIZE], glow: vec![0.0; SIZE * SIZE] } }
    fn at(&mut self, x: i32, y: i32) -> usize { wrap(y) * SIZE + wrap(x) }
    /// Paint `c` over a pixel with coverage a, and raise its glow to `g * a`.
    fn put(&mut self, x: i32, y: i32, c: [f32; 3], a: f32, g: f32) {
        if a <= 0.0 { return; }
        let i = self.at(x, y);
        self.col[i] = mix(self.col[i], c, a.min(1.0));
        self.glow[i] = self.glow[i].max(g * a.min(1.0));
    }
    /// A line from a to b, `w` pixels wide, wrapping round the tile.
    fn line(&mut self, a: [f32; 2], b: [f32; 2], w: f32, c: [f32; 3], g: f32) {
        let (x0, x1) = (a[0].min(b[0]) - w, a[0].max(b[0]) + w);
        let (y0, y1) = (a[1].min(b[1]) - w, a[1].max(b[1]) + w);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let l2 = (dx * dx + dy * dy).max(1e-6);
        for y in y0.floor() as i32..=y1.ceil() as i32 {
            for x in x0.floor() as i32..=x1.ceil() as i32 {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let t = (((px - a[0]) * dx + (py - a[1]) * dy) / l2).clamp(0.0, 1.0);
                let (ex, ey) = (px - a[0] - t * dx, py - a[1] - t * dy);
                let d = (ex * ex + ey * ey).sqrt();
                self.put(x, y, c, cover(w * 0.5 - d), g);
            }
        }
    }
    fn disc(&mut self, cx: f32, cy: f32, r: f32, c: [f32; 3], g: f32) {
        for y in (cy - r - 1.0).floor() as i32..=(cy + r + 1.0).ceil() as i32 {
            for x in (cx - r - 1.0).floor() as i32..=(cx + r + 1.0).ceil() as i32 {
                let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
                self.put(x, y, c, cover(r - d), g);
            }
        }
    }
    fn rect(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, c: [f32; 3], g: f32) {
        for y in x_range(y0, y1) {
            for x in x_range(x0, x1) {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let d = (px - x0).min(x1 - px).min(py - y0).min(y1 - py);
                self.put(x, y, c, cover(d), g);
            }
        }
    }
    /// Fine texture: per-pixel grain, and a little in every row (brushed metal).
    fn grain(&mut self, noise: u32, seed: u32) {
        if noise == 0 { return; }
        let k = noise as f32;
        for y in 0..SIZE {
            let row = (hf(seed ^ 0xB1, y as u32) - 0.5) * k * 0.6;
            for x in 0..SIZE {
                let i = y * SIZE + x;
                if self.glow[i] > 0.5 { continue; }
                let n = (hf(seed ^ 0xB2, i as u32) - 0.5) * k * 0.5 + row;
                for c in &mut self.col[i] { *c += n; }
            }
        }
    }
    /// Scuffs and dark stains.
    fn scuff(&mut self, damage: f32, seed: u32) {
        let n = (damage.clamp(0.0, 1.0) * 14.0) as u32;
        for k in 0..n {
            let (cx, cy) = (hf(seed ^ 0xD1, k) * SIZE as f32, hf(seed ^ 0xD2, k) * SIZE as f32);
            let r = 4.0 + hf(seed ^ 0xD3, k) * 14.0;
            for y in (cy - r) as i32..=(cy + r) as i32 {
                for x in (cx - r) as i32..=(cx + r) as i32 {
                    let d = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt() / r;
                    if d >= 1.0 { continue; }
                    let i = self.at(x, y);
                    let a = 0.22 * (1.0 - d * d);
                    self.col[i] = sc(self.col[i], 1.0 - a);
                    self.glow[i] *= 1.0 - 0.6 * a;
                }
            }
        }
    }
    fn finish(self) -> (Vec<u8>, Vec<f32>) {
        let rgb = self.col.iter().flat_map(|c| c.map(|v| v.round().clamp(0.0, 255.0) as u8)).collect();
        (rgb, self.glow)
    }
}

fn x_range(a: f32, b: f32) -> std::ops::RangeInclusive<i32> { (a - 1.0).floor() as i32..=(b + 1.0).ceil() as i32 }

/// Metal deck plates in staggered rows: bevelled edges, bolts at the corners, the odd vent, and
/// light bars set into some plates.
fn panels(t: &mut Tile, base: [u8; 3], mortar: [u8; 3], seed: u32) {
    let (b, m) = (f3(base), f3(mortar));
    let p = SIZE as i32 / 2;
    for y in 0..SIZE as i32 {
        let row = y / p;
        let off = if row % 2 == 1 { p / 2 } else { 0 };
        for x in 0..SIZE as i32 {
            let px = (x + off).rem_euclid(SIZE as i32);
            let (col, lx, ly) = (px / p, px % p, y % p);
            let id = (row * 2 + col) as u32;
            let tint = 1.0 + (hf(seed ^ 0xA1, id) - 0.5) * 0.12;
            let mut c = sc(b, tint);
            // Seams, then a bevel: lit along the top and left, in shade along the bottom and right.
            if lx < 2 || ly < 2 { c = sc(b, 0.32); }
            else if lx < 5 || ly < 5 { c = sc(c, 1.2); }
            else if lx >= p - 4 || ly >= p - 4 { c = sc(c, 0.74); }
            let i = t.at(x, y);
            t.col[i] = c;
        }
    }
    for row in 0..2 {
        let off = if row % 2 == 1 { p / 2 } else { 0 };
        for col in 0..2 {
            let id = (row * 2 + col) as u32;
            let (x0, y0) = ((col * p - off) as f32, (row * p) as f32);
            for (bx, by) in [(9.0, 9.0), (p as f32 - 9.0, 9.0), (9.0, p as f32 - 9.0), (p as f32 - 9.0, p as f32 - 9.0)] {
                t.disc(x0 + bx, y0 + by, 2.6, sc(b, 0.55), 0.0);
                t.disc(x0 + bx - 0.6, y0 + by - 0.6, 1.2, sc(b, 1.35), 0.0);
            }
            // Two plates on a diagonal always carry a light bar, so every floor has its lights;
            // the other two are plain, vented or lit by chance.
            let kind = if id == 0 || id == 3 { 0.5 } else { hf(seed ^ 0xA2, id) };
            if kind < 0.3 {
                // A vent: dark slots.
                for k in 0..6 {
                    let y = y0 + 22.0 + k as f32 * 9.0;
                    t.rect(x0 + 22.0, y, x0 + p as f32 - 22.0, y + 4.0, sc(b, 0.25), 0.0);
                }
            } else if kind < 0.75 {
                // A light bar set into the plate, in a dark housing.
                let y = y0 + p as f32 * 0.5;
                t.rect(x0 + 16.0, y - 4.0, x0 + p as f32 - 16.0, y + 4.0, sc(b, 0.3), 0.0);
                t.rect(x0 + 19.0, y - 1.6, x0 + p as f32 - 19.0, y + 1.6, m, 1.0);
            }
        }
    }
}

/// A dark floor ruled by glowing lines: major lines four to a tile, faint minor ones between, and
/// a bright point where major lines cross.
fn grid(t: &mut Tile, base: [u8; 3], mortar: [u8; 3]) {
    let (b, m) = (f3(base), f3(mortar));
    let s = SIZE as f32;
    for k in 0..16 {
        let at = k as f32 * s / 16.0;
        if k % 4 == 0 { continue; }
        t.line([at, 0.0], [at, s], 1.0, mix(b, m, 0.3), 0.18);
        t.line([0.0, at], [s, at], 1.0, mix(b, m, 0.3), 0.18);
    }
    for k in 0..4 {
        let at = k as f32 * s / 4.0;
        // A soft halo round each major line, then the line.
        t.line([at, 0.0], [at, s], 6.0, mix(b, m, 0.12), 0.08);
        t.line([0.0, at], [s, at], 6.0, mix(b, m, 0.12), 0.08);
        t.line([at, 0.0], [at, s], 2.2, m, 1.0);
        t.line([0.0, at], [s, at], 2.2, m, 1.0);
    }
    for i in 0..4 {
        for j in 0..4 {
            t.disc(i as f32 * s / 4.0, j as f32 * s / 4.0, 3.2, mix(m, [255.0; 3], 0.5), 1.0);
        }
    }
}

/// Hexagonal plates, seven across and eight down a tile, joined by glowing seams.
fn hex(t: &mut Tile, base: [u8; 3], mortar: [u8; 3], seed: u32) {
    let (b, m) = (f3(base), f3(mortar));
    let (cols, rows) = (7, 8);
    let (cw, rh) = (SIZE as f32 / cols as f32, SIZE as f32 / rows as f32);
    let mut centres = Vec::new();
    for r in 0..rows {
        for c in 0..cols {
            let x = c as f32 * cw + if r % 2 == 1 { cw * 0.5 } else { 0.0 };
            centres.push((x, r as f32 * rh + rh * 0.5, (r * cols + c) as u32));
        }
    }
    let s = SIZE as f32;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let (mut d1, mut d2, mut id) = (f32::MAX, f32::MAX, 0u32);
            for &(cx, cy, k) in &centres {
                let mut dx = (px - cx).abs();
                let mut dy = (py - cy).abs();
                dx = dx.min(s - dx);
                dy = dy.min(s - dy);
                let d = (dx * dx + dy * dy).sqrt();
                if d < d1 { d2 = d1; d1 = d; id = k; } else if d < d2 { d2 = d; }
            }
            // Half the gap between the two nearest centres' distances: the distance to the seam.
            let seam = (d2 - d1) * 0.5;
            let tint = 1.0 + (hf(seed ^ 0xC1, id) - 0.5) * 0.14;
            // Plates dish a little toward the middle.
            let mut c = sc(b, tint * (0.9 + 0.1 * (d1 / (cw * 0.6)).min(1.0)));
            if seam < 4.0 { c = sc(c, 1.12); }
            let i = y * SIZE + x;
            t.col[i] = c;
            let a = cover(1.3 - seam);
            if a > 0.0 { t.col[i] = mix(t.col[i], m, a); t.glow[i] = a; }
        }
    }
}

/// A circuit board: traces running straight and turning at 45 degrees, pads at their ends, and a
/// few chips with pins.
fn circuit(t: &mut Tile, base: [u8; 3], mortar: [u8; 3], seed: u32) {
    let (b, m) = (f3(base), f3(mortar));
    let g = 12.0f32;
    let n = (SIZE as f32 / g) as i32;
    for k in 0..3u32 {
        let (cx, cy) = ((hf(seed ^ 0xE1, k) * n as f32).floor() * g, (hf(seed ^ 0xE2, k) * n as f32).floor() * g);
        let (w, hh) = (g * (2.0 + (hf(seed ^ 0xE3, k) * 3.0).floor()), g * (2.0 + (hf(seed ^ 0xE4, k) * 2.0).floor()));
        t.rect(cx, cy, cx + w, cy + hh, sc(b, 0.45), 0.0);
        t.rect(cx + 2.0, cy + 2.0, cx + w - 2.0, cy + hh - 2.0, sc(b, 0.62), 0.0);
        let mut px = cx + g * 0.5;
        while px < cx + w {
            t.rect(px - 1.5, cy - 4.0, px + 1.5, cy, mix(b, m, 0.5), 0.25);
            t.rect(px - 1.5, cy + hh, px + 1.5, cy + hh + 4.0, mix(b, m, 0.5), 0.25);
            px += g;
        }
    }
    let dirs = [(1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1), (0, -1), (1, -1)];
    for k in 0..26u32 {
        let mut p = [(hf(seed ^ 0xF1, k) * n as f32).floor(), (hf(seed ^ 0xF2, k) * n as f32).floor()];
        let mut d = (hf(seed ^ 0xF3, k) * 8.0) as usize % 8;
        let len = 3 + (hf(seed ^ 0xF4, k) * 9.0) as u32;
        let start = p;
        for s in 0..len {
            if hf(seed ^ 0xF5, k * 64 + s) < 0.25 { d = (d + if hf(seed ^ 0xF6, k * 64 + s) < 0.5 { 1 } else { 7 }) % 8; }
            let q = [p[0] + dirs[d].0 as f32, p[1] + dirs[d].1 as f32];
            t.line([p[0] * g, p[1] * g], [q[0] * g, q[1] * g], 2.0, m, 0.75);
            p = q;
        }
        for e in [start, p] {
            t.disc(e[0] * g, e[1] * g, 3.4, m, 1.0);
            t.disc(e[0] * g, e[1] * g, 1.5, sc(b, 0.5), 0.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_scifi_pattern_tiles_and_has_lights() {
        for p in [Pattern::Panels, Pattern::Grid, Pattern::Hex, Pattern::Circuit] {
            let (rgb, glow) = gen(p, [70, 76, 86], [90, 220, 255], 6, 0.2, 3).unwrap();
            assert_eq!(rgb.len(), SIZE * SIZE * 3);
            assert!(glow.iter().any(|g| *g > 0.9), "{p:?} has no lit parts");
            // Opposite edges meet: the step across the wrap is no bigger than steps inside.
            let px = |x: usize, y: usize| { let i = (y * SIZE + x) * 3; rgb[i] as f32 + rgb[i + 1] as f32 + rgb[i + 2] as f32 };
            let (mut wrap_step, mut inner) = (0.0, 0.0);
            for y in 0..SIZE { wrap_step += (px(SIZE - 1, y) - px(0, y)).abs(); inner += (px(SIZE / 2 - 1, y) - px(SIZE / 2, y)).abs(); }
            assert!(wrap_step < inner * 2.0 + 40.0 * SIZE as f32, "{p:?} shows a seam at the tile's edge ({wrap_step} vs {inner})");
        }
    }
}
