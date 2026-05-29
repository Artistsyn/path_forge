
use crate::renderer::RenderLayers;
use crate::gpu_effects::{GpuEffectSettings, GpuEffects};
use crate::gpu_scene::GpuSceneRenderer;

#[derive(Clone, Debug)]
pub struct ExportUpdate {
    pub stage: String,
    pub message: String,
    pub current: u32,
    pub total: u32,
    pub done: bool,
    pub failed: bool,
}

/// Encode a sequence of RGBA frames as a looping animated GIF.
/// `delay_ms` — per-frame delay in milliseconds (e.g. 83 ≈ 12 fps).
pub fn export_gif(
    frames: &[Vec<u8>],
    width:    u16,
    height:   u16,
    delay_ms: u32,
    path:     &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let file = std::fs::File::create(path)?;
    let mut encoder = gif::Encoder::new(file, width, height, &[])?;
    encoder.set_repeat(gif::Repeat::Infinite)?;

    let delay_hundredths = ((delay_ms + 5) / 10) as u16; // round to 10 ms units

    for rgba in frames {
        // Convert RGBA → RGB (GIF has no alpha; discard alpha channel)
        let rgb: Vec<u8> = rgba.chunks(4)
            .flat_map(|p| [p[0], p[1], p[2]])
            .collect();

        let mut frame = gif::Frame::from_rgb(width, height, &rgb);
        frame.delay = delay_hundredths;
        encoder.write_frame(&frame)?;
    }

    Ok(())
}

/// Render and encode a looping GIF directly frame-by-frame.
/// This avoids buffering every frame in memory and is suitable for longer exports.
pub fn export_loop_gif(
    settings: &crate::settings::PathForgeSettings,
    fps:      u32,
    total_frames: u32,
    smoothing_samples: u32,
    path:     &str,
) -> Result<usize, Box<dyn std::error::Error>> {
    export_loop_gif_with_layers(
        settings,
        fps,
        total_frames,
        smoothing_samples,
        path,
        RenderLayers::all(),
        false,
        false,
        GpuEffectSettings::default(),
        None,
    )
}

