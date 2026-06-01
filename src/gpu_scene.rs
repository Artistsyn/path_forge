use bytemuck::{Pod, Zeroable};
use crate::tiles::TileKey;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SceneParams {
    dim: [u32; 4],
    time_scroll: [f32; 4],
    scene0: [f32; 4],
    sky_top: [f32; 4],
    sky_horizon: [f32; 4],
    void_color: [f32; 4],
    floor_base: [f32; 4],
    floor_mortar: [f32; 4],
    wall_base: [f32; 4],
    wall_mortar: [f32; 4],
    sun_data: [f32; 4],
    sun_color: [f32; 4],
    moon_data: [f32; 4],
    moon_color: [f32; 4],
    misc: [f32; 4],
    wall_data: [f32; 4],
    floor_data: [f32; 4],
    sky_flags: [u32; 4],
    sky_counts: [u32; 4],
    sky_misc: [f32; 4],
    cloud_tint: [f32; 4],
    moon_misc: [f32; 4],
    atmo_scene: [f32; 4],
    atmo_tint: [f32; 4],
    loop_data: [f32; 4],
    tex_data: [f32; 4],
    tex_flags: [u32; 4],
    feature_counts: [u32; 4],
    prop_core: [[f32; 4]; 8],
    prop_geom: [[f32; 4]; 8],
    prop_pos: [[f32; 4]; 8],
    prop_tint: [[f32; 4]; 8],
    prop_misc: [[f32; 4]; 8],
    prop_rows: [[f32; 4]; 8],
    prop_var: [[f32; 4]; 8],
    prop_var2: [[f32; 4]; 8],
    prop_shadow_profile0: [[f32; 4]; 8],
    prop_shadow_profile1: [[f32; 4]; 8],
    atmo_core: [[f32; 4]; 4],
    atmo_glow: [[f32; 4]; 4],
    grass_data:  [f32; 4],   // [density, height, upright, enabled(0/1)]
    grass_color: [f32; 4],   // [r, g, b, 0]
    post_data:   [f32; 4],   // [vignette, fog_density, bloom, grain]
    post_flags:  [u32; 4],   // [fog_enabled, vignette_enabled, bloom_enabled, grain_enabled]
    post_colors: [f32; 4],   // [fog_r, fog_g, fog_b, saturation]
}

pub struct GpuSceneRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    params_buffer: wgpu::Buffer,
    tile_sampler: wgpu::Sampler,
    floor_tile_cache: Option<CachedTileTexture>,
    wall_tile_cache: Option<CachedTileTexture>,
    output_cache: Option<CachedOutputResources>,
    bind_group_cache: Option<CachedBindGroup>,
}

struct CachedTileTexture {
    key: TileKey,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

struct CachedOutputResources {
    width: u32,
    height: u32,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    readback: wgpu::Buffer,
    tight_bpr: u32,
    padded_bpr: u32,
}

struct CachedBindGroup {
    width: u32,
    height: u32,
    floor_key: TileKey,
    wall_key: TileKey,
    bind_group: wgpu::BindGroup,
}

pub fn supports_exact_scene_parity(settings: &crate::settings::PathForgeSettings) -> bool {
    // Parity gate should block only known non-exact paths.
    // The renderer now supports walls/props/atmo/grass/post sufficiently for normal scenes,
    // so we keep the gate focused on sprite-instance paths (still CPU-overlay based).
    if has_sprite_instances(settings) {
        return false;
    }

    true
}

/// Returns true if any enabled prop or atmo layer uses a sprite image path.
pub fn has_sprite_instances(settings: &crate::settings::PathForgeSettings) -> bool {
    settings.props.items.iter().any(|p| {
        p.enabled
            && (!p.sprite_path.trim().is_empty()
                || (p.sprite_pool_enabled && !p.sprite_pool_paths.trim().is_empty()))
    }) || settings.atmo.layers.iter().any(|l| {
        l.enabled
            && (!l.sprite_path.trim().is_empty()
                || (l.sprite_pool_enabled && !l.sprite_pool_paths.trim().is_empty()))
    })
}

fn estimate_atmo_lighting(settings: &crate::settings::PathForgeSettings) -> (f32, [f32; 3]) {
    let mut energy = 0.0f32;
    let mut tint = [0.0f32, 0.0f32, 0.0f32];
    let mut tw = 0.0f32;

    for layer in settings
        .atmo
        .layers
        .iter()
        .filter(|l| l.enabled && l.emits_light && !matches!(l.atmo_type, crate::settings::AtmoType::None))
    {
        let density = (1.0 / layer.torch_spc.max(1) as f32).sqrt().clamp(0.08, 1.0);
        let source =
            (layer.torch_scale * 7.0 + layer.fx_scale * 0.2 + layer.n_motes as f32 * 0.004)
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

fn apply_world_curve_post(buf: &mut [u8], w: usize, h: usize, hy: usize, curve: f32) {
    if w < 2 || h < 2 {
        return;
    }
    let src = buf.to_vec();
    let amp = curve.clamp(-1.0, 1.0) * (h as f32 * 0.22);
    let den = (w - 1) as f32;
    let blend_top = hy.saturating_sub(40) as f32;
    let blend_span = 60.0f32;

    for x in 0..w {
        let nx = x as f32 / den * 2.0 - 1.0;
        for y in 0..h {
            let t = ((y as f32 - blend_top) / blend_span).clamp(0.0, 1.0);
            let weight = t * t * (3.0 - 2.0 * t);
            let off = amp * nx * nx * weight;
            let sy = y as f32 - off;
            let di = (y * w + x) * 4;
            let syc = sy.clamp(0.0, (h - 1) as f32);
            let y0 = syc.floor() as usize;
            let y1 = (y0 + 1).min(h - 1);
            let t2 = syc - y0 as f32;
            let si0 = (y0 * w + x) * 4;
            let si1 = (y1 * w + x) * 4;
            buf[di] = (src[si0] as f32 + (src[si1] as f32 - src[si0] as f32) * t2) as u8;
            buf[di + 1] =
                (src[si0 + 1] as f32 + (src[si1 + 1] as f32 - src[si0 + 1] as f32) * t2) as u8;
            buf[di + 2] =
                (src[si0 + 2] as f32 + (src[si1 + 2] as f32 - src[si0 + 2] as f32) * t2) as u8;
            buf[di + 3] = 255;
        }
    }
}

#[inline]
fn path_curve_shift(curve: f32, max_hw: f32, t: f32) -> f32 {
    let tt = t.clamp(0.0, 1.0);
    curve.clamp(-1.0, 1.0) * max_hw * 0.72 * (tt * tt)
}

#[inline]
fn path_width_weight(top_w: f32, bottom_w: f32, t: f32) -> f32 {
    let tt = t.clamp(0.0, 1.0);
    top_w.clamp(0.0, 2.0) + (bottom_w.clamp(0.0, 2.0) - top_w.clamp(0.0, 2.0)) * tt
}

fn load_sprite_rgba(path: &str) -> Option<(u32, u32, Vec<u8>)> {
    use std::collections::HashMap;
    use std::sync::Mutex;
    static CACHE: std::sync::OnceLock<Mutex<HashMap<String, Option<(u32, u32, Vec<u8>)>>>> =
        std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = cache.lock().ok()?;
    if let Some(entry) = guard.get(path) {
        return entry.clone();
    }
    let result = image::open(path).ok().map(|img| {
        let rgba = img.into_rgba8();
        let (w, h) = (rgba.width(), rgba.height());
        (w, h, rgba.into_raw())
    });
    guard.insert(path.to_owned(), result.clone());
    result
}

fn darken_pixel(buf: &mut [u8], width: u32, height: u32, x: i32, y: i32, amount: f32) {
    if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
        return;
    }
    let idx = ((y as u32 * width + x as u32) * 4) as usize;
    if idx + 2 >= buf.len() {
        return;
    }
    let a = amount.clamp(0.0, 1.0);
    buf[idx] = (buf[idx] as f32 * (1.0 - a)) as u8;
    buf[idx + 1] = (buf[idx + 1] as f32 * (1.0 - a)) as u8;
    buf[idx + 2] = (buf[idx + 2] as f32 * (1.0 - a)) as u8;
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let d = (edge1 - edge0).abs().max(0.0001);
    let t = ((x - edge0) / d).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn sprite_alpha_metrics(iw: u32, ih: u32, pixels: &[u8]) -> Option<(f32, f32, f32, f32)> {
    if iw == 0 || ih == 0 {
        return None;
    }
    let mut min_x = iw;
    let mut max_x = 0u32;
    let mut min_y = ih;
    let mut max_y = 0u32;
    let mut any = false;
    let mut top_w_sum = 0.0f32;
    let mut top_w_n = 0.0f32;
    let mut bot_w_sum = 0.0f32;
    let mut bot_w_n = 0.0f32;

    for y in 0..ih {
        let mut row_min = iw;
        let mut row_max = 0u32;
        let mut row_any = false;
        for x in 0..iw {
            let si = ((y * iw + x) * 4 + 3) as usize;
            if si >= pixels.len() || pixels[si] < 24 {
                continue;
            }
            any = true;
            row_any = true;
            min_x = min_x.min(x);
            max_x = max_x.max(x);
            min_y = min_y.min(y);
            max_y = max_y.max(y);
            row_min = row_min.min(x);
            row_max = row_max.max(x);
        }
        if !row_any {
            continue;
        }
        let row_w = (row_max - row_min + 1) as f32 / iw.max(1) as f32;
        let yn = y as f32 / ih.max(1) as f32;
        if yn < 0.35 {
            top_w_sum += row_w;
            top_w_n += 1.0;
        }
        if yn > 0.65 {
            bot_w_sum += row_w;
            bot_w_n += 1.0;
        }
    }

    if !any {
        return None;
    }

    let bbox_w = (max_x - min_x + 1) as f32 / iw.max(1) as f32;
    let bbox_h = (max_y - min_y + 1) as f32 / ih.max(1) as f32;
    let top_w = if top_w_n > 0.0 { top_w_sum / top_w_n } else { bbox_w };
    let bot_w = if bot_w_n > 0.0 { bot_w_sum / bot_w_n } else { bbox_w };
    Some((bbox_w, bbox_h, top_w, bot_w))
}

fn sprite_shadow_profile_adjusted(
    base0: [f32; 4],
    base1: [f32; 4],
    iw: u32,
    ih: u32,
    pixels: &[u8],
) -> ([f32; 4], [f32; 4]) {
    let mut p0 = base0;
    let mut p1 = base1;
    let Some((bbox_w, bbox_h, top_w, bot_w)) = sprite_alpha_metrics(iw, ih, pixels) else {
        return (p0, p1);
    };

    let top_ratio = (top_w / bbox_w.max(0.01)).clamp(0.25, 1.8);
    let bot_ratio = (bot_w / bbox_w.max(0.01)).clamp(0.25, 1.8);

    p0[0] *= (0.80 + bbox_h * 0.55).clamp(0.45, 1.6);
    p0[1] *= (0.75 + top_ratio * 0.35).clamp(0.55, 1.8);
    p0[2] *= (0.65 + (top_ratio * 0.65 + bot_ratio * 0.20)).clamp(0.45, 2.1);
    p0[3] *= (0.80 + top_ratio * 0.22).clamp(0.50, 1.8);

    p1[0] *= (0.86 + top_ratio * 0.24).clamp(0.55, 1.9);
    p1[1] *= (0.90 + (1.0 - bot_ratio.clamp(0.3, 1.2)) * 0.16).clamp(0.55, 1.4);
    p1[2] *= (0.86 + bot_ratio * 0.16).clamp(0.55, 1.8);
    (p0, p1)
}

fn draw_sprite_projected_shadow(
    buf: &mut [u8],
    width: u32,
    height: u32,
    iw: u32,
    ih: u32,
    pixels: &[u8],
    left: i32,
    full_w: f32,
    sy_floor: f32,
    cast_x: f32,
    cast_y: f32,
    strength: f32,
    ps: f32,
    profile0: [f32; 4],
    profile1: [f32; 4],
    pixel_hitbox: bool,
) {
    if iw == 0 || ih == 0 {
        return;
    }
    let cast_len = (cast_x * cast_x + cast_y * cast_y).sqrt().max(0.001);
    let steps = if ih > 96 { 12 } else { 9 };
    for i in 0..steps {
        let z = i as f32 / (steps - 1).max(1) as f32;
        let syf = ((1.0 - z) * (ih.saturating_sub(1)) as f32)
            .clamp(0.0, ih.saturating_sub(1) as f32);
        let sy = syf as u32;
        let mut min_x: Option<u32> = None;
        let mut max_x: Option<u32> = None;
        for sx in 0..iw {
            let si = ((sy * iw + sx) * 4 + 3) as usize;
            if si >= pixels.len() || pixels[si] < 24 {
                continue;
            }
            min_x = Some(min_x.map_or(sx, |v| v.min(sx)));
            max_x = Some(max_x.map_or(sx, |v| v.max(sx)));
        }
        let (Some(min_x), Some(max_x)) = (min_x, max_x) else { continue; };

        let y_shadow = sy_floor + cast_y * z;
        let x_l = left as f32 + (min_x as f32 / (iw - 1).max(1) as f32) * full_w + cast_x * z;
        let x_r = left as f32 + (max_x as f32 / (iw - 1).max(1) as f32) * full_w + cast_x * z;
        let side_base = profile0[1].max(0.2);
        let side_gain = profile0[2].max(0.1);
        let side_inflate = ps * 0.08 * (side_base + side_gain * z);
        let alpha = strength * (0.10 + z * 0.36);
        let hitbox_scale = if pixel_hitbox { 1.0 } else { 0.82 };
        let thickness = (0.35 + ps * 0.018 * (1.0 + z * 1.7) * hitbox_scale).clamp(0.5, 4.6);
        let yy0 = (y_shadow - thickness).floor() as i32;
        let yy1 = (y_shadow + thickness).ceil() as i32;
        let xx0 = (x_l - side_inflate).floor() as i32;
        let xx1 = (x_r + side_inflate).ceil() as i32;
        for yy in yy0..=yy1 {
            for xx in xx0..=xx1 {
                darken_pixel(buf, width, height, xx, yy, alpha);
            }
        }
    }

    let overhead_near = (ps * profile0[3].max(0.05)).max(0.1);
    let overhead_far = (ps * profile1[0].max(profile0[3] + 0.2)).max(overhead_near + 0.1);
    let overhead_w = 1.0 - smoothstep(overhead_near, overhead_far, cast_len);
    if overhead_w > 0.001 {
        let (bbox_w, _bbox_h, top_w, bot_w) = sprite_alpha_metrics(iw, ih, pixels)
            .unwrap_or((0.8, 0.8, 0.8, 0.7));
        let footprint_w = (ps * (0.48 + bbox_w * 0.9 + top_w * 0.35)).max(0.8);
        let footprint_h = (ps * (0.22 + bot_w * 0.45)).max(0.6);
        let cy = sy_floor + ps * 0.06;
        let x0 = (left as f32 - footprint_w).floor().max(0.0) as i32;
        let x1 = (left as f32 + full_w + footprint_w)
            .ceil()
            .min(width as f32 - 1.0) as i32;
        let y0 = (cy - footprint_h - 2.0).floor().max(0.0) as i32;
        let y1 = (cy + footprint_h + 2.0)
            .ceil()
            .min(height as f32 - 1.0) as i32;
        let cx = left as f32 + full_w * 0.5;
        let base_alpha = strength * overhead_w * 0.58;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let dx = (x as f32 - cx) / footprint_w.max(0.001);
                let dy = (y as f32 - cy) / footprint_h.max(0.001);
                let d2 = dx * dx + dy * dy;
                if d2 >= 1.0 {
                    continue;
                }
                let a = (1.0 - d2).powf(1.35) * base_alpha;
                darken_pixel(buf, width, height, x, y, a);
            }
        }
    }
}

fn add_light_pixel(buf: &mut [u8], width: u32, height: u32, x: i32, y: i32, rgb: [u8; 3], a: f32) {
    if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
        return;
    }
    let idx = ((y as u32 * width + x as u32) * 4) as usize;
    if idx + 2 >= buf.len() {
        return;
    }
    let aa = a.clamp(0.0, 1.0);
    buf[idx] = (buf[idx] as f32 + rgb[0] as f32 * aa).clamp(0.0, 255.0) as u8;
    buf[idx + 1] = (buf[idx + 1] as f32 + rgb[1] as f32 * aa).clamp(0.0, 255.0) as u8;
    buf[idx + 2] = (buf[idx + 2] as f32 + rgb[2] as f32 * aa).clamp(0.0, 255.0) as u8;
}

fn draw_surface_hotspot(
    buf: &mut [u8],
    width: u32,
    height: u32,
    cx: f32,
    cy: f32,
    rx: f32,
    ry: f32,
    rgb: [u8; 3],
    peak: f32,
) {
    let x0 = (cx - rx - 2.0).floor().max(0.0) as i32;
    let x1 = (cx + rx + 2.0).ceil().min(width as f32 - 1.0) as i32;
    let y0 = (cy - ry - 2.0).floor().max(0.0) as i32;
    let y1 = (cy + ry + 2.0).ceil().min(height as f32 - 1.0) as i32;
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = (x as f32 - cx) / rx.max(0.001);
            let dy = (y as f32 - cy) / ry.max(0.001);
            let d2 = dx * dx + dy * dy;
            if d2 >= 1.0 {
                continue;
            }
            let a = (1.0 - d2).powf(1.35) * peak * (1.0 / (1.0 + d2 * 2.4));
            add_light_pixel(buf, width, height, x, y, rgb, a);
        }
    }
}

fn draw_mount_shadow(
    buf: &mut [u8],
    width: u32,
    height: u32,
    sx: f32,
    sy: f32,
    ps: f32,
    shadow_dir: f32,
    light_depth_bias: f32,
    shadow_factor: f32,
) {
    let cast_x = shadow_dir.clamp(-1.5, 1.5) * ps * 0.9 * shadow_factor.clamp(0.4, 3.5);
    let depth_sign = if light_depth_bias >= 0.0 { 1.0 } else { -1.0 };
    let depth_mag = light_depth_bias.abs().clamp(0.0, 1.0);
    let cast_y = depth_sign * ps * (0.08 + depth_mag * 0.10) * shadow_factor.clamp(0.4, 3.5);
    for i in 0..4 {
        let t = i as f32 / 3.0;
        let cx = sx + cast_x * (0.35 + t * 0.85);
        let cy = sy + ps * 0.2 + cast_y * (0.2 + t * 0.9);
        let rx = (ps * (0.32 - t * 0.08)).max(0.8);
        let ry = (ps * (0.12 - t * 0.03)).max(0.6);
        let x0 = (cx - rx).floor() as i32;
        let x1 = (cx + rx).ceil() as i32;
        let y0 = (cy - ry).floor() as i32;
        let y1 = (cy + ry).ceil() as i32;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let dx = (x as f32 - cx) / rx.max(0.001);
                let dy = (y as f32 - cy) / ry.max(0.001);
                let d2 = dx * dx + dy * dy;
                if d2 >= 1.0 {
                    continue;
                }
                let a = (1.0 - d2).powf(1.4) * (0.22 - t * 0.05);
                darken_pixel(buf, width, height, x, y, a);
            }
        }
    }
}

