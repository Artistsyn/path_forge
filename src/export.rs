//! Exports for v3 scenes: GIF, APNG, PNG sequence, sprite sheet + atlas, and a metadata file a
//! game reads to play the loop by distance walked and to place things on the path.
//!
//! One render pass feeds every requested output. Files are written under temporary names and
//! renamed when complete, so a failed or cancelled export never leaves a truncated file behind.

use crate::scene::{Palette, Scene};
use crate::world::palette::{bayer, median_cut, PaletteLut};
use crate::world::view::{View, BEND_K, HILL_K};
use crate::world::{RenderOptions, WorldRenderer};
use crate::scene::Dither;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Format { Gif, Apng, Webp, Png, Sheet, Depth, Layers }

impl Format {
    pub fn parse(s: &str) -> Option<Format> {
        match s.trim().to_lowercase().as_str() {
            "gif" => Some(Format::Gif), "apng" => Some(Format::Apng), "webp" => Some(Format::Webp),
            "png" | "pngs" | "frames" => Some(Format::Png), "sheet" | "spritesheet" | "atlas" => Some(Format::Sheet),
            "depth" => Some(Format::Depth), "layers" => Some(Format::Layers),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ExportJob {
    pub formats: Vec<Format>,
    pub out_dir: PathBuf,
    /// File name stem; the scene name when empty.
    pub name: String,
    /// Frames in the loop; the scene's speed and fps decide when None.
    pub frames: Option<u32>,
    /// Output size; the scene canvas when None.
    pub size: Option<(u32, u32)>,
    /// Ordered dithering in GIFs (hides banding in skies and glows).
    pub gif_dither: bool,
    /// GIF size against fidelity, 0..1: a pixel keeps the colour already on screen while that colour
    /// is still within this much of the true rendered colour (OKLab distance 0..0.06), so it
    /// compresses as transparent. 0 = exact. Error is bounded against the render, so it never builds up.
    pub gif_lossy: f32,
    /// Animated WebP quality 0..100 (lossy); 100 writes lossless frames. None picks: lossless for
    /// palette-limited (pixel-art) scenes, where it is exact and smaller than GIF, else lossy 90.
    pub webp_quality: Option<f32>,
    pub sheet_columns: Option<u32>,
    /// Largest sprite sheet page, pixels per side (many phones cap textures at 4096). Default 4096.
    pub sheet_max: Option<u32>,
    /// Depth band boundaries in metres for `layers`, nearest first. Default [4, 12]: near, mid, far.
    pub layer_bands: Vec<f32>,
    /// Folder that relative sprite paths in the scene resolve against.
    pub base_dir: Option<PathBuf>,
    /// GIF colours to use instead of choosing them from the frames being written (an estimate
    /// encodes a few frames but must choose colours as the whole loop would).
    #[serde(skip)]
    pub gif_palette: Option<Arc<PaletteLut>>,
}

impl std::fmt::Debug for PaletteLut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "PaletteLut({} colours)", self.colors.len()) }
}
impl Default for ExportJob {
    fn default() -> Self {
        Self { formats: vec![Format::Gif], out_dir: PathBuf::from("."), name: String::new(), frames: None, size: None, gif_dither: true, gif_lossy: 0.0, webp_quality: None, sheet_columns: None, sheet_max: None, layer_bands: Vec::new(), base_dir: None, gif_palette: None }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ExportReport {
    pub files: Vec<PathBuf>,
    pub frames: u32,
    pub fps: f32,
    pub loop_seconds: f32,
    pub metres_per_frame: f32,
    pub width: usize,
    pub height: usize,
    pub elapsed_ms: u64,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Progress {
    pub stage: &'static str,
    pub done: u32,
    pub total: u32,
}

/// What a game needs to play the loop and put things on the path. Written as `<name>.json`.
#[derive(Serialize)]
struct Metadata<'a> {
    generator: &'static str,
    /// The renderer that drew the frames: "gpu" or "cpu" (the reference).
    engine: &'static str,
    name: &'a str,
    frames: u32,
    width: usize,
    height: usize,
    loop_length_m: f32,
    metres_per_frame: f32,
    speed_mps: f32,
    fps: f32,
    loop_seconds: f32,
    playback: &'static str,
    files: Vec<String>,
    sheet: Option<SheetInfo>,
    depth: Option<DepthInfo>,
    layers: Option<Vec<LayerBand>>,
    /// Lightning strikes in the loop, for syncing thunder: the frame where each flash peaks.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    lightning: Vec<StrikeInfo>,
    camera: CameraInfo,
    scene: &'a Scene,
}

#[derive(Serialize, Clone)]
struct StrikeInfo { frame: u32, seconds: f32 }

#[derive(Serialize, Clone)]
struct SheetInfo {
    /// One image per page; frame k is on page k / frames_per_page.
    pages: Vec<String>,
    columns: u32,
    rows: u32,
    frames_per_page: u32,
    frame_width: usize,
    frame_height: usize,
}

/// How a depth export encodes distance, and the bands of a layers export.
#[derive(Serialize, Clone)]
struct DepthInfo {
    folder: String,
    encoding: &'static str,
    near_m: f32,
}

#[derive(Serialize, Clone)]
struct LayerBand { name: String, folder: String, from_m: f32, to_m: Option<f32> }

/// Depth is stored as near/d in 16 bits: precise close to the camera, where gameplay happens.
pub const DEPTH_NEAR_M: f32 = 0.25;

/// The camera a scene is rendered with, and the formulas that map the world to the screen.
#[derive(Serialize, Clone, Debug)]
pub struct CameraInfo {
    pub view: View,
    pub bend_k: f32,
    pub hill_k: f32,
    pub projection: &'static str,
    pub path_half_width: &'static str,
    pub lens_curve_note: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stairs_note: Option<&'static str>,
}

impl CameraInfo {
    pub fn new(scene: &Scene, width: usize, height: usize) -> CameraInfo {
        CameraInfo {
            view: View::new(scene, width, height), bend_k: BEND_K, hill_k: HILL_K,
            projection: "For a point x metres right of the path centre, y metres above the ground and d metres ahead: X = x + view.bend*bend_k*d^2; Y = y - view.eye_height + view.hill*hill_k*d^2; screen_x = view.center_px + view.focal_px*X/d; screen_y = view.horizon_px - view.focal_px*Y/d. Something h metres tall at distance d is view.focal_px*h/d pixels tall.",
            path_half_width: "half_width(d) = view.half_width * (max(d, view.near_ground)/view.near_ground)^view.flare",
            lens_curve_note: "When view.lens_curve is not 0, the image is also bent vertically: rows move by lens_curve*0.22*height*nx^2 near and below the horizon (nx = -1..1 across the screen).",
            stairs_note: scene.path.stairs.enabled.then_some("When view.stairs is set, the ground has steps and Y also adds lift(d) = ground(s + d) - ramp(s), where s is how far along the loop the frame is (s = frame * metres_per_frame) and, with u = (w - offset) mod period and k = floor((w - offset) / period): ground(w) = k*steps*rise + rise*min(floor(u/run) + 1, steps); ramp(w) = k*steps*rise + steps*rise*clamp(u/(steps*run), 0, 1). y is then measured from the step at d. view.scroll is 0 here (frame 0)."),
        }
    }
}

fn tmp_path(p: &Path) -> PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

fn sanitize(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else if c.is_whitespace() { '_' } else { '-' }).collect();
    let s = s.trim_matches(['_', '-']).to_owned();
    if s.is_empty() { "pathforge".into() } else { s }
}

/// Per-frame delays in units of 1/`unit` s that add up exactly to the loop duration.
fn delays(frames: u32, loop_seconds: f32, unit: f32) -> Vec<u32> {
    let total = loop_seconds as f64 * unit as f64;
    (0..frames).map(|i| {
        let a = (total * i as f64 / frames as f64).round();
        let b = (total * (i + 1) as f64 / frames as f64).round();
        (b - a).max(1.0) as u32
    }).collect()
}

/// Frames to render: (distance walked in metres, time in seconds or None for "time = distance / speed").
pub struct Seq {
    pub frames: Vec<(f32, Option<f32>)>,
    /// Playing time of the whole sequence.
    pub seconds: f32,
    /// A loop (repeat forever) or a one-shot clip (play once).
    pub looping: bool,
}

/// What one sequence wrote.
struct SeqOut {
    files: Vec<PathBuf>,
    frames: u32,
    width: usize,
    height: usize,
    sheet: Option<SheetInfo>,
    depth: Option<DepthInfo>,
    layers: Vec<LayerBand>,
}

fn prepare(scene: &Scene, job: &ExportJob) -> Result<String, String> {
    if job.formats.is_empty() { return Err("no export formats chosen".into()); }
    std::fs::create_dir_all(&job.out_dir).map_err(|e| format!("{}: {e}", job.out_dir.display()))?;
    Ok(sanitize(if job.name.trim().is_empty() { &scene.name } else { &job.name }))
}

pub fn export(scene: &Scene, job: &ExportJob, mut progress: impl FnMut(Progress), cancel: &AtomicBool) -> Result<ExportReport, String> {
    let t0 = Instant::now();
    let stem = prepare(scene, job)?;
    let loop_seconds = scene.motion.loop_seconds();
    let mut n = job.frames.unwrap_or_else(|| scene.motion.frames()).clamp(2, 4000);
    let mut notes = Vec::new();
    // GIF tops out at 50 fps; keep the loop's frame count divisible by the GIF's frame step.
    let fps = n as f32 / loop_seconds;
    if job.formats.contains(&Format::Gif) && fps > 50.0 { let step = (fps / 50.0).ceil() as u32; n = n.div_ceil(step) * step; }
    let lp = scene.motion.loop_length.max(1.0);
    let seq = Seq { frames: (0..n).map(|k| (lp * k as f32 / n as f32, None)).collect(), seconds: loop_seconds, looping: true };
    let mut renderer = WorldRenderer::auto();
    let out = export_seq(scene, job, &stem, &seq, &mut renderer, &mut progress, cancel, &mut notes)?;
    let (w, h, n) = (out.width, out.height, out.frames);
    let mut files = out.files;
    let (sheet_info, depth_info, bands) = (out.sheet, out.depth, out.layers);

    let meta = Metadata {
        generator: concat!("PathForge ", env!("CARGO_PKG_VERSION"), " (scene v3)"),
        engine: renderer.engine(),
        name: &scene.name, frames: n, width: w, height: h, loop_length_m: lp, metres_per_frame: lp / n as f32,
        speed_mps: scene.motion.speed, fps: n as f32 / loop_seconds, loop_seconds,
        playback: "Frames are evenly spaced in distance. To follow a character, show frame floor(distance_walked / metres_per_frame) mod frames; to play on a timer, advance at fps.",
        files: files.iter().filter_map(|f| f.file_name().map(|s| s.to_string_lossy().into_owned())).collect(),
        sheet: sheet_info.clone(),
        depth: depth_info.clone(),
        layers: (!bands.is_empty()).then(|| bands.clone()),
        lightning: crate::world::lightning_times(scene).into_iter()
            .map(|p| StrikeInfo { frame: (p * n as f32).round() as u32 % n.max(1), seconds: p * loop_seconds }).collect(),
        camera: CameraInfo::new(scene, w, h),
        scene,
    };
    let mpath = job.out_dir.join(format!("{stem}.json"));
    std::fs::write(&mpath, serde_json::to_string_pretty(&meta).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    files.push(mpath);

    Ok(ExportReport {
        files, frames: n, fps: n as f32 / loop_seconds, loop_seconds, metres_per_frame: lp / n as f32,
        width: w, height: h, elapsed_ms: t0.elapsed().as_millis() as u64, notes,
    })
}

/// What an export would come to, worked out from a short run of consecutive frames encoded exactly
/// as the export would, scaled to the whole loop.
pub struct Estimate {
    /// Each file or folder the export writes, with its expected size in bytes.
    pub parts: Vec<(String, u64)>,
    pub total: u64,
    /// The first frame as the chosen animation format stores it (GIF or WebP colours), for a
    /// preview of the final colours.
    pub preview: Option<crate::world::Image>,
    pub sample: u32,
    pub frames: u32,
}

/// Estimate the size of `job` from `sample` consecutive frames (at the real frame spacing).
pub fn estimate(scene: &Scene, job: &ExportJob, sample: u32) -> Result<Estimate, String> {
    let loop_seconds = scene.motion.loop_seconds();
    let mut n = job.frames.unwrap_or_else(|| scene.motion.frames()).clamp(2, 4000);
    let fps = n as f32 / loop_seconds;
    if job.formats.contains(&Format::Gif) && fps > 50.0 { let step = (fps / 50.0).ceil() as u32; n = n.div_ceil(step) * step; }
    let k = sample.clamp(2, n);
    let lp = scene.motion.loop_length.max(1.0);
    let seq = Seq { frames: (0..k).map(|i| (lp * i as f32 / n as f32, None)).collect(), seconds: loop_seconds * k as f32 / n as f32, looping: true };
    let dir = std::env::temp_dir().join(format!("pf_estimate_{}_{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)));
    let mut tmp_job = ExportJob { out_dir: dir.clone(), formats: job.formats.clone(), ..job.clone() };
    // Full-colour GIFs choose 255 colours from 12 frames across the loop; choose them the same way.
    if job.formats.contains(&Format::Gif) && !matches!(scene.style.palette, Palette::Named(_) | Palette::Custom(_) | Palette::Auto(_)) && tmp_job.gif_palette.is_none() {
        let (w, h) = job.size.unwrap_or((scene.canvas.width, scene.canvas.height));
        let opts = RenderOptions { size: Some((w, h)), base_dir: job.base_dir.clone(), ..RenderOptions::default() };
        let mut r = WorldRenderer::auto();
        let samples = 12.min(n);
        let mut px = Vec::new();
        for i in 0..samples {
            let img = r.render(scene, lp * (i * n / samples) as f32 / n as f32, &opts);
            let step = (img.width * img.height / 30_000).max(1);
            px.extend(img.rgba.chunks_exact(4).step_by(step).map(|c| [c[0], c[1], c[2]]));
        }
        tmp_job.gif_palette = Some(Arc::new(PaletteLut::with_bits(median_cut(&px, 255), 6)));
    }
    let stem = prepare(scene, &tmp_job)?;
    let mut notes = Vec::new();
    let out = export_seq(scene, &tmp_job, &stem, &seq, &mut WorldRenderer::auto(), &mut |_| {}, &AtomicBool::new(false), &mut notes);
    let result = out.map(|out| {
        let scale = n as f64 / k as f64;
        let mut parts: Vec<(String, u64)> = Vec::new();
        for f in &out.files {
            let bytes = std::fs::metadata(f).map(|m| m.len()).unwrap_or(0);
            let rel = f.strip_prefix(&dir).unwrap_or(f);
            // Frames written into a folder count as the folder.
            let key = rel.components().next().map(|c| c.as_os_str().to_string_lossy().into_owned()).unwrap_or_default();
            let key = if rel.components().count() > 1 { format!("{key}/") } else { key };
            match parts.iter_mut().find(|p| p.0 == key) { Some(p) => p.1 += bytes, None => parts.push((key, bytes)) }
        }
        for p in &mut parts { p.1 = (p.1 as f64 * scale).round() as u64; }
        let preview = out.files.iter().find(|f| f.extension().is_some_and(|e| e == "gif"))
            .and_then(|f| crate::review::decode_gif(f).ok())
            .or_else(|| out.files.iter().find(|f| f.extension().is_some_and(|e| e == "webp")).and_then(|f| crate::review::decode_webp(f).ok()))
            .and_then(|mut v| if v.is_empty() { None } else { Some(v.swap_remove(0)) });
        let total = parts.iter().map(|p| p.1).sum();
        Estimate { parts, total, preview, sample: k, frames: n }
    });
    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// A stop for a fight: the walk eases to a halt, stands while time runs (flames, particles), and eases
/// back into the loop. Three outputs: `stop` (one-shot), `idle` (loops for as long as the fight
/// lasts) and `go` (one-shot that ends exactly where the loop continues).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Encounter {
    /// Metres the camera travels while slowing down (and again while speeding up).
    pub slow_m: f32,
    /// Loop frame where `stop` begins and `go` hands back (the game enters the encounter there).
    pub start_frame: u32,
}
impl Default for Encounter { fn default() -> Self { Encounter { slow_m: 3.0, start_frame: 0 } } }

/// The (distance, time) frames of the three encounter clips, and a summary for the metadata.
pub fn encounter_clips(scene: &Scene, e: &Encounter) -> (Seq, Seq, Seq, serde_json::Value) {
    let v = scene.motion.speed.max(0.01);
    let l = scene.motion.loop_length.max(1.0);
    let period = scene.motion.loop_seconds();
    let fps = scene.motion.fps.max(1) as f32;
    let n_loop = scene.motion.frames();
    let j = e.start_frame % n_loop;
    let (d0, t0) = (l * j as f32 / n_loop as f32, period * j as f32 / n_loop as f32);
    let slow = e.slow_m.clamp(0.1, l * 4.0);
    // Cosine ease: speed falls from v to 0 (or rises) over t1 seconds while covering `slow` metres.
    let t1 = 2.0 * slow / v;
    let pi = std::f32::consts::PI;
    let ease_out = |t: f32| v * 0.5 * (t + t1 / pi * (pi * t / t1).sin());
    let ease_in = |t: f32| v * 0.5 * (t - t1 / pi * (pi * t / t1).sin());
    // Time-driven effects repeat every `period`; easing shifts time against distance by 2*slow/v,
    // so the stop holds still for h0 more seconds to bring them back into step.
    let h0 = (period - (2.0 * slow / v).rem_euclid(period)).rem_euclid(period);
    // After speeding up, walk on until a whole number of loops has been covered.
    let w2 = (l - (2.0 * slow).rem_euclid(l)).rem_euclid(l);

    let sample = |dur: f32, f: &dyn Fn(f32) -> (f32, f32)| -> Seq {
        let n = ((dur * fps).round() as usize).max(2);
        Seq { frames: (0..n).map(|k| { let (d, t) = f(dur * k as f32 / n as f32); (d, Some(t)) }).collect(), seconds: dur, looping: false }
    };
    let stop = sample(t1 + h0, &|t| (d0 + if t < t1 { ease_out(t) } else { slow }, t0 + t));
    let mut idle = sample(period, &|t| (d0 + slow, t0 + t1 + h0 + t));
    idle.looping = true;
    let go = sample(t1 + w2 / v, &|t| (d0 + slow + if t < t1 { ease_in(t) } else { slow + (t - t1) * v }, t0 + t1 + h0 + t));
    let info = serde_json::json!({
        "start_frame": j, "slow_m": slow, "ease_seconds": t1, "hold_seconds_in_stop": h0, "walk_after_m": w2,
        "stop": {"frames": stop.frames.len(), "seconds": stop.seconds},
        "idle": {"frames": idle.frames.len(), "seconds": idle.seconds},
        "go": {"frames": go.frames.len(), "seconds": go.seconds},
        "playback": format!("Play the loop; when a fight starts, wait for loop frame {j}, then play <name>_stop once, repeat <name>_idle until the fight ends (finish the idle cycle you are in), play <name>_go once, and continue the loop from frame {j}. Every junction matches exactly."),
    });
    (stop, idle, go, info)
}

/// Export the three encounter clips with every format in `job`, plus `<name>_encounter.json`.
pub fn export_encounter(scene: &Scene, job: &ExportJob, e: &Encounter, mut progress: impl FnMut(Progress), cancel: &AtomicBool) -> Result<ExportReport, String> {
    let t0 = Instant::now();
    let stem = prepare(scene, job)?;
    let (stop, idle, go, info) = encounter_clips(scene, e);
    let mut renderer = WorldRenderer::auto();
    let mut notes = Vec::new();
    let mut files = Vec::new();
    let mut clips = serde_json::Map::new();
    let (mut w, mut h, mut frames) = (0, 0, 0);
    for (name, seq) in [("stop", &stop), ("idle", &idle), ("go", &go)] {
        let out = export_seq(scene, job, &format!("{stem}_{name}"), seq, &mut renderer, &mut progress, cancel, &mut notes)?;
        (w, h) = (out.width, out.height);
        frames += out.frames;
        let names: Vec<String> = out.files.iter().filter_map(|f| f.file_name().map(|s| s.to_string_lossy().into_owned())).collect();
        clips.insert(name.into(), serde_json::json!({"files": names, "sheet": out.sheet, "depth": out.depth, "layers": (!out.layers.is_empty()).then_some(&out.layers)}));
        files.extend(out.files);
    }
    let mut meta = info;
    meta["generator"] = serde_json::json!(concat!("PathForge ", env!("CARGO_PKG_VERSION"), " (scene v3)"));
    meta["engine"] = serde_json::json!(renderer.engine());
    meta["clips_files"] = serde_json::Value::Object(clips);
    meta["camera"] = serde_json::to_value(CameraInfo::new(scene, w, h)).unwrap_or_default();
    let mpath = job.out_dir.join(format!("{stem}_encounter.json"));
    std::fs::write(&mpath, serde_json::to_string_pretty(&meta).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    files.push(mpath);
    Ok(ExportReport {
        files, frames, fps: scene.motion.fps as f32, loop_seconds: scene.motion.loop_seconds(),
        metres_per_frame: scene.motion.loop_length / scene.motion.frames() as f32,
        width: w, height: h, elapsed_ms: t0.elapsed().as_millis() as u64, notes,
    })
}

pub use crate::scene::transition::Transition;
use crate::journey::{CrossWalk, Place};
use crate::scene::transition::Crossing;

/// Per frame of a walk of `total` metres whose speed eases from `va` to `vb`: travel (m) and time
/// (s) since the start; plus the walk's duration.
pub fn walk_plan(total: f32, va: f32, vb: f32, fps: u32) -> (Seq, f32) {
    let (va, vb) = (va.max(0.01), vb.max(0.01));
    let v = |s: f32| { let u = (s / total).clamp(0.0, 1.0); va + (vb - va) * u * u * (3.0 - 2.0 * u) };
    // time(s) by integrating ds / v(s)
    let steps = 4000;
    let mut table = vec![(0.0f32, 0.0f32)];
    let mut t = 0.0;
    for k in 1..=steps {
        let (s0, s1) = (total * (k - 1) as f32 / steps as f32, total * k as f32 / steps as f32);
        t += (s1 - s0) / v(0.5 * (s0 + s1));
        table.push((s1, t));
    }
    let duration = t;
    let n = ((duration * fps.max(1) as f32).round() as usize).max(2);
    let frames = (0..n).map(|k| {
        let tk = duration * k as f32 / n as f32;
        let j = table.partition_point(|(_, t)| *t < tk).min(table.len() - 1);
        let (s1, t1) = table[j];
        let (s0, t0) = table[j.saturating_sub(1)];
        let f = if t1 > t0 { (tk - t0) / (t1 - t0) } else { 0.0 };
        (s0 + (s1 - s0) * f, Some(tk))
    }).collect();
    (Seq { frames, seconds: duration, looping: false }, duration)
}

/// The frames of a transition from `a` into `b` with the plan `c`: it starts on the first loop's
/// frame 0 and ends exactly where the second loop's frame 0 continues.
pub fn transition_plan(a: &Scene, b: &Scene, c: &Crossing) -> (Seq, CrossWalk, f32) { transition_plan_from(a, b, c, 0.0) }

/// `transition_plan` starting `start_a` metres along the first loop (one of its frames).
pub fn transition_plan_from(a: &Scene, b: &Scene, c: &Crossing, start_a: f32) -> (Seq, CrossWalk, f32) {
    let walk = CrossWalk::new(c.clone(), start_a, 0.0);
    let (seq, duration) = walk_plan(walk.length(), a.motion.speed, b.motion.speed, b.motion.fps);
    (seq, walk, duration)
}

/// One frame of a transition: both worlds in one picture. The first world's clock starts at 0, the
/// second's reaches 0 at the end, so both ends match their loops.
#[allow(clippy::too_many_arguments)]
pub fn transition_frame(r: &mut WorldRenderer, a: &Scene, a_dir: Option<&Path>, b: &Scene, b_dir: Option<&Path>, walk: &CrossWalk, duration: f32, travel: f32, time: f32, opts: &RenderOptions) -> crate::world::Image {
    // The first loop's clock where the walk starts (its frames' time follows distance).
    let ta = walk.start_a / a.motion.speed.max(0.01);
    walk.frame(r, Place { scene: a, dir: a_dir }, Place { scene: b, dir: b_dir }, travel, ta + time, time - duration, opts)
}

/// Export the transition clip from `a` into `b`, plus `<name>.json` with how to play it.
pub fn export_transition(a: &Scene, b: &Scene, b_dir: Option<PathBuf>, job: &ExportJob, tr: &Transition, progress: impl FnMut(Progress), cancel: &AtomicBool) -> Result<ExportReport, String> {
    export_transition_entries(a, b, b_dir, job, tr, 1, progress, cancel)
}

/// `export_transition` as `entries` clips starting at evenly spaced frames of the first loop, so a
/// game can begin the walk without waiting up to a whole loop for frame 0.
#[allow(clippy::too_many_arguments)]
pub fn export_transition_entries(a: &Scene, b: &Scene, b_dir: Option<PathBuf>, job: &ExportJob, tr: &Transition, entries: u32, mut progress: impl FnMut(Progress), cancel: &AtomicBool) -> Result<ExportReport, String> {
    let t0 = Instant::now();
    let stem = prepare(a, &ExportJob { name: if job.name.trim().is_empty() { format!("{}_to_{}", a.name, b.name) } else { job.name.clone() }, ..job.clone() })?;
    let crossing = crate::scene::transition::plan(a, b, tr);
    let entries = entries.clamp(1, 16);
    let loop_frames = job.frames.unwrap_or_else(|| a.motion.frames()).max(2);
    let mut files = Vec::new();
    let mut notes: Vec<String> = crossing.warnings.clone();
    let mut list = Vec::new();
    let mut last = None;
    let mut engine = "cpu";
    let a_dir = job.base_dir.clone();
    for k in 0..entries {
        let start_frame = k * loop_frames / entries;
        let start_a = a.motion.loop_length.max(1.0) * start_frame as f32 / loop_frames as f32;
        let (seq, walk, duration) = transition_plan_from(a, b, &crossing, start_a);
        let total = walk.length();
        let name = if entries > 1 { format!("{stem}_e{k}") } else { stem.clone() };
        let mut ra = WorldRenderer::auto();
        engine = ra.engine();
        let mut make = |r: &mut WorldRenderer, d: f32, t: Option<f32>, o: &RenderOptions| transition_frame(r, a, a_dir.as_deref(), b, b_dir.as_deref(), &walk, duration, d, t.unwrap_or(0.0), o);
        let out = export_frames(a, job, &name, &seq, &mut ra, &mut make, &mut progress, cancel, &mut notes)?;
        list.push(serde_json::json!({
            "start_frame": start_frame, "frames": out.frames, "seconds": duration,
            "boundary_frame": seq.frames.iter().position(|(d, _)| *d >= crossing.approach).unwrap_or(out.frames as usize),
            "start_m": start_a,
            "travel_m_at_frame": seq.frames.iter().map(|(d, _)| (d * 1000.0).round() / 1000.0).collect::<Vec<_>>(),
            "files": out.files.iter().filter_map(|f| f.file_name().map(|s| s.to_string_lossy().into_owned())).collect::<Vec<_>>(),
        }));
        files.extend(out.files.iter().cloned());
        last = Some((out, duration, total));
    }
    let (out, duration, total) = last.ok_or("no clips")?;
    let meta = serde_json::json!({
        "generator": concat!("PathForge ", env!("CARGO_PKG_VERSION"), " (scene v3)"),
        "engine": engine,
        "from": a.name, "to": b.name, "travel_m": total, "approach_m": crossing.approach,
        "threshold": crossing.threshold.name(), "plan": crossing.notes, "warnings": crossing.warnings,
        "transition": tr,
        "width": out.width, "height": out.height, "loop_frames": loop_frames, "entries": list,
        "playback": if entries > 1 {
            format!("Play the '{}' loop; at the next loop frame listed as an entry's start_frame, play that entry's clip once, then continue with the '{}' loop from its frame 0. Both ends match exactly.", a.name, b.name)
        } else {
            format!("Play the '{}' loop until its frame 0 comes up, play this clip once, then continue with the '{}' loop from its frame 0. Both ends match exactly.", a.name, b.name)
        },
        "sheet": out.sheet, "depth": out.depth, "layers": (!out.layers.is_empty()).then_some(&out.layers),
    });
    let mpath = job.out_dir.join(format!("{stem}.json"));
    std::fs::write(&mpath, serde_json::to_string_pretty(&meta).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    files.push(mpath);
    Ok(ExportReport {
        files, frames: out.frames, fps: out.frames as f32 / duration, loop_seconds: duration, metres_per_frame: total / out.frames as f32,
        width: out.width, height: out.height, elapsed_ms: t0.elapsed().as_millis() as u64, notes,
    })
}

/// A fork as three clips: the walk up to the point where the game must have chosen (both branches
/// in view), then one clip per branch from there into its world. Each branch clip starts exactly
/// where the approach ends and ends exactly on its world's loop frame 0.
#[allow(clippy::too_many_arguments)]
pub fn export_fork(a: &Scene, left: (&Scene, Option<PathBuf>), right: (&Scene, Option<PathBuf>), job: &ExportJob, f: &crate::scene::transition::ForkChoice, mut progress: impl FnMut(Progress), cancel: &AtomicBool) -> Result<ExportReport, String> {
    use crate::journey::ForkWalk;
    use crate::scene::transition::Branch;
    let t0 = Instant::now();
    let stem = prepare(a, &ExportJob { name: if job.name.trim().is_empty() { format!("{}_fork", a.name) } else { job.name.clone() }, ..job.clone() })?;
    let plan = crate::scene::transition::plan_fork(a, left.0, right.0, f);
    let base = ForkWalk::new(plan.clone(), 0.0, [0.0, 0.0]);
    let decide = base.decide_by();
    let fps = a.motion.fps;
    let (va, vl, vr) = (a.motion.speed, left.0.motion.speed, right.0.motion.speed);
    // Up to the decision at the first world's pace; then each branch eases to its world's pace.
    let (approach, t1) = walk_plan(decide.max(0.5), va, va, fps);
    let (seq_l, t2l) = walk_plan(base.length() - decide, va, vl, left.0.motion.fps);
    let (seq_r, t2r) = walk_plan(base.length() - decide, va, vr, right.0.motion.fps);
    // Each branch world's clock reaches 0 where its clip ends.
    let times = |t: f32| [t, t - (t1 + t2l), t - (t1 + t2r)];
    let mut notes = plan.warnings.clone();
    let a_dir = job.base_dir.clone();
    let (pl, pr) = (crate::journey::Place { scene: left.0, dir: left.1.as_deref() }, crate::journey::Place { scene: right.0, dir: right.1.as_deref() });
    let pa = crate::journey::Place { scene: a, dir: a_dir.as_deref() };
    let mut files = Vec::new();
    let mut r = WorldRenderer::auto();
    let mut make = |rr: &mut WorldRenderer, d: f32, t: Option<f32>, o: &RenderOptions| base.frame(rr, pa, pl, pr, d, times(t.unwrap_or(0.0)), o);
    let out0 = export_frames(a, job, &format!("{stem}_approach"), &approach, &mut r, &mut make, &mut progress, cancel, &mut notes)?;
    files.extend(out0.files.iter().cloned());
    let mut clips = vec![serde_json::json!({"clip": "approach", "frames": out0.frames, "seconds": t1, "files": out0.files.iter().filter_map(|f| f.file_name().map(|s| s.to_string_lossy().into_owned())).collect::<Vec<_>>()})];
    for (branch, seq, t2, scene) in [(Branch::Left, &seq_l, t2l, left.0), (Branch::Right, &seq_r, t2r, right.0)] {
        let mut walk = base.clone();
        walk.choose(branch, decide);
        let shifted = Seq { frames: seq.frames.iter().map(|(d, t)| (d + decide, t.map(|t| t + t1))).collect(), seconds: seq.seconds, looping: false };
        let mut make = |rr: &mut WorldRenderer, d: f32, t: Option<f32>, o: &RenderOptions| walk.frame(rr, pa, pl, pr, d, times(t.unwrap_or(0.0)), o);
        let out = export_frames(scene, job, &format!("{stem}_{}", branch.name()), &shifted, &mut r, &mut make, &mut progress, cancel, &mut notes)?;
        clips.push(serde_json::json!({"clip": branch.name(), "to": scene.name, "frames": out.frames, "seconds": t2, "files": out.files.iter().filter_map(|f| f.file_name().map(|s| s.to_string_lossy().into_owned())).collect::<Vec<_>>()}));
        files.extend(out.files.iter().cloned());
    }
    let meta = serde_json::json!({
        "generator": concat!("PathForge ", env!("CARGO_PKG_VERSION"), " (scene v3)"),
        "engine": r.engine(),
        "from": a.name, "left": left.0.name, "right": right.0.name, "default": plan.default_branch.name(),
        "plan": plan.notes, "warnings": plan.warnings, "clips": clips, "width": out0.width, "height": out0.height,
        "playback": format!("Play the '{}' loop until its frame 0, then '{stem}_approach' once; ask the player during it. When it ends, play '{stem}_left' or '{stem}_right' (the {} one if no choice was made), then continue with that world's loop from its frame 0. Every join matches exactly.", a.name, plan.default_branch.name()),
    });
    let mpath = job.out_dir.join(format!("{stem}.json"));
    std::fs::write(&mpath, serde_json::to_string_pretty(&meta).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    files.push(mpath);
    Ok(ExportReport {
        files, frames: out0.frames, fps: fps as f32, loop_seconds: t1, metres_per_frame: decide / out0.frames.max(1) as f32,
        width: out0.width, height: out0.height, elapsed_ms: t0.elapsed().as_millis() as u64, notes,
    })
}

/// Everything a journey needs: each stop's loop, each transition, each fork's clips, and
/// `<name>.journey.json` mapping stops to files and saying how to play them.
pub fn export_journey(j: &crate::journey::Journey, dir: Option<&Path>, job: &ExportJob, entries: u32, mut progress: impl FnMut(Progress), cancel: &AtomicBool) -> Result<ExportReport, String> {
    use crate::journey::Next;
    let t0 = Instant::now();
    let problems = j.problems(dir);
    if !problems.is_empty() { return Err(format!("the journey has problems: {}", problems.join("; "))); }
    std::fs::create_dir_all(&job.out_dir).map_err(|e| format!("{}: {e}", job.out_dir.display()))?;
    let mut files = Vec::new();
    let mut notes = Vec::new();
    let mut stops = serde_json::Map::new();
    let mut last = None;
    for (id, st) in &j.stops {
        if cancel.load(Ordering::Relaxed) { return Err("cancelled".into()); }
        let (scene, sdir) = j.scene_of(id, dir)?;
        let sj = ExportJob { name: id.clone(), base_dir: sdir.clone(), ..job.clone() };
        let rep = export(&scene, &sj, &mut progress, cancel)?;
        let names = |fs: &[PathBuf]| fs.iter().filter_map(|f| f.file_name().map(|s| s.to_string_lossy().into_owned())).collect::<Vec<_>>();
        let mut entry = serde_json::json!({"scene": st.scene, "loop": names(&rep.files), "frames": rep.frames, "fps": rep.fps});
        files.extend(rep.files.iter().cloned());
        notes.extend(rep.notes.iter().cloned());
        match &st.next {
            Next::End => { entry["next"] = serde_json::json!("end"); }
            Next::Go { to, transition } => {
                let (b, bdir) = j.scene_of(to, dir)?;
                let tj = ExportJob { name: format!("{id}_to_{to}"), base_dir: sdir.clone(), ..job.clone() };
                let rep = export_transition_entries(&scene, &b, bdir, &tj, transition, entries, &mut progress, cancel)?;
                entry["next"] = serde_json::json!({"go": to, "clip": names(&rep.files)});
                files.extend(rep.files.iter().cloned());
                notes.extend(rep.notes.iter().cloned());
            }
            Next::Fork { left, right, fork } => {
                let (l, ld) = j.scene_of(left, dir)?;
                let (r, rd) = j.scene_of(right, dir)?;
                let fj = ExportJob { name: format!("{id}_fork"), base_dir: sdir.clone(), ..job.clone() };
                let rep = export_fork(&scene, (&l, ld), (&r, rd), &fj, fork, &mut progress, cancel)?;
                entry["next"] = serde_json::json!({"fork": {"left": left, "right": right}, "clips": names(&rep.files)});
                files.extend(rep.files.iter().cloned());
                notes.extend(rep.notes.iter().cloned());
            }
        }
        stops.insert(id.clone(), entry);
        last = Some(rep);
    }
    let rep = last.ok_or("the journey has no stops")?;
    let stem = sanitize(if j.name.trim().is_empty() { "journey" } else { &j.name });
    let meta = serde_json::json!({
        "generator": concat!("PathForge ", env!("CARGO_PKG_VERSION"), " (journey)"),
        "name": j.name, "start": j.start, "stops": stops,
        "playback": "Loop the start stop. When the game moves on, play the stop's `next` clip (a transition, or a fork's approach then the chosen branch) and continue with the next stop's loop from frame 0; each clip's own .json says exactly when to start it.",
    });
    let mpath = job.out_dir.join(format!("{stem}.journey.json"));
    std::fs::write(&mpath, serde_json::to_string_pretty(&meta).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    files.push(mpath);
    Ok(ExportReport { files, frames: rep.frames, fps: rep.fps, loop_seconds: rep.loop_seconds, metres_per_frame: rep.metres_per_frame, width: rep.width, height: rep.height, elapsed_ms: t0.elapsed().as_millis() as u64, notes })
}

/// Render `seq` once and feed every requested format.
#[allow(clippy::too_many_arguments)]
fn export_seq(scene: &Scene, job: &ExportJob, stem: &str, seq: &Seq, renderer: &mut WorldRenderer, progress: &mut dyn FnMut(Progress), cancel: &AtomicBool, notes: &mut Vec<String>) -> Result<SeqOut, String> {
    let mut plain = |r: &mut WorldRenderer, d: f32, t: Option<f32>, o: &RenderOptions| r.render(scene, d, &RenderOptions { time: t, ..o.clone() });
    export_frames(scene, job, stem, seq, renderer, &mut plain, progress, cancel, notes)
}

/// `export_seq` with any frame source: `render(renderer, distance, time, options)` makes frame images.
#[allow(clippy::too_many_arguments)]
fn export_frames(scene: &Scene, job: &ExportJob, stem: &str, seq: &Seq, renderer: &mut WorldRenderer,
    render: &mut dyn FnMut(&mut WorldRenderer, f32, Option<f32>, &RenderOptions) -> crate::world::Image,
    progress: &mut dyn FnMut(Progress), cancel: &AtomicBool, notes: &mut Vec<String>) -> Result<SeqOut, String> {
    let stem = stem.to_owned();
    let (w, h) = job.size.map(|(a, b)| (a as usize, b as usize)).unwrap_or((scene.canvas.width as usize, scene.canvas.height as usize));
    let n = seq.frames.len().max(1) as u32;
    let seconds = seq.seconds.max(1e-3);
    let fps = n as f32 / seconds;
    // GIF frame delays are whole hundredths and browsers treat < 2 as slow: cap GIF at 50 fps.
    let gif_step = if job.formats.contains(&Format::Gif) && fps > 50.0 { (fps / 50.0).ceil() as u32 } else { 1 };
    if gif_step > 1 { notes.push(format!("{stem}.gif uses every {gif_step}th frame: GIF timing cannot go above 50 fps ({fps:.0} fps requested).")); }
    // Apple's ImageIO decodes animated WebP frame k by decoding frames 0..k (measured on macOS 26.6:
    // 480x854, frame 287 took 1.4 s), whatever the frame flags, so Safari, Preview and Quick Look fall
    // behind within a few frames. Chrome, Firefox, Discord and engines decode it themselves.
    if job.formats.contains(&Format::Webp) && n > 24 { notes.push(format!("{stem}.webp plays slowly in Safari, Preview and Quick Look (Apple's decoder replays every earlier frame); view it in Chrome, Firefox, Discord or vwebp.")); }
    let opts = RenderOptions { size: Some((w as u32, h as u32)), base_dir: job.base_dir.clone(), depth: job.formats.iter().any(|f| matches!(f, Format::Depth | Format::Layers)), ..RenderOptions::default() };
    let mut frame = |renderer: &mut WorldRenderer, k: u32| {
        let (d, t) = seq.frames[(k as usize).min(seq.frames.len() - 1)];
        render(renderer, d, t, &opts)
    };

    // GIF palette: the scene's art palette, or the best 255 colours across the loop.
    let gif = if job.formats.contains(&Format::Gif) {
        let lut = match &scene.style.palette {
            Palette::Named(_) | Palette::Custom(_) => {
                let colors = match &scene.style.palette {
                    Palette::Named(nm) => crate::world::palette::named(nm).ok_or_else(|| format!("unknown palette `{nm}`"))?,
                    Palette::Custom(c) => c.clone(),
                    _ => unreachable!(),
                };
                Arc::new(PaletteLut::with_bits(colors.into_iter().take(255).collect(), 6))
            }
            Palette::Auto(k) => {
                // The same per-loop palette the renderer quantizes every frame with.
                let l = renderer.auto_palette(scene, *k, job.base_dir.clone());
                Arc::new(PaletteLut::with_bits(l.colors.clone(), 6))
            }
            _ if job.gif_palette.is_some() => job.gif_palette.clone().unwrap(),
            _ => {
                let samples = 12.min(n);
                let mut px = Vec::new();
                for k in 0..samples {
                    if cancel.load(Ordering::Relaxed) { return Err("cancelled".into()); }
                    progress(Progress { stage: "palette", done: k, total: samples });
                    let img = frame(renderer, k * n / samples);
                    let step = (img.width * img.height / 30_000).max(1);
                    px.extend(img.rgba.chunks_exact(4).step_by(step).map(|c| [c[0], c[1], c[2]]));
                }
                let target = match scene.style.palette { Palette::Auto(k) => (k as usize).clamp(2, 255), _ => 255 };
                Arc::new(PaletteLut::with_bits(median_cut(&px, target), 6))
            }
        };
        let path = job.out_dir.join(format!("{stem}.gif"));
        let file = BufWriter::new(File::create(tmp_path(&path)).map_err(|e| e.to_string())?);
        let mut pal: Vec<u8> = lut.colors.iter().flat_map(|c| *c).collect();
        while pal.len() < 256 * 3 { pal.push(0); }
        let mut enc = gif::Encoder::new(file, w as u16, h as u16, &pal).map_err(|e| e.to_string())?;
        enc.set_repeat(if seq.looping { gif::Repeat::Infinite } else { gif::Repeat::Finite(0) }).map_err(|e| e.to_string())?;
        let gif_frames = n.div_ceil(gif_step);
        let tol = job.gif_lossy.clamp(0.0, 1.0) * 0.06;
        let labs: Vec<[f32; 3]> = lut.colors.iter().map(|c| crate::world::palette::oklab([c[0] as f32, c[1] as f32, c[2] as f32])).collect();
        let k = labs.len();
        let _ = k;
        Some((path, enc, lut, delays(gif_frames, seconds, 100.0), None::<Vec<u8>>, (labs, tol)))
    } else { None };
    let mut gif = gif;

    let mut apng = if job.formats.contains(&Format::Apng) {
        let path = job.out_dir.join(format!("{stem}.png"));
        let file = BufWriter::new(File::create(tmp_path(&path)).map_err(|e| e.to_string())?);
        let mut enc = png::Encoder::new(file, w as u32, h as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_animated(n, if seq.looping { 0 } else { 1 }).map_err(|e| e.to_string())?;
        let writer = enc.write_header().map_err(|e| e.to_string())?;
        Some((path, writer, delays(n, seconds, 1000.0)))
    } else { None };

    let mut webp = if job.formats.contains(&Format::Webp) {
        use webp_animation::{Encoder, EncoderOptions, EncodingConfig, EncodingType, LossyEncodingConfig};
        let pixel_art = !matches!(scene.style.palette, Palette::Full);
        let q = job.webp_quality.unwrap_or(if pixel_art { 100.0 } else { 90.0 }).clamp(0.0, 100.0);
        let encoding = if q >= 100.0 {
            EncodingConfig { encoding_type: EncodingType::Lossless, quality: 75.0, method: 4 }
        } else {
            EncodingConfig { encoding_type: EncodingType::Lossy(LossyEncodingConfig { use_sharp_yuv: true, ..Default::default() }), quality: q, method: 4 }
        };
        // Every frame a key frame (kmax 1). libwebp's sub-frame diffing compares each frame with the
        // previous *source* frame, so slow gradients drift into stale macroblocks (mean error 3.0 and
        // 1.65% of pixels off by >24 on Forest Path at q90); key frames gave 1.35 / 0.15% at the same
        // size, because in a forward-moving scene almost every pixel changes anyway.
        // Lossless could diff exactly, but measured larger that way (castle_run 1976 KB vs 1853 KB).
        let opts = EncoderOptions { encoding_config: Some(encoding), kmax: 1, kmin: 0, anim_params: webp_animation::AnimParams { loop_count: if seq.looping { 0 } else { 1 } }, ..Default::default() };
        let enc = Encoder::new_with_options((w as u32, h as u32), opts).map_err(|e| format!("webp: {e:?}"))?;
        // Timestamps from cumulative exact times, so the loop lasts exactly loop_seconds.
        let stamps: Vec<i32> = (0..=n).map(|i| (seconds as f64 * 1000.0 * i as f64 / n as f64).round() as i32).collect();
        Some((job.out_dir.join(format!("{stem}.webp")), enc, stamps))
    } else { None };

    let png_dir = job.out_dir.join(format!("{stem}_frames"));
    if job.formats.contains(&Format::Png) {
        std::fs::create_dir_all(&png_dir).map_err(|e| e.to_string())?;
    }
    let sheet_info = if job.formats.contains(&Format::Sheet) {
        let max = job.sheet_max.unwrap_or(4096).max(64) as usize;
        let fit_cols = (max / w).max(1) as u32;
        let cols = job.sheet_columns.unwrap_or_else(|| ((n as f32).sqrt().ceil() as u32).min(fit_cols)).clamp(1, n);
        let rows = n.div_ceil(cols).min((max / h).max(1) as u32);
        let per_page = cols * rows;
        let pages = n.div_ceil(per_page);
        if w > max || h > max { notes.push(format!("frames are larger than the {max} px sheet limit; each page holds one frame")); }
        if pages > 1 { notes.push(format!("sprite sheet split into {pages} pages of at most {max} px")); }
        let names = (0..pages).map(|p| if pages == 1 { format!("{stem}_sheet.png") } else { format!("{stem}_sheet_{p}.png") }).collect();
        Some(SheetInfo { pages: names, columns: cols, rows, frames_per_page: per_page, frame_width: w, frame_height: h })
    } else { None };
    let mut sheet: Vec<Vec<u8>> = sheet_info.as_ref().map(|s| s.pages.iter().map(|_| vec![0u8; s.columns as usize * w * s.rows as usize * h * 4]).collect()).unwrap_or_default();

    // Depth maps and depth-banded layers.
    let depth_dir = job.out_dir.join(format!("{stem}_depth"));
    let depth_info = job.formats.contains(&Format::Depth).then(|| DepthInfo {
        folder: format!("{stem}_depth"), near_m: DEPTH_NEAR_M,
        encoding: "16-bit grey PNG per frame: value v (0..65535) means the surface is d = near_m * 65535 / v metres ahead of the camera (camera-space z, the same d as the camera projection formulas); v = 0 is sky (infinitely far). Something standing d_obj metres ahead is hidden by every pixel where d < d_obj.",
    });
    let bands: Vec<LayerBand> = if job.formats.contains(&Format::Layers) {
        let mut cuts: Vec<f32> = if job.layer_bands.is_empty() { vec![4.0, 12.0] } else { job.layer_bands.clone() };
        cuts.retain(|c| c.is_finite() && *c > 0.0);
        cuts.sort_by(|a, b| a.partial_cmp(b).unwrap());
        cuts.dedup();
        let names: Vec<String> = match cuts.len() { 1 => vec!["near".into(), "far".into()], 2 => vec!["near".into(), "mid".into(), "far".into()], k => (0..=k).map(|i| format!("band{i}")).collect() };
        (0..=cuts.len()).map(|i| LayerBand {
            name: names[i].clone(), folder: format!("{stem}_layers/{}", names[i]),
            from_m: if i == 0 { 0.0 } else { cuts[i - 1] }, to_m: cuts.get(i).copied(),
        }).collect()
    } else { Vec::new() };
    if depth_info.is_some() { std::fs::create_dir_all(&depth_dir).map_err(|e| e.to_string())?; }
    for b in &bands { std::fs::create_dir_all(job.out_dir.join(&b.folder)).map_err(|e| e.to_string())?; }
    let want_depth = depth_info.is_some() || !bands.is_empty();

    // Render once, feed every output. GIF frames are indexed and LZW-compressed in parallel batches.
    const BATCH: u32 = 12;
    let mut i = 0;
    while i < n {
        let end = (i + BATCH).min(n);
        let mut batch = Vec::with_capacity((end - i) as usize);
        for k in i..end {
            if cancel.load(Ordering::Relaxed) {
                if let Some((p, ..)) = &gif { let _ = std::fs::remove_file(tmp_path(p)); }
                if let Some((p, ..)) = &apng { let _ = std::fs::remove_file(tmp_path(p)); }
                return Err("cancelled".into());
            }
            progress(Progress { stage: "render", done: k, total: n });
            batch.push((k, frame(renderer, k)));
        }
        if job.formats.contains(&Format::Png) {
            // PNG frames are independent files: encode the batch side by side.
            batch.par_iter().try_for_each(|(k, img)| {
                let p = png_dir.join(format!("{stem}_{k:04}.png"));
                image::save_buffer(&p, &img.rgba, w as u32, h as u32, image::ExtendedColorType::Rgba8).map_err(|e| e.to_string())
            })?;
        }
        for (k, img) in &batch {
            if let Some((_, enc, stamps)) = &mut webp {
                enc.add_frame(&img.rgba, stamps[*k as usize]).map_err(|e| format!("webp: {e:?}"))?;
            }
            if let Some((_, writer, d)) = &mut apng {
                writer.set_frame_delay(d[*k as usize].min(u16::MAX as u32) as u16, 1000).map_err(|e| e.to_string())?;
                writer.write_image_data(&img.rgba).map_err(|e| e.to_string())?;
            }
            if let Some(si) = &sheet_info {
                let (page, slot) = ((*k / si.frames_per_page) as usize, *k % si.frames_per_page);
                let buf = &mut sheet[page];
                let (cx, cy) = ((slot % si.columns) as usize * w, (slot / si.columns) as usize * h);
                let sw = si.columns as usize * w;
                for y in 0..h {
                    let d0 = ((cy + y) * sw + cx) * 4;
                    buf[d0..d0 + w * 4].copy_from_slice(&img.rgba[y * w * 4..(y + 1) * w * 4]);
                }
            }
            if want_depth {
                let depth = img.depth.as_ref().ok_or("renderer returned no depth")?;
                if depth_info.is_some() {
                    let bytes: Vec<u8> = depth.iter().flat_map(|d| {
                        let v = if d.is_finite() { (DEPTH_NEAR_M / d.max(DEPTH_NEAR_M) * 65535.0).round() as u16 } else { 0 };
                        v.to_be_bytes()
                    }).collect();
                    write_png(&depth_dir.join(format!("{stem}_depth_{k:04}.png")), w, h, png::ColorType::Grayscale, png::BitDepth::Sixteen, &bytes)?;
                }
                for b in &bands {
                    let mut px = img.rgba.clone();
                    for (p, d) in px.chunks_exact_mut(4).zip(depth) {
                        let inside = *d >= b.from_m && b.to_m.is_none_or(|t| *d < t);
                        if !inside { p.copy_from_slice(&[0, 0, 0, 0]); }
                    }
                    write_png(&job.out_dir.join(&b.folder).join(format!("{stem}_{}_{k:04}.png", b.name)), w, h, png::ColorType::Rgba, png::BitDepth::Eight, &px)?;
                }
            }
        }
        if let Some((_, enc, lut, d, prev, (labs, tol))) = &mut gif {
            let picked: Vec<&(u32, crate::world::Image)> = batch.iter().filter(|(k, _)| k % gif_step == 0).collect();
            let strength = if job.gif_dither && scene.style.dither == Dither::None && scene.style.palette == Palette::Full { 0.35 } else { 0.0 };
            let amp = lut.spread * strength;
            let indexed: Vec<Vec<u8>> = picked.par_iter().map(|(_, img)| {
                img.rgba.chunks_exact(4).enumerate().map(|(p, c)| {
                    let t = if amp > 0.0 { bayer(Dither::Bayer4, p % w, p / w) * amp } else { 0.0 };
                    lut.index([c[0] as f32 + t, c[1] as f32 + t, c[2] as f32 + t])
                }).collect()
            }).collect();
            // The true (unquantized) colour of each pixel in OKLab, for the lossy keep test.
            let tol = *tol;
            let truth: Vec<Vec<[f32; 3]>> = if tol > 0.0 {
                picked.par_iter().map(|(_, img)| img.rgba.chunks_exact(4).map(|c| crate::world::palette::oklab([c[0] as f32, c[1] as f32, c[2] as f32])).collect()).collect()
            } else { vec![] };
            // Pixels that match what is on screen (exactly, or within gif_lossy) become transparent;
            // `prev` is what the viewer sees, so tolerance never accumulates. The loop's first frame is whole.
            let mut frames = Vec::with_capacity(indexed.len());
            for (j, idx) in indexed.into_iter().enumerate() {
                let k = picked[j].0 / gif_step;
                let mut px = idx.clone();
                let mut transparent = None;
                match prev.as_mut() {
                    Some(shown) if k > 0 => {
                        // Keep what is on screen when it is the same index, or (lossy) when it is still within
                        // `tol` of the true colour, and no further from it than the new index would be.
                        let t2 = tol * tol;
                        for (p, ((a, s), new)) in px.iter_mut().zip(shown.iter_mut()).zip(&idx).enumerate() {
                            let keep = *new == *s || (tol > 0.0 && {
                                let tr = truth[j][p];
                                let e = |c: [f32; 3]| (c[0] - tr[0]).powi(2) + (c[1] - tr[1]).powi(2) + (c[2] - tr[2]).powi(2);
                                let es = e(labs[*s as usize]);
                                es <= t2 && es <= e(labs[*new as usize]) + t2 * 0.25
                            });
                            if keep { *a = 255; } else { *s = *new; }
                        }
                        transparent = Some(255u8);
                    }
                    _ => *prev = Some(idx),
                }
                frames.push((k, px, transparent));
            }
            let mut encoded: Vec<gif::Frame<'static>> = frames.into_par_iter().map(|(k, px, transparent)| {
                let mut f = gif::Frame::from_indexed_pixels(w as u16, h as u16, px, transparent);
                f.delay = d[k as usize].max(2) as u16;
                f.dispose = gif::DisposalMethod::Keep;
                f.make_lzw_pre_encoded();
                f
            }).collect();
            for f in encoded.drain(..) {
                enc.write_lzw_pre_encoded_frame(&f).map_err(|e| e.to_string())?;
            }
        }
        i = end;
    }
    progress(Progress { stage: "finish", done: n, total: n });

    let mut files = Vec::new();
    if let Some((path, enc, ..)) = gif {
        enc.into_inner().map_err(|e| e.to_string())?;
        std::fs::rename(tmp_path(&path), &path).map_err(|e| e.to_string())?;
        files.push(path);
    }
    if let Some((path, enc, stamps)) = webp {
        let data = enc.finalize(stamps[n as usize]).map_err(|e| format!("webp: {e:?}"))?;
        std::fs::write(tmp_path(&path), &*data).map_err(|e| e.to_string())?;
        std::fs::rename(tmp_path(&path), &path).map_err(|e| e.to_string())?;
        files.push(path);
    }
    if let Some((path, writer, _)) = apng {
        writer.finish().map_err(|e| e.to_string())?;
        std::fs::rename(tmp_path(&path), &path).map_err(|e| e.to_string())?;
        files.push(path);
    }
    if job.formats.contains(&Format::Png) { files.push(png_dir.clone()); }
    if let Some(si) = &sheet_info {
        let d = delays(n, seconds, 1000.0);
        for (p, buf) in sheet.iter().enumerate() {
            let path = job.out_dir.join(&si.pages[p]);
            image::save_buffer(&path, buf, si.columns * w as u32, si.rows * h as u32, image::ExtendedColorType::Rgba8).map_err(|e| e.to_string())?;
            let apath = path.with_extension("json");
            std::fs::write(&apath, atlas_json(&stem, si, p as u32, n, &d)).map_err(|e| e.to_string())?;
            files.push(path);
            files.push(apath);
        }
    }
    if depth_info.is_some() { files.push(depth_dir.clone()); }
    if !bands.is_empty() { files.push(job.out_dir.join(format!("{stem}_layers"))); }

    Ok(SeqOut { files, frames: n, width: w, height: h, sheet: sheet_info, depth: depth_info, layers: bands })
}

/// Atlas in the TexturePacker "hash" layout with Aseprite-style durations and a loop tag.
fn atlas_json(stem: &str, si: &SheetInfo, page: u32, n: u32, delays_ms: &[u32]) -> String {
    let mut frames = serde_json::Map::new();
    let first = page * si.frames_per_page;
    let last = (first + si.frames_per_page).min(n);
    for k in first..last {
        let slot = k - first;
        let (x, y) = ((slot % si.columns) as usize * si.frame_width, (slot / si.columns) as usize * si.frame_height);
        frames.insert(format!("{stem}_{k:04}"), serde_json::json!({
            "frame": {"x": x, "y": y, "w": si.frame_width, "h": si.frame_height},
            "rotated": false, "trimmed": false,
            "spriteSourceSize": {"x": 0, "y": 0, "w": si.frame_width, "h": si.frame_height},
            "sourceSize": {"w": si.frame_width, "h": si.frame_height},
            "duration": delays_ms[k as usize],
        }));
    }
    let others: Vec<String> = si.pages.iter().enumerate().filter(|(p, _)| *p as u32 != page).map(|(_, img)| img.replace(".png", ".json")).collect();
    serde_json::to_string_pretty(&serde_json::json!({
        "frames": frames,
        "meta": {
            "app": "PathForge", "image": si.pages[page as usize], "format": "RGBA8888", "scale": "1",
            "size": {"w": si.columns as usize * si.frame_width, "h": si.rows as usize * si.frame_height},
            "related_multi_packs": others,
            "frameTags": [{"name": "loop", "from": 0, "to": n - 1, "direction": "forward"}],
        }
    })).unwrap_or_default()
}

fn write_png(path: &Path, w: usize, h: usize, color: png::ColorType, depth: png::BitDepth, data: &[u8]) -> Result<(), String> {
    let file = BufWriter::new(File::create(path).map_err(|e| format!("{}: {e}", path.display()))?);
    let mut enc = png::Encoder::new(file, w as u32, h as u32);
    enc.set_color(color);
    enc.set_depth(depth);
    let mut wr = enc.write_header().map_err(|e| e.to_string())?;
    wr.write_image_data(data).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transitions_start_on_one_loop_and_end_on_the_other() {
        use crate::world::{RenderOptions, WorldRenderer};
        let get = |n: &str| crate::scene::presets::ALL.iter().find(|(m, _)| *m == n).unwrap().1();
        for (an, bn) in [("Forest Path", "Haunted Forest"), ("Stone Dungeon", "Forest Path"), ("Forest Path", "Stone Dungeon")] {
            let (a, b) = (get(an), get(bn));
            let c = crate::scene::transition::plan(&a, &b, &Transition::default());
            let (seq, walk, duration) = transition_plan(&a, &b, &c);
            let total = walk.length();
            let opts = RenderOptions { size: Some((120, 214)), ..RenderOptions::default() };
            for gpu in engines() {
            let (mut rt, mut r) = (renderer(gpu), renderer(gpu));
            let (d0, t0) = seq.frames[0];
            let first = transition_frame(&mut rt, &a, None, &b, None, &walk, duration, d0, t0.unwrap(), &opts);
            assert!(first.rgba == r.render(&a, 0.0, &RenderOptions { time: Some(0.0), ..opts.clone() }).rgba, "{an} -> {bn}: first frame is not loop A frame 0");
            let end = transition_frame(&mut rt, &a, None, &b, None, &walk, duration, total, duration, &opts);
            assert!(end.rgba == r.render(&b, 0.0, &RenderOptions { time: Some(0.0), ..opts.clone() }).rgba, "{an} -> {bn}: the clip does not end on loop B frame 0");
            // Just before the boundary both worlds show: neither world's own frame.
            let at = c.approach - 3.0;
            let mid = transition_frame(&mut rt, &a, None, &b, None, &walk, duration, at, duration * at / total, &opts);
            let (_, sa, sb) = walk.at(at);
            let da = crate::review::diff(&mid.rgba, &r.render(&a, sa, &opts).rgba, 0).0;
            let dbb = crate::review::diff(&mid.rgba, &r.render(&b, sb, &opts).rgba, 0).0;
            assert!(da > 1.0 && dbb > 1.0, "{an} -> {bn}: near the boundary both worlds should show: {da} {dbb}");
            }
        }
    }

    /// The engines a test can compare: the CPU, and the GPU where there is one.
    fn engines() -> Vec<bool> {
        let mut e = vec![false];
        #[cfg(feature = "gpu")]
        if crate::world::gpu::shared().is_some() { e.push(true); }
        e
    }
    fn renderer(gpu: bool) -> crate::world::WorldRenderer { if gpu { crate::world::WorldRenderer::auto() } else { crate::world::WorldRenderer::default() } }

    #[test]
    fn fork_clips_join_the_approach_and_end_on_each_loop() {
        use crate::journey::{ForkWalk, Place};
        use crate::scene::transition::{plan_fork, Branch, ForkChoice};
        use crate::world::{RenderOptions, WorldRenderer};
        let get = |n: &str| crate::scene::presets::ALL.iter().find(|(m, _)| *m == n).unwrap().1();
        let (a, l, r) = (get("Forest Path"), get("Mountain Pass"), get("Desert Canyon"));
        let plan = plan_fork(&a, &l, &r, &ForkChoice::default());
        let opts = RenderOptions { size: Some((90, 160)), ..RenderOptions::default() };
        for gpu in engines() {
        let mut rr = renderer(gpu);
        let (pa, pl, pr) = (Place { scene: &a, dir: None }, Place { scene: &l, dir: None }, Place { scene: &r, dir: None });
        let open = ForkWalk::new(plan.clone(), 0.0, [0.0, 0.0]);
        let at = open.decide_by();
        let shared = open.frame(&mut rr, pa, pl, pr, at - 0.01, [1.0, -5.0, -6.0], &opts);
        for b in [Branch::Left, Branch::Right] {
            let mut w = open.clone();
            w.choose(b, at);
            // Where the branch clip begins it is still the approach.
            let first = w.frame(&mut rr, pa, pl, pr, at - 0.01, [1.0, -5.0, -6.0], &opts);
            assert!(first.rgba == shared.rgba, "{b:?}: the branch should begin exactly where the approach ends");
            let end = w.frame(&mut rr, pa, pl, pr, w.length(), [9.0, 0.0, 0.0], &opts);
            let own = rr.render(if b == Branch::Left { &l } else { &r }, 0.0, &RenderOptions { time: Some(0.0), ..opts.clone() });
            assert!(end.rgba == own.rgba, "{b:?}: the branch should end on its loop's frame 0");
        }
        // Halfway down the approach both branches show: it is neither branch world alone.
        let mid = open.frame(&mut rr, pa, pl, pr, plan.approach * 0.85, [1.0, 1.0, 1.0], &opts);
        let (_, _, sb, _) = open.at(plan.approach * 0.85);
        assert!(crate::review::diff(&mid.rgba, &rr.render(&l, sb[0], &opts).rgba, 0).0 > 1.0);
        }
    }

    #[test]
    fn encounter_clips_join_the_loop_exactly() {
        use crate::world::{RenderOptions, WorldRenderer};
        for (name, start) in [("Stone Dungeon", 0u32), ("Night Road", 37)] {
            let s = crate::scene::presets::ALL.iter().find(|(n, _)| *n == name).unwrap().1();
            let (stop, idle, go, _) = encounter_clips(&s, &Encounter { slow_m: 2.7, start_frame: start });
            let mut r = WorldRenderer::default();
            let size = Some((120, 214));
            let at = |r: &mut WorldRenderer, d: f32, t: Option<f32>| r.render(&s, d, &RenderOptions { size, time: t, ..RenderOptions::default() }).rgba;
            let n = s.motion.frames();
            let loop_d = s.motion.loop_length * start as f32 / n as f32;
            let entry = at(&mut r, loop_d, None);
            // stop begins on the loop frame
            let (d, t) = stop.frames[0];
            assert!(crate::review::diff(&at(&mut r, d, t), &entry, 0).0 < 0.01, "{name}: stop does not start on loop frame {start}");
            // stop's next frame would be idle's first; idle wraps onto itself
            let next_stop = (stop.frames[0].0 + 2.7, Some(stop.frames[0].1.unwrap() + stop.seconds));
            assert!(crate::review::diff(&at(&mut r, next_stop.0, next_stop.1), &at(&mut r, idle.frames[0].0, idle.frames[0].1), 0).0 < 0.01, "{name}: stop -> idle");
            // The checks only mean something if standing still is not a frozen picture.
            let third = idle.frames[idle.frames.len() / 3];
            assert!(crate::review::diff(&at(&mut r, third.0, third.1), &at(&mut r, idle.frames[0].0, idle.frames[0].1), 0).0 > 0.05, "{name}: idle is frozen");
            let after_idle = (idle.frames[0].0, Some(idle.frames[0].1.unwrap() + idle.seconds));
            assert!(crate::review::diff(&at(&mut r, after_idle.0, after_idle.1), &at(&mut r, idle.frames[0].0, idle.frames[0].1), 0).0 < 0.01, "{name}: idle does not loop");
            assert!(crate::review::diff(&at(&mut r, go.frames[0].0, go.frames[0].1), &at(&mut r, idle.frames[0].0, idle.frames[0].1), 0).0 < 0.01, "{name}: idle -> go");
            // go's next frame is the loop frame it entered from
            let last = *go.frames.last().unwrap();
            let step = go.seconds / go.frames.len() as f32;
            let end_t = last.1.unwrap() + step;
            let end_d = s.motion.loop_length * (((last.0 - loop_d) / s.motion.loop_length).round()) + loop_d;
            assert!(crate::review::diff(&at(&mut r, end_d, Some(end_t)), &entry, 0).0 < 0.01, "{name}: go does not hand back to loop frame {start}");
        }
    }

    #[test]
    fn depth_matches_the_camera_mapping_games_use() {
        use crate::world::{RenderOptions, View, WorldRenderer};
        for name in ["Stone Dungeon", "Forest Path"] {
            let s = crate::scene::presets::ALL.iter().find(|(n, _)| *n == name).unwrap().1();
            let img = WorldRenderer::default().render(&s, 0.0, &RenderOptions { depth: true, ..RenderOptions::default() });
            let (w, h) = (img.width, img.height);
            let depth = img.depth.unwrap();
            let view = View::new(&s, w, h);
            for row in [h * 70 / 100, h * 80 / 100, h * 95 / 100] {
                let want = crate::mcp::ground_distance(&view, row as f32 + 0.5).unwrap();
                let got = depth[row * w + w / 2];
                assert!((got - want).abs() / want < 0.03, "{name} row {row}: depth {got} vs camera mapping {want}");
            }
        }
    }

    #[test]
    fn delays_sum_to_the_loop_exactly() {
        for (frames, secs) in [(36u32, 1.0f32), (144, 6.0), (7, 1.3), (60, 1.2)] {
            let d = delays(frames, secs, 100.0);
            assert_eq!(d.iter().sum::<u32>(), (secs * 100.0).round() as u32, "{frames} frames over {secs}s");
        }
    }

    #[test]
    fn the_size_estimate_is_close_to_what_the_export_writes() {
        let s = crate::scene::presets::ALL.iter().find(|p| p.0 == "Stone Dungeon").map(|p| (p.1)()).unwrap();
        let dir = std::env::temp_dir().join(format!("pf_est_test_{}", std::process::id()));
        let job = ExportJob { formats: vec![Format::Webp, Format::Gif], out_dir: dir.clone(), name: "t".into(), size: Some((120, 214)), frames: Some(48), ..ExportJob::default() };
        let est = estimate(&s, &job, 6).unwrap();
        let rep = export(&s, &job, |_| {}, &AtomicBool::new(false)).unwrap();
        for ext in ["webp", "gif"] {
            let real = rep.files.iter().find(|f| f.extension().is_some_and(|e| e == ext)).map(|f| std::fs::metadata(f).unwrap().len()).unwrap();
            let guess = est.parts.iter().find(|p| p.0.ends_with(ext)).map(|p| p.1).unwrap();
            let ratio = guess as f64 / real as f64;
            assert!((0.6..1.6).contains(&ratio), "{ext}: estimated {guess} bytes, wrote {real} ({ratio:.2}x)");
        }
        let p = est.preview.expect("a preview from the GIF");
        assert_eq!((p.width, p.height), (120, 214));
        let first = crate::review::decode_gif(&dir.join("t.gif")).unwrap().swap_remove(0);
        assert!(p.rgba == first.rgba, "the preview is the GIF's own first frame");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
