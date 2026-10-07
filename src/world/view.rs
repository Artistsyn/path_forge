//! The camera and path shape: the single projection everything is drawn and placed with.
//!
//! World coordinates are relative to the camera's spot on the path: `x` metres sideways from the
//! path centreline, `y` metres above the ground, `d` metres ahead. The path's bend and hill are
//! camera-relative curves (like classic pseudo-3D road games), so they stay put as you walk.
//! Camera space is x right, y up, z forward; `to_cam` maps world to camera space and `project`
//! maps camera space to pixels.

use crate::scene::Scene;
use serde::Serialize;

/// Closest distance anything is drawn at, metres.
pub const NEAR: f32 = 0.05;
/// Lateral offset at distance d is `bend * BEND_K * d²`.
pub const BEND_K: f32 = 0.012;
/// Vertical offset at distance d is `hill * HILL_K * d²`.
pub const HILL_K: f32 = 0.006;

#[derive(Clone, Copy, Debug, Serialize)]
pub struct View {
    pub width: usize,
    pub height: usize,
    /// Horizon row and centre column, pixels.
    pub horizon_px: f32,
    pub center_px: f32,
    /// Focal length, pixels.
    pub focal_px: f32,
    pub eye_height: f32,
    pub bend: f32,
    pub hill: f32,
    pub half_width: f32,
    pub flare: f32,
    /// Distance of the ground seen on the bottom row, metres (where `half_width` is measured).
    pub near_ground: f32,
    pub lens_curve: f32,
    /// Steps in the ground, and where along the loop the camera stands (see `at`).
    pub stairs: Option<StairProfile>,
    pub scroll: f32,
    /// Rows above and columns either side of the frame that are rendered but not shown (see
    /// `guarded`); 0 for the frame itself.
    #[serde(skip)]
    pub top: usize,
    #[serde(skip)]
    pub left: usize,
    /// A boundary ahead where this world gives way to another (a transition); None for one world.
    #[serde(skip)]
    pub split: Option<Split>,
    /// A branch of a fork: beyond `z0` metres ahead this world runs off sideways, `slope` metres
    /// per metre ahead (the camera turning onto it brings the slope to 0).
    #[serde(skip)]
    pub shear: Option<(f32, f32)>,
}

/// Where one world gives way to the next, as the camera sees it this frame. Path width and ground
/// height are one continuous function across it, so both worlds' geometry meets at the boundary.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Split {
    /// Camera-relative distance of the boundary, metres (negative once the camera has passed it).
    pub zb: f32,
    /// The path beyond the boundary.
    pub half_width_b: f32,
    pub flare_b: f32,
    /// Metres either side of the boundary over which the two path widths blend (0 = a clean step).
    pub taper: f32,
    /// Each world's stairs and where the camera stands along it.
    pub stairs_a: Option<StairProfile>,
    pub scroll_a: f32,
    pub stairs_b: Option<StairProfile>,
    pub scroll_b: f32,
}

impl Split {
    fn ground_a(&self, w: f32) -> f32 { self.stairs_a.map_or(0.0, |s| s.ground(w)) }
    fn ground_b(&self, w: f32) -> f32 { self.stairs_b.map_or(0.0, |s| s.ground(w)) }
    /// Height that lifts the second world's ground to meet the first's at the boundary.
    fn join(&self) -> f32 { self.ground_a(self.scroll_a + self.zb) - self.ground_b(self.scroll_b + self.zb) }
    /// Height of the ground `d` metres ahead relative to the ground the camera stands on.
    fn lift(&self, d: f32) -> f32 {
        let c = self.join();
        let g = if d < self.zb { self.ground_a(self.scroll_a + d) } else { self.ground_b(self.scroll_b + d) + c };
        let eye = if self.zb > 0.0 {
            self.stairs_a.map_or(0.0, |s| s.ramp(self.scroll_a))
        } else {
            self.stairs_b.map_or(0.0, |s| s.ramp(self.scroll_b)) + c
        };
        g - eye
    }
}

