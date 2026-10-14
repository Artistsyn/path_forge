//! Clouds as the sky shows them (`sky.clouds` and `sky.cloud_layers`, every kind but `Soft`):
//! heaps of cumulus standing in rows on level bases, lit above and grey beneath, and sheets
//! (stratus, mackerel, cirrus, overcast) seen from below, receding towards the horizon. All are
//! made of noise that repeats across the sky, so a whole number of drifts brings every cloud back
//! where it began; at any other speed two copies half a loop apart form and dissolve in turn.
//! `sky.wgsl` (`cloud_px`) repeats `CloudK::px` from the same record.

use super::*;
use super::space::fbm_wrap;
use crate::scene::{CloudKind, Clouds};

/// Floats in a layer's record for the GPU (`CloudK::rec`).
pub(super) const REC: usize = 40;
/// How low towards the horizon a sheet's plane is followed: the sky is a dome, not a flat ceiling.
const E0: f32 = 0.3;
const E1: f32 = 1.3;
/// A step of the light through a heap of cloud, in its heights.
const STEP: f32 = 0.14;

/// One layer of cloud for one frame. Lengths are in sky heights (top of the sky to the horizon).
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct CloudK {
    /// Heaps in rows (cumulus), or a sheet seen from below.
    pub heaps: bool,
    /// The pattern repeats every `span` across the sky, in `nx` cells at its coarsest, `vs` cells
    /// to a sky height the other way.
    pub span: f32,
    pub nx: f32,
    pub vs: f32,
    /// Where the noise turns to cloud, and over how much of it the edge fades in.
    pub th: f32,
    pub soft: f32,
    pub opacity: f32,
    /// How dark the thick parts and the undersides are (0..1).
    pub shade: f32,
    pub lit: [f32; 3],
    pub dark: [f32; 3],
    /// How far it has drifted, in spans.
    pub scroll: f32,
    /// Sheets: how much of a finer ripple is mixed in, and the column everything recedes towards.
    pub detail: f32,
    pub uc: f32,
    /// Heaps: rows, the far row's base, the step up to the next, a full cloud's height, and how
    /// much the outline billows.
    pub rows: f32,
    pub base: f32,
    pub step: f32,
    pub height: f32,
    pub billow: f32,
    pub seed: f32,
    /// The sun that lights their thin edges: where (sky heights) and its colour.
    pub sun: f32,
    pub su: f32,
    pub sv: f32,
    pub sc: [f32; 3],
    /// Each row's size, and how many clouds' widths it is across the span.
    pub row_scale: [f32; 3],
    pub row_nx: [f32; 3],
}

impl CloudK {
    pub(super) fn rec(&self) -> [f32; REC] {
        let mut r = [0.0; REC];
        r[..8].copy_from_slice(&[self.heaps as u32 as f32, self.span, self.nx, self.vs, self.th, self.soft, self.opacity, self.shade]);
        r[8..11].copy_from_slice(&self.lit);
        r[11..14].copy_from_slice(&self.dark);
        r[14..24].copy_from_slice(&[self.scroll, 0.0, self.detail, self.uc, self.rows, self.base, self.step, self.height, self.billow, self.seed]);
        r[24..27].copy_from_slice(&[self.sun, self.su, self.sv]);
        r[27..30].copy_from_slice(&self.sc);
        r[30..33].copy_from_slice(&self.row_scale);
        r[33..36].copy_from_slice(&self.row_nx);
        r
    }

    /// The sun through the thin parts of a cloud near it.
    fn lining(&self, u: f32, v: f32, thin: f32) -> [f32; 3] {
        if self.sun == 0.0 { return [0.0; 3]; }
        let near = (-((u - self.su).powi(2) + (v - self.sv).powi(2)) / 0.09).exp();
        scale3(self.sc, near * (0.25 + 0.9 * thin))
    }

