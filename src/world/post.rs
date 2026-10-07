//! Post-processing and style: bloom, tone mapping, grading, outlines, lens bend, palettes, upscale.

use super::palette::{bayer, PaletteLut};
use super::raster::{id, GPixel};
use super::render::Image;
use super::texture::lin_to_srgb;
use super::view::View;
use crate::scene::{Dither, Outline, Scene};
use rayon::prelude::*;

/// HDR linear light to display RGB in 0..255 (as f32 so later stages keep precision).
pub fn finish(view: &View, scene: &Scene, phase: f32, hdr: &[[f32; 3]], post_on: bool) -> Vec<[f32; 3]> {
    let (w, h) = (view.width, view.height);
    let p = &scene.post;
    let bloom = if post_on && p.bloom > 0.001 { Some(bloom_buffer(hdr, w, h)) } else { None };
    let exposure = if post_on { p.exposure.max(0.0) } else { 1.0 };
    let tint = [p.tint[0] as f32 / 255.0, p.tint[1] as f32 / 255.0, p.tint[2] as f32 / 255.0];
    let grain_seed = (phase.rem_euclid(1.0) * 4096.0).round() as u32 % 4096;
    let (cx, cy) = (w as f32 * 0.5, h as f32 * 0.5);
    let max_r2 = cx * cx + cy * cy;
    let mut out = vec![[0.0f32; 3]; w * h];
    out.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        for x in 0..w {
            let i = y * w + x;
            let mut c = hdr[i];
            if let Some(b) = &bloom {
                let s = sample_quarter(b, w, h, x, y);
                for k in 0..3 { c[k] += s[k] * p.bloom * 0.9; }
            }
            let mut rgb = [0.0f32; 3];
            for k in 0..3 {
                let v = c[k] * exposure * 1.15;
                // Filmic curve (Narkowicz ACES fit): soft highlights instead of hard clipping.
                let t = (v * (2.51 * v + 0.03)) / (v * (2.43 * v + 0.59) + 0.14);
                rgb[k] = lin_to_srgb(t) as f32;
            }
            if post_on {
                let lum = 0.299 * rgb[0] + 0.587 * rgb[1] + 0.114 * rgb[2];
                for k in 0..3 {
                    let mut v = lum + (rgb[k] - lum) * p.saturation;
                    v = (v - 128.0) * p.contrast + 128.0;
                    rgb[k] = v * tint[k];
                }
                if p.vignette > 0.001 {
                    let (dx, dy) = (x as f32 - cx, y as f32 - cy);
                    let f = (1.0 - ((dx * dx + dy * dy) / max_r2).powf(1.4) * p.vignette).max(0.0);
                    for k in 0..3 { rgb[k] *= f; }
                }
                if p.grain > 0.001 {
                    let mut hsh = (x as u32).wrapping_mul(0x8da6_b343) ^ (y as u32).wrapping_mul(0xd816_3841) ^ grain_seed.wrapping_mul(0xcb1a_b31f);
                    hsh ^= hsh >> 15; hsh = hsh.wrapping_mul(0x2c1b_3c6d); hsh ^= hsh >> 12;
                    let g = ((hsh & 0xffff) as f32 / 32768.0 - 1.0) * p.grain * 40.0;
                    for k in 0..3 { rgb[k] += g; }
                }
            }
            row[x] = [rgb[0].clamp(0.0, 255.0), rgb[1].clamp(0.0, 255.0), rgb[2].clamp(0.0, 255.0)];
        }
    });
    out
}

/// Bright parts at quarter resolution, blurred.
fn bloom_buffer(hdr: &[[f32; 3]], w: usize, h: usize) -> Vec<[f32; 3]> {
    let (qw, qh) = (w.div_ceil(4), h.div_ceil(4));
    let mut q = vec![[0.0f32; 3]; qw * qh];
    for (i, e) in q.iter_mut().enumerate() {
        let (qx, qy) = (i % qw, i / qw);
        let mut acc = [0.0f32; 3];
        let mut n = 0.0;
        for y in qy * 4..(qy * 4 + 4).min(h) {
            for x in qx * 4..(qx * 4 + 4).min(w) {
                let c = hdr[y * w + x];
                let l = 0.3 * c[0] + 0.55 * c[1] + 0.15 * c[2];
                let k = ((l - 0.7) / l.max(1e-4)).max(0.0);
                for j in 0..3 { acc[j] += c[j] * k; }
                n += 1.0;
            }
        }
        *e = [acc[0] / n, acc[1] / n, acc[2] / n];
    }
    // Two passes of a wide separable blur.
    for _ in 0..2 {
        q = blur(&q, qw, qh, true);
        q = blur(&q, qw, qh, false);
    }
    q
}

