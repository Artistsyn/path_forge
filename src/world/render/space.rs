//! Deep space in a world's sky: a dense starfield, nebulae, a galaxy band and planets, and space
//! continuing below the horizon (`sky.space.below`).
//!
//! Everything sits at infinity, so nothing moves as the camera walks; only a planet's spin
//! moves, in whole turns per loop. `sky.wgsl` repeats every function here step for step (the GPU
//! sky pass), with the numbers `space_k` and `PlanetK::pack` hand it.

use super::*;
use crate::scene::{Planet, PlanetKind, SpaceObject, SpaceObjectKind, StarStyle, TunnelKind};
use crate::world::looks::value_noise;

/// Planets drawn at most, and floats per planet in the sky pass's prims.
pub(super) const MAX_PLANETS: usize = 3;
pub(super) const PLANET_FLOATS: usize = 36;

/// A world's space sky for one frame.
#[derive(Clone, Copy, Debug)]
pub(super) struct SpaceK {
    pub below: bool,
    /// Pixel scale of the star cells (the frame's height / 854), the field's density and gain.
    pub scale: f32,
    pub stars: f32,
    pub sb: f32,
    /// How varied the stars' colours are (`Space::star_colors`).
    pub star_var: f32,
    /// The frame's middle column, the sky's height in pixels, the horizon and top rows.
    pub cx: f32,
    pub unit: f32,
    pub hy: f32,
    pub top: f32,
    pub sky_top: [f32; 3],
    pub sky_hor: [f32; 3],
    pub neb: f32,
    pub neb1: [f32; 3],
    pub neb2: [f32; 3],
    /// 1.6 / nebula_scale: noise cells per sky height.
    pub neb_f: f32,
    pub gal: f32,
    pub gal_col: [f32; 3],
    pub gal_c: f32,
    pub gal_s: f32,
    pub gal_w: f32,
    pub gal_h: f32,
    /// Where the core sits along the band, sky heights from the middle.
    pub gal_core: f32,
    /// Where this seed's sky starts in the noise.
    pub so: f32,
    /// Realistic stars (`StarStyle::Realistic`), and the rest of their numbers.
    pub real: bool,
    pub glow: f32,
    pub clumps: f32,
    pub spikes: f32,
    /// Lines of spikes through a bright star (2, 3 or 4).
    pub lines: f32,
    pub twinkle: f32,
    /// Whole twinkles per loop, and how far through the loop the frame is.
    pub tw_n: f32,
    pub tphase: f32,
    pub neb_dust: f32,
    pub gal_dust: f32,
    /// The objects far off in the sky (`objs[..n_obj]`).
    pub n_obj: usize,
    pub objs: [ObjK; MAX_OBJECTS],
}

/// Objects drawn at most, and floats each in the sky pass's prims.
pub(super) const MAX_OBJECTS: usize = 6;
pub(super) const OBJ_FLOATS: usize = 20;
/// Floats before the objects in the block `SpaceK::pack` writes.
pub(super) const SPACE_HEAD: usize = 12;

/// One object far off in the sky (`SpaceObject`), in sky units: x from the frame's middle and y
/// up from the horizon, in sky heights.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct ObjK {
    pub kind: f32,
    pub x: f32,
    pub y: f32,
    pub r: f32,
    pub c: f32,
    pub s: f32,
    pub aspect: f32,
    pub col: [[f32; 3]; 3],
    pub bright: f32,
    pub so: f32,
    /// How far out it shows, in its radii.
    pub reach: f32,
    /// How far round its `spin` has taken it this frame (radians).
    pub rot: f32,
}

impl ObjK {
    /// `fw` and `fhy` the frame's width and sky height in pixels, `hy` and `oy` its horizon and top rows.
    fn new(o: &SpaceObject, fw: f32, fhy: f32, hy: f32, oy: f32, tphase: f32) -> ObjK {
        let a = o.angle.to_radians();
        let kind = SpaceObjectKind::ALL.iter().position(|k| *k == o.kind).unwrap_or(0);
        let reach = [1.35, 1.45, 1.3, 1.5, 1.3, 1.25, 2.0, 1.6, 1.25, 1.7, 3.0, 2.0, 2.2, 2.6][kind];
        let (mut x, mut y) = ((o.pos[0] - 0.5) * fw / fhy, (hy - (oy + o.pos[1] * fhy)) / fhy);
        if o.kind == SpaceObjectKind::Ship && o.spin != 0 {
            // Across the whole sky along its way, `spin` times a loop, and back round unseen.
            let p = (o.spin.unsigned_abs() as f32 * tphase + hf(o.seed ^ 0x5A7, 0)).rem_euclid(1.0);
            let travel = 1.6 * fw / fhy + 4.0 * o.size.max(0.005);
            let sgn = if o.spin < 0 { -1.0 } else { 1.0 };
            x += a.cos() * sgn * (p - 0.5) * travel;
            y += a.sin() * sgn * (p - 0.5) * travel;
        }
        ObjK {
            kind: kind as f32,
            x,
            y,
            r: o.size.max(0.005),
            c: a.cos(),
            s: a.sin(),
            aspect: o.aspect.clamp(0.05, 1.0),
            col: [rgb_lin(o.colors[0]), rgb_lin(o.colors[1]), rgb_lin(o.colors[2])],
            bright: o.brightness.max(0.0),
            so: (hf(o.seed ^ 0x0B7, 0) * 61.0).floor(),
            reach,
            rot: TAU * o.spin as f32 * tphase,
        }
    }

    fn pack(&self) -> [f32; OBJ_FLOATS] {
        let c = &self.col;
        [
            self.kind, self.x, self.y, self.r, self.c, self.s, self.aspect,
            c[0][0], c[0][1], c[0][2], c[1][0], c[1][1], c[1][2], c[2][0], c[2][1], c[2][2],
            self.bright, self.so, self.reach, self.rot,
        ]
    }
}

impl SpaceK {
    /// What the sky pass reads beyond its uniforms: the realistic sky's numbers, then the objects.
    pub(super) fn pack(&self) -> Vec<f32> {
        let mut o = vec![
            self.real as u32 as f32, self.glow, self.clumps, self.spikes, self.lines, self.twinkle,
            self.tw_n, self.tphase, self.neb_dust, self.gal_dust, self.n_obj as f32, 0.0,
        ];
        debug_assert_eq!(o.len(), SPACE_HEAD);
        for ob in &self.objs[..self.n_obj] { o.extend(ob.pack()); }
        o
    }
}

pub(super) fn space_k(ctx: &Ctx) -> Option<SpaceK> {
    let sky = &ctx.scene.sky;
    let s = &sky.space;
    if !sky.enabled || !s.enabled { return None; }
    let v = &ctx.view;
    let hy = v.horizon_px.max(2.0);
    let (ox, oy) = (v.left as f32, v.top as f32);
    let (fw, fhy) = (v.width as f32 - 2.0 * ox, (hy - oy).max(2.0));
    let ang = s.galaxy_angle.to_radians();
    let objs: Vec<ObjK> = s.objects.iter().filter(|o| o.enabled && o.size > 0.0 && o.brightness > 0.0).take(MAX_OBJECTS)
        .map(|o| ObjK::new(o, fw, fhy, hy, oy, ctx.tphase)).collect();
    Some(SpaceK {
        below: s.below,
        scale: ((v.height as f32 - oy) / 854.0).max(0.25),
        stars: s.stars.clamp(0.0, 3.0),
        sb: s.star_brightness.max(0.0),
        star_var: s.star_colors.clamp(0.0, 1.0),
        cx: ox + v.sky_dx() + 0.5 * fw,
        unit: fhy,
        hy,
        top: oy,
        sky_top: rgb_lin(sky.top),
        sky_hor: rgb_lin(sky.horizon),
        neb: s.nebula.max(0.0),
        neb1: rgb_lin(s.nebula_colors[0]),
        neb2: rgb_lin(s.nebula_colors[1]),
        neb_f: 1.6 / s.nebula_scale.max(0.05),
        gal: s.galaxy.max(0.0),
        gal_col: rgb_lin(s.galaxy_color),
        gal_c: ang.cos(),
        gal_s: ang.sin(),
        gal_w: s.galaxy_width.max(0.01),
        gal_h: s.galaxy_height,
        gal_core: s.galaxy_core * 0.5 * fw / fhy,
        so: (hf(s.seed ^ 0x5A1, 0) * 97.0).floor(),
        real: s.star_style == StarStyle::Realistic,
        glow: s.star_glow.max(0.0),
        clumps: s.star_clumps.clamp(0.0, 1.0),
        spikes: s.star_spikes.max(0.0),
        lines: s.spike_shape.lines() as f32,
        twinkle: s.twinkle.clamp(0.0, 1.0),
        tw_n: s.twinkle_speed.max(1) as f32,
        tphase: ctx.tphase,
        neb_dust: s.nebula_dust.clamp(0.0, 1.0),
        gal_dust: s.galaxy_dust.clamp(0.0, 1.0),
        n_obj: objs.len(),
        objs: {
            let mut a = [ObjK::default(); MAX_OBJECTS];
            a[..objs.len()].copy_from_slice(&objs);
            a
        },
    })
}

/// `looks::hash2` with a salt: several independent numbers per cell.
pub(super) fn cell_hash(x: i32, y: i32, k: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343) ^ (y as u32).wrapping_mul(0xd816_3841) ^ k.wrapping_mul(0x9e37_79b9);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    (h & 0xffff) as f32 / 65535.0
}

pub(super) fn fbm4(x: f32, y: f32) -> f32 {
    let (mut x, mut y, mut amp, mut s) = (x, y, 0.5, 0.0);
    for _ in 0..4 {
        s += amp * value_noise(x, y);
        x = x * 2.03 + 7.1;
        y = y * 2.03 + 7.1;
        amp *= 0.5;
    }
    s
}

/// Value noise repeating every `n` along x (round a planet).
fn noise_wrap(x: f32, y: f32, n: i32) -> f32 {
    let (xf, yf) = (x.floor(), y.floor());
    let (fx, fy) = (x - xf, y - yf);
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let x0 = (xf as i32).rem_euclid(n);
    let x1 = (x0 + 1).rem_euclid(n);
    let yi = yf as i32;
    let h = |a: i32, b: i32| cell_hash(a, b, 0x51);
    let a = h(x0, yi) + (h(x1, yi) - h(x0, yi)) * sx;
    let b = h(x0, yi + 1) + (h(x1, yi + 1) - h(x0, yi + 1)) * sx;
    a + (b - a) * sy
}

