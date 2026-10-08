//! The path as a waterway (`path.waterway`): a current carrying the water's pattern towards the
//! camera, foam lapping at the banks or walls, the bank wet where the water reaches it, and things
//! floating on the surface (foam, leaves, lily pads) drifting with the current. `shade.wgsl`
//! repeats `water_cover` and `wetness` step for step, from the numbers `water_k` works out.

use super::*;
use crate::scene::Floating;

/// A waterway for one frame, as both renderers draw it.
#[derive(Clone, Copy, Debug)]
pub(super) struct WaterK {
    /// How far the current has carried the water so far this loop, metres.
    pub shift: f32,
    pub foam: f32,
    pub foam_col: [f32; 3],
    pub foam_w: f32,
    /// The lapping's phase now, and its wave number along the bank (whole waves per loop).
    pub lap_ph: f32,
    pub lap_k: f32,
    /// Wave numbers of the foam's breakup along the bank (whole waves per loop).
    pub k1: f32,
    pub k2: f32,
    pub wet: f32,
    /// 0 nothing afloat, 1 foam, 2 leaves, 3 lily pads.
    pub kind: u32,
    pub dens: f32,
    pub fcol: [f32; 3],
    pub fsize: f32,
    /// The floating things repeat every `period` metres along the water, which divides both the
    /// loop and the current's travel in a loop, so the loop closes; in cells `cs` metres square.
    pub period: f32,
    pub cs: f32,
    pub seed: u32,
}

pub(super) fn water_k(scene: &Scene, loop_len: f32, path_tile: f32, tphase: f32) -> Option<WaterK> {
    let ww = &scene.path.waterway;
    if !ww.enabled || !scene.path.surface { return None; }
    let n_tiles = (loop_len / path_tile).round().max(1.0) as i64;
    let gcd = |mut a: i64, mut b: i64| { while b != 0 { let t = a % b; a = b; b = t; } a.abs() };
    let g = if ww.flow == 0 { n_tiles } else { gcd(n_tiles, ww.flow as i64) };
    let period = g as f32 * path_tile;
    let cs0 = (ww.float_size * 1.8).max(0.05);
    let cs = period / (period / cs0).round().max(1.0);
    let whole = |len: f32| TAU * (loop_len / len).round().max(1.0) / loop_len;
    Some(WaterK {
        shift: ww.flow as f32 * path_tile * tphase,
        foam: ww.foam.clamp(0.0, 1.0),
        foam_col: rgb_lin(ww.foam_color),
        foam_w: ww.foam_width.max(0.01),
        lap_ph: TAU * ww.lap as f32 * tphase,
        lap_k: whole(5.0),
        k1: whole(2.3),
        k2: whole(1.1),
        wet: ww.wet.max(0.0),
        kind: match ww.floating { Floating::None => 0, Floating::Foam => 1, Floating::Leaves => 2, Floating::LilyPads => 3 },
        dens: ww.float_density.clamp(0.0, 1.0),
        fcol: rgb_lin(ww.float_color),
        fsize: ww.float_size.max(0.02),
        period,
        cs,
        seed: ww.seed,
    })
}

/// Where the water meets the land at depth d: the bank, or with no verge the walls.
pub(super) fn water_edge(ctx: &Ctx, d: f32) -> f32 {
    if !ctx.scene.verge.enabled && ctx.scene.walls.enabled { ctx.wall_x(d) } else { ctx.path_edge(d) }
}

/// Foam and floating things on the water at (x, w) (w = scroll + d), `dist` metres in from the
/// bank, `px` the width of a pixel there in metres: the colour laid over the water, and how much.
pub(super) fn water_cover(k: &WaterK, x: f32, w: f32, dist: f32, px: f32) -> ([f32; 3], f32) {
    let mut col = [0.0f32; 3];
    let mut a = 0.0f32;
    if k.foam > 0.0 {
        let lap = k.foam_w * (0.6 + 0.4 * (k.lap_ph + w * k.lap_k).sin());
        let n = 0.5 + 0.25 * (w * k.k1 + 2.1 * x + 1.3).sin() + 0.25 * (w * k.k2 - 3.7 * x).sin();
        let band = 1.0 - smoothstep(0.0, lap, dist);
        let line = 1.0 - smoothstep(0.0, px.max(0.03), dist);
        a = (k.foam * (band * (0.35 + 0.65 * n) + 0.5 * line)).clamp(0.0, 1.0);
        col = k.foam_col;
    }
    if k.kind != 0 && k.dens > 0.0 {
        let wp = (w + k.shift).rem_euclid(k.period);
        let (ci, cj) = ((x / k.cs).floor() as i32, (wp / k.cs).floor() as i32);
        let key = ci.wrapping_mul(7919).wrapping_add(cj.wrapping_mul(104_729)) as i64;
        let h = |s: u32| hf(k.seed ^ s, key);
        if h(0xF10) < k.dens {
            // At most one to a cell, kept clear of its edges.
            let r = 0.5 * k.fsize * (0.7 + 0.5 * h(0xF11));
            let m = (r / k.cs).min(0.45);
            let cx = (ci as f32 + m + (1.0 - 2.0 * m) * h(0xF12)) * k.cs;
            let cz = (cj as f32 + m + (1.0 - 2.0 * m) * h(0xF13)) * k.cs;
            let ang = TAU * h(0xF14);
            let (dx, dz) = (x - cx, wp - cz);
            let (lx, lz) = (dx * ang.cos() + dz * ang.sin(), -dx * ang.sin() + dz * ang.cos());
            let q = (lx * lx + lz * lz).sqrt();
            let th = lz.atan2(lx);
            let tone = 0.8 + 0.4 * h(0xF15);
            let (sd, c) = match k.kind {
                1 => (q - r * (1.0 + 0.25 * (3.0 * th + TAU * h(0xF16)).sin()), k.fcol),
                2 => {
                    let rz = 0.42 * r;
                    let e = ((lx / r) * (lx / r) + (lz / rz) * (lz / rz)).sqrt();
                    let vein = if lz.abs() < 0.07 * rz { 0.7 } else { 1.0 };
                    ((e - 1.0) * rz, scale3(k.fcol, tone * vein))
                }
                _ => {
                    // A disc with a wedge cut to its middle; a few in flower.
                    let sd = (q - r).max((0.3 - th.abs()) * q);
                    let c = if h(0xF17) < 0.15 && q < 0.22 * r { [1.0, 0.45, 0.65] } else { scale3(k.fcol, tone * (0.75 + 0.25 * smoothstep(r, 0.6 * r, q))) };
                    (sd, c)
                }
            };
            let cover = (0.5 - sd / px.max(1e-4)).clamp(0.0, 1.0) * smoothstep(0.0, 0.15, dist);
            if cover > 0.0 {
                let na = a + cover * (1.0 - a);
                col = scale3(add3(scale3(col, a * (1.0 - cover)), scale3(c, cover)), 1.0 / na);
                a = na;
            }
        }
    }
    (col, a)
}

/// How wet the bank (or wall) is `out` metres from the water: 1 at the water, 0 past `wet`.
pub(super) fn wetness(k: &WaterK, out: f32) -> f32 {
    if k.wet <= 0.0 { 0.0 } else { 1.0 - smoothstep(0.0, k.wet, out.max(0.0)) }
}
