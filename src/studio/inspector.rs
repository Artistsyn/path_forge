//! The inspector: one widget per scene field, generated from the scene's JSON and its JSON Schema.
//!
//! Field names, order, enum choices, tooltips (the doc comments) and defaults all come from the
//! schema, so a field added to the scene appears here with no GUI code. `META` adds what the schema
//! does not know: slider ranges, units, and which fields are Advanced.

use crate::world::palette::NAMED;
use egui::{Color32, RichText, Ui};
use serde_json::{json, Map, Value};

/// Range, unit and visibility for fields matched by path ("*" matches a list index;
/// a pattern starting with "*." matches that suffix anywhere).
struct Meta { pat: &'static str, min: f64, max: f64, unit: &'static str, adv: bool }

const fn m(pat: &'static str, min: f64, max: f64, unit: &'static str) -> Meta { Meta { pat, min, max, unit, adv: false } }
const fn a(pat: &'static str, min: f64, max: f64, unit: &'static str) -> Meta { Meta { pat, min, max, unit, adv: true } }

const META: &[Meta] = &[
    m("canvas.width", 64.0, 2048.0, " px"), m("canvas.height", 64.0, 4096.0, " px"),
    m("camera.eye_height", 0.2, 10.0, " m"), m("camera.horizon", 0.02, 0.98, ""), m("camera.zoom", 0.3, 4.0, "×"), m("camera.lens_curve", -1.0, 1.0, ""),
    m("path.half_width", 0.2, 10.0, " m"), m("path.flare", 0.0, 0.95, ""), m("path.bend", -2.0, 2.0, ""), m("path.hill", -2.0, 2.0, ""),
    m("path.edge_noise", 0.0, 1.0, ""), m("path.edge_dark", 0.0, 1.0, ""),
    m("*.material.tile_size", 0.2, 10.0, " m"), m("*.material.brightness", 0.2, 3.0, "×"), m("*.material.damage", 0.0, 1.0, ""),
    a("*.material.noise", 0.0, 60.0, ""), a("*.material.rotate", 0.0, 1.0, ""),
    m("verge.tuft_density", 0.0, 4.0, "×"), m("verge.tuft_height", 0.02, 1.5, " m"),
    m("walls.gap", 0.0, 20.0, " m"), m("walls.height", 0.0, 20.0, " m"), m("walls.base_shadow", 0.0, 1.0, ""),
    m("ceiling.height", 1.0, 20.0, " m"),
    m("*.radius", 0.0, 0.4, ""), m("*.intensity", 0.0, 5.0, "×"), m("sky.moon.phase", 0.0, 1.0, ""), m("sky.moon.opacity", 0.0, 1.0, ""),
    m("sky.stars.count", 0.0, 1000.0, ""), m("sky.stars.size", 0.2, 4.0, "×"), m("sky.stars.twinkle", 0.0, 1.0, ""),
    m("sky.clouds.count", 0.0, 60.0, ""), m("sky.clouds.drift", 0.0, 5.0, "×"), m("sky.clouds.scale", 0.2, 4.0, "×"), m("sky.clouds.opacity", 0.0, 1.0, ""), a("sky.clouds.variation", 0.0, 1.0, ""),
    m("light.ambient", 0.0, 2.0, ""), m("light.fog.distance", 2.0, 300.0, " m"), m("light.bands", 0.0, 8.0, ""),
    m("fixtures.*.height", 0.0, 6.0, " m"), m("fixtures.*.spacing", 0.5, 48.0, " m"), m("fixtures.*.offset", 0.0, 48.0, " m"),
    m("fixtures.*.lateral", -3.0, 5.0, " m"), m("fixtures.*.size", 0.2, 4.0, "×"), m("fixtures.*.radius", 0.5, 30.0, " m"),
    m("fixtures.*.flicker", 0.0, 3.0, "×"), a("fixtures.*.jitter", 0.0, 2.0, " m"),
    m("props.*.lateral", -3.0, 20.0, " m"), m("props.*.spacing", 0.3, 48.0, " m"), m("props.*.offset", 0.0, 48.0, " m"),
    m("props.*.rows", 1.0, 8.0, ""), m("props.*.row_spacing", 0.5, 20.0, " m"), m("props.*.density", 0.0, 1.0, ""),
    m("props.*.scale", 0.1, 6.0, "×"), m("props.*.scale_var", 0.0, 1.0, ""), m("props.*.shadow_opacity", 0.0, 1.0, ""),
    a("props.*.jitter", 0.0, 5.0, " m"), a("props.*.sink", 0.0, 1.0, " m"),
    m("set_pieces.*.spacing", 4.0, 96.0, " m"), m("set_pieces.*.offset", 0.0, 96.0, " m"), m("set_pieces.*.width", 0.0, 12.0, " m"),
    m("set_pieces.*.height", 1.0, 10.0, " m"), m("set_pieces.*.shadow", 0.0, 1.0, ""),
    m("path.stairs.spacing", 2.0, 96.0, " m"), m("path.stairs.steps", 1.0, 40.0, ""), m("path.stairs.rise", 0.05, 0.4, " m"),
m("path.stairs.run", 0.15, 1.0, " m"), m("path.stairs.offset", 0.0, 96.0, " m"),
m("path.bridge.spacing", 4.0, 96.0, " m"), m("path.bridge.length", 0.5, 64.0, " m"), m("path.bridge.offset", 0.0, 96.0, " m"),
m("path.bridge.depth", 0.5, 120.0, " m"), m("path.bridge.rail_height", 0.2, 2.0, " m"),
m("path.fork.spacing", 4.0, 96.0, " m"), m("path.fork.offset", 0.0, 96.0, " m"), m("path.fork.angle", 5.0, 85.0, "°"),
m("path.fork.half_width", 0.2, 4.0, " m"), m("path.fork.depth", 0.5, 30.0, " m"), m("path.fork.height", 0.5, 6.0, " m"),
m("*.gloss", 0.0, 1.0, ""), m("*.ripples", 0.0, 1.0, ""),
m("weather.lightning.strikes", 0.0, 16.0, " per loop"), m("weather.lightning.intensity", 0.0, 4.0, "×"),
m("weather.fog_banks.spacing", 4.0, 96.0, " m"), m("weather.fog_banks.length", 0.5, 48.0, " m"), m("weather.fog_banks.density", 0.0, 3.0, "/m"),
m("weather.fog_banks.offset", 0.0, 96.0, " m"),
m("particles.*.count", 0.0, 2000.0, ""), m("particles.*.size", 0.2, 6.0, "×"), m("particles.*.speed", 0.0, 6.0, "×"),
    m("post.exposure", 0.0, 4.0, "×"), m("post.contrast", 0.3, 2.0, "×"), m("post.saturation", 0.0, 2.0, "×"),
    m("post.bloom", 0.0, 2.0, ""), m("post.vignette", 0.0, 1.0, ""), m("post.grain", 0.0, 1.0, ""),
    m("style.pixel_size", 1.0, 12.0, " px"), m("style.dither_strength", 0.0, 1.0, ""),
    m("style.paint", 0.0, 8.0, " px"), m("style.grade_strength", 0.0, 1.0, ""), m("style.paper", 0.0, 1.0, ""), m("style.scanlines", 0.0, 1.0, ""),
    m("motion.loop_length", 4.0, 200.0, " m"), m("motion.speed", 0.2, 30.0, " m/s"), m("motion.fps", 6.0, 60.0, " fps"),
    a("*.seed", 0.0, 9999.0, ""), a("*.sprite", 0.0, 0.0, ""),
];

