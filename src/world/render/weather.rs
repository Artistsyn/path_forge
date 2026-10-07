//! Weather and the air in a world's frame: rain, snow, sleet and hail and what they do to the
//! ground, drips, wind, sandstorms, mist, sun shafts and lamp haloes, cloud shadows, heat shimmer,
//! the aurora and rainbows, and drops or frost on the lens.
//!
//! Everything that moves runs on loop time (`tphase`) in whole cycles per loop, or lives one cycle
//! at a time and is placed again by its cycle number modulo the cycles per loop; everything lying
//! on the ground is placed by distance along the path, periodic in the loop. So the last frame of
//! a loop leads into the first, and when the walk stops (an encounter) the weather keeps moving.

use super::*;

/// What the weather is doing in one world this frame, worked out once.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Wx {
    /// Wind across the path now, m/s (gusts included); positive blows to the right.
    pub wind: f32,
    /// How far plants lean and sway (0 = still), the steady lean with the wind, and the phase of
    /// the swaying.
    sway: f32,
    lean: f32,
    sway_phase: f32,
    /// The sky is open (no ceiling): rain and snow reach the ground.
    pub open: bool,
    /// Ground wetness and puddle cover (rain and sleet), and lying snow, 0..1.
    pub wet: f32,
    pub puddles: f32,
    pub snow: f32,
    track: f32,
    /// How hard rain is falling on the puddles (their rings).
    rings: f32,
    /// How much of the sky the haze of a heavy fall or a sandstorm hides (0..1).
    pub veil: f32,
    /// Strength of cloud shadows, and how far they drift sideways in one loop (metres).
    pub clouds: f32,
    cloud_travel: f32,
}

impl Wx {
    /// Sideways lean of a plant's top, as a share of its height, for one with phase `ph`.
    pub(super) fn sway_at(&self, ph: f32) -> f32 {
        if self.sway <= 0.0 { return 0.0; }
        self.sway * (self.lean + 0.35 * (self.sway_phase + ph).sin())
    }
}

/// How much a kind of prop bends in the wind: its top's lean, as a share of its height, in a
/// strong wind.
pub(super) fn sway_factor(kind: PropKind) -> f32 {
    match kind {
        PropKind::Reeds => 0.14,
        PropKind::Willow => 0.09,
        PropKind::Palm => 0.08,
        PropKind::Tree => 0.06,
        PropKind::Bush => 0.05,
        PropKind::Pine => 0.04,
        PropKind::DeadTree => 0.025,
        PropKind::Mushroom | PropKind::Cactus => 0.01,
        _ => 0.0,
    }
}

fn lum(c: [f32; 3]) -> f32 { 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2] }

/// Smooth value noise over the ground, 0..1: free across the path, periodic along it (the cell
/// is snapped to divide the loop), so it scrolls with the world and repeats with the loop.
fn noise2(x: f32, w: f32, cell: f32, loop_len: f32, seed: u32) -> f32 {
    let c = snap_to_loop(cell, loop_len);
    let n = (loop_len / c).round().max(1.0) as i64;
    let (tx, tw) = (x / cell, w / c);
    let (ix, iw) = (tx.floor() as i64, tw.floor() as i64);
    let s = |f: f32| f * f * (3.0 - 2.0 * f);
    let (fx, fw) = (s(tx - ix as f32), s(tw - iw as f32));
    let v = |a: i64, b: i64| hf(seed ^ (a as u32).wrapping_mul(0x9E37_79B1), b);
    // The two rows along the loop, wrapped once (one integer division, not four).
    let w0 = iw.rem_euclid(n);
    let w1 = if w0 + 1 == n { 0 } else { w0 + 1 };
    let (a, b) = (v(ix, w0), v(ix + 1, w0));
    let (c0, d0) = (v(ix, w1), v(ix + 1, w1));
    let top = a + (b - a) * fx;
    let bot = c0 + (d0 - c0) * fx;
    top + (bot - top) * fw
}

/// Ground noise drifting sideways `travel` metres a loop at its exact speed: two layers, each
/// living one loop and fading in and out, half a loop apart, so the loop still closes.
fn drifting_noise(x: f32, w: f32, cell: f32, travel: f32, ctx: &Ctx, seed: u32) -> f32 {
    let mut n = 0.0;
    for j in 0..2u32 {
        let ph = (ctx.tphase + j as f32 * 0.5).rem_euclid(1.0);
        let wt = (std::f32::consts::PI * ph).sin().powi(2);
        let xo = x - travel * (ph - 0.5) + j as f32 * 53.0;
        n += wt * (0.7 * noise2(xo, w, cell, ctx.loop_len, seed ^ 0x51 ^ j) + 0.3 * noise2(xo, w, cell * 0.37, ctx.loop_len, seed ^ 0x53 ^ j));
    }
    n
}

/// The weather's conditions in a world this frame.
pub(super) fn conditions(scene: &Scene, tphase: f32, loop_seconds: f32) -> Wx {
    let w = &scene.weather;
    let mut wx = Wx { open: !scene.ceiling.enabled, ..Wx::default() };
    if w.wind.enabled {
        let gc = cycles(0.22, loop_seconds);
        let ph = hf(w.wind.seed ^ 0x61, 0) * TAU;
        let g = 0.65 * (TAU * gc * tphase + ph).sin() + 0.35 * (TAU * 2.0 * gc * tphase + ph * 1.7).sin();
        let gusts = w.wind.gusts.clamp(0.0, 1.0);
        wx.wind = w.wind.speed * (1.0 + 0.6 * gusts * g);
        wx.sway = w.wind.sway.clamp(0.0, 1.0) * (w.wind.speed.abs() / 8.0).min(1.5);
        wx.lean = w.wind.speed.signum() * 0.6 * (1.0 + 0.6 * gusts * g);
        wx.sway_phase = TAU * cycles(0.45, loop_seconds) * tphase;
    }
    let p = &w.precipitation;
    if p.enabled && wx.open {
        let i = p.intensity.clamp(0.0, 1.0);
        if matches!(p.kind, PrecipKind::Rain | PrecipKind::Sleet) {
            // Even a light rain soon wets the ground; it stays wet after the rain stops.
            wx.wet = p.wetness.clamp(0.0, 1.0) * (0.4 + 0.6 * i.sqrt());
            wx.puddles = p.puddles.clamp(0.0, 1.0);
            wx.rings = i;
        }
        if matches!(p.kind, PrecipKind::Snow | PrecipKind::Sleet) {
            wx.snow = p.cover.clamp(0.0, 1.0) * if p.kind == PrecipKind::Sleet { 0.45 } else { 1.0 };
            wx.track = p.track.clamp(0.0, 1.0);
        }
    }
    let c = &scene.sky.clouds;
    if c.shadows > 0.0 && scene.sky.enabled && scene.sky.sun.enabled && scene.sky.sun.emits_light {
        wx.clouds = c.shadows.clamp(0.0, 1.0);
        let v = if w.wind.enabled { w.wind.speed } else { 1.5 * c.drift };
        wx.cloud_travel = v * loop_seconds;
    }
    wx
}

/// The haze of a heavy fall or a sandstorm, folded into the world's even fog. Returns the fog
/// and how much of the sun's light still gets through; sets how much of the sky is veiled.
pub(super) fn haze(scene: &Scene, fog: Option<([f32; 3], f32)>, wx: &mut Wx) -> (Option<([f32; 3], f32)>, f32) {
    let w = &scene.weather;
    let p = &w.precipitation;
    let horizon = if scene.sky.enabled { rgb_lin(scene.sky.horizon) } else { scale3(rgb_lin(scene.light.ambient_color), 0.5 * scene.light.ambient.max(0.0)) };
    let mut parts: Vec<([f32; 3], f32)> = Vec::new();
    if p.enabled && wx.open && p.haze > 0.0 {
        let k = match p.kind { PrecipKind::Rain => 0.5, PrecipKind::Snow => 1.0, PrecipKind::Sleet => 0.7, PrecipKind::Hail => 0.3 };
        let dens = 0.06 * k * p.haze.clamp(0.0, 1.0) * p.intensity.clamp(0.0, 1.0);
        // The air full of falling water or snow is the grey of the light in it; snow is brighter.
        let col = scale3(mix3(horizon, [lum(horizon); 3], 0.7), if p.kind == PrecipKind::Snow { 1.25 } else { 0.85 });
        if dens > 0.0 { parts.push((mul3(col, rgb_lin(p.tint)), dens)); }
    }
    if w.sandstorm.enabled && wx.open && w.sandstorm.intensity > 0.0 {
        let dens = 0.12 * w.sandstorm.intensity.clamp(0.0, 1.0);
        parts.push((scale3(rgb_lin(w.sandstorm.color), 0.3 + 0.9 * lum(horizon).min(1.0)), dens));
    }
    if parts.is_empty() { return (fog, 1.0); }
    let extra: f32 = parts.iter().map(|p| p.1).sum();
    let col = parts.iter().fold([0.0; 3], |a, (c, d)| add3(a, scale3(*c, d / extra)));
    let base = fog.map_or(0.0, |(_, d)| 1.0 / d);
    let col = match fog { Some((c, _)) => mix3(c, col, extra / (base + extra)), None => col };
    wx.veil = (1.0 - (-extra * 25.0).exp()).min(0.92);
    (Some((col, 1.0 / (base + extra))), (-extra * 30.0).exp())
}

