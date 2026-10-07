//! Screen looks applied after tone mapping: painterly (Kuwahara), colour grades (built-in or .cube),
//! paper grain and CRT scanlines. Every effect depends only on the pixel and its neighbours in the same
//! frame, or on a static pattern, so loops stay seamless.

use rayon::prelude::*;
use std::path::Path;

/// Kuwahara filter: each pixel takes the mean of whichever of its four (r+1)x(r+1) corner windows
/// varies least, so flat areas become strokes while edges stay sharp. Summed-area tables make the
/// cost independent of the radius.
pub fn kuwahara(rgb: &mut [[f32; 3]], w: usize, h: usize, radius: f32) {
    let r = radius.round().clamp(1.0, 16.0) as i64;
    let (sw, sh) = (w + 1, h + 1);
    // Sums of r, g, b and luma squared.
    let mut sat = vec![[0.0f64; 4]; sw * sh];
    for y in 0..h {
        let mut row = [0.0f64; 4];
        for x in 0..w {
            let c = rgb[y * w + x];
            let l = (0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]) as f64;
            row[0] += c[0] as f64; row[1] += c[1] as f64; row[2] += c[2] as f64; row[3] += l * l;
            let above = sat[y * sw + x + 1];
            sat[(y + 1) * sw + x + 1] = [above[0] + row[0], above[1] + row[1], above[2] + row[2], above[3] + row[3]];
        }
    }
    let src = rgb.to_vec();
    let luma = |c: [f32; 3]| (0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]) as f64;
    // Plain luma sums for the variance's mean term.
    let mut lsat = vec![0.0f64; sw * sh];
    for y in 0..h {
        let mut row = 0.0;
        for x in 0..w {
            row += luma(src[y * w + x]);
            lsat[(y + 1) * sw + x + 1] = lsat[y * sw + x + 1] + row;
        }
    }
    rgb.par_chunks_mut(w).enumerate().for_each(|(y, out)| {
        let y = y as i64;
        for x in 0..w as i64 {
            let mut best = (f64::MAX, [0.0f32; 3]);
            for (dx, dy) in [(-r, -r), (0, -r), (-r, 0), (0, 0)] {
                let (x0, y0) = ((x + dx).max(0) as usize, (y + dy).max(0) as usize);
                let (x1, y1) = (((x + dx + r) as usize).min(w - 1) + 1, ((y + dy + r) as usize).min(h - 1) + 1);
                if x1 <= x0 || y1 <= y0 { continue; }
                let n = ((x1 - x0) * (y1 - y0)) as f64;
                let area = |k: usize| sat[y1 * sw + x1][k] - sat[y0 * sw + x1][k] - sat[y1 * sw + x0][k] + sat[y0 * sw + x0][k];
                let lsum = lsat[y1 * sw + x1] - lsat[y0 * sw + x1] - lsat[y1 * sw + x0] + lsat[y0 * sw + x0];
                let mean_l = lsum / n;
                let var = area(3) / n - mean_l * mean_l;
                if var < best.0 { best = (var, [(area(0) / n) as f32, (area(1) / n) as f32, (area(2) / n) as f32]); }
            }
            out[x as usize] = best.1;
        }
    });
}

/// A colour transform on display RGB (0..255).
pub enum Grade {
    Builtin(fn([f32; 3]) -> [f32; 3]),
    Cube { size: usize, data: Vec<[f32; 3]> },
}

pub const BUILTIN: &[&str] = &["Warm", "Cool", "Teal Orange", "Faded", "Night", "Sepia", "Vivid"];

fn lum(c: [f32; 3]) -> f32 { 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2] }
fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] { [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t] }

