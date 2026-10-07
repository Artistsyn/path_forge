//! `pf` — PathForge from a shell: render, look, check, export, plan transitions and forks,
//! export journeys, and manage kits, projects and the agent skill. docs/MANUAL.md explains each.
//!
//! Looking
//!   pf presets                                                   the built-in presets
//!   pf dump    (--preset NAME | --scene FILE)                    v3 scene JSON to stdout (2.0 files are converted)
//!   pf render  (--preset NAME | --scene FILE) [--t 0..1] -o out.png
//!   pf sheet   (--preset NAME | --scene FILE) [--frames 8] [--cols 4] [--scale 0.5] -o out.png
//!   pf gallery [--t 0..1] [--cols 5] [--scale 0.3] -o out.png    first frame of every preset
//!   pf styles  (--preset NAME | --scene FILE) [--t 0..1] [--scale 0.35] [--only NAME] -o out.png
//!   pf kit     --kit @NAME|FOLDER [--only NAME] [-o out.png]     every prop of a kit
//!   pf schema  [--markdown]                                      the scene format (JSON schema, or the reference)
//!
//! Checking
//!   pf seam    [--preset NAME | --scene FILE] [--frames 24] [--steps 1]   the loop joins (every preset by default)
//!   pf bench   [--preset NAME | --scene FILE] [--frames 10]
//!   pf animcheck (--preset NAME | --scene FILE) --file X.gif|X.webp [--frame K] [-o cmp.png]   an export vs fresh renders
//!   pf animsheet --file X.gif|X.webp [--frames 8] [--cols 8] [--scale 0.3] -o out.png
//!   pf gifdiff --a A --b B [--frame K] [-o cmp.png]
//!
//! Exporting
//!   pf export  (--preset NAME | --scene FILE) -o FOLDER [--formats gif,webp,apng,png,sheet,depth,layers]
//!              [--quality 0..100] [--frames N] [--name STEM] [--lossy 0..1] [--dither on|off]
//!              [--clip loop | encounter [--slow M] [--start FRAME]
//!                     | transition (--to-preset NAME | --to-scene FILE) [--approach M] [--entries N]
//!                     | fork (--left-preset NAME | --left-scene FILE) (--right-preset NAME | --right-scene FILE)]
//!
//! Transitions, forks, journeys
//!   pf transition (--preset NAME | --scene FILE) (--to-preset NAME | --to-scene FILE)
//!              [--threshold auto|open|doorway|cave|gate|portal] [--marker auto|none|archway|ruinedarch|gate|banners|portal]
//!              [--approach M] [--json] [--frames 12] [--cols 6] [--scale 0.3] [--frame K] [-o sheet.png]
//!   pf transition (--preset NAME | --scene FILE) (--left-preset NAME | --left-scene FILE) (--right-preset NAME | --right-scene FILE)
//!              [--take left|right] [--angle DEG] [--approach M] [--frames 12] [--cols 6] [--scale 0.3] [-o sheet.png]
//!   pf journey --file J.journey.json [--out FOLDER [--formats webp,...] [--entries N]]
//!
//! Projects and the skill
//!   pf project --new FOLDER [--preset NAME]                      scenes/ assets/ kits/ exports/ and a first scene
//!   pf assets  --scene FILE                                      every file a scene uses, and whether it is there
//!   pf pack    --scene FILE [--out FOLDER] [--name STEM]         copy what it uses beside it, paths made relative
//!   pf skill   [--install DIR|~ [--agent claude,codex,copilot,cursor|all] [--force yes]]
//!
//!   pf parity  [--preset NAME]                                   2.0 engine only: CPU vs GPU
//! `--engine v2` runs the PathForge 2.0 renderers (with `--backend cpu|gpu`) for comparison.

use path_forge::gpu_scene::{self, GpuSceneRenderer};
use path_forge::renderer::PathRenderer;
use path_forge::scene::{self, Scene};
use path_forge::settings::{presets as v2presets, PathForgeSettings};
use path_forge::world::{RenderOptions, WorldRenderer};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

#[derive(Clone)]
enum Item {
    V3(Scene, Option<PathBuf>),
    V2(PathForgeSettings),
}

struct Renderers {
    world: WorldRenderer,
    cpu: PathRenderer,
    gpu: Option<Result<GpuSceneRenderer, String>>,
    gpu_backend: bool,
    /// Render as the studio's preview does: with frame stats and pick ids.
    studio: bool,
}

impl Renderers {
    fn new(gpu_backend: bool) -> Self { Self { world: WorldRenderer::default(), cpu: PathRenderer::default(), gpu: None, gpu_backend, studio: false } }

    /// Size of a frame and the length of one loop in the item's own units.
    fn dims(item: &Item) -> (usize, usize, f32) {
        match item {
            Item::V3(s, _) => (s.canvas.width as usize, s.canvas.height as usize, s.motion.loop_length.max(1.0)),
            Item::V2(s) => (s.canvas.w(), s.canvas.h(), s.anim.loop_s.max(1) as f32),
        }
    }

    /// Render at loop position `pos` (0 .. loop length).
    fn frame(&mut self, item: &Item, pos: f32) -> Result<Vec<u8>, String> {
        match item {
            Item::V3(s, dir) => {
                let opts = RenderOptions { base_dir: dir.clone(), stats: self.studio, pick: self.studio, ..RenderOptions::default() };
                Ok(self.world.render(s, pos, &opts).rgba)
            }
            Item::V2(s) => {
                let global_t = pos / s.anim.loop_s.max(1) as f32;
                if !self.gpu_backend { return Ok(self.cpu.render_to_new_buf(s, pos, global_t)); }
                let gpu = self.gpu.get_or_insert_with(GpuSceneRenderer::new);
                let g = gpu.as_mut().map_err(|e| format!("GPU unavailable: {e}"))?;
                let mut buf = g.render_scene_rgba(s, pos, global_t)?;
                if gpu_scene::has_sprite_instances(s) {
                    gpu_scene::composite_sprite_overlay(&mut buf, s.canvas.w() as u32, s.canvas.h() as u32, s, pos);
                }
                Ok(buf)
            }
        }
    }
}

