/// Software path renderer — Rust port of the JSX canvas render loop plus
/// PathForge 2.0 additions (sky gradient, enhanced depth fog).
use crate::settings::{
    AttachmentSurface, AtmoType, MountSide, LightingPreset, PathForgeSettings, PropType, TILE,
};
use crate::tiles::{gen_floor_tile, gen_wall_tile, seeded_rng, TileKey};
use image::ImageReader;
use std::collections::HashMap;

struct SpriteData {
    w: usize,
    h: usize,
    rgba: Vec<u8>,
}

#[derive(Clone, Copy, Debug)]
pub struct RenderLayers {
    pub sky: bool,
    pub floor: bool,
    pub walls: bool,
    pub atmo: bool,
    pub props: bool,
    pub post: bool,
}

impl RenderLayers {
    pub fn all() -> Self {
        Self { sky: true, floor: true, walls: true, atmo: true, props: true, post: true }
    }
}

// ── Renderer state ─────────────────────────────────────────────────────────
pub struct PathRenderer {
    floor_key:  Option<TileKey>,
    wall_key:   Option<TileKey>,
    floor_tile: Vec<u8>,
    wall_tile:  Vec<u8>,
    sprite_cache: HashMap<String, Option<SpriteData>>,
}

impl Default for PathRenderer {
    fn default() -> Self {
        Self {
            floor_key:  None,
            wall_key:   None,
            floor_tile: vec![128u8; TILE * TILE * 3],
            wall_tile:  vec![80u8;  TILE * TILE * 3],
            sprite_cache: HashMap::new(),
        }
    }
}

impl PathRenderer {
    /// Convenience: render into a freshly-allocated buffer sized for `s.canvas`.
    pub fn render_to_new_buf(&mut self, s: &PathForgeSettings, scroll: f32, global_t: f32) -> Vec<u8> {
        let mut buf = vec![0u8; s.canvas.w() * s.canvas.h() * 4];
        self.render_with_layers(s, scroll, global_t, &RenderLayers::all(), &mut buf);
        buf
    }
    /// Render one frame into `buf` (CANVAS_W × CANVAS_H × 4, RGBA).
    /// `scroll` is the world-space scroll offset (increases each frame).
    /// `global_t` is a dimensionless time counter for animation.
    pub fn render(&mut self, s: &PathForgeSettings, scroll: f32, global_t: f32, buf: &mut Vec<u8>) {
        self.render_with_layers(s, scroll, global_t, &RenderLayers::all(), buf);
    }

    pub fn render_with_layers(
        &mut self,
        s: &PathForgeSettings,
        scroll: f32,
        global_t: f32,
        layers: &RenderLayers,
        buf: &mut Vec<u8>,
    ) {
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
        let light_profile = lighting_profile(&s.scene.lighting_preset);
        let (atmo_light_energy, atmo_light_tint) = estimate_atmo_lighting(s);
        let atmo_light_gain = s.scene.atmo_light_influence.clamp(0.0, 2.0);
        let atmo_tint_gain = s.scene.atmo_tint_influence.clamp(0.0, 1.0);
        let sun_emits = s.sky.sun_enabled && s.sky.sun_emits_light;
        let moon_emits = s.sky.moon_enabled && s.sky.moon_emits_light;
        let sun_alt = (1.0 - s.sky.sun_pos[1] - s.sky.sun_z * 0.5).clamp(0.03, 1.5);
        let moon_alt = (1.0 - s.sky.moon_pos[1] - s.sky.moon_z * 0.5).clamp(0.03, 1.4);
        let mut dir_acc = 0.0f32;
        let mut alt_acc = 0.0f32;
        let mut depth_acc = 0.0f32;
        let mut w_acc = 0.0f32;
        if sun_emits {
            let w = s.sky.sun_radius.max(0.02);
            dir_acc += (s.sky.sun_pos[0] - 0.5).clamp(-1.0, 1.0) * w;
            alt_acc += sun_alt * w;
            depth_acc += s.sky.sun_z.clamp(-1.0, 1.0) * w;
            w_acc += w;
        }
        if moon_emits {
            let w = s.sky.moon_radius.max(0.02) * 0.7;
            dir_acc += (s.sky.moon_pos[0] - 0.5).clamp(-1.0, 1.0) * w;
            alt_acc += moon_alt * w;
            depth_acc += s.sky.moon_z.clamp(-1.0, 1.0) * w;
            w_acc += w;
        }
        let sky_light_dir = if w_acc > 0.0001 { (dir_acc / w_acc).clamp(-1.0, 1.0) } else { 0.0 };
        let light_altitude = if w_acc > 0.0001 { (alt_acc / w_acc).clamp(0.03, 1.5) } else { 0.55 };
        let light_depth_bias = if w_acc > 0.0001 { (depth_acc / w_acc).clamp(-1.0, 1.0) } else { 0.0 };
        let shadow_len_factor = (1.95 / light_altitude).clamp(0.55, 6.5);
        let has_emitter_global = sun_emits
            || moon_emits
            || s.atmo.layers.iter().any(|l| l.enabled && l.emits_light)
            || s.props.items.iter().any(|p| p.enabled && p.emits_light);

        // ── 1. Fill background with void / sky ─────────────────────────────
        for y in 0..ch {
            let (r, g, b) = if layers.sky {
                if s.sky.enabled && y < hy {
                    let t = y as f32 / hy as f32; // 0=top, 1=horizon
                    (lerp(s.sky.top[0] as f32, s.sky.horizon[0] as f32, t) as u8,
                     lerp(s.sky.top[1] as f32, s.sky.horizon[1] as f32, t) as u8,
                     lerp(s.sky.top[2] as f32, s.sky.horizon[2] as f32, t) as u8)
                } else {
                    (vr, vg, vb)
                }
            } else {
                (0, 0, 0)
            };
            for x in 0..cw {
                let pi = (y * cw + x) * 4;
                buf[pi] = r; buf[pi+1] = g; buf[pi+2] = b; buf[pi+3] = 255;
            }
        }
        if layers.sky && s.sky.enabled && hy > 1 {
            draw_sky_bodies(buf, cw, ch, hy, global_t, s);
        }

        // ── 2. Pre-compute path half-width for every row ───────────────────
        let mut phw_arr = vec![0.0f32; ch];
        let mut row_cx_arr = vec![cx; ch];
        for y in hy..ch {
            let t = (y - hy) as f32 / (ch - hy) as f32;
            let width_w = path_width_weight(s.scene.curve_top_weight, s.scene.curve_bottom_weight, t);
            phw_arr[y] = max_hw * width_w * (2.0*t - t*t).max(0.0).powf(pw);
            row_cx_arr[y] = cx + path_curve_x_shift(s.scene.horizon_curve, max_hw, t);
        }

        let ft = &self.floor_tile;
        let wt = &self.wall_tile;
        let floor_tex_scale = s.floor.tex_scale.max(0.05);
        let wall_tex_scale = s.walls.tex_scale.max(0.05);
        let floor_rot_90 = s.floor.tex_rot_90;
        let wall_rot_90 = s.walls.tex_rot_90;

        // ── 3. Floor (perspective-corrected tile sampling) ─────────────────
        if layers.floor {
        for y in (hy+1)..ch {
            let phw = phw_arr[y];
            let row_cx = row_cx_arr[y];
            if phw < 0.5 { continue; }
            let p_  = (y - hy) as f32;
            let d   = cam_h * focal / p_;
            let ds  = (p_ / (ch - hy) as f32 * s.floor.depth_fade).min(1.0);

            let x_min = ((row_cx - phw + 0.5) as i32).max(0) as usize;
            let x_max = ((row_cx + phw - 0.5) as i32).min(cw as i32) as usize;
            let use_curve_sampling = s.scene.horizon_curve.abs() > 0.001;

            for x in x_min..x_max {
                let d_use = if use_curve_sampling {
                    let off = world_curve_offset_for_xy(cw, ch, hy, s.scene.horizon_curve, x, y);
                    let p_adj = ((y as f32 - off) - hy as f32).max(1.0);
                    cam_h * focal / p_adj
                } else {
                    d
                };
                let wz_use = d_use + scroll * TILE as f32;
                let wx = (x as f32 - row_cx) / focal * d_use;
                let dc = (x as f32 - row_cx).abs();
                let es = ((phw - dc) / phw).max(0.0).sqrt();
                let edge_shade = ds * ((1.0 - s.floor.edge_vignette) + s.floor.edge_vignette * es);
                let depth_far = (1.0 - ((y - hy) as f32 / (ch - hy) as f32)).clamp(0.0, 1.0);
                let atmo_boost = atmo_light_energy * atmo_light_gain * depth_far.powf(0.7);
                let ambient = (s.scene.ambient * light_profile.ambient_mult + atmo_boost).clamp(0.08, 2.4);
                let sh = (edge_shade * ambient).powf(light_profile.contrast_pow).clamp(0.0, 1.6);
                let [tr, tg, tb] = sample_tile_rgb_oriented(
                    ft,
                    wx * floor_tex_scale,
                    wz_use * floor_tex_scale,
                    floor_rot_90,
                );
                let tint_a = (atmo_light_energy * atmo_tint_gain * depth_far * 0.30).clamp(0.0, 0.45);
                let tr = lerp(tr, atmo_light_tint[0], tint_a);
                let tg = lerp(tg, atmo_light_tint[1], tint_a);
                let tb = lerp(tb, atmo_light_tint[2], tint_a);
                let pi = (y * cw + x) * 4;
                buf[pi]   = (tr * sh) as u8;
                buf[pi+1] = (tg * sh) as u8;
                buf[pi+2] = (tb * sh) as u8;
            }
        }
        }

        // ── 3.5. Grass tufts at path edges ────────────────────────────────
        if layers.floor && s.scene.grass_enabled {
            let [gr, gg, gb] = s.scene.grass_color;
            let density = s.scene.grass_density.clamp(0.1, 4.0);
            let height_scale = s.scene.grass_height.clamp(0.2, 3.0);
            let upright = s.scene.grass_upright.clamp(0.0, 1.0);
            for y in (hy + 3)..ch {
                let phw = phw_arr[y];
                if phw < 1.5 { continue; }
                let p_  = (y - hy) as f32;
                let d   = cam_h * focal / p_;
                for &sgn in &[-1.0f32, 1.0] {
                    let edge_x = cx + sgn * phw;
                    // Place a tuft cluster every ~TILE world units
                    let wz_pos = d + scroll * TILE as f32;
                    let tuft_seed = inst_hash(42, (wz_pos * 0.5) as i32 + (edge_x as i32 / 4));
                    let spawn_p = density.min(1.0);
                    let r = inst_hash_f(911 + (sgn > 0.0) as u32, y as i32 + (wz_pos as i32));
                    if r > spawn_p { continue; }
                    let jit_x = ((tuft_seed & 7) as f32 - 3.5) * 0.8;
                    let tx = edge_x + jit_x;
                    let th = (focal * 0.085 / d).clamp(2.0, 16.0) * height_scale;
                    let tw = (th * 0.62).clamp(1.0, 7.5);
                    draw_grass_tuft(buf, cw, ch, tx, y as f32, tw, th, [gr, gg, gb], tuft_seed, upright);
                    if density > 1.0 {
                        let extra = ((density - 1.0) * 2.0).round() as i32;
                        for k in 0..extra {
                            let es = inst_hash(tuft_seed + 71 + k as u32, k);
                            let ex = tx + (((es & 15) as f32) - 7.5) * 0.35;
                            let eh = th * (0.75 + ((es >> 8) & 0xff) as f32 / 512.0);
                            let ew = (eh * 0.58).clamp(1.0, 7.5);
                            draw_grass_tuft(buf, cw, ch, ex, y as f32, ew, eh, [gr, gg, gb], es, upright);
                        }
                    }
                }
            }
        }

        // ── 4. Walls (lateral perspective) ────────────────────────────────
        let l_wx = s.walls.l_wx;
        if layers.walls && s.walls.enabled {
        let top_rows = (hy as f32 * s.walls.top_coverage.clamp(0.0, 1.0)).round() as usize;
        let ws = hy.saturating_sub(top_rows);

        for y in ws..ch {
            let phw  = if y >= hy { phw_arr[y] } else { 0.0 };
            let row_cx = if y >= hy { row_cx_arr[y] } else { cx };
            let below = y >= hy;
            for x in 0..cw {
                let dc = (x as f32 - row_cx).abs();
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
                    let base = (s.walls.bright / wz_w.max(0.1)).min(1.0)
                        * (0.42 + ped / s.walls.junc_shadow.max(1.0)).min(1.0);
                    let depth_far = (1.0 - ((y.saturating_sub(hy)) as f32 / (ch - hy) as f32)).clamp(0.0, 1.0);
                    let atmo_boost = atmo_light_energy * atmo_light_gain * depth_far.powf(0.65) * 0.85;
                    let mut sh = (base * (s.scene.ambient * light_profile.ambient_mult + atmo_boost).clamp(0.08, 2.2))
                        .powf(light_profile.contrast_pow)
                        .clamp(0.0, 1.7);
                    sh = sh.max(0.04);
                    let tint_a = (atmo_light_energy * atmo_tint_gain * depth_far * 0.20).clamp(0.0, 0.35);
                    let wr = lerp(wr, atmo_light_tint[0], tint_a);
                    let wg = lerp(wg, atmo_light_tint[1], tint_a);
                    let wb = lerp(wb, atmo_light_tint[2], tint_a);
                    buf[pi]   = (wr * sh + 2.0).min(255.0) as u8;
                    buf[pi+1] = (wg * sh + 2.0).min(255.0) as u8;
                    buf[pi+2] = (wb * sh + 2.0).min(255.0) as u8;
                    buf[pi+3] = 255;
                } else {
                    let ped = (dc - phw).max(0.0);
                    let base = (s.walls.bright / wz_w.max(0.1)).min(1.0)
                        * (0.42 + ped / s.walls.junc_shadow.max(1.0)).min(1.0);
                    let depth_far = (1.0 - ((y.saturating_sub(hy)) as f32 / (ch - hy) as f32)).clamp(0.0, 1.0);
                    let atmo_boost = atmo_light_energy * atmo_light_gain * depth_far.powf(0.65) * 0.85;
                    let mut sh = (base * (s.scene.ambient * light_profile.ambient_mult + atmo_boost).clamp(0.08, 2.2))
                        .powf(light_profile.contrast_pow)
                        .clamp(0.0, 1.7);
                    sh = sh.max(0.04);
                    if sh < 0.005 { continue; }
                    // Keep wall hue and brightness model consistent above/below horizon.
                    let tint_a = (atmo_light_energy * atmo_tint_gain * depth_far * 0.20).clamp(0.0, 0.35);
                    let wr = lerp(wr, atmo_light_tint[0], tint_a);
                    let wg = lerp(wg, atmo_light_tint[1], tint_a);
                    let wb = lerp(wb, atmo_light_tint[2], tint_a);
                    buf[pi]   = (wr * sh + 2.0).min(255.0) as u8;
                    buf[pi+1] = (wg * sh + 2.0).min(255.0) as u8;
                    buf[pi+2] = (wb * sh + 2.0).min(255.0) as u8;
                    buf[pi+3] = 255;
                }
            }
        }
        } // end if s.walls.enabled

