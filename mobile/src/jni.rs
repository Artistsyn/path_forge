//! The Android app's calls (`com.pathforge.mobile.Native`), over JNI.

use crate::{guard, Handle};
use jni::objects::{JByteBuffer, JClass, JString};
use jni::sys::{jboolean, jfloat, jfloatArray, jint, jlong, jstring, JNI_FALSE, JNI_TRUE};
use jni::JNIEnv;

fn string(env: &mut JNIEnv, s: &JString) -> Result<String, String> {
    if s.is_null() { return Ok(String::new()); }
    env.get_string(s).map(Into::into).map_err(|e| e.to_string())
}

fn jstr(env: &mut JNIEnv, s: &str) -> jstring {
    env.new_string(s).map(|j| j.into_raw()).unwrap_or(std::ptr::null_mut())
}

/// The handle behind a Java long.
fn handle<'a>(h: jlong) -> Result<&'a mut Handle, String> {
    if h == 0 { return Err("no scene open".into()); }
    // SAFETY: handles are only made by `open` and freed by `close`; the app uses each from one
    // thread at a time.
    Ok(unsafe { &mut *(h as *mut Handle) })
}

#[no_mangle]
pub extern "system" fn Java_com_pathforge_mobile_Native_presetNames(mut env: JNIEnv, _: JClass) -> jstring {
    let s = guard(String::from("[]"), || Ok(crate::preset_names()));
    jstr(&mut env, &s)
}

#[no_mangle]
pub extern "system" fn Java_com_pathforge_mobile_Native_presetScene(mut env: JNIEnv, _: JClass, name: JString) -> jstring {
    let r = guard(None, || { let n = string(&mut env, &name)?; Ok(crate::preset_scene(&n)) });
    match r { Some(s) => jstr(&mut env, &s), None => std::ptr::null_mut() }
}

#[no_mangle]
pub extern "system" fn Java_com_pathforge_mobile_Native_form(mut env: JNIEnv, _: JClass) -> jstring {
    let s = guard(String::from("{\"parts\":[]}"), || Ok(crate::form()));
    jstr(&mut env, &s)
}

#[no_mangle]
pub extern "system" fn Java_com_pathforge_mobile_Native_normalize(mut env: JNIEnv, _: JClass, json: JString) -> jstring {
    let r = guard(None, || { let j = string(&mut env, &json)?; crate::normalize(&j).map(Some) });
    match r { Some(s) => jstr(&mut env, &s), None => std::ptr::null_mut() }
}

#[no_mangle]
pub extern "system" fn Java_com_pathforge_mobile_Native_lastError(mut env: JNIEnv, _: JClass) -> jstring {
    let e = crate::last_error();
    jstr(&mut env, &e)
}

#[no_mangle]
pub extern "system" fn Java_com_pathforge_mobile_Native_gpuInit(mut env: JNIEnv, _: JClass) -> jstring {
    let name = guard(None, || Ok(crate::gpu().map(|g| g.1))).unwrap_or_default();
    jstr(&mut env, &name)
}

#[no_mangle]
pub extern "system" fn Java_com_pathforge_mobile_Native_open(mut env: JNIEnv, _: JClass, json: JString, base_dir: JString, use_gpu: jboolean) -> jlong {
    guard(0, || {
        let (j, d) = (string(&mut env, &json)?, string(&mut env, &base_dir)?);
        let h = Handle::open(&j, Some(&d), use_gpu == JNI_TRUE)?;
        Ok(Box::into_raw(Box::new(h)) as jlong)
    })
}

#[no_mangle]
pub extern "system" fn Java_com_pathforge_mobile_Native_engine(mut env: JNIEnv, _: JClass, h: jlong) -> jstring {
    let e = guard("cpu", || Ok(handle(h)?.engine()));
    jstr(&mut env, e)
}

#[no_mangle]
pub extern "system" fn Java_com_pathforge_mobile_Native_setScene(mut env: JNIEnv, _: JClass, h: jlong, json: JString) -> jboolean {
    guard(JNI_FALSE, || {
        let j = string(&mut env, &json)?;
        handle(h)?.set_scene(&j)?;
        Ok(JNI_TRUE)
    })
}

#[no_mangle]
pub extern "system" fn Java_com_pathforge_mobile_Native_close(_: JNIEnv, _: JClass, h: jlong) {
    if h != 0 {
        // SAFETY: made by `open`, closed once.
        let _ = std::panic::catch_unwind(|| drop(unsafe { Box::from_raw(h as *mut Handle) }));
    }
}

