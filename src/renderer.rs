/// Software path renderer — Rust port of the JSX canvas render loop plus
/// PathForge 2.0 additions (sky gradient, enhanced depth fog).
use crate::settings::{PathForgeSettings, AtmoType, PropType, TILE};
use crate::tiles::{gen_floor_tile, gen_wall_tile, seeded_rng, TileKey};

// ── Renderer state ─────────────────────────────────────────────────────────
pub struct PathRenderer {
    floor_key:  Option<TileKey>,
    wall_key:   Option<TileKey>,
    floor_tile: Vec<u8>,
    wall_tile:  Vec<u8>,
}

impl Default for PathRenderer {
    fn default() -> Self {
        Self {
            floor_key:  None,
            wall_key:   None,
            floor_tile: vec![128u8; TILE * TILE * 3],
            wall_tile:  vec![80u8;  TILE * TILE * 3],
        }
    }
}

impl PathRenderer {
    /// Convenience: render into a freshly-allocated buffer sized for `s.canvas`.
    pub fn render_to_new_buf(&mut self, s: &PathForgeSettings, scroll: f32, global_t: f32) -> Vec<u8> {
        let mut buf = vec![0u8; s.canvas.w() * s.canvas.h() * 4];
        self.render(s, scroll, global_t, &mut buf);
        buf
    }
    /// Render one frame into `buf` (CANVAS_W × CANVAS_H × 4, RGBA).
    /// `scroll` is the world-space scroll offset (increases each frame).
    /// `global_t` is a dimensionless time counter for animation.
    pub fn render(&mut self, s: &PathForgeSettings, scroll: f32, global_t: f32, buf: &mut Vec<u8>) {
        let cw = s.canvas.w();
        let ch = s.canvas.h();
        assert_eq!(buf.len(), cw * ch * 4);
        self.update_tile_cache(s);

        let cx  = cw as f32 * 0.5;
        let hy_scale = ch as f32 / 768.0;
        let hw_scale = cw as f32 / 576.0;
        let hy  = ((s.scene.horizon_y as f32) * hy_scale)
            .round()
            .clamp(8.0, (ch.saturating_sub(8)) as f32) as usize;
        let focal = (ch - hy) as f32 * s.scene.focal_mult;
        let cam_h = s.scene.cam_h;
        let max_hw = s.scene.max_hw * hw_scale;
        let pw = s.scene.path_power;
        let [vr, vg, vb] = s.scene.void_color;

        // ── 1. Fill background with void / sky ─────────────────────────────
        for y in 0..ch {
            let (r, g, b) = if s.sky.enabled && y < hy {
                let t = y as f32 / hy as f32; // 0=top, 1=horizon
                (lerp(s.sky.top[0] as f32, s.sky.horizon[0] as f32, t) as u8,
                 lerp(s.sky.top[1] as f32, s.sky.horizon[1] as f32, t) as u8,
                 lerp(s.sky.top[2] as f32, s.sky.horizon[2] as f32, t) as u8)
            } else {
                (vr, vg, vb)
            };
            for x in 0..cw {
                let pi = (y * cw + x) * 4;
                buf[pi] = r; buf[pi+1] = g; buf[pi+2] = b; buf[pi+3] = 255;
            }
        }

        // ── 2. Pre-compute path half-width for every row ───────────────────
        let mut phw_arr = vec![0.0f32; ch];
        for y in hy..ch {
            let t = (y - hy) as f32 / (ch - hy) as f32;
            phw_arr[y] = max_hw * (2.0*t - t*t).max(0.0).powf(pw);
        }

        let ft = &self.floor_tile;
        let wt = &self.wall_tile;
        let floor_tex_scale = s.floor.tex_scale.max(0.05);
        let wall_tex_scale = s.walls.tex_scale.max(0.05);
        let floor_rot_90 = s.floor.tex_rot_90;
        let wall_rot_90 = s.walls.tex_rot_90;

        // ── 3. Floor (perspective-corrected tile sampling) ─────────────────
        for y in (hy+1)..ch {
            let phw = phw_arr[y];
            if phw < 0.5 { continue; }
            let p_  = (y - hy) as f32;
            let d   = cam_h * focal / p_;
            let wz  = d + scroll * TILE as f32;
            let ds  = (p_ / (ch - hy) as f32 * s.floor.depth_fade).min(1.0);

            let x_min = ((cx - phw + 0.5) as i32).max(0) as usize;
            let x_max = ((cx + phw - 0.5) as i32).min(cw as i32) as usize;

            for x in x_min..x_max {
                let wx = (x as f32 - cx) / focal * d;
                let dc = (x as f32 - cx).abs();
                let es = ((phw - dc) / phw).max(0.0).sqrt();
                let sh = ds * ((1.0 - s.floor.edge_vignette) + s.floor.edge_vignette * es);
                let [tr, tg, tb] = sample_tile_rgb_oriented(
                    ft,
                    wx * floor_tex_scale,
                    wz * floor_tex_scale,
                    floor_rot_90,
                );
                let pi = (y * cw + x) * 4;
                buf[pi]   = (tr * sh) as u8;
                buf[pi+1] = (tg * sh) as u8;
                buf[pi+2] = (tb * sh) as u8;
            }
        }

        // ── 3.5. Grass tufts at path edges ────────────────────────────────
        if s.scene.grass_enabled {
            let [gr, gg, gb] = s.scene.grass_color;
            for y in (hy + 3)..ch {
                let phw = phw_arr[y];
                if phw < 1.5 { continue; }
                let p_  = (y - hy) as f32;
                let d   = cam_h * focal / p_;
                for &sgn in &[-1.0f32, 1.0] {
                    let edge_x = cx + sgn * phw;
                    // Place a blade cluster every ~TILE world units
                    let wz_pos = d + scroll * TILE as f32;
                    let blade_seed = inst_hash(42, (wz_pos * 0.5) as i32 + (edge_x as i32 / 4));
                    let jit_x = (blade_seed & 3) as f32 - 1.5;
                    let gx = (edge_x + jit_x).round() as i32;
                    let blade_h = ((focal * 0.065 / d).max(1.0).min(9.0)) as usize;
                    let brightness = 0.55 + ((blade_seed >> 4) & 0x3f) as f32 / 256.0;
                    for dy in 0..blade_h {
                        let gy = y as i32 - dy as i32;
                        if gy < 0 || gy >= ch as i32 { continue; }
                        for gx2 in (gx - 1).max(0)..=(gx + 1).min(cw as i32 - 1) {
                            let pi = (gy as usize * cw + gx2 as usize) * 4;
                            let fade = 1.0 - dy as f32 / blade_h as f32;
                            buf[pi]   = ((gr as f32 * brightness * fade).min(255.0) as u8).max(buf[pi]);
                            buf[pi+1] = ((gg as f32 * brightness * fade).min(255.0) as u8).max(buf[pi+1]);
                            buf[pi+2] = ((gb as f32 * brightness * fade * 0.7).min(255.0) as u8).max(buf[pi+2]);
                        }
                    }
                }
            }
        }

        // ── 4. Walls (lateral perspective) ────────────────────────────────
        let l_wx = s.walls.l_wx;
        if s.walls.enabled {
        let wall_fade = s.walls.fade_rows.max(1) as usize;
        let top_rows = (hy as f32 * s.walls.top_coverage.clamp(0.0, 1.0)).round() as usize;
        let ws = hy.saturating_sub(top_rows);

        for y in ws..ch {
            let phw  = if y >= hy { phw_arr[y] } else { 0.0 };
            let below = y >= hy;
            for x in 0..cw {
                let dc = (x as f32 - cx).abs();
                if below && dc <= phw { continue; }

                let dl   = dc.max(0.5);
                let wz_w = focal * l_wx / dl;
                let wy_w = cam_h - (y as i32 - hy as i32) as f32 * wz_w / focal;
                let [wr, wg, wb] = sample_tile_rgb_oriented(
                    wt,
                    wy_w * wall_tex_scale,
                    (wz_w + scroll * TILE as f32) * wall_tex_scale,
                    wall_rot_90,
                );
                let pi   = (y * cw + x) * 4;

                if below {
                    let ped = (dc - phw).max(0.0);
                    let mut sh = (s.walls.bright / wz_w.max(0.1)).min(1.0)
                        * (0.42 + ped / s.walls.junc_shadow.max(1.0)).min(1.0);
                    sh = sh.max(0.04);
                    buf[pi]   = (wr * sh + 2.0).min(255.0) as u8;
                    buf[pi+1] = (wg * sh + 2.0).min(255.0) as u8;
                    buf[pi+2] = (wb * sh + 2.0).min(255.0) as u8;
                    buf[pi+3] = 255;
                } else {
                    let fade = ((y - ws) as f32 / wall_fade as f32).clamp(0.0, 1.0);
                    let ped = (dc - phw).max(0.0);
                    let mut sh = (s.walls.bright / wz_w.max(0.1)).min(1.0)
                        * (0.42 + ped / s.walls.junc_shadow.max(1.0)).min(1.0);
                    sh *= 0.35 + 0.65 * fade;
                    sh = sh.max(0.02);
                    if sh < 0.005 { continue; }
                    // Keep wall hue consistent above horizon; only fade light contribution.
                    buf[pi]   = (wr * sh + 1.0).min(255.0) as u8;
                    buf[pi+1] = (wg * sh + 1.0).min(255.0) as u8;
                    buf[pi+2] = (wb * sh + 1.0).min(255.0) as u8;
                    buf[pi+3] = 255;
                }
            }
        }
        } // end if s.walls.enabled

        // ── 5. Atmosphere (multi-layer) ────────────────────────────────────
        for layer in s.atmo.layers.iter().filter(|l| l.enabled && l.atmo_type != AtmoType::None) {
            if let Some(glow_col) = layer.atmo_type.glow_color() {
                let flames = layer.atmo_type.flame_colors();
                let spc = layer.torch_spc as f32;
                let fx_scale = layer.fx_scale.max(0.2);
                let jitter = layer.placement_jitter.clamp(0.0, 1.0);
                let flicker = layer.flicker.max(0.0);
                // Continuous modular placement — seamlessly wraps at any scroll value
                let min_wz = 0.12f32;
                let max_wz = 24.0f32;
                let n_lo = ((scroll + min_wz) / spc).ceil() as i32;
                let n_hi = ((scroll + max_wz) / spc).floor() as i32;
                for n in n_lo..=n_hi {
                    let wz = n as f32 * spc - scroll;
                    if wz < min_wz || wz > max_wz { continue; }
                    let fr = focal * layer.torch_scale * fx_scale / wz;
                    if fr < 0.5 { continue; }
                    for &sgn in &[-1.0f32, 1.0] {
                        let side_i = if sgn > 0.0 { 1 } else { 0 };
                        let jx = (inst_hash_f(layer.variation_seed + 17 + side_i, n) * 2.0 - 1.0) * jitter;
                        let jy = (inst_hash_f(layer.variation_seed + 23 + side_i, n) * 2.0 - 1.0) * jitter;
                        let sx = cx + focal * sgn * (l_wx + jx * 0.45) / wz;
                        let sy = hy as f32 + focal * (cam_h - layer.torch_h + jy * 0.35) / wz;
                        if sx < -50.0 || sx > cw as f32 + 50.0 { continue; }
                        if sy < -50.0 || sy > ch as f32 + 50.0 { continue; }

                        draw_light_fixture(buf, cw, ch, sx, sy, fr, &layer.atmo_type, sgn);

                        // Glow disc (additive)
                        let gl_r = (fr * 3.8).max(3.0);
                        draw_glow_additive(buf, cw, ch, sx, sy, gl_r, glow_col, 0.75);

                        if let Some(flame_cols) = flames {
                            // Flame flicker
                            let fl = 0.16 * flicker * f32::sin(global_t * 7.0 * std::f32::consts::TAU
                                + n as f32 * 1.7 + sgn * 0.5);
                            let fh = (fr * (1.0 + fl * 0.2)).max(1.0);
                            draw_flame_additive(buf, cw, ch, sx, sy, fh, fl, flame_cols);
                        } else {
                            // Firefly / pulse
                            let pulse = 0.25 + 0.75
                                * (global_t * std::f32::consts::TAU * 4.0
                                   + n as f32 * 1.4 + sgn).sin().abs();
                            let ga = pulse * 0.8;
                            draw_glow_additive(buf, cw, ch, sx, sy, fr.max(1.0), glow_col, ga);
                        }
                    }
                }
            }
        }

        // ── 5.5 Props (back-to-front, continuous modular placement) ──────
        for prop in s.props.items.iter().filter(|p| p.enabled) {
            let min_wz = 0.15f32;
            let max_wz = 30.0f32;
            let spc    = prop.z_spacing.max(0.1);
            let n_lo = ((scroll + min_wz) / spc).ceil() as i32;
            let n_hi = ((scroll + max_wz) / spc).floor() as i32;
            for n in (n_lo..=n_hi).rev() { // back-to-front
                let wz = n as f32 * spc - scroll;
                if wz < min_wz || wz > max_wz { continue; }

                // Per-instance seeded variation (deterministic — same seed → same variant)
                let sv   = inst_hash_f(prop.seed, n);
                let sc_v = prop.scale * (1.0 + (sv * 2.0 - 1.0) * prop.scale_var);
                let ps   = focal * sc_v / wz;
                if ps < 0.8 { continue; }

                // y_sink: shift base downward so props appear grounded
                let sy_floor = hy as f32 + focal * cam_h / wz;
                let sink = prop.y_sink.clamp(0.0, 6.0);
                let sy_base  = sy_floor + sink * ps * 0.9;
                if sy_floor <= hy as f32 || sy_floor >= ch as f32 + ps * 8.0 { continue; }

                // Tint variation (slight colour shift per instance)
                let tv  = ((inst_hash(prop.seed + 2, n) & 0x1f) as i32 - 16) as f32;
                let tint = [
                    (prop.tint[0] as f32 + tv * 0.5).clamp(0.0, 255.0) as u8,
                    (prop.tint[1] as f32 + tv * 0.8).clamp(0.0, 255.0) as u8,
                    (prop.tint[2] as f32 + tv * 0.4).clamp(0.0, 255.0) as u8,
                ];

                let wxs: &[f32] = if prop.mirror { &[prop.wx, -prop.wx] } else { &[prop.wx] };
                for &wx_v in wxs {
                    // x_jitter: small lateral offset per instance (seeded by side)
                    let jit_seed = if wx_v > 0.0 { prop.seed + 3 } else { prop.seed + 4 };
                    let jit = (inst_hash_f(jit_seed, n) * 2.0 - 1.0) * prop.x_jitter;
                    let sx  = cx + focal * (wx_v + jit) / wz;
                    if sx < -(ps * 8.0) || sx > cw as f32 + ps * 8.0 { continue; }
                    let style_mix = prop.tree_style_mix.clamp(0.0, 1.0);
                    let style_bias = prop.tree_style_bias.clamp(-1.0, 1.0);
                    let draw_type = if matches!(prop.prop_type, PropType::Tree | PropType::PineTree | PropType::DeadTree)
                        && style_mix > 0.0
                    {
                        let pick = inst_hash_f(prop.seed + 55, n * 3 + if wx_v > 0.0 { 1 } else { 0 });
                        if pick < style_mix {
                            let bias_pick = inst_hash_f(prop.seed + 63, n * 7);
                            let pine_weight = (0.5 + 0.45 * style_bias).clamp(0.05, 0.95);
                            if bias_pick < pine_weight { PropType::PineTree } else { PropType::DeadTree }
                        } else {
                            PropType::Tree
                        }
                    } else {
                        prop.prop_type.clone()
                    };
                    let rock_var = inst_hash_f(prop.seed + 101, n * 13 + if wx_v > 0.0 { 1 } else { 0 });
                    match draw_type {
                        PropType::Tree     => draw_tree(buf, cw, ch, sx, sy_base, ps, tint),
                        PropType::PineTree => draw_pine_tree(buf, cw, ch, sx, sy_base, ps, tint),
                        PropType::Bush     => draw_bush(buf, cw, ch, sx, sy_base, ps, tint),
                        PropType::Rock     => draw_rock(buf, cw, ch, sx, sy_base, ps, tint, rock_var),
                        PropType::Boulder  => draw_rock(buf, cw, ch, sx, sy_base, ps * 1.7, tint, rock_var),
                        PropType::Cactus   => draw_cactus(buf, cw, ch, sx, sy_base, ps, tint),
                        PropType::DeadTree => draw_dead_tree(buf, cw, ch, sx, sy_base, ps, tint),
                        PropType::Mushroom => draw_mushroom(buf, cw, ch, sx, sy_base, ps, tint),
                    }
                    let embed = (prop.ground_blend.clamp(0.0, 1.0) * (0.35 + sink * 0.2)).clamp(0.0, 1.0);
                    if embed > 0.01 {
                        draw_ground_embed(buf, cw, ch, sx, sy_floor, ps, embed);
                    }
                }
            }
        }

        // ── 6. Floor debris (per enabled layer, continuous modular) ────────
        const D_WX: [f32; 10] = [-0.22,0.28,-0.04,0.14,-0.35,0.38,0.10,-0.18,0.32,-0.08];
        for layer in s.atmo.layers.iter().filter(|l| l.enabled && l.n_debris > 0) {
            let n_deb = (layer.n_debris as usize).min(D_WX.len());
            let spc   = layer.torch_spc as f32;
            let deb_scale = layer.fx_scale.max(0.2);
            let deb_jitter = layer.placement_jitter.clamp(0.0, 1.0);
            let min_wz = 0.12f32;
            let max_wz = 22.0f32;
            let n_lo = ((scroll + min_wz) / spc).ceil() as i32;
            let n_hi = ((scroll + max_wz) / spc).floor() as i32;
            for i in 0..n_deb {
                for n in n_lo..=n_hi {
                    let wz = n as f32 * spc - scroll;
                    if wz < min_wz || wz > max_wz { continue; }
                    let jw = (inst_hash_f(layer.variation_seed + i as u32 + 71, n) * 2.0 - 1.0) * deb_jitter * 0.2;
                    let sx = cx + focal * (D_WX[i] + jw) / wz;
                    let sy = hy as f32 + focal * cam_h / wz;
                    if sy <= hy as f32 || sy >= ch as f32 { continue; }
                    let ty   = (sy - hy as f32) / (ch - hy) as f32;
                    let phw2 = max_hw * (2.0*ty - ty*ty).max(0.0).powf(pw);
                    if (sx - cx).abs() > phw2 * 0.88 { continue; }
                    let dr = (focal * 0.022 * deb_scale / wz).max(0.5);
                    draw_ellipse_dark(buf, cw, ch, sx, sy, dr, dr * 0.4);
                }
            }
        }

        // ── 7. Dust motes (per enabled layer) ─────────────────────────────
        let t2 = global_t * std::f32::consts::TAU;
        for (li, layer) in s.atmo.layers.iter().enumerate().filter(|(_,l)| l.enabled && l.n_motes > 0) {
            let mut rng2 = seeded_rng(77 + li as u32 * 37 + layer.variation_seed.wrapping_mul(11));
            let n_motes = (layer.n_motes as usize).min(25);
            let mote_scale = layer.fx_scale.max(0.2);
            for _ in 0..n_motes {
                let bx  = rng2() * (cw as f32 - 44.0) + 22.0;
                let by  = rng2() * (ch as f32 - hy as f32 - 32.0) + hy as f32 + 14.0;
                let ax  = rng2() * 8.0 + 3.0;
                let ay  = rng2() * 5.0 + 1.0;
                let ppx = rng2() * std::f32::consts::TAU;
                let ppy = rng2() * std::f32::consts::TAU;
                let fx2 = (rng2() * 2.0) as u32 + 1;
                let fy2 = (rng2() * 2.0) as u32 + 1;
                let br  = (50.0 + rng2() * 65.0 * mote_scale) as u8;
                let mx  = bx + ax * f32::sin(fx2 as f32 * t2 + ppx);
                let my  = by + ay * f32::sin(fy2 as f32 * t2 + ppy);
                let px  = mx as usize;
                let py  = my as usize;
                if px < cw && py < ch {
                    let pi = (py * cw + px) * 4;
                    let br2 = (br as f32 * mote_scale).min(255.0) as u8;
                    buf[pi]   = (buf[pi]   as u32 + br2 as u32).min(255) as u8;
                    buf[pi+1] = (buf[pi+1] as u32 + br2 as u32).min(255) as u8;
                    buf[pi+2] = (buf[pi+2] as u32 + br2 as u32).min(255) as u8;
                }
            }
        }

        // ── 8. Post-process ────────────────────────────────────────────────
        let post = &s.post;

        // Saturation: adjust before other effects so vignette stays colour-neutral
        if (post.saturation - 1.0).abs() > 0.02 {
            let sat = post.saturation;
            for i in 0..cw * ch {
                let pi = i * 4;
                let r = buf[pi] as f32;
                let g = buf[pi+1] as f32;
                let b = buf[pi+2] as f32;
                let lum = 0.299 * r + 0.587 * g + 0.114 * b;
                buf[pi]   = (lum + sat * (r - lum)).clamp(0.0, 255.0) as u8;
                buf[pi+1] = (lum + sat * (g - lum)).clamp(0.0, 255.0) as u8;
                buf[pi+2] = (lum + sat * (b - lum)).clamp(0.0, 255.0) as u8;
            }
        }

        // Bloom: fast separable Gaussian on bright pixels
        if post.bloom > 0.01 {
            apply_bloom(buf, cw, ch, post.bloom, 200u8, 4);
        }

        // Depth fog
        if post.fog_enabled && post.fog_density > 0.0 {
            let fc = post.fog_color;
            for y in hy..ch {
                let depth_t = 1.0 - ((y - hy) as f32 / (ch - hy) as f32);
                let fog_a = (depth_t * depth_t * post.fog_density).min(0.92);
                if fog_a < 0.005 { continue; }
                for x in 0..cw {
                    let pi = (y * cw + x) * 4;
                    buf[pi]   = lerp(buf[pi]   as f32, fc[0] as f32, fog_a) as u8;
                    buf[pi+1] = lerp(buf[pi+1] as f32, fc[1] as f32, fog_a) as u8;
                    buf[pi+2] = lerp(buf[pi+2] as f32, fc[2] as f32, fog_a) as u8;
                }
            }
        }

        // Vignette
        if post.vignette > 0.005 {
            let cx_f = cw as f32 * 0.5;
            let cy_f = ch as f32 * 0.5;
            let max_dsq = cx_f * cx_f + cy_f * cy_f;
            let v = post.vignette;
            for y in 0..ch {
                for x in 0..cw {
                    let dx = x as f32 - cx_f;
                    let dy = y as f32 - cy_f;
                    let t = (dx*dx + dy*dy) / max_dsq;
                    let factor = (1.0 - t.powf(1.3) * v).max(0.0);
                    let pi = (y * cw + x) * 4;
                    buf[pi]   = (buf[pi]   as f32 * factor) as u8;
                    buf[pi+1] = (buf[pi+1] as f32 * factor) as u8;
                    buf[pi+2] = (buf[pi+2] as f32 * factor) as u8;
                }
            }
        }

        // Film grain
        if post.grain > 0.005 {
            let grain_scale = post.grain * 38.0;
            let frame_seed  = (global_t * 997.0) as u32;
            for y in 0..ch {
                for x in 0..cw {
                    let g = grain_noise(x, y, frame_seed) * grain_scale;
                    let pi = (y * cw + x) * 4;
                    buf[pi]   = (buf[pi]   as f32 + g).clamp(0.0, 255.0) as u8;
                    buf[pi+1] = (buf[pi+1] as f32 + g).clamp(0.0, 255.0) as u8;
                    buf[pi+2] = (buf[pi+2] as f32 + g).clamp(0.0, 255.0) as u8;
                }
            }
        }

    } // end render()