/// Fields never shown (bookkeeping).
const HIDDEN: &[&str] = &["version", "name"];

fn pattern_of(path: &str) -> String {
    path.split('.').map(|s| if s.parse::<usize>().is_ok() { "*" } else { s }).collect::<Vec<_>>().join(".")
}

fn meta(path: &str) -> Option<&'static Meta> {
    let p = pattern_of(path);
    META.iter().find(|m| m.pat == p).or_else(|| META.iter().find(|m| m.pat.starts_with("*.") && p.ends_with(&m.pat[1..])))
}

/// The scene's JSON Schema, indexed for lookups.
pub struct Schema {
    defs: Map<String, Value>,
    root: Value,
}

impl Schema {
    pub fn new() -> Schema {
        let full = serde_json::to_value(schemars::schema_for!(crate::scene::Scene)).unwrap_or(Value::Null);
        let defs = full.get("$defs").and_then(|d| d.as_object()).cloned().unwrap_or_default();
        Schema { defs, root: full }
    }

    fn ref_name(s: &Value) -> Option<&str> {
        if let Some(r) = s.get("$ref").and_then(|r| r.as_str()) { return r.rsplit('/').next(); }
        for key in ["allOf", "anyOf"] {
            if let Some(list) = s.get(key).and_then(|l| l.as_array()) {
                for x in list { if let Some(n) = Self::ref_name(x) { return Some(n); } }
            }
        }
        None
    }

