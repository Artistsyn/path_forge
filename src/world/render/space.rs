//! Deep space in a world's sky: a dense starfield, nebulae, a galaxy band and planets, and space
//! continuing below the horizon (`sky.space.below`).
//!
//! Everything sits at infinity, so nothing moves as the camera walks; only a planet's spin
//! moves, in whole turns per loop. `sky.wgsl` repeats every function here step for step (the GPU
//! sky pass), with the numbers `space_k` and `PlanetK::pack` hand it.

use super::*;
use crate::scene::{Planet, PlanetKind, TunnelKind};
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
    Some(SpaceK {
        below: s.below,
        scale: ((v.height as f32 - oy) / 854.0).max(0.25),
        stars: s.stars.clamp(0.0, 3.0),
        sb: s.star_brightness.max(0.0),
        star_var: s.star_colors.clamp(0.0, 1.0),
        cx: ox + 0.5 * fw,
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
    })
}

/// `looks::hash2` with a salt: several independent numbers per cell.
fn cell_hash(x: i32, y: i32, k: u32) -> f32 {
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
            kind: match p.kind { PlanetKind::Rocky => 0.0, PlanetKind::Gas => 1.0, PlanetKind::Earth => 2.0, PlanetKind::Ice => 3.0, PlanetKind::Lava => 4.0 },
            cx: ox + p.pos[0] * fw,
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
            self.r_col[0], self.r_col[1], self.r_col[2], self.r_op,
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
        out = mix3(out, col, cov);
    }
    if ring_a > 0.0 && ring_front { out = mix3(out, ring_col, ring_a); }
    out
}

/// The planets of a world's sky this frame, in drawing order.
pub(super) fn planet_list(ctx: &Ctx) -> Vec<PlanetK> {
    match space_k(ctx) {
        Some(_) => ctx.scene.sky.space.planets.iter().filter(|p| p.enabled && p.radius > 0.0).take(MAX_PLANETS).map(|p| PlanetK::new(ctx, p)).collect(),
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
        // Star Bridge has a spinning planet and a ship that bobs and weaves.
        let s = preset("Star Bridge");
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
        assert!(no_edges > no_glow * 1.01, "the panels' light bars should glow ({no_edges} vs {no_glow})");
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
            r_open: 27.0, r_c: 28.0, r_s: 29.0, r_col: [30.0, 31.0, 32.0], r_op: 33.0 };
        for (i, v) in k.pack().iter().enumerate() {
            assert_eq!(*v, if i < 33 { (i + 1) as f32 } else { 0.0 }, "float {i} of a packed planet moved");
        }
        // And sky.wgsl reads them from those places.
        let wgsl = include_str!("../gpu/sky.wgsl");
        for read in ["pl3(b, 4u)", "pl3(b, 7u)", "pl3(b, 10u)", "pl(b, 13u)", "pl3(b, 14u)", "pl(b, 17u)", "pl(b, 18u)", "pl(b, 19u)",
            "pl(b, 20u)", "pl(b, 21u)", "pl(b, 22u)", "pl(b, 23u)", "pl(b, 24u)", "pl(b, 25u)", "pl(b, 26u)", "pl(b, 27u)", "pl(b, 28u)",
            "pl3(b, 29u)", "pl(b, 32u)", &format!("k * {}u", PLANET_FLOATS)] {
            assert!(wgsl.contains(read), "sky.wgsl no longer reads `{read}`");
        }
    }
}
