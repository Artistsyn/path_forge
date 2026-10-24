//! Walking from one world into another: the playback side of transitions and forks, shared by
//! exported clips and the live runtime.
//!
//! A walk is described in metres travelled since it began. From that it knows where the boundary
//! (or the junction) is ahead of the camera and where the camera stands in each world. Its frames
//! begin exactly on the first world's own frame and end exactly on the next world's, so a game can
//! cut from a loop into the walk and out of it again without a seam.

use crate::scene::transition::{Branch, Crossing, ForkPlan};
use crate::scene::Scene;
use crate::world::{Image, Layout, RenderOptions, View, WorldIn, WorldRenderer};
use std::path::PathBuf;

fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A walk's frame while it fades: the first world alone under the walk's frame by `emerge`
/// (1 = the walk's alone), then the world it settles in over that by `settle` (0 = none), as
/// `mix_images` would mix them, the renders held and mixed on the GPU so the frame comes back
/// as one (pipelined, a live road's). Its light is what the walk's frame order left: the last
/// world alone drawn.
fn mixed_frame(r: &mut WorldRenderer, opts: &RenderOptions, emerge: f32, settle: f32,
    first: impl FnOnce(&mut WorldRenderer, &RenderOptions) -> Image,
    walk: impl FnOnce(&mut WorldRenderer, &RenderOptions) -> Image,
    last: impl FnOnce(&mut WorldRenderer, &RenderOptions) -> Image) -> Image {
    let fading = emerge < 1.0;
    let settling = settle > 0.0;
    if !fading && !settling { return walk(r, opts); }
    let held = |blend: Option<f32>, hold: bool| RenderOptions { blend, hold, ..opts.clone() };
    let mut light = None;
    if fading {
        first(r, &held(None, true));
        light = r.lighting().cloned();
    }
    let img = walk(r, &held(fading.then_some(emerge), settling));
    if !settling {
        r.keep_lighting(light);
        return img;
    }
    last(r, &held(Some(settle), false))
}

/// Mix `b` over `a` by `t` (0 = a).
fn mix_images(a: &mut Image, b: &Image, t: f32) { crate::world::render::mix_rgba(&mut a.rgba, &b.rgba, t); }

/// A scene taking part in a walk, with the folder its files are named relative to.
#[derive(Clone, Copy)]
pub struct Place<'a> {
    pub scene: &'a Scene,
    pub dir: Option<&'a std::path::Path>,
}

impl<'a> Place<'a> {
    fn world(&self, distance: f32, time: f32) -> WorldIn<'a> {
        WorldIn { scene: self.scene, distance, time: Some(time), base_dir: self.dir.map(|d| d.to_path_buf()) }
    }
    fn alone(&self, r: &mut WorldRenderer, distance: f32, time: f32, opts: &RenderOptions) -> Image {
        r.render(self.scene, distance, &RenderOptions { time: Some(time), base_dir: self.dir.map(PathBuf::from), ..opts.clone() })
    }
}

/// The camera of a frame, for placing things on it: one view, or during a crossing the first
/// world's view up to the boundary `zb` metres ahead and the second world's beyond it.
#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub view: View,
    pub beyond: Option<(f32, View)>,
    /// Metres the camera stands behind the walker (a swing's third-person view; 0 at the walker's
    /// eyes). Distances ahead are the walker's.
    pub back: f32,
    /// How far past the path's edge the walls stand, in the view's scene and the one beyond
    /// (None: no walls), for `wall_x`.
    pub walls: [Option<f32>; 2],
}

/// How far past the path's edge `scene`'s walls stand (None: it has none).
fn walls_of(scene: &Scene) -> Option<f32> { scene.walls.enabled.then(|| scene.walls.gap.max(0.0)) }