    fn resolve<'a>(&'a self, s: &'a Value) -> &'a Value {
        Self::ref_name(s).and_then(|n| self.defs.get(n)).unwrap_or(s)
    }

    /// The schema of a field inside a struct schema.
    fn field<'a>(&'a self, def: &'a Value, key: &str) -> Option<&'a Value> {
        self.resolve(def).get("properties").and_then(|p| p.get(key))
    }

    /// The schema of a section of the scene (camera, fixtures, ...).
    pub fn section(&self, key: &str) -> Option<&Value> { self.field(&self.root, key) }

    /// The item schema of a list field.
    pub fn items<'a>(&'a self, list: &'a Value) -> Option<&'a Value> { self.resolve(list).get("items") }

    fn enum_values(&self, s: &Value) -> Option<Vec<String>> {
        let d = self.resolve(s);
        if let Some(e) = d.get("enum").and_then(|e| e.as_array()) {
            return Some(e.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect());
        }
        let alts = d.get("oneOf").and_then(|o| o.as_array())?;
        let mut out = Vec::new();
        for alt in alts {
            if let Some(c) = alt.get("const").and_then(|c| c.as_str()) { out.push(c.to_owned()); }
            else if let Some(e) = alt.get("enum").and_then(|e| e.as_array()) { out.extend(e.iter().filter_map(|v| v.as_str().map(str::to_owned))); }
            else { return None; }
        }
        Some(out)
    }

    fn description(&self, s: &Value) -> Option<String> {
        s.get("description").or_else(|| self.resolve(s).get("description")).and_then(|d| d.as_str()).map(str::to_owned)
    }
}

impl Default for Schema { fn default() -> Self { Self::new() } }

pub struct Options {
    pub advanced: bool,
    /// The scene's folder, so picked files are named relative to it (and copied into it).
    pub base: Option<std::path::PathBuf>,
}

/// Edit one struct value. Returns true if anything changed.
pub fn edit_struct(ui: &mut Ui, schema: &Schema, def: &Value, path: &str, value: &mut Value, opts: &Options) -> bool {
    let Some(obj) = value.as_object_mut() else { return false; };
    let def = schema.resolve(def);
    let Some(props) = def.get("properties").and_then(|p| p.as_object()) else { return false; };
    let mut changed = false;
    let mut nested: Vec<(String, &Value)> = Vec::new();
    egui::Grid::new(("grid", path)).num_columns(3).spacing([8.0, 4.0]).striped(false).show(ui, |ui| {
        for (key, fs) in props {
            let p = if path.is_empty() { key.clone() } else { format!("{path}.{key}") };
            if HIDDEN.contains(&p.as_str()) { continue; }
            if meta(&p).is_some_and(|m| m.adv) && !opts.advanced { continue; }
            // Fields left out of the file when empty (a prop layer's `def`) show their default and
            // are added only once edited; inserting null would not load back.
            if !obj.contains_key(key) {
                let Some(d) = missing_default(schema, fs) else { continue };
                // A map of definitions (scene.prop_defs) is edited in the file or over MCP.
                if d.is_object() { continue; }
                let mut tmp = d;
                if field_row(ui, schema, fs, key, &p, &mut tmp, opts) { obj.insert(key.clone(), tmp); changed = true; }
                ui.end_row();
                continue;
            }
            let v = obj.get_mut(key).unwrap();
            // Structs other than colours and pairs open as their own group below the plain fields.
            let resolved = schema.resolve(fs);
            if resolved.get("properties").is_some() && v.is_object() { nested.push((key.clone(), fs)); continue; }
            changed |= field_row(ui, schema, fs, key, &p, v, opts);
            ui.end_row();
        }
    });
    for (key, fs) in nested {
        let p = if path.is_empty() { key.clone() } else { format!("{path}.{key}") };
        let v = obj.get_mut(&key).unwrap();
        let title = label_of(&key);
        let r = egui::CollapsingHeader::new(RichText::new(title).strong()).id_salt(("nested", &p)).default_open(true).show(ui, |ui| {
            edit_struct(ui, schema, fs, &p, v, opts)
        });
        if let Some(desc) = schema.description(fs) { r.header_response.on_hover_text(desc); }
        changed |= r.body_returned.unwrap_or(false);
    }
    changed
}

