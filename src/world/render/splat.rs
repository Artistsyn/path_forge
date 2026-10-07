//! Splats: everything drawn as small shapes over the shaded frame — grass tufts, flames, mist
//! wisps, particles, precipitation and its splashes and curtains, drips, blown sand. Each pass
//! emits its shapes in drawing order and one list draws them, on the CPU here or on the GPU
//! (`gpu/splat.wgsl`), so both engines draw exactly what the passes set up.

use super::*;

#[derive(Clone, Copy)]
pub(super) enum Shape {
    /// A blade of grass: `steps` pixels up a curve; writes the G-buffer (depth and id).
    Blade { bx: f32, sy: f32, lean: f32, len: f32, steps: i64, col: [f32; 3] },
    /// A flame's halo, added.
    Halo { sx: f32, sy: f32, gr: f32, glow: [f32; 3] },
    /// A bright orb where a flame has no colours, added.
    Orb { sx: f32, sy: f32, r: f32, glow: [f32; 3], fade: f32 },
    /// A flame's teardrop body, added.
    Body { sx: f32, cy: f32, rx: f32, ry: f32, lin: [[f32; 3]; 4], fade: f32 },
    /// A soft ellipse of mist over the pixel box x0..=x1, y0..=y1.
    Wisp { sx: f32, sy: f32, rx: f32, ry: f32, x0: i64, x1: i64, y0: i64, y1: i64, lit: [f32; 3], alpha: f32 },
    /// A particle: a dot, or a short streak down (`streak`) or sideways (`sideways`).
    Mote { sx: i64, sy: i64, ri: i64, dy_len: i64, dx_len: i64, r: f32, sideways: bool, streak: bool, additive: bool, emissive: bool, lit: [f32; 3], a: f32, life: f32 },
    /// A soft streak from (x0, y0) to (x1, y1), brighter toward its head.
    Streak { x0: f32, y0: f32, x1: f32, y1: f32, col: [f32; 3], alpha: f32, width: f32 },
    /// A soft round dot of radius `r` pixels.
    Dot { sx: f32, sy: f32, r: f32, col: [f32; 3], alpha: f32 },
    /// Far curtains of rain over the whole frame, added.
    Curtain { amt: f32, slant: f32, cyc: f32, seed: u32, col: [f32; 3], tphase: f32 },
}

#[derive(Clone, Copy)]
pub(super) struct Splat {
    /// Camera depth, for the depth test.
    pub z: f32,
    /// Drawn only where the G-buffer at (x, y) shows open ground about `z` away (splashes).
    pub gate: Option<(i64, i64, f32)>,
    pub shape: Shape,
}

pub(super) fn streak(out: &mut Vec<Splat>, z: f32, (x0, y0): (f32, f32), (x1, y1): (f32, f32), col: [f32; 3], alpha: f32, width: f32) {
    out.push(Splat { z, gate: None, shape: Shape::Streak { x0, y0, x1, y1, col, alpha, width } });
}

pub(super) fn dot(out: &mut Vec<Splat>, z: f32, (sx, sy): (f32, f32), r: f32, col: [f32; 3], alpha: f32) {
    out.push(Splat { z, gate: None, shape: Shape::Dot { sx, sy, r, col, alpha } });
}

impl Splat {
    /// Whether the gate (if any) lets this splat draw.
    fn open(&self, gbuf: &[GPixel], w: usize) -> bool {
        let Some((x, y, cz)) = self.gate else { return true };
        let g = &gbuf[y as usize * w + x as usize];
        g.id == id::GROUND && (g.depth - cz).abs() <= 0.15 * cz + 0.1
    }