    /// Which way the light comes from on screen at (u, v): from the sun, or from up and to the left.
    fn light_dir(&self, u: f32, v: f32) -> (f32, f32) {
        if self.sun == 0.0 { return (-0.45, -0.89); }
        let (dx, dy) = (self.su - u, self.sv - v);
        let l = (dx * dx + dy * dy).sqrt().max(1e-3);
        (dx / l, dy / l)
    }

    /// The layer over the sky at (u, v) (sky heights from the frame's left and top): its colour
    /// times its cover, and its cover.
    pub(super) fn px(&self, u: f32, v: f32) -> ([f32; 3], f32) {
        if self.heaps { self.heaps_px(u, v) } else { self.sheet_px(u, v) }
    }

    fn sheet_px(&self, u: f32, v: f32) -> ([f32; 3], f32) {
        // A level sheet overhead: further off, and so smaller and flatter, the nearer the horizon.
        let z = 1.0 / (E1 - v).max(E0);
        let uu = (u - self.uc) * z / self.span - self.scroll;
        let uu = uu - uu.floor();
        let coarse = fbm_wrap(uu, z + self.seed, self.nx as i32, self.vs);
        let mut n = coarse;
        if self.detail > 0.0 { n = n * (1.0 - self.detail) + self.detail * fbm_wrap(uu, z + self.seed + 3.1, self.nx as i32 * 4, self.vs * 4.0); }
        let d = (n - self.th) / self.soft;
        let a = smoothstep(0.0, 1.0, d) * self.opacity * (1.0 - smoothstep(0.86, 1.0, v));
        if a <= 0.0 { return ([0.0; 3], 0.0); }
        // Seen from below, the thick parts are dark; and each fold is lit on the side it turns to
        // the sun (the coarse noise again, half a cell that way).
        let thick = smoothstep(0.5, 3.0, d);
        let (lx, ly) = self.light_dir(u, v);
        let lift = (0.5 + 3.0 * (coarse - fbm_wrap(uu + lx * 0.4 / self.nx, z + self.seed + ly * 0.4 / self.vs, self.nx as i32, self.vs))).clamp(0.0, 1.0);
        let shadow = (0.7 * thick + 0.5 * (1.0 - lift)).min(1.0);
        let c = add3(mix3(self.lit, self.dark, (1.5 * self.shade).min(1.0) * shadow), self.lining(u, v, 1.0 - thick));
        (scale3(c, a), a)
    }