fn warm(c: [f32; 3]) -> [f32; 3] { [c[0] * 1.08 + 6.0, c[1] * 1.0 + 2.0, c[2] * 0.86] }
fn cool(c: [f32; 3]) -> [f32; 3] { [c[0] * 0.88, c[1] * 0.98 + 2.0, c[2] * 1.1 + 8.0] }
fn teal_orange(c: [f32; 3]) -> [f32; 3] {
    // Shadows toward teal, highlights toward orange; skin-ish midtones kept.
    let t = (lum(c) / 255.0).clamp(0.0, 1.0);
    let shadow = [c[0] * 0.82, c[1] * 1.0 + 6.0, c[2] * 1.08 + 10.0];
    let high = [c[0] * 1.1 + 8.0, c[1] * 1.0 + 2.0, c[2] * 0.82];
    mix(shadow, high, t * t * (3.0 - 2.0 * t))
}
fn faded(c: [f32; 3]) -> [f32; 3] {
    let l = lum(c);
    let d = mix(c, [l, l, l], 0.25);
    [d[0] * 0.82 + 28.0, d[1] * 0.82 + 26.0, d[2] * 0.82 + 30.0]
}
fn night(c: [f32; 3]) -> [f32; 3] {
    let l = lum(c);
    let d = mix(c, [l, l, l], 0.45);
    [d[0] * 0.7, d[1] * 0.82, d[2] * 1.05 + 6.0]
}
fn sepia(c: [f32; 3]) -> [f32; 3] {
    [0.393 * c[0] + 0.769 * c[1] + 0.189 * c[2], 0.349 * c[0] + 0.686 * c[1] + 0.168 * c[2], 0.272 * c[0] + 0.534 * c[1] + 0.131 * c[2]]
}
fn vivid(c: [f32; 3]) -> [f32; 3] {
    let l = lum(c);
    let s = mix([l, l, l], c, 1.35);
    [(s[0] - 128.0) * 1.08 + 128.0, (s[1] - 128.0) * 1.08 + 128.0, (s[2] - 128.0) * 1.08 + 128.0]
}

impl Grade {
    pub fn builtin(name: &str) -> Option<Grade> {
        let f: fn([f32; 3]) -> [f32; 3] = match name.to_lowercase().replace(['-', '_'], " ").as_str() {
            "warm" => warm, "cool" => cool, "teal orange" => teal_orange, "faded" => faded,
            "night" => night, "sepia" => sepia, "vivid" => vivid, _ => return None,
        };
        Some(Grade::Builtin(f))
    }

    /// Parse an Adobe/Resolve .cube 3D LUT (red changes fastest; domain 0..1).
    pub fn load_cube(path: &Path) -> Result<Grade, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut size = 0usize;
        let mut data = Vec::new();
        for line in text.lines() {
            let t = line.trim();
            if t.is_empty() || t.starts_with('#') { continue; }
            if let Some(v) = t.strip_prefix("LUT_3D_SIZE") { size = v.trim().parse().map_err(|_| "bad LUT_3D_SIZE")?; continue; }
            if t.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) { continue; }
            let v: Vec<f32> = t.split_whitespace().filter_map(|x| x.parse().ok()).collect();
            if v.len() == 3 { data.push([v[0] * 255.0, v[1] * 255.0, v[2] * 255.0]); }
        }
        if size < 2 || data.len() != size * size * size {
            return Err(format!("{}: expected a 3D LUT with LUT_3D_SIZE^3 entries (size {size}, {} entries)", path.display(), data.len()));
        }
        Ok(Grade::Cube { size, data })
    }

    pub fn apply(&self, c: [f32; 3]) -> [f32; 3] {
        match self {
            Grade::Builtin(f) => f(c),
            Grade::Cube { size, data } => {
                let n = *size;
                let at = |v: f32| { let f = (v / 255.0).clamp(0.0, 1.0) * (n - 1) as f32; let i = (f as usize).min(n - 2); (i, f - i as f32) };
                let ((r0, fr), (g0, fg), (b0, fb)) = (at(c[0]), at(c[1]), at(c[2]));
                let get = |r: usize, g: usize, b: usize| data[(b * n + g) * n + r];
                let mut out = [0.0f32; 3];
                for (dr, wr) in [(0, 1.0 - fr), (1, fr)] {
                    for (dg, wg) in [(0, 1.0 - fg), (1, fg)] {
                        for (db, wb) in [(0, 1.0 - fb), (1, fb)] {
                            let v = get(r0 + dr, g0 + dg, b0 + db);
                            let wt = wr * wg * wb;
                            for k in 0..3 { out[k] += v[k] * wt; }
                        }
                    }
                }
                out
            }
        }
    }
}

