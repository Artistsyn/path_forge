//! Props described by data: definitions in `scene.prop_defs` or in kits (folders holding kit.json
//! and their images), resolved once per frame to the sprites, height, glow and light a prop layer
//! draws with. Folders in a sprite pool stand for every PNG inside them.

use super::sprites::{Sprite, SpriteCache};
use crate::scene::{Anchor, Kit, PropDef, PropKind, PropLayer, PropLight, Rgb, Scene};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

/// What a prop layer draws.
pub struct ResolvedProp {
    /// Images to pick from per prop; empty = the built-in `kind`.
    pub sprites: Vec<Arc<Sprite>>,
    pub kind: PropKind,
    pub tint: Rgb,
    /// Height in metres at scale 1.
    pub height: f32,
    pub hanging: bool,
    pub glow: f32,
    pub light: Option<PropLight>,
    pub pixelated: bool,
    /// Mirror every image; drawn and built-in props are mirrored at random instead.
    pub flip_x: bool,
    /// Images come from files (fixed orientation unless `flip_x`).
    pub from_files: bool,
}

#[derive(Default)]
pub struct PropDefs {
    kits: HashMap<PathBuf, (Option<SystemTime>, Result<Kit, String>)>,
    dirs: HashMap<PathBuf, (Option<SystemTime>, Vec<PathBuf>)>,
}

/// Kits compiled in, used as `def: "@<name>#<prop>"`: drawn from shapes, so they need no files.
pub const BUILTIN_KITS: [(&str, &str); 1] = [("wayside", include_str!("../../kits/wayside/kit.json"))];

fn mtime(p: &Path) -> Option<SystemTime> { std::fs::metadata(p).and_then(|m| m.modified()).ok() }

/// The kit.json of a kit folder (or the file itself when a .json is named).
pub fn kit_file(path: &Path) -> PathBuf {
    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("json")) { path.to_path_buf() } else { path.join("kit.json") }
}

fn join(base: Option<&Path>, p: &str) -> PathBuf {
    let p = Path::new(p.trim());
    match base { Some(b) if p.is_relative() => b.join(p), _ => p.to_path_buf() }
}

/// A relative kit folder is looked for beside the scene, then in each folder above it, so
/// `kits/swamp` works from any scene below the folder that holds `kits/`. The first place tried is
/// returned when none has a kit.json (and the error names it).
pub fn find_kit(base: Option<&Path>, kit: &str) -> PathBuf {
    let first = join(base, kit);
    if Path::new(kit.trim()).is_absolute() || kit_file(&first).is_file() { return first; }
    let mut dir = base.and_then(Path::parent);
    for _ in 0..8 {
        let Some(d) = dir else { break };
        let c = d.join(kit.trim());
        if kit_file(&c).is_file() { return c; }
        dir = d.parent();
    }
    first
}