/// Four octaves over a planet's (u 0..1 round it, v 0..1 pole to pole): n cells round at the
/// first, vs down it.
pub(super) fn fbm_wrap(u: f32, v: f32, n: i32, vs: f32) -> f32 {
    let (mut n, mut y, mut amp, mut s) = (n, v * vs, 0.5, 0.0);
    for k in 0..4 {
        s += amp * noise_wrap(u * n as f32, y + k as f32 * 7.1, n);
        n *= 2;
        y *= 2.0;
        amp *= 0.5;
    }
    s
}

/// One layer of the starfield: at most one star per square cell, kept clear of the cell's edges.
fn star_layer(px: f32, py: f32, cell: f32, prob: f32, sigma0: f32, gain0: f32, salt: u32, var: f32) -> [f32; 3] {
    // A star narrower than half a pixel would fall between pixel centres: widen it and lower its
    // peak to keep its light.
    let sigma = sigma0.max(0.5);
    let gain = gain0 * (sigma0 / sigma) * (sigma0 / sigma);
    let (ix, iy) = ((px / cell).floor() as i32, (py / cell).floor() as i32);
    if cell_hash(ix, iy, salt) >= prob { return [0.0; 3]; }
    let m = (2.5 * sigma / cell).min(0.45);
    let sx = (ix as f32 + m + (1.0 - 2.0 * m) * cell_hash(ix, iy, salt + 1)) * cell;
    let sy = (iy as f32 + m + (1.0 - 2.0 * m) * cell_hash(ix, iy, salt + 2)) * cell;
    let d2 = (px - sx) * (px - sx) + (py - sy) * (py - sy);
    let b = cell_hash(ix, iy, salt + 3);
    let k = gain * (0.25 + b * b) * (-d2 / (2.0 * sigma * sigma)).exp();
    // Hot blue-white to cool orange, or (var 0) all one faint blue-white.
    let col = mix3([0.72, 0.84, 1.0], [1.0, 0.82, 0.6], cell_hash(ix, iy, salt + 4));
    scale3(if var < 1.0 { mix3([0.86, 0.88, 1.0], col, var) } else { col }, k)
}

/// Space behind everything at pixel (x, y): the base below the horizon, nebulae, the galaxy and
/// the starfield. `c` is what the pixel holds; returns it with space applied.
pub(super) fn backdrop_px(k: &SpaceK, bh: Option<&super::blackhole::BhK>, x: usize, y: usize, c: [f32; 3]) -> [f32; 3] {
    let fy = y as f32;
    let above = fy < k.hy;
    if !above && !k.below { return c; }
    let mut c = c;
    if !above {
        // The gradient mirrored about the horizon.
        let t = ((2.0 * k.hy - fy - k.top) / (k.hy - k.top).max(1.0)).clamp(0.0, 1.0).powf(1.3);
        c = mix3(k.sky_top, k.sky_hor, t);
    }
    if let Some(b) = bh { return super::blackhole::lensed_px(b, k, x, y, c); }
    features(k, x as f32 + 0.5, fy + 0.5, c)
}

/// The nebulae, galaxy and stars seen at (px, py), a pixel centre or (round a black hole) where a
/// bent ray comes from, over `c`.
pub(super) fn features(k: &SpaceK, px: f32, py: f32, c: [f32; 3]) -> [f32; 3] {
    if k.real { return features_real(k, px, py, c); }
    let mut c = c;
    let (sx, sy) = ((px - k.cx) / k.unit, (k.hy - py) / k.unit);
    let mut dark = 0.0;
    if k.neb > 0.0 {
        let (nx, ny) = (sx * k.neb_f + k.so, sy * k.neb_f);
        let n1 = fbm4(nx, ny);
        let n2 = fbm4(nx * 1.7 + 13.1, ny * 1.7 + 5.3);
        let d = smoothstep(0.42, 0.8, n1);
        let fil = 1.0 - (2.0 * value_noise(nx * 5.0, ny * 5.0) - 1.0).abs();
        let fil = fil * fil * fil * fil * smoothstep(0.35, 0.6, n1);
        let col = mix3(k.neb1, k.neb2, smoothstep(0.3, 0.7, n2));
        c = add3(c, scale3(col, k.neb * (0.16 * d + 0.1 * fil)));
        dark = smoothstep(0.55, 0.75, fbm4(nx * 2.0 + 31.0, ny * 2.0)) * 0.6 * k.neb.min(1.0);
    }
    let mut band = 0.0;
    if k.gal > 0.0 {
        let dy = sy - k.gal_h;
        let along = sx * k.gal_c + dy * k.gal_s;
        let across0 = -sx * k.gal_s + dy * k.gal_c;
        let across = across0 + 0.25 * k.gal_w * (value_noise(along * 1.3 + k.so, 3.7) - 0.5);
        let q = across / k.gal_w;
        if q.abs() < 2.6 {
            band = (-q * q).exp();
            let ca = (along - k.gal_core) / 0.22;
            let cq = across / (1.1 * k.gal_w);
            let core = (-ca * ca - cq * cq).exp();
            let lq = across / (0.45 * k.gal_w);
            let dust = smoothstep(0.45, 0.75, fbm4(along * 3.0 + k.so, q * 1.2 + 11.0)) * (-lq * lq).exp();
            let glow = 0.1 * band * (0.6 + 0.8 * fbm4(along * 4.0 + 3.0, q * 2.0)) + 0.22 * core;
            c = add3(c, scale3(k.gal_col, k.gal * glow * (1.0 - 0.85 * dust)));
            dark = dark.max(0.8 * dust);
        }
    }
    for (i, o) in k.objs[..k.n_obj].iter().enumerate() {
        (c, dark) = object_px(k, o, i as u32, px, py, sx, sy, c, dark);
    }
    if k.stars > 0.0 {
        let dens = k.stars * (1.0 + 2.5 * k.gal.min(1.0) * band) * (1.0 - dark);
        let s = k.scale;
        let mut st = star_layer(px, py, 4.5 * s, (0.3 * dens).min(0.95), 0.45 * s, 0.5, 0xA0, k.star_var);
        st = add3(st, star_layer(px, py, 11.0 * s, (0.35 * dens).min(0.95), 0.6 * s, 1.0, 0xB0, k.star_var));
        st = add3(st, star_layer(px, py, 31.0 * s, (0.3 * k.stars).min(0.95), 0.85 * s, 2.2, 0xC0, k.star_var));
        c = add3(c, scale3(st, k.sb * (1.0 - 0.7 * dark)));
    }
    c
}

/// A star's colour from its temperature, t 0 (3000 K, red-orange) .. 1 (20000 K, blue), each
/// about as bright.
fn temp_col(t: f32) -> [f32; 3] {
    let x = t.clamp(0.0, 1.0) * 4.0;
    let col = if x < 1.0 {
        mix3([1.0, 0.46, 0.15], [1.0, 0.68, 0.4], x)
    } else if x < 2.0 {
        mix3([1.0, 0.68, 0.4], [1.0, 0.93, 0.85], x - 1.0)
    } else if x < 3.0 {
        mix3([1.0, 0.93, 0.85], [0.76, 0.84, 1.0], x - 2.0)
    } else {
        mix3([0.76, 0.84, 1.0], [0.5, 0.62, 1.0], x - 3.0)
    };
    let lum = 0.2126 * col[0] + 0.7152 * col[1] + 0.0722 * col[2];
    scale3(col, 1.0 / (0.35 + 0.65 * lum))
}

/// A realistic star's colour from its hash: most yellow-white, fewer orange or blue.
fn star_col(k: &SpaceK, h: f32) -> [f32; 3] {
    let u = 2.0 * h - 1.0;
    mix3([1.0, 0.97, 0.93], temp_col(0.5 + 0.5 * u * u.abs().powf(0.6)), 0.75 * k.star_var)
}

/// How bright the star in cell (ix, iy) is: a steep power law above `f0` (a few bright, most
/// faint), twinkling a whole number of times per loop.
fn star_flux(k: &SpaceK, ix: i32, iy: i32, salt: u32, f0: f32, cap: f32) -> f32 {
    let u = cell_hash(ix, iy, salt + 3).max(0.003);
    let mut f = f0 * u.powf(-0.9).min(cap);
    if k.twinkle > 0.0 {
        let n = k.tw_n * (1.0 + (cell_hash(ix, iy, salt + 5) * 2.0).floor());
        f *= 1.0 + 0.6 * k.twinkle * (TAU * (n * k.tphase + cell_hash(ix, iy, salt + 6))).sin();
    }
    f
}

/// One layer of realistic stars: at most one per square cell (`prob` of them) anywhere in it,
/// from the four cells nearest the pixel, `f0` the faintest.
fn real_layer(k: &SpaceK, px: f32, py: f32, cell: f32, prob: f32, f0: f32, salt: u32) -> [f32; 3] {
    let (ix0, iy0) = ((px / cell - 0.5).floor() as i32, (py / cell - 0.5).floor() as i32);
    let s0 = 0.5 * k.scale;
    let sigma = s0.max(0.5);
    let mut out = [0.0f32; 3];
    for j in 0..2 {
        for i in 0..2 {
            let (ix, iy) = (ix0 + i, iy0 + j);
            if cell_hash(ix, iy, salt) >= prob { continue; }
            let sx = (ix as f32 + cell_hash(ix, iy, salt + 1)) * cell;
            let sy = (iy as f32 + cell_hash(ix, iy, salt + 2)) * cell;
            let d2 = (px - sx) * (px - sx) + (py - sy) * (py - sy);
            let f = star_flux(k, ix, iy, salt, f0, 12.0) * (s0 / sigma) * (s0 / sigma);
            out = add3(out, scale3(star_col(k, cell_hash(ix, iy, salt + 4)), f * (-d2 / (2.0 * sigma * sigma)).exp()));
        }
    }
    out
}

