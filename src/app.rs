use egui::{Color32, RichText, Slider, TextureOptions, Vec2};
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::sync::mpsc;

use crate::node_lab::NodeLabState;
use crate::renderer::PathRenderer;
use crate::settings::{
    presets, AnimSettings, AtmoLayer, AtmoType, FloorPattern, LightingPreset, PathForgeSettings,
    PropInstance, PropType, WallPattern,
};

// ── Background render thread ───────────────────────────────────────────────
struct RenderRequest {
    frame_id:  u64,
    settings:  PathForgeSettings,
    scroll:    f32,
    global_t:  f32,
    gpu_scene_enabled: bool,
    gpu_enabled: bool,
    gpu_effect_mix: f32,
    gpu_saturation: f32,
    gpu_contrast: f32,
    gpu_brightness: f32,
}

struct RenderResult {
    frame_id: u64,
    w: usize,
    h: usize,
    buf: Vec<u8>,
}

#[derive(Clone)]
struct CustomPreset {
    name: String,
    settings: PathForgeSettings,
}

fn spawn_render_thread() -> (mpsc::Sender<RenderRequest>, mpsc::Receiver<RenderResult>) {
    let (req_tx, req_rx) = mpsc::channel::<RenderRequest>();
    let (res_tx, res_rx) = mpsc::channel::<RenderResult>();
    match std::thread::Builder::new()
        .name("pathforge-renderer".into())
        .spawn(move || {
            let mut renderer = PathRenderer::default();
            let mut buf: Vec<u8> = Vec::new();
            let mut gpu_scene: Option<crate::gpu_scene::GpuSceneRenderer> = None;
            let mut gpu_scene_failed = false;
            let mut gpu: Option<crate::gpu_effects::GpuEffects> = None;
            let mut gpu_failed = false;
            while let Ok(req) = req_rx.recv() {
                let frame_len = req.settings.canvas.w() * req.settings.canvas.h() * 4;
                if buf.len() != frame_len {
                    buf.resize(frame_len, 0);
                }
                let allow_gpu_scene = req.gpu_scene_enabled
                    && crate::gpu_scene::supports_exact_scene_parity(&req.settings);
                let result = if allow_gpu_scene {
                    if gpu_scene.is_none() && !gpu_scene_failed {
                        match crate::gpu_scene::GpuSceneRenderer::new() {
                            Ok(g) => gpu_scene = Some(g),
                            Err(e) => {
                                gpu_scene_failed = true;
                                eprintln!("[pathforge-gpu-scene] init failed, fallback to CPU: {e}");
                            }
                        }
                    }
                    if let Some(gs) = gpu_scene.as_ref() {
                        match gs.render_scene_rgba(&req.settings, req.scroll, req.global_t) {
                            Ok(scene) => {
                                buf.copy_from_slice(&scene);
                                if crate::gpu_scene::has_sprite_instances(&req.settings) {
                                    crate::gpu_scene::composite_sprite_overlay(
                                        &mut buf,
                                        req.settings.canvas.w() as u32,
                                        req.settings.canvas.h() as u32,
                                        &req.settings,
                                        req.scroll,
                                    );
                                }
                                Ok(())
                            }
                            Err(e) => {
                                eprintln!("[pathforge-gpu-scene] render failed, fallback to CPU: {e}");
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    renderer.render(&req.settings, req.scroll, req.global_t, &mut buf);
                                }))
                            }
                        }
                    } else {
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            renderer.render(&req.settings, req.scroll, req.global_t, &mut buf);
                        }))
                    }
                } else {
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        renderer.render(&req.settings, req.scroll, req.global_t, &mut buf);
                    }))
                };
                match result {
                    Ok(_) => {
                        if req.gpu_enabled {
                            if gpu.is_none() && !gpu_failed {
                                match crate::gpu_effects::GpuEffects::new() {
                                    Ok(g) => gpu = Some(g),
                                    Err(e) => {
                                        gpu_failed = true;
                                        eprintln!("[pathforge-gpu] GPU init failed, falling back to CPU: {e}");
                                    }
                                }
                            }
                            if let Some(gpu_fx) = gpu.as_ref() {
                                let gpu_settings = crate::gpu_effects::GpuEffectSettings {
                                    saturation: req.gpu_saturation,
                                    contrast: req.gpu_contrast,
                                    brightness: req.gpu_brightness,
                                    effect_mix: req.gpu_effect_mix,
                                };
                                match gpu_fx.process_rgba(
                                    &buf,
                                    req.settings.canvas.w() as u32,
                                    req.settings.canvas.h() as u32,
                                    &gpu_settings,
                                ) {
                                    Ok(processed) => {
                                        buf.copy_from_slice(&processed);
                                    }
                                    Err(e) => {
                                        eprintln!("[pathforge-gpu] GPU process failed, using CPU frame: {e}");
                                    }
                                }
                            }
                        }
                        if res_tx.send(RenderResult {
                            frame_id: req.frame_id,
                            w: req.settings.canvas.w(),
                            h: req.settings.canvas.h(),
                            buf: buf.clone(),
                        }).is_err() { break; }
                    }
                    Err(e) => {
                        let msg = if let Some(s) = e.downcast_ref::<&str>() {
                            s.to_string()
                        } else if let Some(s) = e.downcast_ref::<String>() {
                            s.clone()
                        } else {
                            "unknown panic".to_string()
                        };
                        eprintln!("[pathforge-renderer] PANIC: {msg}");
                        // Reset renderer and buffer so corrupted state can't persist
                        renderer = PathRenderer::default();
                        buf.fill(128); // grey error frame
                        if res_tx.send(RenderResult {
                            frame_id: req.frame_id,
                            w: req.settings.canvas.w(),
                            h: req.settings.canvas.h(),
                            buf: buf.clone(),
                        }).is_err() {
                            eprintln!("[pathforge-renderer] res_tx dead — exiting");
                            break;
                        }
                    }
                }
            }
        }) {
        Ok(_) => {}
        Err(e) => {
            eprintln!("[pathforge] failed to spawn render thread: {e}");
        }
    }
    (req_tx, res_rx)
}

// ── App state ──────────────────────────────────────────────────────────────
pub struct PathForgeApp {
    // --- live settings (currently displayed / edited)
    settings:     PathForgeSettings,
    preset_idx:   usize,
    is_custom:    bool,
    active_custom_idx: Option<usize>,
    custom_presets: Vec<CustomPreset>,
    preset_name_input: String,
    preset_status: String,

    // --- animation state
    playing:      bool,
    scroll:       f32,
    global_t:     f32,
    last_instant: Option<std::time::Instant>,
    fps_counter:  FpsCounter,

    // --- renderer
    render_tx:        mpsc::Sender<RenderRequest>,
    result_rx:        mpsc::Receiver<RenderResult>,
    render_pending:   bool,
    render_dispatch_at: Option<std::time::Instant>,
    latest_frame_id:  u64,
    pixel_buf:        Vec<u8>,
    preview_w:        usize,
    preview_h:        usize,
    texture:          Option<egui::TextureHandle>,

    // --- section open/close state
    open_canvas:  bool,
    open_scene:   bool,
    open_floor:   bool,
    open_walls:   bool,
    open_sky:     bool,
    open_atmo:    bool,
    open_props:   bool,
    open_post:    bool,
    open_anim:    bool,
    open_node_lab: bool,
    open_atmo_layers: Vec<bool>,
    open_prop_items:  Vec<bool>,

    // --- GIF export
    gif_total_frames: u32,
    gif_smoothing_samples: u32,
    motion_quality_tpf_target: f32,
    temporal_smoothing_enabled: bool,
    temporal_smoothing_boost: u32,
    export_individual_layers: bool,
    gpu_scene_preview_enabled: bool,
    gpu_scene_export_enabled: bool,
    gpu_preview_enabled: bool,
    gpu_export_enabled: bool,
    gpu_effect_mix: f32,
    gpu_saturation: f32,
    gpu_contrast: f32,
    gpu_brightness: f32,
    gif_project_name: String,
    exporting:    bool,
    export_msg:   String,
    export_result_rx: Option<mpsc::Receiver<crate::gif_export::ExportUpdate>>,
    export_progress: f32,
    export_stage: String,
    export_last_update: Option<std::time::Instant>,
    export_stall_notified: bool,

    // --- preview performance (export remains full quality)
    preview_scale: f32,
    preview_fps_cap: u32,
    last_preview_dispatch: Option<std::time::Instant>,

    // --- canvas edit fields (apply/revert semantics)
    canvas_w_input: String,
    canvas_h_input: String,

    // --- render control
    dirty:        bool,   // true = pixel_buf needs re-render this frame

    // --- node lab
    node_lab:     NodeLabState,
}

struct FpsCounter {
    frames: u32,
    last:   std::time::Instant,
    fps:    f32,
}
impl FpsCounter {
    fn new() -> Self { Self { frames:0, last: std::time::Instant::now(), fps: 0.0 } }
    fn tick(&mut self) {
        self.frames += 1;
        let elapsed = self.last.elapsed().as_secs_f32();
        if elapsed > 0.5 {
            self.fps = self.frames as f32 / elapsed;
            self.frames = 0;
            self.last = std::time::Instant::now();
        }
    }
}