/// What the weather does to a floor or wall pixel: wet and darker, glossy, puddles with rain
/// rings, lying snow and the trodden track through it. Takes the surface's albedo, gloss and
/// ripples and returns them changed, with the slope rain rings add to the water.
pub(super) fn surface(ctx: &Ctx, g: &GPixel, tex: u8, albedo: [f32; 3], gloss: f32, ripples: f32) -> ([f32; 3], f32, f32, (f32, f32)) {
    let wx = &ctx.wx;
    let (mut a, mut gl, mut rp, mut ring) = (albedo, gloss, ripples, (0.0, 0.0));
    let floor = matches!(g.id, id::GROUND | id::RISER) && matches!(tex, 0 | 1 | 4);
    let w = ctx.scroll + g.d;
    let seed = ctx.scene.weather.precipitation.seed;
    if wx.wet > 0.0 {
        if floor {
            a = scale3(a, 1.0 - if tex == 1 { 0.25 } else { 0.4 } * wx.wet);
            gl = gl.max(0.32 * wx.wet);
            rp = rp.max(0.35 * wx.wet);
            if wx.puddles > 0.0 && g.id == id::GROUND {
                let n = 0.65 * noise2(g.x, w, 1.6, ctx.loop_len, seed ^ 0x9D) + 0.35 * noise2(g.x, w, 0.55, ctx.loop_len, seed ^ 0x9E);
                let th = 1.0 - 0.75 * wx.puddles;
                let pm = smoothstep(th - 0.03, th + 0.03, n);
                if pm > 0.0 {
                    a = scale3(a, 1.0 - 0.45 * pm);
                    gl += (0.9 - gl) * pm;
                    rp += (0.04 - rp) * pm;
                    if wx.rings > 0.0 {
                        let (sx, sz) = rain_rings(ctx, g.x, w);
                        ring = (sx * pm, sz * pm);
                    }
                }
            }
        } else if matches!(g.id, id::WALL_L | id::WALL_R | id::FACADE | id::CLIFF) || id::is_rail(g.id) {
            a = scale3(a, 1.0 - 0.18 * wx.wet);
        }
    }
    if wx.snow > 0.0 {
        const SNOW: [f32; 3] = [0.80, 0.83, 0.88];
        let s = wx.snow;
        if floor && g.id == id::GROUND {
            let n = 0.55 * noise2(g.x, w, 0.5, ctx.loop_len, seed ^ 0xA1) + 0.3 * noise2(g.x, w, 0.17, ctx.loop_len, seed ^ 0xA2) + 0.15 * noise2(g.x, w, 1.6, ctx.loop_len, seed ^ 0xA4);
            // Thin snow is a dusting, thicker in hollows; deep snow covers everything.
            let th = 1.1 - 1.2 * s;
            let mut cov = smoothstep(th - 0.22, th + 0.22, n);
            if tex != 1 && wx.track > 0.0 {
                // Feet and wheels pack the middle of the path into grey slush.
                let edge = ctx.view.path_half_width(g.d).max(0.1);
                let t = wx.track * (1.0 - smoothstep(0.3, 0.75, g.x.abs() / edge));
                let slush = mix3(scale3(albedo, 0.7), [0.42, 0.43, 0.46], 0.55);
                a = mix3(a, slush, t * s.min(1.0) * 0.8);
                cov *= 1.0 - 0.85 * t;
            }
            a = mix3(a, SNOW, cov);
            gl *= 1.0 - cov;
            rp *= 1.0 - cov;
        } else if g.id == id::RISER {
            a = mix3(a, SNOW, 0.35 * s);
        } else if id::is_rail(g.id) && id::rail_parts(g.id).1 == id::TOP {
            // Snow lies along the top of a railing.
            a = mix3(a, SNOW, (1.6 * s).min(0.95));
        } else if matches!(g.id, id::WALL_L | id::WALL_R) {
            // A cap on the top of the wall, and drifts banked against its foot.
            let top = ctx.wall_top();
            let cap = 0.04 + 0.08 * s;
            let drift = 0.35 * s * (0.5 + noise2(0.0, w, 0.9, ctx.loop_len, seed ^ 0xA3));
            let on_top = if top.is_finite() { smoothstep(top - cap - 0.02, top - cap, g.y) } else { 0.0 };
            let at_foot = 1.0 - smoothstep(drift - 0.03, drift, g.y);
            a = mix3(a, SNOW, on_top.max(at_foot));
        }
    }
    (a, gl, rp, ring)
}

/// Slope of the water where raindrops land in a puddle: rings spreading and fading.
fn rain_rings(ctx: &Ctx, x: f32, w: f32) -> (f32, f32) {
    let cell = snap_to_loop(0.42, ctx.loop_len);
    let n = (ctx.loop_len / cell).round() as i64;
    let cr = cycles(1.4, ctx.scene.motion.loop_seconds());
    let seed = ctx.scene.weather.precipitation.seed ^ 0x5151;
    let (cx, cw) = ((x / cell).floor() as i64, (w / cell).floor() as i64);
    let (mut sx, mut sz) = (0.0f32, 0.0f32);
    for ox in -1..=1 {
        for ow in -1..=1 {
            let (ix, iw) = (cx + ox, cw + ow);
            let s = seed ^ (ix as u32).wrapping_mul(0x85EB_CA6B);
            let tt = hf(s ^ 0x31, iw.rem_euclid(n)) + cr * ctx.tphase;
            let k = (tt.floor() as i64).rem_euclid(cr as i64);
            let a = tt - tt.floor();
            let key = iw.rem_euclid(n) * 1031 + k;
            if hf(s ^ 0x32, key) > 0.35 + 0.6 * ctx.wx.rings { continue; }
            let (px, pw) = ((ix as f32 + hf(s ^ 0x33, key)) * cell, (iw as f32 + hf(s ^ 0x34, key)) * cell);
            let (dx, dw) = (x - px, w - pw);
            let r = (dx * dx + dw * dw).sqrt();
            let rad = 0.02 + 0.22 * a;
            // Further than 0.093 m from the ring the envelope is under exp(-7.06) < 1e-3 at any age:
            // the test below would drop it anyway, without the exp.
            if (r - rad).abs() > 0.093 { continue; }
            let env = (1.0 - a).powi(2) * (-((r - rad) / 0.035).powi(2)).exp();
            if env < 1e-3 { continue; }
            let slope = 0.25 * env * (TAU * (r - rad) / 0.045).sin();
            let rr = r.max(1e-4);
            sx += slope * dx / rr;
            sz += slope * dw / rr;
        }
    }
    (sx, sz)
}

/// How much sunlight cloud shadows leave on the ground at (x, d), 1 = none.
pub(super) fn cloud_shade(ctx: &Ctx, x: f32, d: f32) -> f32 {
    let wx = &ctx.wx;
    if wx.clouds <= 0.0 { return 1.0; }
    let n = drifting_noise(x, ctx.scroll + d, 11.0, wx.cloud_travel, ctx, ctx.scene.sky.clouds.seed ^ 0xC1);
    1.0 - wx.clouds * 0.85 * smoothstep(0.4, 0.58, n)
}