impl Camera {
    /// The camera `distance` metres along `scene`'s loop, for a `width` x `height` frame.
    pub fn plain(scene: &Scene, distance: f32, width: u32, height: u32) -> Camera {
        Camera { view: View::new(scene, width.max(2) as usize, height.max(2) as usize).at(distance.rem_euclid(scene.motion.loop_length.max(1.0))), beyond: None, back: 0.0, walls: [walls_of(scene), None] }
    }
    /// How far from the middle of the path, either way, the walls stand `d` metres ahead of the
    /// walker (None where there are none): what stands beside the path stands within this.
    pub fn wall_x(&self, d: f32) -> Option<f32> {
        let d = d + self.back;
        match self.beyond { Some((zb, b)) if d >= zb => self.walls[1].map(|g| b.path_half_width(d) + g), _ => self.walls[0].map(|g| self.view.path_half_width(d) + g) }
    }
    /// The camera partway through a swing, the walker `distance` metres along `scene`'s loop.
    pub fn swinging(scene: &Scene, pose: &crate::scene::SwingPose, distance: f32, width: u32, height: u32) -> Camera {
        let v = View::posed(scene, &pose.camera, width.max(2) as usize, height.max(2) as usize, pose.turning);
        Camera { view: v.at((distance - pose.back).rem_euclid(scene.motion.loop_length.max(1.0))), beyond: None, back: pose.back, walls: [walls_of(scene), None] }
    }
    /// Where a point `x` metres right of the path centre, `y` up and `d` ahead of the walker
    /// appears (pixels from the top left); None behind the camera.
    pub fn project(&self, x: f32, y: f32, d: f32) -> Option<[f32; 2]> {
        let d = d + self.back;
        match self.beyond { Some((zb, b)) if d >= zb => b.world_to_px(x, y, d), _ => self.view.world_to_px(x, y, d) }
    }
    /// How far ahead of the walker the ground on screen row `row` is (None above the horizon).
    pub fn ground_distance(&self, row: f32) -> Option<f32> {
        let d = self.view.ground_distance(row);
        let d = match (self.beyond, d) { (Some((zb, b)), Some(d)) if d >= zb => b.ground_distance(row).map(|e| e.max(zb)), _ => d };
        d.map(|d| d - self.back)
    }
}

/// Where a crossing walk is (`CrossWalk::phase`).
enum Phase { Out(crate::scene::SwingPose), Cross, In(Option<crate::scene::SwingPose>) }

/// Walking through a crossing from one world into the next.
#[derive(Clone, Debug)]
pub struct CrossWalk {
    pub crossing: Crossing,
    /// Where along the first world the walk begins.
    pub start_a: f32,
    /// Where along the second world the walk ends.
    pub end_b: f32,
}

impl CrossWalk {
    /// A walk beginning `start_a` metres along the first world and ending `end_b` metres along the
    /// second (0 for an exported clip, so it ends on the second loop's frame 0).
    pub fn new(crossing: Crossing, start_a: f32, end_b: f32) -> CrossWalk {
        CrossWalk { crossing, start_a, end_b }
    }

    /// Metres from start to end: swinging out of the first scene's view, the crossing, swinging
    /// into the second's.
    pub fn length(&self) -> f32 { self.crossing.swing_out + self.cross_length() + self.crossing.swing_in }

    /// Metres of the crossing itself, looking down the path.
    fn cross_length(&self) -> f32 { self.crossing.approach + self.crossing.overshoot }

    /// At `travel` metres since the start: the boundary's distance ahead of the camera, and the
    /// camera's place along each world.
    pub fn at(&self, travel: f32) -> (f32, f32, f32) {
        // Measured from the end, so the last frame lands exactly on `end_b`.
        (self.crossing.swing_out + self.crossing.approach - travel, self.start_a + travel, self.end_b + (travel - self.length()))
    }

    /// How far the next world has emerged (0 at the start: the frame is the first world's own) and
    /// how far the frame has settled into the second world alone (1 at the end).
    pub fn fades(&self, travel: f32) -> (f32, f32) {
        let fade_in = (self.crossing.approach * 0.3).clamp(1.0, 8.0);
        let (t, len) = (travel - self.crossing.swing_out, self.cross_length());
        (smooth(0.0, fade_in, t), smooth(len - 1.0, len, t))
    }

