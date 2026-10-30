//! The inspector: one widget per scene field, generated from the scene's JSON and its JSON Schema.
//!
//! Field names, order, enum choices, tooltips (the doc comments) and defaults all come from the
//! schema, so a field added to the scene appears here with no GUI code. `META` adds what the schema
//! does not know: slider ranges, units, and which fields are Advanced.

use crate::world::palette::NAMED;
use egui::{Color32, RichText, Ui};
use serde_json::{json, Value};

pub use crate::scene::form::Schema;
use crate::scene::form::{meta, HIDDEN};

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
    let mut lists: Vec<(String, &Value)> = Vec::new();
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
            // A list of structs (cloud layers, planets) is a group of its own too, with add and remove.
            if v.is_array() && schema.items(fs).is_some_and(|it| schema.resolve(it).get("properties").is_some()) { lists.push((key.clone(), fs)); continue; }
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
    for (key, fs) in lists {
        let p = if path.is_empty() { key.clone() } else { format!("{path}.{key}") };
        let Some(items) = schema.items(fs) else { continue };
        let Some(arr) = obj.get_mut(&key).and_then(|v| v.as_array_mut()) else { continue };
        let title = label_of(&key);
        let one = title.trim_end_matches('s').to_lowercase();
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let mut l = ui.label(RichText::new(&title).strong());
            if let Some(desc) = schema.description(fs) { l = l.on_hover_text(desc); }
            let _ = l;
            if ui.small_button("+ Add").on_hover_text(format!("Add a {one}")).clicked() { arr.push(default_item(schema, items)); changed = true; }
            if arr.is_empty() { ui.label(RichText::new("none").weak().italics()); }
        });
        let mut remove = None;
        for (i, item) in arr.iter_mut().enumerate() {
            let ip = format!("{p}.{i}");
            let kind = item.get("kind").and_then(|k| k.as_str()).map(|k| format!(" · {}", spaced(k))).unwrap_or_default();
            let r = egui::CollapsingHeader::new(format!("{} {}{kind}", label_of(&one), i + 1)).id_salt(("item", &ip)).default_open(false).show(ui, |ui| {
                if ui.small_button("🗑 Remove").on_hover_text(format!("Remove this {one} (Undo brings it back)")).clicked() { remove = Some(i); }
                edit_struct(ui, schema, items, &ip, item, opts)
            });
            changed |= r.body_returned.unwrap_or(false);
        }
        if let Some(i) = remove { arr.remove(i); changed = true; }
    }
    changed
}

/// A new item for a list of structs: every field at its default, switched on.
fn default_item(schema: &Schema, items: &Value) -> Value {
    let def = schema.resolve(items);
    let mut o = serde_json::Map::new();
    if let Some(props) = def.get("properties").and_then(|p| p.as_object()) {
        for (k, fs) in props { if let Some(d) = missing_default(schema, fs) { o.insert(k.clone(), d); } }
    }
    if o.contains_key("enabled") { o.insert("enabled".into(), json!(true)); }
    Value::Object(o)
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
    // A set of choices (the kinds a cloud mix picks from): a tick box each.
    if let Some(choices) = v.is_array().then(|| schema.items(fs).and_then(|it| schema.enum_values(it))).flatten() {
        return enum_set(ui, v, &choices);
    }
    match v {
        Value::Bool(b) => ui.checkbox(b, "").changed(),
        Value::Number(_) if path.ends_with(".seed") => {
            let mut changed = false;
            ui.horizontal(|ui| {
                changed = number(ui, fs, path, v);
                if ui.small_button("🎲").on_hover_text("Another random seed").clicked() { *v = json!(random_seed()); changed = true; }
            });
            changed
        }
        Value::Number(_) => number(ui, fs, path, v),
        // A prop layer's `def` and the grade: a pick from the built-in ones, or anything typed
        // (a kit of your own, a .cube file).
        Value::String(s) if crate::scene::form::text_choices(&crate::scene::form::pattern_of(path)).is_some() => {
            let choices = crate::scene::form::text_choices(&crate::scene::form::pattern_of(path)).unwrap_or_default();
            let label = |v: &str| choices.iter().find(|c| c["value"] == v).and_then(|c| c["label"].as_str()).map(str::to_owned);
            let mut changed = false;
            ui.horizontal(|ui| {
                let shown = label(s).unwrap_or_else(|| s.clone());
                egui::ComboBox::from_id_salt(("choice", path)).selected_text(shown).width(170.0).show_ui(ui, |ui| {
                    for c in &choices {
                        let (val, lab) = (c["value"].as_str().unwrap_or(""), c["label"].as_str().unwrap_or(""));
                        let r = ui.selectable_label(s == val, lab);
                        let r = match c["help"].as_str() { Some(h) if !h.is_empty() => r.on_hover_text(h), _ => r };
                        if r.clicked() && s != val { *s = val.to_owned(); changed = true; }
                    }
                });
                changed |= ui.add(egui::TextEdit::singleline(s).desired_width(110.0)).on_hover_text("Or type one: a kit of your own (kits/<name>#<prop>), a .cube file").changed();
            });
            changed
        }
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

/// Tick boxes for a list of enum values, kept in the order of `choices`.
fn enum_set(ui: &mut Ui, v: &mut Value, choices: &[String]) -> bool {
    let have: Vec<String> = v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect()).unwrap_or_default();
    let mut next = have.clone();
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(260.0);
        for c in choices {
            let mut on = have.contains(c);
            if ui.checkbox(&mut on, spaced(c)).changed() {
                next = choices.iter().filter(|k| if *k == c { on } else { have.contains(k) }).cloned().collect();
            }
        }
    });
    if next != have { *v = json!(next); true } else { false }
}

/// A seed from the clock, for the dice button.
fn random_seed() -> u32 {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1);
    let mut x = t ^ (t >> 29);
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    (x >> 32) as u32 % 10000
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
                                r.context_menu(|ui| if ui.button("Remove colour").clicked() { remove = Some(i); ui.close(); });
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
        // An image of your own can say which of its parts glow and which cast a shadow.
        let p = o.get("path").and_then(|x| x.as_str()).unwrap_or("").trim().to_owned();
        if !p.is_empty() {
            let full = if std::path::Path::new(&p).is_absolute() { std::path::PathBuf::from(&p) } else { base.map(|b| b.join(&p)).unwrap_or_else(|| std::path::PathBuf::from(&p)) };
            let masked = crate::world::sprites::mask_path(&full).is_file();
            if ui.add_enabled(full.is_file(), egui::Button::new(if masked { "Glow & shadow mask… ✔" } else { "Glow & shadow mask…" }).small())
                .on_hover_text("Paint which parts of the image glow and give off light, and which are solid and cast a shadow")
                .on_disabled_hover_text("Save the scene first, or pick an image file (not a folder)").clicked() {
                ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("pf-mask-open"), full.to_string_lossy().into_owned()));
            }
        }
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
        "ground_cover" => format!("{} · {}", s("kind"), s("on").to_lowercase()),
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
