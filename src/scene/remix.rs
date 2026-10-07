//! Remix: new variations of a scene. Each part you have not locked is taken from a preset picked at
//! random (a whole part at a time, so walls, verge and ceiling, or lights and their colours, still
//! belong together), then nudged by `jitter`. Same scene, locks and seed, same result.

use super::{presets, Scene};
use serde_json::Value;

/// The parts a scene is remixed by: (name, the scene fields that move together).
pub const GROUPS: [(&str, &[&str]); 8] = [
    ("camera", &["camera"]),
    ("path", &["path"]),
    ("setting", &["verge", "walls", "ceiling"]),
    ("sky", &["sky"]),
    ("lighting", &["light", "fixtures"]),
    ("props", &["props", "set_pieces", "prop_defs"]),
    ("atmosphere", &["particles", "weather"]),
    ("look", &["style", "post"]),
];

fn hash(seed: u32, k: u32) -> u32 {
    let mut h = seed.wrapping_mul(0x9E37_79B9) ^ k.wrapping_mul(0x85EB_CA6B);
    h ^= h >> 15; h = h.wrapping_mul(0x2C1B_3C6D); h ^= h >> 12; h = h.wrapping_mul(0x297A_2D39); h ^= h >> 15;
    h
}
fn unit(seed: u32, k: u32) -> f64 { (hash(seed, k) >> 8) as f64 / 16_777_216.0 }

/// Nudge every fractional number by up to ±`jitter` (as a fraction of itself) and give every
/// `seed` field a new value. Whole numbers (counts, colours) and flags are left alone.
fn nudge(v: &mut Value, jitter: f64, seed: u32, counter: &mut u32) {
    match v {
        Value::Object(o) => for (k, x) in o.iter_mut() {
            if k == "seed" { if x.is_u64() { *counter += 1; *x = Value::from(hash(seed, *counter) % 100_000); } continue; }
            nudge(x, jitter, seed, counter);
        },
        Value::Array(a) => for x in a { nudge(x, jitter, seed, counter) },
        Value::Number(n) if n.is_f64() && jitter > 0.0 => {
            *counter += 1;
            let f = n.as_f64().unwrap_or(0.0) * (1.0 + jitter * (unit(seed, *counter) * 2.0 - 1.0));
            *v = Value::from((f * 1000.0).round() / 1000.0);
        }
        _ => {}
    }
}

/// A variation of `base`: groups named in `keep` stay as they are; the rest come from random
/// presets and are nudged by `jitter` (0..0.5).
pub fn remix(base: &Scene, keep: &[&str], seed: u32, jitter: f32) -> Result<Scene, String> {
    for k in keep {
        if !GROUPS.iter().any(|g| g.0 == *k) {
            return Err(format!("no part '{k}' to keep; parts: {}", GROUPS.iter().map(|g| g.0).collect::<Vec<_>>().join(", ")));
        }
    }
    let mut v = serde_json::to_value(base).map_err(|e| e.to_string())?;
    let mut counter = 0u32;
    for (gi, (name, fields)) in GROUPS.iter().enumerate() {
        if keep.contains(name) { continue; }
        let pick = (hash(seed, 1000 + gi as u32) as usize) % presets::ALL.len();
        let donor = serde_json::to_value((presets::ALL[pick].1)()).map_err(|e| e.to_string())?;
        for f in *fields {
            let mut part = donor.get(*f).cloned().unwrap_or(Value::Null);
            if part.is_null() { if let Some(o) = v.as_object_mut() { o.remove(*f); } continue; }
            nudge(&mut part, jitter.clamp(0.0, 0.5) as f64, seed ^ (gi as u32 * 7919), &mut counter);
            v[*f] = part;
        }
    }
    let mut s: Scene = serde_json::from_value(v).map_err(|e| e.to_string())?;
    // An open setting under no sky is a black void overhead: unless the sky was kept, give it the
    // sky of the preset the setting came from (or the next one that has a sky).
    if !keep.contains(&"sky") && !s.ceiling.enabled && !s.sky.enabled {
        let from = (hash(seed, 1000 + 2) as usize) % presets::ALL.len();
        if let Some(sky) = (0..presets::ALL.len()).map(|k| (presets::ALL[(from + k) % presets::ALL.len()].1)().sky).find(|sk| sk.enabled) {
            s.sky = sky;
        }
    }
    s.name = format!("{} remix {seed}", base.name.split(" remix ").next().unwrap_or(&base.name));
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locked_parts_stay_and_the_rest_change_the_same_way_each_time() {
        let base = (presets::ALL[0].1)();
        let keep = ["camera", "path", "look"];
        let a = remix(&base, &keep, 7, 0.15).unwrap();
        assert_eq!(a.camera, base.camera);
        assert_eq!(a.path, base.path);
        assert_eq!((&a.style, &a.post), (&base.style, &base.post));
        assert_eq!(a.motion, base.motion, "loop length and speed are never remixed");
        assert_eq!(remix(&base, &keep, 7, 0.15).unwrap(), a, "same seed, same remix");
        // Across a few seeds, the unlocked parts do change.
        let changed = (1..6).any(|seed| { let r = remix(&base, &keep, seed, 0.15).unwrap(); r.sky != base.sky || r.props != base.props });
        assert!(changed);
        assert!(remix(&base, &["colour"], 1, 0.0).is_err());
        // No remix leaves an open setting under no sky unless the sky was kept.
        for seed in 0..40 {
            let r = remix(&base, &[], seed, 0.0).unwrap();
            assert!(r.ceiling.enabled || r.sky.enabled, "seed {seed}: open and skyless");
        }
        // Every remix is a scene that renders and still loops seamlessly.
        let mut r = crate::world::WorldRenderer::default();
        let opts = crate::world::RenderOptions { size: Some((90, 160)), ..Default::default() };
        for seed in 1..4 {
            let s = remix(&base, &[], seed, 0.3).unwrap();
            let rep = crate::review::check_loop(&mut r, &s, &opts, 6);
            assert!(rep.exact < 1e-6, "seed {seed}: loop {}", rep.exact);
        }
    }
}