impl PropDefs {
    /// Read a kit (cached until kit.json changes).
    pub fn kit(&mut self, path: &Path) -> Result<Kit, String> {
        if let Some(name) = path.to_str().and_then(|p| p.strip_prefix('@')) {
            let text = BUILTIN_KITS.iter().find(|k| k.0 == name).map(|k| k.1).ok_or_else(|| {
                format!("no built-in kit '@{name}' (there is {})", BUILTIN_KITS.iter().map(|k| format!("@{}", k.0)).collect::<Vec<_>>().join(", "))
            })?;
            return serde_json::from_str(text).map_err(|e| format!("@{name}: {e}"));
        }
        let file = kit_file(path);
        let t = mtime(&file);
        if let Some((ct, k)) = self.kits.get(&file) { if *ct == t { return k.clone(); } }
        let k = std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))
            .and_then(|text| serde_json::from_str::<Kit>(&text).map_err(|e| format!("{}: {e}", file.display())));
        self.kits.insert(file, (t, k.clone()));
        k
    }

    /// The definition a layer's `def` names, and the folder its relative paths start from.
    pub fn find(&mut self, scene: &Scene, base: Option<&Path>, def: &str) -> Result<(PropDef, PathBuf), String> {
        let here = base.map(Path::to_path_buf).unwrap_or_default();
        match def.split_once('#') {
            None => scene.prop_defs.get(def.trim()).cloned().map(|d| (d, here))
                .ok_or_else(|| format!("no prop definition '{}' in this scene's prop_defs", def.trim())),
            Some((kit, name)) => {
                let dir = if kit.trim().starts_with('@') { PathBuf::from(kit.trim()) } else { find_kit(base, kit) };
                let k = self.kit(&dir)?;
                let dir = kit_file(&dir).parent().map(Path::to_path_buf).unwrap_or_default();
                let d = k.props.get(name.trim()).cloned().ok_or_else(|| {
                    let names: Vec<&String> = k.props.keys().collect();
                    format!("kit '{}' has no prop '{}' (it has {names:?})", kit.trim(), name.trim())
                })?;
                Ok((d, dir))
            }
        }
    }

    /// Image files a pool names, folders expanded to the PNGs inside them in name order.
    pub fn pool(&mut self, base: &Path, path: &str, pool: &[String]) -> Vec<PathBuf> {
        let mut entries: Vec<&str> = pool.iter().map(|p| p.trim()).filter(|p| !p.is_empty()).collect();
        if entries.is_empty() && !path.trim().is_empty() { entries.push(path.trim()); }
        let mut out = Vec::new();
        for e in entries {
            let full = join(Some(base), e);
            if full.is_dir() {
                let t = mtime(&full);
                let fresh = matches!(self.dirs.get(&full), Some((ct, _)) if *ct == t);
                if !fresh {
                    let mut files: Vec<PathBuf> = std::fs::read_dir(&full).map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path()))
                        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("png"))).collect()).unwrap_or_default();
                    files.sort();
                    self.dirs.insert(full.clone(), (t, files));
                }
                out.extend(self.dirs[&full].1.iter().cloned());
            } else {
                out.push(full);
            }
        }
        out
    }

    /// What a layer draws. A layer whose `def` cannot be found draws nothing (None), and the error
    /// says why; a definition whose images cannot be read draws its `kind` instead.
    pub fn resolve(&mut self, scene: &Scene, base: Option<&Path>, layer: &PropLayer, sprites: &mut SpriteCache) -> (Option<ResolvedProp>, Option<String>) {
        let builtin = |kind: PropKind, tint: Rgb| ResolvedProp {
            sprites: Vec::new(), kind, tint, height: kind.height_m(), hanging: kind.hangs(), glow: kind.glow(), light: None,
            pixelated: false, flip_x: false, from_files: false,
        };
        let here = base.map(Path::to_path_buf).unwrap_or_default();
        if layer.def.trim().is_empty() {
            let mut r = builtin(layer.kind, layer.tint);
            if layer.sprite.is_set() {
                let files = self.pool(&here, &layer.sprite.path, &layer.sprite.pool);
                r.sprites = files.iter().filter_map(|f| sprites.file_at(f, false)).collect();
                r.from_files = !r.sprites.is_empty();
                r.flip_x = layer.sprite.flip_x;
                r.pixelated = layer.sprite.pixelated;
            }
            return (Some(r), None);
        }
        let (d, dir) = match self.find(scene, base, &layer.def) {
            Ok(x) => x,
            Err(e) => return (None, Some(format!("prop def '{}': {e}; the layer draws nothing", layer.def))),
        };
        let mut r = builtin(d.kind, d.tint.unwrap_or(d.kind.default_tint()));
        r.hanging = d.anchor == Anchor::Ceiling;
        r.glow = d.glow.max(0.0);
        r.light = d.light.enabled.then(|| d.light.clone());
        r.pixelated = d.sprite.pixelated;
        r.flip_x = d.sprite.flip_x;
        let mut warn = None;
        if d.sprite.is_set() {
            let files = self.pool(&dir, &d.sprite.path, &d.sprite.pool);
            let glow_from = d.glow_from.clamp(0.0, 1.0);
            r.sprites = files.iter().filter_map(|f| if glow_from > 0.0 { sprites.file_glowing(f, d.trim, glow_from, d.glow.max(0.0)) } else { sprites.file_at(f, d.trim) }).collect();
            // The glow is in the images' lightest texels, not over the whole prop.
            if glow_from > 0.0 { r.glow = 0.0; }
            r.from_files = true;
            if r.sprites.is_empty() {
                r.from_files = false;
                warn = Some(format!("prop def '{}': no readable PNG in {:?} (under {}); drawing {} instead", layer.def, if d.sprite.pool.is_empty() { vec![d.sprite.path.clone()] } else { d.sprite.pool.clone() }, dir.display(), d.kind.name()));
            } else if files.len() > r.sprites.len() {
                warn = Some(format!("prop def '{}': {} of {} images could not be read", layer.def, files.len() - r.sprites.len(), files.len()));
            }
        } else if !d.shape.is_empty() {
            match sprites.shape(&d.shape) {
                Some((s, h)) => { r.sprites = vec![s]; r.height = h; }
                None => warn = Some(format!("prop def '{}': its shape covers nothing above the ground", layer.def)),
            }
        }
        if d.height > 0.0 { r.height = d.height; }
        (Some(r), warn)
    }
}

