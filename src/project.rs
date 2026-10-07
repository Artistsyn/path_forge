//! Project folders: where PathForge keeps work by default, which files a scene uses, and packing a
//! scene with its files into one folder of relative paths that can be moved, shared or committed
//! with a game.
//!
//! A project folder looks like:
//!
//! ```text
//! MyGame/
//!   scenes/   scene files (.json)
//!   assets/   images and colour grades the scenes use
//!   kits/     prop kits (a folder each, with kit.json)
//!   exports/  what pf_export and the studio write
//! ```
//!
//! Scenes name their files relative to themselves; kits are also found in any folder above a scene.

use crate::scene::Scene;
use serde::Serialize;
use serde_json::Value;
use std::path::{Component, Path, PathBuf};

/// The folders a new project gets.
pub const FOLDERS: [&str; 4] = ["scenes", "assets", "kits", "exports"];

/// Where PathForge works when nothing says otherwise: `$PATH_FORGE_HOME`, else a PathForge folder in
/// the user's Documents (or home) folder. Never the folder the program happened to start in.
pub fn default_home() -> PathBuf {
    home_from(
        std::env::var_os("PATH_FORGE_HOME").filter(|h| !h.is_empty()).map(PathBuf::from),
        std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from),
        std::env::var_os("XDG_DOCUMENTS_DIR").map(PathBuf::from),
        std::env::current_dir().unwrap_or_default(),
    )
}

fn home_from(set: Option<PathBuf>, user: Option<PathBuf>, xdg_docs: Option<PathBuf>, cwd: PathBuf) -> PathBuf {
    if let Some(h) = set { return normal(&if h.is_absolute() { h } else { cwd.join(h) }); }
    match user {
        Some(u) => {
            let docs = xdg_docs.filter(|d| d.is_dir()).unwrap_or_else(|| u.join("Documents"));
            if docs.is_dir() { docs.join("PathForge") } else { u.join("PathForge") }
        }
        None => cwd.join("PathForge"),
    }
}

/// A path with `.` and `..` worked out, without touching the disk.
pub fn normal(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => { if !out.pop() { out.push(".."); } }
            c => out.push(c),
        }
    }
    out
}

/// `p` relative to `base` when it is inside it.
pub fn relative_inside(p: &Path, base: &Path) -> Option<PathBuf> {
    normal(p).strip_prefix(normal(base)).ok().map(Path::to_path_buf)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum AssetKind { Image, ImageFolder, Grade, Kit }

/// One file (or folder) a scene names.
#[derive(Clone, Debug, Serialize)]
pub struct Asset {
    /// JSON pointer of the string that names it.
    pub pointer: String,
    /// As written in the scene (for a kit, the part before `#`).
    pub written: String,
    pub kind: AssetKind,
    pub path: PathBuf,
    pub exists: bool,
}

fn resolve(base: &Path, p: &str) -> PathBuf {
    let p = Path::new(p.trim());
    normal(&if p.is_absolute() { p.to_path_buf() } else { base.join(p) })
}

fn sprite_assets(v: &Value, at: &str, base: &Path, out: &mut Vec<Asset>) {
    let Some(sp) = v.get("sprite") else { return };
    let mut push = |ptr: String, s: &str| {
        if s.trim().is_empty() { return; }
        let path = resolve(base, s);
        let kind = if path.is_dir() { AssetKind::ImageFolder } else { AssetKind::Image };
        out.push(Asset { pointer: ptr, written: s.to_owned(), kind, exists: path.exists(), path });
    };
    if let Some(s) = sp.get("path").and_then(|s| s.as_str()) { push(format!("{at}/sprite/path"), s); }
    if let Some(pool) = sp.get("pool").and_then(|p| p.as_array()) {
        for (j, s) in pool.iter().enumerate() { if let Some(s) = s.as_str() { push(format!("{at}/sprite/pool/{j}"), s); } }
    }
}

/// Every file and folder the scene names, resolved from `base` (the scene's folder). Built-in
/// grades and kits are not files and are left out.
pub fn assets(scene: &Scene, base: &Path) -> Vec<Asset> {
    let v = serde_json::to_value(scene).unwrap_or(Value::Null);
    let mut out = Vec::new();
    for list in ["props", "fixtures", "set_pieces"] {
        if let Some(a) = v.get(list).and_then(|a| a.as_array()) {
            for (i, item) in a.iter().enumerate() { sprite_assets(item, &format!("/{list}/{i}"), base, &mut out); }
        }
    }
    if let Some(defs) = v.get("prop_defs").and_then(|d| d.as_object()) {
        for (name, d) in defs { sprite_assets(d, &format!("/prop_defs/{}", name.replace('~', "~0").replace('/', "~1")), base, &mut out); }
    }
    for (i, p) in scene.props.iter().enumerate() {
        let Some((kit, _)) = p.def.split_once('#') else { continue };
        if kit.trim().is_empty() || kit.trim().starts_with('@') { continue; }
        let path = normal(&crate::world::propdefs::find_kit(Some(base), kit));
        let exists = crate::world::propdefs::kit_file(&path).is_file();
        out.push(Asset { pointer: format!("/props/{i}/def"), written: kit.to_owned(), kind: AssetKind::Kit, path, exists });
    }
    let g = scene.style.grade.trim();
    if g.to_lowercase().ends_with(".cube") {
        let path = resolve(base, g);
        out.push(Asset { pointer: "/style/grade".into(), written: g.to_owned(), kind: AssetKind::Grade, exists: path.is_file(), path });
    }
    out
}

/// A sentence for every file the scene names that is not there.
pub fn asset_warnings(scene: &Scene, base: &Path) -> Vec<String> {
    assets(scene, base).into_iter().filter(|a| !a.exists).map(|a| {
        let what = match a.kind { AssetKind::Kit => "kit (no kit.json)", AssetKind::Grade => "colour grade", _ => "image" };
        format!("{}: {what} '{}' is missing (looked for {})", a.pointer, a.written, a.path.display())
    }).collect()
}

#[derive(Debug, Default, Serialize)]
pub struct PackReport {
    pub scene: PathBuf,
    /// Files and folders copied in, as (from, to).
    pub copied: Vec<(PathBuf, PathBuf)>,
    /// Paths in the scene that were rewritten, as (pointer, old, new).
    pub rewritten: Vec<(String, String, String)>,
    pub missing: Vec<String>,
}

fn copy_into(from: &Path, to: &Path) -> Result<(), String> {
    if normal(from) == normal(to) { return Ok(()); }
    if from.is_dir() {
        std::fs::create_dir_all(to).map_err(|e| format!("{}: {e}", to.display()))?;
        for e in std::fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))? {
            let e = e.map_err(|e| e.to_string())?;
            copy_into(&e.path(), &to.join(e.file_name()))?;
        }
        Ok(())
    } else {
        if let Some(d) = to.parent() { std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?; }
        std::fs::copy(from, to).map(|_| ()).map_err(|e| format!("{} -> {}: {e}", from.display(), to.display()))
    }
}

