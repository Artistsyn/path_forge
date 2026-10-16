//! PathForge on phones: what the Android app (over JNI, `jni.rs`) and the iPhone app (over a C
//! ABI, `capi.rs`, declared in `ios/PathForge/PathForge.h`) call. A `Handle` is one `Runtime`,
//! used from one thread at a time; the apps' previews, exports and wallpapers each open their own.
//! Scenes go back and forth as JSON, so the apps edit them with their platform's own JSON classes
//! and only the drawing happens here.
//!
//! Frames are drawn on the phone's GPU (wgpu: Vulkan on Android, Metal on iOS) where it can,
//! sharing one GPU renderer between handles so its pipelines are built once, and on the CPU where
//! it cannot: a frame the GPU fails is drawn again on the CPU, and from then on every handle stays
//! on the CPU.

use path_forge::runtime::Runtime;
use path_forge::scene::{presets, Scene};
use path_forge::world::gpu::{Gpu, GpuContext};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

pub mod capi;
#[cfg(target_os = "android")]
mod jni;

/// The last thing that went wrong, for the apps to show.
static LAST_ERROR: Mutex<String> = Mutex::new(String::new());
/// The GPU renderer every handle shares, made on first use (None: no usable GPU).
static GPU: OnceLock<Option<(Arc<Gpu>, String)>> = OnceLock::new();
/// Set once the GPU has failed a frame: everything draws on the CPU from then on.
static GPU_BROKEN: AtomicBool = AtomicBool::new(false);

pub fn fail(msg: impl Into<String>) { *LAST_ERROR.lock().unwrap_or_else(|e| e.into_inner()) = msg.into(); }

pub fn last_error() -> String { LAST_ERROR.lock().map(|e| e.clone()).unwrap_or_default() }

/// Run `f`, turning an error or a panic into `fallback` (and the message into `last_error`)
/// instead of unwinding into Java or Swift.
pub fn guard<T>(fallback: T, f: impl FnOnce() -> Result<T, String>) -> T {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => { fail(e); fallback }
        Err(p) => {
            let msg = p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_else(|| "panic".into());
            fail(format!("PathForge panicked: {msg}"));
            fallback
        }
    }
}

/// The shared GPU renderer and its adapter's name, making it on first call.
pub fn gpu() -> Option<(Arc<Gpu>, String)> {
    if GPU_BROKEN.load(Ordering::Relaxed) { return None; }
    GPU.get_or_init(|| {
        let made = catch_unwind(|| -> Result<(Arc<Gpu>, String), String> {
            let ctx = GpuContext::headless()?;
            let info = ctx.info.clone();
            Ok((Arc::new(Gpu::new(ctx)?), info))
        });
        match made {
            Ok(Ok(g)) => Some(g),
            Ok(Err(e)) => { fail(format!("GPU unavailable: {e}")); None }
            Err(_) => { fail("GPU unavailable: it failed while starting"); None }
        }
    }).clone()
}

/// The names of the built-in presets, as a JSON array.
pub fn preset_names() -> String {
    serde_json::to_string(&presets::ALL.iter().map(|(n, _)| *n).collect::<Vec<_>>()).unwrap_or_else(|_| "[]".into())
}

/// A built-in preset's scene as JSON.
pub fn preset_scene(name: &str) -> Option<String> {
    presets::ALL.iter().find(|(n, _)| *n == name).and_then(|(_, f)| serde_json::to_string(&f()).ok())
}

/// "Surprise me": a variation of the scene where the parts named in `keep` (comma separated)
/// stay and the rest come from presets picked by `seed`.
pub fn remix(json: &str, keep: &str, seed: u32) -> Result<String, String> {
    let base = Scene::from_json(json)?;
    let keep: Vec<&str> = keep.split(',').map(str::trim).filter(|k| !k.is_empty()).collect();
    let s = path_forge::scene::remix::remix(&base, &keep, seed, 0.0)?;
    serde_json::to_string(&s).map_err(|e| e.to_string())
}

/// A scene as a share code, and back (path_forge::scene::share).
pub fn share_code(json: &str) -> Result<String, String> { path_forge::scene::share::encode(&Scene::from_json(json)?) }
pub fn unshare(code: &str) -> Result<String, String> {
    serde_json::to_string(&path_forge::scene::share::decode(code)?).map_err(|e| e.to_string())
}

/// A QR code of `text`: its width in modules, then a byte a module (1 dark), row by row.
pub fn qr(text: &str) -> Result<Vec<u8>, String> {
    let (w, dark) = path_forge::scene::share::qr(text)?;
    let mut out = Vec::with_capacity(1 + dark.len());
    out.push(w as u8);
    out.extend(dark.iter().map(|&d| d as u8));
    Ok(out)
}

/// A scene file's JSON read the way the desktop reads it (older formats are converted), written
/// back in the current format.
pub fn normalize(json: &str) -> Result<String, String> {
    let s = Scene::from_json(json)?;
    serde_json::to_string(&s).map_err(|e| e.to_string())
}