impl Default for PathForgeApp {
    fn default() -> Self {
        let settings = PathForgeSettings::default();
        let preview_w = settings.canvas.w();
        let preview_h = settings.canvas.h();
        let pixel_buf = vec![0u8; settings.canvas.w() * settings.canvas.h() * 4];
        let (render_tx, result_rx) = spawn_render_thread();
        // Kick off the first render immediately
        let _ = render_tx.send(RenderRequest {
            frame_id: 1,
            settings:  settings.clone(),
            scroll:    0.0,
            global_t:  0.0,
            gpu_scene_enabled: true,
            gpu_enabled: true,
            gpu_effect_mix: 0.55,
            gpu_saturation: 1.08,
            gpu_contrast: 1.08,
            gpu_brightness: 1.02,
        });
        Self {
            settings,
            preset_idx:     0,
            is_custom:      false,
            active_custom_idx: None,
            custom_presets: Self::load_custom_presets(),
            preset_name_input: String::new(),
            preset_status: String::new(),
            playing:        false,
            scroll:         0.0,
            global_t:       0.0,
            last_instant:   None,
            fps_counter:    FpsCounter::new(),
            render_tx,
            result_rx,
            render_pending:     true,
            render_dispatch_at: Some(std::time::Instant::now()),
            latest_frame_id:    1,
            pixel_buf,
            preview_w,
            preview_h,
            texture:        None,
            open_canvas:    false,
            open_scene:     true,
            open_floor:     true,
            open_walls:     false,
            open_sky:       false,
            open_atmo:      false,
            open_props:     false,
            open_post:      false,
            open_anim:      false,
            open_node_lab:  false,
            open_atmo_layers: vec![false],
            open_prop_items:  vec![],
            gif_total_frames: 36,
            gif_smoothing_samples: 1,
            motion_quality_tpf_target: 0.20,
            temporal_smoothing_enabled: true,
            temporal_smoothing_boost: 2,
            export_individual_layers: false,
            gpu_scene_preview_enabled: true,
            gpu_scene_export_enabled: true,
            gpu_preview_enabled: true,
            gpu_export_enabled: true,
            gpu_effect_mix: 0.55,
            gpu_saturation: 1.08,
            gpu_contrast: 1.08,
            gpu_brightness: 1.02,
            gif_project_name: String::new(),
            exporting:      false,
            export_msg:     String::new(),
            export_result_rx: None,
            export_progress: 0.0,
            export_stage: String::new(),
            export_last_update: None,
            export_stall_notified: false,
            preview_scale: 0.8,
            preview_fps_cap: 30,
            last_preview_dispatch: None,
            canvas_w_input: "480".to_owned(),
            canvas_h_input: "854".to_owned(),
            dirty:          false,
            node_lab:       NodeLabState::default(),
        }
    }
}

// ── eframe::App ────────────────────────────────────────────────────────────
impl eframe::App for PathForgeApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // --- advance animation
        let now = std::time::Instant::now();
        let dt = if let Some(prev) = self.last_instant {
            now.duration_since(prev).as_secs_f32().min(0.1)
        } else { 0.0 };
        self.last_instant = Some(now);

        // Preview/export hardlock mode is always on.
        if !self.settings.anim.seamless_lock {
            self.settings.anim.seamless_lock = true;
            self.dirty = true;
        }

        let loop_len = self.settings.anim.loop_s.max(1) as f32;
        if self.playing {
            self.scroll   = (self.scroll + dt * self.settings.anim.play_speed * loop_len) % loop_len;
            self.global_t = self.scroll / loop_len;
            self.dirty    = true; // always dirty while playing
        }

        // --- collect async GIF export completion
        if self.exporting {
            if let Some(rx) = &self.export_result_rx {
                loop {
                    match rx.try_recv() {
                        Ok(update) => {
                            self.export_stage = update.stage;
                            self.export_msg = update.message;
                            self.export_progress = if update.total > 0 {
                                (update.current as f32 / update.total as f32).clamp(0.0, 1.0)
                            } else {
                                self.export_progress
                            };
                            self.export_last_update = Some(now);
                            self.export_stall_notified = false;
                            if update.done {
                                self.exporting = false;
                                self.export_result_rx = None;
                                break;
                            }
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => break,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            self.export_msg = "Export failed: worker disconnected".to_owned();
                            self.export_progress = 0.0;
                            self.exporting = false;
                            self.export_result_rx = None;
                            break;
                        }
                    }
                }
            }
            if let Some(last) = self.export_last_update {
                if !self.export_stall_notified && now.duration_since(last).as_secs_f32() > 45.0 {
                    self.export_stall_notified = true;
                    let pct = (self.export_progress * 100.0).round() as u32;
                    self.export_msg = format!(
                        "Export is very slow at {}% (stage: {}). Still rendering frames...",
                        pct,
                        if self.export_stage.is_empty() { "unknown" } else { &self.export_stage }
                    );
                }
            }
        }

        // --- collect finished renders (drain all available results)
        let mut got_frame = false;
        loop {
            match self.result_rx.try_recv() {
                Ok(result) => {
                    if result.frame_id != self.latest_frame_id {
                        continue;
                    }
                    if result.buf.len() != result.w * result.h * 4 {
                        continue;
                    }
                    self.preview_w = result.w;
                    self.preview_h = result.h;
                    self.pixel_buf = result.buf;
                    self.render_pending = false;
                    self.render_dispatch_at = None;
                    self.fps_counter.tick();
                    got_frame = true;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    eprintln!("[pathforge] render thread disconnected — respawning");
                    self.respawn_render_thread();
                    break;
                }
            }
        }
        if got_frame {
            let cw = self.preview_w;
            let ch = self.preview_h;
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [cw, ch], &self.pixel_buf);
            let opts = TextureOptions {
                magnification: egui::TextureFilter::Linear,
                minification:  egui::TextureFilter::Linear,
                ..Default::default()
            };
            if let Some(h) = &mut self.texture {
                h.set(image, opts);
            } else {
                self.texture = Some(ctx.load_texture("preview", image, opts));
            }
        }

        // --- watchdog removed (it caused thread leaks — old threads kept running)

        // --- dispatch next render if needed
        if self.dirty && !self.render_pending {
            let mut can_dispatch = true;
            if self.playing {
                let min_dt = 1.0 / self.effective_output_fps().max(10) as f32;
                if let Some(last) = self.last_preview_dispatch {
                    if now.duration_since(last).as_secs_f32() < min_dt {
                        can_dispatch = false;
                    }
                }
            }
            if can_dispatch {
                let mut render_settings = self.settings.clone();
                if self.preview_scale < 0.999 {
                    let src_w = self.settings.canvas.w() as f32;
                    let src_h = self.settings.canvas.h() as f32;
                    let dst_w = (src_w * self.preview_scale).round().clamp(240.0, src_w) as u32;
                    let dst_h = (src_h * self.preview_scale).round().clamp(240.0, src_h) as u32;
                    render_settings.canvas.base_w = dst_w;
                    render_settings.canvas.base_h = dst_h;
                    render_settings.canvas.landscape = false;
                }
                match self.render_tx.send(RenderRequest {
                    frame_id: self.latest_frame_id,
                    settings: render_settings,
                    scroll:   self.scroll,
                    global_t: self.global_t,
                    gpu_scene_enabled: self.gpu_scene_preview_enabled,
                    gpu_enabled: self.gpu_preview_enabled,
                    gpu_effect_mix: self.gpu_effect_mix,
                    gpu_saturation: self.gpu_saturation,
                    gpu_contrast: self.gpu_contrast,
                    gpu_brightness: self.gpu_brightness,
                }) {
                    Ok(()) => {
                        self.dirty = false;
                        self.render_pending = true;
                        self.render_dispatch_at = Some(std::time::Instant::now());
                        self.last_preview_dispatch = Some(now);
                    }
                    Err(_) => {
                        eprintln!("[pathforge] render_tx.send failed — respawning thread");
                        self.respawn_render_thread();
                    }
                }
            }
        }

        // --- UI layout
        self.top_bar(ctx);
        self.right_panel(ctx);
        self.central_panel(ctx);
        self.node_lab.ui(ctx);

        // Keep the event loop alive whenever there is live work to do
        if self.playing || self.render_pending {
            ctx.request_repaint();
        }
    }
}

// ── UI sections ────────────────────────────────────────────────────────────
impl PathForgeApp {
    fn custom_presets_dir() -> PathBuf {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("path_forge_presets")
    }