    // ── Tile cache ──────────────────────────────────────────────────────────
    fn update_tile_cache(&mut self, s: &PathForgeSettings) {
        let fk = TileKey {
            pattern: s.floor.pattern.name().to_owned(),
            base:    s.floor.base,
            mortar:  s.floor.mortar,
            noise:   s.floor.noise + (s.floor.damage.clamp(0.0, 1.0) * 20.0) as u32,
            seed:    11 + s.floor.pattern.gen_seed_offset() + s.floor.variation_seed,
        };
        if self.floor_key.as_ref() != Some(&fk) {
            self.floor_tile = gen_floor_tile(
                &s.floor.pattern, s.floor.base, s.floor.mortar,
                s.floor.noise, s.floor.damage, fk.seed);
            self.floor_key = Some(fk);
        }

        let wk = TileKey {
            pattern: s.walls.pattern.name().to_owned(),
            base:    s.walls.base,
            mortar:  s.walls.mortar,
            noise:   s.walls.noise + (s.walls.damage.clamp(0.0, 1.0) * 20.0) as u32,
            seed:    22 + s.walls.pattern.gen_seed_offset() + s.walls.variation_seed,
        };
        if self.wall_key.as_ref() != Some(&wk) {
            self.wall_tile = gen_wall_tile(
                &s.walls.pattern, s.walls.base, s.walls.mortar,
                s.walls.noise, s.walls.damage, wk.seed);
            self.wall_key = Some(wk);
        }
    }
}

