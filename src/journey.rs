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

/// Mix `b` over `a` by `t` (0 = a).
fn mix_images(a: &mut Image, b: &Image, t: f32) {
    if t <= 0.0 || a.rgba.len() != b.rgba.len() { return; }
    for (x, y) in a.rgba.iter_mut().zip(&b.rgba) {
        *x = (*x as f32 + (*y as f32 - *x as f32) * t).round() as u8;
    }
}

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
}

impl Camera {
    /// The camera `distance` metres along `scene`'s loop, for a `width` x `height` frame.
    pub fn plain(scene: &Scene, distance: f32, width: u32, height: u32) -> Camera {
        Camera { view: View::new(scene, width.max(2) as usize, height.max(2) as usize).at(distance.rem_euclid(scene.motion.loop_length.max(1.0))), beyond: None }
    }
    /// Where a point `x` metres right of the path centre, `y` up and `d` ahead appears (pixels from
    /// the top left); None behind the camera.
    pub fn project(&self, x: f32, y: f32, d: f32) -> Option<[f32; 2]> {
        match self.beyond { Some((zb, b)) if d >= zb => b.world_to_px(x, y, d), _ => self.view.world_to_px(x, y, d) }
    }
    /// How far ahead the ground on screen row `row` is (None above the horizon).
    pub fn ground_distance(&self, row: f32) -> Option<f32> {
        let d = self.view.ground_distance(row);
        match (self.beyond, d) { (Some((zb, b)), Some(d)) if d >= zb => b.ground_distance(row).map(|e| e.max(zb)), _ => d }
    }
}

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

    /// Metres from start to end.
    pub fn length(&self) -> f32 { self.crossing.approach + self.crossing.overshoot }

    /// At `travel` metres since the start: the boundary's distance ahead of the camera, and the
    /// camera's place along each world.
    pub fn at(&self, travel: f32) -> (f32, f32, f32) {
        // Measured from the end, so the last frame lands exactly on `end_b`.
        (self.crossing.approach - travel, self.start_a + travel, self.end_b + (travel - self.length()))
    }

    /// How far the next world has emerged (0 at the start: the frame is the first world's own) and
    /// how far the frame has settled into the second world alone (1 at the end).
    pub fn fades(&self, travel: f32) -> (f32, f32) {
        let fade_in = (self.crossing.approach * 0.3).clamp(1.0, 8.0);
        let len = self.length();
        (smooth(0.0, fade_in, travel), smooth(len - 1.0, len, travel))
    }

    /// The camera at `travel` metres, for a `width` x `height` frame.
    pub fn camera(&self, a: &Scene, b: &Scene, travel: f32, width: u32, height: u32) -> Camera {
        let (zb, sa, sb) = self.at(travel);
        let pa = Camera::plain(a, sa, width, height).view;
        let pb = Camera::plain(b, sb, width, height).view;
        if zb <= 0.0 { Camera { view: pb, beyond: None } } else { Camera { view: pa, beyond: Some((zb, pb)) } }
    }

    /// The frame at `travel` metres, with each world's clock.
    pub fn frame(&self, r: &mut WorldRenderer, a: Place, b: Place, travel: f32, time_a: f32, time_b: f32, opts: &RenderOptions) -> Image {
        let (zb, sa, sb) = self.at(travel);
        let (emerge, settle) = self.fades(travel);
        if emerge <= 0.0 { return a.alone(r, sa, time_a, opts); }
        if settle >= 1.0 { return b.alone(r, sb, time_b, opts); }
        let worlds = [a.world(sa, time_a), b.world(sb, time_b)];
        let mut img = r.render_layout(&worlds, Layout::Cross { crossing: &self.crossing, zb }, opts);
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
}

impl ForkWalk {
    /// A walk beginning `start_a` metres along the first world; taking a branch ends `end` metres
    /// along its world (0 for exported clips).
    pub fn new(plan: ForkPlan, start_a: f32, end: [f32; 2]) -> ForkWalk {
        ForkWalk { plan, start_a, end, chosen: None }
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
        let mut img = r.render_layout(&worlds, Layout::Fork { fork: &self.plan, zb: zj, chosen: taken, steer }, opts);
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
