//! PathForge MCP server: author, preview, check and export endless-path scenes over JSON-RPC on stdio.
//!
//! Scenes are JSON files (schema v3). Every tool that reads a scene takes exactly one of `path`
//! (a scene file), `preset` (a built-in scene) or `scene` (an inline scene object). Edits go
//! through `pf_edit_scene`, which validates the result, rejects field names the schema does not
//! know, writes atomically, and can refuse to overwrite a file changed since it was read.
//!
//! Both stdio framings are accepted: newline-delimited JSON (Claude Code) and Content-Length
//! headers (LSP style); replies use the framing the client used.

use crate::skill;
use crate::export::{self, CameraInfo, ExportJob, Format};
use crate::review;
use crate::scene::{presets, Scene, SCENE_VERSION};
use crate::world::palette::NAMED;
use crate::world::{Image, Layers, RenderOptions, WorldRenderer};
use base64::Engine as _;
use serde_json::{json, Map, Value};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Instant;

pub const INSTRUCTIONS: &str = "PathForge renders endless, seamlessly looping first-person path backgrounds for 2D games (the walking road of a Path of Kings style runner): a path receding to the horizon with walls or verges, props, torches and other light fixtures, sky, particles, post effects and pixel-art styles.

World model, in metres: x is sideways from the path centre (right positive), y is height above the ground, d is distance ahead. The camera walks forward at motion.speed m/s; one loop covers motion.loop_length metres and every repeating thing is snapped so the loop is seamless.

Workflow:
1. pf_presets to pick a starting point, then pf_new_scene to copy it to a file (relative paths resolve under the PathForge home folder shown in every result).
2. pf_edit_scene with a JSON merge patch and/or pointer ops; it returns a preview image. pf_schema shows every field with units and ranges; pf_get_scene reads values.
3. Look before you claim: pf_render (one frame), pf_contact_sheet (frames across the loop), pf_analyze (coverage, brightness and warnings), pf_check_loop (seam at the wrap).
4. pf_camera maps metres to screen pixels and back, for placing enemies, pickups or UI on the path in the game.
5. pf_export writes the loop (clip: loop) or a stop-for-a-fight set (clip: encounter - stop / idle / go clips that join the loop exactly) as animated WebP / GIF / APNG / PNG frames / sprite sheet plus a metadata JSON a game uses to play the loop by distance walked. Prefer webp for full-colour scenes: at quality 90 it is about 1/3 the size of the GIF with less error (no 256-colour limit); quality 80 is about 1/5 and still close to the render. For pixel-art scenes (a style.palette) webp is written lossless: exact, and about 1/3 the size of the GIF.

The full design guide (brief, critique checklist, export choices, pitfalls) is the prompt `pathforge` and the resource pathforge://skill/SKILL.md; read it before a first scene.

Read-only tools never write files. Images are downscaled by `scale` (default 0.5) to keep results small; pass scale 1 to see full resolution.";

/// Folder that relative scene and export paths resolve against.
/// Where relative paths resolve: `$PATH_FORGE_HOME`, else Documents/PathForge (see
/// `project::default_home`).
pub fn home() -> PathBuf { crate::project::default_home() }

pub struct Server {
    renderer: WorldRenderer,
    home: PathBuf,
}

/// What a tool returns: text blocks and images, or an error message.
pub struct Reply {
    content: Vec<Value>,
    is_error: bool,
}

impl Reply {
    fn text(t: impl Into<String>) -> Reply { Reply { content: vec![json!({"type": "text", "text": t.into()})], is_error: false } }
    fn json(v: &Value) -> Reply {
        let mut v = v.clone();
        tidy(&mut v);
        Reply::text(serde_json::to_string_pretty(&v).unwrap_or_default())
    }
    fn error(t: impl Into<String>) -> Reply { Reply { content: vec![json!({"type": "text", "text": t.into()})], is_error: true } }
    fn with_image(mut self, img: &Image) -> Reply {
        match review::png_bytes(img) {
            Ok(png) => self.content.push(json!({"type": "image", "mimeType": "image/png", "data": base64::engine::general_purpose::STANDARD.encode(png)})),
            Err(e) => self.content.push(json!({"type": "text", "text": format!("(preview could not be encoded: {e})")})),
        }
        self
    }
    pub fn to_value(&self) -> Value { json!({"content": self.content, "isError": self.is_error}) }
}

type ToolResult = Result<Reply, String>;

/// A scene as a JSON value with its f32 fields written the short way (0.1, not 0.10000000149011612).
fn scene_value(scene: &Scene) -> Result<Value, String> {
    let text = serde_json::to_string(scene).map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

/// Numbers that are exactly an f32 are written the short way, so 2.1 does not read as 2.0999999046325684.
fn tidy(v: &mut Value) {
    match v {
        Value::Number(n) if n.is_f64() => {
            let x = n.as_f64().unwrap_or(0.0);
            let f = x as f32;
            if f as f64 == x && f.is_finite() {
                if let Ok(short) = format!("{f}").parse::<f64>() { if let Some(m) = serde_json::Number::from_f64(short) { *n = m; } }
            }
        }
        Value::Array(a) => a.iter_mut().for_each(tidy),
        Value::Object(o) => o.values_mut().for_each(tidy),
        _ => {}
    }
}

/// Any serializable value with f32 noise removed, as above.
fn clean<T: serde::Serialize>(v: &T) -> Value {
    serde_json::to_string(v).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null)
}

// ── Tool list ──────────────────────────────────────────────────────────────

fn source_props() -> Value {
    json!({
        "path": {"type": "string", "description": "Scene file (.json). Relative paths resolve under the PathForge home folder."},
        "preset": {"type": "string", "description": "Built-in scene name from pf_presets (case-insensitive)."},
        "scene": {"type": "object", "description": "Inline scene object (v3). Missing fields take their defaults."}
    })
}

fn with_source(mut extra: Value) -> Value {
    let mut props = source_props();
    if let (Some(p), Some(e)) = (props.as_object_mut(), extra.get_mut("properties").and_then(|v| v.as_object_mut())) {
        for (k, v) in std::mem::take(e) { p.insert(k, v); }
    }
    let mut schema = json!({"type": "object", "properties": props});
    if let Some(req) = extra.get("required") { schema["required"] = req.clone(); }
    schema
}

fn tool(name: &str, title: &str, description: &str, schema: Value, read_only: bool) -> Value {
    let mut ann = json!({"title": title, "readOnlyHint": read_only});
    if !read_only { ann["destructiveHint"] = json!(false); ann["idempotentHint"] = json!(true); }
    json!({"name": name, "description": description, "inputSchema": schema, "annotations": ann})
}

/// The names of every tool, in listing order.
pub fn tool_names() -> Vec<String> {
    tools().iter().filter_map(|t| t.get("name").and_then(|n| n.as_str()).map(String::from)).collect()
}

