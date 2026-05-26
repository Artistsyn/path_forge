use egui::{Color32, RichText, Slider, TextureOptions, Vec2};
use std::sync::mpsc;

use crate::renderer::PathRenderer;
use crate::settings::{
    presets, AtmoLayer, AtmoType, FloorPattern, PathForgeSettings,
    PropInstance, PropType, WallPattern,
};

// ── Background render thread ───────────────────────────────────────────────
struct RenderRequest {
    settings:  PathForgeSettings,
    scroll:    f32,
    global_t:  f32,
}

fn spawn_render_thread() -> (mpsc::Sender<RenderRequest>, mpsc::Receiver<Vec<u8>>) {
    let (req_tx, req_rx) = mpsc::channel::<RenderRequest>();
    let (res_tx, res_rx) = mpsc::channel::<Vec<u8>>();
    std::thread::Builder::new()
        .name("pathforge-renderer".into())
        .spawn(move || {
            let mut renderer = PathRenderer::default();
            let mut buf: Vec<u8> = Vec::new();
            while let Ok(req) = req_rx.recv() {
                let frame_len = req.settings.canvas.w() * req.settings.canvas.h() * 4;
                if buf.len() != frame_len {
                    buf.resize(frame_len, 0);
                }
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    renderer.render(&req.settings, req.scroll, req.global_t, &mut buf);
                }));
                match result {
                    Ok(_) => {
                        if res_tx.send(buf.clone()).is_err() { break; }
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
                        if res_tx.send(buf.clone()).is_err() {
                            eprintln!("[pathforge-renderer] res_tx dead — exiting");
                            break;
                        }
                    }
                }
            }
        })
        .expect("failed to spawn render thread");
    (req_tx, res_rx)
}

// ── App state ──────────────────────────────────────────────────────────────
pub struct PathForgeApp {
    // --- live settings (currently displayed / edited)
    settings:     PathForgeSettings,
    preset_idx:   usize,
    is_custom:    bool,

    // --- animation state
    playing:      bool,
    scroll:       f32,
    global_t:     f32,
    last_instant: Option<std::time::Instant>,
    fps_counter:  FpsCounter,

    // --- renderer
    render_tx:        mpsc::Sender<RenderRequest>,
    result_rx:        mpsc::Receiver<Vec<u8>>,
    render_pending:   bool,
    render_dispatch_at: Option<std::time::Instant>,
    pixel_buf:        Vec<u8>,
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
    open_atmo_layers: Vec<bool>,
    open_prop_items:  Vec<bool>,

    // --- GIF export
    gif_fps:      u32,
    exporting:    bool,
    export_msg:   String,