    fn sanitize_preset_name(name: &str) -> String {
        let mut out = String::with_capacity(name.len());
        for ch in name.chars() {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                out.push(ch);
            } else if ch.is_whitespace() {
                out.push('_');
            }
        }
        let out = out.trim_matches('_').to_owned();
        if out.is_empty() { "custom_preset".to_owned() } else { out }
    }

    fn load_custom_presets() -> Vec<CustomPreset> {
        let mut presets_out = Vec::new();
        let dir = Self::custom_presets_dir();
        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }
                let Ok(txt) = fs::read_to_string(&path) else { continue; };
                let Ok(settings) = serde_json::from_str::<PathForgeSettings>(&txt) else { continue; };
                let name = path.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Custom")
                    .replace('_', " ");
                presets_out.push(CustomPreset { name, settings });
            }
            presets_out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        }
        presets_out
    }

    fn save_current_as_custom_preset(&mut self) {
        let raw = self.preset_name_input.trim();
        if raw.is_empty() {
            self.preset_status = "Enter a preset name first".to_owned();
            return;
        }
        let file_name = Self::sanitize_preset_name(raw);
        let dir = Self::custom_presets_dir();
        if let Err(e) = fs::create_dir_all(&dir) {
            self.preset_status = format!("Preset save failed: {e}");
            return;
        }
        let path = dir.join(format!("{file_name}.json"));
        match serde_json::to_string_pretty(&self.settings) {
            Ok(txt) => {
                if let Err(e) = fs::write(&path, txt) {
                    self.preset_status = format!("Preset save failed: {e}");
                    return;
                }
                self.custom_presets = Self::load_custom_presets();
                self.active_custom_idx = self.custom_presets.iter().position(|p| p.name.eq_ignore_ascii_case(raw));
                if self.active_custom_idx.is_none() {
                    self.active_custom_idx = self.custom_presets.iter().position(|p| p.name.eq_ignore_ascii_case(&file_name.replace('_', " ")));
                }
                self.is_custom = true;
                self.preset_status = format!("Saved preset: {raw}");
            }
            Err(e) => {
                self.preset_status = format!("Preset serialize failed: {e}");
            }
        }
    }

    fn apply_lighting_preset_tuning(settings: &mut PathForgeSettings) {
        match settings.scene.lighting_preset {
            LightingPreset::Balanced => {
                settings.scene.ambient = 1.00;
                settings.scene.atmo_light_influence = 0.35;
                settings.scene.atmo_tint_influence = 0.28;
                settings.sky.enabled = true;
                settings.sky.sun_enabled = true;
                settings.sky.sun_pos = [0.72, 0.22];
                settings.sky.sun_color = [255, 228, 170];
                settings.sky.moon_enabled = false;
                settings.sky.stars_enabled = false;
                settings.sky.clouds_enabled = true;
                settings.sky.cloud_opacity = 0.32;
                for p in settings.props.items.iter_mut().filter(|p| p.enabled) {
                    p.shadow_follow_light = 0.65;
                    p.shadow_length = p.shadow_length.clamp(0.6, 1.4);
                }
            }
            LightingPreset::GoldenHour => {
                settings.scene.ambient = 0.88;
                settings.scene.atmo_light_influence = 0.44;
                settings.scene.atmo_tint_influence = 0.46;
                settings.sky.enabled = true;
                settings.sky.sun_enabled = true;
                settings.sky.sun_pos = [0.84, 0.30];
                settings.sky.sun_color = [255, 188, 122];
                settings.sky.moon_enabled = false;
                settings.sky.stars_enabled = false;
                settings.sky.clouds_enabled = true;
                settings.sky.cloud_tint = [238, 194, 154];
                settings.sky.cloud_opacity = 0.40;
                for p in settings.props.items.iter_mut().filter(|p| p.enabled) {
                    p.shadow_follow_light = 0.82;
                    p.shadow_length = (p.shadow_length * 1.28).clamp(0.8, 3.0);
                }
            }
            LightingPreset::HighNoon => {
                settings.scene.ambient = 1.18;
                settings.scene.atmo_light_influence = 0.20;
                settings.scene.atmo_tint_influence = 0.12;
                settings.sky.enabled = true;
                settings.sky.sun_enabled = true;
                settings.sky.sun_pos = [0.50, 0.10];
                settings.sky.sun_color = [255, 244, 210];
                settings.sky.moon_enabled = false;
                settings.sky.stars_enabled = false;
                settings.sky.clouds_enabled = true;
                settings.sky.cloud_tint = [224, 230, 236];
                settings.sky.cloud_opacity = 0.24;
                for p in settings.props.items.iter_mut().filter(|p| p.enabled) {
                    p.shadow_follow_light = 0.55;
                    p.shadow_length = (p.shadow_length * 0.78).clamp(0.35, 2.2);
                }
            }
            LightingPreset::NightNeon => {
                settings.scene.ambient = 0.70;
                settings.scene.atmo_light_influence = 0.86;
                settings.scene.atmo_tint_influence = 0.72;
                settings.sky.enabled = true;
                settings.sky.sun_enabled = false;
                settings.sky.moon_enabled = true;
                settings.sky.moon_pos = [0.24, 0.18];
                settings.sky.stars_enabled = true;
                settings.sky.stars_twinkle = settings.sky.stars_twinkle.max(0.8);
                settings.sky.clouds_enabled = true;
                settings.sky.cloud_tint = [140, 150, 172];
                settings.sky.cloud_opacity = 0.30;
                for p in settings.props.items.iter_mut().filter(|p| p.enabled) {
                    p.shadow_follow_light = 0.72;
                    p.shadow_length = (p.shadow_length * 1.12).clamp(0.6, 3.2);
                }
            }
        }
    }

    fn lighting_preset_preview_text(preset: &LightingPreset) -> &'static str {
        match preset {
            LightingPreset::Balanced => {
                "Will set neutral ambient/atmo defaults, enable sun+clouds, and keep moderate shadow-follow behavior."
            }
            LightingPreset::GoldenHour => {
                "Will warm sky/sun/cloud tint, increase atmospheric tint influence, and lengthen prop shadows."
            }
            LightingPreset::HighNoon => {
                "Will raise ambient light, reduce atmospheric tinting, and shorten prop shadows for overhead sun."
            }
            LightingPreset::NightNeon => {
                "Will lower ambient light, boost atmospheric influence, enable moon+stars mood, and deepen/extend shadows."
            }
        }
    }

    // ── Top bar ─────────────────────────────────────────────────────────────
    fn top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top_bar")
            .frame(egui::Frame::none()
                .fill(Color32::from_rgb(8,6,4))
                .inner_margin(egui::Margin::symmetric(10.0, 5.0)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("PATH").color(Color32::from_rgb(200,144,48))
                        .font(egui::FontId::proportional(14.0)).strong());
                    ui.label(RichText::new("FORGE").color(Color32::from_rgb(60,48,32))
                        .font(egui::FontId::proportional(14.0)).strong());
                    ui.label(RichText::new("2.0").color(Color32::from_rgb(100,80,40))
                        .font(egui::FontId::proportional(10.0)));

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // FPS
                        ui.label(RichText::new(format!("{:.0}fps", self.fps_counter.fps))
                            .color(Color32::from_rgb(40,36,28)).small());

                        ui.separator();

                        // Preset selector
                        let label = if self.is_custom {
                            if let Some(i) = self.active_custom_idx {
                                if i < self.custom_presets.len() {
                                    self.custom_presets[i].name.clone()
                                } else {
                                    "Custom".to_owned()
                                }
                            } else {
                                "Custom".to_owned()
                            }
                        } else {
                            presets::ALL[self.preset_idx].0.to_owned()
                        };
                        egui::ComboBox::from_id_salt("preset")
                            .selected_text(RichText::new(&label)
                                .color(Color32::from_rgb(184,136,32)).small())
                            .show_ui(ui, |ui| {
                                for (i, (name, _maker)) in presets::ALL.iter().enumerate() {
                                    if ui.selectable_label(
                                        !self.is_custom && self.preset_idx == i,
                                        RichText::new(*name).small()
                                    ).clicked() {
                                        self.settings   = presets::ALL[i].1();
                                        self.preset_idx = i;
                                        self.is_custom  = false;
                                        self.active_custom_idx = None;
                                        self.scroll     = 0.0;
                                        self.canvas_w_input = self.settings.canvas.base_w.to_string();
                                        self.canvas_h_input = self.settings.canvas.base_h.to_string();
                                        self.dirty      = true;
                                    }
                                }
                                if !self.custom_presets.is_empty() {
                                    ui.separator();
                                    ui.label(RichText::new("Custom").small().strong());
                                    for (i, cp) in self.custom_presets.iter().enumerate() {
                                        if ui.selectable_label(
                                            self.is_custom && self.active_custom_idx == Some(i),
                                            RichText::new(&cp.name).small()
                                        ).clicked() {
                                            self.settings = cp.settings.clone();
                                            self.is_custom = true;
                                            self.active_custom_idx = Some(i);
                                            self.scroll = 0.0;
                                            self.canvas_w_input = self.settings.canvas.base_w.to_string();
                                            self.canvas_h_input = self.settings.canvas.base_h.to_string();
                                            self.dirty = true;
                                        }
                                    }
                                }
                            });
                        ui.label(RichText::new("PRESET →").color(Color32::from_rgb(40,36,28)).small());
                    });
                });
            });
    }

    // ── Right panel (parameter sections) ────────────────────────────────────
    fn right_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::right("params")
            .min_width(230.0)
            .max_width(270.0)
            .frame(egui::Frame::none()
                .fill(Color32::from_rgb(6,4,2))
                .inner_margin(egui::Margin::same(4.0)))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    // ── CANVAS ───────────────────────────────────────────
                    let o = &mut self.open_canvas;
                    let ch = Self::section(ui, "CANVAS", o, |ui, mut changed| {
                        let c = &mut self.settings.canvas;
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Width").color(Color32::from_rgb(100,80,40)).small());
                            changed |= ui.add(
                                Slider::new(&mut c.base_w, 360u32..=3840)
                                    .integer()
                                    .step_by(16.0)
                            ).changed();
                        });
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Height").color(Color32::from_rgb(100,80,40)).small());
                            changed |= ui.add(
                                Slider::new(&mut c.base_h, 360u32..=3840)
                                    .integer()
                                    .step_by(16.0)
                            ).changed();
                        });
                        changed |= checkbox(ui, "Landscape (rotate 90°)", &mut c.landscape);

                        ui.horizontal(|ui| {
                            ui.label(RichText::new("W").color(Color32::from_rgb(100,80,40)).small());
                            ui.add(egui::TextEdit::singleline(&mut self.canvas_w_input).desired_width(52.0));
                            ui.label(RichText::new("H").color(Color32::from_rgb(100,80,40)).small());
                            ui.add(egui::TextEdit::singleline(&mut self.canvas_h_input).desired_width(52.0));
                        });
                        ui.horizontal(|ui| {
                            if ui.small_button("Apply size").clicked() {
                                let p_w = self.canvas_w_input.trim().parse::<u32>().ok();
                                let p_h = self.canvas_h_input.trim().parse::<u32>().ok();
                                if let (Some(mut w), Some(mut h)) = (p_w, p_h) {
                                    w = w.clamp(360, 3840);
                                    h = h.clamp(360, 3840);
                                    w = ((w + 8) / 16) * 16;
                                    h = ((h + 8) / 16) * 16;
                                    if c.base_w != w || c.base_h != h {
                                        c.base_w = w;
                                        c.base_h = h;
                                        changed = true;
                                    }
                                    self.canvas_w_input = c.base_w.to_string();
                                    self.canvas_h_input = c.base_h.to_string();
                                }
                            }
                            if ui.small_button("Revert").clicked() {
                                self.canvas_w_input = c.base_w.to_string();
                                self.canvas_h_input = c.base_h.to_string();
                            }
                        });
                        changed
                    });
                    if ch {
                        self.settings.canvas.base_w = self.settings.canvas.base_w.clamp(360, 3840);
                        self.settings.canvas.base_h = self.settings.canvas.base_h.clamp(360, 3840);
                        self.latest_frame_id = self.latest_frame_id.wrapping_add(1);
                        self.respawn_render_thread();
                        // Resize buffer to match new canvas dimensions
                        let new_len = self.settings.canvas.w() * self.settings.canvas.h() * 4;
                        self.pixel_buf = vec![0u8; new_len];
                        self.texture   = None;
                        self.canvas_w_input = self.settings.canvas.base_w.to_string();
                        self.canvas_h_input = self.settings.canvas.base_h.to_string();
                        self.is_custom = true;
                        self.dirty     = true;
                    }

                    // ── SCENE ────────────────────────────────────────────
                    let canvas_h_u32 = self.settings.canvas.h() as u32;
                    let canvas_w_f32 = self.settings.canvas.w() as f32;
                    let o = &mut self.open_scene;
                    let ch = Self::section(ui, "SCENE", o, |ui, mut changed| {
                        let mut apply_tuning = false;
                        {
                            let s = &mut self.settings.scene;
                            let h_max = canvas_h_u32.saturating_add(1200).max(640);
                            let hw_max = (canvas_w_f32 * 3.0).max(220.0);
                            changed |= knob_u32(ui, "Horizon Y",       &mut s.horizon_y,  0, h_max);
                            changed |= knob_f32(ui, "Path half-width",  &mut s.max_hw,    5.0, hw_max, 1.0);
                            changed |= knob_f32(ui, "Camera height",    &mut s.cam_h,     0.05, 20.0, 0.05);
                            changed |= knob_f32(ui, "FOV multiplier",   &mut s.focal_mult,0.05, 8.0, 0.05);
                            changed |= knob_f32(ui, "Path curve power", &mut s.path_power,0.05,8.0, 0.05);
                            changed |= knob_f32(ui, "Horizon curve",    &mut s.horizon_curve, -1.0, 1.0, 0.01);
                            changed |= colour3(ui,  "Void / sky",       &mut s.void_color);
                            changed |= checkbox(ui, "Grass at edges",   &mut s.grass_enabled);
                            if s.grass_enabled {
                                changed |= colour3(ui, "Grass colour", &mut s.grass_color);
                                changed |= knob_f32(ui, "Grass density", &mut s.grass_density, 0.1, 4.0, 0.01);
                                changed |= knob_f32(ui, "Grass height", &mut s.grass_height, 0.2, 3.0, 0.01);
                                changed |= knob_f32(ui, "Grass upright", &mut s.grass_upright, 0.0, 1.0, 0.01);
                            }
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("Ambient light").color(Color32::from_rgb(100,80,40)).small());
                                changed |= knob_f32(ui, "", &mut s.ambient, 0.0, 1.0, 0.01);
                            });
                            changed |= pick_lighting_preset(ui, &mut s.lighting_preset);
                            changed |= knob_f32(ui, "Atmo light influence", &mut s.atmo_light_influence, 0.0, 2.0, 0.01);
                            changed |= knob_f32(ui, "Atmo tint influence", &mut s.atmo_tint_influence, 0.0, 1.0, 0.01);
                        }
                        if ui.small_button("Apply Preset Tuning").clicked() {
                            apply_tuning = true;
                        }
                        ui.label(
                            RichText::new(Self::lighting_preset_preview_text(
                                &self.settings.scene.lighting_preset,
                            ))
                            .color(Color32::from_rgb(90, 82, 64))
                            .small(),
                        );
                        if apply_tuning {
                            Self::apply_lighting_preset_tuning(&mut self.settings);
                            changed = true;
                        }
                        changed
                    });
                    if ch { self.is_custom = true; self.dirty = true; }

                    // ── PATH / FLOOR ─────────────────────────────────────
                    let o = &mut self.open_floor;
                    let ch = Self::section(ui, "PATH / FLOOR", o, |ui, mut changed| {
                        let s = &mut self.settings.floor;
                        changed |= knob_f32(ui, "Depth fade",    &mut s.depth_fade,    0.05, 30.0, 0.1);
                        changed |= knob_f32(ui, "Edge vignette", &mut s.edge_vignette, 0.0,  1.0, 0.02);
                        changed |= knob_u32(ui, "Tile noise",    &mut s.noise,         0,   200);
                        changed |= knob_f32(ui, "Texture scale", &mut s.tex_scale,     0.01, 64.0, 0.05);
                        changed |= checkbox(ui, "Rotate texture 90deg", &mut s.tex_rot_90);
                        changed |= knob_f32(ui, "Damage / cracks", &mut s.damage,       0.0, 1.0, 0.01);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Variation seed").color(Color32::from_rgb(100,80,40)).small());
                            changed |= ui.add(egui::DragValue::new(&mut s.variation_seed).speed(1)).changed();
                        });
                        changed |= pick_floor(ui, &mut s.pattern);
                        changed |= colour3(ui, "Base",   &mut s.base);
                        changed |= colour3(ui, "Mortar", &mut s.mortar);
                        changed
                    });
                    if ch { self.is_custom = true; self.dirty = true; }

                    // ── WALLS ─────────────────────────────────────────────
                    let o = &mut self.open_walls;
                    let ch = Self::section(ui, "WALLS", o, |ui, mut changed| {
                        let s = &mut self.settings.walls;
                        changed |= checkbox(ui, "Enable walls",       &mut s.enabled);
                        changed |= knob_f32(ui, "Height to top",      &mut s.top_coverage, 0.0, 1.0, 0.01);
                        changed |= knob_f32(ui, "Texture scale",      &mut s.tex_scale,   0.01, 64.0, 0.05);
                        changed |= checkbox(ui, "Rotate texture 90deg", &mut s.tex_rot_90);
                        changed |= knob_f32(ui, "Damage / cracks",    &mut s.damage,      0.0, 1.0, 0.01);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Variation seed").color(Color32::from_rgb(100,80,40)).small());
                            changed |= ui.add(egui::DragValue::new(&mut s.variation_seed).speed(1)).changed();
                        });
                        changed |= knob_f32(ui, "Wall distance",     &mut s.l_wx,        0.01, 8.0, 0.05);
                        changed |= knob_f32(ui, "Brightness",        &mut s.bright,      0.0, 12.0, 0.1);
                        changed |= knob_f32(ui, "Junction shadow",   &mut s.junc_shadow, 0.0, 240.0, 1.0);
                        changed |= knob_u32(ui, "Void-fade rows",    &mut s.fade_rows,   1,   400);
                        changed |= knob_u32(ui, "Tile noise",        &mut s.noise,       0,   200);
                        changed |= pick_wall(ui, &mut s.pattern);
                        changed |= colour3(ui, "Base",   &mut s.base);
                        changed |= colour3(ui, "Mortar", &mut s.mortar);
                        changed
                    });
                    if ch { self.is_custom = true; self.dirty = true; }

                    // ── SKY (new in 2.0) ─────────────────────────────────
                    let o = &mut self.open_sky;
                    let ch = Self::section(ui, "SKY", o, |ui, mut changed| {
                        let s = &mut self.settings.sky;
                        changed |= checkbox(ui, "Enable sky gradient", &mut s.enabled);
                        changed |= colour3(ui, "Top colour",     &mut s.top);
                        changed |= colour3(ui, "Horizon colour", &mut s.horizon);
                        changed |= checkbox(ui, "Enable stars", &mut s.stars_enabled);
                        if s.stars_enabled {
                            changed |= knob_u32(ui, "Star count", &mut s.stars_count, 0, 4000);
                            changed |= knob_f32(ui, "Star size", &mut s.stars_size, 0.1, 8.0, 0.05);
                            changed |= knob_f32(ui, "Star twinkle", &mut s.stars_twinkle, 0.0, 4.0, 0.05);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("Stars seed").color(Color32::from_rgb(100,80,40)).small());
                                changed |= ui.add(egui::DragValue::new(&mut s.stars_seed).speed(1)).changed();
                            });
                        }
                        changed |= checkbox(ui, "Enable sun", &mut s.sun_enabled);
                        if s.sun_enabled {
                            changed |= knob_f32(ui, "Sun X", &mut s.sun_pos[0], -1.0, 2.0, 0.01);
                            changed |= knob_f32(ui, "Sun Y", &mut s.sun_pos[1], -1.0, 2.0, 0.01);
                            changed |= knob_f32(ui, "Sun radius", &mut s.sun_radius, 0.01, 1.0, 0.005);
                            changed |= colour3(ui, "Sun colour", &mut s.sun_color);
                        }
                        changed |= checkbox(ui, "Enable moon", &mut s.moon_enabled);
                        if s.moon_enabled {
                            changed |= knob_f32(ui, "Moon X", &mut s.moon_pos[0], -1.0, 2.0, 0.01);
                            changed |= knob_f32(ui, "Moon Y", &mut s.moon_pos[1], -1.0, 2.0, 0.01);
                            changed |= knob_f32(ui, "Moon radius", &mut s.moon_radius, 0.01, 1.0, 0.005);
                            changed |= knob_f32(ui, "Moon phase", &mut s.moon_phase, -1.0, 1.0, 0.01);
                            changed |= knob_f32(ui, "Moon alpha", &mut s.moon_alpha, 0.0, 2.0, 0.01);
                            changed |= checkbox(ui, "Moon texture", &mut s.moon_texture_enabled);
                            if s.moon_texture_enabled {
                                changed |= knob_f32(ui, "Texture scale", &mut s.moon_texture_scale, 0.2, 4.0, 0.02);
                            }
                            changed |= colour3(ui, "Moon colour", &mut s.moon_color);
                        }
                        changed |= checkbox(ui, "Enable clouds", &mut s.clouds_enabled);
                        if s.clouds_enabled {
                            changed |= knob_u32(ui, "Cloud count", &mut s.cloud_count, 0, 180);
                            changed |= knob_f32(ui, "Cloud speed", &mut s.cloud_speed, 0.0, 4.0, 0.01);
                            changed |= knob_f32(ui, "Cloud scale", &mut s.cloud_scale, 0.2, 4.0, 0.02);
                            changed |= knob_f32(ui, "Cloud opacity", &mut s.cloud_opacity, 0.0, 2.0, 0.01);
                            changed |= knob_f32(ui, "Cloud variation", &mut s.cloud_variation, 0.0, 1.0, 0.01);
                            changed |= colour3(ui, "Cloud tint", &mut s.cloud_tint);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("Cloud seed").color(Color32::from_rgb(100,80,40)).small());
                                changed |= ui.add(egui::DragValue::new(&mut s.cloud_seed).speed(1)).changed();
                            });
                        }
                        changed
                    });
                    if ch { self.is_custom = true; self.dirty = true; }

                    // ── ATMOSPHERE (multi-layer) ─────────────────────────
                    {
                        let o = &mut self.open_atmo;
                        let ch = Self::section(ui, "ATMOSPHERE", o, |ui, mut changed| {
                            let layers = &mut self.settings.atmo.layers;
                            let olen   = &mut self.open_atmo_layers;
                            while olen.len() < layers.len() { olen.push(false); }
                            olen.truncate(layers.len());

                            let mut to_remove: Option<usize> = None;
                            for (i, layer) in layers.iter_mut().enumerate() {
                                ui.push_id(("atmo-layer", i), |ui| {
                                    ui.horizontal(|ui| {
                                        changed |= ui.checkbox(&mut layer.enabled, "").changed();
                                        let lbl = format!("Layer {} ({})", i+1, layer.atmo_type.name());
                                        ui.toggle_value(&mut olen[i], lbl);
                                        if ui.small_button("✕").clicked() { to_remove = Some(i); }
                                    });
                                    if olen[i] {
                                        changed |= pick_atmo(ui, &mut layer.atmo_type);
                                        changed |= knob_f32(ui, "Light height",  &mut layer.torch_h,     -4.0,  20.0,  0.1);
                                        changed |= knob_u32(ui, "Light spacing", &mut layer.torch_spc,   1,    256);
                                        changed |= knob_f32(ui, "Light size",    &mut layer.torch_scale, 0.001, 2.5, 0.005);
                                        changed |= knob_f32(ui, "FX scale",      &mut layer.fx_scale,    0.05,  12.0, 0.02);
                                        changed |= knob_f32(ui, "Placement jitter", &mut layer.placement_jitter, 0.0, 6.0, 0.01);
                                        changed |= knob_f32(ui, "Flicker",       &mut layer.flicker,     0.0,  8.0, 0.02);
                                        changed |= knob_u32(ui, "Dust/particles",&mut layer.n_motes,     0,    300);
                                        changed |= knob_u32(ui, "Floor debris",  &mut layer.n_debris,    0,    150);
                                        changed |= knob_f32(ui, "Sprite scale",  &mut layer.sprite_scale, 0.05, 20.0, 0.05);
                                        changed |= checkbox(ui, "Sprite flip X", &mut layer.sprite_flip_x);
                                        changed |= checkbox(ui, "Sprite flip Y", &mut layer.sprite_flip_y);
                                        changed |= knob_f32(ui, "Sprite rotation", &mut layer.sprite_rot_deg, -180.0, 180.0, 1.0);
                                        changed |= knob_f32(ui, "Sprite offset X", &mut layer.sprite_offset_x, -20.0, 20.0, 0.05);
                                        changed |= knob_f32(ui, "Sprite offset Y", &mut layer.sprite_offset_y, -20.0, 20.0, 0.05);
                                        ui.horizontal(|ui| {
                                            ui.label(RichText::new("Sprite PNG path").color(Color32::from_rgb(100,80,40)).small());
                                            if ui.small_button("Browse").clicked() {
                                                if let Some(p) = rfd::FileDialog::new()
                                                    .add_filter("PNG image", &["png"])
                                                    .pick_file()
                                                {
                                                    layer.sprite_path = p.display().to_string();
                                                    changed = true;
                                                }
                                            }
                                        });
                                        changed |= ui.text_edit_singleline(&mut layer.sprite_path).changed();
                                        changed |= checkbox(ui, "Randomize from sprite pool", &mut layer.sprite_pool_enabled);
                                        ui.horizontal(|ui| {
                                            ui.label(RichText::new("Sprite pool (; or newline separated)").color(Color32::from_rgb(100,80,40)).small());
                                        });
                                        changed |= ui.text_edit_multiline(&mut layer.sprite_pool_paths).changed();
                                        if !layer.sprite_path.trim().is_empty() {
                                            let ok = Path::new(layer.sprite_path.trim()).exists();
                                            let txt = if ok { "Sprite: found" } else { "Sprite: missing" };
                                            let col = if ok { Color32::from_rgb(90, 190, 90) } else { Color32::from_rgb(200, 90, 70) };
                                            ui.label(RichText::new(txt).color(col).small());
                                        }
                                        ui.horizontal(|ui| {
                                            ui.label(RichText::new("Variation seed").color(Color32::from_rgb(100,80,40)).small());
                                            changed |= ui.add(egui::DragValue::new(&mut layer.variation_seed).speed(1)).changed();
                                        });
                                    }
                                });
                            }
                            if let Some(i) = to_remove {
                                layers.remove(i);
                                olen.remove(i);
                                changed = true;
                            }
                            if ui.small_button("+ Add layer").clicked() {
                                layers.push(AtmoLayer::new(AtmoType::None));
                                olen.push(true);
                                changed = true;
                            }
                            changed
                        });
                        if ch { self.is_custom = true; self.dirty = true; }
                    }

                    // ── PROPS ─────────────────────────────────────────────
                    {
                        let o = &mut self.open_props;
                        let ch = Self::section(ui, "PROPS", o, |ui, mut changed| {
                            let items = &mut self.settings.props.items;
                            let olen  = &mut self.open_prop_items;
                            while olen.len() < items.len() { olen.push(false); }
                            olen.truncate(items.len());

                            let mut to_remove: Option<usize> = None;
                            for (i, item) in items.iter_mut().enumerate() {
                                ui.push_id(("prop-item", i), |ui| {
                                    ui.horizontal(|ui| {
                                        changed |= ui.checkbox(&mut item.enabled, "").changed();
                                        let lbl = format!("[{}] {}", i+1, item.prop_type.name());
                                        ui.toggle_value(&mut olen[i], lbl);
                                        if ui.small_button("✕").clicked() { to_remove = Some(i); }
                                    });
                                    if olen[i] {
                                        changed |= pick_prop_type(ui, &mut item.prop_type);
                                        changed |= knob_f32(ui, "Side offset (wx)", &mut item.wx,        0.01, 12.0, 0.1);
                                        changed |= checkbox(ui, "Mirror",           &mut item.mirror);
                                        changed |= knob_f32(ui, "Start distance",   &mut item.start_wz,  0.01, 32.0, 0.05);
                                        changed |= knob_f32(ui, "End distance",     &mut item.end_wz,    0.2,  256.0, 0.1);
                                        changed |= knob_f32(ui, "Z spacing",        &mut item.z_spacing, 0.05, 64.0, 0.05);
                                        changed |= knob_f32(ui, "Scale",            &mut item.scale,     0.02, 24.0, 0.05);
                                        changed |= knob_f32(ui, "Width scale",      &mut item.width_scale, 0.02, 24.0, 0.05);
                                        changed |= knob_f32(ui, "Height scale",     &mut item.height_scale, 0.02, 24.0, 0.05);
                                        changed |= knob_f32(ui, "Scale variation",  &mut item.scale_var, 0.0, 3.0, 0.01);
                                        changed |= knob_f32(ui, "Width variation",  &mut item.width_var, 0.0, 3.0, 0.01);
                                        changed |= knob_f32(ui, "Height variation", &mut item.height_var, 0.0, 3.0, 0.01);
                                        changed |= knob_f32(ui, "Path follow",      &mut item.path_follow, 0.0, 1.0, 0.01);
                                        changed |= knob_f32(ui, "Edge gap",         &mut item.edge_gap, 0.0, 2.0, 0.01);
                                        changed |= checkbox(ui, "Enable X jitter",  &mut item.x_jitter_enabled);
                                        changed |= knob_f32(ui, "X jitter",         &mut item.x_jitter,  0.0, 4.0, 0.01);
                                        changed |= checkbox(ui, "Enable Y jitter",  &mut item.y_jitter_enabled);
                                        changed |= knob_f32(ui, "Y jitter",         &mut item.y_jitter,  0.0, 4.0, 0.01);
                                        changed |= knob_f32(ui, "Y sink",           &mut item.y_sink,    0.0, 10.0, 0.05);
                                        changed |= knob_f32(ui, "Ground blend",     &mut item.ground_blend, 0.0, 1.0, 0.01);
                                        changed |= knob_f32(ui, "Shadow size",      &mut item.shadow_size, 0.2, 4.0, 0.02);
                                        changed |= knob_f32(ui, "Shadow length",    &mut item.shadow_length, 0.2, 5.0, 0.02);
                                        changed |= knob_f32(ui, "Shadow direction", &mut item.shadow_dir, -2.0, 2.0, 0.02);
                                        changed |= knob_f32(ui, "Shadow follows light", &mut item.shadow_follow_light, 0.0, 1.0, 0.01);
                                        changed |= knob_f32(ui, "Shadow opacity",   &mut item.shadow_opacity, 0.1, 1.5, 0.01);
                                        changed |= knob_f32(ui, "Shadow softness",  &mut item.shadow_softness, 0.3, 3.0, 0.01);
                                        ui.horizontal(|ui| {
                                            if ui.small_button("Shadow: Natural").clicked() {
                                                item.shadow_size = 1.0;
                                                item.shadow_length = 1.0;
                                                item.shadow_softness = 1.0;
                                                item.shadow_opacity = 0.82;
                                                item.shadow_follow_light = 0.7;
                                                changed = true;
                                            }
                                            if ui.small_button("Shadow: Long Cast").clicked() {
                                                item.shadow_size = 1.08;
                                                item.shadow_length = 1.65;
                                                item.shadow_softness = 1.15;
                                                item.shadow_opacity = 0.88;
                                                item.shadow_follow_light = 0.82;
                                                changed = true;
                                            }
                                            if ui.small_button("Shadow: Soft Overcast").clicked() {
                                                item.shadow_size = 1.12;
                                                item.shadow_length = 0.86;
                                                item.shadow_softness = 1.75;
                                                item.shadow_opacity = 0.62;
                                                item.shadow_follow_light = 0.55;
                                                changed = true;
                                            }
                                        });
                                        changed |= knob_f32(ui, "Sprite scale",      &mut item.sprite_scale, 0.05, 20.0, 0.05);
                                        changed |= checkbox(ui, "Sprite flip X", &mut item.sprite_flip_x);
                                        changed |= checkbox(ui, "Sprite flip Y", &mut item.sprite_flip_y);
                                        changed |= knob_f32(ui, "Sprite rotation", &mut item.sprite_rot_deg, -180.0, 180.0, 1.0);
                                        changed |= knob_f32(ui, "Sprite offset X", &mut item.sprite_offset_x, -20.0, 20.0, 0.05);
                                        changed |= knob_f32(ui, "Sprite offset Y", &mut item.sprite_offset_y, -20.0, 20.0, 0.05);
                                        ui.horizontal(|ui| {
                                            ui.label(RichText::new("Sprite PNG path").color(Color32::from_rgb(100,80,40)).small());
                                            if ui.small_button("Browse").clicked() {
                                                if let Some(p) = rfd::FileDialog::new()
                                                    .add_filter("PNG image", &["png"])
                                                    .pick_file()
                                                {
                                                    item.sprite_path = p.display().to_string();
                                                    changed = true;
                                                }
                                            }
                                        });
                                        changed |= ui.text_edit_singleline(&mut item.sprite_path).changed();
                                        changed |= checkbox(ui, "Randomize from sprite pool", &mut item.sprite_pool_enabled);
                                        ui.horizontal(|ui| {
                                            ui.label(RichText::new("Sprite pool (; or newline separated)").color(Color32::from_rgb(100,80,40)).small());
                                        });
                                        changed |= ui.text_edit_multiline(&mut item.sprite_pool_paths).changed();
                                        if !item.sprite_path.trim().is_empty() {
                                            let ok = Path::new(item.sprite_path.trim()).exists();
                                            let txt = if ok { "Sprite: found" } else { "Sprite: missing" };
                                            let col = if ok { Color32::from_rgb(90, 190, 90) } else { Color32::from_rgb(200, 90, 70) };
                                            ui.label(RichText::new(txt).color(col).small());
                                        }
                                        changed |= knob_f32(ui, "Tree style mix",   &mut item.tree_style_mix, 0.0, 1.0, 0.01);
                                        changed |= knob_f32(ui, "Tree style bias",  &mut item.tree_style_bias,-1.0, 1.0, 0.02);
                                        ui.horizontal(|ui| {
                                            ui.label(RichText::new("Rows").color(Color32::from_rgb(100,80,40)).small());
                                            changed |= ui.add(egui::DragValue::new(&mut item.tree_row_count).range(1..=32).speed(1)).changed();
                                        });
                                        changed |= knob_f32(ui, "Row distance", &mut item.tree_row_spacing, 0.0, 20.0, 0.01);
                                        changed |= knob_f32(ui, "Row jitter", &mut item.tree_row_jitter, 0.0, 8.0, 0.01);
                                        ui.horizontal(|ui| {
                                            ui.label(RichText::new("Variation seed").color(Color32::from_rgb(100,80,40)).small());
                                            changed |= ui.add(egui::DragValue::new(&mut item.seed).speed(1)).changed();
                                        });
                                        changed |= colour3(ui, "Tint",              &mut item.tint);
                                    }
                                });
                            }
                            if let Some(i) = to_remove {
                                items.remove(i);
                                olen.remove(i);
                                changed = true;
                            }
                            if ui.small_button("+ Add prop").clicked() {
                                items.push(PropInstance::new(PropType::Tree));
                                olen.push(true);
                                changed = true;
                            }
                            changed
                        });
                        if ch { self.is_custom = true; self.dirty = true; }
                    }

                    // ── POST-FX ───────────────────────────────────────────
                    {
                        let o = &mut self.open_post;
                        let ch = Self::section(ui, "POST-FX", o, |ui, mut changed| {
                            let p = &mut self.settings.post;
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("Vignette").color(Color32::from_rgb(100,80,40)).small());
                                changed |= knob_f32(ui, "", &mut p.vignette, 0.0, 1.0, 0.01);
                            });
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("Saturation").color(Color32::from_rgb(100,80,40)).small());
                                changed |= knob_f32(ui, "", &mut p.saturation, 0.0, 2.5, 0.02);
                            });
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("Bloom").color(Color32::from_rgb(100,80,40)).small());
                                changed |= knob_f32(ui, "", &mut p.bloom, 0.0, 1.0, 0.01);
                            });
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("Film grain").color(Color32::from_rgb(100,80,40)).small());
                                changed |= knob_f32(ui, "", &mut p.grain, 0.0, 0.5, 0.005);
                            });
                            changed |= checkbox(ui, "GPU scene preview (phase 2)", &mut self.gpu_scene_preview_enabled);
                            changed |= checkbox(ui, "GPU scene export (phase 2)", &mut self.gpu_scene_export_enabled);
                            changed |= checkbox(ui, "GPU preview shading", &mut self.gpu_preview_enabled);
                            changed |= checkbox(ui, "GPU export shading", &mut self.gpu_export_enabled);
                            changed |= knob_f32(ui, "GPU shader mix", &mut self.gpu_effect_mix, 0.0, 1.0, 0.01);
                            changed |= knob_f32(ui, "GPU saturation", &mut self.gpu_saturation, 0.5, 2.5, 0.01);
                            changed |= knob_f32(ui, "GPU contrast", &mut self.gpu_contrast, 0.5, 3.0, 0.01);
                            changed |= knob_f32(ui, "GPU brightness", &mut self.gpu_brightness, 0.5, 3.0, 0.01);
                            changed |= checkbox(ui, "Depth fog", &mut p.fog_enabled);
                            if p.fog_enabled {
                                changed |= colour3(ui, "Fog colour", &mut p.fog_color);
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new("Fog density").color(Color32::from_rgb(100,80,40)).small());
                                    changed |= knob_f32(ui, "", &mut p.fog_density, 0.0, 1.0, 0.01);
                                });
                            }
                            changed
                        });
                        if ch { self.is_custom = true; self.dirty = true; }
                    }

                    // ── ANIMATION ────────────────────────────────────────
                    let mut open_anim = self.open_anim;
                    let ch_anim = Self::section(ui, "ANIMATION", &mut open_anim, |ui, mut changed| {
                        let tiles_per_sec;
                        let preview_secs;
                        let loop_rate;
                        {
                            let s = &mut self.settings.anim;
                            changed |= knob_u32(ui, "Loop length (tiles)", &mut s.loop_s,     1, 64);
                            s.seamless_lock = true;
                            let mut tps = path_speed_tiles_per_sec(s);
                            let speed_changed = knob_f32(ui, "Path speed (tiles/sec)", &mut tps, 0.05, 300.0, 0.05);
                            if speed_changed {
                                let new_play = (tps / s.loop_s.max(1) as f32).clamp(0.005, 120.0);
                                if (new_play - s.play_speed).abs() > 0.0001 {
                                    s.play_speed = new_play;
                                    changed = true;
                                }
                            }
                            tiles_per_sec = tps;
                            preview_secs = loop_duration_secs(s);
                            loop_rate = s.play_speed.max(0.005);
                        }
                        ui.label(RichText::new(format!(
                            "Loop duration: {:.2}s | Loop rate: {:.3} loops/sec",
                            preview_secs,
                            loop_rate,
                        )).color(Color32::from_rgb(130,120,95)).small());
                        changed |= knob_u32(ui, "Frames per second (render/export)", &mut self.preview_fps_cap, 1, 120);
                        changed |= checkbox(ui, "Temporal smoothing", &mut self.temporal_smoothing_enabled);
                        if self.temporal_smoothing_enabled {
                            changed |= knob_u32(ui, "Smoothing boost (x fps + frames)", &mut self.temporal_smoothing_boost, 2, 8);
                        } else {
                            self.temporal_smoothing_boost = 1;
                        }
                        changed |= knob_f32(
                            ui,
                            "Quality target (max tiles/frame)",
                            &mut self.motion_quality_tpf_target,
                            0.03,
                            1.00,
                            0.01,
                        );

                        self.motion_quality_tpf_target = self.motion_quality_tpf_target.clamp(0.03, 1.0);

                        let effective_fps = self.effective_output_fps();
                        let hardlock_frames = hardlock_total_frames(&self.settings.anim, effective_fps);
                        if self.gif_total_frames != hardlock_frames {
                            self.gif_total_frames = hardlock_frames;
                            changed = true;
                        }
                        let min_smoothing = min_smoothing_for_speed(
                            tiles_per_sec,
                            effective_fps,
                            self.motion_quality_tpf_target,
                        );
                        if self.gif_smoothing_samples < min_smoothing {
                            self.gif_smoothing_samples = min_smoothing;
                            changed = true;
                        }
                        changed |= knob_u32(ui, "Smoothing subframes", &mut self.gif_smoothing_samples, min_smoothing, 8);

                        changed |= knob_f32(ui, "Preview render scale", &mut self.preview_scale, 0.35, 1.0, 0.01);
                        let est_frames = self.gif_total_frames.max(2);
                        let gif_secs = est_frames as f32 / effective_fps.max(1) as f32;
                        ui.label(RichText::new(format!(
                            "Hardlock export: {} frames @ {} fps ({:.2}s)",
                            est_frames,
                            effective_fps,
                            gif_secs,
                        )).color(Color32::from_rgb(160,150,120)).small());
                        ui.label(RichText::new(format!(
                            "Base FPS: {} | Effective FPS: {} | Effective frames/loop: {}",
                            self.preview_fps_cap.max(1),
                            effective_fps,
                            est_frames,
                        )).color(Color32::from_rgb(130,120,95)).small());
                        ui.label(RichText::new(format!(
                            "Preview loop: {:.2}s | GIF loop: {:.2}s",
                            preview_secs,
                            gif_secs,
                        )).color(Color32::from_rgb(130,120,95)).small());
                        ui.label(RichText::new(
                            "Hardlock is always enabled: preview timing and export timing are automatically synchronized."
                        ).color(Color32::from_rgb(170,140,96)).small());
                        ui.label(RichText::new(
                            "Lower quality target = more smoothing at high speed (cleaner motion). Higher target = fewer subframes (faster export)."
                        ).color(Color32::from_rgb(145,125,92)).small());
                        ui.label(RichText::new(
                            "Path speed controls all relative motion. Frames and minimum smoothing are auto-derived from speed to preserve quality and smoothness across slow and fast loops."
                        ).color(Color32::from_rgb(40,36,28)).small());

                        // ── Seamless Loop Diagnostics ───────────────────
                        ui.add_space(4.0);
                        ui.separator();
                        ui.label(RichText::new("SEAMLESS LOOP DIAGNOSTICS")
                            .color(Color32::from_rgb(150,124,56)).small().strong());
                        ui.label(RichText::new(
                            "For a seamless GIF, each prop z_spacing and atmo torch_spc must divide loop_s evenly. Stars and clouds loop automatically."
                        ).color(Color32::from_rgb(90,80,56)).small());

                        let loop_tiles = self.settings.anim.loop_s.max(1) as f32;
                        let mut all_seamless = true;

                        // Per-prop check
                        for pi in 0..self.settings.props.items.len() {
                            if !self.settings.props.items[pi].enabled { continue; }
                            let spc = self.settings.props.items[pi].z_spacing.max(0.01);
                            let rem = loop_tiles % spc;
                            let drift = rem.min(spc - rem) / spc;
                            let ok = drift < 0.02;
                            if !ok { all_seamless = false; }
                            let label_col = if ok { Color32::from_rgb(72,184,72) } else { Color32::from_rgb(210,90,50) };
                            let marker = if ok { "\u{2713}" } else { "\u{26a0}" };
                            let nearest_n = (loop_tiles / spc).round().max(1.0);
                            let snap = loop_tiles / nearest_n;
                            let snap_val = snap; // capture before borrow
                            let pname = self.settings.props.items[pi].prop_type.name();
                            let pz = self.settings.props.items[pi].z_spacing;
                            let mut do_snap = false;
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(format!(
                                    "{} Prop {}: {} z={:.2}", marker, pi+1, pname, pz))
                                    .color(label_col).small());
                                if !ok && ui.small_button(format!("\u{2192}{:.2}", snap_val)).clicked() {
                                    do_snap = true;
                                }
                            });
                            if do_snap {
                                self.settings.props.items[pi].z_spacing = snap_val;
                                changed = true;
                            }
                        }

                        // Per-atmo check
                        for ai in 0..self.settings.atmo.layers.len() {
                            if !self.settings.atmo.layers[ai].enabled { continue; }
                            if matches!(self.settings.atmo.layers[ai].atmo_type, AtmoType::None) { continue; }
                            let spc_f = self.settings.atmo.layers[ai].torch_spc.max(1) as f32;
                            let rem = loop_tiles % spc_f;
                            let ok = rem < 0.01 || (spc_f - rem) < 0.01;
                            if !ok { all_seamless = false; }
                            let label_col = if ok { Color32::from_rgb(72,184,72) } else { Color32::from_rgb(210,90,50) };
                            let marker = if ok { "\u{2713}" } else { "\u{26a0}" };
                            let nearest_n = (loop_tiles / spc_f).round().max(1.0);
                            let snap_u = (loop_tiles / nearest_n).round().max(1.0) as u32;
                            let aname = self.settings.atmo.layers[ai].atmo_type.name();
                            let aspc = self.settings.atmo.layers[ai].torch_spc;
                            let mut do_snap = false;
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(format!(
                                    "{} Atmo {}: {} spc={}", marker, ai+1, aname, aspc))
                                    .color(label_col).small());
                                if !ok && ui.small_button(format!("\u{2192}{}", snap_u)).clicked() {
                                    do_snap = true;
                                }
                            });
                            if do_snap {
                                self.settings.atmo.layers[ai].torch_spc = snap_u;
                                changed = true;
                            }
                        }

                        let status_col = if all_seamless { Color32::from_rgb(72,184,72) } else { Color32::from_rgb(210,90,50) };
                        let status_txt = if all_seamless {
                            "\u{2713} All spacing seamless for current loop length"
                        } else {
                            "\u{26a0} Use Snap buttons above to fix spacing"
                        };
                        ui.label(RichText::new(status_txt).color(status_col).small().strong());
                        ui.label(RichText::new(format!(
                            "Tip: set z_spacing = loop_s / N (e.g. loop_s={} \u{2192} z=1,2,4,8...)",
                            self.settings.anim.loop_s))
                            .color(Color32::from_rgb(90,80,56)).small());

                        changed
                    });
                    self.open_anim = open_anim;
                    if ch_anim { self.is_custom = true; self.dirty = true; }

                    // ── NODE LAB ───────────────────────────────────────
                    let o = &mut self.open_node_lab;
                    let ch = Self::section(ui, "NODE LAB", o, |ui, mut changed| {
                        changed |= checkbox(ui, "Open Node Lab Window", &mut self.node_lab.open);
                        ui.label(RichText::new(
                            "Floating window scaffold for node-based texture/image graph editing."
                        ).color(Color32::from_rgb(100,90,68)).small());
                        changed
                    });
                    if ch { self.is_custom = true; }

                    ui.separator();

                    // ── GIF EXPORT ───────────────────────────────────────
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("GIF project name").color(Color32::from_rgb(100,80,40)).small());
                        ui.add(egui::TextEdit::singleline(&mut self.gif_project_name).desired_width(180.0));
                    });
                    if checkbox(ui, "Export individual layer GIFs (slower)", &mut self.export_individual_layers) {
                        self.is_custom = true;
                    }
                    if ui.button(RichText::new("▶ EXPORT GIF")
                            .color(Color32::from_rgb(200,144,32)).small())
                        .clicked() && !self.exporting
                    {
                        self.do_export();
                    }
                    if !self.export_msg.is_empty() {
                        ui.label(RichText::new(&self.export_msg)
                            .color(Color32::from_rgb(100,180,80)).small());
                    }
                    if self.exporting {
                        ui.add(egui::ProgressBar::new(self.export_progress).show_percentage());
                        if !self.export_stage.is_empty() {
                            ui.label(RichText::new(format!("Stage: {}", self.export_stage)).color(Color32::from_rgb(180,164,96)).small());
                        }
                    }

                    ui.add_space(6.0);
                    ui.separator();
                    ui.label(RichText::new("CUSTOM PRESETS").color(Color32::from_rgb(184,136,32)).small().strong());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Name").color(Color32::from_rgb(100,80,40)).small());
                        ui.add(egui::TextEdit::singleline(&mut self.preset_name_input).desired_width(140.0));
                    });
                    ui.horizontal(|ui| {
                        if ui.small_button("Save current").clicked() {
                            self.save_current_as_custom_preset();
                        }
                        if ui.small_button("Reload list").clicked() {
                            self.custom_presets = Self::load_custom_presets();
                            self.preset_status = format!("Loaded {} custom presets", self.custom_presets.len());
                        }
                    });
                    if !self.preset_status.is_empty() {
                        ui.label(RichText::new(&self.preset_status).color(Color32::from_rgb(120,180,120)).small());
                    }
                });
            });
    }

    // ── Central panel (canvas + playback controls) ───────────────────────────
    fn central_panel(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(Color32::from_rgb(6,4,2)))
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(8.0);

                    // Canvas — scale to fill available space while keeping aspect ratio
                    if let Some(tex) = &self.texture {
                        let cw      = self.settings.canvas.w() as f32;
                        let ch_f    = self.settings.canvas.h() as f32;
                        let avail   = ui.available_size();
                        // Reserve ~56px for the playback row + status labels below
                        let scale_x = avail.x / cw;
                        let scale_y = (avail.y - 56.0) / ch_f;
                        let scale   = scale_x.min(scale_y).max(0.25);
                        let display = Vec2::new(cw * scale, ch_f * scale);
                        ui.add(
                            egui::Image::new((tex.id(), display))
                                .maintain_aspect_ratio(true)
                        );
                    }

                    ui.add_space(6.0);

                    // Playback row
                    ui.horizontal(|ui| {
                        let btn_label = if self.playing { "⏸ PAUSE" } else { "▶ PLAY" };
                        let btn_col   = if self.playing {
                            Color32::from_rgb(20,16,8)
                        } else {
                            Color32::from_rgb(200,144,48)
                        };
                        if ui.button(RichText::new(btn_label).color(btn_col).small()).clicked() {
                            self.playing = !self.playing;
                            self.dirty = true;
                        }
                        if ui.button(RichText::new("↺").color(Color32::from_rgb(64,56,40))).clicked() {
                            self.scroll = 0.0;
                            self.dirty = true;
                        }
                        ui.add_space(6.0);
                        ui.label(RichText::new(format!(
                            "HY={}  W={}px  loop={}tu",
                            self.settings.scene.horizon_y,
                            (self.settings.scene.max_hw * 2.0) as u32,
                            self.settings.anim.loop_s,
                        )).color(Color32::from_rgb(40,36,28)).small());
                    });

                    ui.add_space(2.0);
                    ui.label(RichText::new(format!(
                        "cam={:.1}  fov×{:.2}  crv={:.2}  hcv={:.2}",
                        self.settings.scene.cam_h,
                        self.settings.scene.focal_mult,
                        self.settings.scene.path_power,
                        self.settings.scene.horizon_curve,
                    )).color(Color32::from_rgb(40,36,28)).small());
                });
            });
    }

    // ── Render thread management ─────────────────────────────────────────────
    fn respawn_render_thread(&mut self) {
        let (new_tx, new_rx) = spawn_render_thread();
        self.render_tx = new_tx;
        self.result_rx = new_rx;
        self.render_pending = false;
        self.render_dispatch_at = None;
    }

    // ── Collapsible section wrapper ───────────────────────────────────────────
    /// Returns `true` if any control inside changed.
    fn section(
        ui:        &mut egui::Ui,
        title:     &str,
        open:      &mut bool,
        mut body:  impl FnMut(&mut egui::Ui, bool) -> bool,
    ) -> bool {
        let mut changed = false;
        let hdr_color = if *open {
            Color32::from_rgb(184,128,32)
        } else {
            Color32::from_rgb(58,48,32)
        };
        let hdr_bg = if *open {
            Color32::from_rgb(19,16,8)
        } else {
            Color32::from_rgb(12,10,8)
        };

        let frame = egui::Frame::none()
            .fill(Color32::from_rgb(8,6,4))
            .inner_margin(egui::Margin::same(0.0))
            .stroke(egui::Stroke::new(1.0, Color32::from_rgb(20,18,12)));

        frame.show(ui, |ui| {
            // Header button
            let hdr = egui::Frame::none()
                .fill(hdr_bg)
                .inner_margin(egui::Margin::symmetric(8.0, 5.0));
            hdr.show(ui, |ui| {
                ui.horizontal(|ui| {
                    if ui.add(egui::Button::new(
                            RichText::new(title).color(hdr_color)
                                .small().strong().monospace())
                        .frame(false)
                        .min_size(Vec2::new(ui.available_width() - 16.0, 0.0))
                    ).clicked() {
                        *open = !*open;
                    }
                    ui.label(RichText::new(if *open { "▲" } else { "▼" })
                        .color(Color32::from_rgb(36,30,20)).small());
                });
            });

            // Body
            if *open {
                let body_frame = egui::Frame::none()
                    .fill(Color32::from_rgb(8,6,4))
                    .inner_margin(egui::Margin::symmetric(8.0, 6.0));
                body_frame.show(ui, |ui| {
                    changed = body(ui, false);
                });
            }
        });
        ui.add_space(2.0);
        changed
    }

    // ── GIF export ─────────────────────────────────────────────────────────
    fn do_export(&mut self) {
        use crate::gif_export::{export_project_gifs_with_progress, ExportUpdate};

        if self.gif_project_name.trim().is_empty() {
            self.export_msg = "Export requires a project name".to_owned();
            self.exporting = false;
            self.export_result_rx = None;
            return;
        }

        let mut settings = self.settings.clone();
        settings.anim.seamless_lock = true;
        let fps = self.effective_output_fps();
        let total_frames = hardlock_total_frames(&settings.anim, fps).max(2);
        let min_smoothing = min_smoothing_for_speed(
            path_speed_tiles_per_sec(&settings.anim),
            fps,
            self.motion_quality_tpf_target,
        );
        let smoothing_samples = self.gif_smoothing_samples.max(min_smoothing);
        let export_individual_layers = self.export_individual_layers;
        let gpu_scene_enabled = self.gpu_scene_export_enabled;
        let gpu_enabled = self.gpu_export_enabled;
        let gpu_settings = crate::gpu_effects::GpuEffectSettings {
            saturation: self.gpu_saturation,
            contrast: self.gpu_contrast,
            brightness: self.gpu_brightness,
            effect_mix: self.gpu_effect_mix,
        };
        let project_name = self.gif_project_name.trim().to_owned();
        let (tx, rx) = mpsc::channel::<ExportUpdate>();
        self.export_result_rx = Some(rx);
        self.exporting = true;
        self.export_progress = 0.0;
        self.export_stage = "init".to_owned();
        self.export_last_update = Some(std::time::Instant::now());
        self.export_stall_notified = false;
        self.export_msg = "Exporting layer GIFs and composite...".to_owned();

        match std::thread::Builder::new().name("pathforge-gif-export".into()).spawn(move || {
            let result = export_project_gifs_with_progress(
                &settings,
                fps,
                total_frames,
                smoothing_samples,
                &project_name,
                export_individual_layers,
                gpu_scene_enabled,
                gpu_enabled,
                gpu_settings,
                |update| {
                    let _ = tx.send(update);
                },
            );
            if let Err(e) = result {
                let _ = tx.send(ExportUpdate {
                    stage: "export".to_owned(),
                    message: format!("Export error: {}", e),
                    current: 0,
                    total: 1,
                    done: true,
                    failed: true,
                });
            }
        }) {
            Ok(_) => {}
            Err(e) => {
                self.exporting = false;
                self.export_result_rx = None;
                self.export_progress = 0.0;
                self.export_msg = format!("Export thread error: {e}");
            }
        }
    }
}