fn export_loop_gif_with_layers(
    settings: &crate::settings::PathForgeSettings,
    fps: u32,
    total_frames: u32,
    smoothing_samples: u32,
    path: &str,
    layers: RenderLayers,
    gpu_scene_enabled: bool,
    gpu_enabled: bool,
    gpu_settings: GpuEffectSettings,
    mut progress: Option<&mut dyn FnMut(u32, u32)>,
) -> Result<usize, Box<dyn std::error::Error>> {
    let fps = fps.max(1);
    let smooth_n = smoothing_samples.max(1);
    let (loop_tiles, n_frames, frame_step, delay_hundredths) =
        frame_plan(settings, fps, total_frames);

    let width = settings.canvas.w() as u16;
    let height = settings.canvas.h() as u16;
    let file = std::fs::File::create(path)?;
    let mut encoder = gif::Encoder::new(file, width, height, &[])?;
    encoder.set_repeat(gif::Repeat::Infinite)?;

    let mut renderer = crate::renderer::PathRenderer::default();
    let mut gpu_scene = if gpu_scene_enabled
        && crate::gpu_scene::supports_exact_scene_parity(settings)
        && layers.sky && layers.floor && layers.walls && layers.atmo && layers.props && layers.post {
        GpuSceneRenderer::new().ok()
    } else {
        None
    };
    let gpu = if gpu_enabled { GpuEffects::new().ok() } else { None };
    let mut rgba = vec![0u8; settings.canvas.w() * settings.canvas.h() * 4];
    let mut rgb = vec![0u8; settings.canvas.w() * settings.canvas.h() * 3];
    let mut accum = vec![0u32; settings.canvas.w() * settings.canvas.h() * 3];

    for i in 0..n_frames {
        if smooth_n <= 1 {
            let scroll = i as f32 * frame_step;
            let global_t = scroll / loop_tiles;
            if let Some(gs) = gpu_scene.as_mut() {
                match gs.render_scene_rgba(settings, scroll, global_t) {
                    Ok(scene) => {
                        rgba.copy_from_slice(&scene);
                        if crate::gpu_scene::has_sprite_instances(settings) {
                            crate::gpu_scene::composite_sprite_overlay(
                                &mut rgba,
                                settings.canvas.w() as u32,
                                settings.canvas.h() as u32,
                                settings,
                                scroll,
                            );
                        }
                    }
                    Err(e) => return Err(format!("gpu scene render failed frame {} to {}: {}", i, path, e).into()),
                }
            } else {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    renderer.render_with_layers(settings, scroll, global_t, &layers, &mut rgba);
                }));
                if result.is_err() {
                    return Err(format!("render panic while exporting frame {} to {}", i, path).into());
                }
            }
            if let Some(gpu_fx) = gpu.as_ref() {
                if let Ok(processed) = gpu_fx.process_rgba(
                    &rgba,
                    settings.canvas.w() as u32,
                    settings.canvas.h() as u32,
                    &gpu_settings,
                ) {
                    rgba.copy_from_slice(&processed);
                }
            }
            for (src, dst) in rgba.chunks_exact(4).zip(rgb.chunks_exact_mut(3)) {
                dst[0] = src[0];
                dst[1] = src[1];
                dst[2] = src[2];
            }
        } else {
            accum.fill(0);
            for sidx in 0..smooth_n {
                let phase = ((i as f32) + (sidx as f32 + 0.5) / smooth_n as f32) / n_frames as f32;
                let scroll = phase * loop_tiles;
                let global_t = phase;
                if let Some(gs) = gpu_scene.as_mut() {
                    match gs.render_scene_rgba(settings, scroll, global_t) {
                        Ok(scene) => {
                            rgba.copy_from_slice(&scene);
                            if crate::gpu_scene::has_sprite_instances(settings) {
                                crate::gpu_scene::composite_sprite_overlay(
                                    &mut rgba,
                                    settings.canvas.w() as u32,
                                    settings.canvas.h() as u32,
                                    settings,
                                    scroll,
                                );
                            }
                        }
                        Err(e) => return Err(format!("gpu scene render failed frame {} to {}: {}", i, path, e).into()),
                    }
                } else {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        renderer.render_with_layers(settings, scroll, global_t, &layers, &mut rgba);
                    }));
                    if result.is_err() {
                        return Err(format!("render panic while exporting frame {} to {}", i, path).into());
                    }
                }
                if let Some(gpu_fx) = gpu.as_ref() {
                    if let Ok(processed) = gpu_fx.process_rgba(
                        &rgba,
                        settings.canvas.w() as u32,
                        settings.canvas.h() as u32,
                        &gpu_settings,
                    ) {
                        rgba.copy_from_slice(&processed);
                    }
                }
                for (p, src) in rgba.chunks_exact(4).enumerate() {
                    let j = p * 3;
                    accum[j] += src[0] as u32;
                    accum[j + 1] += src[1] as u32;
                    accum[j + 2] += src[2] as u32;
                }
            }
            for (dst, sum) in rgb.iter_mut().zip(accum.iter()) {
                *dst = (sum / smooth_n) as u8;
            }
        }

        let mut frame = gif::Frame::from_rgb(width, height, &rgb);
        frame.delay = delay_hundredths;
        encoder.write_frame(&frame)?;
        if let Some(cb) = progress.as_deref_mut() {
            let f = i + 1;
            cb(f, n_frames);
        }
    }

    Ok(n_frames as usize)
}

pub fn export_project_gifs(
    settings: &crate::settings::PathForgeSettings,
    fps: u32,
    total_frames: u32,
    smoothing_samples: u32,
    project_name: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    export_project_gifs_with_progress(
        settings,
        fps,
        total_frames,
        smoothing_samples,
        project_name,
        true,
        false,
        false,
        GpuEffectSettings::default(),
        |_| {},
    )
}

