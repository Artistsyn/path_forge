//! PathForge for games. Rust games use `path_forge_runtime::Runtime` (the same as
//! `path_forge::runtime`); any other engine links this library and calls the C functions below,
//! declared in `include/path_forge.h`.
//!
//! Every function takes the handle from `pf_runtime_open` / `pf_runtime_from_json`, is safe to
//! call with a null handle (it fails), and never unwinds across the boundary: a panic inside
//! becomes an error return.

use std::ffi::{c_char, CStr};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;

pub use path_forge::runtime::{Runtime, Walk};

/// Copy `msg` into a caller's buffer as a NUL-terminated string (truncated to fit).
unsafe fn write_err(err: *mut c_char, err_len: usize, msg: &str) {
    if err.is_null() || err_len == 0 { return; }
    let n = msg.len().min(err_len - 1);
    std::ptr::copy_nonoverlapping(msg.as_ptr() as *const c_char, err, n);
    *err.add(n) = 0;
}

unsafe fn text<'a>(p: *const c_char) -> Option<&'a str> {
    if p.is_null() { None } else { CStr::from_ptr(p).to_str().ok() }
}

fn guard<T>(fallback: T, f: impl FnOnce() -> T) -> T { catch_unwind(AssertUnwindSafe(f)).unwrap_or(fallback) }

/// Open a scene file. Returns null on failure, with the reason in `err` (may be null).
#[no_mangle]
pub unsafe extern "C" fn pf_runtime_open(path: *const c_char, err: *mut c_char, err_len: usize) -> *mut Runtime {
    let r = guard(Err("panic while opening the scene".into()), || {
        let p = text(path).ok_or_else(|| "path is null or not UTF-8".to_string())?;
        Runtime::from_file(p)
    });
    match r {
        Ok(rt) => Box::into_raw(Box::new(rt)),
        Err(e) => { write_err(err, err_len, &e); std::ptr::null_mut() }
    }
}

/// A scene from JSON text; files it names are relative to `base_dir` (may be null).
#[no_mangle]
pub unsafe extern "C" fn pf_runtime_from_json(json: *const c_char, base_dir: *const c_char, err: *mut c_char, err_len: usize) -> *mut Runtime {
    let r = guard(Err("panic while reading the scene".into()), || {
        let j = text(json).ok_or_else(|| "json is null or not UTF-8".to_string())?;
        Runtime::from_json(j, text(base_dir).map(PathBuf::from))
    });
    match r {
        Ok(rt) => Box::into_raw(Box::new(rt)),
        Err(e) => { write_err(err, err_len, &e); std::ptr::null_mut() }
    }
}

#[no_mangle]
pub unsafe extern "C" fn pf_runtime_free(rt: *mut Runtime) {
    if !rt.is_null() { drop(Box::from_raw(rt)); }
}

/// Metres before the view repeats (0 for a null handle).
#[no_mangle]
pub unsafe extern "C" fn pf_runtime_loop_length(rt: *const Runtime) -> f32 { rt.as_ref().map_or(0.0, |r| r.loop_length()) }

/// Seconds after which every timed effect repeats (0 for a null handle).
#[no_mangle]
pub unsafe extern "C" fn pf_runtime_loop_seconds(rt: *const Runtime) -> f32 { rt.as_ref().map_or(0.0, |r| r.loop_seconds()) }

/// The scene's canvas size; returns 0 on success.
#[no_mangle]
pub unsafe extern "C" fn pf_runtime_canvas(rt: *const Runtime, width: *mut u32, height: *mut u32) -> i32 {
    let Some(r) = rt.as_ref() else { return -1 };
    let (w, h) = r.canvas();
    if !width.is_null() { *width = w; }
    if !height.is_null() { *height = h; }
    0
}