    /// Where the walk is at `travel` metres: swinging out of the first scene's view (`a`), crossing
    /// looking down the path, or swinging into the second's (`b`, walking at its pace; None once
    /// it has come round, or with no swing past the crossing).
    fn phase(&self, a: &Scene, b: &Scene, travel: f32) -> Phase {
        use crate::scene::SwingView;
        let c = &self.crossing;
        let sw = &c.swing;
        let into = travel - c.swing_out - self.cross_length();
        if travel < c.swing_out {
            // (No hold on the way out: it turns as the walk goes on.)
            let u = (travel / c.swing_out).clamp(0.0, 1.0);
            let ahead = SwingView { yaw: 0.0, side: 0.0, ..SwingView::of(&a.camera) };
            Phase::Out(sw.between(&SwingView::of(&a.camera), &ahead, &a.camera, u))
        } else if c.swing_in > 0.0 && into >= 0.0 {
            let t = into / b.motion.speed.max(0.01) - sw.hold.max(0.0);
            let u = (t / sw.seconds.max(1e-3)).clamp(0.0, 1.0);
            let ahead = SwingView { yaw: 0.0, side: 0.0, ..SwingView::of(&b.camera) };
            Phase::In((u < 1.0).then(|| sw.between(&ahead, &SwingView::of(&b.camera), &b.camera, u)))
        } else { Phase::Cross }
    }

    /// The camera at `travel` metres, for a `width` x `height` frame.
    pub fn camera(&self, a: &Scene, b: &Scene, travel: f32, width: u32, height: u32) -> Camera {
        let (zb, sa, sb) = self.at(travel);
        match self.phase(a, b, travel) {
            Phase::Out(p) => return Camera::swinging(a, &p, sa, width, height),
            Phase::In(Some(p)) => return Camera::swinging(b, &p, sb, width, height),
            Phase::In(None) => return Camera::plain(b, sb, width, height),
            Phase::Cross => {}
        }
        // Crossing, both looking down the path (when either scene looks from an angle).
        let ahead = |s: &Scene, turned: bool| if turned { crate::scene::transition::looking_ahead(s) } else { s.clone() };
        let (a2, b2) = (ahead(a, self.crossing.swing_out > 0.0), ahead(b, self.crossing.swing_in > 0.0));
        let pa = Camera::plain(&a2, sa, width, height).view;
        let pb = Camera::plain(&b2, sb, width, height).view;
        if zb <= 0.0 { Camera { view: pb, beyond: None, back: 0.0, walls: [walls_of(b), None] } } else { Camera { view: pa, beyond: Some((zb, pb)), back: 0.0, walls: [walls_of(a), walls_of(b)] } }
    }

    /// The frame at `travel` metres, with each world's clock.
    pub fn frame(&self, r: &mut WorldRenderer, a: Place, b: Place, travel: f32, time_a: f32, time_b: f32, opts: &RenderOptions) -> Image {
        let (zb, sa, sb) = self.at(travel);
        match self.phase(a.scene, b.scene, travel) {
            Phase::Out(p) => return a.alone(r, sa, time_a, &RenderOptions { swing: Some(p), ..opts.clone() }),
            Phase::In(Some(p)) => return b.alone(r, sb, time_b, &RenderOptions { swing: Some(p), ..opts.clone() }),
            Phase::In(None) => return b.alone(r, sb, time_b, opts),
            Phase::Cross => {}
        }
        // Crossing, both looking down the path (when either scene looks from an angle).
        let (a2, b2);
        let a = if self.crossing.swing_out > 0.0 { a2 = crate::scene::transition::looking_ahead(a.scene); Place { scene: &a2, ..a } } else { a };
        let b = if self.crossing.swing_in > 0.0 { b2 = crate::scene::transition::looking_ahead(b.scene); Place { scene: &b2, ..b } } else { b };
        let (emerge, settle) = self.fades(travel);
        if emerge <= 0.0 { return a.alone(r, sa, time_a, opts); }
        if settle >= 1.0 { return b.alone(r, sb, time_b, opts); }
        let worlds = [a.world(sa, time_a), b.world(sb, time_b)];
        let layout = Layout::Cross { crossing: &self.crossing, zb };
        if !(opts.pick || opts.depth || opts.stats) {
            return mixed_frame(r, opts, emerge, settle, |r, o| a.alone(r, sa, time_a, o), |r, o| r.render_layout(&worlds, layout, o), |r, o| b.alone(r, sb, time_b, o));
        }
        let mixed;
        let opts = if emerge < 1.0 || settle > 0.0 { mixed = r.unpipelined(opts); &mixed } else { opts };
        let mut img = r.render_layout(&worlds, layout, opts);
        if emerge < 1.0 {
            let mut alone = a.alone(r, sa, time_a, opts);
            mix_images(&mut alone, &img, emerge);
            img.rgba = alone.rgba;
        }
        if settle > 0.0 { mix_images(&mut img, &b.alone(r, sb, time_b, opts), settle); }
        img
    }
}

