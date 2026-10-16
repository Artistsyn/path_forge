//! Animals on the move (`scene.wildlife`): flocks of birds crossing the sky, bats flitting over the
//! path, fish leaping from the water. Each goes round a whole number of times a loop. They are
//! splats, so both renderers draw them alike.

use super::*;

/// One bird or bat: two wings from tip through elbow to the body at (x, y), `span` pixels across,
/// beating (`beat` -1 down .. 1 up).
fn wings(out: &mut Vec<Splat>, z: f32, x: f32, y: f32, span: f32, beat: f32, col: [f32; 3], alpha: f32) {
    let w = (span * 0.09).max(0.8);
    for side in [-1.0f32, 1.0] {
        let elbow = (x + side * 0.45 * span, y - 0.12 * span * beat);
        let tip = (x + side * span, y - 0.5 * span * beat + 0.12 * span);
        splat::streak(out, z, tip, elbow, col, alpha, w * 0.8);
        splat::streak(out, z, elbow, (x, y), col, alpha, w);
    }
    splat::dot(out, z, (x, y + 0.04 * span), (span * 0.08).max(0.6), col, alpha);
}

pub(super) fn draw(ctx: &Ctx, out: &mut Vec<Splat>) {
    let wl = &ctx.scene.wildlife;
    let v = &ctx.view;
    let loop_seconds = ctx.scene.motion.loop_seconds();
    let sky = &ctx.scene.sky;
    if wl.birds.enabled && wl.birds.flocks > 0 && sky.enabled && !sky.tunnel.enabled {
        let b = &wl.birds;
        let hy = v.horizon_px.max(2.0);
        let (ox, oy) = (v.left as f32 + v.sky_dx(), v.top as f32);
        let (fw, fhy) = (v.width as f32 - 2.0 * v.left as f32, (hy - v.top as f32).max(2.0));
        let crossings = (loop_seconds / b.crossing.max(1.0)).round().max(1.0);
        let flaps = cycles(b.flap.max(0.1), loop_seconds);
        let col = mul3(rgb_lin(b.color), add3(ctx.ambient, [0.15; 3]));
        for f in 0..b.flocks.min(12) as i64 {
            let h = |k: u32, i: i64| hf(b.seed.wrapping_mul(0x9E37) ^ k, f * 64 + i);
            let p = (crossings * ctx.tphase + h(0xB1, 0)).rem_euclid(1.0);
            let dir = if h(0xB2, 0) < 0.5 { 1.0 } else { -1.0 };
            let span = b.size.max(0.002) * fhy * (0.7 + 0.6 * h(0xB3, 0));
            let lead_x = ox + fw * (0.5 + dir * (p * 1.5 - 0.75));
            let lead_y = oy + fhy * (b.height + 0.25 * (h(0xB4, 0) - 0.5)).clamp(0.02, 0.95);
            let n = b.count.clamp(1, 40) as i64;
            for i in 0..n {
                // A loose vee: each a little behind and to one side of the one before.
                let rank = ((i + 1) / 2) as f32;
                let side = if i % 2 == 0 { 1.0 } else { -1.0 };
                let x = lead_x - dir * rank * span * 1.4 + (h(0xB5, i) - 0.5) * span;
                let y = lead_y + side * rank * span * 0.7 + (h(0xB6, i) - 0.5) * span * 0.8;
                let beat = (TAU * (flaps * ctx.tphase + h(0xB7, i))).sin();
                let bob = 0.15 * span * (TAU * (flaps * ctx.tphase + h(0xB7, i)) - 1.0).cos();
                wings(out, 1.0e4, x, y + bob, span * 0.5, beat, col, 0.9);
            }
        }
    }
    if wl.bats.enabled && wl.bats.count > 0 {
        let b = &wl.bats;
        let col = mul3(rgb_lin(b.color), add3(ctx.ambient, [0.1; 3]));
        let (c1, c2, c3, flap) = (cycles(0.35, loop_seconds), cycles(0.55, loop_seconds), cycles(0.25, loop_seconds), cycles(9.0, loop_seconds));
        for i in 0..b.count.min(60) as i64 {
            let h = |k: u32| hf(b.seed.wrapping_mul(0x9E37) ^ k, i);
            let t = ctx.tphase;
            let d = 5.0 + 22.0 * h(0xBA) + 3.0 * (TAU * (c3 * t + h(0xBB))).sin();
            let x = (h(0xBC) * 2.0 - 1.0) * (ctx.view.path_half_width(d) + 5.0) + 2.5 * (TAU * (c1 * t + h(0xBD))).sin() + 0.6 * (TAU * (3.0 * c2 * t + h(0xBE))).sin();
            let y = 3.0 + 4.0 * h(0xBF) + 1.0 * (TAU * (c2 * t + h(0xC0))).sin();
            let c = v.to_cam(x, y, d);
            if c[2] < NEAR { continue; }
            let Some([sx, sy]) = v.project(c) else { continue };
            let span = v.px_per_m(c[2]) * b.size.max(0.05) * 0.5;
            if span < 0.6 { continue; }
            let beat = (TAU * (flap * t + h(0xC1))).sin();
            wings(out, c[2], sx, sy, span, beat, ctx.apply_fog(col, c[2]), 0.95);
        }
    }
    if wl.fish.enabled && wl.fish.leaps > 0 {
        let fsh = &wl.fish;
        let sp = snap_to_loop(fsh.spacing.max(1.0), ctx.loop_len);
        let n = (ctx.loop_len / sp).round() as i64;
        let k_lo = ((ctx.scroll + NEAR) / sp).ceil() as i64;
        let k_hi = ((ctx.scroll + ctx.far.min(60.0)) / sp).floor() as i64;
        let base = rgb_lin(fsh.color);
        let (lo, hi) = (fsh.lateral[0].min(fsh.lateral[1]), fsh.lateral[0].max(fsh.lateral[1]));
        for k in k_lo..=k_hi {
            let key = k.rem_euclid(n);
            let h = |s: u32| hf(fsh.seed.wrapping_mul(0x9E37) ^ s, key);
            let p = (fsh.leaps as f32 * ctx.tphase + h(0xF1)).rem_euclid(1.0);
            // In the air for a little of its turn; rings on the water as it goes in and comes out.
            let jump = 0.1;
            let side = if h(0xF2) < 0.5 { -1.0 } else { 1.0 };
            let x0 = side * (lo + (hi - lo) * h(0xF3));
            let d0 = k as f32 * sp - ctx.scroll + (h(0xF4) - 0.5) * sp * 0.8;
            let len = fsh.size.max(0.05);
            let dir = if h(0xF5) < 0.5 { 1.0 } else { -1.0 };
            let ring = |out: &mut Vec<Splat>, x: f32, d: f32, age: f32| {
                let c = v.to_cam(x, 0.02, d);
                if c[2] < NEAR { return; }
                let Some([sx, sy]) = v.project(c) else { return };
                let ppm = v.px_per_m(c[2]);
                let r = ppm * len * (0.3 + 1.6 * age);
                let a = 0.5 * (1.0 - age);
                let lit = ctx.apply_fog(mul3([0.85, 0.9, 0.95], add3(ctx.ambient, [0.2; 3])), c[2]);
                for j in 0..14 {
                    let an = TAU * j as f32 / 14.0;
                    splat::dot(out, c[2] - 0.01, (sx + r * an.cos(), sy + 0.28 * r * an.sin()), (ppm * 0.012).max(0.5), lit, a);
                }
            };
            if p < jump {
                let u = p / jump;
                let along = (u - 0.5) * len * 2.5;
                let (x, y) = (x0 + dir * along, 0.55 * len * 2.0 * (std::f32::consts::PI * u).sin());
                let c = v.to_cam(x, y, d0);
                if c[2] < NEAR { continue; }
                let Some([sx, sy]) = v.project(c) else { continue };
                let ppm = v.px_per_m(c[2]);
                // Nose along the arc: up on the way out, down on the way in.
                let slope = (std::f32::consts::PI * u).cos() * 1.2;
                let (hx, hy) = (dir * 0.5 * len * ppm / (1.0 + slope * slope).sqrt(), -0.5 * len * ppm * slope / (1.0 + slope * slope).sqrt());
                let lit = ctx.apply_fog(mul3(base, add3(ctx.light_at(x, y, d0, None, 0, 1.0), [0.15; 3])), c[2]);
                splat::streak(out, c[2], (sx - hx, sy - hy), (sx + hx, sy + hy), lit, 0.95, (0.22 * len * ppm).max(1.0));
            }
            // Rings where it went in, then where it comes out.
            for (at, x) in [(p, x0 - dir * len * 1.25), ((p - jump).rem_euclid(1.0), x0 + dir * len * 1.25)] {
                let age = at / 0.25;
                if age < 1.0 { ring(out, x, d0, age); }
            }
        }
    }
}
