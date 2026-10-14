//! Post-processing and style: bloom, tone mapping, grading, outlines, lens bend, palettes, upscale.

use super::palette::{bayer, PaletteLut};
use super::raster::{id, GPixel};
use super::render::Image;
use super::texture::lin_to_srgb;
use super::view::View;
use crate::scene::{Dither, Outline, Scene};
use rayon::prelude::*;

/// HDR linear light to display RGB in 0..255 (as f32 so later stages keep precision).
/// (`gbuf` is the frame's, to tell the sky from the world.)
pub fn finish(view: &View, scene: &Scene, phase: f32, hdr: &[[f32; 3]], gbuf: &[GPixel], post_on: bool) -> Vec<[f32; 3]> {
    let (w, h) = (view.width, view.height);
    let p = &scene.post;
    let flare = FlareK::of(scene, post_on);
    let exposure = if post_on { p.exposure.max(0.0) } else { 1.0 };
    let glow = BloomK { exposure, sky: sky_threshold(scene), hy: view.horizon_px };
    let bloom = if post_on && (p.bloom > 0.001 || flare.on()) { Some(bloom_buffer(hdr, gbuf, w, h, p.bloom.max(0.0), glow, flare)) } else { None };
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
                for k in 0..3 { c[k] += s[k]; }
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

/// How much of a pixel's light the lens scatters, by its luminance as exposed: none from
/// anything dimmer than `th`, all of what is 1.2 brighter than that. In the world `th` is
/// BLOOM_TH, white paper, so a bright scene does not fog the picture.
#[inline]
fn bright_pass(l: f32, th: f32) -> f32 {
    let t = ((l - th) / 1.2).clamp(0.0, 1.0);
    t * t.sqrt()
}
pub(crate) const BLOOM_TH: f32 = 0.8;

/// What decides which pixels glow: the exposure (a scene exposed up glows sooner), and the
/// threshold of the sky above row `hy`.
#[derive(Clone, Copy)]
pub(crate) struct BloomK { pub exposure: f32, pub sky: f32, pub hy: f32 }

/// The bloom threshold of a scene's sky. What glows in a sky is what outshines the sky round
/// it: against a day sky only the sun and what it lights to white, against a dark one the
/// moon, an aurora, the brighter stars and nebulae, though none of them is as bright as paper
/// in daylight.
pub(crate) fn sky_threshold(scene: &Scene) -> f32 {
    use super::texture::rgb_lin;
    let lum = |c: [u8; 3]| { let c = rgb_lin(c); 0.3 * c[0] + 0.55 * c[1] + 0.15 * c[2] };
    // (Deep space keeps the world's: its discs, tunnels and star fields are lit to glow as they are.)
    if !scene.sky.enabled || scene.sky.space.enabled || scene.sky.tunnel.enabled { return BLOOM_TH; }
    (0.06 + 2.5 * lum(scene.sky.top).max(lum(scene.sky.horizon))).min(BLOOM_TH)
}

/// The share of the bloom in each of its five blurs, tightest first.
pub(crate) const BLOOM_W: [f32; 5] = [0.30, 0.25, 0.20, 0.15, 0.10];
/// Bloom's overall strength at `post.bloom` 1.
pub(crate) const BLOOM_GAIN: f32 = 1.5;

/// A lens flare's parts (FlareStyle::parts) and strength.
#[derive(Clone, Copy, Default)]
pub(crate) struct FlareK { pub rays: u32, pub ghosts: bool, pub amt: f32 }

impl FlareK {
    pub(crate) fn of(scene: &Scene, post_on: bool) -> FlareK {
        let f = &scene.post.flare;
        let (rays, ghosts) = f.style.parts();
        if !post_on || f.amount <= 0.0 { FlareK::default() } else { FlareK { rays, ghosts, amt: f.amount.min(4.0) } }
    }
    pub(crate) fn on(&self) -> bool { self.rays != 0 || self.ghosts }
}

/// Bright light scattered in the lens, at quarter resolution: the bright parts blurred five times
/// over, each blur twice as wide as the last, and summed (a tight glow round every light that
/// falls off into a wide veil), with the flare's rays and ghosts added.
fn bloom_buffer(hdr: &[[f32; 3]], gbuf: &[GPixel], w: usize, h: usize, bloom: f32, glow: BloomK, flare: FlareK) -> Vec<[f32; 3]> {
    let (qw, qh) = (w.div_ceil(4), h.div_ceil(4));
    let mut q = vec![[0.0f32; 3]; qw * qh];
    q.par_iter_mut().enumerate().for_each(|(i, e)| {
        let (qx, qy) = (i % qw, i / qw);
        let mut acc = [0.0f32; 3];
        let mut n = 0.0;
        for y in qy * 4..(qy * 4 + 4).min(h) {
            for x in qx * 4..(qx * 4 + 4).min(w) {
                let c = hdr[y * w + x];
                let th = if (y as f32) < glow.hy && gbuf[y * w + x].id == id::NONE { glow.sky } else { BLOOM_TH };
                let k = bright_pass((0.3 * c[0] + 0.55 * c[1] + 0.15 * c[2]) * glow.exposure, th);
                for j in 0..3 { acc[j] += c[j] * k; }
                n += 1.0;
            }
        }
        *e = [acc[0] / n, acc[1] / n, acc[2] / n];
    });
    // Rays come from the bright parts as they are, so they stay fine; ghosts from a soft blur.
    let mut sum = if flare.rays != 0 {
        (0..qw * qh).into_par_iter().map(|i| flare_rays(&q, qw, qh, i % qw, i / qw, flare)).collect()
    } else { vec![[0.0f32; 3]; qw * qh] };
    for (k, wk) in BLOOM_W.iter().enumerate() {
        q = blur(&blur(&q, qw, qh, true, 1 << k), qw, qh, false, 1 << k);
        let g = wk * bloom * BLOOM_GAIN;
        sum.iter_mut().zip(&q).for_each(|(s, c)| for j in 0..3 { s[j] += c[j] * g });
        if k == 2 && flare.ghosts {
            let add: Vec<[f32; 3]> = (0..qw * qh).into_par_iter().map(|i| flare_ghosts(&q, qw, qh, i % qw, i / qw, flare)).collect();
            sum.iter_mut().zip(&add).for_each(|(s, c)| for j in 0..3 { s[j] += c[j] });
        }
    }
    sum
}

/// One pass of a five-tap blur with its taps `stride` apart (a B-spline, so repeated at doubling
/// strides it stays smooth: no grid shows).
fn blur(src: &[[f32; 3]], w: usize, h: usize, horizontal: bool, stride: i64) -> Vec<[f32; 3]> {
    const K: [f32; 5] = [0.0625, 0.25, 0.375, 0.25, 0.0625];
    let mut out = vec![[0.0f32; 3]; w * h];
    out.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        for x in 0..w {
            let mut acc = [0.0f32; 3];
            for (t, k) in K.iter().enumerate() {
                let o = (t as i64 - 2) * stride;
                let (sx, sy) = if horizontal { ((x as i64 + o).clamp(0, w as i64 - 1), y as i64) } else { (x as i64, (y as i64 + o).clamp(0, h as i64 - 1)) };
                let c = src[sy as usize * w + sx as usize];
                for j in 0..3 { acc[j] += c[j] * k; }
            }
            row[x] = acc;
        }
    });
    out
}