    fn heaps_px(&self, u: f32, v: f32) -> ([f32; 3], f32) {
        let rows = (self.rows as usize).clamp(1, 3);
        let (mut col, mut alpha) = ([0.0f32; 3], 0.0f32);
        let (lx, ly) = self.light_dir(u, v);
        // Far rows first, low over the horizon; the nearest highest, largest and last.
        for r in 0..rows {
            let s = self.row_scale[r];
            let base = self.base - r as f32 * self.step;
            let h = (base - v) / (self.height * s);
            if !(-0.1..=1.3).contains(&h) { continue; }
            let near = r + 1 == rows && rows > 1;
            let uu = u / self.span - self.scroll * if near { 2.0 } else { 1.0 } + 0.37 * r as f32 + self.seed;
            let uu = uu - uu.floor();
            let nx = self.row_nx[r] as i32;
            // Where the clouds stand along the row, and how tall each has grown.
            let mass = fbm_wrap(uu, 0.0, nx, 0.0);
            let env = smoothstep(self.th, self.th + 0.2, mass);
            if env <= 0.0 { continue; }
            let (nb, arg) = (nx * 4, (base - v) / s + 11.3 * (r + 1) as f32);
            // How deep inside the cloud a point is: under its dome (steep-sided, level-based),
            // pushed out and in by round swellings with creases between them (the noise folded
            // about its middle).
            let dome = |m: f32| smoothstep(self.th, self.th + 0.15, m) * (0.55 + 0.45 * smoothstep(self.th + 0.1, self.th + 0.5, m));
            let swell = |uu: f32, arg: f32| 2.2 * (fbm_wrap(uu, arg, nb, self.vs) - 0.47).abs();
            let depth = |h: f32, n: f32, top: f32| {
                let d = top - h + self.billow * (n - 0.25) * (0.3 + 0.7 * smoothstep(0.0, 0.2, h));
                if h < 0.0 { d.min(env * (h + 0.1) / 0.1) } else { d }
            };
            let top = dome(mass);
            // Its edge is frayed into wisps by a finer noise.
            let d = depth(h, swell(uu, arg), top) + 0.22 * env * (fbm_wrap(uu, arg + 5.3, nb * 4, self.vs * 4.0) - 0.47);
            let a = smoothstep(0.0, self.soft, d) * self.opacity * (0.55 + 0.45 * (r + 1) as f32 / rows as f32);
            if a <= 0.0 { continue; }
            // The light that reaches here has come through the cloud from the sun's side: three
            // steps that way, adding up how much cloud is in between. So the sunward swellings
            // shine, and the far side, the hollows and the base are in the cloud's own shadow.
            let du = lx * STEP * self.height * s / self.span;
            let top3 = dome(fbm_wrap(uu + 3.0 * du, 0.0, nx, 0.0));
            let mut between = 0.0;
            for i in 1..=3 {
                let f = i as f32;
                let di = depth(h - ly * STEP * f, swell(uu + du * f, arg - ly * STEP * f * self.height), top + (top3 - top) * f / 3.0);
                between += (di / 0.3).clamp(0.0, 1.0);
            }
            let light = 0.1 + 0.9 * (-1.1 * between).exp();
            let c = add3(mix3(self.lit, self.dark, (1.5 * self.shade).min(1.0) * (1.0 - light)), self.lining(u, v, 1.0 - smoothstep(0.0, 0.6, d)));
            col = add3(scale3(col, 1.0 - a), scale3(c, a));
            alpha += a * (1.0 - alpha);
        }
        (col, alpha)
    }
}

/// A layer's kind as drawn: `Soft` is the main layer's alone (`render::cloud_list`), and reads
/// as cumulus in a further layer.
fn drawn(kind: CloudKind, main: bool) -> Option<CloudKind> {
    match kind { CloudKind::Soft if main => None, CloudKind::Soft => Some(CloudKind::Cumulus), k => Some(k) }
}

