//! The live runtime: a game embeds PathForge and renders the path itself, every frame, at any
//! speed and resolution, instead of playing exported frames. Distance walked and time are separate
//! inputs, so the hero can speed up, stop for a fight (flames and weather keep moving) and walk on,
//! and the loop never shows a seam.
//!
//! ```no_run
//! use path_forge::runtime::{Runtime, Walk};
//! let mut rt = Runtime::from_file("backgrounds/scenes/crypt.json").unwrap();
//! let mut walk = Walk::default();
//! // each frame:
//! walk.advance(1.0 / 60.0, 3.5, &rt);
//! // where to draw an enemy 8 m ahead on the path:
//! let at = rt.project(walk.distance, 0.0, 0.0, 8.0, 270, 480);
//! // the background (borrowed until the next render; copy it to keep it):
//! let rgba = rt.render(walk.distance, walk.time, 270, 480);
//! # let _ = (rgba, at);
//! ```
//!
//! Without the studio feature (`default-features = false`) the library has no GUI or GPU
//! dependencies. Rendering is on the CPU across threads; for a phone, render small (the scene's
//! pixel size and a quarter-size canvas both cut the cost) and scale up.

//! The runtime can also walk by itself and carry the game from one world into the next:
//! `step` moves on, `frame` draws, `transition_to` walks into another scene through a threshold
//! that `scene::transition::plan` chooses, `fork` offers two branches and `choose` takes one, and a
//! `Journey` file maps a whole game's worth of places so `go` knows where each one leads.

use crate::journey::{Camera, CrossWalk, ForkWalk, Journey, Next, Place, Stop};
use crate::scene::transition::{self as tr, Branch, Crossing, ForkChoice, ForkPlan, Transition};
use crate::scene::{self, Scene};
use crate::world::{RenderOptions, View, WorldRenderer};
use std::path::{Path, PathBuf};

pub struct Runtime {
    scene: Scene,
    base_dir: Option<PathBuf>,
    renderer: WorldRenderer,
    rgba: Vec<u8>,
    /// Frames drawn pipelined (`set_pipelined`).
    pipelined: bool,
    /// The runtime's own walk (see `step`): metres along the current scene and its clock.
    distance: f32,
    time: f32,
    /// Metres walked since the walk began (`WalkState::walked`).
    walked: f64,
    active: Option<Active>,
    /// After `turn_into` left the walk on unstarted: metres from its start to where it hands over.
    onward: Option<f32>,
    journey: Option<(Journey, Option<PathBuf>, String)>,
    /// Real seconds one day lasts when the game lets the days pass (`set_day_length`); 0 holds the hour.
    day_seconds: f32,
    /// Seconds since play began (not wrapped by the loop), for the camera's flight in.
    played: f32,
    /// A swing the game started (`swing_to`) under way.
    swinging: Option<Swinging>,
    /// The camera the current scene came with, for swinging back to (`swing("home")`).
    home: scene::Camera,
    /// Journey stops `warm_next` has still to get ready.
    cold: Vec<String>,
    /// Stops the game has asked for ahead of the rest (`warm`): a cut it is about to make.
    wanted: Vec<String>,
    /// Points `frame` measures the light at (`set_probes`, `lighting`).
    probes: Vec<[f32; 3]>,
    /// The least brightness a frame is let fall to (`set_visibility_floor`), and the exposure
    /// lift now holding it there.
    floor: Option<f32>,
    lift: f32,
    /// Painting ahead of the walk (`set_paint_ahead`).
    ahead: Option<Ahead>,
    /// `warm_order` from the stop it was worked out at (`near_stops`): every frame asks, and it
    /// changes only as the walk reaches a stop.
    near: Option<(String, std::sync::Arc<std::collections::HashMap<String, u32>>)>,
    /// Where the road ends (`set_end`): the walk's metres (`walked`) at the wall, and the wall.
    end: Option<(f64, tr::Barrier)>,
}

/// A thread of its own painting the props of the stops near the walk (`warm_next`'s), so the stop's
/// first draw on the road's thread finds them painted: a cold stop's procedural props took up to
/// 0.9 s to paint on a phone, in the middle of the walk. What it paints goes into the pictures
/// every renderer shares (`sprites::paint_prop_ahead`).
struct Ahead {
    tx: std::sync::mpsc::Sender<(String, Scene, Option<PathBuf>)>,
    done: std::sync::Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
    sent: std::collections::HashSet<String>,
}

/// Run at the start of the painting-ahead thread and its helpers (`set_background_start`).
static BACKGROUND_START: std::sync::OnceLock<fn()> = std::sync::OnceLock::new();

/// Something for the threads that work ahead of the walk to run first: on a phone, a game keeps
/// them on the slower cores the road's threads leave idle. Only the first call counts.
pub fn set_background_start(f: fn()) { let _ = BACKGROUND_START.set(f); }

impl Ahead {
    fn spawn() -> Ahead {
        let (tx, rx) = std::sync::mpsc::channel::<(String, Scene, Option<PathBuf>)>();
        let done = std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashSet::new()));
        let finished = done.clone();
        let start = || if let Some(f) = BACKGROUND_START.get() { f() };
        std::thread::Builder::new().name("pf-ahead".into()).spawn(move || {
            start();
            let pool = rayon::ThreadPoolBuilder::new().num_threads(2).thread_name(|i| format!("pf-ahead-{i}")).start_handler(move |_| start()).build().ok();
            let mut defs = crate::world::propdefs::PropDefs::default();
            let mut sprites = crate::world::sprites::SpriteCache::default();
            // On the CPU: its average colour, and the textures and pictures its frames draw.
            let mut renderer = WorldRenderer::default();
            for (stop, scene, dir) in rx {
                let mut paint = || {
                    paint_props_of(&scene, dir.as_deref(), &mut defs, &mut sprites);
                    renderer.scene_mean(&scene, dir.clone());
                };
                match &pool { Some(p) => p.install(paint), None => paint() }
                finished.lock().unwrap().insert(stop);
            }
        }).expect("the painting-ahead thread");
        Ahead { tx, done, sent: std::collections::HashSet::new() }
    }

    /// Whether `stop` is painted, asking for it (once) when it is not.
    fn ready(&mut self, stop: &str, j: &Journey, dir: Option<&Path>) -> bool {
        if self.done.lock().unwrap().contains(stop) { return true; }
        if self.sent.insert(stop.to_owned()) {
            match j.scene_of(stop, dir) {
                Ok((scene, sdir)) => { let _ = self.tx.send((stop.to_owned(), scene, sdir)); }
                Err(_) => return true,
            }
        }
        false
    }
}

/// Every procedural picture a scene's prop layers draw, every variant, painted (side by side, on
/// the current rayon pool) into the shared pictures; image files are read on the way.
fn paint_props_of(scene: &Scene, dir: Option<&Path>, defs: &mut crate::world::propdefs::PropDefs, sprites: &mut crate::world::sprites::SpriteCache) {
    use crate::scene::PropKind;
    use crate::world::sprites::{paint_asteroid_ahead, paint_prop_ahead, VARIANTS};
    use rayon::prelude::*;
    let mut jobs: Vec<(PropKind, scene::Rgb, bool)> = Vec::new();
    for p in scene.props.iter().filter(|p| p.enabled) {
        let Some(r) = defs.resolve(scene, dir, p, sprites).0 else { continue };
        let floating = p.float > 0.0 || p.orbit.enabled;
        for look in if r.alts.is_empty() { std::slice::from_ref(&r) } else { &r.alts[..] } {
            let rock = floating && matches!(look.kind, PropKind::Rock | PropKind::Boulder);
            if look.sprites.is_empty() && !jobs.contains(&(look.kind, look.tint, rock)) { jobs.push((look.kind, look.tint, rock)); }
        }
    }
    let all: Vec<(PropKind, scene::Rgb, bool, u32)> = jobs.into_iter().flat_map(|(k, t, r)| (0..VARIANTS).map(move |v| (k, t, r, v))).collect();
    // Once there is no room left to keep them, the rest are not painted (the road's thread paints
    // what it then lacks, when it draws it).
    let full = std::sync::atomic::AtomicBool::new(false);
    all.par_iter().for_each(|&(k, t, rock, v)| {
        if full.load(std::sync::atomic::Ordering::Relaxed) { return; }
        let kept = if rock { paint_asteroid_ahead(t, v) } else { paint_prop_ahead(k, t, v) };
        if !kept { full.store(true, std::sync::atomic::Ordering::Relaxed); }
    });
}

/// The camera swinging from one view to another at the game's asking: how, from where, to what
/// camera, and seconds since it began.
struct Swinging { style: scene::Swing, from: scene::SwingView, to: scene::Camera, t: f32 }