/// What to show for a field missing from the file: its default, or empty text for a string
/// (schemars leaves out the default of a field skipped when empty).
fn missing_default(schema: &Schema, fs: &Value) -> Option<Value> {
    fs.get("default").or_else(|| schema.resolve(fs).get("default")).cloned()
        .or_else(|| (fs.get("type").and_then(|t| t.as_str()) == Some("string")).then(|| json!("")))
}

pub fn label_of(key: &str) -> String {
    let s = key.replace('_', " ");
    let mut c = s.chars();
    match c.next() { Some(f) => f.to_uppercase().collect::<String>() + c.as_str(), None => s }
}

fn field_row(ui: &mut Ui, schema: &Schema, fs: &Value, key: &str, path: &str, v: &mut Value, opts: &Options) -> bool {
    let desc = schema.description(fs);
    let mut l = ui.label(label_of(key));
    if let Some(d) = &desc { l = l.on_hover_text(d); }
    let _ = l;
    let before = v.clone();
    let changed = widget(ui, schema, fs, path, v, opts);
    // Reset to the schema default when the value differs from it.
    let default = fs.get("default");
    match default {
        Some(d) if !same(d, v) => {
            if ui.small_button("↺").on_hover_text(format!("Reset to {}", short(d))).clicked() { *v = d.clone(); return true; }
        }
        _ => { ui.label(""); }
    }
    changed && before != *v
}

fn same(a: &Value, b: &Value) -> bool {
    match (a.as_f64(), b.as_f64()) { (Some(x), Some(y)) => (x - y).abs() < 1e-6, _ => a == b }
}

fn short(v: &Value) -> String {
    match v { Value::Number(n) => n.as_f64().map(|f| format!("{}", (f * 1000.0).round() / 1000.0)).unwrap_or_default(), _ => { let s = v.to_string(); if s.len() > 24 { format!("{}…", &s[..24]) } else { s } } }
}

fn is_colour(v: &Value) -> bool {
    v.as_array().is_some_and(|a| a.len() == 3 && a.iter().all(|x| x.as_u64().is_some_and(|n| n <= 255)))
}

fn widget(ui: &mut Ui, schema: &Schema, fs: &Value, path: &str, v: &mut Value, opts: &Options) -> bool {
    if path.ends_with("style.palette") { return palette_widget(ui, v); }
    if path.ends_with(".sprite") { return sprite_widget(ui, v, opts.base.as_deref()); }
    if path.ends_with("style.grade") { return grade_widget(ui, v, opts.base.as_deref()); }
    if let Some(choices) = schema.enum_values(fs) {
        let mut cur = v.as_str().unwrap_or("").to_owned();
        let mut changed = false;
        egui::ComboBox::from_id_salt(("enum", path)).selected_text(spaced(&cur)).width(150.0).show_ui(ui, |ui| {
            for c in &choices { changed |= ui.selectable_value(&mut cur, c.clone(), spaced(c)).changed(); }
        });
        if changed { *v = json!(cur); }
        return changed;
    }
    match v {
        Value::Bool(b) => ui.checkbox(b, "").changed(),
        Value::Number(_) => number(ui, fs, path, v),
        Value::String(s) => ui.add(egui::TextEdit::singleline(s).desired_width(150.0)).changed(),
        Value::Array(_) if is_colour(v) => {
            let a = v.as_array().unwrap();
            let mut c = [a[0].as_u64().unwrap_or(0) as u8, a[1].as_u64().unwrap_or(0) as u8, a[2].as_u64().unwrap_or(0) as u8];
            let r = ui.color_edit_button_srgb(&mut c);
            if r.changed() { *v = json!(c); true } else { false }
        }
        Value::Array(arr) if arr.len() == 2 && arr.iter().all(|x| x.is_number()) => {
            let mut changed = false;
            ui.horizontal(|ui| {
                for (i, x) in arr.iter_mut().enumerate() {
                    let mut f = x.as_f64().unwrap_or(0.0);
                    if ui.add(egui::DragValue::new(&mut f).speed(0.005).range(0.0..=1.0).prefix(if i == 0 { "x " } else { "y " })).changed() {
                        *x = json!(f); changed = true;
                    }
                }
            });
            changed
        }
        _ => { ui.label(RichText::new(short(v)).weak()); false }
    }
}

fn spaced(name: &str) -> String {
    let mut out = String::new();
    for (i, ch) in name.chars().enumerate() {
        if i > 0 && ch.is_uppercase() { out.push(' '); }
        out.push(ch);
    }
    out
}

