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
    Body { sx: f32, cy: f32, rx: f32, ry: f32, lin: [[f32; 3]; 4], fade: f32, ph: [f32; 2] },
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
    /// A waterfall's falling water (`fall_at`), over the box its curtain spans: its middle's
    /// column `cx`, the rows of its lip and foot, its half width there (pixels).
    Fall { cx: f32, top: f32, foot: f32, hw_top: f32, hw_foot: f32, k: FallK },
}

/// A waterfall's curtain: the water's look and its motion this frame.
#[derive(Clone, Copy)]
pub(super) struct FallK {
    pub seed: u32,
    /// Noise cells down the whole fall (in falling time), for the coarse and the fine layer, and
    /// how many cells each is carried down in one loop (whole, so the loop closes; also the
    /// period of the noise down the fall).
    pub ky: [f32; 2],
    pub n: [u32; 2],
    /// Where in the loop's time the frame is (0..1).
    pub t: f32,
    /// Deep (clear) water and aerated white water, lit.
    pub deep: [f32; 3],
    pub foam: [f32; 3],
    pub strength: f32,
    /// 2 g H / u0²: how much faster the water falls at the foot than over the lip.
    pub kk: f32,
}

/// Smooth value noise at (x, y), periodic in y with `period` cells, 0..1.
pub(super) fn vnoise(x: f32, y: f32, period: i64, seed: u32) -> f32 {
    let (ix, iy) = (x.floor(), y.floor());
    let (fx, fy) = (x - ix, y - iy);
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let (ix, iy) = (ix as i64, iy as i64);
    let (r0, r1) = (iy.rem_euclid(period), (iy + 1).rem_euclid(period));
    let h = |a: i64, b: i64| hf(seed ^ (b as u32).wrapping_mul(0x9E37_79B1), a);
    let top = h(ix, r0) + (h(ix + 1, r0) - h(ix, r0)) * sx;
    let bot = h(ix, r1) + (h(ix + 1, r1) - h(ix, r1)) * sx;
    top + (bot - top) * sy
}