impl Swinging {
    fn pose(&self) -> Option<scene::SwingPose> {
        let u = (self.t - self.style.hold.max(0.0)) / self.style.seconds.max(1e-3);
        (self.t < self.style.length()).then(|| self.style.between(&self.from, &scene::SwingView::of(&self.to), &self.to, u))
    }
}

/// A scene the walk is heading into.
struct Bound { scene: Scene, dir: Option<PathBuf>, stop: Option<String> }

/// A transition or fork under way.
enum Active {
    Cross { next: Box<Bound>, walk: CrossWalk, travel: f32, clock: f32 },
    Fork { left: Box<Bound>, right: Box<Bound>, walk: ForkWalk, travel: f32, clock: f32 },
}

/// What PathForge planned for the walk `go` started: the threshold (or "Fork"), why, and what
/// will not look right.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Planned { pub kind: String, pub notes: Vec<String>, pub warnings: Vec<String> }

/// Where the walk stands, for the game.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
pub struct WalkState {
    /// The scene the camera is in (the one it left, until a transition ends).
    pub scene: String,
    /// Metres along the current scene's loop (it comes round: 0 again a loop on).
    pub distance: f32,
    pub time: f32,
    /// Metres walked since the walk began, back steps taken off; never comes round, across
    /// loops and scenes alike. A game keeps things at a place on the road by it: something
    /// at `s` stands `s - walked` metres ahead.
    pub walked: f64,
    /// The journey stop the walk is at, if a journey is loaded.
    pub stop: Option<String>,
    /// 0..1 through a transition or fork under way.
    pub progress: Option<f32>,
    /// A fork is ahead and no branch has been chosen: metres left to choose before the default is taken.
    pub choose_within: Option<f32>,
    /// Metres walked into the transition or fork under way.
    pub travel: Option<f32>,
    /// Metres from the start of the transition or fork under way to where it hands over to the
    /// next place: the crossing's boundary, the fork's junction. After `turn_into` with the walk on
    /// not yet started, that walk's.
    pub handover: Option<f32>,
    /// The walking speed of the scene the camera is in, m/s.
    pub speed: f32,
    /// At a journey stop that leads nowhere, with nothing under way.
    pub end: bool,
    /// The day and night, when the scene has one or the game sets the hour (`set_hour`).
    pub day: Option<scene::DayState>,
    /// Seconds left of the intro (the camera's flight in, its swing in) while it plays: a game
    /// holds the player's controls until it ends.
    pub intro_left: Option<f32>,
    /// Seconds left of a swing the game started (`swing_to`), while it plays.
    pub swing_left: Option<f32>,
    /// Side paths (`path.fork`) opening off the path near the walker, outside a transition:
    /// (metres ahead, -1 left or 1 right), from 6 m behind to 40 m ahead.
    pub side_paths: Vec<(f32, f32)>,
}

/// How many stops out from the walk `warm_next` gets ready: the next ones each way. Each warm
/// holds up a frame by a stop's first draw (a tenth of a second to half a second on a large
/// journey), so three out (twenty stops past a fork) kept a cut to a new place stuttering for
/// seconds; one out is ready before the walk can reach it.
const WARM_REACH: u32 = 1;

/// Journey stops within `WARM_REACH` of `at`, with how far: along the roads either way, and a
/// stop showing the same scene file as one reached costs nothing more (walking a place back).
fn warm_order(j: &Journey, at: &str) -> std::collections::HashMap<String, u32> {
    let mut far: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    let mut todo = std::collections::VecDeque::from([(at.to_owned(), 0u32)]);
    let ahead = |s: &Stop| -> Vec<String> {
        match &s.next {
            Next::End => Vec::new(),
            Next::Go { to, .. } => vec![to.clone()],
            Next::Fork { left, right, .. } => vec![left.clone(), right.clone()],
        }
    };
    while let Some((s, d)) = todo.pop_front() {
        if far.get(&s).is_some_and(|&e| e <= d) || d > WARM_REACH { continue; }
        far.insert(s.clone(), d);
        let Some(st) = j.stops.get(&s) else { continue };
        for n in ahead(st) { todo.push_back((n, d + 1)); }
        for (k, other) in &j.stops {
            if ahead(other).contains(&s) { todo.push_back((k.clone(), d + 1)); }
            if other.scene == st.scene && *k != s { todo.push_back((k.clone(), d)); }
        }
    }
    far
}

impl Runtime {
    /// A scene whose files (sprites, kits, grades) are named relative to `base_dir`.
    pub fn new(scene: Scene, base_dir: Option<PathBuf>) -> Runtime {
        let home = scene.camera.clone();
        Runtime { scene, base_dir, renderer: WorldRenderer::default(), rgba: Vec::new(), pipelined: false, distance: 0.0, time: 0.0, walked: 0.0, active: None, onward: None, journey: None, day_seconds: 0.0, played: 0.0, swinging: None, home, cold: Vec::new(), wanted: Vec::new(), floor: None, lift: 1.0, probes: Vec::new(), ahead: None, near: None, end: None }
    }

    /// Draw frames on the GPU, on a device the game hands over (with wgpu 25, the same one it
    /// draws with), or on the process's own when `None`. Frames match the CPU's within the
    /// renderer's parity gate; leave this off for frames that must match an export bit for bit.
    #[cfg(feature = "gpu")]
    pub fn use_gpu(&mut self, device: Option<std::sync::Arc<crate::world::gpu::GpuContext>>) -> Result<(), String> {
        let ctx = device.or_else(crate::world::gpu::shared).ok_or("no GPU adapter")?;
        self.renderer.set_gpu(Some(std::sync::Arc::new(crate::world::gpu::Gpu::new(ctx)?)));
        Ok(())
    }

    /// Draw on a GPU renderer shared with other runtimes (its pipelines are made once), or on the
    /// CPU with `None`.
    #[cfg(feature = "gpu")]
    pub fn set_gpu(&mut self, gpu: Option<std::sync::Arc<crate::world::gpu::Gpu>>) { self.renderer.set_gpu(gpu); }

    /// Which engine draws this runtime's frames: "gpu" or "cpu".
    pub fn engine(&self) -> &'static str { self.renderer.engine() }

    /// Draw frames pipelined (`RenderOptions::pipelined`): `frame` may return the frame drawn
    /// before (`frame_late`), so the GPU's work overlaps the CPU's. For a live road.
    pub fn set_pipelined(&mut self, on: bool) { self.pipelined = on; }
    /// Whether the last `frame` is the one drawn before it: its state and camera were those
    /// asked for then.
    pub fn frame_late(&self) -> bool { self.renderer.frame_late() }

    /// Time every stage of each frame drawn (`WorldRenderer::profile`), or stop.
    pub fn set_profile(&mut self, on: bool) { self.renderer.profile = on; self.renderer.stages.clear(); }
    /// With the profile on, time each GPU pass too (it stops frames overlapping the GPU's work).
    pub fn set_profile_gpu(&mut self, on: bool) { self.renderer.profile_gpu = on; }
    /// The stages timed since last asked (name, milliseconds summed), loudest first.
    pub fn take_stages(&mut self) -> Vec<(&'static str, f64)> {
        let mut st = std::mem::take(&mut self.renderer.stages);
        st.sort_by(|a, b| b.1.total_cmp(&a.1));
        st
    }

    /// A journey file: the walk begins at its start stop.
    pub fn from_journey(path: impl AsRef<Path>) -> Result<Runtime, String> {
        let path = path.as_ref();
        let j = Journey::load(path)?;
        let dir = path.parent().map(Path::to_path_buf);
        let (scene, sdir) = j.scene_of(&j.start, dir.as_deref())?;
        let start = j.start.clone();
        let mut rt = Runtime::new(scene, sdir);
        rt.cold = j.stops.keys().filter(|k| **k != start).cloned().collect();
        rt.journey = Some((j, dir, start));
        Ok(rt)
    }

    pub fn from_file(path: impl AsRef<Path>) -> Result<Runtime, String> {
        let path = path.as_ref();
        let (scene, _) = scene::load_file(path)?;
        Ok(Runtime::new(scene, path.parent().map(Path::to_path_buf)))
    }

    pub fn from_json(text: &str, base_dir: Option<PathBuf>) -> Result<Runtime, String> {
        Ok(Runtime::new(Scene::from_json(text)?, base_dir))
    }

    pub fn scene(&self) -> &Scene { &self.scene }

    /// Change the scene (another biome, a tweak); caches that depend on it are rebuilt as needed.
    /// This cuts; `transition_to` walks.
    pub fn set_scene(&mut self, scene: Scene) { self.home = scene.camera.clone(); self.scene = scene; self.active = None; self.swinging = None; }

    /// `set_scene` for a scene whose files are named relative to `dir`.
    pub fn set_scene_in(&mut self, scene: Scene, dir: Option<PathBuf>) { self.set_scene(scene); self.base_dir = dir; }