pub fn tools() -> Vec<Value> {
    let scale = json!({"type": "number", "description": "Preview size as a fraction of the canvas (0.05..1). Default 0.5."});
    let hide = json!({"type": "array", "items": {"type": "string", "enum": ["sky", "ground", "walls", "props", "fixtures", "particles", "post"]}, "description": "Layers to leave out, e.g. [\"props\"] to see the bare path."});
    vec![
        tool("pf_presets", "List presets", "Built-in scenes with a one-line summary of each (setting, lights, props, sky). Start new scenes from one of these.", json!({"type": "object", "properties": {}}), true),
        tool("pf_schema", "Scene schema", "JSON Schema of the scene format, with units and meanings from the field docs. Pass `section` (e.g. \"Fixture\", \"PropLayer\", \"SetPiece\", \"Style\", \"Camera\") for one type only; without it, a list of sections and the top-level fields.",
            json!({"type": "object", "properties": {"section": {"type": "string", "description": "Type name to show, e.g. Fixture, PropLayer, Particles, Material, Post, Style, Motion."}}}), true),
        tool("pf_palettes", "Palettes", "Named pixel-art palettes (use in style.palette as {\"Named\": \"<name>\"}) with their colours.", json!({"type": "object", "properties": {}}), true),
        tool("pf_styles", "Style presets", "Named looks (pixel art, Game Boy, painted storybook, toon, CRT, noir...) that set how a scene is drawn without touching its world. Apply one with pf_edit_scene's style_preset. With path, preset or scene, also returns that scene rendered in every style, side by side in the listed order.",
            with_source(json!({"properties": {"t": {"type": "number"}, "scale": {"type": "number", "description": "Size of each image (default 0.25)."}}})), true),
        tool("pf_new_scene", "New scene file", "Copy a preset (or an inline scene) into a new scene file and return a preview. Refuses to overwrite an existing file unless overwrite is true.",
            json!({"type": "object", "properties": {
                "path": {"type": "string", "description": "File to create (.json). Relative paths resolve under the PathForge home folder."},
                "preset": {"type": "string", "description": "Preset to start from (default: Stone Dungeon)."},
                "scene": {"type": "object", "description": "Inline scene to write instead of a preset."},
                "name": {"type": "string", "description": "Scene name (default: the file name)."},
                "overwrite": {"type": "boolean"},
                "preview": {"type": "boolean", "description": "Return a preview image (default true)."},
                "scale": scale.clone()
            }, "required": ["path"]}), false),
        tool("pf_get_scene", "Read scene", "The scene as JSON, or one part of it by JSON pointer (e.g. /fixtures/0 or /post). Also returns the file's revision for pf_edit_scene's `revision` check.",
            with_source(json!({"properties": {"pointer": {"type": "string", "description": "JSON pointer into the scene, e.g. /props/1/spacing. Default: the whole scene."}}})), true),
        tool("pf_edit_scene", "Edit scene", "Change a scene file. `merge` is a JSON merge patch (objects merge, null deletes a key, arrays replace whole); `ops` are pointer edits applied after it: {op: set|remove|append|insert, pointer, value}. 'append' adds to the array at pointer; 'insert' puts value before the index named by the pointer. The result is validated (unknown field names and bad enum values are rejected and nothing is written), saved atomically, and returned with the list of changed fields, warnings and a preview. Pass `revision` from pf_get_scene to refuse the write if the file changed since.",
            json!({"type": "object", "properties": {
                "path": {"type": "string", "description": "Scene file to edit."},
                "merge": {"type": "object", "description": "JSON merge patch (RFC 7396) applied to the scene."},
                "ops": {"type": "array", "items": {"type": "object", "properties": {
                    "op": {"type": "string", "enum": ["set", "remove", "append", "insert"]},
                    "pointer": {"type": "string", "description": "JSON pointer, e.g. /fixtures/0/intensity or /props"},
                    "value": {"description": "New value for set/append/insert."}
                }, "required": ["op", "pointer"]}},
                "style_preset": {"type": "string", "description": "Name from pf_styles, applied before merge and ops (it replaces style and post settings and light.bands)."},
                "revision": {"type": "string", "description": "Revision from pf_get_scene; the edit is refused if the file has changed since."},
                "dry_run": {"type": "boolean", "description": "Validate and preview without writing."},
                "preview": {"type": "boolean", "description": "Return a preview image (default true)."},
                "t": {"type": "number", "description": "Loop position of the preview, 0..1."},
                "scale": scale.clone()
            }, "required": ["path"]}), false),
        tool("pf_render", "Render frame", "Render one frame and return it as an image, with what the frame shows (coverage and brightness of path, walls, props, fixtures, sky...).",
            with_source(json!({"properties": {
                "t": {"type": "number", "description": "Position in the loop, 0..1 (default 0)."},
                "distance": {"type": "number", "description": "Position in metres instead of t."},
                "scale": scale.clone(), "hide": hide.clone()
            }})), true),
        tool("pf_contact_sheet", "Contact sheet", "Several frames spread evenly across the loop in one image: shows how props, lights and particles move and repeat.",
            with_source(json!({"properties": {
                "frames": {"type": "integer", "description": "Frames to show (2..24, default 6)."},
                "columns": {"type": "integer", "description": "Grid columns (default: frames, up to 6)."},
                "scale": {"type": "number", "description": "Size of each frame (default 0.3)."},
                "hide": hide.clone()
            }})), true),
        tool("pf_compare", "Compare scenes", "First frame of several scenes side by side (files and/or presets), labelled in order: for choosing between variations.",
            json!({"type": "object", "properties": {
                "paths": {"type": "array", "items": {"type": "string"}},
                "presets": {"type": "array", "items": {"type": "string"}, "description": "Preset names; [\"*\"] for all of them."},
                "t": {"type": "number"}, "scale": {"type": "number", "description": "Default 0.3."}, "columns": {"type": "integer"}
            }}), true),
        tool("pf_analyze", "Analyze scene", "Measure a scene without looking at it: coverage and mean brightness per kind of surface over several frames, light and billboard counts, render time, and warnings about things that usually look wrong (too dark, blown out, props or lights not visible, path not reading against the verge, props standing in the lights, values out of range).",
            with_source(json!({"properties": {"frames": {"type": "integer", "description": "Frames sampled across the loop (1..12, default 4)."}}})), true),
        tool("pf_check_loop", "Check loop", "Render across the loop and measure the wrap: whether the frame at the loop length matches the first frame exactly, and whether the step into the loop point is larger than a normal step (a pop).",
            with_source(json!({"properties": {"frames": {"type": "integer", "description": "Frames per loop to compare (4..96, default 24)."}, "scale": {"type": "number", "description": "Render scale for the check (default 0.5)."}}})), true),
        tool("pf_camera", "Camera mapping", "How the scene's camera maps the world to the screen, for placing game objects on the path: projection formulas and constants, `points` [{x, y, d}] in metres -> screen pixels, and `rows` (screen y) -> the ground distance and path edges seen on that row. Pixel coordinates are for the canvas size (or `size`).",
            with_source(json!({"properties": {
                "points": {"type": "array", "items": {"type": "object", "properties": {"x": {"type": "number"}, "y": {"type": "number"}, "d": {"type": "number"}}, "required": ["d"]}},
                "rows": {"type": "array", "items": {"type": "number"}, "description": "Screen rows (pixels from the top) to map back to the ground."},
                "size": {"type": "array", "items": {"type": "integer"}, "description": "[width, height] if the game shows the loop at a different size."}
            }})), true),
        tool("pf_export", "Export", "Render the whole loop and write it out: gif, webp (animated, much smaller), apng, png (a folder of frames), sheet (sprite sheet + TexturePacker/Aseprite-style atlas). Always also writes <name>.json with timing, metres per frame, the camera mapping and the scene.",
            with_source(json!({"properties": {
                "out_dir": {"type": "string", "description": "Output folder (relative paths resolve under the PathForge home folder). Default: exports/."},
                "formats": {"type": "array", "items": {"type": "string", "enum": ["gif", "webp", "apng", "png", "sheet", "depth", "layers"]}, "description": "Default [\"gif\"]. webp is far smaller than gif at full colour. depth: 16-bit depth PNG per frame, so the game can hide enemies behind props exactly. layers: RGBA PNG frames per depth band (near/mid/far), to draw characters between them."},
                "clip": {"type": "string", "enum": ["loop", "encounter", "transition"], "description": "transition: a one-shot walk from this scene into another (to_path or to_preset): the new world appears down the road and sweeps toward the camera; starts on this loop's frame 0 and ends on the other loop's frame 0. loop (default): the endless walk. encounter: a stop for a fight in three parts - <name>_stop (eases to a halt), <name>_idle (stands still while flames and particles keep moving; loop it for the fight), <name>_go (eases back and hands over to the loop exactly) - plus <name>_encounter.json with playback rules."},
                "to_path": {"type": "string", "description": "transition: the scene file to walk into."},
                "to_preset": {"type": "string", "description": "transition: the preset to walk into."},
                "approach_m": {"type": "number", "description": "transition: how far ahead the new world first appears (default 30 m)."},
                "slow_m": {"type": "number", "description": "encounter: metres covered while slowing down and again while speeding up (default 3)."},
                "start_frame": {"type": "integer", "description": "encounter: loop frame where the stop begins and the walk resumes (default 0)."},
                "layer_bands": {"type": "array", "items": {"type": "number"}, "description": "Band boundaries in metres for layers, nearest first. Default [4, 12]."},
                "sheet_max": {"type": "integer", "description": "Largest sprite sheet page in pixels (default 4096, a common phone texture limit); longer loops split into pages."},
                "webp_quality": {"type": "number", "description": "Animated WebP quality 0..100; 100 = lossless. Default: lossless for palette (pixel-art) scenes, where it is exact and about 1/3 the size of the GIF; 90 for full colour (80 for mobile builds, about half the size)."},
                "name": {"type": "string", "description": "File name stem (default: scene name)."},
                "frames": {"type": "integer", "description": "Frames in the loop (default: loop seconds x motion.fps)."},
                "size": {"type": "array", "items": {"type": "integer"}, "description": "[width, height] output size (default: the canvas)."},
                "gif_dither": {"type": "boolean", "description": "Ordered dither in the GIF when the scene has no palette (default true)."},
                "gif_lossy": {"type": "number", "description": "GIF size against fidelity, 0..1 (default 0.33, below what the eye notices on smooth gradients; 1 saves ~40% but shows contours): a pixel keeps the colour on screen while it is still close to the true colour. 0 = exact."}
            }})), false),
        tool("pf_kit", "Prop kit", "Props described by data instead of built-in kinds: an image, several images or folders of PNGs, or a shape drawn from parts, plus height, anchor (Ground or Ceiling), glow and a light it gives off. A kit keeps them in a folder as kit.json beside their images, for any scene's prop layer to use as {\"def\": \"<kit folder>#<name>\"} (placement still comes from the layer). Built-in kit: @wayside (lantern_post, hanging_lantern, glowcaps, signpost, fence, barrel, crate; drawn from shapes). Without `props`, shows the kit; with `props` (name -> PropDef, merged into an existing one; null removes it) creates or updates kit.json, validated before writing. pf_schema {\"section\": \"PropDef\"} has every field. Returns the props, warnings (missing images, empty folders) and each prop standing beside a path, in name order.",
            json!({"type": "object", "properties": {
                "kit": {"type": "string", "description": "Kit folder (relative to the PathForge home), or @name for a built-in kit."},
                "props": {"type": "object", "description": "name -> PropDef (or null to remove). Paths inside are relative to the kit folder."},
                "name": {"type": "string", "description": "Kit name (default: the folder name)."},
                "description": {"type": "string"},
                "from": {"type": "string", "description": "When creating a kit: start from the props of a built-in kit, e.g. @wayside."},
                "only": {"type": "string", "description": "Show only this prop."},
                "preset": {"type": "string", "description": "Scene to show the props in (default: a dusk road with nothing beside it)."},
                "preview": {"type": "boolean", "description": "Return the image (default true)."},
                "scale": {"type": "number", "description": "Size of each image (default 0.3)."},
                "columns": {"type": "integer", "description": "Images per row (default 4)."}
            }, "required": ["kit"]}), false),
        tool("pf_import_sprite", "Import sprite", "Put an image into a kit folder: from a `url` (e.g. what an image generator such as PixelLab returns), a local `path`, or `base64` PNG data. The PNG is checked and saved at `file` inside the kit; transparent margins are cut away when it is drawn (unless the prop sets trim: false), so it stands on its lowest visible pixel. With `def`, the image is added to that prop of the kit (its pool grows, so several images give variety), creating the prop when it is new (then `height` in metres is required). Returns the image size, the prop and a preview of it beside a path.",
            json!({"type": "object", "properties": {
                "kit": {"type": "string", "description": "Kit folder (relative to the PathForge home); created if missing."},
                "file": {"type": "string", "description": "Where to save inside the kit, e.g. reeds/reeds_1.png."},
                "url": {"type": "string", "description": "http(s) URL of a PNG."},
                "path": {"type": "string", "description": "Local PNG to copy (relative to the PathForge home)."},
                "base64": {"type": "string", "description": "PNG data."},
                "def": {"type": "string", "description": "Prop in the kit to add the image to (created if new)."},
                "height": {"type": "number", "description": "Height of the prop in metres at scale 1 (required for a new prop; sets it on an existing one)."},
                "description": {"type": "string", "description": "What the prop is, for a new prop."},
                "anchor": {"type": "string", "enum": ["Ground", "Ceiling"]},
                "pixelated": {"type": "boolean", "description": "Pixel art: drawn with hard pixels (default true for a new prop)."},
                "overwrite": {"type": "boolean", "description": "Replace an existing file (default false)."},
                "preview": {"type": "boolean", "description": "Return a preview (default true)."}
            }, "required": ["kit", "file"]}), false),
        tool("pf_pack", "Pack scene", "Make a scene self-contained: copy every file it uses (images, image folders, kits, .cube grades) into one folder and name them relative to the scene, so the folder can be moved, shared or committed with a game. Files already inside the folder stay where they are; images and grades from elsewhere go to assets/, kits to kits/. Without out_dir it gathers the scene's outside files into its own folder and rewrites the scene in place. Missing files are listed and left as they are.",
            json!({"type": "object", "properties": {
                "path": {"type": "string", "description": "Scene file to pack."},
                "out_dir": {"type": "string", "description": "Folder to pack into (default: the scene's own folder)."},
                "name": {"type": "string", "description": "File name of the packed scene, without .json (default: the scene file's)."},
                "in_place": {"type": "boolean", "description": "Allow rewriting the scene file itself when packing into its own folder (default true)."}
            }, "required": ["path"]}), false),
        tool("pf_remix", "Remix scene", "New variations of a scene for brainstorming: the parts you `keep` stay as they are, the others are taken whole from random presets and nudged by `jitter`. Parts: camera, path, setting (verge, walls, ceiling), sky, lighting (light, fixtures), props (props, set pieces, prop_defs), atmosphere (particles, weather), look (style, post). Without save_as, returns `count` variations side by side with their seeds (same seed, same remix); with save_as, writes the remix of `seed` to that scene file and returns its preview.",
            with_source(json!({"properties": {
                "keep": {"type": "array", "items": {"type": "string", "enum": ["camera", "path", "setting", "sky", "lighting", "props", "atmosphere", "look"]}, "description": "Parts to leave as they are. Default: camera, path, look."},
                "seed": {"type": "integer", "description": "First seed (default 1)."},
                "count": {"type": "integer", "description": "Variations to show, 1..8 (default 4)."},
                "jitter": {"type": "number", "description": "How much to nudge numbers in the parts that change, 0..0.5 (default 0.12)."},
                "save_as": {"type": "string", "description": "Scene file to write the remix of `seed` to."},
                "overwrite": {"type": "boolean"},
                "scale": {"type": "number", "description": "Size of each image (default 0.3)."}
            }})), false),
        tool("pf_transition", "Plan and preview a transition or fork", "Walk from this scene (path/preset/scene) into another and see it: the plan PathForge makes from the two scenes (what stands at the boundary: an open blend, a doorway, a cave mouth, a gate or a portal; how far ahead it shows; light through the opening; the eye's adaptation) with notes on why and warnings on what will not look right, plus a contact sheet of the walk. Give to_path or to_preset for a transition, or left_* and right_* for a fork where the player chooses (take: which branch the preview takes). Adjust with `transition` (Transition fields, e.g. {\"threshold\": \"CaveMouth\", \"approach_m\": 30}) or `fork` (ForkChoice fields). With `export` ({formats, out_dir, name, entries}) also writes the clips: a transition (entries > 1 writes clips starting at evenly spaced loop frames, so a game need not wait for frame 0), or a fork's approach plus one clip per branch, each with a .json saying how to play it.",
            with_source(json!({"properties": {
                "to_path": {"type": "string"}, "to_preset": {"type": "string"},
                "left_path": {"type": "string"}, "left_preset": {"type": "string"},
                "right_path": {"type": "string"}, "right_preset": {"type": "string"},
                "transition": {"type": "object", "description": "Transition overrides (see pf_schema section Transition)."},
                "fork": {"type": "object", "description": "ForkChoice overrides (see pf_schema section ForkChoice)."},
                "take": {"type": "string", "enum": ["left", "right"], "description": "Branch the fork preview takes (default: the fork's default branch)."},
                "frames": {"type": "integer", "description": "Frames in the contact sheet (default 8)."},
                "scale": {"type": "number", "description": "Size of each frame (default 0.25)."},
                "export": {"type": "object", "description": "{formats: [...], out_dir, name, entries}: also write the clips."}
            }})), false),
        tool("pf_journey", "Journey of scenes", "A journey maps the places a game walks through: stops (each a scene file relative to the journey, or preset:<name>) and what each leads to (Go through a transition, a Fork where the player chooses, or End). op read: the journey and its problems. op write: save `journey` (whole object) to `path`, checked first. op preview: a sheet with every stop and a frame from each transition or fork. op export: every loop, transition and fork clip plus <name>.journey.json mapping stops to files (formats, out_dir, entries). The live runtime plays the same file (Runtime::from_journey, go, choose).",
            json!({"type": "object", "properties": {
                "op": {"type": "string", "enum": ["read", "write", "preview", "export"]},
                "path": {"type": "string", "description": "Journey file (.journey.json). Relative paths resolve under the PathForge home folder."},
                "journey": {"type": "object", "description": "For write: {name, start, stops: {id: {scene, next: \"End\" | {\"Go\": {to, transition}} | {\"Fork\": {left, right, fork}}}}}."},
                "overwrite": {"type": "boolean"},
                "formats": {"type": "array", "items": {"type": "string"}}, "out_dir": {"type": "string"}, "entries": {"type": "integer"},
                "scale": {"type": "number"}
            }, "required": ["op", "path"]}), false),
    ]
}

// ── Scene sources and files ───────────────────────────────────────────────

