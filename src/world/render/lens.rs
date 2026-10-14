//! Rain on the camera's glass (`weather.lens`, kind `Drops`), as a lens or a windscreen shows it:
//! beads gathering all over the glass, heavy drops that land, hang and then run down in fits and
//! starts, wiping the beads in their way and leaving a wet trail of small ones; every drop a tiny
//! lens holding the scene upside down, and the wet glass between them a little out of focus.
//! `post.wgsl` (`post_lens`) repeats `rain_px` step for step, from what `plan` works out.

use super::*;
use super::weather::bilinear;

/// A heavy drop for one frame: where it landed (x, y), how far down it is now, its radius, its
/// opacity, how far and at what phase its track wanders, and its number (for what it leaves).
pub(super) const RUNNER: usize = 8;

#[derive(Clone, Debug, Default)]
pub(super) struct LensRain {
    /// The beads lie in two grids, one to a cell: the cells' sizes, pixels.
    pub cell: [f32; 2],
    /// The beads' clock (each lives one unit, at its own phase), and how many lives make a loop.
    pub tt: f32,
    pub cyc: u32,
    /// The share of cells that hold a bead.
    pub fill: f32,
    /// How far out of focus the wet glass is (0..1), and the blur's reach in whole pixels along
    /// the axes and the diagonals.
    pub haze: f32,
    pub blur: [i32; 2],
    /// How far across the scene a drop's edge looks, pixels.
    pub reach: f32,
    pub weight: f32,
    pub seed: u32,
    pub runners: Vec<[f32; RUNNER]>,
}

/// The glass is cut into this many upright strips, and the GPU looks only at the heavy drops
/// whose tracks cross a pixel's strip.
pub(super) const STRIPS: usize = 64;

impl LensRain {
    /// For each strip, the heavy drops that may show in it, one bit each (as floats, eight to a
    /// record, to follow the drops in the GPU's list).
    pub(super) fn strips(&self, w: usize) -> Vec<[f32; RUNNER]> {
        let mut m = [0u32; STRIPS];
        let sw = w as f32 / STRIPS as f32;
        for (i, r) in self.runners.iter().enumerate().take(24) {
            let ext = r[3] * 1.5 + r[5] + 2.0;
            let (a, b) = (((r[0] - ext) / sw).floor().max(0.0) as usize, (((r[0] + ext) / sw).floor().max(0.0) as usize).min(STRIPS - 1));
            for s in a..=b { m[s] |= 1 << i; }
        }
        m.chunks(RUNNER).map(|c| std::array::from_fn(|i| c[i] as f32)).collect()
    }
}

/// The rain on a w x h glass now, at `weight` 0..1.
pub(super) fn plan(ctx: &Ctx, w: usize, h: usize, weight: f32) -> LensRain {
    let l = &ctx.scene.weather.lens;
    let ls = ctx.scene.motion.loop_seconds();
    let amount = l.amount.clamp(0.0, 1.0);
    let size = l.size.clamp(0.3, 3.0);
    let (wf, hh) = (w as f32, h as f32);
    let cd = cycles(0.16, ls);
    let n = (amount * 14.0).round().max(1.0) as i64;
    let mut runners = Vec::new();
    for j in 0..n {
        let tt = hf(l.seed ^ 0x41, j) + cd * ctx.tphase;
        let a = tt - tt.floor();
        let key = j * 1031 + (tt.floor() as i64).rem_euclid(cd as i64);
        let alpha = smoothstep(0.0, 0.03, a) * (1.0 - smoothstep(0.85, 1.0, a)) * weight;
        if alpha <= 0.0 { continue; }
        // It lands spread wide, and draws itself up.
        let r = (0.012 + 0.016 * hf(l.seed ^ 0x42, key)) * wf * size * (1.0 + 0.5 * (1.0 - smoothstep(0.0, 0.05, a)));
        let x0 = hf(l.seed ^ 0x43, key) * wf;
        let y0 = hf(l.seed ^ 0x44, key) * hh * 0.8;
        let mut yn = y0;
        // Some grow heavy and run down the glass, stopping and starting.
        if hf(l.seed ^ 0x45, key) < l.trails.clamp(0.0, 1.0) {
            let s = smoothstep(0.2, 0.9, a);
            yn += (s + 0.035 * (TAU * 4.0 * s).sin()) * (0.3 + 0.5 * hf(l.seed ^ 0x46, key)) * hh;
        }
        runners.push([x0, y0, yn, r, alpha, r * (0.15 + 0.45 * hf(l.seed ^ 0x47, key)), TAU * hf(l.seed ^ 0x48, key), key as f32]);
    }
    let cyc = cycles(0.06, ls);
    LensRain {
        cell: [wf / 22.0 * size, wf / 48.0 * size],
        tt: cyc * ctx.tphase,
        cyc: cyc as u32,
        fill: 0.15 + 0.6 * amount,
        haze: l.blur.clamp(0.0, 1.0) * weight,
        blur: { let r = ((1.5 + 4.0 * l.blur.clamp(0.0, 1.0)) * wf / 480.0).round().max(1.0); [r as i32, (0.7 * r).round().max(1.0) as i32] },
        reach: 0.18 * hh,
        weight,
        seed: l.seed,
        runners,
    }
}

/// The drop that covers a pixel most: how much, and where in the scene it looks.
#[derive(Clone, Copy, Default)]
struct Hit { cov: f32, ox: f32, oy: f32, rim: f32 }