/// The few bright stars: big cells, each star anywhere in its own, with a soft halo and (the
/// brightest) diffraction spikes reaching into the cells round it.
fn bright_stars(k: &SpaceK, px: f32, py: f32, salt: u32) -> [f32; 3] {
    let s = k.scale;
    let cell = 70.0 * s;
    let (ix, iy) = ((px / cell).floor() as i32, (py / cell).floor() as i32);
    let prob = (0.3 * k.stars).min(0.95);
    let s0 = 0.6 * s;
    let sigma = s0.max(0.5);
    let mut out = [0.0f32; 3];
    for j in -1..=1 {
        for i in -1..=1 {
            let (cx, cy) = (ix + i, iy + j);
            if cell_hash(cx, cy, salt) >= prob { continue; }
            let (dx, dy) = (px - (cx as f32 + cell_hash(cx, cy, salt + 1)) * cell, py - (cy as f32 + cell_hash(cx, cy, salt + 2)) * cell);
            let d2 = dx * dx + dy * dy;
            if d2 > 0.9 * cell * 0.9 * cell { continue; }
            let f = star_flux(k, cx, cy, salt, 0.5, 30.0);
            let d = d2.sqrt();
            let mut a = f * (s0 / sigma) * (s0 / sigma) * (-d2 / (2.0 * sigma * sigma)).exp() + 0.05 * f * (-d / (2.5 * s)).exp();
            if k.spikes > 0.0 && f > 2.0 {
                let len = s * (1.2 + 1.8 * f.sqrt());
                let w2 = 2.0 * 0.64 * sigma * sigma;
                let mut sp = 0.0;
                let mut l = 0.0;
                while l < k.lines {
                    let th = std::f32::consts::PI * l / k.lines;
                    let (cs, sn) = (th.cos(), th.sin());
                    let (al, pe) = (dx * cs + dy * sn, -dx * sn + dy * cs);
                    sp += (-al.abs() / len).exp() * (-pe * pe / w2).exp();
                    l += 1.0;
                }
                a += 0.25 * k.spikes * f * smoothstep(2.0, 6.0, f) * sp;
            }
            out = add3(out, scale3(star_col(k, cell_hash(cx, cy, salt + 4)), a));
        }
    }
    out
}

/// The realistic sky at (px, py): drifting gas threaded with dust, the galaxy's band of
/// unresolved stars split by dark rifts, the objects, and a starfield of a few bright stars among
/// a great many faint ones gathered in clouds.
fn features_real(k: &SpaceK, px: f32, py: f32, c: [f32; 3]) -> [f32; 3] {
    let mut c = c;
    let (sx, sy) = ((px - k.cx) / k.unit, (k.hy - py) / k.unit);
    let mut dark = 0.0;
    if k.neb > 0.0 {
        let (nx, ny) = (sx * k.neb_f + k.so, sy * k.neb_f);
        let (wx, wy) = (fbm4(nx + 5.2, ny + 1.3), fbm4(nx + 1.7, ny + 9.2));
        let (ux, uy) = (nx + 1.6 * wx, ny + 1.6 * wy);
        let n1 = fbm4(ux, uy);
        let n2 = fbm4(ux * 1.7 + 13.1, uy * 1.7 + 5.3);
        let body = smoothstep(0.42, 0.8, n1);
        let rid = 1.0 - (2.0 * fbm4(ux * 2.6 + 3.0, uy * 2.6) - 1.0).abs();
        let r2 = rid * rid;
        let fil = r2 * r2 * r2 * smoothstep(0.3, 0.55, n1);
        let col = mix3(k.neb1, k.neb2, smoothstep(0.3, 0.7, n2));
        let dust = smoothstep(0.5, 0.72, fbm4(ux * 2.2 + 31.0, uy * 2.2)) * k.neb_dust;
        let gas = k.neb * (0.2 * body * body + 0.12 * fil + 0.03 * smoothstep(0.3, 0.6, n1)) * (1.0 - 0.85 * dust);
        c = add3(scale3(c, 1.0 - 0.5 * dust * k.neb.min(1.0)), scale3(col, gas));
        dark = 0.75 * dust * k.neb.min(1.0);
    }
    let mut band = 0.0;
    if k.gal > 0.0 {
        let dy = sy - k.gal_h;
        let along = sx * k.gal_c + dy * k.gal_s;
        let across0 = -sx * k.gal_s + dy * k.gal_c;
        let across = across0 + 0.3 * k.gal_w * (fbm4(along * 1.1 + k.so, 3.7) - 0.47);
        let q = across / k.gal_w;
        if q.abs() < 3.2 {
            band = (-q * q).exp();
            let skirt = (-0.2 * q * q).exp();
            let mott = 0.35 + 1.3 * smoothstep(0.3, 0.75, fbm4(along * 6.0 + 3.0, q * 1.5 + k.so));
            let ca = (along - k.gal_core) / 0.3;
            let cq = across / (1.6 * k.gal_w);
            let core = (-ca * ca - cq * cq).exp();
            let wq = (q + 0.8 * (fbm4(along * 2.0 + k.so + 9.0, 1.3) - 0.47)) / 0.4;
            let rift = (-wq * wq).exp() * smoothstep(0.35, 0.6, fbm4(along * 2.5 + k.so + 17.0, q * 0.8 + 3.0));
            let lq = q / 0.9;
            let lanes = smoothstep(0.45, 0.7, fbm4(along * 5.0 + k.so, q * 2.0 + 11.0)) * (-lq * lq).exp();
            let dust = k.gal_dust * rift.max(lanes);
            let col = mix3(k.gal_col, mul3(k.gal_col, [1.0, 0.8, 0.58]), 0.8 * core);
            let glow = 0.09 * band * mott + 0.025 * skirt + 0.3 * core;
            c = add3(c, scale3(col, k.gal * glow * (1.0 - 0.92 * dust)));
            dark = dark.max(0.9 * dust);
        }
    }
    for (i, o) in k.objs[..k.n_obj].iter().enumerate() {
        (c, dark) = object_px(k, o, i as u32, px, py, sx, sy, c, dark);
    }
    if k.stars > 0.0 {
        let clump = (1.0 + k.clumps * 3.0 * (fbm4(sx * 2.5 + k.so + 50.0, sy * 2.5 + 20.0) - 0.47)).max(0.1);
        let field = k.stars * clump * (1.0 + 2.5 * k.gal.min(1.0) * band) * (1.0 - dark);
        c = add3(c, scale3([1.0, 0.94, 0.86], 0.012 * k.glow * field * k.sb));
        let s = k.scale;
        let mut st = real_layer(k, px, py, 3.0 * s, (0.22 * field).min(0.95), 0.025, 0xA0);
        st = add3(st, real_layer(k, px, py, 7.0 * s, (0.3 * field).min(0.95), 0.06, 0xB0));
        st = add3(st, real_layer(k, px, py, 18.0 * s, (0.4 * k.stars * clump.sqrt() * (1.0 - 0.5 * dark)).min(0.95), 0.15, 0xC0));
        st = add3(st, bright_stars(k, px, py, 0xE0));
        c = add3(c, scale3(st, k.sb * (1.0 - 0.7 * dark)));
    }
    c
}