    pub fn draw(&self, gbuf: &mut [GPixel], hdr: &mut [[f32; 3]], w: usize, h: usize) {
        if !self.open(gbuf, w) { return; }
        let z = self.z;
        match self.shape {
            Shape::Blade { bx, sy, lean, len, steps, col } => {
                for st in 0..steps {
                    let t = st as f32 / steps as f32;
                    let (px, py) = ((bx + lean * len * t * t * 0.5) as i64, (sy - len * t) as i64);
                    if px < 0 || py < 0 || px as usize >= w || py as usize >= h { continue; }
                    let i = py as usize * w + px as usize;
                    if z >= gbuf[i].depth + 0.05 { continue; }
                    hdr[i] = scale3(col, 0.7 + 0.5 * t);
                    gbuf[i].id = id::TUFT;
                    gbuf[i].depth = z;
                }
            }
            Shape::Halo { sx, sy, gr, glow } => {
                for y in (sy - gr) as i64..=(sy + gr) as i64 {
                    for x in (sx - gr) as i64..=(sx + gr) as i64 {
                        if x < 0 || y < 0 || x as usize >= w || y as usize >= h { continue; }
                        let i = y as usize * w + x as usize;
                        if gbuf[i].depth < z - 0.3 { continue; }
                        let dd = ((x as f32 + 0.5 - sx).powi(2) + (y as f32 + 0.5 - sy).powi(2)).sqrt() / gr;
                        if dd < 1.0 { hdr[i] = add3(hdr[i], scale3(glow, (1.0 - dd).powi(2))); }
                    }
                }
            }
            Shape::Orb { sx, sy, r, glow, fade } => {
                for y in (sy - r) as i64..=(sy + r) as i64 { for x in (sx - r) as i64..=(sx + r) as i64 {
                    if x < 0 || y < 0 || x as usize >= w || y as usize >= h { continue; }
                    let i = y as usize * w + x as usize;
                    if gbuf[i].depth < z - 0.1 { continue; }
                    let dd = ((x as f32 + 0.5 - sx).powi(2) + (y as f32 + 0.5 - sy).powi(2)).sqrt() / r;
                    if dd < 1.0 { hdr[i] = add3(hdr[i], scale3(glow, 3.0 * (1.0 - dd) * fade)); }
                }}
            }
            Shape::Body { sx, cy, rx, ry, lin, fade } => {
                for y in (cy - ry) as i64..=(cy + ry) as i64 { for x in (sx - rx) as i64..=(sx + rx) as i64 {
                    if x < 0 || y < 0 || x as usize >= w || y as usize >= h { continue; }
                    let i = y as usize * w + x as usize;
                    if gbuf[i].depth < z - 0.1 { continue; }
                    let (dx, dy) = ((x as f32 + 0.5 - sx) / rx, (y as f32 + 0.5 - cy) / ry);
                    // Teardrop: narrower toward the tip.
                    let taper = 1.0 + (-dy).max(0.0) * 0.8;
                    let dd = ((dx * taper).powi(2) + dy * dy).sqrt();
                    if dd >= 1.0 { continue; }
                    let (col, a) = if dd < 0.35 { (mix3(lin[0], lin[1], dd / 0.35), 1.0) }
                        else if dd < 0.7 { (mix3(lin[1], lin[2], (dd - 0.35) / 0.35), 0.85) }
                        else { (mix3(lin[2], lin[3], (dd - 0.7) / 0.3), 0.85 * (1.0 - (dd - 0.7) / 0.3)) };
                    hdr[i] = add3(hdr[i], scale3(col, a * 2.2 * fade));
                }}
            }
            Shape::Wisp { sx, sy, rx, ry, x0, x1, y0, y1, lit, alpha } => {
                for py in y0..=y1 {
                    for px in x0..=x1 {
                        let i = py as usize * w + px as usize;
                        if z >= gbuf[i].depth + 0.5 { continue; }
                        let q = ((px as f32 + 0.5 - sx) / rx).powi(2) + ((py as f32 + 0.5 - sy) / ry).powi(2);
                        if q >= 1.0 { continue; }
                        hdr[i] = mix3(hdr[i], lit, alpha * (1.0 - q).powi(2));
                    }
                }
            }
            Shape::Mote { sx, sy, ri, dy_len, dx_len, r, sideways, streak, additive, emissive, lit, a, life } => {
                for oy in -ri..=(ri + dy_len) {
                    for ox in -ri - dx_len..=ri {
                        let (px, py) = (sx + ox, sy + oy);
                        if px < 0 || py < 0 || px as usize >= w || py as usize >= h { continue; }
                        let i = py as usize * w + px as usize;
                        if z >= gbuf[i].depth { continue; }
                        let dd = (if sideways { 0.0 } else { (ox * ox) as f32 } + if streak { 0.0 } else { (oy * oy) as f32 }).sqrt() / (r + 0.5);
                        if dd >= 1.0 { continue; }
                        let k = (1.0 - dd) * a * life;
                        hdr[i] = if additive { add3(hdr[i], scale3(lit, k * if emissive { 1.0 } else { 0.6 })) } else { mix3(hdr[i], lit, k) };
                    }
                }
            }
            Shape::Streak { x0, y0, x1, y1, col, alpha, width } => {
                let (dx, dy) = (x1 - x0, y1 - y0);
                let steps = dx.abs().max(dy.abs()).ceil().max(1.0) as i64;
                let half = (width * 0.5).max(0.5);
                let reach = half.ceil() as i64;
                for s in 0..=steps {
                    let t = s as f32 / steps as f32;
                    let (px, py) = (x0 + dx * t, y0 + dy * t);
                    let yi = py.floor() as i64;
                    if yi < 0 || yi >= h as i64 { continue; }
                    let k = alpha * (0.35 + 0.65 * t);
                    for ox in -reach..=reach + 1 {
                        let xi = px.floor() as i64 + ox;
                        if xi < 0 || xi >= w as i64 { continue; }
                        let cover = (half + 0.5 - (xi as f32 + 0.5 - px).abs()).clamp(0.0, 1.0);
                        if cover <= 0.0 { continue; }
                        let i = yi as usize * w + xi as usize;
                        if z >= gbuf[i].depth { continue; }
                        hdr[i] = mix3(hdr[i], col, k * cover);
                    }
                }
            }
            Shape::Dot { sx, sy, r, col, alpha } => {
                let ri = r.ceil() as i64 + 1;
                for oy in -ri..=ri {
                    for ox in -ri..=ri {
                        let (xi, yi) = (sx.floor() as i64 + ox, sy.floor() as i64 + oy);
                        if xi < 0 || yi < 0 || xi >= w as i64 || yi >= h as i64 { continue; }
                        let i = yi as usize * w + xi as usize;
                        if z >= gbuf[i].depth { continue; }
                        let dd = ((xi as f32 + 0.5 - sx).powi(2) + (yi as f32 + 0.5 - sy).powi(2)).sqrt() / (r + 0.5);
                        if dd >= 1.0 { continue; }
                        hdr[i] = mix3(hdr[i], col, alpha * (1.0 - dd));
                    }
                }
            }
            Shape::Curtain { amt, slant, cyc, seed, col, tphase } => {
                let gbuf = &*gbuf;
                hdr.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
                    for (x, px) in row.iter_mut().enumerate() {
                        let g = &gbuf[y * w + x];
                        let far = if g.id == id::NONE { 1.0 } else { smoothstep(25.0, 60.0, g.depth) };
                        if far <= 0.0 { continue; }
                        let col_id = ((x as f32 - slant * y as f32) / 2.0).floor() as i64;
                        let speed = 1.0 + (hash(seed ^ 0x72, col_id) % 2) as f32;
                        let s = (y as f32 / h as f32 * 2.5 + hf(seed ^ 0x71, col_id) - speed * cyc * tphase).rem_euclid(1.0);
                        let st = smoothstep(0.75, 1.0, s) * (0.4 + 0.6 * hf(seed ^ 0x73, col_id));
                        *px = add3(*px, scale3(col, amt * st * far));
                    }
                });
            }
        }
    }
}

