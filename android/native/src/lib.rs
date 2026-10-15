//! PathForge on Android: the phone app and its live wallpaper call these through JNI
//! (`com.pathforge.mobile.Native`). A handle is one `Runtime`, used from one thread at a time;
//! the app's preview and the wallpaper each open their own. Scenes go back and forth as JSON, so
//! the app edits them with Android's own JSON classes and only the drawing happens here.
//!
//! Frames are drawn on the phone's GPU (wgpu on Vulkan) where it can, sharing one GPU renderer
//! between handles so its pipelines are built once, and on the CPU where it cannot: a frame the
//! GPU fails is drawn again on the CPU, and from then on every handle stays on the CPU.

use jni::objects::{JByteBuffer, JClass, JString};
use jni::sys::{jboolean, jfloat, jfloatArray, jint, jlong, jstring, JNI_FALSE, JNI_TRUE};
use jni::JNIEnv;
use path_forge::runtime::Runtime;
use path_forge::scene::{presets, Scene};
use path_forge::world::gpu::{Gpu, GpuContext};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// The last thing that went wrong, for `lastError`.
static LAST_ERROR: Mutex<String> = Mutex::new(String::new());

/// The GPU renderer every handle shares, made on first use (None: no usable GPU).
static GPU: OnceLock<Option<(Arc<Gpu>, String)>> = OnceLock::new();
/// Set once the GPU has failed a frame: everything draws on the CPU from then on.
static GPU_BROKEN: AtomicBool = AtomicBool::new(false);