/// Walking up to a fork, choosing a branch and taking it into its world.
#[derive(Clone, Debug)]
pub struct ForkWalk {
    pub plan: ForkPlan,
    pub start_a: f32,
    /// Where along each branch world taking that branch ends (left, right).
    pub end: [f32; 2],
    /// The branch taken, and the travel at which it was chosen.
    pub chosen: Option<(Branch, f32)>,
    /// What shuts each branch (left, right), if anything: ways the game has barred.
    pub barred: [Option<crate::scene::transition::Barrier>; 2],
}

impl ForkWalk {
    /// A walk beginning `start_a` metres along the first world; taking a branch ends `end` metres
    /// along its world (0 for exported clips).
    pub fn new(plan: ForkPlan, start_a: f32, end: [f32; 2]) -> ForkWalk {
        ForkWalk { plan, start_a, end, chosen: None, barred: [None, None] }
    }

    pub fn length(&self) -> f32 { self.plan.approach + self.plan.overshoot }

    /// The latest travel at which a choice still has room to turn the camera before the junction.
    pub fn decide_by(&self) -> f32 { (self.plan.approach - self.plan.steer).max(0.0) }

    /// Take a branch at `travel` metres (a later choice turns more sharply; past the junction the
    /// default branch has already been taken and this does nothing).
    pub fn choose(&mut self, branch: Branch, travel: f32) {
        if self.chosen.is_none() { self.chosen = Some((branch, travel.min(self.decide_by()))); }
    }

    /// The branch being taken at `travel` (the default once the junction is close and nothing was chosen).
    pub fn branch(&self, travel: f32) -> Option<(Branch, f32)> {
        self.chosen.or_else(|| (travel >= self.decide_by()).then(|| (self.plan.default_branch, self.decide_by())))
    }

    /// The junction's distance ahead, the camera's place along the first world and each branch's
    /// world, and how far the camera has turned onto the branch taken (0..1).
    pub fn at(&self, travel: f32) -> (f32, f32, [f32; 2], f32) {
        let zj = self.plan.approach - travel;
        let steer = match self.branch(travel) {
            Some((_, at)) => smooth(at, at + self.plan.steer, travel),
            None => 0.0,
        };
        let back = travel - self.length();
        (zj, self.start_a + travel, [self.end[0] + back, self.end[1] + back], steer)
    }

    pub fn frame(&self, r: &mut WorldRenderer, a: Place, left: Place, right: Place, travel: f32, times: [f32; 3], opts: &RenderOptions) -> Image {
        let (zj, sa, sb, steer) = self.at(travel);
        let fade_in = (self.plan.approach * 0.3).clamp(1.0, 8.0);
        let emerge = smooth(0.0, fade_in, travel);
        let len = self.length();
        let settle = smooth(len - 1.0, len, travel);
        let taken = self.branch(travel).map(|(b, _)| b);
        if emerge <= 0.0 { return a.alone(r, sa, times[0], opts); }
        if let (true, Some(b)) = (settle >= 1.0, taken) {
            let (p, d, t) = match b { Branch::Left => (left, sb[0], times[1]), Branch::Right => (right, sb[1], times[2]) };
            return p.alone(r, d, t, opts);
        }
        let worlds = [a.world(sa, times[0]), left.world(sb[0], times[1]), right.world(sb[1], times[2])];
        let layout = Layout::Fork { fork: &self.plan, zb: zj, chosen: taken, steer, barred: &self.barred };
        if !(opts.pick || opts.depth || opts.stats) {
            let settle = if taken.is_some() { settle } else { 0.0 };
            let branch = |r: &mut WorldRenderer, o: &RenderOptions| match taken {
                Some(Branch::Left) => left.alone(r, sb[0], times[1], o),
                _ => right.alone(r, sb[1], times[2], o),
            };
            return mixed_frame(r, opts, emerge, settle, |r, o| a.alone(r, sa, times[0], o), |r, o| r.render_layout(&worlds, layout, o), branch);
        }
        let mixed;
        let opts = if emerge < 1.0 || (settle > 0.0 && taken.is_some()) { mixed = r.unpipelined(opts); &mixed } else { opts };
        let mut img = r.render_layout(&worlds, layout, opts);
        if emerge < 1.0 {
            let mut alone = a.alone(r, sa, times[0], opts);
            mix_images(&mut alone, &img, emerge);
            img.rgba = alone.rgba;
        }
        if let (true, Some(b)) = (settle > 0.0, taken) {
            let (p, d, t) = match b { Branch::Left => (left, sb[0], times[1]), Branch::Right => (right, sb[1], times[2]) };
            mix_images(&mut img, &p.alone(r, d, t, opts), settle);
        }
        img
    }
}