/// CPU sprite overlay: renders sprite-backed prop/atmo instances on top of a GPU scene buffer.
/// Call this after render_scene_rgba when has_sprite_instances() is true.
pub fn composite_sprite_overlay(
    buf: &mut Vec<u8>,
    width: u32,
    height: u32,
    settings: &crate::settings::PathForgeSettings,
    scroll: f32,
) {
    let w = width as f32;
    let h = height as f32;
    let hy_scale = h / 768.0;
    let hw_scale = w / 576.0;
    let horizon = (settings.scene.horizon_y as f32 * hy_scale)
        .round()
        .clamp(8.0, h - 8.0);
    let focal = (h - horizon) * settings.scene.focal_mult;
    let cam_h = settings.scene.cam_h;
    let max_hw = settings.scene.max_hw * hw_scale;
    let path_power = settings.scene.path_power.max(0.05);
    let cx = w * 0.5;
    let curve = settings.scene.horizon_curve.clamp(-1.0, 1.0);
    let rt_shadows = settings.post.realtime_shadows_enabled;
    let sun_emit = settings.sky.sun_enabled && settings.sky.sun_emits_light;
    let moon_emit = settings.sky.moon_enabled && settings.sky.moon_emits_light;
    let atmo_emit = settings.atmo.layers.iter().any(|l| l.enabled && l.emits_light);
    let prop_emit = settings.props.items.iter().any(|p| p.enabled && p.emits_light);
    let has_emitter = sun_emit || moon_emit || atmo_emit || prop_emit;
    let mut light_dir = 0.0f32;
    let mut light_alt = 0.55f32;
    let mut dir_acc = 0.0f32;
    let mut alt_acc = 0.0f32;
    let mut depth_acc = 0.0f32;
    let mut w_acc = 0.0f32;
    let mut light_depth = 0.0f32;
    if sun_emit {
        let w = settings.sky.sun_radius.max(0.02);
        dir_acc += (settings.sky.sun_pos[0] - 0.5).clamp(-1.0, 1.0) * w;
        alt_acc += (1.0 - settings.sky.sun_pos[1] - settings.sky.sun_z * 0.5).clamp(0.03, 1.5) * w;
        depth_acc += settings.sky.sun_z.clamp(-1.0, 1.0) * w;
        w_acc += w;
    }
    if moon_emit {
        let w = settings.sky.moon_radius.max(0.02) * 0.7;
        dir_acc += (settings.sky.moon_pos[0] - 0.5).clamp(-1.0, 1.0) * w;
        alt_acc += (1.0 - settings.sky.moon_pos[1] - settings.sky.moon_z * 0.5).clamp(0.03, 1.4) * w;
        depth_acc += settings.sky.moon_z.clamp(-1.0, 1.0) * w;
        w_acc += w;
    }
    if w_acc > 0.0001 {
        light_dir = (dir_acc / w_acc).clamp(-1.0, 1.0);
        light_alt = (alt_acc / w_acc).clamp(0.03, 1.5);
        light_depth = (depth_acc / w_acc).clamp(-1.0, 1.0);
    }
    let light_shadow_dir = -light_dir;

    // Sprite-backed props
    for p in settings.props.items.iter().filter(|p| p.enabled) {
        let sprite_path = p.sprite_path.trim().to_owned();
        let pool_paths: Vec<String> = if p.sprite_pool_enabled {
            p.sprite_pool_paths
                .split('\n')
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
                .collect()
        } else {
            vec![]
        };
        let effective_path = if !sprite_path.is_empty() {
            Some(sprite_path)
        } else if !pool_paths.is_empty() {
            Some(pool_paths[0].clone())
        } else {
            None
        };
        let Some(path) = effective_path else { continue; };
        let Some((iw, ih, pixels)) = load_sprite_rgba(&path) else { continue; };
        if iw == 0 || ih == 0 { continue; }
        let tcode = prop_type_code(&p.prop_type);
        let (base0, base1) = prop_shadow_profile_defaults(tcode);
        let (shadow_profile0, shadow_profile1) =
            sprite_shadow_profile_adjusted(base0, base1, iw, ih, &pixels);

        let spc = p.z_spacing.max(0.1);
        let wz_near = p.start_wz.max(0.01);
        let wz_far = p.end_wz.max(wz_near + 0.01);
        let ss = p.sprite_scale.max(0.01);
        let shadow_follow_light = p.shadow_follow_light.clamp(0.0, 1.0);
        let manual_shadow_dir = p.shadow_dir.clamp(-2.0, 2.0);
        let final_shadow_dir = if manual_shadow_dir.abs() > 0.001 {
            manual_shadow_dir * (1.0 - shadow_follow_light) + light_shadow_dir * shadow_follow_light
        } else {
            light_shadow_dir
        };
        let shadow_size = p.shadow_size.clamp(0.2, 4.0);
        let shadow_softness = p.shadow_softness.clamp(0.3, 3.0);
        let shadow_opacity = p.shadow_opacity.clamp(0.1, 1.5);
        let shadow_length = p.shadow_length.clamp(0.2, 5.0);

        let n_min = (scroll + wz_near) / spc;
        let n_max = (scroll + wz_far) / spc;
        let n_start = n_min.floor() as i32 - 1;
        let n_end = n_max.ceil() as i32 + 1;

        for n in n_start..=n_end {
            let wz = n as f32 * spc - scroll + p.pos_z;
            if wz < wz_near || wz > wz_far || wz < 0.001 { continue; }

            let ps = focal * p.scale * ss / wz;
            if ps < 1.0 { continue; }

            let sy_floor = horizon + focal * (cam_h - p.pos_y) / wz;
            let fy = sy_floor - horizon;
            let path_t = (fy / (h - horizon).max(1.0)).clamp(0.0, 1.0);
            let width_w = path_width_weight(settings.scene.curve_top_weight, settings.scene.curve_bottom_weight, path_t);
            let row_phw = max_hw
                * width_w
                * (2.0 * path_t - path_t * path_t)
                    .max(0.0)
                    .powf(path_power);
            let row_cx2 = cx;

            let sprite_h = (ps * p.height_scale * 2.0).max(1.0);
            let sprite_w = sprite_h * (iw as f32 / ih as f32) * p.width_scale;

            let mirrors: &[f32] = if p.mirror { &[-1.0, 1.0] } else { &[1.0] };
            for &sgn in mirrors {
                let sx_world = row_cx2 + focal * (p.wx * sgn + p.pos_x) / wz;
                let sx_edge = row_cx2 + sgn * (row_phw + p.edge_gap * ps);
                let sx = sx_world * (1.0 - p.path_follow) + sx_edge * p.path_follow;

                let left = (sx - sprite_w * 0.5) as i32;
                let right = (sx + sprite_w * 0.5) as i32;
                let top = (sy_floor - sprite_h) as i32;
                let bot = (sy_floor + ps * 0.2) as i32;

                let x0 = left.max(0) as u32;
                let x1 = right.min(width as i32 - 1).max(0) as u32;
                let y0 = top.max(0) as u32;
                let y1 = bot.min(height as i32 - 1).max(0) as u32;

                let full_w = (right - left).max(1) as f32;
                let full_h = (bot - top).max(1) as f32;

                if rt_shadows && has_emitter && p.casts_shadow {
                    let cast_len = (0.45 + (1.0 - light_alt).clamp(0.0, 1.2) * 1.95).clamp(0.35, 3.2);
                    let effective_ps = ps * shadow_size;
                    let cast_x = final_shadow_dir * effective_ps * cast_len * shadow_length;
                    let depth_sign = if light_depth >= 0.0 { 1.0 } else { -1.0 };
                    let depth_mag = light_depth.abs().clamp(0.0, 1.0);
                    let cast_y = depth_sign * effective_ps * (0.06 + depth_mag * 0.08) * cast_len;
                    let shadow_strength = (0.08 + shadow_opacity * 0.32)
                        * p.ground_blend.clamp(0.0, 1.0)
                        * (1.1 / shadow_softness.max(0.35));
                    draw_sprite_projected_shadow(
                        buf,
                        width,
                        height,
                        iw,
                        ih,
                        &pixels,
                        left,
                        full_w,
                        sy_floor,
                        cast_x,
                        cast_y,
                        if p.pixel_hitbox_enabled { shadow_strength } else { shadow_strength * 0.78 },
                        effective_ps,
                        shadow_profile0,
                        shadow_profile1,
                        p.pixel_hitbox_enabled,
                    );
                }

                for dy in y0..=y1 {
                    for dx in x0..=x1 {
                        let mut u =
                            (dx as f32 - left as f32) / full_w;
                        let mut v =
                            (dy as f32 - top as f32) / full_h;
                        if p.sprite_flip_x { u = 1.0 - u; }
                        if p.sprite_flip_y { v = 1.0 - v; }
                        let px = ((u * (iw - 1) as f32) as u32).min(iw - 1);
                        let py = ((v * (ih - 1) as f32) as u32).min(ih - 1);
                        let si = ((py * iw + px) * 4) as usize;
                        if si + 3 >= pixels.len() { continue; }
                        let sa = pixels[si + 3];
                        if sa == 0 { continue; }
                        let dst = (dy * width + dx) as usize * 4;
                        if dst + 3 >= buf.len() { continue; }
                        let a = sa as f32 / 255.0;
                        let ia = 1.0 - a;
                        buf[dst]     = (pixels[si]     as f32 * a + buf[dst]     as f32 * ia) as u8;
                        buf[dst + 1] = (pixels[si + 1] as f32 * a + buf[dst + 1] as f32 * ia) as u8;
                        buf[dst + 2] = (pixels[si + 2] as f32 * a + buf[dst + 2] as f32 * ia) as u8;
                    }
                }
            }
        }
    }

    // Sprite-backed atmo layers (billboard sprite at torch position)
    for l in settings.atmo.layers.iter().filter(|l| l.enabled) {
        let sprite_path = l.sprite_path.trim().to_owned();
        let pool_paths: Vec<String> = if l.sprite_pool_enabled {
            l.sprite_pool_paths
                .split('\n')
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
                .collect()
        } else {
            vec![]
        };
        let effective_path = if !sprite_path.is_empty() {
            Some(sprite_path)
        } else if !pool_paths.is_empty() {
            Some(pool_paths[0].clone())
        } else {
            None
        };
        let Some(path) = effective_path else { continue; };
        let Some((iw, ih, pixels)) = load_sprite_rgba(&path) else { continue; };
        if iw == 0 || ih == 0 { continue; }

        let spc = (l.torch_spc.max(1)) as f32;
        let torch_h = l.torch_h;
        let sc = l.torch_scale.max(0.01);

        let n_min = scroll / spc;
        let n_max = (scroll + 24.0) / spc;
        let n_start = n_min.floor() as i32 - 1;
        let n_end = n_max.ceil() as i32 + 1;

        for n in n_start..=n_end {
            let wz = n as f32 * spc - scroll;
            if wz < 0.12 || wz > 24.0 { continue; }

            let ps = focal * sc / wz;
            if ps < 0.45 { continue; }

            let sy = horizon + focal * (cam_h - torch_h) / wz;
            let fy = sy - horizon;
            let path_t = (fy / (h - horizon).max(1.0)).clamp(0.0, 1.0);
            let width_w = path_width_weight(settings.scene.curve_top_weight, settings.scene.curve_bottom_weight, path_t);
            let row_phw = max_hw
                * width_w
                * (2.0 * path_t - path_t * path_t)
                    .max(0.0)
                    .powf(path_power);
            let row_cx2 = cx + path_curve_shift(curve, max_hw, path_t);

            let sprite_h = (ps * 2.0).max(1.0);
            let sprite_w = sprite_h * (iw as f32 / ih as f32);
            let side_x = row_phw + settings.walls.l_wx * 0.42;

            let side_list: &[(f32, u32)] = match l.mount_side {
                crate::settings::MountSide::Both => &[(-1.0, 0), (1.0, 1)],
                crate::settings::MountSide::Left => &[(-1.0, 0)],
                crate::settings::MountSide::Right => &[(1.0, 1)],
                crate::settings::MountSide::Center => &[(0.0, 2)],
            };

            for &(sgn, _side_i) in side_list {
                let (sx, sy_mount) = match l.mount_surface {
                    crate::settings::AttachmentSurface::Wall => {
                        let sx = if sgn.abs() > 0.01 { row_cx2 + sgn * side_x } else { row_cx2 };
                        (sx, sy)
                    }
                    crate::settings::AttachmentSurface::Floor => {
                        let sx = if sgn.abs() > 0.01 { row_cx2 + sgn * side_x * 0.62 } else { row_cx2 };
                        let floor_sy = horizon + focal * cam_h / wz;
                        (sx, floor_sy - focal * torch_h.max(0.0) * 0.35 / wz)
                    }
                    crate::settings::AttachmentSurface::Ceiling => {
                        let sx = if sgn.abs() > 0.01 { row_cx2 + sgn * side_x * 0.9 } else { row_cx2 };
                        (sx, horizon - focal * torch_h.max(0.0) * 0.9 / wz)
                    }
                    crate::settings::AttachmentSurface::Floating => {
                        let sx = if sgn.abs() > 0.01 { row_cx2 + sgn * side_x * 0.45 } else { row_cx2 };
                        (sx, sy)
                    }
                };
                if settings.post.realtime_lighting_enabled && l.emits_light {
                    let glow_rgb = l.atmo_type.glow_color().unwrap_or([220, 180, 110]);
                    let light_scale = match l.atmo_type {
                        crate::settings::AtmoType::Torch | crate::settings::AtmoType::Lantern => 1.28,
                        crate::settings::AtmoType::Candle => 1.02,
                        crate::settings::AtmoType::Firefly => 0.82,
                        crate::settings::AtmoType::Magic
                        | crate::settings::AtmoType::GreenFire
                        | crate::settings::AtmoType::IceWisp => 1.12,
                        crate::settings::AtmoType::None => 1.0,
                    };
                    let hotspot_rx =
                        (ps * light_scale * (1.8 + (1.0 - light_alt).clamp(0.0, 1.0) * 1.2)).max(2.0);
                    let hotspot_ry =
                        (ps * light_scale * (0.9 + light_alt.clamp(0.0, 1.0) * 0.95)).max(1.2);
                    draw_surface_hotspot(
                        buf,
                        width,
                        height,
                        sx,
                        sy_mount + ps * 0.18,
                        hotspot_rx,
                        hotspot_ry,
                        glow_rgb,
                        0.48,
                    );
                    draw_surface_hotspot(
                        buf,
                        width,
                        height,
                        sx,
                        sy_mount + ps * 0.44,
                        hotspot_rx * 1.24,
                        hotspot_ry * 0.92,
                        glow_rgb,
                        0.26,
                    );
                }
                if settings.post.realtime_shadows_enabled && has_emitter && l.casts_shadow {
                    let shadow_factor = (1.95 / light_alt.max(0.03)).clamp(0.55, 6.5);
                    draw_mount_shadow(buf, width, height, sx, sy_mount, ps, light_shadow_dir, light_depth, shadow_factor);
                }
                let left = (sx - sprite_w * 0.5) as i32;
                let right = (sx + sprite_w * 0.5) as i32;
                let top = (sy_mount - sprite_h * 0.5) as i32;
                let bot = (sy_mount + sprite_h * 0.5) as i32;

                let x0 = left.max(0) as u32;
                let x1 = right.min(width as i32 - 1).max(0) as u32;
                let y0 = top.max(0) as u32;
                let y1 = bot.min(height as i32 - 1).max(0) as u32;

                let full_w = (right - left).max(1) as f32;
                let full_h = (bot - top).max(1) as f32;

                for dy in y0..=y1 {
                    for dx in x0..=x1 {
                        let u = (dx as f32 - left as f32) / full_w;
                        let v = (dy as f32 - top as f32) / full_h;
                        let px = ((u * (iw - 1) as f32) as u32).min(iw - 1);
                        let py = ((v * (ih - 1) as f32) as u32).min(ih - 1);
                        let si = ((py * iw + px) * 4) as usize;
                        if si + 3 >= pixels.len() { continue; }
                        let sa = pixels[si + 3];
                        if sa == 0 { continue; }
                        let dst = (dy * width + dx) as usize * 4;
                        if dst + 3 >= buf.len() { continue; }
                        let a = sa as f32 / 255.0;
                        let ia = 1.0 - a;
                        buf[dst]     = (pixels[si]     as f32 * a + buf[dst]     as f32 * ia) as u8;
                        buf[dst + 1] = (pixels[si + 1] as f32 * a + buf[dst + 1] as f32 * ia) as u8;
                        buf[dst + 2] = (pixels[si + 2] as f32 * a + buf[dst + 2] as f32 * ia) as u8;
                    }
                }
            }
        }
    }
}

fn prop_type_code(t: &crate::settings::PropType) -> f32 {
    match t {
        crate::settings::PropType::Tree => 0.0,
        crate::settings::PropType::PineTree => 1.0,
        crate::settings::PropType::Bush => 2.0,
        crate::settings::PropType::Rock => 3.0,
        crate::settings::PropType::Boulder => 4.0,
        crate::settings::PropType::Cactus => 5.0,
        crate::settings::PropType::DeadTree => 6.0,
        crate::settings::PropType::Mushroom => 7.0,
    }
}

fn prop_shadow_profile_defaults(tcode: f32) -> ([f32; 4], [f32; 4]) {
    if tcode < 0.5 {
        // Broadleaf tree
        return ([6.4, 2.2, 2.4, 0.34], [1.58, 0.20, 0.82, 0.0]);
    }
    if tcode < 1.5 {
        // Pine tree
        return ([4.8, 2.1, 2.2, 0.30], [1.45, 0.18, 0.80, 0.0]);
    }
    if tcode < 2.5 {
        // Bush
        return ([3.9, 2.0, 2.0, 0.30], [1.40, 0.16, 0.78, 0.0]);
    }
    if tcode < 4.5 {
        // Rock / boulder
        return ([2.4, 1.9, 1.8, 0.26], [1.20, 0.12, 0.74, 0.0]);
    }
    if tcode < 5.5 {
        // Cactus
        return ([5.0, 1.7, 2.1, 0.28], [1.36, 0.14, 0.80, 0.0]);
    }
    if tcode < 6.5 {
        // Dead tree
        return ([8.4, 1.9, 2.7, 0.32], [1.70, 0.22, 0.88, 0.0]);
    }
    // Mushroom / fallback
    ([3.1, 1.8, 1.9, 0.26], [1.18, 0.12, 0.70, 0.0])
}

fn atmo_type_code(t: &crate::settings::AtmoType) -> f32 {
    match t {
        crate::settings::AtmoType::None => 0.0,
        crate::settings::AtmoType::Torch => 1.0,
        crate::settings::AtmoType::Lantern => 2.0,
        crate::settings::AtmoType::Firefly => 3.0,
        crate::settings::AtmoType::Magic => 4.0,
        crate::settings::AtmoType::GreenFire => 5.0,
        crate::settings::AtmoType::Candle => 6.0,
        crate::settings::AtmoType::IceWisp => 7.0,
    }
}

fn aligned_bytes_per_row(width: u32) -> u32 {
    let raw = width.saturating_mul(4);
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    ((raw + align - 1) / align) * align
}

fn rgb_tile_to_rgba(tile: &[u8]) -> (u32, Vec<u8>) {
    let tex_px = (tile.len() / 3).max(1);
    let side = (tex_px as f32).sqrt() as u32;
    let mut rgba = vec![0u8; (side * side * 4) as usize];
    for i in 0..(side * side) as usize {
        let src = i * 3;
        let dst = i * 4;
        rgba[dst] = tile[src];
        rgba[dst + 1] = tile[src + 1];
        rgba[dst + 2] = tile[src + 2];
        rgba[dst + 3] = 255;
    }
    (side, rgba)
}

fn create_tile_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    rgba: &[u8],
    side: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: side,
            height: side,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::ImageCopyTexture {
            texture: &tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        rgba,
        wgpu::ImageDataLayout {
            offset: 0,
            bytes_per_row: Some(side * 4),
            rows_per_image: Some(side),
        },
        wgpu::Extent3d {
            width: side,
            height: side,
            depth_or_array_layers: 1,
        },
    );
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}

impl GpuSceneRenderer {
    pub fn new() -> Result<Self, String> {
        pollster::block_on(Self::new_async())
    }