/// A name under `dir` not yet taken (by another file than `from`): stem, stem_2, stem_3...
fn free_name(dir: &Path, from: &Path, taken: &mut Vec<PathBuf>) -> PathBuf {
    let stem = from.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "asset".into());
    let ext = from.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    for n in 1.. {
        let name = if n == 1 { format!("{stem}{ext}") } else { format!("{stem}_{n}{ext}") };
        let c = dir.join(name);
        if !taken.contains(&c) && (!c.exists() || same_file(&c, from)) { taken.push(c.clone()); return c; }
    }
    unreachable!()
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::read(a), std::fs::read(b)) { (Ok(x), Ok(y)) => x == y, _ => false }
}

fn set_pointer(v: &mut Value, ptr: &str, new: String) {
    if let Some(slot) = v.pointer_mut(ptr) { *slot = Value::String(new); }
}

fn slash(p: &Path) -> String { p.to_string_lossy().replace('\\', "/") }

/// Write `scene` (whose files are named relative to `base`) into `out` as `<name>.json`, with
/// every file it uses copied in and named relative to it. Files already inside `out` stay where
/// they are; images and grades from elsewhere go to `assets/`, kits to `kits/<folder name>`. With
/// `out` = `base`, this gathers a scene's outside files into its own folder.
pub fn pack(scene: &Scene, base: &Path, out: &Path, name: &str) -> Result<PackReport, String> {
    let out = normal(out);
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let mut v = serde_json::to_value(scene).map_err(|e| e.to_string())?;
    let mut rep = PackReport::default();
    let mut taken = Vec::new();
    // One destination per source, so a file used twice is copied once.
    let mut done: Vec<(PathBuf, String)> = Vec::new();
    for a in assets(scene, base) {
        if !a.exists { rep.missing.push(format!("{} '{}'", a.pointer, a.written)); continue; }
        let new_rel = if let Some((_, rel)) = done.iter().find(|(p, _)| *p == a.path) { rel.clone() } else {
            // Inside the source folder: keep the same place relative to the scene.
            let rel = match relative_inside(&a.path, base) {
                Some(r) => r,
                None => match a.kind {
                    AssetKind::Kit => PathBuf::from("kits").join(a.path.file_name().unwrap_or_default()),
                    _ => free_name(&out.join("assets"), &a.path, &mut taken).strip_prefix(&out).map(Path::to_path_buf).unwrap_or_default(),
                },
            };
            copy_into(&a.path, &out.join(&rel))?;
            if normal(&a.path) != normal(&out.join(&rel)) { rep.copied.push((a.path.clone(), out.join(&rel))); }
            let rel = slash(&rel);
            done.push((a.path.clone(), rel.clone()));
            rel
        };
        let old = v.pointer(&a.pointer).and_then(|x| x.as_str()).unwrap_or("").to_owned();
        let new = if a.kind == AssetKind::Kit { format!("{new_rel}#{}", old.split_once('#').map(|x| x.1).unwrap_or("")) } else { new_rel };
        if new != old { set_pointer(&mut v, &a.pointer, new.clone()); rep.rewritten.push((a.pointer.clone(), old, new)); }
    }
    let packed: Scene = serde_json::from_value(v).map_err(|e| e.to_string())?;
    let file = out.join(format!("{name}.json"));
    crate::scene::save_file(&file, &packed)?;
    rep.scene = file;
    Ok(rep)
}