// ── Internal drawing utilities ─────────────────────────────────────────────

/// Map a world-space float coordinate to a tile texel index (matches JSX floor+mod logic).
#[inline]
fn floor_mod(v: f32, m: i32) -> usize {
    let i = v.floor() as i32;
    i.rem_euclid(m) as usize
}

#[inline]
fn lerp(a: f32, b: f32, t: f32) -> f32 { a + (b - a) * t }

#[inline]
fn sample_tile_rgb(tile: &[u8], u: f32, v: f32) -> [f32; 3] {
    let tex_px = (tile.len() / 3).max(1);
    let tex_side = (tex_px as f32).sqrt() as usize;
    let tex_side = tex_side.max(1);
    let tf = TILE as f32;
    // Crisp nearest-neighbour sampling preserves mortar/brick line definition.
    let uf = ((u.rem_euclid(tf) / tf) * tex_side as f32).clamp(0.0, tex_side as f32 - f32::EPSILON);
    let vf = ((v.rem_euclid(tf) / tf) * tex_side as f32).clamp(0.0, tex_side as f32 - f32::EPSILON);
    let x = (uf.floor() as usize).min(tex_side - 1);
    let y = (vf.floor() as usize).min(tex_side - 1);
    let p = (y * tex_side + x) * 3;
    [tile[p] as f32, tile[p + 1] as f32, tile[p + 2] as f32]
}