/// A quarter image read between its pixels; nothing outside it.
#[inline]
fn q_at(q: &[[f32; 3]], qw: usize, qh: usize, fx: f32, fy: f32) -> [f32; 3] {
    if fx < 0.0 || fy < 0.0 || fx > qw as f32 - 1.0 || fy > qh as f32 - 1.0 { return [0.0; 3]; }
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

/// The directions of a flare's rays (unit vectors, y down), their reach as a share of the quarter
/// image's height or width, their strength and their colour.
pub(crate) fn flare_lines(rays: u32) -> &'static [([f32; 2], f32, f32, [f32; 3])] {
    // cos and sin of 20 and 110 degrees; of 15, 75 and 135; and straight across.
    const STAR: [([f32; 2], f32, f32, [f32; 3]); 2] = [([0.939_692_6, -0.342_020_1], 0.07, 0.5, [1.0, 0.96, 0.9]), ([0.342_020_1, 0.939_692_6], 0.07, 0.5, [1.0, 0.96, 0.9])];
    const BURST: [([f32; 2], f32, f32, [f32; 3]); 3] = [([0.965_925_8, -0.258_819], 0.06, 0.4, [1.0, 0.96, 0.9]), ([0.258_819, -0.965_925_8], 0.06, 0.4, [1.0, 0.96, 0.9]), ([-0.707_106_8, -0.707_106_8], 0.06, 0.4, [1.0, 0.96, 0.9])];
    const STREAK: [([f32; 2], f32, f32, [f32; 3]); 1] = [([1.0, 0.0], 0.3, 1.0, [0.45, 0.65, 1.0])];
    match rays { 4 => &STAR, 6 => &BURST, 1 => &STREAK, _ => &[] }
}

