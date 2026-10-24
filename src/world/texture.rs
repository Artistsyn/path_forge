//! Surface textures: procedural tiles with mip levels, sampled in linear colour.

use crate::scene::{Material, Pattern};
use std::collections::HashMap;

/// sRGB byte to linear light.
#[inline]
pub fn srgb_to_lin(v: u8) -> f32 { SRGB_LUT[v as usize] }

static SRGB_LUT: std::sync::LazyLock<[f32; 256]> = std::sync::LazyLock::new(|| {
    let mut t = [0.0f32; 256];
    for (i, e) in t.iter_mut().enumerate() {
        let c = i as f32 / 255.0;
        *e = if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) };
    }
    t
});

pub fn rgb_lin(c: [u8; 3]) -> [f32; 3] { [srgb_to_lin(c[0]), srgb_to_lin(c[1]), srgb_to_lin(c[2])] }

/// The sRGB curve itself, as `lin_to_srgb` answers it.
fn lin_to_srgb_exact(v: f32) -> u8 {
    let v = v.clamp(0.0, 1.0);
    let s = if v <= 0.0031308 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0 + 0.5) as u8
}

const TO_SRGB_BUCKETS: usize = 4096;

/// `lin_to_srgb` without a powf per channel: the byte at the bottom of each of 4096 buckets, and the
/// linear value where each byte begins (found from the curve itself, so answers match it exactly).
struct ToSrgb { bucket: Vec<u8>, starts: [f32; 257] }

static TO_SRGB: std::sync::LazyLock<ToSrgb> = std::sync::LazyLock::new(|| {
    let mut starts = [f32::INFINITY; 257];
    starts[0] = f32::NEG_INFINITY;
    for k in 1..=255usize {
        // The smallest v in 0..=1 whose byte is k or more: a search over the bits of positive floats.
        let (mut lo, mut hi) = (0u32, 1.0f32.to_bits());
        if lin_to_srgb_exact(f32::from_bits(hi)) < k as u8 { continue; }
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if lin_to_srgb_exact(f32::from_bits(mid)) >= k as u8 { hi = mid; } else { lo = mid + 1; }
        }
        starts[k] = f32::from_bits(lo);
    }
    let bucket = (0..TO_SRGB_BUCKETS).map(|i| lin_to_srgb_exact(i as f32 / TO_SRGB_BUCKETS as f32)).collect();
    ToSrgb { bucket, starts }
});

/// `lin_to_srgb`'s own tables, for the GPU: the byte at the bottom of each of 4096 buckets, and
/// the linear value where each byte begins.
#[cfg(feature = "gpu")]
pub(crate) fn srgb_tables() -> (Vec<u8>, [f32; 257]) { let t = &*TO_SRGB; (t.bucket.clone(), t.starts) }

/// Linear light (0..1) to an sRGB byte.
#[inline]
pub fn lin_to_srgb(v: f32) -> u8 {
    let v = v.clamp(0.0, 1.0);
    let t = &*TO_SRGB;
    let mut b = t.bucket[((v * TO_SRGB_BUCKETS as f32) as usize).min(TO_SRGB_BUCKETS - 1)];
    while b < 255 && v >= t.starts[b as usize + 1] { b += 1; }
    b
}

#[cfg(test)]
#[test]
fn srgb_table_matches_the_curve() {
    // Every float from 0 to 1 a few ulps apart, plus each byte's exact edges.
    let mut v = 0.0f32;
    while v <= 1.0 {
        assert_eq!(lin_to_srgb(v), lin_to_srgb_exact(v), "at {v}");
        v = f32::from_bits(v.to_bits() + 997).max(v + 1e-9);
    }
    for k in 1..=255 {
        let s = TO_SRGB.starts[k];
        assert_eq!(lin_to_srgb(s), lin_to_srgb_exact(s));
        let below = f32::from_bits(s.to_bits() - 1);
        assert_eq!(lin_to_srgb(below), lin_to_srgb_exact(below));
    }
}