// ── Journeys ───────────────────────────────────────────────────────────────

use crate::scene::transition::{ForkChoice, Transition};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// A map of places a game walks through: each stop is a scene, and each says what comes next
/// (another stop through a transition, a fork where the player chooses, or the end). Scene names
/// are files relative to the journey file, or `preset:<name>`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Journey {
    pub version: u32,
    pub name: String,
    /// The stop the walk begins at.
    pub start: String,
    pub stops: BTreeMap<String, Stop>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Stop {
    /// Scene file (relative to the journey) or `preset:<name>`.
    pub scene: String,
    pub next: Next,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub enum Next {
    /// The last stop.
    #[default]
    End,
    /// On to another stop, through a transition.
    Go {
        to: String,
        #[serde(default)]
        transition: Transition,
    },
    /// A fork: the player chooses the left or the right stop.
    Fork {
        left: String,
        right: String,
        #[serde(default)]
        fork: ForkChoice,
    },
}

impl Journey {
    pub fn load(path: &Path) -> Result<Journey, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        std::fs::write(path, serde_json::to_string_pretty(self).map_err(|e| e.to_string())?).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// The scene a stop shows, and the folder its files are relative to.
    pub fn scene_of(&self, stop: &str, dir: Option<&Path>) -> Result<(Scene, Option<PathBuf>), String> {
        let st = self.stops.get(stop).ok_or_else(|| format!("no stop `{stop}`"))?;
        load_place(&st.scene, dir)
    }

    /// Everything wrong with the journey: unknown stops, missing scenes, stops nothing leads to.
    pub fn problems(&self, dir: Option<&Path>) -> Vec<String> {
        let mut out = Vec::new();
        if !self.stops.contains_key(&self.start) { out.push(format!("start `{}` is not a stop", self.start)); }
        let mut reached = std::collections::BTreeSet::new();
        let mut todo = vec![self.start.clone()];
        while let Some(s) = todo.pop() {
            if !reached.insert(s.clone()) { continue; }
            if let Some(st) = self.stops.get(&s) {
                match &st.next {
                    Next::End => {}
                    Next::Go { to, .. } => todo.push(to.clone()),
                    Next::Fork { left, right, .. } => { todo.push(left.clone()); todo.push(right.clone()); }
                }
            }
        }
        for (id, st) in &self.stops {
            if let Err(e) = load_place(&st.scene, dir) { out.push(format!("stop `{id}`: {e}")); }
            let targets: Vec<&String> = match &st.next { Next::End => vec![], Next::Go { to, .. } => vec![to], Next::Fork { left, right, .. } => vec![left, right] };
            for t in targets { if !self.stops.contains_key(t) { out.push(format!("stop `{id}` leads to `{t}`, which is not a stop")); } }
            if !reached.contains(id) { out.push(format!("stop `{id}` cannot be reached from the start")); }
        }
        out
    }
}

/// A scene by file (relative to `dir`) or `preset:<name>`.
pub fn load_place(name: &str, dir: Option<&Path>) -> Result<(Scene, Option<PathBuf>), String> {
    if let Some(p) = name.strip_prefix("preset:") {
        let norm = |s: &str| s.to_lowercase().replace([' ', '_', '-'], "");
        let f = crate::scene::presets::ALL.iter().find(|(n, _)| norm(n) == norm(p)).ok_or_else(|| format!("no preset `{p}`"))?;
        return Ok((f.1(), dir.map(Path::to_path_buf)));
    }
    let p = Path::new(name);
    let full = if p.is_absolute() { p.to_path_buf() } else { dir.map(|d| d.join(p)).unwrap_or_else(|| p.to_path_buf()) };
    let (scene, _) = crate::scene::load_file(&full)?;
    Ok((scene, full.parent().map(Path::to_path_buf)))
}