/// A flare's rays at quarter pixel (x, y): along each line through it, the bright light either
/// side, fading with distance.
fn flare_rays(q: &[[f32; 3]], qw: usize, qh: usize, x: usize, y: usize, f: FlareK) -> [f32; 3] {
    const STEP: f32 = 1.5;
    let mut out = [0.0f32; 3];
    for (dir, reach, gain, tint) in flare_lines(f.rays) {
        let len = (reach * if f.rays == 1 { qw as f32 } else { qh as f32 }).max(2.0);
        let n = (3.0 * len / STEP).ceil() as i32;
        let mut acc = [0.0f32; 3];
        for t in 1..=n {
            let s = t as f32 * STEP;
            let wt = (-s / len).exp();
            for sign in [-1.0f32, 1.0] {
                let c = q_at(q, qw, qh, x as f32 + sign * s * dir[0], y as f32 + sign * s * dir[1]);
                for j in 0..3 { acc[j] += c[j] * wt; }
            }
        }
        let g = gain * f.amt * STEP / len;
        for j in 0..3 { out[j] += acc[j] * g * tint[j]; }
    }
    out
}

/// A flare's ghosts: reflections of the bright light across the middle of the frame, at (scale,
/// strength, colour); the last is a ring.
pub(crate) const GHOSTS: [(f32, f32, [f32; 3]); 5] = [
    (1.0, 0.30, [1.0, 0.8, 0.6]), (0.55, 0.22, [0.6, 1.0, 0.7]), (1.7, 0.16, [0.6, 0.7, 1.0]), (0.25, 0.20, [1.0, 0.6, 0.9]), (-0.4, 0.14, [0.8, 0.9, 1.0]),
];

fn flare_ghosts(q: &[[f32; 3]], qw: usize, qh: usize, x: usize, y: usize, f: FlareK) -> [f32; 3] {
    let (cx, cy) = (0.5 * (qw as f32 - 1.0), 0.5 * (qh as f32 - 1.0));
    let (dx, dy) = (cx - x as f32, cy - y as f32);
    let half = (cx * cx + cy * cy).sqrt().max(1.0);
    let mut out = [0.0f32; 3];
    for (s, g, tint) in GHOSTS {
        let (px, py) = (cx + dx * s, cy + dy * s);
        // Ghosts fade towards the frame's corners.
        let r = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt() / half;
        let fade = 1.0 - ((r - 0.55) / 0.4).clamp(0.0, 1.0);
        if fade <= 0.0 { continue; }
        let c = q_at(q, qw, qh, px, py);
        for j in 0..3 { out[j] += c[j] * g * fade * tint[j] * f.amt; }
    }
    // The ring: the light seen a fixed distance towards the middle.
    let d = (dx * dx + dy * dy).sqrt();
    if d > 1.0 {
        let k = 0.42 * qh as f32 / d;
        let c = q_at(q, qw, qh, x as f32 + dx * k, y as f32 + dy * k);
        for j in 0..3 { out[j] += c[j] * 0.12 * [0.7, 0.8, 1.0][j] * f.amt; }
    }
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
    let is_obj = |g: &GPixel| g.id >= id::PROP && !id::is_rail(g.id) && g.depth < INK_DISTANCE;
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