/// Render the view `distance` metres along the path at `time` seconds into `out` (RGBA8, rows of
/// `width` pixels, `width * height * 4` bytes). Returns 0 on success, -1 for a null handle or
/// buffer, -2 if the buffer is too small, -3 if rendering failed.
#[no_mangle]
pub unsafe extern "C" fn pf_runtime_render(rt: *mut Runtime, distance: f32, time: f32, width: u32, height: u32, out: *mut u8, out_len: usize) -> i32 {
    let Some(r) = rt.as_mut() else { return -1 };
    if out.is_null() { return -1; }
    if out_len < width as usize * height as usize * 4 { return -2; }
    let buf = std::slice::from_raw_parts_mut(out, out_len);
    guard(-3, || if r.render_into(distance, time, width, height, buf).is_ok() { 0 } else { -2 })
}

/// Where a point appears on a `width` x `height` screen: `x` metres right of the path centre, `y`
/// up, `d` ahead. Writes [x, y] pixels to `out_xy`; returns 1 if it is in front of the camera,
/// 0 if behind, -1 for a null handle.
#[no_mangle]
pub unsafe extern "C" fn pf_runtime_project(rt: *const Runtime, distance: f32, x: f32, y: f32, d: f32, width: u32, height: u32, out_xy: *mut f32) -> i32 {
    let Some(r) = rt.as_ref() else { return -1 };
    match guard(None, || r.project(distance, x, y, d, width, height)) {
        Some(p) => { if !out_xy.is_null() { *out_xy = p[0]; *out_xy.add(1) = p[1]; } 1 }
        None => 0,
    }
}

/// How far ahead the ground on screen row `row` is, in metres; negative above the horizon or for
/// a null handle.
#[no_mangle]
pub unsafe extern "C" fn pf_runtime_ground_distance(rt: *const Runtime, distance: f32, row: f32, width: u32, height: u32) -> f32 {
    let Some(r) = rt.as_ref() else { return -1.0 };
    guard(None, || r.ground_distance(distance, row, width, height)).unwrap_or(-1.0)
}

// ── The runtime's own walk: transitions, forks, journeys ─────────────────

/// Open a journey file (scenes and how each leads to the next). NULL on failure.
#[no_mangle]
pub unsafe extern "C" fn pf_runtime_open_journey(path: *const c_char, err: *mut c_char, err_len: usize) -> *mut Runtime {
    let r = guard(Err("panic while opening the journey".into()), || {
        let p = text(path).ok_or_else(|| "path is null or not UTF-8".to_string())?;
        Runtime::from_journey(p)
    });
    match r {
        Ok(rt) => Box::into_raw(Box::new(rt)),
        Err(e) => { write_err(err, err_len, &e); std::ptr::null_mut() }
    }
}

/// Walk on by `dt` seconds at `speed` metres per second, through any transition under way.
#[no_mangle]
pub unsafe extern "C" fn pf_runtime_step(rt: *mut Runtime, dt: f32, speed: f32) -> i32 {
    let Some(r) = rt.as_mut() else { return -1 };
    guard(-3, || { r.step(dt, speed); 0 })
}

/// Render the runtime's own walk into `out` (as `pf_runtime_render`).
#[no_mangle]
pub unsafe extern "C" fn pf_runtime_frame(rt: *mut Runtime, width: u32, height: u32, out: *mut u8, out_len: usize) -> i32 {
    let Some(r) = rt.as_mut() else { return -1 };
    if out.is_null() { return -1; }
    let need = width as usize * height as usize * 4;
    if out_len < need { return -2; }
    let buf = std::slice::from_raw_parts_mut(out, out_len);
    guard(-3, || { let img = r.frame(width, height); buf[..need].copy_from_slice(&img[..need]); 0 })
}

fn transition_from(json: Option<&str>) -> Result<path_forge::scene::transition::Transition, String> {
    match json { Some(j) if !j.trim().is_empty() => serde_json::from_str(j).map_err(|e| format!("transition: {e}")), _ => Ok(Default::default()) }
}