/// A file picked for a scene, named so the scene stays movable: relative when it is inside the
/// scene's folder, else copied into its `assets/` folder first. With no folder yet (an unsaved
/// scene), the absolute path; `pack` gathers it later.
pub fn adopt_file(picked: &Path, base: Option<&Path>) -> Result<String, String> {
    let Some(base) = base else { return Ok(picked.to_string_lossy().into_owned()) };
    if let Some(rel) = relative_inside(picked, base) { return Ok(slash(&rel)); }
    let mut taken = Vec::new();
    let dest = free_name(&base.join("assets"), picked, &mut taken);
    copy_into(picked, &dest)?;
    Ok(slash(dest.strip_prefix(base).unwrap_or(&dest)))
}

/// Make a project folder (scenes, assets, kits, exports) with one scene to start from.
pub fn new_project(dir: &Path, preset: &str) -> Result<PathBuf, String> {
    for f in FOLDERS { std::fs::create_dir_all(dir.join(f)).map_err(|e| format!("{}: {e}", dir.join(f).display()))?; }
    let mut s = crate::scene::presets::ALL.iter().find(|p| p.0.eq_ignore_ascii_case(preset)).map(|p| (p.1)())
        .ok_or_else(|| format!("no preset '{preset}'"))?;
    let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "scene".into());
    s.name = name.clone();
    let file = dir.join("scenes").join(format!("{}.json", name.to_lowercase().replace(' ', "_")));
    if !file.exists() { crate::scene::save_file(&file, &s)?; }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{PropDef, PropLayer, Side, SpriteRef};
    use crate::world::{RenderOptions, WorldRenderer};

    #[test]
    fn work_goes_to_a_user_folder_unless_told_otherwise() {
        let tmp = std::env::temp_dir().join(format!("pf_home_{}", std::process::id()));
        std::fs::create_dir_all(tmp.join("Documents")).unwrap();
        let cwd = PathBuf::from("/");
        assert_eq!(home_from(None, Some(tmp.clone()), None, cwd.clone()), tmp.join("Documents/PathForge"));
        assert_eq!(home_from(None, Some(tmp.join("nodocs")), None, cwd.clone()), tmp.join("nodocs/PathForge"));
        assert_eq!(home_from(Some("games/bg".into()), Some(tmp.clone()), None, PathBuf::from("/work")), PathBuf::from("/work/games/bg"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    fn png(path: &Path, rgb: [u8; 3]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut img = image::RgbaImage::new(12, 24);
        for y in 2..22 { for x in 3..9 { img.put_pixel(x, y, image::Rgba([rgb[0], rgb[1], rgb[2], 255])); } }
        img.save(path).unwrap();
    }

    #[test]
    fn a_packed_scene_needs_nothing_outside_its_folder() {
        let root = std::env::temp_dir().join(format!("pf_pack_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (home, elsewhere) = (root.join("home"), root.join("elsewhere"));
        // A kit two folders up from the scene, an image named by an absolute path, a definition
        // reaching outside the scene folder, a colour grade, and one file that is not there.
        png(&home.join("kits/swamp/reeds/a.png"), [70, 140, 60]);
        std::fs::write(home.join("kits/swamp/kit.json"), r#"{"name": "Swamp", "props": {"reeds": {"sprite": {"pool": ["reeds"]}, "height": 1.4}}}"#).unwrap();
        png(&elsewhere.join("tree.png"), [40, 90, 30]);
        png(&home.join("shared/rock.png"), [120, 110, 100]);
        let mut cube = String::from("LUT_3D_SIZE 2\n");
        for b in 0..2 { for g in 0..2 { for r in 0..2 { cube += &format!("{} {} {}\n", r as f32 * 0.9, g as f32, b as f32 * 0.8); } } }
        std::fs::write(elsewhere.join("cool.cube"), cube).unwrap();
        let scenes = home.join("scenes/night");
        std::fs::create_dir_all(&scenes).unwrap();
        let mut s = crate::review::kit_backdrop();
        let at = |d: &str, off: f32| PropLayer { def: d.into(), side: Side::Center, lateral: 0.5, spacing: 24.0, offset: off, jitter: 0.0, scale_var: 0.0, ..PropLayer::default() };
        s.props = vec![
            at("kits/swamp#reeds", 4.0),
            PropLayer { sprite: SpriteRef { path: elsewhere.join("tree.png").to_string_lossy().into(), ..SpriteRef::default() }, ..at("", 6.0) },
            at("rock", 8.0),
            PropLayer { sprite: SpriteRef { path: "nope.png".into(), ..SpriteRef::default() }, ..at("", 10.0) },
        ];
        s.prop_defs.insert("rock".into(), PropDef { sprite: SpriteRef { path: "../../shared/rock.png".into(), ..SpriteRef::default() }, height: 0.8, ..PropDef::default() });
        s.style.grade = elsewhere.join("cool.cube").to_string_lossy().into();
        s.style.grade_strength = 1.0;
        let warns = asset_warnings(&s, &scenes);
        assert_eq!(warns.len(), 1, "{warns:?}");
        assert!(warns[0].contains("/props/3/sprite/path") && warns[0].contains("nope.png"));

        let mut r = WorldRenderer::default();
        let before = r.render(&s, 0.0, &RenderOptions { base_dir: Some(scenes.clone()), ..RenderOptions::default() });
        // The comparison below only means something if the grade and the images take part.
        let mut plain = s.clone();
        plain.style.grade.clear();
        assert!(r.render(&plain, 0.0, &RenderOptions { base_dir: Some(scenes.clone()), ..RenderOptions::default() }).rgba != before.rgba, "the grade did not load");
        let mut bare = s.clone();
        bare.props.truncate(0);
        let mut stats = |sc: &Scene| r.render(sc, 0.0, &RenderOptions { base_dir: Some(scenes.clone()), stats: true, ..RenderOptions::default() }).stats.unwrap();
        let (with, without) = (stats(&s), stats(&bare));
        assert!(with.coverage.get("props").copied().unwrap_or(0.0) > without.coverage.get("props").copied().unwrap_or(0.0) + 0.002, "the images did not draw");
        let out = root.join("packed");
        let rep = pack(&s, &scenes, &out, "night").unwrap();
        assert_eq!(rep.missing.len(), 1);
        for f in ["night.json", "kits/swamp/kit.json", "kits/swamp/reeds/a.png", "assets/tree.png", "assets/rock.png", "assets/cool.cube"] {
            assert!(out.join(f).is_file(), "{f} missing from the pack: {:?}", rep.copied);
        }
        // Take away everything the original used: the pack still renders the same frame.
        std::fs::rename(&home, root.join("home_gone")).unwrap();
        std::fs::rename(&elsewhere, root.join("elsewhere_gone")).unwrap();
        let (packed, _) = crate::scene::load_file(&out.join("night.json")).unwrap();
        assert_eq!(packed.props[1].sprite.path, "assets/tree.png");
        assert_eq!(packed.props[0].def, "kits/swamp#reeds");
        assert_eq!(packed.style.grade, "assets/cool.cube");
        let after = WorldRenderer::default().render(&packed, 0.0, &RenderOptions { base_dir: Some(out.clone()), ..RenderOptions::default() });
        assert!(before.rgba == after.rgba, "the packed scene renders differently");
        assert_eq!(asset_warnings(&packed, &out).len(), 1, "only the file that was never there");
        // A file picked from elsewhere is copied in; one inside is named relative.
        let pick = root.join("elsewhere_gone/tree.png");
        assert_eq!(adopt_file(&pick, Some(&out)).unwrap(), "assets/tree.png", "same content: no second copy");
        png(&root.join("elsewhere_gone/tree2/tree.png"), [1, 2, 3]);
        assert_eq!(adopt_file(&root.join("elsewhere_gone/tree2/tree.png"), Some(&out)).unwrap(), "assets/tree_2.png");
        assert_eq!(adopt_file(&out.join("kits/swamp/reeds/a.png"), Some(&out)).unwrap(), "kits/swamp/reeds/a.png");
        // Packing again in place changes nothing.
        let again = pack(&packed, &out, &out, "night").unwrap();
        assert!(again.copied.is_empty() && again.rewritten.is_empty(), "{:?} {:?}", again.copied, again.rewritten);
        let _ = std::fs::remove_dir_all(&root);
    }
}