/// Mean absolute RGB difference (0..255) and the share of pixels whose largest channel differs by more than `thresh`.
fn diff(a: &[u8], b: &[u8], thresh: u8) -> (f64, f64) {
    let mut sum = 0u64;
    let mut over = 0u64;
    let n = (a.len() / 4).max(1);
    for (pa, pb) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
        let mut m = 0u8;
        for c in 0..3 {
            let d = pa[c].abs_diff(pb[c]);
            sum += d as u64;
            m = m.max(d);
        }
        if m > thresh { over += 1; }
    }
    (sum as f64 / (n as f64 * 3.0), over as f64 / n as f64)
}

struct Args { cmd: String, opts: HashMap<String, String> }

fn parse_args() -> Result<Args, String> {
    let mut it = std::env::args().skip(1).peekable();
    let cmd = it.next().ok_or_else(usage)?;
    let mut opts = HashMap::new();
    // A flag followed by another flag (or by nothing) stands alone (`--json`, `--markdown`);
    // a value may still start with '-' when it is a number (`--approach -5`).
    let is_flag = |a: &str| a.starts_with('-') && !a[1..].starts_with(|c: char| c.is_ascii_digit() || c == '.');
    while let Some(a) = it.next() {
        let key = a.strip_prefix("--").or_else(|| a.strip_prefix('-'))
            .ok_or_else(|| format!("unexpected argument `{a}`\n{}", usage()))?;
        let val = match it.peek() { Some(n) if !is_flag(n) => it.next().unwrap_or_default(), _ => "true".to_owned() };
        opts.insert(key.to_owned(), val);
    }
    Ok(Args { cmd, opts })
}

fn usage() -> String {
    "usage: pf <presets|dump|render|sheet|gallery|styles|kit|schema|seam|bench|animcheck|animsheet|gifdiff|export|transition|journey|project|assets|pack|skill|parity> [--preset NAME | --scene FILE] [options]  (see src/bin/pf.rs)".into()
}

fn norm(s: &str) -> String { s.to_lowercase().replace([' ', '_', '-'], "") }

fn v2(args: &Args) -> bool { args.opts.get("engine").is_some_and(|e| e == "v2") }

/// The scenes a command runs over: one named preset, one scene file, or every built-in preset.
fn items(args: &Args) -> Result<Vec<(String, Item)>, String> {
    if let Some(path) = args.opts.get("scene") {
        let txt = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        let dir = std::path::Path::new(path).parent().map(|p| p.to_path_buf());
        if v2(args) {
            let s: PathForgeSettings = serde_json::from_str(&txt).map_err(|e| format!("{path}: {e}"))?;
            return Ok(vec![(path.clone(), Item::V2(s))]);
        }
        return Ok(vec![(path.clone(), Item::V3(Scene::from_json(&txt).map_err(|e| format!("{path}: {e}"))?, dir))]);
    }
    let want = args.opts.get("preset").map(|n| norm(n));
    let list: Vec<(String, Item)> = if v2(args) {
        v2presets::ALL.iter().map(|(n, make)| (n.to_string(), Item::V2(make()))).collect()
    } else {
        scene::presets::ALL.iter().map(|(n, make)| (n.to_string(), Item::V3(make(), None))).collect()
    };
    match want {
        Some(w) => {
            let found: Vec<_> = list.into_iter().filter(|(n, _)| norm(n) == w).collect();
            if found.is_empty() { Err(format!("no preset `{}`; run `pf presets`", args.opts["preset"])) } else { Ok(found) }
        }
        None => Ok(list),
    }
}

fn one_item(args: &Args) -> Result<(String, Item), String> {
    if !args.opts.contains_key("scene") && !args.opts.contains_key("preset") {
        return Err("this command needs --preset NAME or --scene FILE".into());
    }
    Ok(items(args)?.remove(0))
}

fn opt<T: std::str::FromStr>(args: &Args, key: &str, default: T) -> Result<T, String> {
    match args.opts.get(key) {
        Some(v) => v.parse().map_err(|_| format!("--{key}: cannot read `{v}`")),
        None => Ok(default),
    }
}

fn gpu_backend(args: &Args) -> Result<bool, String> {
    match args.opts.get("backend").map(String::as_str).unwrap_or("cpu") {
        "cpu" => Ok(false),
        "gpu" => Ok(true),
        other => Err(format!("--backend: expected cpu or gpu, got `{other}`")),
    }
}

fn save_png(path: &str, w: usize, h: usize, rgba: &[u8]) -> Result<(), String> {
    image::save_buffer(path, rgba, w as u32, h as u32, image::ExtendedColorType::Rgba8)
        .map_err(|e| format!("{path}: {e}"))
}

fn cmd_render(args: &Args) -> Result<(), String> {
    let (name, item) = one_item(args)?;
    let out = args.opts.get("o").or(args.opts.get("out")).ok_or("render needs -o out.png")?;
    let t: f32 = opt(args, "t", 0.0)?;
    let mut r = Renderers::new(gpu_backend(args)?);
    let (w, h, len) = Renderers::dims(&item);
    let t0 = Instant::now();
    let buf = r.frame(&item, t * len)?;
    save_png(out, w, h, &buf)?;
    println!("{name}: wrote {out} ({w}x{h}, t={t}, {:.1} ms)", t0.elapsed().as_secs_f64() * 1000.0);
    Ok(())
}

