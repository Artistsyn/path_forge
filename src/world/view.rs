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
    /// The path's winding (`path.winding`): up to four waves, each (amplitude in metres, wave
    /// number per metre, phase where the loop starts, how much the view turns with it). See
    /// `bend_x`.
    pub wind: [[f32; 4]; 4],
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
    /// The camera looking to the side (`camera.moves`): how far everything slides across the
    /// frame, pixels (the sky with the rest), and the furthest it does over the loop.
    #[serde(skip)]
    pub pan_px: f32,
    #[serde(skip)]
    pub pan_max: f32,
    /// The camera turned (`camera.yaw`): its cosine and sine, and where it stands sideways
    /// (`camera.side`, metres). `turned` is false for a camera looking straight down the path,
    /// which then sees exactly what it always has.
    #[serde(skip)]
    pub yaw_c: f32,
    #[serde(skip)]
    pub yaw_s: f32,
    #[serde(skip)]
    pub cam_x: f32,
    #[serde(skip)]
    pub turned: bool,
    /// How far the camera's own side of the world is cut away for a side view (0 none, 1 gone:
    /// see `near_fade`), which side that is (-1 left, 1 right), and the cut's distance past the
    /// path's edge (`camera.cutaway`). A side view has 1, other views 0; a camera swinging round
    /// (`CameraMoves::swing_in`) dissolves the near side between them.
    #[serde(skip)]
    pub side_k: f32,
    #[serde(skip)]
    pub near: f32,
    #[serde(skip)]
    pub cutaway: f32,
    /// How far props that face the path once the camera turns (`Facing::PathWhenTurned`) have
    /// come round from facing the camera (0) to facing the path (1).
    #[serde(skip)]
    pub face_k: f32,
    /// The sky staying put in the world as the camera turns (`camera.world_sky`): how far it
    /// slides across the frame, in frame widths.
    #[serde(skip)]
    pub sky_turn: f32,
}

/// How far a camera's turn has gone in what it changes besides the angle (see `View::posed`):
/// the near side cut away, the path straightened, and buildings come round to face the path.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Turning { pub side_k: f32, pub straight: f32, pub face_k: f32 }

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
    /// Whether this view's world lies before the boundary: with no taper it keeps its own path
    /// width on the boundary itself, so its walls meet the end wall there without slanting in.
    pub before: bool,
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

/// A scene's winding as waves (the first two slots; a crossing puts the next scene's in the
/// others): a whole number of bends to the loop, and a second wave one bend more to the loop
/// that makes them uneven.
fn winding(scene: &Scene) -> [[f32; 4]; 4] {
    let w = &scene.path.winding;
    let mut out = [[0.0; 4]; 4];
    let amp = w.amplitude.clamp(0.0, 20.0);
    if !w.enabled || amp <= 0.0 { return out; }
    let tau = std::f32::consts::TAU;
    let len = scene.motion.loop_length.max(1.0);
    let n = (len / w.length.clamp(4.0, 1000.0)).round().max(1.0);
    let (turn, uneven) = (w.turn.clamp(0.0, 1.0), w.irregular.clamp(0.0, 1.0));
    out[0] = [amp, tau * n / len, tau * w.phase, turn];
    if uneven > 0.0 { out[1] = [0.45 * amp * uneven, tau * (n + 1.0) / len, tau * (w.phase * 1.7 + 0.31), turn]; }
    out
}

/// A view's sideways bend with its per-frame parts worked out (`View::bender`): the same numbers
/// as `bend_x`, `shear_x` and `to_cam` give, without each pixel taking the sine and cosine of
/// every wave's phase again.
#[derive(Clone, Copy, Debug)]
pub struct Bend {
    bk: f32,
    /// Each wave that is there: amplitude, wave number, phase now, its sine and cosine, turn * k.
    wind: [[f32; 6]; 4],
    n: usize,
    shear: Option<(f32, f32)>,
    /// cam_x, yaw_c, yaw_s when the camera has turned.
    turned: Option<[f32; 3]>,
}