/// Every prop layer whose definition or images cannot be used, said in a sentence each.
pub fn def_warnings(scene: &Scene, base: Option<&Path>) -> Vec<String> {
    let (mut defs, mut sprites) = (PropDefs::default(), SpriteCache::default());
    scene.props.iter().enumerate().filter(|(_, p)| p.enabled)
        .filter_map(|(i, p)| defs.resolve(scene, base, p, &mut sprites).1.map(|w| format!("props[{i}]: {w}")))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{PropLayer, Shape, ShapePart, Side};
    use crate::world::{RenderOptions, WorldRenderer};

    fn layer(def: &str) -> PropLayer {
        PropLayer { def: def.into(), side: Side::Center, lateral: 0.6, spacing: 24.0, offset: 4.5, jitter: 0.0, scale_var: 0.0, ..PropLayer::default() }
    }

    #[test]
    fn every_builtin_kit_prop_resolves() {
        let s = crate::review::kit_backdrop();
        for (kit, _) in BUILTIN_KITS {
            let k = PropDefs::default().kit(Path::new(&format!("@{kit}"))).unwrap();
            assert!(!k.props.is_empty());
            for (name, d) in &k.props {
                assert!(!d.description.is_empty(), "@{kit}#{name} has no description");
                let mut t = s.clone();
                t.props = vec![layer(&format!("@{kit}#{name}"))];
                assert!(def_warnings(&t, None).is_empty(), "@{kit}#{name}: {:?}", def_warnings(&t, None));
                let (r, _) = PropDefs::default().resolve(&t, None, &t.props[0], &mut SpriteCache::default());
                assert_eq!(r.unwrap().sprites.len(), 1, "@{kit}#{name} draws its shape");
            }
        }
    }

    #[test]
    fn a_drawn_prop_takes_its_height_and_glows_only_where_its_parts_do() {
        let parts = vec![
            ShapePart { shape: Shape::Rect { min: [-0.05, 0.0], max: [0.05, 2.0] }, color: [80, 60, 40], ..ShapePart::default() },
            ShapePart { shape: Shape::Rect { min: [-0.2, 1.5], max: [0.2, 1.9] }, color: [255, 200, 120], glow: 2.0, shade: 0.0, ..ShapePart::default() },
        ];
        let (s, h) = super::super::sprites::paint_shape(&parts).unwrap();
        assert!((h - 2.0).abs() < 1e-4);
        assert!((s.aspect - 0.4 / 2.0).abs() < 0.02, "aspect {}", s.aspect);
        let g = s.glow.as_ref().expect("glowing parts give the sprite a glow layer");
        // Middle of the lantern glows; the post below it does not.
        assert!(g.sample(0.5, 0.2, 1000.0, true)[0] > 1.9);
        assert!(g.sample(0.5, 0.8, 1000.0, true)[0] < 0.01);
        // Nothing to draw: no sprite.
        assert!(super::super::sprites::paint_shape(&[ShapePart { cut: true, ..parts[0].clone() }]).is_none());
    }

    #[test]
    fn a_prop_that_gives_light_lights_the_road_and_keeps_the_loop() {
        let mut s = crate::review::kit_backdrop();
        s.props = vec![PropLayer { spacing: 8.0, ..layer("@wayside#lantern_post") }];
        let mut r = WorldRenderer::default();
        let opts = RenderOptions { stats: true, ..RenderOptions::default() };
        let lit = r.render(&s, 0.0, &opts).stats.unwrap();
        let mut dark = s.clone();
        dark.prop_defs.insert("post".into(), PropDefs::default().find(&s, None, "@wayside#lantern_post").unwrap().0);
        dark.prop_defs.get_mut("post").unwrap().light.enabled = false;
        dark.props[0].def = "post".into();
        let unlit = r.render(&dark, 0.0, &opts).stats.unwrap();
        assert!(lit.lights > unlit.lights, "{} vs {} lights", lit.lights, unlit.lights);
        let (a, b) = (lit.luma["path"], unlit.luma["path"]);
        assert!(a > b * 1.1, "the lamps should brighten the road: {a} vs {b}");
        let rep = crate::review::check_loop(&mut r, &s, &RenderOptions::default(), 8);
        assert!(rep.exact < 1e-6, "loop {}", rep.exact);
    }

    #[test]
    fn kits_from_folders_trim_their_images_and_say_what_is_wrong() {
        let root = std::env::temp_dir().join(format!("pf_kit_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let kit = root.join("kits/bog");
        std::fs::create_dir_all(kit.join("reeds")).unwrap();
        // Two 40x40 images, each a 10x20 opaque block with empty margins all round.
        for (i, name) in ["a.png", "b.png"].iter().enumerate() {
            let mut img = image::RgbaImage::new(40, 40);
            for y in 12..32 { for x in 15..25 { img.put_pixel(x, y, image::Rgba([60, 120 + i as u8 * 40, 50, 255])); } }
            img.save(kit.join("reeds").join(name)).unwrap();
        }
        // A lamp: light flame on top, dark post below.
        let mut lamp = image::RgbaImage::new(8, 16);
        for y in 0..16 { for x in 0..8 { lamp.put_pixel(x, y, if y < 6 { image::Rgba([255, 230, 170, 255]) } else { image::Rgba([60, 40, 30, 255]) }); } }
        lamp.save(kit.join("lamp.png")).unwrap();
        std::fs::write(kit.join("kit.json"), r#"{"name": "Bog", "props": {
            "lamp": {"description": "lamp", "sprite": {"path": "lamp.png"}, "height": 2, "glow": 1.5, "glow_from": 0.6},
            "reeds": {"description": "reeds", "sprite": {"pool": ["reeds"]}, "height": 1.4},
            "untrimmed": {"description": "reeds", "sprite": {"path": "reeds/a.png"}, "trim": false},
            "missing": {"description": "nothing", "sprite": {"path": "nope.png"}, "kind": "Bush"}
        }}"#).unwrap();
        let mut s = crate::review::kit_backdrop();
        // A scene two folders down still finds kits/ by looking upward.
        let deep = root.join("scenes/night");
        std::fs::create_dir_all(&deep).unwrap();
        assert!(PropDefs::default().find(&s, Some(&deep), "kits/bog#reeds").is_ok());
        s.props = vec![layer("kits/bog#reeds"), layer("kits/bog#untrimmed"), layer("kits/bog#missing"), layer("kits/bog#gone"), layer("local")];
        let (mut defs, mut sprites) = (PropDefs::default(), SpriteCache::default());
        let base = Some(root.as_path());
        let (r, w) = defs.resolve(&s, base, &s.props[0], &mut sprites);
        assert!(w.is_none(), "{w:?}");
        let r = r.unwrap();
        assert_eq!(r.sprites.len(), 2, "a folder in the pool stands for its PNGs");
        assert!((r.sprites[0].aspect - 0.5).abs() < 1e-4, "trimmed to the 10x20 block: {}", r.sprites[0].aspect);
        assert_eq!(r.height, 1.4);
        let r = defs.resolve(&s, base, &s.props[1], &mut sprites).0.unwrap();
        assert!((r.sprites[0].aspect - 1.0).abs() < 1e-4, "trim: false keeps the margins");
        // Only the flame glows, by `glow`, and not the whole prop.
        let mut t = s.clone();
        t.props = vec![layer("kits/bog#lamp")];
        let r = defs.resolve(&t, base, &t.props[0], &mut sprites).0.unwrap();
        assert_eq!(r.glow, 0.0);
        let g = r.sprites[0].glow.as_ref().expect("a glow layer from the light texels");
        assert!((g.sample(0.5, 0.1, 1000.0, true)[0] - 1.5).abs() < 1e-3);
        assert!(g.sample(0.5, 0.8, 1000.0, true)[0] < 1e-3);
        let warns = def_warnings(&s, base);
        assert_eq!(warns.len(), 3, "{warns:?}");
        assert!(warns[0].contains("props[2]") && warns[0].contains("nope.png") && warns[0].contains("Bush"), "{}", warns[0]);
        assert!(warns[1].contains("no prop 'gone'") && warns[1].contains("missing") && warns[1].contains("draws nothing"), "{}", warns[1]);
        assert!(defs.resolve(&s, base, &s.props[3], &mut sprites).0.is_none());
        assert!(warns[2].contains("no prop definition 'local'"), "{}", warns[2]);
        // The same layers render (falling back where they must) and the scene still loops.
        let mut r = WorldRenderer::default();
        let rep = crate::review::check_loop(&mut r, &s, &RenderOptions { base_dir: Some(root.clone()), ..RenderOptions::default() }, 6);
        assert!(rep.exact < 1e-6);
        let _ = std::fs::remove_dir_all(&root);
    }
}