pub fn export_project_gifs_with_progress(
    settings: &crate::settings::PathForgeSettings,
    fps: u32,
    total_frames: u32,
    smoothing_samples: u32,
    project_name: &str,
    export_individual_layers: bool,
    gpu_scene_enabled: bool,
    gpu_enabled: bool,
    gpu_settings: GpuEffectSettings,
    mut on_update: impl FnMut(ExportUpdate),
) -> Result<String, Box<dyn std::error::Error>> {
    let safe_name = sanitize_project_name(project_name);
    let project_dir = std::env::current_dir()?
        .join("exported_gif")
        .join(safe_name);
    std::fs::create_dir_all(&project_dir)?;

    let sky_path = project_dir.join("01_sky.gif");
    let floor_path = project_dir.join("02_path_floor.gif");
    let walls_path = project_dir.join("03_walls.gif");
    let atmo_path = project_dir.join("04_atmosphere.gif");
    let props_path = project_dir.join("05_props.gif");
    let composite_path = project_dir.join("06_composite_layered.gif");

    let mut generated: Vec<String> = Vec::new();
    let mut failed: Vec<String> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    let mut frame_count: usize = 0;
    let has_sky = layer_has_sky(settings);
    let has_floor = layer_has_floor(settings);
    let has_walls = layer_has_walls(settings);
    let has_atmo = layer_has_atmo(settings);
    let has_props = layer_has_props(settings);

    let enabled_layers = if export_individual_layers {
        [has_sky, has_floor, has_walls, has_atmo, has_props]
            .into_iter()
            .filter(|v| *v)
            .count() as u32
    } else {
        0
    };
    let total_layers: u32 = enabled_layers + 1; // + composite
    let mut completed_layers: u32 = 0;

    let mut run_layer = |name: &str, path: &std::path::Path, layers: RenderLayers, enabled: bool| {
        if !enabled {
            skipped.push(name.to_owned());
            on_update(ExportUpdate {
                stage: name.to_owned(),
                message: format!("Skipping {} (disabled/empty)", name),
                current: completed_layers,
                total: total_layers,
                done: false,
                failed: false,
            });
            return;
        }
        on_update(ExportUpdate {
            stage: name.to_owned(),
            message: format!("Starting {}", name),
            current: 0,
            total: 1,
            done: false,
            failed: false,
        });
        let mut frame_progress = |f: u32, t: u32| {
            on_update(ExportUpdate {
                stage: name.to_owned(),
                message: format!("{} frame {}/{}", name, f, t),
                current: f,
                total: t,
                done: false,
                failed: false,
            });
        };
        let layer_smoothing = effective_layer_smoothing(smoothing_samples, layers);
        match export_loop_gif_with_layers(
            settings,
            fps,
            total_frames,
            layer_smoothing,
            path.to_string_lossy().as_ref(),
            layers,
            gpu_scene_enabled,
            gpu_enabled,
            gpu_settings,
            Some(&mut frame_progress),
        ) {
            Ok(n) => {
                if frame_count == 0 { frame_count = n; }
                generated.push(name.to_owned());
                completed_layers += 1;
                on_update(ExportUpdate {
                    stage: name.to_owned(),
                    message: format!("Finished {}", name),
                    current: completed_layers,
                    total: total_layers,
                    done: false,
                    failed: false,
                });
            }
            Err(e) => {
                failed.push(format!("{} ({})", name, e));
                completed_layers += 1;
                on_update(ExportUpdate {
                    stage: name.to_owned(),
                    message: format!("Failed {}: {}", name, e),
                    current: completed_layers,
                    total: total_layers,
                    done: false,
                    failed: true,
                });
            }
        }
    };

    if export_individual_layers {
        run_layer("sky", &sky_path, RenderLayers { sky: true, floor: false, walls: false, atmo: false, props: false, post: false }, has_sky);
        run_layer("path_floor", &floor_path, RenderLayers { sky: false, floor: true, walls: false, atmo: false, props: false, post: false }, has_floor);
        run_layer("walls", &walls_path, RenderLayers { sky: false, floor: false, walls: true, atmo: false, props: false, post: false }, has_walls);
        run_layer("atmosphere", &atmo_path, RenderLayers { sky: false, floor: false, walls: false, atmo: true, props: false, post: false }, has_atmo);
        run_layer("props", &props_path, RenderLayers { sky: false, floor: false, walls: false, atmo: false, props: true, post: false }, has_props);
    } else {
        skipped.push("sky".to_owned());
        skipped.push("path_floor".to_owned());
        skipped.push("walls".to_owned());
        skipped.push("atmosphere".to_owned());
        skipped.push("props".to_owned());
        on_update(ExportUpdate {
            stage: "composite_layered".to_owned(),
            message: "Fast export mode: skipping individual layer GIFs".to_owned(),
            current: 0,
            total: total_layers,
            done: false,
            failed: false,
        });
    }

    let mut composite_progress = |f: u32, t: u32| {
        on_update(ExportUpdate {
            stage: "composite_layered".to_owned(),
            message: format!("composite_layered frame {}/{}", f, t),
            current: f,
            total: t,
            done: false,
            failed: false,
        });
    };

    match export_composite_layered_gif(
        settings,
        fps,
        total_frames,
        smoothing_samples,
        gpu_scene_enabled,
        gpu_enabled,
        gpu_settings,
        composite_path.to_string_lossy().as_ref(),
        Some(&mut composite_progress),
    ) {
        Ok(n) => {
            if frame_count == 0 { frame_count = n; }
            generated.push("composite_layered".to_owned());
            completed_layers += 1;
            on_update(ExportUpdate {
                stage: "composite_layered".to_owned(),
                message: "Finished composite_layered".to_owned(),
                current: completed_layers,
                total: total_layers,
                done: false,
                failed: false,
            });
        }
        Err(e) => {
            failed.push(format!("composite_layered ({})", e));
            completed_layers += 1;
            on_update(ExportUpdate {
                stage: "composite_layered".to_owned(),
                message: format!("Failed composite_layered: {}", e),
                current: completed_layers,
                total: total_layers,
                done: false,
                failed: true,
            });
        }
    }

    if generated.is_empty() {
        let msg = format!(
            "No GIFs generated in {}. Failures: {}",
            project_dir.display(),
            failed.join("; ")
        );
        on_update(ExportUpdate {
            stage: "export".to_owned(),
            message: msg.clone(),
            current: 0,
            total: total_layers,
            done: true,
            failed: true,
        });
        return Err(msg.into());
    }

    let mut msg = format!(
        "Saved {} frames in {} | Generated: {}",
        frame_count.max(1),
        project_dir.display(),
        generated.join(", ")
    );
    if !skipped.is_empty() {
        msg.push_str(&format!(" | Skipped: {}", skipped.join(", ")));
    }
    if !failed.is_empty() {
        msg.push_str(&format!(" | Failed: {}", failed.join(", ")));
    }
    on_update(ExportUpdate {
        stage: "export".to_owned(),
        message: msg.clone(),
        current: completed_layers,
        total: total_layers,
        done: true,
        failed: !failed.is_empty(),
    });
    Ok(msg)
}