    async fn new_async() -> Result<Self, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            dx12_shader_compiler: Default::default(),
            ..Default::default()
        });
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await
            .ok_or_else(|| "No GPU adapter available".to_owned())?;

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("path_forge_gpu_scene_device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    memory_hints: wgpu::MemoryHints::Performance,
                },
                None,
            )
            .await
            .map_err(|e| format!("request_device failed: {e}"))?;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("path_forge_gpu_scene_shader"),
            source: wgpu::ShaderSource::Wgsl(SCENE_SHADER.into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("path_forge_gpu_scene_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("path_forge_gpu_scene_pl"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("path_forge_gpu_scene_pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: "main",
            compilation_options: Default::default(),
            cache: None,
        });

        let tile_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("path_forge_gpu_scene_tile_sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("path_forge_gpu_scene_params"),
            size: std::mem::size_of::<SceneParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Ok(Self {
            device,
            queue,
            pipeline,
            bind_group_layout,
            params_buffer,
            tile_sampler,
            floor_tile_cache: None,
            wall_tile_cache: None,
            output_cache: None,
            bind_group_cache: None,
        })
    }

    fn floor_key(settings: &crate::settings::PathForgeSettings) -> TileKey {
        TileKey {
            pattern: settings.floor.pattern.name().to_owned(),
            base: settings.floor.base,
            mortar: settings.floor.mortar,
            noise: settings.floor.noise + (settings.floor.damage.clamp(0.0, 1.0) * 20.0) as u32,
            seed: 11 + settings.floor.pattern.gen_seed_offset() + settings.floor.variation_seed,
        }
    }

    fn wall_key(settings: &crate::settings::PathForgeSettings) -> TileKey {
        TileKey {
            pattern: settings.walls.pattern.name().to_owned(),
            base: settings.walls.base,
            mortar: settings.walls.mortar,
            noise: settings.walls.noise + (settings.walls.damage.clamp(0.0, 1.0) * 20.0) as u32,
            seed: 22 + settings.walls.pattern.gen_seed_offset() + settings.walls.variation_seed,
        }
    }

    fn ensure_tile_textures(&mut self, settings: &crate::settings::PathForgeSettings) {
        let floor_key = Self::floor_key(settings);
        let floor_dirty = self
            .floor_tile_cache
            .as_ref()
            .map(|c| c.key != floor_key)
            .unwrap_or(true);
        if floor_dirty {
            let floor_tile = crate::tiles::gen_floor_tile(
                &settings.floor.pattern,
                settings.floor.base,
                settings.floor.mortar,
                settings.floor.noise,
                settings.floor.damage,
                floor_key.seed,
            );
            let (floor_side, floor_rgba) = rgb_tile_to_rgba(&floor_tile);
            let (texture, view) = create_tile_texture(
                &self.device,
                &self.queue,
                "path_forge_gpu_scene_floor_tile",
                &floor_rgba,
                floor_side,
            );
            self.floor_tile_cache = Some(CachedTileTexture {
                key: floor_key,
                texture,
                view,
            });
        }

        let wall_key = Self::wall_key(settings);
        let wall_dirty = self
            .wall_tile_cache
            .as_ref()
            .map(|c| c.key != wall_key)
            .unwrap_or(true);
        if wall_dirty {
            let wall_tile = crate::tiles::gen_wall_tile(
                &settings.walls.pattern,
                settings.walls.base,
                settings.walls.mortar,
                settings.walls.noise,
                settings.walls.damage,
                wall_key.seed,
            );
            let (wall_side, wall_rgba) = rgb_tile_to_rgba(&wall_tile);
            let (texture, view) = create_tile_texture(
                &self.device,
                &self.queue,
                "path_forge_gpu_scene_wall_tile",
                &wall_rgba,
                wall_side,
            );
            self.wall_tile_cache = Some(CachedTileTexture {
                key: wall_key,
                texture,
                view,
            });
        }
    }

    fn ensure_output_resources(&mut self, width: u32, height: u32) {
        let reuse = self
            .output_cache
            .as_ref()
            .map(|c| c.width == width && c.height == height)
            .unwrap_or(false);
        if reuse {
            return;
        }

        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("path_forge_gpu_scene_out"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let tight_bpr = width.saturating_mul(4);
        let padded_bpr = aligned_bytes_per_row(width);
        let out_size = (padded_bpr as u64).saturating_mul(height as u64);
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("path_forge_gpu_scene_readback"),
            size: out_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        self.output_cache = Some(CachedOutputResources {
            width,
            height,
            texture,
            view,
            readback,
            tight_bpr,
            padded_bpr,
        });
    }

    pub fn render_scene_rgba(
        &mut self,
        settings: &crate::settings::PathForgeSettings,
        scroll: f32,
        global_t: f32,
    ) -> Result<Vec<u8>, String> {
        let width = settings.canvas.w() as u32;
        let height = settings.canvas.h() as u32;
        if width == 0 || height == 0 {
            return Err("Invalid size".to_owned());
        }

        let hy_scale = height as f32 / 768.0;
        let hw_scale = width as f32 / 576.0;
        let horizon = ((settings.scene.horizon_y as f32) * hy_scale)
            .round()
            .clamp(8.0, (height.saturating_sub(8)) as f32);

        let mut prop_core = [[0.0f32; 4]; 8];
        let mut prop_geom = [[0.0f32; 4]; 8];
        let mut prop_pos = [[0.0f32; 4]; 8];
        let mut prop_tint = [[0.0f32; 4]; 8];
        let mut prop_misc = [[0.0f32; 4]; 8];
        let mut prop_rows = [[0.0f32; 4]; 8];
        let mut prop_var = [[0.0f32; 4]; 8];
        let mut prop_var2 = [[0.0f32; 4]; 8];
        let mut prop_shadow_profile0 = [[0.0f32; 4]; 8];
        let mut prop_shadow_profile1 = [[0.0f32; 4]; 8];
        let mut prop_count = 0u32;
        let mut prop_emitter_present = false;
        for p in settings.props.items.iter().filter(|p| p.enabled).take(8) {
            if p.emits_light {
                prop_emitter_present = true;
            }
            // Sprite-backed props are handled by the CPU overlay after GPU render;
            // leave the slot as zero (disabled) so the GPU skips them.
            let is_sprite = !p.sprite_path.trim().is_empty()
                || (p.sprite_pool_enabled && !p.sprite_pool_paths.trim().is_empty());
            if is_sprite {
                prop_count += 1;
                continue;
            }
            let i = prop_count as usize;
            let prop_tcode = prop_type_code(&p.prop_type);
            let (profile0, profile1) = prop_shadow_profile_defaults(prop_tcode);
            prop_core[i] = [
                1.0,
                prop_tcode,
                if p.mirror { 1.0 } else { 0.0 },
                p.z_spacing.max(0.1),
            ];
            prop_geom[i] = [
                p.wx,
                p.start_wz.max(0.01),
                p.end_wz.max(p.start_wz.max(0.01) + 0.01),
                p.scale.max(0.05),
            ];
            prop_tint[i] = [
                p.tint[0] as f32 / 255.0,
                p.tint[1] as f32 / 255.0,
                p.tint[2] as f32 / 255.0,
                p.path_follow.clamp(0.0, 1.0),
            ];
            prop_pos[i] = [
                p.pos_x.clamp(-32.0, 32.0),
                p.pos_y.clamp(-24.0, 24.0),
                p.pos_z.clamp(-48.0, 48.0),
                if p.pixel_hitbox_enabled { 1.0 } else { 0.0 },
            ];
            prop_misc[i] = [
                p.edge_gap.max(0.0),
                p.seed as f32,
                p.tree_style_mix.clamp(0.0, 1.0),
                p.tree_style_bias.clamp(-1.0, 1.0),
            ];
            prop_rows[i] = [
                p.tree_row_count.max(1).min(64) as f32,
                p.tree_row_spacing.max(0.0),
                p.tree_row_jitter.max(0.0),
                p.y_sink.clamp(0.0, 6.0),
            ];
            prop_var[i] = [
                p.width_scale.max(0.02),
                p.height_scale.max(0.02),
                p.scale_var.max(0.0),
                p.x_jitter.max(0.0),
            ];
            prop_var2[i] = [
                p.y_jitter.max(0.0),
                p.width_var.max(0.0),
                p.height_var.max(0.0),
                (if p.x_jitter_enabled { 1.0 } else { 0.0 })
                    + (if p.y_jitter_enabled { 2.0 } else { 0.0 })
                    + (if p.emits_light { 4.0 } else { 0.0 })
                    + (if p.casts_shadow { 8.0 } else { 0.0 }),
            ];
            prop_shadow_profile0[i] = profile0;
            prop_shadow_profile1[i] = profile1;
            prop_count += 1;
        }

        let mut atmo_core = [[0.0f32; 4]; 4];
        let mut atmo_glow = [[0.0f32; 4]; 4];
        let mut atmo_count = 0u32;
        let mut atmo_emitter_present = false;
        for l in settings
            .atmo
            .layers
            .iter()
            .filter(|l| l.enabled && !matches!(l.atmo_type, crate::settings::AtmoType::None))
            .take(4)
        {
            let i = atmo_count as usize;
            if l.emits_light {
                atmo_emitter_present = true;
            }
            let mount_surface = match l.mount_surface {
                crate::settings::AttachmentSurface::Floating => 0.0,
                crate::settings::AttachmentSurface::Wall => 1.0,
                crate::settings::AttachmentSurface::Floor => 2.0,
                crate::settings::AttachmentSurface::Ceiling => 3.0,
            };
            let mount_side = match l.mount_side {
                crate::settings::MountSide::Both => 0.0,
                crate::settings::MountSide::Left => 1.0,
                crate::settings::MountSide::Right => 2.0,
                crate::settings::MountSide::Center => 3.0,
            };
            let mount_pack = mount_surface
                + mount_side * 10.0
                + if l.emits_light { 100.0 } else { 0.0 }
                + if l.casts_shadow { 200.0 } else { 0.0 };
            atmo_core[i] = [
                atmo_type_code(&l.atmo_type),
                l.torch_h,
                l.torch_spc.max(1) as f32,
                mount_pack,
            ];
            let glow = l.atmo_type.glow_color().unwrap_or([220, 180, 110]);
            atmo_glow[i] = [
                glow[0] as f32 / 255.0,
                glow[1] as f32 / 255.0,
                glow[2] as f32 / 255.0,
                l.torch_scale.max(0.01) * l.fx_scale.max(0.2),
            ];
            atmo_count += 1;
        }

        let (atmo_energy, atmo_tint_rgb) = estimate_atmo_lighting(settings);
        let sun_emits = settings.sky.sun_enabled && settings.sky.sun_emits_light;
        let moon_emits = settings.sky.moon_enabled && settings.sky.moon_emits_light;
        let mut light_dir_acc = 0.0f32;
        let mut light_alt_acc = 0.0f32;
        let mut light_depth_acc = 0.0f32;
        let mut light_weight = 0.0f32;
        let sun_alt = (1.0 - settings.sky.sun_pos[1] - settings.sky.sun_z * 0.5).clamp(0.03, 1.5);
        let moon_alt = (1.0 - settings.sky.moon_pos[1] - settings.sky.moon_z * 0.5).clamp(0.03, 1.4);
        if sun_emits {
            let w = settings.sky.sun_radius.max(0.02);
            light_dir_acc += (settings.sky.sun_pos[0] - 0.5).clamp(-1.0, 1.0) * w;
            light_alt_acc += sun_alt * w;
            light_depth_acc += settings.sky.sun_z.clamp(-1.0, 1.0) * w;
            light_weight += w;
        }
        if moon_emits {
            let w = settings.sky.moon_radius.max(0.02) * 0.7;
            light_dir_acc += (settings.sky.moon_pos[0] - 0.5).clamp(-1.0, 1.0) * w;
            light_alt_acc += moon_alt * w;
            light_depth_acc += settings.sky.moon_z.clamp(-1.0, 1.0) * w;
            light_weight += w;
        }
        let dominant_light_dir = if light_weight > 0.0001 {
            (light_dir_acc / light_weight).clamp(-1.0, 1.0)
        } else {
            0.0
        };
        let light_altitude = if light_weight > 0.0001 {
            (light_alt_acc / light_weight).clamp(0.03, 1.5)
        } else {
            0.55
        };
        let light_depth_bias = if light_weight > 0.0001 {
            (light_depth_acc / light_weight).clamp(-1.0, 1.0)
        } else {
            0.0
        };
        let shadow_len_factor = (1.95 / light_altitude).clamp(0.55, 6.5);
        let sky_light_energy = (if sun_emits { 0.75 } else { 0.0 }) + (if moon_emits { 0.45 } else { 0.0 });
        let emitter_energy = (sky_light_energy + atmo_energy * 0.55).clamp(0.0, 2.5);
        let has_any_emitter = sun_emits || moon_emits || atmo_emitter_present || prop_emitter_present;

        let params = SceneParams {
            dim: [
                width,
                height,
                if settings.sky.enabled { 1 } else { 0 },
                if settings.walls.enabled { 1 } else { 0 },
            ],
            time_scroll: [scroll, global_t, horizon, settings.scene.horizon_curve.clamp(-1.0, 1.0)],
            scene0: [
                settings.scene.max_hw * hw_scale,
                settings.scene.cam_h,
                settings.scene.focal_mult,
                settings.scene.path_power,
            ],
            sky_top: to_rgba_f32(settings.sky.top),
            sky_horizon: to_rgba_f32(settings.sky.horizon),
            void_color: to_rgba_f32(settings.scene.void_color),
            floor_base: to_rgba_f32(settings.floor.base),
            floor_mortar: to_rgba_f32(settings.floor.mortar),
            wall_base: to_rgba_f32(settings.walls.base),
            wall_mortar: to_rgba_f32(settings.walls.mortar),
            sun_data: [
                settings.sky.sun_pos[0],
                settings.sky.sun_pos[1],
                settings.sky.sun_radius,
                if settings.sky.sun_enabled { 1.0 } else { 0.0 },
            ],
            sun_color: [
                settings.sky.sun_color[0] as f32 / 255.0,
                settings.sky.sun_color[1] as f32 / 255.0,
                settings.sky.sun_color[2] as f32 / 255.0,
                settings.sky.sun_z,
            ],
            moon_data: [
                settings.sky.moon_pos[0],
                settings.sky.moon_pos[1],
                settings.sky.moon_radius,
                settings.sky.moon_alpha.clamp(0.0, 2.0),
            ],
            moon_color: [
                settings.sky.moon_color[0] as f32 / 255.0,
                settings.sky.moon_color[1] as f32 / 255.0,
                settings.sky.moon_color[2] as f32 / 255.0,
                settings.sky.moon_z,
            ],
            misc: [
                crate::settings::TILE as f32,
                settings.scene.ambient.clamp(0.0, 2.0),
                settings.walls.l_wx,
                settings.walls.bright.clamp(0.0, 12.0),
            ],
            wall_data: [
                settings.walls.top_coverage.clamp(0.0, 1.0),
                settings.walls.junc_shadow.max(1.0),
                settings.walls.fade_rows.max(1) as f32,
                if settings.post.realtime_shadows_enabled { 1.0 } else { 0.0 },
            ],
            floor_data: [
                settings.floor.depth_fade.clamp(0.05, 30.0),
                settings.floor.edge_vignette.clamp(0.0, 1.0),
                settings.scene.curve_top_weight.clamp(0.0, 2.0),
                settings.scene.curve_bottom_weight.clamp(0.0, 2.0),
            ],
            sky_flags: [
                if settings.sky.stars_enabled { 1 } else { 0 },
                if settings.sky.clouds_enabled { 1 } else { 0 },
                if settings.sky.moon_texture_enabled { 1 } else { 0 },
                if settings.anim.seamless_lock { 1 } else { 0 },
            ],
            sky_counts: [
                settings.sky.stars_count.min(8000),
                settings.sky.cloud_count.min(220),
                settings.sky.stars_seed,
                settings.sky.cloud_seed,
            ],
            sky_misc: [
                settings.sky.stars_size.clamp(0.05, 6.0),
                settings.sky.stars_twinkle.clamp(0.0, 8.0),
                settings.sky.cloud_speed.clamp(0.0, 8.0),
                settings.sky.cloud_scale.clamp(0.2, 6.0),
            ],
            cloud_tint: [
                settings.sky.cloud_tint[0] as f32 / 255.0,
                settings.sky.cloud_tint[1] as f32 / 255.0,
                settings.sky.cloud_tint[2] as f32 / 255.0,
                settings.sky.cloud_opacity.clamp(0.0, 2.0),
            ],
            moon_misc: [
                settings.sky.moon_phase.clamp(-1.0, 1.0),
                settings.sky.moon_texture_scale.clamp(0.2, 4.0),
                settings.sky.cloud_variation.clamp(0.0, 1.0),
                light_depth_bias,
            ],
            atmo_scene: [
                settings.scene.atmo_light_influence.clamp(0.0, 2.0),
                settings.scene.atmo_tint_influence.clamp(0.0, 1.0),
                atmo_energy,
                if settings.post.realtime_lighting_enabled { 1.0 } else { 0.0 },
            ],
            atmo_tint: [
                atmo_tint_rgb[0] / 255.0,
                atmo_tint_rgb[1] / 255.0,
                atmo_tint_rgb[2] / 255.0,
                dominant_light_dir,
            ],
            loop_data: [
                settings.anim.loop_s.max(1) as f32,
                emitter_energy,
                if has_any_emitter { 1.0 } else { 0.0 },
                shadow_len_factor,
            ],
            tex_data: [
                settings.floor.tex_scale.max(0.05),
                settings.walls.tex_scale.max(0.05),
                0.0,
                0.0,
            ],
            tex_flags: [
                if settings.floor.tex_rot_90 { 1 } else { 0 },
                if settings.walls.tex_rot_90 { 1 } else { 0 },
                0,
                0,
            ],
            feature_counts: [prop_count, atmo_count, 0, 0],
            prop_core,
            prop_geom,
            prop_pos,
            prop_tint,
            prop_misc,
            prop_rows,
            prop_var,
            prop_var2,
            prop_shadow_profile0,
            prop_shadow_profile1,
            atmo_core,
            atmo_glow,
            grass_data: [
                settings.scene.grass_density.clamp(0.1, 4.0),
                settings.scene.grass_height.clamp(0.2, 3.0),
                settings.scene.grass_upright.clamp(0.0, 1.0),
                if settings.scene.grass_enabled { 1.0 } else { 0.0 },
            ],
            grass_color: [
                settings.scene.grass_color[0] as f32 / 255.0,
                settings.scene.grass_color[1] as f32 / 255.0,
                settings.scene.grass_color[2] as f32 / 255.0,
                0.0,
            ],
            post_data: [
                settings.post.vignette.clamp(0.0, 2.0),
                settings.post.fog_density.clamp(0.0, 1.0),
                settings.post.bloom.clamp(0.0, 2.0),
                settings.post.grain.clamp(0.0, 1.0),
            ],
            post_flags: [
                if settings.post.fog_enabled { 1 } else { 0 },
                if settings.post.vignette_enabled { 1 } else { 0 },
                if settings.post.bloom_enabled { 1 } else { 0 },
                if settings.post.grain_enabled { 1 } else { 0 },
            ],
            post_colors: [
                settings.post.fog_color[0] as f32 / 255.0,
                settings.post.fog_color[1] as f32 / 255.0,
                settings.post.fog_color[2] as f32 / 255.0,
                if settings.post.saturation_enabled {
                    settings.post.saturation.clamp(0.0, 3.0)
                } else {
                    1.0
                },
            ],
        };

        self.queue
            .write_buffer(&self.params_buffer, 0, bytemuck::bytes_of(&params));

        self.ensure_tile_textures(settings);
        self.ensure_output_resources(width, height);
        let floor_cached = self
            .floor_tile_cache
            .as_ref()
            .ok_or_else(|| "GPU floor tile cache unavailable".to_owned())?;
        let wall_cached = self
            .wall_tile_cache
            .as_ref()
            .ok_or_else(|| "GPU wall tile cache unavailable".to_owned())?;
        let floor_key = floor_cached.key.clone();
        let wall_key = wall_cached.key.clone();

        let bind_group_dirty = self
            .bind_group_cache
            .as_ref()
            .map(|c| {
                c.width != width
                    || c.height != height
                    || c.floor_key != floor_key
                    || c.wall_key != wall_key
            })
            .unwrap_or(true);

        if bind_group_dirty {
            let bind_group = {
                let floor_view = &self
                    .floor_tile_cache
                    .as_ref()
                    .ok_or_else(|| "GPU floor tile cache unavailable".to_owned())?
                    .view;
                let wall_view = &self
                    .wall_tile_cache
                    .as_ref()
                    .ok_or_else(|| "GPU wall tile cache unavailable".to_owned())?
                    .view;
                let out_view = &self
                    .output_cache
                    .as_ref()
                    .ok_or_else(|| "GPU output cache unavailable".to_owned())?
                    .view;

                self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("path_forge_gpu_scene_bg"),
                    layout: &self.bind_group_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(out_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: self.params_buffer.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::TextureView(floor_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: wgpu::BindingResource::TextureView(wall_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 4,
                            resource: wgpu::BindingResource::Sampler(&self.tile_sampler),
                        },
                    ],
                })
            };
            self.bind_group_cache = Some(CachedBindGroup {
                width,
                height,
                floor_key,
                wall_key,
                bind_group,
            });
        }
        let bind_group_cached = self
            .bind_group_cache
            .as_ref()
            .ok_or_else(|| "GPU bind group cache unavailable".to_owned())?;

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("path_forge_gpu_scene_encoder"),
            });

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("path_forge_gpu_scene_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group_cached.bind_group, &[]);
            pass.dispatch_workgroups(width.div_ceil(8), height.div_ceil(8), 1);
        }

        let output_cached = self
            .output_cache
            .as_ref()
            .ok_or_else(|| "GPU output cache unavailable".to_owned())?;

        encoder.copy_texture_to_buffer(
            wgpu::ImageCopyTexture {
                texture: &output_cached.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyBuffer {
                buffer: &output_cached.readback,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(output_cached.padded_bpr),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        self.queue.submit(Some(encoder.finish()));

        let slice = output_cached.readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |res| {
            let _ = tx.send(res);
        });
        self.device.poll(wgpu::Maintain::Wait);
        match rx.recv() {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(format!("GPU scene readback map failed: {e}")),
            Err(_) => return Err("GPU scene readback channel closed".to_owned()),
        }

        let mapped = slice.get_mapped_range();
        let mut out = vec![0u8; (output_cached.tight_bpr as usize) * (height as usize)];
        let padded_bpr_usize = output_cached.padded_bpr as usize;
        let tight_bpr_usize = output_cached.tight_bpr as usize;
        for row in 0..height as usize {
            let src_off = row * padded_bpr_usize;
            let dst_off = row * tight_bpr_usize;
            out[dst_off..dst_off + tight_bpr_usize]
                .copy_from_slice(&mapped[src_off..src_off + tight_bpr_usize]);
        }
        drop(mapped);
        output_cached.readback.unmap();

        if settings.scene.horizon_curve.abs() > 0.001 {
            apply_world_curve_post(
                &mut out,
                width as usize,
                height as usize,
                horizon as usize,
                settings.scene.horizon_curve,
            );
        }

        Ok(out)
    }
}

