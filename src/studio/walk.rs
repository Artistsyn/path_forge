//! The walk window: design a transition into another scene, or a fork into two, and watch it.
//! PathForge plans it from the scenes (what stands at the boundary, where it first shows, light
//! through the opening, the eye's adaptation); the window shows that plan, its notes and warnings,
//! and a live preview to scrub or play, and exports the clips.

use super::{home, worker, Studio};
use crate::scene::transition::{Branch, ForkChoice, Marker, StyleFrom, Threshold, Transition};
use crate::scene::{self, presets, Scene};
use egui::{Color32, RichText, TextureHandle, TextureOptions, Vec2};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

/// A scene to walk into: a preset, or a file.
#[derive(Clone, Default, PartialEq)]
pub struct Target { preset: usize, file: Option<PathBuf> }

impl Target {
    fn label(&self) -> String {
        match &self.file { Some(f) => f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), None => presets::ALL[self.preset].0.to_owned() }
    }
    fn load(&self) -> Result<(Scene, Option<PathBuf>), String> {
        match &self.file {
            Some(f) => Ok((scene::load_file(f)?.0, f.parent().map(PathBuf::from))),
            None => Ok((presets::ALL[self.preset].1(), None)),
        }
    }
}

pub struct WalkUi {
    pub open: bool,
    fork: bool,
    to: Target,
    left: Target,
    right: Target,
    pub transition: Transition,
    pub fork_choice: ForkChoice,
    take: Branch,
    travel: f32,
    playing: bool,
    last: Option<Instant>,
    entries: u32,
    preview: Option<worker::WalkPreview>,
    tex: Option<TextureHandle>,
    sent: String,
    tag: u64,
    // From the latest frame.
    length: f32,
    at: f32,
    kind: String,
    notes: Vec<String>,
    warnings: Vec<String>,
    error: Option<String>,
    task: Option<worker::ExportTask>,
    shot_frames: u32,
    result: Option<Result<crate::export::ExportReport, String>>,
}

impl Default for WalkUi {
    fn default() -> Self {
        let find = |n: &str| presets::ALL.iter().position(|p| p.0 == n).unwrap_or(0);
        WalkUi {
            open: false, fork: false, to: Target { preset: find("Forest Path"), file: None },
            left: Target { preset: find("Mountain Pass"), file: None }, right: Target { preset: find("Desert Canyon"), file: None },
            transition: Transition::default(), fork_choice: ForkChoice::default(), take: Branch::Left,
            travel: 0.0, playing: false, last: None, entries: 1, preview: None, tex: None, sent: String::new(), tag: 0,
            length: 1.0, at: 0.0, kind: String::new(), notes: Vec::new(), warnings: Vec::new(), error: None, task: None, shot_frames: 0, result: None,
        }
    }
}

fn pick_target(ui: &mut egui::Ui, id: &str, t: &mut Target) {
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt(id).selected_text(t.label()).width(170.0).show_ui(ui, |ui| {
            for (i, (n, _)) in presets::ALL.iter().enumerate() {
                if ui.selectable_label(t.file.is_none() && t.preset == i, *n).clicked() { t.preset = i; t.file = None; }
            }
        });
        if ui.button("File…").clicked() {
            if let Some(f) = rfd::FileDialog::new().add_filter("PathForge scene", &["json"]).set_directory(home()).pick_file() { t.file = Some(f); }
        }
    });
}

/// A slider where 0 means "PathForge decides".
fn auto_slider(ui: &mut egui::Ui, v: &mut f32, range: std::ops::RangeInclusive<f32>, text: &str) {
    ui.horizontal(|ui| {
        ui.add(egui::Slider::new(v, range).text(text));
        if *v == 0.0 { ui.label(RichText::new("auto").weak()); } else if ui.small_button("auto").clicked() { *v = 0.0; }
    });
}