/// A soft streak from (x0, y0) to (x1, y1), brighter toward its head, behind whatever is nearer
/// than `z`.
#[allow(clippy::too_many_arguments)]
fn streak(gbuf: &[GPixel], hdr: &mut [[f32; 3]], w: usize, h: usize, z: f32, (x0, y0): (f32, f32), (x1, y1): (f32, f32), col: [f32; 3], alpha: f32, width: f32) {
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

/// A soft round dot of radius `r` pixels.
#[allow(clippy::too_many_arguments)]
fn dot(gbuf: &[GPixel], hdr: &mut [[f32; 3]], w: usize, h: usize, z: f32, (sx, sy): (f32, f32), r: f32, col: [f32; 3], alpha: f32) {
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

/// The light at a point in the air: every lamp near the camera, the ambient and sky alone farther
/// off (where the fog has most of it anyway).
fn air_light(ctx: &Ctx, x: f32, y: f32, d: f32) -> [f32; 3] { air_light_among(ctx, x, y, d, None) }

/// `air_light` looking only at the lamps in `near` (the ones that can reach this part of the screen).
fn air_light_among(ctx: &Ctx, x: f32, y: f32, d: f32, near: Option<&[u16]>) -> [f32; 3] {
    if d < 18.0 { ctx.light_among(x, y, d, None, 0, 1.0, None, near) } else { ctx.sky_lights.iter().fold(ctx.ambient, |a, s| add3(a, scale3(s.color, 0.5))) }
}

/// Half the width of the ground in view at the far end of `range`, or between the walls.
fn spread(ctx: &Ctx, range: f32) -> f32 {
    if ctx.scene.walls.enabled { ctx.wall_x(5.0) } else { 0.5 * ctx.view.width as f32 / ctx.view.focal_px * range + 2.0 }
}

/// A splash where a drop lands at (x, d), `age` 0..1 through it: a ring spreading on the ground
/// and a few droplets thrown up.
#[allow(clippy::too_many_arguments)]
fn splash(ctx: &Ctx, gbuf: &[GPixel], hdr: &mut [[f32; 3]], x: f32, d: f32, age: f32, col: [f32; 3], size: f32, seed: u32, key: i64) {
    let v = &ctx.view;
    let (w, h) = (v.width, v.height);
    let r = size * (0.015 + 0.07 * age);
    let fade = (1.0 - age) * 0.55;
    let c = v.to_cam(x, 0.0, d);
    if c[2] < 0.3 { return; }
    let per = (TAU * r * v.px_per_m(c[2]) / 1.5).clamp(8.0, 64.0) as i64;
    for k in 0..per {
        let th = TAU * k as f32 / per as f32;
        let p = v.to_cam(x + r * th.cos(), 0.004, d + r * th.sin());
        if let Some(s) = v.project(p) { dot(gbuf, hdr, w, h, p[2] - 0.02, (s[0], s[1]), 0.5, col, fade); }
    }
    for k in 0..3 {
        let th = TAU * hf(seed ^ 0x77, key * 4 + k);
        let up = size * 0.05 * (std::f32::consts::PI * age).sin() * (0.6 + 0.4 * hf(seed ^ 0x78, key * 4 + k));
        let p = v.to_cam(x + 0.8 * r * th.cos(), up, d + 0.8 * r * th.sin());
        if let Some(s) = v.project(p) { dot(gbuf, hdr, w, h, p[2] - 0.02, (s[0], s[1]), 0.6, col, fade * 1.2); }
    }
}

/// Rain, snow, sleet or hail falling everywhere the sky is open, splashes where it lands, and
/// far curtains of rain.
pub(super) fn draw_precip(ctx: &Ctx, gbuf: &[GPixel], hdr: &mut [[f32; 3]]) {
    let p = &ctx.scene.weather.precipitation;
    if !p.enabled || !ctx.wx.open || p.intensity <= 0.0 { return; }
    let v = &ctx.view;
    let (w, h) = (v.width, v.height);
    let ls = ctx.scene.motion.loop_seconds();
    let i = p.intensity.clamp(0.0, 1.0);
    let size = p.size.clamp(0.2, 5.0);
    // Falling drops are drawn out to a little over 20 m (past that they are too small to make out,
    // and the haze stands in for them), densely, so the air near the camera is full.
    let range = ctx.far.min(22.0);
    let lat = spread(ctx, range);
    let top = 10.0f32;
    let tint = rgb_lin(p.tint);
    let copies = (range / ctx.loop_len).ceil() as i64 + 1;
    let area = 2.0 * lat * ctx.loop_len;
    #[derive(Clone, Copy, PartialEq)]
    enum Fall { Drop, Flake, Slush, Hail }
    let species: &[(Fall, f32)] = match p.kind {
        PrecipKind::Rain => &[(Fall::Drop, 1.0)],
        PrecipKind::Snow => &[(Fall::Flake, 1.0)],
        PrecipKind::Sleet => &[(Fall::Drop, 0.5), (Fall::Slush, 0.5)],
        PrecipKind::Hail => &[(Fall::Hail, 1.0)],
    };
    // Drops (or flakes) per square metre of ground at full intensity.
    let dens = match p.kind { PrecipKind::Rain => 18.0, PrecipKind::Snow => 14.0, PrecipKind::Sleet => 16.0, PrecipKind::Hail => 6.0 };
    let wind = ctx.wx.wind;
    for (si, &(kind, share)) in species.iter().enumerate() {
        let base = p.seed ^ (si as u32 + 1).wrapping_mul(0x2545_F491);
        let speed = match kind { Fall::Drop => 9.0, Fall::Flake => 1.1, Fall::Slush => 3.5, Fall::Hail => 14.0 };
        // Whole falls per loop, so every drop is back at the top when the loop ends.
        let c = cycles(speed / top, ls);
        let fall_t = ls / c;
        let speed = top / fall_t;
        let wob_c = cycles(0.4, ls);
        let n = ((dens * share * i * area) as i64).min(30_000);
        for k in 0..n {
            let tt = hf(base ^ 0x11, k) + c * ctx.tphase;
            let f = tt - tt.floor();
            let cyc = (tt.floor() as i64).rem_euclid(c as i64);
            // Each fall starts somewhere new (the same places again every loop).
            let key = k * 1031 + cyc;
            let x0 = (hf(base ^ 0x12, key) * 2.0 - 1.0) * lat;
            let d0 = hf(base ^ 0x13, key) * ctx.loop_len;
            let y = top * (1.0 - f);
            let wob = if matches!(kind, Fall::Flake | Fall::Slush) { 0.25 * (TAU * wob_c * ctx.tphase + hf(base ^ 0x14, k) * TAU).sin() } else { 0.0 };
            // Blown across the band and back in at the other side, so a strong wind does not empty it.
            let x = (x0 + wind * f * fall_t + wob + lat).rem_euclid(2.0 * lat) - lat;
            for cpy in 0..copies {
                let d = (d0 - ctx.scroll).rem_euclid(ctx.loop_len) + cpy as f32 * ctx.loop_len;
                if d < 0.25 || d > range || !ctx.owns(x, y, d) { continue; }
                let c0 = v.to_cam(x, y, d);
                if c0[2] < 0.2 { continue; }
                let Some([sx, sy]) = v.project(c0) else { continue };
                if sx < -40.0 || sx > w as f32 + 40.0 || sy < -80.0 || sy > h as f32 + 4.0 { continue; }
                // Far off a drop fades into the haze (its colour goes the fog's way, and it thins).
                let fade = ctx.fog_t(c0[2]);
                if fade < 0.02 { continue; }
                let light = mul3(air_light(ctx, x, y, d), tint);
                let seen = |c: [f32; 3]| ctx.apply_fog(c, c0[2]);
                let pxm = v.px_per_m(c0[2]);
                match kind {
                    Fall::Drop => {
                        // Smeared over the frame's exposure: from where it was 1/30 s before.
                        let dt = 0.035;
                        let Some([ex, ey]) = v.project(v.to_cam(x - wind * dt, y + speed * dt, d)) else { continue };
                        let col = seen(mul3(light, [0.62, 0.66, 0.74]));
                        let near = smoothstep(0.3, 2.0, c0[2]);
                        streak(gbuf, hdr, w, h, c0[2], (ex, ey), (sx, sy), col, (0.18 + 0.22 * i) * (0.35 + 0.65 * near) * fade, (pxm * 0.0025 * size).max(0.7));
                    }
                    Fall::Hail => {
                        let Some([ex, ey]) = v.project(v.to_cam(x - wind * 0.012, y + speed * 0.012, d)) else { continue };
                        let col = seen(mul3(light, [0.9, 0.92, 0.95]));
                        streak(gbuf, hdr, w, h, c0[2], (ex, ey), (sx, sy), col, 0.5 * fade, (pxm * 0.007 * size).max(0.7));
                        dot(gbuf, hdr, w, h, c0[2], (sx, sy), (pxm * 0.0045 * size).max(0.5), col, 0.95 * fade);
                    }
                    Fall::Flake | Fall::Slush => {
                        let (col, r, a) = if kind == Fall::Flake { ([0.85, 0.87, 0.92], 0.011, 0.85) } else { ([0.62, 0.66, 0.72], 0.008, 0.6) };
                        let r = (pxm * r * size * (0.6 + 0.8 * hf(base ^ 0x15, k))).max(0.5);
                        dot(gbuf, hdr, w, h, c0[2], (sx, sy), r, seen(mul3(light, col)), a * fade.sqrt());
                    }
                }
            }
        }
    }
    // Splashes where drops land, near enough to see.
    if p.splashes > 0.0 && p.kind != PrecipKind::Snow {
        let cs = cycles(2.2, ls);
        let near_lat = lat.min(9.0);
        let n = ((p.splashes.clamp(0.0, 1.0) * i * 3.0 * 2.0 * near_lat * ctx.loop_len) as i64).min(8000);
        let life = 0.22;
        let hail = p.kind == PrecipKind::Hail;
        for k in 0..n {
            let tt = hf(p.seed ^ 0x21, k) + cs * ctx.tphase;
            let a = tt - tt.floor();
            if a > life { continue; }
            let key = k * 1031 + (tt.floor() as i64).rem_euclid(cs as i64);
            let x = (hf(p.seed ^ 0x22, key) * 2.0 - 1.0) * near_lat;
            let d = (hf(p.seed ^ 0x23, key) * ctx.loop_len - ctx.scroll).rem_euclid(ctx.loop_len);
            if !(0.4..=20.0).contains(&d) || !ctx.owns(x, 0.0, d) || ctx.on_bridge(d) && x.abs() > ctx.path_edge(d) { continue; }
            // Only on open ground the camera can see.
            let c = v.to_cam(x, 0.0, d);
            let Some([sx, sy]) = v.project(c) else { continue };
            let (xi, yi) = (sx as i64, sy as i64);
            if xi < 0 || yi < 0 || xi >= w as i64 || yi >= h as i64 { continue; }
            let g = &gbuf[yi as usize * w + xi as usize];
            if g.id != id::GROUND || (g.depth - c[2]).abs() > 0.15 * c[2] + 0.1 { continue; }
            let light = ctx.apply_fog(mul3(air_light(ctx, x, 0.05, d), tint), c[2]);
            let age = a / life;
            if hail {
                // A hailstone bouncing.
                let p3 = v.to_cam(x, 0.1 * size * (std::f32::consts::PI * age).sin(), d);
                if let Some(s) = v.project(p3) { dot(gbuf, hdr, w, h, p3[2] - 0.02, (s[0], s[1]), (v.px_per_m(p3[2]) * 0.0045 * size).max(0.5), mul3(light, [0.9, 0.92, 0.95]), 0.9 * (1.0 - age)); }
            } else {
                splash(ctx, gbuf, hdr, x, d, age, mul3(light, [0.75, 0.78, 0.85]), size, p.seed, key);
            }
        }
    }
    // Far off, the rain is curtains in the haze rather than drops.
    if p.haze > 0.0 && matches!(p.kind, PrecipKind::Rain | PrecipKind::Sleet) {
        let amt = p.haze.clamp(0.0, 1.0) * i * 0.1;
        let cyc = cycles(2.5, ls);
        let slant = wind * 0.05;
        let col = scale3(ctx.fog_col, ctx.gain * 1.3);
        let seed = p.seed;
        hdr.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
            for (x, px) in row.iter_mut().enumerate() {
                let g = &gbuf[y * w + x];
                let far = if g.id == id::NONE { 1.0 } else { smoothstep(25.0, 60.0, g.depth) };
                if far <= 0.0 { continue; }
                let col_id = ((x as f32 - slant * y as f32) / 2.0).floor() as i64;
                let speed = 1.0 + (hash(seed ^ 0x72, col_id) % 2) as f32;
                let s = (y as f32 / h as f32 * 2.5 + hf(seed ^ 0x71, col_id) - speed * cyc * ctx.tphase).rem_euclid(1.0);
                let st = smoothstep(0.75, 1.0, s) * (0.4 + 0.6 * hf(seed ^ 0x73, col_id));
                *px = add3(*px, scale3(col, amt * st * far));
            }
        });
    }
}