fn find_preset(name: &str) -> Result<Scene, String> {
    let key = |s: &str| s.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase();
    presets::ALL.iter().find(|(n, _)| key(n) == key(name)).map(|(_, f)| f())
        .ok_or_else(|| format!("no preset named '{name}'. Presets: {}", presets::ALL.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ")))
}

use crate::scene::revision;

fn to_scene(v: Value) -> Result<Scene, String> {
    let mut v = v;
    if v.get("version").is_none() { v["version"] = json!(SCENE_VERSION); }
    crate::scene::fill_kind_defaults(&mut v);
    let scene: Scene = serde_json::from_value(v.clone()).map_err(|e| format!("not a valid scene: {e}"))?;
    let unknown = unknown_fields(&v, &scene_value(&scene).unwrap_or(Value::Null), "");
    if !unknown.is_empty() {
        return Err(format!("unknown field(s): {}. pf_schema lists the fields each part takes.", unknown.join(", ")));
    }
    Ok(scene)
}

/// Keys present in `given` that do not survive a round trip through the scene types.
fn unknown_fields(given: &Value, kept: &Value, at: &str) -> Vec<String> {
    let mut out = Vec::new();
    match (given, kept) {
        (Value::Object(g), Value::Object(k)) => for (key, gv) in g {
            let p = format!("{at}/{key}");
            // Fields left out of files when empty (a prop layer's `def`) come back missing.
            let empty = match gv { Value::Null => true, Value::String(t) => t.is_empty(), Value::Array(a) => a.is_empty(), Value::Object(o) => o.is_empty(), _ => false };
            match k.get(key) { Some(kv) => out.extend(unknown_fields(gv, kv, &p)), None if empty => {}, None => out.push(p) }
        },
        (Value::Array(g), Value::Array(k)) => for (i, (gv, kv)) in g.iter().zip(k).enumerate() {
            out.extend(unknown_fields(gv, kv, &format!("{at}/{i}")));
        },
        _ => {}
    }
    out
}

/// Pointers of the leaves that differ between two values.
fn changed(a: &Value, b: &Value, at: &str, out: &mut Vec<String>) {
    if out.len() > 60 { return; }
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            for (k, xv) in x { match y.get(k) { Some(yv) => changed(xv, yv, &format!("{at}/{k}"), out), None => out.push(format!("{at}/{k} (removed)")) } }
            for k in y.keys() { if !x.contains_key(k) { out.push(format!("{at}/{k} (added)")); } }
        }
        (Value::Array(x), Value::Array(y)) if x.len() == y.len() => for (i, (xv, yv)) in x.iter().zip(y).enumerate() { changed(xv, yv, &format!("{at}/{i}"), out) },
        (Value::Array(x), Value::Array(y)) => out.push(format!("{at} ({} -> {} items)", x.len(), y.len())),
        _ => if a != b { out.push(format!("{at}: {} -> {}", short(a), short(b))) },
    }
}

fn short(v: &Value) -> String {
    let s = v.to_string();
    if s.len() > 60 { format!("{}...", &s[..s.char_indices().nth(57).map(|c| c.0).unwrap_or(s.len())]) } else { s }
}

fn merge_patch(target: &mut Value, patch: &Value) {
    if let Value::Object(p) = patch {
        if !target.is_object() { *target = Value::Object(Map::new()); }
        let t = target.as_object_mut().unwrap();
        for (k, v) in p {
            if v.is_null() { t.remove(k); } else { merge_patch(t.entry(k.clone()).or_insert(Value::Null), v); }
        }
    } else {
        *target = patch.clone();
    }
}

fn split_pointer(pointer: &str) -> Result<(String, String), String> {
    if !pointer.starts_with('/') { return Err(format!("pointer '{pointer}' must start with '/'")); }
    let i = pointer.rfind('/').unwrap();
    Ok((pointer[..i].to_owned(), pointer[i + 1..].replace("~1", "/").replace("~0", "~")))
}

fn apply_op(doc: &mut Value, op: &Value) -> Result<(), String> {
    let kind = op.get("op").and_then(|v| v.as_str()).ok_or("each op needs \"op\"")?;
    let pointer = op.get("pointer").or_else(|| op.get("path")).and_then(|v| v.as_str()).ok_or("each op needs \"pointer\"")?;
    let value = op.get("value").cloned();
    let need = |v: Option<Value>| v.ok_or_else(|| format!("{kind} {pointer}: needs \"value\""));
    match kind {
        "append" => {
            let arr = doc.pointer_mut(pointer).ok_or_else(|| format!("append: nothing at {pointer}"))?;
            arr.as_array_mut().ok_or_else(|| format!("append: {pointer} is not a list"))?.push(need(value)?);
        }
        "set" | "insert" | "remove" => {
            let (parent, last) = split_pointer(pointer)?;
            let p = doc.pointer_mut(&parent).ok_or_else(|| format!("{kind}: nothing at {}", if parent.is_empty() { "/" } else { &parent }))?;
            match p {
                Value::Object(m) => match kind {
                    "remove" => { m.remove(&last).ok_or_else(|| format!("remove: no field {pointer}"))?; }
                    _ => { m.insert(last, need(value)?); }
                },
                Value::Array(a) => {
                    let idx = if last == "-" { a.len() } else { last.parse::<usize>().map_err(|_| format!("{kind}: '{last}' is not a list index"))? };
                    match kind {
                        "remove" if idx < a.len() => { a.remove(idx); }
                        "set" if idx < a.len() => a[idx] = need(value)?,
                        "set" | "insert" if idx == a.len() => a.push(need(value)?),
                        "insert" if idx < a.len() => a.insert(idx, need(value)?),
                        _ => return Err(format!("{kind}: index {idx} is out of range (list has {} items)", a.len())),
                    }
                }
                _ => return Err(format!("{kind}: {parent} is not an object or list")),
            }
        }
        other => return Err(format!("unknown op '{other}' (set, remove, append, insert)")),
    }
    Ok(())
}

fn summary(s: &Scene) -> String {
    let setting = if s.walls.enabled { "walled" } else if s.verge.enabled { "open, with verges" } else { "open" };
    let sky = if s.sky.enabled { if s.sky.sun.enabled { "day sky" } else if s.sky.moon.body.enabled { "night sky" } else { "sky" } } else { "no sky" };
    let fx: Vec<String> = s.fixtures.iter().filter(|f| f.enabled).map(|f| format!("{} every {}m", f.kind.name(), f.spacing)).collect();
    let pr: Vec<&str> = s.props.iter().map(|p| p.kind.name()).collect();
    let pa: Vec<&str> = s.particles.iter().map(|p| p.kind.name()).collect();
    let mut out = format!("{} {}, {}, {} path", setting, s.path.material.pattern.name().to_lowercase(), sky, if s.path.bend.abs() > 0.05 { "bending" } else { "straight" });
    if !fx.is_empty() { out += &format!("; lights: {}", fx.join(", ")); }
    if !pr.is_empty() { out += &format!("; props: {}", pr.join(", ")); }
    let sp: Vec<&str> = s.set_pieces.iter().filter(|p| p.enabled).map(|p| p.kind.name()).collect();
    if !sp.is_empty() { out += &format!("; set pieces: {}", sp.join(", ")); }
    if !pa.is_empty() { out += &format!("; particles: {}", pa.join(", ")); }
    let strikes = crate::world::lightning_times(s);
    if !strikes.is_empty() {
        let at: Vec<String> = strikes.iter().map(|p| format!("{:.2}s", p * s.motion.loop_seconds())).collect();
        out += &format!("; lightning at {}", at.join(", "));
    }
    if s.path.stairs.enabled {
        let st = &s.path.stairs;
        out += &format!("; stairs {} {} steps every {} m", if st.descending { "down" } else { "up" }, st.steps, st.spacing);
    }
    if s.path.bridge.enabled {
        let b = &s.path.bridge;
        out += &format!("; bridges {} m long every {} m over a {} m drop", b.length, b.spacing, b.depth);
    }
    if s.path.fork.enabled {
        let kind = if s.walls.enabled { "side passages" } else { "forks" };
        out += &format!("; {kind} every {} m", s.path.fork.spacing);
    }
    if s.weather.fog_banks.enabled { out += &format!("; fog banks every {} m", s.weather.fog_banks.spacing); }
    let w = &s.weather;
    if w.precipitation.enabled { out += &format!("; {:?} at {:.0}%", w.precipitation.kind, w.precipitation.intensity * 100.0).to_lowercase(); }
    if w.drips.enabled { out += "; drips"; }
    if w.wind.enabled { out += &format!("; wind {} m/s", w.wind.speed); }
    for (on, name) in [(w.sandstorm.enabled, "sandstorm"), (w.mist.enabled, "ground mist"), (w.light_shafts.enabled, "light shafts"), (w.heat_shimmer.enabled, "heat shimmer"),
        (s.sky.clouds.shadows > 0.0, "cloud shadows"), (s.sky.aurora.enabled, "aurora"), (s.sky.rainbow.enabled, "rainbow")] {
        if on { out += &format!("; {name}"); }
    }
    if w.lens.enabled { out += &format!("; {:?} on the lens", w.lens.kind).to_lowercase(); }
    if !matches!(s.style.palette, crate::scene::Palette::Full) || s.style.pixel_size > 1 { out += &format!("; pixel style {}px", s.style.pixel_size); }
    out
}

fn hidden_layers(args: &Value) -> Result<Layers, String> {
    let mut l = Layers::default();
    if let Some(list) = args.get("hide").and_then(|v| v.as_array()) {
        for item in list {
            match item.as_str().unwrap_or("") {
                "sky" => l.sky = false, "ground" => l.ground = false, "walls" => l.walls = false, "props" => l.props = false,
                "fixtures" => l.fixtures = false, "particles" => l.particles = false, "post" => l.post = false,
                other => return Err(format!("unknown layer '{other}' (sky, ground, walls, props, fixtures, particles, post)")),
            }
        }
    }
    Ok(l)
}

fn num(args: &Value, key: &str) -> Option<f64> { args.get(key).and_then(|v| v.as_f64()) }
fn int(args: &Value, key: &str) -> Option<i64> { args.get(key).and_then(|v| v.as_i64()) }
fn flag(args: &Value, key: &str, default: bool) -> bool { args.get(key).and_then(|v| v.as_bool()).unwrap_or(default) }
fn string<'a>(args: &'a Value, key: &str) -> Option<&'a str> { args.get(key).and_then(|v| v.as_str()).filter(|s| !s.trim().is_empty()) }

/// A loaded scene and where it came from.
struct Loaded {
    scene: Scene,
    file: Option<PathBuf>,
    revision: Option<String>,
    label: String,
}

impl Server {
    pub fn new() -> Server { Server { renderer: WorldRenderer::default(), home: home() } }

    fn resolve(&self, p: &str) -> PathBuf {
        let p = PathBuf::from(p);
        let abs = if p.is_absolute() { p } else { self.home.join(p) };
        abs.components().collect()
    }

    fn load(&self, args: &Value) -> Result<Loaded, String> {
        let given = ["path", "preset", "scene"].iter().filter(|k| args.get(**k).is_some_and(|v| !v.is_null())).count();
        if given != 1 { return Err("give exactly one of `path` (scene file), `preset` (built-in name) or `scene` (inline object)".into()); }
        if let Some(p) = string(args, "path") {
            let file = self.resolve(p);
            let text = std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
            let scene = Scene::from_json(&text).map_err(|e| format!("{}: {e}", file.display()))?;
            return Ok(Loaded { scene, revision: Some(revision(&text)), label: file.display().to_string(), file: Some(file) });
        }
        if let Some(name) = string(args, "preset") {
            return Ok(Loaded { scene: find_preset(name)?, file: None, revision: None, label: format!("preset {name}") });
        }
        let scene = to_scene(args.get("scene").cloned().unwrap_or(Value::Null))?;
        Ok(Loaded { scene, file: None, revision: None, label: "inline scene".into() })
    }

    fn opts(&self, l: &Loaded, scale: f32, layers: Layers, stats: bool) -> RenderOptions {
        let (w, h) = (l.scene.canvas.width.max(16), l.scene.canvas.height.max(16));
        let size = if scale < 0.999 { Some((((w as f32 * scale).round() as u32).max(16), ((h as f32 * scale).round() as u32).max(16))) } else { None };
        RenderOptions { size, base_dir: Some(self.base_of(l)), layers, palette: None, stats, depth: false, time: None, pick: false }
    }

    /// Render at `scale` of the canvas. Pixel styles render at full size and are box-scaled down so
    /// their art pixels survive; everything else renders directly at the preview size (faster).
    fn preview(&mut self, l: &Loaded, distance: f32, scale: f32, layers: Layers) -> Image {
        let pixel_style = l.scene.style.pixel_size > 1;
        if pixel_style {
            let img = self.renderer.render(&l.scene, distance, &self.opts(l, 1.0, layers, true));
            let stats = img.stats.clone();
            let mut small = review::downscale(&img, scale);
            small.stats = stats;
            small
        } else {
            self.renderer.render(&l.scene, distance, &self.opts(l, scale, layers, true))
        }
    }

    /// Where a scene's relative paths (sprites, kits, grades) start: its folder, or the home folder
    /// for presets and inline scenes.
    fn base_of(&self, l: &Loaded) -> PathBuf { l.file.as_ref().and_then(|f| f.parent().map(Path::to_path_buf)).unwrap_or_else(|| self.home.clone()) }

    fn write_scene(&self, file: &Path, scene: &Scene) -> Result<String, String> { crate::scene::save_file(file, scene) }