    /// Metres before the view repeats.
    pub fn loop_length(&self) -> f32 { self.scene.motion.loop_length.max(1.0) }

    /// Seconds after which every timed effect (flames, clouds, weather) repeats.
    pub fn loop_seconds(&self) -> f32 { self.scene.motion.loop_seconds() }

    /// The scene's own canvas size, in pixels.
    pub fn canvas(&self) -> (u32, u32) { (self.scene.canvas.width, self.scene.canvas.height) }

    fn opts(&self, time: f32, width: u32, height: u32) -> RenderOptions {
        RenderOptions { size: Some((width.max(1), height.max(1))), base_dir: self.base_dir.clone(), time: Some(time), ..RenderOptions::default() }
    }

    /// The view `distance` metres along the path at `time` seconds, as RGBA8 rows of `width` pixels.
    pub fn render(&mut self, distance: f32, time: f32, width: u32, height: u32) -> &[u8] {
        let opts = self.opts(time, width, height);
        self.rgba = self.renderer.render(&self.scene, distance, &opts).rgba;
        &self.rgba
    }

    /// Get one more journey stop ready before the walk reaches it: its average colour (which a
    /// fork or crossing needs on its first frame) and the sprites and textures it paints, drawn
    /// once at `width` x `height` and thrown away. Otherwise the first frame into each new place
    /// pays for them, a hitch of a few tenths of a second. One stop per call, so a game spreads
    /// them over frames: first any the game asked for (`warm`), then the nearest to where the walk
    /// is, by the journey's roads either way and by a shared scene (the same place walked back),
    /// only as far as `WARM_REACH` stops. Each costs a frame's worth of time or more, so warming the
    /// whole journey up front stalled the frames the game was waiting on for many seconds; the
    /// ring moves with the walk instead. False once nothing near is left (and with no journey).
    pub fn warm_next(&mut self, width: u32, height: u32) -> bool {
        let Some(near) = self.near_stops() else { return false };
        let Some((j, dir, _)) = self.journey.as_ref() else { return false };
        // With painting ahead, every near stop is asked for, and one is drawn only once painted.
        if let Some(a) = self.ahead.as_mut() {
            let mut order: Vec<(&String, u32)> = self.wanted.iter().rev().map(|s| (s, 0)).chain(self.cold.iter().filter_map(|s| near.get(s).map(|d| (s, *d)))).collect();
            order.sort_by_key(|o| o.1);
            let mut first = None;
            for (s, _) in order { if a.ready(s, j, dir.as_deref()) && first.is_none() { first = Some(s.clone()); } }
            match first {
                Some(s) => { self.wanted.retain(|w| *w != s); self.cold.retain(|c| *c != s); return self.warm_stop(s, width, height); }
                None => return !self.wanted.is_empty() || self.cold.iter().any(|s| near.contains_key(s)),
            }
        }
        let stop = match self.wanted.pop() {
            Some(s) => s,
            None => {
                let Some((i, _)) = self.cold.iter().enumerate().filter_map(|(i, s)| near.get(s).map(|d| (i, *d))).min_by_key(|(_, d)| *d) else { return false };
                self.cold.swap_remove(i)
            }
        };
        self.cold.retain(|s| *s != stop);
        self.warm_stop(stop, width, height)
    }

    /// Have the threads that work ahead of the walk paint each near stop's props before
    /// `warm_next` draws it (off by default: then `warm_next` paints them as it draws).
    pub fn set_paint_ahead(&mut self, on: bool) {
        if on != self.ahead.is_some() { self.ahead = on.then(Ahead::spawn); }
    }

    /// `warm_next`'s draw of one stop (nothing more to do once it is got ready ahead: its pictures
    /// are sent to the GPU as they come into view).
    fn warm_stop(&mut self, stop: String, width: u32, height: u32) -> bool {
        let Some(near) = self.near_stops() else { return false };
        let Some((j, dir, _)) = self.journey.as_ref() else { return false };
        if self.ahead.is_some() { return !self.wanted.is_empty() || self.cold.iter().any(|s| near.contains_key(s)); }
        if let Ok((scene, sdir)) = j.scene_of(&stop, dir.as_deref()) {
            self.renderer.scene_mean(&scene, sdir.clone());
            let opts = RenderOptions { base_dir: sdir, ..self.opts(0.0, width, height) };
            let _ = self.renderer.render(&scene, 0.0, &opts);
        }
        !self.wanted.is_empty() || self.cold.iter().any(|s| near.contains_key(s))
    }

    /// The journey's stops near the walk, with how far (`warm_order`), worked out again only when
    /// the walk is at another stop. (Worked out every frame it took a twentieth of the road's
    /// thread on a large journey.)
    fn near_stops(&mut self) -> Option<std::sync::Arc<std::collections::HashMap<String, u32>>> {
        let (j, _, at) = self.journey.as_ref()?;
        if let Some((_, near)) = self.near.as_ref().filter(|(from, _)| from == at) { return Some(near.clone()); }
        let near = std::sync::Arc::new(warm_order(j, at));
        self.near = Some((at.clone(), near.clone()));
        Some(near)
    }

    /// Shut the fork ahead's branches (left, right) `BAR_PAST` metres past the junction, each in
    /// its own world (`Barrier`): a way the game will not let the walk take, seen to be shut.
    /// False with no fork under way.
    pub fn bar(&mut self, left: Option<tr::Barrier>, right: Option<tr::Barrier>) -> bool {
        let Some(Active::Fork { walk, .. }) = self.active.as_mut() else { return false };
        // Through an opened one the walk goes on until it is past the wall, not handing over to
        // the branch alone with the doorway still ahead.
        if left.iter().chain(&right).any(|b| b.open) { walk.plan.overshoot = walk.plan.overshoot.max(tr::BAR_PAST + 3.0); }
        walk.barred = [left, right];
        true
    }

    /// Close the road at `walked` metres of the walk (`WalkState::walked`) with a wall across it
    /// (`Barrier`, its picture in its foot), until taken off (None): a dead end the game holds
    /// the walk short of, seen to end.
    pub fn set_end(&mut self, end: Option<(f64, tr::Barrier)>) { self.end = end; }

    /// Never let the road fall darker than `brightness` (the mean perceived brightness, 0..1, of
    /// the frame below the horizon band, where the path and what stands on it are): exposure is
    /// raised over the scene's own, gently, frame by frame, until it is that bright, and let back
    /// down as the light returns. A place that keeps dark (`Post::keep_dark`) is left as drawn,
    /// for threats that are meant to hide. None: every scene as drawn.
    pub fn set_visibility_floor(&mut self, brightness: Option<f32>) {
        self.floor = brightness.map(|b| b.clamp(0.0, 0.6));
        if self.floor.is_none() { self.lift = 1.0; self.renderer.set_lift(None); }
    }

    /// Measure the frame just drawn and move the lift toward holding the floor (`set_visibility_floor`).
    fn hold_floor(&mut self, w: usize, h: usize) {
        let Some(floor) = self.floor else { return };
        if self.scene.post.keep_dark || w == 0 || h == 0 || self.rgba.len() < w * h * 4 {
            self.lift += (1.0 - self.lift) * 0.1;
        } else {
            let (mut sum, mut n) = (0.0f32, 0.0f32);
            for y in (h * 2 / 5..h).step_by(3) {
                for x in (0..w).step_by(5) {
                    let p = &self.rgba[(y * w + x) * 4..(y * w + x) * 4 + 3];
                    sum += (0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32) / 255.0;
                    n += 1.0;
                }
            }
            let mean = (sum / n.max(1.0)).max(1e-3);
            // Toward the lift that would bring the mean up to the floor (picture brightness goes
            // as about the square root of exposure, sRGB), never below the scene's own; down
            // again only once well clear of it.
            let want = if mean < floor { self.lift * (floor / mean).powi(2) } else if mean > floor * 1.25 { self.lift * (floor * 1.25 / mean).powi(2) } else { self.lift };
            self.lift += (want.clamp(1.0, 3.0) - self.lift) * 0.06;
        }
        self.renderer.set_lift(Some(self.lift));
    }

    /// Whether any stop the game asked for (`warm`) is still to be got ready.
    pub fn warm_wanted(&self) -> bool { !self.wanted.is_empty() }

    /// Have `warm_next` get these stops ready next, before the ones near the walk (the first
    /// soonest): the stops of a cutscene the game is about to cut to. Already warm ones are skipped.
    pub fn warm(&mut self, stops: &[String]) {
        for s in stops.iter().rev() {
            if self.cold.contains(s) && !self.wanted.contains(s) { self.wanted.push(s.clone()); }
        }
    }