/// One object far off in the sky (`ObjK`, the `i`th) over `c` at pixel (px, py), sky point
/// (sx, sy); returns the colour and how much dust now hides the stars behind.
#[allow(clippy::too_many_arguments)]
fn object_px(k: &SpaceK, o: &ObjK, i: u32, px: f32, py: f32, sx: f32, sy: f32, c: [f32; 3], dark: f32) -> ([f32; 3], f32) {
    let (dx, dy) = (sx - o.x, sy - o.y);
    let u = (dx * o.c + dy * o.s) / o.r;
    let v = (-dx * o.s + dy * o.c) / (o.r * o.aspect);
    let rho = (u * u + v * v).sqrt();
    // A belt runs right across the sky: only its thickness bounds it.
    if if o.kind as i32 == 11 { v.abs() > o.reach } else { rho > o.reach } { return (c, dark); }
    let (b, so) = (o.bright, o.so);
    let [c0, c1, c2] = o.col;
    let salt = 0x300 + 0x10 * i;
    // A point of light at the centre (a white dwarf, a comet's nucleus), in pixels.
    let dp2 = (dx * dx + dy * dy) * k.unit * k.unit;
    let ps = (0.6 * k.scale).max(0.5);
    let point = (-dp2 / (2.0 * ps * ps)).exp();
    let mut c = c;
    let mut dark = dark;
    match o.kind as i32 {
        0 => {
            let (a, bb) = (u * 1.6 + so, v * 1.6);
            let (w1, w2) = (fbm4(a + 5.2, bb + 1.3), fbm4(a + 1.7, bb + 9.2));
            let (wa, wb) = (a + 1.4 * w1, bb + 1.4 * w2);
            let n = fbm4(wa, wb);
            let fall = 1.0 - smoothstep(0.25, 1.25, rho + 0.6 * (n - 0.47));
            let body = fall * (0.3 + 1.4 * smoothstep(0.4, 0.75, n));
            let rid = 1.0 - (2.0 * fbm4(wa * 2.3 + 7.0, wb * 2.3) - 1.0).abs();
            let r2 = rid * rid;
            let fil = r2 * r2 * rid * fall;
            let core = (-rho * rho * 5.0).exp();
            let col = mix3(c0, c1, (core * 1.4).min(1.0));
            let dn = fbm4(wa * 1.8 + 31.0, wb * 1.8 + 4.0);
            let dust = smoothstep(0.52, 0.7, dn) * fall;
            let rim = smoothstep(0.44, 0.52, dn) * (1.0 - smoothstep(0.52, 0.6, dn)) * fall;
            let gas = b * (0.2 * body + 0.16 * fil + 0.3 * core * fall);
            c = add3(scale3(c, 1.0 - 0.5 * dust), add3(scale3(col, gas * (1.0 - 0.7 * dust)), scale3(c2, 0.12 * b * rim)));
            dark = dark.max(0.8 * dust);
            let ys = real_layer(k, px, py, 13.0 * k.scale, 0.55 * (-rho * rho * 3.0).exp(), 0.5, salt);
            c = add3(c, scale3(ys, k.sb));
        }
        1 => {
            let (a, bb) = (u * 1.4 + so, v * 1.4);
            let n = fbm4(a + 1.4 * fbm4(a + 5.2, bb + 1.3), bb + 1.4 * fbm4(a + 1.7, bb + 9.2));
            let streak = fbm4(u * 1.2 + so + 3.0, v * 7.0);
            let glow = (-rho * rho * 2.2).exp() * (0.35 + 0.9 * smoothstep(0.35, 0.75, n)) * (0.5 + 1.0 * streak);
            let halo = (-rho * rho * 9.0).exp();
            c = add3(c, add3(scale3(mix3(c0, c2, smoothstep(0.4, 0.7, streak)), 0.45 * b * glow), scale3(c1, 0.2 * b * halo)));
            dark = dark.max(0.3 * smoothstep(0.6, 0.75, n) * (-rho * rho * 2.0).exp());
            let ys = real_layer(k, px, py, 16.0 * k.scale, 0.85 * (-rho * rho * 7.0).exp(), 1.0, salt);
            c = add3(c, scale3(ys, k.sb));
        }
        2 => {
            let (a, bb) = (u * 1.5 + so, v * 1.5);
            let n = fbm4(a + 1.3 * fbm4(a + 5.2, bb + 1.3), bb + 1.3 * fbm4(a + 1.7, bb + 9.2));
            let shape = 1.0 - smoothstep(0.15, 1.0, rho + 1.6 * (0.5 - n));
            let op = (shape * (0.4 + 1.2 * n)).clamp(0.0, 1.0) * b.min(1.0);
            let lit = 0.3 + 0.7 * smoothstep(-0.3, 0.9, u / rho.max(1e-3));
            let rim = smoothstep(0.02, 0.25, shape) * (1.0 - smoothstep(0.25, 0.6, shape)) * lit;
            c = add3(scale3(c, 1.0 - 0.95 * op), add3(scale3(c1, 0.04 * op * b), scale3(c0, 0.06 * b * rim)));
            dark = dark.max(0.97 * op);
        }
        3 => {
            let rn = rho + 0.1 * (fbm4(u * 3.0 + so, v * 3.0) - 0.47);
            let fil = fbm4(u * 7.0 + so, v * 7.0);
            let (q1, q2, q3) = ((rn - 0.55) / 0.16, rn / 0.45, (rn - 0.8) / 0.1);
            let (shell, inner, outer) = ((-q1 * q1).exp(), (-q2 * q2).exp(), (-q3 * q3).exp());
            let halo = (-rn * rn * 1.5).exp();
            let gas = add3(add3(scale3(c1, 0.3 * shell * (0.6 + 0.8 * fil) + 0.22 * inner), scale3(c0, 0.35 * outer * (0.5 + fil) + 0.1 * shell)), scale3(c2, 0.05 * halo));
            c = add3(c, add3(scale3(gas, b), scale3([0.85, 0.9, 1.0], 2.0 * b * point)));
        }
        4 => {
            let rn = rho + 0.2 * (fbm4(u * 2.0 + so, v * 2.0) - 0.47);
            let f1 = 1.0 - (2.0 * fbm4(u * 3.2 + so + 11.0, v * 3.2) - 1.0).abs();
            let f2 = 1.0 - (2.0 * fbm4(u * 5.0 + so + 23.0, v * 5.0 + 4.0) - 1.0).abs();
            let (f1, f2) = ((f1 * f1) * (f1 * f1), (f2 * f2) * (f2 * f2));
            let (f1, f2) = (f1 * f1, f2 * f2);
            let qs = (rn - 0.8) / 0.14;
            let shell = (-qs * qs).exp();
            let qf = rn / 0.75;
            let fill = (-qf * qf).exp();
            let gas = add3(add3(scale3(c0, 0.5 * shell * (0.15 + 1.6 * f1)), scale3(c1, 0.65 * shell * f2)), scale3(c2, 0.02 * fill + 0.12 * fill * f2));
            c = add3(c, scale3(gas, b));
        }
        5 => {
            let rr = rho.max(0.015);
            let ph = 2.0 * v.atan2(u) - 3.6 * rr.ln() + so;
            let arm = 0.5 + 0.5 * ph.cos();
            let arm = arm * arm * arm;
            let l = 0.5 + 0.5 * (ph - 0.55).cos();
            let (l2, l4) = (l * l, l * l * l * l);
            let lane = l4 * l4 * l2 * smoothstep(0.08, 0.25, rr);
            let n = fbm4(u * 6.0 + so, v * 6.0);
            let disk = (-rr * 3.0).exp() * (1.0 - smoothstep(0.75, 1.2, rr));
            let bulge = (-rr * rr * 45.0).exp();
            let knots = smoothstep(0.62, 0.8, fbm4(u * 16.0 + so, v * 16.0)) * arm * disk;
            let arms = disk * (0.18 + 1.2 * arm * (0.5 + 0.8 * n));
            let gal = add3(add3(scale3(c0, 0.35 * arms * (1.0 - 0.75 * lane)), scale3(c1, 0.9 * bulge + 0.15 * disk)), scale3(c2, 0.5 * knots));
            c = add3(c, scale3(gal, b));
        }
        6 => {
            let glow = (-rho * 3.5).exp() * (1.0 - smoothstep(1.2, 2.0, rho));
            let core = (-rho * rho * 80.0).exp();
            c = add3(c, add3(scale3(c0, 0.35 * b * glow), scale3(c1, 0.8 * b * core)));
        }
        7 => {
            let prof = (1.0 - smoothstep(1.0, 1.6, rho)) / (1.0 + 10.0 * rho * rho);
            c = add3(c, scale3(c1, 0.08 * b * prof * prof.sqrt()));
            let st = add3(
                real_layer(k, px, py, 2.6 * k.scale, prof.min(0.92), 0.2 * b, salt),
                real_layer(k, px, py, 7.0 * k.scale, (0.9 * prof).min(0.92), 0.6 * b, salt + 8),
            );
            c = add3(c, scale3(mul3(st, c0), k.sb));
        }
        9 => {
            // Two stars round their common centre, the lighter one further out, gas streaming
            // from it to the other.
            let (cr, sr) = (o.rot.cos(), o.rot.sin());
            let (p1, p2) = ([0.4 * cr, 0.4 * sr], [-0.7 * cr, -0.7 * sr]);
            let pxu = o.r * k.unit;
            let s0 = (0.7 * k.scale).max(0.5) / pxu.max(1e-3);
            let d1 = ((u - p1[0]).powi(2) + (v - p1[1]).powi(2)).sqrt();
            let d2 = ((u - p2[0]).powi(2) + (v - p2[1]).powi(2)).sqrt();
            let core1 = (-d1 * d1 / (2.0 * 2.56 * s0 * s0)).exp();
            let core2 = (-d2 * d2 / (2.0 * s0 * s0)).exp();
            let (ax, ay) = (p1[0] - p2[0], p1[1] - p2[1]);
            let t = (((u - p2[0]) * ax + (v - p2[1]) * ay) / (ax * ax + ay * ay).max(1e-4)).clamp(0.0, 1.0);
            let sd = ((p2[0] + ax * t - u).powi(2) + (p2[1] + ay * t - v).powi(2)).sqrt() / (0.02 + 0.03 * t);
            let stream = (-sd * sd).exp() * 4.0 * t * (1.0 - t);
            let light = add3(add3(scale3(c0, 3.0 * core1 + 0.35 * (-d1 / 0.12).exp()), scale3(c1, 2.0 * core2 + 0.25 * (-d2 / 0.08).exp())), scale3(c2, 0.5 * stream));
            c = add3(c, scale3(light, b));
        }
        10 => {
            // A dead star spinning: two beams sweeping round, inside the wisps it threw off.
            let pxu = o.r * k.unit;
            let s0 = (0.7 * k.scale).max(0.5) / pxu.max(1e-3);
            let core = 3.0 * (-rho * rho / (2.0 * s0 * s0)).exp() + 0.4 * (-rho / 0.05).exp();
            let (cr, sr) = (o.rot.cos(), o.rot.sin());
            let along = u * cr + v * sr;
            let perp = -u * sr + v * cr;
            let w = 0.02 + 0.1 * along.abs();
            let beam = (-(perp / w) * (perp / w)).exp() * (-along.abs() / 1.1).exp() * (1.0 - smoothstep(2.5, 3.0, along.abs()));
            let f = 1.0 - (2.0 * fbm4(u * 3.0 + so, v * 3.0) - 1.0).abs();
            let f2 = f * f;
            let qw = rho / 0.9;
            let wisp = f2 * f2 * (-qw * qw).exp();
            c = add3(c, scale3(add3(add3(scale3(c0, core), scale3(c1, 0.9 * beam)), scale3(c2, 0.25 * wisp)), b));
        }
        11 => {
            // A band of dust with rocks drifting along it, lit from one side.
            let prof = (-v * v * 1.5).exp();
            c = add3(c, scale3(c0, 0.05 * b * prof * (0.6 + 0.8 * fbm4(u * 2.0 + so, v * 1.5))));
            let pxu = o.r * k.unit;
            // Drifting a whole number of cells a loop, so it comes round seamlessly.
            let drift = o.rot / TAU;
            for (cu, cvs, salt, big) in [(6.0f32, 3.0f32, 0x340u32, 1.0f32), (16.0, 7.0, 0x350, 0.45)] {
                let (us, vs) = ((u + drift) * cu, v * cvs);
                let (ix, iy) = (us.floor() as i32, vs.floor() as i32);
                let cell_v = (iy as f32 + 0.5) / cvs;
                if cell_hash(ix, iy, salt + i * 16) >= 0.7 * (-cell_v * cell_v * 1.5).exp() { continue; }
                let h3 = cell_hash(ix, iy, salt + i * 16 + 3);
                let ccx = ix as f32 + 0.3 + 0.4 * cell_hash(ix, iy, salt + i * 16 + 1);
                let ccy = iy as f32 + 0.3 + 0.4 * cell_hash(ix, iy, salt + i * 16 + 2);
                let (du, dv) = ((us - ccx) / cu, (vs - ccy) / cvs * o.aspect);
                let rr = big * (0.02 + 0.05 * h3 * h3);
                let d = (du * du + dv * dv).sqrt() / rr;
                if d >= 1.0 { continue; }
                let lit = (0.5 - 0.5 * du / rr).clamp(0.0, 1.0);
                let col = scale3(mix3(c1, c2, 0.6 * lit), 0.15 + 0.85 * lit);
                let a = ((1.0 - d) * rr * pxu).clamp(0.0, 1.0);
                c = mix3(c, scale3(col, 0.9 * b), a);
                dark = dark.max(a);
            }
        }
        12 => {
            // A sun close up: its disc darker to the rim, mottled and spotted; its corona; flares.
            let rd = 0.45;
            let e = (rho - rd).max(0.0);
            let ang = v.atan2(u);
            let (ca, sa) = (ang.cos(), ang.sin());
            let streak = 0.6 + 0.8 * fbm4(ca * 3.0 + so, sa * 3.0);
            let corona = scale3(c1, 0.9 * (-e / 0.06).exp() + 0.35 * (-e / 0.25).exp() * streak);
            let n = fbm4(ca * 4.0 + 0.5 * o.rot.cos() + so, sa * 4.0 + 0.5 * o.rot.sin());
            let flare = smoothstep(0.55, 0.8, n) * (-e / (0.05 + 0.12 * n)).exp();
            c = add3(c, scale3(add3(corona, scale3(c2, 1.5 * flare)), b));
            let pxu = o.r * k.unit;
            let cov = ((rd - rho) * pxu + 0.5).clamp(0.0, 1.0);
            if cov > 0.0 {
                let mu = (1.0 - (rho / rd).min(1.0).powi(2)).max(0.0).sqrt();
                let gran = fbm4(u * 40.0 + so, v * 40.0);
                let spots = smoothstep(0.72, 0.8, fbm4(u * 6.0 + so + 5.0, v * 6.0));
                let disc = scale3(mix3(c2, c0, 0.5 + 0.5 * mu), 1.8 * (0.35 + 0.65 * mu) * (0.9 + 0.2 * gran) * (1.0 - 0.8 * spots) * b);
                c = mix3(c, disc, cov);
            }
        }
        13 => {
            // A ship far off: a dark hull lit along its top, its engine glowing, a trail behind,
            // and blinking lights at its wingtips.
            let pxu = o.r * k.unit;
            let q = ((u + 0.1) / 0.65).powi(2);
            let hw = 0.2 * (1.0 - q).max(0.0).sqrt() * if u > 0.0 { 1.0 - 0.6 * u } else { 1.0 };
            if u > -0.6 && u < 0.6 && hw > 0.0 {
                let a = ((hw - v.abs()) * pxu).clamp(0.0, 1.0);
                if a > 0.0 {
                    let shade = 0.55 + 0.45 * (v / hw).clamp(-1.0, 1.0);
                    c = mix3(c, scale3(c0, shade * (0.3 + 0.7 * b)), a);
                }
            }
            let (ex, ey) = (u + 0.62, v);
            let s0 = 0.07f32.max(1.0 / pxu.max(1e-3));
            let glow = 2.5 * (-(ex * ex + ey * ey) / (2.0 * s0 * s0)).exp();
            let trail = if ex < 0.0 { (-(v / 0.05) * (v / 0.05)).exp() * (ex * 1.8).exp() * 0.8 } else { 0.0 };
            let blink = 0.5 + 0.5 * (o.rot * 8.0).sin();
            let ls = (0.6 * k.scale).max(0.5) / pxu.max(1e-3);
            let tip = |tv: f32| { let (du, dv) = (u - 0.05, v - tv); (-(du * du + dv * dv) / (2.0 * ls * ls)).exp() };
            let wing = 0.2 * (1.0 - ((0.05f32 + 0.1) / 0.65).powi(2)).sqrt();
            c = add3(c, add3(scale3(c1, b * (glow + trail)), scale3(c2, b * 2.0 * blink * (tip(wing) + tip(-wing)))));
        }
        _ => {
            let up = u.max(0.0);
            let on = smoothstep(-0.03, 0.02, u);
            let wi = 0.012 + 0.05 * up;
            let qi = v / wi;
            let ion = on * (-qi * qi).exp() * (-up * 2.0).exp() * (0.55 + 0.9 * fbm4(up * 9.0 + so, qi * 0.4));
            let wd = 0.025 + 0.22 * up;
            let qd = (v - 0.22 * up * up) / wd;
            let dust = on * (-qd * qd).exp() * (-up * 1.6).exp();
            let (qc, qh) = (rho / 0.035, rho / 0.12);
            let (coma, head) = ((-qc * qc).exp(), (-qh * qh).exp());
            let tail = add3(add3(scale3(c0, 0.4 * dust + 0.15 * head), scale3(c1, 0.45 * ion)), scale3(c2, 1.2 * coma));
            c = add3(c, add3(scale3(tail, b), scale3([1.0, 1.0, 0.95], 2.0 * b * point)));
        }
    }
    (c, dark)
}