/// Water dripping from the ceiling, or from the tops of the walls: a bead forms, falls and
/// splashes, from the same spots every time.
pub(super) fn draw_drips(ctx: &Ctx, gbuf: &[GPixel], hdr: &mut [[f32; 3]]) {
    let dr = &ctx.scene.weather.drips;
    if !dr.enabled || dr.rate <= 0.0 { return; }
    let s = ctx.scene;
    let (top, from_walls) = if s.ceiling.enabled { (s.ceiling.height, false) } else if s.walls.enabled && s.walls.height > 0.0 { (s.walls.height, true) } else { return };
    let v = &ctx.view;
    let (w, h) = (v.width, v.height);
    let ls = s.motion.loop_seconds();
    let cd = cycles(0.4, ls);
    let period = ls / cd;
    let fall_t = (2.0 * top / 9.8).sqrt();
    // The share of each drip's cycle spent forming at the top (the rest falling and splashing).
    let form = (1.0 - (fall_t + 0.15) / period).clamp(0.1, 0.9);
    let range = ctx.far.min(25.0);
    let copies = (range / ctx.loop_len).ceil() as i64 + 1;
    let col = rgb_lin(dr.color);
    let lat = if s.walls.enabled { ctx.wall_x(5.0) } else { ctx.view.path_half_width(5.0) + 1.0 };
    let n = ((dr.rate * ctx.loop_len) as i64).clamp(1, 4000);
    for k in 0..n {
        let x = if from_walls { if k % 2 == 0 { -(lat - 0.04) } else { lat - 0.04 } } else { (hf(dr.seed ^ 0xD1, k) * 2.0 - 1.0) * lat * 0.9 };
        let d0 = hf(dr.seed ^ 0xD2, k) * ctx.loop_len;
        let a = (hf(dr.seed ^ 0xD3, k) + cd * ctx.tphase).rem_euclid(1.0);
        for cpy in 0..copies {
            let d = (d0 - ctx.scroll).rem_euclid(ctx.loop_len) + cpy as f32 * ctx.loop_len;
            if d < 0.3 || d > range || !ctx.owns(x, 0.0, d) { continue; }
            let z = v.to_cam(x, 0.0, d)[2];
            // Wet drops catch the light: a little brighter than what lights them.
            let light = |y: f32| ctx.apply_fog(scale3(mul3(air_light(ctx, x, y, d), col), 1.4), z);
            if a < form {
                let p = v.to_cam(x, top - 0.012, d);
                if let Some(sp) = v.project(p) { dot(gbuf, hdr, w, h, p[2] - 0.01, (sp[0], sp[1]), (v.px_per_m(p[2]) * 0.006 * a / form).max(0.4), light(top), 0.8); }
                continue;
            }
            let tf = (a - form) * period;
            let y = top - 4.9 * tf * tf;
            if y > 0.0 {
                let vel = 9.8 * tf;
                let p0 = v.to_cam(x, y, d);
                let (Some(s0), Some(s1)) = (v.project(p0), v.project(v.to_cam(x, y + vel * 0.03, d))) else { continue };
                streak(gbuf, hdr, w, h, p0[2], (s1[0], s1[1]), (s0[0], s0[1]), light(y), 0.7, (v.px_per_m(p0[2]) * 0.004).max(0.6));
            } else {
                let after = tf - fall_t;
                if after < 0.15 { splash(ctx, gbuf, hdr, x, d, after / 0.15, light(0.05), 0.6, dr.seed, k); }
            }
        }
    }
}

/// Sand streaming past on the wind.
pub(super) fn draw_sandstorm(ctx: &Ctx, gbuf: &[GPixel], hdr: &mut [[f32; 3]]) {
    let st = &ctx.scene.weather.sandstorm;
    if !st.enabled || !ctx.wx.open || st.intensity <= 0.0 { return; }
    let v = &ctx.view;
    let (w, h) = (v.width, v.height);
    let ls = ctx.scene.motion.loop_seconds();
    let k = st.intensity.clamp(0.0, 1.0);
    let dir = if ctx.wx.wind != 0.0 { ctx.wx.wind.signum() } else { 1.0 };
    let speed = ctx.wx.wind.abs().max(5.0 + 9.0 * k);
    let range = ctx.far.min(30.0);
    let lat = spread(ctx, range);
    let copies = (range / ctx.loop_len).ceil() as i64 + 1;
    let col = rgb_lin(st.color);
    let cl = cycles(2.0, ls);
    let life = ls / cl;
    let n = ((4.0 * k * 2.0 * lat * ctx.loop_len) as i64).min(20_000);
    for j in 0..n {
        let tt = hf(st.seed ^ 0xE1, j) + cl * ctx.tphase;
        let a = tt - tt.floor();
        let key = j * 1031 + (tt.floor() as i64).rem_euclid(cl as i64);
        let y = 3.0 * hf(st.seed ^ 0xE2, key).powi(2) + 0.02;
        let x = (hf(st.seed ^ 0xE3, key) * 2.0 - 1.0) * lat + dir * speed * (a - 0.5) * life;
        let d0 = hf(st.seed ^ 0xE4, key) * ctx.loop_len;
        let alpha = (std::f32::consts::PI * a).sin() * 0.45;
        for cpy in 0..copies {
            let d = (d0 - ctx.scroll).rem_euclid(ctx.loop_len) + cpy as f32 * ctx.loop_len;
            if d < 0.3 || d > range || !ctx.owns(x, y, d) { continue; }
            let c0 = v.to_cam(x, y, d);
            let (Some(s0), Some(s1)) = (v.project(c0), v.project(v.to_cam(x - dir * speed * 0.03, y, d))) else { continue };
            if s0[0] < -60.0 || s0[0] > w as f32 + 60.0 || s0[1] < 0.0 || s0[1] > h as f32 { continue; }
            // Lit sand, a little brighter than the haze it flies through, thinning into it.
            let lit = ctx.apply_fog(scale3(mul3(air_light(ctx, x, y, d), col), 1.3), c0[2]);
            streak(gbuf, hdr, w, h, c0[2], (s1[0], s1[1]), (s0[0], s0[1]), lit, alpha * ctx.fog_t(c0[2]).sqrt(), (v.px_per_m(c0[2]) * 0.004).max(0.6));
        }
    }
}

/// Wisps of mist rising from the ground, swelling and fading.
pub(super) fn draw_wisps(ctx: &Ctx, gbuf: &[GPixel], hdr: &mut [[f32; 3]]) {
    let m = &ctx.scene.weather.mist;
    if !m.enabled || m.wisps <= 0.0 { return; }
    let v = &ctx.view;
    let (w, h) = (v.width, v.height);
    let ls = ctx.scene.motion.loop_seconds();
    let cw = cycles(1.0 / 6.0, ls);
    let life = ls / cw;
    let range = ctx.far.min(30.0);
    let lat = if ctx.scene.walls.enabled { ctx.wall_x(5.0) } else { ctx.view.path_half_width(5.0) + 5.0 };
    let copies = (range / ctx.loop_len).ceil() as i64 + 1;
    let col = rgb_lin(m.color);
    let n = ((m.wisps.clamp(0.0, 1.0) * 1.6 * ctx.loop_len) as i64).min(2000);
    for j in 0..n {
        let tt = hf(m.seed ^ 0xF1, j) + cw * ctx.tphase;
        let a = tt - tt.floor();
        let key = j * 1031 + (tt.floor() as i64).rem_euclid(cw as i64);
        let x = (hf(m.seed ^ 0xF2, key) * 2.0 - 1.0) * lat + ctx.wx.wind * 0.5 * (a - 0.5) * life;
        let d0 = hf(m.seed ^ 0xF3, key) * ctx.loop_len;
        let y = 0.15 + m.height.max(0.2) * 1.4 * a;
        let size = 0.45 + 0.7 * a;
        let alpha = 0.22 * m.wisps.min(1.0) * (std::f32::consts::PI * a).sin();
        for cpy in 0..copies {
            let d = (d0 - ctx.scroll).rem_euclid(ctx.loop_len) + cpy as f32 * ctx.loop_len;
            if d < 0.8 || d > range || !ctx.owns(x, y, d) { continue; }
            let c = v.to_cam(x, y, d);
            let Some([sx, sy]) = v.project(c) else { continue };
            let r = v.px_per_m(c[2]) * size;
            let lit = ctx.apply_fog(mul3(air_light(ctx, x, y, d), col), c[2]);
            // Soft and wider than tall.
            let (rx, ry) = (r, r * 0.6);
            let (x0, x1) = (((sx - rx).floor().max(0.0)) as usize, ((sx + rx).ceil().min(w as f32 - 1.0)).max(0.0) as usize);
            let (y0, y1) = (((sy - ry).floor().max(0.0)) as usize, ((sy + ry).ceil().min(h as f32 - 1.0)).max(0.0) as usize);
            for py in y0..=y1 {
                for px in x0..=x1 {
                    let i = py * w + px;
                    if c[2] >= gbuf[i].depth + 0.5 { continue; }
                    let q = ((px as f32 + 0.5 - sx) / rx).powi(2) + ((py as f32 + 0.5 - sy) / ry).powi(2);
                    if q >= 1.0 { continue; }
                    hdr[i] = mix3(hdr[i], lit, alpha * (1.0 - q).powi(2));
                }
            }
        }
    }
}