fn export_composite_layered_gif(
    settings: &crate::settings::PathForgeSettings,
    fps: u32,
    total_frames: u32,
    smoothing_samples: u32,
    gpu_scene_enabled: bool,
    gpu_enabled: bool,
    gpu_settings: GpuEffectSettings,
    path: &str,
    mut on_progress: Option<&mut dyn FnMut(u32, u32)>,
) -> Result<usize, Box<dyn std::error::Error>> {
    let smooth_n = smoothing_samples.max(1);
    let (loop_tiles, n_frames, frame_step, delay_hundredths) =
        frame_plan(settings, fps, total_frames);

    let width = settings.canvas.w() as u16;
    let height = settings.canvas.h() as u16;
    let file = std::fs::File::create(path)?;
    let mut encoder = gif::Encoder::new(file, width, height, &[])?;
    encoder.set_repeat(gif::Repeat::Infinite)?;

    let mut renderer = crate::renderer::PathRenderer::default();
    let mut gpu_scene = if gpu_scene_enabled && crate::gpu_scene::supports_exact_scene_parity(settings) {
        GpuSceneRenderer::new().ok()
    } else {
        None
    };
    let gpu = if gpu_enabled { GpuEffects::new().ok() } else { None };
    let px_count = settings.canvas.w() * settings.canvas.h();
    let mut composite_rgba = vec![0u8; px_count * 4];
    let mut rgb = vec![0u8; px_count * 3];
    let mut accum = vec![0u32; px_count * 3];
    let mut rgba_from_accum = vec![0u8; px_count * 4];

    let has_sky = layer_has_sky(settings);
    let has_floor = layer_has_floor(settings);
    let has_walls = layer_has_walls(settings);
    let has_atmo = layer_has_atmo(settings);
    let has_props = layer_has_props(settings);

    let l_full = RenderLayers {
        sky: has_sky,
        floor: has_floor,
        walls: has_walls,
        atmo: has_atmo,
        props: has_props,
        post: false,
    };

    for i in 0..n_frames {
        if smooth_n <= 1 {
            let scroll = i as f32 * frame_step;
            let global_t = scroll / loop_tiles;

            if let Some(gs) = gpu_scene.as_mut() {
                match gs.render_scene_rgba(settings, scroll, global_t) {
                    Ok(mut scene) => {
                        if crate::gpu_scene::has_sprite_instances(settings) {
                            crate::gpu_scene::composite_sprite_overlay(
                                &mut scene,
                                settings.canvas.w() as u32,
                                settings.canvas.h() as u32,
                                settings,
                                scroll,
                            );
                        }
                        composite_rgba.copy_from_slice(&scene);
                        if let Some(gpu_fx) = gpu.as_ref() {
                            if let Ok(processed) = gpu_fx.process_rgba(
                                &composite_rgba,
                                settings.canvas.w() as u32,
                                settings.canvas.h() as u32,
                                &gpu_settings,
                            ) {
                                for (src, dst) in processed.chunks_exact(4).zip(rgb.chunks_exact_mut(3)) {
                                    dst[0] = src[0];
                                    dst[1] = src[1];
                                    dst[2] = src[2];
                                }
                            } else {
                                for (src, dst) in composite_rgba.chunks_exact(4).zip(rgb.chunks_exact_mut(3)) {
                                    dst[0] = src[0];
                                    dst[1] = src[1];
                                    dst[2] = src[2];
                                }
                            }
                        } else {
                            for (src, dst) in composite_rgba.chunks_exact(4).zip(rgb.chunks_exact_mut(3)) {
                                dst[0] = src[0];
                                dst[1] = src[1];
                                dst[2] = src[2];
                            }
                        }
                        let mut frame = gif::Frame::from_rgb(width, height, &rgb);
                        frame.delay = delay_hundredths;
                        encoder.write_frame(&frame)?;
                        continue;
                    }
                    Err(e) => {
                        return Err(format!("gpu scene render failed composite frame {} to {}: {}", i, path, e).into());
                    }
                }
            }

            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                renderer.render_with_layers(settings, scroll, global_t, &l_full, &mut composite_rgba);
            }));
            if result.is_err() {
                return Err(format!("render panic while exporting composite frame {} to {}", i, path).into());
            }

            if let Some(gpu_fx) = gpu.as_ref() {
                if let Ok(processed) = gpu_fx.process_rgba(
                    &composite_rgba,
                    settings.canvas.w() as u32,
                    settings.canvas.h() as u32,
                    &gpu_settings,
                ) {
                    for (src, dst) in processed.chunks_exact(4).zip(rgb.chunks_exact_mut(3)) {
                        dst[0] = src[0];
                        dst[1] = src[1];
                        dst[2] = src[2];
                    }
                } else {
                    for (src, dst) in composite_rgba.chunks_exact(4).zip(rgb.chunks_exact_mut(3)) {
                        dst[0] = src[0];
                        dst[1] = src[1];
                        dst[2] = src[2];
                    }
                }
            } else {
                for (src, dst) in composite_rgba.chunks_exact(4).zip(rgb.chunks_exact_mut(3)) {
                    dst[0] = src[0];
                    dst[1] = src[1];
                    dst[2] = src[2];
                }
            }
        } else {
            accum.fill(0);
            for sidx in 0..smooth_n {
                let phase = ((i as f32) + (sidx as f32 + 0.5) / smooth_n as f32) / n_frames as f32;
                let scroll = phase * loop_tiles;
                let global_t = phase;

                if let Some(gs) = gpu_scene.as_mut() {
                    match gs.render_scene_rgba(settings, scroll, global_t) {
                        Ok(mut scene) => {
                            if crate::gpu_scene::has_sprite_instances(settings) {
                                crate::gpu_scene::composite_sprite_overlay(
                                    &mut scene,
                                    settings.canvas.w() as u32,
                                    settings.canvas.h() as u32,
                                    settings,
                                    scroll,
                                );
                            }
                            for (p, src) in scene.chunks_exact(4).enumerate() {
                                let j = p * 3;
                                accum[j] += src[0] as u32;
                                accum[j + 1] += src[1] as u32;
                                accum[j + 2] += src[2] as u32;
                            }
                            continue;
                        }
                        Err(e) => return Err(format!("gpu scene render failed composite frame {} to {}: {}", i, path, e).into()),
                    }
                }

                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    renderer.render_with_layers(settings, scroll, global_t, &l_full, &mut composite_rgba);
                }));
                if result.is_err() {
                    return Err(format!("render panic while exporting composite frame {} to {}", i, path).into());
                }

                for p in 0..px_count {
                    let i4 = p * 4;
                    let j = p * 3;
                    accum[j] += composite_rgba[i4] as u32;
                    accum[j + 1] += composite_rgba[i4 + 1] as u32;
                    accum[j + 2] += composite_rgba[i4 + 2] as u32;
                }
            }
            if let Some(gpu_fx) = gpu.as_ref() {
                for p in 0..px_count {
                    let j = p * 3;
                    rgba_from_accum[p * 4] = (accum[j] / smooth_n) as u8;
                    rgba_from_accum[p * 4 + 1] = (accum[j + 1] / smooth_n) as u8;
                    rgba_from_accum[p * 4 + 2] = (accum[j + 2] / smooth_n) as u8;
                    rgba_from_accum[p * 4 + 3] = 255;
                }
                if let Ok(processed) = gpu_fx.process_rgba(
                    &rgba_from_accum,
                    settings.canvas.w() as u32,
                    settings.canvas.h() as u32,
                    &gpu_settings,
                ) {
                    for (src, dst) in processed.chunks_exact(4).zip(rgb.chunks_exact_mut(3)) {
                        dst[0] = src[0];
                        dst[1] = src[1];
                        dst[2] = src[2];
                    }
                } else {
                    for (dst, sum) in rgb.iter_mut().zip(accum.iter()) {
                        *dst = (sum / smooth_n) as u8;
                    }
                }
            } else {
            for (dst, sum) in rgb.iter_mut().zip(accum.iter()) {
                *dst = (sum / smooth_n) as u8;
            }
            }
        }

        let mut frame = gif::Frame::from_rgb(width, height, &rgb);
        frame.delay = delay_hundredths;
        encoder.write_frame(&frame)?;

        if let Some(progress) = on_progress.as_deref_mut() {
            progress(i + 1, n_frames);
        }
    }

    Ok(n_frames as usize)
}