impl PathForgeApp {
    fn effective_output_fps(&self) -> u32 {
        let base = self.preview_fps_cap.max(1);
        let boost = if self.temporal_smoothing_enabled {
            self.temporal_smoothing_boost.max(2)
        } else {
            1
        };
        base.saturating_mul(boost).min(240)
    }
}

fn path_speed_tiles_per_sec(anim: &AnimSettings) -> f32 {
    (anim.play_speed.max(0.005) * anim.loop_s.max(1) as f32).max(0.01)
}

fn loop_duration_secs(anim: &AnimSettings) -> f32 {
    let path_speed = path_speed_tiles_per_sec(anim);
    (anim.loop_s.max(1) as f32 / path_speed).max(0.001)
}

fn hardlock_total_frames(anim: &AnimSettings, fps: u32) -> u32 {
    let fps = fps.max(1);
    let raw = (loop_duration_secs(anim) * fps as f32).round() as u32;
    raw.clamp(12, 720)
}

fn min_smoothing_for_speed(path_speed_tps: f32, fps: u32, target_tiles_per_frame: f32) -> u32 {
    let fps = fps.max(1) as f32;
    let target = target_tiles_per_frame.clamp(0.03, 1.0);
    let tiles_per_frame = path_speed_tps.max(0.01) / fps;
    (tiles_per_frame / target).ceil().max(1.0).min(8.0) as u32
}