/// How far the export running now has got (frames done, of how many), and a request to stop it.
static EXPORT_DONE: AtomicU32 = AtomicU32::new(0);
static EXPORT_TOTAL: AtomicU32 = AtomicU32::new(0);
static EXPORT_CANCEL: AtomicBool = AtomicBool::new(false);

/// What an export of the loop is to be.
pub struct Animation<'a> {
    /// "gif" or "webp".
    pub format: &'a str,
    pub width: u32,
    pub height: u32,
    /// Frames a second (the loop's length decides how many frames).
    pub fps: u32,
    /// WebP quality 0..100 (100 lossless); GIF: how much smaller to make it at some cost, 0..1.
    pub quality: f32,
    pub gpu: bool,
}

/// Write the scene's whole loop as an animated GIF or WebP that loops without a seam, into `dir`
/// as `name`.gif / .webp; returns its path. `export_progress` tells how far it has got and
/// `export_cancel` stops it.
pub fn export_animation(json: &str, dir: &str, name: &str, a: &Animation) -> Result<String, String> {
    use path_forge::export::{export_with, ExportJob, Format};
    let scene = Scene::from_json(json)?;
    let format = match a.format { "gif" => Format::Gif, "webp" => Format::Webp, f => return Err(format!("no such animation format: {f}")) };
    let frames = ((scene.motion.loop_seconds() * a.fps.clamp(4, 60) as f32).round() as u32).clamp(8, 1200);
    let job = ExportJob {
        formats: vec![format], out_dir: dir.into(), name: name.to_string(), frames: Some(frames),
        size: Some((a.width.clamp(16, 4096), a.height.clamp(16, 4096))),
        gif_lossy: if format == Format::Gif { a.quality.clamp(0.0, 1.0) * 0.03 } else { 0.0 },
        webp_quality: (format == Format::Webp).then_some(a.quality.clamp(0.0, 100.0)),
        ..ExportJob::default()
    };
    let mut renderer = path_forge::world::WorldRenderer::default();
    if a.gpu { if let Some((g, _)) = gpu() { renderer.set_gpu(Some(g)); } }
    EXPORT_CANCEL.store(false, Ordering::Relaxed);
    EXPORT_DONE.store(0, Ordering::Relaxed);
    EXPORT_TOTAL.store(frames, Ordering::Relaxed);
    let report = export_with(&scene, &job, &mut renderer, |p| {
        EXPORT_DONE.store(p.done, Ordering::Relaxed);
        EXPORT_TOTAL.store(p.total.max(1), Ordering::Relaxed);
    }, &EXPORT_CANCEL)?;
    let ext = if format == Format::Gif { "gif" } else { "webp" };
    // Only the animation is wanted on a phone: not the game's metadata beside it.
    for f in &report.files {
        if f.extension().is_some_and(|e| e == "json") { let _ = std::fs::remove_file(f); }
    }
    report.files.iter().find(|f| f.extension().is_some_and(|e| e == ext)).map(|f| f.to_string_lossy().into_owned())
        .ok_or_else(|| "the export wrote no animation".to_string())
}

/// (frames done, of how many) of the export running now.
pub fn export_progress() -> (u32, u32) { (EXPORT_DONE.load(Ordering::Relaxed), EXPORT_TOTAL.load(Ordering::Relaxed)) }

pub fn export_cancel() { EXPORT_CANCEL.store(true, Ordering::Relaxed); }

/// Every scene setting as an editor builds itself from it (`path_forge::scene::form::form`).
pub fn form() -> String { path_forge::scene::form::form().to_string() }

/// The device's clock, hours 0..24 local time, as the platform last gave it (`set_clock`); NaN
/// until it does.
static CLOCK: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x7fc0_0000);

/// The device's local time of day in hours (0..24), for scenes whose day and night follow the
/// clock (`sky.day_night.real_time`). A negative hour forgets it.
pub fn set_clock(hour: f32) {
    let v = if hour >= 0.0 { hour.rem_euclid(24.0) } else { f32::NAN };
    CLOCK.store(v.to_bits(), Ordering::Relaxed);
}

fn clock() -> Option<f32> { let v = f32::from_bits(CLOCK.load(Ordering::Relaxed)); (!v.is_nan()).then_some(v) }

/// One scene being drawn.
pub struct Handle { rt: Runtime }

impl Handle {
    /// A scene (`base_dir`: where its files are), drawn on the GPU when `use_gpu` and there is one.
    pub fn open(json: &str, base_dir: Option<&str>, use_gpu: bool) -> Result<Handle, String> {
        let mut rt = Runtime::from_json(json, base_dir.filter(|d| !d.is_empty()).map(Into::into))?;
        if use_gpu {
            if let Some((g, _)) = gpu() { rt.set_gpu(Some(g)); }
        }
        Ok(Handle { rt })
    }