/// The ground height of `path.stairs` along the world: flights of `steps` risers `run` apart every
/// `period` metres, each flight adding `rise * steps` (negative when descending).
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct StairProfile { pub period: f32, pub run: f32, pub rise: f32, pub steps: u32, pub offset: f32 }

impl StairProfile {
    pub fn new(s: &crate::scene::Stairs, loop_len: f32) -> Option<StairProfile> {
        if !s.enabled || s.steps == 0 || s.rise.abs() < 1e-4 { return None; }
        let period = crate::world::render::snap_to_loop(s.spacing.max(0.5), loop_len.max(1.0));
        let run = s.run.clamp(0.05, 2.0);
        // A flight must fit inside its period, with at least one run of landing.
        let steps = s.steps.min(((period / run).floor() as u32).saturating_sub(1)).max(1);
        Some(StairProfile { period, run, rise: s.rise.abs().min(1.0) * if s.descending { -1.0 } else { 1.0 }, steps, offset: s.offset })
    }
    fn split(&self, w: f32) -> (f32, f32) {
        let y = w - self.offset;
        let k = (y / self.period).floor();
        (k, y - k * self.period)
    }
    /// Height of the step surface at world distance `w`. A riser at `w` itself counts as climbed.
    pub fn ground(&self, w: f32) -> f32 {
        let (k, u) = self.split(w);
        let n = self.steps as f32;
        k * n * self.rise + self.rise * ((u / self.run).floor() + 1.0).min(n)
    }
    /// Height a walker's eye follows: a straight ramp over each flight, level on the landing.
    pub fn ramp(&self, w: f32) -> f32 {
        let (k, u) = self.split(w);
        let n = self.steps as f32;
        k * n * self.rise + n * self.rise * (u / (n * self.run)).clamp(0.0, 1.0)
    }
    /// World positions of the risers in [a, b).
    pub fn risers(&self, a: f32, b: f32) -> Vec<f32> {
        let mut out = Vec::new();
        let k0 = ((a - self.offset) / self.period).floor() as i64;
        let k1 = ((b - self.offset) / self.period).floor() as i64;
        for k in k0..=k1 {
            for j in 0..self.steps {
                let e = self.offset + k as f32 * self.period + j as f32 * self.run;
                if e >= a && e < b { out.push(e); }
            }
        }
        out
    }
    /// Distance from the last riser behind `w` (the start of the step `w` is on).
    pub fn since_riser(&self, w: f32) -> f32 {
        let (_, u) = self.split(w);
        let flight = self.steps as f32 * self.run;
        if u < flight { u.rem_euclid(self.run) } else { u - flight + self.run }
    }
    /// Distance from `w` to the next riser ahead.
    pub fn to_next_riser(&self, w: f32) -> f32 {
        let (_, u) = self.split(w);
        let flight = self.steps as f32 * self.run;
        if u < flight - self.run { self.run - u.rem_euclid(self.run) } else { self.period - u }
    }
}

impl View {
    pub fn new(scene: &Scene, width: usize, height: usize) -> View {
        let h = height.max(2) as f32;
        let horizon_px = (scene.camera.horizon.clamp(0.02, 0.98) * h).round();
        let zoom = scene.camera.zoom.clamp(0.1, 8.0);
        let focal_px = (h - horizon_px).max(1.0) * zoom;
        let eye_height = scene.camera.eye_height.clamp(0.05, 50.0);
        View {
            width, height, horizon_px, center_px: width as f32 * 0.5, focal_px, eye_height,
            bend: scene.path.bend.clamp(-2.0, 2.0), hill: scene.path.hill.clamp(-2.0, 2.0),
            half_width: scene.path.half_width.clamp(0.05, 50.0), flare: scene.path.flare.clamp(0.0, 0.95),
            near_ground: eye_height * zoom, lens_curve: scene.camera.lens_curve.clamp(-1.0, 1.0),
            stairs: StairProfile::new(&scene.path.stairs, scene.motion.loop_length), scroll: 0.0, top: 0, left: 0, split: None, shear: None,
        }
    }

