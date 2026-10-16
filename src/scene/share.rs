//! Share codes: a scene as one short line of text, to paste into a message or scan as a QR code.
//!
//! A code holds what the scene changes from whichever preset it is closest to (a JSON merge
//! patch of the scene's fields), compressed and written in URL-safe base64 after `PF1.`. A preset
//! with a few edits comes to a few dozen characters; a scene far from every preset, a few
//! thousand. Decoding applies the patch to that preset, so a code made by one version of
//! PathForge opens in a later one as long as the preset it names still exists (codes made from
//! nothing but defaults name no preset).

use super::{presets, Scene};
use base64::Engine;
use serde_json::{Map, Value};

/// What every share code starts with (the format's version).
pub const PREFIX: &str = "PF1.";

/// What `b` changes from `a`: the keys of `b` whose values differ, objects compared key by key.
fn diff(a: &Value, b: &Value) -> Option<Value> {
    if a == b { return None; }
    match (a, b) {
        (Value::Object(ao), Value::Object(bo)) => {
            let mut out = Map::new();
            for (k, bv) in bo {
                match ao.get(k) {
                    Some(av) => { if let Some(d) = diff(av, bv) { out.insert(k.clone(), d); } }
                    None => { out.insert(k.clone(), bv.clone()); }
                }
            }
            Some(Value::Object(out))
        }
        // A list as long as before: what changed in each item ({"$a": [patch or null...]}).
        (Value::Array(aa), Value::Array(ba)) if aa.len() == ba.len() => {
            let items: Vec<Value> = aa.iter().zip(ba).map(|(x, y)| diff(x, y).unwrap_or(Value::Null)).collect();
            let whole = Value::Array(ba.clone());
            let each = serde_json::json!({ ARRAY: items });
            Some(if each.to_string().len() < whole.to_string().len() { each } else { whole })
        }
        _ => Some(b.clone()),
    }
}

/// The key of a patch that changes a list item by item (no scene field is named so).
const ARRAY: &str = "$a";

/// `a` with `patch` laid over it (the inverse of `diff`).
fn apply(a: &mut Value, patch: &Value) {
    if let (Value::Array(aa), Some(Value::Array(items))) = (&mut *a, patch.get(ARRAY)) {
        for (x, p) in aa.iter_mut().zip(items) { if !p.is_null() { apply(x, p); } }
        return;
    }
    match (a, patch) {
        (Value::Object(ao), Value::Object(po)) => {
            for (k, pv) in po {
                match ao.get_mut(k) {
                    Some(av) if (av.is_object() || av.is_array()) && pv.is_object() => apply(av, pv),
                    _ => { ao.insert(k.clone(), pv.clone()); }
                }
            }
        }
        (a, p) => *a = p.clone(),
    }
}

