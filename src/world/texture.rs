//! Surface textures: procedural tiles with mip levels, sampled in linear colour.

use crate::scene::{Material, Pattern};
use std::collections::HashMap;

/// sRGB byte to linear light.
pub fn srgb_to_lin(v: u8) -> f32 { SRGB_LUT.with(|l| l[v as usize]) }

thread_local! {
    static SRGB_LUT: [f32; 256] = {
        let mut t = [0.0f32; 256];
        for (i, e) in t.iter_mut().enumerate() {
            let c = i as f32 / 255.0;
            *e = if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) };
        }
        t
    };
}

pub fn rgb_lin(c: [u8; 3]) -> [f32; 3] { [srgb_to_lin(c[0]), srgb_to_lin(c[1]), srgb_to_lin(c[2])] }

/// Linear light (0..1) to an sRGB byte.
#[inline]
pub fn lin_to_srgb(v: f32) -> u8 {
    let v = v.clamp(0.0, 1.0);
    let s = if v <= 0.0031308 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0 + 0.5) as u8
}

/// A square tiling texture in linear colour with a mip chain.
pub struct Texture {
    /// levels[0] is full size; each level halves.
    pub levels: Vec<(usize, Vec<[f32; 3]>)>,
}

impl Texture {
    pub fn from_material(m: &Material) -> Texture {
        let size = 192usize;
        let rgb: Vec<u8> = if m.pattern == Pattern::Plain {
            let mut v = Vec::with_capacity(size * size * 3);
            for _ in 0..size * size { v.extend_from_slice(&m.base); }
            v
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
        Texture { levels }
    }

    /// Sample at texture coordinates in repeats (1.0 = one full tile), with a footprint in repeats per pixel.
    #[inline]
    pub fn sample(&self, u: f32, v: f32, footprint: f32) -> [f32; 3] {
        let base = self.levels[0].0 as f32;
        let texels = (footprint * base).max(1e-6);
        let lod = (texels.log2().max(0.0) as usize).min(self.levels.len() - 1);
        let (s, data) = &self.levels[lod];
        let x = ((u.rem_euclid(1.0)) * *s as f32) as usize % s;
        let y = ((v.rem_euclid(1.0)) * *s as f32) as usize % s;
        data[y * s + x]
    }
}

/// Textures keyed by material, rebuilt only when a material changes.
#[derive(Default)]
pub struct TextureCache {
    map: HashMap<String, std::sync::Arc<Texture>>,
}

impl TextureCache {
    pub fn get(&mut self, m: &Material) -> std::sync::Arc<Texture> {
        let key = format!("{:?}{:?}{:?}{}{}{}{}", m.pattern, m.base, m.mortar, m.noise, m.damage, m.seed, m.brightness);
        if self.map.len() > 64 { self.map.clear(); }
        self.map.entry(key).or_insert_with(|| std::sync::Arc::new(Texture::from_material(m))).clone()
    }
}