/// One planet's numbers for a frame (packed for the GPU in this order by `pack`).
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct PlanetK {
    kind: f32,
    cx: f32,
    cy: f32,
    r: f32,
    col: [f32; 3],
    col2: [f32; 3],
    atm: [f32; 3],
    atm_k: f32,
    /// Toward its sun (x right, y up, z toward the viewer).
    s: [f32; 3],
    tc: f32,
    ts: f32,
    /// How far it has turned (radians).
    lon0: f32,
    clouds: f32,
    lights: f32,
    so: f32,
    ring_on: f32,
    r_in: f32,
    r_out: f32,
    r_open: f32,
    r_c: f32,
    r_s: f32,
    r_col: [f32; 3],
    r_op: f32,
    /// A moon's shadow across it: its centre on the disc and radius, in the planet's radii (r 0: none).
    sh: [f32; 3],
}

impl PlanetK {
    pub(super) fn new(ctx: &Ctx, p: &Planet) -> PlanetK {
        let v = &ctx.view;
        let hy = v.horizon_px.max(2.0);
        let (ox, oy) = (v.left as f32, v.top as f32);
        let (fw, fhy) = (v.width as f32 - 2.0 * ox, (hy - oy).max(2.0));
        let (la, th) = (p.light_angle.to_radians(), p.night.clamp(0.0, 1.0) * std::f32::consts::PI);
        let (ta, ra) = (p.tilt.to_radians(), p.rings.angle.to_radians());
        PlanetK {
            kind: match p.kind { PlanetKind::Rocky => 0.0, PlanetKind::Gas => 1.0, PlanetKind::Earth => 2.0, PlanetKind::Ice => 3.0, PlanetKind::Lava => 4.0, PlanetKind::Desert => 5.0 },
            cx: ox + v.sky_dx() + p.pos[0] * fw,
            cy: oy + p.pos[1] * fhy,
            r: (p.radius * fhy).max(1.0),
            col: rgb_lin(p.color),
            col2: rgb_lin(p.color2),
            atm: rgb_lin(p.atmosphere),
            atm_k: p.atmosphere_strength.max(0.0),
            s: [la.cos() * th.sin(), la.sin() * th.sin(), th.cos()],
            tc: ta.cos(),
            ts: ta.sin(),
            lon0: TAU * p.spin as f32 * ctx.tphase,
            clouds: p.clouds.clamp(0.0, 1.0),
            lights: p.city_lights.max(0.0),
            so: (hf(p.seed ^ 0x9E1, 0) * 50.0).floor(),
            ring_on: if p.rings.enabled && p.rings.opacity > 0.0 { 1.0 } else { 0.0 },
            r_in: p.rings.inner.max(1.0),
            r_out: p.rings.outer.max(p.rings.inner.max(1.0) + 0.01),
            r_open: p.rings.open.clamp(0.02, 1.0),
            r_c: ra.cos(),
            r_s: ra.sin(),
            r_col: rgb_lin(p.rings.color),
            r_op: p.rings.opacity.clamp(0.0, 1.0),
            sh: [0.0; 3],
        }
    }

    pub(super) fn pack(&self) -> [f32; PLANET_FLOATS] {
        let mut o = [0.0f32; PLANET_FLOATS];
        let vals = [
            self.kind, self.cx, self.cy, self.r,
            self.col[0], self.col[1], self.col[2], self.col2[0], self.col2[1], self.col2[2],
            self.atm[0], self.atm[1], self.atm[2], self.atm_k,
            self.s[0], self.s[1], self.s[2], self.tc, self.ts, self.lon0, self.clouds, self.lights, self.so,
            self.ring_on, self.r_in, self.r_out, self.r_open, self.r_c, self.r_s,
            self.r_col[0], self.r_col[1], self.r_col[2], self.r_op, self.sh[0], self.sh[1], self.sh[2],
        ];
        o[..vals.len()].copy_from_slice(&vals);
        o
    }

    /// Bounding box in pixels: the disc, its air and its rings.
    pub(super) fn reach(&self) -> f32 {
        let mut e: f32 = if self.atm_k > 0.0 { 1.25 } else { 1.02 };
        if self.ring_on > 0.0 { e = e.max(self.r_out + 0.02); }
        e * self.r + 1.0
    }
}

/// Albedo, light of its own, and how mirror-like (oceans), at (u, v) on the surface.
fn surface(p: &PlanetK, u: f32, v: f32, ndl: f32) -> ([f32; 3], [f32; 3], f32) {
    let white = [0.92, 0.94, 0.97];
    let uo = u + p.so * 0.013;
    match p.kind as i32 {
        1 => {
            let turb = fbm_wrap(uo, v, 8, 10.0);
            let band = 0.5 + 0.5 * (TAU * (v * 4.5 + 0.35 * turb)).sin();
            let fine = 0.5 + 0.5 * (TAU * (v * 13.0 + 0.6 * turb)).sin();
            let mut a = mix3(p.col, p.col2, band * 0.75 + fine * 0.25);
            // A storm.
            let du = (u - 0.3).rem_euclid(1.0) - 0.5;
            let dv = v - 0.62;
            let spot = (-(du * 9.0) * (du * 9.0) - (dv * 22.0) * (dv * 22.0)).exp();
            a = mix3(a, mix3(p.col2, p.col, 0.3), 0.7 * spot);
            a = mix3(a, white, p.clouds * 0.25 * fine);
            (a, [0.0; 3], 0.0)
        }
        2 => {
            let h = fbm_wrap(uo, v, 5, 2.5);
            let land = smoothstep(0.5, 0.53, h);
            let mut a = mix3(p.col, scale3(p.col2, 0.75 + 0.5 * fbm_wrap(uo, v, 20, 10.0)), land);
            let ice = smoothstep(0.78, 0.86, (v - 0.5).abs() * 2.0 + 0.08 * (h - 0.5));
            a = mix3(a, white, ice);
            let cl = (smoothstep(0.5, 0.72, fbm_wrap(uo + 0.13, v, 7, 3.5)) * p.clouds * 1.6).min(1.0);
            a = mix3(a, white, cl);
            let night = 1.0 - smoothstep(-0.1, 0.15, ndl);
            let city = land * (1.0 - ice) * (1.0 - 0.7 * cl) * smoothstep(0.62, 0.8, fbm_wrap(uo, v, 160, 80.0));
            (a, scale3([1.0, 0.7, 0.35], p.lights * 0.6 * city * night), (1.0 - land) * (1.0 - cl) * (1.0 - ice))
        }
        3 => {
            let h = fbm_wrap(uo, v, 6, 3.0);
            let r = 1.0 - (2.0 * fbm_wrap(uo, v, 12, 6.0) - 1.0).abs();
            let cr = r * r * r;
            (scale3(mix3(p.col, p.col2, cr * cr), 0.85 + 0.3 * h), [0.0; 3], 0.0)
        }
        4 => {
            let h = fbm_wrap(uo, v, 8, 4.0);
            let r = 1.0 - (2.0 * fbm_wrap(uo + 0.21, v, 10, 5.0) - 1.0).abs();
            let crack = smoothstep(0.82, 0.97, r);
            (scale3(p.col, 0.7 + 0.6 * h), scale3(p.col2, crack * 1.6 * (0.7 + 0.3 * h)), 0.0)
        }
        5 => {
            // Dunes in long bands, canyons cut through, dust blowing, small pale caps.
            let h = fbm_wrap(uo, v, 6, 3.0);
            let dunes = 0.5 + 0.5 * (TAU * (v * 14.0 + 0.8 * fbm_wrap(uo, v, 10, 6.0))).sin();
            let canyon = smoothstep(0.85, 0.95, 1.0 - (2.0 * fbm_wrap(uo + 0.4, v, 12, 6.0) - 1.0).abs());
            let mut a = scale3(mix3(p.col, p.col2, smoothstep(0.4, 0.65, h)), 0.9 + 0.12 * dunes);
            a = mix3(a, scale3(p.col2, 0.5), 0.7 * canyon);
            let caps = smoothstep(0.86, 0.92, (v - 0.5).abs() * 2.0 + 0.05 * (h - 0.5));
            a = mix3(a, [0.92, 0.9, 0.88], 0.9 * caps);
            let dust = smoothstep(0.55, 0.75, fbm_wrap(uo + 0.2, v, 7, 4.0)) * p.clouds;
            (mix3(a, scale3(p.col, 1.2), 0.4 * dust), [0.0; 3], 0.0)
        }
        _ => {
            let h = fbm_wrap(uo, v, 6, 3.0);
            let h2 = fbm_wrap(uo + 0.37, v, 24, 12.0);
            let pits = smoothstep(0.6, 0.8, fbm_wrap(uo, v, 16, 8.0));
            (scale3(mix3(p.col, p.col2, smoothstep(0.35, 0.65, h)), (0.8 + 0.4 * h2) * (1.0 - 0.25 * pits)), [0.0; 3], 0.0)
        }
    }
}