fn blur(src: &[[f32; 3]], w: usize, h: usize, horizontal: bool) -> Vec<[f32; 3]> {
    const K: [f32; 7] = [0.03, 0.1, 0.2, 0.34, 0.2, 0.1, 0.03];
    let mut out = vec![[0.0f32; 3]; w * h];
    out.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        for x in 0..w {
            let mut acc = [0.0f32; 3];
            for (t, k) in K.iter().enumerate() {
                let o = t as i64 - 3;
                let (sx, sy) = if horizontal { ((x as i64 + o * 2).clamp(0, w as i64 - 1), y as i64) } else { (x as i64, (y as i64 + o * 2).clamp(0, h as i64 - 1)) };
                let c = src[sy as usize * w + sx as usize];
                for j in 0..3 { acc[j] += c[j] * k; }
            }
            row[x] = acc;
        }
    });
    out
}

fn sample_quarter(q: &[[f32; 3]], w: usize, h: usize, x: usize, y: usize) -> [f32; 3] {
    let (qw, qh) = (w.div_ceil(4), h.div_ceil(4));
    let fx = ((x as f32 + 0.5) / 4.0 - 0.5).clamp(0.0, qw as f32 - 1.0);
    let fy = ((y as f32 + 0.5) / 4.0 - 0.5).clamp(0.0, qh as f32 - 1.0);
    let (x0, y0) = (fx as usize, fy as usize);
    let (x1, y1) = ((x0 + 1).min(qw - 1), (y0 + 1).min(qh - 1));
    let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
    let mut out = [0.0f32; 3];
    for k in 0..3 {
        let a = q[y0 * qw + x0][k] * (1.0 - tx) + q[y0 * qw + x1][k] * tx;
        let b = q[y1 * qw + x0][k] * (1.0 - tx) + q[y1 * qw + x1][k] * tx;
        out[k] = a * (1.0 - ty) + b * ty;
    }
    out
}

/// Draw a one-pixel outline around objects (and optionally around every surface edge).
pub fn outline(rgb: &mut [[f32; 3]], gbuf: &[GPixel], w: usize, h: usize, o: &Outline) {
    let col = [o.color[0] as f32, o.color[1] as f32, o.color[2] as f32];
    // Ink lines only around things near enough to carry them; far away they turn into noise.
    const INK_DISTANCE: f32 = 14.0;
    let is_obj = |g: &GPixel| g.id >= id::PROP && g.depth < INK_DISTANCE;
    let marks: Vec<bool> = (0..w * h).into_par_iter().map(|i| {
        let (x, y) = (i % w, i / w);
        let g = &gbuf[i];
        let mut hit = false;
        for (dx, dy) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1)] {
            let (nx, ny) = (x as i64 + dx, y as i64 + dy);
            if nx < 0 || ny < 0 || nx as usize >= w || ny as usize >= h { continue; }
            let n = &gbuf[ny as usize * w + nx as usize];
            if is_obj(n) && !is_obj(g) && n.depth < g.depth { hit = true; break; }
            if is_obj(n) && is_obj(g) && n.depth < g.depth * 0.85 { hit = true; break; }
            if !o.objects_only && n.id != g.id && n.depth < g.depth && g.id != id::NONE && n.id != id::NONE
                && !(n.id == id::GROUND && (g.id == id::WALL_L || g.id == id::WALL_R)) { hit = true; break; }
        }
        hit
    }).collect();
    for (i, m) in marks.into_iter().enumerate() { if m { rgb[i] = col; } }
}