    pub fn call(&mut self, name: &str, args: &Value) -> Reply {
        let r = match name {
            "pf_presets" => self.presets(),
            "pf_schema" => self.schema(args),
            "pf_palettes" => self.palettes(),
            "pf_styles" => self.styles(args),
            "pf_new_scene" => self.new_scene(args),
            "pf_get_scene" => self.get_scene(args),
            "pf_edit_scene" => self.edit_scene(args),
            "pf_render" => self.render(args),
            "pf_contact_sheet" => self.contact_sheet(args),
            "pf_compare" => self.compare(args),
            "pf_analyze" => self.analyze(args),
            "pf_check_loop" => self.check_loop(args),
            "pf_camera" => self.camera(args),
            "pf_export" => self.export(args),
            "pf_kit" => self.kit(args),
            "pf_pack" => self.pack(args),
            "pf_remix" => self.remix(args),
            "pf_transition" => self.transition(args),
            "pf_journey" => self.journey(args),
            "pf_import_sprite" => self.import_sprite(args),
            other => Err(format!("unknown tool '{other}'")),
        };
        r.unwrap_or_else(Reply::error)
    }

    // ── Tools ──────────────────────────────────────────────────────────────

    fn presets(&self) -> ToolResult {
        let list: Vec<Value> = presets::ALL.iter().map(|(n, f)| {
            let s = f();
            json!({"name": n, "summary": summary(&s)})
        }).collect();
        Ok(Reply::json(&json!({"presets": list, "home": self.home})))
    }

    fn schema(&self, args: &Value) -> ToolResult {
        let full = serde_json::to_value(schemars::schema_for!(Scene)).map_err(|e| e.to_string())?;
        let mut defs = full.get("$defs").cloned().unwrap_or(json!({}));
        // Transitions, forks and journeys live beside scenes, not in them.
        for extra in [serde_json::to_value(schemars::schema_for!(crate::journey::Journey)).map_err(|e| e.to_string())?] {
            let mut top = extra.clone();
            if let (Some(d), Some(o)) = (extra.get("$defs").and_then(|d| d.as_object()), defs.as_object_mut()) {
                for (k, v) in d { o.entry(k.clone()).or_insert(v.clone()); }
            }
            if let (Some(t), Some(o)) = (top.as_object_mut(), defs.as_object_mut()) { t.remove("$defs"); o.insert("Journey".into(), Value::Object(t.clone())); }
        }
        match string(args, "section") {
            None => {
                let mut top = full.clone();
                if let Some(o) = top.as_object_mut() { o.remove("$defs"); }
                let sections: Vec<&String> = defs.as_object().map(|o| o.keys().collect()).unwrap_or_default();
                Ok(Reply::json(&json!({"scene": top, "sections": sections, "note": "Pass section=<name> for one of the sections. Every field is optional; missing fields take their defaults."})))
            }
            Some(sec) => {
                let o = defs.as_object().cloned().unwrap_or_default();
                let key = o.keys().find(|k| k.eq_ignore_ascii_case(sec)).cloned()
                    .ok_or_else(|| format!("no section '{sec}'. Sections: {}", o.keys().cloned().collect::<Vec<_>>().join(", ")))?;
                let def = &o[&key];
                // Also include the types this section refers to, one level down, so enums are spelled out.
                let mut refs = Map::new();
                collect_refs(def, &o, &mut refs, 2);
                refs.remove(&key);
                let default = default_of(&key);
                Ok(Reply::json(&json!({"section": key, "schema": def, "referenced": refs, "default": clean(&default)})))
            }
        }
    }

    fn styles(&mut self, args: &Value) -> ToolResult {
        use crate::scene::styles::ALL;
        let list: Vec<Value> = ALL.iter().map(|s| json!({"name": s.name, "about": s.about})).collect();
        let info = json!({"styles": list, "grades": crate::world::looks::BUILTIN, "note": "style.grade also takes a .cube LUT path relative to the scene file."});
        let has_source = ["path", "preset", "scene"].iter().any(|k| args.get(*k).is_some_and(|v| !v.is_null()));
        if !has_source { return Ok(Reply::json(&info)); }
        let l = self.load(args)?;
        let scale = num(args, "scale").unwrap_or(0.25) as f32;
        let t = num(args, "t").unwrap_or(0.0) as f32;
        let mut imgs = Vec::new();
        for st in ALL {
            let mut s = l.scene.clone();
            (st.apply)(&mut s);
            let ls = Loaded { scene: s, file: l.file.clone(), revision: None, label: String::new() };
            let d = t * ls.scene.motion.loop_length;
            imgs.push(self.preview(&ls, d, scale, Layers::default()));
        }
        Ok(Reply::json(&info).with_image(&review::sheet(&imgs, 5)))
    }

    fn palettes(&self) -> ToolResult {
        let list: Vec<Value> = NAMED.iter().map(|(n, hex)| json!({"name": n, "colors": hex.split_whitespace().map(|h| format!("#{h}")).collect::<Vec<_>>()})).collect();
        Ok(Reply::json(&json!({
            "palettes": list,
            "usage": "style.palette = \"Full\" | {\"Named\": \"PICO-8\"} | {\"Auto\": 16} (best 16 colours for the scene) | {\"Custom\": [[r,g,b], ...]}. Pair with style.pixel_size (2..8) and style.dither (\"None\", \"Bayer2\", \"Bayer4\", \"Bayer8\"). Enum values are spelled as in pf_schema (PascalCase)."
        })))
    }

    fn new_scene(&mut self, args: &Value) -> ToolResult {
        let p = string(args, "path").ok_or("`path` is required")?;
        let file = self.resolve(p);
        if file.exists() && !flag(args, "overwrite", false) {
            return Err(format!("{} already exists; pass overwrite: true to replace it, or edit it with pf_edit_scene", file.display()));
        }
        let mut scene = match (args.get("scene"), string(args, "preset")) {
            (Some(v), _) if !v.is_null() => to_scene(v.clone())?,
            (_, Some(name)) => find_preset(name)?,
            _ => find_preset("Stone Dungeon")?,
        };
        scene.version = SCENE_VERSION;
        if let Some(n) = string(args, "name") { scene.name = n.to_owned(); }
        else if args.get("scene").is_none() {
            scene.name = file.file_stem().map(|s| s.to_string_lossy().replace(['_', '-'], " ")).unwrap_or(scene.name);
        }
        let rev = self.write_scene(&file, &scene)?;
        let mut reply = Reply::json(&json!({"path": file, "revision": rev, "summary": summary(&scene)}));
        if flag(args, "preview", true) {
            let l = Loaded { scene, file: Some(file), revision: None, label: String::new() };
            let img = self.preview(&l, 0.0, num(args, "scale").unwrap_or(0.5) as f32, Layers::default());
            reply = reply.with_image(&img);
        }
        Ok(reply)
    }

    fn get_scene(&self, args: &Value) -> ToolResult {
        let l = self.load(args)?;
        let v = scene_value(&l.scene)?;
        let part = match string(args, "pointer") {
            Some(ptr) if ptr != "/" => v.pointer(ptr).cloned().ok_or_else(|| format!("nothing at {ptr}"))?,
            _ => v,
        };
        Ok(Reply::json(&json!({"source": l.label, "revision": l.revision, "value": part})))
    }

    fn edit_scene(&mut self, args: &Value) -> ToolResult {
        let p = string(args, "path").ok_or("`path` is required (pf_new_scene creates a file from a preset)")?;
        let file = self.resolve(p);
        let text = std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        if let Some(want) = string(args, "revision") {
            let have = revision(&text);
            if want != have {
                return Err(format!("{} changed since revision {want} (now {have}); read it again with pf_get_scene and redo the edit", file.display()));
            }
        }
        let before = scene_value(&Scene::from_json(&text)?)?;
        let mut doc = before.clone();
        if args.get("merge").is_none() && args.get("ops").is_none() && args.get("style_preset").is_none() { return Err("nothing to do: pass `style_preset`, `merge` and/or `ops`".into()); }
        if let Some(name) = string(args, "style_preset") {
            let st = crate::scene::styles::find(name).ok_or_else(|| format!("no style '{name}'. Styles: {}", crate::scene::styles::ALL.iter().map(|s| s.name).collect::<Vec<_>>().join(", ")))?;
            let mut sc: Scene = serde_json::from_value(doc.clone()).map_err(|e| e.to_string())?;
            (st.apply)(&mut sc);
            doc = scene_value(&sc)?;
        }
        if let Some(m) = args.get("merge") {
            if !m.is_object() { return Err("`merge` must be an object".into()); }
            merge_patch(&mut doc, m);
        }
        if let Some(ops) = args.get("ops") {
            let ops = ops.as_array().ok_or("`ops` must be a list")?;
            for (i, op) in ops.iter().enumerate() { apply_op(&mut doc, op).map_err(|e| format!("op {i}: {e}"))?; }
        }
        let scene = to_scene(doc)?;
        let after = scene_value(&scene)?;
        let mut diffs = Vec::new();
        changed(&before, &after, "", &mut diffs);
        let dry = flag(args, "dry_run", false);
        let rev = if dry || diffs.is_empty() { revision(&text) } else { self.write_scene(&file, &scene)? };
        let mut warnings = scene_warnings(&scene);
        warnings.extend(crate::world::propdefs::def_warnings(&scene, file.parent()));
        if let Some(dir) = file.parent() { warnings.extend(crate::project::asset_warnings(&scene, dir)); }
        if let Some(m) = args.get("merge").and_then(|m| m.as_object()) {
            for (k, v) in m {
                if v.is_array() {
                    warnings.insert(0, format!("merge replaced the whole /{k} list, so fields you did not give took their defaults; to change one item use ops, e.g. {{\"op\": \"set\", \"pointer\": \"/{k}/0/offset\", \"value\": 4}}"));
                }
            }
        }
        let mut reply = Reply::json(&json!({
            "path": file, "written": !dry && !diffs.is_empty(), "revision": rev,
            "changed": if diffs.is_empty() { json!("nothing (the values were already set)") } else { json!(diffs) },
            "warnings": warnings,
        }));
        if flag(args, "preview", true) {
            let l = Loaded { scene, file: Some(file), revision: None, label: String::new() };
            let d = num(args, "t").unwrap_or(0.0) as f32 * l.scene.motion.loop_length;
            let img = self.preview(&l, d, num(args, "scale").unwrap_or(0.5) as f32, Layers::default());
            reply = reply.with_image(&img);
        }
        Ok(reply)
    }

    fn render(&mut self, args: &Value) -> ToolResult {
        let l = self.load(args)?;
        let len = l.scene.motion.loop_length.max(1.0);
        let d = num(args, "distance").map(|d| d as f32).unwrap_or_else(|| num(args, "t").unwrap_or(0.0) as f32 * len);
        let t0 = Instant::now();
        let img = self.preview(&l, d, num(args, "scale").unwrap_or(0.5) as f32, hidden_layers(args)?);
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        Ok(Reply::json(&json!({
            "source": l.label, "distance_m": round2(d), "t": round3(d / len), "size": [img.width, img.height],
            "render_ms": (ms * 10.0).round() / 10.0, "frame": clean(&img.stats),
        })).with_image(&img))
    }

    fn contact_sheet(&mut self, args: &Value) -> ToolResult {
        let l = self.load(args)?;
        let n = int(args, "frames").unwrap_or(6).clamp(2, 24) as usize;
        let cols = int(args, "columns").map(|c| c.max(1) as usize).unwrap_or(n.min(6));
        let scale = num(args, "scale").unwrap_or(0.3) as f32;
        let layers = hidden_layers(args)?;
        let len = l.scene.motion.loop_length.max(1.0);
        let frames: Vec<Image> = (0..n).map(|i| self.preview(&l, len * i as f32 / n as f32, scale, layers)).collect();
        let sheet = review::sheet(&frames, cols);
        let at: Vec<String> = (0..n).map(|i| format!("{:.1}m", len * i as f32 / n as f32)).collect();
        Ok(Reply::json(&json!({"source": l.label, "frames": n, "columns": cols, "distances": at, "loop_length_m": len, "loop_seconds": l.scene.motion.loop_seconds()})).with_image(&sheet))
    }

    fn remix(&mut self, args: &Value) -> ToolResult {
        use crate::scene::remix::remix;
        let l = self.load(args)?;
        let keep: Vec<String> = match args.get("keep").and_then(|k| k.as_array()) {
            Some(a) => a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect(),
            None => vec!["camera".into(), "path".into(), "look".into()],
        };
        let keep: Vec<&str> = keep.iter().map(String::as_str).collect();
        let seed = int(args, "seed").unwrap_or(1).max(0) as u32;
        let jitter = num(args, "jitter").unwrap_or(0.12) as f32;
        let scale = num(args, "scale").unwrap_or(0.3) as f32;
        if let Some(out) = string(args, "save_as") {
            let file = self.resolve(out);
            if file.exists() && !flag(args, "overwrite", false) { return Err(format!("{} exists; pass overwrite: true to replace it", file.display())); }
            let s = remix(&l.scene, &keep, seed, jitter)?;
            let rev = self.write_scene(&file, &s)?;
            let saved = Loaded { scene: s, file: Some(file.clone()), revision: Some(rev.clone()), label: file.display().to_string() };
            let img = self.preview(&saved, 0.0, scale.max(0.4), Layers::default());
            return Ok(Reply::json(&json!({"path": file, "revision": rev, "seed": seed, "kept": keep, "summary": summary(&saved.scene)})).with_image(&img));
        }
        let count = int(args, "count").unwrap_or(4).clamp(1, 8) as u32;
        let mut imgs = Vec::new();
        let mut seeds = Vec::new();
        for k in 0..count {
            let s = remix(&l.scene, &keep, seed + k, jitter)?;
            let item = Loaded { scene: s, file: l.file.clone(), revision: None, label: String::new() };
            imgs.push(self.preview(&item, 0.0, scale, Layers::default()));
            seeds.push(json!({"seed": seed + k, "summary": summary(&item.scene)}));
        }
        Ok(Reply::json(&json!({"source": l.label, "kept": keep, "variations": seeds,
            "next": "pick one and write it with save_as and its seed; images run left to right"})).with_image(&review::sheet(&imgs, count.min(4) as usize)))
    }