/// A square tiling texture in linear colour with a mip chain.
pub struct Texture {
    /// levels[0] is full size; each level halves.
    pub levels: Vec<(usize, Vec<[f32; 3]>)>,
    /// How much each texel glows in its own colour, level by level like `levels`; empty when
    /// nothing glows.
    pub emit: Vec<Vec<f32>>,
    /// Read between texels (bilinear) rather than texel by texel: water, whose soft mottling
    /// broke into squares up close.
    pub smooth: bool,
}

impl Texture {
    pub fn from_material(m: &Material) -> Texture {
        let size = 192usize;
        let mut glow: Option<Vec<f32>> = None;
        let rgb: Vec<u8> = if m.pattern == Pattern::Plain {
            let mut v = Vec::with_capacity(size * size * 3);
            for _ in 0..size * size { v.extend_from_slice(&m.base); }
            v
        } else if let Some((rgb, mask)) = super::scifi::gen(m.pattern, m.base, m.mortar, m.noise, m.damage, 11 + m.seed) {
            if m.glow > 0.0 { glow = Some(mask.iter().map(|g| g * m.glow).collect()); }
            rgb
        } else {
            crate::tiles::gen_tile(m.pattern.name(), m.base, m.mortar, m.noise, m.damage, 11 + m.seed)
        };
        let side = ((rgb.len() / 3) as f32).sqrt() as usize;
        let gain = m.brightness.max(0.0);
        let base: Vec<[f32; 3]> = rgb.chunks_exact(3).map(|c| {
            let l = rgb_lin([c[0], c[1], c[2]]);
            [l[0] * gain, l[1] * gain, l[2] * gain]
        }).collect();
        let mut levels = vec![(side, base)];
        while levels.last().unwrap().0 > 3 {
            let (s, prev) = levels.last().unwrap();
            let n = s / 2;
            let mut next = vec![[0.0f32; 3]; n * n];
            for y in 0..n {
                for x in 0..n {
                    let mut acc = [0.0f32; 3];
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let p = prev[(y * 2 + dy) * s + x * 2 + dx];
                        for k in 0..3 { acc[k] += p[k] * 0.25; }
                    }
                    next[y * n + x] = acc;
                }
            }
            levels.push((n, next));
        }
        let mut emit = Vec::new();
        if let Some(g) = glow {
            emit.push(g);
            for (s, _) in levels.iter().skip(1) {
                let (ps, prev) = (s * 2, emit.last().unwrap());
                let next: Vec<f32> = (0..s * s).map(|i| {
                    let (x, y) = (i % s, i / s);
                    0.25 * (prev[(y * 2) * ps + x * 2] + prev[(y * 2) * ps + x * 2 + 1] + prev[(y * 2 + 1) * ps + x * 2] + prev[(y * 2 + 1) * ps + x * 2 + 1])
                }).collect();
                emit.push(next);
            }
        }
        Texture { levels, emit, smooth: m.pattern == Pattern::Water }
    }

    /// How much the texel `sample` reads glows (0 when nothing does).
    #[inline]
    pub fn sample_emit(&self, u: f32, v: f32, footprint: f32) -> f32 {
        if self.emit.is_empty() { return 0.0; }
        let base = self.levels[0].0 as f32;
        let texels = (footprint * base).max(1e-6);
        let lod = (texels.log2().max(0.0) as usize).min(self.levels.len() - 1);
        let s = self.levels[lod].0;
        let x = ((u.rem_euclid(1.0)) * s as f32) as usize % s;
        let y = ((v.rem_euclid(1.0)) * s as f32) as usize % s;
        self.emit[lod][y * s + x]
    }

    /// Sample at texture coordinates in repeats (1.0 = one full tile), with a footprint in repeats per pixel.
    #[inline]
    pub fn sample(&self, u: f32, v: f32, footprint: f32) -> [f32; 3] {
        let base = self.levels[0].0 as f32;
        let texels = (footprint * base).max(1e-6);
        let lod = (texels.log2().max(0.0) as usize).min(self.levels.len() - 1);
        let (s, data) = &self.levels[lod];
        if self.smooth {
            let sf = *s as f32;
            let (fx, fy) = (u.rem_euclid(1.0) * sf - 0.5, v.rem_euclid(1.0) * sf - 0.5);
            let (x0, y0) = (fx.floor(), fy.floor());
            let (tx, ty) = (fx - x0, fy - y0);
            let (xa, ya) = ((x0 as i64).rem_euclid(*s as i64) as usize, (y0 as i64).rem_euclid(*s as i64) as usize);
            let (xb, yb) = ((xa + 1) % s, (ya + 1) % s);
            let (a, b, c, d) = (data[ya * s + xa], data[ya * s + xb], data[yb * s + xa], data[yb * s + xb]);
            return [0, 1, 2].map(|k| {
                let top = a[k] + (b[k] - a[k]) * tx;
                let bot = c[k] + (d[k] - c[k]) * tx;
                top + (bot - top) * ty
            });
        }
        let x = ((u.rem_euclid(1.0)) * *s as f32) as usize % s;
        let y = ((v.rem_euclid(1.0)) * *s as f32) as usize % s;
        data[y * s + x]
    }
}