/// Several frames of one loop in a grid, so motion and the loop point can be judged in one image.
fn cmd_sheet(args: &Args) -> Result<(), String> {
    let (name, item) = one_item(args)?;
    let out = args.opts.get("o").or(args.opts.get("out")).ok_or("sheet needs -o out.png")?;
    let frames: usize = opt(args, "frames", 8usize)?.max(1);
    let cols: usize = opt(args, "cols", 4usize)?.max(1);
    let scale: f32 = opt(args, "scale", 0.5f32)?.clamp(0.05, 1.0);
    let mut r = Renderers::new(gpu_backend(args)?);
    let (w, h, len) = Renderers::dims(&item);
    let (tw, th) = (((w as f32) * scale).round().max(1.0) as usize, ((h as f32) * scale).round().max(1.0) as usize);
    let rows = frames.div_ceil(cols);
    let gap = 4;
    let (sw, sh) = (cols * tw + (cols + 1) * gap, rows * th + (rows + 1) * gap);
    let mut sheet = vec![0u8; sw * sh * 4];
    for px in sheet.chunks_exact_mut(4) { px.copy_from_slice(&[40, 40, 48, 255]); }
    for i in 0..frames {
        let buf = r.frame(&item, len * i as f32 / frames as f32)?;
        let (ox, oy) = (gap + (i % cols) * (tw + gap), gap + (i / cols) * (th + gap));
        for y in 0..th {
            // Box-filter each sheet pixel so thumbnails do not alias.
            let (sy0, sy1) = (y * h / th, ((y + 1) * h / th).max(y * h / th + 1).min(h));
            for x in 0..tw {
                let (sx0, sx1) = (x * w / tw, ((x + 1) * w / tw).max(x * w / tw + 1).min(w));
                let mut acc = [0u32; 3];
                let mut n = 0u32;
                for sy in sy0..sy1 { for sx in sx0..sx1 {
                    let si = (sy * w + sx) * 4;
                    for c in 0..3 { acc[c] += buf[si + c] as u32; }
                    n += 1;
                }}
                let di = ((oy + y) * sw + ox + x) * 4;
                for c in 0..3 { sheet[di + c] = (acc[c] / n.max(1)) as u8; }
            }
        }
    }
    save_png(out, sw, sh, &sheet)?;
    println!("{name}: wrote {out} ({frames} frames, {cols} columns)");
    Ok(())
}

/// The loop point should look like every other frame step. Reports:
///   exact — frame at the loop length vs frame at 0 (must be ~0 for a seamless loop)
///   step  — mean difference between consecutive frames (normal motion)
///   wrap  — difference between the last frame and the first (what the viewer sees at the loop point)
fn cmd_seam(args: &Args) -> Result<(), String> {
    let frames: usize = opt(args, "frames", 24usize)?.max(2);
    let mut r = Renderers::new(gpu_backend(args)?);
    println!("{:<16} {:>9} {:>8} {:>9} {:>9} {:>7}  verdict", "scene", "exact", "exact%", "step", "wrap", "wrap/st");
    let mut failures = 0;
    for (name, item) in items(args)? {
        let (_, _, len) = Renderers::dims(&item);
        let mut prev: Option<Vec<u8>> = None;
        let mut first: Option<Vec<u8>> = None;
        let mut step_sum = 0.0;
        let mut steps = Vec::new();
        for i in 0..frames {
            let f = r.frame(&item, len * i as f32 / frames as f32)?;
            if let Some(p) = &prev {
                let d = diff(p, &f, 8).0;
                step_sum += d;
                steps.push(d);
            }
            if first.is_none() { first = Some(f.clone()); }
            prev = Some(f);
        }
        let first = first.unwrap();
        let last = prev.unwrap();
        let end = r.frame(&item, len)?;
        let (exact, exact_px) = diff(&end, &first, 8);
        let step = step_sum / (frames - 1) as f64;
        let wrap = diff(&last, &first, 8).0;
        let ratio = if step > 1e-9 { wrap / step } else { 0.0 };
        let ok = exact < 0.05 && exact_px < 0.0005;
        if !ok { failures += 1; }
        println!("{:<16} {:>9.3} {:>7.2}% {:>9.3} {:>9.3} {:>7.2}  {}",
            name, exact, exact_px * 100.0, step, wrap, ratio, if ok { "seamless" } else { "JUMP AT LOOP" });
        if args.opts.get("steps").is_some_and(|v| v != "0") {
            let list: Vec<String> = steps.iter().chain(std::iter::once(&wrap)).map(|d| format!("{d:.1}")).collect();
            println!("    steps 0->1 .. last->first: {}", list.join(" "));
        }
    }
    if failures > 0 { println!("{failures} scene(s) jump at the loop point"); }
    Ok(())
}

fn cmd_parity(args: &Args) -> Result<(), String> {
    let mut cpu = Renderers::new(false);
    let mut gpu = Renderers::new(true);
    println!("{:<16} {:>6} {:>9} {:>9}", "scene", "t", "mean", "px>16");
    let mut a = Args { cmd: args.cmd.clone(), opts: args.opts.clone() };
    a.opts.insert("engine".into(), "v2".into());
    for (name, item) in items(&a)? {
        let (_, _, len) = Renderers::dims(&item);
        for t in [0.0f32, 0.37] {
            let c = cpu.frame(&item, t * len)?;
            let g = gpu.frame(&item, t * len)?;
            let (mean, over) = diff(&c, &g, 16);
            println!("{:<16} {:>6.2} {:>9.3} {:>8.2}%", name, t, mean, over * 100.0);
        }
    }
    Ok(())
}