    fn compare(&mut self, args: &Value) -> ToolResult {
        let mut items: Vec<Loaded> = Vec::new();
        if let Some(ps) = args.get("paths").and_then(|v| v.as_array()) {
            for p in ps { items.push(self.load(&json!({"path": p}))?); }
        }
        if let Some(ps) = args.get("presets").and_then(|v| v.as_array()) {
            if ps.iter().any(|p| p.as_str() == Some("*")) {
                for (n, _) in presets::ALL { items.push(self.load(&json!({"preset": n}))?); }
            } else {
                for p in ps { items.push(self.load(&json!({"preset": p}))?); }
            }
        }
        if items.is_empty() { return Err("give `paths` and/or `presets` to compare".into()); }
        if items.len() > 30 { return Err("compare at most 30 scenes at a time".into()); }
        let scale = num(args, "scale").unwrap_or(0.3) as f32;
        let t = num(args, "t").unwrap_or(0.0) as f32;
        let mut imgs = Vec::new();
        for l in &items {
            let d = t * l.scene.motion.loop_length;
            imgs.push(self.preview(l, d, scale, Layers::default()));
        }
        let cols = int(args, "columns").map(|c| c.max(1) as usize).unwrap_or(items.len().min(5));
        let order: Vec<String> = items.iter().enumerate().map(|(i, l)| format!("{}: {}", i + 1, l.label)).collect();
        Ok(Reply::json(&json!({"order": order, "columns": cols, "note": "Images run left to right, then top to bottom."})).with_image(&review::sheet(&imgs, cols)))
    }

    fn analyze(&mut self, args: &Value) -> ToolResult {
        let l = self.load(args)?;
        let n = int(args, "frames").unwrap_or(4).clamp(1, 12) as usize;
        let len = l.scene.motion.loop_length.max(1.0);
        let opts = self.opts(&l, 1.0, Layers::default(), true);
        let mut cov: std::collections::BTreeMap<String, (f32, f32, f32)> = Default::default();
        let (mut mean, mut lights, mut bills, mut ms) = (0.0f32, 0usize, 0usize, 0.0f64);
        for i in 0..n {
            let t0 = Instant::now();
            let img = self.renderer.render(&l.scene, len * i as f32 / n as f32, &opts);
            ms += t0.elapsed().as_secs_f64() * 1000.0;
            let s = img.stats.unwrap_or_default();
            for (k, c) in &s.coverage {
                let e = cov.entry(k.clone()).or_insert((0.0, 0.0, 0.0));
                e.0 += c;
                e.1 += s.luma.get(k).copied().unwrap_or(0.0) * c;
                e.2 = e.2.max(*c);
            }
            mean += s.mean_luma;
            lights = lights.max(s.lights);
            bills = bills.max(s.billboards);
        }
        let nf = n as f32;
        let mut surfaces = Map::new();
        for (k, (c, lw, peak)) in &cov {
            let avg = c / nf;
            surfaces.insert(k.clone(), json!({"coverage": round3(avg), "peak_coverage": round3(*peak), "mean_luma": if *c > 0.0 { (lw / c).round() as f64 } else { 0.0 }}));
        }
        let mean = mean / nf;
        let mut warnings = scene_warnings(&l.scene);
        warnings.extend(crate::world::propdefs::def_warnings(&l.scene, Some(&self.base_of(&l))));
        warnings.extend(crate::project::asset_warnings(&l.scene, &self.base_of(&l)));
        let surf: std::collections::BTreeMap<String, review::Surface> = cov.iter()
            .map(|(k, (c, lw, peak))| (k.clone(), review::Surface { coverage: c / nf, luma: if *c > 0.0 { lw / c } else { 0.0 }, peak: *peak })).collect();
        warnings.extend(review::stats_warnings(&l.scene, mean, &surf));
        Ok(Reply::json(&json!({
            "source": l.label, "frames_sampled": n, "size": [l.scene.canvas.width, l.scene.canvas.height],
            "mean_luma": mean.round() as f64, "surfaces": surfaces, "lights": lights, "billboards": bills,
            "render_ms_per_frame": ((ms / n as f64) * 10.0).round() / 10.0,
            "loop": {"length_m": len as f64, "seconds": round3(l.scene.motion.loop_seconds()), "frames": l.scene.motion.frames(), "metres_per_frame": round3(len / l.scene.motion.frames() as f32)},
            "warnings": warnings,
        })))
    }

    fn check_loop(&mut self, args: &Value) -> ToolResult {
        let l = self.load(args)?;
        let frames = int(args, "frames").unwrap_or(24).clamp(4, 96) as usize;
        let opts = self.opts(&l, num(args, "scale").unwrap_or(0.5) as f32, Layers::default(), false);
        let t0 = Instant::now();
        let rep = review::check_loop(&mut self.renderer, &l.scene, &opts, frames);
        Ok(Reply::json(&json!({"source": l.label, "frames": frames, "report": clean(&rep), "elapsed_ms": t0.elapsed().as_millis() as u64,
            "how_to_read": "exact ~0 means the loop closes. wrap_over_step near 1 means the loop point looks like any other step; much above 2 means something pops at the wrap."})))
    }

    /// The second (or a branch) scene of a transition from `prefix`_path / `prefix`_preset.
    fn other(&self, args: &Value, prefix: &str) -> Result<Option<Loaded>, String> {
        match (string(args, &format!("{prefix}_path")), string(args, &format!("{prefix}_preset"))) {
            (Some(p), _) => Ok(Some(self.load(&json!({"path": p}))?)),
            (_, Some(p)) => Ok(Some(self.load(&json!({"preset": p}))?)),
            _ => Ok(None),
        }
    }

    fn export_job(&self, l: &Loaded, e: &Value) -> Result<ExportJob, String> {
        let formats = match e.get("formats").and_then(|v| v.as_array()) {
            Some(a) => a.iter().map(|f| f.as_str().and_then(Format::parse).ok_or_else(|| format!("unknown format {f}"))).collect::<Result<Vec<_>, _>>()?,
            None => vec![Format::Webp],
        };
        Ok(ExportJob { formats, out_dir: self.resolve(string(e, "out_dir").unwrap_or("exports")), name: string(e, "name").unwrap_or("").to_owned(), base_dir: Some(self.base_of(l)), ..ExportJob::default() })
    }

    fn transition(&mut self, args: &Value) -> ToolResult {
        use crate::journey::{CrossWalk, ForkWalk, Place};
        use crate::scene::transition::{self as tr, Branch, ForkChoice, Transition};
        let l = self.load(args)?;
        let frames = int(args, "frames").unwrap_or(8).clamp(2, 24) as usize;
        let scale = num(args, "scale").unwrap_or(0.25).clamp(0.08, 1.0) as f32;
        let opts = self.opts(&l, scale, Layers::default(), false);
        let dir_of = |x: &Loaded| x.file.as_ref().and_then(|f| f.parent().map(Path::to_path_buf));
        let a_dir = Some(self.base_of(&l));
        let pa = Place { scene: &l.scene, dir: a_dir.as_deref() };
        let mut imgs = Vec::new();
        let cancel = AtomicBool::new(false);
        let names = |fs: &[PathBuf]| fs.iter().map(|f| f.display().to_string()).collect::<Vec<_>>();
        if let (Some(left), Some(right)) = (self.other(args, "left")?, self.other(args, "right")?) {
            let f: ForkChoice = match args.get("fork") { Some(v) if !v.is_null() => serde_json::from_value(v.clone()).map_err(|e| format!("fork: {e}"))?, _ => ForkChoice::default() };
            let plan = tr::plan_fork(&l.scene, &left.scene, &right.scene, &f);
            let mut walk = ForkWalk::new(plan.clone(), 0.0, [0.0, 0.0]);
            let take = match string(args, "take") { Some("right") => Branch::Right, Some("left") => Branch::Left, _ => plan.default_branch };
            let (ld, rd) = (dir_of(&left), dir_of(&right));
            let (pl, pr) = (Place { scene: &left.scene, dir: ld.as_deref() }, Place { scene: &right.scene, dir: rd.as_deref() });
            let len = walk.length();
            let at = walk.decide_by() * 0.5;
            for i in 0..frames {
                let travel = len * i as f32 / (frames - 1) as f32;
                if travel >= at { walk.choose(take, at); }
                let t = travel / l.scene.motion.speed.max(0.01);
                imgs.push(walk.frame(&mut self.renderer, pa, pl, pr, travel, [t, t, t], &opts));
            }
            let mut out = json!({"from": l.label, "left": left.label, "right": right.label, "taken": take.name(), "plan": plan,
                "next": "adjust with `fork` (angle, approach_m, blend_m, wedge_m, steer_m, default_branch), then export; frames run left to right, top to bottom"});
            if let Some(e) = args.get("export").filter(|e| e.is_object()) {
                let job = self.export_job(&l, e)?;
                let rep = export::export_fork(&l.scene, (&left.scene, ld.clone()), (&right.scene, rd.clone()), &job, &f, |_| {}, &cancel)?;
                out["exported"] = json!(names(&rep.files));
            }
            return Ok(Reply::json(&out).with_image(&review::sheet(&imgs, frames.min(4))));
        }
        let b = self.other(args, "to")?.ok_or("give to_path or to_preset for a transition, or left_* and right_* for a fork")?;
        let t: Transition = match args.get("transition") { Some(v) if !v.is_null() => serde_json::from_value(v.clone()).map_err(|e| format!("transition: {e}"))?, _ => Transition::default() };
        let c = tr::plan(&l.scene, &b.scene, &t);
        let walk = CrossWalk::new(c.clone(), 0.0, 0.0);
        let bd = dir_of(&b);
        let pb = Place { scene: &b.scene, dir: bd.as_deref() };
        let len = walk.length();
        for i in 0..frames {
            let travel = len * i as f32 / (frames - 1) as f32;
            let tt = travel / l.scene.motion.speed.max(0.01);
            imgs.push(walk.frame(&mut self.renderer, pa, pb, travel, tt, tt, &opts));
        }
        let mut out = json!({"from": l.label, "to": b.label, "plan": c,
            "next": "adjust with `transition` (threshold, approach_m, blend_m, facade, light_spill, adaptation, marker, style), then export; frames run left to right, top to bottom"});
        if let Some(e) = args.get("export").filter(|e| e.is_object()) {
            let job = self.export_job(&l, e)?;
            let entries = int(e, "entries").unwrap_or(1).clamp(1, 16) as u32;
            let rep = export::export_transition_entries(&l.scene, &b.scene, bd.clone(), &job, &t, entries, |_| {}, &cancel)?;
            out["exported"] = json!(names(&rep.files));
        }
        Ok(Reply::json(&out).with_image(&review::sheet(&imgs, frames.min(4))))
    }