/// Every layer of noise cloud this frame, in drawing order.
pub(super) fn layers(ctx: &Ctx) -> Vec<CloudK> {
    let sky = &ctx.scene.sky;
    let v = &ctx.view;
    let hy = v.horizon_px.max(2.0);
    let (ox, oy) = (v.left as f32, v.top as f32);
    let (fw, fhy) = (v.width as f32 - 2.0 * ox, (hy - oy).max(2.0));
    let lit = add3(scale3(ctx.ambient, 0.6), ctx.sky_lights.iter().fold([0.0; 3], |a, s| add3(a, scale3(s.color, 0.4))));
    let sun = sky.sun.enabled.then(|| (sky.sun.pos[0] * fw / fhy, sky.sun.pos[1].clamp(0.0, 1.0), scale3(rgb_lin(sky.sun.color), 0.6)));
    let span = 1.6 * fw / fhy;
    let mut out = Vec::new();
    let all = std::iter::once((&sky.clouds, true)).chain(sky.cloud_layers.iter().map(|c| (c, false)));
    for (li, (cl, main)) in all.take(4).enumerate() {
        let Some(kind) = drawn(cl.kind, main) else { continue };
        if !cl.enabled || cl.opacity <= 0.0 { continue; }
        out.extend(layer(ctx, cl, kind, li, span, 0.5 * fw / fhy, lit, sun));
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn layer(ctx: &Ctx, cl: &Clouds, kind: CloudKind, li: usize, span: f32, uc: f32, lit: [f32; 3], sun: Option<(f32, f32, [f32; 3])>) -> Vec<CloudK> {
    let cover = cl.cover.clamp(0.0, 1.0);
    let scale = cl.scale.clamp(0.2, 4.0);
    let ragged = cl.variation.clamp(0.0, 1.0);
    // Cloud that rain or snow is falling from is heavier and darker, whatever its kind.
    let p = &ctx.scene.weather.precipitation;
    let fall = if p.enabled && ctx.wx.open { p.intensity.clamp(0.0, 1.0) } else { 0.0 };
    let col = scale3(mul3(rgb_lin(cl.tint), add3(lit, [0.35; 3])), 1.0 - 0.35 * fall);
    let grey = |k: f32| mul3(col, scale3([0.42, 0.45, 0.52], k * (1.0 - 0.3 * fall)));
    let cells = |per: f32| (span * per / scale).round().max(2.0);
    let base = CloudK {
        span, uc, lit: col, dark: grey(1.0), shade: cl.shade.clamp(0.0, 1.0), seed: hf(cl.seed ^ 0x91, li as i64),
        sun: sun.is_some() as u32 as f32, su: sun.map_or(0.0, |s| s.0), sv: sun.map_or(0.0, |s| s.1), sc: sun.map_or([0.0; 3], |s| s.2),
        opacity: (cl.opacity.max(0.0) * 2.0).min(1.0),
        ..CloudK::default()
    };
    // How much of the noise is cloud, for sheets: none at cover 0, all of it at 1.
    let sheet_th = |spread: f32| 0.47 + spread * (1.0 - 2.0 * cover);
    let sheet = |per: f32, along: f32, th: f32, soft: f32, detail: f32| CloudK { nx: cells(per), vs: along / scale, th, soft, detail, ..base };
    let heaps = |tall: bool| {
        let rows = if tall { 2usize } else { 3 };
        let per = if tall { 1.1 } else { 1.6 };
        let (bottom, step, height) = if tall { (0.93, 0.16, 0.62) } else { (0.9, 0.21, 0.36) };
        let mut k = CloudK {
            heaps: true, rows: rows as f32, vs: 4.0 * per, base: bottom, step, height,
            th: 0.47 + 0.36 * (0.5 - cover) - 0.16, soft: 0.06 + 0.08 * ragged, billow: 0.4 + 0.4 * ragged, lit: scale3(col, 1.15), dark: grey(0.62), ..base
        };
        for r in 0..rows {
            let s = scale * (0.42 + 0.58 * ((r + 1) as f32 / rows as f32).powf(1.3));
            (k.row_scale[r], k.row_nx[r]) = (s, (span * per / s).round().max(3.0));
        }
        k
    };
    let thin = |k: f32| (cl.opacity.max(0.0) * k).min(1.0);
    let made: Vec<CloudK> = match kind {
        CloudKind::Cumulus | CloudKind::Soft => vec![heaps(false)],
        CloudKind::Towering => vec![CloudK { dark: grey(0.45), ..heaps(true) }],
        CloudKind::Stratus => vec![CloudK { opacity: thin(1.5), ..sheet(0.9, 2.6, sheet_th(0.22), 0.08 + 0.12 * ragged, 0.25) }],
        CloudKind::Mackerel => vec![CloudK { opacity: thin(1.5), ..sheet(1.3, 1.3, sheet_th(0.2), 0.05 + 0.08 * ragged, 0.5) }],
        CloudKind::Cirrus => vec![CloudK { opacity: thin(0.9), shade: 0.0, ..sheet(2.6, 0.75, sheet_th(0.2) + 0.06, 0.1 + 0.1 * ragged, 0.4) }],
        CloudKind::Overcast => vec![CloudK { dark: grey(0.75), ..sheet(1.2, 1.6, sheet_th(0.3) - 0.2, 0.3, 0.3) }],
        // A low dark deck, heavy-bellied and ragged.
        CloudKind::Rain => vec![CloudK { lit: grey(0.62), dark: grey(0.24), shade: base.shade.max(0.7), ..sheet(1.1, 1.4, sheet_th(0.3) - 0.3, 0.22, 0.4) }],
        // An even, pale, heavy sky with hardly a feature in it.
        CloudKind::Snow => vec![CloudK { lit: scale3(col, 0.6), dark: grey(1.0), ..sheet(0.8, 1.0, sheet_th(0.3) - 0.32, 0.4, 0.15) }],
        // A black deck, and ragged scud torn off it driving along underneath.
        CloudKind::Storm => vec![
            CloudK { lit: grey(0.42), dark: grey(0.13), shade: base.shade.max(0.8), ..sheet(1.0, 1.3, sheet_th(0.3) - 0.3, 0.22, 0.4) },
            CloudK { lit: grey(0.3), dark: grey(0.1), shade: 1.0, seed: base.seed + 0.41, ..sheet(2.2, 2.6, sheet_th(0.2) + 0.03, 0.07, 0.5) },
        ],
    };
    // A whole number of drifts a loop brings the pattern round to where it began. At any other
    // speed, two copies half a loop apart each form, drift and dissolve, never both gone.
    let drift = cl.drift.max(0.0);
    if (drift - drift.round()).abs() < 1e-3 {
        return made.into_iter().map(|k| CloudK { scroll: drift.round() * ctx.tphase, ..k }).collect();
    }
    let ph = (ctx.tphase + hf(cl.seed ^ 0x77, li as i64)).rem_euclid(1.0);
    made.into_iter().flat_map(|k| [(ph, 0.0), ((ph + 0.5).rem_euclid(1.0), 0.37)].into_iter().filter_map(move |(p, off)| {
        let w = 1.0 - (2.0 * p - 1.0).abs();
        (w > 0.0).then(|| CloudK { scroll: drift * (p - 0.5) + off, opacity: k.opacity * w, ..k })
    })).collect()
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{RenderOptions, WorldRenderer};

    fn sky_mean(kind: CloudKind, cover: f32, t: f32) -> f32 {
        let mut s = crate::scene::presets::ALL.iter().find(|(m, _)| *m == "Desert Ruins").unwrap().1();
        s.props.clear();
        s.set_pieces.clear();
        s.sky.cloud_layers.clear();
        s.sky.clouds = Clouds { enabled: true, kind, cover, drift: 0.4, ..Clouds::default() };
        let img = WorldRenderer::default().render(&s, 3.0, &RenderOptions { size: Some((90, 160)), time: Some(t), ..RenderOptions::default() });
        // The top fifth of the frame is sky.
        img.rgba[..90 * 32 * 4].chunks(4).map(|p| p[0] as f32 + p[1] as f32 + p[2] as f32).sum::<f32>() / (90.0 * 32.0 * 3.0)
    }

    #[test]
    fn every_kind_shows_and_weather_clouds_are_dark() {
        let clear = sky_mean(CloudKind::Cumulus, 0.0, 0.0);
        for kind in [CloudKind::Cumulus, CloudKind::Towering, CloudKind::Stratus, CloudKind::Mackerel, CloudKind::Cirrus, CloudKind::Overcast, CloudKind::Rain, CloudKind::Snow, CloudKind::Storm] {
            let with = sky_mean(kind, 0.8, 0.0);
            assert!((with - clear).abs() > 1.0, "{kind:?} should show: {with} against {clear}");
        }
        let (rain, storm, snow) = (sky_mean(CloudKind::Rain, 0.7, 0.0), sky_mean(CloudKind::Storm, 0.7, 0.0), sky_mean(CloudKind::Snow, 0.7, 0.0));
        assert!(storm < rain && rain < snow && rain < clear - 20.0, "storm {storm} rain {rain} snow {snow} clear {clear}");
    }

    #[test]
    fn a_fractional_drift_still_closes_the_loop() {
        let secs = crate::scene::presets::ALL.iter().find(|(m, _)| *m == "Desert Ruins").unwrap().1().motion.loop_seconds();
        for kind in [CloudKind::Cumulus, CloudKind::Mackerel] {
            let (a, b, mid) = (sky_mean(kind, 0.6, 0.0), sky_mean(kind, 0.6, secs), sky_mean(kind, 0.6, secs * 0.37));
            assert!((a - b).abs() < 0.01, "{kind:?}: {a} at the start, {b} a loop later");
            assert!(mid > 0.0);
        }
    }
}