/// How thick the mist is at (x, d) as a share of its density: drifting banks.
fn mist_patch(ctx: &Ctx, x: f32, d: f32) -> f32 {
    let m = &ctx.scene.weather.mist;
    let p = m.patchiness.clamp(0.0, 1.0);
    if p <= 0.0 { return 1.0; }
    let travel = if ctx.scene.weather.wind.enabled { ctx.scene.weather.wind.speed * 0.5 } else { 0.4 } * ctx.scene.motion.loop_seconds();
    let n = drifting_noise(x, ctx.scroll + d, 5.0, travel, ctx, m.seed ^ 0xB1);
    (1.0 - p + p * 2.0 * smoothstep(0.2, 0.8, n)).max(0.0)
}

/// Mist lying over the ground: each pixel takes as much of it as its line of sight passes
/// through, so near ground seen from above stays clear and the distance goes white.
pub(super) fn apply_mist(ctxs: &[Ctx], gbuf: &[GPixel], hdr: &mut [[f32; 3]]) {
    if !ctxs.iter().any(|c| c.scene.weather.mist.enabled && c.scene.weather.mist.density > 0.0) { return; }
    let w = ctxs[0].view.width;
    // The lamps that can reach the air each tile lights (the point air_light measures from).
    let tiles = super::tile_lights_at(ctxs, gbuf, |ctx, g| {
        let m = &ctx.scene.weather.mist;
        (g.id != id::NONE && g.d < 18.0).then(|| ctx.view.to_cam(g.x, (m.height.max(0.05) * 0.5).min(ctx.view.eye_height), g.d))
    });
    hdr.par_chunks_mut(w).enumerate().for_each(|(row, line)| {
        for (x, px) in line.iter_mut().enumerate() {
            let g = &gbuf[row * w + x];
            let ctx = &ctxs[(g.realm as usize).min(ctxs.len() - 1)];
            let m = &ctx.scene.weather.mist;
            if !m.enabled || m.density <= 0.0 { continue; }
            let v = &ctx.view;
            let eye = v.eye_height;
            let top = m.height.max(0.05);
            // Rise of the line of sight per metre ahead, and its length per metre ahead.
            let slope = (v.horizon_px - row as f32 - 0.5) / v.focal_px;
            let across = (x as f32 + 0.5 - v.center_px) / v.focal_px;
            let stretch = (1.0 + slope * slope + across * across).sqrt();
            let (len, z, gx, gd) = if g.id == id::NONE {
                // Open sky: only from inside the mist, looking out through its top.
                if eye >= top { continue; }
                let z = if slope > 1e-3 { ((top - eye) / slope).min(ctx.far) } else { ctx.far };
                (z * stretch, z, 0.0, z)
            } else {
                let z = g.depth.min(ctx.far * 2.0);
                let y_seen = eye + slope * z;
                let (lo, hi) = (eye.min(y_seen), eye.max(y_seen));
                let inside = if hi <= top { 1.0 } else if lo >= top { 0.0 } else { (top - lo) / (hi - lo).max(1e-4) };
                (z * stretch * inside, z, g.x, g.d)
            };
            if len <= 1e-3 { continue; }
            let tau = m.density.max(0.0) * mist_patch(ctx, gx, gd) * len;
            let a = 1.0 - (-tau).exp();
            if a < 0.003 { continue; }
            let light = if g.id == id::NONE { ctx.sky_lights.iter().fold(ctx.ambient, |a, s| add3(a, scale3(s.color, 0.5))) } else { air_light_among(ctx, gx, (top * 0.5).min(eye), gd, tiles.as_ref().map(|t| t.at(x, row))) };
            let col = ctx.apply_fog(mul3(rgb_lin(m.color), light), z * 0.5);
            *px = mix3(*px, col, a);
        }
    });
}

/// How much the air scatters light: a little always, more in fog, mist, haze and dust.
fn air_density(ctx: &Ctx) -> f32 {
    let s = ctx.scene;
    let fog = ctx.fog.map_or(0.0, |(_, d)| (25.0 / d).min(1.0) * 0.5);
    let mist = if s.weather.mist.enabled { 0.3 } else { 0.0 };
    (0.3 + fog + mist + ctx.wx.veil * 0.8).min(1.0)
}

/// Light made visible by the air: rays from the sun streaming past whatever stands against the
/// sky, and haloes round lamps. Worked out at half resolution and added over the frame.
pub(super) fn light_shafts(ctxs: &[Ctx], here: usize, gbuf: &[GPixel], hdr: &mut [[f32; 3]]) {
    let ctx = &ctxs[here];
    let ls = &ctx.scene.weather.light_shafts;
    if !ls.enabled || (ls.sun <= 0.0 && ls.lamps <= 0.0) { return; }
    let v = &ctx.view;
    let (w, h) = (v.width, v.height);
    let q = 2usize;
    let (mw, mh) = (w.div_ceil(q), h.div_ceil(q));
    let air = air_density(ctx);
    let mut add = vec![[0.0f32; 3]; mw * mh];
    let sky = &ctx.scene.sky;
    let hy = v.horizon_px.max(2.0);
    let sun_light = if sky.enabled && sky.sun.enabled && sky.sun.emits_light { ctx.sky_lights.first().map(|s| s.color) } else { None };
    if let (true, Some(sun_col)) = (ls.sun > 0.0, sun_light) {
        let (ox, oy) = (v.left as f32, v.top as f32);
        let (fw, fhy) = (w as f32 - 2.0 * ox, (hy - oy).max(2.0));
        let (sx, sy) = ((ox + sky.sun.pos[0] * fw) / q as f32, (oy + sky.sun.pos[1].clamp(0.0, 1.0) * fhy) / q as f32);
        // What the rays are made of: open sky, far brighter round the sun than anywhere else (as
        // the real sky is), so gaps in leaves or walls near the sun stream light.
        let glow_r = 0.28 * fhy / q as f32;
        let mask: Vec<f32> = (0..mw * mh).into_par_iter().map(|c| {
            let (cx, cy) = (c % mw * q, c / mw * q);
            let near_sun = 0.1 + 12.0 * (-(((c % mw) as f32 + 0.5 - sx).powi(2) + ((c / mw) as f32 + 0.5 - sy).powi(2)) / (glow_r * glow_r)).exp();
            let mut open = 0;
            for y in cy..(cy + q).min(h) {
                if y as f32 >= hy { continue; }
                for x in cx..(cx + q).min(w) { if gbuf[y * w + x].id == id::NONE { open += 1; } }
            }
            near_sun * open as f32 / (q * q) as f32
        }).collect();
        let steps = 48;
        let k = ls.sun.max(0.0) * air * 0.6;
        add.par_iter_mut().enumerate().for_each(|(c, o)| {
            let (px, py) = ((c % mw) as f32 + 0.5, (c / mw) as f32 + 0.5);
            let (dx, dy) = ((sx - px) / steps as f32, (sy - py) / steps as f32);
            let (mut x, mut y, mut wgt, mut acc) = (px, py, 1.0f32, 0.0f32);
            for _ in 0..steps {
                x += dx;
                y += dy;
                let (ix, iy) = (x as i64, y as i64);
                if ix >= 0 && iy >= 0 && (ix as usize) < mw && (iy as usize) < mh { acc += mask[iy as usize * mw + ix as usize] * wgt; }
                wgt *= 0.975;
            }
            // Light gathered along the way, saturating softly toward the sun's own glare.
            *o = add3(*o, scale3(sun_col, (1.0 - (-acc * 0.04).exp()) * k));
        });
    }
    if ls.lamps > 0.0 {
        let lights: Vec<&PointLight> = ctxs.iter().flat_map(|c| c.lights.iter()).collect();
        if !lights.is_empty() {
            let k = ls.lamps.max(0.0) * air * 0.012 * ctx.gain;
            let ray = |cx: usize, cy: usize| {
                let dir = [(cx as f32 + 0.5 - v.center_px) / v.focal_px, (v.horizon_px - cy as f32 - 0.5) / v.focal_px, 1.0];
                let dl = dot3(dir, dir).sqrt();
                ([dir[0] / dl, dir[1] / dl, dir[2] / dl], dl)
            };
            let cell_px = |c: usize| ((c % mw * q + q / 2).min(w - 1), (c / mw * q + q / 2).min(h - 1));
            // Which lamps can light the air along any ray of each tile of cells: a lamp adds only
            // where the line of sight passes within its radius, so one whose cone of reach misses
            // the tile's cone of rays (both sides of the eye) adds nothing there.
            const TILE: usize = 16;
            let (tc, tr) = (mw.div_ceil(TILE), mh.div_ceil(TILE));
            let lists: Vec<Vec<u16>> = (0..tc * tr).into_par_iter().map(|t| {
                let (c0, r0) = (t % tc * TILE, t / tc * TILE);
                let (c1, r1) = ((c0 + TILE).min(mw) - 1, (r0 + TILE).min(mh) - 1);
                let (xa, ya) = cell_px(r0 * mw + c0);
                let (xb, yb) = cell_px(r1 * mw + c1);
                let (mid, _) = ray((xa + xb) / 2, (ya + yb) / 2);
                let ang = |d: [f32; 3]| dot3(d, mid).clamp(-1.0, 1.0).acos();
                let edges = [(xa, ya), (xb, ya), (xa, yb), (xb, yb), ((xa + xb) / 2, ya), ((xa + xb) / 2, yb), (xa, (ya + yb) / 2), (xb, (ya + yb) / 2)];
                let spread = edges.iter().map(|&(x, y)| ang(ray(x, y).0)).fold(0.0f32, f32::max) * 1.05 + 0.01;
                lights.iter().enumerate().filter(|(_, l)| {
                    let dist = dot3(l.pos, l.pos).sqrt();
                    if dist <= l.radius * 1.01 { return true; }
                    let reach = (l.radius / dist).min(1.0).asin() + spread;
                    let th = ang([l.pos[0] / dist, l.pos[1] / dist, l.pos[2] / dist]);
                    th <= reach || th >= std::f32::consts::PI - reach
                }).map(|(i, _)| i as u16).collect()
            }).collect();
            add.par_iter_mut().enumerate().for_each(|(c, o)| {
                let (cx, cy) = cell_px(c);
                let (dir, dl) = ray(cx, cy);
                let depth = gbuf[cy * w + cx].depth.min(80.0);
                let reach = depth * dl;
                let mut glow = [0.0f32; 3];
                for l in lists[(c / mw / TILE) * tc + (c % mw) / TILE].iter().map(|&i| lights[i as usize]) {
                    // Light scattered toward the eye along the line of sight, from a point light
                    // in clear air: the integral of 1/r^2 along the ray.
                    let b = dot3(dir, l.pos);
                    let h2 = (dot3(l.pos, l.pos) - b * b).max(0.0);
                    if h2 > l.radius * l.radius { continue; }
                    let hh = (h2 + 0.02).sqrt();
                    let integ = (((reach - b) / hh).atan() - (-b / hh).atan()) / hh;
                    let fall = (1.0 - h2.sqrt() / l.radius).powi(2);
                    glow = add3(glow, scale3(l.color, integ * fall));
                }
                *o = add3(*o, scale3(glow, k));
            });
        }
    }
    hdr.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        for (x, px) in row.iter_mut().enumerate() {
            // Bilinear from the half-resolution cells.
            let fx = (x as f32 + 0.5) / q as f32 - 0.5;
            let fy = (y as f32 + 0.5) / q as f32 - 0.5;
            let (x0, y0) = (fx.floor().max(0.0) as usize, fy.floor().max(0.0) as usize);
            let (x1, y1) = ((x0 + 1).min(mw - 1), (y0 + 1).min(mh - 1));
            let (tx, ty) = ((fx - x0 as f32).clamp(0.0, 1.0), (fy - y0 as f32).clamp(0.0, 1.0));
            let top = mix3(add[y0 * mw + x0], add[y0 * mw + x1], tx);
            let bot = mix3(add[y1 * mw + x0], add[y1 * mw + x1], tx);
            *px = add3(*px, mix3(top, bot, ty));
        }
    });
}