fn effective_layer_smoothing(base: u32, layers: RenderLayers) -> u32 {
    let base = base.max(1);
    if layers.sky && !layers.floor && !layers.walls && !layers.atmo && !layers.props {
        return 1;
    }
    if layers.atmo && !layers.floor && !layers.walls && !layers.sky && !layers.props {
        return base.min(2);
    }
    base
}

fn layer_has_sky(settings: &crate::settings::PathForgeSettings) -> bool {
    settings.sky.enabled
        || settings.sky.sun_enabled
        || settings.sky.moon_enabled
        || settings.sky.stars_enabled
        || settings.sky.clouds_enabled
}

fn layer_has_floor(_settings: &crate::settings::PathForgeSettings) -> bool {
    true
}

fn layer_has_walls(settings: &crate::settings::PathForgeSettings) -> bool {
    settings.walls.enabled
}

fn layer_has_atmo(settings: &crate::settings::PathForgeSettings) -> bool {
    settings.atmo.layers.iter().any(|l| {
        l.enabled
            && (
                l.atmo_type != crate::settings::AtmoType::None
                    || l.n_motes > 0
                    || l.n_debris > 0
                    || !l.sprite_path.trim().is_empty()
                    || (l.sprite_pool_enabled && !l.sprite_pool_paths.trim().is_empty())
            )
    })
}

