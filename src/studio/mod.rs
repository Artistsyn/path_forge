//! PathForge Studio: the desktop editor for v3 scenes.
//!
//! Layout: outliner (scene sections and list items) on the left, the looping preview in the
//! middle with a timeline under it, the inspector for the selection on the right. The preview
//! renders on a background thread, edits never reset playback, a whole drag is one undo step,
//! and the open file is watched so edits made elsewhere (the MCP server, a text editor) appear live.

pub mod inspector;
pub mod worker;
pub mod history;
mod walk;

use crate::export::{ExportJob, ExportReport, Format};
use crate::review::{self, LoopReport};
use crate::scene::{self, presets, Scene};
use crate::world::View;
use egui::{Color32, Key, KeyboardShortcut, Modifiers, Pos2, Rect, RichText, Sense, Stroke, TextureHandle, TextureOptions, Vec2};
use inspector::{Options, Schema};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

const SECTIONS: &[(&str, &str)] = &[
    ("camera", "Camera"), ("path", "Path"), ("verge", "Verge"), ("walls", "Walls"), ("ceiling", "Ceiling"),
    ("sky", "Sky"), ("light", "Lighting"), ("weather", "Weather"), ("post", "Post"), ("style", "Style"), ("motion", "Motion"), ("canvas", "Canvas"),
];
const LISTS: &[(&str, &str, &str)] = &[("fixtures", "Lights", "Fixture"), ("props", "Props", "PropLayer"), ("set_pieces", "Set pieces", "SetPiece"), ("particles", "Particles", "Particles")];
const HISTORY: usize = 200;
const THUMB_FRAMES: usize = 8;

/// The remix window: which parts to keep, how much to nudge, and the scene it remixes from (taken
/// when the window opens, so remixing again does not compound).
struct RemixUi { open: bool, keep: [bool; 8], seed: u32, jitter: f32, base: Option<Scene> }
impl Default for RemixUi {
    fn default() -> Self { RemixUi { open: false, keep: [true, true, false, false, false, false, false, true], seed: 1, jitter: 0.12, base: None } }
}

#[derive(Clone, PartialEq)]
enum Selection { Section(&'static str), Item(&'static str, usize) }

#[derive(Clone, Copy, PartialEq)]
enum Drag { Horizon, Width, Sun, Moon }

struct ExportDialog {
    open: bool,
    formats: Vec<(Format, &'static str, bool)>,
    out_dir: String,
    name: String,
    auto_frames: bool,
    frames: u32,
    auto_quality: bool,
    webp_quality: f32,
    gif_lossy: f32,
    gif_dither: bool,
    scale: f32,
    clip: u8, // 0 loop, 1 encounter, 2 transition
    slow_m: f32,
    to_preset: usize,
    to_file: Option<PathBuf>,
    task: Option<worker::ExportTask>,
    result: Option<Result<ExportReport, String>>,
    /// Size estimate: the settings it is for, the settings waiting to settle (and since when), the
    /// running estimate, and the result with its preview of the final colours.
    est_key: String,
    est_wait: Option<(String, Instant)>,
    est_rx: Option<Receiver<Result<crate::export::Estimate, String>>>,
    est: Option<Result<crate::export::Estimate, String>>,
    est_tex: Option<TextureHandle>,
}

impl Default for ExportDialog {
    fn default() -> Self {
        ExportDialog {
            open: false,
            formats: vec![(Format::Webp, "WebP (animated)", true), (Format::Gif, "GIF", false), (Format::Apng, "APNG", false), (Format::Sheet, "Sprite sheet + atlas", false), (Format::Png, "PNG frames", false),
                (Format::Depth, "Depth maps (16-bit)", false), (Format::Layers, "Depth layers: near / mid / far", false)],
            out_dir: String::new(), name: String::new(), auto_frames: true, frames: 144,
            auto_quality: true, webp_quality: 90.0, gif_lossy: 0.33, gif_dither: true, scale: 1.0, clip: 0, slow_m: 3.0, to_preset: 0, to_file: None,
            task: None, result: None, est_key: String::new(), est_wait: None, est_rx: None, est: None, est_tex: None,
        }
    }
}

pub struct Studio {
    schema: Schema,
    doc: Value,
    scene: Arc<Scene>,
    scene_error: Option<String>,
    file: Option<PathBuf>,
    saved: Value,
    disk_stamp: Option<(SystemTime, u64)>,
    disk_changed: bool,
    last_watch: Instant,
    history: history::History,
    selection: Selection,
    advanced: bool,
    guides: bool,
    // Playback: position in metres along the loop; frames snap to the export frame grid.
    playing: bool,
    pos: f32,
    last_tick: Instant,
    preview: worker::Preview,
    texture: Option<TextureHandle>,
    shown_tag: u64,
    tag: u64,
    pending: bool,
    sent: Option<(u64, f32)>,
    render_ms: f32,
    stats: Option<crate::world::render::FrameStats>,
    drag: Option<Drag>,
    /// What each pixel of the shown frame is (click to select), and the outline drawn around the
    /// selection, with the frame number and selection it was drawn for.
    pick: Option<crate::world::render::Pick>,
    frame_no: u64,
    outline: Option<(TextureHandle, (u64, Selection))>,
    gallery_open: bool,
    /// Gallery thumbnails: each preset's frames across its loop, played while the gallery is open.
    thumbs: Vec<Vec<TextureHandle>>,
    thumbs_rx: Option<Receiver<(usize, usize, crate::world::Image)>>,
    remix: RemixUi,
    walk: walk::WalkUi,
    loop_rx: Option<Receiver<LoopReport>>,
    loop_report: Option<LoopReport>,
    export: ExportDialog,
    status: String,
    title: String,
}

fn to_doc(scene: &Scene) -> Value {
    serde_json::from_str(&serde_json::to_string(scene).unwrap_or_default()).unwrap_or(Value::Null)
}

fn stamp(p: &Path) -> Option<(SystemTime, u64)> {
    let m = std::fs::metadata(p).ok()?;
    Some((m.modified().ok()?, m.len()))
}

pub fn home() -> PathBuf { crate::mcp::home() }

/// A folder of the home folder (scenes, exports), made on first use so dialogs can open in it.
fn home_sub(name: &str) -> PathBuf {
    let d = home().join(name);
    let _ = std::fs::create_dir_all(&d);
    d
}

impl Studio {
    pub fn new(cc: &eframe::CreationContext<'_>, open: Option<PathBuf>, section: Option<String>) -> Studio {
        style(&cc.egui_ctx);
        let scene = presets::ALL.iter().find(|(n, _)| *n == "Stone Dungeon").map(|(_, f)| f()).unwrap_or_default();
        let doc = to_doc(&scene);
        let mut s = Studio {
            schema: Schema::new(), saved: doc.clone(), history: history::History::new(&doc, HISTORY), doc, scene: Arc::new(scene), scene_error: None,
            file: None, disk_stamp: None, disk_changed: false, last_watch: Instant::now(),
            selection: Selection::Section("camera"), advanced: false, guides: false,
            playing: true, pos: 0.0, last_tick: Instant::now(),
            preview: worker::Preview::spawn(cc.egui_ctx.clone()), texture: None, shown_tag: 0, tag: 1, pending: false, sent: None,
            render_ms: 0.0, stats: None, drag: None, pick: None, frame_no: 0, outline: None,
            gallery_open: open.is_none(), thumbs: vec![Vec::new(); presets::ALL.len()], thumbs_rx: None, remix: RemixUi::default(), walk: walk::WalkUi::default(),
            loop_rx: None, loop_report: None, export: ExportDialog::default(), status: String::new(), title: String::new(),
        };
        if let Some(p) = open { s.open_file(&p); }
        // `--section camera` or `--section fixtures/0` opens on that part of the scene.
        if let Some(sec) = section {
            let mut it = sec.splitn(2, '/');
            let key = it.next().unwrap_or("");
            if let Some((k, _)) = SECTIONS.iter().find(|(k, _)| *k == key) { s.selection = Selection::Section(k); }
            if let Some((k, ..)) = LISTS.iter().find(|(k, ..)| *k == key) {
                s.selection = Selection::Item(k, it.next().and_then(|i| i.parse().ok()).unwrap_or(0));
            }
        }
        s
    }