fn cmd_bench(args: &Args) -> Result<(), String> {
    let frames: usize = opt(args, "frames", 10usize)?.max(1);
    let mut r = Renderers::new(gpu_backend(args)?);
    r.studio = args.opts.contains_key("studio");
    println!("{:<16} {:>10}", "scene", "ms/frame");
    for (name, item) in items(args)? {
        let (_, _, len) = Renderers::dims(&item);
        r.frame(&item, 0.0)?; // warm caches
        let stages = args.opts.contains_key("stages");
        r.world.profile = stages;
        r.world.stages.clear();
        let t0 = Instant::now();
        for i in 0..frames {
            r.frame(&item, len * (i as f32 + 0.5) / frames as f32)?;
        }
        println!("{:<16} {:>10.1}", name, t0.elapsed().as_secs_f64() * 1000.0 / frames as f64);
        if stages {
            // Loudest first: where the frame's time goes.
            let mut st = r.world.stages.clone();
            st.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
            let line: Vec<String> = st.iter().filter(|(_, ms)| *ms / frames as f64 >= 0.05).map(|(n, ms)| format!("{n} {:.1}", ms / frames as f64)).collect();
            println!("    {}", line.join(" | "));
        }
    }
    Ok(())
}

/// First frame of every scene side by side, for reviewing presets at a glance.
fn cmd_gallery(args: &Args) -> Result<(), String> {
    let out = args.opts.get("o").or(args.opts.get("out")).ok_or("gallery needs -o out.png")?;
    let cols: usize = opt(args, "cols", 5usize)?.max(1);
    let scale: f32 = opt(args, "scale", 0.3f32)?.clamp(0.05, 1.0);
    let t: f32 = opt(args, "t", 0.0)?;
    let list = items(args)?;
    let mut r = Renderers::new(gpu_backend(args)?);
    let (w0, h0, _) = Renderers::dims(&list[0].1);
    let (tw, th) = ((w0 as f32 * scale) as usize, (h0 as f32 * scale) as usize);
    let rows = list.len().div_ceil(cols);
    let gap = 4;
    let (sw, sh) = (cols * tw + (cols + 1) * gap, rows * th + (rows + 1) * gap);
    let mut sheet = vec![0u8; sw * sh * 4];
    for px in sheet.chunks_exact_mut(4) { px.copy_from_slice(&[40, 40, 48, 255]); }
    for (i, (_, item)) in list.iter().enumerate() {
        let (w, h, len) = Renderers::dims(item);
        let buf = r.frame(item, t * len)?;
        let (ox, oy) = (gap + (i % cols) * (tw + gap), gap + (i / cols) * (th + gap));
        for y in 0..th {
            let (sy0, sy1) = (y * h / th, ((y + 1) * h / th).max(y * h / th + 1).min(h));
            for x in 0..tw {
                let (sx0, sx1) = (x * w / tw, ((x + 1) * w / tw).max(x * w / tw + 1).min(w));
                let mut acc = [0u32; 3];
                let mut n = 0u32;
                for sy in sy0..sy1 { for sx in sx0..sx1 {
                    let si = (sy * w + sx) * 4;
                    for c in 0..3 { acc[c] += buf[si + c] as u32; }
                    n += 1;
                }}
                let di = ((oy + y) * sw + ox + x) * 4;
                for c in 0..3 { sheet[di + c] = (acc[c] / n.max(1)) as u8; }
            }
        }
    }
    save_png(out, sw, sh, &sheet)?;
    println!("wrote {out}: {} scenes in reading order: {}", list.len(), list.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>().join(", "));
    Ok(())
}

fn cmd_gifdiff(args: &Args) -> Result<(), String> {
    use path_forge::review;
    let a = review::decode_animation(std::path::Path::new(args.opts.get("a").ok_or("--a A.gif")?))?;
    let b = review::decode_animation(std::path::Path::new(args.opts.get("b").ok_or("--b B.gif")?))?;
    if a.len() != b.len() || a.first().map(|i| (i.width, i.height)) != b.first().map(|i| (i.width, i.height)) {
        return Err(format!("different GIFs: {} vs {} frames", a.len(), b.len()));
    }
    let diffs: Vec<(f64, f64)> = a.iter().zip(&b).map(|(x, y)| review::diff(&x.rgba, &y.rgba, 24)).collect();
    let mean = diffs.iter().map(|d| d.0).sum::<f64>() / diffs.len().max(1) as f64;
    let (worst, wd) = diffs.iter().enumerate().fold((0, (0.0, 0.0)), |m, (i, d)| if d.0 > m.1 .0 { (i, *d) } else { m });
    let over = diffs.iter().map(|d| d.1).sum::<f64>() / diffs.len().max(1) as f64;
    println!("{} frames: mean abs difference {mean:.2}/255, {:.2}% of pixels off by more than 24; worst frame {worst} ({:.2}, {:.2}%)", a.len(), over * 100.0, wd.0, wd.1 * 100.0);
    if let Some(out) = args.opts.get("o") {
        let k = args.opts.get("frame").and_then(|f| f.parse().ok()).unwrap_or(worst).min(a.len() - 1);
        let sheet = review::sheet(&[a[k].clone(), b[k].clone(), review::diff_image(&a[k], &b[k], 4.0)], 3);
        std::fs::write(out, review::png_bytes(&sheet)?).map_err(|e| e.to_string())?;
        println!("frame {k}: {out}");
    }
    Ok(())
}

fn cmd_animsheet(args: &Args) -> Result<(), String> {
    use path_forge::review;
    let frames = review::decode_animation(std::path::Path::new(args.opts.get("file").ok_or("--file X.webp")?))?;
    let k: usize = args.opts.get("frames").and_then(|v| v.parse().ok()).unwrap_or(8).clamp(1, frames.len());
    let scale: f32 = args.opts.get("scale").and_then(|v| v.parse().ok()).unwrap_or(0.3);
    let cols: usize = args.opts.get("cols").and_then(|v| v.parse().ok()).unwrap_or(k.min(8));
    let picked: Vec<_> = (0..k).map(|i| review::downscale(&frames[i * frames.len() / k], scale)).collect();
    let out = args.opts.get("o").ok_or("-o out.png")?;
    std::fs::write(out, review::png_bytes(&review::sheet(&picked, cols))?).map_err(|e| e.to_string())?;
    println!("{} of {} frames: {out}", k, frames.len());
    Ok(())
}