        // ── 5. Atmosphere (multi-layer) ────────────────────────────────────
        if layers.atmo {
        for layer in s.atmo.layers.iter().filter(|l| l.enabled && l.atmo_type != AtmoType::None) {
            if let Some(glow_col) = layer.atmo_type.glow_color() {
                let flames = layer.atmo_type.flame_colors();
                let spc = layer.torch_spc as f32;
                let fx_scale = layer.fx_scale.max(0.2);
                let jitter = layer.placement_jitter.max(0.0);
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
                    let side_list: &[(f32, u32)] = match layer.mount_side {
                        MountSide::Both => &[(-1.0, 0), (1.0, 1)],
                        MountSide::Left => &[(-1.0, 0)],
                        MountSide::Right => &[(1.0, 1)],
                        MountSide::Center => &[(0.0, 2)],
                    };
                    for &(sgn, side_i) in side_list {
                        let jx = (inst_hash_f(layer.variation_seed + 17 + side_i, n) * 2.0 - 1.0) * jitter;
                        let jy = (inst_hash_f(layer.variation_seed + 23 + side_i, n) * 2.0 - 1.0) * jitter;
                        let path_cx = path_center_for_wz(cx, hy, ch, max_hw, focal, cam_h, s.scene.horizon_curve, wz);
                        let (sx, sy) = match layer.mount_surface {
                            AttachmentSurface::Wall => {
                                let x_off = if sgn.abs() > 0.01 {
                                    sgn * (l_wx + jx * 0.45)
                                } else {
                                    jx * 0.25
                                };
                                (
                                    path_cx + focal * x_off / wz,
                                    hy as f32 + focal * (cam_h - layer.torch_h + jy * 0.35) / wz,
                                )
                            }
                            AttachmentSurface::Floor => {
                                let x_off = if sgn.abs() > 0.01 {
                                    sgn * (l_wx * 0.62 + jx * 0.35)
                                } else {
                                    jx * 0.25
                                };
                                let floor_sy = hy as f32 + focal * cam_h / wz;
                                (
                                    path_cx + focal * x_off / wz,
                                    floor_sy - focal * layer.torch_h.max(0.0) * 0.35 / wz + jy * 0.25,
                                )
                            }
                            AttachmentSurface::Ceiling => {
                                let x_off = if sgn.abs() > 0.01 {
                                    sgn * (l_wx * 0.9 + jx * 0.35)
                                } else {
                                    jx * 0.2
                                };
                                (
                                    path_cx + focal * x_off / wz,
                                    hy as f32 - focal * layer.torch_h.max(0.0) * 0.9 / wz + jy * 0.2,
                                )
                            }
                            AttachmentSurface::Floating => {
                                let x_off = if sgn.abs() > 0.01 {
                                    sgn * (l_wx * 0.45 + jx * 0.35)
                                } else {
                                    jx * 0.2
                                };
                                (
                                    path_cx + focal * x_off / wz,
                                    hy as f32 + focal * (cam_h - layer.torch_h + jy * 0.35) / wz,
                                )
                            }
                        };
                        if sx < -50.0 || sx > cw as f32 + 50.0 { continue; }
                        if sy < -50.0 || sy > ch as f32 + 50.0 { continue; }

                        if s.post.realtime_lighting_enabled && layer.emits_light {
                            let light_cx = if matches!(layer.atmo_type, AtmoType::Torch | AtmoType::Lantern) {
                                sx - sgn * fr * 0.10
                            } else {
                                sx
                            };
                            let light_scale = match layer.atmo_type {
                                AtmoType::Torch | AtmoType::Lantern => 1.28,
                                AtmoType::Candle => 1.02,
                                AtmoType::Firefly => 0.82,
                                AtmoType::Magic | AtmoType::GreenFire | AtmoType::IceWisp => 1.12,
                                AtmoType::None => 1.0,
                            };
                            draw_local_surface_light(
                                buf,
                                cw,
                                ch,
                                light_cx,
                                sy,
                                fr * light_scale,
                                glow_col,
                                light_altitude,
                            );
                        }

                        if s.post.realtime_shadows_enabled && has_emitter_global && layer.casts_shadow {
                            draw_fixture_mount_shadow(
                                buf,
                                cw,
                                ch,
                                sx,
                                sy,
                                fr,
                                -sky_light_dir,
                                shadow_len_factor,
                                light_depth_bias,
                                hy as f32,
                            );
                        }

                        let draw_side = if sgn < 0.0 { -1.0 } else { 1.0 };
                        draw_light_fixture(buf, cw, ch, sx, sy, fr, &layer.atmo_type, draw_side);

                        if let Some(sprite_path) = pick_sprite_path(
                            &layer.sprite_path,
                            &layer.sprite_pool_paths,
                            layer.sprite_pool_enabled,
                            layer.variation_seed.wrapping_add(17),
                            n * 29 + side_i as i32,
                        ) {
                            let sp = self.get_sprite(&sprite_path).and_then(|x| x.as_ref());
                            if let Some(sp) = sp {
                                draw_sprite_billboard(
                                    buf,
                                    cw,
                                    ch,
                                    sx,
                                    sy + fr * 0.55,
                                    fr * 2.4,
                                    sp,
                                    layer.sprite_scale.max(0.05),
                                    layer.sprite_rot_deg,
                                    layer.sprite_offset_x,
                                    layer.sprite_offset_y,
                                    layer.sprite_flip_x,
                                    layer.sprite_flip_y,
                                );
                            }
                        }

                        // Glow disc (additive)
                        let gl_r = (fr * 3.8).max(3.0);
                        let gl_peak = if s.post.realtime_lighting_enabled { 0.92 } else { 0.75 };
                        draw_glow_additive(buf, cw, ch, sx, sy, gl_r, glow_col, gl_peak);

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
        }

        // ── 5.5 Props (back-to-front, continuous modular placement) ──────
        let auto_light_dir = sky_light_dir;
        if layers.props {
        for prop in s.props.items.iter().filter(|p| p.enabled) {
            let min_wz = prop.start_wz.max(0.01);
            let max_wz = prop.end_wz.max(min_wz + 0.01);
            let spc    = prop.z_spacing.max(0.1);
            let n_lo = ((scroll + min_wz) / spc).ceil() as i32;
            let n_hi = ((scroll + max_wz) / spc).floor() as i32;
            for n in (n_lo..=n_hi).rev() { // back-to-front
                let wz = n as f32 * spc - scroll + prop.pos_z;
                if wz < min_wz || wz > max_wz { continue; }

                // Seam-safe randomization key: keep per-instance style/tint/variation periodic
                // over the loop length when seamless lock is enabled.
                let n_seed = loop_seed_index(s, n, spc);

                // Per-instance seeded variation (deterministic — same seed → same variant)
                let sv   = inst_hash_f(prop.seed, n_seed);
                let sc_v = prop.scale * (1.0 + (sv * 2.0 - 1.0) * prop.scale_var);
                let wv = 1.0 + (inst_hash_f(prop.seed + 41, n_seed) * 2.0 - 1.0) * prop.width_var;
                let hv = 1.0 + (inst_hash_f(prop.seed + 43, n_seed) * 2.0 - 1.0) * prop.height_var;
                let ps_x = focal * sc_v * prop.width_scale.max(0.02) * wv.max(0.02) / wz;
                let ps_y = focal * sc_v * prop.height_scale.max(0.02) * hv.max(0.02) / wz;
                let ps   = (ps_x + ps_y) * 0.5;
                if ps < 0.8 { continue; }

                // y_sink: shift base downward so props appear grounded
                let sy_floor = hy as f32 + focal * (cam_h - prop.pos_y) / wz;
                let sink = prop.y_sink.clamp(0.0, 6.0);
                let jy = if prop.y_jitter_enabled {
                    (inst_hash_f(prop.seed + 7, n_seed) * 2.0 - 1.0) * prop.y_jitter
                } else {
                    0.0
                };
                let sy_base  = sy_floor + sink * ps_y * 0.9 + jy * ps_y;
                if sy_floor <= hy as f32 || sy_floor >= ch as f32 + ps * 8.0 { continue; }

                // Tint variation (slight colour shift per instance)
                let tv  = ((inst_hash(prop.seed + 2, n_seed) & 0x1f) as i32 - 16) as f32;
                let tint = [
                    (prop.tint[0] as f32 + tv * 0.5).clamp(0.0, 255.0) as u8,
                    (prop.tint[1] as f32 + tv * 0.8).clamp(0.0, 255.0) as u8,
                    (prop.tint[2] as f32 + tv * 0.4).clamp(0.0, 255.0) as u8,
                ];

                let wxs: &[f32] = if prop.mirror { &[prop.wx, -prop.wx] } else { &[prop.wx] };
                for &wx_v in wxs {
                    // x_jitter: small lateral offset per instance (seeded by side)
                    let jit_seed = if wx_v > 0.0 { prop.seed + 3 } else { prop.seed + 4 };
                    let jit = if prop.x_jitter_enabled {
                        (inst_hash_f(jit_seed, n_seed) * 2.0 - 1.0) * prop.x_jitter
                    } else {
                        0.0
                    };
                    let sx_world = cx + focal * (wx_v + prop.pos_x + jit) / wz;
                    let ty = ((sy_floor - hy as f32) / (ch - hy) as f32).clamp(0.0, 1.0);
                    let width_w = path_width_weight(s.scene.curve_top_weight, s.scene.curve_bottom_weight, ty);
                    let phw2 = max_hw * width_w * (2.0 * ty - ty * ty).max(0.0).powf(pw);
                    let sgn = if wx_v >= 0.0 { 1.0 } else { -1.0 };
                    let sx_edge = cx + sgn * (phw2 + prop.edge_gap.max(0.0) * ps_x) + jit * ps_x * 0.85;
                    let follow = prop.path_follow.clamp(0.0, 1.0);
                    let sx = lerp(sx_world, sx_edge, follow);
                    if sx < -(ps * 8.0) || sx > cw as f32 + ps * 8.0 { continue; }
                    let style_mix = prop.tree_style_mix.clamp(0.0, 1.0);
                    let style_bias = prop.tree_style_bias.clamp(-1.0, 1.0);
                    let draw_type = if matches!(prop.prop_type, PropType::Tree | PropType::PineTree | PropType::DeadTree)
                        && style_mix > 0.0
                    {
                        let pick = inst_hash_f(prop.seed + 55, n_seed * 3 + if wx_v > 0.0 { 1 } else { 0 });
                        if pick < style_mix {
                            let bias_pick = inst_hash_f(prop.seed + 63, n_seed * 7);
                            let pine_weight = (0.5 + 0.45 * style_bias).clamp(0.05, 0.95);
                            if bias_pick < pine_weight { PropType::PineTree } else { PropType::DeadTree }
                        } else {
                            PropType::Tree
                        }
                    } else {
                        prop.prop_type.clone()
                    };
                    let rock_var = inst_hash_f(prop.seed + 101, n_seed * 13 + if wx_v > 0.0 { 1 } else { 0 });
                    let row_count = prop.tree_row_count.max(1).min(64) as usize;
                    let row_distance_add = prop.tree_row_spacing.max(0.0);
                    let row_jitter = prop.tree_row_jitter.max(0.0)
                        + if prop.x_jitter_enabled { prop.x_jitter.max(0.0) * 0.35 } else { 0.0 };
                    let pos_scale = focal.max(0.001) / wz.max(0.001);
                    let edge_inner_px = phw2 + prop.edge_gap.max(0.0) * ps_x;
                    let base_abs = edge_inner_px / pos_scale;
                    let base_row_spacing = row_distance_add.max(0.75);
                    for row_idx in 0..row_count {
                        let row_seed = inst_hash_f(
                            prop.seed + 211 + if wx_v > 0.0 { 1 } else { 0 },
                            n_seed * 19 + row_idx as i32,
                        );
                        let row_offset = row_idx as f32 * base_row_spacing;
                        let row_jit = (row_seed * 2.0 - 1.0) * row_jitter;
                        let row_abs = (base_abs + row_offset + row_jit).max(0.0);
                        let row_wx = sgn * row_abs;
                        let row_edge_px = sgn * (row_abs - base_abs) * pos_scale;
                        let sx_world = cx + focal * (row_wx + jit) / wz;
                        let sx_edge = cx + sgn * edge_inner_px + row_edge_px + jit * pos_scale * 0.85;
                        let sx = lerp(sx_world, sx_edge, follow);
                        if sx < -(ps * 8.0) || sx > cw as f32 + ps * 8.0 { continue; }
                        let sprite_key = n_seed
                            .wrapping_mul(131)
                            .wrapping_add(row_idx as i32 * 31)
                            .wrapping_add(if wx_v > 0.0 { 17 } else { 3 });
                        if let Some(sprite_path) = pick_sprite_path(
                            &prop.sprite_path,
                            &prop.sprite_pool_paths,
                            prop.sprite_pool_enabled,
                            prop.seed.wrapping_add(313),
                            sprite_key,
                        ) {
                            let sp = self.get_sprite(&sprite_path).and_then(|x| x.as_ref());
                            if let Some(sp) = sp {
                                draw_sprite_billboard(
                                    buf,
                                    cw,
                                    ch,
                                    sx,
                                    sy_base,
                                    ps_y * 4.2,
                                    sp,
                                    prop.sprite_scale.max(0.05),
                                    prop.sprite_rot_deg,
                                    prop.sprite_offset_x,
                                    prop.sprite_offset_y,
                                    prop.sprite_flip_x,
                                    prop.sprite_flip_y,
                                );
                            } else {
                                match draw_type {
                                    PropType::Tree     => draw_tree(buf, cw, ch, sx, sy_base, ps_x, ps_y, tint),
                                    PropType::PineTree => draw_pine_tree(buf, cw, ch, sx, sy_base, ps_x, ps_y, tint),
                                    PropType::Bush     => draw_bush(buf, cw, ch, sx, sy_base, ps_x, ps_y, tint),
                                    PropType::Rock     => draw_rock(buf, cw, ch, sx, sy_base, ps_x, ps_y, tint, rock_var),
                                    PropType::Boulder  => draw_rock(buf, cw, ch, sx, sy_base, ps_x * 1.7, ps_y * 1.7, tint, rock_var),
                                    PropType::Cactus   => draw_cactus(buf, cw, ch, sx, sy_base, ps_x, ps_y, tint),
                                    PropType::DeadTree => draw_dead_tree(buf, cw, ch, sx, sy_base, ps_x, ps_y, tint),
                                    PropType::Mushroom => draw_mushroom(buf, cw, ch, sx, sy_base, ps_x, ps_y, tint),
                                }
                            }
                        } else {
                            match draw_type {
                                PropType::Tree     => draw_tree(buf, cw, ch, sx, sy_base, ps_x, ps_y, tint),
                                PropType::PineTree => draw_pine_tree(buf, cw, ch, sx, sy_base, ps_x, ps_y, tint),
                                PropType::Bush     => draw_bush(buf, cw, ch, sx, sy_base, ps_x, ps_y, tint),
                                PropType::Rock     => draw_rock(buf, cw, ch, sx, sy_base, ps_x, ps_y, tint, rock_var),
                                PropType::Boulder  => draw_rock(buf, cw, ch, sx, sy_base, ps_x * 1.7, ps_y * 1.7, tint, rock_var),
                                PropType::Cactus   => draw_cactus(buf, cw, ch, sx, sy_base, ps_x, ps_y, tint),
                                PropType::DeadTree => draw_dead_tree(buf, cw, ch, sx, sy_base, ps_x, ps_y, tint),
                                PropType::Mushroom => draw_mushroom(buf, cw, ch, sx, sy_base, ps_x, ps_y, tint),
                            }
                        }
                        let tree_shadow_mode = match draw_type {
                            PropType::Tree => 1,
                            PropType::PineTree => 2,
                            PropType::DeadTree => 3,
                            _ => 0,
                        };
                        let treeish = tree_shadow_mode != 0;
                        let root_blend = (prop.ground_blend.clamp(0.0, 1.0) * (0.24 + sink * 0.20)).clamp(0.0, 1.0);
                        if treeish && root_blend > 0.01 {
                            draw_root_contact_blend(buf, cw, ch, sx, sy_floor, ps, root_blend, hy as f32);
                        }
                        let embed = (prop.ground_blend.clamp(0.0, 1.0) * (0.35 + sink * 0.2)).clamp(0.0, 1.0);
                        if embed > 0.01 && prop.casts_shadow && has_emitter_global {
                            let cast_dir = (prop.shadow_dir - auto_light_dir * prop.shadow_follow_light.clamp(0.0, 1.0)).clamp(-2.0, 2.0);
                            let shadow_len = (prop.shadow_length * light_profile.shadow_len_mult * shadow_len_factor).clamp(0.2, 8.5);
                            let shadow_embed = (embed * light_profile.shadow_darken_mult).clamp(0.0, 1.0);
                            draw_ground_embed(
                                buf,
                                cw,
                                ch,
                                sx,
                                sy_floor,
                                ps,
                                shadow_embed,
                                prop.shadow_size,
                                shadow_len,
                                cast_dir,
                                prop.shadow_softness,
                                prop.shadow_opacity,
                                prop.pixel_hitbox_enabled,
                                tree_shadow_mode,
                                light_depth_bias,
                                hy as f32,
                            );
                        }
                    }
                }
            }
        }
        }

        // ── 6. Floor debris (per enabled layer, continuous modular) ────────
        const D_WX: [f32; 10] = [-0.22,0.28,-0.04,0.14,-0.35,0.38,0.10,-0.18,0.32,-0.08];
        if layers.atmo {
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
                    let path_cx = path_center_for_wz(cx, hy, ch, max_hw, focal, cam_h, s.scene.horizon_curve, wz);
                    let sx = path_cx + focal * (D_WX[i] + jw) / wz;
                    let sy = hy as f32 + focal * cam_h / wz;
                    if sy <= hy as f32 || sy >= ch as f32 { continue; }
                    let ty   = (sy - hy as f32) / (ch - hy) as f32;
                    let width_w = path_width_weight(s.scene.curve_top_weight, s.scene.curve_bottom_weight, ty);
                    let phw2 = max_hw * width_w * (2.0*ty - ty*ty).max(0.0).powf(pw);
                    if (sx - cx).abs() > phw2 * 0.88 { continue; }
                    let dsz = (focal * 0.018 * deb_scale / wz).clamp(0.45, 2.1);
                    let chip_seed = inst_hash(layer.variation_seed + i as u32 + 131, n);
                    draw_debris_chip(buf, cw, ch, sx, sy, dsz, chip_seed);
                }
            }
            }
        }

