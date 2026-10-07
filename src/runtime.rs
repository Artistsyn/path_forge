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

use crate::journey::{Camera, CrossWalk, ForkWalk, Journey, Next, Place};
use crate::scene::transition::{self as tr, Branch, Crossing, ForkChoice, ForkPlan, Transition};
use crate::scene::{self, Scene};
use crate::world::{RenderOptions, View, WorldRenderer};
use std::path::{Path, PathBuf};

pub struct Runtime {
    scene: Scene,
    base_dir: Option<PathBuf>,
    renderer: WorldRenderer,
    rgba: Vec<u8>,
    /// The runtime's own walk (see `step`): metres along the current scene and its clock.
    distance: f32,
    time: f32,
    active: Option<Active>,
    journey: Option<(Journey, Option<PathBuf>, String)>,
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
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct WalkState {
    /// The scene the camera is in (the one it left, until a transition ends).
    pub scene: String,
    pub distance: f32,
    pub time: f32,
    /// The journey stop the walk is at, if a journey is loaded.
    pub stop: Option<String>,
    /// 0..1 through a transition or fork under way.
    pub progress: Option<f32>,
    /// A fork is ahead and no branch has been chosen: metres left to choose before the default is taken.
    pub choose_within: Option<f32>,
    /// The walking speed of the scene the camera is in, m/s.
    pub speed: f32,
    /// At a journey stop that leads nowhere, with nothing under way.
    pub end: bool,
}

impl Runtime {
    /// A scene whose files (sprites, kits, grades) are named relative to `base_dir`.
    pub fn new(scene: Scene, base_dir: Option<PathBuf>) -> Runtime {
        Runtime { scene, base_dir, renderer: WorldRenderer::default(), rgba: Vec::new(), distance: 0.0, time: 0.0, active: None, journey: None }
    }

    /// A journey file: the walk begins at its start stop.
    pub fn from_journey(path: impl AsRef<Path>) -> Result<Runtime, String> {
        let path = path.as_ref();
        let j = Journey::load(path)?;
        let dir = path.parent().map(Path::to_path_buf);
        let (scene, sdir) = j.scene_of(&j.start, dir.as_deref())?;
        let start = j.start.clone();
        let mut rt = Runtime::new(scene, sdir);
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
    pub fn set_scene(&mut self, scene: Scene) { self.scene = scene; self.active = None; }

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
    pub fn step(&mut self, dt: f32, speed: f32) {
        let ds = (speed * dt).max(0.0);
        self.time = (self.time + dt).rem_euclid(self.loop_seconds().max(1e-3));
        self.distance += ds;
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
        self.scene = next.scene;
        self.base_dir = next.dir;
        self.distance = distance.rem_euclid(self.loop_length());
        self.time = clock;
        if let (Some((_, _, at)), Some(stop)) = (self.journey.as_mut(), next.stop) { *at = stop; }
    }

    /// The current frame of the runtime's own walk, `width` x `height` RGBA8.
    pub fn frame(&mut self, width: u32, height: u32) -> &[u8] {
        let opts = RenderOptions { size: Some((width.max(1), height.max(1))), ..RenderOptions::default() };
        let a = Place { scene: &self.scene, dir: self.base_dir.as_deref() };
        let img = match &self.active {
            None => {
                let o = RenderOptions { base_dir: self.base_dir.clone(), time: Some(self.time), ..opts };
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

    /// Where the walk stands.
    pub fn state(&self) -> WalkState {
        let (progress, choose_within) = match &self.active {
            None => (None, None),
            Some(Active::Cross { walk, travel, .. }) => (Some((travel / walk.length()).min(1.0)), None),
            Some(Active::Fork { walk, travel, .. }) => (Some((travel / walk.length()).min(1.0)), walk.chosen.is_none().then(|| (walk.decide_by() - travel).max(0.0))),
        };
        let end = self.active.is_none() && self.journey.as_ref().is_some_and(|(j, _, at)| j.stops.get(at).is_some_and(|s| matches!(s.next, Next::End)));
        WalkState { scene: self.scene.name.clone(), distance: self.distance, time: self.time, stop: self.journey.as_ref().map(|j| j.2.clone()), progress, choose_within, speed: self.scene.motion.speed, end }
    }

    /// The camera of the runtime's own walk now, for a `width` x `height` frame. In a transition,
    /// points past the boundary are placed by the next scene's camera.
    pub fn camera_now(&self, width: u32, height: u32) -> Camera {
        match &self.active {
            Some(Active::Cross { next, walk, travel, .. }) => walk.camera(&self.scene, &next.scene, *travel, width, height),
            _ => Camera::plain(&self.scene, self.distance, width, height),
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
        r.go().unwrap();
        r.finish();
        assert_eq!(r.state().stop.as_deref(), Some("woods"));
        r.go().unwrap();
        r.choose(Branch::Right);
        r.finish();
        assert_eq!(r.state().stop.as_deref(), Some("sands"));
        assert!(r.go().is_err(), "the end leads nowhere");
        let mut bad = j.clone();
        bad.stops.insert("lost".into(), crate::journey::Stop { scene: "missing.json".into(), next: Next::Go { to: "nowhere".into(), transition: Transition::default() } });
        assert_eq!(bad.problems(Some(&dir)).len(), 3, "{:?}", bad.problems(Some(&dir)));
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