// ── Widget helpers ─────────────────────────────────────────────────────────

/// Float slider + label. Returns true if value changed.
fn knob_f32(ui: &mut egui::Ui, label: &str, val: &mut f32, min: f32, max: f32, step: f64) -> bool {
    let before = *val;
    ui.horizontal(|ui| {
        if !label.is_empty() {
            ui.label(RichText::new(label).color(Color32::from_rgb(100,90,68)).small());
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let dec = if step < 0.01 { 3 } else if step < 0.1 { 2 } else { 1 };
            let mut dv = egui::DragValue::new(val)
                .speed(step)
                .range(min..=max)
                .fixed_decimals(dec);
            if dec >= 2 { dv = dv.max_decimals(4); }
            ui.add(dv);
        });
    });
    ui.add(
        Slider::new(val, min..=max)
            .step_by(step)
            .show_value(false)
    );
    *val = val.clamp(min, max);
    ui.add_space(2.0);
    (*val - before).abs() > 1e-6
}

/// Integer slider + label. Returns true if value changed.
fn knob_u32(ui: &mut egui::Ui, label: &str, val: &mut u32, min: u32, max: u32) -> bool {
    let before = *val;
    ui.horizontal(|ui| {
        if !label.is_empty() {
            ui.label(RichText::new(label).color(Color32::from_rgb(100,90,68)).small());
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add(egui::DragValue::new(val).speed(1).range(min..=max));
        });
    });
    ui.add(
        Slider::new(val, min..=max)
            .integer()
            .show_value(false)
    );
    *val = (*val).clamp(min, max);
    ui.add_space(2.0);
    *val != before
}

