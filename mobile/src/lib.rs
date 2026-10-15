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
use std::sync::atomic::{AtomicBool, Ordering};
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

/// A scene file's JSON read the way the desktop reads it (older formats are converted), written
/// back in the current format.
pub fn normalize(json: &str) -> Result<String, String> {
    let s = Scene::from_json(json)?;
    serde_json::to_string(&s).map_err(|e| e.to_string())
}

/// Every scene setting as an editor builds itself from it (`path_forge::scene::form::form`).
pub fn form() -> String { path_forge::scene::form::form().to_string() }

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