/// Walk into the scene file `path` from here. `transition_json` (may be NULL) overrides the
/// choices (threshold, approach_m, ...). 0 on success, -1 null handle, -2 with the reason in err.
#[no_mangle]
pub unsafe extern "C" fn pf_runtime_transition_to(rt: *mut Runtime, path: *const c_char, transition_json: *const c_char, err: *mut c_char, err_len: usize) -> i32 {
    let Some(r) = rt.as_mut() else { return -1 };
    let res = guard(Err("panic while planning the transition".to_string()), || {
        let p = text(path).ok_or_else(|| "path is null or not UTF-8".to_string())?;
        let (scene, _) = path_forge::scene::load_file(std::path::Path::new(p))?;
        let t = transition_from(text(transition_json))?;
        r.transition_to(scene, std::path::Path::new(p).parent().map(PathBuf::from), &t);
        Ok(())
    });
    match res { Ok(()) => 0, Err(e) => { write_err(err, err_len, &e); -2 } }
}

/// A fork ahead into the scene files `left` and `right`; `fork_json` (may be NULL) overrides
/// the choices. Then call `pf_runtime_choose`. 0 on success, -2 with the reason in err.
#[no_mangle]
pub unsafe extern "C" fn pf_runtime_fork(rt: *mut Runtime, left: *const c_char, right: *const c_char, fork_json: *const c_char, err: *mut c_char, err_len: usize) -> i32 {
    let Some(r) = rt.as_mut() else { return -1 };
    let res = guard(Err("panic while planning the fork".to_string()), || {
        let load = |p: *const c_char| -> Result<(path_forge::scene::Scene, Option<PathBuf>), String> {
            let p = text(p).ok_or_else(|| "path is null or not UTF-8".to_string())?;
            Ok((path_forge::scene::load_file(std::path::Path::new(p))?.0, std::path::Path::new(p).parent().map(PathBuf::from)))
        };
        let f = match text(fork_json) { Some(j) if !j.trim().is_empty() => serde_json::from_str(j).map_err(|e| format!("fork: {e}"))?, _ => Default::default() };
        r.fork(load(left)?, load(right)?, &f);
        Ok(())
    });
    match res { Ok(()) => 0, Err(e) => { write_err(err, err_len, &e); -2 } }
}

/// Take a branch of the fork ahead: 0 left, 1 right. 1 if taken, 0 if there is no fork or it is too late.
#[no_mangle]
pub unsafe extern "C" fn pf_runtime_choose(rt: *mut Runtime, side: i32) -> i32 {
    let Some(r) = rt.as_mut() else { return -1 };
    let b = if side == 1 { path_forge::scene::transition::Branch::Right } else { path_forge::scene::transition::Branch::Left };
    guard(0, || r.choose(b) as i32)
}

/// Walk to where the journey leads from the current stop. 0 on success, -2 with the reason in err.
#[no_mangle]
pub unsafe extern "C" fn pf_runtime_go(rt: *mut Runtime, err: *mut c_char, err_len: usize) -> i32 {
    let Some(r) = rt.as_mut() else { return -1 };
    match guard(Err("panic".to_string()), || r.go()) { Ok(_) => 0, Err(e) => { write_err(err, err_len, &e); -2 } }
}

/// Where the walk stands, as JSON {scene, distance, time, stop, progress, choose_within} written
/// into `out`. Returns the length written, or -1.
#[no_mangle]
pub unsafe extern "C" fn pf_runtime_state(rt: *const Runtime, out: *mut c_char, out_len: usize) -> i32 {
    let Some(r) = rt.as_ref() else { return -1 };
    let json = serde_json::to_string(&r.state()).unwrap_or_default();
    write_err(out, out_len, &json);
    json.len().min(out_len.saturating_sub(1)) as i32
}