fn number(ui: &mut Ui, fs: &Value, path: &str, v: &mut Value) -> bool {
    let integer = v.is_u64() || v.is_i64() || schema_is_integer(fs);
    let mut f = v.as_f64().unwrap_or(0.0);
    let meta = meta(path);
    let changed = match meta {
        Some(m) if m.max > m.min => {
            let mut s = egui::Slider::new(&mut f, m.min..=m.max).suffix(m.unit);
            if integer { s = s.integer(); } else { s = s.max_decimals(3); }
            ui.add(s).changed()
        }
        _ => {
            let speed = if integer { 1.0 } else { (f.abs() * 0.01).max(0.01) };
            let mut d = egui::DragValue::new(&mut f).speed(speed).suffix(meta.map(|m| m.unit).unwrap_or(""));
            if integer { d = d.fixed_decimals(0); } else { d = d.max_decimals(3); }
            ui.add(d).changed()
        }
    };
    if changed {
        *v = if integer { json!(f.round().max(if v.is_u64() || schema_unsigned(fs) { 0.0 } else { f64::MIN }) as i64) } else { json!((f * 10000.0).round() / 10000.0) };
    }
    changed
}

fn schema_is_integer(fs: &Value) -> bool { fs.get("type").and_then(|t| t.as_str()) == Some("integer") }
fn schema_unsigned(fs: &Value) -> bool { fs.get("format").and_then(|t| t.as_str()).is_some_and(|f| f.starts_with('u')) || fs.get("minimum").and_then(|m| m.as_f64()) == Some(0.0) }

fn palette_widget(ui: &mut Ui, v: &mut Value) -> bool {
    let kind = match v { Value::String(s) => s.clone(), Value::Object(o) => o.keys().next().cloned().unwrap_or_default(), _ => "Full".into() };
    let mut changed = false;
    ui.vertical(|ui| {
        let mut k = kind.clone();
        egui::ComboBox::from_id_salt("palette-kind").selected_text(match k.as_str() { "Full" => "Full colour", "Named" => "Named palette", "Auto" => "Best N colours", "Custom" => "Custom", _ => "?" }).show_ui(ui, |ui| {
            for (key, label) in [("Full", "Full colour"), ("Named", "Named palette"), ("Auto", "Best N colours"), ("Custom", "Custom")] {
                ui.selectable_value(&mut k, key.to_owned(), label);
            }
        });
        if k != kind {
            *v = match k.as_str() {
                "Named" => json!({"Named": NAMED[0].0}),
                "Auto" => json!({"Auto": 16}),
                "Custom" => json!({"Custom": [[20, 16, 24], [120, 90, 70], [230, 210, 170]]}),
                _ => json!("Full"),
            };
            changed = true;
        }
        match v {
            Value::Object(o) if o.contains_key("Named") => {
                let mut name = o["Named"].as_str().unwrap_or("").to_owned();
                egui::ComboBox::from_id_salt("palette-name").selected_text(&name).show_ui(ui, |ui| {
                    for (n, hex) in NAMED {
                        let r = ui.selectable_value(&mut name, (*n).to_owned(), *n);
                        r.on_hover_ui(|ui| { swatches(ui, &crate::world::palette::parse_hex_list(hex)); });
                    }
                });
                if o["Named"].as_str() != Some(name.as_str()) { o.insert("Named".into(), json!(name)); changed = true; }
                if let Some(cols) = crate::world::palette::named(o["Named"].as_str().unwrap_or("")) { swatches(ui, &cols); }
            }
            Value::Object(o) if o.contains_key("Auto") => {
                let mut n = o["Auto"].as_u64().unwrap_or(16);
                if ui.add(egui::Slider::new(&mut n, 2..=64).suffix(" colours")).changed() { o.insert("Auto".into(), json!(n)); changed = true; }
            }
            Value::Object(o) if o.contains_key("Custom") => {
                if let Some(list) = o.get_mut("Custom").and_then(|l| l.as_array_mut()) {
                    let mut remove = None;
                    ui.horizontal_wrapped(|ui| {
                        for (i, c) in list.iter_mut().enumerate() {
                            if let Some(a) = c.as_array() {
                                let mut rgb = [a.first().and_then(|x| x.as_u64()).unwrap_or(0) as u8, a.get(1).and_then(|x| x.as_u64()).unwrap_or(0) as u8, a.get(2).and_then(|x| x.as_u64()).unwrap_or(0) as u8];
                                let r = ui.color_edit_button_srgb(&mut rgb);
                                if r.changed() { *c = json!(rgb); changed = true; }
                                r.context_menu(|ui| if ui.button("Remove colour").clicked() { remove = Some(i); ui.close_menu(); });
                            }
                        }
                        if ui.small_button("+").on_hover_text("Add a colour").clicked() { list.push(json!([128, 128, 128])); changed = true; }
                    });
                    if let Some(i) = remove { if list.len() > 2 { list.remove(i); changed = true; } }
                }
            }
            _ => {}
        }
    });
    changed
}