fn pack(base: &str, parts: Map<String, Value>, patch: Value) -> Result<String, String> {
    let mut body = serde_json::json!({ "b": base, "d": patch });
    if !parts.is_empty() { body["g"] = Value::Object(parts); }
    let bytes = serde_json::to_vec(&body).map_err(|e| e.to_string())?;
    let z = miniz_oxide::deflate::compress_to_vec(&bytes, 10);
    Ok(format!("{PREFIX}{}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(z)))
}

fn len_of(d: &Option<Value>) -> usize { d.as_ref().map_or(0, |v| v.to_string().len()) }

/// The share code of a scene. Each of its parts (sky, path, lights...) is written as what it
/// changes from the preset whose same part is nearest, so a scene put together from presets'
/// parts (a remix) comes out as short as one preset with a few edits. Checked to give the
/// scene back exactly.
pub fn encode(scene: &Scene) -> Result<String, String> {
    let v = serde_json::to_value(scene).map_err(|e| e.to_string())?;
    let Value::Object(vo) = &v else { return Err("a scene is an object".into()) };
    let ps: Vec<(&str, Value)> = presets::ALL.iter().map(|(n, f)| serde_json::to_value(f()).map(|p| (*n, p))).collect::<Result<_, _>>().map_err(|e| e.to_string())?;
    // The base: the preset nearest overall. Then each part from whichever preset is nearer
    // for it than the base is.
    let cost = |i: usize, k: &String| ps[i].1.get(k).map_or(usize::MAX / 4, |pv| len_of(&diff(pv, &vo[k])));
    let base = (0..ps.len()).min_by_key(|&i| vo.keys().map(|k| cost(i, k)).sum::<usize>()).unwrap_or(0);
    let (mut parts, mut start) = (Map::new(), ps[base].1.clone());
    for k in vo.keys() {
        let here = cost(base, k);
        if here == 0 { continue; }
        let pick = (0..ps.len()).min_by_key(|&i| cost(i, k)).unwrap_or(base);
        // (Naming another preset costs its name; take it only when that pays.)
        if cost(pick, k) + ps[pick].0.len() + k.len() + 8 < here {
            parts.insert(k.clone(), Value::from(ps[pick].0));
            start[k] = ps[pick].1[k].clone();
        }
    }
    let code = pack(ps[base].0, parts, diff(&start, &v).unwrap_or(Value::Object(Map::new())))?;
    if decode(&code).ok().as_ref() == Some(scene) { return Ok(code); }
    // (Should a patch not give the scene back, the whole scene goes.)
    pack("", Map::new(), v)
}

/// The scene a share code holds. Spaces and line breaks in the code are ignored, so a code
/// wrapped by a messenger still opens.
pub fn decode(code: &str) -> Result<Scene, String> {
    let code: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    let body = code.strip_prefix(PREFIX).ok_or_else(|| format!("not a PathForge share code (they start with {PREFIX})"))?;
    let z = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(body.trim_end_matches('=')).map_err(|e| format!("the code is damaged: {e}"))?;
    let bytes = miniz_oxide::inflate::decompress_to_vec_with_limit(&z, 16 << 20).map_err(|e| format!("the code is damaged: {e:?}"))?;
    let body: Value = serde_json::from_slice(&bytes).map_err(|e| format!("the code is damaged: {e}"))?;
    let preset = |name: &str| -> Result<Value, String> {
        let make = presets::ALL.iter().find(|p| p.0 == name).ok_or_else(|| format!("the code is made from the preset '{name}', which this version does not have"))?;
        serde_json::to_value((make.1)()).map_err(|e| e.to_string())
    };
    let base = body.get("b").and_then(Value::as_str).unwrap_or("");
    let mut v = if base.is_empty() { Value::Object(Map::new()) } else { preset(base)? };
    if let Some(Value::Object(parts)) = body.get("g") {
        for (k, name) in parts {
            let p = preset(name.as_str().unwrap_or(""))?;
            v[k] = p.get(k).cloned().unwrap_or(Value::Null);
        }
    }
    apply(&mut v, body.get("d").unwrap_or(&Value::Null));
    Scene::from_json(&v.to_string())
}

/// A QR code of `text`: its width in modules and whether each module (row by row) is dark.
pub fn qr(text: &str) -> Result<(usize, Vec<bool>), String> {
    let q = qrcode::QrCode::with_error_correction_level(text.as_bytes(), qrcode::EcLevel::L)
        .map_err(|e| format!("too long for a QR code ({} characters): {e}", text.len()))?;
    let w = q.width();
    Ok((w, q.to_colors().into_iter().map(|c| c == qrcode::Color::Dark).collect()))
}

/// The QR code as a PNG: `scale` pixels a module, with the four-module quiet margin round it.
pub fn qr_png(text: &str, scale: u32) -> Result<Vec<u8>, String> {
    let (w, dark) = qr(text)?;
    let s = scale.max(1) as usize;
    let side = (w + 8) * s;
    let mut px = vec![255u8; side * side];
    for (i, d) in dark.iter().enumerate() {
        if !d { continue; }
        let (mx, my) = (i % w + 4, i / w + 4);
        for y in my * s..(my + 1) * s { px[y * side + mx * s..y * side + (mx + 1) * s].fill(0); }
    }
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, side as u32, side as u32);
        enc.set_color(png::ColorType::Grayscale);
        enc.set_depth(png::BitDepth::Eight);
        let mut wr = enc.write_header().map_err(|e| e.to_string())?;
        wr.write_image_data(&px).map_err(|e| e.to_string())?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_give_the_scene_back_and_stay_short_for_a_preset() {
        let mut s = (presets::ALL.iter().find(|p| p.0 == "Forest Path").unwrap().1)();
        s.sky.sun.pos = [0.3, 0.2];
        s.post.vignette = 0.5;
        let code = encode(&s).unwrap();
        assert!(code.starts_with(PREFIX) && code.len() < 160, "{} chars: {code}", code.len());
        assert_eq!(decode(&code).unwrap(), s);
        // Wrapped by a messenger, it still opens.
        let wrapped = format!("{}\n {}", &code[..20], &code[20..]);
        assert_eq!(decode(&wrapped).unwrap(), s);
        // A remix (parts of several presets) is short too, and fits a QR code; nudged all over,
        // it is long but still gives the scene back.
        let r = crate::scene::remix::remix(&s, &["camera"], 7, 0.0).unwrap();
        let rc = encode(&r).unwrap();
        assert_eq!(decode(&rc).unwrap(), r);
        assert!(rc.len() < 1200, "{} chars", rc.len());
        let j = crate::scene::remix::remix(&s, &["camera"], 7, 0.2).unwrap();
        assert_eq!(decode(&encode(&j).unwrap()).unwrap(), j);
        let (w, dark) = qr(&rc).unwrap();
        assert_eq!(dark.len(), w * w);
        assert!(qr_png(&rc, 3).unwrap().starts_with(b"\x89PNG"));
        assert!(decode("hello").is_err() && decode("PF1.!!!").is_err());
    }
}