#[inline]
fn sample_tile_rgb_oriented(tile: &[u8], u: f32, v: f32, rot_90: bool) -> [f32; 3] {
    if rot_90 {
        sample_tile_rgb(tile, v, -u)
    } else {
        sample_tile_rgb(tile, u, v)
    }
}

/// Additive radial glow (replicates Canvas 2D `globalCompositeOperation='lighter'` + radial gradient).
fn draw_glow_additive(
    buf: &mut [u8], w: usize, h: usize,
    cx: f32, cy: f32, radius: f32, color: [u8; 3], peak_alpha: f32,
) {
    let ri = radius as i32 + 1;
    let x0 = (cx as i32 - ri).clamp(0, w as i32) as usize;
    let x1 = (cx as i32 + ri).clamp(0, w as i32) as usize;
    let y0 = (cy as i32 - ri).clamp(0, h as i32) as usize;
    let y1 = (cy as i32 + ri).clamp(0, h as i32) as usize;
    if x1 <= x0 || y1 <= y0 { return; }

    for y in y0..y1 {
        for x in x0..x1 {
            let dx   = x as f32 - cx;
            let dy   = y as f32 - cy;
            let dist = (dx*dx + dy*dy).sqrt();
            if dist >= radius { continue; }

            let t = 1.0 - dist / radius; // 1 at centre, 0 at edge
            // Two-stop gradient: 0.75→0.18→0 matching JSX addColorStop values
            let ga = if t > 0.6 {
                peak_alpha * lerp(0.18, 0.75, (t - 0.6) / 0.4)
            } else {
                peak_alpha * 0.18 * t / 0.6
            };
            let ga = ga.min(1.0);

            let pi = (y * w + x) * 4;
            buf[pi]   = (buf[pi]   as f32 + color[0] as f32 * ga).min(255.0) as u8;
            buf[pi+1] = (buf[pi+1] as f32 + color[1] as f32 * ga).min(255.0) as u8;
            buf[pi+2] = (buf[pi+2] as f32 + color[2] as f32 * ga).min(255.0) as u8;
        }
    }
}