/// A planet at pixel (x, y) over `c`: its far rings, its air's halo, the disc, its near rings.
pub(super) fn planet_px(p: &PlanetK, x: usize, y: usize, c: [f32; 3]) -> [f32; 3] {
    let dx = (x as f32 + 0.5 - p.cx) / p.r;
    let dy = (y as f32 + 0.5 - p.cy) / p.r;
    let aa = 1.0 / p.r;
    let rr = (dx * dx + dy * dy).sqrt();
    let mut out = c;
    // Rings: (a, b) across and down the ring plane's line of sight; the near half (b > 0) is
    // in front of the planet.
    let (mut ring_a, mut ring_front, mut ring_col) = (0.0, false, [0.0; 3]);
    if p.ring_on > 0.0 {
        let a = dx * p.r_c + dy * p.r_s;
        let b = -dx * p.r_s + dy * p.r_c;
        let bo = b / p.r_open;
        let rho = (a * a + bo * bo).sqrt();
        let w = aa / p.r_open.min(1.0).max(0.15);
        if rho > p.r_in - w && rho < p.r_out + w {
            let t = ((rho - p.r_in) / (p.r_out - p.r_in)).clamp(0.0, 1.0);
            let bands = 0.55 + 0.45 * value_noise(t * 38.0 + p.so, 0.5);
            let gq = (t - 0.62) / 0.025;
            let gap = 1.0 - 0.85 * (-gq * gq).exp();
            let edges = smoothstep(p.r_in - w, p.r_in + w, rho) * (1.0 - smoothstep(p.r_out - w, p.r_out + w, rho));
            ring_a = p.r_op * bands * gap * edges;
            // The planet's shadow on the rings: the ring point, in the same turned frame as the light.
            let sa = p.s[0] * p.r_c - p.s[1] * p.r_s;
            let sb = p.s[0] * p.r_s + p.s[1] * p.r_c;
            let q = [a, -b, bo * (1.0 - p.r_open * p.r_open).sqrt()];
            let qs = q[0] * sa + q[1] * sb + q[2] * p.s[2];
            let shadow = qs < 0.0 && q[0] * q[0] + q[1] * q[1] + q[2] * q[2] - qs * qs < 1.0;
            ring_col = scale3(p.r_col, if shadow { 0.08 } else { 1.0 });
            ring_front = b > 0.0;
        }
    }
    if ring_a > 0.0 && !ring_front { out = mix3(out, ring_col, ring_a); }
    let cov = ((1.0 - rr) / aa + 0.5).clamp(0.0, 1.0);
    if p.atm_k > 0.0 && rr >= 1.0 - aa && rr < 1.25 {
        // The lit side's air glows past the rim.
        let e = (rr - 1.0).max(0.0);
        let lit = smoothstep(-0.35, 0.45, (dx * p.s[0] - dy * p.s[1]) / rr.max(1e-4));
        out = add3(out, scale3(p.atm, p.atm_k * (-e / 0.045).exp() * lit * (1.0 - cov)));
    }
    if cov > 0.0 {
        let rc = rr.max(1.0);
        let (nx, ny) = (dx / rc, -dy / rc);
        let nz = (1.0 - nx * nx - ny * ny).max(0.0).sqrt();
        let ndl = nx * p.s[0] + ny * p.s[1] + nz * p.s[2];
        let diff = smoothstep(-0.05, 0.05, ndl) * (0.12 + 0.88 * ndl.clamp(0.0, 1.0));
        let xt = nx * p.tc + ny * p.ts;
        let yt = -nx * p.ts + ny * p.tc;
        let lat = yt.clamp(-1.0, 1.0).asin();
        let lon = xt.atan2(nz) + p.lon0;
        let (u, v) = ((lon / TAU).rem_euclid(1.0), lat / std::f32::consts::PI + 0.5);
        let (alb, emit, gloss) = surface(p, u, v, ndl);
        let mut col = add3(scale3(alb, 1.1 * diff + 0.012), emit);
        if gloss > 0.0 && ndl > 0.0 {
            // The sun's glint on water.
            let h = [p.s[0], p.s[1], p.s[2] + 1.0];
            let hl = (h[0] * h[0] + h[1] * h[1] + h[2] * h[2]).sqrt().max(1e-5);
            let nh = ((nx * h[0] + ny * h[1] + nz * h[2]) / hl).max(0.0);
            let nh2 = nh * nh;
            let nh8 = nh2 * nh2 * nh2 * nh2;
            let nh40 = nh8 * nh8 * nh8 * nh8 * nh8;
            col = add3(col, scale3([1.0, 0.95, 0.85], 0.6 * gloss * nh40));
        }
        if p.atm_k > 0.0 {
            let rim = 1.0 - nz;
            col = add3(col, scale3(p.atm, p.atm_k * (rim * rim * rim * smoothstep(-0.3, 0.3, ndl) + 0.15 * diff)));
        }
        if p.sh[2] > 0.0 {
            // A moon's shadow: a dark core and a soft edge.
            let ds = ((dx - p.sh[0]).powi(2) + (dy - p.sh[1]).powi(2)).sqrt();
            col = scale3(col, 1.0 - 0.88 * (1.0 - smoothstep(0.75 * p.sh[2], 1.25 * p.sh[2], ds)));
        }
        out = mix3(out, col, cov);
    }
    if ring_a > 0.0 && ring_front { out = mix3(out, ring_col, ring_a); }
    out
}

/// The planets of a world's sky this frame, in drawing order.
pub(super) fn planet_list(ctx: &Ctx) -> Vec<PlanetK> {
    match space_k(ctx) {
        Some(_) => {
            let ps: Vec<&Planet> = ctx.scene.sky.space.planets.iter().filter(|p| p.enabled && p.radius > 0.0).take(MAX_PLANETS).collect();
            let mut ks: Vec<(PlanetK, f32)> = ps.iter().map(|p| (PlanetK::new(ctx, p), 0.0)).collect();
            // Moons go round their planets: placed from them, in front or behind, their shadows
            // falling across them.
            for i in 0..ps.len() {
                let o = &ps[i].orbit;
                let j = o.around as usize;
                if !o.enabled || j >= ps.len() || j == i || ps[j].orbit.enabled { continue; }
                let parent = ks[j].0;
                let th = TAU * (o.phase + o.turns as f32 * ctx.tphase);
                let (ti, ro) = (o.tilt.to_radians(), o.roll.to_radians());
                let r = o.distance.max(1.05);
                let (x, yu, z) = (r * th.cos(), r * th.sin() * ti.sin(), r * th.sin() * ti.cos());
                let (xs, ys) = (x * ro.cos() - yu * ro.sin(), x * ro.sin() + yu * ro.cos());
                ks[i].0.cx = parent.cx + xs * parent.r;
                ks[i].0.cy = parent.cy - ys * parent.r;
                ks[i].1 = z;
                // Where the shadow falls: along the sunlight from the moon to the planet's face.
                let s = parent.s;
                let ms = xs * s[0] + ys * s[1] + z * s[2];
                let m2 = xs * xs + ys * ys + z * z;
                let disc = ms * ms - m2 + 1.0;
                let t = ms - disc.max(0.0).sqrt();
                let q = [xs - t * s[0], ys - t * s[1], z - t * s[2]];
                if disc > 0.0 && t > 0.0 && q[2] > 0.0 { ks[j].0.sh = [q[0], -q[1], ks[i].0.r / parent.r]; }
            }
            // Back to front: moons behind their planets first.
            ks.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
            ks.into_iter().map(|(k, _)| k).collect()
        }
        None => Vec::new(),
    }
}

/// Space behind everything (see `backdrop_px`), over every empty pixel it reaches.
pub(super) fn draw_backdrop(ctx: &Ctx, gbuf: &[GPixel], hdr: &mut [[f32; 3]]) {
    let Some(k) = space_k(ctx) else { return };
    let bh = super::blackhole::bh_k(ctx);
    let w = ctx.view.width;
    hdr.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        if (y as f32) >= k.hy && !k.below { return; }
        for (x, px) in row.iter_mut().enumerate() {
            if gbuf[y * w + x].id == id::NONE { *px = backdrop_px(&k, bh.as_ref(), x, y, *px); }
        }
    });
}

/// The planets, over every empty pixel inside their reach.
pub(super) fn draw_planets(ctx: &Ctx, gbuf: &[GPixel], hdr: &mut [[f32; 3]]) {
    let Some(k) = space_k(ctx) else { return };
    let (w, h) = (ctx.view.width, ctx.view.height);
    for p in planet_list(ctx) {
        let e = p.reach();
        let (y0, y1) = ((p.cy - e).floor().max(0.0) as usize, ((p.cy + e).ceil().max(0.0) as usize).min(h));
        let (x0, x1) = ((p.cx - e).floor().max(0.0) as usize, ((p.cx + e).ceil().max(0.0) as usize).min(w));
        if y0 >= y1 || x0 >= x1 { continue; }
        hdr[y0 * w..y1 * w].par_chunks_mut(w).enumerate().for_each(|(j, row)| {
            let y = y0 + j;
            if (y as f32) >= k.hy && !k.below { return; }
            for x in x0..x1 {
                if gbuf[y * w + x].id == id::NONE { row[x] = planet_px(&p, x, y, row[x]); }
            }
        });
    }
}

