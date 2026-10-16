//! The iPhone app's calls, over a C ABI (declared in `ios/PathForge/PathForge.h`). Strings the
//! library returns are the caller's to free with `pfm_free`.

use crate::{guard, Handle};
use std::ffi::{c_char, CStr, CString};

fn arg(s: *const c_char) -> Result<String, String> {
    if s.is_null() { return Ok(String::new()); }
    // SAFETY: the caller passes a NUL-terminated string, alive for the call.
    unsafe { CStr::from_ptr(s) }.to_str().map(str::to_owned).map_err(|e| e.to_string())
}

fn out(s: String) -> *mut c_char {
    CString::new(s.replace('\0', "")).map(CString::into_raw).unwrap_or(std::ptr::null_mut())
}

fn handle<'a>(h: *mut Handle) -> Result<&'a mut Handle, String> {
    // SAFETY: handles are only made by `pfm_open` and freed by `pfm_close`; the app uses each
    // from one thread at a time.
    unsafe { h.as_mut() }.ok_or_else(|| "no scene open".to_string())
}

/// Free a string the library returned.
#[no_mangle]
pub extern "C" fn pfm_free(s: *mut c_char) {
    if !s.is_null() {
        // SAFETY: made by `out`.
        drop(unsafe { CString::from_raw(s) });
    }
}

/// The built-in presets' names, as a JSON array.
#[no_mangle]
pub extern "C" fn pfm_preset_names() -> *mut c_char { out(guard(String::from("[]"), || Ok(crate::preset_names()))) }

/// A preset's scene as JSON, or NULL.
#[no_mangle]
pub extern "C" fn pfm_preset_scene(name: *const c_char) -> *mut c_char {
    match guard(None, || Ok(crate::preset_scene(&arg(name)?))) { Some(s) => out(s), None => std::ptr::null_mut() }
}

/// Every scene setting as an editor builds itself from it (path_forge::scene::form::form), as JSON.
#[no_mangle]
pub extern "C" fn pfm_form() -> *mut c_char { out(guard(String::from("{\"parts\":[]}"), || Ok(crate::form()))) }

/// A scene file read as the desktop reads it (older formats converted), or NULL (see pfm_last_error).
#[no_mangle]
pub extern "C" fn pfm_normalize(json: *const c_char) -> *mut c_char {
    match guard(None, || crate::normalize(&arg(json)?).map(Some)) { Some(s) => out(s), None => std::ptr::null_mut() }
}

#[no_mangle]
pub extern "C" fn pfm_last_error() -> *mut c_char { out(crate::last_error()) }

/// Start the GPU renderer: the adapter's name, or "" where there is none (see pfm_last_error).
#[no_mangle]
pub extern "C" fn pfm_gpu_init() -> *mut c_char { out(guard(None, || Ok(crate::gpu().map(|g| g.1))).unwrap_or_default()) }

/// Open a scene, drawn on the GPU when `gpu` and there is one; NULL on failure.
#[no_mangle]
pub extern "C" fn pfm_open(json: *const c_char, base_dir: *const c_char, gpu: bool) -> *mut Handle {
    guard(std::ptr::null_mut(), || {
        let (j, d) = (arg(json)?, arg(base_dir)?);
        Ok(Box::into_raw(Box::new(Handle::open(&j, Some(&d), gpu)?)))
    })
}

#[no_mangle]
pub extern "C" fn pfm_set_scene(h: *mut Handle, json: *const c_char) -> bool {
    guard(false, || { handle(h)?.set_scene(&arg(json)?)?; Ok(true) })
}

#[no_mangle]
pub extern "C" fn pfm_close(h: *mut Handle) {
    if !h.is_null() {
        // SAFETY: made by `pfm_open`, closed once.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(unsafe { Box::from_raw(h) })));
    }
}