/// Textures keyed by material, rebuilt only when a material changes. Each renderer keeps the ones
/// it has asked for; all of them come from `TEXTURES`, made once for every renderer in the process
/// (and for the thread that gets places ready ahead of the walk).
#[derive(Default)]
pub struct TextureCache {
    map: HashMap<String, std::sync::Arc<Texture>>,
}

/// Every renderer's textures, within a budget of bytes: past it the ones longest unused are let
/// go. (Each renderer cleared its own at 64 before, made them all again on the road's thread, and
/// sent them to the GPU again as new ones.)
struct SharedTextures { map: HashMap<String, (std::sync::Arc<Texture>, u64)>, bytes: usize, tick: u64 }

const TEXTURE_BUDGET: usize = 96 << 20;

static TEXTURES: std::sync::LazyLock<std::sync::Mutex<SharedTextures>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(SharedTextures { map: HashMap::new(), bytes: 0, tick: 0 }));

fn texture_bytes(t: &Texture) -> usize {
    t.levels.iter().map(|l| l.1.len() * 12).sum::<usize>() + t.emit.iter().map(|e| e.len() * 4).sum::<usize>()
}

impl TextureCache {
    pub fn get(&mut self, m: &Material) -> std::sync::Arc<Texture> {
        let key = format!("{:?}{:?}{:?}{}{}{}{}/{}", m.pattern, m.base, m.mortar, m.noise, m.damage, m.seed, m.brightness, m.glow);
        if let Some(t) = self.map.get(&key) { return t.clone(); }
        let shared = |st: &mut SharedTextures| { st.tick += 1; let tick = st.tick; st.map.get_mut(&key).map(|e| { e.1 = tick; e.0.clone() }) };
        let found = shared(&mut TEXTURES.lock().unwrap());
        let t = found.unwrap_or_else(|| {
            // Made outside the lock (another thread waits only for a lookup); kept once.
            let t = std::sync::Arc::new(Texture::from_material(m));
            let mut st = TEXTURES.lock().unwrap();
            if let Some(e) = st.map.get(&key) { return e.0.clone(); }
            st.tick += 1;
            let tick = st.tick;
            st.bytes += texture_bytes(&t);
            st.map.insert(key.clone(), (t.clone(), tick));
            while st.bytes > TEXTURE_BUDGET && st.map.len() > 1 {
                let Some(old) = st.map.iter().min_by_key(|e| e.1 .1).map(|e| e.0.clone()) else { break };
                if let Some((o, _)) = st.map.remove(&old) { st.bytes -= texture_bytes(&o); }
            }
            t
        });
        if self.map.len() > 256 { self.map.clear(); }
        self.map.insert(key, t.clone());
        t
    }
}