/// The sky behind a heavy fall or a sandstorm, veiled by the haze in front of it.
pub(super) fn veil_sky(ctx: &Ctx, gbuf: &[GPixel], hdr: &mut [[f32; 3]]) {
    let veil = ctx.wx.veil;
    if veil <= 0.0 { return; }
    let col = ctx.fog_col;
    hdr.par_iter_mut().zip(gbuf.par_iter()).for_each(|(c, g)| if g.id == id::NONE { *c = mix3(*c, col, veil); });
}

/// Northern lights: curtains of light hanging in the sky, rippling and folding.
pub(super) fn aurora(ctx: &Ctx, gbuf: &[GPixel], hdr: &mut [[f32; 3]]) {
    let a = &ctx.scene.sky.aurora;
    if !a.enabled || a.intensity <= 0.0 { return; }
    let v = &ctx.view;
    let w = v.width;
    let hy = v.horizon_px.max(2.0);
    let (ox, oy) = (v.left as f32, v.top as f32);
    let (fw, fhy) = (w as f32 - 2.0 * ox, (hy - oy).max(2.0));
    let ls = ctx.scene.motion.loop_seconds();
    let sp = a.speed.max(0.0);
    let (c1, c2, c3) = (cycles(0.05 * sp, ls), cycles(0.09 * sp, ls), cycles(0.21 * sp, ls));
    let t = TAU * ctx.tphase;
    let (lo, hi) = (rgb_lin(a.low), rgb_lin(a.high));
    let seed = a.seed;
    let (p1, p2) = (hf(seed ^ 0x81, 0) * TAU, hf(seed ^ 0x82, 0) * TAU);
    let height = a.height.clamp(0.0, 1.0);
    let k = a.intensity.max(0.0) * 0.55;
    hdr.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        if y as f32 >= hy { return; }
        let ny = (y as f32 + 0.5 - oy) / fhy;
        for (x, px) in row.iter_mut().enumerate() {
            if gbuf[y * w + x].id != id::NONE { continue; }
            let nx = (x as f32 + 0.5 - ox) / fw;
            // The foot of the curtain wanders across the sky.
            let foot = 0.15 + 0.6 * height + 0.1 * (TAU * nx * 1.3 + c1 * t + p1).sin() + 0.04 * (TAU * nx * 3.7 - c2 * t + p2).sin();
            let up = foot - ny;
            let profile = if up < 0.0 { (-(up / 0.02).powi(2)).exp() } else { (-up / 0.22).exp() };
            if profile < 0.01 { continue; }
            // Folds of the curtain, and fine rays running up it.
            let fold = 0.5 + 0.5 * (TAU * (nx * 2.0 + 0.3 * (TAU * nx * 0.7 + c1 * t).sin()) + p2).sin();
            let ray = 0.55 + 0.45 * noise1(seed ^ 0x83, nx * 150.0 + 3.0 * (c3 * t + p1).sin());
            let col = mix3(lo, hi, (up / 0.3).clamp(0.0, 1.0));
            *px = add3(*px, scale3(col, k * profile * (0.25 + 0.75 * fold) * ray));
        }
    });
}

/// A rainbow round a point below the horizon, red outside, and a fainter, reversed second bow.
pub(super) fn rainbow(ctx: &Ctx, gbuf: &[GPixel], hdr: &mut [[f32; 3]]) {
    let r = &ctx.scene.sky.rainbow;
    if !r.enabled || r.intensity <= 0.0 { return; }
    let v = &ctx.view;
    let w = v.width;
    let hy = v.horizon_px.max(2.0);
    let (ox, oy) = (v.left as f32, v.top as f32);
    let fw = w as f32 - 2.0 * ox;
    let fhy = (hy - oy).max(2.0);
    // A real bow (42 degrees round the point opposite the sun) is wider than a portrait view, so
    // size 1 spans the frame instead; the band and the second bow keep their real proportions.
    let rad = 0.6 * fw * r.size.max(0.05);
    let (cx, cy) = (ox + r.x * fw, hy + 0.12 * rad);
    let band = 0.06 * rad;
    let (rad2, band2) = (1.21 * rad, 1.7 * band);
    let sun = ctx.sky_lights.first().map_or(0.6, |s| lum(s.color).clamp(0.2, 2.0));
    let k = r.intensity.max(0.0) * 0.32 * sun;
    let spectrum = |t: f32| -> [f32; 3] {
        // Violet (0) to red (1).
        let t = t.clamp(0.0, 1.0);
        [smoothstep(0.45, 0.85, t) + 0.35 * (1.0 - smoothstep(0.0, 0.15, t)), smoothstep(0.2, 0.5, t) * (1.0 - smoothstep(0.7, 0.9, t)), 1.0 - smoothstep(0.25, 0.55, t)]
    };
    hdr.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        if y as f32 >= hy { return; }
        // The bow fades into the haze near the horizon.
        let low = smoothstep(hy, hy - 0.08 * fhy, y as f32);
        for (x, px) in row.iter_mut().enumerate() {
            if gbuf[y * w + x].id != id::NONE { continue; }
            let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
            let mut c = [0.0f32; 3];
            let t1 = (d - (rad - band * 0.5)) / band;
            if (0.0..1.0).contains(&t1) { c = add3(c, scale3(spectrum(t1), (std::f32::consts::PI * t1).sin().sqrt())); }
            if d < rad - band * 0.5 { c = add3(c, [0.05; 3]); }
            if r.double {
                let t2 = (d - (rad2 - band2 * 0.5)) / band2;
                if (0.0..1.0).contains(&t2) { c = add3(c, scale3(spectrum(1.0 - t2), 0.4 * (std::f32::consts::PI * t2).sin().sqrt())); }
            }
            if c != [0.0; 3] { *px = add3(*px, scale3(c, k * low)); }
        }
    });
}

