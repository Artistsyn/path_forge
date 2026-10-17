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

/// The version of this C interface, raised whenever a function is added or changes, so a program
/// that loads the library at run time (an editor's preview) can tell what it offers. Version 2
/// added `pf_runtime_open_place`, `pf_runtime_go_plan` and `pf_journey_outline`; version 3
/// `pf_runtime_swing_to` and `pf_runtime_view`.
pub const PF_ABI_VERSION: u32 = 3;

#[no_mangle]
pub extern "C" fn pf_abi_version() -> u32 { PF_ABI_VERSION }

/// Write `json` into `out` (truncated to fit, NUL-terminated) and return its full length, so a
/// caller whose buffer was too small can call again with a larger one.
unsafe fn write_json(out: *mut c_char, out_len: usize, json: &str) -> i32 {
    write_err(out, out_len, json);
    json.len() as i32
}

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

/// Open a place as a journey stop names one: a scene file (relative to `base_dir`, which may be
/// NULL) or a built-in preset as `preset:Name`. NULL on failure, with the reason in `err`.
#[no_mangle]
pub unsafe extern "C" fn pf_runtime_open_place(name: *const c_char, base_dir: *const c_char, err: *mut c_char, err_len: usize) -> *mut Runtime {
    let r = guard(Err("panic while opening the place".into()), || {
        let n = text(name).ok_or_else(|| "name is null or not UTF-8".to_string())?;
        let dir = text(base_dir).map(PathBuf::from);
        path_forge::journey::load_place(n, dir.as_deref()).map(|(s, d)| Runtime::new(s, d))
    });
    match r {
        Ok(rt) => Box::into_raw(Box::new(rt)),
        Err(e) => { write_err(err, err_len, &e); std::ptr::null_mut() }
    }
}