fn cmd_styles(args: &Args) -> Result<(), String> {
    use path_forge::review;
    let (scene, dir) = match one_item(args)?.1 { Item::V3(s, d) => (s, d), Item::V2(_) => return Err("styles needs a v3 scene".into()) };
    let t: f32 = args.opts.get("t").and_then(|v| v.parse().ok()).unwrap_or(0.0);
    let scale: f32 = args.opts.get("scale").and_then(|v| v.parse().ok()).unwrap_or(0.35);
    let out = args.opts.get("o").ok_or("-o out.png")?;
    let mut r = WorldRenderer::default();
    let mut imgs = Vec::new();
    let only = args.opts.get("only").map(|o| o.to_lowercase());
    for st in scene::styles::ALL.iter().filter(|st| only.as_ref().is_none_or(|o| st.name.to_lowercase().contains(o.as_str()))) {
        let mut s = scene.clone();
        (st.apply)(&mut s);
        let t0 = Instant::now();
        let img = r.render(&s, t * s.motion.loop_length, &RenderOptions { base_dir: dir.clone(), ..RenderOptions::default() });
        println!("{:<20} {:>6.1} ms", st.name, t0.elapsed().as_secs_f64() * 1000.0);
        imgs.push(review::downscale(&img, scale));
    }
    std::fs::write(out, review::png_bytes(&review::sheet(&imgs, imgs.len().min(5)))?).map_err(|e| e.to_string())?;
    println!("{out}");
    Ok(())
}

/// `pf kit --kit @wayside|DIR [-o sheet.png] [--preset P | --scene F] [--scale 0.35] [--cols 4]`:
/// list a kit's props and render each one beside a path.
fn cmd_kit(args: &Args) -> Result<(), String> {
    use path_forge::review;
    let kit = args.opts.get("kit").ok_or("--kit @NAME or a kit folder")?;
    let (backdrop, base) = if args.opts.contains_key("scene") || args.opts.contains_key("preset") {
        match one_item(args)?.1 { Item::V3(s, d) => (s, d), Item::V2(_) => return Err("kit needs a v3 scene as backdrop".into()) }
    } else { (review::kit_backdrop(), std::env::current_dir().ok()) };
    let scale: f32 = opt(args, "scale", 0.35f32)?;
    let cols: usize = opt(args, "cols", 4usize)?;
    let (img, names, warnings) = review::kit_sheet(&mut WorldRenderer::default(), kit, args.opts.get("only").map(|s| s.as_str()), &backdrop, base, scale, cols)?;
    for (i, (n, d)) in names.iter().enumerate() { println!("{:>2}  {:<20} {}", i + 1, n, d); }
    for w in &warnings { println!("warning: {w}"); }
    if let Some(out) = args.opts.get("o") {
        std::fs::write(out, review::png_bytes(&img)?).map_err(|e| e.to_string())?;
        println!("{out}");
    }
    Ok(())
}

/// `pf project --new DIR [--preset P]`: a project folder (scenes, assets, kits, exports) with a
/// starting scene. Without --new, prints where PathForge keeps work by default.
fn cmd_project(args: &Args) -> Result<(), String> {
    match args.opts.get("new") {
        Some(dir) => {
            let file = path_forge::project::new_project(std::path::Path::new(dir), args.opts.get("preset").map(|s| s.as_str()).unwrap_or("Stone Dungeon"))?;
            println!("{}", file.display());
        }
        None => println!("{}", path_forge::project::default_home().display()),
    }
    Ok(())
}

/// `pf assets --scene F`: every file the scene uses, and whether it is there.
fn cmd_assets(args: &Args) -> Result<(), String> {
    let f = args.opts.get("scene").ok_or("--scene FILE")?;
    let file = std::path::Path::new(f);
    let (scene, _) = scene::load_file(file)?;
    let base = file.parent().map(|p| if p.as_os_str().is_empty() { std::path::Path::new(".") } else { p }).unwrap_or(std::path::Path::new("."));
    let list = path_forge::project::assets(&scene, base);
    if list.is_empty() { println!("the scene uses no files"); }
    for a in &list { println!("{:<8} {:<22} {:<40} {}", if a.exists { "ok" } else { "MISSING" }, a.pointer, a.written, a.path.display()); }
    Ok(())
}

/// `pf pack --scene F [--out DIR] [--name N]`: copy the scene and every file it uses into one
/// folder, named relative to the scene (default: gather them into the scene's own folder).
fn cmd_pack(args: &Args) -> Result<(), String> {
    let f = args.opts.get("scene").ok_or("--scene FILE")?;
    let file = std::path::Path::new(f);
    let (scene, _) = scene::load_file(file)?;
    let base = file.parent().filter(|p| !p.as_os_str().is_empty()).map(|p| p.to_path_buf()).unwrap_or_else(|| ".".into());
    let out = args.opts.get("out").map(std::path::PathBuf::from).unwrap_or_else(|| base.clone());
    let name = args.opts.get("name").cloned().unwrap_or_else(|| file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "scene".into()));
    let rep = path_forge::project::pack(&scene, &base, &out, &name)?;
    for (a, b) in &rep.copied { println!("copied   {} -> {}", a.display(), b.display()); }
    for (p, a, b) in &rep.rewritten { println!("path     {p}: {a} -> {b}"); }
    for m in &rep.missing { println!("MISSING  {m}"); }
    println!("{}", rep.scene.display());
    Ok(())
}