    // ── Document ───────────────────────────────────────────────────────────

    fn dirty(&self) -> bool { self.doc != self.saved }

    /// Rebuild the typed scene from the JSON being edited.
    fn sync_scene(&mut self) {
        match serde_json::from_value::<Scene>(self.doc.clone()) {
            Ok(s) => { self.scene = Arc::new(s); self.scene_error = None; self.tag += 1; self.loop_report = None; }
            Err(e) => self.scene_error = Some(e.to_string()),
        }
    }

    fn replace_doc(&mut self, doc: Value, keep_undo: bool) {
        let old = std::mem::replace(&mut self.doc, doc);
        self.history.replaced(old, &self.doc, keep_undo);
        self.sync_scene();
    }

    /// Edits become one undo step once the pointer is up and no text field has focus, so a whole
    /// drag (or a typed number) is a single step.
    fn commit_edits(&mut self, ctx: &egui::Context) {
        let busy = ctx.input(|i| i.pointer.any_down()) || ctx.memory(|m| m.focused().is_some());
        self.history.commit(&self.doc, busy);
    }

    fn undo(&mut self) { if self.history.undo(&mut self.doc) { self.sync_scene(); } }

    fn redo(&mut self) { if self.history.redo(&mut self.doc) { self.sync_scene(); } }

    fn base_dir(&self) -> Option<PathBuf> { self.file.as_ref().and_then(|f| f.parent().map(Path::to_path_buf)) }

    fn new_from_preset(&mut self, i: usize) {
        let (name, make) = presets::ALL[i];
        let s = make();
        self.replace_doc(to_doc(&s), true);
        self.saved = Value::Null;
        self.file = None;
        self.disk_stamp = None;
        self.status = format!("New scene from {name} (not saved yet)");
    }

    fn open_file(&mut self, p: &Path) {
        match scene::load_file(p) {
            Ok((s, _)) => {
                let doc = to_doc(&s);
                self.replace_doc(doc.clone(), true);
                self.saved = doc;
                self.file = Some(p.to_path_buf());
                self.disk_stamp = stamp(p);
                self.disk_changed = false;
                self.status = format!("Opened {}", p.display());
            }
            Err(e) => self.status = format!("Could not open: {e}"),
        }
    }

    fn save(&mut self) {
        let Some(p) = self.file.clone() else { return self.save_as(); };
        if self.scene_error.is_some() { self.status = "Not saved: the scene has an error".into(); return; }
        match scene::save_file(&p, &self.scene) {
            Ok(_) => { self.saved = self.doc.clone(); self.disk_stamp = stamp(&p); self.disk_changed = false; self.status = format!("Saved {}", p.display()); }
            Err(e) => self.status = format!("Save failed: {e}"),
        }
    }

    fn save_as(&mut self) {
        let dir = self.base_dir().unwrap_or_else(|| home_sub("scenes"));
        let name = format!("{}.json", self.scene.name.to_lowercase().replace(' ', "_"));
        if let Some(p) = rfd::FileDialog::new().set_directory(&dir).set_file_name(&name).add_filter("PathForge scene", &["json"]).save_file() {
            self.file = Some(p);
            self.save();
        }
    }

    /// Pick up edits made to the open file by someone else (the MCP server, a text editor).
    fn watch_file(&mut self) {
        if self.last_watch.elapsed() < Duration::from_millis(400) { return; }
        self.last_watch = Instant::now();
        let Some(p) = self.file.clone() else { return };
        let now = stamp(&p);
        if now.is_none() || now == self.disk_stamp { return; }
        self.disk_stamp = now;
        if self.dirty() { self.disk_changed = true; return; }
        self.reload_from_disk(&p);
    }

    fn reload_from_disk(&mut self, p: &Path) {
        match scene::load_file(p) {
            Ok((s, _)) => {
                let doc = to_doc(&s);
                if doc != self.doc {
                    self.replace_doc(doc.clone(), true);
                    self.status = "Reloaded: the file changed on disk (Undo brings back the previous version)".into();
                }
                self.saved = doc;
                self.disk_changed = false;
            }
            Err(e) => self.status = format!("The file changed on disk but could not be read: {e}"),
        }
    }

    // ── Playback ───────────────────────────────────────────────────────────

    fn frames(&self) -> u32 { self.scene.motion.frames() }
    fn loop_len(&self) -> f32 { self.scene.motion.loop_length.max(1.0) }
    fn frame_index(&self) -> u32 { ((self.pos / self.loop_len() * self.frames() as f32).floor() as u32).min(self.frames() - 1) }
    fn frame_distance(&self, k: u32) -> f32 { self.loop_len() * k as f32 / self.frames() as f32 }

    fn tick(&mut self) {
        let dt = self.last_tick.elapsed().as_secs_f32().min(0.1);
        self.last_tick = Instant::now();
        if self.playing && self.drag.is_none() {
            self.pos = (self.pos + dt * self.scene.motion.speed).rem_euclid(self.loop_len());
        }
    }

    fn step(&mut self, by: i32) {
        self.playing = false;
        let n = self.frames() as i32;
        let k = (self.frame_index() as i32 + by).rem_euclid(n);
        self.pos = self.frame_distance(k as u32);
    }

    fn request_frame(&mut self) {
        let d = self.frame_distance(self.frame_index());
        let want = (self.tag, d);
        if self.sent == Some(want) || self.pending { return; }
        self.preview.request(worker::Request { scene: self.scene.clone(), base_dir: self.base_dir(), distance: d, tag: self.tag });
        self.sent = Some(want);
        self.pending = true;
    }

    fn receive_frame(&mut self, ctx: &egui::Context) {
        if let Some(f) = self.preview.poll() {
            self.pending = false;
            self.render_ms = f.ms;
            self.shown_tag = f.tag;
            self.stats = f.image.stats.clone();
            self.pick = f.image.pick.clone();
            self.frame_no += 1;
            let img = egui::ColorImage::from_rgba_unmultiplied([f.image.width, f.image.height], &f.image.rgba);
            let opts = if self.scene.style.pixel_size > 1 { TextureOptions::NEAREST } else { TextureOptions::LINEAR };
            match &mut self.texture {
                Some(t) => t.set(img, opts),
                None => self.texture = Some(ctx.load_texture("preview", img, opts)),
            }
            let _ = f.distance;
        }
    }