fn to_rgba_f32(rgb: [u8; 3]) -> [f32; 4] {
    [
        rgb[0] as f32 / 255.0,
        rgb[1] as f32 / 255.0,
        rgb[2] as f32 / 255.0,
        1.0,
    ]
}

const SCENE_SHADER: &str = r#"
struct Params {
    dim: vec4<u32>,
    time_scroll: vec4<f32>,
    scene0: vec4<f32>,
    sky_top: vec4<f32>,
    sky_horizon: vec4<f32>,
    void_color: vec4<f32>,
    floor_base: vec4<f32>,
    floor_mortar: vec4<f32>,
    wall_base: vec4<f32>,
    wall_mortar: vec4<f32>,
    sun_data: vec4<f32>,
    sun_color: vec4<f32>,
    moon_data: vec4<f32>,
    moon_color: vec4<f32>,
    misc: vec4<f32>,
    wall_data: vec4<f32>,
    floor_data: vec4<f32>,
    sky_flags: vec4<u32>,
    sky_counts: vec4<u32>,
    sky_misc: vec4<f32>,
    cloud_tint: vec4<f32>,
    moon_misc: vec4<f32>,
    atmo_scene: vec4<f32>,
    atmo_tint: vec4<f32>,
    loop_data: vec4<f32>,
    tex_data: vec4<f32>,
    tex_flags: vec4<u32>,
    feature_counts: vec4<u32>,
    prop_core: array<vec4<f32>, 8>,
    prop_geom: array<vec4<f32>, 8>,
    prop_pos: array<vec4<f32>, 8>,
    prop_tint: array<vec4<f32>, 8>,
    prop_misc: array<vec4<f32>, 8>,
    prop_rows: array<vec4<f32>, 8>,
    prop_var: array<vec4<f32>, 8>,
    prop_var2: array<vec4<f32>, 8>,
    prop_shadow_profile0: array<vec4<f32>, 8>,
    prop_shadow_profile1: array<vec4<f32>, 8>,
    atmo_core: array<vec4<f32>, 4>,
    atmo_glow: array<vec4<f32>, 4>,
    grass_data:  vec4<f32>,
    grass_color: vec4<f32>,
    post_data:   vec4<f32>,
    post_flags:  vec4<u32>,
    post_colors: vec4<f32>,
};

@group(0) @binding(0) var dst_tex: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(1) var<uniform> params: Params;
@group(0) @binding(2) var floor_tex: texture_2d<f32>;
@group(0) @binding(3) var wall_tex: texture_2d<f32>;
@group(0) @binding(4) var tile_sampler: sampler;

fn hash2(p: vec2<f32>) -> f32 {
    let h = sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453123;
    return fract(h);
}