fn cmd_animcheck(args: &Args) -> Result<(), String> {
    use path_forge::review;
    let file = args.opts.get("file").ok_or("--file X.gif|X.webp")?;
    let frames = review::decode_animation(std::path::Path::new(file))?;
    let (scene, dir) = match one_item(args)?.1 { Item::V3(s, d) => (s, d), Item::V2(_) => return Err("animcheck needs a v3 scene".into()) };
    let (w, h) = (frames[0].width, frames[0].height);
    let opts = RenderOptions { size: Some((w as u32, h as u32)), base_dir: dir, ..RenderOptions::default() };
    let mut r = WorldRenderer::default();
    let n = frames.len();
    let lp = scene.motion.loop_length.max(1.0);
    let mut worst = (0usize, 0.0f64);
    let (mut sum, mut over) = (0.0, 0.0);
    for (k, f) in frames.iter().enumerate() {
        let truth = r.render(&scene, lp * k as f32 / n as f32, &opts);
        let (d, o) = review::diff(&truth.rgba, &f.rgba, 24);
        sum += d; over += o;
        if d > worst.1 { worst = (k, d); }
    }
    let bytes = std::fs::metadata(file).map(|m| m.len()).unwrap_or(0);
    println!("{file}: {n} frames, {} KB ({:.1} KB/frame); against fresh renders: mean abs error {:.2}/255, {:.2}% of pixels off by more than 24; worst frame {} ({:.2})",
        bytes / 1024, bytes as f64 / 1024.0 / n as f64, sum / n as f64, over / n as f64 * 100.0, worst.0, worst.1);
    if let Some(out) = args.opts.get("o") {
        let k = args.opts.get("frame").and_then(|f| f.parse().ok()).unwrap_or(worst.0).min(n - 1);
        let truth = r.render(&scene, lp * k as f32 / n as f32, &opts);
        let sheet = review::sheet(&[truth.clone(), frames[k].clone(), review::diff_image(&truth, &frames[k], 4.0)], 3);
        std::fs::write(out, review::png_bytes(&sheet)?).map_err(|e| e.to_string())?;
        println!("frame {k} (render | file | difference x4): {out}");
    }
    Ok(())
}

fn cmd_export(args: &Args) -> Result<(), String> {
    let (name, item) = one_item(args)?;
    let Item::V3(scene, dir) = item else { return Err("export runs on v3 scenes".into()) };
    let out = args.opts.get("o").or(args.opts.get("out")).ok_or("export needs -o FOLDER")?;
    let formats = args.opts.get("formats").map(String::as_str).unwrap_or("gif")
        .split(',').map(|f| path_forge::export::Format::parse(f).ok_or_else(|| format!("unknown format `{f}` (gif, apng, png, sheet)")))
        .collect::<Result<Vec<_>, _>>()?;
    let job = path_forge::export::ExportJob {
        formats, out_dir: PathBuf::from(out), name: args.opts.get("name").cloned().unwrap_or_default(),
        frames: args.opts.get("frames").map(|f| f.parse()).transpose().map_err(|_| "--frames: not a number")?,
        gif_lossy: args.opts.get("lossy").map(|f| f.parse()).transpose().map_err(|_| "--lossy: not a number")?.unwrap_or(0.33),
        gif_dither: args.opts.get("dither").is_none_or(|v| v != "off"),
        webp_quality: args.opts.get("quality").map(|f| f.parse()).transpose().map_err(|_| "--quality: not a number")?,
        base_dir: dir, ..Default::default()
    };
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let report = match args.opts.get("clip").map(String::as_str).unwrap_or("loop") {
        "loop" => path_forge::export::export(&scene, &job, |_| {}, &cancel)?,
        "encounter" => {
            let e = path_forge::export::Encounter {
                slow_m: args.opts.get("slow").and_then(|v| v.parse().ok()).unwrap_or(3.0),
                start_frame: args.opts.get("start").and_then(|v| v.parse().ok()).unwrap_or(0),
            };
            path_forge::export::export_encounter(&scene, &job, &e, |_| {}, &cancel)?
        }
        "transition" => {
            let (b, b_dir) = if let Some(p) = args.opts.get("to-scene") {
                let pb = PathBuf::from(p);
                (scene::load_file(&pb)?.0, pb.parent().map(|d| d.to_path_buf()))
            } else {
                let name = args.opts.get("to-preset").ok_or("--clip transition needs --to-preset NAME or --to-scene FILE")?;
                (scene::presets::ALL.iter().find(|(n, _)| norm(n) == norm(name)).ok_or(format!("no preset {name}"))?.1(), None)
            };
            let tr = path_forge::export::Transition { approach_m: args.opts.get("approach").and_then(|v| v.parse().ok()).unwrap_or(0.0), ..Default::default() };
            let entries: u32 = args.opts.get("entries").and_then(|v| v.parse().ok()).unwrap_or(1);
            path_forge::export::export_transition_entries(&scene, &b, b_dir, &job, &tr, entries, |_| {}, &cancel)?
        }
        "fork" => {
            let side = |file: &str, preset: &str| -> Result<(Scene, Option<PathBuf>), String> {
                if let Some(p) = args.opts.get(file) { let pb = PathBuf::from(p); return Ok((scene::load_file(&pb)?.0, pb.parent().map(|d| d.to_path_buf()))); }
                let name = args.opts.get(preset).ok_or(format!("--clip fork needs --{preset} NAME or --{file} FILE"))?;
                Ok((scene::presets::ALL.iter().find(|(n, _)| norm(n) == norm(name)).ok_or(format!("no preset {name}"))?.1(), None))
            };
            let (l, ld) = side("left-scene", "left-preset")?;
            let (r, rd) = side("right-scene", "right-preset")?;
            path_forge::export::export_fork(&scene, (&l, ld), (&r, rd), &job, &Default::default(), |_| {}, &cancel)?
        }
        other => return Err(format!("--clip {other}: expected loop, encounter, transition or fork")),
    };
    println!("{name}: {} frames at {:.1} fps ({:.2} s loop, {:.3} m per frame) in {} ms", report.frames, report.fps, report.loop_seconds, report.metres_per_frame, report.elapsed_ms);
    for f in &report.files {
        let size = std::fs::metadata(f).map(|m| if m.is_dir() { String::from("folder") } else { format!("{:.0} KB", m.len() as f64 / 1024.0) }).unwrap_or_default();
        println!("  {} ({size})", f.display());
    }
    for n in &report.notes { println!("  note: {n}"); }
    Ok(())
}