fn grade_widget(ui: &mut Ui, v: &mut Value, base: Option<&std::path::Path>) -> bool {
    let cur = v.as_str().unwrap_or("").to_owned();
    let mut pick = cur.clone();
    let shown = if cur.is_empty() { "None".to_owned() } else if cur.to_lowercase().ends_with(".cube") { std::path::Path::new(&cur).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or(cur.clone()) } else { cur.clone() };
    let mut file = false;
    egui::ComboBox::from_id_salt("grade").selected_text(shown).width(150.0).show_ui(ui, |ui| {
        ui.selectable_value(&mut pick, String::new(), "None");
        for g in crate::world::looks::BUILTIN { ui.selectable_value(&mut pick, (*g).to_owned(), *g); }
        if ui.selectable_label(false, "LUT file (.cube)…").clicked() { file = true; }
    });
    if file {
        if let Some(f) = rfd::FileDialog::new().add_filter("3D LUT", &["cube"]).pick_file() {
            if let Ok(p) = crate::project::adopt_file(&f, base) { pick = p; }
        }
    }
    if pick != cur { *v = json!(pick); true } else { false }
}

fn swatches(ui: &mut Ui, cols: &[[u8; 3]]) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(1.0, 1.0);
        for c in cols {
            let (r, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
            ui.painter().rect_filled(r, 1.0, Color32::from_rgb(c[0], c[1], c[2]));
        }
    });
}

fn sprite_widget(ui: &mut Ui, v: &mut Value, base: Option<&std::path::Path>) -> bool {
    let mut changed = false;
    if !v.is_object() { *v = json!({"path": ""}); }
    let o = v.as_object_mut().unwrap();
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            let mut p = o.get("path").and_then(|x| x.as_str()).unwrap_or("").to_owned();
            if ui.add(egui::TextEdit::singleline(&mut p).hint_text("procedural").desired_width(110.0)).changed() { o.insert("path".into(), json!(p)); changed = true; }
            if ui.small_button("…").on_hover_text("Choose an image (PNG with transparency)").clicked() {
                if let Some(f) = rfd::FileDialog::new().add_filter("Image", &["png"]).pick_file() {
                    // Kept with the scene: relative, copied into its assets/ when from elsewhere.
                    if let Ok(p) = crate::project::adopt_file(&f, base) { o.insert("path".into(), json!(p)); changed = true; }
                }
            }
        });
        for key in ["pixelated", "flip_x"] {
            let mut b = o.get(key).and_then(|x| x.as_bool()).unwrap_or(false);
            if ui.checkbox(&mut b, label_of(key)).changed() { o.insert(key.into(), json!(b)); changed = true; }
        }
    });
    changed
}

/// One line describing a list item, for the outliner.
pub fn item_summary(section: &str, v: &Value) -> String {
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(spaced).unwrap_or_default();
    let f = |k: &str| v.get(k).and_then(|x| x.as_f64()).unwrap_or(0.0);
    match section {
        "fixtures" => format!("{} · {} · every {} m", s("kind"), s("side").to_lowercase(), f("spacing")),
        "props" => format!("{} · {} · every {} m", s("kind"), s("side").to_lowercase(), f("spacing")),
        "particles" => format!("{} · {}", s("kind"), v.get("count").and_then(|x| x.as_u64()).unwrap_or(0)),
        "set_pieces" => format!("{} · every {} m", s("kind"), f("spacing")),
        _ => section.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_left_out_of_files_have_defaults_to_show() {
        // A prop layer's `def` is left out of a file when empty; the inspector shows its default.
        let schema = Schema::new();
        let full = serde_json::to_value(schemars::schema_for!(crate::scene::Scene)).unwrap();
        let layer = &full["$defs"]["PropLayer"]["properties"]["def"];
        assert_eq!(missing_default(&schema, layer), Some(json!("")), "{layer}");
        // A map of definitions shows nothing rather than a null that would not load back.
        let defs = &full["properties"]["prop_defs"];
        assert!(missing_default(&schema, defs).is_none_or(|d| d.is_object()), "{defs}");
    }
}