fn layer_has_props(settings: &crate::settings::PathForgeSettings) -> bool {
    settings.props.items.iter().any(|p| p.enabled)
}

fn frame_plan(
    settings: &crate::settings::PathForgeSettings,
    fps: u32,
    total_frames: u32,
) -> (f32, u32, f32, u16) {
    let fps = fps.max(1);
    let loop_tiles = settings.anim.loop_s.max(1) as f32;
    let n_frames = total_frames.clamp(2, 20000);
    let frame_step = loop_tiles / n_frames as f32;
    let delay_hundredths = ((100.0 / fps as f32).round() as u16).max(1);
    (loop_tiles, n_frames, frame_step, delay_hundredths)
}

fn sanitize_project_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            out.push(ch);
        } else if ch.is_whitespace() {
            out.push('_');
        }
    }
    let out = out.trim_matches('_').to_owned();
    if out.is_empty() { "project".to_owned() } else { out }
}

/// Render every frame of one full loop and return them as RGBA vectors.
pub fn render_loop_frames(
    renderer: &mut crate::renderer::PathRenderer,
    settings: &crate::settings::PathForgeSettings,
    fps:      u32,
    cycles:   u32,
) -> Vec<Vec<u8>> {
    let loop_tiles  = settings.anim.loop_s.max(1) as f32 * cycles.max(1) as f32;
    let n_frames    = (loop_tiles * fps as f32).round() as u32;
    let n_frames    = n_frames.max(1);
    let frame_step  = loop_tiles / n_frames as f32;
    let cw          = settings.canvas.w();
    let ch          = settings.canvas.h();

    let mut frames = Vec::with_capacity(n_frames as usize);

    for i in 0..n_frames {
        let scroll    = i as f32 * frame_step;
        let global_t  = scroll / loop_tiles; // 0..1

        let mut buf = vec![0u8; cw * ch * 4];
        renderer.render(settings, scroll, global_t, &mut buf);
        frames.push(buf);
    }

    frames
}