#[no_mangle]
pub extern "system" fn Java_com_pathforge_mobile_Native_info(env: JNIEnv, _: JClass, h: jlong) -> jfloatArray {
    let Some(v) = guard(None, || Ok(Some(handle(h)?.info()))) else { return std::ptr::null_mut() };
    match env.new_float_array(v.len() as i32) {
        Ok(arr) => { let _ = env.set_float_array_region(&arr, 0, &v); arr.into_raw() }
        Err(_) => std::ptr::null_mut(),
    }
}

/// The phone's local time of day, hours (0..24), for scenes that follow the clock.
#[no_mangle]
pub extern "system" fn Java_com_pathforge_mobile_Native_setClock(_: JNIEnv, _: JClass, hour: jfloat) { crate::set_clock(hour); }

/// Into a direct buffer of w * h * 4 bytes (RGBA, as `Bitmap.copyPixelsFromBuffer` reads an
/// ARGB_8888 bitmap).
#[no_mangle]
pub extern "system" fn Java_com_pathforge_mobile_Native_render(env: JNIEnv, _: JClass, h: jlong, distance: jfloat, time: jfloat, w: jint, hh: jint, buf: JByteBuffer) -> jboolean {
    guard(JNI_FALSE, || {
        if w <= 0 || hh <= 0 { return Err("empty frame".into()); }
        let ptr = env.get_direct_buffer_address(&buf).map_err(|e| e.to_string())?;
        let cap = env.get_direct_buffer_capacity(&buf).map_err(|e| e.to_string())?;
        // SAFETY: a direct buffer's memory stays put while Java holds it, as the caller does.
        let out = unsafe { std::slice::from_raw_parts_mut(ptr, cap) };
        handle(h)?.render(distance, time, w as u32, hh as u32, out)?;
        Ok(JNI_TRUE)
    })
}

/// The loop as an animated GIF or WebP in `dir`; its path, or null (see lastError).
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn Java_com_pathforge_mobile_Native_exportAnimation(mut env: JNIEnv, _: JClass, json: JString, dir: JString, name: JString, format: JString,
    w: jint, h: jint, fps: jint, quality: jfloat, gpu: jboolean) -> jstring {
    let r = guard(None, || {
        let (j, d, n, f) = (string(&mut env, &json)?, string(&mut env, &dir)?, string(&mut env, &name)?, string(&mut env, &format)?);
        let a = crate::Animation { format: &f, width: w.max(1) as u32, height: h.max(1) as u32, fps: fps.max(1) as u32, quality, gpu: gpu == JNI_TRUE };
        crate::export_animation(&j, &d, &n, &a).map(Some)
    });
    match r { Some(p) => jstr(&mut env, &p), None => std::ptr::null_mut() }
}

/// [frames done, of how many] of the export running now.
#[no_mangle]
pub extern "system" fn Java_com_pathforge_mobile_Native_exportProgress(env: JNIEnv, _: JClass) -> jni::sys::jintArray {
    let (d, t) = crate::export_progress();
    match env.new_int_array(2) {
        Ok(arr) => { let _ = env.set_int_array_region(&arr, 0, &[d as i32, t as i32]); arr.into_raw() }
        Err(_) => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_pathforge_mobile_Native_exportCancel(_: JNIEnv, _: JClass) { crate::export_cancel(); }

#[no_mangle]
pub extern "system" fn Java_com_pathforge_mobile_Native_remix(mut env: JNIEnv, _: JClass, json: JString, keep: JString, seed: jint) -> jstring {
    let r = guard(None, || { let (j, k) = (string(&mut env, &json)?, string(&mut env, &keep)?); crate::remix(&j, &k, seed as u32).map(Some) });
    match r { Some(s) => jstr(&mut env, &s), None => std::ptr::null_mut() }
}

#[no_mangle]
pub extern "system" fn Java_com_pathforge_mobile_Native_shareCode(mut env: JNIEnv, _: JClass, json: JString) -> jstring {
    let r = guard(None, || { let j = string(&mut env, &json)?; crate::share_code(&j).map(Some) });
    match r { Some(s) => jstr(&mut env, &s), None => std::ptr::null_mut() }
}

#[no_mangle]
pub extern "system" fn Java_com_pathforge_mobile_Native_unshare(mut env: JNIEnv, _: JClass, code: JString) -> jstring {
    let r = guard(None, || { let c = string(&mut env, &code)?; crate::unshare(&c).map(Some) });
    match r { Some(s) => jstr(&mut env, &s), None => std::ptr::null_mut() }
}

#[no_mangle]
pub extern "system" fn Java_com_pathforge_mobile_Native_qr(mut env: JNIEnv, _: JClass, text: JString) -> jni::sys::jbyteArray {
    let r = guard(None, || { let t = string(&mut env, &text)?; crate::qr(&t).map(Some) });
    match r { Some(b) => env.byte_array_from_slice(&b).map(|a| a.into_raw()).unwrap_or(std::ptr::null_mut()), None => std::ptr::null_mut() }
}