    fn journey(&mut self, args: &Value) -> ToolResult {
        use crate::journey::{Journey, Next};
        let path = self.resolve(string(args, "path").ok_or("journey needs `path`")?);
        let dir = path.parent().map(Path::to_path_buf);
        let op = string(args, "op").unwrap_or("read");
        if op == "write" {
            let j: Journey = serde_json::from_value(args.get("journey").cloned().ok_or("write needs `journey`")?).map_err(|e| format!("journey: {e}"))?;
            if path.exists() && !flag(args, "overwrite", false) { return Err(format!("{} exists; pass overwrite: true to replace it", path.display())); }
            let problems = j.problems(dir.as_deref());
            if let Some(d) = &dir { std::fs::create_dir_all(d).map_err(|e| e.to_string())?; }
            j.save(&path)?;
            return Ok(Reply::json(&json!({"path": path, "problems": problems, "next": if problems.is_empty() { "preview it (op preview) or export it" } else { "fix the problems, then write again" }})));
        }
        let j = Journey::load(&path)?;
        let problems = j.problems(dir.as_deref());
        match op {
            "read" => Ok(Reply::json(&json!({"path": path, "journey": j, "problems": problems}))),
            "preview" => {
                use crate::journey::{CrossWalk, ForkWalk, Place};
                let scale = num(args, "scale").unwrap_or(0.2).clamp(0.08, 0.6) as f32;
                let mut imgs = Vec::new();
                let mut captions = Vec::new();
                for (id, st) in &j.stops {
                    let (scene, sdir) = j.scene_of(id, dir.as_deref())?;
                    let l = Loaded { scene: scene.clone(), file: None, revision: None, label: id.clone() };
                    let opts = RenderOptions { base_dir: sdir.clone(), ..self.opts(&l, scale, Layers::default(), false) };
                    imgs.push(self.renderer.render(&scene, 0.0, &opts));
                    captions.push(format!("{id}: {}", st.scene));
                    let pa = Place { scene: &scene, dir: sdir.as_deref() };
                    match &st.next {
                        Next::End => {}
                        Next::Go { to, transition } => {
                            let (b, bd) = j.scene_of(to, dir.as_deref())?;
                            let c = crate::scene::transition::plan(&scene, &b, transition);
                            let w = CrossWalk::new(c.clone(), 0.0, 0.0);
                            let at = (c.approach - 3.0).max(0.0);
                            imgs.push(w.frame(&mut self.renderer, pa, Place { scene: &b, dir: bd.as_deref() }, at, 0.0, 0.0, &opts));
                            captions.push(format!("{id} -> {to}: {}", c.threshold.name()));
                        }
                        Next::Fork { left, right, fork } => {
                            let (lsc, ld) = j.scene_of(left, dir.as_deref())?;
                            let (rsc, rd) = j.scene_of(right, dir.as_deref())?;
                            let p = crate::scene::transition::plan_fork(&scene, &lsc, &rsc, fork);
                            let w = ForkWalk::new(p, 0.0, [0.0, 0.0]);
                            imgs.push(w.frame(&mut self.renderer, pa, Place { scene: &lsc, dir: ld.as_deref() }, Place { scene: &rsc, dir: rd.as_deref() }, w.decide_by(), [0.0; 3], &opts));
                            captions.push(format!("{id}: fork {left} | {right}"));
                        }
                    }
                }
                Ok(Reply::json(&json!({"path": path, "images": captions, "problems": problems})).with_image(&review::sheet(&imgs, 4)))
            }
            "export" => {
                if !problems.is_empty() { return Err(format!("fix the journey first: {}", problems.join("; "))); }
                let formats = match args.get("formats").and_then(|v| v.as_array()) {
                    Some(a) => a.iter().map(|f| f.as_str().and_then(Format::parse).ok_or_else(|| format!("unknown format {f}"))).collect::<Result<Vec<_>, _>>()?,
                    None => vec![Format::Webp],
                };
                let job = ExportJob { formats, out_dir: self.resolve(string(args, "out_dir").unwrap_or("exports")), ..ExportJob::default() };
                let cancel = AtomicBool::new(false);
                let rep = export::export_journey(&j, dir.as_deref(), &job, int(args, "entries").unwrap_or(1).clamp(1, 16) as u32, |_| {}, &cancel)?;
                Ok(Reply::json(&json!({"files": rep.files, "notes": rep.notes})))
            }
            other => Err(format!("unknown op `{other}` (read, write, preview, export)")),
        }
    }

    fn camera(&self, args: &Value) -> ToolResult {
        let l = self.load(args)?;
        let (w, h) = match args.get("size").and_then(|v| v.as_array()) {
            Some(a) if a.len() == 2 => (a[0].as_u64().unwrap_or(0).max(16) as usize, a[1].as_u64().unwrap_or(0).max(16) as usize),
            _ => (l.scene.canvas.width as usize, l.scene.canvas.height as usize),
        };
        let cam = CameraInfo::new(&l.scene, w, h);
        let v = &cam.view;
        let mut points = Vec::new();
        if let Some(ps) = args.get("points").and_then(|v| v.as_array()) {
            for p in ps {
                let (x, y, d) = (num(p, "x").unwrap_or(0.0) as f32, num(p, "y").unwrap_or(0.0) as f32, num(p, "d").unwrap_or(0.0) as f32);
                let px = v.world_to_px(x, y, d);
                points.push(json!({"x": round3(x), "y": round3(y), "d": round3(d), "screen": px.map(|p| [round1(p[0]), round1(p[1])]),
                    "px_per_m": round1(v.px_per_m(d)), "visible": px.is_some_and(|p| p[0] >= 0.0 && p[0] < w as f32 && p[1] >= 0.0 && p[1] < h as f32)}));
            }
        }
        let mut rows = Vec::new();
        if let Some(rs) = args.get("rows").and_then(|v| v.as_array()) {
            for r in rs {
                let row = r.as_f64().unwrap_or(0.0) as f32;
                rows.push(match ground_distance(v, row) {
                    Some(d) => {
                        let hw = v.path_half_width(d);
                        let left = v.world_to_px(-hw, 0.0, d).map(|p| round1(p[0]));
                        let right = v.world_to_px(hw, 0.0, d).map(|p| round1(p[0]));
                        json!({"row": round1(row), "ground_d": round2(d), "path_half_width_m": round2(hw), "path_left_px": left, "path_right_px": right, "px_per_m": round1(v.px_per_m(d))})
                    }
                    None => json!({"row": round1(row), "ground_d": null, "note": "this row is at or above the horizon: no ground there"}),
                });
            }
        }
        // A ready-made table: where the ground is at a few distances, and how tall a 1.8 m figure stands there.
        let table: Vec<Value> = [1.0f32, 2.0, 3.0, 5.0, 8.0, 12.0, 20.0, 40.0].iter().filter_map(|&d| {
            let feet = v.world_to_px(0.0, 0.0, d)?;
            if feet[1] > h as f32 * 1.2 { return None; }
            Some(json!({"d": d, "centre_ground_px": [round1(feet[0]), round1(feet[1])], "figure_1_8m_px": round1(1.8 * v.px_per_m(d)), "path_half_width_m": round2(v.path_half_width(d))}))
        }).collect();
        Ok(Reply::json(&json!({
            "source": l.label, "size": [w, h], "camera": clean(&cam), "points": points, "rows": rows, "ground_table": table,
            "scroll": {"speed_mps": round3(l.scene.motion.speed), "loop_length_m": round3(l.scene.motion.loop_length), "metres_per_frame": round3(l.scene.motion.loop_length / l.scene.motion.frames() as f32),
                "note": "Something standing still on the path at distance d gets closer by speed*dt every second: d(t) = d0 - speed*t. Draw it at world_to_px(x, y, d(t)) while d > ~0.5 m."}
        })))
    }

    /// A kit folder named by a tool argument, or a built-in kit (`@name`).
    fn kit_dir(&self, kit: &str) -> PathBuf {
        let k = kit.trim();
        if k.starts_with('@') { PathBuf::from(k) } else { self.resolve(k) }
    }

    /// Validate a kit given as JSON (unknown fields rejected), and write it atomically.
    fn write_kit(&self, dir: &Path, v: Value) -> Result<crate::scene::Kit, String> {
        let kit: crate::scene::Kit = serde_json::from_value(v.clone()).map_err(|e| format!("not a valid kit: {e}"))?;
        let unknown = unknown_fields(&v, &serde_json::to_value(&kit).map_err(|e| e.to_string())?, "");
        if !unknown.is_empty() { return Err(format!("unknown field(s): {}. pf_schema {{\"section\": \"PropDef\"}} lists the fields a prop takes.", unknown.join(", "))); }
        let file = crate::world::propdefs::kit_file(dir);
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let tmp = file.with_extension("json.part");
        std::fs::write(&tmp, serde_json::to_string_pretty(&kit).map_err(|e| e.to_string())? + "\n").map_err(|e| format!("{}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &file).map_err(|e| format!("{}: {e}", file.display()))?;
        Ok(kit)
    }

    /// The kit's props, warnings and (optionally) a picture of them, as a reply.
    fn kit_reply(&mut self, kit: &str, dir: &Path, args: &Value, mut extra: Map<String, Value>) -> ToolResult {
        use crate::world::propdefs::PropDefs;
        let k = PropDefs::default().kit(dir)?;
        let kit_ref = if kit.trim().starts_with('@') { kit.trim().to_owned() } else { dir.strip_prefix(&self.home).unwrap_or(dir).display().to_string() };
        let (mut defs, mut sprites) = (PropDefs::default(), crate::world::sprites::SpriteCache::default());
        let probe = crate::scene::Scene::default();
        let props: Vec<Value> = k.props.iter().map(|(n, d)| {
            // The height it is drawn at: given, the shape's own, or its kind's.
            let layer = crate::scene::PropLayer { def: format!("{}#{n}", dir.display()), ..Default::default() };
            let height = defs.resolve(&probe, None, &layer, &mut sprites).0.map(|r| (r.height * 100.0).round() / 100.0);
            let glowing_parts = d.shape.iter().filter(|p| p.glow > 0.0).count();
            let look = if d.sprite.is_set() { if d.sprite.pool.is_empty() { json!({"image": d.sprite.path}) } else { json!({"images": d.sprite.pool}) } }
                else if !d.shape.is_empty() { json!(format!("shape of {} parts", d.shape.len())) } else { json!(format!("built-in {}", d.kind.name())) };
            json!({"name": n, "use_as": {"def": format!("{kit_ref}#{n}")}, "description": d.description, "look": look,
                "height_m": height, "anchor": d.anchor, "glow": d.glow, "glowing_parts": glowing_parts, "light": d.light.enabled.then(|| &d.light)})
        }).collect();
        let backdrop = match string(args, "preset") { Some(p) => find_preset(p)?, None => review::kit_backdrop() };
        let only = string(args, "only");
        let (img, _, warnings) = review::kit_sheet(&mut self.renderer, &dir.to_string_lossy(), only, &backdrop, Some(self.home.clone()),
            num(args, "scale").unwrap_or(0.3).clamp(0.05, 1.0) as f32, int(args, "columns").unwrap_or(4).clamp(1, 12) as usize)?;
        extra.insert("kit".into(), json!(kit_ref));
        extra.insert("name".into(), json!(k.name));
        if !k.description.is_empty() { extra.insert("description".into(), json!(k.description)); }
        extra.insert("props".into(), json!(props));
        extra.insert("warnings".into(), json!(warnings));
        extra.insert("image".into(), json!(format!("props in name order{}", only.map(|o| format!(" (only {o})")).unwrap_or_default())));
        let reply = Reply::json(&Value::Object(extra));
        Ok(if flag(args, "preview", true) { reply.with_image(&img) } else { reply })
    }

    fn kit(&mut self, args: &Value) -> ToolResult {
        let kit = string(args, "kit").ok_or("`kit`: a kit folder (relative to the PathForge home) or @name for a built-in kit")?;
        let dir = self.kit_dir(kit);
        let editing = ["props", "name", "description", "from"].iter().any(|k| args.get(*k).is_some_and(|v| !v.is_null()));
        let mut extra = Map::new();
        if editing {
            if kit.trim().starts_with('@') { return Err(format!("built-in kits cannot be changed; make your own from it: {{\"kit\": \"kits/mine\", \"from\": \"{}\"}}", kit.trim())); }
            let file = crate::world::propdefs::kit_file(&dir);
            let mut v = match std::fs::read_to_string(&file) {
                Ok(text) => serde_json::from_str::<Value>(&text).map_err(|e| format!("{}: {e}", file.display()))?,
                Err(_) => {
                    let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let mut v = json!({"name": name, "props": {}});
                    if let Some(from) = string(args, "from") {
                        let k = crate::world::propdefs::PropDefs::default().kit(Path::new(from.trim()))?;
                        if !from.trim().starts_with('@') && k.props.values().any(|d| d.sprite.is_set()) {
                            return Err("`from` copies a built-in kit (@name); a folder kit's images would not come along".into());
                        }
                        v["props"] = serde_json::to_value(&k.props).map_err(|e| e.to_string())?;
                    }
                    v
                }
            };
            if let Some(n) = string(args, "name") { v["name"] = json!(n); }
            if let Some(d) = args.get("description").and_then(|d| d.as_str()) { v["description"] = json!(d); }
            if let Some(p) = args.get("props") {
                let p = p.as_object().ok_or("`props` must be an object: name -> prop definition (or null to remove)")?;
                if !v["props"].is_object() { v["props"] = json!({}); }
                for (name, def) in p {
                    if def.is_null() { v["props"].as_object_mut().unwrap().remove(name); continue; }
                    let slot = v["props"].as_object_mut().unwrap().entry(name.clone()).or_insert(json!({}));
                    merge_patch(slot, def);
                }
            }
            self.write_kit(&dir, v)?;
            extra.insert("written".into(), json!(file));
        }
        self.kit_reply(kit, &dir, args, extra)
    }

    fn import_sprite(&mut self, args: &Value) -> ToolResult {
        let kit = string(args, "kit").ok_or("`kit`: the kit folder to put the image in")?;
        if kit.trim().starts_with('@') { return Err("built-in kits cannot take images; give a kit folder".into()); }
        let dir = self.kit_dir(kit);
        let rel = string(args, "file").ok_or("`file`: where to save it inside the kit, e.g. reeds/reeds_1.png")?.trim().replace('\\', "/");
        if rel.starts_with('/') || rel.split('/').any(|c| c == "..") || !rel.to_lowercase().ends_with(".png") {
            return Err("`file` must be a relative .png path inside the kit (no ..)".into());
        }
        let target = dir.join(&rel);
        if target.exists() && !flag(args, "overwrite", false) { return Err(format!("{} exists; pass overwrite: true to replace it", target.display())); }
        let sources = ["url", "path", "base64"].iter().filter(|k| string(args, k).is_some()).count();
        if sources != 1 { return Err("give exactly one of `url`, `path` or `base64`".into()); }
        let bytes: Vec<u8> = if let Some(url) = string(args, "url") {
            let url = url.trim();
            if !(url.starts_with("https://") || url.starts_with("http://")) { return Err("`url` must be http(s)".into()); }
            // curl ships with macOS, Linux and Windows 10+, and spares a TLS stack in this binary.
            let out = std::process::Command::new("curl").args(["-fsSL", "--max-time", "60", "--max-filesize", "33554432", url]).output()
                .map_err(|e| format!("could not run curl to download it: {e}"))?;
            if !out.status.success() { return Err(format!("download failed: {}", String::from_utf8_lossy(&out.stderr).trim())); }
            out.stdout
        } else if let Some(p) = string(args, "path") {
            let src = self.resolve(p);
            std::fs::read(&src).map_err(|e| format!("{}: {e}", src.display()))?
        } else {
            let b = string(args, "base64").unwrap();
            let b = b.split_once("base64,").map(|x| x.1).unwrap_or(b);
            base64::engine::general_purpose::STANDARD.decode(b.trim()).map_err(|e| format!("base64: {e}"))?
        };
        let img = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png).map_err(|e| format!("not a PNG image: {e}"))?.to_rgba8();
        let (w, h) = img.dimensions();
        let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
        for (x, y, p) in img.enumerate_pixels() { if p[3] > 0 { x0 = x0.min(x); y0 = y0.min(y); x1 = x1.max(x + 1); y1 = y1.max(y + 1); } }
        if x1 <= x0 { return Err("the image is fully transparent".into()); }
        let opaque_bg = img.pixels().all(|p| p[3] == 255);
        let kit_json = crate::world::propdefs::kit_file(&dir);
        if let Some(def) = string(args, "def") {
            let exists = std::fs::read_to_string(&kit_json).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok())
                .is_some_and(|v| v["props"].get(def).is_some());
            if !exists && num(args, "height").is_none() { return Err(format!("`height` (metres) is needed to make the new prop '{def}'; nothing was saved")); }
        }
        if let Some(parent) = target.parent() { std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?; }
        let tmp = target.with_extension("png.part");
        std::fs::write(&tmp, &bytes).map_err(|e| format!("{}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &target).map_err(|e| format!("{}: {e}", target.display()))?;
        let mut extra = Map::new();
        extra.insert("saved".into(), json!(target));
        extra.insert("size".into(), json!([w, h]));
        extra.insert("visible".into(), json!({"x": x0, "y": y0, "width": x1 - x0, "height": y1 - y0}));
        if opaque_bg { extra.insert("note".into(), json!("the image has no transparency, so it will draw as a rectangle; generate it with a transparent background")); }
        let Some(def) = string(args, "def") else {
            extra.insert("next".into(), json!(format!("use it with pf_kit {{\"kit\": \"{kit}\", \"props\": {{\"<name>\": {{\"sprite\": {{\"path\": \"{rel}\"}}, \"height\": <metres>}}}}}}")));
            return Ok(Reply::json(&Value::Object(extra)));
        };
        let mut v = std::fs::read_to_string(&kit_json).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .unwrap_or_else(|| json!({"name": dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), "props": {}}));
        if !v["props"].is_object() { v["props"] = json!({}); }
        let props = v["props"].as_object_mut().unwrap();
        let new = !props.contains_key(def);
        let d = props.entry(def.to_owned()).or_insert(json!({"pixelated": true}));
        if new {
            d["sprite"] = json!({"pool": [rel], "pixelated": flag(args, "pixelated", true)});
            d.as_object_mut().unwrap().remove("pixelated");
            if let Some(t) = string(args, "description") { d["description"] = json!(t); }
        } else {
            let sp = d.get("sprite").cloned().unwrap_or(json!({}));
            let mut pool: Vec<Value> = sp.get("pool").and_then(|p| p.as_array()).cloned().unwrap_or_default();
            if let Some(p) = sp.get("path").and_then(|p| p.as_str()).filter(|p| !p.is_empty()) { if pool.is_empty() { pool.push(json!(p)); } }
            if !pool.iter().any(|p| p.as_str() == Some(rel.as_str())) { pool.push(json!(rel)); }
            d["sprite"]["pool"] = json!(pool);
            d["sprite"]["path"] = json!("");
            if let Some(px) = args.get("pixelated").and_then(|b| b.as_bool()) { d["sprite"]["pixelated"] = json!(px); }
        }
        if let Some(hm) = num(args, "height") { d["height"] = json!(hm); }
        if let Some(a) = string(args, "anchor") { d["anchor"] = json!(a); }
        self.write_kit(&dir, v)?;
        extra.insert("prop".into(), json!(def));
        extra.insert("created".into(), json!(new));
        let mut a = args.clone();
        a["only"] = json!(def);
        self.kit_reply(kit, &dir, &a, extra)
    }

    fn pack(&mut self, args: &Value) -> ToolResult {
        let p = string(args, "path").ok_or("`path`: the scene file to pack")?;
        let file = self.resolve(p);
        let (scene, _) = crate::scene::load_file(&file)?;
        let base = file.parent().map(Path::to_path_buf).unwrap_or_else(|| self.home.clone());
        let out = string(args, "out_dir").map(|o| self.resolve(o)).unwrap_or_else(|| base.clone());
        let name = string(args, "name").map(str::to_owned)
            .unwrap_or_else(|| file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "scene".into()));
        if out == base && name == file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default() && !flag(args, "in_place", true) {
            return Err("packing into the scene's own folder rewrites the scene file; pass in_place: true or give out_dir".into());
        }
        let rep = crate::project::pack(&scene, &base, &out, &name)?;
        Ok(Reply::json(&json!({
            "scene": rep.scene, "copied": rep.copied.iter().map(|(a, b)| json!({"from": a, "to": b})).collect::<Vec<_>>(),
            "rewritten": rep.rewritten.iter().map(|(p, a, b)| json!({"pointer": p, "from": a, "to": b})).collect::<Vec<_>>(),
            "missing": rep.missing,
            "note": if rep.missing.is_empty() { "everything the scene uses is now inside the folder, named relative to the scene" } else { "some files the scene names were not found and were left as they are" },
        })))
    }