    /// `render` into a buffer the caller owns (`width * height * 4` bytes).
    pub fn render_into(&mut self, distance: f32, time: f32, width: u32, height: u32, out: &mut [u8]) -> Result<(), String> {
        let need = width as usize * height as usize * 4;
        if out.len() < need { return Err(format!("buffer holds {} bytes; {width}x{height} RGBA needs {need}", out.len())); }
        let opts = self.opts(time, width, height);
        let img = self.renderer.render(&self.scene, distance, &opts);
        out[..need].copy_from_slice(&img.rgba[..need]);
        Ok(())
    }

    /// The camera at `distance`, for a screen of `width` x `height`.
    pub fn view(&self, distance: f32, width: u32, height: u32) -> View {
        View::new(&self.scene, width.max(2) as usize, height.max(2) as usize).at(distance.rem_euclid(self.loop_length()))
    }

    /// Where on screen (pixels from the top left) a point appears: `x` metres right of the path
    /// centre, `y` up from the ground, `d` ahead of the camera. None when it is behind the camera.
    pub fn project(&self, distance: f32, x: f32, y: f32, d: f32, width: u32, height: u32) -> Option<[f32; 2]> {
        self.view(distance, width, height).world_to_px(x, y, d)
    }

    /// How far ahead the ground seen on screen row `row` is, in metres (None above the horizon).
    pub fn ground_distance(&self, distance: f32, row: f32, width: u32, height: u32) -> Option<f32> {
        crate::mcp::ground_distance(&self.view(distance, width, height), row)
    }

    /// Screen x of the left and right path edges `d` metres ahead, and the row they are on.
    pub fn path_edges(&self, distance: f32, d: f32, width: u32, height: u32) -> Option<(f32, f32, f32)> {
        let v = self.view(distance, width, height);
        let hw = v.path_half_width(d);
        let (l, r) = (v.world_to_px(-hw, 0.0, d)?, v.world_to_px(hw, 0.0, d)?);
        Some((l[0], r[0], l[1]))
    }

    // ── The runtime's own walk ─────────────────────────────────────────────

    /// Walk on by `dt` seconds at `speed` metres per second (0 stands still; time still passes),
    /// through any transition under way. When it ends, the next scene becomes the current one.
    /// A negative speed walks back: along a scene's loop freely, and through a crossing or fork
    /// back to where it began (or, at a fork, to where its branch was taken) but no further.
    pub fn step(&mut self, dt: f32, speed: f32) {
        if self.day_seconds > 0.0 {
            let h = self.hour() + 24.0 * dt / self.day_seconds;
            self.renderer.set_hour(Some(h));
        }
        let mut ds = if speed.is_finite() { speed * dt.max(0.0) } else { 0.0 };
        // How far back a walk under way can go.
        let floor = match &self.active {
            None => None,
            Some(Active::Cross { travel, .. }) => Some((*travel, 0.0)),
            Some(Active::Fork { walk, travel, .. }) => Some((*travel, walk.branch(*travel).map_or(0.0, |(_, at)| at))),
        };
        if let Some((travel, floor)) = floor { if ds < 0.0 { ds = ds.max((floor - travel).min(0.0)); } }
        self.played += dt.max(0.0);
        if let Some(sw) = &mut self.swinging {
            sw.t += dt.max(0.0);
            if sw.t >= sw.style.length() { self.scene.camera = sw.to.clone(); self.swinging = None; }
        }
        self.time = (self.time + dt).rem_euclid(self.loop_seconds().max(1e-3));
        self.distance += ds;
        self.walked += ds as f64;
        let done = match &mut self.active {
            None => None,
            Some(Active::Cross { walk, travel, clock, .. }) => {
                *travel += ds;
                *clock += dt;
                (*travel >= walk.length()).then(|| walk.at(*travel).2)
            }
            Some(Active::Fork { walk, travel, clock, .. }) => {
                *travel += ds;
                *clock += dt;
                (*travel >= walk.length()).then(|| { let (_, _, sb, _) = walk.at(*travel); match walk.branch(*travel) { Some((Branch::Right, _)) => sb[1], _ => sb[0] } })
            }
        };
        if let Some(at) = done { self.arrive(at); }
        if self.active.is_none() { self.distance = self.distance.rem_euclid(self.loop_length()); }
    }

    fn arrive(&mut self, distance: f32) {
        let (next, clock) = match self.active.take() {
            Some(Active::Cross { next, clock, .. }) => (*next, clock),
            Some(Active::Fork { left, right, walk, travel, clock }) => (if walk.branch(travel).is_some_and(|(b, _)| b == Branch::Right) { *right } else { *left }, clock),
            None => return,
        };
        self.onward = None;
        self.scene = next.scene;
        self.home = self.scene.camera.clone();
        self.base_dir = next.dir;
        self.distance = distance.rem_euclid(self.loop_length());
        self.time = clock;
        if let (Some((_, _, at)), Some(stop)) = (self.journey.as_mut(), next.stop) { *at = stop; }
    }

    /// Play the camera's flight in and swing in (`camera.moves.fly_in`, `swing_in`) again from the
    /// top, as at the start.
    pub fn restart_intro(&mut self) { self.played = 0.0; self.swinging = None; }

    /// Swing the camera from where it is now (partway through another swing or the intro, even)
    /// to view `to` (its `back` is not used: the walk's camera stands at the walker), the way
    /// `style` says (its `from` is not used; 0 seconds is 2.5). The view stays once it lands:
    /// the scene's camera becomes it. Not while a transition is under way.
    pub fn swing_to(&mut self, to: scene::SwingView, style: scene::Swing) -> Result<(), String> {
        if self.active.is_some() { return Err("a transition is under way: swing once it has ended".into()); }
        let from = self.view_now();
        let mut style = style;
        if style.seconds <= 0.0 { style.seconds = 2.5; }
        let c = &self.scene.camera;
        let to = scene::Camera { yaw: to.yaw.clamp(-90.0, 90.0), side: to.side, eye_height: to.eye_height, horizon: to.horizon, zoom: to.zoom, ..c.clone() };
        // (Any intro still playing ends here: the swing takes the camera on from where it is.)
        let m = &c.moves;
        self.played = self.played.max(m.fly_in.max(0.0).max(m.swing_in.length()));
        self.swinging = Some(Swinging { style, from, to, t: 0.0 });
        Ok(())
    }

    /// `swing_to` as a game names it: a view by name (`home`: the camera the scene came with;
    /// `ahead`: looking down the path; `side` / `side_left`: a side-scroller's view from the right
    /// / left of the path), or JSON {"view": a name, "yaw", "side", "eye_height", "horizon",
    /// "zoom", "seconds", "hold", "ease", "overshoot", "orbit", "bars"}, every field optional (the
    /// view's fields override the named view's, or the camera's as it is; no hold unless given).
    pub fn swing(&mut self, spec: &str) -> Result<(), String> {
        let spec = spec.trim();
        let v: serde_json::Value = if spec.starts_with('{') { serde_json::from_str(spec).map_err(|e| format!("swing: {e}"))? } else { serde_json::json!({ "view": spec }) };
        let mut style: scene::Swing = serde_json::from_value(v.clone()).map_err(|e| format!("swing: {e}"))?;
        if v.get("hold").is_none() { style.hold = 0.0; }
        let now = scene::SwingView::of(&self.scene.camera);
        let mut to = match v.get("view").and_then(|n| n.as_str()) {
            None => now,
            Some("home") => scene::SwingView::of(&self.home),
            Some("ahead") => scene::SwingView { yaw: 0.0, side: 0.0, ..now },
            Some("side") => scene::SwingView { yaw: -90.0, side: 6.0, ..now },
            Some("side_left") => scene::SwingView { yaw: 90.0, side: -6.0, ..now },
            Some(o) => return Err(format!("swing: no view `{o}` (home, ahead, side, side_left)")),
        };
        let f = |k: &str, d: f32| v.get(k).and_then(|x| x.as_f64()).map_or(d, |x| x as f32);
        (to.yaw, to.side, to.eye_height, to.horizon, to.zoom) = (f("yaw", to.yaw), f("side", to.side), f("eye_height", to.eye_height), f("horizon", to.horizon), f("zoom", to.zoom));
        self.swing_to(to, style)
    }

    /// The camera's view now: the scene's own, or where a swing has it.
    pub fn view_now(&self) -> scene::SwingView {
        let pose = self.swinging.as_ref().and_then(Swinging::pose).or_else(|| self.intro().and_then(|t| self.scene.camera.moves.swing_in.pose(&self.scene.camera, t)));
        match pose {
            Some(p) => scene::SwingView { back: p.back, ..scene::SwingView::of(&p.camera) },
            None => scene::SwingView::of(&self.scene.camera),
        }
    }

    /// Seconds left of a swing the game started, while it plays.
    pub fn swing_left(&self) -> Option<f32> { self.swinging.as_ref().map(|s| s.style.length() - s.t) }