/// Plan a transition (or a fork) between scenes, print what was chosen and why, and draw a
/// contact sheet of the walk.
fn cmd_transition(args: &Args) -> Result<(), String> {
    use path_forge::journey::{CrossWalk, ForkWalk, Place};
    use path_forge::scene::transition::{self as tr, Branch, ForkChoice, Marker, Threshold, Transition};
    let (_, item) = one_item(args)?;
    let Item::V3(a, a_dir) = item else { return Err("transition needs v3 scenes".into()) };
    let load = |file: &str, preset: &str| -> Result<Option<(Scene, Option<PathBuf>)>, String> {
        if let Some(f) = args.opts.get(file) {
            let p = PathBuf::from(f);
            return Ok(Some((scene::load_file(&p)?.0, p.parent().map(|d| d.to_path_buf()))));
        }
        match args.opts.get(preset) {
            Some(n) => Ok(Some((scene::presets::ALL.iter().find(|(m, _)| norm(m) == norm(n)).ok_or(format!("no preset {n}"))?.1(), None))),
            None => Ok(None),
        }
    };
    let frames: usize = opt(args, "frames", 12usize)?.max(2);
    let cols: usize = opt(args, "cols", 6usize)?.max(1);
    let scale: f32 = opt(args, "scale", 0.3f32)?.clamp(0.05, 1.0);
    let size = ((a.canvas.width as f32 * scale).round().max(16.0) as u32, (a.canvas.height as f32 * scale).round().max(16.0) as u32);
    let opts = RenderOptions { size: Some(size), ..RenderOptions::default() };
    let mut r = WorldRenderer::default();
    let mut imgs = Vec::new();
    let say = |notes: &[String], warnings: &[String]| {
        for n in notes { println!("  plan: {n}"); }
        for w in warnings { println!("  warning: {w}"); }
    };
    let pa = Place { scene: &a, dir: a_dir.as_deref() };
    if let (Some((l, ld)), Some((rt, rd))) = (load("left-scene", "left-preset")?, load("right-scene", "right-preset")?) {
        let mut f = ForkChoice::default();
        if let Some(v) = args.opts.get("angle") { f.angle = v.parse().map_err(|_| "--angle: not a number")?; }
        if let Some(v) = args.opts.get("approach") { f.approach_m = v.parse().map_err(|_| "--approach: not a number")?; }
        let plan = tr::plan_fork(&a, &l, &rt, &f);
        println!("fork from {} into {} (left) or {} (right):", a.name, l.name, rt.name);
        say(&plan.notes, &plan.warnings);
        let mut walk = ForkWalk::new(plan, 0.0, [0.0, 0.0]);
        let take = match args.opts.get("take").map(String::as_str) { Some("right") => Branch::Right, Some("left") => Branch::Left, _ => walk.plan.default_branch };
        let len = walk.length();
        let at = walk.decide_by() * 0.5;
        for i in 0..frames {
            let travel = len * i as f32 / (frames - 1) as f32;
            if travel >= at { walk.choose(take, at); }
            let t = travel / a.motion.speed.max(0.01);
            imgs.push(walk.frame(&mut r, pa, Place { scene: &l, dir: ld.as_deref() }, Place { scene: &rt, dir: rd.as_deref() }, travel, [t, t, t], &opts));
        }
    } else {
        let (b, bd) = load("to-scene", "to-preset")?.ok_or("transition needs --to-preset/--to-scene, or --left-preset and --right-preset for a fork")?;
        let mut t = Transition::default();
        if let Some(v) = args.opts.get("approach") { t.approach_m = v.parse().map_err(|_| "--approach: not a number")?; }
        if let Some(v) = args.opts.get("threshold") {
            t.threshold = match norm(v).as_str() {
                "open" => Threshold::Open, "doorway" | "door" => Threshold::Doorway, "cave" | "cavemouth" => Threshold::CaveMouth,
                "gate" => Threshold::Gate, "portal" => Threshold::Portal, "auto" => Threshold::Auto, _ => return Err(format!("--threshold {v}: open, doorway, cave, gate, portal or auto")),
            };
        }
        if let Some(v) = args.opts.get("marker") {
            t.marker = match norm(v).as_str() {
                "none" => Marker::None, "archway" | "arch" => Marker::Archway, "ruinedarch" => Marker::RuinedArch, "gate" => Marker::Gate,
                "banners" => Marker::Banners, "portal" => Marker::Portal, _ => Marker::Auto,
            };
        }
        let c = tr::plan(&a, &b, &t);
        println!("{} -> {}: {} (approach {:.0} m, overshoot {:.0} m{})", a.name, b.name, c.threshold.name(), c.approach, c.overshoot, if c.blend > 0.0 { format!(", blend {:.0} m", c.blend) } else { String::new() });
        say(&c.notes, &c.warnings);
        if args.opts.contains_key("json") { println!("{}", serde_json::to_string_pretty(&c).map_err(|e| e.to_string())?); }
        let walk = CrossWalk::new(c, 0.0, 0.0);
        let len = walk.length();
        for i in 0..frames {
            let travel = len * i as f32 / (frames - 1) as f32;
            let t = travel / a.motion.speed.max(0.01);
            imgs.push(walk.frame(&mut r, pa, Place { scene: &b, dir: bd.as_deref() }, travel, t, t, &opts));
        }
    }
    if let Some(k) = args.opts.get("frame") {
        // One frame of the walk, by its index among --frames.
        let k: usize = k.parse().map_err(|_| "--frame: not a number")?;
        imgs = vec![imgs.get(k).cloned().ok_or(format!("--frame {k}: only {} frames", imgs.len()))?];
    }
    if let Some(out) = args.opts.get("o") {
        std::fs::write(out, path_forge::review::png_bytes(&path_forge::review::sheet(&imgs, cols))?).map_err(|e| e.to_string())?;
        println!("  sheet: {out}");
    }
    Ok(())
}