    // --- render control
    dirty:        bool,   // true = pixel_buf needs re-render this frame
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
        let pixel_buf = vec![0u8; settings.canvas.w() * settings.canvas.h() * 4];
        let (render_tx, result_rx) = spawn_render_thread();
        // Kick off the first render immediately
        let _ = render_tx.send(RenderRequest {
            settings:  settings.clone(),
            scroll:    0.0,
            global_t:  0.0,
        });
        Self {
            settings,
            preset_idx:     0,
            is_custom:      false,
            playing:        false,
            scroll:         0.0,
            global_t:       0.0,
            last_instant:   None,
            fps_counter:    FpsCounter::new(),
            render_tx,
            result_rx,
            render_pending:     true,
            render_dispatch_at: Some(std::time::Instant::now()),
            pixel_buf,
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
            open_atmo_layers: vec![false],
            open_prop_items:  vec![],
            gif_fps:        24,
            exporting:      false,
            export_msg:     String::new(),
            dirty:          false,
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

        let loop_len = self.settings.anim.loop_s.max(1) as f32;
        if self.playing {
            self.scroll   = (self.scroll + dt * self.settings.anim.play_speed * loop_len) % loop_len;
            self.global_t = self.scroll / loop_len;
            self.dirty    = true; // always dirty while playing
        }

        // --- collect finished renders (drain all available results)
        let mut got_frame = false;
        loop {
            match self.result_rx.try_recv() {
                Ok(new_buf) => {
                    self.pixel_buf = new_buf;
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
            let cw = self.settings.canvas.w();
            let ch = self.settings.canvas.h();
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
            match self.render_tx.send(RenderRequest {
                settings: self.settings.clone(),
                scroll:   self.scroll,
                global_t: self.global_t,
            }) {
                Ok(()) => {
                    self.dirty = false;
                    self.render_pending = true;
                    self.render_dispatch_at = Some(std::time::Instant::now());
                }
                Err(_) => {
                    eprintln!("[pathforge] render_tx.send failed — respawning thread");
                    self.respawn_render_thread();
                }
            }
        }

        // --- UI layout
        self.top_bar(ctx);
        self.right_panel(ctx);
        self.central_panel(ctx);

        // Keep the event loop alive whenever there is live work to do
        if self.playing || self.render_pending {
            ctx.request_repaint();
        }
    }
}

// ── UI sections ────────────────────────────────────────────────────────────
impl PathForgeApp {
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
                            "Custom".to_owned()
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
                                        self.scroll     = 0.0;
                                        self.dirty      = true;
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
                            changed |= ui.add(Slider::new(&mut c.base_w, 360u32..=2160).integer()).changed();
                        });
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Height").color(Color32::from_rgb(100,80,40)).small());
                            changed |= ui.add(Slider::new(&mut c.base_h, 640u32..=3840).integer()).changed();
                        });
                        changed |= checkbox(ui, "Landscape (rotate 90°)", &mut c.landscape);
                        changed
                    });
                    if ch {
                        // Resize buffer to match new canvas dimensions
                        let new_len = self.settings.canvas.w() * self.settings.canvas.h() * 4;
                        self.pixel_buf = vec![0u8; new_len];
                        self.texture   = None;
                        self.is_custom = true;
                        self.dirty     = true;
                    }

                    // ── SCENE ────────────────────────────────────────────
                    let canvas_h_u32 = self.settings.canvas.h() as u32;
                    let canvas_w_f32 = self.settings.canvas.w() as f32;
                    let o = &mut self.open_scene;
                    let ch = Self::section(ui, "SCENE", o, |ui, mut changed| {
                        let s = &mut self.settings.scene;
                        let h_max = canvas_h_u32.saturating_sub(120).max(320);
                        let hw_max = (canvas_w_f32 * 0.48).max(220.0);
                        changed |= knob_u32(ui, "Horizon Y",       &mut s.horizon_y,  80, h_max);
                        changed |= knob_f32(ui, "Path half-width",  &mut s.max_hw,    30.0, hw_max, 1.0);
                        changed |= knob_f32(ui, "Camera height",    &mut s.cam_h,     0.5, 4.0, 0.05);
                        changed |= knob_f32(ui, "FOV multiplier",   &mut s.focal_mult,0.5, 2.0, 0.05);
                        changed |= knob_f32(ui, "Path curve power", &mut s.path_power,0.25,2.5, 0.05);
                        changed |= colour3(ui,  "Void / sky",       &mut s.void_color);
                        changed |= checkbox(ui, "Grass at edges",   &mut s.grass_enabled);
                        if s.grass_enabled {
                            changed |= colour3(ui, "Grass colour", &mut s.grass_color);
                        }
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Ambient light").color(Color32::from_rgb(100,80,40)).small());
                            changed |= ui.add(Slider::new(&mut s.ambient, 0.0f32..=1.0).step_by(0.01)).changed();
                        });
                        changed
                    });
                    if ch { self.is_custom = true; self.dirty = true; }

                    // ── PATH / FLOOR ─────────────────────────────────────
                    let o = &mut self.open_floor;
                    let ch = Self::section(ui, "PATH / FLOOR", o, |ui, mut changed| {
                        let s = &mut self.settings.floor;
                        changed |= knob_f32(ui, "Depth fade",    &mut s.depth_fade,    0.5, 10.0, 0.1);
                        changed |= knob_f32(ui, "Edge vignette", &mut s.edge_vignette, 0.0,  1.0, 0.02);
                        changed |= knob_u32(ui, "Tile noise",    &mut s.noise,         0,   30);
                        changed |= knob_f32(ui, "Texture scale", &mut s.tex_scale,     0.10, 24.0, 0.05);
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
                        changed |= knob_f32(ui, "Texture scale",      &mut s.tex_scale,   0.10, 24.0, 0.05);
                        changed |= checkbox(ui, "Rotate texture 90deg", &mut s.tex_rot_90);
                        changed |= knob_f32(ui, "Damage / cracks",    &mut s.damage,      0.0, 1.0, 0.01);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Variation seed").color(Color32::from_rgb(100,80,40)).small());
                            changed |= ui.add(egui::DragValue::new(&mut s.variation_seed).speed(1)).changed();
                        });
                        changed |= knob_f32(ui, "Wall distance",     &mut s.l_wx,        0.3, 2.5, 0.05);
                        changed |= knob_f32(ui, "Brightness",        &mut s.bright,      0.4, 5.0, 0.1);
                        changed |= knob_f32(ui, "Junction shadow",   &mut s.junc_shadow, 2.0, 60.0, 1.0);
                        changed |= knob_u32(ui, "Void-fade rows",    &mut s.fade_rows,   4,   60);
                        changed |= knob_u32(ui, "Tile noise",        &mut s.noise,       0,   25);
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
                                        changed |= knob_f32(ui, "Light height",  &mut layer.torch_h,     0.3,  5.0,  0.1);
                                        changed |= knob_u32(ui, "Light spacing", &mut layer.torch_spc,   1,    8);
                                        changed |= knob_f32(ui, "Light size",    &mut layer.torch_scale, 0.01, 0.20, 0.005);
                                        changed |= knob_f32(ui, "FX scale",      &mut layer.fx_scale,    0.2,  3.0, 0.02);
                                        changed |= knob_f32(ui, "Placement jitter", &mut layer.placement_jitter, 0.0, 1.0, 0.01);
                                        changed |= knob_f32(ui, "Flicker",       &mut layer.flicker,     0.0,  2.0, 0.02);
                                        changed |= knob_u32(ui, "Dust/particles",&mut layer.n_motes,     0,    25);
                                        changed |= knob_u32(ui, "Floor debris",  &mut layer.n_debris,    0,    10);
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
                                        changed |= knob_f32(ui, "Side offset (wx)", &mut item.wx,        0.5, 4.0, 0.1);
                                        changed |= checkbox(ui, "Mirror",           &mut item.mirror);
                                        changed |= knob_f32(ui, "Z spacing",        &mut item.z_spacing, 0.5, 6.0, 0.25);
                                        changed |= knob_f32(ui, "Scale",            &mut item.scale,     0.2, 3.0, 0.1);
                                        changed |= knob_f32(ui, "Scale variation",  &mut item.scale_var, 0.0, 0.6, 0.01);
                                        changed |= knob_f32(ui, "X jitter",         &mut item.x_jitter,  0.0, 0.5, 0.01);
                                        changed |= knob_f32(ui, "Y sink",           &mut item.y_sink,    0.0, 4.5, 0.05);
                                        changed |= knob_f32(ui, "Ground blend",     &mut item.ground_blend, 0.0, 1.0, 0.01);
                                        changed |= knob_f32(ui, "Tree style mix",   &mut item.tree_style_mix, 0.0, 1.0, 0.01);
                                        changed |= knob_f32(ui, "Tree style bias",  &mut item.tree_style_bias,-1.0, 1.0, 0.02);
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
                                changed |= ui.add(Slider::new(&mut p.vignette, 0.0f32..=1.0).step_by(0.01)).changed();
                            });
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("Saturation").color(Color32::from_rgb(100,80,40)).small());
                                changed |= ui.add(Slider::new(&mut p.saturation, 0.0f32..=2.5).step_by(0.02)).changed();
                            });
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("Bloom").color(Color32::from_rgb(100,80,40)).small());
                                changed |= ui.add(Slider::new(&mut p.bloom, 0.0f32..=1.0).step_by(0.01)).changed();
                            });
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("Film grain").color(Color32::from_rgb(100,80,40)).small());
                                changed |= ui.add(Slider::new(&mut p.grain, 0.0f32..=0.5).step_by(0.005)).changed();
                            });
                            changed |= checkbox(ui, "Depth fog", &mut p.fog_enabled);
                            if p.fog_enabled {
                                changed |= colour3(ui, "Fog colour", &mut p.fog_color);
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new("Fog density").color(Color32::from_rgb(100,80,40)).small());
                                    changed |= ui.add(Slider::new(&mut p.fog_density, 0.0f32..=1.0).step_by(0.01)).changed();
                                });
                            }
                            changed
                        });
                        if ch { self.is_custom = true; self.dirty = true; }
                    }

                    // ── ANIMATION ────────────────────────────────────────
                    let o = &mut self.open_anim;
                    Self::section(ui, "ANIMATION", o, |ui, mut changed| {
                        let s = &mut self.settings.anim;
                        changed |= knob_u32(ui, "Loop speed (tiles)", &mut s.loop_s,     1, 8);
                        changed |= knob_f32(ui, "Preview speed",      &mut s.play_speed, 0.1, 5.0, 0.1);
                        ui.label(RichText::new(
                            "Keep loop speed integer for seamless GIF export."
                        ).color(Color32::from_rgb(40,36,28)).small());
                        changed
                    });

                    ui.separator();

                    // ── GIF EXPORT ───────────────────────────────────────
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("GIF FPS").color(Color32::from_rgb(100,80,40)).small());
                        ui.add(Slider::new(&mut self.gif_fps, 6..=60).integer());
                    });
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
                        "cam={:.1}  fov×{:.2}  crv={:.2}",
                        self.settings.scene.cam_h,
                        self.settings.scene.focal_mult,
                        self.settings.scene.path_power,
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
        use crate::gif_export::{render_loop_frames, export_gif};
        use crate::renderer::PathRenderer;

        let mut local_renderer = PathRenderer::default();
        let frames = render_loop_frames(&mut local_renderer, &self.settings, self.gif_fps);
        let path   = "pathforge_export.gif";
        let delay  = 1000 / self.gif_fps;

        match export_gif(&frames, self.settings.canvas.w() as u16, self.settings.canvas.h() as u16, delay, path) {
            Ok(_)  => self.export_msg = format!("Saved {} frames → {}", frames.len(), path),
            Err(e) => self.export_msg = format!("Export error: {e}"),
        }
    }
}

// ── Widget helpers ─────────────────────────────────────────────────────────

/// Float slider + label. Returns true if value changed.
fn knob_f32(ui: &mut egui::Ui, label: &str, val: &mut f32, min: f32, max: f32, step: f64) -> bool {
    let before = *val;
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).color(Color32::from_rgb(100,90,68)).small());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(format!("{val:.2}")).color(Color32::from_rgb(200,152,42)).small().monospace());
        });
    });
    ui.add(
        Slider::new(val, min..=max)
            .step_by(step)
            .show_value(false)
    );
    ui.add_space(2.0);
    (*val - before).abs() > 1e-6
}

/// Integer slider + label. Returns true if value changed.
fn knob_u32(ui: &mut egui::Ui, label: &str, val: &mut u32, min: u32, max: u32) -> bool {
    let before = *val;
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).color(Color32::from_rgb(100,90,68)).small());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(format!("{val}")).color(Color32::from_rgb(200,152,42)).small().monospace());
        });
    });
    ui.add(
        Slider::new(val, min..=max)
            .integer()
            .show_value(false)
    );
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
