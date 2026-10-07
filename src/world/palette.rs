//! Palettes and ordered dithering for pixel-art styles.

use crate::scene::{Dither, Rgb};

/// Built-in palettes by name.
pub const NAMED: &[(&str, &str)] = &[
    ("PICO-8", "000000 1d2b53 7e2553 008751 ab5236 5f574f c2c3c7 fff1e8 ff004d ffa300 ffec27 00e436 29adff 83769c ff77a8 ffccaa"),
    ("Sweetie 16", "1a1c2c 5d275d b13e53 ef7d57 ffcd75 a7f070 38b764 257179 29366f 3b5dc9 41a6f6 73eff7 f4f4f4 94b0c2 566c86 333c57"),
    ("Endesga 32", "be4a2f d77643 ead4aa e4a672 b86f50 733e39 3e2731 a22633 e43b44 f77622 feae34 fee761 63c74d 3e8948 265c42 193c3e 124e89 0099db 2ce8f5 ffffff c0cbdc 8b9bb4 5a6988 3a4466 262b44 181425 ff0044 68386c b55088 f6757a e8b796 c28569"),
    ("Game Boy", "0f380f 306230 8bac0f 9bbc0f"),
    ("Dark Fantasy 16", "0d0b10 1c1622 2e2233 47334a 6b4a5e 3a2a1e 5e3f26 8a5a32 c08a4a e8c37a 4a5a3a 6f8a4a 2c3a4a 4a6a8a 9ab0c0 e6e0d0"),
    ("Ember 8", "120a08 2e140c 5a2410 9a3a12 d8661a f4a33a fde08a fff8e0"),
    ("Moonlit 8", "0a0c18 141c34 22305a 3a4e80 5e78a8 8ea8c8 c4d4e4 f0f4f8"),
];

pub fn parse_hex_list(s: &str) -> Vec<Rgb> {
    s.split_whitespace().filter_map(|h| {
        let v = u32::from_str_radix(h.trim_start_matches('#'), 16).ok()?;
        Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
    }).collect()
}

pub fn named(name: &str) -> Option<Vec<Rgb>> {
    NAMED.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, hex)| parse_hex_list(hex))
}

/// Nearest-colour lookup over a grid of RGB space (32 or 64 steps per channel).
pub struct PaletteLut {
    pub colors: Vec<Rgb>,
    lut: Vec<u8>,
    bits: u32,
    /// Typical distance between neighbouring palette colours, in 0..255 units; scales the dither.
    pub spread: f32,
    /// Lightness range when the palette is a ramp (see `ramp_range`).
    pub ramp: Option<(f32, f32)>,
}

/// sRGB (0..255) to OKLab (Ottosson 2020): distances here track how different colours look,
/// so a small palette keeps the hue of what it replaces instead of jumping to a closer-in-RGB hue.
pub fn oklab(c: [f32; 3]) -> [f32; 3] {
    let lin = |v: f32| { let v = (v / 255.0).clamp(0.0, 1.0); if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) } };
    let (r, g, b) = (lin(c[0]), lin(c[1]), lin(c[2]));
    let l = (0.412_221_5 * r + 0.536_332_5 * g + 0.051_445_9 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_9 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    [0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
     1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
     0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s]
}