/// [loop length (m), loop seconds, fps, canvas width, canvas height] into `info` (5 floats).
#[no_mangle]
pub extern "C" fn pfm_info(h: *mut Handle, info: *mut f32) -> bool {
    guard(false, || {
        if info.is_null() { return Err("no room for the info".into()); }
        let v = handle(h)?.info();
        // SAFETY: the caller passes room for 5 floats.
        unsafe { std::ptr::copy_nonoverlapping(v.as_ptr(), info, 5) };
        Ok(true)
    })
}

/// True when the handle draws on the GPU now.
#[no_mangle]
pub extern "C" fn pfm_on_gpu(h: *mut Handle) -> bool { guard(false, || Ok(handle(h)?.engine() == "gpu")) }

/// Draw into `rgba` (`len` bytes, at least w * h * 4: RGBA rows, premultiplied alpha is not
/// needed as every pixel is opaque).
#[no_mangle]
pub extern "C" fn pfm_render(h: *mut Handle, distance: f32, time: f32, w: u32, hh: u32, rgba: *mut u8, len: usize) -> bool {
    guard(false, || {
        if rgba.is_null() { return Err("no buffer".into()); }
        // SAFETY: the caller passes `len` writable bytes.
        let buf = unsafe { std::slice::from_raw_parts_mut(rgba, len) };
        handle(h)?.render(distance, time, w, hh, buf)?;
        Ok(true)
    })
}

/// The loop as an animated GIF or WebP ("gif" / "webp") in `dir`: its path (free with pfm_free),
/// or NULL (see pfm_last_error).
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "C" fn pfm_export(json: *const c_char, dir: *const c_char, name: *const c_char, format: *const c_char, w: u32, h: u32, fps: u32, quality: f32, gpu: bool) -> *mut c_char {
    match guard(None, || {
        let f = arg(format)?;
        let a = crate::Animation { format: &f, width: w, height: h, fps, quality, gpu };
        crate::export_animation(&arg(json)?, &arg(dir)?, &arg(name)?, &a).map(Some)
    }) { Some(p) => out(p), None => std::ptr::null_mut() }
}

/// Frames done, and of how many, of the export running now.
#[no_mangle]
pub extern "C" fn pfm_export_progress(done: *mut u32, total: *mut u32) {
    let (d, t) = crate::export_progress();
    // SAFETY: the caller passes two writable u32s (or NULL).
    unsafe {
        if let Some(p) = done.as_mut() { *p = d; }
        if let Some(p) = total.as_mut() { *p = t; }
    }
}

#[no_mangle]
pub extern "C" fn pfm_export_cancel() { crate::export_cancel(); }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_c_calls_open_draw_and_close() {
        let names = pfm_preset_names();
        // SAFETY: strings from the library, freed once.
        let list = unsafe { CStr::from_ptr(names) }.to_str().unwrap().to_owned();
        pfm_free(names);
        assert!(list.contains("Sunset Pier"));
        let name = CString::new("Sunset Pier").unwrap();
        let scene = pfm_preset_scene(name.as_ptr());
        assert!(!scene.is_null());
        let h = pfm_open(scene, std::ptr::null(), false);
        pfm_free(scene);
        assert!(!h.is_null());
        let mut info = [0f32; 5];
        assert!(pfm_info(h, info.as_mut_ptr()));
        assert!(info[0] > 0.0 && info[1] > 0.0);
        let mut px = vec![0u8; 30 * 50 * 4];
        assert!(pfm_render(h, 1.0, 0.5, 30, 50, px.as_mut_ptr(), px.len()));
        assert!(!pfm_render(h, 1.0, 0.5, 300, 500, px.as_mut_ptr(), px.len()), "too small a buffer is refused");
        pfm_close(h);
        let bad = CString::new("{ not json").unwrap();
        assert!(pfm_open(bad.as_ptr(), std::ptr::null(), false).is_null());
        let e = pfm_last_error();
        assert!(!unsafe { CStr::from_ptr(e) }.to_bytes().is_empty());
        pfm_free(e);
    }
}