/// Heat haze: the picture wavers just above the horizon and over the distant ground.
pub(super) fn heat_shimmer(ctxs: &[Ctx], gbuf: &[GPixel], hdr: &mut Vec<[f32; 3]>) {
    if !ctxs.iter().any(|c| c.scene.weather.heat_shimmer.enabled && c.scene.weather.heat_shimmer.strength > 0.0) { return; }
    let (w, h) = (ctxs[0].view.width, ctxs[0].view.height);
    let src = hdr.clone();
    let unit = 854.0 / h as f32;
    hdr.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        for (x, px) in row.iter_mut().enumerate() {
            let g = &gbuf[y * w + x];
            let ctx = &ctxs[(g.realm as usize).min(ctxs.len() - 1)];
            let hs = &ctx.scene.weather.heat_shimmer;
            if !hs.enabled || hs.strength <= 0.0 { continue; }
            let hy = ctx.view.horizon_px;
            let yf = y as f32 + 0.5;
            let band = smoothstep(hy - 0.07 * h as f32, hy - 0.01 * h as f32, yf) * (1.0 - smoothstep(hy + 0.02 * h as f32, hy + 0.3 * h as f32, yf));
            let far = if g.id == id::NONE { 1.0 } else { smoothstep(5.0, 35.0, g.depth) };
            let amp = hs.strength * 2.5 / unit * band * far;
            if amp < 0.03 { continue; }
            let ls = ctx.scene.motion.loop_seconds();
            let (c1, c2) = (cycles(1.1 * hs.speed.max(0.0), ls), cycles(1.9 * hs.speed.max(0.0), ls));
            let t = TAU * ctx.tphase;
            let (fx, fy) = (x as f32 * unit, yf * unit);
            let dx = amp * (0.6 * (0.31 * fy + 0.05 * fx + c1 * t).sin() + 0.4 * (0.77 * fy - 0.09 * fx - c2 * t + 1.3).sin());
            let dy = amp * 0.3 * (0.21 * fx + 0.45 * fy + c2 * t).sin();
            *px = bilinear(&src, w, h, x as f32 + dx, y as f32 + dy);
        }
    });
}