    pub fn set_scene(&mut self, json: &str) -> Result<(), String> {
        self.rt.set_scene(Scene::from_json(json)?);
        Ok(())
    }

    /// [loop length (m), loop seconds, fps, canvas width, canvas height].
    pub fn info(&self) -> [f32; 5] {
        let (w, h) = self.rt.canvas();
        [self.rt.loop_length(), self.rt.loop_seconds(), self.rt.scene().motion.fps as f32, w as f32, h as f32]
    }

    /// "gpu" or "cpu": which engine draws now.
    pub fn engine(&self) -> &'static str { self.rt.engine() }

    /// Draw `distance` metres along at `time` seconds into `out` (w * h RGBA bytes). A frame the
    /// GPU fails is drawn on the CPU, which every handle then keeps to.
    pub fn render(&mut self, distance: f32, time: f32, w: u32, h: u32, out: &mut [u8]) -> Result<(), String> {
        if w == 0 || h == 0 { return Err("empty frame".into()); }
        let r = &mut self.rt;
        let dn = &r.scene().sky.day_night;
        r.set_hour(if dn.enabled && dn.real_time { clock() } else { None });
        if r.engine() == "gpu" {
            if !GPU_BROKEN.load(Ordering::Relaxed) {
                match catch_unwind(AssertUnwindSafe(|| r.render_into(distance, time, w, h, out))) {
                    Ok(Ok(())) => return Ok(()),
                    Ok(Err(e)) => fail(format!("GPU frame failed, drawing on the CPU: {e}")),
                    Err(_) => fail("GPU frame failed, drawing on the CPU"),
                }
                GPU_BROKEN.store(true, Ordering::Relaxed);
            }
            r.set_gpu(None);
        }
        r.render_into(distance, time, w, h, out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scene_on_the_clock_follows_it() {
        let mut s: path_forge::scene::Scene = serde_json::from_str(&preset_scene("Forest Path").unwrap()).unwrap();
        s.sky.day_night.enabled = true;
        s.sky.day_night.real_time = true;
        let mut h = Handle::open(&serde_json::to_string(&s).unwrap(), None, false).unwrap();
        let mut at = |hour: f32| { set_clock(hour); let mut b = vec![0u8; 36 * 64 * 4]; h.render(1.0, 0.5, 36, 64, &mut b).unwrap(); b.iter().map(|v| *v as u64).sum::<u64>() };
        let (noon, night) = (at(12.0), at(0.5));
        set_clock(-1.0);
        assert!(noon > night * 2, "noon {noon} vs night {night}");
    }

    #[test]
    fn presets_round_trip_and_render() {
        let names: Vec<String> = serde_json::from_str(&preset_names()).unwrap();
        assert!(names.iter().any(|n| n == "Sunset Pier"));
        let json = preset_scene("Sunset Pier").unwrap();
        assert_eq!(normalize(&json).unwrap(), json);
        let mut h = Handle::open(&json, None, false).unwrap();
        let mut buf = vec![0u8; 36 * 64 * 4];
        h.render(1.0, 0.5, 36, 64, &mut buf).unwrap();
        assert!(buf.chunks(4).all(|p| p[3] == 255), "every pixel opaque");
        assert!(preset_scene("No Such Place").is_none());
        assert!(form().contains("\"parts\""));
        // The loop as an animation, small and short.
        let dir = std::env::temp_dir().join(format!("pfm-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for format in ["gif", "webp"] {
            let path = export_animation(&json, dir.to_str().unwrap(), "loop", &Animation { format, width: 36, height: 64, fps: 4, quality: 80.0, gpu: false }).unwrap();
            let bytes = std::fs::read(&path).unwrap();
            assert!(path.ends_with(format) && bytes.len() > 100, "{path}: {} bytes", bytes.len());
            assert_eq!(export_progress().0, export_progress().1, "all frames done");
        }
        assert!(std::fs::read_dir(&dir).unwrap().all(|f| !f.unwrap().path().to_string_lossy().ends_with(".json")), "no metadata left beside it");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_shared_gpu_draws_what_the_cpu_draws() {
        let Some((g, name)) = gpu() else { eprintln!("no GPU here, skipped"); return };
        eprintln!("GPU: {name}");
        let json = preset_scene("Sunset Pier").unwrap();
        let (mut a, mut b) = (Runtime::from_json(&json, None).unwrap(), Runtime::from_json(&json, None).unwrap());
        b.set_gpu(Some(g));
        assert_eq!((a.engine(), b.engine()), ("cpu", "gpu"));
        let (mut ca, mut cb) = (vec![0u8; 90 * 160 * 4], vec![0u8; 90 * 160 * 4]);
        a.render_into(3.0, 1.0, 90, 160, &mut ca).unwrap();
        b.render_into(3.0, 1.0, 90, 160, &mut cb).unwrap();
        let worst = ca.iter().zip(&cb).map(|(x, y)| x.abs_diff(*y)).max().unwrap();
        assert!(worst <= 4, "GPU and CPU differ by {worst}");
    }
}