/// If the palette is a ramp (lightness alone decides the colour: no two colours of similar
/// lightness differ in hue, as in Game Boy greens or a hue-shifted fire ramp), the lightness range.
pub fn ramp_range(labs: &[[f32; 3]]) -> Option<(f32, f32)> {
    if labs.len() < 3 { return None; }
    let lo = labs.iter().map(|c| c[0]).fold(f32::MAX, f32::min);
    let hi = labs.iter().map(|c| c[0]).fold(f32::MIN, f32::max);
    if hi - lo < 0.2 { return None; }
    for (i, a) in labs.iter().enumerate() {
        for b in &labs[i + 1..] {
            let chroma = ((a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
            if (a[0] - b[0]).abs() < 0.08 && chroma > 0.09 { return None; }
        }
    }
    Some((lo, hi))
}

fn dist2(a: [f32; 3], b: [f32; 3]) -> f32 {
    let (dl, da, db) = (a[0] - b[0], a[1] - b[1], a[2] - b[2]);
    dl * dl + da * da + db * db
}

impl PaletteLut {
    pub fn new(colors: Vec<Rgb>) -> PaletteLut { Self::with_bits(colors, 5) }

    /// `bits` per channel: 5 for small art palettes, 6 for 256-colour export palettes.
    pub fn with_bits(colors: Vec<Rgb>, bits: u32) -> PaletteLut {
        use rayon::prelude::*;
        let bits = bits.clamp(4, 6);
        let colors = if colors.is_empty() { vec![[0, 0, 0], [255, 255, 255]] } else { colors };
        let n = 1usize << bits;
        let step = 256.0 / n as f32;
        let labs: Vec<[f32; 3]> = colors.iter().map(|c| oklab([c[0] as f32, c[1] as f32, c[2] as f32])).collect();
        let ramp = ramp_range(&labs);
        let lut: Vec<u8> = (0..n * n * n).into_par_iter().map(|i| {
            let c = oklab([((i >> (2 * bits)) & (n - 1)) as f32 * step + step * 0.5, ((i >> bits) & (n - 1)) as f32 * step + step * 0.5, (i & (n - 1)) as f32 * step + step * 0.5]);
            match ramp {
                // A one-hue ramp: spread the input's lightness over the ramp and match lightness only,
                // so every shade is used (nearest colour would put most of a scene on one shade).
                Some((lo, hi)) => {
                    let target = lo + c[0].clamp(0.0, 1.0) * (hi - lo);
                    labs.iter().enumerate().min_by(|a, b| (a.1[0] - target).abs().partial_cmp(&(b.1[0] - target).abs()).unwrap()).map(|(k, _)| k as u8).unwrap_or(0)
                }
                None => labs.iter().enumerate()
                    .min_by(|a, b| dist2(c, *a.1).partial_cmp(&dist2(c, *b.1)).unwrap())
                    .map(|(k, _)| k as u8).unwrap_or(0),
            }
        }).collect();
        // Dither amplitude follows the typical RGB gap between neighbouring palette colours.
        let rgb_dist = |a: Rgb, b: Rgb| { let d = |k: usize| a[k] as f32 - b[k] as f32; (d(0) * d(0) + d(1) * d(1) + d(2) * d(2)).sqrt() / 3.0f32.sqrt() };
        let mut spread = 0.0;
        for (i, a) in colors.iter().enumerate() {
            let nearest = labs.iter().enumerate().filter(|(j, b)| *j != i && *b != &labs[i])
                .min_by(|x, y| dist2(labs[i], *x.1).partial_cmp(&dist2(labs[i], *y.1)).unwrap())
                .map(|(j, _)| rgb_dist(*a, colors[j]));
            if let Some(d) = nearest { spread += d; }
        }
        let spread = (spread / colors.len() as f32).clamp(8.0, 96.0);
        PaletteLut { colors, lut, bits, spread, ramp }
    }

    #[inline]
    pub fn index(&self, c: [f32; 3]) -> u8 {
        let sh = 8 - self.bits;
        let q = |v: f32| v.clamp(0.0, 255.0) as usize >> sh;
        self.lut[(q(c[0]) << (2 * self.bits)) | (q(c[1]) << self.bits) | q(c[2])]
    }

    #[inline]
    pub fn nearest(&self, c: [f32; 3]) -> Rgb { self.colors[self.index(c) as usize] }
}

/// Ordered dither threshold in -0.5..0.5 for pixel (x, y).
#[inline]
pub fn bayer(d: Dither, x: usize, y: usize) -> f32 {
    const B2: [u8; 4] = [0, 2, 3, 1];
    const B4: [u8; 16] = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5];
    const B8: [u8; 64] = [
        0, 32, 8, 40, 2, 34, 10, 42, 48, 16, 56, 24, 50, 18, 58, 26, 12, 44, 4, 36, 14, 46, 6, 38, 60, 28, 52, 20, 62, 30, 54, 22,
        3, 35, 11, 43, 1, 33, 9, 41, 51, 19, 59, 27, 49, 17, 57, 25, 15, 47, 7, 39, 13, 45, 5, 37, 63, 31, 55, 23, 61, 29, 53, 21,
    ];
    match d {
        Dither::None => 0.0,
        Dither::Bayer2 => (B2[(y & 1) * 2 + (x & 1)] as f32 + 0.5) / 4.0 - 0.5,
        Dither::Bayer4 => (B4[(y & 3) * 4 + (x & 3)] as f32 + 0.5) / 16.0 - 0.5,
        Dither::Bayer8 => (B8[(y & 7) * 8 + (x & 7)] as f32 + 0.5) / 64.0 - 0.5,
    }
}

/// Pick `n` colours that represent `pixels` well (median cut).
pub fn median_cut(pixels: &[Rgb], n: usize) -> Vec<Rgb> {
    let n = n.clamp(2, 256);
    if pixels.is_empty() { return vec![[0, 0, 0]; 2]; }
    let mut boxes: Vec<Vec<Rgb>> = vec![pixels.to_vec()];
    while boxes.len() < n {
        // Split the box with the widest channel range.
        let (bi, ch, _) = boxes.iter().enumerate().filter(|(_, b)| b.len() > 1).map(|(i, b)| {
            let mut best = (0usize, 0i32);
            for c in 0..3 {
                let (lo, hi) = b.iter().fold((255u8, 0u8), |(lo, hi), p| (lo.min(p[c]), hi.max(p[c])));
                let r = hi as i32 - lo as i32;
                if r > best.1 { best = (c, r); }
            }
            (i, best.0, best.1)
        }).max_by_key(|(_, _, r)| *r).unwrap_or((0, 0, 0));
        if boxes[bi].len() < 2 { break; }
        let mut b = std::mem::take(&mut boxes[bi]);
        b.sort_unstable_by_key(|p| p[ch]);
        let tail = b.split_off(b.len() / 2);
        boxes[bi] = b;
        boxes.push(tail);
    }
    boxes.iter().filter(|b| !b.is_empty()).map(|b| {
        let mut s = [0u64; 3];
        for p in b { for c in 0..3 { s[c] += p[c] as u64; } }
        let k = b.len() as u64;
        [(s[0] / k) as u8, (s[1] / k) as u8, (s[2] / k) as u8]
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn labs(name: &str) -> Vec<[f32; 3]> { named(name).unwrap().iter().map(|c| oklab([c[0] as f32, c[1] as f32, c[2] as f32])).collect() }

    #[test]
    fn single_hue_ramps_are_recognised_and_full_palettes_are_not() {
        for ramp in ["Game Boy", "Moonlit 8", "Ember 8"] { assert!(ramp_range(&labs(ramp)).is_some(), "{ramp}"); }
        for full in ["PICO-8", "Sweetie 16", "Endesga 32"] { assert!(ramp_range(&labs(full)).is_none(), "{full}"); }
    }

    #[test]
    fn a_ramp_uses_every_shade_across_a_grey_scale() {
        let lut = PaletteLut::new(named("Game Boy").unwrap());
        let used: std::collections::BTreeSet<u8> = (0..=255).step_by(5).map(|v| lut.index([v as f32; 3])).collect();
        assert_eq!(used.len(), 4);
    }
}