/// Additive elongated flame glow (replicates JSX createRadialGradient + ellipse scale transform).
fn draw_flame_additive(
    buf: &mut [u8], w: usize, h: usize,
    cx: f32, cy: f32, fh: f32, fl: f32, colors: [[u8; 3]; 4],
) {
    let sx = 0.6 + fl.abs() * 0.08;  // X scale (matches JSX ctx.scale)
    let sy = 1.6 + fl * 0.35;        // Y scale
    let center_y = cy - fh * 0.5;    // shift up like JSX ctx.translate(sx, sy - fh*0.5)

    let half_w = (fh * sx * 1.2 + 1.0) as i32;
    let half_h = (fh * sy * 1.2 + 1.0) as i32;

    let x0 = (cx as i32 - half_w).clamp(0, w as i32) as usize;
    let x1 = (cx as i32 + half_w).clamp(0, w as i32) as usize;
    let y0 = (center_y as i32 - half_h).clamp(0, h as i32) as usize;
    let y1 = (center_y as i32 + half_h).clamp(0, h as i32) as usize;
    if x1 <= x0 || y1 <= y0 { return; }

    for y in y0..y1 {
        for x in x0..x1 {
            let dx   = (x as f32 - cx)       / (sx * fh);
            let dy   = (y as f32 - center_y) / (sy * fh);
            let dist = (dx*dx + dy*dy).sqrt();
            if dist >= 1.0 { continue; }

            // 4-stop gradient: dist thresholds 0→0.35→0.65→1.0
            let (cr, cg, cb, ca) = if dist < 0.35 {
                let t = dist / 0.35;
                (lerp(colors[0][0] as f32, colors[1][0] as f32, t),
                 lerp(colors[0][1] as f32, colors[1][1] as f32, t),
                 lerp(colors[0][2] as f32, colors[1][2] as f32, t),
                 lerp(0.95, 0.80, t))
            } else if dist < 0.65 {
                let t = (dist - 0.35) / 0.30;
                (lerp(colors[1][0] as f32, colors[2][0] as f32, t),
                 lerp(colors[1][1] as f32, colors[2][1] as f32, t),
                 lerp(colors[1][2] as f32, colors[2][2] as f32, t),
                 lerp(0.80, 0.50, t))
            } else {
                let t = (dist - 0.65) / 0.35;
                (lerp(colors[2][0] as f32, colors[3][0] as f32, t),
                 lerp(colors[2][1] as f32, colors[3][1] as f32, t),
                 lerp(colors[2][2] as f32, colors[3][2] as f32, t),
                 lerp(0.50, 0.00, t))
            };

            let pi = (y * w + x) * 4;
            buf[pi]   = (buf[pi]   as f32 + cr * ca).min(255.0) as u8;
            buf[pi+1] = (buf[pi+1] as f32 + cg * ca).min(255.0) as u8;
            buf[pi+2] = (buf[pi+2] as f32 + cb * ca).min(255.0) as u8;
        }
    }
}

/// Dark semi-transparent ellipse for floor debris (dark blob, alpha ~0.72 over-paint).
fn draw_ellipse_dark(buf: &mut [u8], w: usize, h: usize, cx: f32, cy: f32, rx: f32, ry: f32) {
    let x0 = (cx as i32 - rx as i32 - 1).clamp(0, w as i32) as usize;
    let x1 = (cx as i32 + rx as i32 + 1).clamp(0, w as i32) as usize;
    let y0 = (cy as i32 - ry as i32 - 1).clamp(0, h as i32) as usize;
    let y1 = (cy as i32 + ry as i32 + 1).clamp(0, h as i32) as usize;
    if x1 <= x0 || y1 <= y0 { return; }

    for y in y0..y1 {
        for x in x0..x1 {
            let dx = (x as f32 - cx) / rx;
            let dy = (y as f32 - cy) / ry.max(0.1);
            if dx*dx + dy*dy < 1.0 {
                let pi = (y * w + x) * 4;
                let a = 0.72f32;
                buf[pi]   = (buf[pi]   as f32 * (1.0 - a) + 16.0 * a) as u8;
                buf[pi+1] = (buf[pi+1] as f32 * (1.0 - a) + 12.0 * a) as u8;
                buf[pi+2] = (buf[pi+2] as f32 * (1.0 - a) +  9.0 * a) as u8;
            }
        }
    }
}

fn draw_ground_embed(buf: &mut [u8], w: usize, h: usize, sx: f32, sy_floor: f32, ps: f32, amount: f32) {
    let a = amount.clamp(0.0, 1.0);
    let r0x = (ps * 1.9).max(0.6);
    let r0y = (ps * 0.42).max(0.4);
    let r1x = r0x * 1.45;
    let r1y = r0y * 1.8;
    let y0 = sy_floor + ps * 0.05;
    draw_ellipse_dark(buf, w, h, sx, y0, r1x, r1y);
    draw_ellipse_dark(buf, w, h, sx, y0 + ps * 0.04, r0x, r0y);

    let x0 = (sx as i32 - r1x as i32 - 2).clamp(0, w as i32) as usize;
    let x1 = (sx as i32 + r1x as i32 + 2).clamp(0, w as i32) as usize;
    let y_start = y0.max(0.0) as usize;
    let y_end = (y0 + ps * 1.1).min(h as f32) as usize;
    for y in y_start..y_end {
        let t = ((y as f32 - y0) / (ps * 1.1).max(0.1)).clamp(0.0, 1.0);
        let blend = (1.0 - t) * a * 0.28;
        for x in x0..x1 {
            let pi = (y * w + x) * 4;
            buf[pi] = (buf[pi] as f32 * (1.0 - blend)) as u8;
            buf[pi + 1] = (buf[pi + 1] as f32 * (1.0 - blend)) as u8;
            buf[pi + 2] = (buf[pi + 2] as f32 * (1.0 - blend)) as u8;
        }
    }
}