/// Draw a list in order.
pub(super) fn draw_all(list: &[Splat], gbuf: &mut [GPixel], hdr: &mut [[f32; 3]], w: usize, h: usize) {
    for s in list { s.draw(gbuf, hdr, w, h); }
}

/// The splat for the GPU's splat pass, and the pixel box it can touch (inclusive, for binning).
#[cfg(feature = "gpu")]
impl Splat {
    pub fn gpu(&self, w: usize, h: usize) -> (super::super::gpu::SplatGpu, (i64, i64, i64, i64)) {
        let mut f = [0.0f32; 19];
        let bits = |v: i64| f32::from_bits(v as i32 as u32);
        let set3 = |f: &mut [f32; 19], k: usize, c: [f32; 3]| f[k..k + 3].copy_from_slice(&c);
        let (kind, bbox) = match self.shape {
            Shape::Blade { bx, sy, lean, len, steps, col } => {
                (f[0], f[1], f[2], f[3], f[4]) = (bx, sy, lean, len, bits(steps));
                set3(&mut f, 5, col);
                let (mut b, mut any) = ((i64::MAX, i64::MAX, i64::MIN, i64::MIN), false);
                for st in 0..steps {
                    let t = st as f32 / steps as f32;
                    let (px, py) = ((bx + lean * len * t * t * 0.5) as i64, (sy - len * t) as i64);
                    b = (b.0.min(px), b.1.min(py), b.2.max(px), b.3.max(py));
                    any = true;
                }
                (0, if any { (b.0 - 1, b.1 - 1, b.2 + 1, b.3 + 1) } else { (1, 1, 0, 0) })
            }
            Shape::Halo { sx, sy, gr, glow } => {
                (f[0], f[1], f[2]) = (sx, sy, gr);
                set3(&mut f, 3, glow);
                (1, ((sx - gr) as i64, (sy - gr) as i64, (sx + gr) as i64, (sy + gr) as i64))
            }
            Shape::Orb { sx, sy, r, glow, fade } => {
                (f[0], f[1], f[2], f[6]) = (sx, sy, r, fade);
                set3(&mut f, 3, glow);
                (2, ((sx - r) as i64, (sy - r) as i64, (sx + r) as i64, (sy + r) as i64))
            }
            Shape::Body { sx, cy, rx, ry, lin, fade } => {
                (f[0], f[1], f[2], f[3], f[16]) = (sx, cy, rx, ry, fade);
                for (k, c) in lin.iter().enumerate() { set3(&mut f, 4 + 3 * k, *c); }
                (3, ((sx - rx) as i64, (cy - ry) as i64, (sx + rx) as i64, (cy + ry) as i64))
            }
            Shape::Wisp { sx, sy, rx, ry, x0, x1, y0, y1, lit, alpha } => {
                (f[0], f[1], f[2], f[3], f[4], f[5], f[6], f[7], f[11]) = (sx, sy, rx, ry, bits(x0), bits(x1), bits(y0), bits(y1), alpha);
                set3(&mut f, 8, lit);
                (4, (x0, y0, x1, y1))
            }
            Shape::Mote { sx, sy, ri, dy_len, dx_len, r, sideways, streak, additive, emissive, lit, a, life } => {
                (f[0], f[1], f[2], f[3], f[4], f[5]) = (bits(sx), bits(sy), bits(ri), bits(dy_len), bits(dx_len), r);
                f[6] = f32::from_bits(sideways as u32 | (streak as u32) << 1 | (additive as u32) << 2 | (emissive as u32) << 3);
                set3(&mut f, 7, lit);
                (f[10], f[11]) = (a, life);
                (5, (sx - ri - dx_len, sy - ri, sx + ri, sy + ri + dy_len))
            }
            Shape::Streak { x0, y0, x1, y1, col, alpha, width } => {
                let (dx, dy) = (x1 - x0, y1 - y0);
                let steps = dx.abs().max(dy.abs()).ceil().max(1.0) as i64;
                let half = (width * 0.5).max(0.5);
                let reach = half.ceil() as i64;
                (f[0], f[1], f[2], f[3], f[7], f[8], f[9], f[10]) = (x0, y0, x1, y1, alpha, bits(steps), half, bits(reach));
                set3(&mut f, 4, col);
                let (xa, xb) = (x0.min(x1).floor() as i64, x0.max(x1).floor() as i64);
                let (ya, yb) = (y0.min(y1).floor() as i64, y0.max(y1).floor() as i64);
                (6, (xa - reach - 2, ya - 1, xb + reach + 2, yb + 1))
            }
            Shape::Dot { sx, sy, r, col, alpha } => {
                let ri = r.ceil() as i64 + 1;
                (f[0], f[1], f[2], f[6], f[7]) = (sx, sy, r, alpha, bits(ri));
                set3(&mut f, 3, col);
                let (cx, cy) = (sx.floor() as i64, sy.floor() as i64);
                (7, (cx - ri, cy - ri, cx + ri, cy + ri))
            }
            Shape::Curtain { amt, slant, cyc, seed, col, tphase } => {
                (f[0], f[1], f[2], f[3], f[7]) = (amt, slant, cyc, f32::from_bits(seed), tphase);
                set3(&mut f, 4, col);
                (8, (0, 0, w as i64 - 1, h as i64 - 1))
            }
        };
        let (gx, gy, gcz) = self.gate.map_or((-1, -1, 0.0), |(x, y, z)| (x as i32, y as i32, z));
        (super::super::gpu::SplatGpu { kind, z: self.z, gx, gy, gcz, f }, bbox)
    }
}

/// A list as the GPU's splat pass takes it: the splats and their per-tile lists.
#[cfg(feature = "gpu")]
pub(super) fn gpu_list(list: &[Splat], w: usize, h: usize) -> (Vec<super::super::gpu::SplatGpu>, Vec<u32>) {
    let (gs, boxes): (Vec<_>, Vec<_>) = list.par_iter().map(|s| s.gpu(w, h)).unzip();
    let bins = super::super::gpu::bin_tiles(w, h, &boxes);
    (gs, bins)
}