/// Value noise repeating every `nx` along x and every `ny` along y.
fn noise_wrap2(x: f32, y: f32, nx: i32, ny: i32) -> f32 {
    let (xf, yf) = (x.floor(), y.floor());
    let (fx, fy) = (x - xf, y - yf);
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let (x0, y0) = ((xf as i32).rem_euclid(nx), (yf as i32).rem_euclid(ny));
    let (x1, y1) = ((x0 + 1).rem_euclid(nx), (y0 + 1).rem_euclid(ny));
    let h = |a: i32, b: i32| cell_hash(a, b, 0x52);
    let a = h(x0, y0) + (h(x1, y0) - h(x0, y0)) * sx;
    let b = h(x0, y1) + (h(x1, y1) - h(x0, y1)) * sx;
    a + (b - a) * sy
}

/// A tunnel for one frame (`Tunnel`), as both renderers draw it.
#[derive(Clone, Copy, Debug)]
pub(super) struct TunnelK {
    /// 0 hyperspace, 1 wormhole.
    pub kind: f32,
    /// The vanishing point, pixels, and the focal length.
    pub vx: f32,
    pub vy: f32,
    pub focal: f32,
    pub radius: f32,
    /// How far down the tube the walls have come (the walk and the rush), metres, within a loop.
    pub travel: f32,
    /// Turns spun so far, and turns wound per metre.
    pub spin: f32,
    pub twist: f32,
    pub loop_len: f32,
    /// Hyperspace: streak lanes round the tube, and their repeat along it (dividing the loop).
    pub lanes: f32,
    pub period: f32,
    /// Wormhole: noise cells round the tube and along one loop (whole numbers).
    pub nx: f32,
    pub ny: f32,
    pub c0: [f32; 3],
    pub c1: [f32; 3],
    pub c2: [f32; 3],
    pub k: f32,
    pub core: f32,
    /// The frame's pixel scale (height / 854), and how deep the far end's light reaches.
    pub scale: f32,
    pub fade: f32,
}

pub(super) fn tunnel_k(ctx: &Ctx) -> Option<TunnelK> {
    let sky = &ctx.scene.sky;
    let t = &sky.tunnel;
    if !sky.enabled || !t.enabled || t.intensity <= 0.0 { return None; }
    let v = &ctx.view;
    let r = t.radius.max(0.5);
    let ll = ctx.loop_len;
    let along = |len: f32| (ll / len).round().max(1.0);
    Some(TunnelK {
        kind: match t.kind { TunnelKind::Hyperspace => 0.0, TunnelKind::Wormhole => 1.0 },
        vx: v.center_px,
        vy: v.horizon_px,
        focal: v.focal_px.max(1.0),
        radius: r,
        travel: (ctx.scroll + t.rush as f32 * ll * ctx.tphase).rem_euclid(ll),
        spin: (t.spin as f32 * ctx.tphase).rem_euclid(1.0),
        twist: t.twist as f32 / ll,
        loop_len: ll,
        lanes: (120.0 * t.density.max(0.05)).round().max(8.0),
        period: ll / along(6.0 * r),
        nx: (10.0 * t.density.max(0.05)).round().max(2.0),
        ny: along(10.0 * r / t.density.max(0.05)),
        c0: rgb_lin(t.colors[0]),
        c1: rgb_lin(t.colors[1]),
        c2: rgb_lin(t.colors[2]),
        k: t.intensity,
        core: t.core.max(0.0),
        scale: ((v.height as f32 - v.top as f32) / 854.0).max(0.25),
        fade: 40.0 * r,
    })
}

/// The light a tunnel's colour sheds on the path, added to the ambient.
pub(super) fn tunnel_glow(sky: &crate::scene::Sky) -> [f32; 3] {
    let t = &sky.tunnel;
    if !sky.enabled || !t.enabled { return [0.0; 3]; }
    scale3(mix3(rgb_lin(t.colors[1]), rgb_lin(t.colors[0]), 0.4), 0.12 * t.light.max(0.0) * t.intensity.max(0.0))
}

/// The tunnel seen at pixel (x, y).
pub(super) fn tunnel_px(k: &TunnelK, x: usize, y: usize) -> [f32; 3] {
    let (dx, dy) = (x as f32 + 0.5 - k.vx, y as f32 + 0.5 - k.vy);
    let rs = (dx * dx + dy * dy).sqrt().max(0.5);
    // Where the tube's wall is seen at this pixel: its depth down the tube, and round it.
    let z = k.radius * k.focal / rs;
    let w = k.travel + z;
    let mut a = dy.atan2(dx) / TAU + 0.5 + k.spin + k.twist * w;
    let far = 1.0 - (-z / k.fade).exp();
    let mut tube;
    if k.kind < 0.5 {
        let lane = a.rem_euclid(1.0) * k.lanes;
        let li = lane.floor();
        let fl = lane - li - 0.5;
        let ln = (li as i32).rem_euclid(k.lanes as i32);
        let spacing = TAU * rs / k.lanes;
        let width = 0.9 * k.scale;
        let mut v = 0.0;
        if cell_hash(ln, 7, 0x71) < 0.7 {
            let len = 0.25 + 0.5 * cell_hash(ln, 2, 0x73);
            let s = (w / k.period + cell_hash(ln, 1, 0x72)).rem_euclid(1.0);
            let along = smoothstep(0.0, 0.04, s) * (1.0 - smoothstep(len * 0.6, len, s));
            v = along * smoothstep(width, 0.0, fl.abs() * spacing);
        }
        // Where the lanes crowd together (toward the far end) they blend into their average.
        let mean = 0.7 * 0.4 * (2.0 * width / spacing).min(1.0);
        v += (mean - v) * smoothstep(3.0, 1.0, spacing / width);
        let walls = 0.5 + 0.5 * noise_wrap2(a.rem_euclid(1.0) * 12.0, w / k.loop_len * k.ny * 2.0, 12, (k.ny * 2.0) as i32);
        let hue = mix3(k.c1, k.c2, 0.5 * cell_hash(ln, 3, 0x74));
        tube = add3(scale3(k.c0, walls), scale3(hue, 1.6 * v * (1.0 - far)));
    } else {
        // The throat spirals: the further down, the further round.
        a += 0.25 * (1.0 + z / k.radius).ln();
        let au = a.rem_euclid(1.0);
        let yw = w / k.loop_len * k.ny;
        let (nx, ny) = (k.nx as i32, k.ny as i32);
        let val = 0.55 * noise_wrap2(au * k.nx, yw, nx, ny) + 0.3 * noise_wrap2(au * k.nx * 2.0, yw * 2.0, nx * 2, ny * 2)
            + 0.15 * noise_wrap2(au * k.nx * 4.0, yw * 4.0, nx * 4, ny * 4);
        let bands = smoothstep(0.35, 0.8, val);
        let f = 1.0 - (2.0 * noise_wrap2(au * k.nx * 3.0 + 0.5 * val, yw * 3.0, nx * 3, ny * 3) - 1.0).abs();
        let f2 = f * f;
        let fil = f2 * f2 * f2;
        tube = add3(scale3(k.c0, 0.4 + 0.6 * val), scale3(k.c1, (0.9 * bands + 0.7 * fil) * (1.0 - 0.6 * far)));
    }
    // The far end: the walls fade into its light, and a glow round it.
    let q = rs / k.focal;
    tube = mix3(tube, scale3(k.c2, 0.6 * k.core), far);
    tube = add3(tube, scale3(k.c2, 1.2 * k.core * (-q / 0.03).exp()));
    scale3(add3(tube, scale3(k.c1, 0.15 * k.core * (-q / 0.15).exp())), k.k)
}

