//! The scene file reference, written from the scene's JSON schema (the types' own doc comments
//! and defaults), so it cannot fall out of step with the code: `pf schema --markdown` prints it,
//! `docs/SCENE_REFERENCE.md` is that output, and a test fails when the two differ.

use crate::journey::Journey;
use crate::scene::transition::{ForkChoice, Transition};
use crate::scene::Scene;
use serde_json::{Map, Value};

/// The JSON schema of a scene, with the transition, fork and journey types beside it.
pub fn schema() -> Value {
    let mut full = serde_json::to_value(schemars::schema_for!(Scene)).unwrap_or_default();
    let mut defs = full.get("$defs").cloned().and_then(|d| d.as_object().cloned()).unwrap_or_default();
    for (name, extra) in [
        ("Journey", serde_json::to_value(schemars::schema_for!(Journey)).unwrap_or_default()),
        ("Transition", serde_json::to_value(schemars::schema_for!(Transition)).unwrap_or_default()),
        ("ForkChoice", serde_json::to_value(schemars::schema_for!(ForkChoice)).unwrap_or_default()),
    ] {
        let mut top = extra.clone();
        if let Some(d) = extra.get("$defs").and_then(Value::as_object) {
            for (k, v) in d { defs.entry(k.clone()).or_insert(v.clone()); }
        }
        if let Some(t) = top.as_object_mut() { t.remove("$defs"); t.remove("$schema"); }
        defs.insert(name.into(), top);
    }
    if let Some(o) = full.as_object_mut() { o.insert("$defs".into(), Value::Object(defs)); }
    full
}

fn anchor(name: &str) -> String { name.to_lowercase() }

fn ty(s: &Value) -> String {
    if let Some(r) = s.get("$ref").and_then(Value::as_str) {
        let n = r.rsplit('/').next().unwrap_or(r);
        return format!("[{n}](#{})", anchor(n));
    }
    for k in ["anyOf", "oneOf"] {
        if let Some(a) = s.get(k).and_then(Value::as_array) {
            let parts: Vec<String> = a.iter().filter(|x| x.get("type").and_then(Value::as_str) != Some("null")).map(ty).collect();
            let nullable = a.iter().any(|x| x.get("type").and_then(Value::as_str) == Some("null"));
            return format!("{}{}", parts.join(" or "), if nullable { " or null" } else { "" });
        }
    }
    if let Some(e) = s.get("enum").and_then(Value::as_array) {
        return e.iter().map(|v| format!("`{}`", v.as_str().map(str::to_owned).unwrap_or_else(|| v.to_string()))).collect::<Vec<_>>().join(" \\| ");
    }
    if let Some(c) = s.get("const") { return format!("`{}`", c.as_str().map(str::to_owned).unwrap_or_else(|| c.to_string())); }
    let types: Vec<String> = match s.get("type") {
        Some(Value::String(t)) => vec![t.clone()],
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).filter(|t| *t != "null").map(str::to_owned).collect(),
        _ => vec![],
    };
    let nullable = matches!(s.get("type"), Some(Value::Array(a)) if a.iter().any(|t| t == "null"));
    let base = match types.first().map(String::as_str) {
        Some("array") => {
            let item = s.get("items").map(ty).unwrap_or_else(|| "value".into());
            match (s.get("minItems"), s.get("maxItems")) { (Some(a), Some(b)) if a == b => format!("{item} × {a}"), _ => format!("list of {item}") }
        }
        Some("object") => match s.get("additionalProperties") { Some(v) if v.is_object() => format!("map of name → {}", ty(v)), _ => "object".into() },
        Some("number") => "number".into(),
        Some("integer") => "integer".into(),
        Some(t) => t.into(),
        None => "value".into(),
    };
    if nullable { format!("{base} or null") } else { base }
}

fn cell(s: &str) -> String { s.replace('|', "\\|").replace('\n', " ") }

fn default_text(v: &Value) -> String {
    let t = match v { Value::String(s) => format!("\"{s}\""), other => other.to_string() };
    if t.len() > 48 { "(see the type)".into() } else { format!("`{}`", cell(&t)) }
}

fn section(out: &mut String, name: &str, s: &Value) {
    out.push_str(&format!("### {name}\n\n"));
    if let Some(d) = s.get("description").and_then(Value::as_str) { out.push_str(&format!("{}\n\n", d.trim())); }
    if let Some(props) = s.get("properties").and_then(Value::as_object) {
        out.push_str("| Field | Type | Default | Meaning |\n|---|---|---|---|\n");
        for (k, p) in props {
            let d = p.get("default").map(default_text).unwrap_or_default();
            let desc = p.get("description").and_then(Value::as_str).unwrap_or("");
            out.push_str(&format!("| `{k}` | {} | {d} | {} |\n", cell(&ty(p)), cell(desc.trim())));
        }
        out.push('\n');
    } else if let Some(variants) = s.get("oneOf").or(s.get("anyOf")).and_then(Value::as_array) {
        out.push_str("| Value | Meaning |\n|---|---|\n");
        for v in variants {
            let label = if let Some(c) = v.get("const") { format!("`{}`", c.as_str().unwrap_or_default()) }
                else if let Some(e) = v.get("enum").and_then(Value::as_array) { e.iter().filter_map(Value::as_str).map(|x| format!("`{x}`")).collect::<Vec<_>>().join(", ") }
                else if let Some(p) = v.get("properties").and_then(Value::as_object) { p.iter().map(|(k, x)| format!("`{{\"{k}\": {}}}`", cell(&ty(x)))).collect::<Vec<_>>().join(", ") }
                else { ty(v) };
            out.push_str(&format!("| {} | {} |\n", cell(&label), cell(v.get("description").and_then(Value::as_str).unwrap_or("").trim())));
        }
        out.push('\n');
    } else if s.get("enum").is_some() {
        out.push_str(&format!("One of {}.\n\n", ty(s)));
    } else {
        out.push_str(&format!("{}.\n\n", ty(s)));
    }
}

/// The reference as Markdown: the scene's top level, then every type it uses, by name.
pub fn scene_reference() -> String {
    let full = schema();
    let mut out = String::from(
        "# PathForge scene reference\n\n\
         Generated from the code by `pf schema --markdown`; do not edit by hand (a test compares this \
         file with the code). Every field is optional: a missing field takes its default. Enum values \
         are written as shown (`\"Torch\"`, `{\"Named\": \"PICO-8\"}`). Lengths are metres, angles \
         degrees, colours `[r, g, b]` 0..255, times seconds.\n\n\
         See [MANUAL.md](MANUAL.md) for what each part does and how to use it.\n\n",
    );
    let mut top = full.clone();
    if let Some(o) = top.as_object_mut() { o.remove("$defs"); }
    section(&mut out, "Scene", &top);
    let defs: Map<String, Value> = full.get("$defs").and_then(Value::as_object).cloned().unwrap_or_default();
    // Transitions, forks and journeys first among the rest: they are files of their own.
    let mut names: Vec<&String> = defs.keys().collect();
    names.sort_by_key(|n| (!matches!(n.as_str(), "Journey" | "Transition" | "ForkChoice"), n.to_lowercase()));
    out.push_str("## Types\n\n");
    for n in names { section(&mut out, n, &defs[n]); }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_checked_in_scene_reference_matches_the_code() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/SCENE_REFERENCE.md");
        let on_disk = std::fs::read_to_string(&path).unwrap_or_default().replace("\r\n", "\n");
        assert!(
            on_disk == super::scene_reference(),
            "docs/SCENE_REFERENCE.md is out of date: run `pf schema --markdown > docs/SCENE_REFERENCE.md`"
        );
    }
}