    /// The view from `scroll` metres along the loop (only stairs depend on it).
    pub fn at(mut self, scroll: f32) -> View { self.scroll = scroll; self }

    /// The same camera on a larger canvas: `top` more rows above the frame and `left` more columns
    /// either side, so what lies just outside the frame is rendered too (reflections need it).
    /// The frame is the window starting at (`left`, `top`).
    pub fn guarded(mut self, left: usize, top: usize) -> View {
        self.width += 2 * left;
        self.height += top;
        self.horizon_px += top as f32;
        self.center_px += left as f32;
        self.top += top;
        self.left += left;
        self
    }

    /// The frame inside a guarded view.
    pub fn frame(mut self) -> View {
        self.width -= 2 * self.left;
        self.height -= self.top;
        self.horizon_px -= self.top as f32;
        self.center_px -= self.left as f32;
        self.top = 0;
        self.left = 0;
        self
    }

    /// Height of the ground `d` metres ahead relative to the ground the camera stands on.
    #[inline]
    pub fn lift(&self, d: f32) -> f32 {
        if let Some(sp) = &self.split { return sp.lift(d); }
        match &self.stairs { Some(s) => s.ground(self.scroll + d) - s.ramp(self.scroll), None => 0.0 }
    }

    #[inline]
    pub fn bend_x(&self, d: f32) -> f32 { self.bend * BEND_K * d * d }
    #[inline]
    pub fn hill_y(&self, d: f32) -> f32 { self.hill * HILL_K * d * d }
    /// Sideways offset of a fork branch `d` metres ahead.
    #[inline]
    pub fn shear_x(&self, d: f32) -> f32 { match self.shear { Some((z0, k)) => k * (d - z0).max(0.0), None => 0.0 } }

    /// Path half-width at distance `d`, metres.
    #[inline]
    pub fn path_half_width(&self, d: f32) -> f32 {
        let a = self.flared(self.half_width, self.flare, d);
        let Some(sp) = &self.split else { return a };
        let t = if sp.taper > 0.0 {
            let u = ((d - sp.zb + sp.taper) / (2.0 * sp.taper)).clamp(0.0, 1.0);
            u * u * (3.0 - 2.0 * u)
        } else if d < sp.zb { 0.0 } else { 1.0 };
        if t <= 0.0 { return a; }
        a + (self.flared(sp.half_width_b, sp.flare_b, d) - a) * t
    }

    #[inline]
    fn flared(&self, half_width: f32, flare: f32, d: f32) -> f32 {
        if flare <= 0.0 { return half_width; }
        half_width * (d.max(self.near_ground) / self.near_ground).powf(flare)
    }

    /// World (x sideways, y up, d ahead) to camera space.
    #[inline]
    pub fn to_cam(&self, x: f32, y: f32, d: f32) -> [f32; 3] {
        [x + self.bend_x(d) + self.shear_x(d), y - self.eye_height + self.hill_y(d) + self.lift(d), d]
    }

    /// Camera space to pixels; None behind the near plane.
    #[inline]
    pub fn project(&self, c: [f32; 3]) -> Option<[f32; 2]> {
        if c[2] < NEAR * 0.5 { return None; }
        Some([self.center_px + self.focal_px * c[0] / c[2], self.horizon_px - self.focal_px * c[1] / c[2]])
    }

    /// World point straight to pixels.
    #[inline]
    pub fn world_to_px(&self, x: f32, y: f32, d: f32) -> Option<[f32; 2]> { self.project(self.to_cam(x, y, d)) }

    /// How far ahead the ground seen on screen row `row` is, in metres (None above the horizon).
    pub fn ground_distance(&self, row: f32) -> Option<f32> { crate::mcp::ground_distance(self, row) }

    /// Pixels per metre for something standing at distance `z`.
    #[inline]
    pub fn px_per_m(&self, z: f32) -> f32 { self.focal_px / z.max(NEAR) }
}