fn draw_light_fixture(
    buf: &mut [u8],
    w: usize,
    h: usize,
    sx: f32,
    sy: f32,
    fr: f32,
    atmo_type: &AtmoType,
    side: f32,
) {
    let post = [58, 46, 34];
    let metal = [115, 110, 96];
    let dark_metal = [62, 58, 52];
    let warm = [208, 148, 84];
    let sign = if side >= 0.0 { 1.0 } else { -1.0 };

    match atmo_type {
        AtmoType::Torch | AtmoType::GreenFire | AtmoType::Magic | AtmoType::IceWisp => {
            let stem_h = (fr * 1.6).max(2.0);
            let cup_w = (fr * 0.7).max(1.0);
            fill_rect_px(
                buf,
                w,
                h,
                (sx - sign * fr * 0.9) as i32,
                (sy + fr * 0.05) as i32,
                (sx + sign * fr * 0.15) as i32,
                (sy + fr * 0.18) as i32,
                dark_metal,
            );
            fill_rect_px(
                buf,
                w,
                h,
                (sx - cup_w * 0.35) as i32,
                (sy + fr * 0.12) as i32,
                (sx + cup_w * 0.35) as i32,
                (sy + fr * 0.24) as i32,
                metal,
            );
            fill_rect_px(
                buf,
                w,
                h,
                (sx - cup_w * 0.15) as i32,
                (sy + fr * 0.24) as i32,
                (sx + cup_w * 0.15) as i32,
                (sy + fr * 0.24 + stem_h) as i32,
                post,
            );
        }
        AtmoType::Lantern => {
            let lx = sx;
            let ly = sy + fr * 0.2;
            fill_rect_px(
                buf,
                w,
                h,
                (lx - sign * fr * 0.9) as i32,
                (ly - fr * 0.25) as i32,
                (lx + sign * fr * 0.05) as i32,
                (ly - fr * 0.18) as i32,
                dark_metal,
            );
            draw_line_px(buf, w, h, lx - sign * fr * 0.28, ly - fr * 0.18, lx - sign * fr * 0.08, ly + fr * 0.02, metal);
            fill_rect_px(
                buf,
                w,
                h,
                (lx - fr * 0.20) as i32,
                (ly + fr * 0.02) as i32,
                (lx + fr * 0.20) as i32,
                (ly + fr * 0.45) as i32,
                dark_metal,
            );
            fill_ellipse(buf, w, h, lx, ly + fr * 0.27, fr * 0.14, fr * 0.15, warm);
        }
        AtmoType::Candle => {
            let cx = sx;
            let cy = sy + fr * 0.28;
            fill_rect_px(
                buf,
                w,
                h,
                (cx - fr * 0.16) as i32,
                (cy + fr * 0.06) as i32,
                (cx + fr * 0.16) as i32,
                (cy + fr * 0.16) as i32,
                dark_metal,
            );
            fill_rect_px(
                buf,
                w,
                h,
                (cx - fr * 0.06) as i32,
                (cy - fr * 0.18) as i32,
                (cx + fr * 0.06) as i32,
                (cy + fr * 0.05) as i32,
                [224, 212, 185],
            );
        }
        _ => {}
    }
}

// ── Post-process helpers ───────────────────────────────────────────────────

/// Deterministic per-instance integer hash (returns 0..u32::MAX).
#[inline]
fn inst_hash(seed: u32, n: i32) -> u32 {
    let mut h = seed.wrapping_add(n.unsigned_abs())
                    .wrapping_mul(2246822519)
                    .wrapping_add(1013904223);
    h ^= h >> 16;
    h = h.wrapping_mul(0x45d9f3b);
    h ^= h >> 16;
    h
}

/// Per-instance float in 0..1.
#[inline]
fn inst_hash_f(seed: u32, n: i32) -> f32 {
    (inst_hash(seed, n) >> 8) as f32 / 0x00FF_FFFFu32 as f32
}

/// Film grain noise — returns -1..1 centred value.
#[inline]
fn grain_noise(x: usize, y: usize, frame_seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(2246822519)
                    .wrapping_add((y as u32).wrapping_mul(2654435761))
                    .wrapping_add(frame_seed.wrapping_mul(1013904223));
    h ^= h >> 16;
    (h & 0xFFFF) as f32 / 0x7FFF as f32 - 1.0
}

/// Fast approximate bloom: single-pass bright-extract + 8-tap sampling.
fn apply_bloom(buf: &mut Vec<u8>, w: usize, h: usize, intensity: f32, threshold: u8, radius: i32) {
    // Build bright-pass buffer (RGB only, no alpha)
    let mut bright = vec![0i16; w * h * 3];
    for i in 0..w * h {
        let pi = i * 4;
        let r = buf[pi]; let g = buf[pi+1]; let b = buf[pi+2];
        if r.max(g).max(b) > threshold {
            let bi = i * 3;
            bright[bi]   = r.saturating_sub(threshold) as i16;
            bright[bi+1] = g.saturating_sub(threshold) as i16;
            bright[bi+2] = b.saturating_sub(threshold) as i16;
        }
    }

    // Horizontal box blur
    let mut hblur = vec![0i32; w * h * 3];
    let ksize = (radius * 2 + 1) as usize;
    for y in 0..h {
        let mut rsum = 0i32; let mut gsum = 0i32; let mut bsum = 0i32;
        // Prime the window
        for kx in 0..ksize {
            let sx = (kx as i32 - radius).clamp(0, w as i32 - 1) as usize;
            let bi = (y * w + sx) * 3;
            rsum += bright[bi] as i32; gsum += bright[bi+1] as i32; bsum += bright[bi+2] as i32;
        }
        for x in 0..w {
            let bi = (y * w + x) * 3;
            hblur[bi]   = rsum / ksize as i32;
            hblur[bi+1] = gsum / ksize as i32;
            hblur[bi+2] = bsum / ksize as i32;
            // Slide window
            let remove_x = (x as i32 - radius).clamp(0, w as i32 - 1) as usize;
            let add_x    = (x as i32 + radius + 1).clamp(0, w as i32 - 1) as usize;
            let rb = (y * w + remove_x) * 3;
            let ab = (y * w + add_x) * 3;
            rsum += bright[ab] as i32 - bright[rb] as i32;
            gsum += bright[ab+1] as i32 - bright[rb+1] as i32;
            bsum += bright[ab+2] as i32 - bright[rb+2] as i32;
        }
    }
    drop(bright);

    // Vertical blur + add to output
    let scale = intensity * (1.0 / (ksize as f32 * 0.9));
    for x in 0..w {
        let mut rsum = 0i32; let mut gsum = 0i32; let mut bsum = 0i32;
        for ky in 0..ksize {
            let sy = (ky as i32 - radius).clamp(0, h as i32 - 1) as usize;
            let bi = (sy * w + x) * 3;
            rsum += hblur[bi]; gsum += hblur[bi+1]; bsum += hblur[bi+2];
        }
        for y in 0..h {
            let pi = (y * w + x) * 4;
            let dr = (rsum as f32 * scale) as u8;
            let dg = (gsum as f32 * scale) as u8;
            let db = (bsum as f32 * scale) as u8;
            buf[pi]   = buf[pi].saturating_add(dr);
            buf[pi+1] = buf[pi+1].saturating_add(dg);
            buf[pi+2] = buf[pi+2].saturating_add(db);
            let remove_y = (y as i32 - radius).clamp(0, h as i32 - 1) as usize;
            let add_y    = (y as i32 + radius + 1).clamp(0, h as i32 - 1) as usize;
            let rb = (remove_y * w + x) * 3;
            let ab = (add_y    * w + x) * 3;
            rsum += hblur[ab] - hblur[rb];
            gsum += hblur[ab+1] - hblur[rb+1];
            bsum += hblur[ab+2] - hblur[rb+2];
        }
    }
}