/// A drop of radius `r` pixels seen at (dx, dy) radii from its middle: a lens, looking straight
/// through at its middle and `reach` pixels the other way at its edge, so it holds the scene
/// round it upside down.
fn bead(dx: f32, dy: f32, r: f32, alpha: f32, reach: f32, best: &mut Hit) {
    let q = dx * dx + dy * dy;
    if q >= 1.0 { return; }
    let d = q.sqrt();
    let e = (1.5 / r).clamp(0.12, 0.9);
    let cov = alpha * (1.0 - smoothstep(1.0 - e, 1.0, d));
    if cov > best.cov {
        let k = (0.35 + 0.65 * q) * reach;
        *best = Hit { cov, ox: -dx * k, oy: -dy * k, rim: smoothstep(0.6, 1.0, d) };
    }
}

/// The pixel (x, y) of `src` seen through the rain on the glass.
pub(super) fn rain_px(k: &LensRain, src: &[[f32; 3]], w: usize, h: usize, x: usize, y: usize) -> [f32; 3] {
    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
    let hh = h as f32;
    let wk = TAU / (0.22 * hh);
    let mut best = Hit::default();
    let mut clear = 0.0f32;
    for run in &k.runners {
        let [x0, y0, yn, rad, alpha, wa, wp, id] = *run;
        if (px - x0).abs() > rad * 1.5 + wa || py < y0 - rad * 1.5 || py > yn + rad { continue; }
        let path = |yy: f32| x0 + wa * (wk * yy + wp).sin();
        // The drop itself, drawn out a little above as it hangs.
        let dy = py - yn;
        bead((px - path(yn)) / rad, dy / if dy < 0.0 { 1.35 * rad } else { 0.95 * rad }, rad, alpha, k.reach, &mut best);
        if py < yn && py > y0 {
            // Its trail: wet glass, narrowing and drying further behind it.
            let behind = yn - py;
            let fade = 1.0 - smoothstep(0.0, 0.45 * hh, behind);
            let tw = rad * (0.7 - 0.3 * smoothstep(0.0, 10.0 * rad, behind));
            let cx = (px - path(py)) / tw;
            let wet = (1.0 - smoothstep(0.5, 1.0, cx.abs())) * fade * alpha;
            clear = clear.max(wet);
            // The trail is a thread of water: it bends what is behind it sideways.
            if 0.85 * wet > best.cov { best = Hit { cov: 0.85 * wet, ox: -cx * 0.35 * k.reach, oy: 0.0, rim: 0.6 * smoothstep(0.4, 1.0, cx.abs()) }; }
            // And the small drops it leaves along it.
            let s = 2.4 * rad;
            let bi = (py / s).floor();
            let key = (id as i32).wrapping_mul(7919).wrapping_add(bi as i32) as i64;
            let br = rad * (0.16 + 0.2 * hf(k.seed ^ 0x4A, key)) * fade;
            let by = (bi + 0.3 + 0.4 * hf(k.seed ^ 0x4B, key)) * s;
            if br > 0.4 && by < yn - rad && by > y0 {
                let bx = path(by) + (hf(k.seed ^ 0x4C, key) - 0.5) * tw;
                bead((px - bx) / br, (py - by) / (br * 1.15), br, alpha, k.reach * 0.6, &mut best);
            }
        }
    }
    for layer in 0..2usize {
        let cell = k.cell[layer];
        let (ci, cj) = ((px / cell).floor() as i32, (py / cell).floor() as i32);
        let key = ci.wrapping_mul(7919).wrapping_add(cj.wrapping_mul(104_729)).wrapping_add(layer as i32 * 15_485_863);
        // Each bead gathers, sits and dries at its own time, and the next comes somewhere new.
        let tt = hf(k.seed ^ 0x51, key as i64) + k.tt;
        let a = tt - tt.floor();
        let key = key.wrapping_mul(31).wrapping_add((tt.floor() as i32).rem_euclid(k.cyc.max(1) as i32)) as i64;
        if hf(k.seed ^ 0x52, key) >= k.fill * if layer == 0 { 0.5 } else { 1.0 } { continue; }
        let m = 0.14 + 0.26 * hf(k.seed ^ 0x53, key);
        let rb = cell * m * (0.6 + 0.4 * smoothstep(0.0, 0.5, a));
        let cx = (ci as f32 + m + (1.0 - 2.0 * m) * hf(k.seed ^ 0x54, key)) * cell;
        let cy = (cj as f32 + m + (1.0 - 2.0 * m) * hf(k.seed ^ 0x55, key)) * cell;
        // A running drop takes the beads in its way with it.
        let alpha = smoothstep(0.0, 0.08, a) * (1.0 - smoothstep(0.75, 1.0, a)) * (1.0 - clear) * k.weight;
        bead((px - cx) / rb, (py - cy) / (rb * (0.85 + 0.15 * hf(k.seed ^ 0x56, key))), rb, alpha, k.reach * (0.35 + 1.6 * m), &mut best);
    }
    let (fx, fy) = (x as f32, y as f32);
    let mut c = src[y * w + x];
    let hz = k.haze * (1.0 - clear);
    if hz > 0.0 {
        let [r, d] = k.blur;
        let tap = |dx: i32, dy: i32| src[(y as i32 + dy).clamp(0, h as i32 - 1) as usize * w + (x as i32 + dx).clamp(0, w as i32 - 1) as usize];
        let ring = add3(add3(tap(-r, 0), tap(r, 0)), add3(tap(0, -r), tap(0, r)));
        let diag = add3(add3(tap(-d, -d), tap(d, -d)), add3(tap(-d, d), tap(d, d)));
        c = mix3(c, scale3(add3(ring, diag), 0.125), hz);
    }
    if best.cov > 0.0 {
        // Bright where it gathers the light, a fine dark line round its edge.
        let refr = bilinear(src, w, h, fx + best.ox, fy + best.oy);
        c = mix3(c, scale3(refr, 1.08 * (1.0 - 0.35 * best.rim)), best.cov);
    }
    c
}