    // ── Panels ─────────────────────────────────────────────────────────────

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::menu::bar(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("New from preset…").clicked() { self.gallery_open = true; ui.close_menu(); }
                if ui.button("Open…  ⌘O").clicked() { ui.close_menu(); self.open_dialog(); }
                ui.separator();
                if ui.add_enabled(true, egui::Button::new("Save  ⌘S")).clicked() { ui.close_menu(); self.save(); }
                if ui.button("Save as…  ⇧⌘S").clicked() { ui.close_menu(); self.save_as(); }
                ui.separator();
                if ui.button("Export…  ⌘E").clicked() { ui.close_menu(); self.open_export(); }
                if let Some(p) = self.file.clone() {
                    if ui.button("Show in Finder").clicked() { ui.close_menu(); reveal(&p); }
                    ui.separator();
                    if ui.button("Gather files into the scene folder").on_hover_text("Copy every image, kit and grade the scene uses from elsewhere into its folder, and name them relative to the scene").clicked() { ui.close_menu(); self.pack(None); }
                    if ui.button("Pack into folder…").on_hover_text("Write the scene and every file it uses into one folder that can be moved or shared").clicked() {
                        ui.close_menu();
                        if let Some(d) = rfd::FileDialog::new().set_directory(home()).pick_folder() { self.pack(Some(d)); }
                    }
                }
            });
            ui.menu_button("Edit", |ui| {
                if ui.add_enabled(self.history.can_undo(&self.doc), egui::Button::new("Undo  ⌘Z")).clicked() { ui.close_menu(); self.undo(); }
                if ui.add_enabled(self.history.can_redo(), egui::Button::new("Redo  ⇧⌘Z")).clicked() { ui.close_menu(); self.redo(); }
                ui.separator();
                if ui.button("Remix…").on_hover_text("New variations: re-roll the parts you do not lock").clicked() { ui.close_menu(); self.remix.open = true; self.remix.base = Some((*self.scene).clone()); }
                if ui.button("Transition / fork…").on_hover_text("Walk on into another scene, or to a fork where the player chooses").clicked() { ui.close_menu(); self.walk.open = true; }
            });
            ui.menu_button("View", |ui| {
                ui.checkbox(&mut self.guides, "Camera guides (G)");
                ui.checkbox(&mut self.advanced, "Advanced fields");
            });
            ui.separator();
            let name = self.file.as_ref().and_then(|f| f.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Untitled".into());
            ui.label(RichText::new(format!("{}{}", name, if self.dirty() { " •" } else { "" })).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(RichText::new("Export…").strong()).clicked() { self.open_export(); }
                ui.label(RichText::new(format!("{:.0} ms/frame", self.render_ms)).weak())
                    .on_hover_text("Time to render one preview frame at full canvas size");
            });
        });
    }

    /// Pack the saved scene into `out` (None: gather into its own folder and reload it).
    fn pack(&mut self, out: Option<PathBuf>) {
        let Some(file) = self.file.clone() else { return };
        if self.doc != self.saved { self.save(); }
        let base = self.base_dir().unwrap_or_else(home);
        let name = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "scene".into());
        let in_place = out.is_none();
        match crate::project::pack(&self.scene, &base, out.as_deref().unwrap_or(&base), &name) {
            Ok(rep) => {
                let missing = if rep.missing.is_empty() { String::new() } else { format!("; missing: {}", rep.missing.join(", ")) };
                if in_place { self.open_file(&file); } else { reveal(&rep.scene); }
                self.status = format!("{} {} file(s), {} path(s) made relative{missing}", if in_place { "Gathered" } else { "Packed" }, rep.copied.len(), rep.rewritten.len());
            }
            Err(e) => self.status = format!("Pack failed: {e}"),
        }
    }

    fn open_dialog(&mut self) {
        let dir = self.base_dir().unwrap_or_else(|| home_sub("scenes"));
        if let Some(p) = rfd::FileDialog::new().set_directory(dir).add_filter("PathForge scene", &["json"]).pick_file() { self.open_file(&p); }
    }

    fn open_export(&mut self) {
        if self.export.out_dir.is_empty() {
            self.export.out_dir = self.base_dir().map(|d| d.join("exports")).unwrap_or_else(|| home_sub("exports")).display().to_string();
        }
        self.export.frames = self.frames();
        self.export.open = true;
    }

    fn outliner(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().id_salt("outliner").show(ui, |ui| {
            ui.add_space(4.0);
            let mut name = self.doc.get("name").and_then(|n| n.as_str()).unwrap_or("").to_owned();
            if ui.add(egui::TextEdit::singleline(&mut name).hint_text("Scene name").desired_width(f32::INFINITY)).changed() {
                self.doc["name"] = Value::String(name);
            }
            ui.add_space(6.0);
            ui.label(RichText::new("WORLD").small().weak());
            for (key, label) in SECTIONS {
                let enabled = self.doc.get(*key).and_then(|v| v.get("enabled")).and_then(|e| e.as_bool());
                let sel = self.selection == Selection::Section(key);
                ui.horizontal(|ui| {
                    if let Some(mut on) = enabled {
                        if ui.checkbox(&mut on, "").on_hover_text("Show or hide").changed() { self.doc[*key]["enabled"] = Value::Bool(on); }
                    } else { ui.add_space(24.0); }
                    let text = if enabled == Some(false) { RichText::new(*label).weak() } else { RichText::new(*label) };
                    if ui.selectable_label(sel, text).clicked() { self.selection = Selection::Section(key); }
                });
            }
            for (key, label, def) in LISTS {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(label.to_uppercase()).small().weak());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("+").on_hover_text(format!("Add {}", label.trim_end_matches('s').to_lowercase())).clicked() {
                            let item = crate::mcp::default_of(def);
                            if let Some(list) = self.doc[*key].as_array_mut() {
                                list.push(item);
                                self.selection = Selection::Item(key, list.len() - 1);
                            }
                        }
                    });
                });
                let len = self.doc[*key].as_array().map(|a| a.len()).unwrap_or(0);
                let mut action: Option<(&str, usize)> = None;
                for i in 0..len {
                    let item = &self.doc[*key][i];
                    let summary = inspector::item_summary(key, item);
                    let mut on = item.get("enabled").and_then(|e| e.as_bool()).unwrap_or(true);
                    let sel = self.selection == Selection::Item(key, i);
                    ui.horizontal(|ui| {
                        if ui.checkbox(&mut on, "").changed() { self.doc[*key][i]["enabled"] = Value::Bool(on); }
                        let text = if on { RichText::new(&summary) } else { RichText::new(&summary).weak() };
                        let r = ui.selectable_label(sel, text);
                        if r.clicked() { self.selection = Selection::Item(key, i); }
                        r.context_menu(|ui| {
                            if ui.button("Duplicate").clicked() { action = Some(("dup", i)); ui.close_menu(); }
                            if ui.add_enabled(i > 0, egui::Button::new("Move up")).clicked() { action = Some(("up", i)); ui.close_menu(); }
                            if ui.add_enabled(i + 1 < len, egui::Button::new("Move down")).clicked() { action = Some(("down", i)); ui.close_menu(); }
                            if ui.button("Delete").clicked() { action = Some(("del", i)); ui.close_menu(); }
                        });
                    });
                }
                if let (Some((what, i)), Some(list)) = (action, self.doc[*key].as_array_mut()) {
                    match what {
                        "dup" => { let v = list[i].clone(); list.insert(i + 1, v); self.selection = Selection::Item(key, i + 1); }
                        "up" => { list.swap(i, i - 1); self.selection = Selection::Item(key, i - 1); }
                        "down" => { list.swap(i, i + 1); self.selection = Selection::Item(key, i + 1); }
                        _ => {
                            list.remove(i);
                            self.selection = if list.is_empty() { Selection::Section("camera") } else { Selection::Item(key, i.min(list.len() - 1)) };
                        }
                    }
                }
                if len == 0 { ui.label(RichText::new("  none").weak().italics()); }
            }
        });
    }

    fn inspector(&mut self, ui: &mut egui::Ui) {
        let opts = Options { advanced: self.advanced, base: self.base_dir() };
        egui::ScrollArea::vertical().id_salt("inspector").show(ui, |ui| {
            match self.selection.clone() {
                Selection::Section(key) => {
                    let title = SECTIONS.iter().find(|(k, _)| *k == key).map(|(_, l)| *l).unwrap_or(key);
                    ui.heading(title);
                    if key == "style" {
                        ui.horizontal(|ui| {
                            ui.label("Apply look");
                            let mut chosen = None;
                            egui::ComboBox::from_id_salt("style-preset").selected_text("Choose…").width(180.0).show_ui(ui, |ui| {
                                for st in scene::styles::ALL {
                                    if ui.selectable_label(false, st.name).on_hover_text(st.about).clicked() { chosen = Some(st); }
                                }
                            });
                            if let Some(st) = chosen {
                                if let Ok(mut sc) = serde_json::from_value::<Scene>(self.doc.clone()) {
                                    (st.apply)(&mut sc);
                                    self.doc = to_doc(&sc);
                                    self.status = format!("Applied the {} look (Undo to go back)", st.name);
                                }
                            }
                        });
                        ui.label(RichText::new("A look sets style, post and light bands; the world stays as it is.").weak().small());
                        ui.add_space(4.0);
                    }
                    if let Some(def) = self.schema.section(key).cloned() {
                        if let Some(d) = def.get("description").and_then(|d| d.as_str()) { ui.label(RichText::new(d).weak()); }
                        ui.add_space(4.0);
                        let v = &mut self.doc[key];
                        inspector::edit_struct(ui, &self.schema, &def, key, v, &opts);
                    }
                }
                Selection::Item(key, i) => {
                    let label = LISTS.iter().find(|(k, ..)| *k == key).map(|(_, l, _)| *l).unwrap_or(key);
                    let len = self.doc[key].as_array().map(|a| a.len()).unwrap_or(0);
                    if i >= len { self.selection = Selection::Section("camera"); return; }
                    ui.heading(format!("{} {}", label.trim_end_matches('s'), i + 1));
                    if let Some(def) = self.schema.section(key).and_then(|l| self.schema.items(l)).cloned() {
                        let path = format!("{key}.{i}");
                        let v = &mut self.doc[key][i];
                        inspector::edit_struct(ui, &self.schema, &def, &path, v, &opts);
                    }
                }
            }
            ui.add_space(12.0);
            self.checks(ui);
        });
    }

    fn checks(&mut self, ui: &mut egui::Ui) {
        let mut warnings = crate::mcp::scene_warnings(&self.scene);
        if let Some(st) = &self.stats {
            warnings.extend(review::stats_warnings(&self.scene, st.mean_luma, &review::surfaces(st)));
        }
        let title = if warnings.is_empty() { "Checks: all clear".to_owned() } else { format!("Checks ({})", warnings.len()) };
        egui::CollapsingHeader::new(RichText::new(title).strong()).id_salt("checks").default_open(true).show(ui, |ui| {
            if let Some(e) = &self.scene_error { ui.colored_label(Color32::from_rgb(240, 110, 100), format!("Scene error: {e}")); }
            if warnings.is_empty() { ui.label(RichText::new("Nothing looks wrong in this frame.").weak()); }
            for w in &warnings { ui.label(format!("• {w}")); }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let busy = self.loop_rx.is_some();
                if ui.add_enabled(!busy, egui::Button::new(if busy { "Checking loop…" } else { "Check loop seam" })).clicked() {
                    self.loop_rx = Some(worker::check_loop(ui.ctx().clone(), (*self.scene).clone(), self.base_dir()));
                }
                if let Some(r) = &self.loop_report {
                    let col = if r.seamless && r.wrap_over_step <= 2.0 { Color32::from_rgb(120, 200, 130) } else { Color32::from_rgb(240, 170, 90) };
                    ui.colored_label(col, if r.seamless { "seamless" } else { "jumps" }).on_hover_text(&r.verdict);
                }
            });
            if let Some(r) = &self.loop_report { ui.label(RichText::new(&r.verdict).weak()); }
        });
    }

    fn timeline(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button(if self.playing { "⏸" } else { "▶" }).on_hover_text("Play / pause (Space)").clicked() { self.playing = !self.playing; }
            if ui.button("⏮").on_hover_text("Previous frame (←)").clicked() { self.step(-1); }
            if ui.button("⏭").on_hover_text("Next frame (→)").clicked() { self.step(1); }
            let n = self.frames();
            let mut k = self.frame_index();
            let w = (ui.available_width() - 360.0).max(120.0);
            ui.spacing_mut().slider_width = w;
            if ui.add(egui::Slider::new(&mut k, 0..=n - 1).show_value(false)).changed() {
                self.playing = false;
                self.pos = self.frame_distance(k);
            }
            let m = &self.scene.motion;
            ui.label(RichText::new(format!("frame {:>3}/{n}  ·  {:.2} s / {:.2} s  ·  {:.1} m", k + 1, self.pos / m.speed.max(0.01), m.loop_seconds(), self.pos)).monospace());
        });
    }

    fn viewport(&mut self, ui: &mut egui::Ui) {
        let avail = ui.available_rect_before_wrap();
        let (cw, ch) = (self.scene.canvas.width.max(16) as f32, self.scene.canvas.height.max(16) as f32);
        let s = (avail.width() / cw).min(avail.height() / ch);
        let s = if self.scene.style.pixel_size > 1 && s >= 1.0 { s.floor() } else { s };
        let rect = Rect::from_center_size(avail.center(), Vec2::new(cw * s, ch * s));
        let resp = ui.allocate_rect(avail, Sense::click_and_drag());
        let painter = ui.painter_at(avail);
        painter.rect_filled(avail, 0.0, Color32::from_gray(18));
        if let Some(t) = &self.texture {
            painter.image(t.id(), rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        } else {
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, "Rendering…", egui::FontId::proportional(16.0), Color32::GRAY);
        }
        if self.shown_tag != self.tag && self.texture.is_some() {
            painter.text(rect.right_top() + Vec2::new(-8.0, 8.0), egui::Align2::RIGHT_TOP, "updating…", egui::FontId::proportional(12.0), Color32::from_white_alpha(140));
        }

        // Guides and handles, in canvas pixels mapped onto the shown rect.
        let view = View::new(&self.scene, cw as usize, ch as usize);
        let to_screen = |p: [f32; 2]| Pos2::new(rect.left() + p[0] * s, rect.top() + p[1] * s);
        let horizon_y = rect.top() + view.horizon_px * s;
        let near_d = (view.focal_px * view.eye_height / (ch - view.horizon_px).max(1.0)).max(0.3) * 1.05;
        let hw = view.path_half_width(near_d);
        let edge_r = view.world_to_px(hw, 0.0, near_d).map(to_screen);
        let edge_l = view.world_to_px(-hw, 0.0, near_d).map(to_screen);
        let pointer = resp.hover_pos();
        let near_horizon = pointer.is_some_and(|p| (p.y - horizon_y).abs() < 6.0 && rect.contains(p));
        let near_edge = pointer.is_some_and(|p| [edge_l, edge_r].iter().flatten().any(|e| e.distance(p) < 12.0));
        let show = self.guides || self.drag.is_some() || near_horizon || near_edge;
        if show {
            let col = Color32::from_rgba_unmultiplied(120, 200, 255, 170);
            painter.line_segment([Pos2::new(rect.left(), horizon_y), Pos2::new(rect.right(), horizon_y)], Stroke::new(1.0_f32, col));
            painter.text(Pos2::new(rect.left() + 6.0, horizon_y - 2.0), egui::Align2::LEFT_BOTTOM, "horizon", egui::FontId::proportional(11.0), col);
            for d in [2.0f32, 5.0, 10.0, 20.0, 40.0] {
                let hw = view.path_half_width(d);
                if let (Some(a), Some(b)) = (view.world_to_px(-hw, 0.0, d), view.world_to_px(hw, 0.0, d)) {
                    let (a, b) = (to_screen(a), to_screen(b));
                    if a.y > rect.bottom() { continue; }
                    painter.line_segment([a, b], Stroke::new(1.0_f32, col.gamma_multiply(0.6)));
                    painter.text(b + Vec2::new(4.0, 0.0), egui::Align2::LEFT_CENTER, format!("{d} m"), egui::FontId::proportional(10.0), col);
                }
            }
            for e in [edge_l, edge_r].into_iter().flatten() { painter.circle_stroke(e, 6.0, Stroke::new(1.5_f32, col)); }
        }
        if near_horizon || near_edge || self.drag.is_some() { ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical); }
        if near_edge && self.drag.is_none() { ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal); }

        // The sun and moon: handles where they are drawn, in the sky above the horizon.
        let sky = &self.scene.sky;
        let body_at = |pos: [f32; 2]| to_screen([pos[0] * cw, pos[1].clamp(0.0, 1.0) * view.horizon_px]);
        let bodies: Vec<(Drag, Pos2)> = [(Drag::Sun, sky.enabled && sky.sun.enabled, sky.sun.pos), (Drag::Moon, sky.enabled && sky.moon.body.enabled, sky.moon.body.pos)]
            .into_iter().filter(|b| b.1).map(|b| (b.0, body_at(b.2))).collect();
        let near_body = pointer.and_then(|p| bodies.iter().find(|b| b.1.distance(p) < 14.0).map(|b| b.0));
        if near_body.is_some() || self.guides || matches!(self.drag, Some(Drag::Sun | Drag::Moon)) {
            let col = Color32::from_rgba_unmultiplied(255, 220, 140, 190);
            for (k, at) in &bodies {
                painter.circle_stroke(*at, 10.0, Stroke::new(1.5_f32, col));
                painter.text(*at + Vec2::new(13.0, 0.0), egui::Align2::LEFT_CENTER, if *k == Drag::Sun { "sun" } else { "moon" }, egui::FontId::proportional(11.0), col);
            }
            if near_body.is_some() { ui.ctx().set_cursor_icon(egui::CursorIcon::Grab); }
        }

        // The selected thing, outlined.
        self.draw_outline(ui.ctx(), &painter, rect);

        if resp.drag_started() {
            self.drag = if let Some(b) = near_body { Some(b) } else if near_edge { Some(Drag::Width) } else if near_horizon { Some(Drag::Horizon) } else { None };
        }
        if let (Some(d), Some(p)) = (self.drag, resp.interact_pointer_pos()) {
            match d {
                Drag::Horizon => {
                    let h = ((p.y - rect.top()) / (ch * s)).clamp(0.02, 0.98);
                    self.doc["camera"]["horizon"] = serde_json::json!((h * 1000.0).round() / 1000.0);
                }
                Drag::Width => {
                    // The world x under the pointer at the handle's distance, measured from the path centre.
                    let px = (p.x - rect.left()) / s;
                    let x = ((px - view.center_px) * near_d / view.focal_px - view.bend_x(near_d)).abs();
                    let flare_k = view.path_half_width(near_d) / view.half_width.max(1e-3);
                    let w = (x / flare_k).clamp(0.2, 10.0);
                    self.doc["path"]["half_width"] = serde_json::json!((w * 100.0).round() / 100.0);
                }
                Drag::Sun | Drag::Moon => {
                    let x = ((p.x - rect.left()) / (cw * s)).clamp(0.0, 1.0);
                    let y = ((p.y - rect.top()) / (view.horizon_px * s).max(1.0)).clamp(0.0, 1.0);
                    let pos = serde_json::json!([(x * 1000.0).round() / 1000.0, (y * 1000.0).round() / 1000.0]);
                    if d == Drag::Sun { self.doc["sky"]["sun"]["pos"] = pos; } else { self.doc["sky"]["moon"]["body"]["pos"] = pos; }
                }
            }
        }
        if resp.drag_stopped() { self.drag = None; }
        if resp.clicked() && !near_edge && !near_horizon && near_body.is_none() {
            if let Some(p) = resp.interact_pointer_pos() { self.select_at(p, rect); }
        }
        if resp.hovered() && self.drag.is_none() && !near_edge && !near_horizon && near_body.is_none() {
            resp.on_hover_text("Click anything to select it · drag the horizon, path edges, sun or moon to move them (G shows guides) · Space plays");
        }
    }

    /// Select what the frame shows under a viewport point: a prop layer, light or set piece, or
    /// the surface behind them (path, verge, walls, ceiling, sky).
    fn select_at(&mut self, p: Pos2, rect: Rect) {
        use crate::world::render::{pick_item, pick_section};
        let Some(pk) = &self.pick else { return };
        let (u, v) = ((p.x - rect.left()) / rect.width(), (p.y - rect.top()) / rect.height());
        if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) { return; }
        let id = pk.ids[(v * pk.height as f32) as usize * pk.width + (u * pk.width as f32) as usize];
        let lists = |name: &str| LISTS.iter().find(|l| l.0 == name).map(|l| l.0);
        let sel = match (pick_item(id), pick_section(id)) {
            (Some((list, i)), _) => lists(list).map(|l| Selection::Item(l, i)),
            (None, Some(sec)) => SECTIONS.iter().find(|s| s.0 == sec).map(|s| Selection::Section(s.0)),
            _ => None,
        };
        if let Some(sel) = sel {
            self.status = match &sel {
                Selection::Item(l, i) => format!("Selected {l}[{i}]"),
                Selection::Section(s) => format!("Selected {s}"),
            };
            self.selection = sel;
        }
    }

    /// Outline the pixels of the shown frame that belong to the selection.
    fn draw_outline(&mut self, ctx: &egui::Context, painter: &egui::Painter, rect: Rect) {
        use crate::world::render::{pick_item, pick_section};
        let Some(pk) = &self.pick else { return };
        // Sections without pixels of their own (camera, lighting...) have nothing to outline.
        if matches!(self.selection, Selection::Section(s) if !matches!(s, "path" | "verge" | "walls" | "ceiling")) { return; }
        let key = (self.frame_no, self.selection.clone());
        if self.outline.as_ref().map(|o| &o.1) != Some(&key) {
            let hit = |id: u16| match &self.selection {
                Selection::Item(l, i) => pick_item(id) == Some((*l, *i)),
                Selection::Section(s) => pick_section(id) == Some(*s) && matches!(*s, "path" | "verge" | "walls" | "ceiling"),
            };
            let (w, h) = (pk.width, pk.height);
            let mut px = vec![Color32::TRANSPARENT; w * h];
            let mut any = false;
            for y in 0..h {
                for x in 0..w {
                    if !hit(pk.ids[y * w + x]) { continue; }
                    let edge = x == 0 || y == 0 || x + 1 == w || y + 1 == h
                        || !hit(pk.ids[y * w + x - 1]) || !hit(pk.ids[y * w + x + 1]) || !hit(pk.ids[(y - 1) * w + x]) || !hit(pk.ids[(y + 1) * w + x]);
                    if edge { px[y * w + x] = Color32::from_rgba_unmultiplied(110, 210, 255, 230); any = true; }
                }
            }
            let img = egui::ColorImage { size: [w, h], pixels: px };
            if !any { self.outline = None; return; }
            match &mut self.outline {
                Some((t, k)) => { t.set(img, TextureOptions::NEAREST); *k = key; }
                None => self.outline = Some((ctx.load_texture("selection", img, TextureOptions::NEAREST), key)),
            }
        }
        if let Some((t, _)) = &self.outline {
            painter.image(t.id(), rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        }
    }

    fn remix_window(&mut self, ctx: &egui::Context) {
        use crate::scene::remix::{remix, GROUPS};
        if !self.remix.open { return; }
        let mut open = true;
        let mut go = None;
        egui::Window::new("Remix").open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
            ui.label(RichText::new("Parts you keep stay as they are; the rest are taken from random presets and nudged.").weak());
            egui::Grid::new("remix_keep").num_columns(4).show(ui, |ui| {
                for (i, (name, _)) in GROUPS.iter().enumerate() {
                    ui.checkbox(&mut self.remix.keep[i], format!("Keep {name}"));
                    if i % 4 == 3 { ui.end_row(); }
                }
            });
            ui.add(egui::Slider::new(&mut self.remix.jitter, 0.0..=0.4).text("nudge"));
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut self.remix.seed).prefix("seed "));
                if ui.button("⟳ Remix").on_hover_text("Next seed").clicked() { self.remix.seed = self.remix.seed.wrapping_add(1); go = Some(self.remix.seed); }
                if ui.button("Apply seed").clicked() { go = Some(self.remix.seed); }
                if ui.button("Remix from this").on_hover_text("Make the current scene the one remixed from").clicked() { self.remix.base = Some((*self.scene).clone()); }
            });
            ui.label(RichText::new("Each remix is one undo step.").weak());
        });
        if let Some(seed) = go {
            let base = self.remix.base.clone().unwrap_or_else(|| (*self.scene).clone());
            let keep: Vec<&str> = GROUPS.iter().zip(self.remix.keep).filter(|(_, k)| *k).map(|(g, _)| g.0).collect();
            match remix(&base, &keep, seed, self.remix.jitter) {
                Ok(s) => { self.replace_doc(to_doc(&s), true); self.status = format!("Remix {seed}"); }
                Err(e) => self.status = format!("Remix failed: {e}"),
            }
        }
        self.remix.open = open;
    }

    fn gallery(&mut self, ctx: &egui::Context) {
        if !self.gallery_open { return; }
        if self.thumbs_rx.is_none() && self.thumbs.iter().all(|t| t.is_empty()) {
            self.thumbs_rx = Some(worker::thumbnails(ctx.clone(), 0.25, THUMB_FRAMES));
        }
        if let Some(rx) = &self.thumbs_rx {
            while let Ok((i, k, img)) = rx.try_recv() {
                let ci = egui::ColorImage::from_rgba_unmultiplied([img.width, img.height], &img.rgba);
                self.thumbs[i].push(ctx.load_texture(format!("thumb{i}_{k}"), ci, TextureOptions::LINEAR));
            }
        }
        // Play the loops: a frame every 1/8 s, in step across the gallery.
        let tick = (ctx.input(|i| i.time) * 8.0) as usize;
        ctx.request_repaint_after(std::time::Duration::from_millis(125));
        let mut open = self.gallery_open;
        let mut pick = None;
        // Never wider or taller than the app window: the thumbnails wrap into rows instead.
        let screen = ctx.screen_rect();
        // Explicit rows sized to the app window, so the gallery is never wider than the screen.
        let cell = Vec2::new(120.0, 213.0);
        let cols = (((screen.width() - 80.0) / (cell.x + 16.0)).floor() as usize).clamp(2, 8);
        let width = cols as f32 * (cell.x + 16.0);
        egui::Window::new("Start from a preset").open(&mut open).collapsible(false).resizable(false)
            .fixed_size(Vec2::new(width, (screen.height() - 120.0).min(2.0 * (cell.y + 40.0) + 60.0)))
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO).show(ctx, |ui| {
            ui.label(RichText::new("Pick a starting point; every setting can be changed after.").weak());
            egui::ScrollArea::vertical().id_salt("gallery").auto_shrink([false, false]).show(ui, |ui| {
                egui::Grid::new("gallery-grid").spacing([12.0, 10.0]).show(ui, |ui| {
                    for (i, (name, _)) in presets::ALL.iter().enumerate() {
                        ui.vertical(|ui| {
                            let frames = &self.thumbs[i];
                            let r = match frames.get(if frames.len() == THUMB_FRAMES { tick % THUMB_FRAMES } else { 0 }) {
                                Some(t) => ui.add(egui::ImageButton::new(egui::Image::new((t.id(), cell)))),
                                None => ui.add_sized(cell, egui::Button::new("…")),
                            };
                            if r.clicked() { pick = Some(i); }
                            ui.label(*name);
                        });
                        if i % cols == cols - 1 { ui.end_row(); }
                    }
                });
            });
        });
        if let Some(i) = pick { self.new_from_preset(i); open = false; }
        self.gallery_open = open;
    }

    /// The export the dialog's settings describe.
    fn export_job(&self) -> ExportJob {
        let (e, scene) = (&self.export, &self.scene);
        ExportJob {
            formats: e.formats.iter().filter(|f| f.2).map(|f| f.0).collect(),
            out_dir: PathBuf::from(&e.out_dir),
            name: e.name.clone(),
            frames: if e.auto_frames { None } else { Some(e.frames) },
            size: if e.scale < 0.999 { Some(((scene.canvas.width as f32 * e.scale) as u32, (scene.canvas.height as f32 * e.scale) as u32)) } else { None },
            gif_dither: e.gif_dither, gif_lossy: e.gif_lossy,
            webp_quality: if e.auto_quality { None } else { Some(e.webp_quality) },
            base_dir: self.base_dir(),
            ..ExportJob::default()
        }
    }

    fn export_window(&mut self, ctx: &egui::Context) {
        if !self.export.open { return; }
        // Collect progress from a running export.
        let mut finished = None;
        if let Some(t) = &mut self.export.task {
            while let Ok(ev) = t.rx.try_recv() {
                match ev {
                    worker::ExportEvent::Progress(p) => t.last = Some(p),
                    worker::ExportEvent::Done(r) => finished = Some(r),
                }
            }
        }
        if let Some(r) = finished { self.export.task = None; self.export.result = Some(r); }

        // Re-estimate the sizes once the settings have stayed put for half a second.
        if self.export.clip == 0 && self.export.task.is_none() && self.export.formats.iter().any(|f| f.2) {
            let job = self.export_job();
            let key = format!("{}|{:?}|{:?}|{:?}|{}|{}|{:?}", self.tag, job.formats, job.frames, job.size, job.gif_dither, job.gif_lossy, job.webp_quality);
            if key != self.export.est_key {
                match &self.export.est_wait {
                    Some((k, t)) if *k == key => if t.elapsed() > Duration::from_millis(500) && self.export.est_rx.is_none() {
                        self.export.est_rx = Some(worker::estimate(ctx.clone(), self.scene.clone(), job));
                        self.export.est_key = key;
                        self.export.est_wait = None;
                    } else { ctx.request_repaint_after(Duration::from_millis(120)); },
                    _ => { self.export.est_wait = Some((key, Instant::now())); ctx.request_repaint_after(Duration::from_millis(120)); }
                }
            }
        }
        if let Some(rx) = &self.export.est_rx {
            if let Ok(r) = rx.try_recv() {
                self.export.est_rx = None;
                if let Ok(est) = &r {
                    self.export.est_tex = est.preview.as_ref().map(|p| {
                        let ci = egui::ColorImage::from_rgba_unmultiplied([p.width, p.height], &p.rgba);
                        ctx.load_texture("export-preview", ci, TextureOptions::NEAREST)
                    });
                }
                self.export.est = Some(r);
            }
        }

        let mut open = self.export.open;
        let mut start = false;
        let scene = self.scene.clone();
        let e = &mut self.export;
        egui::Window::new("Export").open(&mut open).collapsible(false).resizable(false).default_width(420.0).show(ctx, |ui| {
            let running = e.task.is_some();
            ui.add_enabled_ui(!running, |ui| {
                egui::Grid::new("export-grid").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                    ui.label("Formats");
                    ui.vertical(|ui| { for (_, label, on) in e.formats.iter_mut() { ui.checkbox(on, *label); } });
                    ui.end_row();
                    let palette = !matches!(scene.style.palette, crate::scene::Palette::Full);
                    if e.formats.iter().any(|f| f.0 == Format::Webp && f.2) {
                        ui.label("WebP quality");
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut e.auto_quality, "Auto").on_hover_text("Lossless for palette (pixel-art) scenes; 90 for full colour");
                            if e.auto_quality { ui.label(RichText::new(if palette { "lossless (palette scene)" } else { "90" }).weak()); }
                            else { ui.add(egui::Slider::new(&mut e.webp_quality, 50.0..=100.0).suffix(" (100 = lossless)")); }
                        });
                        ui.end_row();
                    }
                    if e.formats.iter().any(|f| f.0 == Format::Gif && f.2) {
                        ui.label("GIF");
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut e.gif_dither, "Dither");
                            ui.add(egui::Slider::new(&mut e.gif_lossy, 0.0..=1.0).text("lossy")).on_hover_text("0.33 is below what the eye notices on smooth gradients; higher saves more but shows contours");
                        });
                        ui.end_row();
                    }
                    ui.label("Clip");
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut e.clip, 0, "Loop");
                        ui.selectable_value(&mut e.clip, 1, "Encounter").on_hover_text("Stop for a fight: stop (ease to a halt), idle (loop while fighting; flames keep moving), go (ease back into the loop). Every junction matches exactly.");
                        ui.selectable_value(&mut e.clip, 2, "Transition").on_hover_text("Walk from this scene into another: the new world appears down the road and comes toward you. Starts on this loop's frame 0, ends on the other's.");
                    });
                    ui.end_row();
                    if e.clip == 1 {
                        ui.label("");
                        ui.add(egui::Slider::new(&mut e.slow_m, 0.5..=12.0).suffix(" m to stop"));
                        ui.end_row();
                    }
                    if e.clip == 2 {
                        ui.label("Into");
                        ui.horizontal(|ui| {
                            let label = match &e.to_file { Some(f) => f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), None => presets::ALL[e.to_preset].0.to_owned() };
                            egui::ComboBox::from_id_salt("to-preset").selected_text(label).show_ui(ui, |ui| {
                                for (i, (n, _)) in presets::ALL.iter().enumerate() {
                                    if ui.selectable_label(e.to_file.is_none() && e.to_preset == i, *n).clicked() { e.to_preset = i; e.to_file = None; }
                                }
                            });
                            if ui.small_button("File…").clicked() {
                                if let Some(f) = rfd::FileDialog::new().add_filter("PathForge scene", &["json"]).pick_file() { e.to_file = Some(f); }
                            }
                        });
                    }
                    ui.end_row();
                    ui.label("Frames");
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut e.auto_frames, "From motion");
                        if e.auto_frames { ui.label(RichText::new(format!("{} ({:.1} s at {} fps)", scene.motion.frames(), scene.motion.loop_seconds(), scene.motion.fps)).weak()); }
                        else { ui.add(egui::DragValue::new(&mut e.frames).range(2..=2000)); }
                    });
                    ui.end_row();
                    ui.label("Size");
                    ui.horizontal(|ui| {
                        for (s, label) in [(1.0, "1×"), (0.75, "¾"), (0.5, "½")] { ui.selectable_value(&mut e.scale, s, label); }
                        ui.label(RichText::new(format!("{} × {}", (scene.canvas.width as f32 * e.scale) as u32, (scene.canvas.height as f32 * e.scale) as u32)).weak());
                    });
                    ui.end_row();
                    ui.label("Folder");
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut e.out_dir).desired_width(250.0));
                        if ui.small_button("…").clicked() {
                            if let Some(d) = rfd::FileDialog::new().set_directory(&e.out_dir).pick_folder() { e.out_dir = d.display().to_string(); }
                        }
                    });
                    ui.end_row();
                    ui.label("Name");
                    ui.add(egui::TextEdit::singleline(&mut e.name).hint_text(&scene.name).desired_width(250.0));
                    ui.end_row();
                });
            });
            ui.separator();
            // What it will come to: sizes from a few frames encoded the same way, and the first frame
            // in the colours the file will have.
            if e.clip == 0 {
                ui.horizontal(|ui| {
                    if let Some(t) = &e.est_tex {
                        let h = 120.0;
                        let w = h * t.size()[0] as f32 / t.size()[1].max(1) as f32;
                        ui.image((t.id(), Vec2::new(w, h))).on_hover_text("First frame in the colours the file will have");
                    }
                    ui.vertical(|ui| {
                        let busy = e.est_rx.is_some() || e.est_wait.is_some();
                        match &e.est {
                            Some(Ok(est)) => {
                                ui.label(RichText::new(format!("About {}{}", human(est.total), if busy { "  (updating…)" } else { "" })).strong());
                                for (name, b) in &est.parts { ui.label(RichText::new(format!("{name}  ≈ {}", human(*b))).weak()); }
                                ui.label(RichText::new(format!("from {} of {} frames, encoded the same way", est.sample, est.frames)).weak().small());
                            }
                            Some(Err(err)) => { ui.colored_label(Color32::from_rgb(240, 110, 100), format!("Estimate failed: {err}")); }
                            None => { ui.label(RichText::new("Estimating size…").weak()); }
                        }
                    });
                });
                ui.separator();
            }
            if let Some(t) = &e.task {
                let (frac, text) = t.last.as_ref().map(|p| (p.done as f32 / p.total.max(1) as f32, format!("{} {}/{}", p.stage, p.done, p.total))).unwrap_or((0.0, "starting".into()));
                ui.add(egui::ProgressBar::new(frac).text(text));
                if ui.button("Cancel").clicked() { t.cancel(); }
                ctx.request_repaint_after(Duration::from_millis(100));
            } else {
                ui.horizontal(|ui| {
                    let any = e.formats.iter().any(|f| f.2);
                    if ui.add_enabled(any, egui::Button::new(RichText::new("Export").strong())).clicked() { start = true; }
                    ui.label(RichText::new("A metadata .json for the game is always written too.").weak());
                });
            }
            match &e.result {
                Some(Ok(r)) => {
                    ui.colored_label(Color32::from_rgb(120, 200, 130), format!("Done in {:.1} s: {} frames, {:.3} m per frame", r.elapsed_ms as f32 / 1000.0, r.frames, r.metres_per_frame));
                    for f in &r.files {
                        let kb = std::fs::metadata(f).map(|m| m.len() / 1024).unwrap_or(0);
                        ui.horizontal(|ui| {
                            ui.label(format!("{}  ({} KB)", f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), kb));
                            if ui.small_button("Show").clicked() { reveal(f); }
                        });
                    }
                    for n in &r.notes { ui.label(RichText::new(n).weak()); }
                }
                Some(Err(err)) => { ui.colored_label(Color32::from_rgb(240, 110, 100), err); }
                None => {}
            }
        });
        if start {
            let job = self.export_job();
            let e = &mut self.export;
            e.result = None;
            let kind = match e.clip {
                1 => worker::ClipKind::Encounter(crate::export::Encounter { slow_m: e.slow_m, start_frame: 0 }),
                2 => match &e.to_file {
                    Some(f) => match scene::load_file(f) {
                        Ok((b, _)) => worker::ClipKind::Transition(Box::new(b), f.parent().map(Path::to_path_buf), Box::new(self.walk.transition.clone()), 1),
                        Err(err) => { e.result = Some(Err(err)); worker::ClipKind::None }
                    },
                    None => worker::ClipKind::Transition(Box::new(presets::ALL[e.to_preset].1()), None, Box::new(self.walk.transition.clone()), 1),
                },
                _ => worker::ClipKind::Loop,
            };
            if !matches!(kind, worker::ClipKind::None) {
                e.task = Some(worker::ExportTask::spawn(ctx.clone(), (*scene).clone(), job, kind));
            }
        }
        self.export.open = open;
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let typing = ctx.memory(|m| m.focused().is_some());
        let sc = |m: Modifiers, k: Key| ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(m, k)));
        if sc(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z) { self.redo(); }
        if sc(Modifiers::COMMAND, Key::Z) { self.undo(); }
        if sc(Modifiers::COMMAND | Modifiers::SHIFT, Key::S) { self.save_as(); }
        if sc(Modifiers::COMMAND, Key::S) { self.save(); }
        if sc(Modifiers::COMMAND, Key::O) { self.open_dialog(); }
        if sc(Modifiers::COMMAND, Key::E) { self.open_export(); }
        if !typing {
            if ctx.input(|i| i.key_pressed(Key::Space)) { self.playing = !self.playing; }
            if ctx.input(|i| i.key_pressed(Key::ArrowLeft)) { self.step(-1); }
            if ctx.input(|i| i.key_pressed(Key::ArrowRight)) { self.step(1); }
            if ctx.input(|i| i.key_pressed(Key::G)) { self.guides = !self.guides; }
        }
    }
}