/// `pf_runtime_project` for the runtime's own walk.
#[no_mangle]
pub unsafe extern "C" fn pf_runtime_project_now(rt: *const Runtime, x: f32, y: f32, d: f32, width: u32, height: u32, out_xy: *mut f32) -> i32 {
    let Some(r) = rt.as_ref() else { return -1 };
    match guard(None, || r.project_now(x, y, d, width, height)) {
        Some(p) => { if !out_xy.is_null() { *out_xy = p[0]; *out_xy.add(1) = p[1]; } 1 }
        None => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[test]
    fn the_walk_crosses_into_another_scene_through_the_c_functions() {
        unsafe {
            let dir = std::env::temp_dir().join(format!("pf_c_walk_{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let get = |n: &str| path_forge::scene::presets::ALL.iter().find(|p| p.0 == n).map(|p| (p.1)()).unwrap();
            let (a, b) = (dir.join("a.json"), dir.join("b.json"));
            std::fs::write(&a, serde_json::to_string(&get("Stone Dungeon")).unwrap()).unwrap();
            std::fs::write(&b, serde_json::to_string(&get("Forest Path")).unwrap()).unwrap();
            let mut err = [0 as c_char; 256];
            let pa = CString::new(a.to_str().unwrap()).unwrap();
            let pb = CString::new(b.to_str().unwrap()).unwrap();
            let rt = pf_runtime_open(pa.as_ptr(), err.as_mut_ptr(), err.len());
            assert!(!rt.is_null());
            let tj = CString::new(r#"{"threshold":"CaveMouth"}"#).unwrap();
            assert_eq!(pf_runtime_transition_to(rt, pb.as_ptr(), tj.as_ptr(), err.as_mut_ptr(), err.len()), 0);
            let mut buf = vec![0u8; 36 * 64 * 4];
            for _ in 0..200 { pf_runtime_step(rt, 0.1, 4.0); }
            assert_eq!(pf_runtime_frame(rt, 36, 64, buf.as_mut_ptr(), buf.len()), 0);
            let mut out = [0 as c_char; 512];
            let n = pf_runtime_state(rt, out.as_mut_ptr(), out.len());
            let state = CStr::from_ptr(out.as_ptr()).to_str().unwrap();
            assert!(n > 0 && state.contains("Forest Path"), "{state}");
            assert_eq!(pf_runtime_choose(rt, 1), 0, "no fork ahead");
            pf_runtime_free(rt);
            std::fs::remove_dir_all(&dir).ok();
        }
    }

    #[test]
    fn the_c_functions_render_and_fail_cleanly() {
        unsafe {
            let mut err = [0 as c_char; 256];
            let bad = CString::new("/no/such/scene.json").unwrap();
            assert!(pf_runtime_open(bad.as_ptr(), err.as_mut_ptr(), err.len()).is_null());
            assert!(CStr::from_ptr(err.as_ptr()).to_str().unwrap().contains("scene.json"));
            let s = path_forge::scene::presets::ALL[0].1();
            let json = CString::new(serde_json::to_string(&s).unwrap()).unwrap();
            let rt = pf_runtime_from_json(json.as_ptr(), std::ptr::null(), err.as_mut_ptr(), err.len());
            assert!(!rt.is_null());
            assert!(pf_runtime_loop_length(rt) > 0.0);
            let mut buf = vec![0u8; 64 * 96 * 4];
            assert_eq!(pf_runtime_render(rt, 2.0, 0.5, 64, 96, buf.as_mut_ptr(), buf.len()), 0);
            assert!(buf.chunks(4).any(|p| p[0] > 0), "something was drawn");
            assert_eq!(pf_runtime_render(rt, 2.0, 0.5, 64, 96, buf.as_mut_ptr(), 10), -2);
            let mut xy = [0f32; 2];
            assert_eq!(pf_runtime_project(rt, 0.0, 0.0, 0.0, 6.0, 64, 96, xy.as_mut_ptr()), 1);
            assert!((pf_runtime_ground_distance(rt, 0.0, xy[1], 64, 96) - 6.0).abs() < 0.1);
            assert_eq!(pf_runtime_render(std::ptr::null_mut(), 0.0, 0.0, 64, 96, buf.as_mut_ptr(), buf.len()), -1);
            pf_runtime_free(rt);
            pf_runtime_free(std::ptr::null_mut());
        }
    }
}