/// The tunnel over every empty pixel (it hides the sky behind it).
pub(super) fn draw_tunnel(ctx: &Ctx, gbuf: &[GPixel], hdr: &mut [[f32; 3]]) {
    let Some(k) = tunnel_k(ctx) else { return };
    let w = ctx.view.width;
    hdr.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        for (x, px) in row.iter_mut().enumerate() {
            if gbuf[y * w + x].id == id::NONE { *px = tunnel_px(&k, x, y); }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preset(n: &str) -> Scene { crate::scene::presets::ALL.iter().find(|(m, _)| *m == n).unwrap().1() }
    fn diff(a: &Image, b: &Image) -> f32 { a.rgba.iter().zip(&b.rgba).map(|(x, y)| (*x as f32 - *y as f32).abs()).sum::<f32>() / a.rgba.len() as f32 }
    fn opts(t: f32) -> RenderOptions { RenderOptions { size: Some((90, 160)), time: Some(t), ..RenderOptions::default() } }

    #[test]
    fn space_and_its_companion_keep_the_loop_seamless() {
        let mut r = WorldRenderer::default();
        // Star Bridge has a spinning planet; a ship that bobs and weaves flies beside it.
        let mut s = preset("Star Bridge");
        s.companions = vec![crate::scene::Companion::default()];
        let (len, secs) = (s.motion.loop_length, s.motion.loop_seconds());
        assert_eq!(r.render(&s, 0.0, &opts(0.0)).rgba, r.render(&s, len, &opts(secs)).rgba, "the loop must close");
        let c = r.render(&s, 7.3, &opts(2.1));
        let d = r.render(&s, 7.3 + len, &opts(2.1 + secs));
        assert!(diff(&c, &d) < 0.06, "a whole loop of time later the frame must repeat ({})", diff(&c, &d));
        // Standing still, the ship and the planet still move.
        let e = r.render(&s, 7.3, &opts(2.1 + secs * 0.37));
        assert!(diff(&c, &e) > 0.2, "the companion should move with time alone ({})", diff(&c, &e));
    }

    #[test]
    fn tunnels_and_edge_lights_keep_the_loop_seamless() {
        let mut r = WorldRenderer::default();
        // Hyperspace Run: rushing streaks, running edge lights, glowing panels and a ship; Wormhole:
        // a twisting, spinning throat.
        for name in ["Hyperspace Run", "Wormhole", "Starship Flight", "Hyperspace Jump"] {
            let s = preset(name);
            let (len, secs) = (s.motion.loop_length, s.motion.loop_seconds());
            assert_eq!(r.render(&s, 0.0, &opts(0.0)).rgba, r.render(&s, len, &opts(secs)).rgba, "{name}: the loop must close");
            let c = r.render(&s, 7.3, &opts(2.1));
            let d = r.render(&s, 7.3 + len, &opts(2.1 + secs));
            assert!(diff(&c, &d) < 0.06, "{name}: a whole loop of time later the frame must repeat ({})", diff(&c, &d));
            let e = r.render(&s, 7.3, &opts(2.1 + secs * 0.37));
            assert!(diff(&c, &e) > 0.2, "{name}: the tunnel should move with time alone ({})", diff(&c, &e));
        }
    }

    #[test]
    fn open_flight_draws_no_ground() {
        let mut r = WorldRenderer::default();
        let s = preset("Starship Flight");
        let img = r.render(&s, 3.0, &RenderOptions { size: Some((90, 160)), time: Some(0.5), pick: true, ..RenderOptions::default() });
        let pick = img.pick.expect("pick ids");
        assert!(!pick.ids.iter().any(|&id| id == PICK_PATH || id == PICK_VERGE), "with path.surface off nothing of the ground may be drawn");
        let mut walk = s.clone();
        walk.path.surface = true;
        let img = r.render(&walk, 3.0, &RenderOptions { size: Some((90, 160)), time: Some(0.5), pick: true, ..RenderOptions::default() });
        assert!(img.pick.unwrap().ids.iter().any(|&id| id == PICK_PATH), "the same scene with its surface on shows the path");
    }

    #[test]
    fn glowing_materials_and_edge_lights_add_light() {
        let mut r = WorldRenderer::default();
        let mut s = preset("Hyperspace Run");
        s.sky.tunnel.enabled = false;
        s.companions.clear();
        let sum = |img: &Image| img.rgba.iter().map(|v| *v as f32).sum::<f32>();
        let lit = sum(&r.render(&s, 3.0, &opts(0.5)));
        s.path.edge_lights.enabled = false;
        let no_edges = sum(&r.render(&s, 3.0, &opts(0.5)));
        s.path.material.glow = 0.0;
        s.path.bridge.deck.glow = 0.0;
        let no_glow = sum(&r.render(&s, 3.0, &opts(0.5)));
        assert!(lit > no_edges * 1.01, "edge lights should brighten the deck ({lit} vs {no_edges})");
        // The railings stand on the deck now and hide some of it, so the bars show a little less.
        assert!(no_edges > no_glow * 1.005, "the panels' light bars should glow ({no_edges} vs {no_glow})");
    }

    #[test]
    fn space_shows_below_the_horizon_only_when_asked() {
        let mut r = WorldRenderer::default();
        let mut s = preset("Star Bridge");
        s.companions.clear();
        s.props.clear();
        // The band just under the horizon (0.4 of the height), where the drop shows beside the deck.
        let lower = |img: &Image| { let n = img.rgba.len(); img.rgba[n * 45 / 100..n * 70 / 100].iter().map(|v| *v as f32).sum::<f32>() };
        let on = r.render(&s, 3.0, &opts(0.5));
        s.sky.space.below = false;
        let off = r.render(&s, 3.0, &opts(0.5));
        assert!(lower(&on) > lower(&off) * 1.05, "stars and nebula should fill the drop beside the deck ({} vs {})", lower(&on), lower(&off));
    }

    #[test]
    fn planet_numbers_sit_where_the_shader_reads_them() {
        // Each field packed as its own position + 1, so a moved field shows which.
        let k = PlanetK { kind: 1.0, cx: 2.0, cy: 3.0, r: 4.0, col: [5.0, 6.0, 7.0], col2: [8.0, 9.0, 10.0], atm: [11.0, 12.0, 13.0], atm_k: 14.0,
            s: [15.0, 16.0, 17.0], tc: 18.0, ts: 19.0, lon0: 20.0, clouds: 21.0, lights: 22.0, so: 23.0, ring_on: 24.0, r_in: 25.0, r_out: 26.0,
            r_open: 27.0, r_c: 28.0, r_s: 29.0, r_col: [30.0, 31.0, 32.0], r_op: 33.0, sh: [34.0, 35.0, 36.0] };
        for (i, v) in k.pack().iter().enumerate() {
            assert_eq!(*v, (i + 1) as f32, "float {i} of a packed planet moved");
        }
        // And sky.wgsl reads them from those places.
        let wgsl = include_str!("../gpu/sky.wgsl");
        for read in ["pl3(b, 4u)", "pl3(b, 7u)", "pl3(b, 10u)", "pl(b, 13u)", "pl3(b, 14u)", "pl(b, 17u)", "pl(b, 18u)", "pl(b, 19u)",
            "pl(b, 20u)", "pl(b, 21u)", "pl(b, 22u)", "pl(b, 23u)", "pl(b, 24u)", "pl(b, 25u)", "pl(b, 26u)", "pl(b, 27u)", "pl(b, 28u)",
            "pl3(b, 29u)", "pl(b, 32u)", "pl(b, 33u)", "pl(b, 34u)", "pl(b, 35u)", &format!("k * {}u", PLANET_FLOATS)] {
            assert!(wgsl.contains(read), "sky.wgsl no longer reads `{read}`");
        }
    }

    /// A space sky alone: no props, ship or planets, so only the backdrop shows above the horizon.
    fn bare_sky() -> Scene {
        let mut s = preset("Nebula Crossing");
        s.companions.clear();
        s.props.clear();
        s.sky.space.planets.clear();
        s
    }
    fn luma(img: &Image, top: f32) -> Vec<f32> {
        let n = (img.rgba.len() / 4) as f32 * top;
        img.rgba.chunks_exact(4).take(n as usize).map(|p| 0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32).collect()
    }

    #[test]
    fn realistic_stars_are_a_few_bright_among_many_faint() {
        let mut r = WorldRenderer::default();
        let mut s = bare_sky();
        s.sky.space.objects.clear();
        (s.sky.space.nebula, s.sky.space.galaxy) = (0.0, 0.0);
        let o = RenderOptions { size: Some((360, 640)), time: Some(0.5), ..RenderOptions::default() };
        let mut l = luma(&r.render(&s, 3.0, &o), 0.4);
        l.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let at = |q: f32| l[((l.len() - 1) as f32 * q) as usize];
        // Mostly dark sky between the stars, a scatter of faint ones, and a very few near white.
        assert!(at(0.5) < 40.0, "most of the sky should be dark ({})", at(0.5));
        assert!(at(0.9995) > 150.0, "a few stars should be bright ({})", at(0.9995));
        let bright = l.iter().filter(|v| **v > 130.0).count() as f32 / l.len() as f32;
        let faint = l.iter().filter(|v| **v > at(0.5) + 12.0 && **v < 100.0).count() as f32 / l.len() as f32;
        assert!(faint > 4.0 * bright, "faint stars should far outnumber bright ones ({faint} vs {bright})");
        // The classic field is the even one it was: no star much brighter than the rest.
        s.sky.space.star_style = crate::scene::StarStyle::Classic;
        let c = luma(&r.render(&s, 3.0, &o), 0.4);
        assert!(c.iter().any(|v| *v > 40.0), "the classic stars still show");
    }

    #[test]
    fn every_kind_of_space_object_draws_and_stays_put() {
        let mut r = WorldRenderer::default();
        let mut s = bare_sky();
        s.sky.space.objects.clear();
        let base = r.render(&s, 3.0, &opts(0.5));
        for kind in crate::scene::SpaceObjectKind::ALL {
            let mut t = s.clone();
            t.sky.space.objects = vec![crate::scene::SpaceObject { kind, colors: kind.colors(), pos: [0.5, 0.4], size: 0.25, spin: 0, ..crate::scene::SpaceObject::default() }];
            let a = r.render(&t, 3.0, &opts(0.5));
            assert!(diff(&a, &base) > 0.3, "{kind:?} should show ({})", diff(&a, &base));
            // At infinity: walking on and waiting change nothing of it (no twinkle here).
            t.sky.space.twinkle = 0.0;
            let (b, c) = (r.render(&t, 3.0, &opts(0.5)), r.render(&t, 9.0, &opts(1.7)));
            let sky = |img: &Image| img.rgba[..img.rgba.len() * 3 / 10].to_vec();
            assert_eq!(sky(&b), sky(&c), "{kind:?} should stay put in the sky");
        }
    }

    #[test]
    fn spinning_objects_and_orbiting_moons_move_and_keep_the_loop() {
        let mut r = WorldRenderer::default();
        let mut s = preset("Asteroid Belt");
        s.companions.clear();
        s.props.clear();
        let (len, secs) = (s.motion.loop_length, s.motion.loop_seconds());
        assert_eq!(r.render(&s, 0.0, &opts(0.0)).rgba, r.render(&s, len, &opts(secs)).rgba, "the loop must close");
        let (a, b) = (r.render(&s, 3.0, &opts(secs * 0.1)), r.render(&s, 3.0, &opts(secs * 0.4)));
        assert!(diff(&a, &b) > 0.2, "the ship, pulsar, belt and moon move with time ({})", diff(&a, &b));
    }

    #[test]
    fn a_moon_in_front_of_its_planet_casts_its_shadow_on_it() {
        let ctx_planets = |phase: f32| {
            let mut s = preset("Asteroid Belt");
            s.sky.space.planets[1].orbit.turns = 0;
            s.sky.space.planets[1].orbit.phase = phase;
            s.sky.space.planets[1].radius = 0.06;
            let mut r = WorldRenderer::default();
            r.render(&s, 3.0, &RenderOptions { size: Some((240, 427)), time: Some(0.5), ..RenderOptions::default() })
        };
        // A quarter round: in front of the planet (its shadow falls on it); three quarters: behind.
        let (front, behind) = (ctx_planets(0.25), ctx_planets(0.75));
        assert!(diff(&front, &behind) > 0.1, "in front the moon and its shadow show on the planet");
    }

    #[test]
    fn twinkling_stars_keep_the_loop_seamless() {
        let mut r = WorldRenderer::default();
        let mut s = bare_sky();
        s.sky.space.twinkle = 1.0;
        let (len, secs) = (s.motion.loop_length, s.motion.loop_seconds());
        assert_eq!(r.render(&s, 0.0, &opts(0.0)).rgba, r.render(&s, len, &opts(secs)).rgba, "the loop must close");
        let a = r.render(&s, 3.0, &opts(secs * 0.2));
        let b = r.render(&s, 3.0, &opts(secs * 0.45));
        assert!(diff(&a, &b) > 0.05, "the stars should twinkle with time alone ({})", diff(&a, &b));
    }

    #[test]
    fn space_numbers_sit_where_the_shader_reads_them() {
        let wgsl = include_str!("../gpu/sky.wgsl");
        assert!(wgsl.contains(&format!("S.sx_prims + {}u + i * {}u", SPACE_HEAD, OBJ_FLOATS)), "sky.wgsl must step objects as SpaceK::pack lays them out");
        // The kinds in the order ObjK numbers them.
        assert_eq!(crate::scene::SpaceObjectKind::ALL.len(), 14);
        assert!(wgsl.contains("} else if kind == 13 {"), "sky.wgsl's object kinds");
    }
}