/// A waterfall's curtain at (u, v): u across it (-1..1 edge to edge), v down it (0 at the lip,
/// 1 at the foot). Water speeds up as it falls, so its patterns are carried down in falling time
/// (`tn`) and stretch lower down; it leaves the lip as a dark glassy tongue, breaks into white
/// aerated columns past its break-up, and frays at the edges and into spray toward the foot.
/// The colour there and how opaque it is, or None where there is no water.
pub(super) fn fall_at(u: f32, v: f32, k: &FallK) -> Option<([f32; 3], f32)> {
    if !(0.0..=1.0).contains(&v) || u.abs() > 1.1 { return None; }
    let tn = ((1.0 + k.kk * v).sqrt() - 1.0) / ((1.0 + k.kk).sqrt() - 1.0);
    let wob = std::f32::consts::TAU * (2.0 * k.t + 3.0 * v);
    let uu = u + 0.03 * v * wob.sin();
    let (p1, p2) = (k.n[0].max(1) as i64, k.n[1].max(1) as i64);
    let y1 = tn * k.ky[0] - k.n[0] as f32 * k.t;
    let y2 = tn * k.ky[1] - k.n[1] as f32 * k.t;
    // Columns where the lip lets more water over, and streaks racing down them.
    let col = smoothstep(0.3, 0.7, vnoise(uu * 4.0 + 5.0, 0.5, 1, k.seed ^ 0xC0));
    let s1 = 0.6 * vnoise(uu * 9.0, y1, p1, k.seed ^ 0xC1) + 0.4 * vnoise(uu * 20.0 + 3.0, 2.0 * y1, 2 * p1, k.seed ^ 0xC2);
    let s2 = vnoise(uu * 28.0 + 7.0, y2, p2, k.seed ^ 0xC3);
    let dens = col * (0.55 + 0.45 * s1) + 0.3 * s2 + 0.15;
    let aer = smoothstep(0.06, 0.4, v);
    let ne = vnoise(uu.signum() * 3.0 + 11.0, y1, p1, k.seed ^ 0xC4);
    let edge = 1.0 - smoothstep(0.72, 1.0, uu.abs() + 0.25 * v * (ne - 0.5));
    let thr = 0.12 + 0.4 * v * v;
    let mut a = smoothstep(thr - 0.1, thr + 0.1, dens * edge) * (1.0 - (-3.5 * dens).exp());
    let mut rgb = scale3(mix3(k.deep, k.foam, aer * (0.55 + 0.45 * s1)), 0.62 + 0.6 * s2);
    // The lip: a dark glassy tongue with the sky caught along its bend.
    let lip = 1.0 - smoothstep(0.0, 0.07, v);
    let band = (-((v - 0.025) / 0.012).powi(2)).exp();
    rgb = add3(mix3(rgb, scale3(k.deep, 0.75), lip), scale3(k.foam, 0.5 * band));
    a += (0.92 * edge - a) * lip;
    // Glints on the clear water near the top.
    let gy = (y1 * 3.0).floor() as i64;
    if hf(k.seed ^ 0xC5 ^ (gy.rem_euclid(3 * p1) as u32).wrapping_mul(0x85EB_CA6B), (uu * 30.0).floor() as i64) > 0.97 {
        rgb = add3(rgb, scale3(k.foam, 0.6 * (1.0 - 0.5 * aer)));
    }
    // Into the spray at the foot.
    let foot = smoothstep(0.85, 1.0, v);
    rgb = mix3(rgb, k.foam, foot);
    a *= 1.0 - 0.4 * foot;
    Some((rgb, a * k.strength))
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

/// A flame at (dx, dy) across its box (dx -1..1 side to side, dy -1 at its tip .. 1 at its
/// base), its two wavers at `ph`, in its four colours (white-hot heart .. red edges): the light
/// it adds there, and how much, or None outside it. It is widest low down and sways more the
/// higher up it is; tongues lick off its top; its base burns a little blue.
pub(super) fn flame_at(dx: f32, dy: f32, ph: [f32; 2], lin: &[[f32; 3]; 4]) -> Option<([f32; 3], f32)> {
    let hh = (1.0 - dy) * 0.5;
    if !(0.0..1.0).contains(&hh) { return None; }
    let sway = hh * hh * (0.28 * (ph[0] + 3.0 * hh).sin() + 0.12 * (ph[1] + 7.0 * hh).sin());
    // Rounded at the base, widest low down, drawn out to the tip.
    let width = (1.0 - hh).sqrt() * (0.75 + 0.6 * hh) / 0.9 * (hh / 0.16).min(1.0).sqrt();
    let mut q = (dx - sway).abs() / width.max(1e-3);
    // Tongues licking off the top.
    if hh > 0.5 { q += (hh - 0.5) * 1.1 * (0.5 + 0.5 * (ph[1] * 1.3 + dx * 6.0 + hh * 9.0).sin()); }
    if q >= 1.0 { return None; }
    let a = ((1.0 - q) / 0.3).min(1.0);
    let t = (1.0 - q) * (1.0 - 0.55 * hh);
    let mut col = if t > 0.62 { lin[0] } else if t > 0.42 { mix3(lin[1], lin[0], (t - 0.42) / 0.2) }
        else if t > 0.22 { mix3(lin[2], lin[1], (t - 0.22) / 0.2) } else { mix3(lin[3], lin[2], t / 0.22) };
    let blue = (1.0 - hh / 0.12).max(0.0) * ((q - 0.3) / 0.4).clamp(0.0, 1.0);
    col = mix3(col, [0.1, 0.2, 0.8], 0.3 * blue);
    // A flame is far brighter at its heart than a screen can show: the bloom spreads it.
    Some((col, a * 2.2 * (1.0 + 1.6 * t * t)))
}

/// How tight the bright heart of a lamp's halo is (as a share of its radius, squared).
const HALO_CORE: f32 = 0.035;

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
                        // Light scattered by the air round a lamp: brightest against it and falling away
                        // fast, as the inverse square does, into a long faint tail.
                        if dd < 1.0 { hdr[i] = add3(hdr[i], scale3(glow, (1.0 - dd).powi(2) * (HALO_CORE / (dd * dd + HALO_CORE)))); }
                    }
                }
            }
            Shape::Orb { sx, sy, r, glow, fade } => {
                for y in (sy - r) as i64..=(sy + r) as i64 { for x in (sx - r) as i64..=(sx + r) as i64 {
                    if x < 0 || y < 0 || x as usize >= w || y as usize >= h { continue; }
                    let i = y as usize * w + x as usize;
                    if gbuf[i].depth < z - 0.1 { continue; }
                    let dd = ((x as f32 + 0.5 - sx).powi(2) + (y as f32 + 0.5 - sy).powi(2)).sqrt() / r;
                    if dd < 1.0 { hdr[i] = add3(hdr[i], scale3(glow, 6.0 * (1.0 - dd).powi(2) * fade)); }
                }}
            }
            Shape::Body { sx, cy, rx, ry, lin, fade, ph } => {
                let bx = 1.4 * rx;
                for y in (cy - ry) as i64..=(cy + ry) as i64 { for x in (sx - bx) as i64..=(sx + bx) as i64 {
                    if x < 0 || y < 0 || x as usize >= w || y as usize >= h { continue; }
                    let i = y as usize * w + x as usize;
                    if gbuf[i].depth < z - 0.1 { continue; }
                    let (dx, dy) = ((x as f32 + 0.5 - sx) / rx, (y as f32 + 0.5 - cy) / ry);
                    if let Some((col, a)) = flame_at(dx, dy, ph, &lin) {
                        hdr[i] = add3(hdr[i], scale3(col, a * fade));
                    }
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
            Shape::Fall { cx, top, foot, hw_top, hw_foot, k } => {
                let hw = hw_top.max(hw_foot) * 1.1;
                let (xa, xb) = (((cx - hw).floor() as i64).max(0), ((cx + hw).ceil() as i64).min(w as i64 - 1));
                let (ya, yb) = ((top.floor() as i64).max(0), (foot.ceil() as i64).min(h as i64 - 1));
                for py in ya..=yb {
                    let v = (py as f32 + 0.5 - top) / (foot - top).max(1.0);
                    let half = hw_top + (hw_foot - hw_top) * v.clamp(0.0, 1.0);
                    for px in xa..=xb {
                        let i = py as usize * w + px as usize;
                        if z >= gbuf[i].depth { continue; }
                        let u = (px as f32 + 0.5 - cx) / half.max(0.5);
                        if let Some((c, a)) = fall_at(u, v, &k) { hdr[i] = mix3(hdr[i], c, a.clamp(0.0, 1.0)); }
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
            Shape::Body { sx, cy, rx, ry, lin, fade, ph } => {
                (f[0], f[1], f[2], f[3], f[16], f[17], f[18]) = (sx, cy, rx, ry, fade, ph[0], ph[1]);
                for (k, c) in lin.iter().enumerate() { set3(&mut f, 4 + 3 * k, *c); }
                (3, ((sx - 1.4 * rx) as i64, (cy - ry) as i64, (sx + 1.4 * rx) as i64, (cy + ry) as i64))
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
            Shape::Fall { cx, top, foot, hw_top, hw_foot, k } => {
                (f[0], f[1], f[2], f[3], f[4], f[5]) = (cx, top, foot, hw_top, hw_foot, f32::from_bits(k.seed));
                (f[6], f[7], f[8], f[9]) = (k.ky[0], k.ky[1], k.t, f32::from_bits(k.n[0] | k.n[1] << 16));
                set3(&mut f, 10, k.deep);
                set3(&mut f, 13, k.foam);
                (f[16], f[17]) = (k.strength, k.kk);
                let hw = hw_top.max(hw_foot) * 1.1;
                (9, ((cx - hw).floor() as i64, top.floor() as i64, (cx + hw).ceil() as i64, foot.ceil() as i64))
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