/// RGB colour swatch + label. Returns true if value changed.
fn colour3(ui: &mut egui::Ui, label: &str, rgb: &mut [u8; 3]) -> bool {
    let before = *rgb;
    ui.horizontal(|ui| {
        let mut f = [rgb[0] as f32 / 255.0, rgb[1] as f32 / 255.0, rgb[2] as f32 / 255.0];
        if egui::color_picker::color_edit_button_rgb(ui, &mut f).changed() {
            rgb[0] = (f[0] * 255.0) as u8;
            rgb[1] = (f[1] * 255.0) as u8;
            rgb[2] = (f[2] * 255.0) as u8;
        }
        ui.label(RichText::new(label).color(Color32::from_rgb(64,58,44)).small());
    });
    *rgb != before
}

fn checkbox(ui: &mut egui::Ui, label: &str, val: &mut bool) -> bool {
    let before = *val;
    ui.horizontal(|ui| {
        ui.checkbox(val, RichText::new(label).color(Color32::from_rgb(100,90,68)).small());
    });
    *val != before
}

fn pick_floor(ui: &mut egui::Ui, pat: &mut FloorPattern) -> bool {
    let before = pat.clone();
    ui.horizontal(|ui| {
        ui.label(RichText::new("Pattern").color(Color32::from_rgb(100,90,68)).small());
    });
    egui::ComboBox::from_id_salt("floor_pattern")
        .selected_text(RichText::new(pat.name()).small())
        .show_ui(ui, |ui| {
            for p in FloorPattern::all() {
                ui.selectable_value(pat, p.clone(), RichText::new(p.name()).small());
            }
        });
    ui.add_space(4.0);
    *pat != before
}