/// A journey file's outline, without opening it to walk: JSON {"start", "stops": [{"id", "scene",
/// "next"}], "problems": [...]} written into `out`, where "next" is "end", {"go": to} or
/// {"fork": [left, right]}. Returns the JSON's full length, or -2 with the reason in `out`.
#[no_mangle]
pub unsafe extern "C" fn pf_journey_outline(path: *const c_char, out: *mut c_char, out_len: usize) -> i32 {
    let r = guard(Err("panic while reading the journey".to_string()), || {
        use path_forge::journey::{Journey, Next};
        let p = text(path).ok_or_else(|| "path is null or not UTF-8".to_string())?;
        let j = Journey::load(std::path::Path::new(p))?;
        let stops: Vec<serde_json::Value> = j.stops.iter().map(|(id, st)| {
            let next = match &st.next {
                Next::End => serde_json::json!("end"),
                Next::Go { to, .. } => serde_json::json!({ "go": to }),
                Next::Fork { left, right, .. } => serde_json::json!({ "fork": [left, right] }),
            };
            serde_json::json!({ "id": id, "scene": st.scene, "next": next })
        }).collect();
        let problems = j.problems(std::path::Path::new(p).parent());
        Ok(serde_json::json!({ "start": j.start, "stops": stops, "problems": problems }).to_string())
    });
    match r { Ok(j) => write_json(out, out_len, &j), Err(e) => { write_err(out, out_len, &e); -2 } }
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

/// `pf_runtime_go`, and what it planned: JSON {"kind", "notes", "warnings"} (the transition's
/// threshold or "Fork", what was chosen and anything that went against the scenes) written into
/// `out`. Returns its full length, -1 for a null handle, or -2 with the reason in `out`.
#[no_mangle]
pub unsafe extern "C" fn pf_runtime_go_plan(rt: *mut Runtime, out: *mut c_char, out_len: usize) -> i32 {
    let Some(r) = rt.as_mut() else { return -1 };
    match guard(Err("panic".to_string()), || r.go()) {
        Ok(p) => write_json(out, out_len, &serde_json::to_string(&p).unwrap_or_default()),
        Err(e) => { write_err(out, out_len, &e); -2 }
    }
}

/// Where the walk stands, as JSON {scene, distance, time, stop, progress, choose_within, day,
/// intro_left, swing_left} written into `out`. Returns the length written, or -1.
#[no_mangle]
pub unsafe extern "C" fn pf_runtime_state(rt: *const Runtime, out: *mut c_char, out_len: usize) -> i32 {
    let Some(r) = rt.as_ref() else { return -1 };
    let json = serde_json::to_string(&r.state()).unwrap_or_default();
    write_err(out, out_len, &json);
    json.len().min(out_len.saturating_sub(1)) as i32
}

/// Set the hour of the day (0..24) on every scene the runtime draws; a negative hour gives each
/// scene back its own. 0, or -1 for a null handle.
#[no_mangle]
pub unsafe extern "C" fn pf_runtime_set_hour(rt: *mut Runtime, hour: f32) -> i32 {
    let Some(r) = rt.as_mut() else { return -1 };
    r.set_hour((hour >= 0.0).then_some(hour));
    0
}

/// Play the camera's flight in (`camera.moves.fly_in`) again from the top, over the next
/// pf_runtime_step seconds. 0, or -1 for a null handle.
#[no_mangle]
pub unsafe extern "C" fn pf_runtime_restart_intro(rt: *mut Runtime) -> i32 {
    let Some(r) = rt.as_mut() else { return -1 };
    r.restart_intro();
    0
}

/// Swing the camera from where it is now to another view, as JSON (every field optional):
/// {"yaw","side","eye_height","horizon","zoom"} the view (missing ones stay as the camera has
/// them), {"seconds","hold","ease","overshoot","orbit","bars"} how (as camera.moves.swing_in; 0
/// seconds is 2.5). The view stays once it lands. 0 when it has begun, -1 for a null handle, -2
/// with the reason in `err` (bad JSON, or a transition under way).
#[no_mangle]
pub unsafe extern "C" fn pf_runtime_swing_to(rt: *mut Runtime, json: *const c_char, err: *mut c_char, err_len: usize) -> i32 {
    let Some(r) = rt.as_mut() else { return -1 };
    let res = guard(Err("panic".to_string()), || {
        let text = text(json).ok_or("json is null or not UTF-8")?;
        let v: serde_json::Value = serde_json::from_str(text).map_err(|e| format!("swing: {e}"))?;
        let mut style: path_forge::scene::Swing = serde_json::from_value(v.clone()).map_err(|e| format!("swing: {e}"))?;
        // (A game's swing goes when asked unless it says to hold.)
        if v.get("hold").is_none() { style.hold = 0.0; }
        let mut to = path_forge::scene::SwingView::of(&r.scene().camera);
        let f = |k: &str, d: f32| v.get(k).and_then(|x| x.as_f64()).map_or(d, |x| x as f32);
        (to.yaw, to.side, to.eye_height, to.horizon, to.zoom) = (f("yaw", to.yaw), f("side", to.side), f("eye_height", to.eye_height), f("horizon", to.horizon), f("zoom", to.zoom));
        r.swing_to(to, style)
    });
    match res { Ok(()) => 0, Err(e) => { write_err(err, err_len, &e); -2 } }
}

/// The camera's view now, as JSON {"yaw","side","back","eye_height","horizon","zoom"} (where a
/// swing or the intro has it; `back` metres behind the walker). Returns its length, or -1.
#[no_mangle]
pub unsafe extern "C" fn pf_runtime_view(rt: *const Runtime, out: *mut c_char, out_len: usize) -> i32 {
    let Some(r) = rt.as_ref() else { return -1 };
    write_json(out, out_len, &serde_json::to_string(&r.view_now()).unwrap_or_default())
}

/// Let the days pass: one day every `seconds` of pf_runtime_step time (0 holds the hour).
#[no_mangle]
pub unsafe extern "C" fn pf_runtime_set_day_length(rt: *mut Runtime, seconds: f32) -> i32 {
    let Some(r) = rt.as_mut() else { return -1 };
    r.set_day_length(seconds);
    0
}

/// The day now: writes the hour and the daylight (0 night .. 1 day) and returns 1, or 0 when
/// the scene has no day and night (no open sky), -1 for a null handle. Either pointer may be NULL.
#[no_mangle]
pub unsafe extern "C" fn pf_runtime_day(rt: *const Runtime, hour: *mut f32, daylight: *mut f32) -> i32 {
    let Some(r) = rt.as_ref() else { return -1 };
    match r.day() {
        Some(d) => {
            if !hour.is_null() { *hour = d.hour; }
            if !daylight.is_null() { *daylight = d.daylight; }
            1
        }
        None => 0,
    }
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
    fn a_game_swings_the_camera_through_the_c_interface() {
        unsafe {
            let mut err = [0 as c_char; 256];
            let preset = CString::new("preset:Village Side Scroll").unwrap();
            let rt = pf_runtime_open_place(preset.as_ptr(), std::ptr::null(), err.as_mut_ptr(), err.len());
            assert!(!rt.is_null());
            let to = CString::new(r#"{"yaw": 0, "side": 0, "seconds": 1.0, "ease": "Out"}"#).unwrap();
            assert_eq!(pf_runtime_swing_to(rt, to.as_ptr(), err.as_mut_ptr(), err.len()), 0, "{:?}", CStr::from_ptr(err.as_ptr()));
            let mut out = [0 as c_char; 512];
            pf_runtime_state(rt, out.as_mut_ptr(), out.len());
            assert!(CStr::from_ptr(out.as_ptr()).to_str().unwrap().contains("\"swing_left\":1"), "{:?}", CStr::from_ptr(out.as_ptr()));
            pf_runtime_step(rt, 0.5, 0.0);
            assert!(pf_runtime_view(rt, out.as_mut_ptr(), out.len()) > 0);
            let v: serde_json::Value = serde_json::from_str(CStr::from_ptr(out.as_ptr()).to_str().unwrap()).unwrap();
            let yaw = v["yaw"].as_f64().unwrap();
            assert!(yaw > -90.0 && yaw < 0.0, "partway round: {v}");
            pf_runtime_step(rt, 0.6, 0.0);
            pf_runtime_view(rt, out.as_mut_ptr(), out.len());
            let v: serde_json::Value = serde_json::from_str(CStr::from_ptr(out.as_ptr()).to_str().unwrap()).unwrap();
            assert_eq!((v["yaw"].as_f64(), v["side"].as_f64()), (Some(0.0), Some(0.0)), "landed and stays");
            let bad = CString::new("{nope").unwrap();
            assert_eq!(pf_runtime_swing_to(rt, bad.as_ptr(), err.as_mut_ptr(), err.len()), -2);
            pf_runtime_free(rt);
        }
    }

    #[test]
    fn an_editor_opens_presets_reads_journeys_and_hears_the_plan() {
        unsafe {
            assert_eq!(pf_abi_version(), PF_ABI_VERSION);
            let mut err = [0 as c_char; 256];
            let preset = CString::new("preset:Forest Path").unwrap();
            let rt = pf_runtime_open_place(preset.as_ptr(), std::ptr::null(), err.as_mut_ptr(), err.len());
            assert!(!rt.is_null(), "{:?}", CStr::from_ptr(err.as_ptr()));
            pf_runtime_free(rt);
            let nope = CString::new("preset:Nowhere").unwrap();
            assert!(pf_runtime_open_place(nope.as_ptr(), std::ptr::null(), err.as_mut_ptr(), err.len()).is_null());
            assert!(CStr::from_ptr(err.as_ptr()).to_str().unwrap().contains("Nowhere"));

            let dir = std::env::temp_dir().join(format!("pf_c_outline_{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let jp = dir.join("g.journey.json");
            std::fs::write(&jp, r#"{"version":1,"name":"g","start":"a","stops":{
                "a":{"scene":"preset:Stone Dungeon","next":{"Go":{"to":"b","transition":{}}}},
                "b":{"scene":"preset:Forest Path","next":"End"}}}"#).unwrap();
            let pj = CString::new(jp.to_str().unwrap()).unwrap();
            let mut out = vec![0 as c_char; 4096];
            let n = pf_journey_outline(pj.as_ptr(), out.as_mut_ptr(), out.len());
            let v: serde_json::Value = serde_json::from_str(CStr::from_ptr(out.as_ptr()).to_str().unwrap()).unwrap();
            assert!(n > 0 && v["start"] == "a" && v["stops"].as_array().unwrap().len() == 2, "{v}");
            assert!(v["problems"].as_array().unwrap().is_empty(), "{v}");
            // Too small a buffer: the full length comes back, to call again.
            let mut tiny = [0 as c_char; 8];
            assert_eq!(pf_journey_outline(pj.as_ptr(), tiny.as_mut_ptr(), tiny.len()), n);

            let rt = pf_runtime_open_journey(pj.as_ptr(), err.as_mut_ptr(), err.len());
            let n = pf_runtime_go_plan(rt, out.as_mut_ptr(), out.len());
            let plan: serde_json::Value = serde_json::from_str(CStr::from_ptr(out.as_ptr()).to_str().unwrap()).unwrap();
            assert!(n > 0 && plan["kind"].as_str().is_some_and(|k| !k.is_empty()), "{plan}");
            for _ in 0..200 { pf_runtime_step(rt, 0.25, 20.0); }
            assert_eq!(pf_runtime_go_plan(rt, out.as_mut_ptr(), out.len()), -2, "the end of the journey");
            pf_runtime_free(rt);
            std::fs::remove_dir_all(&dir).ok();
        }
    }

    #[test]
    fn a_game_sets_the_hour_and_lets_the_days_pass() {
        unsafe {
            let dir = std::env::temp_dir().join(format!("pf_c_day_{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let fp = path_forge::scene::presets::ALL.iter().find(|p| p.0 == "Forest Path").map(|p| (p.1)()).unwrap();
            let a = dir.join("a.json");
            std::fs::write(&a, serde_json::to_string(&fp).unwrap()).unwrap();
            let mut err = [0 as c_char; 256];
            let pa = CString::new(a.to_str().unwrap()).unwrap();
            let rt = pf_runtime_open(pa.as_ptr(), err.as_mut_ptr(), err.len());
            let (mut h, mut dl) = (0.0f32, 0.0f32);
            assert_eq!(pf_runtime_day(rt, &mut h, &mut dl), 0, "no day and night until asked");
            let mut noon = vec![0u8; 36 * 64 * 4];
            let mut night = noon.clone();
            assert_eq!(pf_runtime_set_hour(rt, 12.0), 0);
            assert_eq!(pf_runtime_day(rt, &mut h, &mut dl), 1);
            assert!((h - 12.0).abs() < 1e-4 && dl > 0.99, "noon: {h} {dl}");
            pf_runtime_frame(rt, 36, 64, noon.as_mut_ptr(), noon.len());
            // A day every 24 seconds: 12 seconds on it is midnight.
            pf_runtime_set_day_length(rt, 24.0);
            for _ in 0..120 { pf_runtime_step(rt, 0.1, 0.0); }
            pf_runtime_day(rt, &mut h, &mut dl);
            assert!((h.rem_euclid(24.0) - 0.0).abs() < 0.05 || (h - 24.0).abs() < 0.05, "midnight: {h}");
            assert!(dl < 0.01, "no daylight at midnight: {dl}");
            pf_runtime_frame(rt, 36, 64, night.as_mut_ptr(), night.len());
            let sum = |b: &[u8]| b.iter().map(|v| *v as u64).sum::<u64>();
            assert!(sum(&night) * 2 < sum(&noon), "midnight should be far darker than noon ({} vs {})", sum(&night), sum(&noon));
            let mut out = [0 as c_char; 1024];
            pf_runtime_state(rt, out.as_mut_ptr(), out.len());
            let state = CStr::from_ptr(out.as_ptr()).to_str().unwrap();
            assert!(state.contains("\"phase\":\"night\""), "{state}");
            pf_runtime_set_hour(rt, -1.0);
            pf_runtime_set_day_length(rt, 0.0);
            assert_eq!(pf_runtime_day(rt, &mut h, &mut dl), 0, "the scene's own sky again");
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