    /// Seconds into the intro (the flight in, the swing in), while it is playing.
    fn intro(&self) -> Option<f32> {
        let m = &self.scene.camera.moves;
        let f = m.fly_in.max(0.0).max(m.swing_in.length());
        (f > 0.0 && self.played < f).then_some(self.played)
    }

    /// Seconds left of the intro while it plays (a game holds the player's controls until it ends).
    pub fn intro_left(&self) -> Option<f32> {
        let m = &self.scene.camera.moves;
        self.intro().filter(|_| self.active.is_none()).map(|t| m.fly_in.max(0.0).max(m.swing_in.length()) - t)
    }

    /// Points (x, y, d in metres, as `View::project` takes them) each `frame` measures the light at,
    /// for a game lighting its own sprites as the world is lit; none by default.
    pub fn set_probes(&mut self, points: Vec<[f32; 3]>) { self.probes = points; }
    /// The light the last `frame` found at the `set_probes` points.
    pub fn lighting(&self) -> Option<&crate::world::render::Lighting> { self.renderer.lighting() }

    /// The current frame of the runtime's own walk, `width` x `height` RGBA8.
    pub fn frame(&mut self, width: u32, height: u32) -> &[u8] {
        let opts = RenderOptions { size: Some((width.max(1), height.max(1))), probes: self.probes.clone(), pipelined: self.pipelined, ..RenderOptions::default() };
        self.renderer.set_end(self.end.as_ref().map(|(at, wall)| ((at - self.walked) as f32, wall.clone())));
        let a = Place { scene: &self.scene, dir: self.base_dir.as_deref() };
        let img = match &self.active {
            None => {
                let swing = self.swinging.as_ref().and_then(Swinging::pose);
                let o = RenderOptions { base_dir: self.base_dir.clone(), time: Some(self.time), intro: self.intro(), swing, ..opts };
                self.renderer.render(&self.scene, self.distance, &o)
            }
            Some(Active::Cross { next, walk, travel, clock }) => {
                walk.frame(&mut self.renderer, a, Place { scene: &next.scene, dir: next.dir.as_deref() }, *travel, self.time, *clock, &opts)
            }
            Some(Active::Fork { left, right, walk, travel, clock }) => {
                let (l, r) = (Place { scene: &left.scene, dir: left.dir.as_deref() }, Place { scene: &right.scene, dir: right.dir.as_deref() });
                walk.frame(&mut self.renderer, a, l, r, *travel, [self.time, *clock, *clock], &opts)
            }
        };
        self.rgba = img.rgba;
        self.hold_floor(width as usize, height as usize);
        &self.rgba
    }

    /// Walk from here into `next` (files relative to `dir`). Returns the plan: what stands at the
    /// boundary and why, and anything that will not look right. A transition already under way
    /// is finished first (it cuts to its end).
    pub fn transition_to(&mut self, next: Scene, dir: Option<PathBuf>, t: &Transition) -> Crossing {
        self.finish();
        let c = tr::plan(&self.scene, &next, t);
        let walk = CrossWalk::new(c.clone(), self.distance, 0.0);
        self.active = Some(Active::Cross { next: Box::new(Bound { scene: next, dir, stop: None }), walk, travel: 0.0, clock: 0.0 });
        c
    }

    /// A fork ahead: the path splits into `left` and `right`. Call `choose` before the junction;
    /// otherwise the fork's default branch is taken.
    pub fn fork(&mut self, left: (Scene, Option<PathBuf>), right: (Scene, Option<PathBuf>), f: &ForkChoice) -> ForkPlan {
        self.finish();
        let p = tr::plan_fork(&self.scene, &left.0, &right.0, f);
        let walk = ForkWalk::new(p.clone(), self.distance, [0.0, 0.0]);
        self.active = Some(Active::Fork {
            left: Box::new(Bound { scene: left.0, dir: left.1, stop: None }), right: Box::new(Bound { scene: right.0, dir: right.1, stop: None }),
            walk, travel: 0.0, clock: 0.0,
        });
        p
    }

    /// Take a branch of the fork ahead. Returns false when there is no fork, or the choice came too
    /// late (the default branch has already been taken).
    pub fn choose(&mut self, branch: Branch) -> bool {
        match &mut self.active {
            Some(Active::Fork { walk, travel, .. }) if walk.chosen.is_none() && *travel <= walk.decide_by() => { walk.choose(branch, *travel); true }
            _ => false,
        }
    }

    /// End a transition under way at once (cut to the next scene).
    pub fn finish(&mut self) {
        // A swing the game started lands at once.
        if let Some(sw) = self.swinging.take() { self.scene.camera = sw.to; }
        let end = match &self.active {
            Some(Active::Cross { walk, .. }) => Some(walk.at(walk.length()).2),
            Some(Active::Fork { walk, .. }) => { let (_, _, sb, _) = walk.at(walk.length()); Some(match walk.branch(walk.length()) { Some((Branch::Right, _)) => sb[1], _ => sb[0] }) }
            None => None,
        };
        if let Some(d) = end { self.arrive(d); }
    }

    /// Start walking to where the journey leads from the current stop: a transition, or a fork
    /// (then `choose`). Returns what PathForge planned for it. Fails at the journey's end or
    /// without a journey.
    pub fn go(&mut self) -> Result<Planned, String> {
        self.finish();
        self.onward = None;
        let (j, dir, at) = self.journey.as_ref().ok_or("no journey loaded")?;
        let stop = j.stops.get(at).ok_or_else(|| format!("no stop `{at}`"))?;
        match stop.next.clone() {
            Next::End => Err(format!("`{at}` is the end of the journey")),
            Next::Go { to, transition } => {
                let (scene, sdir) = j.scene_of(&to, dir.as_deref())?;
                let c = self.transition_to(scene, sdir, &transition);
                if let Some(Active::Cross { next, .. }) = &mut self.active { next.stop = Some(to); }
                Ok(Planned { kind: c.threshold.name().to_owned(), notes: c.notes, warnings: c.warnings })
            }
            Next::Fork { left, right, fork } => {
                let l = j.scene_of(&left, dir.as_deref())?;
                let r = j.scene_of(&right, dir.as_deref())?;
                let p = self.fork(l, r, &fork);
                if let Some(Active::Fork { left: lb, right: rb, .. }) = &mut self.active { lb.stop = Some(left); rb.stop = Some(right); }
                Ok(Planned { kind: "Fork".into(), notes: p.notes, warnings: p.warnings })
            }
        }
    }

    /// Cut to the journey stop `stop`, at the start of its scene's loop with nothing under way: a
    /// shrine the player wakes at, a fast travel. `walked` carries on, so things a game keeps by it
    /// stay where they were. Fails without a journey or for a stop it does not have.
    pub fn warp(&mut self, stop: &str) -> Result<(), String> {
        let (j, dir, _) = self.journey.as_ref().ok_or("no journey loaded")?;
        if !j.stops.contains_key(stop) { return Err(format!("no stop `{stop}`")); }
        let (scene, sdir) = j.scene_of(stop, dir.as_deref())?;
        self.set_scene_in(scene, sdir);
        self.onward = None;
        self.distance = 0.0;
        if let Some((_, _, at)) = self.journey.as_mut() { *at = stop.to_owned(); }
        Ok(())
    }

    /// `warp`, but `metres` into the stop's scene: turning round partway along a place, the walk
    /// back starts where the player stood. Negative metres stand short of the stop's start, on
    /// its ground (turned round past a place's end, before the next one's boundary).
    pub fn warp_at(&mut self, stop: &str, metres: f32) -> Result<(), String> {
        self.warp(stop)?;
        self.distance = metres.rem_euclid(self.loop_length().max(1e-3));
        Ok(())
    }

    /// Turn round onto `stop`, `ahead` metres short of where the walk on from it hands over (the
    /// boundary of its crossing, the junction of its fork): the walk on starts partway when it is
    /// that close, and otherwise waits, the stop's end that much further short of it
    /// (`WalkState::handover` says how far past the end it is). A stop that leads nowhere is
    /// warped to.
    pub fn turn_into(&mut self, stop: &str, ahead: f32) -> Result<(), String> {
        self.start_on(stop, |handover| handover - ahead.max(0.0))
    }

    /// Cut to `stop` with its walk on already `travel` metres under way: turned round short of
    /// where a place was walked into, the crossing or fork back out of it is that far along.
    pub fn walk_on(&mut self, stop: &str, travel: f32) -> Result<(), String> {
        self.start_on(stop, |_| travel.max(0.0))
    }