/// Bend the image around the horizon like a wide lens (same curve as PathForge 2.0).
pub fn lens_warp(rgb: &mut [[f32; 3]], w: usize, h: usize, horizon: f32, curve: f32, nearest: bool) {
    let src = rgb.to_vec();
    let amp = curve.clamp(-1.0, 1.0) * h as f32 * 0.22;
    let den = (w.max(2) - 1) as f32;
    let blend_top = (horizon - 40.0 * h as f32 / 854.0).max(0.0);
    let span = 60.0 * h as f32 / 854.0;
    rgb.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        let t = ((y as f32 - blend_top) / span).clamp(0.0, 1.0);
        let weight = t * t * (3.0 - 2.0 * t);
        for x in 0..w {
            let nx = x as f32 / den * 2.0 - 1.0;
            let sy = (y as f32 - amp * nx * nx * weight).clamp(0.0, (h - 1) as f32);
            row[x] = if nearest {
                src[(sy.round() as usize).min(h - 1) * w + x]
            } else {
                let y0 = sy.floor() as usize;
                let y1 = (y0 + 1).min(h - 1);
                let f = sy - y0 as f32;
                let (a, b) = (src[y0 * w + x], src[y1 * w + x]);
                [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f, a[2] + (b[2] - a[2]) * f]
            };
        }
    });
}

/// Snap every pixel to the palette, with ordered dithering anchored to the art-pixel grid.
pub fn quantize(rgb: &mut [[f32; 3]], w: usize, lut: &PaletteLut, dither: Dither, strength: f32) {
    let amp = lut.spread * strength.clamp(0.0, 1.0) * 1.6;
    rgb.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        for (x, c) in row.iter_mut().enumerate() {
            let t = bayer(dither, x, y) * amp;
            let q = lut.nearest([c[0] + t, c[1] + t, c[2] + t]);
            *c = [q[0] as f32, q[1] as f32, q[2] as f32];
        }
    });
}

/// Scale the art-pixel image up to the output size with crisp square pixels.
pub fn upscale(rgb: &[[f32; 3]], w: usize, h: usize, px: usize, out_w: usize, out_h: usize) -> Image {
    let mut rgba = vec![255u8; out_w * out_h * 4];
    rgba.par_chunks_mut(out_w * 4).enumerate().for_each(|(y, row)| {
        let sy = (y / px).min(h - 1);
        for x in 0..out_w {
            let c = rgb[sy * w + (x / px).min(w - 1)];
            row[x * 4] = c[0] as u8;
            row[x * 4 + 1] = c[1] as u8;
            row[x * 4 + 2] = c[2] as u8;
        }
    });
    Image { width: out_w, height: out_h, rgba, stats: None, depth: None, pick: None }
}

/// The lens bend of `lens_warp`, applied to a depth buffer (nearest sample, so depths never blend).
pub fn lens_warp_depth(depth: &mut [f32], w: usize, h: usize, horizon: f32, curve: f32) {
    let src = depth.to_vec();
    let amp = curve.clamp(-1.0, 1.0) * h as f32 * 0.22;
    let den = (w.max(2) - 1) as f32;
    let blend_top = (horizon - 40.0 * h as f32 / 854.0).max(0.0);
    let span = 60.0 * h as f32 / 854.0;
    depth.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        let t = ((y as f32 - blend_top) / span).clamp(0.0, 1.0);
        let weight = t * t * (3.0 - 2.0 * t);
        for x in 0..w {
            let nx = x as f32 / den * 2.0 - 1.0;
            let sy = (y as f32 - amp * nx * nx * weight).clamp(0.0, (h - 1) as f32);
            row[x] = src[(sy.round() as usize).min(h - 1) * w + x];
        }
    });
}

/// Depth at output size, one art pixel per px x px block.
pub fn upscale_depth(d: &[f32], w: usize, h: usize, px: usize, out_w: usize, out_h: usize) -> Vec<f32> {
    let mut out = vec![f32::INFINITY; out_w * out_h];
    out.par_chunks_mut(out_w).enumerate().for_each(|(y, row)| {
        let sy = (y / px).min(h - 1);
        for (x, v) in row.iter_mut().enumerate() { *v = d[sy * w + (x / px).min(w - 1)]; }
    });
    out
}