/// The shared GPU renderer and its adapter's name, making it on first call.
fn gpu() -> Option<(Arc<Gpu>, String)> {
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

fn fail(msg: impl Into<String>) { *LAST_ERROR.lock().unwrap_or_else(|e| e.into_inner()) = msg.into(); }

/// Run `f`, turning a panic into an error instead of unwinding into Java.
fn guard<T>(fallback: T, f: impl FnOnce() -> Result<T, String>) -> T {
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

fn string(env: &mut JNIEnv, s: &JString) -> Result<String, String> {
    if s.is_null() { return Ok(String::new()); }
    env.get_string(s).map(Into::into).map_err(|e| e.to_string())
}

fn jstr(env: &mut JNIEnv, s: &str) -> jstring {
    env.new_string(s).map(|j| j.into_raw()).unwrap_or(std::ptr::null_mut())
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

/// The runtime behind a handle.
fn rt<'a>(h: jlong) -> Result<&'a mut Runtime, String> {
    if h == 0 { return Err("no scene open".into()); }
    // SAFETY: handles are only made by `open` and freed by `close`; the app uses each from one
    // thread at a time.
    Ok(unsafe { &mut *(h as *mut Runtime) })
}

#[no_mangle]
pub extern "system" fn Java_com_pathforge_mobile_Native_presetNames(mut env: JNIEnv, _: JClass) -> jstring {
    let s = guard(String::from("[]"), || Ok(preset_names()));
    jstr(&mut env, &s)
}

#[no_mangle]
pub extern "system" fn Java_com_pathforge_mobile_Native_presetScene(mut env: JNIEnv, _: JClass, name: JString) -> jstring {
    let r = guard(None, || { let n = string(&mut env, &name)?; Ok(preset_scene(&n)) });
    match r { Some(s) => jstr(&mut env, &s), None => std::ptr::null_mut() }
}

#[no_mangle]
pub extern "system" fn Java_com_pathforge_mobile_Native_normalize(mut env: JNIEnv, _: JClass, json: JString) -> jstring {
    let r = guard(None, || { let j = string(&mut env, &json)?; normalize(&j).map(Some) });
    match r { Some(s) => jstr(&mut env, &s), None => std::ptr::null_mut() }
}

#[no_mangle]
pub extern "system" fn Java_com_pathforge_mobile_Native_lastError(mut env: JNIEnv, _: JClass) -> jstring {
    let e = LAST_ERROR.lock().map(|e| e.clone()).unwrap_or_default();
    jstr(&mut env, &e)
}

/// Start the GPU renderer (building its pipelines takes a moment): its adapter's name, or ""
/// where there is none (see lastError).
#[no_mangle]
pub extern "system" fn Java_com_pathforge_mobile_Native_gpuInit(mut env: JNIEnv, _: JClass) -> jstring {
    let name = gpu().map(|g| g.1).unwrap_or_default();
    jstr(&mut env, &name)
}

/// Open a scene (JSON; `base_dir` is where its files are, or empty), drawn on the GPU when
/// `use_gpu` and there is one. 0 on failure (see lastError).
#[no_mangle]
pub extern "system" fn Java_com_pathforge_mobile_Native_open(mut env: JNIEnv, _: JClass, json: JString, base_dir: JString, use_gpu: jboolean) -> jlong {
    guard(0, || {
        let (j, d) = (string(&mut env, &json)?, string(&mut env, &base_dir)?);
        #[allow(unused_mut)]
        let mut rt = Runtime::from_json(&j, (!d.is_empty()).then(|| d.into()))?;
        if use_gpu == JNI_TRUE {
            if let Some((g, _)) = gpu() { rt.set_gpu(Some(g)); }
        }
        Ok(Box::into_raw(Box::new(rt)) as jlong)
    })
}

/// "gpu" or "cpu": which engine draws a handle's frames now.
#[no_mangle]
pub extern "system" fn Java_com_pathforge_mobile_Native_engine(mut env: JNIEnv, _: JClass, h: jlong) -> jstring {
    let e = guard("cpu", || Ok(rt(h)?.engine()));
    jstr(&mut env, e)
}

/// Swap the scene a handle draws.
#[no_mangle]
pub extern "system" fn Java_com_pathforge_mobile_Native_setScene(mut env: JNIEnv, _: JClass, h: jlong, json: JString) -> jboolean {
    guard(JNI_FALSE, || {
        let j = string(&mut env, &json)?;
        rt(h)?.set_scene(Scene::from_json(&j)?);
        Ok(JNI_TRUE)
    })
}

#[no_mangle]
pub extern "system" fn Java_com_pathforge_mobile_Native_close(_: JNIEnv, _: JClass, h: jlong) {
    if h != 0 {
        // SAFETY: made by `open`, closed once.
        let _ = catch_unwind(|| drop(unsafe { Box::from_raw(h as *mut Runtime) }));
    }
}

/// [loop length (m), loop seconds, fps, canvas width, canvas height].
#[no_mangle]
pub extern "system" fn Java_com_pathforge_mobile_Native_info(env: JNIEnv, _: JClass, h: jlong) -> jfloatArray {
    let v = guard(None, || {
        let r = rt(h)?;
        let (w, hh) = r.canvas();
        Ok(Some([r.loop_length(), r.loop_seconds(), r.scene().motion.fps as f32, w as f32, hh as f32]))
    });
    let Some(v) = v else { return std::ptr::null_mut() };
    match env.new_float_array(v.len() as i32) {
        Ok(arr) => { let _ = env.set_float_array_region(&arr, 0, &v); arr.into_raw() }
        Err(_) => std::ptr::null_mut(),
    }
}

/// Draw `distance` metres along at `time` seconds into a direct buffer of `w * h * 4` bytes
/// (RGBA, as `Bitmap.copyPixelsFromBuffer` reads an ARGB_8888 bitmap).
#[no_mangle]
pub extern "system" fn Java_com_pathforge_mobile_Native_render(env: JNIEnv, _: JClass, h: jlong, distance: jfloat, time: jfloat, w: jint, hh: jint, buf: JByteBuffer) -> jboolean {
    guard(JNI_FALSE, || {
        if w <= 0 || hh <= 0 { return Err("empty frame".into()); }
        let ptr = env.get_direct_buffer_address(&buf).map_err(|e| e.to_string())?;
        let cap = env.get_direct_buffer_capacity(&buf).map_err(|e| e.to_string())?;
        // SAFETY: a direct buffer's memory stays put while Java holds it, as the caller does.
        let out = unsafe { std::slice::from_raw_parts_mut(ptr, cap) };
        let r = rt(h)?;
        if r.engine() == "gpu" {
            if !GPU_BROKEN.load(Ordering::Relaxed) {
                match catch_unwind(AssertUnwindSafe(|| r.render_into(distance, time, w as u32, hh as u32, out))) {
                    Ok(Ok(())) => return Ok(JNI_TRUE),
                    Ok(Err(e)) => fail(format!("GPU frame failed, drawing on the CPU: {e}")),
                    Err(_) => fail("GPU frame failed, drawing on the CPU"),
                }
                GPU_BROKEN.store(true, Ordering::Relaxed);
            }
            r.set_gpu(None);
        }
        r.render_into(distance, time, w as u32, hh as u32, out)?;
        Ok(JNI_TRUE)
    })
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
        let mut rt = Runtime::from_json(&json, None).unwrap();
        let mut buf = vec![0u8; 36 * 64 * 4];
        rt.render_into(1.0, 0.5, 36, 64, &mut buf).unwrap();
        assert!(buf.chunks(4).all(|p| p[3] == 255), "every pixel opaque");
        assert!(preset_scene("No Such Place").is_none());
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