    /// Warp to `stop` and start its walk on at the travel `into` gives for its hand-over; one that
    /// would start before its start is not started (`onward` keeps the hand-over).
    fn start_on(&mut self, stop: &str, into: impl Fn(f32) -> f32) -> Result<(), String> {
        self.warp(stop)?;
        if self.go().is_err() { return Ok(()); }
        let Some(handover) = self.handover() else { return Ok(()) };
        let into = into(handover);
        if into < 0.0 {
            self.active = None;
            self.onward = Some(handover);
            return Ok(());
        }
        match &mut self.active {
            Some(Active::Cross { travel, .. }) => *travel = into,
            Some(Active::Fork { travel, walk, .. }) => { *travel = into; walk.begun_at(into); }
            None => {}
        }
        self.distance += into;
        Ok(())
    }

    /// Metres from the start of the walk under way to where it hands over (`WalkState::handover`).
    fn handover(&self) -> Option<f32> {
        match &self.active {
            Some(Active::Cross { walk, .. }) => Some(walk.crossing.swing_out + walk.crossing.approach),
            Some(Active::Fork { walk, .. }) => Some(walk.plan.approach),
            None => None,
        }
    }

    // ── Day and night ──────────────────────────────────────────────────────

    /// Set the hour of the day (0..24) on every scene the runtime draws, transitions and forks
    /// included: the sun, sky, light, fog, stars, moon and lamps follow it (`sky.day_night`, on
    /// with the scene's own settings if it was off). None gives each scene back its own hour.
    /// Scenes without an open sky (dungeons, space, tunnels) are not touched.
    pub fn set_hour(&mut self, hour: Option<f32>) { self.renderer.set_hour(hour); }

    /// Lay a grade over every scene the runtime draws, transitions and forks included, until it
    /// is taken off (None): JSON merging into the scenes' `post` (exposure, contrast, saturation,
    /// bloom, vignette, grain, tint, flare, focus), the rest as each scene has it. A game's
    /// cutscene look; a field `Post` lacks is refused.
    pub fn set_grade(&mut self, grade: Option<&str>) -> Result<(), String> {
        let v = grade.map(|g| serde_json::from_str(g).map_err(|e| format!("grade: {e}"))).transpose()?;
        self.renderer.set_grade(v)
    }

    /// Let the days pass as the game runs: one day every `seconds` of `step` time, from the hour
    /// set (or the scene's own). 0 holds the hour where it is.
    pub fn set_day_length(&mut self, seconds: f32) {
        if seconds > 0.0 && self.renderer.hour().is_none() { self.renderer.set_hour(Some(self.hour())); }
        self.day_seconds = seconds.max(0.0);
    }

    /// The hour now: the one set, else the scene's own (moving with the loop when its days pass
    /// with it).
    pub fn hour(&self) -> f32 {
        let dn = &self.scene.sky.day_night;
        self.renderer.hour().unwrap_or_else(|| dn.hour_at(self.time / self.loop_seconds().max(1e-3)))
    }

    /// Where the day stands now (sun height, daylight, "night"/"dawn"/"day"/"dusk"), when the
    /// scene has a day and night or the hour is set; None for scenes without an open sky.
    pub fn day(&self) -> Option<scene::DayState> {
        let sky = &self.scene.sky;
        let on = sky.enabled && !sky.space.enabled && !sky.tunnel.enabled && (sky.day_night.enabled || self.renderer.hour().is_some());
        on.then(|| sky.day_night.at(self.hour()))
    }

    /// Where the walk stands.
    pub fn state(&self) -> WalkState {
        let (progress, choose_within) = match &self.active {
            None => (None, None),
            Some(Active::Cross { walk, travel, .. }) => (Some((travel / walk.length()).min(1.0)), None),
            Some(Active::Fork { walk, travel, .. }) => (Some((travel / walk.length()).min(1.0)), walk.chosen.is_none().then(|| (walk.decide_by() - travel).max(0.0))),
        };
        let end = self.active.is_none() && self.journey.as_ref().is_some_and(|(j, _, at)| j.stops.get(at).is_some_and(|s| matches!(s.next, Next::End)));
        let travel = match &self.active { Some(Active::Cross { travel, .. } | Active::Fork { travel, .. }) => Some(*travel), None => None };
        WalkState { scene: self.scene.name.clone(), distance: self.distance, walked: self.walked, time: self.time, stop: self.journey.as_ref().map(|j| j.2.clone()), progress, choose_within,
            travel, handover: self.handover().or(self.onward), speed: self.scene.motion.speed, end, day: self.day(), intro_left: self.intro_left(), swing_left: self.swing_left(),
            side_paths: if self.active.is_none() { crate::world::render::side_paths(&self.scene, self.distance - 6.0, self.distance + 40.0).into_iter().map(|(w, side)| (w - self.distance, side)).collect() } else { Vec::new() } }
    }

    /// The camera of the runtime's own walk now, for a `width` x `height` frame. In a transition,
    /// points past the boundary are placed by the next scene's camera.
    pub fn camera_now(&self, width: u32, height: u32) -> Camera {
        match &self.active {
            Some(Active::Cross { next, walk, travel, .. }) => walk.camera(&self.scene, &next.scene, *travel, width, height),
            _ => match self.swinging.as_ref().and_then(Swinging::pose).or_else(|| self.intro().and_then(|t| self.scene.camera.moves.swing_in.pose(&self.scene.camera, t))) {
                Some(p) => Camera::swinging(&self.scene, &p, self.distance, width, height),
                None => Camera::plain(&self.scene, self.distance, width, height),
            },
        }
    }

    /// `project` for the runtime's own walk: where a point `d` metres ahead appears now.
    pub fn project_now(&self, x: f32, y: f32, d: f32, width: u32, height: u32) -> Option<[f32; 2]> { self.camera_now(width, height).project(x, y, d) }

    /// `ground_distance` for the runtime's own walk.
    pub fn ground_distance_now(&self, row: f32, width: u32, height: u32) -> Option<f32> { self.camera_now(width, height).ground_distance(row) }
}

/// Distance walked and time passed, for a game loop. Both are kept within one loop, so a game
/// can run for hours without losing precision (the view repeats each loop anyway).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Walk {
    pub distance: f32,
    pub time: f32,
}