// ── Prop drawing helpers ───────────────────────────────────────────────────

#[inline]
fn darker(c: [u8; 3], f: f32) -> [u8; 3] {
    [(c[0] as f32 * f) as u8, (c[1] as f32 * f) as u8, (c[2] as f32 * f) as u8]
}
#[inline]
fn lighter(c: [u8; 3], f: f32) -> [u8; 3] {
    [(c[0] as f32 * f).min(255.0) as u8, (c[1] as f32 * f).min(255.0) as u8, (c[2] as f32 * f).min(255.0) as u8]
}

/// Filled solid ellipse (opaque over-paint). All clamps guard against usize underflow.
fn fill_ellipse(buf: &mut [u8], w: usize, h: usize,
    cx: f32, cy: f32, rx: f32, ry: f32, color: [u8; 3])
{
    if rx < 0.5 || ry < 0.5 { return; }
    let x0 = (cx as i32 - rx as i32 - 1).clamp(0, w as i32) as usize;
    let x1 = (cx as i32 + rx as i32 + 1).clamp(0, w as i32) as usize;
    let y0 = (cy as i32 - ry as i32 - 1).clamp(0, h as i32) as usize;
    let y1 = (cy as i32 + ry as i32 + 1).clamp(0, h as i32) as usize;
    if x1 <= x0 || y1 <= y0 { return; }
    for y in y0..y1 {
        for x in x0..x1 {
            let dx = (x as f32 - cx) / rx;
            let dy = (y as f32 - cy) / ry;
            if dx*dx + dy*dy < 1.0 {
                let pi = (y * w + x) * 4;
                buf[pi] = color[0]; buf[pi+1] = color[1]; buf[pi+2] = color[2];
            }
        }
    }
}

/// Filled solid axis-aligned rectangle (opaque over-paint).
fn fill_rect_px(buf: &mut [u8], w: usize, h: usize,
    x0i: i32, y0i: i32, x1i: i32, y1i: i32, color: [u8; 3])
{
    let x0 = x0i.clamp(0, w as i32) as usize;
    let x1 = x1i.clamp(0, w as i32) as usize;
    let y0 = y0i.clamp(0, h as i32) as usize;
    let y1 = y1i.clamp(0, h as i32) as usize;
    for y in y0..y1 {
        for x in x0..x1 {
            let pi = (y * w + x) * 4;
            buf[pi] = color[0]; buf[pi+1] = color[1]; buf[pi+2] = color[2];
        }
    }
}

/// Scanline-filled triangle with apex at (apex_x, apex_y) and a base of width base_w at apex_y+height.
fn fill_triangle(buf: &mut [u8], w: usize, h: usize,
    apex_x: f32, apex_y: f32, base_w: f32, height: f32, color: [u8; 3])
{
    if height < 0.5 || base_w < 0.5 { return; }
    let y_start = (apex_y as i32).max(0) as usize;
    let y_end   = ((apex_y + height) as i32).min(h as i32) as usize;
    for y in y_start..y_end {
        let t  = (y as f32 - apex_y) / height;
        let hw = t * base_w * 0.5;
        let x0 = ((apex_x - hw) as i32).clamp(0, w as i32) as usize;
        let x1 = ((apex_x + hw) as i32).clamp(0, w as i32) as usize;
        for x in x0..x1 {
            let pi = (y * w + x) * 4;
            buf[pi] = color[0]; buf[pi+1] = color[1]; buf[pi+2] = color[2];
        }
    }
}

/// 1-pixel-wide Bresenham-style line.
fn draw_line_px(buf: &mut [u8], w: usize, h: usize,
    x0: f32, y0: f32, x1: f32, y1: f32, color: [u8; 3])
{
    let len = ((x1-x0)*(x1-x0) + (y1-y0)*(y1-y0)).sqrt().ceil() as usize + 1;
    for i in 0..=len {
        let t = i as f32 / len as f32;
        let px = (x0 + t*(x1-x0)) as i32;
        let py = (y0 + t*(y1-y0)) as i32;
        if px >= 0 && px < w as i32 && py >= 0 && py < h as i32 {
            let pi = (py as usize * w + px as usize) * 4;
            buf[pi] = color[0]; buf[pi+1] = color[1]; buf[pi+2] = color[2];
        }
    }
}

// ── Prop draw functions (ps = pixel_scale = focal * prop.scale / wz) ───────

fn draw_tree(buf: &mut [u8], w: usize, h: usize, sx: f32, sy_base: f32, ps: f32, tint: [u8;3]) {
    let tw = (ps * 0.65).max(1.0);
    let th = ps * 4.2;
    let trunk_col = [
        (tint[0] as f32 * 0.32 + 20.0 * 0.68) as u8,
        (tint[1] as f32 * 0.20 + 14.0 * 0.80) as u8,
        (tint[2] as f32 * 0.10 +  8.0 * 0.90) as u8,
    ];
    fill_rect_px(buf, w, h,
        (sx - tw*0.5) as i32, (sy_base - th) as i32,
        (sx + tw*0.5) as i32, sy_base as i32, trunk_col);

    let cy_b = sy_base - th;
    fill_ellipse(buf, w, h, sx + ps*0.35, cy_b - ps*0.7, ps*2.6, ps*2.3, darker(tint, 0.55));
    fill_ellipse(buf, w, h, sx - ps*0.2,  cy_b - ps*1.8, ps*2.4, ps*2.1, darker(tint, 0.78));
    fill_ellipse(buf, w, h, sx - ps*0.4,  cy_b - ps*3.2, ps*1.8, ps*1.7, tint);
    fill_ellipse(buf, w, h, sx - ps*0.55, cy_b - ps*4.2, ps*1.2, ps*1.1, lighter(tint, 1.35));
}

fn draw_pine_tree(buf: &mut [u8], w: usize, h: usize, sx: f32, sy_base: f32, ps: f32, tint: [u8;3]) {
    let tw = (ps * 0.5).max(1.0);
    let th = ps * 2.2;
    let trunk_col = darker(tint, 0.30);
    fill_rect_px(buf, w, h,
        (sx - tw*0.5) as i32, (sy_base - th) as i32,
        (sx + tw*0.5) as i32, sy_base as i32, trunk_col);

    let base_y = sy_base - th;
    // 3 stacked tiers (bottom → top): each tier is a filled triangle with apex up
    let tiers: [(f32, f32, f32, f32); 3] = [
        (0.0,  3.4,  2.6, 0.65),   // (y_offset_up, base_w, height, color_f)
        (1.8,  2.8,  2.2, 0.82),
        (3.4,  2.1,  1.9, 1.00),
    ];
    for (y_off, bw, bh, cf) in tiers {
        let apex_y = base_y - y_off * ps - bh * ps;
        let col = if cf >= 1.0 { lighter(tint, cf) } else { darker(tint, cf) };
        fill_triangle(buf, w, h, sx, apex_y, bw * ps, bh * ps, col);
    }
}