    fn export(&mut self, args: &Value) -> ToolResult {
        let l = self.load(args)?;
        let formats = match args.get("formats").and_then(|v| v.as_array()) {
            Some(a) => a.iter().map(|f| f.as_str().and_then(Format::parse).ok_or_else(|| format!("unknown format {f} (gif, webp, apng, png, sheet, depth, layers)"))).collect::<Result<Vec<_>, _>>()?,
            None => vec![Format::Gif],
        };
        let size = match args.get("size").and_then(|v| v.as_array()) {
            Some(a) if a.len() == 2 => Some((a[0].as_u64().unwrap_or(0).clamp(16, 4096) as u32, a[1].as_u64().unwrap_or(0).clamp(16, 4096) as u32)),
            _ => None,
        };
        let job = ExportJob {
            formats, out_dir: self.resolve(string(args, "out_dir").unwrap_or("exports")),
            name: string(args, "name").unwrap_or("").to_owned(),
            frames: int(args, "frames").map(|f| f.clamp(2, 4000) as u32), size,
            gif_dither: flag(args, "gif_dither", true), gif_lossy: num(args, "gif_lossy").unwrap_or(0.33) as f32,
            webp_quality: num(args, "webp_quality").map(|q| q as f32), sheet_columns: None,
            sheet_max: int(args, "sheet_max").map(|m| m.clamp(64, 16384) as u32),
            layer_bands: args.get("layer_bands").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_f64()).map(|x| x as f32).collect()).unwrap_or_default(),
            base_dir: Some(self.base_of(&l)), gif_palette: None,
        };
        let cancel = AtomicBool::new(false);
        let report = match string(args, "clip").unwrap_or("loop") {
            "loop" => export::export(&l.scene, &job, |_| {}, &cancel)?,
            "encounter" => {
                let e = export::Encounter { slow_m: num(args, "slow_m").unwrap_or(3.0) as f32, start_frame: int(args, "start_frame").unwrap_or(0).max(0) as u32 };
                export::export_encounter(&l.scene, &job, &e, |_| {}, &cancel)?
            }
            "transition" => {
                let to = match (string(args, "to_path"), string(args, "to_preset")) {
                    (Some(p), _) => self.load(&json!({"path": p}))?,
                    (_, Some(p)) => self.load(&json!({"preset": p}))?,
                    _ => return Err("transition needs to_path or to_preset".into()),
                };
                let tr = export::Transition { approach_m: num(args, "approach_m").unwrap_or(0.0) as f32, ..Default::default() };
                let b_dir = to.file.as_ref().and_then(|f| f.parent().map(Path::to_path_buf));
                export::export_transition(&l.scene, &to.scene, b_dir, &job, &tr, |_| {}, &cancel)?
            }
            other => return Err(format!("unknown clip '{other}' (loop, encounter, transition)")),
        };
        let sizes: Vec<Value> = report.files.iter().map(|f| {
            let bytes = std::fs::metadata(f).map(|m| if m.is_dir() { dir_size(f) } else { m.len() }).unwrap_or(0);
            json!({"file": f, "kb": bytes / 1024})
        }).collect();
        Ok(Reply::json(&json!({"source": l.label, "report": clean(&report), "sizes": sizes})))
    }
}

impl Default for Server { fn default() -> Self { Self::new() } }

fn dir_size(p: &Path) -> u64 {
    std::fs::read_dir(p).map(|rd| rd.filter_map(|e| e.ok()).filter_map(|e| e.metadata().ok()).map(|m| m.len()).sum()).unwrap_or(0)
}

fn round1(v: f32) -> f64 { (v as f64 * 10.0).round() / 10.0 }
fn round2(v: f32) -> f64 { (v as f64 * 100.0).round() / 100.0 }
fn round3(v: f32) -> f64 { (v as f64 * 1000.0).round() / 1000.0 }

/// Distance ahead of the ground point seen on screen row `row` (path centre): the nearest piece
/// of ground that covers the row and faces the camera. Steps hide what is behind them, and a step
/// above eye level shows only its riser.
pub(crate) fn ground_distance(v: &crate::world::View, row: f32) -> Option<f32> {
    let y_at = |d: f32| v.world_to_px(0.0, 0.0, d).map(|p| p[1]);
    let (lo, hi) = (0.06f32, 2000.0f32);
    if row > y_at(lo)? { return Some(lo); }
    // A stretch of level ground between a and b (no riser inside) covers the rows from y(a) up to
    // y(b) when it is below the eye; the first row reached is narrowed down by bisection.
    let flat = |a: f32, b: f32| -> Option<f32> {
        if v.to_cam(0.0, 0.0, 0.5 * (a + b))[1] >= 0.0 { return None; }
        let (ya, yb) = (y_at(a)?, y_at(b)?);
        // A twentieth of a pixel of slack: the stretch stops a hair short of its riser, and a row at
        // the very edge of the step must still find it.
        if !(ya >= row - 0.05 && row >= yb - 0.05) { return None; }
        let (mut a, mut b) = (a, b);
        for _ in 0..40 {
            let mid = 0.5 * (a + b);
            if y_at(mid)? > row { a = mid } else { b = mid }
        }
        Some(b)
    };
    let risers: Vec<f32> = v.stairs.map(|st| st.risers(v.scroll + lo, v.scroll + hi).into_iter().map(|e| e - v.scroll).collect()).unwrap_or_default();
    const E: f32 = 1e-4;
    let (mut a, mut next) = (lo, 0usize);
    while a < hi {
        let b = (a * 1.01 + 0.002).min(hi);
        if next < risers.len() && risers[next] <= b {
            let e = risers[next];
            next += 1;
            if let Some(d) = flat(a, e - E) { return Some(d); }
            // The riser covers the rows between the two steps; it faces the camera only going up.
            let (below, above) = (y_at(e - E)?, y_at(e + E)?);
            if above < below && row <= below && row >= above { return Some(e); }
            a = e + E;
            continue;
        }
        if let Some(d) = flat(a, b) { return Some(d); }
        a = b;
    }
    None
}

fn collect_refs(v: &Value, defs: &Map<String, Value>, out: &mut Map<String, Value>, depth: u32) {
    match v {
        Value::Object(o) => {
            if let Some(r) = o.get("$ref").and_then(|r| r.as_str()) {
                let name = r.rsplit('/').next().unwrap_or("");
                if !out.contains_key(name) {
                    if let Some(d) = defs.get(name) {
                        out.insert(name.to_owned(), d.clone());
                        if depth > 0 { collect_refs(d, defs, out, depth - 1); }
                    }
                }
            }
            for x in o.values() { collect_refs(x, defs, out, depth); }
        }
        Value::Array(a) => for x in a { collect_refs(x, defs, out, depth) },
        _ => {}
    }
}

/// Default values of one section, so a new list entry can be written from them.
pub fn default_of(section: &str) -> Value {
    use crate::scene::*;
    let v = match section {
        "Fixture" => serde_json::to_value(Fixture::default()),
        "PropLayer" => serde_json::to_value(PropLayer::default()),
        "PropDef" => serde_json::to_value(PropDef::default()),
        "ShapePart" => serde_json::to_value(ShapePart::default()),
        "PropLight" => serde_json::to_value(PropLight::default()),
        "Particles" => serde_json::to_value(Particles::default()),
        "SetPiece" => serde_json::to_value(SetPiece::default()),
        "Material" => serde_json::to_value(Material::default()),
        "Camera" => serde_json::to_value(Camera::default()),
        "PathShape" => serde_json::to_value(PathShape::default()),
        "Verge" => serde_json::to_value(Verge::default()),
        "Walls" => serde_json::to_value(Walls::default()),
        "Ceiling" => serde_json::to_value(Ceiling::default()),
        "Sky" => serde_json::to_value(Sky::default()),
        "Lighting" => serde_json::to_value(Lighting::default()),
        "Post" => serde_json::to_value(Post::default()),
        "Style" => serde_json::to_value(Style::default()),
        "Motion" => serde_json::to_value(Motion::default()),
        "Canvas" => serde_json::to_value(Canvas::default()),
        "Transition" => serde_json::to_value(transition::Transition::default()),
        "ForkChoice" => serde_json::to_value(transition::ForkChoice::default()),
        "Stop" => serde_json::to_value(crate::journey::Stop::default()),
        _ => return Value::Null,
    };
    v.unwrap_or(Value::Null)
}