impl eframe::App for Studio {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.receive_frame(ctx);
        if let Some(rx) = &self.loop_rx {
            if let Ok(r) = rx.try_recv() { self.loop_report = Some(r); self.loop_rx = None; }
        }
        self.shortcuts(ctx);
        self.tick();
        self.watch_file();

        let before = self.doc.clone();
        egui::TopBottomPanel::top("menu").show(ctx, |ui| self.menu_bar(ui));
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if self.disk_changed {
                    ui.colored_label(Color32::from_rgb(240, 170, 90), "The file changed on disk while you have unsaved edits.");
                    if ui.button("Load theirs").clicked() { if let Some(p) = self.file.clone() { self.reload_from_disk(&p); } }
                    if ui.button("Keep mine").clicked() { self.disk_changed = false; }
                } else {
                    ui.label(RichText::new(&self.status).weak());
                }
            });
        });
        egui::TopBottomPanel::bottom("timeline").show(ctx, |ui| { ui.add_space(4.0); self.timeline(ui); ui.add_space(2.0); });
        egui::SidePanel::left("outliner").resizable(true).default_width(230.0).show(ctx, |ui| self.outliner(ui));
        egui::SidePanel::right("inspector").resizable(true).default_width(380.0).min_width(320.0).show(ctx, |ui| self.inspector(ui));
        egui::CentralPanel::default().frame(egui::Frame::none()).show(ctx, |ui| self.viewport(ui));
        self.gallery(ctx);
        self.remix_window(ctx);
        self.export_window(ctx);
        self.walk_window(ctx);
        self.dev_shot(ctx);

        if self.doc != before { self.sync_scene(); }
        self.commit_edits(ctx);
        self.request_frame();

        let title = format!("{}{} — PathForge", self.file.as_ref().and_then(|f| f.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Untitled".into()), if self.dirty() { " •" } else { "" });
        if title != self.title { ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone())); self.title = title; }

        // Repaint continuously only while something moves; otherwise wake for the file watcher.
        if self.playing || self.drag.is_some() || self.pending { ctx.request_repaint(); }
        else { ctx.request_repaint_after(Duration::from_millis(500)); }
    }
}

