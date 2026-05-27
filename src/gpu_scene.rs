use bytemuck::{Pod, Zeroable};

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
    sky_flags: [u32; 4],
    sky_counts: [u32; 4],
    sky_misc: [f32; 4],
    cloud_tint: [f32; 4],
    moon_misc: [f32; 4],
    feature_counts: [u32; 4],
    prop_core: [[f32; 4]; 8],
    prop_geom: [[f32; 4]; 8],
    prop_tint: [[f32; 4]; 8],
    prop_misc: [[f32; 4]; 8],
    atmo_core: [[f32; 4]; 4],
    atmo_glow: [[f32; 4]; 4],
    grass_data:  [f32; 4],   // [density, height, upright, enabled(0/1)]
    grass_color: [f32; 4],   // [r, g, b, 0]
    post_data:   [f32; 4],   // [vignette, fog_density, bloom, grain]
    post_flags:  [u32; 4],   // [fog_enabled, 0, 0, 0]
    post_colors: [f32; 4],   // [fog_r, fog_g, fog_b, saturation]
}

pub struct GpuSceneRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    bind_group_layout: wgpu::BindGroupLayout,
}

pub fn supports_exact_scene_parity(settings: &crate::settings::PathForgeSettings) -> bool {
    // Temporary safety gate: CPU renderer is currently the only exact reference.
    // The GPU scene shader is still missing several CPU features (tile sampling, full
    // lighting profile parity, curve-aware floor sampling, and full prop pipelines),
    // so claiming "exact scene parity" causes visible regressions in real presets.
    let _ = settings;
    false
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

        let spc = p.z_spacing.max(0.1);
        let wz_near = p.start_wz.max(0.01);
        let wz_far = p.end_wz.max(wz_near + 0.01);
        let ss = p.sprite_scale.max(0.01);

        let n_min = (scroll + wz_near) / spc;
        let n_max = (scroll + wz_far) / spc;
        let n_start = n_min.floor() as i32 - 1;
        let n_end = n_max.ceil() as i32 + 1;

        for n in n_start..=n_end {
            let wz = n as f32 * spc - scroll;
            if wz < wz_near || wz > wz_far || wz < 0.001 { continue; }

            let ps = focal * p.scale * ss / wz;
            if ps < 1.0 { continue; }

            let sy_floor = horizon + focal * cam_h / wz;
            let fy = sy_floor - horizon;
            let path_t = (fy / (h - horizon).max(1.0)).clamp(0.0, 1.0);
            let row_phw = max_hw
                * (2.0 * path_t - path_t * path_t)
                    .max(0.0)
                    .powf(path_power);
            let row_cx2 = cx + curve * max_hw * 0.72 * (path_t * path_t);

            let sprite_h = (ps * p.height_scale * 2.0).max(1.0);
            let sprite_w = sprite_h * (iw as f32 / ih as f32) * p.width_scale;

            let mirrors: &[f32] = if p.mirror { &[-1.0, 1.0] } else { &[1.0] };
            for &sgn in mirrors {
                let sx_world = row_cx2 + focal * p.wx * sgn / wz;
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
            let row_phw = max_hw
                * (2.0 * path_t - path_t * path_t)
                    .max(0.0)
                    .powf(path_power);
            let row_cx2 = cx + curve * max_hw * 0.72 * (path_t * path_t);

            let sprite_h = (ps * 2.0).max(1.0);
            let sprite_w = sprite_h * (iw as f32 / ih as f32);
            let side_x = row_phw + settings.walls.l_wx * 0.42;

            for &sgn in &[-1.0f32, 1.0] {
                let sx = row_cx2 + sgn * side_x;
                let left = (sx - sprite_w * 0.5) as i32;
                let right = (sx + sprite_w * 0.5) as i32;
                let top = (sy - sprite_h * 0.5) as i32;
                let bot = (sy + sprite_h * 0.5) as i32;

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

        Ok(Self {
            device,
            queue,
            pipeline,
            bind_group_layout,
        })
    }

    pub fn render_scene_rgba(
        &self,
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
        let mut prop_tint = [[0.0f32; 4]; 8];
        let mut prop_misc = [[0.0f32; 4]; 8];
        let mut prop_count = 0u32;
        for p in settings.props.items.iter().filter(|p| p.enabled).take(8) {
            // Sprite-backed props are handled by the CPU overlay after GPU render;
            // leave the slot as zero (disabled) so the GPU skips them.
            let is_sprite = !p.sprite_path.trim().is_empty()
                || (p.sprite_pool_enabled && !p.sprite_pool_paths.trim().is_empty());
            if is_sprite {
                prop_count += 1;
                continue;
            }
            let i = prop_count as usize;
            prop_core[i] = [
                1.0,
                prop_type_code(&p.prop_type),
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
            prop_misc[i] = [
                p.edge_gap.max(0.0),
                p.seed as f32,
                p.tree_style_mix.clamp(0.0, 1.0),
                p.tree_style_bias.clamp(-1.0, 1.0),
            ];
            prop_count += 1;
        }

        let mut atmo_core = [[0.0f32; 4]; 4];
        let mut atmo_glow = [[0.0f32; 4]; 4];
        let mut atmo_count = 0u32;
        for l in settings
            .atmo
            .layers
            .iter()
            .filter(|l| l.enabled && !matches!(l.atmo_type, crate::settings::AtmoType::None))
            .take(4)
        {
            let i = atmo_count as usize;
            atmo_core[i] = [
                1.0,
                atmo_type_code(&l.atmo_type),
                l.torch_h,
                l.torch_spc.max(1) as f32,
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
            sun_color: to_rgba_f32(settings.sky.sun_color),
            moon_data: [
                settings.sky.moon_pos[0],
                settings.sky.moon_pos[1],
                settings.sky.moon_radius,
                settings.sky.moon_alpha.clamp(0.0, 2.0),
            ],
            moon_color: to_rgba_f32(settings.sky.moon_color),
            misc: [
                crate::settings::TILE as f32,
                settings.scene.ambient.clamp(0.0, 2.0),
                settings.walls.l_wx,
                settings.walls.bright.clamp(0.0, 12.0),
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
                0.0,
            ],
            feature_counts: [prop_count, atmo_count, 0, 0],
            prop_core,
            prop_geom,
            prop_tint,
            prop_misc,
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
            post_flags: [if settings.post.fog_enabled { 1 } else { 0 }, 0, 0, 0],
            post_colors: [
                settings.post.fog_color[0] as f32 / 255.0,
                settings.post.fog_color[1] as f32 / 255.0,
                settings.post.fog_color[2] as f32 / 255.0,
                settings.post.saturation.clamp(0.0, 3.0),
            ],
        };

        let out_tex = self.device.create_texture(&wgpu::TextureDescriptor {
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

        let params_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("path_forge_gpu_scene_params"),
            size: std::mem::size_of::<SceneParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&params_buf, 0, bytemuck::bytes_of(&params));

        let out_view = out_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("path_forge_gpu_scene_bg"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&out_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: params_buf.as_entire_binding(),
                },
            ],
        });

        let tight_bpr = width.saturating_mul(4);
        let padded_bpr = aligned_bytes_per_row(width);
        let out_size = (padded_bpr as u64).saturating_mul(height as u64);
        let out_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("path_forge_gpu_scene_readback"),
            size: out_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

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
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(width.div_ceil(8), height.div_ceil(8), 1);
        }

        encoder.copy_texture_to_buffer(
            wgpu::ImageCopyTexture {
                texture: &out_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyBuffer {
                buffer: &out_buf,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bpr),
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

        let slice = out_buf.slice(..);
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
        let mut out = vec![0u8; (tight_bpr as usize) * (height as usize)];
        let padded_bpr_usize = padded_bpr as usize;
        let tight_bpr_usize = tight_bpr as usize;
        for row in 0..height as usize {
            let src_off = row * padded_bpr_usize;
            let dst_off = row * tight_bpr_usize;
            out[dst_off..dst_off + tight_bpr_usize]
                .copy_from_slice(&mapped[src_off..src_off + tight_bpr_usize]);
        }
        drop(mapped);
        out_buf.unmap();
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
    sky_flags: vec4<u32>,
    sky_counts: vec4<u32>,
    sky_misc: vec4<f32>,
    cloud_tint: vec4<f32>,
    moon_misc: vec4<f32>,
    feature_counts: vec4<u32>,
    prop_core: array<vec4<f32>, 8>,
    prop_geom: array<vec4<f32>, 8>,
    prop_tint: array<vec4<f32>, 8>,
    prop_misc: array<vec4<f32>, 8>,
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

fn shape_circle(p: vec2<f32>, c: vec2<f32>, r: f32) -> f32 {
    let d = distance(p, c);
    return clamp(1.0 - d / max(r, 0.001), 0.0, 1.0);
}

fn shape_rect(p: vec2<f32>, c: vec2<f32>, hx: f32, hy: f32) -> f32 {
    let dx = abs(p.x - c.x) / max(hx, 0.001);
    let dy = abs(p.y - c.y) / max(hy, 0.001);
    return select(0.0, 1.0, dx <= 1.0 && dy <= 1.0);
}

fn prop_draw(
    pxy: vec2<f32>,
    base_col: vec3<f32>,
    tcode: f32,
    sx: f32,
    sy_base: f32,
    ps: f32,
) -> vec3<f32> {
    var acc = 0.0;
    let trunk = shape_rect(pxy, vec2<f32>(sx, sy_base - ps * 0.55), ps * 0.10, ps * 0.50);
    let canopy_a = shape_circle(pxy, vec2<f32>(sx, sy_base - ps * 1.28), ps * 0.56);
    let canopy_b = shape_circle(pxy, vec2<f32>(sx - ps * 0.28, sy_base - ps * 1.05), ps * 0.38);
    let canopy_c = shape_circle(pxy, vec2<f32>(sx + ps * 0.28, sy_base - ps * 1.02), ps * 0.38);
    let pine = shape_circle(pxy, vec2<f32>(sx, sy_base - ps * 1.34), ps * 0.40)
        + shape_circle(pxy, vec2<f32>(sx, sy_base - ps * 1.04), ps * 0.50)
        + shape_circle(pxy, vec2<f32>(sx, sy_base - ps * 0.78), ps * 0.60);
    let bush = shape_circle(pxy, vec2<f32>(sx, sy_base - ps * 0.58), ps * 0.46)
        + shape_circle(pxy, vec2<f32>(sx - ps * 0.30, sy_base - ps * 0.50), ps * 0.32)
        + shape_circle(pxy, vec2<f32>(sx + ps * 0.30, sy_base - ps * 0.48), ps * 0.32);
    let rock = shape_circle(pxy, vec2<f32>(sx, sy_base - ps * 0.33), ps * 0.36);
    let cactus = shape_rect(pxy, vec2<f32>(sx, sy_base - ps * 0.75), ps * 0.12, ps * 0.62)
        + shape_rect(pxy, vec2<f32>(sx - ps * 0.26, sy_base - ps * 0.85), ps * 0.08, ps * 0.28)
        + shape_rect(pxy, vec2<f32>(sx + ps * 0.26, sy_base - ps * 0.90), ps * 0.08, ps * 0.24);
    let mushroom = shape_rect(pxy, vec2<f32>(sx, sy_base - ps * 0.28), ps * 0.06, ps * 0.20)
        + shape_circle(pxy, vec2<f32>(sx, sy_base - ps * 0.52), ps * 0.28);
    let dead = shape_rect(pxy, vec2<f32>(sx, sy_base - ps * 0.65), ps * 0.09, ps * 0.60)
        + shape_rect(pxy, vec2<f32>(sx + ps * 0.22, sy_base - ps * 1.00), ps * 0.18, ps * 0.06)
        + shape_rect(pxy, vec2<f32>(sx - ps * 0.20, sy_base - ps * 0.92), ps * 0.15, ps * 0.05);

    if (tcode < 0.5) {
        acc = max(max(canopy_a, canopy_b), canopy_c) + trunk;
    } else if (tcode < 1.5) {
        acc = pine + trunk;
    } else if (tcode < 2.5) {
        acc = bush;
    } else if (tcode < 4.5) {
        acc = rock;
    } else if (tcode < 5.5) {
        acc = cactus;
    } else if (tcode < 6.5) {
        acc = dead;
    } else {
        acc = mushroom;
    }

    let a = clamp(acc, 0.0, 1.0);
    return lerp3(base_col, base_col * 0.22, a);
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

    var col = params.void_color.rgb;

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
            let cs = clamp(params.sky_misc.w, 0.2, 6.0);
            let drift = params.time_scroll.y * params.sky_misc.z;
            let nseed = vec2<f32>(f32(params.sky_counts.w & 1023u) * 0.13, f32((params.sky_counts.w >> 10u) & 1023u) * 0.11);
            let npos = vec2<f32>(x / max(horizon * 0.24 * cs, 1.0), y / max(horizon * 0.16 * cs, 1.0));
            let p = npos + nseed + vec2<f32>(drift * 0.26, drift * 0.03);
            let v = clamp(params.moon_misc.z, 0.0, 1.0);
            let n = fbm2(p * (0.95 + v * 0.65));
            let cloud = smoothstep(0.58 - v * 0.14, 0.84 + v * 0.05, n);
            let cloud_a = cloud * clamp(params.cloud_tint.w, 0.0, 2.0) * 0.36;
            col = lerp3(col, params.cloud_tint.rgb, clamp(cloud_a, 0.0, 0.85));
        }

        if (params.sun_data.w > 0.5) {
            let sun_cx = params.sun_data.x * w;
            let sun_cy = params.sun_data.y * horizon;
            let sun_r = max(params.sun_data.z * horizon, 1.0);
            let d = distance(vec2<f32>(x, y), vec2<f32>(sun_cx, sun_cy));
            let glow = clamp(1.0 - d / (sun_r * 2.3), 0.0, 1.0);
            col = col + params.sun_color.rgb * glow * 0.35;
        }

        let moon_alpha = clamp(params.moon_data.w, 0.0, 2.0);
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
                        let tx = nx * 4.2 * ts;
                        let ty = ny * 4.2 * ts;
                        let coarse = fbm2(vec2<f32>(tx, ty) + vec2<f32>(3.7, 11.1));
                        let medium = fbm2(vec2<f32>(tx * 1.95, ty * 1.95) + vec2<f32>(19.3, -5.4));
                        let rim_b = clamp(1.0 - d_main, 0.0, 1.0);
                        var crater = 0.88 + coarse * 0.06 + medium * 0.04 + (rim_b - 0.5) * 0.015;
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
        let phw = max_hw * pow(max(2.0 * t - t * t, 0.0), path_power);
        let curve_shift = curve * max_hw * 0.72 * (t * t);
        let row_cx = cx + curve_shift;

        let d = cam_h * focal / p;
        let wx = (x - row_cx) / max(focal, 0.001) * d;
        let wz = d + params.time_scroll.x * params.misc.x;
        let edge = abs(x - row_cx);

        if (edge <= phw) {
            let tile_u = floor(wx * 0.10);
            let tile_v = floor(wz * 0.10);
            let n = hash2(vec2<f32>(tile_u, tile_v));
            let mortar = step(0.88, n);
            let base = lerp3(params.floor_base.rgb, params.floor_mortar.rgb, mortar);
            let depth = clamp(1.0 - t, 0.0, 1.0);
            let shade = clamp(params.misc.y * (0.3 + depth * 0.8), 0.05, 2.0);
            col = base * shade;
        } else if (params.dim.w == 1u) {
            let wall_n = hash2(vec2<f32>(floor((x - row_cx) * 0.11), floor(wz * 0.07)));
            let mortar = step(0.90, wall_n);
            let base = lerp3(params.wall_base.rgb, params.wall_mortar.rgb, mortar);
            let side_fade = clamp(1.0 - min(edge - phw, 180.0) / 180.0, 0.0, 1.0);
            let bright = clamp(params.misc.w / max(d, 0.2), 0.05, 1.2) * side_fade;
            col = base * bright;
        }

        // Native GPU atmosphere fixtures + glow (up to 4 layers).
        for (var ai: i32 = 0; ai < i32(min(params.feature_counts.y, 4u)); ai = ai + 1) {
            let ac = params.atmo_core[ai];
            if (ac.x < 0.5) { continue; }
            let torch_h = ac.z;
            let spc = max(ac.w, 1.0);
            let n = floor((params.time_scroll.x + d) / spc + 0.5);
            let wz_i = n * spc - params.time_scroll.x;
            if (wz_i < 0.12 || wz_i > 24.0) { continue; }
            let fr = focal * params.atmo_glow[ai].w / max(wz_i, 0.001);
            if (fr < 0.45) { continue; }
            let sy = horizon + focal * (cam_h - torch_h) / max(wz_i, 0.001);
            let path_t = clamp((sy - horizon) / max(h - horizon, 1.0), 0.0, 1.0);
            let row_phw = max_hw * pow(max(2.0 * path_t - path_t * path_t, 0.0), path_power);
            let row_cx2 = cx + curve * max_hw * 0.72 * (path_t * path_t);
            let side_x = row_phw + params.misc.z * 0.42;
            let sxl = row_cx2 - side_x;
            let sxr = row_cx2 + side_x;
            let gcol = params.atmo_glow[ai].rgb;
            let gl = shape_circle(vec2<f32>(x, y), vec2<f32>(sxl, sy), fr * 3.0);
            let gr = shape_circle(vec2<f32>(x, y), vec2<f32>(sxr, sy), fr * 3.0);
            col = col + gcol * (gl + gr) * 0.34;
        }

        // Native GPU props (up to 8 slots; sprite-backed slots are zeroed and skipped).
        for (var pi: i32 = 0; pi < i32(min(params.feature_counts.x, 8u)); pi = pi + 1) {
            let pc = params.prop_core[pi];
            if (pc.x < 0.5) { continue; }
            let tcode = pc.y;
            let mirror = pc.z > 0.5;
            let spc = max(pc.w, 0.1);
            let pg = params.prop_geom[pi];
            let wx = pg.x;
            let wz_min = pg.y;
            let wz_max = pg.z;
            let sc = pg.w;
            let pt = params.prop_tint[pi];
            let edge_gap = params.prop_misc[pi].x;
            let follow = clamp(pt.w, 0.0, 1.0);
            let tint = pt.rgb;

            let n = floor((params.time_scroll.x + d) / spc + 0.5);
            let wz_i = n * spc - params.time_scroll.x;
            if (wz_i < wz_min || wz_i > wz_max) { continue; }
            let ps = focal * sc / max(wz_i, 0.001);
            if (ps < 0.75) { continue; }
            let sy_floor = horizon + focal * cam_h / max(wz_i, 0.001);
            let sy_base = sy_floor + ps * 0.45;
            let path_t = clamp((sy_floor - horizon) / max(h - horizon, 1.0), 0.0, 1.0);
            let row_phw = max_hw * pow(max(2.0 * path_t - path_t * path_t, 0.0), path_power);
            let row_cx2 = cx + curve * max_hw * 0.72 * (path_t * path_t);

            for (var si: i32 = 0; si < 2; si = si + 1) {
                if (si == 1 && !mirror) { continue; }
                let sgn = select(1.0, -1.0, si == 1);
                let sx_world = row_cx2 + focal * (wx * sgn) / max(wz_i, 0.001);
                let sx_edge = row_cx2 + sgn * (row_phw + edge_gap * ps);
                let sx = mix(sx_world, sx_edge, follow);
                col = prop_draw(vec2<f32>(x, y), col * tint, tcode, sx, sy_base, ps);
            }
        }
    }

    // ── Grass at path edges ──────────────────────────────────────────────────
    let fy_g = y - horizon;
    if (params.grass_data.w > 0.5 && fy_g > 0.0) {
        let wz_g = focal * cam_h / max(fy_g, 1.0);
        let path_t_g = clamp(fy_g / max(f32(h) - horizon, 1.0), 0.0, 1.0);
        let phw_g = max_hw * pow(max(2.0 * path_t_g - path_t_g * path_t_g, 0.0), path_power);
        let gph = focal / max(wz_g, 0.001);
        let grass_h = max(params.grass_data.y * gph * 8.0, 2.0);
        let gdensity = params.grass_data.x;
        let gcol = params.grass_color.rgb;
        for (var gs: i32 = 0; gs < 2; gs = gs + 1) {
            let edge_x = select(cx + phw_g, cx - phw_g, gs == 0);
            let dx = (x - edge_x) / max(grass_h * 0.5, 1.0);
            if (dx > -1.5 && dx < 1.5) {
                let gn = fbm2(vec2<f32>((x + wz_g * 13.7) * gdensity * 0.1, y * 0.12));
                let edge_fade = 1.0 - abs(dx) / 1.5;
                let alpha = clamp(gn * edge_fade * 1.8 - 0.2, 0.0, 0.75);
                col = lerp3(col, gcol * (0.55 + gn * 0.45), alpha);
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
    // Bloom (luminance threshold self-brighten — single-pass approximation)
    let bloom = params.post_data.z;
    if (bloom > 0.001) {
        let bl = max(dot(col, vec3<f32>(0.2126, 0.7152, 0.0722)) - 0.65, 0.0);
        col = col + col * bl * bloom * 0.5;
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
    if (vig > 0.001) {
        let uvx = (x / f32(w) - 0.5) * 2.0;
        let uvy = (y / f32(h) - 0.5) * 2.0;
        let vig_d = length(vec2<f32>(uvx, uvy));
        let vig_mask = smoothstep(0.5, 1.5, vig_d);
        col = lerp3(col, vec3<f32>(0.0, 0.0, 0.0), vig_mask * vig);
    }
    // Film grain (time-seeded hash)
    let grain = params.post_data.w;
    if (grain > 0.001) {
        let gn = hash2(vec2<f32>(x * 1.3 + params.time_scroll.y * 100.0, y * 1.7 + params.time_scroll.y * 73.0));
        col = col + (gn - 0.5) * grain * 0.12;
    }

    textureStore(dst_tex, vec2<i32>(i32(gid.x), i32(gid.y)), vec4<f32>(clamp(col, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0));
}
"#;