fn pick_wall(ui: &mut egui::Ui, pat: &mut WallPattern) -> bool {
    let before = pat.clone();
    ui.horizontal(|ui| {
        ui.label(RichText::new("Pattern").color(Color32::from_rgb(100,90,68)).small());
    });
    egui::ComboBox::from_id_salt("wall_pattern")
        .selected_text(RichText::new(pat.name()).small())
        .show_ui(ui, |ui| {
            for p in WallPattern::all() {
                ui.selectable_value(pat, p.clone(), RichText::new(p.name()).small());
            }
        });
    ui.add_space(4.0);
    *pat != before
}

fn pick_atmo(ui: &mut egui::Ui, atmo: &mut AtmoType) -> bool {
    let before = atmo.clone();
    ui.horizontal(|ui| {
        ui.label(RichText::new("Light type").color(Color32::from_rgb(100,90,68)).small());
    });
    egui::ComboBox::from_id_salt("atmo_type")
        .selected_text(RichText::new(atmo.name()).small())
        .show_ui(ui, |ui| {
            for a in AtmoType::all() {
                ui.selectable_value(atmo, a.clone(), RichText::new(a.name()).small());
            }
        });
    ui.add_space(4.0);
    *atmo != before
}

fn pick_prop_type(ui: &mut egui::Ui, pt: &mut PropType) -> bool {
    let before = pt.clone();
    ui.horizontal(|ui| {
        ui.label(RichText::new("Prop type").color(Color32::from_rgb(100,90,68)).small());
    });
    egui::ComboBox::from_id_salt("prop_type")
        .selected_text(RichText::new(pt.name()).small())
        .show_ui(ui, |ui| {
            for p in PropType::all() {
                ui.selectable_value(pt, p.clone(), RichText::new(p.name()).small());
            }
        });
    ui.add_space(4.0);
    *pt != before
}

fn pick_lighting_preset(ui: &mut egui::Ui, preset: &mut LightingPreset) -> bool {
    let before = preset.clone();
    ui.horizontal(|ui| {
        ui.label(RichText::new("Lighting preset").color(Color32::from_rgb(100,90,68)).small());
    });
    egui::ComboBox::from_id_salt("lighting_preset")
        .selected_text(RichText::new(preset.name()).small())
        .show_ui(ui, |ui| {
            for p in LightingPreset::all() {
                ui.selectable_value(preset, p.clone(), RichText::new(p.name()).small());
            }
        });
    ui.add_space(4.0);
    *preset != before
}