/// Bytes as KB or MB.
fn human(b: u64) -> String {
    if b >= 1_000_000 { format!("{:.1} MB", b as f64 / 1_000_000.0) } else { format!("{} KB", (b as f64 / 1000.0).round() as u64) }
}

fn reveal(p: &Path) {
    let _ = std::process::Command::new("open").arg("-R").arg(p).spawn();
}

fn style(ctx: &egui::Context) {
    // Follow the system light/dark setting; tune both themes.
    let mut d = egui::Visuals::dark();
    d.panel_fill = Color32::from_rgb(28, 28, 33);
    d.window_fill = Color32::from_rgb(32, 32, 38);
    d.extreme_bg_color = Color32::from_rgb(20, 20, 24);
    d.selection.bg_fill = Color32::from_rgb(70, 96, 150);
    d.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, Color32::from_gray(205));
    ctx.set_visuals_of(egui::Theme::Dark, d);
    let mut l = egui::Visuals::light();
    l.panel_fill = Color32::from_rgb(244, 244, 246);
    l.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, Color32::from_gray(40));
    ctx.set_visuals_of(egui::Theme::Light, l);
    for t in [egui::Theme::Dark, egui::Theme::Light] {
        ctx.style_mut_of(t, |s| { s.spacing.item_spacing = egui::vec2(8.0, 5.0); s.spacing.button_padding = egui::vec2(6.0, 3.0); });
    }
}