fn bilinear(src: &[[f32; 3]], w: usize, h: usize, x: f32, y: f32) -> [f32; 3] {
    let x = x.clamp(0.0, w as f32 - 1.0);
    let y = y.clamp(0.0, h as f32 - 1.0);
    let (x0, y0) = (x.floor() as usize, y.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (tx, ty) = (x - x0 as f32, y - y0 as f32);
    let top = mix3(src[y0 * w + x0], src[y0 * w + x1], tx);
    let bot = mix3(src[y1 * w + x0], src[y1 * w + x1], tx);
    mix3(top, bot, ty)
}

/// Raindrops or frost on the lens, `weight` 0..1 (a crossing hands the lens from one world to
/// the next).
pub(super) fn lens(ctx: &Ctx, weight: f32, hdr: &mut Vec<[f32; 3]>) {
    let l = &ctx.scene.weather.lens;
    if !l.enabled || l.amount <= 0.0 || weight <= 0.0 { return; }
    let (w, h) = (ctx.view.width, ctx.view.height);
    let src = hdr.clone();
    let ls = ctx.scene.motion.loop_seconds();
    let amount = l.amount.clamp(0.0, 1.0);
    match l.kind {
        LensKind::Drops => {
            let cd = cycles(0.18, ls);
            let n = (amount * 30.0).round().max(1.0) as i64;
            for j in 0..n {
                let tt = hf(l.seed ^ 0x41, j) + cd * ctx.tphase;
                let a = tt - tt.floor();
                let key = j * 1031 + (tt.floor() as i64).rem_euclid(cd as i64);
                let r = (0.018 + 0.04 * hf(l.seed ^ 0x42, key)) * w as f32;
                let cx = hf(l.seed ^ 0x43, key) * w as f32;
                let mut cy = hf(l.seed ^ 0x44, key) * h as f32;
                // Some grow heavy and run down the glass.
                if hf(l.seed ^ 0x45, key) > 0.55 { cy += smoothstep(0.35, 1.0, a) * (0.15 + 0.3 * hf(l.seed ^ 0x46, key)) * h as f32; }
                let alpha = smoothstep(0.0, 0.06, a) * (1.0 - smoothstep(0.8, 1.0, a)) * weight;
                if alpha <= 0.0 { continue; }
                let ry = r * 1.1;
                for py in (cy - ry).floor().max(0.0) as usize..=((cy + ry).ceil() as usize).min(h - 1) {
                    for px in (cx - r).floor().max(0.0) as usize..=((cx + r).ceil() as usize).min(w - 1) {
                        let (dx, dy) = ((px as f32 + 0.5 - cx) / r, (py as f32 + 0.5 - cy) / ry);
                        let q = dx * dx + dy * dy;
                        if q >= 1.0 { continue; }
                        // A drop is a tiny fisheye: the scene upside down, squeezed, a little blurred.
                        let (sx, sy) = (cx - dx * r * 2.4, cy - dy * ry * 2.4);
                        let refr = scale3(add3(add3(bilinear(&src, w, h, sx - 1.0, sy), bilinear(&src, w, h, sx + 1.0, sy)), bilinear(&src, w, h, sx, sy + 1.0)), 1.0 / 3.0);
                        let rim = smoothstep(0.55, 1.0, q.sqrt());
                        let glint = (-((dx + 0.35).powi(2) + (dy + 0.4).powi(2)) / 0.02).exp();
                        let c = add3(scale3(refr, 1.0 - 0.55 * rim), scale3(add3(refr, [0.25; 3]), glint * 0.8));
                        let i = py * w + px;
                        hdr[i] = mix3(hdr[i], c, alpha * (1.0 - 0.3 * rim));
                    }
                }
            }
        }
        LensKind::Frost => {
            use super::super::looks::value_noise as vn;
            let mean = src.iter().map(|c| lum(*c)).sum::<f32>() / src.len().max(1) as f32;
            let frost = scale3([0.82, 0.88, 0.95], 0.15 + 0.85 * mean.min(1.5));
            let tw = cycles(0.5, ls);
            let th = 1.0 - 0.55 * amount;
            let aspect = w as f32 / h as f32;
            hdr.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
                for (x, px) in row.iter_mut().enumerate() {
                    let (u, v) = ((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
                    let (eu, ev) = ((2.0 * u - 1.0).abs(), (2.0 * v - 1.0).abs());
                    let e = eu.max(ev).max(((eu * eu + ev * ev).sqrt() / 1.41) * 0.95);
                    let n = 0.6 * vn(u * 9.0 * aspect + l.seed as f32, v * 9.0) + 0.4 * vn(u * 23.0 * aspect, v * 23.0 + l.seed as f32);
                    let fine = vn(u * 70.0 * aspect, v * 70.0);
                    // Feathery crystals: the edge of the frost is broken up by fine noise, and it
                    // thins into separate crystals as it reaches in.
                    let f = smoothstep(th - 0.06, th + 0.18, e + (n - 0.5) * 0.35) * (0.45 + 0.55 * smoothstep(0.3, 0.7, fine + (e - th) * 1.5));
                    if f <= 0.0 { continue; }
                    let blur = scale3(add3(add3(bilinear(&src, w, h, x as f32 - 2.0, y as f32), bilinear(&src, w, h, x as f32 + 2.0, y as f32)), add3(bilinear(&src, w, h, x as f32, y as f32 - 2.0), bilinear(&src, w, h, x as f32, y as f32 + 2.0))), 0.25);
                    let mut ice = add3(scale3(blur, 0.7), scale3(frost, 0.35 + 0.45 * fine));
                    // Ice crystals glint now and then.
                    if fine > 0.9 {
                        let ph = hf(l.seed ^ 0x49, (x as i64) * 7919 + y as i64) * TAU;
                        ice = add3(ice, scale3(frost, 1.5 * (TAU * tw * ctx.tphase + ph).sin().max(0.0).powi(8)));
                    }
                    *px = mix3(*px, ice, f * 0.92 * weight);
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preset(n: &str) -> Scene { crate::scene::presets::ALL.iter().find(|(m, _)| *m == n).unwrap().1() }
    fn diff(a: &Image, b: &Image) -> f32 { a.rgba.iter().zip(&b.rgba).map(|(x, y)| (*x as f32 - *y as f32).abs()).sum::<f32>() / a.rgba.len() as f32 }
    fn opts(t: f32) -> RenderOptions { RenderOptions { size: Some((90, 160)), time: Some(t), ..RenderOptions::default() } }

    /// Every effect on at once, in an open scene, a night scene, a desert and a roofed one.
    fn stormy() -> Vec<Scene> {
        let mut a = preset("Forest Path");
        a.weather.precipitation = Precipitation { enabled: true, intensity: 0.8, puddles: 0.6, ..Precipitation::default() };
        a.weather.wind = Wind { enabled: true, speed: 6.0, ..Wind::default() };
        a.weather.light_shafts = LightShafts { enabled: true, ..LightShafts::default() };
        a.weather.mist = Mist { enabled: true, ..Mist::default() };
        a.weather.heat_shimmer = HeatShimmer { enabled: true, ..HeatShimmer::default() };
        a.weather.lens = Lens { enabled: true, ..Lens::default() };
        a.sky.clouds.shadows = 0.8;
        a.sky.rainbow = Rainbow { enabled: true, ..Rainbow::default() };
        a.particles.push(Particles { kind: ParticleKind::Petals, count: 400, color: ParticleKind::Petals.default_color(), ..Particles::default() });
        let mut b = preset("Night Road");
        b.weather.precipitation = Precipitation { enabled: true, kind: PrecipKind::Snow, intensity: 0.7, ..Precipitation::default() };
        b.weather.wind = Wind { enabled: true, speed: -3.0, ..Wind::default() };
        b.weather.lens = Lens { enabled: true, kind: LensKind::Frost, ..Lens::default() };
        b.sky.aurora = Aurora { enabled: true, ..Aurora::default() };
        let mut c = preset("Desert Canyon");
        c.weather.sandstorm = Sandstorm { enabled: true, ..Sandstorm::default() };
        c.weather.precipitation = Precipitation { enabled: true, kind: PrecipKind::Hail, intensity: 0.5, ..Precipitation::default() };
        c.particles.push(Particles { kind: ParticleKind::Dust, count: 300, ..Particles::default() });
        c.weather.wind = Wind { enabled: true, speed: 9.0, ..Wind::default() };
        let mut d = preset("Mossy Sewer");
        d.weather.drips = Drips { enabled: true, rate: 3.0, ..Drips::default() };
        d.weather.precipitation = Precipitation { enabled: true, kind: PrecipKind::Sleet, ..Precipitation::default() };
        d.weather.light_shafts = LightShafts { enabled: true, ..LightShafts::default() };
        vec![a, b, c, d]
    }

    #[test]
    fn every_weather_effect_keeps_the_loop_seamless() {
        let mut r = WorldRenderer::default();
        for s in stormy() {
            let (len, secs) = (s.motion.loop_length, s.motion.loop_seconds());
            let a = r.render(&s, 0.0, &opts(0.0));
            let b = r.render(&s, len, &opts(secs));
            assert_eq!(a.rgba, b.rgba, "{}: the loop must close", s.name);
            // Stopped part-way (distance and time apart, as in an encounter), one loop of time later:
            // the same frame, to the last bit of the loop phase (t / loop_seconds wraps with rounding).
            let c = r.render(&s, 7.3, &opts(2.1));
            let d = r.render(&s, 7.3 + len, &opts(2.1 + secs));
            assert!(diff(&c, &d) < 0.06, "{}: a whole loop of time later the frame must repeat ({})", s.name, diff(&c, &d));
            // And the weather moves while the walk stands still.
            let e = r.render(&s, 7.3, &opts(2.1 + secs * 0.37));
            assert!(diff(&c, &e) > 0.2, "{}: the weather should move with time alone", s.name);
        }
    }

    #[test]
    fn each_effect_changes_the_picture() {
        let mut r = WorldRenderer::default();
        let cases: Vec<(&str, &str, Box<dyn Fn(&mut Scene)>)> = vec![
            ("rain", "Dark Street", Box::new(|s| { s.particles.clear(); s.weather.precipitation = Precipitation { enabled: true, ..Precipitation::default() }; })),
            ("snow", "Mountain Pass", Box::new(|s| s.weather.precipitation = Precipitation { enabled: true, kind: PrecipKind::Snow, ..Precipitation::default() })),
            ("hail", "Forest Path", Box::new(|s| s.weather.precipitation = Precipitation { enabled: true, kind: PrecipKind::Hail, ..Precipitation::default() })),
            ("drips", "Mossy Sewer", Box::new(|s| s.weather.drips = Drips { enabled: true, rate: 4.0, ..Drips::default() })),
            ("wind", "Forest Path", Box::new(|s| s.weather.wind = Wind { enabled: true, speed: 9.0, sway: 1.0, ..Wind::default() })),
            ("sandstorm", "Desert Canyon", Box::new(|s| s.weather.sandstorm = Sandstorm { enabled: true, ..Sandstorm::default() })),
            ("mist", "Haunted Forest", Box::new(|s| s.weather.mist = Mist { enabled: true, ..Mist::default() })),
            ("sun shafts", "Forest Path", Box::new(|s| s.weather.light_shafts = LightShafts { enabled: true, sun: 1.0, lamps: 0.0 })),
            ("lamp haloes", "Stone Dungeon", Box::new(|s| s.weather.light_shafts = LightShafts { enabled: true, sun: 0.0, lamps: 1.0 })),
            ("heat shimmer", "Desert Canyon", Box::new(|s| s.weather.heat_shimmer = HeatShimmer { enabled: true, strength: 1.0, ..HeatShimmer::default() })),
            ("drops", "Dark Street", Box::new(|s| s.weather.lens = Lens { enabled: true, amount: 1.0, ..Lens::default() })),
            ("frost", "Ice Cave", Box::new(|s| s.weather.lens = Lens { enabled: true, kind: LensKind::Frost, ..Lens::default() })),
            ("cloud shadows", "Desert Ruins", Box::new(|s| s.sky.clouds.shadows = 1.0)),
            ("aurora", "Night Road", Box::new(|s| s.sky.aurora = Aurora { enabled: true, ..Aurora::default() })),
            ("rainbow", "Desert Ruins", Box::new(|s| s.sky.rainbow = Rainbow { enabled: true, ..Rainbow::default() })),
            ("petals", "Forest Path", Box::new(|s| s.particles.push(Particles { kind: ParticleKind::Petals, count: 1500, ..Particles::default() }))),
        ];
        let mut weak = Vec::new();
        for (name, p, edit) in cases {
            // Measured against the scene with no weather at all (some presets have some).
            let mut base = preset(p);
            base.weather = Weather::default();
            base.sky.clouds.shadows = 0.0;
            let mut s = base.clone();
            edit(&mut s);
            let t = s.motion.loop_seconds() * 0.3;
            let d = diff(&r.render(&base, 3.0, &opts(t)), &r.render(&s, 3.0, &opts(t)));
            // Shimmer moves the picture by about half a pixel at this tiny size.
            if d <= if name == "heat shimmer" { 0.03 } else { 0.15 } { weak.push(format!("{name} on {p}: {d:.3}")); }
        }
        assert!(weak.is_empty(), "effects that barely change the picture: {weak:?}");
    }

    #[test]
    fn rain_and_snow_stay_out_from_under_a_roof() {
        let mut r = WorldRenderer::default();
        let base = preset("Stone Dungeon");
        for kind in [PrecipKind::Rain, PrecipKind::Snow, PrecipKind::Hail] {
            let mut s = base.clone();
            s.weather.precipitation = Precipitation { enabled: true, kind, intensity: 1.0, haze: 1.0, ..Precipitation::default() };
            assert_eq!(r.render(&base, 2.0, &opts(1.0)).rgba, r.render(&s, 2.0, &opts(1.0)).rgba, "{kind:?} under a ceiling");
        }
    }

    #[test]
    fn rain_darkens_the_ground_and_snow_whitens_it() {
        let mut r = WorldRenderer::default();
        let mut base = preset("Forest Path");
        base.sky.clouds.enabled = false;
        base.weather = Weather::default();
        // The ground alone: the bottom third of the frame, with nothing falling.
        let ground = |img: &Image| { let n = img.rgba.len(); let g = &img.rgba[n * 2 / 3..]; g.chunks(4).map(|p| p[0] as f32 + p[1] as f32 + p[2] as f32).sum::<f32>() / g.len() as f32 };
        let o = RenderOptions { size: Some((90, 160)), time: Some(1.0), layers: Layers { particles: false, ..Layers::default() }, ..RenderOptions::default() };
        let dry = ground(&r.render(&base, 2.0, &o));
        let mut wet = base.clone();
        wet.weather.precipitation = Precipitation { enabled: true, haze: 0.0, ..Precipitation::default() };
        let mut snowy = base.clone();
        snowy.weather.precipitation = Precipitation { enabled: true, kind: PrecipKind::Snow, haze: 0.0, track: 0.0, ..Precipitation::default() };
        let (w, s) = (ground(&r.render(&wet, 2.0, &o)), ground(&r.render(&snowy, 2.0, &o)));
        assert!(w < dry * 0.9, "wet ground {w} vs dry {dry}");
        assert!(s > dry * 1.3, "snowy ground {s} vs dry {dry}");
    }

    #[test]
    fn wind_sways_the_trees_and_stillness_stays_still() {
        let mut r = WorldRenderer::default();
        let mut s = preset("Forest Path");
        s.sky.clouds.enabled = false;
        s.particles.clear();
        s.weather = Weather::default();
        let secs = s.motion.loop_seconds();
        let calm = diff(&r.render(&s, 3.0, &opts(0.0)), &r.render(&s, 3.0, &opts(secs * 0.25)));
        assert!(calm < 1e-6, "nothing moves in still air: {calm}");
        s.weather.wind = Wind { enabled: true, speed: 9.0, sway: 1.0, ..Wind::default() };
        let windy = diff(&r.render(&s, 3.0, &opts(0.0)), &r.render(&s, 3.0, &opts(secs * 0.25)));
        assert!(windy > 0.2, "trees should sway: {windy}");
        assert!(sway_factor(PropKind::Rock) == 0.0 && sway_factor(PropKind::Reeds) > sway_factor(PropKind::Tree));
    }
}