/// Check a journey file, list where each stop leads, and with --out export everything it needs.
fn cmd_journey(args: &Args) -> Result<(), String> {
    use path_forge::journey::{Journey, Next};
    let file = args.opts.get("file").ok_or("journey needs --file J.journey.json")?;
    let path = PathBuf::from(file);
    let j = Journey::load(&path)?;
    let dir = path.parent().map(|d| d.to_path_buf());
    println!("{} (start: {})", if j.name.is_empty() { file.as_str() } else { j.name.as_str() }, j.start);
    for (id, st) in &j.stops {
        let next = match &st.next {
            Next::End => "end".to_string(),
            Next::Go { to, transition } => format!("-> {to} ({})", transition.threshold.name()),
            Next::Fork { left, right, .. } => format!("fork: {left} | {right}"),
        };
        println!("  {id}: {} {next}", st.scene);
    }
    let problems = j.problems(dir.as_deref());
    for p in &problems { println!("  problem: {p}"); }
    if let Some(out) = args.opts.get("out") {
        if !problems.is_empty() { return Err("fix the problems before exporting".into()); }
        let formats = args.opts.get("formats").map(String::as_str).unwrap_or("webp")
            .split(',').map(|f| path_forge::export::Format::parse(f).ok_or_else(|| format!("unknown format `{f}`")))
            .collect::<Result<Vec<_>, _>>()?;
        let job = path_forge::export::ExportJob { formats, out_dir: PathBuf::from(out), ..Default::default() };
        let entries: u32 = args.opts.get("entries").and_then(|v| v.parse().ok()).unwrap_or(1);
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let rep = path_forge::export::export_journey(&j, dir.as_deref(), &job, entries, |_| {}, &cancel)?;
        for f in &rep.files { println!("  {}", f.display()); }
        for n in &rep.notes { println!("  note: {n}"); }
    }
    Ok(())
}

fn cmd_dump(args: &Args) -> Result<(), String> {
    let (_, item) = one_item(args)?;
    let txt = match item {
        Item::V3(s, _) => serde_json::to_string_pretty(&s),
        Item::V2(s) => serde_json::to_string_pretty(&s),
    }.map_err(|e| e.to_string())?;
    println!("{txt}");
    Ok(())
}

/// The agent skill: printed, or installed where coding agents load skills.
///   pf skill                                  print SKILL.md
///   pf skill --install .                      this project: .claude/skills and .agents/skills
///   pf skill --install ~                      your home folder, for every project
///   pf skill --install DIR --agent all        also .github/skills (Copilot) and .cursor/skills
///   --force yes                               replace a copy someone has edited
fn cmd_skill(args: &Args) -> Result<(), String> {
    use path_forge::skill::{self, Agent, Installed};
    let Some(dir) = args.opts.get("install") else {
        if args.opts.contains_key("agent") { return Err("--agent needs --install DIR".into()); }
        print!("{}", skill::SKILL_MD);
        return Ok(());
    };
    let root = std::fs::canonicalize(dir).map_err(|e| format!("{dir}: {e}"))?;
    let home = std::env::var_os("HOME").and_then(|h| std::fs::canonicalize(h).ok());
    let user = home.as_deref() == Some(root.as_path());
    let agents = match args.opts.get("agent") { Some(a) => Agent::parse(a)?, None => Agent::DEFAULT.to_vec() };
    let force = matches!(args.opts.get("force").map(|s| s.as_str()), Some("yes" | "true" | "1"));
    let mut kept = 0;
    for r in skill::install(&agents, &root, user, force)? {
        match r {
            Installed::Written(p) => println!("wrote      {}", p.display()),
            Installed::Unchanged(p) => println!("up to date {}", p.display()),
            Installed::Kept(p) => { kept += 1; println!("kept       {} (edited by someone; --force yes replaces it)", p.display()) }
        }
    }
    if kept == 0 {
        println!("Agents that use MCP can also read it from pf_mcp without installing: prompt `pathforge`, resource {}.", skill::RESOURCE_URI);
    }
    Ok(())
}

fn main() -> ExitCode {
    let res = parse_args().and_then(|args| match args.cmd.as_str() {
        "presets" => {
            if v2(&args) { for (n, _) in v2presets::ALL { println!("{n}"); } }
            else { for (n, _) in scene::presets::ALL { println!("{n}"); } }
            Ok(())
        }
        "dump" => cmd_dump(&args),
        "render" => cmd_render(&args),
        "transition" => cmd_transition(&args),
        "journey" => cmd_journey(&args),
        "sheet" => cmd_sheet(&args),
        "seam" => cmd_seam(&args),
        "parity" => cmd_parity(&args),
        "bench" => cmd_bench(&args),
        "gallery" => cmd_gallery(&args),
        "export" => cmd_export(&args),
"gifdiff" => cmd_gifdiff(&args),
"animcheck" => cmd_animcheck(&args),
"styles" => cmd_styles(&args),
"animsheet" => cmd_animsheet(&args),
"skill" => cmd_skill(&args),
"kit" => cmd_kit(&args),
"pack" => cmd_pack(&args),
"assets" => cmd_assets(&args),
"project" => cmd_project(&args),
"schema" => { if args.opts.contains_key("markdown") { print!("{}", path_forge::docs::scene_reference()); } else { println!("{}", serde_json::to_string_pretty(&path_forge::docs::schema()).map_err(|e| e.to_string()).unwrap_or_default()); } Ok(()) }
        other => Err(format!("unknown command `{other}`\n{}", usage())),
    });
    match res {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => { eprintln!("pf: {e}"); ExitCode::FAILURE }
    }
}