        // ── 7. Dust motes (per enabled layer) ─────────────────────────────
        let t2 = global_t * std::f32::consts::TAU;
        if layers.atmo {
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
        }

        // ── 8. Post-process ────────────────────────────────────────────────
        if layers.post {
        let post = &s.post;

        // Saturation: adjust before other effects so vignette stays colour-neutral
        if post.saturation_enabled && (post.saturation - 1.0).abs() > 0.02 {
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
        if post.bloom_enabled && post.bloom > 0.01 {
            let thr = (165.0 - post.bloom.clamp(0.0, 2.0) * 42.0).clamp(112.0, 190.0) as u8;
            let rad = (4.0 + post.bloom.clamp(0.0, 2.0) * 3.5).round() as i32;
            apply_bloom(buf, cw, ch, post.bloom, thr, rad.clamp(3, 10));
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
        if post.vignette_enabled && post.vignette > 0.005 {
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
        if post.grain_enabled && post.grain > 0.005 {
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
        }

        // Keep horizon curvature in all render modes (including isolated layer exports).
        if s.scene.horizon_curve.abs() > 0.001 {
            apply_world_curve(buf, cw, ch, hy, s.scene.horizon_curve);
        }

    } // end render_with_layers()

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

    fn get_sprite(&mut self, path: &str) -> Option<&Option<SpriteData>> {
        let key = path.trim();
        if key.is_empty() {
            return None;
        }
        if !self.sprite_cache.contains_key(key) {
            let loaded = (|| {
                let img = ImageReader::open(key).ok()?.decode().ok()?.to_rgba8();
                let (w, h) = img.dimensions();
                Some(SpriteData {
                    w: w as usize,
                    h: h as usize,
                    rgba: img.into_raw(),
                })
            })();
            self.sprite_cache.insert(key.to_owned(), loaded);
        }
        self.sprite_cache.get(key)
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
fn path_curve_x_shift(curve: f32, max_hw: f32, t: f32) -> f32 {
    let k = curve.clamp(-1.0, 1.0);
    let tt = t.clamp(0.0, 1.0);
    k * max_hw * 0.72 * (tt * tt)
}

#[inline]
fn path_width_weight(top_w: f32, bottom_w: f32, t: f32) -> f32 {
    lerp(top_w.clamp(0.0, 2.0), bottom_w.clamp(0.0, 2.0), t.clamp(0.0, 1.0))
}

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

fn pick_sprite_path(
    single_path: &str,
    pool_paths: &str,
    pool_enabled: bool,
    seed: u32,
    key: i32,
) -> Option<String> {
    if pool_enabled {
        let pool: Vec<&str> = pool_paths
            .split(|c| c == ';' || c == '\n' || c == '\r')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .collect();
        if !pool.is_empty() {
            let idx = (inst_hash(seed ^ 0x9E37_79B9, key) as usize) % pool.len();
            return Some(pool[idx].to_owned());
        }
    }
    let p = single_path.trim();
    if p.is_empty() { None } else { Some(p.to_owned()) }
}

struct LightingProfile {
    ambient_mult: f32,
    contrast_pow: f32,
    shadow_len_mult: f32,
    shadow_darken_mult: f32,
}

fn lighting_profile(preset: &LightingPreset) -> LightingProfile {
    match preset {
        LightingPreset::Balanced => LightingProfile {
            ambient_mult: 1.00,
            contrast_pow: 1.00,
            shadow_len_mult: 1.00,
            shadow_darken_mult: 1.00,
        },
        LightingPreset::GoldenHour => LightingProfile {
            ambient_mult: 0.92,
            contrast_pow: 1.08,
            shadow_len_mult: 1.35,
            shadow_darken_mult: 1.10,
        },
        LightingPreset::HighNoon => LightingProfile {
            ambient_mult: 1.18,
            contrast_pow: 0.95,
            shadow_len_mult: 0.70,
            shadow_darken_mult: 0.84,
        },
        LightingPreset::NightNeon => LightingProfile {
            ambient_mult: 0.72,
            contrast_pow: 1.14,
            shadow_len_mult: 1.15,
            shadow_darken_mult: 1.18,
        },
    }
}

fn estimate_atmo_lighting(s: &PathForgeSettings) -> (f32, [f32; 3]) {
    let mut energy = 0.0f32;
    let mut tint = [0.0f32, 0.0f32, 0.0f32];
    let mut tw = 0.0f32;

    for layer in s.atmo.layers.iter().filter(|l| l.enabled && l.emits_light && l.atmo_type != AtmoType::None) {
        let density = (1.0 / layer.torch_spc.max(1) as f32).sqrt().clamp(0.08, 1.0);
        let source = (layer.torch_scale * 7.0 + layer.fx_scale * 0.2 + layer.n_motes as f32 * 0.004)
            .clamp(0.0, 3.0);
        let layer_e = source * density;
        energy += layer_e;
        if let Some(c) = layer.atmo_type.glow_color() {
            tint[0] += c[0] as f32 * layer_e;
            tint[1] += c[1] as f32 * layer_e;
            tint[2] += c[2] as f32 * layer_e;
            tw += layer_e;
        }
    }

    if tw <= 0.0001 {
        tint = [160.0, 140.0, 120.0];
    } else {
        tint[0] /= tw;
        tint[1] /= tw;
        tint[2] /= tw;
    }

    (energy.clamp(0.0, 1.25), tint)
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

fn draw_debris_chip(buf: &mut [u8], w: usize, h: usize, cx: f32, cy: f32, sz: f32, seed: u32) {
    let theta = ((seed & 1023) as f32 / 1023.0) * std::f32::consts::TAU;
    let cs = theta.cos();
    let sn = theta.sin();
    let half = (sz * 1.4).max(0.4);
    let thick = (sz * 0.6).max(0.35);
    let x0 = (cx - half - thick - 2.0).floor().max(0.0) as usize;
    let x1 = (cx + half + thick + 2.0).ceil().min(w as f32) as usize;
    let y0 = (cy - half - thick - 2.0).floor().max(0.0) as usize;
    let y1 = (cy + half + thick + 2.0).ceil().min(h as f32) as usize;
    if x1 <= x0 || y1 <= y0 { return; }

    for y in y0..y1 {
        for x in x0..x1 {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let u =  cs * dx + sn * dy;
            let v = -sn * dx + cs * dy;
            let nu = (u / half).abs();
            let nv = (v / thick).abs();
            if nu > 1.0 || nv > 1.0 { continue; }
            let fall = (1.0 - nu).min(1.0 - nv).clamp(0.0, 1.0);
            let a = 0.42 * fall;
            let pi = (y * w + x) * 4;
            buf[pi]   = (buf[pi]   as f32 * (1.0 - a) + 14.0 * a) as u8;
            buf[pi+1] = (buf[pi+1] as f32 * (1.0 - a) + 11.0 * a) as u8;
            buf[pi+2] = (buf[pi+2] as f32 * (1.0 - a) +  8.0 * a) as u8;
        }
    }
}

fn apply_world_curve(buf: &mut [u8], w: usize, h: usize, hy: usize, curve: f32) {
    if w < 2 || h < 2 { return; }
    let src = buf.to_vec();
    let amp = curve.clamp(-1.0, 1.0) * (h as f32 * 0.22);
    let den = (w - 1) as f32;
    // Keep the very top sky untouched, but make sure curvature is already visible
    // at the horizon transition line itself.
    let blend_top = hy.saturating_sub(40) as f32;
    let blend_span = 60.0f32;

    for x in 0..w {
        let nx = x as f32 / den * 2.0 - 1.0;
        for y in 0..h {
            let t = ((y as f32 - blend_top) / blend_span).clamp(0.0, 1.0);
            let weight = t * t * (3.0 - 2.0 * t); // smoothstep
            let off = amp * nx * nx * weight;
            let sy = y as f32 - off;
            let di = (y * w + x) * 4;
            let syc = sy.clamp(0.0, (h - 1) as f32);
            let y0 = syc.floor() as usize;
            let y1 = (y0 + 1).min(h - 1);
            let t2 = syc - y0 as f32;
            let si0 = (y0 * w + x) * 4;
            let si1 = (y1 * w + x) * 4;
            buf[di] = lerp(src[si0] as f32, src[si1] as f32, t2) as u8;
            buf[di + 1] = lerp(src[si0 + 1] as f32, src[si1 + 1] as f32, t2) as u8;
            buf[di + 2] = lerp(src[si0 + 2] as f32, src[si1 + 2] as f32, t2) as u8;
            buf[di + 3] = 255;
        }
    }
}

fn world_curve_offset_for_xy(w: usize, h: usize, hy: usize, curve: f32, x: usize, y: usize) -> f32 {
    let amp = curve.clamp(-1.0, 1.0) * (h as f32 * 0.22);
    let den = (w - 1).max(1) as f32;
    let nx = x as f32 / den * 2.0 - 1.0;
    let blend_top = hy.saturating_sub(40) as f32;
    let blend_span = 60.0f32;
    let t = ((y as f32 - blend_top) / blend_span).clamp(0.0, 1.0);
    let weight = t * t * (3.0 - 2.0 * t);
    amp * nx * nx * weight
}

fn draw_ground_embed(
    buf: &mut [u8],
    w: usize,
    h: usize,
    sx: f32,
    sy_floor: f32,
    ps: f32,
    amount: f32,
    shadow_size: f32,
    shadow_length: f32,
    shadow_dir: f32,
    shadow_softness: f32,
    shadow_opacity: f32,
    pixel_hitbox: bool,
    tree_shadow_mode: u8,
    light_depth_bias: f32,
    ground_top_y: f32,
) {
    let a = amount.clamp(0.0, 1.0);
    let opacity = shadow_opacity.clamp(0.05, 2.5);
    let sh_size = shadow_size.clamp(0.2, 5.0);
    let sh_len = shadow_length.clamp(0.2, 10.0);
    let sh_soft = shadow_softness.clamp(0.3, 3.0);
    let cast = shadow_dir.clamp(-2.0, 2.0);
    let cast_x = cast * ps * 0.65 * sh_len;
    let depth_sign = if light_depth_bias >= 0.0 { 1.0 } else { -1.0 };
    let depth_mag = light_depth_bias.abs().clamp(0.0, 1.0);
    let y_cast_mag = if depth_sign > 0.0 {
        0.25 + depth_mag * 0.95
    } else {
        0.10 + depth_mag * 0.38
    };
    let cast_y = depth_sign * ps * y_cast_mag * sh_len;
    let spread = 1.0 + cast.abs() * 0.22 + depth_mag * 0.42 + (sh_len / 10.0) * 0.22;

    let r0x = (ps * 1.9 * sh_size * (0.9 + 0.22 * sh_soft) * spread).max(0.6);
    let r0y = (ps * 0.42 * sh_size * (0.8 + 0.36 * sh_soft) * (0.94 + depth_mag * 0.20)).max(0.4);
    let r1x = r0x * (1.45 + 0.95 * sh_len);
    let r1y = r0y * (1.05 + 0.12 * sh_len);
    let y0 = sy_floor + ps * 0.05;
    draw_ellipse_dark(buf, w, h, sx + cast_x * 0.55, y0 + cast_y, r1x, r1y);
    draw_ellipse_dark(buf, w, h, sx + cast_x * 0.35, y0 + ps * 0.04 + cast_y * 0.6, r0x, r0y);

    // Directional penumbra tail for long-cast shadows.
    let tail_steps = (3.0 + sh_len * 1.8).round().clamp(3.0, 14.0) as usize;
    for i in 0..tail_steps {
        let t = i as f32 / tail_steps.max(1) as f32;
        let tx = sx + cast_x * (0.5 + 0.9 * t);
        let ty = y0 + cast_y * (0.5 + 0.9 * t);
        let tr_x = r0x * (0.9 + sh_len * 0.45 * (1.0 - t));
        let tr_y = r0y * (0.8 + 0.22 * (1.0 - t));
        draw_ellipse_dark(buf, w, h, tx, ty, tr_x.max(0.5), tr_y.max(0.35));
    }

    let x0 = ((sx + cast_x * 0.5) as i32 - r1x as i32 - 2).clamp(0, w as i32) as usize;
    let x1 = ((sx + cast_x * 0.5) as i32 + r1x as i32 + 2).clamp(0, w as i32) as usize;
    let y_a = y0;
    let y_b = y0 + ps * (1.1 + 0.45 * sh_len) + cast_y;
    let y_start = y_a.min(y_b).max(ground_top_y).max(0.0) as usize;
    let y_end = y_a.max(y_b).min(h as f32) as usize;
    if y_end <= y_start {
        return;
    }
    for y in y_start..y_end {
        let t = ((y as f32 - y0) / (ps * 1.1).max(0.1)).clamp(0.0, 1.0);
        let blend_base = (1.0 - t).powf(1.0 / sh_soft.max(0.1)) * a * opacity * 0.42;
        for x in x0..x1 {
            let blend = if pixel_hitbox {
                let cell = (ps * 0.18).max(1.0);
                let qx = ((x as f32 - (sx + cast_x * 0.75)) / cell + 0.5).floor();
                let qy = ((y as f32 - (y0 + cast_y * 0.75 + ps * 0.10)) / cell + 0.5).floor();
                let d = (qx * qx + qy * qy).sqrt();
                let m = (1.0 - d / ((r1x / cell) * 0.95).max(1.0)).clamp(0.0, 1.0);
                blend_base.max(m * a * opacity * 0.40)
            } else {
                blend_base
            };
            let pi = (y * w + x) * 4;
            buf[pi] = (buf[pi] as f32 * (1.0 - blend)) as u8;
            buf[pi + 1] = (buf[pi + 1] as f32 * (1.0 - blend)) as u8;
            buf[pi + 2] = (buf[pi + 2] as f32 * (1.0 - blend)) as u8;
        }
    }

    // Extra narrow penumbra for tree trunks so trees cast a readable stem shadow,
    // not only a canopy blob.
    if tree_shadow_mode != 0 {
        let trunk_w = (ps * (0.33 + depth_mag * 0.08 + cast.abs() * 0.04) * sh_size).max(0.85);
        let trunk_h = (ps * 1.15).max(1.4);
        let tx0 = sx + cast_x * 0.05;
        let ty0 = sy_floor + ps * 0.01;
        let tx1 = sx + cast_x * (0.68 + depth_mag * 0.08);
        let ty1 = sy_floor + cast_y * (0.58 + depth_mag * 0.08) + ps * 0.08;
        let steps = (3.0 + sh_len * 2.8).round().clamp(4.0, 28.0) as usize;
        for i in 0..steps {
            let t = i as f32 / steps.max(1) as f32;
            let tx = tx0 + (tx1 - tx0) * t;
            let ty = ty0 + (ty1 - ty0) * t;
            let widen = 1.0 + t * (0.12 + depth_mag * 0.10 + cast.abs() * 0.04);
            let rx = trunk_w * widen;
            let ry = (trunk_w * 0.50 + trunk_h * 0.08) * (0.96 + t * 0.18);
            draw_ellipse_dark(buf, w, h, tx, ty, rx.max(0.55), ry.max(0.35));
        }
        let canopy_cx = sx + cast_x * (0.88 + depth_mag * 0.05);
        let canopy_cy = sy_floor + cast_y * (0.72 + depth_mag * 0.06) - ps * 0.06;
        match tree_shadow_mode {
            1 => {
                let canopy_rx = (ps * (1.05 + 0.22 * sh_size + 0.16 * cast.abs())).max(0.9);
                let canopy_ry = (ps * (0.72 + 0.10 * sh_size)).max(0.58);
                draw_ellipse_dark(buf, w, h, canopy_cx, canopy_cy, canopy_rx, canopy_ry);
                draw_ellipse_dark(buf, w, h, canopy_cx + cast_x * 0.10, canopy_cy - canopy_ry * 0.10, canopy_rx * 0.82, canopy_ry * 0.86);
            }
            2 => {
                let base_rx = (ps * (0.86 + 0.12 * sh_size + 0.08 * cast.abs())).max(0.72);
                let base_ry = (ps * (0.58 + 0.08 * sh_size)).max(0.50);
                draw_ellipse_dark(buf, w, h, canopy_cx, canopy_cy + base_ry * 0.18, base_rx, base_ry);
                draw_ellipse_dark(buf, w, h, canopy_cx + cast_x * 0.06, canopy_cy - base_ry * 0.42, base_rx * 0.70, base_ry * 0.78);
                draw_ellipse_dark(buf, w, h, canopy_cx + cast_x * 0.12, canopy_cy - base_ry * 0.92, base_rx * 0.46, base_ry * 0.60);
            }
            3 => {
                let dead_rx = (ps * (0.42 + 0.07 * cast.abs())).max(0.55);
                let dead_ry = (ps * 0.34).max(0.40);
                draw_ellipse_dark(buf, w, h, canopy_cx + cast_x * 0.10, canopy_cy - dead_ry * 0.25, dead_rx, dead_ry);
            }
            _ => {}
        }
    }
}

fn draw_root_contact_blend(
    buf: &mut [u8],
    w: usize,
    h: usize,
    sx: f32,
    sy_floor: f32,
    ps: f32,
    amount: f32,
    ground_top_y: f32,
) {
    let a = amount.clamp(0.0, 1.0);
    if a < 0.01 {
        return;
    }
    let cx = sx;
    let cy = (sy_floor + ps * 0.04).max(ground_top_y);
    let rx = (ps * (0.40 + a * 0.22)).max(0.75);
    let ry = (ps * (0.12 + a * 0.10)).max(0.45);
    draw_ellipse_dark(buf, w, h, cx, cy, rx, ry);
    draw_ellipse_dark(buf, w, h, cx - rx * 0.55, cy + ry * 0.28, rx * 0.52, ry * 0.72);
    draw_ellipse_dark(buf, w, h, cx + rx * 0.55, cy + ry * 0.28, rx * 0.52, ry * 0.72);

    // Smooth contour refinement for pixel-outline hitboxes: keep the silhouette crisp without
    // falling back to blocky cells.
    let x0 = (cx - rx * 1.15).floor().max(0.0) as usize;
    let x1 = (cx + rx * 1.15).ceil().min(w as f32 - 1.0) as usize;
    let y0 = (cy - ry * 0.95).floor().max(ground_top_y).max(0.0) as usize;
    let y1 = (cy + ry * 1.45).ceil().min(h as f32 - 1.0) as usize;
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = (x as f32 - cx) / rx.max(0.001);
            let dy = (y as f32 - cy) / ry.max(0.001);
            let d2 = dx * dx + dy * dy;
            if d2 >= 1.0 {
                continue;
            }
            let blend = (1.0 - d2).powf(1.65) * (0.07 + a * 0.12);
            let pi = (y * w + x) * 4;
            buf[pi] = (buf[pi] as f32 * (1.0 - blend)) as u8;
            buf[pi + 1] = (buf[pi + 1] as f32 * (1.0 - blend)) as u8;
            buf[pi + 2] = (buf[pi + 2] as f32 * (1.0 - blend)) as u8;
        }
    }
}

fn draw_local_surface_light(
    buf: &mut [u8],
    w: usize,
    h: usize,
    sx: f32,
    sy: f32,
    fr: f32,
    glow_col: [u8; 3],
    light_altitude: f32,
) {
    let alt = light_altitude.clamp(0.05, 1.5);
    let rx = (fr * (3.15 + (1.0 - alt).clamp(0.0, 1.0) * 2.05)).max(6.0);
    let ry = (fr * (1.72 + alt.clamp(0.0, 1.0) * 1.5)).max(4.0);
    let x0 = (sx - rx - 2.0).floor().max(0.0) as usize;
    let x1 = (sx + rx + 2.0).ceil().min(w as f32 - 1.0) as usize;
    let y0 = (sy - ry - 2.0).floor().max(0.0) as usize;
    let y1 = (sy + ry + 2.0).ceil().min(h as f32 - 1.0) as usize;
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = (x as f32 - sx) / rx.max(0.001);
            let dy = (y as f32 - sy) / ry.max(0.001);
            let d2 = dx * dx + dy * dy;
            if d2 >= 1.0 {
                continue;
            }
            let falloff = 1.0 / (1.0 + d2 * 3.4);
            let a = (1.0 - d2).powf(1.45) * 0.34 * falloff;
            let pi = (y * w + x) * 4;
            buf[pi] = (buf[pi] as f32 + glow_col[0] as f32 * a).clamp(0.0, 255.0) as u8;
            buf[pi + 1] = (buf[pi + 1] as f32 + glow_col[1] as f32 * a).clamp(0.0, 255.0) as u8;
            buf[pi + 2] = (buf[pi + 2] as f32 + glow_col[2] as f32 * a).clamp(0.0, 255.0) as u8;
        }
    }

    // Broad wall pool so wall-mounted torches read as lighting the surface, not just glowing in air.
    let wx = sx;
    let wy = sy + fr * 0.06;
    let wrx = (fr * 2.25).max(6.0);
    let wry = (fr * 1.04).max(3.0);
    let wx0 = (wx - wrx - 2.0).floor().max(0.0) as usize;
    let wx1 = (wx + wrx + 2.0).ceil().min(w as f32 - 1.0) as usize;
    let wy0 = (wy - wry - 2.0).floor().max(0.0) as usize;
    let wy1 = (wy + wry + 2.0).ceil().min(h as f32 - 1.0) as usize;
    for y in wy0..=wy1 {
        for x in wx0..=wx1 {
            let dx = (x as f32 - wx) / wrx.max(0.001);
            let dy = (y as f32 - wy) / wry.max(0.001);
            let d2 = dx * dx + dy * dy;
            if d2 >= 1.0 {
                continue;
            }
            let a = (1.0 - d2).powf(1.35) * 0.18;
            let pi = (y * w + x) * 4;
            buf[pi] = (buf[pi] as f32 + glow_col[0] as f32 * a).clamp(0.0, 255.0) as u8;
            buf[pi + 1] = (buf[pi + 1] as f32 + glow_col[1] as f32 * a).clamp(0.0, 255.0) as u8;
            buf[pi + 2] = (buf[pi + 2] as f32 + glow_col[2] as f32 * a).clamp(0.0, 255.0) as u8;
        }
    }

    // Bright near-source hotspot that blends into normal lighting and then dissipates.
    let hx = sx;
    let hy = sy + fr * 0.05;
    let hrx = (fr * 0.95).max(1.0);
    let hry = (fr * 0.75).max(0.8);
    let hx0 = (hx - hrx - 1.0).floor().max(0.0) as usize;
    let hx1 = (hx + hrx + 1.0).ceil().min(w as f32 - 1.0) as usize;
    let hy0 = (hy - hry - 1.0).floor().max(0.0) as usize;
    let hy1 = (hy + hry + 1.0).ceil().min(h as f32 - 1.0) as usize;
    for y in hy0..=hy1 {
        for x in hx0..=hx1 {
            let dx = (x as f32 - hx) / hrx.max(0.001);
            let dy = (y as f32 - hy) / hry.max(0.001);
            let d2 = dx * dx + dy * dy;
            if d2 >= 1.0 {
                continue;
            }
            let a = (1.0 - d2).powf(1.15) * 0.56;
            let pi = (y * w + x) * 4;
            buf[pi] = (buf[pi] as f32 + glow_col[0] as f32 * a).clamp(0.0, 255.0) as u8;
            buf[pi + 1] = (buf[pi + 1] as f32 + glow_col[1] as f32 * a).clamp(0.0, 255.0) as u8;
            buf[pi + 2] = (buf[pi + 2] as f32 + glow_col[2] as f32 * a).clamp(0.0, 255.0) as u8;
        }
    }

    // Secondary downward lobe for stronger near-wall/floor light pooling.
    let py = sy + fr * (0.52 + (1.0 - alt).clamp(0.0, 1.0) * 0.34);
    let prx = (rx * 1.16).max(6.0);
    let pry = (ry * 0.88).max(3.0);
    let px0 = (sx - prx - 2.0).floor().max(0.0) as usize;
    let px1 = (sx + prx + 2.0).ceil().min(w as f32 - 1.0) as usize;
    let py0 = (py - pry - 2.0).floor().max(0.0) as usize;
    let py1 = (py + pry + 2.0).ceil().min(h as f32 - 1.0) as usize;
    for y in py0..=py1 {
        for x in px0..=px1 {
            let dx = (x as f32 - sx) / prx.max(0.001);
            let dy = (y as f32 - py) / pry.max(0.001);
            let d2 = dx * dx + dy * dy;
            if d2 >= 1.0 {
                continue;
            }
            let a = (1.0 - d2).powf(1.38) * 0.32;
            let pi = (y * w + x) * 4;
            buf[pi] = (buf[pi] as f32 + glow_col[0] as f32 * a).clamp(0.0, 255.0) as u8;
            buf[pi + 1] = (buf[pi + 1] as f32 + glow_col[1] as f32 * a).clamp(0.0, 255.0) as u8;
            buf[pi + 2] = (buf[pi + 2] as f32 + glow_col[2] as f32 * a).clamp(0.0, 255.0) as u8;
        }
    }

    // Upper-wall lobe so wall extension above fixtures stays lit in tall-wall scenes.
    let uy = sy - fr * (0.62 + (1.0 - alt).clamp(0.0, 1.0) * 0.18);
    let urx = (rx * 0.92).max(4.0);
    let ury = (ry * 0.66).max(2.4);
    let ux0 = (sx - urx - 2.0).floor().max(0.0) as usize;
    let ux1 = (sx + urx + 2.0).ceil().min(w as f32 - 1.0) as usize;
    let uy0 = (uy - ury - 2.0).floor().max(0.0) as usize;
    let uy1 = (uy + ury + 2.0).ceil().min(h as f32 - 1.0) as usize;
    for y in uy0..=uy1 {
        for x in ux0..=ux1 {
            let dx = (x as f32 - sx) / urx.max(0.001);
            let dy = (y as f32 - uy) / ury.max(0.001);
            let d2 = dx * dx + dy * dy;
            if d2 >= 1.0 {
                continue;
            }
            let a = (1.0 - d2).powf(1.44) * 0.20;
            let pi = (y * w + x) * 4;
            buf[pi] = (buf[pi] as f32 + glow_col[0] as f32 * a).clamp(0.0, 255.0) as u8;
            buf[pi + 1] = (buf[pi + 1] as f32 + glow_col[1] as f32 * a).clamp(0.0, 255.0) as u8;
            buf[pi + 2] = (buf[pi + 2] as f32 + glow_col[2] as f32 * a).clamp(0.0, 255.0) as u8;
        }
    }
}

fn draw_fixture_mount_shadow(
    buf: &mut [u8],
    w: usize,
    h: usize,
    sx: f32,
    sy: f32,
    fr: f32,
    light_dir: f32,
    shadow_len_factor: f32,
    light_depth_bias: f32,
    ground_top_y: f32,
) {
    let cast_x = light_dir.clamp(-1.5, 1.5) * fr * 1.35 * shadow_len_factor.clamp(0.4, 6.5);
    let depth_sign = if light_depth_bias >= 0.0 { 1.0 } else { -1.0 };
    let depth_mag = light_depth_bias.abs().clamp(0.0, 1.0);
    let cast_y = depth_sign
        * fr
        * (0.12 + depth_mag * 0.55)
        * shadow_len_factor.clamp(0.4, 6.5);
    let rx = (fr * 0.90).max(1.1);
    let ry = (fr * 0.30).max(0.9);
    for i in 0..4 {
        let t = i as f32 / 3.0;
        let cx = sx + cast_x * (0.3 + t * 0.9);
        let cy = sy + fr * 0.24 + cast_y * (0.2 + t * 0.9);
        if cy < ground_top_y {
            continue;
        }
        draw_ellipse_dark(buf, w, h, cx, cy, rx * (1.0 - t * 0.35), ry * (1.0 - t * 0.22));
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
            let mx = sx - sign * fr * 0.10;
            fill_rect_any(buf, w, h, sx - sign * fr * 0.32, sy + fr * 0.12, sx - sign * fr * 0.08, sy + fr * 0.18, dark_metal);
            fill_rect_any(buf, w, h, mx - cup_w * 0.28, sy + fr * 0.12, mx + cup_w * 0.28, sy + fr * 0.24, metal);
            fill_rect_any(buf, w, h, mx - cup_w * 0.13, sy + fr * 0.24, mx + cup_w * 0.13, sy + fr * 0.24 + stem_h, post);
            let flame_cx = mx + sign * fr * 0.01;
            let flame_cy = sy + fr * 0.03;
            fill_ellipse(buf, w, h, flame_cx, flame_cy + fr * 0.08, fr * 0.30, fr * 0.46, [245, 178, 88]);
            fill_ellipse(buf, w, h, flame_cx, flame_cy - fr * 0.02, fr * 0.18, fr * 0.28, [255, 226, 160]);
            fill_ellipse(buf, w, h, flame_cx, flame_cy - fr * 0.11, fr * 0.09, fr * 0.14, [255, 244, 218]);
        }
        AtmoType::Lantern => {
            let ly = sy + fr * 0.12;
            let bar_left = sx - sign * fr * 0.58;
            let bar_right = sx - sign * fr * 0.22;
            let hook_x = (bar_left + bar_right) * 0.5;
            let lx = hook_x;
            // Wall arm and support.
            fill_rect_any(buf, w, h, bar_left - sign * fr * 0.12, ly - fr * 0.30, bar_left - sign * fr * 0.02, ly - fr * 0.06, dark_metal);
            fill_rect_any(buf, w, h, bar_left, ly - fr * 0.24, bar_right, ly - fr * 0.16, dark_metal);
            draw_line_px(buf, w, h, bar_right - sign * fr * 0.05, ly - fr * 0.20, bar_right, ly - fr * 0.06, metal);
            draw_line_px(buf, w, h, bar_left - sign * fr * 0.02, ly - fr * 0.24, hook_x, ly - fr * 0.04, metal);
            // Hanging point + chain.
            fill_ellipse(buf, w, h, hook_x, ly - fr * 0.02, fr * 0.05, fr * 0.03, metal);
            fill_rect_any(buf, w, h, hook_x - fr * 0.012, ly - fr * 0.20, hook_x + fr * 0.012, ly - fr * 0.02, metal);
            fill_rect_any(buf, w, h, hook_x - fr * 0.018, ly - fr * 0.01, hook_x + fr * 0.018, ly + fr * 0.22, metal);
            // Lantern cap/frame/glass/body.
            fill_rect_any(buf, w, h, lx - fr * 0.18, ly + fr * 0.16, lx + fr * 0.18, ly + fr * 0.24, dark_metal);
            fill_rect_any(buf, w, h, lx - fr * 0.24, ly + fr * 0.24, lx + fr * 0.24, ly + fr * 0.78, dark_metal);
            fill_rect_any(buf, w, h, lx - fr * 0.14, ly + fr * 0.30, lx + fr * 0.14, ly + fr * 0.70, [192, 146, 82]);
            fill_rect_any(buf, w, h, lx - fr * 0.18, ly + fr * 0.78, lx + fr * 0.18, ly + fr * 0.86, dark_metal);
            fill_ellipse(buf, w, h, lx, ly + fr * 0.52, fr * 0.12, fr * 0.21, warm);
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

fn draw_sky_bodies(buf: &mut [u8], w: usize, _h: usize, hy: usize, global_t: f32, s: &PathForgeSettings) {
    let seamless = s.anim.seamless_lock;
    if s.sky.stars_enabled {
        let mut rng = seeded_rng(1009u32.wrapping_add(s.sky.stars_seed));
        let count = s.sky.stars_count.min(8000) as usize;
        let tw = s.sky.stars_twinkle.max(0.0).min(8.0);
        let base_sz = s.sky.stars_size.max(0.05);
        for i in 0..count {
            let x = (rng() * w as f32) as i32;
            let y = (rng() * hy as f32) as i32;
            let freq = if seamless {
                let fi = 1 + ((inst_hash(3001u32.wrapping_add(s.sky.stars_seed), i as i32) % 9) as i32);
                fi as f32
            } else {
                1.8 + inst_hash_f(3001u32.wrapping_add(s.sky.stars_seed), i as i32) * 8.2
            };
            let phase = inst_hash_f(701u32.wrapping_add(s.sky.stars_seed), i as i32) * std::f32::consts::TAU;
            let flick = (f32::sin(global_t * std::f32::consts::TAU * freq + phase) * 0.5 + 0.5).powf(1.45);
            let pulse = (1.0 - (tw * 0.22).min(0.92)) + flick * (tw * 0.35).min(1.0);
            let b = (130.0 + 125.0 * pulse).clamp(0.0, 255.0) as u8;
            let r = (base_sz * (0.75 + rng() * 0.8)).max(0.5) as i32;
            for oy in -r..=r {
                for ox in -r..=r {
                    if ox * ox + oy * oy > r * r { continue; }
                    let px = x + ox;
                    let py = y + oy;
                    if px < 0 || py < 0 || px >= w as i32 || py >= hy as i32 { continue; }
                    let pi = (py as usize * w + px as usize) * 4;
                    buf[pi] = buf[pi].saturating_add((b as f32 * 0.22) as u8);
                    buf[pi + 1] = buf[pi + 1].saturating_add((b as f32 * 0.22) as u8);
                    buf[pi + 2] = buf[pi + 2].saturating_add((b as f32 * 0.25) as u8);
                }
            }
        }
    }

    if s.sky.clouds_enabled {
        let mut rng = seeded_rng(7001u32.wrapping_add(s.sky.cloud_seed));
        let count = s.sky.cloud_count.min(220) as usize;
        let speed = s.sky.cloud_speed.clamp(0.0, 8.0);
        let scale = s.sky.cloud_scale.clamp(0.2, 6.0);
        let alpha = s.sky.cloud_opacity.clamp(0.0, 2.0);
        for _ in 0..count {
            let base_x = rng() * w as f32;
            let y = hy as f32 * (0.10 + rng() * 0.55);
            let phase = rng();
            let span = w as f32 + 180.0;
            let drift = if seamless {
                let cycles = (speed * (0.45 + rng() * 0.9)).round().clamp(1.0, 16.0);
                global_t * span * cycles + phase * span
            } else {
                (global_t * speed * w as f32 * (0.45 + rng() * 0.9)) + phase * w as f32
            };
            let x = (base_x + drift).rem_euclid(span) - 90.0;
            let size = hy as f32 * (0.028 + rng() * 0.055) * scale;
            draw_cloud_cluster(
                buf,
                w,
                hy,
                x,
                y,
                size,
                s.sky.cloud_tint,
                alpha * (0.7 + 0.3 * rng()),
                s.sky.cloud_variation,
            );
        }
    }

    if s.sky.sun_enabled {
        let cx = s.sky.sun_pos[0] * w as f32;
        let cy = s.sky.sun_pos[1] * hy as f32;
        let r = s.sky.sun_radius * hy as f32;
        draw_glow_additive(buf, w, hy, cx, cy, (r * 2.5).max(1.0), s.sky.sun_color, 0.42);
        fill_ellipse(buf, w, hy, cx, cy, r.max(0.5), r.max(0.5), s.sky.sun_color);
    }
    if s.sky.moon_enabled {
        let cx = s.sky.moon_pos[0] * w as f32;
        let cy = s.sky.moon_pos[1] * hy as f32;
        let r = s.sky.moon_radius * hy as f32;
        draw_moon_phase(
            buf,
            w,
            hy,
            cx,
            cy,
            r.max(0.5),
            s.sky.moon_color,
            s.sky.moon_phase,
            s.sky.moon_alpha,
            s.sky.moon_texture_enabled,
            s.sky.moon_texture_scale,
        );
    }
}

fn draw_moon_phase(
    buf: &mut [u8],
    w: usize,
    h: usize,
    cx: f32,
    cy: f32,
    r: f32,
    color: [u8; 3],
    phase: f32,
    alpha: f32,
    texture_enabled: bool,
    texture_scale: f32,
) {
    let a = alpha.clamp(0.0, 2.0);
    if a <= 0.001 || r <= 0.2 { return; }
    draw_glow_additive(buf, w, h, cx, cy, (r * 2.15).max(1.0), color, 0.22 * a);

    let x0 = (cx - r - 1.0).floor().max(0.0) as usize;
    let x1 = (cx + r + 1.0).ceil().min(w as f32) as usize;
    let y0 = (cy - r - 1.0).floor().max(0.0) as usize;
    let y1 = (cy + r + 1.0).ceil().min(h as f32) as usize;
    let sx = -phase.clamp(-1.0, 1.0) * 2.0;

    for y in y0..y1 {
        for x in x0..x1 {
            let nx = (x as f32 - cx) / r;
            let ny = (y as f32 - cy) / r;
            let d_main = nx * nx + ny * ny;
            if d_main > 1.0 { continue; }
            let d_lit = (nx - sx) * (nx - sx) + ny * ny;
            let lit = (1.0 - d_lit).clamp(0.0, 1.0);
            if lit <= 0.0001 { continue; }
            let rim = (1.0 - d_main).clamp(0.0, 1.0);
            let aa = (rim * 2.5).min(1.0);
            let pa = a * lit * aa;
            let moon_col = if texture_enabled {
                moon_texture_tint(x as f32, y as f32, cx, cy, r, color, texture_scale)
            } else {
                color
            };
            blend_rgb(buf, w, x, y, moon_col, pa);
        }
    }
}

fn moon_texture_tint(x: f32, y: f32, cx: f32, cy: f32, r: f32, color: [u8; 3], scale: f32) -> [u8; 3] {
    let nx = (x - cx) / r.max(0.5);
    let ny = (y - cy) / r.max(0.5);
    let s = scale.clamp(0.2, 4.0);

    let coarse = smooth_noise2(nx * 4.2 * s, ny * 4.2 * s, 0xA53C_91E1);
    let medium = smooth_noise2(nx * 8.5 * s + 9.7, ny * 8.5 * s - 3.1, 0x91E1_A53C);
    let fine   = smooth_noise2(nx * 15.0 * s - 2.4, ny * 15.0 * s + 7.9, 0x7FEB_352D);
    let rim = (1.0 - (nx * nx + ny * ny)).clamp(0.0, 1.0);
    let mut crater = 0.88 + coarse * 0.06 + medium * 0.04 + fine * 0.02 + (rim - 0.5) * 0.015;
    crater = crater.clamp(0.55, 1.08);
    [
        (color[0] as f32 * crater).clamp(0.0, 255.0) as u8,
        (color[1] as f32 * crater).clamp(0.0, 255.0) as u8,
        (color[2] as f32 * crater).clamp(0.0, 255.0) as u8,
    ]
}

fn smooth_noise2(x: f32, y: f32, seed: u32) -> f32 {
    let x0 = x.floor() as i32;
    let y0 = y.floor() as i32;
    let xf = x - x0 as f32;
    let yf = y - y0 as f32;
    let u = xf * xf * (3.0 - 2.0 * xf);
    let v = yf * yf * (3.0 - 2.0 * yf);

    let a = hash_noise(seed, x0, y0);
    let b = hash_noise(seed, x0 + 1, y0);
    let c = hash_noise(seed, x0, y0 + 1);
    let d = hash_noise(seed, x0 + 1, y0 + 1);

    let ab = lerp(a, b, u);
    let cd = lerp(c, d, u);
    lerp(ab, cd, v) * 2.0 - 1.0
}

fn hash_noise(seed: u32, x: i32, y: i32) -> f32 {
    let mix = seed
        ^ (x as u32).wrapping_mul(374761393)
        ^ (y as u32).wrapping_mul(668265263);
    (inst_hash(mix, x.wrapping_mul(31) ^ y.wrapping_mul(17)) & 0x00FF_FFFF) as f32 / 0x00FF_FFFFu32 as f32
}

fn draw_cloud_cluster(
    buf: &mut [u8],
    w: usize,
    h: usize,
    cx: f32,
    cy: f32,
    sz: f32,
    color: [u8; 3],
    alpha: f32,
    variation: f32,
) {
    let a = alpha.clamp(0.0, 2.0);
    if a <= 0.001 || sz <= 0.2 { return; }
    let v = variation.clamp(0.0, 1.0);
    let blob_count = 3 + (v * 4.0).round() as usize;
    for i in 0..blob_count {
        let t = if blob_count <= 1 { 0.5 } else { i as f32 / (blob_count - 1) as f32 };
        let px = (t - 0.5) * (1.15 + v * 0.95) + (v * 0.25 * (t * 13.0).sin());
        let py = (0.05 * (t * 7.0).cos()) * (0.8 + 0.5 * v);
        let sx = (0.50 + 0.60 * ((t * 7.0).sin().abs())) * (0.9 + v * 0.30);
        let sy = (0.30 + 0.34 * ((t * 5.0).cos().abs())) * (0.9 + v * 0.25);
        draw_soft_ellipse(buf, w, h, cx + px * sz, cy + py * sz, sz * sx, sz * sy, color, a * (0.40 + 0.15 * v));
    }
}

fn draw_soft_ellipse(
    buf: &mut [u8],
    w: usize,
    h: usize,
    cx: f32,
    cy: f32,
    rx: f32,
    ry: f32,
    color: [u8; 3],
    alpha: f32,
) {
    if rx <= 0.1 || ry <= 0.1 || alpha <= 0.001 { return; }
    let x0 = (cx - rx - 1.0).floor().max(0.0) as usize;
    let x1 = (cx + rx + 1.0).ceil().min(w as f32) as usize;
    let y0 = (cy - ry - 1.0).floor().max(0.0) as usize;
    let y1 = (cy + ry + 1.0).ceil().min(h as f32) as usize;
    for y in y0..y1 {
        for x in x0..x1 {
            let dx = (x as f32 - cx) / rx;
            let dy = (y as f32 - cy) / ry;
            let d = dx * dx + dy * dy;
            if d >= 1.0 { continue; }
            let a = alpha * (1.0 - d).powf(1.2);
            blend_rgb(buf, w, x, y, color, a);
        }
    }
}

fn draw_grass_tuft(
    buf: &mut [u8],
    w: usize,
    h: usize,
    x: f32,
    y_base: f32,
    w_span: f32,
    h_span: f32,
    color: [u8; 3],
    seed: u32,
    upright: f32,
) {
    let blades = (3 + (seed % 5)) as usize;
    let upright_k = upright.clamp(0.0, 1.0);
    for i in 0..blades {
        let hv = inst_hash(seed + 113 + i as u32, i as i32);
        let t = (hv & 0xffff) as f32 / 65535.0;
        let side = ((hv >> 17) & 1) as f32 * 2.0 - 1.0;
        let bx = x + ((i as f32 / blades as f32) - 0.5) * w_span * (1.05 - upright_k * 0.35);
        let len = h_span * (0.55 + t * 0.9);
        let bend_base = 0.03 + ((hv >> 6) & 0xff) as f32 / 255.0 * 0.16;
        let bend = side * len * bend_base * (1.0 - upright_k * 0.82);
        let steps = len.max(2.0) as usize;
        for s in 0..steps {
            let u = s as f32 / steps as f32;
            let px = bx + bend * u * u;
            let py = y_base - len * u;
            if px < 0.0 || py < 0.0 || px >= w as f32 || py >= h as f32 { continue; }
            let shade = 0.72 + 0.35 * (1.0 - u);
            let col = [
                (color[0] as f32 * shade).clamp(0.0, 255.0) as u8,
                (color[1] as f32 * shade).clamp(0.0, 255.0) as u8,
                (color[2] as f32 * (shade * 0.88)).clamp(0.0, 255.0) as u8,
            ];
            blend_rgb(buf, w, px as usize, py as usize, col, 0.30 + 0.45 * (1.0 - u));
        }
    }
}

fn blend_rgb(buf: &mut [u8], w: usize, x: usize, y: usize, c: [u8; 3], a: f32) {
    let aa = a.clamp(0.0, 1.0);
    if aa <= 0.0 || x >= w || y >= buf.len() / 4 / w.max(1) { return; }
    let i = (y * w + x) * 4;
    let inv = 1.0 - aa;
    buf[i] = (buf[i] as f32 * inv + c[0] as f32 * aa) as u8;
    buf[i + 1] = (buf[i + 1] as f32 * inv + c[1] as f32 * aa) as u8;
    buf[i + 2] = (buf[i + 2] as f32 * inv + c[2] as f32 * aa) as u8;
}

fn path_center_for_wz(
    cx: f32,
    hy: usize,
    ch: usize,
    max_hw: f32,
    focal: f32,
    cam_h: f32,
    horizon_curve: f32,
    wz: f32,
) -> f32 {
    let sy = hy as f32 + focal * cam_h / wz.max(0.001);
    let denom = (ch - hy).max(1) as f32;
    let t = ((sy - hy as f32) / denom).clamp(0.0, 1.0);
    cx + path_curve_x_shift(horizon_curve, max_hw, t)
}

fn draw_sprite_billboard(
    buf: &mut [u8],
    w: usize,
    h: usize,
    sx: f32,
    sy_base: f32,
    ps: f32,
    sprite: &SpriteData,
    sprite_scale: f32,
    rot_deg: f32,
    offset_x: f32,
    offset_y: f32,
    flip_x: bool,
    flip_y: bool,
) {
    let sh = (ps * sprite_scale).max(1.0);
    let sw = sh * (sprite.w as f32 / sprite.h as f32).max(0.01);
    let x0 = (sx - sw * 0.5 + offset_x * ps).floor() as i32;
    let y0 = (sy_base - sh + offset_y * ps).floor() as i32;
    let x1 = (sx + sw * 0.5).ceil() as i32;
    let y1 = sy_base.ceil() as i32;
    if x1 <= x0 || y1 <= y0 { return; }
    let dw = (x1 - x0).max(1) as usize;
    let dh = (y1 - y0).max(1) as usize;

    let ang = rot_deg.to_radians();
    let cs = ang.cos();
    let sn = ang.sin();

    for dy in 0..dh {
        let py = y0 + dy as i32;
        if py < 0 || py >= h as i32 { continue; }
        for dx in 0..dw {
            let px = x0 + dx as i32;
            if px < 0 || px >= w as i32 { continue; }
            let mut u = dx as f32 / dw as f32 - 0.5;
            let mut v = dy as f32 / dh as f32 - 0.5;
            if flip_x { u = -u; }
            if flip_y { v = -v; }
            let ru = cs * u + sn * v;
            let rv = -sn * u + cs * v;
            let uu = ru + 0.5;
            let vv = rv + 0.5;
            if !(0.0..=1.0).contains(&uu) || !(0.0..=1.0).contains(&vv) { continue; }
            let sx2 = (uu * (sprite.w - 1) as f32) as usize;
            let sy = (vv * (sprite.h - 1) as f32) as usize;
            let si = (sy * sprite.w + sx2) * 4;
            let a = sprite.rgba[si + 3] as f32 / 255.0;
            if a <= 0.001 { continue; }
            let di = (py as usize * w + px as usize) * 4;
            let inv = 1.0 - a;
            buf[di] = (buf[di] as f32 * inv + sprite.rgba[si] as f32 * a) as u8;
            buf[di + 1] = (buf[di + 1] as f32 * inv + sprite.rgba[si + 1] as f32 * a) as u8;
            buf[di + 2] = (buf[di + 2] as f32 * inv + sprite.rgba[si + 2] as f32 * a) as u8;
        }
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

#[inline]
fn loop_seed_index(s: &PathForgeSettings, n: i32, spacing: f32) -> i32 {
    if !s.anim.seamless_lock {
        return n;
    }
    let loop_len = s.anim.loop_s.max(1) as f32;
    let slots = ((loop_len / spacing.max(0.001)).round() as i32).max(1);
    n.rem_euclid(slots)
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

fn fill_rect_any(buf: &mut [u8], w: usize, h: usize,
    x0: f32, y0: f32, x1: f32, y1: f32, color: [u8; 3])
{
    fill_rect_px(
        buf,
        w,
        h,
        x0.min(x1) as i32,
        y0.min(y1) as i32,
        x0.max(x1) as i32,
        y0.max(y1) as i32,
        color,
    );
}

/// Scanline-filled triangle with apex at (apex_x, apex_y) and a base of width base_w at apex_y+height.
fn fill_triangle(buf: &mut [u8], w: usize, h: usize,
    apex_x: f32, apex_y: f32, base_w: f32, height: f32, color: [u8; 3])
{
    if height < 0.5 || base_w < 0.5 { return; }
    let y_start = (apex_y as i32).max(0) as usize;
    let y_end   = (apex_y + height).clamp(0.0, h as f32) as usize;
    for y in y_start..y_end {
        let t  = (y as f32 - apex_y) / height;
        let hw = t * base_w * 0.5;
        let x_max = w.saturating_sub(1) as i32;
        let x0 = ((apex_x - hw) as i32).clamp(0, x_max) as usize;
        let x1 = ((apex_x + hw) as i32).clamp(0, x_max) as usize;
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

fn draw_tree(buf: &mut [u8], w: usize, h: usize, sx: f32, sy_base: f32, ps_x: f32, ps_y: f32, tint: [u8;3]) {
    let tw = (ps_x * 0.62).max(1.0);
    let th = ps_y * 4.4;
    let trunk_col = [
        (tint[0] as f32 * 0.32 + 20.0 * 0.68) as u8,
        (tint[1] as f32 * 0.20 + 14.0 * 0.80) as u8,
        (tint[2] as f32 * 0.10 +  8.0 * 0.90) as u8,
    ];
    fill_rect_px(buf, w, h,
        (sx - tw*0.5) as i32, (sy_base - th) as i32,
        (sx + tw*0.5) as i32, sy_base as i32, trunk_col);

    let cy_b = sy_base - th;
    fill_ellipse(buf, w, h, sx,             cy_b - ps_y * 0.55, ps_x * 2.95, ps_y * 2.25, darker(tint, 0.58));
    fill_ellipse(buf, w, h, sx - ps_x*0.30, cy_b - ps_y * 1.72, ps_x * 2.20, ps_y * 1.95, darker(tint, 0.80));
    fill_ellipse(buf, w, h, sx + ps_x*0.28, cy_b - ps_y * 1.65, ps_x * 2.05, ps_y * 1.80, tint);
    fill_ellipse(buf, w, h, sx,             cy_b - ps_y * 2.95, ps_x * 1.65, ps_y * 1.45, lighter(tint, 1.22));
}

fn draw_pine_tree(buf: &mut [u8], w: usize, h: usize, sx: f32, sy_base: f32, ps_x: f32, ps_y: f32, tint: [u8;3]) {
    let tw = (ps_x * 0.54).max(1.0);
    let th = ps_y * 2.45;
    let trunk_col = darker(tint, 0.30);
    fill_rect_px(buf, w, h,
        (sx - tw*0.5) as i32, (sy_base - th) as i32,
        (sx + tw*0.5) as i32, sy_base as i32, trunk_col);

    let base_y = sy_base - th;
    // 4 stacked tiers for fuller silhouette while keeping a straight trunk.
    let tiers: [(f32, f32, f32, f32); 3] = [
        (0.0,  3.6,  2.4, 0.64),
        (1.4,  3.0,  2.2, 0.76),
        (2.7,  2.4,  2.0, 0.90),
    ];
    for (y_off, bw, bh, cf) in tiers {
        let apex_y = base_y - y_off * ps_y - bh * ps_y;
        let col = if cf >= 1.0 { lighter(tint, cf) } else { darker(tint, cf) };
        fill_triangle(buf, w, h, sx, apex_y, bw * ps_x, bh * ps_y, col);
    }
    fill_triangle(buf, w, h, sx, base_y - 4.1 * ps_y, 1.8 * ps_x, 1.6 * ps_y, lighter(tint, 1.08));
}

fn draw_bush(buf: &mut [u8], w: usize, h: usize, sx: f32, sy_base: f32, ps_x: f32, ps_y: f32, tint: [u8;3]) {
    let rx = ps_x * 2.2;
    let ry = ps_y * 1.7;
    fill_ellipse(buf, w, h, sx - ps_x,      sy_base - ps_y*0.9, rx,       ry*0.60, darker(tint, 0.60));
    fill_ellipse(buf, w, h, sx + ps_x*0.65, sy_base - ps_y*0.7, rx*0.85,  ry*0.55, darker(tint, 0.75));
    fill_ellipse(buf, w, h, sx,             sy_base - ps_y*1.3, rx*0.90,  ry*0.68, tint);
    fill_ellipse(buf, w, h, sx - ps_x*0.3,  sy_base - ps_y*2.1, rx*0.65,  ry*0.58, lighter(tint, 1.28));
}

fn draw_rock(buf: &mut [u8], w: usize, h: usize, sx: f32, sy_base: f32, ps_x: f32, ps_y: f32, tint: [u8;3], variant: f32) {
    let vr = variant.clamp(0.0, 1.0);
    let rx = ps_x * (1.7 + 0.8 * vr);
    let ry = ps_y * (1.1 + 0.7 * (1.0 - vr));
    let cy = sy_base - ry * (0.72 + 0.12 * vr);
    let skew = (vr * 2.0 - 1.0) * ps_x * 0.45;

    fill_ellipse(buf, w, h, sx + skew * 0.3, cy + ps_y * 0.20, rx * 1.02, ry * 0.98, darker(tint, 0.52));
    fill_ellipse(buf, w, h, sx - skew * 0.2, cy - ps_y * 0.02, rx * 0.86, ry * 0.82, tint);
    fill_ellipse(buf, w, h, sx - rx * 0.36, cy - ry * 0.30, rx * 0.26, ry * 0.22, lighter(tint, 1.28));
    fill_ellipse(buf, w, h, sx + rx * 0.18, cy - ry * 0.22, rx * 0.18, ry * 0.15, lighter(tint, 1.16));

    let crack = darker(tint, 0.34);
    draw_line_px(buf, w, h, sx - rx * 0.30, cy - ry * 0.08, sx + rx * 0.12, cy + ry * 0.06, crack);
    draw_line_px(buf, w, h, sx - rx * 0.02, cy - ry * 0.22, sx + rx * 0.22, cy + ry * 0.12, crack);
    if vr > 0.45 {
        draw_line_px(buf, w, h, sx - rx * 0.20, cy + ry * 0.05, sx + rx * 0.06, cy + ry * 0.23, crack);
    }
}

fn draw_cactus(buf: &mut [u8], w: usize, h: usize, sx: f32, sy_base: f32, ps_x: f32, ps_y: f32, tint: [u8;3]) {
    let tw = (ps_x * 0.95).max(1.0);
    let th = ps_y * 5.2;
    fill_rect_px(buf, w, h,
        (sx - tw*0.5) as i32, (sy_base - th) as i32,
        (sx + tw*0.5) as i32, sy_base as i32, tint);

    // Left arm: horizontal then up
    let arm_y = sy_base - th * 0.55;
    let aw = (ps_x * 0.7).max(1.0);
    fill_rect_px(buf, w, h,
        (sx - tw*0.5 - ps_x*2.2) as i32, arm_y as i32,
        (sx - tw*0.5          ) as i32, (arm_y + aw) as i32, tint);
    fill_rect_px(buf, w, h,
        (sx - tw*0.5 - ps_x*2.2) as i32, (arm_y - ps_y*1.6) as i32,
        (sx - tw*0.5 - ps_x*2.2 + aw) as i32, arm_y as i32, tint);
    // Right arm: horizontal then up
    let arm_y2 = sy_base - th * 0.40;
    fill_rect_px(buf, w, h,
        (sx + tw*0.5          ) as i32, (arm_y2 - aw*0.5) as i32,
        (sx + tw*0.5 + ps_x*1.9 ) as i32, (arm_y2 + aw*0.5) as i32, tint);
    fill_rect_px(buf, w, h,
        (sx + tw*0.5 + ps_x*1.9 - aw) as i32, (arm_y2 - ps_y*1.8) as i32,
        (sx + tw*0.5 + ps_x*1.9      ) as i32, (arm_y2 - aw*0.5) as i32, tint);
    // Rounded top
    fill_ellipse(buf, w, h, sx, sy_base - th, tw*0.75, tw*0.75, lighter(tint, 1.2));
}

fn draw_dead_tree(buf: &mut [u8], w: usize, h: usize, sx: f32, sy_base: f32, ps_x: f32, ps_y: f32, tint: [u8;3]) {
    let tw = (ps_x * 0.68).max(1.0);
    let th = ps_y * 5.2;
    fill_rect_px(buf, w, h,
        (sx - tw*0.5) as i32, (sy_base - th) as i32,
        (sx + tw*0.5) as i32, sy_base as i32, tint);

    // Bare branches with small thickness for a less stick-like silhouette.
    let col_light = lighter(tint, 1.3);
    let branches: [(f32, f32, f32, f32); 4] = [
        (-1.2, 0.74, -1.0, 0.42),
        ( 1.5, 0.60, -0.9, 0.36),
        (-0.8, 0.46, -0.7, 0.27),
        ( 1.0, 0.34, -0.55, 0.23),
    ];
    for (dx, hf, dy, lm) in branches {
        let bx0 = sx;
        let by0 = sy_base - th * hf;
        let bx1 = sx + dx * ps_x * 4.0 * lm;
        let by1 = by0 + dy * ps_y * 4.0 * lm;
        draw_line_px(buf, w, h, bx0, by0, bx1, by1, col_light);
        draw_line_px(buf, w, h, bx0 + 1.0, by0, bx1 + 1.0, by1, darker(col_light, 0.86));
    }
}

fn draw_mushroom(buf: &mut [u8], w: usize, h: usize, sx: f32, sy_base: f32, ps_x: f32, ps_y: f32, tint: [u8;3]) {
    let sw = (ps_x * 0.58).max(1.0);
    let sh = ps_y * 2.0;
    let stem_col: [u8; 3] = [225, 215, 195];
    fill_rect_px(buf, w, h,
        (sx - sw*0.5) as i32, (sy_base - sh) as i32,
        (sx + sw*0.5) as i32, sy_base as i32, stem_col);

    let crx = ps_x * 1.95;
    let cry = ps_y * 1.25;
    let cap_cy = sy_base - sh - cry * 0.42;
    fill_ellipse(buf, w, h, sx + crx*0.10, cap_cy + cry*0.08, crx, cry, darker(tint, 0.72));
    fill_ellipse(buf, w, h, sx - crx*0.08, cap_cy - cry*0.06, crx*0.86, cry*0.80, tint);
    // White spots
    fill_ellipse(buf, w, h, sx - crx*0.22, cap_cy - cry*0.32, ps_x*0.30, ps_y*0.24, [245,245,245]);
    fill_ellipse(buf, w, h, sx + crx*0.32, cap_cy - cry*0.10, ps_x*0.22, ps_y*0.18, [245,245,245]);
    fill_ellipse(buf, w, h, sx - crx*0.50, cap_cy + cry*0.05, ps_x*0.16, ps_y*0.13, [240,240,240]);
}