impl Walk {
    /// Move on by `dt` seconds at `speed` metres per second (0 stands still; time still passes).
    pub fn advance(&mut self, dt: f32, speed: f32, rt: &Runtime) {
        self.distance = (self.distance + speed * dt).rem_euclid(rt.loop_length());
        self.time = (self.time + dt).rem_euclid(rt.loop_seconds().max(1e-3));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rt(name: &str) -> Runtime {
        Runtime::new(scene::presets::ALL.iter().find(|p| p.0 == name).map(|p| (p.1)()).unwrap(), None)
    }

    #[test]
    fn a_visibility_floor_lifts_a_dark_road_until_it_can_be_seen_and_leaves_a_dark_place_dark() {
        let mean = |r: &Runtime, w: usize, h: usize| {
            let px: Vec<f32> = (h * 2 / 5..h).flat_map(|y| (0..w).map(move |x| (y, x))).map(|(y, x)| {
                let p = &r.rgba[(y * w + x) * 4..(y * w + x) * 4 + 3];
                (0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32) / 255.0
            }).collect();
            px.iter().sum::<f32>() / px.len() as f32
        };
        let (w, h) = (64, 36);
        let mut r = rt("Night Road");
        r.frame(w as u32, h as u32);
        let dark = mean(&r, w, h);
        r.set_visibility_floor(Some(dark * 1.6));
        for _ in 0..120 { r.frame(w as u32, h as u32); }
        assert!(r.lift > 1.2 && mean(&r, w, h) > dark * 1.35, "lift {} brightness {} from {dark}", r.lift, mean(&r, w, h));
        // Off again: the scene as drawn.
        r.set_visibility_floor(None);
        r.frame(w as u32, h as u32);
        assert!((mean(&r, w, h) - dark).abs() < 0.01);
        // A place that keeps dark is not lifted.
        let mut s = r.scene().clone();
        s.post.keep_dark = true;
        let mut k = Runtime::new(s, None);
        k.set_visibility_floor(Some(dark * 1.6));
        for _ in 0..60 { k.frame(w as u32, h as u32); }
        assert!(k.lift < 1.01, "{}", k.lift);
    }

    #[test]
    fn renders_at_any_size_and_matches_the_exported_loop() {
        let mut r = rt("Stone Dungeon");
        for (w, h) in [(90u32, 160u32), (270, 480), (101, 333)] {
            assert_eq!(r.render(3.0, 1.0, w, h).len(), (w * h * 4) as usize);
        }
        // The frame an export would hold at that distance (time follows distance there).
        let len = r.loop_length();
        let secs = r.loop_seconds();
        let a = r.render(len * 0.25, secs * 0.25, 120, 214).to_vec();
        let exported = WorldRenderer::default().render(r.scene(), len * 0.25, &RenderOptions { size: Some((120, 214)), ..RenderOptions::default() });
        assert!(a == exported.rgba, "the runtime should draw exactly the exported frame");
        // A whole loop on, it is the same frame: no seam however far the hero walks.
        let b = r.render(len * 0.25 + 7.0 * len, secs * 0.25 + 3.0 * secs, 120, 214).to_vec();
        assert!(a == b);
        let mut buf = vec![0u8; 120 * 214 * 4];
        r.render_into(len * 0.25, secs * 0.25, 120, 214, &mut buf).unwrap();
        assert!(buf == a);
        assert!(r.render_into(0.0, 0.0, 120, 214, &mut buf[..10]).is_err());
    }

    /// A game walks back (a backstep, a knockback): along the loop to exactly where it was, and
    /// back through a crossing to where it began but no further; at a fork no further back than
    /// where its branch was taken.
    #[test]
    fn a_walker_steps_back_along_the_loop_and_through_a_crossing_or_fork() {
        let mut r = rt("Stone Dungeon");
        let len = r.loop_length();
        r.step(1.0, 3.0);
        let d = r.state().distance;
        r.step(0.5, 4.0);
        r.step(0.5, -4.0);
        assert!((r.state().distance - d).abs() < 1e-4, "back where it was ({} vs {d})", r.state().distance);
        // The distance walked does not come round with the loop.
        r.step(2.0, len);
        assert!((r.state().walked - (3.0 + 2.0 * len as f64)).abs() < 1e-3, "walked {}", r.state().walked);
        r.step(2.0, -len);
        // Back past the loop's start it comes round to its end, and draws that view.
        r.step(1.0, -(d + 1.5));
        assert!((r.state().distance - (len - 1.5)).abs() < 1e-3, "{}", r.state().distance);
        let t = r.state().time;
        let back = r.frame(60, 107).to_vec();
        assert!(back == r.render(len - 1.5, t, 60, 107).to_vec());
        // A broken speed walks nowhere rather than poisoning the walk.
        r.step(1.0, f32::NAN);
        assert!((r.state().distance - (len - 1.5)).abs() < 1e-3);

        // A crossing: back to its start and no further, then on again.
        let next = rt("Forest Path").scene().clone();
        r.transition_to(next, None, &crate::scene::transition::Transition::default());
        r.step(1.0, 4.0);
        let p = r.state().progress.expect("crossing");
        r.step(1.0, -10.0);
        assert_eq!(r.state().progress, Some(0.0), "back at the crossing's start");
        assert_eq!(r.state().scene, "Stone Dungeon", "still in the scene it left");
        r.step(1.0, 4.0);
        assert!((r.state().progress.unwrap() - p).abs() < 1e-5);

        // A fork: once a branch is taken it stays taken.
        let mut f = rt("Stone Dungeon");
        let (a, b) = (rt("Forest Path").scene().clone(), rt("Ice Cave").scene().clone());
        f.fork((a, None), (b, None), &crate::scene::transition::ForkChoice::default());
        f.step(1.0, 2.0);
        assert!(f.choose(crate::scene::transition::Branch::Right));
        f.step(1.0, 3.0);
        let at5 = f.state().progress.unwrap();
        f.step(1.0, -20.0);
        let floor = f.state().progress.unwrap();
        assert!((floor * 5.0 / at5 - 2.0).abs() < 1e-3, "back to where the branch was taken (2 m), not {} m", floor * 5.0 / at5);
    }

    #[test]
    fn the_flight_in_plays_once_from_the_start() {
        let mut r = rt("Waterfall Glen");
        let start = r.frame(60, 107).to_vec();
        r.step(0.5, 0.0);
        let early = r.frame(60, 107).to_vec();
        assert!(start != early, "the camera is coming down");
        for _ in 0..12 { r.step(0.5, 0.0); }
        let landed = r.frame(60, 107).to_vec();
        let opts = RenderOptions { size: Some((60, 107)), time: Some(r.state().time), ..RenderOptions::default() };
        assert!(landed == WorldRenderer::default().render(r.scene(), r.state().distance, &opts).rgba, "landed, it is the walk's own camera");
        r.restart_intro();
        assert!(r.frame(60, 107).to_vec() != landed, "and it can fly in again");
    }

    #[test]
    fn the_swing_in_tells_the_game_when_it_ends_and_where_the_walker_is() {
        let mut r = rt("Village Side Scroll");
        let mut s = r.scene().clone();
        s.camera.moves.swing_in = scene::Swing { seconds: 2.0, hold: 0.5, ..scene::Swing::default() };
        // (A straight path, so the walker's spot is in the middle of the frame looking down it.)
        s.path.winding.enabled = false;
        r.set_scene(s);
        assert_eq!(r.state().intro_left, Some(2.5));
        // Behind the walker at first: the walker's spot in the middle of the frame, ahead.
        let (w, h) = (160, 90);
        let at = r.project_now(0.0, 0.0, 0.0, w, h).expect("the walker is in view");
        assert!((at[0] - 80.0).abs() < 1.0, "{at:?}");
        r.step(1.5, 0.0);
        assert_eq!(r.state().intro_left, Some(1.0));
        let mid = r.project_now(0.0, 0.0, 0.0, w, h).expect("still in view, coming round");
        assert!((mid[0] - 80.0).abs() < 1.0, "{mid:?}");
        r.step(1.0, 0.0);
        assert_eq!(r.state().intro_left, None, "over: the game takes the controls");
        let opts = RenderOptions { size: Some((w, h)), time: Some(r.state().time), ..RenderOptions::default() };
        assert!(r.frame(w, h).to_vec() == WorldRenderer::default().render(r.scene(), r.state().distance, &opts).rgba, "and it is the scene's own camera");
    }

    #[test]
    fn a_game_swings_the_camera_to_another_view_and_back() {
        let mut r = rt("Village Side Scroll");
        let side = r.scene().camera.clone();
        let (w, h) = (160, 90);
        let before = r.frame(w, h).to_vec();
        // Round to look down the path, over two seconds.
        let ahead = scene::SwingView { yaw: 0.0, side: 0.0, eye_height: 1.7, horizon: 0.4, zoom: 1.0, back: 0.0 };
        r.swing_to(ahead, scene::Swing { seconds: 2.0, hold: 0.0, ..scene::Swing::default() }).unwrap();
        assert_eq!(r.state().swing_left, Some(2.0));
        assert!(r.frame(w, h).to_vec() == before, "it starts where the camera was");
        r.step(1.0, 0.0);
        let mid = r.view_now();
        assert!(mid.yaw > side.yaw && mid.yaw < 0.0, "partway round ({})", mid.yaw);
        r.step(1.0, 0.0);
        assert_eq!(r.state().swing_left, None);
        assert_eq!((r.scene().camera.yaw, r.scene().camera.side, r.scene().camera.horizon), (0.0, 0.0, 0.4), "it stays on the new view");
        // And back, interrupted halfway by another swing: it carries on from where it is.
        r.swing_to(scene::SwingView::of(&side), scene::Swing::default()).unwrap();
        r.step(1.25, 0.0);
        let half = r.view_now();
        r.swing_to(ahead, scene::Swing { seconds: 1.0, hold: 0.0, ..scene::Swing::default() }).unwrap();
        let from = r.view_now();
        assert!((from.yaw - half.yaw).abs() < 1e-3 && (from.side - half.side).abs() < 1e-3, "no jump: {from:?} vs {half:?}");
    }

    #[test]
    fn standing_still_the_flames_still_move() {
        let mut r = rt("Stone Dungeon");
        let a = r.render(5.0, 0.0, 120, 214).to_vec();
        let b = r.render(5.0, 0.37, 120, 214).to_vec();
        assert!(a != b, "time alone should change the frame (flames)");
    }

    #[test]
    fn the_camera_answers_where_things_are_at_the_size_asked() {
        let r = rt("Forest Path");
        let (w, h) = (270, 480);
        let p = r.project(0.0, 0.0, 0.0, 8.0, w, h).unwrap();
        assert!((p[0] - w as f32 / 2.0).abs() < 0.5, "the path centre is mid-screen: {p:?}");
        let d = r.ground_distance(0.0, p[1], w, h).unwrap();
        assert!((d - 8.0).abs() < 0.05, "row {} maps back to {d} m", p[1]);
        let (l, rr, row) = r.path_edges(0.0, 8.0, w, h).unwrap();
        assert!(l < p[0] && rr > p[0] && (row - p[1]).abs() < 1e-3);
        assert!(r.ground_distance(0.0, 1.0, w, h).is_none(), "the top row is sky");
        // Twice the resolution, twice the pixels.
        let p2 = r.project(0.0, 0.0, 0.0, 8.0, w * 2, h * 2).unwrap();
        assert!((p2[1] - 2.0 * p[1]).abs() < 1.5);
    }

    #[test]
    fn walks_into_the_next_scene_and_takes_the_chosen_branch() {
        let mut r = rt("Stone Dungeon");
        r.step(0.5, 4.0);
        let c = r.transition_to(rt("Forest Path").scene().clone(), None, &Transition::default());
        let len = c.approach + c.overshoot;
        let mut seen_mid = false;
        let mut walked = 0.0;
        while r.state().progress.is_some() {
            r.step(0.25, 4.0);
            walked += 1.0;
            if !seen_mid && r.state().progress.unwrap_or(0.0) > 0.5 { assert_eq!(r.frame(54, 96).len(), 54 * 96 * 4); seen_mid = true; }
            assert!(walked < len * 2.0, "the transition never ended");
        }
        assert_eq!(r.scene().name, "Forest Path");
        // Then a fork, choosing right.
        let plan = r.fork((rt("Mountain Pass").scene().clone(), None), (rt("Desert Canyon").scene().clone(), None), &ForkChoice::default());
        assert!(r.state().choose_within.is_some_and(|m| m > 0.0));
        r.step(0.5, 4.0);
        assert!(r.choose(Branch::Right));
        assert!(!r.choose(Branch::Left), "a branch can be chosen once");
        for _ in 0..((plan.approach + plan.overshoot) as usize * 2) { r.step(0.25, 4.0); }
        assert_eq!(r.scene().name, "Desert Canyon");
        // Unchosen: the default branch.
        r.fork((rt("Mountain Pass").scene().clone(), None), (rt("Desert Canyon").scene().clone(), None), &ForkChoice::default());
        r.finish();
        assert_eq!(r.scene().name, "Mountain Pass");
    }

    #[test]
    fn a_journey_says_where_each_place_leads() {
        let dir = std::env::temp_dir().join(format!("pf_journey_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut j = Journey { version: 1, name: "test".into(), start: "crypt".into(), ..Journey::default() };
        j.stops.insert("crypt".into(), crate::journey::Stop { scene: "preset:Stone Dungeon".into(), next: Next::Go { to: "woods".into(), transition: Transition::default() } });
        j.stops.insert("woods".into(), crate::journey::Stop { scene: "preset:Forest Path".into(), next: Next::Fork { left: "pass".into(), right: "sands".into(), fork: ForkChoice::default() } });
        j.stops.insert("pass".into(), crate::journey::Stop { scene: "preset:Mountain Pass".into(), next: Next::End });
        j.stops.insert("sands".into(), crate::journey::Stop { scene: "preset:Desert Canyon".into(), next: Next::End });
        assert!(j.problems(Some(&dir)).is_empty(), "{:?}", j.problems(Some(&dir)));
        let path = dir.join("game.journey.json");
        j.save(&path).unwrap();
        let mut r = Runtime::from_journey(&path).unwrap();
        assert_eq!(r.state().stop.as_deref(), Some("crypt"));
        // The stops next to the walk are got ready one call at a time (the crypt leads only to
        // the woods), the walk left where it was; asked for, a far one is got ready too.
        let before = r.state();
        let calls = std::iter::repeat_with(|| r.warm_next(48, 27)).take_while(|more| *more).count() + 1;
        assert_eq!((calls, r.warm_next(48, 27), r.state()), (1, false, before.clone()));
        assert_eq!(r.cold.len(), 2);
        r.warm(&["sands".to_owned()]);
        assert!(r.warm_wanted());
        assert!(!r.warm_next(48, 27) && !r.warm_wanted() && r.cold == ["pass"], "{:?}", r.cold);
        assert_eq!(r.state(), before);
        r.go().unwrap();
        r.finish();
        assert_eq!(r.state().stop.as_deref(), Some("woods"));
        r.go().unwrap();
        r.choose(Branch::Right);
        r.finish();
        assert_eq!(r.state().stop.as_deref(), Some("sands"));
        assert!(r.go().is_err(), "the end leads nowhere");
        // Back to the first stop (a shrine to wake at), the metres walked kept.
        r.step(1.0, 3.0);
        let walked = r.state().walked;
        r.warp("crypt").unwrap();
        assert_eq!((r.state().stop.as_deref(), r.state().walked, r.state().distance), (Some("crypt"), walked, 0.0));
        assert!(r.go().is_ok() && r.warp("lost").is_err());
        let mut bad = j.clone();
        bad.stops.insert("lost".into(), crate::journey::Stop { scene: "missing.json".into(), next: Next::Go { to: "nowhere".into(), transition: Transition::default() } });
        assert_eq!(bad.problems(Some(&dir)).len(), 3, "{:?}", bad.problems(Some(&dir)));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn turned_round_short_of_a_boundary_or_junction_the_walk_on_starts_that_far_short_of_it() {
        let dir = std::env::temp_dir().join(format!("pf_turn_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut j = Journey { version: 1, name: "turn".into(), start: "crypt".into(), ..Journey::default() };
        j.stops.insert("crypt".into(), crate::journey::Stop { scene: "preset:Stone Dungeon".into(), next: Next::Go { to: "woods".into(), transition: Transition::default() } });
        j.stops.insert("woods".into(), crate::journey::Stop { scene: "preset:Forest Path".into(), next: Next::Fork { left: "crypt".into(), right: "sands".into(), fork: ForkChoice::default() } });
        j.stops.insert("sands".into(), crate::journey::Stop { scene: "preset:Desert Canyon".into(), next: Next::End });
        let path = dir.join("turn.journey.json");
        j.save(&path).unwrap();
        let mut r = Runtime::from_journey(&path).unwrap();
        // Walking a crossing, the state says how far in it is and where its boundary is.
        r.go().unwrap();
        r.step(1.0, 2.5);
        let (travel, boundary) = (r.state().travel, r.state().handover.expect("a boundary"));
        assert_eq!(travel, Some(2.5));
        assert!(boundary > 2.5, "{boundary}");
        // Turned round 3 m short of the crypt's boundary: its crossing is under way, that far short.
        r.turn_into("crypt", 3.0).unwrap();
        let st = r.state();
        assert_eq!(st.stop.as_deref(), Some("crypt"));
        assert!((st.travel.unwrap() - (boundary - 3.0)).abs() < 1e-4 && st.handover == Some(boundary), "{st:?}");
        // Further than the boundary is from the crossing's start: nothing under way yet, and the
        // state says how far past the stop's end the boundary will be.
        r.turn_into("crypt", boundary + 7.0).unwrap();
        let st = r.state();
        assert_eq!((st.stop.as_deref(), st.travel, st.handover, st.progress), (Some("crypt"), None, Some(boundary), None));
        // Walking on again starts the crossing from its start; then the state is the crossing's.
        r.go().unwrap();
        assert_eq!((r.state().travel, r.state().handover), (Some(0.0), Some(boundary)));
        // A fork's hand-over is its junction.
        r.turn_into("woods", 5.0).unwrap();
        let st = r.state();
        assert!(st.choose_within.is_some(), "the fork is ahead: {st:?}");
        assert!((st.handover.unwrap() - st.travel.unwrap() - 5.0).abs() < 1e-4, "{st:?}");
        // Inside the last metres where the choice is made: it is made where the walk begins, the
        // camera not yet turned onto a branch.
        r.turn_into("woods", 3.0).unwrap();
        assert_eq!(r.state().choose_within, Some(0.0));
        let Some(Active::Fork { walk, travel, .. }) = &r.active else { panic!("no fork under way") };
        assert!(walk.at(*travel).3 < 1e-4, "already turned {}", walk.at(*travel).3);
        // Cut in with the walk on partway: the crypt's crossing 4 m along.
        r.walk_on("crypt", 4.0).unwrap();
        assert_eq!((r.state().stop.as_deref(), r.state().travel, r.state().handover), (Some("crypt"), Some(4.0), Some(boundary)));
        // A stop that leads nowhere is simply warped to.
        r.turn_into("sands", 2.0).unwrap();
        assert_eq!((r.state().stop.as_deref(), r.state().travel, r.state().handover), (Some("sands"), None, None));
        // Short of a stop's start, on its ground.
        r.warp_at("crypt", -4.0).unwrap();
        assert!((r.state().distance - (r.loop_length() - 4.0)).abs() < 1e-3);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_long_walk_stays_inside_one_loop() {
        let r = rt("Stone Dungeon");
        let mut w = Walk::default();
        for _ in 0..100_000 { w.advance(1.0 / 60.0, 4.0, &r); }
        assert!(w.distance >= 0.0 && w.distance < r.loop_length());
        assert!(w.time >= 0.0 && w.time < r.loop_seconds());
        let before = w.distance;
        w.advance(0.5, 0.0, &r);
        assert_eq!(w.distance, before, "speed 0 stands still");
    }
}