impl Bend {
    /// `View::bend_x`.
    #[inline]
    pub fn x(&self, d: f32) -> f32 {
        let mut x = self.bk * d * d;
        for &[a, k, p, sp, cp, tk] in &self.wind[..self.n] { x += a * ((k * d + p).sin() - sp - tk * d * cp); }
        x
    }
    /// `View::shear_x`.
    #[inline]
    pub fn shear(&self, d: f32) -> f32 { match self.shear { Some((z0, k)) => k * (d - z0).max(0.0), None => 0.0 } }
    /// Its numbers as geom.wgsl's `bend_of` and `cam_xz_of` read them: the curve, the count of
    /// waves and each wave's six, then the shear (on, z0, k) and the turn (on, cam_x, yaw cos, sin).
    pub fn gpu_words(&self) -> [f32; 33] {
        let mut out = [0.0; 33];
        (out[0], out[1]) = (self.bk, self.n as f32);
        for (k, wave) in self.wind.iter().enumerate() { out[2 + 6 * k..8 + 6 * k].copy_from_slice(wave); }
        if let Some((z0, k)) = self.shear { out[26..29].copy_from_slice(&[1.0, z0, k]); }
        if let Some(turn) = self.turned { out[29] = 1.0; out[30..33].copy_from_slice(&turn); }
        out
    }
    /// `View::to_cam`'s x and z (its height is not needed here).
    #[inline]
    pub fn cam_xz(&self, x: f32, d: f32) -> [f32; 2] {
        let cx = x + self.x(d) + self.shear(d);
        let Some([cam_x, yc, ys]) = self.turned else { return [cx, d] };
        let px = cx - cam_x;
        [yc * px - ys * d, ys * px + yc * d]
    }
}

impl View {
    pub fn new(scene: &Scene, width: usize, height: usize) -> View { View::with_camera(scene, &scene.camera, width, height) }

    /// The view of `scene` through `cam` in place of its own camera (a camera move between views).
    pub fn with_camera(scene: &Scene, cam: &crate::scene::Camera, width: usize, height: usize) -> View {
        let side = if cam.side_view() { 1.0 } else { 0.0 };
        View::posed(scene, cam, width, height, Turning { side_k: side, straight: side, face_k: if cam.turned() { 1.0 } else { 0.0 } })
    }

    /// `with_camera` with the cut, the straightening and the turn of buildings as given.
    pub fn posed(scene: &Scene, cam: &crate::scene::Camera, width: usize, height: usize, k: Turning) -> View {
        let h = height.max(2) as f32;
        let horizon_px = (cam.horizon.clamp(0.02, 0.98) * h).round();
        let zoom = cam.zoom.clamp(0.1, 8.0);
        let focal_px = (h - horizon_px).max(1.0) * zoom;
        let eye_height = cam.eye_height.clamp(0.05, 50.0);
        let yaw = cam.yaw.clamp(-90.0, 90.0).to_radians();
        let turned = cam.turned();
        // A side view straightens the path: its bends would swing it toward and away from the camera.
        let keep = 1.0 - k.straight.clamp(0.0, 1.0);
        let mut wind = winding(scene);
        for w in wind.iter_mut() { w[0] *= keep; }
        let cam_x = if turned { cam.side.clamp(-200.0, 200.0) } else { 0.0 };
        View {
            width, height, horizon_px, center_px: width as f32 * 0.5, focal_px, eye_height,
            bend: scene.path.bend.clamp(-2.0, 2.0) * keep, wind, hill: scene.path.hill.clamp(-2.0, 2.0),
            half_width: scene.path.half_width.clamp(0.05, 50.0), flare: scene.path.flare.clamp(0.0, 0.95),
            near_ground: eye_height * zoom, lens_curve: cam.lens_curve.clamp(-1.0, 1.0),
            stairs: StairProfile::new(&scene.path.stairs, scene.motion.loop_length), scroll: 0.0, top: 0, left: 0, split: None, shear: None, pan_px: 0.0, pan_max: 0.0,
            yaw_c: yaw.cos(), yaw_s: yaw.sin(), cam_x, turned,
            side_k: k.side_k.clamp(0.0, 1.0), near: if cam_x != 0.0 { cam_x.signum() } else if yaw != 0.0 { -yaw.signum() } else { 1.0 },
            cutaway: cam.cutaway.max(0.0), face_k: k.face_k.clamp(0.0, 1.0),
            sky_turn: if cam.world_sky { -yaw / 2.4 } else { 0.0 },
        }
    }