fn draw_bush(buf: &mut [u8], w: usize, h: usize, sx: f32, sy_base: f32, ps: f32, tint: [u8;3]) {
    let r = ps * 2.2;
    fill_ellipse(buf, w, h, sx - ps,      sy_base - ps*0.9, r,       r*0.60, darker(tint, 0.60));
    fill_ellipse(buf, w, h, sx + ps*0.65, sy_base - ps*0.7, r*0.85,  r*0.55, darker(tint, 0.75));
    fill_ellipse(buf, w, h, sx,           sy_base - ps*1.3, r*0.90,  r*0.68, tint);
    fill_ellipse(buf, w, h, sx - ps*0.3,  sy_base - ps*2.1, r*0.65,  r*0.58, lighter(tint, 1.28));
}

fn draw_rock(buf: &mut [u8], w: usize, h: usize, sx: f32, sy_base: f32, ps: f32, tint: [u8;3], variant: f32) {
    let vr = variant.clamp(0.0, 1.0);
    let rx = ps * (1.7 + 0.8 * vr);
    let ry = ps * (1.1 + 0.7 * (1.0 - vr));
    let cy = sy_base - ry * (0.72 + 0.12 * vr);
    let skew = (vr * 2.0 - 1.0) * ps * 0.45;

    fill_ellipse(buf, w, h, sx + skew * 0.3, cy + ps * 0.20, rx * 1.02, ry * 0.98, darker(tint, 0.52));
    fill_ellipse(buf, w, h, sx - skew * 0.2, cy - ps * 0.02, rx * 0.86, ry * 0.82, tint);
    fill_ellipse(buf, w, h, sx - rx * 0.36, cy - ry * 0.30, rx * 0.26, ry * 0.22, lighter(tint, 1.28));
    fill_ellipse(buf, w, h, sx + rx * 0.18, cy - ry * 0.22, rx * 0.18, ry * 0.15, lighter(tint, 1.16));

    let crack = darker(tint, 0.34);
    draw_line_px(buf, w, h, sx - rx * 0.30, cy - ry * 0.08, sx + rx * 0.12, cy + ry * 0.06, crack);
    draw_line_px(buf, w, h, sx - rx * 0.02, cy - ry * 0.22, sx + rx * 0.22, cy + ry * 0.12, crack);
    if vr > 0.45 {
        draw_line_px(buf, w, h, sx - rx * 0.20, cy + ry * 0.05, sx + rx * 0.06, cy + ry * 0.23, crack);
    }
}

fn draw_cactus(buf: &mut [u8], w: usize, h: usize, sx: f32, sy_base: f32, ps: f32, tint: [u8;3]) {
    let tw = (ps * 0.95).max(1.0);
    let th = ps * 5.2;
    fill_rect_px(buf, w, h,
        (sx - tw*0.5) as i32, (sy_base - th) as i32,
        (sx + tw*0.5) as i32, sy_base as i32, tint);

    // Left arm: horizontal then up
    let arm_y = sy_base - th * 0.55;
    let aw = (ps * 0.7).max(1.0);
    fill_rect_px(buf, w, h,
        (sx - tw*0.5 - ps*2.2) as i32, arm_y as i32,
        (sx - tw*0.5          ) as i32, (arm_y + aw) as i32, tint);
    fill_rect_px(buf, w, h,
        (sx - tw*0.5 - ps*2.2) as i32, (arm_y - ps*1.6) as i32,
        (sx - tw*0.5 - ps*2.2 + aw) as i32, arm_y as i32, tint);
    // Right arm: horizontal then up
    let arm_y2 = sy_base - th * 0.40;
    fill_rect_px(buf, w, h,
        (sx + tw*0.5          ) as i32, (arm_y2 - aw*0.5) as i32,
        (sx + tw*0.5 + ps*1.9 ) as i32, (arm_y2 + aw*0.5) as i32, tint);
    fill_rect_px(buf, w, h,
        (sx + tw*0.5 + ps*1.9 - aw) as i32, (arm_y2 - ps*1.8) as i32,
        (sx + tw*0.5 + ps*1.9      ) as i32, (arm_y2 - aw*0.5) as i32, tint);
    // Rounded top
    fill_ellipse(buf, w, h, sx, sy_base - th, tw*0.75, tw*0.75, lighter(tint, 1.2));
}

fn draw_dead_tree(buf: &mut [u8], w: usize, h: usize, sx: f32, sy_base: f32, ps: f32, tint: [u8;3]) {
    let tw = (ps * 0.62).max(1.0);
    let th = ps * 5.0;
    fill_rect_px(buf, w, h,
        (sx - tw*0.5) as i32, (sy_base - th) as i32,
        (sx + tw*0.5) as i32, sy_base as i32, tint);

    // Bare branches (4 angled lines)
    let col_light = lighter(tint, 1.3);
    let branches: [(f32, f32, f32, f32); 4] = [
        (-1.4, 0.68, -1.0, 0.38),  // (dx_end, h_start_frac, dy_end, len_mult)
        ( 1.6, 0.52,  -0.9, 0.30),
        (-0.9, 0.38,  -0.7, 0.22),
        ( 1.1, 0.24,  -0.5, 0.18),
    ];
    for (dx, hf, dy, lm) in branches {
        let bx0 = sx;
        let by0 = sy_base - th * hf;
        let bx1 = sx + dx * ps * 4.0 * lm;
        let by1 = by0 + dy * ps * 4.0 * lm;
        draw_line_px(buf, w, h, bx0, by0, bx1, by1, col_light);
    }
}

fn draw_mushroom(buf: &mut [u8], w: usize, h: usize, sx: f32, sy_base: f32, ps: f32, tint: [u8;3]) {
    let sw = (ps * 0.58).max(1.0);
    let sh = ps * 2.0;
    let stem_col: [u8; 3] = [225, 215, 195];
    fill_rect_px(buf, w, h,
        (sx - sw*0.5) as i32, (sy_base - sh) as i32,
        (sx + sw*0.5) as i32, sy_base as i32, stem_col);

    let crx = ps * 1.95;
    let cry = ps * 1.25;
    let cap_cy = sy_base - sh - cry * 0.42;
    fill_ellipse(buf, w, h, sx + crx*0.10, cap_cy + cry*0.08, crx, cry, darker(tint, 0.72));
    fill_ellipse(buf, w, h, sx - crx*0.08, cap_cy - cry*0.06, crx*0.86, cry*0.80, tint);
    // White spots
    fill_ellipse(buf, w, h, sx - crx*0.22, cap_cy - cry*0.32, ps*0.30, ps*0.24, [245,245,245]);
    fill_ellipse(buf, w, h, sx + crx*0.32, cap_cy - cry*0.10, ps*0.22, ps*0.18, [245,245,245]);
    fill_ellipse(buf, w, h, sx - crx*0.50, cap_cy + cry*0.05, ps*0.16, ps*0.13, [240,240,240]);
}