impl Studio {
    /// Test tooling: `PF_STUDIO_WALK=1` opens this window at start (`fork` for a fork) and
    /// `PF_STUDIO_SHOT=<file.png>` saves the studio window a few seconds in, then quits.
    pub(super) fn dev_shot(&mut self, ctx: &egui::Context) {
        let w = &mut self.walk;
        if w.shot_frames == 0 {
            if let Ok(v) = std::env::var("PF_STUDIO_WALK") { w.open = true; w.fork = v == "fork"; w.travel = std::env::var("PF_STUDIO_TRAVEL").ok().and_then(|t| t.parse().ok()).unwrap_or(0.0); }
        }
        let Ok(path) = std::env::var("PF_STUDIO_SHOT") else { return };
        w.shot_frames += 1;
        ctx.request_repaint();
        if w.shot_frames == 240 { ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot); }
        let shot = ctx.input(|i| i.events.iter().find_map(|e| match e { egui::Event::Screenshot { image, .. } => Some(image.clone()), _ => None }));
        if let Some(img) = shot {
            let [w, h] = img.size;
            let rgba: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
            let _ = image::save_buffer(&path, &rgba, w as u32, h as u32, image::ExtendedColorType::Rgba8);
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    pub(super) fn walk_window(&mut self, ctx: &egui::Context) {
        if !self.walk.open { return; }
        let preview = self.walk.preview.get_or_insert_with(|| worker::WalkPreview::spawn(ctx.clone()));
        if let Some(f) = preview.poll() {
            let w = &mut self.walk;
            let ci = egui::ColorImage::from_rgba_unmultiplied([f.image.width, f.image.height], &f.image.rgba);
            match &mut w.tex { Some(t) => t.set(ci, TextureOptions::LINEAR), None => w.tex = Some(ctx.load_texture("walk", ci, TextureOptions::LINEAR)) }
            w.length = f.length;
            w.travel = w.travel.min(w.length);
            w.at = f.at;
            w.kind = f.kind;
            w.notes = f.notes;
            w.warnings = f.warnings;
        }
        // Playing: walk on at the scene's speed, and stop at the end.
        if self.walk.playing {
            let now = Instant::now();
            let dt = self.walk.last.map_or(0.0, |l| now.duration_since(l).as_secs_f32()).min(0.1);
            self.walk.last = Some(now);
            self.walk.travel += dt * self.scene.motion.speed.max(0.1);
            if self.walk.travel >= self.walk.length { self.walk.travel = self.walk.length; self.walk.playing = false; }
            ctx.request_repaint();
        } else { self.walk.last = None; }

        let mut open = true;
        let mut export = false;
        egui::Window::new("Walk on: transition or fork").open(&mut open).resizable(true).default_width(720.0).show(ctx, |ui| {
            let w = &mut self.walk;
            ui.horizontal(|ui| {
                ui.selectable_value(&mut w.fork, false, "Transition into a scene");
                ui.selectable_value(&mut w.fork, true, "Fork: the player chooses");
            });
            ui.separator();
            ui.columns(2, |cols| {
                let ui = &mut cols[0];
                if !w.fork {
                    ui.label("Into");
                    pick_target(ui, "walk_to", &mut w.to);
                    let t = &mut w.transition;
                    egui::ComboBox::from_label("Threshold").selected_text(t.threshold.name()).show_ui(ui, |ui| {
                        for th in [Threshold::Auto, Threshold::Open, Threshold::Doorway, Threshold::CaveMouth, Threshold::Gate, Threshold::Portal] {
                            ui.selectable_value(&mut t.threshold, th, th.name());
                        }
                    });
                    auto_slider(ui, &mut t.approach_m, 0.0..=90.0, "first shows (m)");
                    auto_slider(ui, &mut t.blend_m, 0.0..=20.0, "blend band (m)");
                    ui.add(egui::Slider::new(&mut t.light_spill, 0.0..=2.0).text("light through the opening"));
                    ui.add(egui::Slider::new(&mut t.adaptation, 0.0..=1.0).text("eye adaptation"));
                    ui.add(egui::Slider::new(&mut t.camera_blend_m, 0.5..=12.0).text("camera change (m)"));
                    auto_slider(ui, &mut t.facade_height, 0.0..=30.0, "face height (m)");
                    egui::ComboBox::from_label("Structure").selected_text(format!("{:?}", t.marker)).show_ui(ui, |ui| {
                        for m in [Marker::Auto, Marker::None, Marker::Archway, Marker::RuinedArch, Marker::Gate, Marker::Banners, Marker::Portal] {
                            ui.selectable_value(&mut t.marker, m, format!("{m:?}"));
                        }
                    });
                    egui::ComboBox::from_label("Look").selected_text(format!("{:?}", t.style)).show_ui(ui, |ui| {
                        for st in [StyleFrom::Auto, StyleFrom::First, StyleFrom::Second] { ui.selectable_value(&mut t.style, st, format!("{st:?}")); }
                    });
                    ui.add(egui::DragValue::new(&mut w.entries).range(1..=16).prefix("clips starting across the loop: "))
                        .on_hover_text("More than 1 writes clips that start at evenly spaced loop frames, so a game can begin the walk without waiting for frame 0");
                } else {
                    ui.label("Left branch");
                    pick_target(ui, "walk_left", &mut w.left);
                    ui.label("Right branch");
                    pick_target(ui, "walk_right", &mut w.right);
                    let f = &mut w.fork_choice;
                    auto_slider(ui, &mut f.angle, 0.0..=45.0, "branch angle (°)");
                    auto_slider(ui, &mut f.approach_m, 0.0..=80.0, "junction first shows (m)");
                    auto_slider(ui, &mut f.blend_m, 0.0..=20.0, "branch blend (m)");
                    auto_slider(ui, &mut f.wedge_m, 0.0..=40.0, "ground between branches (m)");
                    ui.add(egui::Slider::new(&mut f.steer_m, 1.0..=15.0).text("turn (m)"));
                    ui.horizontal(|ui| {
                        ui.label("If no choice:");
                        ui.selectable_value(&mut f.default_branch, Branch::Left, "left");
                        ui.selectable_value(&mut f.default_branch, Branch::Right, "right");
                    });
                    ui.horizontal(|ui| {
                        ui.label("Preview takes:");
                        ui.selectable_value(&mut w.take, Branch::Left, "left");
                        ui.selectable_value(&mut w.take, Branch::Right, "right");
                    });
                }
                ui.separator();
                ui.label(RichText::new(if w.fork { "Plan".to_string() } else { format!("Plan: {}", w.kind) }).strong());
                for n in &w.notes { ui.label(format!("• {n}")); }
                for n in &w.warnings { ui.colored_label(Color32::from_rgb(230, 180, 60), format!("⚠ {n}")); }
                if let Some(e) = &w.error { ui.colored_label(Color32::from_rgb(230, 90, 80), e); }

                let ui = &mut cols[1];
                if let Some(t) = &w.tex {
                    let size = t.size_vec2();
                    let k = (ui.available_width() / size.x).min(520.0 / size.y);
                    ui.image((t.id(), Vec2::new(size.x * k, size.y * k)));
                }
                ui.horizontal(|ui| {
                    if ui.button(if w.playing { "⏸" } else { "▶" }).clicked() {
                        if !w.playing && w.travel >= w.length - 0.01 { w.travel = 0.0; }
                        w.playing = !w.playing;
                    }
                    ui.add(egui::Slider::new(&mut w.travel, 0.0..=w.length.max(1.0)).clamp_to_range(false).text("m").fixed_decimals(1));
                    w.travel = w.travel.max(0.0);
                });
                ui.label(RichText::new(format!("{} at {:.0} m of {:.0} m walked", if w.fork { "junction" } else { "boundary" }, w.at, w.length)).weak());
                ui.separator();
                match &w.task {
                    Some(t) => {
                        ui.horizontal(|ui| {
                            if let Some(p) = &t.last { ui.add(egui::ProgressBar::new(p.done as f32 / p.total.max(1) as f32).desired_width(200.0)); }
                            if ui.button("Cancel").clicked() { t.cancel(); }
                        });
                    }
                    None => {
                        if ui.button(RichText::new("Export clips").strong()).on_hover_text("Uses the formats and folder of the Export window").clicked() { export = true; }
                    }
                }
                match &w.result {
                    Some(Ok(r)) => { for f in &r.files { ui.label(RichText::new(f.display().to_string()).small()); } }
                    Some(Err(e)) => { ui.colored_label(Color32::from_rgb(230, 90, 80), e); }
                    None => {}
                }
            });
        });
        self.walk.open = open;

        // Ask for the frame the settings describe, when they change.
        let a_dir = self.base_dir();
        let w = &mut self.walk;
        let spec = if w.fork {
            match (w.left.load(), w.right.load()) {
                (Ok((l, ld)), Ok((r, rd))) => Ok(worker::WalkSpec::Fork { l: Arc::new(l), l_dir: ld, r: Arc::new(r), r_dir: rd, f: w.fork_choice.clone(), take: w.take }),
                (Err(e), _) | (_, Err(e)) => Err(e),
            }
        } else {
            w.to.load().map(|(b, bd)| worker::WalkSpec::Cross { b: Arc::new(b), b_dir: bd, t: w.transition.clone() })
        };
        let key = format!("{}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:.3}|{}", w.fork, w.to.file, (w.to.preset, w.left.preset, w.right.preset), (&w.left.file, &w.right.file),
            serde_json::to_string(&w.transition).unwrap_or_default(), serde_json::to_string(&w.fork_choice).unwrap_or_default(), w.take, w.travel,
            serde_json::to_string(&*self.scene).map(|s| s.len()).unwrap_or(0));
        match spec {
            Err(e) => w.error = Some(e),
            Ok(spec) => {
                w.error = None;
                if key != w.sent {
                    w.sent = key;
                    w.tag += 1;
                    let size = ((self.scene.canvas.width / 2).max(32), (self.scene.canvas.height / 2).max(32));
                    if let Some(p) = &w.preview {
                        p.request(worker::WalkRequest { a: self.scene.clone(), a_dir: a_dir.clone(), spec: spec.clone(), travel: w.travel, size, tag: w.tag });
                    }
                }
                if export {
                    let job = self.export_job();
                    let w = &mut self.walk;
                    w.result = None;
                    let kind = match spec {
                        worker::WalkSpec::Cross { b, b_dir, t } => worker::ClipKind::Transition(Box::new((*b).clone()), b_dir, Box::new(t), w.entries),
                        worker::WalkSpec::Fork { l, l_dir, r, r_dir, f, .. } => worker::ClipKind::Fork(Box::new(((*l).clone(), l_dir)), Box::new(((*r).clone(), r_dir)), Box::new(f)),
                    };
                    w.task = Some(worker::ExportTask::spawn(ctx.clone(), (*self.scene).clone(), job, kind));
                }
            }
        }
        // Export progress.
        let w = &mut self.walk;
        if let Some(t) = &mut w.task {
            let mut done = None;
            while let Ok(ev) = t.rx.try_recv() {
                match ev {
                    worker::ExportEvent::Progress(p) => t.last = Some(p),
                    worker::ExportEvent::Done(r) => done = Some(r),
                }
            }
            if let Some(r) = done { w.result = Some(r); w.task = None; }
        }
    }
}