    /// How far something standing `x` metres from the middle of the path has dissolved because it
    /// is on the camera's side of a side view's cut (0 not at all, 1 gone).
    pub fn near_fade(&self, x: f32) -> f32 {
        if self.side_k <= 0.0 || x * self.near <= self.half_width + self.cutaway { 0.0 } else { self.side_k }
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

    /// How far sideways the path's centre is `d` metres ahead: the steady bend, and the
    /// winding, measured from where the camera stands and (as far as it turns with the path)
    /// from the way the path heads there.
    #[inline]
    pub fn bend_x(&self, d: f32) -> f32 {
        let mut x = self.bend * BEND_K * d * d;
        for [a, k, p0, turn] in self.wind {
            if a != 0.0 {
                let p = k * self.scroll + p0;
                x += a * ((k * d + p).sin() - p.sin() - turn * k * d * p.cos());
            }
        }
        x
    }

    /// `bend_x`, `shear_x` and `to_cam`'s ground plane with what stays the same across the frame
    /// worked out once (the same numbers, bit for bit): for loops over every pixel.
    pub fn bender(&self) -> Bend {
        let mut b = Bend { bk: self.bend * BEND_K, wind: [[0.0; 6]; 4], n: 0, shear: self.shear, turned: self.turned.then_some([self.cam_x, self.yaw_c, self.yaw_s]) };
        for [a, k, p0, turn] in self.wind {
            if a != 0.0 {
                let p = k * self.scroll + p0;
                // (turn * k * d * cos p is ((turn * k) * d) * cos p.)
                b.wind[b.n] = [a, k, p, p.sin(), p.cos(), turn * k];
                b.n += 1;
            }
        }
        b
    }

    /// How far the camera has turned to follow the winding path, as the tangent of the angle:
    /// the far end of the path stands this far to the left of straight ahead, per metre of
    /// distance (`bend_x(d) / d` far away).
    pub fn turn_tan(&self) -> f32 {
        self.wind.iter().filter(|t| t[0] != 0.0).map(|&[a, k, p0, turn]| a * turn * k * (k * self.scroll + p0).cos()).sum()
    }

    /// How far sideways the sky stands from where the scene puts it, pixels. The sky is
    /// infinitely far, so as the camera turns with the path all of it slides across the frame
    /// together, the way the far end of the path does.
    pub fn sky_dx(&self) -> f32 { -self.focal_px * self.turn_tan() + self.pan_px + self.sky_turn * (self.width - 2 * self.left) as f32 }

    /// The furthest the sky slides either way over the loop, pixels (0 when the camera never turns).
    pub fn sky_swing(&self) -> f32 {
        self.focal_px * self.wind.iter().map(|&[a, k, _, turn]| (a * turn * k).abs()).sum::<f32>() + self.pan_max + self.sky_turn.abs() * (self.width - 2 * self.left) as f32
    }

    /// The winding's waves as they stand now (the phase at the camera in place of the loop's
    /// start), for the GPU.
    pub fn wind_now(&self) -> [[f32; 4]; 4] { self.wind.map(|[a, k, p0, turn]| [a, k, k * self.scroll + p0, turn]) }

    /// The shortest of the winding's waves, metres (None when the path does not wind).
    pub fn wind_length(&self) -> Option<f32> {
        self.wind.iter().filter(|t| t[0] != 0.0 && t[1] > 0.0).map(|t| std::f32::consts::TAU / t[1]).reduce(f32::min)
    }
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
        } else if d < sp.zb || (sp.before && d <= sp.zb) { 0.0 } else { 1.0 };
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
        let c = [x + self.bend_x(d) + self.shear_x(d), y - self.eye_height + self.hill_y(d) + self.lift(d), d];
        if !self.turned { return c; }
        // Turned: from where the camera stands, round the vertical by the yaw.
        let px = c[0] - self.cam_x;
        [self.yaw_c * px - self.yaw_s * c[2], c[1], self.yaw_s * px + self.yaw_c * c[2]]
    }

    /// A direction the camera sees, back in the world (x, y, d): `rot_dir` undone.
    #[inline]
    pub fn unrot_dir(&self, v: [f32; 3]) -> [f32; 3] {
        if !self.turned { return v; }
        [self.yaw_c * v[0] + self.yaw_s * v[2], v[1], -self.yaw_s * v[0] + self.yaw_c * v[2]]
    }

    /// A direction in the world (x, y, d) as the camera sees it (turned with it).
    #[inline]
    pub fn rot_dir(&self, v: [f32; 3]) -> [f32; 3] {
        if !self.turned { return v; }
        [self.yaw_c * v[0] - self.yaw_s * v[2], v[1], self.yaw_s * v[0] + self.yaw_c * v[2]]
    }

    /// The ground point (x, d) moved `m` metres toward the camera (away with `m` negative): back
    /// along the path for a camera looking down it, straight at it for a turned one.
    #[inline]
    pub fn toward_camera(&self, x: f32, d: f32, m: f32) -> (f32, f32) {
        if !self.turned { return (x, d - m); }
        let (dx, dd) = (self.cam_x - x, -d);
        let n = (dx * dx + dd * dd).sqrt().max(1e-3);
        (x + m * dx / n, d + m * dd / n)
    }

    /// How far ahead of the camera (its depth) a point on the ground at (x, d) is: `d` itself for
    /// a camera looking down the path.
    #[inline]
    pub fn depth_of(&self, x: f32, d: f32) -> f32 { if self.turned { self.to_cam(x, 0.0, d)[2] } else { d } }

    /// The stretch of the path (metres ahead of where the camera stands, from .. to) and how far
    /// out to either side a turned camera can see out to `far`, with `margin` more all round.
    pub fn footprint(&self, far: f32, margin: f32) -> (f32, f32, f32) {
        if !self.turned { return (0.0, far, 0.0); }
        // Half the field of view across, a little wider for the edges of tall things.
        let half = ((self.width as f32 * 0.5 + self.pan_px.abs()) / self.focal_px).atan() + 0.12;
        let yaw = self.yaw_s.atan2(self.yaw_c);
        let (mut lo, mut hi, mut xs) = (0.0f32, 0.0f32, self.cam_x.abs());
        for k in 0..=16 {
            let a = yaw - half + 2.0 * half * k as f32 / 16.0;
            let (dd, xx) = (far * a.cos(), self.cam_x + far * a.sin());
            lo = lo.min(dd);
            hi = hi.max(dd);
            xs = xs.max(xx.abs());
        }
        (lo - margin, hi + margin, xs + margin)
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

#[cfg(test)]
mod winding_tests {
    use super::*;

    #[test]
    fn the_sky_slides_as_far_as_the_far_end_of_the_path() {
        let mut s = Scene::default();
        (s.motion.loop_length, s.path.bend) = (48.0, 0.0);
        assert_eq!(View::new(&s, 480, 854).at(7.0).sky_dx(), 0.0);
        s.path.winding = crate::scene::Winding { enabled: true, amplitude: 2.0, length: 20.0, turn: 0.7, irregular: 0.5, phase: 0.2 };
        let v = View::new(&s, 480, 854);
        for scroll in [0.0, 5.0, 17.0, 31.0] {
            let v = v.at(scroll);
            let far = 1.0e6;
            let end = v.focal_px * v.bend_x(far) / far;
            assert!((v.sky_dx() - end).abs() < 0.05, "{scroll}: sky {} path end {end}", v.sky_dx());
            assert!(v.sky_dx().abs() <= v.sky_swing() + 1e-3);
        }
        assert!((v.at(0.0).sky_dx() - v.at(48.0).sky_dx()).abs() < 0.02);
        assert!(v.sky_swing() > 100.0);
    }

    #[test]
    fn a_winding_path_starts_under_the_camera_and_comes_round_with_the_loop() {
        let mut s = Scene::default();
        s.motion.loop_length = 48.0;
        s.path.winding = crate::scene::Winding { enabled: true, amplitude: 2.0, length: 20.0, turn: 1.0, irregular: 0.5, phase: 0.2 };
        let v = View::new(&s, 90, 160);
        // Two whole bends fit the loop (24 m each, the nearest to the 20 asked for); the wave that
        // makes them uneven has three, 16 m each.
        assert!((v.wind_length().unwrap() - 16.0).abs() < 1e-3, "{:?}", v.wind_length());
        for scroll in [0.0, 7.3, 31.0] {
            let at = v.at(scroll);
            // The path is under the camera, and with the view turned fully it leaves straight ahead.
            assert!(at.bend_x(0.0).abs() < 1e-6);
            assert!(at.bend_x(0.5).abs() < 0.02, "{}", at.bend_x(0.5));
            // It does wind further on, and is the same a whole loop later.
            assert!((0..60).any(|d| at.bend_x(d as f32).abs() > 1.0));
            for d in [3.0, 17.0, 40.0] { assert!((at.bend_x(d) - v.at(scroll + 48.0).bend_x(d)).abs() < 2e-3); }
        }
        s.path.winding.enabled = false;
        assert_eq!(View::new(&s, 90, 160).wind_length(), None);
    }
}