fn noise2(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let a = hash2(i);
    let b = hash2(i + vec2<f32>(1.0, 0.0));
    let c = hash2(i + vec2<f32>(0.0, 1.0));
    let d = hash2(i + vec2<f32>(1.0, 1.0));
    let u = f * f * (vec2<f32>(3.0, 3.0) - 2.0 * f);
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn fbm2(p: vec2<f32>) -> f32 {
    var a = 0.5;
    var f = 1.0;
    var s = 0.0;
    for (var i: i32 = 0; i < 4; i = i + 1) {
        s = s + noise2(p * f) * a;
        f = f * 2.03;
        a = a * 0.52;
    }
    return s;
}

fn lerp3(a: vec3<f32>, b: vec3<f32>, t: f32) -> vec3<f32> {
    return a + (b - a) * t;
}

fn sample_tile_tex(tex: texture_2d<f32>, u: f32, v: f32, rot_90: bool) -> vec3<f32> {
    let tf = max(params.misc.x, 1.0);
    var uu = u;
    var vv = v;
    if (rot_90) {
        let tu = uu;
        uu = vv;
        vv = -tu;
    }
    let uv = vec2<f32>(fract(uu / tf), fract(vv / tf));
    return textureSampleLevel(tex, tile_sampler, uv, 0.0).rgb;
}

fn world_curve_offset(x: f32, y: f32, w: f32, h: f32, horizon: f32, curve: f32) -> f32 {
    let amp = clamp(curve, -1.0, 1.0) * (h * 0.22);
    let den = max(w - 1.0, 1.0);
    let nx = x / den * 2.0 - 1.0;
    let blend_top = max(horizon - 40.0, 0.0);
    let blend_span = 60.0;
    let t = clamp((y - blend_top) / blend_span, 0.0, 1.0);
    let weight = t * t * (3.0 - 2.0 * t);
    return amp * nx * nx * weight;
}

fn path_curve_shift(curve: f32, max_hw: f32, t: f32) -> f32 {
    let tt = clamp(t, 0.0, 1.0);
    return clamp(curve, -1.0, 1.0) * max_hw * 0.72 * (tt * tt);
}

fn path_width_weight(t: f32) -> f32 {
    let tt = clamp(t, 0.0, 1.0);
    let top_w = clamp(params.floor_data.z, 0.0, 2.0);
    let bottom_w = clamp(params.floor_data.w, 0.0, 2.0);
    return mix(top_w, bottom_w, tt);
}

fn posmod_i32(v: i32, m: i32) -> i32 {
    let r = v % m;
    return select(r + m, r, r >= 0);
}

fn loop_seed_index(n: f32, spacing: f32) -> i32 {
    let ni = i32(n);
    if (params.sky_flags.w == 0u) {
        return ni;
    }
    let slots = max(i32(round(params.loop_data.x / max(spacing, 0.001))), 1);
    return posmod_i32(ni, slots);
}

fn shape_circle(p: vec2<f32>, c: vec2<f32>, r: f32) -> f32 {
    let d = distance(p, c);
    return select(0.0, 1.0, d <= r);
}

fn shape_ellipse(p: vec2<f32>, c: vec2<f32>, rx: f32, ry: f32) -> f32 {
    let dx = (p.x - c.x) / max(rx, 0.001);
    let dy = (p.y - c.y) / max(ry, 0.001);
    return select(0.0, 1.0, dx * dx + dy * dy <= 1.0);
}

fn shape_rect(p: vec2<f32>, c: vec2<f32>, hx: f32, hy: f32) -> f32 {
    let dx = abs(p.x - c.x) / max(hx, 0.001);
    let dy = abs(p.y - c.y) / max(hy, 0.001);
    return select(0.0, 1.0, dx <= 1.0 && dy <= 1.0);
}

fn shape_triangle(p: vec2<f32>, apex_x: f32, apex_y: f32, base_w: f32, height: f32) -> f32 {
    if (height < 0.5 || base_w < 0.5) {
        return 0.0;
    }
    if (p.y < apex_y || p.y > apex_y + height) {
        return 0.0;
    }
    let t = clamp((p.y - apex_y) / height, 0.0, 1.0);
    let hw = t * base_w * 0.5;
    return select(0.0, 1.0, abs(p.x - apex_x) <= hw);
}

fn shape_line(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let ab = b - a;
    let ap = p - a;
    let den = max(dot(ab, ab), 0.001);
    let t = clamp(dot(ap, ab) / den, 0.0, 1.0);
    let q = a + ab * t;
    return select(0.0, 1.0, distance(p, q) <= 0.6);
}

fn shape_capsule(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>, r: f32) -> f32 {
    let ab = b - a;
    let ap = p - a;
    let den = max(dot(ab, ab), 0.001);
    let t = clamp(dot(ap, ab) / den, 0.0, 1.0);
    let q = a + ab * t;
    return select(0.0, 1.0, distance(p, q) <= max(r, 0.1));
}

fn darker(c: vec3<f32>, f: f32) -> vec3<f32> {
    return c * max(f, 0.0);
}

fn lighter(c: vec3<f32>, f: f32) -> vec3<f32> {
    return min(c * max(f, 0.0), vec3<f32>(1.0, 1.0, 1.0));
}

fn glow_strength(dist: f32, radius: f32, peak_alpha: f32) -> f32 {
    if (dist >= radius || radius <= 0.001) {
        return 0.0;
    }
    let t = 1.0 - dist / radius;
    if (t > 0.6) {
        return min(peak_alpha * (0.18 + (0.75 - 0.18) * ((t - 0.6) / 0.4)), 1.0);
    }
    return min(peak_alpha * 0.18 * t / 0.6, 1.0);
}

fn flame_strength(p: vec2<f32>, cx: f32, cy: f32, fh: f32, fl: f32) -> f32 {
    let sx = 0.6 + abs(fl) * 0.08;
    let sy = 1.6 + fl * 0.35;
    let center_y = cy - fh * 0.5;
    let dx = (p.x - cx) / max(sx * fh, 0.001);
    let dy = (p.y - center_y) / max(sy * fh, 0.001);
    let dist = sqrt(dx * dx + dy * dy);
    if (dist >= 1.0) { return 0.0; }
    if (dist < 0.35) {
        let t = dist / 0.35;
        return mix(0.95, 0.80, t);
    }
    if (dist < 0.65) {
        let t = (dist - 0.35) / 0.30;
        return mix(0.80, 0.50, t);
    }
    let t = (dist - 0.65) / 0.35;
    return mix(0.50, 0.0, t);
}

fn grass_blade_strength(
    pxy: vec2<f32>,
    x: f32,
    y_base: f32,
    len: f32,
    bend: f32,
    thickness: f32,
) -> f32 {
    if (len <= 0.5) { return 0.0; }
    let steps = i32(max(len, 2.0));
    var best = 0.0;
    for (var s: i32 = 0; s < steps; s = s + 1) {
        let u = f32(s) / max(f32(steps - 1), 1.0);
        let px = x + bend * u * u;
        let py = y_base - len * u;
        let dx = abs(pxy.x - px) / max(thickness, 0.001);
        let dy = abs(pxy.y - py) / max(thickness * 0.85, 0.001);
        let dist = max(dx, dy);
        if (dist < 1.0) {
            let stem = 1.0 - dist;
            let tip_alpha = 0.30 + 0.45 * (1.0 - u);
            best = max(best, clamp(tip_alpha * (0.65 + 0.35 * stem), 0.0, 1.0));
        }
    }
    return clamp(best, 0.0, 1.0);
}

fn grass_tuft_strength(
    pxy: vec2<f32>,
    x: f32,
    y_base: f32,
    w_span: f32,
    h_span: f32,
    color_mix: f32,
    seed: f32,
    upright: f32,
) -> f32 {
    let blades = 3 + i32(fract(seed) * 5.0);
    let upright_k = clamp(upright, 0.0, 1.0);
    var best = 0.0;
    for (var i: i32 = 0; i < blades; i = i + 1) {
        let hv = hash2(vec2<f32>(seed + 113.0 + f32(i), f32(i)));
        let side = select(-1.0, 1.0, fract(hv * 7.0) > 0.5);
        let t = fract(hv * 65535.0);
        let bx = x + ((f32(i) / max(f32(blades), 1.0)) - 0.5) * w_span * (1.05 - upright_k * 0.35);
        let len = h_span * (0.55 + t * 0.90);
        let bend_base = 0.03 + fract(hv * 255.0) * 0.16;
        let bend = side * len * bend_base * (1.0 - upright_k * 0.82);
        let thickness = max(0.22 + color_mix * 0.05, 0.20);
        best = max(best, grass_blade_strength(pxy, bx, y_base, len, bend, thickness));
    }
    return clamp(best, 0.0, 1.0);
}

fn paint(base: vec3<f32>, paint_col: vec3<f32>, m: f32) -> vec3<f32> {
    return select(base, paint_col, m > 0.5);
}

// Top-down footprint of the imagined 3D tree, used when light is overhead.
// When light_dir≈0 and light_depth_bias≈0 the shadow collapses to this shape.
fn tree_topdown_footprint(pxy: vec2<f32>, sx: f32, sy_floor: f32, ps_x: f32, ps_y: f32, tree_shadow_mode: f32) -> f32 {
    if (tree_shadow_mode < 1.5) {
        // Round canopy only: overhead blob should be canopy-dominant.
        let c0 = shape_ellipse(pxy, vec2<f32>(sx, sy_floor + ps_y * 0.12), ps_x * 2.58, ps_x * 1.88);
        let c1 = shape_ellipse(pxy, vec2<f32>(sx - ps_x * 0.10, sy_floor + ps_y * 0.10), ps_x * 2.02, ps_x * 1.50);
        let c2 = shape_ellipse(pxy, vec2<f32>(sx + ps_x * 0.10, sy_floor + ps_y * 0.10), ps_x * 1.62, ps_x * 1.20);
        return clamp(max(c0, max(c1, c2)), 0.0, 1.0);
    } else if (tree_shadow_mode < 2.5) {
        // Pine canopy only: cone tiers compressed in top-down view.
        let t0 = shape_ellipse(pxy, vec2<f32>(sx, sy_floor + ps_y * 0.10), ps_x * 2.22, ps_x * 1.64);
        let t1 = shape_ellipse(pxy, vec2<f32>(sx, sy_floor + ps_y * 0.08), ps_x * 1.55, ps_x * 1.15);
        let t2 = shape_ellipse(pxy, vec2<f32>(sx, sy_floor + ps_y * 0.06), ps_x * 0.95, ps_x * 0.72);
        return clamp(max(t0, max(t1, t2)), 0.0, 1.0);
    } else {
        // Dead canopy is sparse; use subtle branch spread only.
        let b0 = shape_ellipse(pxy, vec2<f32>(sx, sy_floor + ps_y * 0.07), ps_x * 0.92, ps_x * 0.66);
        let arm_l = shape_capsule(pxy, vec2<f32>(sx, sy_floor + ps_y * 0.06), vec2<f32>(sx - ps_x * 1.00, sy_floor + ps_y * 0.07), max(ps_x * 0.11, 0.18));
        let arm_r = shape_capsule(pxy, vec2<f32>(sx, sy_floor + ps_y * 0.06), vec2<f32>(sx + ps_x * 0.98, sy_floor + ps_y * 0.07), max(ps_x * 0.10, 0.17));
        return clamp(max(b0, max(arm_l, arm_r)), 0.0, 1.0);
    }
}

fn tree_object_mask_local(local: vec2<f32>, ps_x: f32, ps_y: f32, tree_shadow_mode: f32) -> f32 {
    var trunk_h = ps_y * 3.0;
    var trunk_w = max(ps_x * 0.55, 0.55);
    if (tree_shadow_mode < 1.5) {
        trunk_h = ps_y * 4.35;
        trunk_w = max(ps_x * 0.62, 0.60);
    } else if (tree_shadow_mode < 2.5) {
        trunk_h = ps_y * 2.45;
        trunk_w = max(ps_x * 0.54, 0.56);
    } else {
        trunk_h = ps_y * 3.25;
        trunk_w = max(ps_x * 0.48, 0.50);
    }

    var m = shape_rect(local, vec2<f32>(0.0, -trunk_h * 0.5), trunk_w * 0.5, trunk_h * 0.5);

    if (tree_shadow_mode < 1.5) {
        let cy_b = -trunk_h;
        let c0 = shape_ellipse(local, vec2<f32>(0.0, cy_b - ps_y * 0.55), ps_x * 2.95, ps_y * 2.25);
        let c1 = shape_ellipse(local, vec2<f32>(-ps_x * 0.30, cy_b - ps_y * 1.72), ps_x * 2.20, ps_y * 1.95);
        let c2 = shape_ellipse(local, vec2<f32>( ps_x * 0.28, cy_b - ps_y * 1.65), ps_x * 2.05, ps_y * 1.80);
        let c3 = shape_ellipse(local, vec2<f32>(0.0, cy_b - ps_y * 2.95), ps_x * 1.65, ps_y * 1.45);
        m = max(m, max(c0, max(c1, max(c2, c3))));
    } else if (tree_shadow_mode < 2.5) {
        let base_y = -trunk_h;
        let p0 = shape_triangle(local, 0.0, base_y - 2.4 * ps_y, 3.6 * ps_x, 2.4 * ps_y);
        let p1 = shape_triangle(local, 0.0, base_y - 1.4 * ps_y - 2.2 * ps_y, 3.0 * ps_x, 2.2 * ps_y);
        let p2 = shape_triangle(local, 0.0, base_y - 2.7 * ps_y - 2.0 * ps_y, 2.4 * ps_x, 2.0 * ps_y);
        let p3 = shape_triangle(local, 0.0, base_y - 4.1 * ps_y, 1.8 * ps_x, 1.6 * ps_y);
        m = max(m, max(p0, max(p1, max(p2, p3))));
    } else {
        let branch0 = shape_capsule(local, vec2<f32>(0.0, -trunk_h * 0.72), vec2<f32>(-ps_x * 1.05, -trunk_h * 1.28), max(ps_x * 0.15, 0.22));
        let branch1 = shape_capsule(local, vec2<f32>(0.0, -trunk_h * 0.64), vec2<f32>( ps_x * 1.00, -trunk_h * 1.12), max(ps_x * 0.13, 0.20));
        let branch2 = shape_capsule(local, vec2<f32>(0.0, -trunk_h * 0.46), vec2<f32>(-ps_x * 0.78, -trunk_h * 0.88), max(ps_x * 0.12, 0.18));
        m = max(m, max(branch0, max(branch1, branch2)));
    }

    return clamp(m, 0.0, 1.0);
}

fn tree_trunk_mask_local(local: vec2<f32>, ps_x: f32, ps_y: f32, tree_shadow_mode: f32) -> f32 {
    var trunk_h = ps_y * 3.0;
    var trunk_w = max(ps_x * 0.55, 0.55);
    if (tree_shadow_mode < 1.5) {
        trunk_h = ps_y * 4.35;
        trunk_w = max(ps_x * 0.62, 0.60);
    } else if (tree_shadow_mode < 2.5) {
        trunk_h = ps_y * 2.45;
        trunk_w = max(ps_x * 0.54, 0.56);
    } else {
        trunk_h = ps_y * 3.25;
        trunk_w = max(ps_x * 0.48, 0.50);
    }
    return clamp(
        shape_rect(local, vec2<f32>(0.0, -trunk_h * 0.5), trunk_w * 0.5, trunk_h * 0.5),
        0.0,
        1.0,
    );
}

fn shadow_tree_mode_from_tcode(tcode: f32, profile_tree_mode: f32) -> f32 {
    if (profile_tree_mode > 0.5) {
        return profile_tree_mode;
    }
    if (tcode < 0.5) {
        return 1.0;
    } else if (tcode < 1.5) {
        return 2.0;
    } else if (tcode >= 6.0 && tcode < 6.5) {
        return 3.0;
    }
    return 0.0;
}

fn prop_object_mask_local(local: vec2<f32>, ps_x: f32, ps_y: f32, tcode: f32, tree_shadow_mode: f32) -> f32 {
    if (tree_shadow_mode > 0.5) {
        return tree_object_mask_local(local, ps_x, ps_y, tree_shadow_mode);
    }

    if (tcode < 2.5) {
        // Rounded shrub
        let c0 = shape_ellipse(local, vec2<f32>(-ps_x * 0.35, -ps_y * 1.20), ps_x * 1.70, ps_y * 1.20);
        let c1 = shape_ellipse(local, vec2<f32>( ps_x * 0.32, -ps_y * 1.08), ps_x * 1.45, ps_y * 1.05);
        let c2 = shape_ellipse(local, vec2<f32>(0.0, -ps_y * 1.72), ps_x * 1.20, ps_y * 0.95);
        return clamp(max(c0, max(c1, c2)), 0.0, 1.0);
    }

    if (tcode < 4.5) {
        // Rock/boulder silhouette
        let r0 = shape_ellipse(local, vec2<f32>(0.0, -ps_y * 0.85), ps_x * 1.95, ps_y * 1.25);
        let r1 = shape_ellipse(local, vec2<f32>(-ps_x * 0.30, -ps_y * 1.05), ps_x * 1.35, ps_y * 0.98);
        return clamp(max(r0, r1), 0.0, 1.0);
    }

    if (tcode < 5.5) {
        // Cactus: trunk + side arms
        let stem = shape_rect(local, vec2<f32>(0.0, -ps_y * 2.10), ps_x * 0.62, ps_y * 2.10);
        let arm_l = shape_capsule(local, vec2<f32>(-ps_x * 0.62, -ps_y * 1.55), vec2<f32>(-ps_x * 1.58, -ps_y * 1.15), max(ps_x * 0.18, 0.24));
        let arm_r = shape_capsule(local, vec2<f32>( ps_x * 0.60, -ps_y * 1.30), vec2<f32>( ps_x * 1.45, -ps_y * 1.05), max(ps_x * 0.17, 0.22));
        return clamp(max(stem, max(arm_l, arm_r)), 0.0, 1.0);
    }

    // Mushroom / generic cap
    let stem = shape_rect(local, vec2<f32>(0.0, -ps_y * 1.00), ps_x * 0.34, ps_y * 1.00);
    let cap = shape_ellipse(local, vec2<f32>(0.0, -ps_y * 2.05), ps_x * 1.72, ps_y * 1.02);
    return clamp(max(stem, cap), 0.0, 1.0);
}

fn prop_topdown_footprint(pxy: vec2<f32>, sx: f32, sy_floor: f32, ps_x: f32, ps_y: f32, tcode: f32, tree_shadow_mode: f32) -> f32 {
    if (tree_shadow_mode > 0.5) {
        return tree_topdown_footprint(pxy, sx, sy_floor, ps_x, ps_y, tree_shadow_mode);
    }

    if (tcode < 2.5) {
        let e0 = shape_ellipse(pxy, vec2<f32>(sx, sy_floor + ps_y * 0.08), ps_x * 1.68, ps_x * 1.26);
        let e1 = shape_ellipse(pxy, vec2<f32>(sx, sy_floor + ps_y * 0.06), ps_x * 1.18, ps_x * 0.88);
        return clamp(max(e0, e1), 0.0, 1.0);
    }
    if (tcode < 4.5) {
        return clamp(shape_ellipse(pxy, vec2<f32>(sx, sy_floor + ps_y * 0.08), ps_x * 1.48, ps_x * 1.02), 0.0, 1.0);
    }
    if (tcode < 5.5) {
        let stem = shape_ellipse(pxy, vec2<f32>(sx, sy_floor + ps_y * 0.06), ps_x * 0.58, ps_x * 0.44);
        let arm_l = shape_capsule(pxy, vec2<f32>(sx - ps_x * 0.45, sy_floor + ps_y * 0.07), vec2<f32>(sx - ps_x * 1.30, sy_floor + ps_y * 0.07), max(ps_x * 0.14, 0.20));
        let arm_r = shape_capsule(pxy, vec2<f32>(sx + ps_x * 0.45, sy_floor + ps_y * 0.07), vec2<f32>(sx + ps_x * 1.20, sy_floor + ps_y * 0.07), max(ps_x * 0.13, 0.19));
        return clamp(max(stem, max(arm_l, arm_r)), 0.0, 1.0);
    }
    let cap = shape_ellipse(pxy, vec2<f32>(sx, sy_floor + ps_y * 0.08), ps_x * 1.18, ps_x * 0.90);
    let stem = shape_ellipse(pxy, vec2<f32>(sx, sy_floor + ps_y * 0.06), ps_x * 0.38, ps_x * 0.30);
    return clamp(max(cap, stem), 0.0, 1.0);
}

fn prop_shadow_height(ps_y: f32, tcode: f32, tree_shadow_mode: f32, shadow_profile0: vec4<f32>) -> f32 {
    if (shadow_profile0.x > 0.01) {
        return ps_y * max(shadow_profile0.x, 0.5);
    }
    if (tree_shadow_mode > 0.5) {
        return ps_y * select(select(6.4, 8.4, tree_shadow_mode < 1.5), 4.8, tree_shadow_mode < 2.5);
    }
    if (tcode < 2.5) {
        return ps_y * 3.9;
    }
    if (tcode < 4.5) {
        return ps_y * 2.4;
    }
    if (tcode < 5.5) {
        return ps_y * 5.0;
    }
    return ps_y * 3.1;
}

fn prop_ground_shadow_strength(
    pxy: vec2<f32>,
    horizon: f32,
    sx: f32,
    sy_floor: f32,
    ps_x: f32,
    ps_y: f32,
    light_dir: f32,
    light_depth_bias: f32,
    shadow_len: f32,
    pixel_hitbox: bool,
    shadow_tcode: f32,
    shadow_profile0: vec4<f32>,
    shadow_profile1: vec4<f32>,
) -> f32 {
    if (pxy.y < horizon) {
        return 0.0;
    }
    let shift_x = light_dir * ps_x * shadow_len;
    let depth_sign = select(-1.0, 1.0, light_depth_bias >= 0.0);
    let depth_mag = clamp(abs(light_depth_bias), 0.0, 1.0);
    let y_mag = 0.18 + depth_mag * 0.52;
    let shift_y = depth_sign * ps_y * shadow_len * y_mag;
    let tree_shadow_mode = shadow_tree_mode_from_tcode(shadow_tcode, shadow_profile1.w);
    let cast_y_bias = depth_sign * ps_y * (0.06 + depth_mag * 0.05);
    let cast_vec = vec2<f32>(shift_x * (0.90 + shadow_len * 0.08), shift_y * (0.90 + shadow_len * 0.08) + cast_y_bias);
    let cast_len = max(length(cast_vec), 0.001);
    let cast_dir = cast_vec / cast_len;
    let cast_perp = vec2<f32>(-cast_dir.y, cast_dir.x);
    let rel = pxy - vec2<f32>(sx, sy_floor);
    let along = dot(rel, cast_dir);
    let perp = abs(dot(rel, cast_perp));

    // Strict one-direction casting; this avoids dual/opposite shadow stacks.
    let forward_start = max(shadow_profile1.y, 0.01);
    let forward_end = max(shadow_profile1.z, forward_start + 0.01);
    let forward_mask = smoothstep(-ps_x * forward_start, ps_x * forward_end, along);
    let along_n = clamp(along / cast_len, 0.0, 1.0);
    let side_width_base = max(shadow_profile0.y, 0.2);
    let side_width_gain = max(shadow_profile0.z, 0.1);
    let side_width = max(ps_x * (side_width_base + along_n * side_width_gain), 0.001);
    let side_mask = clamp(1.0 - perp / side_width, 0.0, 1.0);

    // One projector core for trees + generic sprites.
    let obj_h = prop_shadow_height(ps_y, shadow_tcode, tree_shadow_mode, shadow_profile0);
    var projected = 0.0;
    for (var i: i32 = 0; i < 9; i = i + 1) {
        let z = f32(i) / 8.0;
        let source_pos = pxy - cast_vec * z;
        let ground_local_y = source_pos.y - sy_floor;
        let mirrored_local_y = -z * obj_h - ground_local_y * 0.20;
        let generic_local_y = -z * obj_h + ground_local_y * 0.20;
        let local = vec2<f32>(
            source_pos.x - sx,
            select(generic_local_y, mirrored_local_y, tree_shadow_mode > 0.5),
        );
        let obj = prop_object_mask_local(local, ps_x, ps_y, shadow_tcode, tree_shadow_mode);
        let z_align = 1.0 - abs(z - along_n) * 1.85;
        let z_weight = clamp(z_align, 0.0, 1.0) * (0.64 + z * 0.36);
        projected = max(projected, obj * z_weight);
    }
    projected *= side_mask * forward_mask;

    // Ground connection near the base to prevent detached silhouettes.
    let base_link = shape_capsule(
        pxy,
        vec2<f32>(sx, sy_floor + ps_y * 0.03),
        vec2<f32>(sx + cast_vec.x * 0.34, sy_floor + cast_vec.y * 0.34 + ps_y * 0.03),
        max(ps_x * 0.32, 0.48),
    );
    let directional_shadow = max(projected, base_link * forward_mask * smoothstep(0.12, 0.85, along_n));

    // Overhead blend from top-down footprint into directional cast.
    let footprint = prop_topdown_footprint(pxy, sx, sy_floor, ps_x, ps_y, shadow_tcode, tree_shadow_mode);
    let overhead_near = max(shadow_profile0.w, 0.05);
    let overhead_far = max(shadow_profile1.x, overhead_near + 0.2);
    let overhead_w = 1.0 - smoothstep(ps_x * overhead_near, ps_x * overhead_far, cast_len);
    var shaped = clamp(max(directional_shadow, footprint * overhead_w), 0.0, 1.0);

    if (!pixel_hitbox) {
        return shaped;
    }
    if (tree_shadow_mode > 0.5) {
        return clamp(smoothstep(0.18, 0.86, shaped), 0.0, 1.0);
    }
    return clamp(smoothstep(0.15, 0.88, shaped), 0.0, 1.0);
}

fn prop_root_contact_strength(
    pxy: vec2<f32>,
    horizon: f32,
    sx: f32,
    sy_floor: f32,
    ps_x: f32,
    ps_y: f32,
    amount: f32,
    treeish: bool,
) -> f32 {
    if (!treeish || pxy.y < horizon) {
        return 0.0;
    }
    let a = clamp(amount, 0.0, 1.0);
    if (a <= 0.001) {
        return 0.0;
    }
    let cy = sy_floor + ps_y * 0.04;
    let rx = max(ps_x * (0.78 + a * 0.36), 0.7);
    let ry = max(ps_y * (0.24 + a * 0.24), 0.45);
    let core = shape_ellipse(pxy, vec2<f32>(sx, cy), rx, ry);
    let lobe_l = shape_ellipse(pxy, vec2<f32>(sx - rx * 0.55, cy + ry * 0.20), rx * 0.52, ry * 0.74);
    let lobe_r = shape_ellipse(pxy, vec2<f32>(sx + rx * 0.55, cy + ry * 0.20), rx * 0.52, ry * 0.74);
    return clamp((core * 0.78 + max(lobe_l, lobe_r) * 0.62) * (0.60 + a * 0.32), 0.0, 1.0);
}

fn prop_draw(
    pxy: vec2<f32>,
    base_col: vec3<f32>,
    tint_col: vec3<f32>,
    tcode: f32,
    sx: f32,
    sy_base: f32,
    ps_x: f32,
    ps_y: f32,
    rock_var: f32,
) -> vec3<f32> {
    var col = base_col;
    if (tcode < 0.5) {
        let tw = max(ps_x * 0.62, 1.0);
        let th = ps_y * 4.4;
        let trunk_col = vec3<f32>(
            tint_col.x * 0.32 + (20.0 / 255.0) * 0.68,
            tint_col.y * 0.20 + (14.0 / 255.0) * 0.80,
            tint_col.z * 0.10 + (8.0 / 255.0) * 0.90,
        );
        col = paint(col, trunk_col, shape_rect(pxy, vec2<f32>(sx, sy_base - th * 0.5), tw * 0.5, th * 0.5));
        let cy_b = sy_base - th;
        col = paint(col, darker(tint_col, 0.58), shape_ellipse(pxy, vec2<f32>(sx, cy_b - ps_y * 0.55), ps_x * 2.95, ps_y * 2.25));
        col = paint(col, darker(tint_col, 0.80), shape_ellipse(pxy, vec2<f32>(sx - ps_x * 0.30, cy_b - ps_y * 1.72), ps_x * 2.20, ps_y * 1.95));
        col = paint(col, tint_col, shape_ellipse(pxy, vec2<f32>(sx + ps_x * 0.28, cy_b - ps_y * 1.65), ps_x * 2.05, ps_y * 1.80));
        col = paint(col, lighter(tint_col, 1.22), shape_ellipse(pxy, vec2<f32>(sx, cy_b - ps_y * 2.95), ps_x * 1.65, ps_y * 1.45));
        return col;
    }

    if (tcode < 1.5) {
        let tw = max(ps_x * 0.54, 1.0);
        let th = ps_y * 2.45;
        col = paint(col, darker(tint_col, 0.30), shape_rect(pxy, vec2<f32>(sx, sy_base - th * 0.5), tw * 0.5, th * 0.5));
        let base_y = sy_base - th;
        col = paint(col, darker(tint_col, 0.64), shape_triangle(pxy, sx, base_y - 0.0 * ps_y - 2.4 * ps_y, 3.6 * ps_x, 2.4 * ps_y));
        col = paint(col, darker(tint_col, 0.76), shape_triangle(pxy, sx, base_y - 1.4 * ps_y - 2.2 * ps_y, 3.0 * ps_x, 2.2 * ps_y));
        col = paint(col, darker(tint_col, 0.90), shape_triangle(pxy, sx, base_y - 2.7 * ps_y - 2.0 * ps_y, 2.4 * ps_x, 2.0 * ps_y));
        col = paint(col, lighter(tint_col, 1.08), shape_triangle(pxy, sx, base_y - 4.1 * ps_y, 1.8 * ps_x, 1.6 * ps_y));
        return col;
    }

    if (tcode < 2.5) {
        let rx = ps_x * 2.2;
        let ry = ps_y * 1.7;
        col = paint(col, darker(tint_col, 0.60), shape_ellipse(pxy, vec2<f32>(sx - ps_x, sy_base - ps_y * 0.9), rx, ry * 0.60));
        col = paint(col, darker(tint_col, 0.75), shape_ellipse(pxy, vec2<f32>(sx + ps_x * 0.65, sy_base - ps_y * 0.7), rx * 0.85, ry * 0.55));
        col = paint(col, tint_col, shape_ellipse(pxy, vec2<f32>(sx, sy_base - ps_y * 1.3), rx * 0.90, ry * 0.68));
        col = paint(col, lighter(tint_col, 1.28), shape_ellipse(pxy, vec2<f32>(sx - ps_x * 0.3, sy_base - ps_y * 2.1), rx * 0.65, ry * 0.58));
        return col;
    }

    if (tcode < 4.5) {
        let is_boulder = tcode >= 3.5;
        let s_mul = select(1.0, 1.7, is_boulder);
        let pxr = ps_x * s_mul;
        let pyr = ps_y * s_mul;
        let vr = clamp(rock_var, 0.0, 1.0);
        let rx = pxr * (1.7 + 0.8 * vr);
        let ry = pyr * (1.1 + 0.7 * (1.0 - vr));
        let cy = sy_base - ry * (0.72 + 0.12 * vr);
        let skew = (vr * 2.0 - 1.0) * pxr * 0.45;
        col = paint(col, darker(tint_col, 0.52), shape_ellipse(pxy, vec2<f32>(sx + skew * 0.3, cy + pyr * 0.20), rx * 1.02, ry * 0.98));
        col = paint(col, tint_col, shape_ellipse(pxy, vec2<f32>(sx - skew * 0.2, cy - pyr * 0.02), rx * 0.86, ry * 0.82));
        col = paint(col, lighter(tint_col, 1.28), shape_ellipse(pxy, vec2<f32>(sx - rx * 0.36, cy - ry * 0.30), rx * 0.26, ry * 0.22));
        col = paint(col, lighter(tint_col, 1.16), shape_ellipse(pxy, vec2<f32>(sx + rx * 0.18, cy - ry * 0.22), rx * 0.18, ry * 0.15));
        let crack = darker(tint_col, 0.34);
        col = paint(col, crack, shape_line(pxy, vec2<f32>(sx - rx * 0.30, cy - ry * 0.08), vec2<f32>(sx + rx * 0.12, cy + ry * 0.06)));
        col = paint(col, crack, shape_line(pxy, vec2<f32>(sx - rx * 0.02, cy - ry * 0.22), vec2<f32>(sx + rx * 0.22, cy + ry * 0.12)));
        if (vr > 0.45) {
            col = paint(col, crack, shape_line(pxy, vec2<f32>(sx - rx * 0.20, cy + ry * 0.05), vec2<f32>(sx + rx * 0.06, cy + ry * 0.23)));
        }
        return col;
    }

    if (tcode < 5.5) {
        let tw = max(ps_x * 0.95, 1.0);
        let th = ps_y * 5.2;
        col = paint(col, tint_col, shape_rect(pxy, vec2<f32>(sx, sy_base - th * 0.5), tw * 0.5, th * 0.5));
        let arm_y = sy_base - th * 0.55;
        let aw = max(ps_x * 0.7, 1.0);
        col = paint(col, tint_col, shape_rect(pxy, vec2<f32>(sx - tw * 0.5 - ps_x * 1.1, arm_y + aw * 0.5), ps_x * 1.1, aw * 0.5));
        col = paint(col, tint_col, shape_rect(pxy, vec2<f32>(sx - tw * 0.5 - ps_x * 2.2 + aw * 0.5, arm_y - ps_y * 0.8), aw * 0.5, ps_y * 0.8));
        let arm_y2 = sy_base - th * 0.40;
        col = paint(col, tint_col, shape_rect(pxy, vec2<f32>(sx + tw * 0.5 + ps_x * 0.95, arm_y2), ps_x * 0.95, aw * 0.5));
        col = paint(col, tint_col, shape_rect(pxy, vec2<f32>(sx + tw * 0.5 + ps_x * 1.9 - aw * 0.5, arm_y2 - ps_y * 0.9 - aw * 0.25), aw * 0.5, ps_y * 0.9 - aw * 0.25));
        col = paint(col, lighter(tint_col, 1.2), shape_ellipse(pxy, vec2<f32>(sx, sy_base - th), tw * 0.75, tw * 0.75));
        return col;
    }

    if (tcode < 6.5) {
        let tw = max(ps_x * 0.68, 1.0);
        let th = ps_y * 5.2;
        col = paint(col, tint_col, shape_rect(pxy, vec2<f32>(sx, sy_base - th * 0.5), tw * 0.5, th * 0.5));
        let col_light = lighter(tint_col, 1.3);
        let b0a = vec2<f32>(sx, sy_base - th * 0.74);
        let b0b = vec2<f32>(sx + (-1.2) * ps_x * 4.0 * 0.42, b0a.y + (-1.0) * ps_y * 4.0 * 0.42);
        col = paint(col, col_light, shape_line(pxy, b0a, b0b));
        col = paint(col, darker(col_light, 0.86), shape_line(pxy, b0a + vec2<f32>(1.0, 0.0), b0b + vec2<f32>(1.0, 0.0)));
        let b1a = vec2<f32>(sx, sy_base - th * 0.60);
        let b1b = vec2<f32>(sx + (1.5) * ps_x * 4.0 * 0.36, b1a.y + (-0.9) * ps_y * 4.0 * 0.36);
        col = paint(col, col_light, shape_line(pxy, b1a, b1b));
        col = paint(col, darker(col_light, 0.86), shape_line(pxy, b1a + vec2<f32>(1.0, 0.0), b1b + vec2<f32>(1.0, 0.0)));
        let b2a = vec2<f32>(sx, sy_base - th * 0.46);
        let b2b = vec2<f32>(sx + (-0.8) * ps_x * 4.0 * 0.27, b2a.y + (-0.7) * ps_y * 4.0 * 0.27);
        col = paint(col, col_light, shape_line(pxy, b2a, b2b));
        col = paint(col, darker(col_light, 0.86), shape_line(pxy, b2a + vec2<f32>(1.0, 0.0), b2b + vec2<f32>(1.0, 0.0)));
        let b3a = vec2<f32>(sx, sy_base - th * 0.34);
        let b3b = vec2<f32>(sx + (1.0) * ps_x * 4.0 * 0.23, b3a.y + (-0.55) * ps_y * 4.0 * 0.23);
        col = paint(col, col_light, shape_line(pxy, b3a, b3b));
        col = paint(col, darker(col_light, 0.86), shape_line(pxy, b3a + vec2<f32>(1.0, 0.0), b3b + vec2<f32>(1.0, 0.0)));
        return col;
    }

    let sw = max(ps_x * 0.58, 1.0);
    let sh = ps_y * 2.0;
    let stem_col = vec3<f32>(225.0 / 255.0, 215.0 / 255.0, 195.0 / 255.0);
    col = paint(col, stem_col, shape_rect(pxy, vec2<f32>(sx, sy_base - sh * 0.5), sw * 0.5, sh * 0.5));
    let crx = ps_x * 1.95;
    let cry = ps_y * 1.25;
    let cap_cy = sy_base - sh - cry * 0.42;
    col = paint(col, darker(tint_col, 0.72), shape_ellipse(pxy, vec2<f32>(sx + crx * 0.10, cap_cy + cry * 0.08), crx, cry));
    col = paint(col, tint_col, shape_ellipse(pxy, vec2<f32>(sx - crx * 0.08, cap_cy - cry * 0.06), crx * 0.86, cry * 0.80));
    col = paint(col, vec3<f32>(245.0 / 255.0, 245.0 / 255.0, 245.0 / 255.0), shape_ellipse(pxy, vec2<f32>(sx - crx * 0.22, cap_cy - cry * 0.32), ps_x * 0.30, ps_y * 0.24));
    col = paint(col, vec3<f32>(245.0 / 255.0, 245.0 / 255.0, 245.0 / 255.0), shape_ellipse(pxy, vec2<f32>(sx + crx * 0.32, cap_cy - cry * 0.10), ps_x * 0.22, ps_y * 0.18));
    col = paint(col, vec3<f32>(240.0 / 255.0, 240.0 / 255.0, 240.0 / 255.0), shape_ellipse(pxy, vec2<f32>(sx - crx * 0.50, cap_cy + cry * 0.05), ps_x * 0.16, ps_y * 0.13));
    return col;
}

fn atmo_fixture_draw(
    pxy: vec2<f32>,
    base_col: vec3<f32>,
    tcode: f32,
    sx: f32,
    sy: f32,
    fr: f32,
    side: f32,
) -> vec3<f32> {
    var col = base_col;
    let post = vec3<f32>(58.0 / 255.0, 46.0 / 255.0, 34.0 / 255.0);
    let metal = vec3<f32>(115.0 / 255.0, 110.0 / 255.0, 96.0 / 255.0);
    let dark_metal = vec3<f32>(62.0 / 255.0, 58.0 / 255.0, 52.0 / 255.0);
    let warm = vec3<f32>(208.0 / 255.0, 148.0 / 255.0, 84.0 / 255.0);
    let sgn = select(-1.0, 1.0, side >= 0.0);

    // Torch/greenfire/magic/icewisp
    if (tcode < 1.5 || (tcode >= 4.0 && tcode < 5.5) || tcode >= 6.5) {
        let stem_h = max(fr * 1.6, 2.0);
        let cup_w = max(fr * 0.7, 1.0);
        let mx = sx - sgn * fr * 0.10;
        col = paint(col, dark_metal, shape_rect(pxy, vec2<f32>(sx - sgn * fr * 0.20, sy + fr * 0.15), abs(fr * 0.12), fr * 0.03));
        col = paint(col, metal, shape_rect(pxy, vec2<f32>(mx, sy + fr * 0.18), cup_w * 0.28, fr * 0.06));
        col = paint(col, post, shape_rect(pxy, vec2<f32>(mx, sy + fr * 0.24 + stem_h * 0.5), cup_w * 0.13, stem_h * 0.5));
        // Flame body, matching the CPU elongated additive flame profile.
        let flame_col = select(
            warm,
            vec3<f32>(74.0 / 255.0, 182.0 / 255.0, 88.0 / 255.0),
            tcode >= 4.0 && tcode < 5.5,
        );
        let flame_col2 = select(
            vec3<f32>(245.0 / 255.0, 222.0 / 255.0, 160.0 / 255.0),
            vec3<f32>(154.0 / 255.0, 238.0 / 255.0, 255.0 / 255.0),
            tcode >= 6.5,
        );
        let flame_cx = mx;
        let flame_cy = sy - fr * 0.01;
        let flame_outer = shape_ellipse(pxy, vec2<f32>(flame_cx, flame_cy + fr * 0.07), fr * 0.22, fr * 0.43);
        let flame_mid = shape_ellipse(pxy, vec2<f32>(flame_cx, flame_cy - fr * 0.04), fr * 0.14, fr * 0.26);
        let flame_tip = shape_ellipse(pxy, vec2<f32>(flame_cx, flame_cy - fr * 0.16), fr * 0.07, fr * 0.12);
        let flame_core = shape_ellipse(pxy, vec2<f32>(flame_cx, flame_cy - fr * 0.11), fr * 0.045, fr * 0.088);
        let flame_a = max(flame_strength(pxy, flame_cx, flame_cy, fr * 1.1, 0.0), flame_outer * 0.60);
        col = lerp3(col, dark_metal, clamp(flame_outer * 0.30, 0.0, 0.22));
        col = lerp3(col, mix(flame_col, flame_col2, 0.42), clamp(flame_a * 0.78 + flame_mid * 0.22, 0.0, 0.88));
        col = lerp3(col, flame_col2, clamp(flame_a * 0.48 + flame_tip * 0.34, 0.0, 0.74));
        col = lerp3(col, vec3<f32>(1.0, 0.98, 0.86), clamp(flame_core * 0.86 + flame_tip * 0.28, 0.0, 0.92));
        let pool_a = glow_strength(distance(pxy, vec2<f32>(mx, sy + fr * 0.14)), fr * 1.46, 0.84);
        col = lerp3(col, mix(flame_col, flame_col2, 0.30), clamp(pool_a * 0.20, 0.0, 0.28));
        return col;
    }

    // Lantern
    if (tcode < 2.5) {
        let ly = sy + fr * 0.12;
        let bar_left = sx - sgn * fr * 0.58;
        let bar_right = sx - sgn * fr * 0.22;
        let hook_x = (bar_left + bar_right) * 0.5;
        let lx = hook_x;
        let fx = sx - sgn * fr * 0.10;
        col = paint(col, dark_metal, shape_rect(pxy, vec2<f32>(bar_left - sgn * fr * 0.07, ly - fr * 0.18), fr * 0.05, fr * 0.12));
        col = paint(col, dark_metal, shape_rect(pxy, vec2<f32>((bar_left + bar_right) * 0.5, ly - fr * 0.20), abs(bar_right - bar_left) * 0.5, fr * 0.040));
        col = paint(col, metal, shape_line(pxy, vec2<f32>(bar_right - sgn * fr * 0.05, ly - fr * 0.20), vec2<f32>(bar_right, ly - fr * 0.06)));
        col = paint(col, metal, shape_line(pxy, vec2<f32>(bar_left - sgn * fr * 0.02, ly - fr * 0.24), vec2<f32>(hook_x, ly - fr * 0.04)));
        col = paint(col, metal, shape_ellipse(pxy, vec2<f32>(hook_x, ly - fr * 0.02), fr * 0.05, fr * 0.03));
        col = paint(col, metal, shape_rect(pxy, vec2<f32>(hook_x, ly - fr * 0.11), fr * 0.014, fr * 0.09));
        col = paint(col, metal, shape_rect(pxy, vec2<f32>(hook_x, ly + fr * 0.10), fr * 0.02, fr * 0.12));
        col = paint(col, dark_metal, shape_rect(pxy, vec2<f32>(lx, ly + fr * 0.20), fr * 0.18, fr * 0.04));
        col = paint(col, dark_metal, shape_rect(pxy, vec2<f32>(lx, ly + fr * 0.51), fr * 0.24, fr * 0.27));
        col = paint(col, vec3<f32>(192.0 / 255.0, 146.0 / 255.0, 82.0 / 255.0), shape_rect(pxy, vec2<f32>(lx, ly + fr * 0.50), fr * 0.14, fr * 0.20));
        col = paint(col, dark_metal, shape_rect(pxy, vec2<f32>(lx, ly + fr * 0.82), fr * 0.18, fr * 0.04));
        col = paint(col, warm, shape_ellipse(pxy, vec2<f32>(lx, ly + fr * 0.52), fr * 0.12, fr * 0.21));
        col = lerp3(col, warm, clamp(glow_strength(distance(pxy, vec2<f32>(fx, ly + fr * 0.20)), fr * 1.70, 0.85) * 0.34, 0.0, 0.38));
        return col;
    }

    // Firefly: tiny bioluminescent mote near mount, no heavy fixture body.
    if (tcode < 3.5) {
        let core = vec3<f32>(112.0 / 255.0, 238.0 / 255.0, 94.0 / 255.0);
        col = paint(col, core, shape_ellipse(pxy, vec2<f32>(sx, sy + fr * 0.12), fr * 0.08, fr * 0.12));
        col = lerp3(col, core, clamp(glow_strength(distance(pxy, vec2<f32>(sx, sy + fr * 0.16)), fr * 1.25, 0.82), 0.0, 0.5));
        return col;
    }

    // Candle
    if (tcode >= 5.5 && tcode < 6.5) {
        let cy = sy + fr * 0.28;
        col = paint(col, dark_metal, shape_rect(pxy, vec2<f32>(sx, cy + fr * 0.11), fr * 0.16, fr * 0.05));
        col = paint(col, vec3<f32>(224.0 / 255.0, 212.0 / 255.0, 185.0 / 255.0), shape_rect(pxy, vec2<f32>(sx, cy - fr * 0.065), fr * 0.06, fr * 0.115));
        return col;
    }

    let glow_color = select(
        vec3<f32>(208.0 / 255.0, 148.0 / 255.0, 84.0 / 255.0),
        vec3<f32>(74.0 / 255.0, 182.0 / 255.0, 88.0 / 255.0),
        tcode >= 4.0 && tcode < 5.5,
    );
    let glow_mix = glow_strength(distance(pxy, vec2<f32>(sx, sy)), fr * 3.8, 0.75);
    return lerp3(col, glow_color, clamp(glow_mix * 0.28, 0.0, 0.45));
}

fn cloud_rng_next(state: ptr<function, u32>) -> f32 {
    var s = *state;
    s = s ^ (s << 13u);
    s = s ^ (s >> 17u);
    s = s ^ (s << 5u);
    *state = s;
    return f32(s) / 4294967296.0;
}

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= params.dim.x || gid.y >= params.dim.y) {
        return;
    }

    let w = f32(params.dim.x);
    let h = f32(params.dim.y);
    let x = f32(gid.x);
    let y = f32(gid.y);
    let cx = w * 0.5;
    let horizon = params.time_scroll.z;
    let curve = clamp(params.time_scroll.w, -1.0, 1.0);
    let max_hw = params.scene0.x;
    let cam_h = params.scene0.y;
    let focal_mult = params.scene0.z;
    let path_power = max(params.scene0.w, 0.05);
    let focal = (h - horizon) * focal_mult;
    let rt_lighting = params.atmo_scene.w > 0.5;
    let rt_shadows = params.wall_data.w > 0.5;
    let has_emitter = params.loop_data.z > 0.5;
    let light_energy = select(0.0, clamp(params.loop_data.y, 0.0, 2.5), rt_lighting && has_emitter);
    let light_dir = clamp(params.atmo_tint.w, -1.0, 1.0);
    let shadow_dir = -light_dir;
    let shadow_len = (0.90 + light_energy * 0.35) * clamp(params.loop_data.w, 0.55, 6.5);

    var col = params.void_color.rgb;
    var prop_covered = false;

    // Above-horizon wall band: use the same wall shading model as below horizon.
    if (params.dim.w == 1u && y < horizon) {
        let ws = max(horizon - horizon * clamp(params.wall_data.x, 0.0, 1.0), 0.0);
        if (y >= ws) {
            let edge = abs(x - cx);
            let dl = max(edge, 0.5);
            let wz_w = focal * params.misc.z / dl;
            let wy_w = cam_h - (y - horizon) * wz_w / max(focal, 0.001);
            let base_col = sample_tile_tex(
                wall_tex,
                wy_w * params.tex_data.y,
                (wz_w + params.time_scroll.x * params.misc.x) * params.tex_data.y,
                params.tex_flags.y > 0u,
            );
            let ped = max(edge, 0.0);
            let base = clamp(params.misc.w / max(wz_w, 0.1), 0.0, 1.0)
                * clamp(0.42 + ped / max(params.wall_data.y, 1.0), 0.0, 1.0);
            let depth_near = 1.0 - clamp((y - ws) / max(horizon - ws, 1.0), 0.0, 1.0);
            let atmo_boost = params.atmo_scene.z * params.atmo_scene.x * pow(depth_near, 0.65) * 0.85;
            let ambient = clamp(params.misc.y + atmo_boost, 0.08, 2.2);
            let emitter_boost = light_energy * 0.16 * pow(depth_near, 0.60);
            let bright = clamp(base * (ambient + emitter_boost), 0.04, 1.7);
            let tint_a = clamp(params.atmo_scene.z * params.atmo_scene.y * depth_near * 0.20, 0.0, 0.35);
            let base_t = lerp3(base_col, params.atmo_tint.rgb, tint_a);
            col = base_t * bright;
        }
    }

    if (params.dim.z == 1u && y < horizon) {
        let t = clamp(y / max(horizon, 1.0), 0.0, 1.0);
        col = lerp3(params.sky_top.rgb, params.sky_horizon.rgb, t);

        if (params.sky_flags.x == 1u) {
            let sz = max(params.sky_misc.x, 0.05);
            let star_density = clamp(f32(params.sky_counts.x) / max(w * max(horizon, 1.0), 1.0), 0.0, 0.20);
            let npos = vec2<f32>(x / max(sz * 1.8, 0.2), y / max(sz * 1.8, 0.2));
            let cell = floor(npos);
            let hv = hash2(cell + vec2<f32>(f32(params.sky_counts.z & 1023u), f32((params.sky_counts.z >> 10u) & 1023u)));
            let th = 1.0 - star_density * 2.3;
            if (hv > th) {
                let tw = clamp(params.sky_misc.y, 0.0, 8.0);
                let phase = hash2(cell + vec2<f32>(13.7, 71.3)) * 6.2831853;
                let freq = 1.0 + floor(hash2(cell + vec2<f32>(17.3, 29.1)) * 9.0);
                let flick = pow(sin(params.time_scroll.y * 6.2831853 * freq + phase) * 0.5 + 0.5, 1.45);
                let pulse = (1.0 - min(tw * 0.22, 0.92)) + flick * min(tw * 0.35, 1.0);
                let uv = fract(npos) - vec2<f32>(0.5, 0.5);
                let d2 = dot(uv, uv);
                let shape = exp(-d2 * 26.0);
                let b = clamp((0.55 + 0.45 * pulse) * shape, 0.0, 1.0);
                col = col + vec3<f32>(b * 0.70, b * 0.72, b * 0.78);
            }
        }

        if (params.sky_flags.y == 1u) {
            var rng_state = (7001u + params.sky_counts.w) ^ 0xdeadbeefu;
            let count = min(params.sky_counts.y, 220u);
            let speed = clamp(params.sky_misc.z, 0.0, 8.0);
            let scale = clamp(params.sky_misc.w, 0.2, 6.0);
            let variation = clamp(params.moon_misc.z, 0.0, 1.0);
            let alpha_base = clamp(params.cloud_tint.w, 0.0, 2.0);
            let span = w + 180.0;
            for (var ci: u32 = 0u; ci < count; ci = ci + 1u) {
                let base_x = cloud_rng_next(&rng_state) * w;
                let cy = horizon * (0.10 + cloud_rng_next(&rng_state) * 0.55);
                let phase = cloud_rng_next(&rng_state);
                let drift_rng = 0.45 + cloud_rng_next(&rng_state) * 0.9;
                var drift = 0.0;
                if (params.sky_flags.w == 1u) {
                    let cycles = clamp(round(speed * drift_rng), 1.0, 16.0);
                    drift = params.time_scroll.y * span * cycles + phase * span;
                } else {
                    drift = params.time_scroll.y * speed * w * drift_rng + phase * w;
                }
                let cx_cloud = (base_x + drift) - floor((base_x + drift) / span) * span - 90.0;
                let sz = horizon * (0.028 + cloud_rng_next(&rng_state) * 0.055) * scale;
                let a_cloud = alpha_base * (0.7 + 0.3 * cloud_rng_next(&rng_state));

                let blob_count = 3 + i32(round(variation * 4.0));
                for (var bi: i32 = 0; bi < blob_count; bi = bi + 1) {
                    let bt = select(0.5, f32(bi) / max(f32(blob_count - 1), 1.0), blob_count > 1);
                    let px = (bt - 0.5) * (1.15 + variation * 0.95) + (variation * 0.25 * sin(bt * 13.0));
                    let py = (0.05 * cos(bt * 7.0)) * (0.8 + 0.5 * variation);
                    let sx = (0.50 + 0.60 * abs(sin(bt * 7.0))) * (0.9 + variation * 0.30) * sz;
                    let sy = (0.30 + 0.34 * abs(cos(bt * 5.0))) * (0.9 + variation * 0.25) * sz;
                    let ex = cx_cloud + px * sz;
                    let ey = cy + py * sz;
                    let dx = (x - ex) / max(sx, 0.001);
                    let dy = (y - ey) / max(sy, 0.001);
                    let d = dx * dx + dy * dy;
                    if (d < 1.0) {
                        let a = clamp(a_cloud * (0.40 + 0.15 * variation) * pow(1.0 - d, 1.2), 0.0, 1.0);
                        col = lerp3(col, params.cloud_tint.rgb, a);
                    }
                }
            }
        }

        let sun_vis = smoothstep(0.0, 0.08, params.sun_color.w);
        if (params.sun_data.w > 0.5 && sun_vis > 0.001) {
            let sun_cx = params.sun_data.x * w;
            let sun_cy = params.sun_data.y * horizon;
            let sun_r = max(params.sun_data.z * horizon, 1.0);
            let d = distance(vec2<f32>(x, y), vec2<f32>(sun_cx, sun_cy));
            let glow = clamp(1.0 - d / (sun_r * 2.3), 0.0, 1.0);
            col = col + params.sun_color.rgb * glow * 0.35 * sun_vis;
        }

        let moon_vis = smoothstep(0.0, 0.08, params.moon_color.w);
        let moon_alpha = clamp(params.moon_data.w, 0.0, 2.0) * moon_vis;
        if (moon_alpha > 0.01) {
            let moon_cx = params.moon_data.x * w;
            let moon_cy = params.moon_data.y * horizon;
            let moon_r = max(params.moon_data.z * horizon, 1.0);
            let pxy = vec2<f32>(x, y);
            let d = distance(pxy, vec2<f32>(moon_cx, moon_cy));
            let glow = clamp(1.0 - d / (moon_r * 2.2), 0.0, 1.0);
            col = col + params.moon_color.rgb * glow * 0.22 * moon_alpha;

            let nx = (x - moon_cx) / moon_r;
            let ny = (y - moon_cy) / moon_r;
            let d_main = nx * nx + ny * ny;
            if (d_main <= 1.0) {
                let phase = clamp(params.moon_misc.x, -1.0, 1.0);
                let sx = -phase * 2.0;
                let d_lit = (nx - sx) * (nx - sx) + ny * ny;
                let lit = clamp(1.0 - d_lit, 0.0, 1.0);
                let rim = clamp(1.0 - d_main, 0.0, 1.0);
                let aa = min(rim * 2.5, 1.0);
                let pa = moon_alpha * lit * aa;
                if (pa > 0.0001) {
                    var moon_col = params.moon_color.rgb;
                    if (params.sky_flags.z == 1u) {
                        let ts = clamp(params.moon_misc.y, 0.2, 4.0);
                        let coarse = fbm2(vec2<f32>(nx * 4.2 * ts, ny * 4.2 * ts) + vec2<f32>(3.7, 11.1));
                        let medium = fbm2(vec2<f32>(nx * 8.5 * ts + 9.7, ny * 8.5 * ts - 3.1) + vec2<f32>(19.3, -5.4));
                        let fine = fbm2(vec2<f32>(nx * 15.0 * ts - 2.4, ny * 15.0 * ts + 7.9) + vec2<f32>(7.1, 23.9));
                        let rim_b = clamp(1.0 - d_main, 0.0, 1.0);
                        var crater = 0.88 + coarse * 0.06 + medium * 0.04 + fine * 0.02 + (rim_b - 0.5) * 0.015;
                        crater = clamp(crater, 0.55, 1.08);
                        moon_col = moon_col * crater;
                    }
                    col = lerp3(col, moon_col, clamp(pa, 0.0, 1.0));
                }
            }
        }
    } else if (y >= horizon + 1.0) {
        let p = max(y - horizon, 1.0);
        let t = clamp((y - horizon) / max(h - horizon, 1.0), 0.0, 1.0);
        let width_w = path_width_weight(t);
        let phw = max_hw * width_w * pow(max(2.0 * t - t * t, 0.0), path_power);
        let curve_shift = path_curve_shift(curve, max_hw, t);
        let row_cx = cx + curve_shift;

        let d = cam_h * focal / p;
        let use_curve_sampling = abs(curve) > 0.001;
        let off = world_curve_offset(x, y, w, h, horizon, curve);
        let p_adj = max((y - off) - horizon, 1.0);
        let d_use = select(d, cam_h * focal / p_adj, use_curve_sampling);
        let wx = (x - row_cx) / max(focal, 0.001) * d_use;
        let wz = d_use + params.time_scroll.x * params.misc.x;
        let edge = abs(x - row_cx);

        if (edge <= phw) {
            let base = sample_tile_tex(
                floor_tex,
                wx * params.tex_data.x,
                wz * params.tex_data.x,
                params.tex_flags.x > 0u,
            );
            let p_norm = (y - horizon) / max(h - horizon, 1.0);
            let ds = min(max(p_norm, 0.0) * max(params.floor_data.x, 0.05), 1.0);
            let dc = abs(x - row_cx);
            let es = sqrt(max((phw - dc) / max(phw, 0.001), 0.0));
            let edge_shade = ds * ((1.0 - clamp(params.floor_data.y, 0.0, 1.0)) + clamp(params.floor_data.y, 0.0, 1.0) * es);
            let depth = clamp(1.0 - t, 0.0, 1.0);
            let atmo_boost = params.atmo_scene.z * params.atmo_scene.x * pow(depth, 0.7);
            let emitter_boost = light_energy * 0.20 * pow(depth, 0.65);
            let ambient = clamp(params.misc.y + atmo_boost, 0.08, 2.4);
            let shade = clamp(edge_shade * (ambient + emitter_boost), 0.03, 2.2);
            let tint_a = clamp(params.atmo_scene.z * params.atmo_scene.y * depth * 0.30, 0.0, 0.45);
            let base_t = lerp3(base, params.atmo_tint.rgb, tint_a);
            col = base_t * shade;
        } else if (params.dim.w == 1u) {
            let dl = max(edge, 0.5);
            let wz_w = focal * params.misc.z / dl;
            let wy_w = cam_h - (y - horizon) * wz_w / max(focal, 0.001);
            let base = sample_tile_tex(
                wall_tex,
                wy_w * params.tex_data.y,
                (wz_w + params.time_scroll.x * params.misc.x) * params.tex_data.y,
                params.tex_flags.y > 0u,
            );
            let ped = max(edge - phw, 0.0);
            let base_bright = clamp(params.misc.w / max(wz_w, 0.1), 0.0, 1.0)
                * clamp(0.42 + ped / max(params.wall_data.y, 1.0), 0.0, 1.0);
            let depth = clamp(1.0 - t, 0.0, 1.0);
            let atmo_boost = params.atmo_scene.z * params.atmo_scene.x * pow(depth, 0.65) * 0.85;
            let emitter_boost = light_energy * 0.16 * pow(depth, 0.60);
            let ambient = clamp(params.misc.y + atmo_boost, 0.08, 2.2);
            let bright = clamp(base_bright * (ambient + emitter_boost), 0.04, 1.7);
            let tint_a = clamp(params.atmo_scene.z * params.atmo_scene.y * depth * 0.20, 0.0, 0.35);
            let base_t = lerp3(base, params.atmo_tint.rgb, tint_a);
            col = base_t * bright;
        }

        // Native GPU props (up to 8 slots; sprite-backed slots are zeroed and skipped).
        for (var pi: i32 = 0; pi < i32(min(params.feature_counts.x, 8u)); pi = pi + 1) {
            let pc = params.prop_core[pi];
            if (pc.x < 0.5) { continue; }
            let tcode = pc.y;
            let mirror = pc.z > 0.5;
            let spc = max(pc.w, 0.1);
            let pg = params.prop_geom[pi];
            let ppos = params.prop_pos[pi];
            let wx = pg.x;
            let wz_min = pg.y;
            let wz_max = pg.z;
            let sc = pg.w;
            let pt = params.prop_tint[pi];
            let edge_gap = params.prop_misc[pi].x;
            let seed = max(params.prop_misc[pi].y, 0.0);
            let style_mix = clamp(params.prop_misc[pi].z, 0.0, 1.0);
            let style_bias = clamp(params.prop_misc[pi].w, -1.0, 1.0);
            let follow = clamp(pt.w, 0.0, 1.0);
            let prow = params.prop_rows[pi];
            let row_count = clamp(i32(round(prow.x)), 1, 64);
            let row_spacing = max(prow.y, 0.75);
            let pvar = params.prop_var[pi];
            let pvar2 = params.prop_var2[pi];
            let pflags = i32(round(pvar2.w));
            let x_jitter_enabled = (pflags & 1) != 0;
            let y_jitter_enabled = (pflags & 2) != 0;
            let emits_light = (pflags & 4) != 0;
            let casts_shadow = (pflags & 8) != 0;
            let pixel_hitbox = ppos.w > 0.5;
            let row_jitter = max(prow.z, 0.0) + select(0.0, pvar.w * 0.35, x_jitter_enabled);
            let n_lo = i32(ceil((params.time_scroll.x + wz_min) / spc));
            let n_hi = i32(floor((params.time_scroll.x + wz_max) / spc));
            for (var n_i: i32 = n_hi; n_i >= n_lo; n_i = n_i - 1) {
                let n = f32(n_i);
                let n_seed = loop_seed_index(n, spc);
                let tv = floor(hash2(vec2<f32>(seed + 2.0, f32(n_seed))) * 32.0) - 16.0;
                let tint = clamp(
                    pt.rgb + vec3<f32>(tv * 0.5 / 255.0, tv * 0.8 / 255.0, tv * 0.4 / 255.0),
                    vec3<f32>(0.0, 0.0, 0.0),
                    vec3<f32>(1.0, 1.0, 1.0),
                );

                let wz_i = n * spc - params.time_scroll.x + ppos.z;
                if (wz_i < wz_min || wz_i > wz_max) { continue; }
                let sv = hash2(vec2<f32>(seed, f32(n_seed)));
                let sc_v = sc * (1.0 + (sv * 2.0 - 1.0) * max(pvar.z, 0.0));
                let wv = 1.0 + (hash2(vec2<f32>(seed + 41.0, f32(n_seed))) * 2.0 - 1.0) * max(pvar2.y, 0.0);
                let hv = 1.0 + (hash2(vec2<f32>(seed + 43.0, f32(n_seed))) * 2.0 - 1.0) * max(pvar2.z, 0.0);
                let ps_x = focal * sc_v * max(pvar.x, 0.02) * max(wv, 0.02) / max(wz_i, 0.001);
                let ps_y = focal * sc_v * max(pvar.y, 0.02) * max(hv, 0.02) / max(wz_i, 0.001);
                let ps = (ps_x + ps_y) * 0.5;
                if (ps < 0.8) { continue; }
                let y_sink = clamp(prow.w, 0.0, 6.0);
                let sy_floor = horizon + focal * (cam_h - ppos.y) / max(wz_i, 0.001);
                let yjit = select(
                    0.0,
                    (hash2(vec2<f32>(seed + 7.0, f32(n_seed))) * 2.0 - 1.0) * pvar2.x,
                    y_jitter_enabled,
                );
                let sy_base = sy_floor + y_sink * ps_y * 0.9 + yjit * ps_y;
                let path_t = clamp((sy_floor - horizon) / max(h - horizon, 1.0), 0.0, 1.0);
                let row_width_w = path_width_weight(path_t);
                let row_phw = max_hw * row_width_w * pow(max(2.0 * path_t - path_t * path_t, 0.0), path_power);
                let row_cx2 = cx;
                let pos_scale = focal / max(wz_i, 0.001);
                let edge_inner_px = row_phw + edge_gap * ps_x;
                let base_abs = edge_inner_px / max(pos_scale, 0.001);

                for (var si: i32 = 0; si < 2; si = si + 1) {
                    if (si == 1 && !mirror) { continue; }
                    let sgn = select(1.0, -1.0, si == 1);
                    let jit_seed = select(seed + 4.0, seed + 3.0, si == 0);
                    let jit = select(
                        0.0,
                        (hash2(vec2<f32>(jit_seed, f32(n_seed))) * 2.0 - 1.0) * pvar.w,
                        x_jitter_enabled,
                    );
                    var draw_tcode = tcode;
                    if ((tcode < 1.5 || (tcode >= 6.0 && tcode < 6.5)) && style_mix > 0.0) {
                        let side_key = n_seed * 3 + select(0, 1, si == 1);
                        let pick = hash2(vec2<f32>(seed + 55.0, f32(side_key)));
                        if (pick < style_mix) {
                            let pine_weight = clamp(0.5 + 0.45 * style_bias, 0.05, 0.95);
                            let bias_pick = hash2(vec2<f32>(seed + 63.0, f32(n_seed * 7)));
                            draw_tcode = select(6.0, 1.0, bias_pick < pine_weight);
                        } else {
                            draw_tcode = 0.0;
                        }
                    }
                    for (var ri: i32 = 0; ri < row_count; ri = ri + 1) {
                        let row_seed = hash2(vec2<f32>(
                            seed + 211.0 + select(0.0, 1.0, si == 1),
                            f32(n_seed * 19 + ri),
                        ));
                        let row_offset = f32(ri) * row_spacing;
                        let row_jit = (row_seed * 2.0 - 1.0) * row_jitter;
                        let row_abs = max(base_abs + row_offset + row_jit, 0.0);
                        let row_wx = sgn * row_abs;
                        let row_edge_px = sgn * (row_abs - base_abs) * pos_scale;
                        let sx_world = row_cx2 + focal * (row_wx + ppos.x + jit) / max(wz_i, 0.001);
                        let sx_edge = row_cx2 + sgn * edge_inner_px + row_edge_px + jit * pos_scale * 0.85;
                        let sx = mix(sx_world, sx_edge, follow);
                        var tree_shadow_mode = 0.0;
                        if (draw_tcode < 0.5) {
                            tree_shadow_mode = 1.0;
                        } else if (draw_tcode < 1.5) {
                            tree_shadow_mode = 2.0;
                        } else if (draw_tcode >= 6.0 && draw_tcode < 6.5) {
                            tree_shadow_mode = 3.0;
                        }
                        let treeish = tree_shadow_mode > 0.5;
                        if (casts_shadow && rt_shadows && has_emitter) {
                            let shadow = prop_ground_shadow_strength(
                                vec2<f32>(x, y),
                                horizon,
                                sx,
                                sy_floor,
                                ps_x,
                                ps_y,
                                shadow_dir,
                                params.moon_misc.w,
                                shadow_len,
                                pixel_hitbox,
                                draw_tcode,
                                params.prop_shadow_profile0[pi],
                                params.prop_shadow_profile1[pi],
                            );
                            if (shadow > 0.0) {
                                col = lerp3(col, vec3<f32>(12.0 / 255.0, 10.0 / 255.0, 8.0 / 255.0), shadow * 0.46);
                            }
                        }
                        let rock_var = hash2(vec2<f32>(
                            seed + 101.0,
                            f32(n_seed * 13 + select(0, 1, si == 1)),
                        ));
                        let col_before = col;
                        col = prop_draw(vec2<f32>(x, y), col, tint, draw_tcode, sx, sy_base, ps_x, ps_y, rock_var);
                        let root_contact = prop_root_contact_strength(
                            vec2<f32>(x, y),
                            horizon,
                            sx,
                            sy_floor,
                            ps_x,
                            ps_y,
                            clamp(0.22 + y_sink * 0.12, 0.0, 1.0),
                            treeish,
                        );
                        if (root_contact > 0.0) {
                            col = lerp3(col, vec3<f32>(14.0 / 255.0, 12.0 / 255.0, 9.0 / 255.0), root_contact * 0.26);
                        }
                        if (emits_light && rt_lighting) {
                            let glow = glow_strength(distance(vec2<f32>(x, y), vec2<f32>(sx, sy_base - ps_y * 1.2)), ps_y * 2.6, 0.42);
                            col = col + tint * glow * 0.14;
                        }
                        if (any(abs(col - col_before) > vec3<f32>(0.0005, 0.0005, 0.0005))) {
                            prop_covered = true;
                        }
                    }
                }
            }
        }
    }

    // Native GPU props (sky rows): keeps upper silhouettes from being clipped at the horizon.
    if (y < horizon + 1.0) {
        for (var pi: i32 = 0; pi < i32(min(params.feature_counts.x, 8u)); pi = pi + 1) {
            let pc = params.prop_core[pi];
            if (pc.x < 0.5) { continue; }
            let tcode = pc.y;
            let mirror = pc.z > 0.5;
            let spc = max(pc.w, 0.1);
            let pg = params.prop_geom[pi];
            let ppos = params.prop_pos[pi];
            let wz_min = pg.y;
            let wz_max = pg.z;
            let sc = pg.w;
            let pt = params.prop_tint[pi];
            let edge_gap = params.prop_misc[pi].x;
            let seed = max(params.prop_misc[pi].y, 0.0);
            let style_mix = clamp(params.prop_misc[pi].z, 0.0, 1.0);
            let style_bias = clamp(params.prop_misc[pi].w, -1.0, 1.0);
            let follow = clamp(pt.w, 0.0, 1.0);
            let prow = params.prop_rows[pi];
            let row_count = clamp(i32(round(prow.x)), 1, 64);
            let row_spacing = max(prow.y, 0.75);
            let pvar = params.prop_var[pi];
            let pvar2 = params.prop_var2[pi];
            let pflags = i32(round(pvar2.w));
            let x_jitter_enabled = (pflags & 1) != 0;
            let y_jitter_enabled = (pflags & 2) != 0;
            let emits_light = (pflags & 4) != 0;
            let casts_shadow = (pflags & 8) != 0;
            let pixel_hitbox = ppos.w > 0.5;
            let row_jitter = max(prow.z, 0.0) + select(0.0, pvar.w * 0.35, x_jitter_enabled);
            let n_lo = i32(ceil((params.time_scroll.x + wz_min) / spc));
            let n_hi = i32(floor((params.time_scroll.x + wz_max) / spc));
            for (var n_i: i32 = n_hi; n_i >= n_lo; n_i = n_i - 1) {
                let n = f32(n_i);
                let n_seed = loop_seed_index(n, spc);
                let tv = floor(hash2(vec2<f32>(seed + 2.0, f32(n_seed))) * 32.0) - 16.0;
                let tint = clamp(
                    pt.rgb + vec3<f32>(tv * 0.5 / 255.0, tv * 0.8 / 255.0, tv * 0.4 / 255.0),
                    vec3<f32>(0.0, 0.0, 0.0),
                    vec3<f32>(1.0, 1.0, 1.0),
                );

                let wz_i = n * spc - params.time_scroll.x + ppos.z;
                if (wz_i < wz_min || wz_i > wz_max) { continue; }
                let sv = hash2(vec2<f32>(seed, f32(n_seed)));
                let sc_v = sc * (1.0 + (sv * 2.0 - 1.0) * max(pvar.z, 0.0));
                let wv = 1.0 + (hash2(vec2<f32>(seed + 41.0, f32(n_seed))) * 2.0 - 1.0) * max(pvar2.y, 0.0);
                let hv = 1.0 + (hash2(vec2<f32>(seed + 43.0, f32(n_seed))) * 2.0 - 1.0) * max(pvar2.z, 0.0);
                let ps_x = focal * sc_v * max(pvar.x, 0.02) * max(wv, 0.02) / max(wz_i, 0.001);
                let ps_y = focal * sc_v * max(pvar.y, 0.02) * max(hv, 0.02) / max(wz_i, 0.001);
                let ps = (ps_x + ps_y) * 0.5;
                if (ps < 0.8) { continue; }
                let y_sink = clamp(prow.w, 0.0, 6.0);
                let sy_floor = horizon + focal * (cam_h - ppos.y) / max(wz_i, 0.001);
                let yjit = select(
                    0.0,
                    (hash2(vec2<f32>(seed + 7.0, f32(n_seed))) * 2.0 - 1.0) * pvar2.x,
                    y_jitter_enabled,
                );
                let sy_base = sy_floor + y_sink * ps_y * 0.9 + yjit * ps_y;
                let path_t = clamp((sy_floor - horizon) / max(h - horizon, 1.0), 0.0, 1.0);
                let row_width_w = path_width_weight(path_t);
                let row_phw = max_hw * row_width_w * pow(max(2.0 * path_t - path_t * path_t, 0.0), path_power);
                let row_cx2 = cx;
                let pos_scale = focal / max(wz_i, 0.001);
                let edge_inner_px = row_phw + edge_gap * ps_x;
                let base_abs = edge_inner_px / max(pos_scale, 0.001);

                for (var si: i32 = 0; si < 2; si = si + 1) {
                    if (si == 1 && !mirror) { continue; }
                    let sgn = select(1.0, -1.0, si == 1);
                    let jit_seed = select(seed + 4.0, seed + 3.0, si == 0);
                    let jit = select(
                        0.0,
                        (hash2(vec2<f32>(jit_seed, f32(n_seed))) * 2.0 - 1.0) * pvar.w,
                        x_jitter_enabled,
                    );
                    var draw_tcode = tcode;
                    if ((tcode < 1.5 || (tcode >= 6.0 && tcode < 6.5)) && style_mix > 0.0) {
                        let side_key = n_seed * 3 + select(0, 1, si == 1);
                        let pick = hash2(vec2<f32>(seed + 55.0, f32(side_key)));
                        if (pick < style_mix) {
                            let pine_weight = clamp(0.5 + 0.45 * style_bias, 0.05, 0.95);
                            let bias_pick = hash2(vec2<f32>(seed + 63.0, f32(n_seed * 7)));
                            draw_tcode = select(6.0, 1.0, bias_pick < pine_weight);
                        } else {
                            draw_tcode = 0.0;
                        }
                    }
                    for (var ri: i32 = 0; ri < row_count; ri = ri + 1) {
                        let row_seed = hash2(vec2<f32>(
                            seed + 211.0 + select(0.0, 1.0, si == 1),
                            f32(n_seed * 19 + ri),
                        ));
                        let row_offset = f32(ri) * row_spacing;
                        let row_jit = (row_seed * 2.0 - 1.0) * row_jitter;
                        let row_abs = max(base_abs + row_offset + row_jit, 0.0);
                        let row_wx = sgn * row_abs;
                        let row_edge_px = sgn * (row_abs - base_abs) * pos_scale;
                        let sx_world = row_cx2 + focal * (row_wx + ppos.x + jit) / max(wz_i, 0.001);
                        let sx_edge = row_cx2 + sgn * edge_inner_px + row_edge_px + jit * pos_scale * 0.85;
                        let sx = mix(sx_world, sx_edge, follow);
                        var tree_shadow_mode = 0.0;
                        if (draw_tcode < 0.5) {
                            tree_shadow_mode = 1.0;
                        } else if (draw_tcode < 1.5) {
                            tree_shadow_mode = 2.0;
                        } else if (draw_tcode >= 6.0 && draw_tcode < 6.5) {
                            tree_shadow_mode = 3.0;
                        }
                        let treeish = tree_shadow_mode > 0.5;
                        if (casts_shadow && rt_shadows && has_emitter) {
                            let shadow = prop_ground_shadow_strength(
                                vec2<f32>(x, y),
                                horizon,
                                sx,
                                sy_floor,
                                ps_x,
                                ps_y,
                                shadow_dir,
                                params.moon_misc.w,
                                shadow_len,
                                pixel_hitbox,
                                draw_tcode,
                                params.prop_shadow_profile0[pi],
                                params.prop_shadow_profile1[pi],
                            );
                            if (shadow > 0.0) {
                                col = lerp3(col, vec3<f32>(12.0 / 255.0, 10.0 / 255.0, 8.0 / 255.0), shadow * 0.46);
                            }
                        }
                        let rock_var = hash2(vec2<f32>(
                            seed + 101.0,
                            f32(n_seed * 13 + select(0, 1, si == 1)),
                        ));
                        let col_before = col;
                        col = prop_draw(vec2<f32>(x, y), col, tint, draw_tcode, sx, sy_base, ps_x, ps_y, rock_var);
                        let root_contact = prop_root_contact_strength(
                            vec2<f32>(x, y),
                            horizon,
                            sx,
                            sy_floor,
                            ps_x,
                            ps_y,
                            clamp(0.22 + y_sink * 0.12, 0.0, 1.0),
                            treeish,
                        );
                        if (root_contact > 0.0) {
                            col = lerp3(col, vec3<f32>(14.0 / 255.0, 12.0 / 255.0, 9.0 / 255.0), root_contact * 0.26);
                        }
                        if (emits_light && rt_lighting) {
                            let glow = glow_strength(distance(vec2<f32>(x, y), vec2<f32>(sx, sy_base - ps_y * 1.2)), ps_y * 2.6, 0.42);
                            col = col + tint * glow * 0.14;
                        }
                        if (any(abs(col - col_before) > vec3<f32>(0.0005, 0.0005, 0.0005))) {
                            prop_covered = true;
                        }
                    }
                }
            }
        }
    }

    // Native GPU atmosphere fixtures + glow (up to 4 layers) for both sky and ground pixels.
    let d_seed = cam_h * focal / max(abs(y - horizon), 1.0);
    for (var ai: i32 = 0; ai < i32(min(params.feature_counts.y, 4u)); ai = ai + 1) {
        let ac = params.atmo_core[ai];
        let tcode = ac.x;
        if (tcode < 0.5) { continue; }
        let torch_h = ac.y;
        let spc = max(ac.z, 1.0);
        let emit_pack = floor(ac.w / 100.0);
        let emits = (emit_pack - 2.0 * floor(emit_pack * 0.5)) >= 0.5;
        let casts_shadow = floor(ac.w / 200.0) >= 0.5;
        let mount_pack = ac.w
            - select(0.0, 100.0, emits)
            - select(0.0, 200.0, casts_shadow);
        let mount_surface = i32(floor(mount_pack)) % 10;
        let mount_side = i32(floor(mount_pack / 10.0));
        let n_lo = i32(ceil((params.time_scroll.x + 0.12) / spc));
        let n_hi = i32(floor((params.time_scroll.x + 24.0) / spc));
        for (var n_i: i32 = n_lo; n_i <= n_hi; n_i = n_i + 1) {
            let n = f32(n_i);
            let wz_i = n * spc - params.time_scroll.x;
            let fr = focal * params.atmo_glow[ai].w / max(wz_i, 0.001);
            if (fr < 0.45) { continue; }
            let sy_wall = horizon + focal * (cam_h - torch_h) / max(wz_i, 0.001);
            let sy_floor = horizon + focal * cam_h / max(wz_i, 0.001);
            let sy = select(
                select(
                    select(sy_wall, sy_floor - focal * max(torch_h, 0.0) * 0.35 / max(wz_i, 0.001), mount_surface == 2),
                    horizon - focal * max(torch_h, 0.0) * 0.9 / max(wz_i, 0.001),
                    mount_surface == 3,
                ),
                sy_wall,
                mount_surface == 1,
            );
            let path_t = clamp((sy_floor - horizon) / max(h - horizon, 1.0), 0.0, 1.0);
            let row_cx2 = cx + path_curve_shift(curve, max_hw, path_t);
            let side_off = focal * params.misc.z / max(wz_i, 0.001);
            let attach_mul = select(select(select(0.45, 0.62, mount_surface == 2), 0.9, mount_surface == 3), 1.0, mount_surface == 1);
            let sxl = row_cx2 - side_off * attach_mul;
            let sxr = row_cx2 + side_off * attach_mul;
            let sxc = row_cx2;
            let draw_center = mount_side == 3 || (mount_surface != 1 && mount_side == 0);
            let draw_left = mount_side == 1 || mount_side == 0;
            let draw_right = mount_side == 2 || mount_side == 0;
            if (draw_left) {
                col = atmo_fixture_draw(vec2<f32>(x, y), col, tcode, sxl, sy, fr, -1.0);
            }
            if (draw_right) {
                col = atmo_fixture_draw(vec2<f32>(x, y), col, tcode, sxr, sy, fr, 1.0);
            }
            if (draw_center) {
                col = atmo_fixture_draw(vec2<f32>(x, y), col, tcode, sxc, sy, fr, 1.0);
            }
            let gcol = params.atmo_glow[ai].rgb;
            let pxy = vec2<f32>(x, y);
            let torch_like = (tcode < 1.5) || (tcode >= 4.0 && tcode < 5.5) || (tcode >= 6.5);
            let flame_rad = fr * select(3.0, 1.95, torch_like);
            let flame_peak = select(0.62, 0.78, torch_like);
            let gl = select(0.0, glow_strength(distance(pxy, vec2<f32>(sxl, sy)), flame_rad, flame_peak), draw_left);
            let gr = select(0.0, glow_strength(distance(pxy, vec2<f32>(sxr, sy)), flame_rad, flame_peak), draw_right);
            let gc = select(0.0, glow_strength(distance(pxy, vec2<f32>(sxc, sy)), flame_rad, flame_peak), draw_center);
            let fl = select(0.0, flame_strength(pxy, sxl, sy + fr * 0.02, fr * 1.1, 0.0), draw_left);
            let frg = select(0.0, flame_strength(pxy, sxr, sy + fr * 0.02, fr * 1.1, 0.0), draw_right);
            let fc = select(0.0, flame_strength(pxy, sxc, sy + fr * 0.02, fr * 1.1, 0.0), draw_center);
            if (emits && rt_lighting) {
                let surf_l = select(0.0, glow_strength(distance(pxy, vec2<f32>(sxl, sy + fr * 0.30)), fr * select(2.35, 1.50, torch_like), select(0.68, 0.80, torch_like)), draw_left);
                let surf_r = select(0.0, glow_strength(distance(pxy, vec2<f32>(sxr, sy + fr * 0.30)), fr * select(2.35, 1.50, torch_like), select(0.68, 0.80, torch_like)), draw_right);
                let surf_c = select(0.0, glow_strength(distance(pxy, vec2<f32>(sxc, sy + fr * 0.30)), fr * select(2.35, 1.50, torch_like), select(0.68, 0.80, torch_like)), draw_center);
                let mount_l = select(0.0, glow_strength(distance(pxy, vec2<f32>(sxl, sy + fr * 0.60)), fr * select(1.45, 1.05, torch_like), select(0.76, 0.84, torch_like)), draw_left);
                let mount_r = select(0.0, glow_strength(distance(pxy, vec2<f32>(sxr, sy + fr * 0.60)), fr * select(1.45, 1.05, torch_like), select(0.76, 0.84, torch_like)), draw_right);
                let mount_c = select(0.0, glow_strength(distance(pxy, vec2<f32>(sxc, sy + fr * 0.60)), fr * select(1.45, 1.05, torch_like), select(0.76, 0.84, torch_like)), draw_center);
                let top_l = select(0.0, glow_strength(distance(pxy, vec2<f32>(sxl, sy - fr * 0.36)), fr * 1.55, 0.70), draw_left);
                let top_r = select(0.0, glow_strength(distance(pxy, vec2<f32>(sxr, sy - fr * 0.36)), fr * 1.55, 0.70), draw_right);
                let top_c = select(0.0, glow_strength(distance(pxy, vec2<f32>(sxc, sy - fr * 0.36)), fr * 1.55, 0.70), draw_center);
                let hot_l = select(0.0, glow_strength(distance(pxy, vec2<f32>(sxl, sy + fr * 0.00)), fr * 0.54, 0.92), draw_left);
                let hot_r = select(0.0, glow_strength(distance(pxy, vec2<f32>(sxr, sy + fr * 0.00)), fr * 0.54, 0.92), draw_right);
                let hot_c = select(0.0, glow_strength(distance(pxy, vec2<f32>(sxc, sy + fr * 0.00)), fr * 0.54, 0.92), draw_center);
                let att_l = select(0.0, 1.0 / (1.0 + pow(distance(pxy, vec2<f32>(sxl, sy)) / max(fr * 2.3, 0.001), 2.0) * 3.2), draw_left);
                let att_r = select(0.0, 1.0 / (1.0 + pow(distance(pxy, vec2<f32>(sxr, sy)) / max(fr * 2.3, 0.001), 2.0) * 3.2), draw_right);
                let att_c = select(0.0, 1.0 / (1.0 + pow(distance(pxy, vec2<f32>(sxc, sy)) / max(fr * 2.3, 0.001), 2.0) * 3.2), draw_center);
                col = col + gcol * (gl * att_l + gr * att_r + gc * att_c) * select(0.24, 0.13, torch_like);
                col = col + gcol * (fl * att_l + frg * att_r + fc * att_c) * 0.14;
                col = col + gcol * (surf_l * att_l + surf_r * att_r + surf_c * att_c) * select(0.34, 0.23, torch_like);
                col = col + gcol * (mount_l * att_l + mount_r * att_r + mount_c * att_c) * select(0.28, 0.18, torch_like);
                col = col + gcol * (top_l * att_l + top_r * att_r + top_c * att_c) * select(0.20, 0.14, torch_like);
                col = col + gcol * (hot_l * att_l + hot_r * att_r + hot_c * att_c) * select(0.30, 0.46, torch_like);
            }
            if (rt_shadows && has_emitter && casts_shadow) {
                let slf = clamp(params.loop_data.w, 0.55, 6.5);
                let lox = shadow_dir * fr * 0.86 * slf;
                let loy = fr * 0.20 * slf;
                let shl = select(0.0, glow_strength(distance(pxy, vec2<f32>(sxl + lox, sy + loy)), fr * 0.66, 0.78), draw_left);
                let shr = select(0.0, glow_strength(distance(pxy, vec2<f32>(sxr + lox, sy + loy)), fr * 0.66, 0.78), draw_right);
                let shc = select(0.0, glow_strength(distance(pxy, vec2<f32>(sxc + lox, sy + loy)), fr * 0.66, 0.78), draw_center);
                let tail_l = select(0.0, glow_strength(distance(pxy, vec2<f32>(sxl + lox * 1.35, sy + loy * 1.25)), fr * 0.92, 0.78), draw_left);
                let tail_r = select(0.0, glow_strength(distance(pxy, vec2<f32>(sxr + lox * 1.35, sy + loy * 1.25)), fr * 0.92, 0.78), draw_right);
                let tail_c = select(0.0, glow_strength(distance(pxy, vec2<f32>(sxc + lox * 1.35, sy + loy * 1.25)), fr * 0.92, 0.78), draw_center);
                let sh = clamp((shl + shr + shc) * 0.22 + (tail_l + tail_r + tail_c) * 0.10, 0.0, 0.42);
                col = lerp3(col, vec3<f32>(12.0 / 255.0, 10.0 / 255.0, 8.0 / 255.0), sh);
            }
        }
    }

    // ── Grass at path edges ──────────────────────────────────────────────────
    let fy_g = y - horizon;
    if (params.grass_data.w > 0.5 && fy_g > 0.0) {
        let wz_g = focal * cam_h / max(fy_g, 1.0);
        let path_t_g = clamp(fy_g / max(f32(h) - horizon, 1.0), 0.0, 1.0);
        let width_w_g = path_width_weight(path_t_g);
        let phw_g = max_hw * width_w_g * pow(max(2.0 * path_t_g - path_t_g * path_t_g, 0.0), path_power);
        let gph = focal / max(wz_g, 0.001);
        let gdensity = params.grass_data.x;
        let gcol = params.grass_color.rgb;
        let row_cx_g = cx + path_curve_shift(curve, max_hw, path_t_g);
        for (var gs: i32 = 0; gs < 2; gs = gs + 1) {
            let sgn = select(-1.0, 1.0, gs == 1);
            let edge_x = row_cx_g + sgn * phw_g;
            if (sgn * (x - edge_x) <= 0.0) { continue; }
            let side_dist = abs(x - edge_x);
            if (side_dist > max(10.0, gph * 14.0)) { continue; }
            let tz = wz_g + params.time_scroll.x * params.misc.x;
            let tuft_seed = hash2(vec2<f32>(42.0, floor((tz * 0.5) + edge_x / 4.0)));
            let spawn_p = min(gdensity, 1.0);
            let r = hash2(vec2<f32>(911.0 + select(0.0, 1.0, gs == 1), floor(fy_g + tz)));
            if (r > spawn_p) { continue; }
            let jit_x = (fract(tuft_seed * 7.0) * 7.0 - 3.5) * 0.8;
            let tx = edge_x + jit_x;
            let th = clamp(focal * 0.085 / max(wz_g, 0.001), 2.0, 16.0) * params.grass_data.y;
            let tw = clamp(th * 0.62, 1.0, 7.5);
            let tuft = grass_tuft_strength(vec2<f32>(x, y), tx, y, tw, th, gdensity, tuft_seed, params.grass_data.z);
            if (tuft > 0.0 && !prop_covered) {
                let shade = 0.82 + 0.30 * (1.0 - clamp((y - fy_g) / max(th, 1.0), 0.0, 1.0));
                col = lerp3(col, gcol * shade, clamp(tuft * 0.98, 0.0, 0.84));
            }
            if (gdensity > 1.0) {
                let extra = i32(round((gdensity - 1.0) * 2.0));
                for (var k: i32 = 0; k < extra; k = k + 1) {
                    let es = hash2(vec2<f32>(tuft_seed + 71.0 + f32(k), f32(k)));
                    let ex = tx + (fract(es * 15.0) * 15.0 - 7.5) * 0.35;
                    let eh = th * (0.75 + fract(es * 255.0) * 0.5);
                    let ew = clamp(eh * 0.58, 1.0, 7.5);
                    let tuft2 = grass_tuft_strength(vec2<f32>(x, y), ex, y, ew, eh, gdensity, es, params.grass_data.z);
                    if (tuft2 > 0.0 && !prop_covered) {
                        let shade2 = 0.82 + 0.30 * (1.0 - clamp((y - fy_g) / max(eh, 1.0), 0.0, 1.0));
                        col = lerp3(col, gcol * shade2, clamp(tuft2 * 0.92, 0.0, 0.78));
                    }
                }
            }
        }
    }

    // ── Post-processing ──────────────────────────────────────────────────────
    // Saturation
    let sat = params.post_colors.w;
    if (abs(sat - 1.0) > 0.001) {
        let luma = dot(col, vec3<f32>(0.2126, 0.7152, 0.0722));
        col = lerp3(vec3<f32>(luma, luma, luma), col, sat);
    }
    // Bloom (single-pass bright response with emitter-aware lift)
    let bloom = params.post_data.z;
    if (params.post_flags.z > 0u && bloom > 0.001) {
        let lum = dot(col, vec3<f32>(0.2126, 0.7152, 0.0722));
        let thr = mix(0.62, 0.42, clamp(bloom, 0.0, 1.0));
        let bright = clamp((lum - thr) / max(1.0 - thr, 0.001), 0.0, 1.0);
        let soft_knee = bright * bright * (3.0 - 2.0 * bright);
        let emitter_lift = 1.0 + light_energy * 0.35;
        let gain = bloom * (0.65 + lum * 0.55) * emitter_lift;
        col = col + col * soft_knee * gain;
    }
    // Fog toward horizon
    if (params.post_flags.x > 0u) {
        let fog_density = params.post_data.y;
        let fog_t = clamp((horizon - y) / max(horizon, 1.0), 0.0, 1.0);
        let fog_a = clamp(fog_t * fog_density * 1.6, 0.0, 0.85);
        col = lerp3(col, params.post_colors.rgb, fog_a);
    }
    // Vignette
    let vig = params.post_data.x;
    if (params.post_flags.y > 0u && vig > 0.001) {
        let uvx = (x / f32(w) - 0.5) * 2.0;
        let uvy = (y / f32(h) - 0.5) * 2.0;
        let vig_d = length(vec2<f32>(uvx, uvy));
        let vig_mask = smoothstep(0.5, 1.5, vig_d);
        col = lerp3(col, vec3<f32>(0.0, 0.0, 0.0), vig_mask * vig);
    }
    // Film grain (time-seeded hash)
    let grain = params.post_data.w;
    if (params.post_flags.w > 0u && grain > 0.001) {
        let gn = hash2(vec2<f32>(x * 1.3 + params.time_scroll.y * 100.0, y * 1.7 + params.time_scroll.y * 73.0));
        col = col + (gn - 0.5) * grain * 0.12;
    }

    textureStore(dst_tex, vec2<i32>(i32(gid.x), i32(gid.y)), vec4<f32>(clamp(col, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0));
}
"#;