pub fn grade(rgb: &mut [[f32; 3]], g: &Grade, strength: f32) {
    let s = strength.clamp(0.0, 1.0);
    rgb.par_iter_mut().for_each(|c| {
        let d = g.apply(*c);
        let m = mix(*c, d, s);
        *c = [m[0].clamp(0.0, 255.0), m[1].clamp(0.0, 255.0), m[2].clamp(0.0, 255.0)];
    });
}

fn hash2(x: i64, y: i64) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343) ^ (y as u32).wrapping_mul(0xd816_3841) ^ 0x9e37_79b9;
    h ^= h >> 15; h = h.wrapping_mul(0x2c1b_3c6d); h ^= h >> 12;
    (h & 0xffff) as f32 / 65535.0
}

pub(crate) fn value_noise(x: f32, y: f32) -> f32 {
    let (xi, yi) = (x.floor() as i64, y.floor() as i64);
    let (fx, fy) = (x - xi as f32, y - yi as f32);
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let a = hash2(xi, yi) + (hash2(xi + 1, yi) - hash2(xi, yi)) * sx;
    let b = hash2(xi, yi + 1) + (hash2(xi + 1, yi + 1) - hash2(xi, yi + 1)) * sx;
    a + (b - a) * sy
}

/// Paper texture and CRT scanlines on the final RGBA image (output pixels).
pub fn surface(rgba: &mut [u8], w: usize, paper: f32, scanlines: f32, art_px: usize) {
    let paper = paper.clamp(0.0, 1.0);
    let scan = scanlines.clamp(0.0, 1.0);
    if paper < 0.001 && scan < 0.001 { return; }
    let period = art_px.max(2);
    rgba.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
        let line = if scan > 0.0 && y % period == period - 1 { 1.0 - 0.55 * scan } else { 1.0 };
        for x in 0..w {
            let mut f = line;
            if paper > 0.0 {
                // Fibres: fine noise stretched along x, over a soft blotchy base.
                let fine = value_noise(x as f32 * 0.9, y as f32 * 0.35) - 0.5;
                let blot = value_noise(x as f32 * 0.035, y as f32 * 0.035) - 0.5;
                f *= 1.0 + (fine * 0.22 + blot * 0.16) * paper;
            }
            for c in 0..3 {
                let i = x * 4 + c;
                row[i] = (row[i] as f32 * f).clamp(0.0, 255.0) as u8;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube(name: &str, f: impl Fn(f32, f32, f32) -> [f32; 3]) -> std::path::PathBuf {
        let mut text = String::from("TITLE \"test\"\nLUT_3D_SIZE 5\n");
        for b in 0..5 { for g in 0..5 { for r in 0..5 {
            let v = f(r as f32 / 4.0, g as f32 / 4.0, b as f32 / 4.0);
            text += &format!("{} {} {}\n", v[0], v[1], v[2]);
        }}}
        let p = std::env::temp_dir().join(format!("pf_{name}_{}.cube", std::process::id()));
        std::fs::write(&p, text).unwrap();
        p
    }

    #[test]
    fn cube_luts_load_red_fastest_and_interpolate() {
        let id = Grade::load_cube(&cube("id", |r, g, b| [r, g, b])).unwrap();
        let c = id.apply([200.0, 90.0, 30.0]);
        assert!((c[0] - 200.0).abs() < 0.5 && (c[1] - 90.0).abs() < 0.5 && (c[2] - 30.0).abs() < 0.5, "{c:?}");
        let swap = Grade::load_cube(&cube("swap", |r, g, b| [b, g, r])).unwrap();
        let c = swap.apply([200.0, 90.0, 30.0]);
        assert!((c[0] - 30.0).abs() < 0.5 && (c[2] - 200.0).abs() < 0.5, "{c:?}");
    }

    #[test]
    fn kuwahara_keeps_a_hard_edge() {
        let (w, h) = (16, 8);
        let mut img: Vec<[f32; 3]> = (0..w * h).map(|i| if i % w < 8 { [20.0; 3] } else { [220.0; 3] }).collect();
        kuwahara(&mut img, w, h, 3.0);
        assert!(img[4 * w + 6][0] < 40.0 && img[4 * w + 9][0] > 200.0, "edge blurred: {:?} {:?}", img[4 * w + 6], img[4 * w + 9]);
    }
}