/// Problems visible from the scene values alone.
pub fn scene_warnings(s: &Scene) -> Vec<String> {
    let mut w = Vec::new();
    let range = |w: &mut Vec<String>, name: &str, v: f32, lo: f32, hi: f32| {
        if v < lo || v > hi { w.push(format!("{name} = {v} is outside {lo}..{hi} and is clamped when rendering")); }
    };
    range(&mut w, "camera.horizon", s.camera.horizon, 0.02, 0.98);
    range(&mut w, "camera.zoom", s.camera.zoom, 0.1, 8.0);
    range(&mut w, "camera.lens_curve", s.camera.lens_curve, -1.0, 1.0);
    range(&mut w, "path.bend", s.path.bend, -2.0, 2.0);
    range(&mut w, "path.hill", s.path.hill, -2.0, 2.0);
    range(&mut w, "path.flare", s.path.flare, 0.0, 0.95);
    if s.motion.loop_length < 4.0 { w.push(format!("motion.loop_length {}m is very short: anything spaced wider than the loop repeats every loop anyway", s.motion.loop_length)); }
    let b = &s.path.bridge;
    if b.enabled && s.walls.enabled && s.walls.gap < 1.0 {
        w.push(format!("path.bridge has almost nothing to cross: the walls stand {} m from the path, so the gap beside the deck is that narrow. Set walls.gap to 1.5 m or more (or turn the walls off)", s.walls.gap));
    }
    if b.enabled && b.length >= b.spacing - 0.5 { w.push("path.bridge.length fills its whole spacing: the ground never comes back between bridges".into()); }
    let f = &s.path.fork;
    if f.enabled && f.side == crate::scene::ForkSide::Alternate {
        let n = (s.motion.loop_length / f.spacing.max(4.0)).round().max(1.0) as i64;
        if n % 2 == 1 { w.push(format!("path.fork alternates sides but {n} fork(s) fit in a loop: an odd number cannot alternate all the way round, so two neighbours share a side at the loop point (one per loop: always the left). Halve fork.spacing or pick a side")); }
    }
    if b.enabled && s.path.stairs.enabled { w.push("path.bridge and path.stairs together: a bridge over stairs is not modelled; the deck follows the steps".into()); }
    if s.motion.frames() > 600 { w.push(format!("{} frames per loop: exports will be large; shorten motion.loop_length or lower motion.fps", s.motion.frames())); }
    let len = s.motion.loop_length.max(1.0);
    for (i, f) in s.fixtures.iter().enumerate().filter(|(_, f)| f.enabled) {
        if f.spacing < 1.0 { w.push(format!("fixtures/{i}: spacing {}m puts a light every step; very bright and slow", f.spacing)); }
        if f.spacing > len { w.push(format!("fixtures/{i}: spacing {}m is longer than the loop ({len}m), so it is snapped to the loop length", f.spacing)); }
        for (j, p) in s.props.iter().enumerate() {
            let a = crate::world::render::snap_to_loop(f.spacing.max(0.5), len);
            let b = crate::world::render::snap_to_loop(p.spacing.max(0.5), len);
            let same_side = matches!((f.side, p.side), (crate::scene::Side::Both, _) | (_, crate::scene::Side::Both)) || f.side == p.side;
            if same_side && f.mount != crate::scene::Mount::Floating && (a - b).abs() < 0.01 && ((f.offset - p.offset).rem_euclid(a)).min((p.offset - f.offset).rem_euclid(a)) < 0.6 {
                w.push(format!("fixtures/{i} ({}) and props/{j} ({}) share spacing {a:.1}m and offset, so every light sits on a prop: shift one offset by about half the spacing", f.kind.name(), p.kind.name()));
            }
        }
    }
    for (i, p) in s.props.iter().enumerate() {
        if p.spacing < 0.3 { w.push(format!("props/{i}: spacing {}m is very dense (slow, and props overlap)", p.spacing)); }
    }
    w
}

// ── JSON-RPC over stdio ────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum Framing { Lines, ContentLength }

fn read_message(input: &mut impl BufRead) -> std::io::Result<Option<(String, Framing)>> {
    let mut line = String::new();
    loop {
        line.clear();
        if input.read_line(&mut line)? == 0 { return Ok(None); }
        let t = line.trim();
        if t.is_empty() { continue; }
        if t.len() > 15 && t[..15].eq_ignore_ascii_case("content-length:") {
            let n: usize = t[15..].trim().parse().map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "bad Content-Length"))?;
            // Skip the remaining headers up to the blank line.
            loop {
                line.clear();
                if input.read_line(&mut line)? == 0 { return Ok(None); }
                if line.trim().is_empty() { break; }
            }
            let mut buf = vec![0u8; n];
            input.read_exact(&mut buf)?;
            return Ok(Some((String::from_utf8_lossy(&buf).into_owned(), Framing::ContentLength)));
        }
        return Ok(Some((t.to_owned(), Framing::Lines)));
    }
}

fn write_message(out: &mut impl Write, v: &Value, framing: Framing) -> std::io::Result<()> {
    let body = serde_json::to_string(v).unwrap_or_default();
    match framing {
        Framing::Lines => writeln!(out, "{body}")?,
        Framing::ContentLength => write!(out, "Content-Length: {}\r\n\r\n{body}", body.len())?,
    }
    out.flush()
}

/// Answer one JSON-RPC message; None for notifications.
pub fn handle(server: &mut Option<Server>, msg: &Value) -> Option<Value> {
    let id = msg.get("id").cloned();
    let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    let result: Result<Value, (i64, String)> = match method {
        "initialize" => {
            let version = params.get("protocolVersion").and_then(|v| v.as_str()).unwrap_or("2025-06-18");
            Ok(json!({
                "protocolVersion": version,
                "capabilities": {"tools": {"listChanged": false}, "prompts": {"listChanged": false}, "resources": {"listChanged": false}},
                "serverInfo": {"name": "path_forge", "title": "PathForge", "version": env!("CARGO_PKG_VERSION")},
                "instructions": INSTRUCTIONS,
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": tools()})),
        "tools/call" => {
            let name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            // The renderer is built on first use, so initialize and tools/list answer at once.
            let s = server.get_or_insert_with(Server::new);
            let reply = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| s.call(name, &args)))
                .unwrap_or_else(|_| Reply::error(format!("{name} panicked; the scene may hold a value the renderer cannot handle")));
            Ok(reply.to_value())
        }
        "resources/list" => Ok(json!({"resources": [{
            "uri": skill::RESOURCE_URI, "name": skill::NAME, "title": "PathForge design guide",
            "description": skill::description(), "mimeType": "text/markdown", "size": skill::SKILL_MD.len(),
        }]})),
        "resources/read" => match params.get("uri").and_then(|u| u.as_str()) {
            Some(u) if u == skill::RESOURCE_URI => Ok(json!({"contents": [{"uri": u, "mimeType": "text/markdown", "text": skill::SKILL_MD}]})),
            other => Err((-32002, format!("resource not found: {}", other.unwrap_or("(no uri)")))),
        },
        "resources/templates/list" => Ok(json!({"resourceTemplates": []})),
        "prompts/list" => Ok(json!({"prompts": [{"name": skill::NAME, "title": "PathForge design loop", "description": skill::description(), "arguments": [
            {"name": "brief", "description": "What the background is for: biome, mood, style, output", "required": false},
        ]}]})),
        "prompts/get" => match params.get("name").and_then(|n| n.as_str()) {
            Some(n) if n == skill::NAME => {
                let brief = params.pointer("/arguments/brief").and_then(|b| b.as_str()).unwrap_or("").trim().to_string();
                let mut text = skill::SKILL_MD.to_string();
                if !brief.is_empty() { text += &format!("\n\n---\n\nFollow the guide above for this brief: {brief}"); }
                Ok(json!({"description": skill::description(), "messages": [{"role": "user", "content": {"type": "text", "text": text}}]}))
            }
            other => Err((-32602, format!("unknown prompt: {}", other.unwrap_or("(no name)")))),
        },
        m if m.starts_with("notifications/") => return None,
        _ => Err((-32601, format!("method not found: {method}"))),
    };
    id.as_ref()?;
    Some(match result {
        Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
        Err((code, message)) => json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}),
    })
}

pub fn serve_stdio() -> std::io::Result<()> {
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let stdout = std::io::stdout();
    let mut server: Option<Server> = None;
    while let Some((text, framing)) = read_message(&mut input)? {
        let reply = match serde_json::from_str::<Value>(&text) {
            Ok(msg) => handle(&mut server, &msg),
            Err(e) => Some(json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": format!("parse error: {e}")}})),
        };
        if let Some(r) = reply { write_message(&mut stdout.lock(), &r, framing)?; }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only_tools_are_marked_and_writers_are_not() {
        let writers = ["pf_new_scene", "pf_edit_scene", "pf_export", "pf_kit", "pf_import_sprite", "pf_pack", "pf_remix", "pf_transition", "pf_journey"];
        for t in tools() {
            let name = t["name"].as_str().unwrap();
            let ro = t["annotations"]["readOnlyHint"].as_bool().unwrap();
            assert_eq!(ro, !writers.contains(&name), "{name}");
        }
    }

    #[test]
    fn every_listed_tool_is_dispatched() {
        let mut s = Server::new();
        for t in tools() {
            let name = t["name"].as_str().unwrap();
            let r = s.call(name, &json!({"__probe": true}));
            let text = r.content[0]["text"].as_str().unwrap_or("");
            assert!(!text.starts_with("unknown tool"), "{name} is listed but not dispatched");
        }
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let err = to_scene(json!({"path": {"half_width": 1.0, "halfwidth": 2.0}})).unwrap_err();
        assert!(err.contains("/path/halfwidth"), "{err}");
        assert!(to_scene(json!({"path": {"half_width": 1.0}})).is_ok());
    }

    #[test]
    fn pointer_ops_edit_lists() {
        let mut doc = serde_json::to_value(find_preset("Stone Dungeon").unwrap()).unwrap();
        let n = doc["fixtures"].as_array().unwrap().len();
        apply_op(&mut doc, &json!({"op": "append", "pointer": "/fixtures", "value": {"kind": "Lantern"}})).unwrap();
        apply_op(&mut doc, &json!({"op": "set", "pointer": "/fixtures/0/intensity", "value": 2.0})).unwrap();
        apply_op(&mut doc, &json!({"op": "remove", "pointer": "/fixtures/0"})).unwrap();
        assert_eq!(doc["fixtures"].as_array().unwrap().len(), n);
        let s = to_scene(doc).unwrap();
        assert_eq!(s.fixtures.last().unwrap().kind, crate::scene::FixtureKind::Lantern);
    }

    #[test]
    fn rows_map_back_to_the_distance_they_came_from() {
        let s = find_preset("Forest Path").unwrap();
        let v = crate::world::View::new(&s, 480, 854);
        for d in [1.5f32, 4.0, 10.0, 30.0] {
            let row = v.world_to_px(0.0, 0.0, d).unwrap()[1];
            let back = ground_distance(&v, row).unwrap();
            assert!((back - d).abs() / d < 0.01, "{d} -> row {row} -> {back}");
        }
        // On stairs a row may show a nearer step instead (it hides what is behind), never a further one.
        for (preset, frames) in [("Tower Stair", 1), ("Stone Dungeon", 3)] {
            let mut s = find_preset(preset).unwrap();
            if !s.path.stairs.enabled { s.path.stairs = crate::scene::Stairs { enabled: true, descending: true, ..Default::default() }; }
            for f in 0..frames {
                let v = crate::world::View::new(&s, 480, 854).at(f as f32 * 1.7);
                let mut exact = 0;
                for k in 0..60 {
                    let d = 0.8 + k as f32 * 0.07;
                    let Some(p) = v.world_to_px(0.0, 0.0, d) else { continue };
                    if p[1] > 854.0 { continue; }
                    let back = ground_distance(&v, p[1]).unwrap();
                    assert!(back <= d * 1.01, "{preset}: {d} -> row {} -> {back}, further than the point itself", p[1]);
                    // What was found covers the row: a step there, or the riser (a band of rows) at a step's edge.
                    let (r0, r1) = (v.world_to_px(0.0, 0.0, back - 1e-3).unwrap()[1], v.world_to_px(0.0, 0.0, back + 1e-3).unwrap()[1]);
                    assert!(p[1] >= r0.min(r1) - 1.0 && p[1] <= r0.max(r1) + 1.0, "{preset}: {d} -> {back} covers rows {r0}..{r1}, not {}", p[1]);
                    if (back - d).abs() / d < 0.01 { exact += 1; }
                }
                assert!(exact > 10, "{preset}: only {exact} visible points mapped back");
            }
        }
    }
}
