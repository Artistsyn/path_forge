//! Frame renderer for v3 scenes.
//!
//! Order: geometry -> G-buffer -> instances (props, fixtures, lights) -> shadow masks ->
//! deferred lighting -> sky -> billboards -> flames and glows -> particles -> post and style.
//! Every time-varying input is a function of the loop phase with whole-number cycles, and every
//! spatial repeat is snapped to divide the loop, so `distance` and `distance + loop_length`
//! render identically.

use super::palette::{self, PaletteLut};
use super::raster::{self, id, GPixel, Tri, Vert};
use super::sprites::{fixture_body, Sprite, SpriteCache};
use super::texture::{rgb_lin, Texture, TextureCache};
use super::view::{View, NEAR};
use crate::scene::*;
use crate::scene::transition::{Branch, Crossing, ForkPlan, SkyRule, StyleFrom};
use super::view::Split;
use rayon::prelude::*;
use std::f32::consts::TAU;
use std::path::PathBuf;
use std::sync::Arc;

mod weather;
mod splat;
mod space;
mod wildlife;
mod blackhole;
mod water;
mod lens;
mod clouds;
mod cover;
use splat::{Shape, Splat};

/// Which parts of the world to draw, for layered exports.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layers {
    pub sky: bool,
    pub ground: bool,
    pub walls: bool,
    pub props: bool,
    pub fixtures: bool,
    pub particles: bool,
    pub post: bool,
}
impl Default for Layers {
    fn default() -> Self { Self { sky: true, ground: true, walls: true, props: true, fixtures: true, particles: true, post: true } }
}

#[derive(Clone, Default)]
pub struct RenderOptions {
    /// Output size; None uses the scene canvas.
    pub size: Option<(u32, u32)>,
    /// Folder that relative sprite paths are resolved against.
    pub base_dir: Option<PathBuf>,
    pub layers: Layers,
    /// Use this palette instead of the scene's (lets an export share one palette across frames).
    pub palette: Option<Arc<PaletteLut>>,
    /// Measure what the frame shows (coverage and brightness per kind of surface).
    pub stats: bool,
    /// Also return each output pixel's distance from the camera (metres; infinity for sky).
    pub depth: bool,
    /// Time in seconds for flames, particles, twinkle and drift. None: the time it takes to walk
    /// `distance` at motion.speed (what a loop plays). Set it to keep things alive while the camera stops.
    pub time: Option<f32>,
    /// Also return which part of the scene each pixel shows (see `Pick`), for clicking to select.
    pub pick: bool,
    /// Seconds since play began, for the camera's flight in and swing in (`camera.moves.fly_in`,
    /// `swing_in`); None plays neither (a loop, which must close).
    pub intro: Option<f32>,
    /// The camera partway through a swing a game or a transition is making, in place of the
    /// scene's own (and of a swing in).
    pub swing: Option<crate::scene::SwingPose>,
}

/// Which part of the scene each pixel of a frame shows, at the frame's render resolution (the
/// canvas divided by the pixel size): a surface (`PICK_PATH`...) or a list item (`pick_item`).
#[derive(Clone, Debug)]
pub struct Pick { pub width: usize, pub height: usize, pub ids: Vec<u16> }

pub const PICK_PATH: u16 = 1;
pub const PICK_VERGE: u16 = 2;
pub const PICK_WALLS: u16 = 3;
pub const PICK_CEILING: u16 = 4;
pub const PICK_SKY: u16 = 5;
const PICK_FIXTURES: u16 = 1 << 12;
const PICK_PROPS: u16 = 2 << 12;
const PICK_SET_PIECES: u16 = 3 << 12;
const PICK_COMPANIONS: u16 = 4 << 12;

/// The scene list and index a pick id names: ("fixtures" | "props" | "set_pieces" | "companions", index).
pub fn pick_item(id: u16) -> Option<(&'static str, usize)> {
    let list = match id >> 12 { 1 => "fixtures", 2 => "props", 3 => "set_pieces", 4 => "companions", _ => return None };
    Some((list, (id & 0x0FFF) as usize))
}

/// The scene section a surface pick id names ("path", "verge", "walls", "ceiling", "sky").
pub fn pick_section(id: u16) -> Option<&'static str> {
    Some(match id { PICK_PATH => "path", PICK_VERGE => "verge", PICK_WALLS => "walls", PICK_CEILING => "ceiling", PICK_SKY => "sky", _ => return None })
}

#[derive(Clone)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
    pub stats: Option<FrameStats>,
    /// Camera-space depth per output pixel in metres (f32::INFINITY where only sky shows), when asked.
    pub depth: Option<Vec<f32>>,
    pub pick: Option<Pick>,
}

/// Share of the frame each kind of thing covers, and how bright it is there (0..255 luma).
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct FrameStats {
    pub coverage: std::collections::BTreeMap<String, f32>,
    pub luma: std::collections::BTreeMap<String, f32>,
    pub mean_luma: f32,
    pub lights: usize,
    pub billboards: usize,
}

#[derive(Default)]
pub struct WorldRenderer {
    textures: TextureCache,
    sprites: SpriteCache,
    defs: super::propdefs::PropDefs,
    palette: Option<(Palette, Arc<PaletteLut>)>,
    /// Auto palette for one scene (keyed by the scene's JSON), chosen from frames across the loop.
    auto_palette: Option<(String, Arc<PaletteLut>)>,
    /// Lightness levels of one scene's loop (2nd and 98th percentile luma), for ramp palettes.
    levels: Option<(String, (f32, f32))>,
    /// Colour grades by name or resolved .cube path; None remembers a grade that failed to load.
    grades: std::collections::HashMap<String, Option<Arc<super::looks::Grade>>>,
    /// Average colour of scenes over their loop (keyed by the scene's JSON), for transitions.
    means: std::collections::HashMap<String, [f32; 3]>,
    /// Time each stage of a frame took, summed over frames while `profile` is on (`pf bench --stages`).
    pub profile: bool,
    pub stages: Vec<(&'static str, f64)>,
    /// GPU passes, when a device was given (`set_gpu`); every pass without one runs on the CPU.
    #[cfg(feature = "gpu")]
    gpu: Option<Arc<super::gpu::Gpu>>,
    /// An hour of the day forced on every scene with a sky (`set_hour`): a game's or a live
    /// wallpaper's clock.
    hour: Option<f32>,
}

/// Times the stages of one frame when profiling is on; costs one branch per stage when it is off.
struct Prof { t: Option<std::time::Instant>, out: Vec<(&'static str, f64)> }
impl Prof {
    fn new(on: bool) -> Prof { Prof { t: on.then(std::time::Instant::now), out: Vec::new() } }
    fn mark(&mut self, name: &'static str) {
        if let Some(t) = self.t.as_mut() {
            let now = std::time::Instant::now();
            self.out.push((name, (now - *t).as_secs_f64() * 1000.0));
            *t = now;
        }
    }
}

// ── Loop-safe helpers ──────────────────────────────────────────────────────

/// Snap a repeat length so a whole number of repeats fits in the loop.
pub fn snap_to_loop(len: f32, loop_len: f32) -> f32 {
    let n = (loop_len / len.max(0.01)).round().max(1.0);
    loop_len / n
}

/// Whole cycles per loop for something that should happen about `per_second` times a second.
fn cycles(per_second: f32, loop_seconds: f32) -> f32 { (per_second * loop_seconds).round().max(1.0) }

#[inline]
fn hash(seed: u32, k: i64) -> u32 {
    let mut h = (k as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (seed as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    h ^= h >> 31;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 29;
    (h >> 16) as u32
}
#[inline]
fn hf(seed: u32, k: i64) -> f32 { (hash(seed, k) >> 8) as f32 / 16_777_216.0 }

/// Smooth periodic noise along the path (period divides the loop), 0..1.
fn path_noise(dw: f32, loop_len: f32, cell: f32, seed: u32) -> f32 {
    let c = snap_to_loop(cell, loop_len);
    let n = (loop_len / c).round() as i64;
    let t = dw / c;
    let i = t.floor() as i64;
    let f = t - i as f32;
    let f = f * f * (3.0 - 2.0 * f);
    let a = hf(seed, i.rem_euclid(n));
    let b = hf(seed, (i + 1).rem_euclid(n));
    a + (b - a) * f
}

/// Smooth minimum of two signed distances: their union with the corners rounded over k.
fn smin(a: f32, b: f32, k: f32) -> f32 {
    let h = (k - (a - b).abs()).max(0.0) / k;
    a.min(b) - h * h * k * 0.25
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn add3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] { [a[0] + b[0], a[1] + b[1], a[2] + b[2]] }
fn sub3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] { [a[0] - b[0], a[1] - b[1], a[2] - b[2]] }
fn scale3(a: [f32; 3], k: f32) -> [f32; 3] { [a[0] * k, a[1] * k, a[2] * k] }
fn mul3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] { [a[0] * b[0], a[1] * b[1], a[2] * b[2]] }
fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] { [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t] }
fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 { a[0] * b[0] + a[1] * b[1] + a[2] * b[2] }

// ── Frame context ──────────────────────────────────────────────────────────

struct PointLight {
    pos: [f32; 3],
    color: [f32; 3],
    radius: f32,
}

struct DirLight {
    dir: [f32; 3],
    color: [f32; 3],
}

struct Billboard {
    z: f32,
    world: [f32; 3],
    height: f32,
    sprite: Arc<Sprite>,
    flip: bool,
    nearest: bool,
    id: u8,
    emissive: f32,
    shadow: f32,
    /// The point light this prop gives off, which does not light its own card.
    own_light: Option<usize>,
    /// The scene list item it comes from, for picking (see `pick_item`).
    source: u16,
    /// The world it stands in (see `GPixel::realm`).
    realm: u8,
    /// How far its top leans sideways in the wind now, metres (its base stays put).
    sway: f32,
    /// Its own colour cast, over the light that falls on it (see `PropLayer::tint_var`).
    tint: [f32; 3],
    /// How its picture stirs in the wind now.
    wind: CardWind,
    /// Standing in the world facing one way (a building's front along the path, `Facing::Path`)
    /// rather than turned toward the camera: drawn in perspective.
    panel: Option<Panel>,
    /// Or a cylinder (`PropDef::tubes`): drawn true, ray by ray.
    tube: Option<Tube>,
    /// How far it has dissolved (0..1), as a swinging camera cuts its side of the world away: a
    /// share of its pixels left out, in an ordered pattern (`dissolved`).
    fade: f32,
}

/// A cylinder standing in the world (`PropDef::tubes`): its axis from `a` to `b` (x, y, d), `r`
/// round, its bark and its sawn ends' colours (linear).
#[derive(Clone, Copy, Debug)]
struct Tube { a: [f32; 3], b: [f32; 3], r: f32, bark: [f32; 3], end: [f32; 3] }

/// A tube as the camera sees it, for `tube_at`: its axis from `a` along the unit `u` for `len`
/// (camera space), `r` round, its colours, the light falling on it from each side (+x, +y, +z,
/// -x, -y, -z of the camera), and the camera's 1 / focal length and centre in pixels.
#[derive(Clone, Copy, Debug)]
struct TubeCard { a: [f32; 3], u: [f32; 3], len: f32, r: f32, bark: [f32; 3], end: [f32; 3], cube: [[f32; 3]; 6], kf: f32, cx: f32, cy: f32 }

/// A tube at pixel centre (fx, fy): its depth there, how much of the pixel it covers, and its
/// colour as lit; None where it is not. Its side is found as where the pixel's ray comes within
/// `r` of its axis (a half pixel beyond for the soft edge of its outline), its ends as the ray
/// crossing the discs that close it. `cards.wgsl` repeats it step for step.
fn tube_at(t: &TubeCard, fx: f32, fy: f32) -> Option<(f32, f32, [f32; 3])> {
    let d = [(fx - t.cx) * t.kf, (t.cy - fy) * t.kf, 1.0];
    let (a, u, r) = (t.a, t.u, t.r);
    let du = dot3(d, u);
    let au = dot3(a, u);
    // Across the axis: the ray d t and the axis point a, less their parts along it.
    let dp = [d[0] - du * u[0], d[1] - du * u[1], d[2] - du * u[2]];
    let ap = [a[0] - au * u[0], a[1] - au * u[1], a[2] - au * u[2]];
    let qa = dot3(dp, dp);
    let qb = dot3(dp, ap);
    let aa = dot3(ap, ap);
    let mut best: Option<(f32, f32, [f32; 3])> = None;
    if qa > 1e-12 {
        let tc = qb / qa;
        let rho = (aa - qb * tc).max(0.0).sqrt();
        let pix = tc.max(NEAR) * t.kf;
        let cover = ((r - rho) / pix + 0.5).clamp(0.0, 1.0);
        if cover > 0.0 {
            // Its near side where the ray meets it, or its outline where it only grazes it.
            let th = if rho < r { tc - ((r * r - rho * rho) / qa).sqrt() } else { tc };
            let s = th * du - au;
            if th > NEAR && (0.0..=t.len).contains(&s) {
                let n = [th * d[0] - a[0] - s * u[0], th * d[1] - a[1] - s * u[1], th * d[2] - a[2] - s * u[2]];
                let nl = dot3(n, n).sqrt().max(1e-9);
                let n = [n[0] / nl, n[1] / nl, n[2] / nl];
                // Bark, shaded round as a log's painted picture is (by the height on it its
                // surface faces), lit as a wall facing out from it is.
                let k = log_shade(0.5 + 0.5 * n[1]);
                let h = (n[0] * n[0] + n[2] * n[2]).sqrt();
                let out = if h > 1e-3 { [n[0] / h, 0.0, n[2] / h] } else { [0.0, 0.0, -1.0] };
                let lit = mul3(scale3(t.bark, k), tube_light(t, out));
                best = Some((th, cover, lit));
            }
        }
    }
    // Its sawn ends: rings round the pith, a rim of bark.
    if du.abs() > 1e-6 {
        for (sc, sg) in [(0.0f32, -1.0f32), (t.len, 1.0)] {
            let tq = (au + sc) / du;
            if tq <= NEAR || best.is_some_and(|b| b.0 <= tq) { continue; }
            let p = [tq * d[0] - a[0] - sc * u[0], tq * d[1] - a[1] - sc * u[1], tq * d[2] - a[2] - sc * u[2]];
            let q = dot3(p, p).sqrt() / r;
            if q > 1.0 { continue; }
            let col = if q > 0.86 { scale3(t.bark, 0.7) } else if q < 0.07 { scale3(t.end, 0.62) } else { scale3(t.end, 0.86 + 0.14 * (26.0 * q).cos()) };
            let n = [sg * u[0], sg * u[1], sg * u[2]];
            best = Some((tq, 1.0, mul3(col, tube_light(t, n))));
        }
    }
    best
}

/// How a round log is shaded across its height, from its underside (0) to its top (1): the
/// kit's painted logs use the same curve (`roundness` in tools/kit_buildings.py).
fn log_shade(v: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    (0.25 + 0.75 * (std::f32::consts::PI * v).sin().max(0.0).sqrt()) * (0.8 + 0.4 * v)
}

/// The light on a tube's surface turned along `n` (camera space), from its six sides.
fn tube_light(t: &TubeCard, n: [f32; 3]) -> [f32; 3] {
    let mut l = [0.0f32; 3];
    for (i, &c) in n.iter().enumerate() {
        let side = if c >= 0.0 { t.cube[i] } else { t.cube[i + 3] };
        let w = c * c;
        for j in 0..3 { l[j] += w * side[j]; }
    }
    l
}

/// A card standing in the world (x, y, d): its picture's top left corner is at `tl`, and its
/// width and height run along `across` and `down`, so any flat face (a wall, a roof's slope). It
/// is lit as a face turned to the camera. `strip` draws only one column of the picture (u)
/// stretched across it: the plain end walls of a prop with depth.
#[derive(Clone, Copy, Debug)]
struct Panel { tl: [f32; 3], across: [f32; 3], down: [f32; 3], strip: Option<f32> }

/// The wind in a card's picture: the base and trunk stay where they are while the rest shifts by
/// a field of waves running downwind. Lengths are shares of the card's width (`v`, of its
/// height); `cards.wgsl` repeats `at`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct CardWind {
    /// The waves' reach sideways and up and down, and a steady trail downwind.
    amp: f32,
    v: f32,
    lean: f32,
    /// How fast the waves change down the card: 1 for leaves, less for strands that swing whole.
    kv: f32,
    t1: f32,
    t2: f32,
    /// The stirring is whole at height `c0` (0 the card's top, 1 its base) and gone by `c1`.
    c0: f32,
    c1: f32,
    /// Half the width of the trunk, which stands still from `foot` down; the stillness fades out
    /// gently to the sides and upwards, so nothing near it is wrenched.
    trunk: f32,
    foot: f32,
}

impl CardWind {
    /// How far the picture has moved at (u, v) of the card.
    fn at(&self, u: f32, vv: f32) -> (f32, f32) {
        let mut w = smoothstep(self.c1, self.c0, vv);
        if self.trunk > 0.0 { w *= 1.0 - (1.0 - smoothstep(self.trunk, 4.0 * self.trunk, (u - 0.5).abs())) * smoothstep(self.foot - 0.45, self.foot, vv); }
        let h = self.kv * vv;
        let (a, b) = (TAU * (1.6 * u + 1.1 * h) - self.t1, TAU * (4.3 * u - 2.7 * h) - self.t2);
        let c = TAU * (11.0 * u + 7.0 * h) - self.t1 - self.t2;
        (w * (self.lean + self.amp * (0.55 * a.sin() + 0.3 * b.sin() + 0.15 * c.sin())), w * self.v * (b + 1.3).sin())
    }
}

struct Flame {
    world: [f32; 3],
    /// Only the halo: the flame is in the fixture's own image (its mask says where).
    halo_only: bool,
    size: f32,
    colors: Option<[Rgb; 4]>,
    glow: [f32; 3],
    flicker: f32,
    /// Where its two wavers are (radians), each a whole number of times round per loop.
    phase: [f32; 2],
    /// Embers rising from it (torches, braziers, campfires), and how its ember stream is seeded.
    embers: u8,
    seed: u32,
    realm: u8,
}

/// Everything about the frame that shading needs, computed once.
struct Ctx<'a> {
    scene: &'a Scene,
    view: View,
    loop_len: f32,
    /// Phase of time in the loop's period (loop_seconds): drives everything that moves on its own.
    tphase: f32,
    scroll: f32,
    far: f32,
    /// A turned camera (View::turned): the stretch of the path it can see (metres from where it
    /// stands; behind it too) and how far out to either side.
    d_lo: f32,
    d_hi: f32,
    x_reach: f32,
    path_tex: Arc<Texture>,
    verge_tex: Arc<Texture>,
    wall_tex: Arc<Texture>,
    ceil_tex: Arc<Texture>,
    path_tile: f32,
    /// The path as a waterway: its current, foam and what floats on it.
    water: Option<water::WaterK>,
    /// What grows on the ground (`ground_cover`).
    cover: Vec<cover::CoverK>,
    verge_tile: f32,
    wall_tile: f32,
    ceil_tile: f32,
    /// Bridges: where they are, their deck and what lies below.
    bridge: Option<Spans>,
    /// Forks: branch paths, or side passages between walls.
    fork: Option<Forks>,
    deck_tex: Arc<Texture>,
    bottom_tex: Arc<Texture>,
    deck_tile: f32,
    bottom_tile: f32,
    ambient: [f32; 3],
    sky_lights: Vec<DirLight>,
    lights: Vec<PointLight>,
    /// Smoke rising from props this frame: where it leaves them (path space), how it looks, and
    /// a seed for its puffs.
    smokes: Vec<([f32; 3], crate::scene::PropSmoke, u32)>,
    /// Wires of strings of lights: close points along them (path space).
    wire_pts: Vec<[f32; 3]>,
    /// Waterfalls in view: (x of the water's middle, top height, d, width, index into scene.waterfalls).
    falls: Vec<([f32; 3], f32, usize)>,
    fog: Option<([f32; 3], f32)>,
    /// Colour fog fades to (also used by fog banks when the even fog is off).
    fog_col: [f32; 3],
    void_lin: [f32; 3],
    /// Which world this is in a frame with several (0 alone).
    realm: u8,
    /// Where the worlds of the frame meet, if there are several.
    bounds: Option<&'a Bounds>,
    /// The face around a threshold's opening (texture, tile) and the opening's shape.
    facade: Option<(Arc<Texture>, f32)>,
    opening: Option<Opening>,
    /// Light falling through a threshold's opening from the other world.
    portal: Option<Portal>,
    /// The other world's air between the camera and the boundary (for the world beyond it).
    front: Option<FrontAir>,
    /// Past this distance the ground is the verge between a fork's branches, not the path.
    verge_beyond: Option<f32>,
    /// What the eye's adaptation multiplies this world's light by (1 alone).
    gain: f32,
    /// What the weather is doing in this world this frame.
    wx: weather::Wx,
}

/// Where the worlds of a frame meet: each world's part of the view as convex pieces in camera
/// space, and the edges across which two worlds mix in patches instead of meeting at a line.
pub(crate) struct Bounds {
    realms: Vec<Vec<Vec<raster::Half>>>,
    edges: Vec<Edge>,
    /// Where along the next world the camera stands, so patches stay put on the ground.
    anchor: f32,
    seed: u32,
}

/// A line where two worlds mix: `line` gives signed metres from it (world `a` on the negative
/// side, `b` on the positive), over `width` either side, only inside the `within` half-planes.
struct Edge { line: raster::Half, width: f32, a: u8, b: u8, within: Vec<raster::Half> }

impl Bounds {
    /// The next world begins `zb` metres ahead, mixing with this one over `blend` metres.
    fn cross(zb: f32, blend: f32, anchor: f32, seed: u32) -> Bounds {
        let edges = if blend > 0.0 { vec![Edge { line: (0.0, 1.0, -zb), width: blend, a: 0, b: 1, within: vec![] }] } else { vec![] };
        Bounds { realms: vec![vec![vec![(0.0, -1.0, zb)]], vec![vec![(0.0, 1.0, -zb)]]], edges, anchor, seed }
    }

    /// A fork `zb` metres ahead: branch centres run off at `slopes[1]` and `slopes[2]` (sideways
    /// metres per metre ahead, the first world's ground at `slopes[0]`), with that ground left
    /// between the branches for `wedge` metres past where they part.
    /// `taken`: the branch being taken (1 or 2) and how far the camera has turned onto it; the two
    /// branch worlds then split along that branch's far edge instead of down the middle, so the
    /// shared start of the branches becomes the chosen world as the camera turns.
    #[allow(clippy::too_many_arguments)]
    fn fork(zb: f32, slopes: [f32; 3], hw: [f32; 3], wedge: f32, blend: f32, taken: Option<(usize, f32)>, anchor: f32, seed: u32) -> Bounds {
        let m = 0.6;
        let (sl, sr) = (slopes[1], slopes[2]);
        // The split between the branch worlds: x = sd (z - zb) + cd.
        let (sd, cd) = {
            // Down the middle; once a branch is taken it starts at that branch's far edge (so the
            // shared start of the branches is the chosen world) and still runs off down the middle.
            let edge = match taken { Some((1, _)) => hw[1] + m, Some((2, _)) => -(hw[2] + m), _ => 0.0 };
            (0.5 * (sl + sr), edge * taken.map_or(0.0, |(_, t)| t.clamp(0.0, 1.0)))
        };
        // x <= eL(z) = sl (z - zb) + hwL + m, and x >= eR(z) = sr (z - zb) - hwR - m.
        let in_left = (-1.0, sl, -sl * zb + hw[1] + m);
        let in_right = (1.0, -sr, sr * zb + hw[2] + m);
        let left_of_div = (-1.0, sd, -sd * zb + cd);
        let right_of_div = (1.0, -sd, sd * zb - cd);
        let past = (0.0, 1.0, -zb);
        let apex = zb + (hw[1] + hw[2] + 2.0 * m) / (sr - sl).max(1e-3);
        let end = apex + wedge;
        let before_end = (0.0, -1.0, end);
        let after_end = (0.0, 1.0, -end);
        let neg = |h: raster::Half| (-h.0, -h.1, -h.2);
        // Past the junction: the first world's ground between the branches (the wedge) first, and
        // everything else left or right of the split. Pieces are disjoint and cover everything.
        let (out_l, out_r) = (neg(in_left), neg(in_right));
        let side = |d: raster::Half| vec![
            vec![past, d, in_left],
            vec![past, d, out_l, in_right],
            vec![past, d, out_l, out_r, after_end],
        ];
        let realms = vec![
            vec![vec![(0.0, -1.0, zb)], vec![past, out_l, out_r, before_end]],
            side(left_of_div),
            side(right_of_div),
        ];
        let norm = |h: raster::Half| { let l = (h.0 * h.0 + h.1 * h.1).sqrt().max(1e-6); (h.0 / l, h.1 / l, h.2 / l) };
        let mut edges = Vec::new();
        if blend > 0.0 {
            edges.push(Edge { line: past, width: blend, a: 0, b: 1, within: vec![left_of_div] });
            edges.push(Edge { line: past, width: blend, a: 0, b: 2, within: vec![right_of_div] });
            edges.push(Edge { line: norm(right_of_div), width: blend * 0.6, a: 1, b: 2, within: vec![past] });
        }
        edges.push(Edge { line: norm(neg(in_left)), width: 2.5, a: 1, b: 0, within: vec![past, before_end] });
        edges.push(Edge { line: norm(in_right), width: 2.5, a: 0, b: 2, within: vec![past, before_end] });
        Bounds { realms, edges, anchor, seed }
    }

    /// Which world's region a camera-space point (x, z) is in.
    fn region_of(&self, x: f32, z: f32) -> u8 {
        for (r, pieces) in self.realms.iter().enumerate() {
            if pieces.iter().any(|p| p.iter().all(|h| h.0 * x + h.1 * z + h.2 >= 0.0)) { return r as u8; }
        }
        0
    }

    /// Which world shows at camera-space ground point (x, z), patches included.
    fn realm_at(&self, x: f32, z: f32) -> u8 { self.realm_mix(x, z).0 }

    /// `realm_at`, plus at the edge of a patch the other world and how much of it shows (0..0.5),
    /// so patch edges are soft instead of cut out.
    fn realm_mix(&self, x: f32, z: f32) -> (u8, u8, f32) {
        let r = self.region_of(x, z);
        for e in &self.edges {
            if r != e.a && r != e.b { continue; }
            if !e.within.iter().all(|h| h.0 * x + h.1 * z + h.2 >= 0.0) { continue; }
            let s = e.line.0 * x + e.line.1 * z + e.line.2;
            if s.abs() >= e.width { continue; }
            let w = z + self.anchor;
            // Large patches with ragged edges: three octaves.
            let n = super::looks::value_noise(x * 0.35 + self.seed as f32 * 0.37, w * 0.35) * 0.55
                + super::looks::value_noise(x * 1.1 + 31.0, w * 1.1 + self.seed as f32) * 0.3
                + super::looks::value_noise(x * 3.7 + 7.0, w * 3.7 - 11.0) * 0.15;
            let d = s / e.width * 0.5 + 0.5 - n;
            let (main, other) = if d > 0.0 { (e.b, e.a) } else { (e.a, e.b) };
            return (main, other, 0.5 * (1.0 - smoothstep(0.0, 0.07, d.abs())));
        }
        (r, r, 0.0)
    }
}

/// The opening in a threshold: a door or a cave mouth in a face (a hillside, an end wall).
#[derive(Clone, Copy, Debug)]
struct Opening {
    /// Half width and height of the opening before its rim is roughened, metres.
    hw: f32,
    top: f32,
    /// 0 flat top, 1 a full arch.
    arch: f32,
    /// Raggedness of the rim, 0..1.
    rim: f32,
    seed: u32,
    /// The face around it: half width and height, and how much its top follows a hill (0..1).
    outer_hw: f32,
    outer_top: f32,
    hill: f32,
}

/// Smooth noise along a line, -1..1.
fn noise1(seed: u32, x: f32) -> f32 {
    let i = x.floor() as i64;
    let f = x - i as f32;
    let f = f * f * (3.0 - 2.0 * f);
    let (a, b) = (hf(seed, i), hf(seed, i + 1));
    (a + (b - a) * f) * 2.0 - 1.0
}

impl Opening {
    fn half_width(&self) -> f32 { self.hw * (1.0 - 0.1 * self.rim) }
    /// Height of the opening above x (0 outside it).
    fn top_at(&self, x: f32) -> f32 {
        let hw = self.half_width();
        if x.abs() >= hw { return 0.0; }
        let u = x / hw;
        let arch = (1.0 - u * u).max(0.0).powf(0.45);
        let t = self.top * ((1.0 - self.arch) + self.arch * arch);
        let rough = 1.0 + self.rim * (0.08 * noise1(self.seed ^ 0x51, x * 2.2) + 0.04 * noise1(self.seed ^ 0x52, x * 7.0));
        t * rough
    }
    /// Height of the face above x.
    fn outer_at(&self, x: f32) -> f32 {
        if self.hill <= 0.0 { return self.outer_top; }
        let u = (x / self.outer_hw.max(1.0)).abs().min(1.0);
        let hillside = self.outer_top * (0.4 + 0.6 * (1.0 - u * u).sqrt());
        let t = self.outer_top + (hillside - self.outer_top) * self.hill;
        t * (1.0 + self.hill * (0.1 * noise1(self.seed ^ 0x61, x * 0.3) + 0.04 * noise1(self.seed ^ 0x62, x * 1.3)))
    }
    /// Metres from a point on the face to the opening's rim.
    fn rim_distance(&self, x: f32, y: f32) -> f32 {
        let hw = self.half_width();
        if x.abs() < hw { return (y - self.top_at(x)).max(0.0); }
        let edge_top = self.top_at(x.signum() * (hw - 1e-3));
        let dy = (y - edge_top).max(0.0);
        ((x.abs() - hw).powi(2) + dy * dy).sqrt()
    }
}

/// A threshold's opening seen as a light: the other world's average light, coming through it.
#[derive(Clone, Copy, Debug)]
struct Portal {
    /// Its corners in camera space.
    corners: [[f32; 3]; 4],
    radiance: [f32; 3],
    zb: f32,
    /// Lights what is in front of the boundary (true) or beyond it (false).
    front: bool,
}

/// The first world's air between the camera and the boundary, which the second world is seen through.
#[derive(Clone, Copy, Debug)]
struct FrontAir { zb: f32, fog: Option<([f32; 3], f32)>, fog_col: [f32; 3], gain: f32 }

/// Light a polygon of uniform radiance 1 gives a surface at `p` with normal `n` (Lambert's formula
/// for a polygon: the cosine-weighted share of the hemisphere it covers, 0..1).
fn polygon_light(p: [f32; 3], n: [f32; 3], corners: &[[f32; 3]; 4]) -> f32 {
    let unit = |v: [f32; 3]| { let l = dot3(v, v).sqrt().max(1e-6); [v[0] / l, v[1] / l, v[2] / l] };
    let mut sum = 0.0;
    for i in 0..4 {
        let a = unit(sub3(corners[i], p));
        let b = unit(sub3(corners[(i + 1) % 4], p));
        let th = dot3(a, b).clamp(-1.0, 1.0).acos();
        let c = [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
        let l = dot3(c, c).sqrt();
        if l > 1e-6 { sum += th * dot3(c, n) / l; }
    }
    (sum / TAU).abs().min(1.0)
}

/// The face around a threshold's opening, as triangles facing the camera just before the boundary.
fn build_facade(view: &View, op: &Opening, zb: f32) -> Vec<Tri> {
    let z = zb - 0.01;
    if z <= NEAR { return Vec::new(); }
    let hw = op.half_width();
    // Columns: fine around the rim, coarser out across the face.
    let mut xs = vec![0.0f32];
    let mut x = 0.0f32;
    while x < op.outer_hw {
        let step = if (x - hw).abs() < 0.4 { 0.025 } else if x < hw { 0.06 } else { 0.08 * (1.0 + (x - hw - 0.4).max(0.0) * 0.25) };
        let nx = (x + step).min(op.outer_hw);
        if x < hw && nx > hw { xs.push(hw); }
        x = nx;
        xs.push(x);
    }
    let mut all: Vec<f32> = xs.iter().rev().map(|x| -x).collect();
    all.extend(xs.iter().skip(1));
    let vert = |x: f32, y: f32| Vert { cam: view.to_cam(x, y, z), world: [x, y, z] };
    // Columns beside the opening are cut at the height of its edge, so every column shares the
    // exact vertices of its neighbours (a vertex in the middle of a neighbour's edge leaves cracks).
    let edge = op.top_at(hw - 1e-4);
    let mut tris = Vec::new();
    for pair in all.windows(2) {
        let (x0, x1) = (pair[0], pair[1]);
        let inside = (0.5 * (x0 + x1)).abs() < hw;
        let (u0, u1) = (op.outer_at(x0), op.outer_at(x1));
        if inside {
            let lo = |x: f32| op.top_at(x.clamp(-hw + 1e-4, hw - 1e-4));
            let (l0, l1) = (lo(x0), lo(x1));
            if u0 <= l0 && u1 <= l1 { continue; }
            raster::quad(&mut tris, [vert(x0, l0.min(u0)), vert(x1, l1.min(u1)), vert(x1, u1), vert(x0, u0)], id::FACADE);
        } else {
            let (c0, c1) = (edge.min(u0), edge.min(u1));
            if c0 > 0.0 || c1 > 0.0 { raster::quad(&mut tris, [vert(x0, 0.0), vert(x1, 0.0), vert(x1, c1), vert(x0, c0)], id::FACADE); }
            if u0 > c0 || u1 > c1 { raster::quad(&mut tris, [vert(x0, c0), vert(x1, c1), vert(x1, u1), vert(x0, u0)], id::FACADE); }
        }
    }
    tris
}

impl<'a> Ctx<'a> {
    /// Whether this world shows at world point (x, y, d) in a frame with several.
    fn owns(&self, x: f32, y: f32, d: f32) -> bool {
        match self.bounds {
            None => true,
            Some(b) => { let c = self.view.to_cam(x, y, d); b.realm_at(c[0], c[2]) == self.realm }
        }
    }

    fn wall_x(&self, d: f32) -> f32 { self.view.path_half_width(d) + self.scene.walls.gap.max(0.0) }

    /// Whether ground at (x, d) is a fork: a branch path on open ground, or a side passage's floor.
    /// With the split style, it is ground the branch roads pave that the main road alone would not.
    fn on_fork(&self, x: f32, d: f32) -> bool {
        let Some(f) = &self.fork else { return false };
        if self.scene.walls.enabled { return x.abs() > self.wall_x(d) - 1e-3; }
        if f.split { let (m, _, u) = self.split_field(x, d); return u < 0.0 && u < m; }
        f.on_branch(x, self.scroll + d, self.view.path_half_width(d))
    }

    /// The split style's ground: (main road, branch roads, both together) as signed distances in
    /// metres, the last a smooth union so the roads meet in rounded corners.
    fn split_field(&self, x: f32, d: f32) -> (f32, f32, f32) {
        let f = self.fork.as_ref().unwrap();
        let m = x.abs() - self.path_edge(d);
        let b = f.branch_sd(x, self.scroll + d, self.view.path_half_width(d));
        (m, b, smin(m, b, f.fillet()))
    }

    /// The darkening toward the road's edges (path.edge_dark) where a split road's edges run: the
    /// main road's falloff over the outer half of its width, carried round the branches.
    fn split_edge_dark(&self, x: f32, d: f32) -> f32 {
        let f = self.fork.as_ref().unwrap();
        let (m, b, _) = self.split_field(x, d);
        let edge = self.path_edge(d).max(0.05);
        let (rm, rb) = (0.5 * edge, 0.5 * f.hw);
        let n = smin(m / rm, b / rb, f.fillet() / rm.min(rb));
        1.0 - self.scene.path.edge_dark.clamp(0.0, 1.0) * (1.0 - smoothstep(0.0, 1.0, -n))
    }

    /// Whether `d` metres ahead is on a bridge (the ground beside the path has dropped away).
    fn on_bridge(&self, d: f32) -> bool { self.bridge.is_some_and(|b| b.contains(self.scroll + d)) }

    /// The stretch of the path to gather things over, metres from where the camera stands: as a
    /// gatherer reckons it for a camera looking down the path (`lo`..`hi`), or all a turned camera
    /// can see.
    fn span(&self, lo: f32, hi: f32) -> (f32, f32) { if self.view.turned { (self.d_lo, self.d_hi) } else { (lo, hi) } }

    /// Whether something standing `x` metres from the middle of the path is on the camera's side
    /// of the cut a side view makes (`camera.cutaway` past the path's edge), and so left out.
    fn cut_away(&self, x: f32) -> bool { self.view.near_fade(x) >= 1.0 }

    /// How far something at `x` has dissolved while a swinging camera cuts its side away (0 when
    /// it has not, or is gone altogether: see `cut_away`).
    fn fading(&self, x: f32) -> f32 { let k = self.view.near_fade(x); if k < 1.0 { k } else { 0.0 } }

    fn wall_top(&self) -> f32 {
        let w = &self.scene.walls;
        let mut top = if w.height > 0.0 { w.height } else { 1.0e4 };
        if self.scene.ceiling.enabled { top = top.min(self.scene.ceiling.height.max(0.2)); }
        top
    }

    /// Path edge at world distance (wobbles with edge noise).
    fn path_edge(&self, d: f32) -> f32 {
        let dw = d + self.scroll;
        let n = path_noise(dw, self.loop_len, 0.9, 101) * 0.65 + path_noise(dw, self.loop_len, 0.3, 202) * 0.35;
        self.view.path_half_width(d) + self.scene.path.edge_noise * (n * 2.0 - 1.0)
    }

    /// 0 when the walls or ceiling block light from `dir` at world point (x, y, d).
    fn sky_visibility(&self, x: f32, y: f32, d: f32, dir: [f32; 3], skip: u8) -> f32 {
        if self.scene.ceiling.enabled { return 0.0; }
        if !self.scene.walls.enabled { return 1.0; }
        let xw = self.wall_x(d);
        let top = self.wall_top();
        if dir[0] > 1e-4 && skip != id::WALL_R {
            let t = (xw - x) / dir[0];
            if t > 0.0 && y + dir[1] * t < top { return 0.0; }
        }
        if dir[0] < -1e-4 && skip != id::WALL_L {
            let t = (-xw - x) / dir[0];
            if t > 0.0 && y + dir[1] * t < top { return 0.0; }
        }
        1.0
    }

    /// Light arriving at a point with normal `n` (None = faces the camera, for billboards and particles).
    fn light_at(&self, x: f32, y: f32, d: f32, n: Option<[f32; 3]>, skip: u8, sun_mask: f32) -> [f32; 3] {
        self.light_except(x, y, d, n, skip, sun_mask, None)
    }

    /// `light_at`, leaving out one point light (a lamp does not light its own card evenly).
    #[allow(clippy::too_many_arguments)]
    fn light_except(&self, x: f32, y: f32, d: f32, n: Option<[f32; 3]>, skip: u8, sun_mask: f32, except: Option<usize>) -> [f32; 3] {
        self.light_among(x, y, d, n, skip, sun_mask, except, None)
    }

    /// `light_except`, looking only at the point lights in `only` when given: the ones that can
    /// reach this part of the screen (any other adds nothing, so the answer is the same).
    #[allow(clippy::too_many_arguments)]
    fn light_among(&self, x: f32, y: f32, d: f32, n: Option<[f32; 3]>, skip: u8, sun_mask: f32, except: Option<usize>, only: Option<&[u16]>) -> [f32; 3] {
        let mut l = self.ambient;
        for s in &self.sky_lights {
            let ndl = match n { Some(n) => dot3(n, s.dir).max(0.0), None => (0.45 - 0.35 * self.view.rot_dir(s.dir)[2]).clamp(0.15, 0.8) };
            if ndl <= 0.0 { continue; }
            let vis = self.sky_visibility(x, y, d, s.dir, skip) * sun_mask;
            if vis > 0.0 { l = add3(l, scale3(s.color, ndl * vis)); }
        }
        if !self.lights.is_empty() {
            let p = self.view.to_cam(x, y, d);
            // (Lights stand where the camera sees them: the normal is turned to match.)
            let n = n.map(|n| self.view.rot_dir(n));
            let mut add = |li: usize| {
                if Some(li) == except { return; }
                let pl = &self.lights[li];
                let v = [pl.pos[0] - p[0], pl.pos[1] - p[1], pl.pos[2] - p[2]];
                let d2 = dot3(v, v);
                let r2 = pl.radius * pl.radius;
                if d2 >= r2 { return; }
                let w = 1.0 - d2 / r2;
                let att = w * w / (1.0 + 0.3 * d2);
                let ndl = match n {
                    Some(n) => (dot3(n, v) / d2.sqrt().max(1e-4)).max(0.0) * 0.85 + 0.15,
                    None => 0.8,
                };
                l = add3(l, scale3(pl.color, att * ndl));
            };
            match only {
                Some(list) => for &li in list { add(li as usize) },
                None => for li in 0..self.lights.len() { add(li) },
            }
        }
        if let Some(pt) = &self.portal {
            let mut p = self.view.to_cam(x, y, d);
            if (p[2] < pt.zb) == pt.front {
                // Measured a little back from the opening: right at its plane the light would jump to
                // half a hemisphere on a sliver of jamb and draw a bright outline round the door.
                p[2] = if pt.front { p[2].min(pt.zb - 0.35) } else { p[2].max(pt.zb + 0.35) };
                let mid = scale3(add3(add3(pt.corners[0], pt.corners[1]), add3(pt.corners[2], pt.corners[3])), 0.25);
                let to = sub3(mid, p);
                let (nn, k) = match n {
                    Some(n) => (n, 1.0),
                    None => { let l = dot3(to, to).sqrt().max(1e-4); ([to[0] / l, to[1] / l, to[2] / l], 0.6) }
                };
                if dot3(nn, to) > 0.0 {
                    l = add3(l, scale3(pt.radiance, k * polygon_light(p, nn, &pt.corners)));
                }
            }
        }
        let bands = self.scene.light.bands;
        if bands >= 2 {
            let lum = 0.3 * l[0] + 0.55 * l[1] + 0.15 * l[2];
            if lum > 1e-5 {
                let q = (lum * bands as f32 * 0.6).round() / (bands as f32 * 0.6);
                l = scale3(l, q / lum);
            }
        }
        l
    }

    fn apply_fog(&self, c: [f32; 3], z: f32) -> [f32; 3] {
        let g = self.gain;
        let Some(fa) = self.front.filter(|f| f.zb > 0.0) else {
            let t = self.fog_t(z);
            return if t >= 1.0 { scale3(c, g) } else { mix3(scale3(self.fog_col, g), scale3(c, g), t) };
        };
        // Seen from the first world: this world's fog over the stretch beyond the boundary, then
        // the first world's air in front of it (each at its own world's adaptation).
        let near = z.min(fa.zb);
        let own = self.own_fog_t(z - near, near);
        let c = if own >= 1.0 { scale3(c, g) } else { mix3(scale3(self.fog_col, g), scale3(c, g), own) };
        let t = match fa.fog { Some((_, dist)) => (-near / dist).exp(), None => 1.0 };
        if t >= 1.0 { c } else { mix3(scale3(fa.fog_col, fa.gain), c, t) }
    }

    /// `apply_fog` as c * mul + add, for things drawn many pixels at a time.
    fn fog_affine(&self, z: f32) -> (f32, [f32; 3]) {
        let add = self.apply_fog([0.0; 3], z);
        (self.apply_fog([1.0; 3], z)[1] - add[1], add)
    }

    /// How much of a surface z metres ahead shows through the fog (1 = clear): the even fog plus
    /// whatever fog banks lie between the camera and it.
    fn fog_t(&self, z: f32) -> f32 {
        let Some(fa) = self.front.filter(|f| f.zb > 0.0) else { return self.own_fog_t(z, 0.0) };
        let near = z.min(fa.zb);
        let t = match fa.fog { Some((_, dist)) => (-near / dist).exp(), None => 1.0 };
        t * self.own_fog_t(z - near, near)
    }

    /// This world's fog over `len` metres starting `from` metres ahead.
    fn own_fog_t(&self, len: f32, from: f32) -> f32 {
        let even = match self.fog { Some((_, dist)) => (-len.min(1.0e4) / dist).exp(), None => 1.0 };
        let banks = if from > 0.0 { self.bank_t(from + len) / self.bank_t(from).max(1e-6) } else { self.bank_t(len) };
        even * banks
    }

    /// Transmittance through the fog banks alone, from the camera to z metres ahead.
    fn bank_t(&self, z: f32) -> f32 {
        bank_transmittance(&self.scene.weather.fog_banks, self.loop_len, self.scroll, z.clamp(0.0, self.far))
    }
}

/// One world taking part in a frame: a scene, where the camera stands in it, and its clock.
#[derive(Clone)]
pub struct WorldIn<'a> {
    pub scene: &'a Scene,
    /// Metres along this scene's path.
    pub distance: f32,
    /// Seconds on this scene's clock; None = the time it takes to walk `distance`.
    pub time: Option<f32>,
    /// Folder the scene's files are named relative to.
    pub base_dir: Option<PathBuf>,
}

/// How far through its loop's time a world's frame is (0..1), as `world_ctx` reckons it.
fn loop_phase(w: &WorldIn) -> f32 {
    let loop_len = w.scene.motion.loop_length.max(1.0);
    match w.time {
        Some(t) => (t / w.scene.motion.loop_seconds().max(1e-3)).rem_euclid(1.0),
        None => w.distance.rem_euclid(loop_len) / loop_len,
    }
}

/// How the worlds of a frame with more than one are laid out.
#[derive(Clone, Copy)]
pub enum Layout<'a> {
    /// The second world begins `zb` metres ahead (negative once the camera has passed it).
    Cross { crossing: &'a Crossing, zb: f32 },
    /// The path splits `zb` metres ahead into a left branch (world 1) and a right one (world 2).
    /// `chosen` is the branch taken, `steer` how far the camera has turned onto it (0..1).
    Fork { fork: &'a ForkPlan, zb: f32, chosen: Option<Branch>, steer: f32 },
}

/// How much of the sky each world contributes at a column of the frame.
#[derive(Clone)]
struct SkyW { base: Vec<f32>, fork: Option<ForkSky> }

/// A fork's sky: each branch world's sky over its side of the view (split at the angle the
/// branches part at), growing as the junction nears; the side not taken fades as the camera turns.
#[derive(Clone, Copy)]
struct ForkSky { center: f32, focal: f32, split: f32, near: f32, keep: [f32; 2] }

impl SkyW {
    fn at(&self, r: usize, x: usize) -> f32 {
        let Some(f) = &self.fork else { return self.base.get(r).copied().unwrap_or(0.0) };
        let a = (x as f32 + 0.5 - f.center) / f.focal;
        let wl = 1.0 - smoothstep(f.split - 0.07, f.split + 0.07, a);
        let (l, rr) = (wl * f.keep[0], (1.0 - wl) * f.keep[1]);
        let tot = l + rr;
        // Where only the faded side would show, the side kept fills in.
        let left = if tot > 1e-4 { l / tot } else if f.keep[0] >= f.keep[1] { 1.0 } else { 0.0 };
        match r { 0 => 1.0 - f.near, 1 => f.near * left, _ => f.near * (1.0 - left) }
    }
    /// Whether world `r` shows in the sky anywhere.
    fn any(&self, r: usize) -> bool {
        match &self.fork { None => self.base.get(r).copied().unwrap_or(0.0) > 0.001, Some(f) => if r == 0 { f.near < 0.999 } else { f.near > 0.001 && f.keep[r - 1] > 0.001 } }
    }
}

/// Frame-wide decisions for a frame with more than one world.
struct Plan {
    zb: f32,
    /// The world the camera is heading into (or 0 alone).
    next: usize,
    /// 0..1: how far the camera has become the next world's.
    cam_t: f32,
    /// Whose look the frame takes.
    look: usize,
    /// How much of the sky each world contributes (sums to 1).
    sky_w: SkyW,
    shear: Vec<Option<(f32, f32)>>,
    bounds: Option<Bounds>,
    taper: f32,
    adaptation: f32,
}

impl WorldRenderer {
    /// Run the passes that have a GPU form on this device (None: everything on the CPU).
    #[cfg(feature = "gpu")]
    pub fn set_gpu(&mut self, gpu: Option<Arc<super::gpu::Gpu>>) { self.gpu = gpu; }

    /// Force the hour of the day (0..24) on every scene with an open sky, turning its day and
    /// night (`sky.day_night`) on with the scene's own settings; None leaves each scene's own.
    pub fn set_hour(&mut self, hour: Option<f32>) { self.hour = hour.map(|h| h.rem_euclid(24.0)); }
    pub fn hour(&self) -> Option<f32> { self.hour }

    /// A renderer that draws on the GPU where there is one (`gpu::shared`), else on the CPU.
    pub fn auto() -> WorldRenderer {
        #[allow(unused_mut)]
        let mut r = WorldRenderer::default();
        #[cfg(feature = "gpu")]
        if let Some(c) = super::gpu::shared() {
            match super::gpu::Gpu::new(c) {
                Ok(g) => r.set_gpu(Some(Arc::new(g))),
                Err(e) => eprintln!("GPU renderer unavailable, rendering on the CPU: {e}"),
            }
        }
        r
    }

    /// Which engine draws this renderer's frames: "gpu" or "cpu".
    pub fn engine(&self) -> &'static str {
        #[cfg(feature = "gpu")]
        if self.gpu.is_some() { return "gpu"; }
        "cpu"
    }
    #[cfg(feature = "gpu")]
    pub fn gpu(&self) -> Option<&Arc<super::gpu::Gpu>> { self.gpu.as_ref() }

    /// Render the scene `distance` metres along the path.
    pub fn render(&mut self, scene: &Scene, distance: f32, opts: &RenderOptions) -> Image {
        let w = WorldIn { scene, distance, time: opts.time, base_dir: opts.base_dir.clone() };
        let ws = std::slice::from_ref(&w);
        let mut img = self.render_worlds(ws, None, opts);
        if let Some(s) = swing_now(ws, opts) { letterbox(&mut img, s.bars); }
        img
    }

    /// One frame of a transition or a fork: every world drawn together in one picture, each in its
    /// own part of the view, lit and fogged as one place. `worlds` is [current, next] for a
    /// crossing and [current, left, right] for a fork.
    pub fn render_layout(&mut self, worlds: &[WorldIn], layout: Layout, opts: &RenderOptions) -> Image {
        let need = match layout { Layout::Cross { .. } => 2, Layout::Fork { .. } => 3 };
        assert!(worlds.len() >= need, "a {} frame needs {need} worlds", if need == 2 { "crossing" } else { "fork" });
        let mut img = self.render_worlds(&worlds[..need], Some(layout), opts);
        if let Some(s) = swing_now(&worlds[..need], opts) { letterbox(&mut img, s.bars); }
        img
    }

    fn plan(worlds: &[WorldIn], layout: Option<Layout>) -> Plan {
        let one = Plan { zb: f32::INFINITY, next: 0, cam_t: 0.0, look: 0, sky_w: SkyW { base: vec![1.0], fork: None }, shear: vec![None], bounds: None, taper: 0.0, adaptation: 0.0 };
        let Some(layout) = layout else { return one };
        let past = |zb: f32, m: f32| smoothstep(m, -m, zb);
        match layout {
            Layout::Cross { crossing: c, zb } => {
                let cam_t = past(zb, c.camera_blend);
                let look = match c.style { StyleFrom::First => 0, StyleFrom::Second => 1, StyleFrom::Auto => (zb <= 0.0) as usize };
                let u = match c.sky { SkyRule::First => 0.0, SkyRule::Second => 1.0, SkyRule::Blend => past(zb, c.blend.max(2.0)) };
                Plan {
                    zb, next: 1, cam_t, look, sky_w: SkyW { base: vec![1.0 - u, u], fork: None }, shear: vec![None, None],
                    bounds: Some(Bounds::cross(zb, c.blend, worlds[1].distance, c.seed)), taper: c.taper, adaptation: c.adaptation,
                }
            }
            Layout::Fork { fork: f, zb, chosen, steer } => {
                let next = chosen.unwrap_or(f.default_branch).realm();
                let steer = if chosen.is_some() { steer.clamp(0.0, 1.0) } else { 0.0 };
                let cam_t = past(zb, f.camera_blend) * steer;
                let look = match f.style { StyleFrom::First => 0, StyleFrom::Second => next, StyleFrom::Auto => if zb <= 0.0 && steer > 0.5 { next } else { 0 } };
                // Turning onto a branch swings everything past the junction the other way.
                let base = [0.0, -f.slope, f.slope];
                let shift = steer * base[next];
                let slopes = base.map(|s| s - shift);
                let taken = if chosen.is_some() { steer } else { 0.0 };
                let keep = match next { 1 => [1.0, 1.0 - taken], _ => [1.0 - taken, 1.0] };
                // The frame's camera is filled in by the caller (see `render_worlds`).
                let sky_w = SkyW { base: vec![1.0, 0.0, 0.0], fork: Some(ForkSky { center: 0.0, focal: 1.0, split: 0.5 * (slopes[1] + slopes[2]), near: smoothstep(f.approach, 0.0, zb).max(cam_t), keep }) };
                let hw = [0.0, worlds[1].scene.path.half_width, worlds[2].scene.path.half_width];
                Plan {
                    zb, next, cam_t, look, sky_w, shear: slopes.iter().map(|&s| Some((zb, s))).collect(),
                    bounds: Some(Bounds::fork(zb, slopes, hw, f.wedge, f.blend, chosen.map(|b| (b.realm(), steer)), worlds[next].distance, f.seed)), taper: 0.0, adaptation: f.adaptation,
                }
            }
        }
    }

    fn render_worlds(&mut self, worlds: &[WorldIn], layout: Option<Layout>, opts: &RenderOptions) -> Image {
        // The season recolours a scene's foliage and ground: draw a copy that has them.
        let seasons: Vec<Option<Scene>> = worlds.iter().map(|w| crate::scene::Season::bake(w.scene, loop_phase(w))).collect();
        if seasons.iter().any(Option::is_some) {
            let ws: Vec<WorldIn> = worlds.iter().zip(&seasons).map(|(w, b)| WorldIn { scene: b.as_ref().unwrap_or(w.scene), ..w.clone() }).collect();
            return self.render_worlds(&ws, layout, opts);
        }
        // A day and night sets a scene's colours for its hour: draw a copy that has them.
        let forced = self.hour;
        let days: Vec<Option<Scene>> = worlds.iter().map(|w| {
            let dn = &w.scene.sky.day_night;
            let hour = forced.or_else(|| (dn.enabled && dn.days_per_loop > 0).then(|| dn.hour_at(loop_phase(w))));
            crate::scene::DayNight::bake(w.scene, hour)
        }).collect();
        if days.iter().any(Option::is_some) {
            let ws: Vec<WorldIn> = worlds.iter().zip(&days).map(|(w, b)| WorldIn { scene: b.as_ref().unwrap_or(w.scene), ..w.clone() }).collect();
            return self.render_worlds(&ws, layout, opts);
        }
        // A sunset takes a scene's colours towards its own: draw a copy that has them.
        let baked: Vec<Option<Scene>> = worlds.iter().map(|w| crate::scene::Sunset::bake(w.scene)).collect();
        if baked.iter().any(Option::is_some) {
            let ws: Vec<WorldIn> = worlds.iter().zip(&baked).map(|(w, b)| WorldIn { scene: b.as_ref().unwrap_or(w.scene), ..w.clone() }).collect();
            return self.render_worlds(&ws, layout, opts);
        }
        // The camera's moves: where they put it this frame. Standing back (a dolly) moves the
        // walk itself back, when there is one world to walk.
        let moves = &worlds[0].scene.camera.moves;
        let pose = (moves.any()).then(|| {
            let w0 = &worlds[0];
            let len = w0.scene.motion.loop_length.max(1.0);
            moves.pose(w0.distance, len, loop_phase(w0), opts.intro, w0.scene.camera.eye_height, w0.scene.camera.horizon)
        });
        let pan_max = moves.pan_max();
        // The swing in: the camera coming round into the scene's own view (one world to walk).
        let swing = swing_now(worlds, opts);
        let back = pose.map_or(0.0, |p| p.back) + swing.as_ref().map_or(0.0, |s| s.back);
        let backed: Vec<WorldIn>;
        let worlds = if back != 0.0 && worlds.len() == 1 {
            let time = Some(worlds[0].time.unwrap_or_else(|| worlds[0].distance / worlds[0].scene.motion.speed.max(0.01)));
            backed = vec![WorldIn { distance: worlds[0].distance - back, time, ..worlds[0].clone() }];
            &backed[..]
        } else { worlds };
        let mut prof = Prof::new(self.profile);
        let mut plan = Self::plan(worlds, layout);
        let n = plan.shear.len();
        let layers = opts.layers;
        let look = worlds[plan.look].scene;
        let (out_w, out_h) = opts.size.unwrap_or((look.canvas.width, look.canvas.height));
        let (out_w, out_h) = (out_w.max(1) as usize, out_h.max(1) as usize);
        let px = look.style.pixel_size.clamp(1, 16) as usize;
        let (w, h) = (out_w.div_ceil(px).max(2), out_h.div_ceil(px).max(2));

        // The camera: the first world's (or where a swing has it), becoming the next world's as it crosses.
        let mut cam = match &swing {
            Some(s) => View::posed(worlds[0].scene, &s.camera, w, h, s.turning),
            None => View::new(worlds[0].scene, w, h),
        };
        if plan.next > 0 {
            let vn = View::new(worlds[plan.next].scene, w, h);
            let t = plan.cam_t;
            let mix = |a: f32, b: f32| a + (b - a) * t;
            cam.horizon_px = mix(cam.horizon_px, vn.horizon_px);
            cam.focal_px = mix(cam.focal_px, vn.focal_px);
            cam.eye_height = mix(cam.eye_height, vn.eye_height);
            cam.bend = mix(cam.bend, vn.bend);
            // One scene's winding dies away as the next one's comes in.
            for i in 0..2 {
                cam.wind[i + 2] = [vn.wind[i][0] * t, vn.wind[i][1], vn.wind[i][2], vn.wind[i][3]];
                cam.wind[i][0] *= 1.0 - t;
            }
            cam.hill = mix(cam.hill, vn.hill);
            cam.near_ground = mix(cam.near_ground, vn.near_ground);
            cam.lens_curve = mix(cam.lens_curve, vn.lens_curve);
        }
        if let Some(p) = pose {
            cam.eye_height = (cam.eye_height + p.lift).clamp(0.05, 60.0);
            cam.focal_px *= p.zoom.clamp(0.1, 10.0);
            cam.horizon_px += p.tilt * cam.focal_px + p.horizon * h as f32;
            cam.pan_px = -p.pan * cam.focal_px;
            cam.pan_max = pan_max * cam.focal_px;
            cam.center_px += cam.pan_px;
        }
        // Each world's view: the shared camera with its own path, stairs and place along the loop.
        let mut views: Vec<View> = (0..n).map(|r| {
            let s = worlds[r].scene;
            let own = View::new(s, w, h);
            let at = worlds[r].distance.rem_euclid(s.motion.loop_length.max(1.0));
            let mut v = View { stairs: own.stairs, scroll: at, shear: plan.shear[r], ..cam };
            if n > 1 {
                let (a, b) = (&worlds[0], &worlds[r.max(plan.next).max(1)]);
                let (va, vb) = (View::new(a.scene, w, h), View::new(b.scene, w, h));
                let (fa, fb) = if matches!(layout, Some(Layout::Fork { .. })) { (own, own) } else { (va, vb) };
                v.half_width = fa.half_width;
                v.flare = fa.flare;
                v.split = Some(Split {
                    zb: plan.zb, half_width_b: fb.half_width, flare_b: fb.flare, taper: plan.taper,
                    stairs_a: va.stairs, scroll_a: a.distance.rem_euclid(a.scene.motion.loop_length.max(1.0)),
                    stairs_b: if r == 0 { vb.stairs } else { own.stairs }, scroll_b: if r == 0 { b.distance } else { worlds[r].distance }.rem_euclid(worlds[if r == 0 { plan.next.max(1) } else { r }].scene.motion.loop_length.max(1.0)),
                });
            }
            v
        }).collect();
        // Glossy floors reflect what stands above and beside the frame too, so that much more of
        // the world is rendered around it and cut away before post.
        let (mut gl, mut gt) = (0, 0);
        for r in 0..n {
            let (l, t) = reflection_guard(worlds[r].scene, &views[r], &layers);
            gl = gl.max(l);
            gt = gt.max(t);
        }
        for v in views.iter_mut() { *v = v.guarded(gl, gt); }
        let (bw, bh) = (views[0].width, views[0].height);
        if let Some(f) = plan.sky_w.fork.as_mut() { f.center = views[0].center_px; f.focal = views[0].focal_px; }

        // What each world looks like on average, for the eye's adaptation and light through openings.
        let means: Vec<[f32; 3]> = if n > 1 { (0..n).map(|r| self.scene_mean(worlds[r].scene, worlds[r].base_dir.clone())).collect() } else { vec![[0.5; 3]] };
        let lum = |c: [f32; 3]| (0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]).max(1e-4);

        let bounds = plan.bounds.as_ref();
        let mut ctxs: Vec<Ctx> = Vec::with_capacity(n);
        let mut strikes = Vec::with_capacity(n);
        for r in 0..n {
            let (ctx, ls, strike, flash) = self.world_ctx(&worlds[r], views[r], r as u8, bounds);
            ctxs.push(ctx);
            strikes.push((ls, strike, flash));
        }

        // A crossing's threshold: the face around the opening, the opening itself, and the light
        // that falls through it each way.
        let mut facade_tris = Vec::new();
        if let (Some(Layout::Cross { crossing: c, zb }), true) = (layout, n == 2) {
            if zb > NEAR && zb < ctxs[0].far {
                let (a, b) = (&ctxs[0], &ctxs[1]);
                let wide = { let v = &a.view; ((v.width as f32 * 0.5) / v.focal_px * zb + v.bend_x(zb).abs()) * 1.4 + 4.0 };
                let roof = |s: &Scene| s.ceiling.enabled.then(|| s.ceiling.height.max(0.2));
                let a_hw = if roof(a.scene).is_some() && a.scene.walls.enabled { a.wall_x(zb) } else { wide };
                let a_top = roof(a.scene).unwrap_or_else(|| c.facade.as_ref().map_or(f32::INFINITY, |f| f.height));
                let b_hw = if b.scene.walls.enabled { b.wall_x(zb) } else if roof(b.scene).is_some() { wide } else { f32::INFINITY };
                let b_top = roof(b.scene).unwrap_or(f32::INFINITY);
                let (hw, top) = (a_hw.min(b_hw), a_top.min(b_top));
                let op = Opening {
                    hw, top, arch: c.arch, rim: c.rim, seed: c.seed, outer_hw: a_hw, outer_top: a_top,
                    hill: if roof(a.scene).is_none() { c.facade.as_ref().map_or(0.0, |f| f.rough) } else { 0.0 },
                };
                if let Some(f) = c.facade.as_ref().filter(|_| hw < a_hw - 0.01 || top < a_top - 0.01 || c.rim > 0.0) {
                    if top.is_finite() && a_top.is_finite() {
                        facade_tris = build_facade(&a.view, &op, zb);
                        let tile = snap_to_loop(f.material.tile_size, a.loop_len);
                        let tex = self.textures.get(&f.material);
                        ctxs[0].facade = Some((tex, tile));
                        ctxs[0].opening = Some(op);
                    }
                }
                // Light through the opening: into a roofed world from the one beyond.
                if c.spill > 0.0 && top.is_finite() {
                    // Each scene is authored as an eye adapted to it sees it: if the eye goes halfway
                    // (in log terms) toward what it looks at, a scene's real brightness goes as its
                    // shown average squared, so light from one world reaches the other at the ratio
                    // of their averages (scaled down with less adaptation).
                    let k = c.adaptation.max(0.25);
                    let ratio = |from: usize, to: usize| (lum(means[from]) / lum(means[to])).powf(k);
                    let (oh, ow) = (top.min(30.0), hw.min(60.0));
                    let corners = |v: &View| [[-ow, 0.0], [ow, 0.0], [ow, oh], [-ow, oh]].map(|[x, y]| v.to_cam(x, y, zb));
                    if roof(ctxs[0].scene).is_some() {
                        let rad = scale3(means[1], c.spill * ratio(1, 0));
                        let cs = corners(&ctxs[0].view);
                        ctxs[0].portal = Some(Portal { corners: cs, radiance: rad, zb, front: true });
                    }
                    if roof(ctxs[1].scene).is_some() {
                        let rad = scale3(means[0], c.spill * ratio(0, 1));
                        let cs = corners(&ctxs[1].view);
                        ctxs[1].portal = Some(Portal { corners: cs, radiance: rad, zb, front: false });
                    }
                }
            }
            // The air in front of the boundary is the first world's.
            if zb > 0.0 {
                let a = &ctxs[0];
                ctxs[1].front = Some(FrontAir { zb, fog: a.fog, fog_col: a.fog_col, gain: 1.0 });
            }
        }
        if let Some(Layout::Fork { zb, .. }) = layout {
            // Past the junction, the first world is only the ground between the branches.
            ctxs[0].verge_beyond = Some(zb);
        }

        prof.mark("setup");
        // Geometry: each world's, cut to its part of the view.
        let mut tris = Vec::new();
        for ctx in &ctxs {
            let t = build_geometry(ctx, &layers);
            match bounds {
                None => tris.extend(t),
                Some(b) => tris.extend(raster::clip_region(t, &b.realms[ctx.realm as usize], ctx.realm)),
            }
        }
        tris.extend(facade_tris);
        prof.mark("geometry");
        let mut gbuf = vec![GPixel::EMPTY; bw * bh];
        // A frame of one world on the GPU is rasterised there (with its shadow masks and lamp
        // tiles); several worlds need the G-buffer here first, to mix them and adapt the eye.
        #[cfg(feature = "gpu")]
        let gpu_geom = self.gpu.is_some() && n == 1;
        #[cfg(not(feature = "gpu"))]
        let gpu_geom = false;
        #[cfg(feature = "gpu")]
        let prepared = if gpu_geom { raster::prepare(bw, bh, &tris, &ctxs[0].view) } else { Vec::new() };
        if !gpu_geom { raster::rasterize(&mut gbuf, bw, bh, &tris, &ctxs[0].view); }
        prof.mark("raster");
        // Where open ground, walls or roofs of two worlds meet over a band, mix them in patches.
        let mut soft: Vec<(u8, f32)> = Vec::new();
        if let Some(b) = bounds.filter(|b| !b.edges.is_empty()) {
            soft = vec![(0, 0.0); bw * bh];
            gbuf.par_iter_mut().zip(soft.par_iter_mut()).for_each(|(g, sf)| {
                if !matches!(g.id, id::GROUND | id::WALL_L | id::WALL_R | id::CEILING) { return; }
                let from = &ctxs[g.realm as usize];
                let c = from.view.to_cam(g.x, g.y, g.d);
                let (to, other, k) = b.realm_mix(c[0], c[2]);
                let has = |r: usize| r < ctxs.len() && match g.id { id::GROUND => true, id::CEILING => ctxs[r].scene.ceiling.enabled, _ => ctxs[r].scene.walls.enabled };
                let (to, other) = (to as usize, other as usize);
                if to != g.realm as usize && has(to) {
                    let t = &ctxs[to];
                    g.x = c[0] - t.view.bend_x(g.d) - t.view.shear_x(g.d);
                    g.realm = to as u8;
                }
                if k > 0.0 && other != g.realm as usize && has(other) { *sf = (other as u8, k); }
            });
        }

        // Instances and lights.
        prof.mark("realm mix");
        let mut bills: Vec<Billboard> = Vec::new();
        let mut flames: Vec<Flame> = Vec::new();
        for r in 0..n {
            let ls = strikes[r].0;
            let base = worlds[r].base_dir.clone();
            let o = RenderOptions { base_dir: base, ..opts.clone() };
            let first = bills.len();
            if layers.fixtures {
                self.collect_fixtures(&mut ctxs[r], ls, &o, &mut bills, &mut flames);
            }
            if layers.props {
                self.collect_props(&mut ctxs[r], &o, &mut bills);
                self.collect_set_pieces(&mut ctxs[r], &o, &mut bills);
                self.collect_waterfalls(&mut ctxs[r], &mut bills);
                self.collect_companions(&mut ctxs[r], &o, &mut bills);
            }
            // And what stands on its own side of the path, with the light it gives.
            if ctxs[r].view.side_k > 0.0 && ctxs[r].view.side_k < 1.0 {
                for b in &mut bills[first..] {
                    b.fade = b.fade.max(ctxs[r].fading(b.world[0]));
                    b.shadow *= 1.0 - b.fade;
                    // (A lamp's light fades where it is made: `collect_fixtures`.)
                    if let (Some(l), true, false) = (b.own_light, b.fade > 0.0, b.id == id::FIXTURE) { let c = &mut ctxs[r].lights[l]; c.color = scale3(c.color, 1.0 - b.fade); }
                }
            }
        }
        // A structure standing in the threshold.
        if let Some(Layout::Cross { crossing: c, zb }) = layout {
            if let Some(kind) = c.marker.filter(|_| zb > NEAR + 0.2 && zb < 150.0 && layers.props) {
                let a = &ctxs[0];
                let op = a.opening;
                let ow = op.map_or(2.0 * (a.view.path_half_width(zb) + 0.6), |o| 2.0 * o.half_width());
                let oh = op.map(|o| o.top).filter(|t| t.is_finite()).unwrap_or(4.5).min(12.0);
                let tint = c.facade.as_ref().map_or([128, 118, 104], |f| f.material.base);
                let accent = if kind == SetPieceKind::Portal { worlds[1].scene.sky.horizon } else { [150, 40, 36] };
                let (sprite, _, height) = self.sprites.set_piece(kind, tint, accent, ow, oh, c.seed);
                if kind == SetPieceKind::Portal {
                    let pos = a.view.to_cam(0.0, oh * 0.5, zb);
                    ctxs[0].lights.push(PointLight { pos, color: scale3(rgb_lin(accent), 1.6), radius: (ow + oh).max(4.0) });
                }
                let z = ctxs[0].view.to_cam(0.0, 0.0, zb)[2];
                bills.push(Billboard {
                    fade: 0.0, z: z - 0.05, world: [0.0, 0.0, zb - 0.05], height, sprite, flip: false, nearest: false, id: id::PROP,
                    emissive: if kind == SetPieceKind::Portal { 0.9 } else { 0.0 }, shadow: 0.6, own_light: None, source: 0, realm: 0, panel: None, tube: None, sway: 0.0, tint: [1.0; 3], wind: CardWind::default(),
                });
            }
        }
        // Lamps near a boundary light both sides of it.
        if let Some(zb) = (n > 1).then_some(plan.zb) {
            let shared: Vec<(usize, [f32; 3], [f32; 3], f32)> = ctxs.iter().enumerate()
                .flat_map(|(r, c)| c.lights.iter().filter(|l| (l.pos[2] - zb).abs() < l.radius).map(move |l| (r, l.pos, l.color, l.radius)))
                .collect();
            for (from, pos, color, radius) in shared {
                for (r, c) in ctxs.iter_mut().enumerate() {
                    if r != from { c.lights.push(PointLight { pos, color, radius }); }
                }
            }
        }

        // The eye adapts to what fills the view: a small bright exit glares from a dark passage, a
        // cave mouth is black from daylight, and each settles as it fills the frame.
        if n > 1 && plan.adaptation > 0.0 {
            let mut cover = vec![0.0f32; n];
            for (i, g) in gbuf.iter().enumerate() {
                if g.id == id::NONE { for r in 0..n { cover[r] += plan.sky_w.at(r, i % bw); } } else { cover[g.realm as usize] += 1.0; }
            }
            // The eye also stays partly adapted to the light it stands in, so a dark doorway that
            // fills the view does not yet turn the sunlit ground around it white.
            let here = if plan.zb > 0.0 { 0 } else { plan.next };
            let total: f32 = cover.iter().sum::<f32>().max(1.0);
            let stand = 0.6;
            let eye = (0..n).map(|r| (cover[r] / total + if r == here { stand } else { 0.0 }) / (1.0 + stand) * lum(means[r]).ln()).sum::<f32>().exp();
            for r in 0..n { ctxs[r].gain = (lum(means[r]) / eye).powf(plan.adaptation).clamp(0.2, 4.0); }
            let g0 = ctxs[0].gain;
            for c in ctxs.iter_mut() { if let Some(f) = c.front.as_mut() { f.gain = g0; } }
        }

        prof.mark("instances");
        // Shadow masks on the ground: sun shadows of props, and contact darkening under them.
        let mut sun_mask = vec![1.0f32; bw * bh];
        let mut ao_mask = vec![1.0f32; bw * bh];
        if layers.ground && !gpu_geom {
            for ctx in &ctxs { prop_shadows(ctx, &bills, &gbuf, &mut sun_mask, &mut ao_mask); }
        }
        // Cloud shadows drifting over the ground and walls.
        if ctxs.iter().any(|c| c.wx.clouds > 0.0) && !gpu_geom {
            sun_mask.par_iter_mut().zip(gbuf.par_iter()).for_each(|(m, g)| {
                if g.id != id::NONE { *m *= weather::cloud_shade(&ctxs[(g.realm as usize).min(n - 1)], g.x, g.d); }
            });
        }

        prof.mark("shadows");
        // Post and style take the look of one world (post settings blend as the camera crosses).
        let blended;
        let post_scene: &Scene = if n > 1 {
            let (a, b) = (&worlds[0].scene.post, &worlds[plan.next].scene.post);
            let t = plan.cam_t;
            let mix = |x: f32, y: f32| x + (y - x) * t;
            let mut s = look.clone();
            s.post = Post {
                exposure: mix(a.exposure, b.exposure), contrast: mix(a.contrast, b.contrast), saturation: mix(a.saturation, b.saturation),
                bloom: mix(a.bloom, b.bloom), vignette: mix(a.vignette, b.vignette), grain: mix(a.grain, b.grain),
                tint: [0, 1, 2].map(|k| mix(a.tint[k] as f32, b.tint[k] as f32).round() as u8),
                // One flare fades out before the other's style fades in.
                flare: if a.flare.style == b.flare.style { Flare { style: a.flare.style, amount: mix(a.flare.amount, b.flare.amount) } }
                    else if t < 0.5 { Flare { style: a.flare.style, amount: a.flare.amount * (1.0 - 2.0 * t) } }
                    else { Flare { style: b.flare.style, amount: b.flare.amount * (2.0 * t - 1.0) } },
                focus: if t < 0.5 { a.focus.clone() } else { b.focus.clone() },
            };
            blended = s;
            &blended
        } else { look };
        // Deferred lighting.
        let mut hdr = vec![[0.0f32; 3]; bw * bh];
        let mut refl = vec![Refl::default(); bw * bh];
        // The world whose air the camera is in.
        let here = if plan.zb > 0.0 { 0 } else { plan.next };
        // On the GPU, the frame's buffers stay there from here on; `gpu_has` says which ones hold
        // newer data than the CPU's copies, and a CPU pass fetches what it needs. Each world is
        // drawn over its own pixels with its own parameters.
        // Post on the GPU needs the palette, its levels and the grade first (finding them can render
        // frames of its own).
        #[cfg(feature = "gpu")]
        let gpu_ok = self.gpu.is_some() && ctxs.len() <= super::gpu::MAX_WORLDS;
        #[cfg(feature = "gpu")]
        let mut gpu_look = if gpu_ok {
            let base = worlds[plan.look].base_dir.clone();
            let look_opts = RenderOptions { base_dir: base.clone(), ..opts.clone() };
            let lut = look_opts.palette.clone().or_else(|| self.palette_for(look, &look_opts));
            let levels = lut.as_ref().filter(|l| l.ramp.is_some()).map(|_| self.loop_levels(look, base.clone()));
            let grade = if layers.post && !look.style.grade.trim().is_empty() && look.style.grade_strength > 0.0 { self.grade_for(&look.style.grade, base.as_deref()) } else { None };
            Some((lut, levels, grade))
        } else { None };
        #[cfg(feature = "gpu")]
        let gpu_arc = self.gpu.clone();
        #[cfg(feature = "gpu")]
        let mut gf = match &gpu_arc {
            Some(g) if gpu_ok => {
                let tiles = if gpu_geom { None } else { tile_lights(&ctxs, &gbuf) };
                let mut light_base = 0;
                let mut params: Vec<_> = ctxs.iter().enumerate().map(|(r, ctx)| {
                    let mut p = gpu_params(ctx, &layers);
                    gpu_world_extras(&mut p, ctx, &ctxs);
                    (p.realm, p.tex_base, p.light_base, p.multi) = (r as u32, 8 * r as u32, light_base, (n > 1) as u32);
                    light_base += (ctx.lights.len() + ctx.sky_lights.len()) as u32;
                    p
                }).collect();
                if let Some(t) = &tiles { params[0].tiles_on = 1; params[0].tile_cols = t.cols as u32; }
                // Lamp tiles made by the geometry pass (the same rule as `tile_lights`).
                if gpu_geom && ctxs[0].lights.len() >= 4 {
                    (params[0].tiles_on, params[0].tile_cols, params[0].tile_cap) = (2, bw.div_ceil(LIGHT_TILE) as u32, ctxs[0].lights.len() as u32);
                }
                let mut f = g.frame(&params);
                f.profile = self.profile;
                (f.here, f.look) = (here, plan.look);
                // Whether any pixel can reflect (shade gives gloss only to these).
                let reflects = params.iter().any(|p| p.path_gloss > 0.0 || p.verge_gloss > 0.0 || p.deck_gloss > 0.0 || (p.bridge_on != 0 && p.br_bottom == 1) || p.wx_wet > 0.0 || p.wx_puddles > 0.0);
                Some((f, GpuHas { reflects, ..GpuHas::default() }, tiles))
            }
            _ => None,
        };
        #[cfg(feature = "gpu")]
        if let Some((f, has, tiles)) = gf.as_mut() {
            if gpu_geom {
                f.geometry(&gpu_geom_input(&ctxs[0], &prepared, &bills, &layers));
                has.gbuf = true;
            } else {
                f.put_gbuf(&gbuf);
                f.put_masks(&sun_mask, &ao_mask);
                f.put_soft(&soft);
            }
            gpu_shade(f, &ctxs, tiles.take());
            has.hdr = true;
            has.refl = true;
        }
        #[cfg(feature = "gpu")]
        let on_gpu = gf.is_some();
        #[cfg(not(feature = "gpu"))]
        let on_gpu = false;
        if !on_gpu { shade(&ctxs, &plan.sky_w, &soft, &gbuf, &sun_mask, &ao_mask, &layers, &mut hdr, &mut refl); }
        prof.mark("shade");
        let bank_t = ctxs[here].bank_t(ctxs[here].far);
        // On the GPU the sky, the veil and the bank are drawn in one pass per world: one world's over
        // the sky shading, several worlds' each weighted by how much it counts at each column.
        #[cfg(feature = "gpu")]
        if let Some((f, _, _)) = gf.as_mut() {
            let bank = (bank_t < 1.0).then(|| (bank_t, ctxs[here].fog_col));
            let strike = |r: usize| match &strikes[r] { (_, Some(s), flash) => Some((s, *flash)), _ => None };
            if n == 1 {
                let draw = layers.sky && plan.sky_w.any(0) && worlds[0].scene.sky.enabled;
                if draw || bank.is_some() { f.sky(&gpu_sky(&ctxs[0], draw, strike(0), bank), 0); }
            } else {
                for r in 0..n {
                    let draw = layers.sky && plan.sky_w.any(r) && worlds[r].scene.sky.enabled;
                    let mut input = gpu_sky(&ctxs[r], draw, strike(r), if r + 1 == n { bank } else { None });
                    let p = &mut input.params;
                    p.mode = if r == 0 { 1 } else { 2 };
                    match &plan.sky_w.fork {
                        Some(fs) => (p.w_fork, p.w_center, p.w_focal, p.w_split, p.w_near, p.w_keep) = (1, fs.center, fs.focal, fs.split, fs.near, fs.keep),
                        None => p.w_base = plan.sky_w.base.get(r).copied().unwrap_or(0.0),
                    }
                    f.sky(&input, r);
                }
            }
        }
        if layers.sky && !on_gpu {
            let skies: Vec<usize> = (0..n).filter(|&r| plan.sky_w.any(r) && worlds[r].scene.sky.enabled).collect();
            let draw = |r: usize, hdr: &mut Vec<[f32; 3]>| {
                draw_sky_bodies(&ctxs[r], &gbuf, hdr);
                if let (_, Some(s), flash) = &strikes[r] { draw_lightning(&ctxs[r], s, *flash, &gbuf, hdr); }
                weather::veil_sky(&ctxs[r], &gbuf, hdr);
                space::draw_tunnel(&ctxs[r], &gbuf, hdr);
            };
            if n == 1 {
                if !skies.is_empty() { draw(0, &mut hdr); }
            } else if !skies.is_empty() {
                // Each sky drawn on its own, at its world's adaptation and through the first
                // world's air, then mixed where the sky shows.
                // Worlds without a sky keep the dark they showed in shading.
                let mut acc: Vec<[f32; 3]> = (0..bw * bh).map(|i| if gbuf[i].id != id::NONE { hdr[i] } else {
                    (0..n).filter(|r| !skies.contains(r) && plan.sky_w.at(*r, i % bw) > 0.0)
                        .fold([0.0; 3], |a, r| add3(a, scale3(sky_seen(&ctxs[r], sky_base(&ctxs[r], i / bw, &layers)), plan.sky_w.at(r, i % bw))))
                }).collect();
                for &r in &skies {
                    let mut one: Vec<[f32; 3]> = (0..bw * bh).map(|i| if gbuf[i].id == id::NONE { sky_base(&ctxs[r], i / bw, &layers) } else { [0.0; 3] }).collect();
                    draw(r, &mut one);
                    for (i, ((a, o), g)) in acc.iter_mut().zip(&one).zip(&gbuf).enumerate() {
                        if g.id == id::NONE { *a = add3(*a, scale3(sky_seen(&ctxs[r], *o), plan.sky_w.at(r, i % bw))); }
                    }
                }
                hdr = acc;
            }
        }
        // A fog bank the camera is inside or looking through hides the sky too (the even fog is
        // already matched to the sky by its colour).
        if bank_t < 1.0 && !on_gpu {
            let fc = ctxs[here].fog_col;
            hdr.par_iter_mut().zip(gbuf.par_iter()).for_each(|(c, g)| if g.id == id::NONE { *c = mix3(fc, *c, bank_t); });
        }

        prof.mark("sky");
        // The sky as it is before props are drawn over it, for reflections of the sky behind them.
        let sky_under_props = (!on_gpu && refl.iter().any(|r| r.r >= 0.002)).then(|| hdr.clone());
        #[cfg(feature = "gpu")]
        if let Some((f, has, _)) = gf.as_mut() { if has.reflects { f.keep_sky(); } }
        // Billboards far to near.
        bills.sort_by(|a, b| b.z.partial_cmp(&a.z).unwrap_or(std::cmp::Ordering::Equal));
        let crisp = px > 1;
        let mut pick = if opts.pick { vec![0u16; bw * bh] } else { Vec::new() };
        {
            // Set up every card, then draw them far to near in bands of rows side by side: within
            // a band the order is the same as drawing one card after another.
            let cards: Vec<Card> = bills.par_iter().filter_map(|b| Card::new(&ctxs[b.realm as usize], b, crisp)).collect();
            // On the GPU, then everything after this still runs on the CPU: bring the GPU's
            // results back.
            #[cfg(feature = "gpu")]
            if let Some((f, has, _)) = gf.as_mut() {
                if !cards.is_empty() {
                    let list: Vec<_> = cards.par_iter().map(|c| c.gpu()).collect();
                    let boxes: Vec<_> = cards.iter().map(|c| (c.x0, c.y0, c.x1, c.y1)).collect();
                    f.cards(&list, &super::gpu::bin_tiles(bw, bh, &boxes), opts.pick);
                    has.gbuf = true;
                    has.cards = true;
                }
            }
            #[cfg(feature = "gpu")]
            let cpu_cards = gf.is_none();
            #[cfg(not(feature = "gpu"))]
            let cpu_cards = true;
            const BAND: usize = 8;
            let band = |k: usize, g: &mut [GPixel], c: &mut [[f32; 3]], mut p: Option<&mut [u16]>| {
                let (r0, r1) = ((k * BAND) as i64, (k * BAND + g.len() / bw) as i64 - 1);
                for card in cards.iter().filter(|c| c.y1 >= r0 && c.y0 <= r1) { card.draw_rows(k * BAND, g, c, p.as_deref_mut()); }
            };
            if !cpu_cards {
            } else if opts.pick {
                gbuf.par_chunks_mut(bw * BAND).zip(hdr.par_chunks_mut(bw * BAND)).zip(pick.par_chunks_mut(bw * BAND)).enumerate()
                    .for_each(|(k, ((g, c), p))| band(k, g, c, Some(p)));
            } else {
                gbuf.par_chunks_mut(bw * BAND).zip(hdr.par_chunks_mut(bw * BAND)).enumerate().for_each(|(k, (g, c))| band(k, g, c, None));
            }
        }
        prof.mark("billboards");
        // Grass, flames, the wisps, particles and weather, as one list of splats each pass adds to.
        // On the GPU each list is kept and drawn in two dispatches (blades, which write the
        // G-buffer, then the rest), and everything after this still runs on the CPU: bring the
        // GPU's results back.
        #[cfg(feature = "gpu")]
        let splat_gpu = gf.is_some();
        #[cfg(not(feature = "gpu"))]
        let splat_gpu = false;
        let mut tufts: Vec<Splat> = Vec::new();
        for ctx in &ctxs {
            if layers.ground && ctx.scene.verge.tufts { draw_tufts(ctx, &mut tufts); }
        }
        if !splat_gpu { splat::draw_all(&tufts, &mut gbuf, &mut hdr, bw, bh); }
        prof.mark("tufts");
        let mut splats: Vec<Splat> = Vec::new();
        let mut later: Vec<Splat> = Vec::new();
        let mut run = |list: &mut Vec<Splat>, gbuf: &mut Vec<GPixel>, hdr: &mut Vec<[f32; 3]>| {
            if splat_gpu { later.append(list); } else { splat::draw_all(list, gbuf, hdr, bw, bh); list.clear(); }
        };
        for f in &flames {
            draw_flame(&ctxs[f.realm as usize], f, &mut splats);
        }
        run(&mut splats, &mut gbuf, &mut hdr);
        prof.mark("flames");
        if layers.particles {
            for (r, ctx) in ctxs.iter().enumerate() {
                wildlife::draw(ctx, &mut splats);
                draw_falls(ctx, &mut splats);
                draw_cascades(ctx, &mut splats);
                draw_wires(ctx, &mut splats);
                draw_smoke(ctx, &mut splats);
                weather::draw_wisps(ctx, &mut splats);
                run(&mut splats, &mut gbuf, &mut hdr);
                prof.mark("wisps");
                for p in ctx.scene.particles.iter().filter(|p| p.enabled && p.count > 0) {
                    draw_particles(ctx, p, strikes[r].0, &mut splats);
                }
                run(&mut splats, &mut gbuf, &mut hdr);
                prof.mark("particles");
                weather::draw_precip(ctx, &mut splats);
                run(&mut splats, &mut gbuf, &mut hdr);
                prof.mark("precip");
                weather::draw_drips(ctx, &mut splats);
                weather::draw_sandstorm(ctx, &mut splats);
                run(&mut splats, &mut gbuf, &mut hdr);
                prof.mark("drips+sand");
            }
        }
        #[cfg(feature = "gpu")]
        if let Some((f, has, _)) = gf.as_mut() {
            if !tufts.is_empty() {
                let (l, b) = splat::gpu_list(&tufts, bw, bh);
                f.splats(0, &l, &b);
                has.gbuf = true;
            }
            if !later.is_empty() {
                let (l, b) = splat::gpu_list(&later, bw, bh);
                f.splats(1, &l, &b);
            }
            // Reflections in glossy floors, now that everything they could show is drawn.
            if has.reflects { f.reflect(); }
            has.refl = false;
            // The air: mist over everything low, each world's over its own pixels.
            if layers.particles {
                let airs: Vec<_> = ctxs.iter().map(|c| {
                    let m = &c.scene.weather.mist;
                    (m.enabled && m.density > 0.0).then(|| {
                        let travel = if c.scene.weather.wind.enabled { c.scene.weather.wind.speed * 0.5 } else { 0.4 } * c.scene.motion.loop_seconds();
                        {
                            let (glow_on, sun_x, sun_y, fhy, sun) = weather::mist_sun(c).map_or((0, 0.0, 0.0, 1.0, [0.0; 3]), |(x, y, f, s)| (1, x, y, f, s));
                            super::gpu::AirParams {
                                mist_on: 1, density: m.density, top: m.height.max(0.05), patch: m.patchiness, travel, seed: m.seed, mist: rgb_lin(m.color),
                                soft: m.soft.clamp(0.0, 1.0), over: (m.over == crate::scene::MistOver::Path) as u32, spread: m.spread, glow_on, sun_x, sun_y, fhy, sun, from: m.from.max(0.0),
                            }
                        }
                    })
                }).collect();
                f.mist(&airs);
            }
            // Then light scattered in it.
            if layers.particles {
                if let Some(su) = weather::shaft_setup(&ctxs, here) {
                    let mut sp = super::gpu::ShaftParams { mw: su.mw as u32, mh: su.mh as u32, hy: ctxs[here].view.horizon_px.max(2.0), ..Default::default() };
                    if let Some((sx, sy, glow_r, k, col)) = su.sun { (sp.sun_on, sp.sx, sp.sy, sp.glow_r, sp.k_sun, sp.sun) = (1, sx, sy, glow_r, k, col); }
                    let lamps: Vec<[f32; 8]> = ctxs.iter().flat_map(|c| c.lights.iter()).map(|l| [l.pos[0], l.pos[1], l.pos[2], l.radius, l.color[0], l.color[1], l.color[2], 0.0]).collect();
                    let lists = match su.lamps {
                        Some((k, tc, lists)) => { (sp.lamps_on, sp.k_lamp, sp.tc, sp.n_lights) = (1, k, tc as u32, lamps.len() as u32); lists }
                        None => Vec::new(),
                    };
                    f.shafts(&sp, &lists, &lamps);
                }
            }
            // Post and style too, when its buffers fit: then only the finished frame comes back.
            // In a frame of several worlds, lenses of both worlds at once and pick ids (each world
            // names its own ground) are left to the CPU.
            let fits = f.post_words(w, h, layers.post && look.style.paint >= 0.5).is_some();
            let lens_of = |r: usize, k: f32| { let l = &ctxs[r].scene.weather.lens; (layers.post && l.enabled && l.amount > 0.0 && k > 0.0).then_some((&ctxs[r], k)) };
            let lenses: Vec<(&Ctx, f32)> = if n > 1 { [lens_of(0, 1.0 - plan.cam_t), lens_of(plan.next, plan.cam_t)].into_iter().flatten().collect() } else { lens_of(0, 1.0).into_iter().collect() };
            let multi_ok = n == 1 || (!opts.pick && lenses.len() <= 1);
            if let (true, true, Some((lut, levels, grade))) = (fits, multi_ok, gpu_look.take()) {
                let li = plan.look;
                let fv = ctxs[li].view.frame();
                let input = gpu_post_params(&ctxs[li], &ctxs, &fv, post_scene, lenses.first().copied(), &layers, opts, (gl, gt, w, h, out_w, out_h, px), crisp, has.cards, lut.as_deref(), levels, grade.as_deref());
                let out = f.post(input);
                prof.mark("gpu post");
                if self.profile { prof.out.extend(f.pass_times()); }
                let mut img = Image { width: out_w, height: out_h, rgba: out.rgba, stats: None, depth: None, pick: None };
                img.pick = out.pick.map(|ids| Pick { width: w, height: h, ids });
                img.depth = out.depth.map(|mut d| {
                    if fv.lens_curve.abs() > 0.001 { super::post::lens_warp_depth(&mut d, w, h, fv.horizon_px, fv.lens_curve); }
                    super::post::upscale_depth(&d, w, h, px, out_w, out_h)
                });
                img.stats = out.stats.map(|rows| stats_from_rows(&ctxs[li], &rows, w * h, bills.len()));
                for (name, ms) in prof.out {
                    match self.stages.iter_mut().find(|(n, _)| *n == name) { Some(e) => e.1 += ms, None => self.stages.push((name, ms)) }
                }
                return img;
            }
            f.download(super::gpu::Down {
                gbuf: has.gbuf.then_some(&mut gbuf[..]),
                hdr: has.hdr.then_some(&mut hdr[..]),
                refl: has.refl.then_some(bytemuck::cast_slice_mut(&mut refl[..])),
                pick: (has.cards && opts.pick).then_some(&mut pick[..]),
            });
            prof.mark("gpu wait+read");
            if self.profile { prof.out.extend(f.pass_times()); }
        }
        #[cfg(feature = "gpu")]
        drop(gf);

        // Reflections in glossy floors, now that everything they could show is drawn.
        if !splat_gpu && refl.iter().any(|r| r.r >= 0.002) {
            reflect_pass(&ctxs[0], &gbuf, &refl, sky_under_props.as_deref(), &mut hdr);
        }
        prof.mark("reflect");
        // The air: mist over everything low, then light scattered in it.
        if layers.particles {
            if !splat_gpu { weather::apply_mist(&ctxs, &gbuf, &mut hdr); }
            prof.mark("mist");
            if !splat_gpu { weather::light_shafts(&ctxs, if plan.zb > 0.0 { 0 } else { plan.next }, &gbuf, &mut hdr); }
            prof.mark("shafts");
        }
        if gl > 0 || gt > 0 {
            hdr = crop(&hdr, bw, gl, gt, w, h);
            gbuf = crop(&gbuf, bw, gl, gt, w, h);
            if opts.pick { pick = crop(&pick, bw, gl, gt, w, h); }
            for c in ctxs.iter_mut() { c.view = c.view.frame(); }
        }
        let pick = opts.pick.then(|| {
            // What no card covers is the surface behind it.
            pick.par_iter_mut().zip(gbuf.par_iter()).for_each(|(p, g)| {
                if *p != 0 { return; }
                let ctx = &ctxs[g.realm as usize];
                *p = match g.id {
                    id::NONE => PICK_SKY,
                    id::WALL_L | id::WALL_R => PICK_WALLS,
                    id::CEILING => PICK_CEILING,
                    id::GROUND if ctx.scene.verge.enabled && g.x.abs() > ctx.path_edge(g.d) => PICK_VERGE,
                    _ => PICK_PATH,
                };
            });
            Pick { width: w, height: h, ids: pick }
        });

        prof.mark("crop+pick");
        // Heat haze and the lens, on the finished frame.
        if layers.post {
            weather::heat_shimmer(&ctxs, &gbuf, &mut hdr);
            if let Some(k) = weather::FocusK::of(post_scene, h, true) { weather::focus(&k, &gbuf, &mut hdr, w, h); }
            if n > 1 {
                weather::lens(&ctxs[0], 1.0 - plan.cam_t, &mut hdr);
                weather::lens(&ctxs[plan.next], plan.cam_t, &mut hdr);
            } else {
                weather::lens(&ctxs[0], 1.0, &mut hdr);
            }
        }

        // Post and style, in the look of one world (post settings blend as the camera crosses).
        let li = plan.look;
        let scene: &Scene = post_scene;
        let look_opts = RenderOptions { base_dir: worlds[li].base_dir.clone(), ..opts.clone() };
        let ctx = &ctxs[li];
        let tphase = ctx.tphase;
        prof.mark("shimmer+lens");
        let mut rgb = super::post::finish(&ctx.view, scene, tphase, &hdr, &gbuf, layers.post);
        prof.mark("post");
        if layers.post && scene.style.paint >= 0.5 {
            super::looks::kuwahara(&mut rgb, w, h, scene.style.paint);
        }
        if layers.post && !scene.style.grade.trim().is_empty() && scene.style.grade_strength > 0.0 {
            if let Some(g) = self.grade_for(&scene.style.grade, look_opts.base_dir.as_deref()) {
                super::looks::grade(&mut rgb, &g, scene.style.grade_strength);
            }
        }
        if scene.style.outline.enabled {
            super::post::outline(&mut rgb, &gbuf, w, h, &scene.style.outline);
        }
        if ctx.view.lens_curve.abs() > 0.001 {
            super::post::lens_warp(&mut rgb, w, h, ctx.view.horizon_px, ctx.view.lens_curve, crisp || scene.style.palette != Palette::Full);
        }
        let lut = look_opts.palette.clone().or_else(|| self.palette_for(look, &look_opts));
        if let Some(lut) = &lut {
            if lut.ramp.is_some() {
                // Ramp palettes have few shades: stretch the loop's own lightness range across them.
                let (lo, hi) = self.loop_levels(look, look_opts.base_dir.clone());
                let k = 255.0 / (hi - lo).max(8.0);
                rgb.par_iter_mut().for_each(|c| for v in c.iter_mut() { *v = ((*v - lo) * k).clamp(0.0, 255.0); });
            }
            super::post::quantize(&mut rgb, w, &lut, scene.style.dither, scene.style.dither_strength);
        }
        prof.mark("style");
        let ctx = &ctxs[li];
        let stats = opts.stats.then(|| frame_stats(ctx, &gbuf, &rgb, bills.len()));
        let depth = opts.depth.then(|| {
            let mut d: Vec<f32> = gbuf.iter().map(|g| g.depth).collect();
            if ctx.view.lens_curve.abs() > 0.001 { super::post::lens_warp_depth(&mut d, w, h, ctx.view.horizon_px, ctx.view.lens_curve); }
            super::post::upscale_depth(&d, w, h, px, out_w, out_h)
        });
        let mut img = super::post::upscale(&rgb, w, h, px, out_w, out_h);
        img.depth = depth;
        if layers.post { super::looks::surface(&mut img.rgba, out_w, scene.style.paper, scene.style.scanlines, px); }
        img.stats = stats;
        img.pick = pick;
        prof.mark("stats+upscale");
        for (name, ms) in prof.out {
            match self.stages.iter_mut().find(|(n, _)| *n == name) { Some(e) => e.1 += ms, None => self.stages.push((name, ms)) }
        }
        img
    }

    /// One world's frame context: its lights, fog, textures and clock, seen through `view`.
    fn world_ctx<'s>(&mut self, wi: &WorldIn<'s>, view: View, realm: u8, bounds: Option<&'s Bounds>) -> (Ctx<'s>, f32, Option<Strike>, [f32; 3]) {
        let scene = wi.scene;
        let loop_len = scene.motion.loop_length.max(1.0);
        let scroll = wi.distance.rem_euclid(loop_len);
        let phase = scroll / loop_len;
        let loop_seconds = scene.motion.loop_seconds();
        let tphase = match wi.time { Some(t) => (t / loop_seconds.max(1e-3)).rem_euclid(1.0), None => phase };
        let fog = if scene.light.fog.enabled {
            let col = if scene.light.fog.match_sky && scene.sky.enabled { scene.sky.horizon } else { scene.light.fog.color };
            Some((rgb_lin(col), scene.light.fog.distance.max(1.0)))
        } else { None };
        let mut wx = weather::conditions(scene, tphase, loop_seconds);
        let (fog, sun_through) = weather::haze(scene, fog, &mut wx);
        let far = match fog { Some((_, d)) => (d * 5.0).clamp(20.0, 320.0), None => 320.0 };
        let strike = lightning_at(&scene.weather.lightning, tphase, loop_seconds, scene.motion.frames());
        let flash = scale3(rgb_lin(scene.weather.lightning.color), strike.as_ref().map_or(0.0, |s| s.flash) * scene.weather.lightning.intensity.max(0.0));
        let fog_col = add3(fog.map(|f| f.0).unwrap_or(rgb_lin(scene.light.fog.color)), scale3(flash, 0.12));

        let mut sky_lights = Vec::new();
        for (body, gain) in [(&scene.sky.sun, 1.8 * sun_through), (&scene.sky.moon.body, 0.6 * sun_through.sqrt())] {
            if scene.sky.enabled && body.enabled && body.emits_light {
                // As the camera turns with the path, the light comes from that much further round.
                // (A turned camera sees the sky where it always does, the light turning with it,
                // unless the sky stays in the world: see `sky_light_dir`.)
                sky_lights.push(DirLight { dir: sky_light_dir(&view, body.pos), color: scale3(rgb_lin(body.color), gain * body.intensity.max(0.0)) });
            }
        }
        // Lightning lights the world from above, out where the bolt is: tops and the ground catch it,
        // faces toward the camera stay dark against the bright sky.
        if let Some(s) = &strike {
            let dir = [(s.x - 0.5) * 1.1, 1.0, 0.7];
            let n = dot3(dir, dir).sqrt();
            sky_lights.push(DirLight { dir: [dir[0] / n, dir[1] / n, dir[2] / n], color: flash });
        }

        let (d_lo, d_hi, x_reach) = view.footprint(far, 8.0);
        let ctx = Ctx {
            scene, view, loop_len, tphase, scroll, far, d_lo, d_hi, x_reach,
            path_tex: self.textures.get(&scene.path.material),
            verge_tex: self.textures.get(&scene.verge.material),
            wall_tex: self.textures.get(&scene.walls.material),
            ceil_tex: self.textures.get(&scene.ceiling.material),
            path_tile: snap_to_loop(scene.path.material.tile_size, loop_len),
            water: water::water_k(scene, loop_len, snap_to_loop(scene.path.material.tile_size, loop_len), tphase),
            cover: cover::plan(scene, loop_len),
            verge_tile: snap_to_loop(scene.verge.material.tile_size, loop_len),
            wall_tile: snap_to_loop(scene.walls.material.tile_size, loop_len),
            ceil_tile: snap_to_loop(scene.ceiling.material.tile_size, loop_len),
            bridge: Spans::new(&scene.path.bridge, loop_len, &scene.fixtures),
            fork: Forks::new(&scene.path.fork, scene.path.edge_noise, loop_len),
            deck_tex: self.textures.get(&scene.path.bridge.deck),
            bottom_tex: match scene.path.bridge.bottom {
                BridgeBottom::Water => self.textures.get(&Material { pattern: Pattern::Plain, base: scene.path.bridge.bottom_color, ..Material::default() }),
                _ => self.textures.get(if scene.verge.enabled { &scene.verge.material } else { &scene.path.material }),
            },
            deck_tile: snap_to_loop(scene.path.bridge.deck.tile_size, loop_len),
            bottom_tile: snap_to_loop(if scene.verge.enabled { scene.verge.material.tile_size } else { scene.path.material.tile_size }, loop_len),
            ambient: add3(add3(scale3(rgb_lin(scene.light.ambient_color), scene.light.ambient.max(0.0)), scale3(flash, 0.12)), add3(weather::aurora_glow(&scene.sky.aurora), add3(space::tunnel_glow(&scene.sky), blackhole::bh_glow(&scene.sky)))),
            sky_lights, lights: Vec::new(), smokes: Vec::new(), wire_pts: Vec::new(), falls: Vec::new(), fog, fog_col, void_lin: rgb_lin(scene.light.void_color),
            realm, bounds, facade: None, opening: None, portal: None, front: None, verge_beyond: None, gain: 1.0, wx,
        };
        (ctx, loop_seconds, strike, flash)
    }

    /// The average colour (linear) a scene shows over its loop, for the eye's adaptation and for
    /// light falling through an opening into another world.
    pub fn scene_mean(&mut self, scene: &Scene, base_dir: Option<PathBuf>) -> [f32; 3] {
        let key = serde_json::to_string(scene).unwrap_or_default();
        if let Some(m) = self.means.get(&key) { return *m; }
        let mut full = scene.clone();
        full.style.palette = Palette::Full;
        full.style.pixel_size = 1;
        let opts = RenderOptions { size: Some((54, 96)), base_dir, ..RenderOptions::default() };
        let mut acc = [0.0f32; 3];
        let mut count = 0.0f32;
        // Frames this small are quicker on the CPU than a trip to the GPU and back.
        #[cfg(feature = "gpu")]
        let gpu = self.gpu.take();
        for k in 0..4 {
            let img = self.render(&full, full.motion.loop_length * k as f32 / 4.0, &opts);
            for c in img.rgba.chunks_exact(4) {
                acc = add3(acc, rgb_lin([c[0], c[1], c[2]]));
                count += 1.0;
            }
        }
        #[cfg(feature = "gpu")]
        { self.gpu = gpu; }
        let m = scale3(acc, 1.0 / count.max(1.0));
        if self.means.len() > 32 { self.means.clear(); }
        self.means.insert(key, m);
        m
    }

    fn grade_for(&mut self, name: &str, base: Option<&std::path::Path>) -> Option<Arc<super::looks::Grade>> {
        let name = name.trim();
        let key = if name.to_lowercase().ends_with(".cube") {
            let p = std::path::Path::new(name);
            let full = if p.is_absolute() { p.to_path_buf() } else { base.map(|b| b.join(p)).unwrap_or_else(|| p.to_path_buf()) };
            full.display().to_string()
        } else { name.to_owned() };
        if let Some(g) = self.grades.get(&key) { return g.clone(); }
        let g = if key.to_lowercase().ends_with(".cube") {
            match super::looks::Grade::load_cube(std::path::Path::new(&key)) {
                Ok(g) => Some(Arc::new(g)),
                Err(e) => { eprintln!("grade: {e}"); None }
            }
        } else { super::looks::Grade::builtin(&key).map(Arc::new) };
        self.grades.insert(key, g.clone());
        g
    }

    /// The 2nd and 98th percentile luma (0..255) over frames across the loop, before any palette.
    pub fn loop_levels(&mut self, scene: &Scene, base_dir: Option<PathBuf>) -> (f32, f32) {
        let key = serde_json::to_string(scene).unwrap_or_default();
        if let Some((k, l)) = &self.levels { if *k == key { return *l; } }
        let mut full = scene.clone();
        full.style.palette = Palette::Full;
        full.style.dither = crate::scene::Dither::None;
        let size = ((scene.canvas.width / 2).max(16), (scene.canvas.height / 2).max(16));
        let opts = RenderOptions { size: Some(size), base_dir, ..RenderOptions::default() };
        let mut lum: Vec<f32> = Vec::new();
        for k in 0..8 {
            let img = self.render(&full, full.motion.loop_length * k as f32 / 8.0, &opts);
            lum.extend(img.rgba.chunks_exact(4).step_by(3).map(|c| 0.299 * c[0] as f32 + 0.587 * c[1] as f32 + 0.114 * c[2] as f32));
        }
        lum.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let at = |q: f32| lum.get(((lum.len() as f32 - 1.0) * q) as usize).copied().unwrap_or(0.0);
        let l = (at(0.02), at(0.98));
        self.levels = Some((key, l));
        l
    }

    /// The palette an Auto scene uses: the best `n` colours over frames spread across the whole loop,
    /// so every frame (preview, single render, export) shares one palette and nothing flickers.
    pub fn auto_palette(&mut self, scene: &Scene, n: u32, base_dir: Option<PathBuf>) -> Arc<PaletteLut> {
        let key = serde_json::to_string(scene).unwrap_or_default();
        if let Some((k, lut)) = &self.auto_palette { if *k == key { return lut.clone(); } }
        let mut full = scene.clone();
        full.style.palette = Palette::Full;
        let size = ((scene.canvas.width / 2).max(16), (scene.canvas.height / 2).max(16));
        let opts = RenderOptions { size: Some(size), base_dir, ..RenderOptions::default() };
        let mut px: Vec<Rgb> = Vec::new();
        let samples = 8;
        for k in 0..samples {
            let img = self.render(&full, full.motion.loop_length * k as f32 / samples as f32, &opts);
            let step = (img.width * img.height / 6_000).max(1);
            px.extend(img.rgba.chunks_exact(4).step_by(step).map(|c| [c[0], c[1], c[2]]));
        }
        let lut = Arc::new(PaletteLut::new(palette::median_cut(&px, n.clamp(2, 255) as usize)));
        self.auto_palette = Some((key, lut.clone()));
        lut
    }

    fn palette_for(&mut self, scene: &Scene, opts: &RenderOptions) -> Option<Arc<PaletteLut>> {
        let p = &scene.style.palette;
        let colors = match p {
            Palette::Full => return None,
            Palette::Named(n) => palette::named(n)?,
            Palette::Custom(c) => c.clone(),
            Palette::Auto(n) => return Some(self.auto_palette(scene, *n, opts.base_dir.clone())),
        };
        if let Some((key, lut)) = &self.palette {
            if key == p { return Some(lut.clone()); }
        }
        let lut = Arc::new(PaletteLut::new(colors));
        self.palette = Some((p.clone(), lut.clone()));
        Some(lut)
    }

    fn collect_fixtures(&mut self, ctx: &mut Ctx, loop_seconds: f32, opts: &RenderOptions, bills: &mut Vec<Billboard>, flames: &mut Vec<Flame>) {
        let scene = ctx.scene;
        for (fi, f) in scene.fixtures.iter().enumerate().filter(|(_, f)| f.enabled) {
            // Lights that go with a set piece stand where it does: its spacing, and its offset plus their own.
            let (spacing, offset) = match f.place {
                LightPlace::SetPieces => match scene.set_pieces.get(f.set_piece as usize).filter(|p| p.enabled) {
                    Some(p) => (p.spacing.max(1.0), p.offset + f.offset),
                    None => continue,
                },
                _ => (f.spacing, f.offset),
            };
            let f = &Fixture { spacing, offset, ..f.clone() };
            let sp = snap_to_loop(f.spacing, ctx.loop_len);
            let n = (ctx.loop_len / sp).round() as i64;
            let reach = f.radius.max(0.0);
            // What holds it up where no wall does.
            let free = matches!(f.mount, Mount::Ground) || (matches!(f.mount, Mount::Wall) && !scene.walls.enabled);
            let bracketed = matches!(f.kind, FixtureKind::Torch | FixtureKind::GreenFire | FixtureKind::Magic | FixtureKind::IceWisp | FixtureKind::Lantern);
            let post = match f.post {
                _ if !free => LampPost::Off,
                LampPost::Auto if matches!(f.mount, Mount::Wall) => if f.kind == FixtureKind::PaperLantern { LampPost::Crook } else { LampPost::Pole },
                LampPost::Auto => f.kind.natural_post(),
                p => p,
            };
            let post_sprite = (post != LampPost::Off).then(|| self.sprites.post(post));
            let tint = (f.tint != [255, 255, 255]).then(|| rgb_lin(f.tint));
            let pulse = cycles(0.5, loop_seconds);
            // Those shifted along the path can come from the places either side of what is in view.
            let (along, stagger) = (f.jitter_along.clamp(0.0, 1.0), f.stagger.clamp(0.0, 1.0));
            let shifted = along > 0.0 || stagger > 0.0;
            let (lo, hi) = ctx.span(-reach, ctx.far.min(80.0));
            let k_lo = ((ctx.scroll + lo - f.offset) / sp).ceil() as i64 - if shifted { 2 } else { 0 };
            let k_hi = ((ctx.scroll + hi - f.offset) / sp).floor() as i64 + if shifted { 1 } else { 0 };
            let cluster = f.cluster.clamp(1, 8) as i64;
            let (body_h0, mut flame_frac) = fixture_body(f.kind);
            let body = if f.sprite.is_set() {
                let path = f.sprite.pool.first().cloned().unwrap_or_else(|| f.sprite.path.clone());
                self.sprites.file(opts.base_dir.as_deref(), &path)
            } else {
                self.sprites.fixture(f.kind)
            };
            // An image of your own with a painted mask: the light comes from where it glows, in its
            // colour, and nothing is drawn over it but the halo.
            let lamp = body.as_ref().and_then(|b| b.lamp).filter(|_| f.sprite.is_set());
            if let Some(l) = lamp { flame_frac = l.v; }
            let body_h0 = if lamp.is_some() && body_h0 <= 0.0 { 0.6 } else { body_h0 };
            let flick_m1 = cycles(2.3, loop_seconds);
            // How fast a flame's body sways and its tongues lick.
            let (sway_m, lick_m) = (cycles(1.7, loop_seconds), cycles(3.1, loop_seconds));
            let flick_m2 = cycles(5.1, loop_seconds);
            let seed = f.seed.wrapping_add(fi as u32 * 7919);
            // Where they stand: every `spacing`, or on the railing posts of each bridge in view.
            let on_posts = ctx.bridge.filter(|_| f.bridge_posts);
            let scroll = ctx.scroll;
            let (places, n): (Vec<(i64, f32)>, i64) = match on_posts {
                Some(b) => {
                    let per = ((ctx.loop_len / b.period).round() as i64).max(1);
                    let v = b.within(scroll + lo - 1.0, scroll + hi + 1.0).into_iter().flat_map(|(s0, e)| {
                        let kb = ((s0 - b.offset) / b.period).round() as i64;
                        b.post_places(s0, e).into_iter().enumerate().map(move |(j, w)| (kb.rem_euclid(per) * 16 + j as i64, w - scroll))
                    }).collect();
                    (v, per * 16)
                }
                None => ((k_lo..=k_hi).map(|k| (k.rem_euclid(n), k as f32 * sp + f.offset - scroll)).collect(), n),
            };
            // A post on each side of the deck, whatever the side.
            let signs: &[f32] = if on_posts.is_some() && f.side == Side::Center { &[-1.0, 1.0] } else { f.side.signs() };
            // Where each light hangs, for strings of bulbs between neighbours: (side, d, [x, y]).
            let mut anchors: Vec<(usize, f32, [f32; 2])> = Vec::new();
            for (key, d0) in places {
                for (si, &sign) in signs.iter().enumerate() { for c in 0..cluster {
                    // (The first of a cluster keeps the key a lone light has, so old scenes stand as they did.)
                    let skey = key * 3 + si as i64 + c * 3 * n;
                    if f.density < 1.0 && hf(seed ^ 0xD3, skey) >= f.density { continue; }
                    let d = if on_posts.is_some() { d0 } else {
                        d0 + if si == 1 { stagger * sp } else { 0.0 } + if along > 0.0 { (hf(seed ^ 0x9A, skey) - 0.5) * along * sp } else { 0.0 }
                    };
                    if (shifted || on_posts.is_some()) && (d < lo || d > hi) { continue; }
                    match f.place {
                        _ if on_posts.is_some() => {}
                        LightPlace::Bridges if !ctx.on_bridge(d) => continue,
                        LightPlace::OffBridges if ctx.on_bridge(d) => continue,
                        _ => {}
                    }
                    let size = (f.size * (1.0 + (hf(seed ^ 0x5C, skey) * 2.0 - 1.0) * f.size_var.clamp(0.0, 0.9))).max(0.05);
                    let body_h = body_h0 * size;
                    let bright = 1.0 + (hf(seed ^ 0x5D, skey) * 2.0 - 1.0) * f.intensity_var.clamp(0.0, 1.0);
                    // Warmer or cooler than the next one.
                    let cv = 0.5 * f.color_var.clamp(0.0, 1.0);
                    let (ca, cb) = (hf(seed ^ 0x71, skey) * 2.0 - 1.0, hf(seed ^ 0x72, skey) * 2.0 - 1.0);
                    let hue = [1.0 + cv * ca, 1.0 + cv * cb, (1.0 - 0.5 * cv * (ca + cb)).max(0.1)];
                    let jx = (hf(seed ^ 0x51, skey) * 2.0 - 1.0) * f.jitter;
                    let mut jy = (hf(seed ^ 0x77, skey) * 2.0 - 1.0) * f.jitter * 0.5;
                    if f.height_var > 0.0 { jy = (jy + (hf(seed ^ 0x78, skey) * 2.0 - 1.0) * f.height_var).max(0.08 - f.height); }
                    let ph = hf(seed ^ 0x33, skey) * TAU;
                    let wob = if matches!(f.mount, Mount::Floating) {
                        let c = cycles(0.25, loop_seconds);
                        [f.jitter * 0.6 * (TAU * c * ctx.tphase + ph).sin(), f.jitter * 0.3 * (TAU * c * ctx.tphase + ph * 1.7).cos()]
                    } else { [0.0, 0.0] };
                    let hw = ctx.view.path_half_width(d.max(NEAR));
                    let (x, flame_y) = match f.mount {
                        Mount::Wall => {
                            let xw = if scene.walls.enabled { ctx.wall_x(d.max(NEAR)) - 0.06 } else { hw + f.lateral };
                            (sign * xw + if sign == 0.0 { jx } else { 0.0 }, f.height + jy)
                        }
                        Mount::Ground | Mount::Floating => (sign * (hw + f.lateral) + jx + wob[0], f.height + jy + wob[1]),
                        Mount::Ceiling => {
                            let top = if scene.ceiling.enabled { scene.ceiling.height } else if scene.walls.enabled && scene.walls.height > 0.0 { scene.walls.height } else { f.height + 1.0 };
                            (sign * hw * 0.6 + jx, (top - 0.55 * size).min(f.height.max(0.3)))
                        }
                    };
                    if ctx.cut_away(x) { continue; }
                    // Over a bridge's gap there is no ground beside the path: a lamp that stood there
                    // hangs on the railing instead, so the rhythm of light carries across.
                    if matches!(f.mount, Mount::Wall) && scene.walls.enabled && sign != 0.0
                        && ctx.fork.is_some_and(|fk| fk.opens(ctx.scroll + d, sign)) { continue; }
                    let standing = matches!(f.mount, Mount::Ground) || (matches!(f.mount, Mount::Wall) && !scene.walls.enabled);
                    let x = if standing && x.abs() > hw && ctx.on_bridge(d) { x.signum() * (hw - 0.5 * ctx.bridge.map_or(0.0, |b| b.rail_inset())) } else { x };
                    // On a post: in its middle, above its top.
                    let (x, flame_y) = match on_posts {
                        Some(b) => {
                            let (out, top) = b.post_reach();
                            (sign * (hw + out) + jx, flame_y.max(top + 0.25 * size))
                        }
                        None => (x, flame_y),
                    };
                    if !ctx.owns(x, 0.0, d) { continue; }
                    // A flame gutters; glass and paper round one steady it; a beacon comes and goes.
                    let steady = match f.kind { FixtureKind::StreetLamp | FixtureKind::Glowshroom => 0.12, FixtureKind::PaperLantern => 0.4, _ => 1.0 };
                    let flicker = if f.kind == FixtureKind::Beacon {
                        0.12 + 1.5 * (TAU * pulse * ctx.tphase + ph).sin().max(0.0).powi(6)
                    } else if f.kind == FixtureKind::Neon {
                        // A neon tube hums steady, then stutters off and on now and then.
                        let buzz = (cycles(0.6, loop_seconds) * ctx.tphase + ph / TAU).rem_euclid(1.0);
                        let stutter = if f.flicker > 0.0 && buzz < 0.04 * f.flicker.min(2.0) { 0.15 + 0.6 * (TAU * 9.0 * buzz / 0.04).sin().abs() } else { 1.0 };
                        stutter * (1.0 + 0.015 * (TAU * flick_m2 * ctx.tphase + ph).sin())
                    } else {
                        1.0 + steady * f.flicker * (0.12 * (TAU * flick_m1 * ctx.tphase + ph).sin() + 0.06 * (TAU * flick_m2 * ctx.tphase + ph * 2.3).sin())
                    };
                    // (Dimming as a swinging camera dissolves its side of the world.)
                    let mut glow = scale3(mul3(lamp.map_or(rgb_lin(f.kind.glow()), |l| l.color), hue), bright * (1.0 - ctx.fading(x)));
                    if let Some(t) = tint { glow = mul3(glow, t); }
                    // (Its own lamp does not light its post: that is in the lamp's shadow, and reads dark against the glow.)
                    // Where the glow sits across an image of your own (mirrored on the right-hand wall).
                    let lamp_dx = lamp.zip(body.as_ref()).map_or(0.0, |(l, b)| {
                        let mirrored = sign > 0.0 && (matches!(f.mount, Mount::Wall) || (bracketed && post == LampPost::Pole));
                        (if mirrored { 0.5 - l.u } else { l.u - 0.5 }) * b.aspect * body_h
                    });
                    let mut own_light = None;
                    if f.light && f.intensity > 0.0 {
                        own_light = Some(ctx.lights.len());
                        ctx.lights.push(PointLight {
                            pos: ctx.view.to_cam(x + lamp_dx, flame_y, d),
                            color: scale3(glow, f.intensity * 2.0 * flicker),
                            radius: reach.max(0.5),
                        });
                    }
                    if f.string_lights.enabled { anchors.push((si, d, [x + lamp_dx, flame_y])); }
                    // (A turned camera sees the path behind where it stands too: what is in front of
                    // the camera is what counts there.)
                    if if ctx.view.turned { ctx.view.depth_of(x, d) < NEAR } else { d < NEAR } { continue; }
                    // Fixture body, placed so its flame point sits at the flame height.
                    let mut body_w = 0.0;
                    if let Some(sprite) = &body {
                        if body_h > 0.0 {
                            let top = flame_y + body_h * flame_frac;
                            let mut base = top - body_h;
                            body_w = sprite.aspect * body_h;
                            // Outwards from the path: where a bracket's plate is, and a post stands.
                            let out = if sign < 0.0 { -1.0 } else { 1.0 };
                            let on_pole = bracketed && post == LampPost::Pole;
                            if let Some(ps) = &post_sprite {
                                let source = PICK_FIXTURES | fi as u16;
                                match post {
                                    // Hanging from the arm's end, the post standing further out.
                                    LampPost::Crook => {
                                        let hgt = (top + 0.04) / (1.0 - super::sprites::CROOK_HOOK[1]);
                                        let xc = x + out * (0.5 - super::sprites::CROOK_HOOK[0]) * ps.aspect * hgt;
                                        bills.push(Billboard { source, own_light, flip: out < 0.0, ..post_billboard(ctx, ps, xc, hgt, d, size) });
                                    }
                                    // Beside a bracketed light, at its plate; under any other.
                                    LampPost::Pole if bracketed => bills.push(Billboard { source, own_light, ..post_billboard(ctx, ps, x + out * 0.5 * body_w, top + 0.12, d, size) }),
                                    _ if base > 0.05 => bills.push(Billboard { source, own_light: if post == LampPost::Pedestal { None } else { own_light }, ..post_billboard(ctx, ps, x, base, d, size) }),
                                    _ => {}
                                }
                            }
                            if base < 0.0 && matches!(f.mount, Mount::Ground) { base = 0.0; }
                            let z = ctx.view.to_cam(x, base, d)[2];
                            // Glass, paper and caps that are lit from inside show it.
                            let emissive = match f.kind {
                                FixtureKind::Crystal => 1.2, FixtureKind::Glowshroom => 1.0, FixtureKind::Beacon => 0.05 + flicker.min(1.3),
                                // (Street lamps, lanterns, torches, braziers and campfires glow where
                                // their paintings' glow maps say: their panes, coals and embers.)
                                FixtureKind::PaperLantern => 0.9, _ => 0.0,
                            };
                            bills.push(Billboard {
                                fade: 0.0, z, world: [x, base, d], height: body_h, sprite: sprite.clone(),
                                flip: sign > 0.0 && (matches!(f.mount, Mount::Wall) || on_pole), nearest: f.sprite.pixelated,
                                id: id::FIXTURE, emissive, shadow: 0.0, own_light: None, source: PICK_FIXTURES | fi as u16, realm: ctx.realm, panel: None, tube: None, sway: 0.0,
                                // A neon sign's tubes take its colour, and stutter with it.
                                tint: if f.kind == FixtureKind::Neon && !f.sprite.is_set() { let g = glow; let m = g[0].max(g[1]).max(g[2]).max(1e-3); scale3(g, flicker.min(1.2) / m) } else { [1.0; 3] },
                                wind: CardWind::default() });
                        }
                    }
                    let flame_size = match f.kind {
                        FixtureKind::Candle => 0.45, FixtureKind::Brazier => 1.8, FixtureKind::Firefly => 0.35, FixtureKind::Crystal => 1.4,
                        FixtureKind::Campfire => 2.4, FixtureKind::StreetLamp => 1.5, FixtureKind::PaperLantern => 1.3, FixtureKind::Glowshroom => 0.9,
                        FixtureKind::Chandelier => 0.4, _ => 1.0,
                    } * size;
                    let colors = f.kind.flame().map(|cs| {
                        if cv > 0.0 || tint.is_some() {
                            let t = tint.map_or([1.0; 3], |_| [0, 1, 2].map(|i| f.tint[i] as f32 / 255.0));
                            cs.map(|c| [0, 1, 2].map(|i| (c[i] as f32 * hue[i] * t[i]).min(255.0) as u8))
                        } else { cs }
                    });
                    let glow_f = scale3(glow, f.intensity.max(0.0));
                    let phase = [TAU * sway_m * ctx.tphase + ph, TAU * lick_m * ctx.tphase + ph * 1.7];
                    let fseed = hash(seed ^ 0xE3, skey) as u32;
                    let embers = match f.kind { FixtureKind::Torch | FixtureKind::GreenFire => 3, FixtureKind::Brazier => 5, FixtureKind::Campfire => 7, _ => 0 };
                    if f.kind == FixtureKind::Chandelier && body_w > 0.0 {
                        // A flame on every candle round the ring.
                        for [u, v] in super::sprites::chandelier_flames() {
                            flames.push(Flame { halo_only: false, world: [x + u * body_w, flame_y + v * body_h, d], size: flame_size, colors, glow: scale3(glow_f, 0.45), flicker, phase: [phase[0] + 7.0 * u, phase[1] + 5.0 * v], embers: 0, seed: fseed, realm: ctx.realm });
                        }
                        continue;
                    }
                    flames.push(Flame { halo_only: lamp.is_some() || f.kind == FixtureKind::Neon, world: [x + lamp_dx, flame_y, d], size: flame_size, colors, glow: glow_f, flicker, phase, embers, seed: fseed, realm: ctx.realm });
                } }
            }
            if f.string_lights.enabled && f.string_lights.bulbs > 0 {
                let sl = &f.string_lights;
                anchors.sort_by(|a, b| (a.0, a.1).partial_cmp(&(b.0, b.1)).unwrap_or(std::cmp::Ordering::Equal));
                let tw = cycles(0.7, loop_seconds);
                for pair in anchors.windows(2) {
                    let ((sa, da, pa), (sb, db, pb)) = (pair[0], pair[1]);
                    // Only between neighbours on the same side.
                    if sa != sb || db - da > 1.6 * sp + 0.5 || db <= da { continue; }
                    let at = |t: f32| [pa[0] + (pb[0] - pa[0]) * t, pa[1] + (pb[1] - pa[1]) * t - sl.sag * 4.0 * t * (1.0 - t), da + (db - da) * t];
                    // The wire, as close dots along its curve.
                    let span = ((pb[0] - pa[0]).powi(2) + (db - da).powi(2)).sqrt();
                    let steps = (span / 0.12).ceil().clamp(4.0, 400.0) as usize;
                    for k in 0..=steps { ctx.wire_pts.push(at(k as f32 / steps as f32)); }
                    let n_b = sl.bulbs.min(64);
                    for k in 1..=n_b {
                        let t = k as f32 / (n_b + 1) as f32;
                        let p = at(t);
                        let idx = ((da * 7.0).round() as i64).wrapping_mul(64) + k as i64;
                        let col = if sl.colors.is_empty() { sl.color } else { sl.colors[(k as usize - 1) % sl.colors.len()] };
                        let twk = 1.0 + sl.twinkle.clamp(0.0, 1.0) * 0.5 * (TAU * (tw * ctx.tphase + hf(seed ^ 0x5B, idx))).sin();
                        flames.push(Flame { halo_only: false, world: [p[0], p[1] - 0.05, p[2]], size: 0.35 * sl.size.max(0.05), colors: None,
                            glow: scale3(rgb_lin(col), 0.6 * sl.brightness.max(0.0)), flicker: twk, phase: [0.0; 2], embers: 0, seed: 0, realm: ctx.realm });
                    }
                }
            }
        }
    }

    /// Waterfalls beside the path: their rock faces (cards), and where the water falls for the
    /// splat pass (`draw_falls`).
    fn collect_waterfalls(&mut self, ctx: &mut Ctx, bills: &mut Vec<Billboard>) {
        let scene = ctx.scene;
        for (wi, wf) in scene.waterfalls.iter().enumerate().filter(|(_, w)| w.enabled && w.height > 0.0) {
            let sp = snap_to_loop(wf.spacing.max(5.0), ctx.loop_len);
            let (lo, hi) = ctx.span(-2.0, ctx.far.min(150.0));
            let k_lo = ((ctx.scroll + lo - wf.offset) / sp).ceil() as i64;
            let k_hi = ((ctx.scroll + hi - wf.offset) / sp).floor() as i64;
            let n = (ctx.loop_len / sp).round() as i64;
            for k in k_lo..=k_hi {
                let d = k as f32 * sp + wf.offset - ctx.scroll;
                if !ctx.view.turned && d < NEAR { continue; }
                for &sign in wf.side.signs() {
                    let x = sign * (ctx.view.path_half_width(d) + wf.lateral + 0.5 * wf.width);
                    if ctx.cut_away(x - sign * 0.5 * wf.width) || ctx.view.depth_of(x, d) < NEAR { continue; }
                    let h = wf.height.max(0.5);
                    let sprite = self.sprites.cliff(wf.rock, hash(wf.seed ^ 0xC2, k.rem_euclid(n)) as u32);
                    let base = -0.05;
                    // The rock face stands just behind the water, as the camera sees it.
                    let (rx, rd) = ctx.view.toward_camera(x, d, -0.3);
                    let z = ctx.view.to_cam(rx, base, rd)[2];
                    bills.push(Billboard {
                        fade: 0.0, z, world: [rx, base, rd], height: h * 1.25, sprite, sway: 0.0, tint: [1.0; 3], wind: CardWind::default(),
                        flip: sign > 0.0, nearest: false, id: id::PROP, emissive: 0.0, shadow: 0.0, own_light: None, source: 0, realm: ctx.realm, panel: None, tube: None,
                    });
                    ctx.falls.push(([x, h * 1.05, d], wf.width, wi));
                }
            }
        }
    }

    /// Structures spanning the path. They are flat cards across it, which is exactly right for an
    /// arch or a gate seen by a camera that always looks down the path.
    fn collect_set_pieces(&mut self, ctx: &mut Ctx, opts: &RenderOptions, bills: &mut Vec<Billboard>) {
        let scene = ctx.scene;
        let far = ctx.far.min(150.0);
        for (li, p) in scene.set_pieces.iter().enumerate().filter(|(_, p)| p.enabled) {
            let sp = snap_to_loop(p.spacing.max(1.0), ctx.loop_len);
            let n = (ctx.loop_len / sp).round() as i64;
            // (A card turned to the camera: seen from the side it stands face on where the path
            // goes through it, as a side-scroller shows a gate, rather than edge on.)
            // Portals light the path a little before they come into view.
            let (lo, hi) = ctx.span(-6.0, far);
            let k_lo = ((ctx.scroll + lo - p.offset) / sp).ceil() as i64;
            let k_hi = ((ctx.scroll + hi - p.offset) / sp).floor() as i64;
            let seed = p.seed.wrapping_add(li as u32 * 7_919);
            for k in k_lo..=k_hi {
                let d = k as f32 * sp + p.offset - ctx.scroll;
                let variant = hash(seed ^ 0x5E7, k.rem_euclid(n));
                let ow = if p.width > 0.0 { p.width } else {
                    // Between walls a stone arch or gate keeps part of each pier clear of them, or
                    // the walls would hide the piers and leave its ring standing on nothing.
                    let pier = if matches!(p.kind, SetPieceKind::Archway | SetPieceKind::RuinedArch | SetPieceKind::Gate) { 0.55 } else { 0.05 };
                    let side = if scene.walls.enabled { let w = ctx.wall_x(d); (w - pier).max(0.6 * w) } else { ctx.view.path_half_width(d) + 0.6 };
                    2.0 * side
                };
                let oh = p.height.max(1.0);
                if !ctx.owns(0.0, 0.0, d) { continue; }
                if p.kind == SetPieceKind::Portal {
                    ctx.lights.push(PointLight {
                        pos: ctx.view.to_cam(0.0, oh * 0.5, d),
                        color: scale3(rgb_lin(p.accent), 1.6),
                        radius: (ow + oh).max(4.0),
                    });
                }
                if ctx.view.depth_of(0.0, d) < NEAR + 0.2 { continue; }
                let file = p.sprite.path.trim();
                let (sprite, height) = if !file.is_empty() {
                    match self.sprites.file(opts.base_dir.as_deref(), file) {
                        Some(sp) => { let h = (ow + 1.0) / sp.aspect.max(0.05); (sp, h) }
                        None => { let (sp, _, h) = self.sprites.set_piece(p.kind, p.tint, p.accent, ow, oh, variant); (sp, h) }
                    }
                } else {
                    let (sp, _, h) = self.sprites.set_piece(p.kind, p.tint, p.accent, ow, oh, variant);
                    (sp, h)
                };
                let z = ctx.view.to_cam(0.0, 0.0, d)[2];
                // Banners stir in the wind; arches and gates stand firm.
                let sway = if p.kind == SetPieceKind::Banners { ctx.wx.sway_at(hf(seed ^ 0x5E8, k.rem_euclid(n)) * TAU) * 0.05 * height } else { 0.0 };
                bills.push(Billboard {
                    fade: 0.0, z, world: [0.0, 0.0, d], height, sprite, flip: false, nearest: p.sprite.pixelated, id: id::PROP,
                    emissive: if p.kind == SetPieceKind::Portal { 0.9 } else { 0.0 },
                    shadow: p.shadow.clamp(0.0, 1.0), own_light: None, source: PICK_SET_PIECES | li as u16, realm: ctx.realm, panel: None, tube: None, sway, tint: [1.0; 3], wind: CardWind::default() });
            }
        }
    }

    /// Things flying along with the camera (`Companion`): at a fixed distance ahead, so they keep
    /// their place in the frame and follow the path round its bends.
    fn collect_companions(&mut self, ctx: &mut Ctx, opts: &RenderOptions, bills: &mut Vec<Billboard>) {
        let scene = ctx.scene;
        for (ci, c) in scene.companions.iter().enumerate().filter(|(_, c)| c.enabled && c.size > 0.0) {
            let ph = hf(c.seed ^ 0xC0, ci as i64) * TAU;
            let x = c.offset[0] + c.weave * (TAU * c.weave_cycles as f32 * ctx.tphase + ph).sin();
            let y = c.offset[1] + c.bob * (TAU * c.bob_cycles as f32 * ctx.tphase + 1.7 * ph).sin();
            let d = c.offset[2];
            if d < NEAR + 0.2 || !ctx.owns(x, y, d) { continue; }
            let file = c.sprite.path.trim();
            let drawn = if file.is_empty() { None } else {
                let p = std::path::Path::new(file);
                let full = match opts.base_dir.as_deref() { Some(b) if p.is_relative() => b.join(p), _ => p.to_path_buf() };
                if c.glow_from > 0.0 { self.sprites.file_glowing(&full, true, c.glow_from, 1.5) } else { self.sprites.file_at(&full, true) }
            };
            // The engines' height on the card, as a share of its height.
            let (sprite, height, engines_at) = match drawn {
                Some(sp) => (sp, c.size, 0.4),
                None => match self.sprites.shape(&ship_parts(c)) {
                    Some((sp, h)) => (sp, h, 0.27 * c.size * 0.5 / h),
                    None => continue,
                },
            };
            let base = y - height * engines_at;
            let mut own_light = None;
            if c.light > 0.0 {
                own_light = Some(ctx.lights.len());
                ctx.lights.push(PointLight {
                    pos: ctx.view.to_cam(x, y, d - 0.4),
                    color: scale3(rgb_lin(c.engine), 2.0 * c.light),
                    radius: 5.0 + 3.0 * c.light,
                });
            }
            let z = ctx.view.to_cam(x, base, d)[2];
            bills.push(Billboard {
                fade: 0.0, z, world: [x, base, d], height, sprite, flip: c.sprite.flip_x, nearest: c.sprite.pixelated, id: id::PROP,
                emissive: 0.0, shadow: 0.0, own_light, source: PICK_COMPANIONS | ci as u16, realm: ctx.realm, panel: None, tube: None, sway: 0.0, tint: [1.0; 3], wind: CardWind::default(),
            });
        }
    }

    fn collect_props(&mut self, ctx: &mut Ctx, opts: &RenderOptions, bills: &mut Vec<Billboard>) {
        let scene = ctx.scene;
        let far = ctx.far.min(150.0);
        let loop_seconds = scene.motion.loop_seconds();
        let (flick_m1, flick_m2) = (cycles(2.3, loop_seconds), cycles(5.1, loop_seconds));
        // Hanging things (icicles) start at the ceiling, or the top of the walls; with neither there
        // is nothing to hang from, so they stand like the rest.
        let ceiling = if scene.ceiling.enabled { Some(scene.ceiling.height) } else if scene.walls.enabled && scene.walls.height > 0.0 { Some(scene.walls.height) } else { None };
        for (li, p) in scene.props.iter().enumerate().filter(|(_, p)| p.enabled) {
            let Some(group) = self.defs.resolve(scene, opts.base_dir.as_deref(), p, &mut self.sprites).0 else { continue };
            let sp = snap_to_loop(p.spacing, ctx.loop_len);
            let n = (ctx.loop_len / sp).round() as i64;
            // (One place more at each end: a prop shifted along the path can stand in view when its place does not.)
            let (lo, hi) = ctx.span(NEAR, far);
            let k_lo = ((ctx.scroll + lo - p.offset) / sp).ceil() as i64 - 2;
            let k_hi = ((ctx.scroll + hi - p.offset) / sp).floor() as i64 + 1;
            let (along, stagger) = (p.jitter_along.clamp(0.0, 1.0), p.stagger.clamp(0.0, 1.0));
            let seed = p.seed.wrapping_add(li as u32 * 104_729);
            for k in k_lo..=k_hi {
                let key = k.rem_euclid(n);
                let d0 = k as f32 * sp + p.offset - ctx.scroll;
                for (si, &sign) in p.side.signs().iter().enumerate() {
                    for row in 0..p.rows.max(1) {
                        let ikey = (key * 4 + si as i64) * 64 + row as i64;
                        if hf(seed ^ 0xD3, ikey) > p.density { continue; }
                        // Which of a group's definitions this one is (the same each time round the loop).
                        let look = group.pick(hash(seed ^ 0x6B, ikey));
                        let hanging = look.hanging && ceiling.is_some();
                        let light = look.light.as_ref().filter(|l| l.intensity > 0.0 && l.radius > 0.0);
                        // Rows further out are staggered along the path so they do not line up.
                        let shift = if row > 0 { along.max(0.8) } else { along };
                        let d = d0 + if si == 1 { stagger * sp } else { 0.0 } + if shift > 0.0 { (hf(seed ^ 0x9A, ikey) - 0.5) * sp * shift } else { 0.0 };
                        if !ctx.view.turned && (d < NEAR || d > far) { continue; }
                        let jit = (hf(seed ^ 0x1F, ikey) * 2.0 - 1.0) * p.jitter;
                        let edge = ctx.view.path_half_width(d);
                        // Round the path on a ring: each strand winds along it as a screw's thread does, and
                        // the whole turns, in whole turns per loop both ways (a wormhole's own, if asked).
                        let ring = p.orbit.enabled.then(|| {
                            let o = &p.orbit;
                            let tn = &scene.sky.tunnel;
                            let (twist, spin) = if o.with_tunnel && scene.sky.enabled && tn.enabled { (tn.twist, tn.spin) } else { (o.twist, o.spin) };
                            let rows = p.rows.max(1);
                            let strand = (si as u32 * rows + row) as f32 / (p.side.signs().len() as u32 * rows) as f32;
                            let a = strand + (hf(seed ^ 0xB7, ikey) * 2.0 - 1.0) * o.stray.clamp(0.0, 0.5)
                                - spin as f32 * ctx.tphase - twist as f32 * (k as f32 * sp + p.offset) / ctx.loop_len;
                            let rad = (o.radius + (hf(seed ^ 0xB8, ikey) * 2.0 - 1.0) * o.radius_var).max(0.2);
                            let cy = if o.height > 0.0 { o.height } else { scene.camera.eye_height };
                            (rad * (TAU * a).cos(), cy - rad * (TAU * a).sin())
                        });
                        let x = match ring {
                            Some((rx, _)) => rx,
                            None if sign == 0.0 => p.lateral + jit,
                            None => sign * (edge + p.lateral + row as f32 * p.row_spacing + jit),
                        };
                        if ctx.view.turned && (ctx.cut_away(x) || ctx.view.depth_of(x, d) < NEAR || d < lo || d > hi) { continue; }
                        // Nothing stands beside the deck of a bridge: the ground there has fallen away.
                        let floating = p.float > 0.0 || ring.is_some();
                        if ctx.on_bridge(d) && x.abs() > edge - 0.05 && !floating { continue; }
                        // Nor on a branch path, nor in the mouth of a side passage.
                        if ring.is_none() && ctx.on_fork(x, d) { continue; }
                        if !ctx.owns(x, 0.0, d) { continue; }
                        let sc = p.scale.max(0.01) * (1.0 + (hf(seed ^ 0x5C, ikey) * 2.0 - 1.0) * p.scale_var.clamp(0.0, 0.95));
                        let height = look.height * sc;
                        let variant = hash(seed ^ 0x7E, ikey);
                        let floating_rock = floating && matches!(look.kind, PropKind::Rock | PropKind::Boulder);
                        let sprite = if look.sprites.is_empty() && floating_rock {
                            self.sprites.asteroid(look.tint, variant)
                        } else if look.sprites.is_empty() {
                            self.sprites.prop(look.kind, look.tint, variant)
                        } else {
                            look.sprites[(variant as usize) % look.sprites.len()].clone()
                        };
                        let base = match ceiling {
                            _ if ring.is_some() => ring.map_or(0.0, |(_, ry)| ry) - 0.5 * height,
                            Some(top) if hanging => top - height,
                            _ if floating => p.float + (hf(seed ^ 0xF1, ikey) * 2.0 - 1.0) * p.float_var.max(0.0) - 0.5 * height,
                            _ => -p.sink.clamp(0.0, 0.9) * height,
                        };
                        // Standing along the path, its front to it (a building), not turned to the camera.
                        let facing = p.facing.unwrap_or(look.facing);
                        let stands = sign != 0.0 && ring.is_none() && !floating && !hanging
                            && (facing == Facing::Path || (facing == Facing::PathWhenTurned && ctx.view.turned));
                        // Facing the path once the camera turns: while it swings round (`View::face_k`)
                        // the prop comes round with it about its foot, from facing the camera to
                        // facing the path. `spin` turns an offset (x, d) from its foot that way.
                        let (rc, rs) = if stands && facing == Facing::PathWhenTurned && ctx.view.face_k < 1.0 {
                            let (np, nc) = ([-sign, 0.0], [-ctx.view.yaw_s, -ctx.view.yaw_c]);
                            let a = (np[0] * nc[1] - np[1] * nc[0]).atan2(np[0] * nc[0] + np[1] * nc[1]) * (1.0 - ctx.view.face_k);
                            (a.cos(), a.sin())
                        } else { (1.0, 0.0) };
                        let spin = |dx: f32, dd: f32| [dx * rc - dd * rs, dx * rs + dd * rc];
                        let mut own_light = None;
                        // An image whose mask marks a glowing part lights what is round it, from there.
                        if let (None, Some(lm), true) = (light, sprite.lamp, d <= 60.0) {
                            own_light = Some(ctx.lights.len());
                            let ph = hf(seed ^ 0x33, ikey) * TAU;
                            let flicker = 1.0 + 0.5 * (0.12 * (TAU * flick_m1 * ctx.tphase + ph).sin() + 0.06 * (TAU * flick_m2 * ctx.tphase + ph * 2.3).sin());
                            let flipped = if look.from_files { look.flip_x } else { variant & 0x100 != 0 };
                            let u = if flipped { 1.0 - lm.u } else { lm.u };
                            ctx.lights.push(PointLight {
                                pos: ctx.view.to_cam(x + (u - 0.5) * sprite.aspect * height, base + height * (1.0 - lm.v), d),
                                color: scale3(lm.color, 2.0 * flicker),
                                radius: (2.5 * height).clamp(2.5, 10.0),
                            });
                        }
                        if let Some(l) = light {
                            // Lamps far down the path light nothing the camera can make out.
                            if d <= 60.0 {
                                own_light = Some(ctx.lights.len());
                                let ph = hf(seed ^ 0x33, ikey) * TAU;
                                let flicker = 1.0 + l.flicker.clamp(0.0, 2.0) * (0.12 * (TAU * flick_m1 * ctx.tphase + ph).sin() + 0.06 * (TAU * flick_m2 * ctx.tphase + ph * 2.3).sin());
                                ctx.lights.push(PointLight {
                                    pos: ctx.view.to_cam(x, base + height * l.at.clamp(0.0, 1.5), d),
                                    color: scale3(rgb_lin(l.color), l.intensity * 2.0 * flicker),
                                    radius: l.radius.max(0.5),
                                });
                            }
                        }
                        if let Some(sm) = &look.smoke {
                            let flipped = if look.from_files { look.flip_x } else { variant & 0x100 != 0 };
                            let k = height / look.natural;
                            let sx = if flipped { -sm.at[0] } else { sm.at[0] } * k;
                            // (Standing along the path, its front runs along the path.)
                            let at = if stands { let o = spin(sign * sm.back.max(0.0) * k, -sign * sx); [x + o[0], base + sm.at[1] * k, d + o[1]] } else { [x + sx, base + sm.at[1] * k, d] };
                            ctx.smokes.push((at, sm.clone(), hash(seed ^ 0x5E0, ikey) as u32));
                        }
                        let z = ctx.view.to_cam(x, base, d)[2];
                        let mut sway = if hanging { 0.0 } else { ctx.wx.sway_at(hf(seed ^ 0x3C, ikey) * TAU) * weather::sway_factor(look.kind) * height };
                        if p.lean_var > 0.0 && !hanging { sway += (hf(seed ^ 0x3E, ikey) * 2.0 - 1.0) * p.lean_var.min(0.3) * height; }
                        // No two alike: each a little lighter or darker, warmer or cooler.
                        let tv = p.tint_var.clamp(0.0, 0.6);
                        let (ta, tb) = (hf(seed ^ 0x71, ikey) * 2.0 - 1.0, hf(seed ^ 0x72, ikey) * 2.0 - 1.0);
                        let mut tint = [1.0 + tv * (ta + 0.5 * tb), 1.0 + tv * ta, 1.0 + tv * (ta - 0.5 * tb)];
                        // Hanging in a tunnel it is lit by the walls all round it, far more than the path
                        // beneath is: that light is added to whatever else falls on it.
                        let tn = &scene.sky.tunnel;
                        if floating && scene.sky.enabled && tn.enabled && tn.intensity > 0.0 {
                            let wall = scale3(mix3(rgb_lin(tn.colors[1]), rgb_lin(tn.colors[0]), 0.4), 0.9 * tn.intensity);
                            let here = ctx.light_except(x, base + 0.5 * height, d, None, 0, weather::cloud_shade(ctx, x, d), own_light);
                            for i in 0..3 { tint[i] *= 1.0 + wall[i] / here[i].max(1e-3); }
                        }
                        let wind = if hanging || look.from_files { CardWind::default() } else { ctx.wx.card_wind(look.kind, hf(seed ^ 0x3D, ikey) * TAU, sprite.aspect) };
                        let flip = if look.from_files { look.flip_x } else { variant & 0x100 != 0 };
                        let wide = height * sprite.aspect;
                        let panel = stands.then(|| {
                            let (o, a) = (spin(0.0, sign * 0.5 * wide), spin(0.0, -sign * wide));
                            Panel { tl: [x + o[0], base + height, d + o[1]], across: [a[0], 0.0, a[1]], down: [0.0, -height, 0.0], strip: None }
                        });
                        let shadow = if p.shadow && !hanging && !floating { p.shadow_opacity.clamp(0.0, 1.0) } else { 0.0 };
                        let source = PICK_PROPS | li as u16;
                        let face = |sprite: Arc<Sprite>, tl: [f32; 3], across: [f32; 3], down: [f32; 3], strip: Option<f32>| {
                            let mid = [tl[0] + 0.5 * (across[0] + down[0]), tl[1] + 0.5 * (across[1] + down[1]), tl[2] + 0.5 * (across[2] + down[2])];
                            let len = |a: [f32; 3]| (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
                            Billboard {
                                fade: 0.0, z: ctx.view.to_cam(mid[0], mid[1], mid[2])[2], world: mid, height: len(down), sprite, sway: 0.0, tint, wind: CardWind::default(), flip: false,
                                nearest: look.pixelated, id: id::PROP, emissive: look.glow, shadow, own_light, source, realm: ctx.realm,
                                panel: Some(Panel { tl, across, down, strip }), tube: None,
                            }
                        };
                        // The rest of it: its own faces, placed from its space (x across its front, y up,
                        // z back from the path) into the world, mirrored with it.
                        if stands && !look.faces.is_empty() {
                            let k = height / look.natural;
                            let fx = if flip { -1.0 } else { 1.0 };
                            let at = |c: [f32; 3]| { let o = spin(sign * c[2] * k, -sign * fx * c[0] * k); [x + o[0], base + c[1] * k, d + o[1]] };
                            for f in &look.faces {
                                let [tl, tr, bl] = f.corners.map(at);
                                let sub = |a: [f32; 3], b: [f32; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
                                bills.push(face(f.sprite.clone(), tl, sub(tr, tl), sub(bl, tl), None));
                            }
                        }
                        if stands && !look.tubes.is_empty() {
                            let k = height / look.natural;
                            let fx = if flip { -1.0 } else { 1.0 };
                            let at = |c: [f32; 3]| { let o = spin(sign * c[2] * k, -sign * fx * c[0] * k); [x + o[0], base + c[1] * k, d + o[1]] };
                            for &(f, t, rad, bark, end) in &look.tubes {
                                let (a, b) = (at(f), at(t));
                                let mid = [0.5 * (a[0] + b[0]), 0.5 * (a[1] + b[1]), 0.5 * (a[2] + b[2])];
                                bills.push(Billboard {
                                    fade: 0.0, z: ctx.view.to_cam(mid[0], mid[1], mid[2])[2], world: mid, height: 2.0 * rad * k, sprite: sprite.clone(), sway: 0.0, tint,
                                    wind: CardWind::default(), flip: false, nearest: false, id: id::PROP, emissive: 0.0, shadow: 0.0, own_light, source, realm: ctx.realm,
                                    panel: None, tube: Some(Tube { a, b, r: rad * k, bark, end }),
                                });
                            }
                        }
                        if let Some(pn) = panel.filter(|_| look.faces.is_empty()) {
                            // Or with depth, its ends are walls that deep, drawn from the edges of its picture.
                            let depth = p.depth.unwrap_or(look.depth).max(0.0);
                            if depth > 0.01 {
                                for (end, strip) in [(0.0f32, 0.012f32), (1.0, 0.988)] {
                                    let strip = if flip { 1.0 - strip } else { strip };
                                    let tl = [pn.tl[0] + end * pn.across[0], pn.tl[1], pn.tl[2] + end * pn.across[2]];
                                    let back = spin(sign * depth, 0.0);
                                    bills.push(face(sprite.clone(), tl, [back[0], 0.0, back[1]], pn.down, Some(strip)));
                                }
                            }
                        }
                        let (sway, wind) = if panel.is_some() { (0.0, CardWind::default()) } else { (sway, wind) };
                        bills.push(Billboard {
                            fade: 0.0, z, world: [x, base, d], height, sprite, sway, tint, wind, flip,
                            nearest: look.pixelated, id: id::PROP, emissive: look.glow,
                            shadow, own_light, source, realm: ctx.realm, panel, tube: None });
                    }
                }
            }
        }
    }
}

/// A thin iron post under a ground-mounted fixture.
/// Columns either side and rows above the frame that glossy floors can reflect: the deepest
/// mirror row (from the bottom row of the frame), and the widest sideways wobble ripples give.
/// (0, 0) when nothing in the scene is glossy. A rough surface's blur may reach past the top row;
/// it repeats that row, distant sky by then, rather than rendering more.
fn reflection_guard(scene: &Scene, v: &View, layers: &Layers) -> (usize, usize) {
    if !layers.ground { return (0, 0); }
    let mut mats: Vec<(f32, f32)> = vec![(scene.path.material.gloss, scene.path.material.ripples)];
    if scene.verge.enabled { mats.push((scene.verge.material.gloss, scene.verge.material.ripples)); }
    if scene.path.bridge.enabled {
        mats.push((scene.path.bridge.deck.gloss, scene.path.bridge.deck.ripples));
        if scene.path.bridge.bottom == BridgeBottom::Water { mats.push((1.0, 0.3)); }
    }
    // Rain makes any floor glossy, and puddles are still water with rings.
    let wx = weather::conditions(scene, 0.0, scene.motion.loop_seconds());
    if wx.wet > 0.0 { mats.push((0.32 * wx.wet, 0.35 * wx.wet)); }
    if wx.puddles > 0.0 { mats.push((0.9, 0.3)); }
    let (mut rip, mut any) = (0.0f32, false);
    for (g, r) in mats {
        if g.clamp(0.0, 1.0) <= 0.001 { continue; }
        any = true;
        rip = rip.max(r.clamp(0.0, 1.0));
    }
    if !any { return (0, 0); }
    // The same quantities reflect_pass uses, at the bottom row.
    let yc = v.height as f32 - 0.5;
    let top = (yc - 2.0 * v.horizon_px).max(0.0).ceil() as usize + 1;
    // Sideways slope: at most 0.13 * ripples * |cos| of each wave train's heading (see ripple_slope).
    let left = if rip > 0.0 { (0.13 * WAVE_COS_SUM * rip * (yc - v.horizon_px) * 0.5).ceil() as usize + 1 } else { 0 };
    (left, top)
}

/// The `w` x `h` window at (`x0`, `y0`) of a buffer `bw` wide.
fn crop<T: Clone>(buf: &[T], bw: usize, x0: usize, y0: usize, w: usize, h: usize) -> Vec<T> {
    let mut out = Vec::with_capacity(w * h);
    for y in y0..y0 + h { out.extend_from_slice(&buf[y * bw + x0..y * bw + x0 + w]); }
    out
}

/// The drawn ship of a `Companion`, seen from behind: swept wings, hull, fin, canopy, two engines
/// and the wingtip lights. In metres, wingspan `size`.
fn ship_parts(c: &crate::scene::Companion) -> Vec<crate::scene::ShapePart> {
    use crate::scene::{Shape, ShapePart};
    let k = c.size * 0.5;
    let q = |x: f32, y: f32| [x * k, y * k];
    let dim = |c: Rgb, f: f32| [(c[0] as f32 * f) as u8, (c[1] as f32 * f) as u8, (c[2] as f32 * f) as u8];
    let part = |shape: Shape, color: Rgb, shade: f32, glow: f32| ShapePart { shape, color, shade, cut: false, glow };
    let ell = |x: f32, y: f32, rx: f32, ry: f32| Shape::Ellipse { center: q(x, y), radius: [rx * k, ry * k] };
    let poly = |pts: &[(f32, f32)]| Shape::Poly { points: pts.iter().map(|&(x, y)| q(x, y)).collect() };
    let mut parts = Vec::new();
    for sx in [-1.0f32, 1.0] {
        parts.push(part(poly(&[(sx * 1.0, 0.18), (sx * 0.18, 0.40), (sx * 0.18, 0.26), (sx * 0.95, 0.10)]), dim(c.color, 0.85), 0.4, 0.0));
        parts.push(part(poly(&[(sx * 0.85, 0.16), (sx * 0.5, 0.25), (sx * 0.5, 0.21), (sx * 0.84, 0.13)]), c.accent, 0.3, 0.0));
    }
    parts.push(part(poly(&[(-0.04, 0.46), (0.04, 0.46), (0.02, 0.78), (-0.01, 0.78)]), dim(c.color, 0.9), 0.5, 0.0));
    parts.push(part(ell(0.0, 0.32, 0.24, 0.2), c.color, 0.8, 0.0));
    // The belly in its own shadow, and a seam across the hull.
    parts.push(part(ell(0.0, 0.22, 0.2, 0.09), dim(c.color, 0.55), 0.3, 0.0));
    parts.push(part(Shape::Rect { min: q(-0.23, 0.355), max: q(0.23, 0.365) }, dim(c.color, 0.6), 0.0, 0.0));
    parts.push(part(ell(0.0, 0.44, 0.13, 0.08), c.accent, 0.6, 0.3));
    for sx in [-1.0f32, 1.0] {
        parts.push(part(ell(sx * 0.11, 0.27, 0.085, 0.075), [34, 37, 44], 0.3, 0.0));
        // A hot core inside a softer ring of exhaust.
        parts.push(part(ell(sx * 0.11, 0.27, 0.065, 0.057), dim(c.engine, 0.7), 0.0, 0.6));
        parts.push(part(ell(sx * 0.11, 0.27, 0.04, 0.035), c.engine, 0.0, 1.2));
    }
    parts.push(part(ell(-0.97, 0.15, 0.025, 0.025), [255, 60, 50], 0.0, 1.5));
    parts.push(part(ell(0.97, 0.15, 0.025, 0.025), [60, 255, 120], 0.0, 1.5));
    parts
}

fn post_billboard(ctx: &Ctx, sprite: &Arc<Sprite>, x: f32, top: f32, d: f32, size: f32) -> Billboard {
    let z = ctx.view.to_cam(x, 0.0, d)[2];
    Billboard {
        fade: 0.0, z: z + 0.001, world: [x, 0.0, d], height: top, sprite: sprite.clone(), flip: false, nearest: false,
        id: id::FIXTURE, emissive: 0.0, shadow: 0.35 * size.min(1.0), own_light: None, source: 0, realm: ctx.realm, panel: None, tube: None, sway: 0.0, tint: [1.0; 3], wind: CardWind::default(),
    }
}

// ── Geometry ───────────────────────────────────────────────────────────────

/// Where the bridges are along the world.
#[derive(Clone, Copy, Debug)]
struct Spans {
    period: f32, len: f32, offset: f32, bottom: BridgeBottom, railing: Railing, rail_h: f32, pillars: bool, floor: f32,
    /// The deck's thickness (never as deep as the gap).
    thick: f32,
    /// Lights stand on the railing's posts (a fixture's `bridge_posts`): every bridge then has a
    /// post at each end and `between` more evenly spaced between them, each side.
    posts: bool,
    between: u32,
}

impl Spans {
    /// How far in from the deck's edge the railing's parts are set, so that its widest part (an
    /// end pillar's cap, or the rail itself) ends flush with the edge instead of overhanging it.
    fn rail_inset(&self) -> f32 {
        let ends = |w: f32, xi: f32| if self.pillars || self.posts { xi + w + (w * 0.12).max(0.02) } else { 0.0 };
        match self.railing {
            Railing::None => 0.0,
            Railing::Parapet => ends(0.46, -0.08).max(0.34),
            Railing::Balustrade => ends(0.44, -0.07).max(0.31),
            Railing::Posts => ends(0.18, -0.04).max(0.1),
            Railing::Iron => ends(0.42, -0.06).max(0.14),
            Railing::Rope => ends(0.2, -0.02).max(0.13),
        }
    }

    /// The railing's end pillar (or post): its width, how far out from the railing's line it
    /// stands, its height, and its material.
    fn pillar(&self) -> (f32, f32, f32, u8) {
        let rh = self.rail_h;
        match self.railing {
            Railing::Parapet => (0.46, -0.08, rh + 0.16, id::STONE),
            Railing::Balustrade => (0.44, -0.07, rh + 0.12, id::STONE),
            Railing::Posts => (0.18, -0.04, rh + 0.18, id::WOOD),
            Railing::Iron => (0.42, -0.06, rh + 0.14, id::STONE),
            Railing::Rope => (0.2, -0.02, rh + 0.25, id::WOOD),
            Railing::None => (0.2, 0.0, rh + 0.2, id::WOOD),
        }
    }

    /// Where the posts that carry lights stand along the bridge [s0, e] (their middles): one at
    /// each end and `between` evenly between.
    fn post_places(&self, s0: f32, e: f32) -> Vec<f32> {
        let w = self.pillar().0;
        let (a, z) = (s0 + 0.5 * w, e - 0.5 * w);
        let n = self.between as usize + 1;
        (0..=n).map(|k| a + (z - a) * k as f32 / n as f32).collect()
    }

    /// Sideways from the deck's edge to a post's middle (outward positive), and its top.
    fn post_reach(&self) -> (f32, f32) {
        let (w, xi, ht, _) = self.pillar();
        (xi + 0.5 * w - self.rail_inset(), ht + (w * 0.12).max(0.02) * 1.6)
    }

    fn new(b: &Bridge, loop_len: f32, fixtures: &[Fixture]) -> Option<Spans> {
        if !b.enabled || b.length <= 0.0 { return None; }
        let period = snap_to_loop(b.spacing.max(2.0), loop_len);
        let depth = b.depth.clamp(0.5, 200.0);
        // What the eye meets at the bottom: the water's surface, or the floor of the gap.
        let floor = if b.bottom == BridgeBottom::Water { b.water_level.clamp(0.1, depth) } else { depth };
        Some(Spans {
            period, len: b.length.clamp(0.5, (period - 0.5).max(0.5)), offset: b.offset,
            bottom: b.bottom, railing: b.railing, rail_h: b.rail_height.clamp(0.2, 3.0), pillars: b.end_pillars, floor,
            thick: b.thickness.clamp(0.0, 2.0).min(0.8 * floor),
            posts: fixtures.iter().any(|f| f.enabled && f.bridge_posts),
            between: fixtures.iter().filter(|f| f.enabled && f.bridge_posts).map(|f| f.posts_between.min(15)).max().unwrap_or(0),
        })
    }
    fn contains(&self, w: f32) -> bool { (w - self.offset).rem_euclid(self.period) < self.len }
    /// Each bridge overlapping [a, b), as (start, end) in world metres.
    fn within(&self, a: f32, b: f32) -> Vec<(f32, f32)> {
        let k0 = ((a - self.offset - self.len) / self.period).floor() as i64;
        let k1 = ((b - self.offset) / self.period).ceil() as i64;
        (k0..=k1).map(|k| { let s = self.offset + k as f32 * self.period; (s, s + self.len) }).filter(|&(s, e)| e > a && s < b).collect()
    }
}

/// Where the forks are along the world.
#[derive(Clone, Copy, Debug)]
struct Forks { period: f32, offset: f32, side: ForkSide, split: bool, tan: f32, hw: f32, noise: f32, depth: f32, height: f32, per_loop: i64 }

impl Forks {
    fn new(f: &Fork, edge_noise: f32, loop_len: f32) -> Option<Forks> {
        if !f.enabled || f.half_width <= 0.0 { return None; }
        let period = snap_to_loop(f.spacing.max(4.0), loop_len);
        let hw = f.half_width.clamp(0.2, period * 0.2);
        Some(Forks {
            period, offset: f.offset, side: f.side, split: f.style == ForkStyle::Split, tan: f.angle.clamp(5.0, 85.0).to_radians().tan(),
            hw, noise: edge_noise.clamp(0.0, hw * 0.5), depth: f.depth.clamp(0.5, 100.0), height: f.height.max(0.5),
            per_loop: (loop_len / period).round().max(1.0) as i64,
        })
    }
    fn side_of(&self, k: i64) -> f32 {
        // Sides follow the fork's place in the loop, not its count from the start, or the fork just
        // ahead would change sides when the loop wraps. With an odd number per loop two neighbours
        // share a side at the wrap (one per loop: always the left).
        match self.side { ForkSide::Left | ForkSide::Both => -1.0, ForkSide::Right => 1.0, ForkSide::Alternate => if k.rem_euclid(self.per_loop) % 2 == 0 { -1.0 } else { 1.0 } }
    }
    /// The sides fork k leaves on: one, or both (the second is 0 when there is only one).
    fn sides(&self, k: i64) -> [f32; 2] { if self.side == ForkSide::Both { [-1.0, 1.0] } else { [self.side_of(k), 0.0] } }
    fn has_side(&self, k: i64, side: f32) -> bool { self.sides(k).contains(&side) }
    /// Junctions whose branch could reach world distance w: (where, which side).
    #[cfg(test)]
    fn near(&self, w: f32, reach: f32) -> impl Iterator<Item = (f32, f32)> + '_ {
        let k0 = ((w - self.offset - reach) / self.period).floor() as i64;
        let k1 = ((w - self.offset + self.hw * 2.0) / self.period).floor() as i64;
        (k0..=k1).flat_map(move |k| self.sides(k).into_iter().filter(|&d| d != 0.0).map(move |d| (self.offset + k as f32 * self.period, d)))
    }
    /// Whether ground at (x, w) is on a branch path. A branch leaves the main path's edge at its
    /// junction and runs off at the fork angle, with a rounded mouth.
    fn on_branch(&self, x: f32, w: f32, main_hw: f32) -> bool {
        let reach = 300.0 / self.tan;
        let norm = (1.0 + self.tan * self.tan).sqrt();
        let hit = |j: f32, side: f32| {
            let t = w - j;
            if t < -self.hw || x * side <= 0.0 { return false; }
            // The centreline starts inside the main path and heads off at the fork angle.
            let x0 = side * main_hw * 0.5;
            let dist = if t >= 0.0 { ((x - x0) - side * self.tan * t).abs() / norm } else { ((x - x0).powi(2) + t * t).sqrt() };
            dist < self.hw
        };
        // Only junctions in two narrow windows can reach (x, w): the one whose centreline crosses
        // x near here (t* = (|x| - main_hw/2) / tan, give or take hw * norm / tan), and those
        // within hw of w (the rounded mouth). Each is widened by a period against rounding, and
        // kept inside the range `near` would walk, so the answer is the same, junction for junction.
        let k = |m: f32| ((m - self.offset) / self.period).floor() as i64;
        let (k0, k1) = (k(w - reach), k(w + self.hw * 2.0));
        let ts = (x.abs() - main_hw * 0.5) / self.tan;
        let spread = self.hw * norm / self.tan;
        let at = |k: i64| self.offset + k as f32 * self.period;
        let window = |lo: f32, hi: f32| (k(lo) - 1).max(k0)..=(k(hi) + 1).min(k1);
        window(w - ts - spread, w - ts + spread).chain(window(w - self.hw, w + self.hw))
            .any(|k| self.sides(k).into_iter().any(|side| side != 0.0 && hit(at(k), side)))
    }
    /// How far the split style rounds the corners where a branch meets the road, metres.
    fn fillet(&self) -> f32 { self.hw }
    /// The split style: signed distance (metres, below 0 inside) from ground at (x, w) to the
    /// nearest branch road. A branch leaves the middle of the main road at its junction, running
    /// alongside it at first (one wide paved surface) and curving away until it runs on at the fork
    /// angle; a grass point opens between the two once they have drawn apart. Its edges wobble like
    /// the main road's, differently for each fork in the loop.
    ///
    /// Distance is measured across the road (along x) and scaled by the centreline's slope there,
    /// so a point is within e of the centreline only where |x - xc(t)| < e * sqrt(1 + slope^2) <=
    /// e * norm; xc rising with t, that bounds the junctions worth looking at to a window.
    fn branch_sd(&self, x: f32, w: f32, main_hw: f32) -> f32 {
        let r = (main_hw + self.hw) / self.tan;
        let norm = (1.0 + self.tan * self.tan).sqrt();
        let cap = self.hw + self.noise + self.fillet();
        let e = cap * norm;
        // The distance along the road at which the centreline is v out from the middle.
        let tc = |v: f32| if v <= 0.0 { 0.0 } else { ((v / self.tan + r).powi(2) - r * r).sqrt() };
        let reach = 300.0 / self.tan;
        let (t_lo, t_hi) = (tc(x.abs() - e), tc(x.abs() + e).min(reach));
        let k = |m: f32| ((m - self.offset) / self.period).floor() as i64;
        let lo = w - t_hi;
        let hi = w - t_lo + if t_lo == 0.0 { cap } else { 0.0 };
        (k(lo) - 1..=k(hi) + 1).fold(f32::INFINITY, |best, kk| self.branch_sd_at(x, w, kk, r, best))
    }
    /// One junction's branches' signed distance at (x, w), or `best` if that is nearer.
    fn branch_sd_at(&self, x: f32, w: f32, kk: i64, r: f32, mut best: f32) -> f32 {
        let cap = self.hw + self.noise + self.fillet();
        let t = w - (self.offset + kk as f32 * self.period);
        if t < -cap || t > 300.0 / self.tan { return best; }
        for side in self.sides(kk) {
            if side == 0.0 || x * side <= 0.0 { continue; }
            let dist = if t >= 0.0 {
                let q = (t * t + r * r).sqrt();
                let xc = side * self.tan * (q - r);
                let slope = self.tan * t / q;
                (x - xc).abs() / (1.0 + slope * slope).sqrt()
            } else {
                (x * x + t * t).sqrt()
            };
            if dist - self.hw - self.noise >= best { continue; }
            let seed = 303 + 7 * (kk.rem_euclid(self.per_loop) as u32) + if side > 0.0 { 3 } else { 0 };
            let tn = t.max(0.0);
            let n = path_noise(tn, 1024.0, 0.9, seed) * 0.65 + path_noise(tn, 1024.0, 0.3, seed + 1) * 0.35;
            best = best.min(dist - (self.hw + self.noise * (n * 2.0 - 1.0)));
        }
        best
    }
    /// Whether a side passage (between walls) opens on `side` at world distance w.
    fn opens(&self, w: f32, side: f32) -> bool {
        let k = ((w - self.offset) / self.period).floor() as i64;
        let u = w - self.offset - k as f32 * self.period;
        u < self.hw * 2.0 && self.has_side(k, side)
    }
    /// Each side passage overlapping [a, b): (start, end, side), world metres.
    fn openings(&self, a: f32, b: f32) -> Vec<(f32, f32, f32)> {
        let k0 = ((a - self.offset) / self.period).floor() as i64 - 1;
        let k1 = ((b - self.offset) / self.period).ceil() as i64;
        (k0..=k1).flat_map(|k| { let s = self.offset + k as f32 * self.period; self.sides(k).into_iter().filter(|&d| d != 0.0).map(move |d| (s, s + self.hw * 2.0, d)) })
            .filter(|&(s, e, _)| e > a && s < b).collect()
    }
}

/// Side passages fall into darkness the further they run from the corridor.
fn passage_dark(ctx: &Ctx, g: &GPixel) -> f32 {
    // Only under a roof: an alley open to the sky is lit like the street.
    if ctx.fork.is_none() || !ctx.scene.walls.enabled || !ctx.scene.ceiling.enabled { return 1.0; }
    let beyond = g.x.abs() - ctx.wall_x(g.d);
    if beyond > 0.0 { 0.15 + 0.85 * (-beyond / 1.2).exp() } else { 1.0 }
}

fn build_geometry(ctx: &Ctx, layers: &Layers) -> Vec<Tri> {
    let v = &ctx.view;
    let s = ctx.scene;
    // Open flight: no ground at all, so nothing of the path, its verge or its bridges is built.
    let ground_layer = Layers { ground: layers.ground && s.path.surface, ..*layers };
    let layers = &ground_layer;
    // A turned camera sees the path behind where it stands and to either side: the ground is cut
    // from the near end of what it sees to the far end, finer close to the camera.
    let (lo, hi) = if v.turned { (ctx.d_lo, ctx.d_hi) } else { (NEAR, ctx.far) };
    let mut ds = vec![lo];
    while *ds.last().unwrap() < hi {
        let d = *ds.last().unwrap();
        // (Fine enough to follow the bends of a winding path.)
        let near = if v.turned { d.abs().max(v.cam_x.abs()) } else { d };
        ds.push(d + (near * 0.05).max(0.05).min(v.wind_length().map_or(f32::MAX, |l| l / 24.0)));
    }
    // Bridges: surfaces are cut where each one starts and ends, so a quad is either over the gap or not.
    let spans: Vec<(f32, f32)> = ctx.bridge.map(|b| b.within(v.scroll + lo.min(0.0), v.scroll + hi).into_iter().map(|(s, e)| (s - v.scroll, e - v.scroll)).collect()).unwrap_or_default();
    let walls_on = s.walls.enabled && layers.walls;
    let openings: Vec<(f32, f32, f32)> = match (&ctx.fork, walls_on) {
        (Some(f), true) => f.openings(v.scroll + lo.min(0.0), v.scroll + hi).into_iter().map(|(a, b, side)| (a - v.scroll, b - v.scroll, side)).collect(),
        _ => Vec::new(),
    };
    let cuts: Vec<f32> = spans.iter().flat_map(|&(s, e)| [s, e]).chain(openings.iter().flat_map(|&(a, b, _)| [a, b]))
        .filter(|&c| c > lo && c < hi).collect();
    if !cuts.is_empty() {
        ds.retain(|&x| x == lo || cuts.iter().all(|&c| (x - c).abs() > 0.01));
        ds.extend(&cuts);
        ds.sort_by(|a, b| a.partial_cmp(b).unwrap());
    }
    // Steps: every surface is cut at each riser, so a quad never spans two step heights. The quad
    // before a riser ends a hair short of it (on the lower step); the riser face fills the gap.
    const STEP_EPS: f32 = 1e-3;
    let risers: Vec<f32> = match &v.stairs {
        Some(st) => st.risers(v.scroll + lo + STEP_EPS, v.scroll + hi).into_iter().map(|e| e - v.scroll).collect(),
        None => Vec::new(),
    };
    if !risers.is_empty() {
        // Make room so each riser's two cuts stay exactly as computed: the riser below reuses them.
        ds.retain(|&x| x == lo || risers.iter().all(|&r| (x - r).abs() > 0.01 && (x - (r - STEP_EPS)).abs() > 0.01));
        ds.extend(risers.iter().flat_map(|&d| [d - STEP_EPS, d]));
        ds.sort_by(|a, b| a.partial_cmp(b).unwrap());
    }
    let wide = |d: f32| ((v.width as f32 * 0.5) / v.focal_px * d + v.bend_x(d).abs()) * 1.4 + 4.0;
    let vert = |x: f32, y: f32, d: f32| Vert { cam: v.to_cam(x, y, d), world: [x, y, d] };
    let walls = s.walls.enabled && layers.walls;
    let top = ctx.wall_top();
    let ext = |d: f32| {
        if walls { ctx.wall_x(d) } else if s.verge.enabled { if v.turned { ctx.x_reach } else { wide(d) } } else { v.path_half_width(d) + s.path.edge_noise.abs() }
    };
    // A side view cuts away what stands between the camera and the path: the wall and the ceiling.
    // (A camera swinging round dissolves them first: see `raster::dissolved`.)
    let side = v.side_k >= 1.0;
    let near_side = v.near;
    let mut tris = Vec::with_capacity(ds.len() * 8);
    if layers.ground {
        for &d in &risers {
            // Built from the very vertices of the step below (its far edge) and the step above (its
            // near edge), so the faces share edges exactly and no pixel falls between them.
            let below = d - STEP_EPS;
            let low = v.lift(below) - v.lift(d);
            let (eb, ea) = (ext(below), ext(d));
            let foot = |x: f32| Vert { cam: v.to_cam(x, 0.0, below), world: [x, low, d] };
            raster::quad(&mut tris, [foot(-eb), foot(eb), vert(ea, 0.0, d), vert(-ea, 0.0, d)], id::RISER);
        }
    }
    if let (Some(b), true) = (ctx.bridge, layers.ground) {
        for &(s0, e) in &spans {
            // The far side of the gap, facing the camera: its top edge is the near edge of the ground
            // beyond, its foot the far edge of the bottom, so the three meet without cracks.
            // (A turned camera sees the near side of the gap too, facing the other way.)
            for (at, show) in [(e, e > NEAR && e < ctx.far), (s0, v.turned)] {
                if !show || at < lo || at > hi { continue; }
                // All the way across, under the deck's end as well as beside it, so no hole shows
                // under the deck where the ground meets it.
                let x = ext(at).max(v.path_half_width(at));
                raster::quad(&mut tris, [vert(-x, -b.floor, at), vert(x, -b.floor, at), vert(x, 0.0, at), vert(-x, 0.0, at)], id::CLIFF);
            }
            deck_slab(v, b, s0, e, &ds, (if v.turned { lo } else { NEAR }, hi), &mut tris);
            railing(v, b, s0, e, &ds, (if v.turned { lo } else { NEAR }, hi), &mut tris);
        }
    }
    if layers.ground { fences(ctx, &ds, &mut tris); }
    // Side passages: an alley (open to the sky) without a ceiling, a doorway with one.
    let passage_h = ctx.fork.filter(|_| !openings.is_empty() && s.ceiling.enabled).map(|f| f.height.min(top));
    if let Some(f) = ctx.fork.filter(|_| !openings.is_empty()) {
        let h = passage_h.unwrap_or(top);
        for &(_, e, side) in &openings {
            // The far post of each opening faces the camera; the near one faces away and is never seen.
            if e <= lo || e >= hi { continue; }
            let (wx, px) = (side * ctx.wall_x(e), side * (ctx.wall_x(e) + f.depth));
            // It faces the camera like the far side of a bridge's gap, and is shaded as such.
            raster::quad(&mut tris, [vert(wx, 0.0, e), vert(px, 0.0, e), vert(px, h, e), vert(wx, h, e)], id::CLIFF);
        }
    }
    for pair in ds.windows(2) {
        let (d0, d1) = (pair[0], pair[1]);
        if d1 - d0 < STEP_EPS * 1.5 { continue; }
        let span = ctx.bridge.filter(|b| b.contains(v.scroll + 0.5 * (d0 + d1)));
        if layers.ground {
            let (e0, e1) = (ext(d0), ext(d1));
            match ctx.bridge {
                None => raster::quad(&mut tris, [vert(-e0, 0.0, d0), vert(e0, 0.0, d0), vert(e1, 0.0, d1), vert(-e1, 0.0, d1)], id::GROUND),
                Some(b) => {
                    // Cut at the deck edges everywhere, so the pieces beside the deck line up with the
                    // cliff faces at the far end of each gap.
                    let (h0, h1) = (v.path_half_width(d0), v.path_half_width(d1));
                    raster::quad(&mut tris, [vert(-h0, 0.0, d0), vert(h0, 0.0, d0), vert(h1, 0.0, d1), vert(-h1, 0.0, d1)], id::GROUND);
                    let (y, pid) = if span.is_some() { (-b.floor, id::CHASM) } else { (0.0, id::GROUND) };
                    if (e0 > h0 + 1e-4 || e1 > h1 + 1e-4) && !(span.is_some() && b.bottom == BridgeBottom::Void) {
                        for sign in [-1.0f32, 1.0] {
                            raster::quad(&mut tris, [vert(sign * h0, y, d0), vert(sign * e0, y, d0), vert(sign * e1, y, d1), vert(sign * h1, y, d1)], pid);
                        }
                    }
                }
            }
        }
        if walls {
            // Over a gap the walls carry on down to its bottom.
            let foot = span.map_or(0.0, |b| -b.floor);
            // With side passages every wall is also cut at the passage height, so the far post of each
            // opening shares its edge with the wall beside it.
            let ph = passage_h.filter(|&h| h < top - 1e-3);
            let open = ctx.fork.filter(|_| !openings.is_empty());
            let mid = v.scroll + 0.5 * (d0 + d1);
            for (sign, wid) in [(-1.0f32, id::WALL_L), (1.0, id::WALL_R)] {
                if side && sign == near_side { continue; }
                let (x0, x1) = (sign * ctx.wall_x(d0), sign * ctx.wall_x(d1));
                if let Some(f) = open.filter(|f| f.opens(mid, sign)) {
                    // A side passage: floor and ceiling run back into the dark, a lintel above.
                    let (p0, p1) = (sign * (ctx.wall_x(d0) + f.depth), sign * (ctx.wall_x(d1) + f.depth));
                    let h = ph.unwrap_or(top);
                    if layers.ground {
                        raster::quad(&mut tris, [vert(x0, 0.0, d0), vert(p0, 0.0, d0), vert(p1, 0.0, d1), vert(x1, 0.0, d1)], id::GROUND);
                    }
                    if let Some(h) = ph {
                        raster::quad(&mut tris, [vert(x0, h, d0), vert(x1, h, d1), vert(x1, top, d1), vert(x0, top, d0)], wid);
                        raster::quad(&mut tris, [vert(x0, h, d0), vert(x1, h, d1), vert(p1, h, d1), vert(p0, h, d0)], id::CEILING);
                    } else {
                        // Open to the sky: an alley that ends in a wall rather than in nothing.
                        raster::quad(&mut tris, [vert(p0, 0.0, d0), vert(p1, 0.0, d1), vert(p1, h, d1), vert(p0, h, d0)], wid);
                    }
                    continue;
                }
                match ph {
                    Some(h) => {
                        raster::quad(&mut tris, [vert(x0, foot, d0), vert(x1, foot, d1), vert(x1, h, d1), vert(x0, h, d0)], wid);
                        raster::quad(&mut tris, [vert(x0, h, d0), vert(x1, h, d1), vert(x1, top, d1), vert(x0, top, d0)], wid);
                    }
                    None => raster::quad(&mut tris, [vert(x0, foot, d0), vert(x1, foot, d1), vert(x1, top, d1), vert(x0, top, d0)], wid),
                }
            }
        }
        if s.ceiling.enabled && layers.walls && !side {
            let ch = s.ceiling.height.max(0.2);
            let ext = |d: f32| if walls { ctx.wall_x(d) } else if v.turned { ctx.x_reach } else { wide(d) };
            let (e0, e1) = (ext(d0), ext(d1));
            raster::quad(&mut tris, [vert(-e0, ch, d0), vert(-e1, ch, d1), vert(e1, ch, d1), vert(e0, ch, d0)], id::CEILING);
        }
    }
    tris
}

// ── Shading ────────────────────────────────────────────────────────────────

/// The camera partway through a swing this frame: one a game or transition is making, or the
/// first world's swing in while the intro plays (one world to walk).
fn swing_now(worlds: &[WorldIn], opts: &RenderOptions) -> Option<crate::scene::SwingPose> {
    if let Some(s) = &opts.swing { return Some(s.clone()); }
    let c = &worlds[0].scene.camera;
    opts.intro.filter(|_| worlds.len() == 1).and_then(|t| c.moves.swing_in.pose(c, t))
}

/// Black bars over the top and bottom of a frame, `share` of its height each.
fn letterbox(img: &mut Image, share: f32) {
    let n = (share.clamp(0.0, 0.5) * img.height as f32).round() as usize;
    if n == 0 { return; }
    let w = img.width;
    for y in (0..n).chain(img.height.saturating_sub(n)..img.height) {
        for p in img.rgba[y * w * 4..(y + 1) * w * 4].chunks_exact_mut(4) { p[..3].fill(0); }
    }
}

/// Where the light of a sun or moon at `pos` in the picture comes from, in the world: the
/// picture is 2.4 radians of sky across, and the sky turns with the camera (less what it slides
/// back across the frame when it stays in the world, `View::sky_turn`).
fn sky_light_dir(view: &View, pos: [f32; 2]) -> [f32; 3] {
    let yaw = if view.turned { view.yaw_s.atan2(view.yaw_c) } else { 0.0 };
    let az = (pos[0] + view.sky_turn - 0.5) * 2.4 - view.turn_tan().atan() + yaw;
    let el = (1.0 - pos[1].clamp(0.0, 1.0)) * 1.15 + 0.08;
    [az.sin() * el.cos(), el.sin(), az.cos() * el.cos()]
}

/// The paint of a scene's fences (the first painted one's colour), linear.
pub(crate) fn fence_paint(s: &Scene) -> [f32; 3] {
    rgb_lin(s.fences.iter().find(|f| f.enabled && f.style != FenceStyle::Rails).map_or([236, 232, 220], |f| f.color))
}

/// The fences beside the path: posts, rails and pickets (or boards) as boxes that follow the
/// path's edge, built from the faces the camera can see, as a bridge's railing is.
fn fences(ctx: &Ctx, ds: &[f32], tris: &mut Vec<Tri>) {
    let v = &ctx.view;
    let far = if v.turned { ctx.d_hi } else { ctx.far.min(120.0) };
    let near = if v.turned { ctx.d_lo } else { NEAR };
    let vert = |x: f32, y: f32, d: f32| Vert { cam: v.to_cam(x, y, d), world: [x, y, d] };
    for f in ctx.scene.fences.iter().filter(|f| f.enabled && f.height > 0.0) {
        let sp = snap_to_loop(f.spacing.max(1.0), ctx.loop_len);
        let len = f.length.clamp(0.5, sp);
        // A run as long as the spacing joins the next one: one fence with no break.
        let joined = len >= sp - 1e-3;
        let base = |d: f32| v.path_half_width(d) + f.lateral;
        // A box over [d0, d1] standing `xi`..`xo` metres out from the fence's line (both its ends
        // for a turned camera, as a railing's).
        let block = |tris: &mut Vec<Tri>, sign: f32, d0: f32, d1: f32, xi: f32, xo: f32, y0: f32, y1: f32, m: u8| {
            if if v.turned { d1 < near || d0 > far } else { d0 < near || d0 > far } { return; }
            let h = base(0.5 * (d0 + d1));
            let (a, c) = (sign * (h + xi), sign * (h + xo));
            raster::quad(tris, [vert(a, y0, d0), vert(c, y0, d0), vert(c, y1, d0), vert(a, y1, d0)], id::rail(m, id::FRONT));
            if v.turned { raster::quad(tris, [vert(a, y0, d1), vert(c, y0, d1), vert(c, y1, d1), vert(a, y1, d1)], id::rail(m, id::FRONT)); }
            raster::quad(tris, [vert(a, y0, d0), vert(a, y0, d1), vert(a, y1, d1), vert(a, y1, d0)], id::rail(m, id::INNER));
            raster::quad(tris, [vert(c, y0, d0), vert(c, y0, d1), vert(c, y1, d1), vert(c, y1, d0)], id::rail(m, id::OUTER));
            raster::quad(tris, [vert(a, y1, d0), vert(a, y1, d1), vert(c, y1, d1), vert(c, y1, d0)], id::rail(m, id::TOP));
        };
        // A long box from `a` to `b` along the line, cut where the ground is so it bends with it.
        let run = |tris: &mut Vec<Tri>, sign: f32, a: f32, b: f32, xi: f32, xo: f32, y0: f32, y1: f32, m: u8| {
            let (lo, hi) = (a.max(near), b.min(far));
            if hi <= lo { return; }
            let mut cuts: Vec<f32> = ds.iter().copied().filter(|&d| d > lo && d < hi).collect();
            cuts.push(lo);
            cuts.push(hi);
            cuts.sort_by(|p, q| p.partial_cmp(q).unwrap());
            cuts.dedup_by(|p, q| (*p - *q).abs() < 1e-3);
            let at = |d: f32| (sign * (base(d) + xi), sign * (base(d) + xo));
            if a >= near {
                let (i, o) = at(a);
                raster::quad(tris, [vert(i, y0, a), vert(o, y0, a), vert(o, y1, a), vert(i, y1, a)], id::rail(m, id::FRONT));
            }
            if v.turned && b <= far {
                let (i, o) = at(b);
                raster::quad(tris, [vert(i, y0, b), vert(o, y0, b), vert(o, y1, b), vert(i, y1, b)], id::rail(m, id::FRONT));
            }
            for w in cuts.windows(2) {
                let ((i0, o0), (i1, o1), d0, d1) = (at(w[0]), at(w[1]), w[0], w[1]);
                raster::quad(tris, [vert(i0, y0, d0), vert(i1, y0, d1), vert(i1, y1, d1), vert(i0, y1, d0)], id::rail(m, id::INNER));
                raster::quad(tris, [vert(o0, y0, d0), vert(o1, y0, d1), vert(o1, y1, d1), vert(o0, y1, d0)], id::rail(m, id::OUTER));
                raster::quad(tris, [vert(i0, y1, d0), vert(i1, y1, d1), vert(o1, y1, d1), vert(o0, y1, d0)], id::rail(m, id::TOP));
            }
        };
        let ht = f.height;
        let (paint, wood) = (id::FENCE, id::WOOD);
        let k_lo = ((ctx.scroll + near.min(0.0) - len - f.offset) / sp).floor() as i64;
        let k_hi = ((ctx.scroll + far - f.offset) / sp).ceil() as i64;
        for k in k_lo..=k_hi {
            let a = k as f32 * sp + f.offset - ctx.scroll;
            let b = a + len;
            if b < near || a > far { continue; }
            // Not over a bridge's drop.
            if ctx.on_bridge(0.5 * (a + b)) { continue; }
            let posts = ((len / f.post_spacing.max(0.5)).round() as usize).max(1);
            for sign in f.side.signs().iter().copied() {
                if ctx.cut_away(sign * (ctx.view.half_width + f.lateral)) { continue; }
                let (pw, pm) = match f.style { FenceStyle::Rails => (0.12, wood), _ => (0.09, paint) };
                // Posts (a joined fence's last post is the next run's first).
                let last = if joined { posts - 1 } else { posts };
                for j in 0..=last {
                    let d = a + len * j as f32 / posts as f32;
                    block(tris, sign, d - 0.5 * pw, d + 0.5 * pw, -0.5 * pw, 0.5 * pw, 0.0, ht + 0.06, pm);
                }
                match f.style {
                    FenceStyle::Picket => {
                        for y in [0.22 * ht, 0.72 * ht] { run(tris, sign, a, b, 0.03, 0.055, y, y + 0.07, paint); }
                        // Pickets, each with a narrower point on top; far off they merge into a panel.
                        let n = (len / 0.14).round().max(1.0) as usize;
                        for j in 0..n {
                            let c = a + len * (j as f32 + 0.5) / n as f32;
                            if c > 35.0 {
                                run(tris, sign, (c - 0.07).max(a), b, -0.02, 0.005, 0.04, ht * 0.97, paint);
                                break;
                            }
                            block(tris, sign, c - 0.037, c + 0.037, -0.02, 0.005, 0.04, ht - 0.05, paint);
                            block(tris, sign, c - 0.018, c + 0.018, -0.02, 0.005, ht - 0.05, ht, paint);
                        }
                    }
                    FenceStyle::Rails => {
                        for y in [0.28 * ht, 0.6 * ht, 0.9 * ht] { run(tris, sign, a, b, -0.07, -0.02, y - 0.09, y, wood); }
                    }
                    FenceStyle::Boards => {
                        run(tris, sign, a, b, -0.02, 0.01, 0.02, ht - 0.04, paint);
                        run(tris, sign, a, b, -0.04, 0.03, ht - 0.04, ht + 0.02, paint);
                    }
                }
            }
        }
    }
}

/// One bridge's railings, both sides. Every part is a box built from the faces the camera can
/// see (its far end always faces away), so it reads as solid from any bend of the path, and each
/// face carries its own id, so it is lit with its own normal.
/// The deck's thickness under its surface over the gap [s0, e]: its edges either side, in the deck's
/// own material. (Its ends are where the gap's end walls are, which run under them.)
fn deck_slab(v: &View, b: Spans, s0: f32, e: f32, ds: &[f32], (near, far): (f32, f32), tris: &mut Vec<Tri>) {
    let t = b.thick;
    let (lo, hi) = (s0.max(near), e.min(far));
    if t <= 0.0 || hi <= lo { return; }
    let vert = |x: f32, y: f32, d: f32| Vert { cam: v.to_cam(x, y, d), world: [x, y, d] };
    let mut cuts: Vec<f32> = ds.iter().copied().filter(|&d| d > lo && d < hi).collect();
    cuts.push(lo);
    cuts.push(hi);
    cuts.sort_by(|p, q| p.partial_cmp(q).unwrap());
    cuts.dedup_by(|p, q| (*p - *q).abs() < 1e-3);
    for sign in [-1.0f32, 1.0] {
        for w in cuts.windows(2) {
            let (x0, x1) = (sign * v.path_half_width(w[0]), sign * v.path_half_width(w[1]));
            raster::quad(tris, [vert(x0, -t, w[0]), vert(x1, -t, w[1]), vert(x1, 0.0, w[1]), vert(x0, 0.0, w[0])], id::rail(id::WOOD, id::OUTER));
        }
    }
}

fn railing(v: &View, b: Spans, s0: f32, e: f32, ds: &[f32], (near, far): (f32, f32), tris: &mut Vec<Tri>) {
    if b.railing == Railing::None { return; }
    let rh = b.rail_h;
    // The railing stands on the deck: its outer face flush with the deck's edge.
    let inset = b.rail_inset();
    let vert = |x: f32, y: f32, d: f32| Vert { cam: v.to_cam(x, y, d), world: [x, y, d] };
    // A box over [d0, d1] whose sides stand `xi` and `xo` metres out from the deck's edge. (A
    // turned camera sees its far end too, beside or behind where it stands: both ends are faces
    // then, each lit from the side the camera is on.)
    let block = |tris: &mut Vec<Tri>, sign: f32, d0: f32, d1: f32, xi: f32, xo: f32, y0: f32, y1: f32, m: u8| {
        if if v.turned { d1 < near || d0 > far } else { d0 < near || d0 > far } { return; }
        let h = v.path_half_width(0.5 * (d0 + d1)) - inset;
        let (a, c) = (sign * (h + xi), sign * (h + xo));
        raster::quad(tris, [vert(a, y0, d0), vert(c, y0, d0), vert(c, y1, d0), vert(a, y1, d0)], id::rail(m, id::FRONT));
        if v.turned { raster::quad(tris, [vert(a, y0, d1), vert(c, y0, d1), vert(c, y1, d1), vert(a, y1, d1)], id::rail(m, id::FRONT)); }
        raster::quad(tris, [vert(a, y0, d0), vert(a, y0, d1), vert(a, y1, d1), vert(a, y1, d0)], id::rail(m, id::INNER));
        raster::quad(tris, [vert(c, y0, d0), vert(c, y0, d1), vert(c, y1, d1), vert(c, y1, d0)], id::rail(m, id::OUTER));
        raster::quad(tris, [vert(a, y1, d0), vert(a, y1, d1), vert(c, y1, d1), vert(c, y1, d0)], id::rail(m, id::TOP));
    };
    // A long box from `a` to `b` that follows the deck's edge, cut where the ground is so it bends
    // with the path; `sag` lets a rope hang between its ends.
    let run = |tris: &mut Vec<Tri>, sign: f32, a: f32, b: f32, xi: f32, xo: f32, y0: f32, y1: f32, sag: f32, m: u8| {
        let (lo, hi) = (a.max(near), b.min(far));
        if hi <= lo { return; }
        let pieces = if sag > 0.0 { 8 } else { 1 };
        let mut cuts: Vec<f32> = (0..=pieces).map(|k| a + (b - a) * k as f32 / pieces as f32).filter(|&c| c > lo && c < hi).collect();
        cuts.extend(ds.iter().copied().filter(|&d| d > lo && d < hi));
        cuts.push(lo);
        cuts.push(hi);
        cuts.sort_by(|p, q| p.partial_cmp(q).unwrap());
        cuts.dedup_by(|p, q| (*p - *q).abs() < 1e-3);
        let at = |d: f32| {
            let t = ((d - a) / (b - a).max(1e-3)).clamp(0.0, 1.0);
            let (h, dip) = (v.path_half_width(d) - inset, sag * 4.0 * t * (1.0 - t));
            (sign * (h + xi), sign * (h + xo), y0 - dip, y1 - dip)
        };
        if a >= near {
            let (i, o, l, u) = at(a);
            raster::quad(tris, [vert(i, l, a), vert(o, l, a), vert(o, u, a), vert(i, u, a)], id::rail(m, id::FRONT));
        }
        if v.turned && b <= far {
            let (i, o, l, u) = at(b);
            raster::quad(tris, [vert(i, l, b), vert(o, l, b), vert(o, u, b), vert(i, u, b)], id::rail(m, id::FRONT));
        }
        for w in cuts.windows(2) {
            let ((i0, o0, l0, u0), (i1, o1, l1, u1), d0, d1) = (at(w[0]), at(w[1]), w[0], w[1]);
            raster::quad(tris, [vert(i0, l0, d0), vert(i1, l1, d1), vert(i1, u1, d1), vert(i0, u0, d0)], id::rail(m, id::INNER));
            raster::quad(tris, [vert(o0, l0, d0), vert(o1, l1, d1), vert(o1, u1, d1), vert(o0, u0, d0)], id::rail(m, id::OUTER));
            raster::quad(tris, [vert(i0, u0, d0), vert(i1, u1, d1), vert(o1, u1, d1), vert(o0, u0, d0)], id::rail(m, id::TOP));
        }
    };
    // A pillar `w` square, its cap overhanging a little; one of stone stands on a plinth as wide
    // as the cap, so it reads as built from every side.
    let pillar = |tris: &mut Vec<Tri>, sign: f32, d0: f32, w: f32, xi: f32, ht: f32, m: u8| {
        let o = (w * 0.12).max(0.02);
        let foot = if m == id::STONE { (o * 2.5).min(0.3 * ht) } else { 0.0 };
        if foot > 0.0 { block(tris, sign, d0 - o, d0 + w + o, xi - o, xi + w + o, 0.0, foot, m); }
        block(tris, sign, d0, d0 + w, xi, xi + w, foot, ht, m);
        block(tris, sign, d0 - o, d0 + w + o, xi - o, xi + w + o, ht, ht + o * 1.6, m);
    };
    // Evenly spaced positions from one end to the other, about `gap` apart.
    let spaced = |a: f32, b: f32, gap: f32| {
        let n = ((b - a) / gap).ceil().max(1.0) as usize;
        (0..=n).map(move |k| a + (b - a) * k as f32 / n as f32)
    };
    let (stone, wood, paint) = (id::STONE, id::WOOD, id::PAINT);
    for sign in [-1.0f32, 1.0] {
        // The pillars at each end, and where the railing between them starts and ends.
        let ends = |tris: &mut Vec<Tri>, w: f32, xi: f32, ht: f32, m: u8| {
            if !b.pillars && !b.posts { return (s0, e); }
            pillar(tris, sign, s0, w, xi, ht, m);
            pillar(tris, sign, e - w, w, xi, ht, m);
            (s0 + w, e - w)
        };
        // Posts that carry lights between the end ones.
        if b.posts {
            let (w, xi, ht, m) = b.pillar();
            let places = b.post_places(s0, e);
            for &c in &places[1..places.len() - 1] { pillar(tris, sign, c - 0.5 * w, w, xi, ht, m); }
        }
        match b.railing {
            Railing::None => {
                if b.posts {
                    let (w, xi, ht, m) = b.pillar();
                    ends(tris, w, xi, ht, m);
                }
            }
            Railing::Parapet => {
                let (a, z) = ends(tris, 0.46, -0.08, rh + 0.16, stone);
                run(tris, sign, a, z, 0.0, 0.3, 0.0, rh - 0.08, 0.0, stone);
                run(tris, sign, a, z, -0.04, 0.34, rh - 0.08, rh, 0.0, stone);
            }
            Railing::Balustrade => {
                let (a, z) = ends(tris, 0.44, -0.07, rh + 0.12, stone);
                run(tris, sign, a, z, -0.03, 0.27, 0.0, 0.14, 0.0, stone);
                run(tris, sign, a, z, -0.04, 0.31, rh - 0.1, rh, 0.0, stone);
                // Balusters: a foot, a swelling belly, a slender neck and a head under the rail.
                let (lo, tall) = (0.14, (rh - 0.24).max(0.05));
                let n = ((z - a) / 0.26).round().max(1.0) as usize;
                for k in 0..n {
                    let c = a + (z - a) * (k as f32 + 0.5) / n as f32;
                    for (f0, f1, w) in [(0.0, 0.1, 0.15), (0.1, 0.55, 0.13), (0.55, 0.88, 0.075), (0.88, 1.0, 0.13)] {
                        block(tris, sign, c - w * 0.5, c + w * 0.5, 0.12 - w * 0.5, 0.12 + w * 0.5, lo + f0 * tall, lo + f1 * tall, stone);
                    }
                }
            }
            Railing::Posts => {
                let (a, z) = ends(tris, 0.18, -0.04, rh + 0.18, wood);
                for p in spaced(s0, e, 1.6) {
                    if b.pillars && (p <= s0 + 1e-3 || p >= e - 1e-3) { continue; }
                    let p = p.clamp(s0, e - 0.1);
                    block(tris, sign, p, p + 0.1, 0.0, 0.1, 0.0, rh + 0.06, wood);
                }
                for (lo, hi) in [(rh - 0.08, rh), (rh * 0.5 - 0.06, rh * 0.5)] {
                    run(tris, sign, a, z, -0.035, 0.0, lo, hi, 0.0, wood);
                }
            }
            Railing::Iron => {
                let (a, z) = ends(tris, 0.42, -0.06, rh + 0.14, stone);
                run(tris, sign, a, z, -0.02, 0.14, 0.0, 0.1, 0.0, stone);
                for (lo, hi) in [(rh - 0.05, rh), (0.16, 0.2)] {
                    run(tris, sign, a, z, 0.03, 0.08, lo, hi, 0.0, paint);
                }
                // Bars run through the top rail to a point; a stouter post every few metres.
                let n = ((z - a) / 0.12).round().max(1.0) as usize;
                for k in 0..n {
                    let c = a + (z - a) * (k as f32 + 0.5) / n as f32;
                    block(tris, sign, c - 0.01, c + 0.01, 0.045, 0.065, 0.1, rh + 0.05, paint);
                }
                for p in spaced(a, z, 2.4) {
                    if b.pillars && (p <= a + 1e-3 || p >= z - 1e-3) { continue; }
                    let p = p.clamp(a, z - 0.05);
                    block(tris, sign, p, p + 0.05, 0.03, 0.08, 0.1, rh + 0.1, paint);
                }
            }
            Railing::Rope => {
                let (a, z) = ends(tris, 0.2, -0.02, rh + 0.25, wood);
                let posts: Vec<f32> = spaced(a, z, 2.5).collect();
                for &p in &posts {
                    if b.pillars && (p <= a + 1e-3 || p >= z - 1e-3) { continue; }
                    let p = p.clamp(a, z - 0.13);
                    block(tris, sign, p, p + 0.13, 0.0, 0.13, 0.0, rh + 0.1, wood);
                }
                for w in posts.windows(2) {
                    let sag = 0.06 + 0.03 * (w[1] - w[0]);
                    for (lo, hi) in [(rh - 0.04, rh), (rh * 0.55 - 0.035, rh * 0.55)] {
                        run(tris, sign, w[0], w[1], -0.035, 0.0, lo, hi, sag, paint);
                    }
                }
            }
        }
    }
}

/// Texture coordinates (in repeats) and which texture a G-buffer pixel uses.
#[inline]
fn surface_uv(ctx: &Ctx, g: &GPixel) -> Option<(f32, f32, u8)> {
    let dw = g.d + ctx.scroll;
    let rot = |m: &Material, u: f32, v: f32| if m.rotate { (v, -u) } else { (u, v) };
    match g.id {
        id::GROUND if ctx.on_bridge(g.d) => {
            let (u, v) = rot(&ctx.scene.path.bridge.deck, g.x / ctx.deck_tile, dw / ctx.deck_tile);
            Some((u, v, 4))
        }
        id::CHASM => Some((g.x / ctx.bottom_tile, dw / ctx.bottom_tile, 5)),
        id::CLIFF => Some(if ctx.scene.walls.enabled {
            ((g.x + dw) / ctx.wall_tile, -g.y / ctx.wall_tile, 2)
        } else if ctx.scene.verge.enabled {
            (g.x / ctx.verge_tile, -g.y / ctx.verge_tile, 1)
        } else {
            (g.x / ctx.path_tile, -g.y / ctx.path_tile, 0)
        }),
        r if id::is_rail(r) => {
            let (m, face) = id::rail_parts(r);
            let (u, v) = match face { id::TOP => (dw, g.x), id::FRONT => (g.x, -g.y), _ => (dw, -g.y) };
            Some(match m {
                id::STONE if ctx.scene.walls.enabled => (u / ctx.wall_tile, v / ctx.wall_tile, 2),
                id::STONE => (u / ctx.path_tile, v / ctx.path_tile, 0),
                id::WOOD => (u / ctx.deck_tile, v / ctx.deck_tile, 4),
                // Iron or rope: `rail_color`, with the grain of the deck in it.
                _ => (u / ctx.deck_tile, v / ctx.deck_tile, 7),
            })
        }
        id::FACADE => {
            let (_, tile) = ctx.facade.as_ref()?;
            Some((g.x / tile, -g.y / tile, 6))
        }
        id::GROUND | id::RISER => {
            // A riser continues the texture of the step above it, as if folded down over the edge.
            let along = if g.id == id::RISER { dw + g.y } else { dw };
            // Only the ground left between a fork's branches turns to verge; patches of this world's
            // path mixing into the branches keep their surface.
            let between = ctx.scene.verge.enabled && ctx.verge_beyond.is_some_and(|z| g.d > z) && ctx.bounds.is_some_and(|b| {
                let c = ctx.view.to_cam(g.x, g.y, g.d);
                b.region_of(c[0], c[2]) == 0
            });
            let on_path = !between && (!ctx.scene.verge.enabled || g.x.abs() < ctx.path_edge(g.d) || ctx.on_fork(g.x, g.d));
            if on_path {
                // A waterway's current carries its pattern towards the camera.
                let along = along + ctx.water.map_or(0.0, |k| k.shift);
                let (u, v) = rot(&ctx.scene.path.material, g.x / ctx.path_tile, along / ctx.path_tile);
                Some((u, v, 0))
            } else {
                let (u, v) = rot(&ctx.scene.verge.material, g.x / ctx.verge_tile, along / ctx.verge_tile);
                Some((u, v, 1))
            }
        }
        id::WALL_L | id::WALL_R => {
            let (u, v) = rot(&ctx.scene.walls.material, dw / ctx.wall_tile, -g.y / ctx.wall_tile);
            Some((u, v, 2))
        }
        id::CEILING => {
            let (u, v) = rot(&ctx.scene.ceiling.material, g.x / ctx.ceil_tile, dw / ctx.ceil_tile);
            Some((u, v, 3))
        }
        _ => None,
    }
}

/// What a glossy floor pixel needs from the reflection pass: how much it reflects (fog included),
/// the colour it already assumed from the sky, how rough it is, and its ripple slope.
#[repr(C)]
#[derive(Clone, Copy, Default)]
#[cfg_attr(feature = "gpu", derive(bytemuck::Pod, bytemuck::Zeroable))]
struct Refl { r: f32, env: [f32; 3], rough: f32, sx: f32 }

/// Gloss and ripples of the material a floor pixel shows.
fn gloss_of(ctx: &Ctx, tex: u8, gid: u8) -> (f32, f32) {
    if gid != id::GROUND && gid != id::CHASM { return (0.0, 0.0); }
    let s = ctx.scene;
    let m = match tex {
        0 => &s.path.material,
        1 => &s.verge.material,
        4 => &s.path.bridge.deck,
        5 if s.path.bridge.bottom == BridgeBottom::Water => return (1.0, 0.3),
        5 => if s.verge.enabled { &s.verge.material } else { &s.path.material },
        _ => return (0.0, 0.0),
    };
    (m.gloss.clamp(0.0, 1.0), m.ripples.clamp(0.0, 1.0))
}

/// Slope of the water surface at (x, w): three wave trains that drift with the world and cycle a
/// whole number of times per loop. Waves shorter than a few pixels fade out instead of sparkling.
const WAVES: [(f32, f32, f32, f32); 4] = [(1.9, 0.5, 0.0, 2.0), (1.15, -0.8, 1.3, 3.0), (0.62, 0.2, 2.1, 5.0), (0.37, 1.1, 0.7, 7.0)];
/// Sum of |cos heading| over WAVES: the most sideways slope they can add up to, per unit amplitude.
const WAVE_COS_SUM: f32 = 3.008;

fn ripple_slope(ctx: &Ctx, x: f32, w: f32, ripples: f32, footprint: f32) -> (f32, f32) {
    if ripples <= 0.0 { return (0.0, 0.0); }
    let t = ctx.tphase * TAU;
    let (mut sx, mut sz) = (0.0f32, 0.0f32);
    // Each wave train's heading, worked out once rather than per pixel.
    static HEADINGS: std::sync::LazyLock<[(f32, f32); 4]> = std::sync::LazyLock::new(|| WAVES.map(|(_, ang, _, _)| (ang.cos(), ang.sin())));
    for ((lambda, _, ph, cyc), (dx, dz)) in WAVES.into_iter().zip(*HEADINGS) {
        let fade = ((lambda / footprint.max(1e-3) - 3.0) / 6.0).clamp(0.0, 1.0);
        if fade <= 0.0 { continue; }
        let k = TAU / lambda;
        let s = 0.13 * ripples * fade * (k * (x * dx + w * dz) + ph - cyc * t).cos();
        sx += s * dx;
        sz += s * dz;
    }
    (sx, sz)
}

/// The sky (or the dark beyond the world) a world shows on row `y` where nothing is drawn.
fn sky_base(ctx: &Ctx, y: usize, layers: &Layers) -> [f32; 3] {
    let scene = ctx.scene;
    let hy = ctx.view.horizon_px.max(1.0);
    let top = ctx.view.top as f32;
    let (sky_top, sky_hor) = (rgb_lin(scene.sky.top), rgb_lin(scene.sky.horizon));
    if layers.sky && scene.sky.enabled && (y as f32) < hy {
        mix3(sky_top, sky_hor, ((y as f32 - top) / (hy - top).max(1.0)).clamp(0.0, 1.0).powf(1.3))
    } else if layers.sky && scene.sky.enabled && ctx.fog.is_some() && scene.light.fog.match_sky {
        sky_hor
    } else {
        ctx.void_lin
    }
}

/// A world's sky as the eye sees it in a frame of several: at its adaptation, through the first
/// world's air when it lies beyond a boundary.
fn sky_seen(ctx: &Ctx, c: [f32; 3]) -> [f32; 3] {
    let c = scale3(c, ctx.gain);
    match ctx.front.filter(|f| f.zb > 0.0) {
        Some(fa) => match fa.fog { Some((_, dist)) => mix3(scale3(fa.fog_col, fa.gain), c, (-fa.zb / dist).exp()), None => c },
        None => c,
    }
}

/// Light one G-buffer pixel as world `ctx` sees it: its colour (fog and adaptation included) and
/// what the reflection pass needs, or None where nothing is drawn.
#[allow(clippy::too_many_arguments)]
fn shade_px(ctx: &Ctx, g: &GPixel, gbuf: &[GPixel], i: usize, x: usize, y: usize, sun_mask: &[f32], ao_mask: &[f32], layers: &Layers, near_lights: Option<&[u16]>, uvs: Option<&[Option<(f32, f32, u8)>]>) -> Option<([f32; 3], Refl)> {
    let (w, h) = (ctx.view.width, ctx.view.height);
    let hy = ctx.view.horizon_px.max(1.0);
    let top = ctx.view.top as f32;
    let sky_t = |row: f32| ((row - top) / (hy - top).max(1.0)).clamp(0.0, 1.0).powf(1.3);
    let (sky_top, sky_hor) = (rgb_lin(ctx.scene.sky.top), rgb_lin(ctx.scene.sky.horizon));
    let mut rf = Refl::default();
    let scene = ctx.scene;
    // `uvs` holds every pixel's surface_uv, worked out once, when `g` is the G-buffer's own pixel.
    let (u, v, tex) = match uvs { Some(b) => b[i]?, None => surface_uv(ctx, g)? };
    // Footprint from neighbouring pixels on the same surface, like GPU derivatives.
    let mut fp = 0.0f32;
    for j in [if x + 1 < w { i + 1 } else { i - 1 }, if y + 1 < h { i + w } else { i - w }] {
        if gbuf[j].id == g.id && gbuf[j].realm == g.realm {
            if let Some((u2, v2, t2)) = match uvs { Some(b) => b[j], None => surface_uv(ctx, &gbuf[j]) } {
                if t2 == tex { fp = fp.max((u2 - u).abs().max((v2 - v).abs())); }
            }
        }
    }
    let texture = match tex { 0 => &ctx.path_tex, 1 => &ctx.verge_tex, 2 => &ctx.wall_tex, 3 => &ctx.ceil_tex, 4 | 7 => &ctx.deck_tex, 6 => &ctx.facade.as_ref().unwrap().0, _ => &ctx.bottom_tex };
    let mut albedo = texture.sample(u, v, fp.min(4.0));
    // Glowing parts of the texture shine in their own colour; railings take only its grain.
    let mut glow_c = if tex == 7 { [0.0; 3] } else { scale3(albedo, texture.sample_emit(u, v, fp.min(4.0)) * GLOW_K) };
    if let Some(k) = edge_k(ctx).filter(|_| g.id == id::GROUND && (tex == 0 || tex == 4) && !ctx.on_fork(g.x, g.d)) {
        let (mut px_x, mut px_d) = (0.0f32, 0.0f32);
        for j in [if x > 0 { i - 1 } else { i + 1 }, if x + 1 < w { i + 1 } else { i - 1 }] {
            if gbuf[j].id == id::GROUND { px_x = px_x.max((gbuf[j].x - g.x).abs()); }
        }
        for j in [if y > 0 { i - w } else { i + w }, if y + 1 < h { i + w } else { i - w }] {
            if gbuf[j].id == id::GROUND { px_d = px_d.max((gbuf[j].d - g.d).abs()); }
        }
        // On a bridge the lights run inside the railing.
        let inset = if ctx.on_bridge(g.d) { ctx.bridge.map_or(0.0, |b| b.rail_inset()) } else { 0.0 };
        glow_c = add3(glow_c, edge_light(&k, (ctx.path_edge(g.d) - inset).max(0.05), g.x, ctx.scroll + g.d, px_x, px_d, albedo));
    }
    if tex == 7 {
        let b = &scene.path.bridge;
        let luma = |c: [f32; 3]| 0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2];
        let grain = (luma(albedo) / luma(rgb_lin(b.deck.base)).max(1e-3)).clamp(0.6, 1.3);
        let paint = if id::is_rail(g.id) && id::rail_parts(g.id).0 == id::FENCE { fence_paint(scene) } else { rgb_lin(b.rail_color) };
        albedo = scale3(paint, grain);
    }
    // A waterway: the foam and what floats on the water, and the bank or walls wet beside it.
    let (mut gloss_k, mut wet_g) = (1.0f32, 0.0f32);
    if let Some(k) = &ctx.water {
        if g.id == id::GROUND && tex == 0 {
            let px = g.depth / ctx.view.focal_px;
            let (c, a, shine) = water::water_cover(k, g.x, ctx.scroll + g.d, water::water_dist(ctx, g.x, g.d), px);
            albedo = mix3(albedo, c, a);
            gloss_k = shine;
        } else if g.id == id::GROUND && tex == 1 {
            let wt = water::wetness(k, -water::water_dist(ctx, g.x, g.d));
            albedo = scale3(albedo, 1.0 - 0.45 * wt);
            wet_g = 0.35 * wt;
        } else if g.id == id::WALL_L || g.id == id::WALL_R {
            albedo = scale3(albedo, 1.0 - 0.45 * water::wetness(k, g.y));
        }
    }
    // What grows on the ground hides it.
    if !ctx.cover.is_empty() && g.id == id::GROUND && tex <= 1 {
        let (c, a) = cover::at(ctx, &ctx.cover, g, tex, cover::jitter(i));
        if a > 0.0 {
            albedo = mix3(albedo, c, a);
            gloss_k *= 1.0 - a;
            wet_g *= 1.0 - a;
        }
    }
    let mut ao = ao_mask[i];
    let (normal, skip) = match g.id {
        id::GROUND => {
            ao *= passage_dark(ctx, g);
            if tex == 0 && ctx.fork.is_some_and(|f| f.split) && !scene.walls.enabled {
                ao *= ctx.split_edge_dark(g.x, g.d);
            } else if tex == 0 && !ctx.on_fork(g.x, g.d) {
                let edge = ctx.path_edge(g.d).max(0.05);
                ao *= 1.0 - scene.path.edge_dark.clamp(0.0, 1.0) * smoothstep(0.5, 1.0, g.x.abs() / edge);
            }
            if scene.walls.enabled {
                let dist = (ctx.wall_x(g.d) - g.x.abs()).max(0.0);
                ao *= 1.0 - scene.walls.base_shadow.clamp(0.0, 1.0) * (-dist / 0.45).exp();
            }
            if let Some(st) = &ctx.view.stairs {
                let at = ctx.scroll + g.d;
                // The worn front edge of each step (its nosing) catches the light; it is what makes
                // a flight read, above all going down, where the risers face away.
                // Both are a few centimetres wide: further away than a pixel covers, they widen
                // to the pixel and dim to match, so they average out instead of flickering.
                let mut px_d = 0.0f32;
                for j in [i.wrapping_sub(w), i + w] {
                    if j < gbuf.len() && gbuf[j].id == id::GROUND { px_d = px_d.max((gbuf[j].d - g.d).abs()); }
                }
                let filt = |width: f32| { let wd = width.max(px_d); (wd, width / wd) };
                let nose = if st.rise > 0.0 { st.since_riser(at) } else { st.to_next_riser(at) };
                let (nw, nk) = filt(0.035);
                ao *= 1.0 + 0.35 * nk * smoothstep(nw, 0.0, nose);
                // Contact shade where a step meets the riser of the next one up.
                if st.rise > 0.0 {
                    let (cw, ck) = filt(0.06);
                    ao *= 1.0 - 0.45 * ck * (-st.to_next_riser(at) / cw).exp();
                }
            }
            ([0.0, 1.0, 0.0], 0)
        }
        id::RISER => {
            // Faces the camera on the way up (on the way down risers are hidden behind their steps).
            if tex == 0 {
                let edge = ctx.path_edge(g.d).max(0.05);
                ao *= 1.0 - scene.path.edge_dark.clamp(0.0, 1.0) * smoothstep(0.5, 1.0, g.x.abs() / edge);
            }
            // Shade under the nosing, at the top of the riser.
            ao *= 0.9 * (1.0 - 0.35 * smoothstep(-0.03, 0.0, g.y));
            let up = ctx.view.stairs.map_or(true, |st| st.rise > 0.0);
            ([0.0, 0.0, if up { -1.0 } else { 1.0 }], 0)
        }
        id::CHASM => {
            let b = &scene.path.bridge;
            // Little light reaches the bottom of a deep gap.
            ao *= 0.25 + 0.75 * (-ctx.bridge.map_or(b.depth, |sp| sp.floor).max(0.0) / 8.0).exp();
            if b.bottom == BridgeBottom::Ground { albedo = mul3(albedo, rgb_lin(b.bottom_color)); }
            ([0.0, 1.0, 0.0], 0)
        }
        id::CLIFF => { ao *= 0.85 * passage_dark(ctx, g); ([0.0, 0.0, -1.0], 0) }
        r if id::is_rail(r) => {
            let out = if g.x > 0.0 { 1.0 } else { -1.0 };
            // A little shade where it stands on the deck.
            ao *= 1.0 - 0.3 * (-g.y.max(0.0) / 0.25).exp();
            // (An end faces the camera: back along the path, or ahead where a turned camera sees
            // it from behind.)
            let end = if g.d < 0.0 { 1.0 } else { -1.0 };
            (match id::rail_parts(r).1 { id::INNER => [-out, 0.0, 0.0], id::OUTER => [out, 0.0, 0.0], id::TOP => [0.0, 1.0, 0.0], _ => [0.0, 0.0, end] }, 0)
        }
        id::FACADE => {
            // The face darkens into the opening's reveal and toward its foot.
            if let Some(op) = &ctx.opening { ao *= 0.5 + 0.5 * smoothstep(0.0, 0.35, op.rim_distance(g.x, g.y)); }
            ao *= 1.0 - 0.3 * (-g.y / 0.6).exp();
            ([0.0, 0.0, -1.0], 0)
        }
        id::WALL_L => { ao *= 1.0 - scene.walls.base_shadow.clamp(0.0, 1.0) * (-g.y / 0.5).exp(); ([1.0, 0.0, 0.0], id::WALL_L) }
        id::WALL_R => { ao *= 1.0 - scene.walls.base_shadow.clamp(0.0, 1.0) * (-g.y / 0.5).exp(); ([-1.0, 0.0, 0.0], id::WALL_R) }
        _ => { ao *= passage_dark(ctx, g); ([0.0, -1.0, 0.0], id::CEILING) }
    };
    let (gloss, ripples) = gloss_of(ctx, tex, g.id);
    let gloss = (gloss * gloss_k).max(wet_g);
    // Rain darkens and wets, puddles mirror, snow covers.
    let (albedo, gloss, ripples, rings) = weather::surface(ctx, g, tex, albedo, gloss, ripples);
    // Water shows a cast shadow only faintly: most of what it returns is reflection.
    let sun_d = if ctx.water.is_some() && g.id == id::GROUND && tex == 0 { 1.0 - 0.4 * (1.0 - sun_mask[i]) } else { sun_mask[i] };
    let light = ctx.light_among(g.x, g.y, g.d, Some(normal), skip, sun_d, None, near_lights);
    let mut c = mul3(albedo, scale3(light, ao));
    if gloss > 0.001 {
        let p = ctx.view.to_cam(g.x, g.y, g.d);
        let plen = dot3(p, p).sqrt().max(1e-4);
        let v = [-p[0] / plen, -p[1] / plen, -p[2] / plen];
        let eye = (-p[1]).max(0.05);
        let footprint = (p[2] * p[2] / (ctx.view.focal_px * eye)).max(p[2] / ctx.view.focal_px);
        let (sx, sz) = ripple_slope(ctx, g.x, ctx.scroll + g.d, ripples, footprint);
        let (sx, sz) = (sx + rings.0, sz + rings.1);
        let n = { let n = [-sx, 1.0, -sz]; let l = dot3(n, n).sqrt(); ctx.view.rot_dir([n[0] / l, n[1] / l, n[2] / l]) };
        let ndv = dot3(n, v).max(1e-3);
        let fres = |c: f32| 0.02 + 0.98 * (1.0 - c).max(0.0).powi(5);
        let r = gloss * fres(ndv);
        // Wet stone is rough and smears what it reflects; still water is a mirror.
        let rough = (0.04 + 0.45 * (1.0 - gloss) + 0.12 * ripples).min(0.7);
        // What the surface reflects when the trace finds nothing on screen: the sky at the
        // reflected elevation, or the dim room.
        let rv = [2.0 * ndv * n[0] - v[0], 2.0 * ndv * n[1] - v[1], 2.0 * ndv * n[2] - v[2]];
        let env = if layers.sky && scene.sky.enabled {
            let row_at = if rv[2] > 1e-3 { hy - ctx.view.focal_px * rv[1].max(0.0) / rv[2] } else { 0.0 };
            mix3(sky_top, sky_hor, sky_t(row_at))
        } else {
            add3(scale3(ctx.ambient, 0.3), if ctx.fog.is_some() || scene.weather.fog_banks.enabled { scale3(ctx.fog_col, 0.5) } else { [0.0; 3] })
        };
        // Glints: every lamp, and the sun or moon, as a specular lobe.
        let e = (2.0 / (rough * rough) - 2.0).clamp(4.0, 600.0);
        let norm = (e + 8.0) / (8.0 * std::f32::consts::PI);
        let mut spec = [0.0f32; 3];
        let mut lobe = |l: [f32; 3], col: [f32; 3], k: f32| {
            let ndl = dot3(n, l);
            if ndl <= 0.0 { return; }
            let hv = { let hv = [l[0] + v[0], l[1] + v[1], l[2] + v[2]]; let hl = dot3(hv, hv).sqrt().max(1e-5); [hv[0] / hl, hv[1] / hl, hv[2] / hl] };
            let s = norm * dot3(n, hv).max(0.0).powf(e) * fres(dot3(hv, v).max(0.0)) * ndl * gloss * k;
            spec = add3(spec, scale3(col, s));
        };
        let picked = near_lights.map(|l| l.iter().map(|&k| &ctx.lights[k as usize]));
        let all = near_lights.is_none().then(|| ctx.lights.iter());
        for pl in picked.into_iter().flatten().chain(all.into_iter().flatten()) {
            let lp = [pl.pos[0] - p[0], pl.pos[1] - p[1], pl.pos[2] - p[2]];
            let d2 = dot3(lp, lp);
            let r2 = pl.radius * pl.radius;
            if d2 >= r2 { continue; }
            let wgt = 1.0 - d2 / r2;
            let dist = d2.sqrt().max(1e-4);
            lobe([lp[0] / dist, lp[1] / dist, lp[2] / dist], pl.color, wgt * wgt / (1.0 + 0.3 * d2));
        }
        for sl in &ctx.sky_lights { lobe(ctx.view.rot_dir(sl.dir), sl.color, sun_mask[i]); }
        c = add3(add3(scale3(c, 1.0 - r), scale3(env, r)), spec);
        rf = Refl { r: r * ctx.fog_t(g.depth), env: scale3(env, ctx.gain), rough, sx };
    }
    Some((ctx.apply_fog(add3(c, glow_c), g.depth), rf))
}

/// A texel glowing 1 (Material.glow 1 on a lit part) shines this many times its own colour.
const GLOW_K: f32 = 2.5;

/// The geometry pass's input for a frame of one world: its triangles set up and binned, the shadow
/// casters, cloud shade and (with enough lamps to be worth it) the lamps to sort into tiles.
#[cfg(feature = "gpu")]
fn gpu_geom_input(ctx: &Ctx, prepared: &[raster::Prepared], bills: &[Billboard], layers: &Layers) -> super::gpu::GeomIn {
    use super::gpu::{bin_tiles, CasterGpu, CasterIn, TriGpu};
    let (w, h) = (ctx.view.width, ctx.view.height);
    let tris: Vec<TriGpu> = prepared.par_iter().map(TriGpu::new).collect();
    // Small triangles run one thread each on the GPU; the rest are binned into tiles.
    let small_tri = std::env::var("PF_SMALL_TRI").ok().and_then(|v| v.parse().ok()).unwrap_or(raster::SMALL_TRI);
    let skip: Vec<bool> = prepared.iter().map(|t| t.max_x - t.min_x < small_tri && t.max_y - t.min_y < small_tri).collect();
    let tri_bins = super::gpu::bin_triangles(w, h, prepared, &skip);
    let small: Vec<u32> = skip.iter().enumerate().filter(|(_, &s)| s).map(|(i, _)| i as u32).collect();
    let (sun, list) = if layers.ground { shadow_casters(ctx, bills) } else { (None, Vec::new()) };
    let bits = |v: usize| f32::from_bits(v as u32);
    let mut cboxes = Vec::with_capacity(list.len());
    let casters: Vec<CasterIn> = list.iter().map(|c| {
        let mut f = [0.0f32; 40];
        (f[0], f[1], f[2], f[3], f[4], f[5], f[6]) = (c.foot[0], c.foot[1], c.axis[0], c.axis[1], c.ra, c.shadow, c.rb);
        let (mut x0, mut y0, mut x1, mut y1) = (i64::MAX, i64::MAX, i64::MIN, i64::MIN);
        let mut grow = |r: (usize, usize, usize, usize)| { x0 = x0.min(r.0 as i64); y0 = y0.min(r.1 as i64); x1 = x1.max(r.2 as i64); y1 = y1.max(r.3 as i64); };
        if let Some(r) = c.contact { f[8] = bits(1); (f[9], f[10], f[11], f[12]) = (bits(r.0), bits(r.1), bits(r.2), bits(r.3)); grow(r); }
        if let (Some(_), Some(r)) = (sun, c.sun) { f[13] = bits(1); (f[14], f[15], f[16], f[17]) = (bits(r.0), bits(r.1), bits(r.2), bits(r.3)); grow(r); }
        // (19..21: the sprite level, filled in by Frame::geometry; a fence's shape instead.)
        let sprite = match &c.pic {
            Pic::Sprite(b) => {
                f[7] = f32::from_bits(b.flip as u32);
                f[18] = b.panel.and_then(|p| p.strip).unwrap_or(-1.0);
                Some((b.sprite.clone(), b.sprite.lod(192.0)))
            }
            Pic::Fence(fc) => {
                f[18] = -1.0;
                (f[19], f[20], f[21], f[37], f[38]) = (f32::from_bits(fc.bits()), fc.len, fc.ht, f32::from_bits(1), fc.pw);
                None
            }
        };
        f[22..25].copy_from_slice(&c.c0);
        f[25..28].copy_from_slice(&c.across);
        f[28..31].copy_from_slice(&c.down);
        (f[31], f[32]) = (c.ia, c.idn);
        f[33..36].copy_from_slice(&c.n);
        f[36] = c.nl;
        cboxes.push(if x0 <= x1 { (x0, y0, x1, y1) } else { (1, 1, 0, 0) });
        CasterIn { c: CasterGpu { f }, sprite }
    }).collect();
    let cast_bins = bin_tiles(w, h, &cboxes);
    let clouds = weather::cloud_params(ctx);
    let lamps = (ctx.lights.len() >= 4).then(|| ctx.lights.iter().map(|l| [l.pos[0], l.pos[1], l.pos[2], l.radius, l.color[0], l.color[1], l.color[2], 0.0]).collect());
    super::gpu::GeomIn { tris, tri_bins, small, casters, cast_bins, sun, clouds, lamps, fade: raster::Dissolve::of(&ctx.view) }
}

/// What a world adds to the GPU's parameters in a frame of several: the boundary its view joins
/// across, the branches' shear, the first world's air in front of it, light through a threshold,
/// the face round it, and the ground between a fork's branches.
#[cfg(feature = "gpu")]
fn gpu_world_extras(p: &mut super::gpu::WorldParams, ctx: &Ctx, ctxs: &[Ctx]) {
    let v = &ctx.view;
    if let Some(sp) = &v.split {
        (p.sp_on, p.sp_zb, p.sp_hw_b, p.sp_flare_b, p.sp_taper) = (1, sp.zb, sp.half_width_b, sp.flare_b, sp.taper);
        if let Some(st) = sp.stairs_a { (p.sa_on, p.sa_period, p.sa_run, p.sa_rise, p.sa_steps, p.sa_offset) = (1, st.period, st.run, st.rise, st.steps as f32, st.offset); }
        if let Some(st) = sp.stairs_b { (p.sb_on, p.sb_period, p.sb_run, p.sb_rise, p.sb_steps, p.sb_offset) = (1, st.period, st.run, st.rise, st.steps as f32, st.offset); }
        (p.sa_scroll, p.sb_scroll) = (sp.scroll_a, sp.scroll_b);
    }
    for (r, c) in ctxs.iter().enumerate().take(super::gpu::MAX_WORLDS) {
        if let Some((z0, k)) = c.view.shear { (p.sh_z0[r], p.sh_k[r]) = (z0, k); }
    }
    if let Some(fa) = &ctx.front {
        (p.fa_on, p.fa_zb, p.fa_col, p.fa_gain) = (1, fa.zb, fa.fog_col, fa.gain);
        if let Some((_, dist)) = fa.fog { (p.fa_fog_on, p.fa_fog_dist) = (1, dist); }
    }
    if let Some(pt) = &ctx.portal {
        (p.pt_on, p.pt_zb, p.pt_front, p.pt_rad) = (1, pt.zb, pt.front as u32, pt.radiance);
        for (k, c) in pt.corners.iter().enumerate() { p.pt_c[k * 3..k * 3 + 3].copy_from_slice(c); }
    }
    if let Some(op) = &ctx.opening {
        (p.op_on, p.op_hw, p.op_top, p.op_arch, p.op_rim, p.op_seed) = (1, op.hw, op.top, op.arch, op.rim, op.seed);
        (p.op_outer_hw, p.op_outer_top, p.op_hill) = (op.outer_hw, op.outer_top, op.hill);
    }
    if let Some((_, tile)) = &ctx.facade { p.fac_tile = *tile; }
    if let (Some(z), Some(b)) = (ctx.verge_beyond, ctx.bounds) {
        // Past the junction the first world's region is its one piece there: the wedge.
        if let Some(wedge) = b.realms[0].get(1).filter(|w| w.len() == 4) {
            (p.vb_on, p.vb_z) = (1, z);
            for (k, h) in wedge.iter().enumerate() { p.wedge[k * 3..k * 3 + 3].copy_from_slice(&[h.0, h.1, h.2]); }
        }
    }
}

/// The world's parameters as the GPU passes read them (gpu::WorldParams, `World` in common.wgsl).
#[cfg(feature = "gpu")]
fn gpu_params(ctx: &Ctx, layers: &Layers) -> super::gpu::WorldParams {
    let el = edge_k(ctx);
    let (s, v) = (ctx.scene, &ctx.view);
    let b = &s.path.bridge;
    let fb = &s.weather.fog_banks;
    let st = v.stairs;
    let sp = ctx.bridge;
    let fk = ctx.fork;
    let rot = |m: &Material, bit: u32| if m.rotate { bit } else { 0 };
    super::gpu::WorldParams {
        width: v.width as u32, height: v.height as u32, horizon_px: v.horizon_px, center_px: v.center_px,
        focal_px: v.focal_px, eye_height: v.eye_height, bend: v.bend, hill: v.hill,
        half_width: v.half_width, flare: v.flare, near_ground: v.near_ground, scroll: v.scroll,
        top: v.top as f32, left: v.left as f32,
        stairs_on: st.is_some() as u32, st_period: st.map_or(1.0, |t| t.period), st_run: st.map_or(1.0, |t| t.run), st_rise: st.map_or(0.0, |t| t.rise),
        st_steps: st.map_or(1.0, |t| t.steps as f32), st_offset: st.map_or(0.0, |t| t.offset),
        loop_len: ctx.loop_len, tphase: ctx.tphase, far: ctx.far, gain: ctx.gain,
        fog_on: ctx.fog.is_some() as u32, fog_dist: ctx.fog.map_or(1.0, |f| f.1), fog: ctx.fog_col,
        void: ctx.void_lin, ambient: ctx.ambient, sky_top: rgb_lin(s.sky.top), sky_hor: rgb_lin(s.sky.horizon),
        sky_on: (layers.sky && s.sky.enabled) as u32, match_sky: s.light.fog.match_sky as u32,
        env_fog: (ctx.fog.is_some() || fb.enabled) as u32,
        edge_noise: s.path.edge_noise, edge_dark: s.path.edge_dark, verge_on: s.verge.enabled as u32, walls_on: s.walls.enabled as u32,
        walls_gap: s.walls.gap, base_shadow: s.walls.base_shadow, wall_top: ctx.wall_top(), ceiling_on: s.ceiling.enabled as u32,
        bands: s.light.bands,
        path_tile: ctx.path_tile, verge_tile: ctx.verge_tile, wall_tile: ctx.wall_tile, ceil_tile: ctx.ceil_tile, deck_tile: ctx.deck_tile, bottom_tile: ctx.bottom_tile,
        rotate: rot(&s.path.material, 1) | rot(&s.verge.material, 2) | rot(&s.walls.material, 4) | rot(&s.ceiling.material, 8) | rot(&b.deck, 16),
        path_gloss: s.path.material.gloss, path_rip: s.path.material.ripples, verge_gloss: s.verge.material.gloss, verge_rip: s.verge.material.ripples,
        deck_gloss: b.deck.gloss, deck_rip: b.deck.ripples,
        bridge_on: sp.is_some() as u32, br_period: sp.map_or(1.0, |p| p.period), br_len: sp.map_or(0.0, |p| p.len), br_offset: sp.map_or(0.0, |p| p.offset),
        br_floor: sp.map_or(b.depth, |p| p.floor),
        br_bottom: match b.bottom { BridgeBottom::Ground => 0, BridgeBottom::Water => 1, BridgeBottom::Void => 2 },
        bottom: rgb_lin(b.bottom_color), deck_base: rgb_lin(b.deck.base), rail: rgb_lin(b.rail_color),
        fork_on: fk.is_some() as u32, fk_period: fk.map_or(1.0, |f| f.period), fk_offset: fk.map_or(0.0, |f| f.offset),
        fk_side: fk.map_or(0, |f| match f.side { ForkSide::Left => 0, ForkSide::Right => 1, ForkSide::Alternate => 2, ForkSide::Both => 3 }),
        fk_tan: fk.map_or(1.0, |f| f.tan), fk_hw: fk.map_or(0.0, |f| f.hw), fk_per_loop: fk.map_or(1, |f| f.per_loop as u32),
        fb_on: fb.enabled as u32, fb_density: fb.density, fb_spacing: fb.spacing, fb_length: fb.length, fb_offset: fb.offset,
        wx_wet: ctx.wx.wet, wx_puddles: ctx.wx.puddles, wx_snow: ctx.wx.snow, wx_track: ctx.wx.track, wx_rings: ctx.wx.rings,
        precip_seed: s.weather.precipitation.seed, loop_seconds: s.motion.loop_seconds(),
        n_lights: ctx.lights.len() as u32, n_sky: ctx.sky_lights.len() as u32, tiles_on: 0, tile_cols: 0,
        zero: 0, fk_style: fk.map_or(0, |f| f.split as u32), fk_noise: fk.map_or(0.0, |f| f.noise), tile_cap: 0,
        el_on: el.is_some() as u32, el_col: el.map_or([0.0; 3], |e| e.col), el_hw: el.map_or(0.0, |e| e.hw), el_inset: el.map_or(0.0, |e| e.inset),
        el_period: el.map_or(0.0, |e| e.period), el_phase: el.map_or(0.0, |e| e.phase),
        ww_on: ctx.water.is_some() as u32,
        ww_shift: ctx.water.map_or(0.0, |k| k.shift), ww_foam: ctx.water.map_or(0.0, |k| k.foam), ww_foam_col: ctx.water.map_or([0.0; 3], |k| k.foam_col),
        ww_foam_w: ctx.water.map_or(1.0, |k| k.foam_w), ww_lap_ph: ctx.water.map_or(0.0, |k| k.lap_ph), ww_lap_k: ctx.water.map_or(0.0, |k| k.lap_k),
        ww_k1: ctx.water.map_or(0.0, |k| k.k1), ww_k2: ctx.water.map_or(0.0, |k| k.k2), ww_wet: ctx.water.map_or(0.0, |k| k.wet),
        ww_kind: ctx.water.map_or(0, |k| k.kind), ww_dens: ctx.water.map_or(0.0, |k| k.dens), ww_fcol: ctx.water.map_or([0.0; 3], |k| k.fcol),
        ww_fsize: ctx.water.map_or(1.0, |k| k.fsize), ww_period: ctx.water.map_or(1.0, |k| k.period), ww_cs: ctx.water.map_or(1.0, |k| k.cs), ww_seed: ctx.water.map_or(0, |k| k.seed),
        br_inset: ctx.bridge.map_or(0.0, |b| b.rail_inset()), ww_frozen: ctx.water.map_or(0.0, |k| k.frozen), ww_hw: ctx.water.map_or(1.0, |k| k.hw), fence: fence_paint(s), spare: [0.0; 2], wind: v.wind_now(), gc_n: ctx.cover.len() as u32, gc_sway: ctx.wx.sway_parts().0, gc_lean: ctx.wx.sway_parts().1, gc_phase: ctx.wx.sway_parts().2, gc: { let mut a = [[0.0f32; 4]; 12]; for (i, k) in ctx.cover.iter().enumerate() { a[i * 4..i * 4 + 4].copy_from_slice(&k.rec()); } a },
        cam_yc: v.yaw_c, cam_ys: v.yaw_s, cam_x: v.cam_x, cam_turned: v.turned as u32,
        ..bytemuck::Zeroable::zeroed()
    }
}

/// The path's edge lights for a frame (`EdgeLights`), as both renderers use them.
#[derive(Clone, Copy, Debug)]
struct EdgeK { col: [f32; 3], hw: f32, inset: f32, period: f32, phase: f32 }

fn edge_k(ctx: &Ctx) -> Option<EdgeK> {
    let e = &ctx.scene.path.edge_lights;
    if !e.enabled || e.strength <= 0.0 { return None; }
    Some(EdgeK {
        col: scale3(rgb_lin(e.color), e.strength),
        hw: e.width.max(0.005) * 0.5,
        inset: e.inset,
        period: if e.dash > 0.0 { snap_to_loop(e.dash, ctx.loop_len) } else { 0.0 },
        phase: e.flow as f32 * ctx.tphase,
    })
}

/// Light from the edge lights at a ground pixel: the lines themselves and the floor they light.
/// `px_x` and `px_d` are how far a pixel reaches across and along the path, metres.
fn edge_light(k: &EdgeK, edge: f32, x: f32, dw: f32, px_x: f32, px_d: f32, albedo: [f32; 3]) -> [f32; 3] {
    let d = (x.abs() - (edge - k.inset)).abs();
    // A line narrower than the pixel widens to it and dims to match.
    let wd = k.hw.max(px_x);
    let line = (k.hw / wd) * smoothstep(wd, 0.0, d);
    let mut dash = 1.0;
    if k.period > 0.0 {
        let f = (dw / k.period - k.phase).rem_euclid(1.0);
        let on = smoothstep(0.0, 0.08, f) * (1.0 - smoothstep(0.5, 0.58, f));
        dash = on + (0.5 - on) * smoothstep(0.25, 0.6, px_d / k.period);
    }
    let spill = 0.8 * (-d / 0.3).exp() * (0.5 + 0.5 * dash);
    add3(scale3(k.col, 2.0 * line * dash), mul3(albedo, scale3(k.col, spill)))
}

/// Which of a GPU frame's buffers hold newer data than the CPU's copies.
#[cfg(feature = "gpu")]
#[derive(Default)]
struct GpuHas { hdr: bool, refl: bool, gbuf: bool, cards: bool, reflects: bool }

/// render::shade on the GPU for one world (the G-buffer and masks are already there).
#[cfg(feature = "gpu")]
fn gpu_shade(f: &mut super::gpu::Frame, ctxs: &[Ctx], mut tiles: Option<TileLights>) {
    let inputs: Vec<_> = ctxs.iter().map(|ctx| super::gpu::ShadeInput {
        textures: [&ctx.path_tex, &ctx.verge_tex, &ctx.wall_tex, &ctx.ceil_tex, &ctx.deck_tex, &ctx.bottom_tex, ctx.facade.as_ref().map_or(&ctx.path_tex, |f| &f.0)],
        lights: ctx.lights.iter().map(|l| [l.pos[0], l.pos[1], l.pos[2], l.radius, l.color[0], l.color[1], l.color[2], 0.0]).collect(),
        sky: ctx.sky_lights.iter().map(|l| [l.dir[0], l.dir[1], l.dir[2], 0.0, l.color[0], l.color[1], l.color[2], 0.0]).collect(),
        tiles: tiles.take().map(|t| t.lists),
    }).collect();
    f.shade(&inputs);
}

/// Post and style's settings for the GPU, from the same scene values `render_worlds` reads.
/// `dims` is (gl, gt, w, h, out_w, out_h, px); `fv` the view after the crop.
#[cfg(feature = "gpu")]
#[allow(clippy::too_many_arguments)]
fn gpu_post_params<'a>(ctx: &Ctx, ctxs: &[Ctx], fv: &View, scene: &Scene, lens: Option<(&Ctx, f32)>, layers: &Layers, opts: &RenderOptions, dims: (usize, usize, usize, usize, usize, usize, usize),
                       crisp: bool, cards: bool, lut: Option<&'a PaletteLut>, levels: Option<(f32, f32)>, grade: Option<&'a super::looks::Grade>) -> super::gpu::PostIn<'a> {
    let (gl, gt, w, h, out_w, out_h, px) = dims;
    let mut p = super::gpu::PostParams { w: w as u32, h: h as u32, gl: gl as u32, gt: gt as u32, out_w: out_w as u32, out_h: out_h as u32, px: px as u32, ..Default::default() };
    p.pick_on = opts.pick as u32;
    let _ = cards;
    p.hz = fv.horizon_px;
    p.unit = 854.0 / h as f32;
    let post_on = layers.post;
    // Heat haze: each world's over its own pixels.
    let haze = |c: &Ctx| {
        let hs = &c.scene.weather.heat_shimmer;
        let ls = c.scene.motion.loop_seconds();
        (post_on && hs.enabled && hs.strength > 0.0).then(|| (hs.strength, cycles(1.1 * hs.speed.max(0.0), ls), cycles(1.9 * hs.speed.max(0.0), ls), c.tphase))
    };
    let hazes: Vec<_> = ctxs.iter().map(haze).collect();
    let shimmer = hazes.iter().any(|h| h.is_some());
    if shimmer {
        p.sh_on = 1;
        if let Some(Some((s0, a, b, t))) = hazes.first() { (p.sh_strength, p.sh_c1, p.sh_c2, p.sh_t0) = (*s0, *a, *b, *t); }
        for r in 1..hazes.len().min(3) {
            if let Some((s0, a, b, t)) = hazes[r] { (p.sh_s[r - 1], p.sh_a[r - 1], p.sh_b[r - 1], p.sh_t[r - 1]) = (s0, a, b, t); }
        }
    }
    // The lens (raindrops or frost) of the world `lens` names, at its weight and on its clock.
    let mut drops = Vec::new();
    if let Some((lc, weight)) = lens {
        let l = &lc.scene.weather.lens;
        let lls = lc.scene.motion.loop_seconds();
        p.lens_weight = weight;
        p.lens_seed = l.seed;
        p.lens_tphase = lc.tphase;
        match l.kind {
            LensKind::Drops => {
                p.lens_kind = 1;
                let k = lens::plan(lc, w, h, weight);
                (p.ln_cell0, p.ln_cell1, p.ln_tt, p.ln_cyc) = (k.cell[0], k.cell[1], k.tt, k.cyc);
                (p.ln_fill, p.ln_haze, p.ln_blur, p.ln_diag, p.ln_reach) = (k.fill, k.haze, k.blur[0], k.blur[1], k.reach);
                drops = k.runners.clone();
                drops.extend(k.strips(w));
            }
            LensKind::Frost => {
                p.lens_kind = 2;
                (p.th, p.tw, p.aspect) = (1.0 - 0.55 * l.amount.clamp(0.0, 1.0), cycles(0.5, lls), w as f32 / h as f32);
            }
        }
    }
    let pp = &scene.post;
    p.post_on = post_on as u32;
    let flare = super::post::FlareK::of(scene, post_on);
    (p.fl_rays, p.fl_ghosts, p.fl_amt) = (flare.rays, flare.ghosts as u32, flare.amt);
    let bloom = post_on && (pp.bloom > 0.001 || flare.on());
    p.bloom = if post_on && pp.bloom > 0.001 { pp.bloom } else { 0.0 };
    p.exposure = if post_on { pp.exposure.max(0.0) } else { 1.0 };
    p.bl_sky = super::post::sky_threshold(scene);
    p.grain_seed = (ctx.tphase.rem_euclid(1.0) * 4096.0).round() as u32 % 4096;
    (p.tint_r, p.tint_g, p.tint_b) = (pp.tint[0] as f32 / 255.0, pp.tint[1] as f32 / 255.0, pp.tint[2] as f32 / 255.0);
    (p.saturation, p.contrast, p.vignette, p.grain) = (pp.saturation, pp.contrast, pp.vignette, pp.grain);
    let st = &scene.style;
    let kuwahara = post_on && st.paint >= 0.5;
    if kuwahara { p.kuw_r = st.paint.round().clamp(1.0, 16.0) as i32; }
    let mut cube = None;
    if let Some(g) = grade {
        p.grade_s = st.grade_strength;
        match g {
            super::looks::Grade::Cube { size, data } => { (p.grade, p.grade_n) = (8, *size as u32); cube = Some(&data[..]); }
            super::looks::Grade::Builtin(_) => {
                let name = st.grade.trim().to_lowercase().replace(['-', '_'], " ");
                p.grade = ["warm", "cool", "teal orange", "faded", "night", "sepia", "vivid"].iter().position(|n| *n == name).map_or(0, |k| k as u32 + 1);
            }
        }
    }
    if st.outline.enabled {
        (p.ol_on, p.ol_objects_only) = (1, st.outline.objects_only as u32);
        (p.ol_r, p.ol_g, p.ol_b) = (st.outline.color[0] as f32, st.outline.color[1] as f32, st.outline.color[2] as f32);
    }
    let warp = fv.lens_curve.abs() > 0.001;
    if warp {
        p.lw_on = 1;
        p.lw_amp = fv.lens_curve.clamp(-1.0, 1.0) * h as f32 * 0.22;
        p.lw_den = (w.max(2) - 1) as f32;
        p.lw_blend_top = (fv.horizon_px - 40.0 * h as f32 / 854.0).max(0.0);
        p.lw_span = 60.0 * h as f32 / 854.0;
        p.lw_nearest = (crisp || st.palette != Palette::Full) as u32;
    }
    let mut lut_in = None;
    if let Some(lut) = lut {
        if let (Some(_), Some((lo, hi))) = (lut.ramp, levels) { (p.ramp_on, p.ramp_lo, p.ramp_k) = (1, lo, 255.0 / (hi - lo).max(8.0)); }
        let (bits, table) = lut.table();
        (p.q_on, p.q_bits, p.q_amp) = (1, bits, lut.spread * st.dither_strength.clamp(0.0, 1.0) * 1.6);
        p.dither = match st.dither { Dither::None => 0, Dither::Bayer2 => 1, Dither::Bayer4 => 2, Dither::Bayer8 => 3 };
        lut_in = Some((bits, table, lut.colors.iter().map(|c| (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32).collect()));
    }
    if post_on { (p.paper, p.scan) = (st.paper, st.scanlines); }
    p.stats_on = opts.stats as u32;
    (p.sky_enabled, p.verge_enabled) = (scene.sky.enabled as u32, scene.verge.enabled as u32);
    let key = 1 + lut.map_or(0, |l| l as *const PaletteLut as usize) ^ grade.map_or(0, |g| (g as *const super::looks::Grade as usize) << 1);
    // Depth of field, over the frame after the haze.
    let focus = super::render::weather::FocusK::of(scene, h, post_on);
    if let Some(k) = focus { (p.fo_on, p.fo_tilt, p.fo_dist, p.fo_max, p.fo_band, p.fo_half) = (1, k.tilt as u32, k.dist, k.max_r, k.band, k.half); }
    super::gpu::PostIn { params: p, drops, key, lut: lut_in, cube, kuwahara, shimmer, focus: focus.is_some(), warp, bloom, want_depth: opts.depth }
}

/// `frame_stats` from the GPU's per-row sums (11 counts, 11 luma sums, the total, a row each).
#[cfg(feature = "gpu")]
fn stats_from_rows(ctx: &Ctx, rows: &[f32], px: usize, billboards: usize) -> FrameStats {
    const CLASSES: [&str; 11] = ["sky", "void", "chasm", "bridge", "path", "verge", "walls", "ceiling", "props", "fixtures", "grass"];
    let (mut n, mut l, mut total) = ([0.0f64; 11], [0.0f64; 11], 0.0f64);
    for r in rows.chunks_exact(23) {
        for k in 0..11 { n[k] += r[k] as f64; l[k] += r[11 + k] as f64; }
        total += r[22] as f64;
    }
    let px = px.max(1) as f64;
    let seen = || (0..11).filter(|&k| n[k] > 0.0);
    FrameStats {
        coverage: seen().map(|k| (CLASSES[k].to_string(), (n[k] / px) as f32)).collect(),
        luma: seen().map(|k| (CLASSES[k].to_string(), (l[k] / n[k]) as f32)).collect(),
        mean_luma: (total / px) as f32,
        lights: ctx.lights.len(),
        billboards,
    }
}

/// The sky pass's input for one world: the same constants `draw_sky_bodies`, `draw_lightning`,
/// `veil_sky` and the fog bank compute, with the stars, cloud blobs and bolt segments as lists.
/// `draw` is false when only the bank applies; `bank` is its (transmittance, colour) when < 1.
#[cfg(feature = "gpu")]
fn gpu_sky(ctx: &Ctx, draw: bool, strike: Option<(&Strike, [f32; 3])>, bank: Option<(f32, [f32; 3])>) -> super::gpu::SkyInput {
    use super::gpu::{bin_tiles, SkyParams};
    let v = &ctx.view;
    let (w, h) = (v.width, v.height);
    let sky = &ctx.scene.sky;
    let hy = v.horizon_px.max(2.0);
    let (ox, oy) = (v.left as f32, v.top as f32);
    let (fw, fhy) = (w as f32 - 2.0 * ox, (hy - oy).max(2.0));
    let mut p: SkyParams = bytemuck::Zeroable::zeroed();
    // (`ox` is only where the cloud layers hang from, so it slides with the sky.)
    let sdx = v.sky_dx();
    (p.hy, p.ox, p.oy, p.fw, p.fhy, p.tile_cols) = (hy, ox + sdx, oy, fw, fhy, w.div_ceil(16) as u32);
    p.craters = moon_craters(sky.moon.seed);
    let mut prims: Vec<f32> = Vec::new();
    let mut bins: Vec<u32> = Vec::new();
    if draw {
        if sky.moon.body.enabled {
            p.md_on = 1;
            p.md = [ox + sdx + sky.moon.body.pos[0] * fw, oy + sky.moon.body.pos[1].clamp(0.0, 1.0) * fhy, sky.moon.body.radius * fhy];
        }
        if let Some(gl) = clouds::glow(ctx) {
            (p.ss_on, p.ss_prims) = (1, prims.len() as u32);
            prims.extend(gl.rec());
        }
        let stars = star_list(ctx);
        if !stars.is_empty() {
            (p.stars_on, p.star_prims, p.star_bins) = (1, prims.len() as u32, bins.len() as u32);
            let boxes: Vec<_> = stars.iter().map(|s| { let (x, y) = (s.x.floor() as i64, s.y.floor() as i64); (x - s.ri, y - s.ri, x + s.ri, y + s.ri) }).collect();
            for s in &stars { prims.extend([s.x, s.y, s.r, s.ri as f32, s.b, s.t]); }
            bins.extend(bin_tiles(w, h, &boxes));
        }
        let meteors = meteor_list(ctx);
        if !meteors.is_empty() {
            (p.mt_n, p.mt_prims) = (meteors.len() as u32, prims.len() as u32);
            for m in &meteors { prims.extend(m); }
        }
        if let Some(k) = weather::aurora_k(ctx) {
            p.au_on = 1;
            (p.au_a, p.au_m, p.au_cx, p.au_unit) = (k.a, k.m, k.cx, k.unit);
            (p.au_lo, p.au_hi, p.au_acc) = (k.lo, k.hi, k.acc);
            (p.au_foot, p.au_arc, p.au_tall, p.au_rays, p.au_waves) = (k.foot, k.arc, k.tall, k.rays, k.waves);
            (p.au_pres_lo, p.au_pres_hi, p.au_edge, p.au_k, p.au_so) = (k.pres_lo, k.pres_hi, k.edge, k.k, k.so);
        }
        if let Some(k) = space::space_k(ctx) {
            (p.sp_on, p.sp_below, p.sp_scale, p.sp_stars, p.sp_sb) = (1, k.below as u32, k.scale, k.stars, k.sb);
            (p.sp_cx, p.sp_unit, p.sp_top, p.sp_st, p.sp_sh) = (k.cx, k.unit, k.top, k.sky_top, k.sky_hor);
            (p.sp_neb, p.sp_n1, p.sp_n2, p.sp_neb_f) = (k.neb, k.neb1, k.neb2, k.neb_f);
            (p.sp_gal, p.sp_gc, p.sp_gal_c, p.sp_gal_s, p.sp_gal_w, p.sp_gal_h, p.sp_gal_core) = (k.gal, k.gal_col, k.gal_c, k.gal_s, k.gal_w, k.gal_h, k.gal_core);
            p.sp_so = k.so;
            p.sp_star_var = k.star_var;
            p.sx_prims = prims.len() as u32;
            prims.extend(k.pack());
            let planets = space::planet_list(ctx);
            (p.pl_n, p.pl_prims) = (planets.len() as u32, prims.len() as u32);
            for pk in &planets { prims.extend(pk.pack()); }
            if let Some(bk) = blackhole::bh_k(ctx) {
                (p.bh_on, p.bh_prims) = (1, prims.len() as u32);
                prims.extend(bk.pack());
            }
        }
        p.s_air = if sky.space.enabled { 0.0 } else { 1.0 };
        if let Some(k) = space::tunnel_k(ctx) {
            (p.tn_on, p.tn_kind, p.tn_vx, p.tn_vy, p.tn_focal, p.tn_radius, p.tn_travel) = (1, k.kind, k.vx, k.vy, k.focal, k.radius, k.travel);
            (p.tn_spin, p.tn_twist, p.tn_loop, p.tn_lanes, p.tn_period, p.tn_nx, p.tn_ny) = (k.spin, k.twist, k.loop_len, k.lanes, k.period, k.nx, k.ny);
            (p.tn_c0, p.tn_c1, p.tn_c2, p.tn_k, p.tn_core, p.tn_scale, p.tn_fade) = (k.c0, k.c1, k.c2, k.k, k.core, k.scale, k.fade);
        }
        if sky.sun.enabled {
            let su = &sky.sun;
            p.sun_on = 1;
            (p.s_x, p.s_y, p.s_r) = (ox + sdx + su.pos[0] * fw, oy + su.pos[1].clamp(0.0, 1.0) * fhy, (su.radius * fhy).max(0.75));
            p.s_c = scale3(rgb_lin(su.color), 0.4 + 0.6 * su.intensity.clamp(0.0, 3.0));
            let fog = ctx.fog.map_or(0.0, |(_, d)| (40.0 / d).min(1.5));
            p.s_spread = 1.0 + 2.5 * ctx.wx.veil + fog;
            p.s_focal = v.focal_px.max(1.0);
        }
        if sky.moon.body.enabled {
            let m = &sky.moon;
            let r = m.body.radius * fhy;
            p.moon_on = 1;
            p.m_c = rgb_lin(m.body.color);
            p.m_gr = r * (1.0 + MOON_HALO);
            let ph = m.phase.clamp(-1.0, 1.0);
            let angle = ph.abs() * std::f32::consts::PI;
            (p.m_sx, p.m_sy) = (if ph < 0.0 { -angle.sin() } else { angle.sin() }, angle.cos());
            p.m_litf = 0.5 * (1.0 + angle.cos());
            p.m_op = m.opacity;
            p.m_craters = m.craters as u32;
            (p.m_seed, p.m_relief, p.m_gain) = (m.seed, m.relief.clamp(0.0, 3.0), moon_gain(sky));
        }
        if let Some(cs) = cloud_list(ctx) {
            if !cs.blobs.is_empty() {
                (p.cl_on, p.cl_prims, p.cl_bins) = (1, prims.len() as u32, bins.len() as u32);
                p.cl_col = cs.col;
                if let Some((sx, sy, sc)) = cs.sun_at { (p.cl_sun, p.cl_sx, p.cl_sy, p.cl_sc) = (1, sx, sy, sc); }
                p.cl_op = cs.opacity;
                let boxes: Vec<_> = cs.blobs.iter().map(|b| ((b.x - b.rx) as i64, (b.y - b.ry) as i64, (b.x + b.rx) as i64, (b.y + b.ry) as i64)).collect();
                for b in &cs.blobs { prims.extend([b.x, b.y, b.rx, b.ry, b.life, 0.0]); }
                bins.extend(bin_tiles(w, h, &boxes));
            }
        }
        let layers = clouds::layers(ctx);
        if !layers.is_empty() {
            (p.cl_n, p.cl_recs) = (layers.len() as u32, prims.len() as u32);
            for k in &layers { prims.extend(k.rec()); }
        }
        let r = &sky.rainbow;
        if r.enabled && r.intensity > 0.0 {
            let rad = 0.6 * fw * r.size.max(0.05);
            let band = 0.06 * rad;
            p.rb_on = 1;
            (p.rb_cx, p.rb_cy, p.rb_rad, p.rb_band, p.rb_rad2, p.rb_band2) = (ox + sdx + r.x * fw, hy + 0.12 * rad, rad, band, 1.21 * rad, 1.7 * band);
            let sun = ctx.sky_lights.first().map_or(0.6, |s| weather::lum(s.color).clamp(0.2, 2.0));
            p.rb_k = r.intensity.max(0.0) * 0.32 * sun;
            p.rb_double = r.double as u32;
            p.rb_sat = r.saturation.clamp(0.0, 1.0);
        }
        let halos = weather::halo_list(ctx);
        if !halos.is_empty() {
            (p.hl_n, p.hl_prims) = (halos.len() as u32, prims.len() as u32);
            for h in &halos { prims.extend(h); }
        }
        if let Some((s, flash)) = strike {
            (p.fl_on, p.fl) = (1, flash);
            let l = &ctx.scene.weather.lightning;
            if l.bolts && s.bolt >= 0.05 {
                let scale = (h as f32 - oy) / 854.0;
                let segs = bolt_segments(ctx, s);
                let glow_r = 7.0 * scale;
                let (mut bx0, mut bx1, mut by1) = (f32::MAX, f32::MIN, 0.0f32);
                for (a, b, _) in &segs { bx0 = bx0.min(a[0].min(b[0])); bx1 = bx1.max(a[0].max(b[0])); by1 = by1.max(a[1].max(b[1])); }
                p.bolt_on = 1;
                p.b_xa = (bx0 - glow_r * 3.0).max(0.0) as usize as u32;
                p.b_xb = ((bx1 + glow_r * 3.0) as usize).min(w - 1) as u32;
                p.b_yb = ((by1 + glow_r) as usize).min(h - 1) as u32;
                p.b_gain = s.bolt * l.intensity.max(0.0);
                (p.b_core, p.b_tint, p.b_glow, p.b_scale) = (rgb_lin([235, 240, 255]), rgb_lin(l.color), glow_r, scale);
                (p.b_nsegs, p.b_prims) = (segs.len() as u32, prims.len() as u32);
                for (a, b, wgt) in &segs { prims.extend([a[0], a[1], b[0], b[1], *wgt, 0.0]); }
            }
        }
        p.veil = ctx.wx.veil.max(0.0);
    }
    if let Some((t, c)) = bank { (p.bank_on, p.bank_t, p.bank) = (1, t, c); }
    super::gpu::SkyInput { params: p, prims, bins }
}

/// Which point lights can reach each 16 x 16 tile of the screen: those whose sphere touches the
/// box around everything the tile shows. A pixel then looks at a handful of lamps, not all of them.
pub(crate) struct TileLights { cols: usize, lists: Vec<Vec<u16>> }
const LIGHT_TILE: usize = 16;

impl TileLights {
    #[inline]
    pub(crate) fn at(&self, x: usize, y: usize) -> &[u16] { &self.lists[(y / LIGHT_TILE) * self.cols + x / LIGHT_TILE] }
}

/// None when there is nothing worth culling (few lights) or several worlds share the frame.
fn tile_lights(ctxs: &[Ctx], gbuf: &[GPixel]) -> Option<TileLights> {
    tile_lights_at(ctxs, gbuf, |ctx, g| (g.id != id::NONE).then(|| ctx.view.to_cam(g.x, g.y, g.d)))
}

/// `tile_lights` for whatever point each pixel lights (camera space), or None for pixels that light none.
fn tile_lights_at(ctxs: &[Ctx], gbuf: &[GPixel], point: impl Fn(&Ctx, &GPixel) -> Option<[f32; 3]> + Sync) -> Option<TileLights> {
    if ctxs.len() != 1 || ctxs[0].lights.len() < 4 { return None; }
    let ctx = &ctxs[0];
    let (w, h) = (ctx.view.width, ctx.view.height);
    let (cols, rows) = (w.div_ceil(LIGHT_TILE), h.div_ceil(LIGHT_TILE));
    let lists = (0..cols * rows).into_par_iter().map(|t| {
        let (tx, ty) = (t % cols, t / cols);
        let (mut lo, mut hi) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
        for y in ty * LIGHT_TILE..((ty + 1) * LIGHT_TILE).min(h) {
            for x in tx * LIGHT_TILE..((tx + 1) * LIGHT_TILE).min(w) {
                // The same point the light loop measures from.
                let Some(p) = point(ctx, &gbuf[y * w + x]) else { continue };
                for k in 0..3 { lo[k] = lo[k].min(p[k]); hi[k] = hi[k].max(p[k]); }
            }
        }
        if lo[0] > hi[0] { return Vec::new(); }
        ctx.lights.iter().enumerate().filter(|(_, pl)| {
            let d2: f32 = (0..3).map(|k| { let e = (lo[k] - pl.pos[k]).max(pl.pos[k] - hi[k]).max(0.0); e * e }).sum();
            // A margin keeps the test on the safe side of rounding.
            d2 < pl.radius * pl.radius * 1.0001 + 1e-4
        }).map(|(i, _)| i as u16).collect()
    }).collect();
    Some(TileLights { cols, lists })
}

fn shade(ctxs: &[Ctx], sky_w: &SkyW, soft: &[(u8, f32)], gbuf: &[GPixel], sun_mask: &[f32], ao_mask: &[f32], layers: &Layers, hdr: &mut [[f32; 3]], refl: &mut [Refl]) {
    let w = ctxs[0].view.width;
    // What a pixel with nothing on it shows: each world's sky, weighted by how much it counts.
    let empty = |x: usize, y: usize| -> [f32; 3] {
        if ctxs.len() == 1 { return sky_base(&ctxs[0], y, layers); }
        let mut c = [0.0f32; 3];
        for (r, ctx) in ctxs.iter().enumerate() {
            let k = sky_w.at(r, x);
            if k <= 0.0 { continue; }
            c = add3(c, scale3(sky_seen(ctx, sky_base(ctx, y, layers)), k));
        }
        c
    };
    let tiles = tile_lights(ctxs, gbuf);
    // Each pixel's texture coordinates once, for itself and for its neighbours' footprints.
    let uvs: Vec<Option<(f32, f32, u8)>> = gbuf.par_iter().map(|g| surface_uv(&ctxs[(g.realm as usize).min(ctxs.len() - 1)], g)).collect();
    hdr.par_chunks_mut(w).zip(refl.par_chunks_mut(w)).enumerate().for_each(|(y, (row, rrow))| {
        for x in 0..w {
            let i = y * w + x;
            let g = &gbuf[i];
            let ctx = &ctxs[(g.realm as usize).min(ctxs.len() - 1)];
            let near = tiles.as_ref().map(|t| t.at(x, y));
            let Some((c, rf)) = shade_px(ctx, g, gbuf, i, x, y, sun_mask, ao_mask, layers, near, Some(&uvs)) else {
                row[x] = empty(x, y);
                continue;
            };
            let (c, rf) = match soft.get(i).filter(|s| s.1 > 0.0) {
                // At the edge of a patch where two worlds mix, a little of each.
                Some(&(o, k)) => {
                    let other = &ctxs[o as usize];
                    let from = &ctxs[g.realm as usize];
                    let cam = from.view.to_cam(g.x, g.y, g.d);
                    let go = GPixel { x: cam[0] - other.view.bend_x(g.d) - other.view.shear_x(g.d), realm: o, ..*g };
                    match shade_px(other, &go, gbuf, i, x, y, sun_mask, ao_mask, layers, None, None) {
                        Some((c2, r2)) => (mix3(c, c2, k), if r2.r > rf.r { r2 } else { rf }),
                        None => (c, rf),
                    }
                }
                None => (c, rf),
            };
            row[x] = c;
            rrow[x] = rf;
        }
    });
}

/// Each column's running sums of running sums (f64, so wide blurs keep their precision), for a
/// triangle-weighted average over any span of rows in constant time. Rows past either end repeat
/// the end row, as the sums are extended exactly that way.
struct ColumnBlur { h: usize, q2: Vec<[f64; 3]>, ends: Vec<([f64; 3], [f64; 3], [f64; 3])> }

impl ColumnBlur {
    fn new(src: &[[f32; 3]], w: usize, h: usize) -> ColumnBlur {
        // Column by column, eight at a time so each row's read is one stretch of memory.
        const BLOCK: usize = 8;
        let mut q2 = vec![[0.0f64; 3]; w * (h + 1)];
        let mut ends = vec![([0.0f64; 3], [0.0f64; 3], [0.0f64; 3]); w];
        q2.par_chunks_mut(BLOCK * (h + 1)).zip(ends.par_chunks_mut(BLOCK)).enumerate().for_each(|(b, (q, e))| {
            let cols = e.len();
            let (mut q1, mut acc) = (vec![[0.0f64; 3]; cols], vec![[0.0f64; 3]; cols]);
            for r in 0..h {
                for c in 0..cols {
                    let v = src[r * w + b * BLOCK + c];
                    for k in 0..3 {
                        // q2[r] = sum of q1[j] for j < r; q1[r] = sum of rows j < r.
                        acc[c][k] += q1[c][k];
                        q1[c][k] += v[k] as f64;
                    }
                    q[c * (h + 1) + r + 1] = acc[c];
                }
            }
            for c in 0..cols {
                let (first, last) = (src[b * BLOCK + c], src[(h - 1) * w + b * BLOCK + c]);
                e[c] = (q1[c], first.map(|v| v as f64), last.map(|v| v as f64));
            }
        });
        ColumnBlur { h, q2, ends }
    }

    /// Sum over rows j < i of (the sum over rows below j), for any i: inside the column from the
    /// table, beyond its ends as if the end rows repeated for ever.
    #[inline]
    fn q2(&self, x: usize, i: i64, k: usize) -> f64 {
        let h = self.h as i64;
        let (q1h, first, last) = &self.ends[x];
        if i <= 0 {
            // q1(j) = j * first for j < 0, so the sum over i <= j < 0 is first * -(i)(1 - i)/2 negated.
            let m = (-i) as f64;
            first[k] * m * (m + 1.0) * 0.5
        } else if i <= h {
            self.q2[x * (self.h + 1) + i as usize][k]
        } else {
            let m = (i - h) as f64;
            self.q2[x * (self.h + 1) + self.h][k] + m * q1h[k] + last[k] * m * (m - 1.0) * 0.5
        }
    }

    /// Triangle-weighted average of column `x` round row `r0`: weight (half + 1 - |offset|).
    fn triangle(&self, x: usize, r0: i64, half: i64) -> [f32; 3] {
        let half = half.max(0);
        let norm = 1.0 / ((half + 1) * (half + 1)) as f64;
        [0, 1, 2].map(|k| {
            let t = self.q2(x, r0 + half + 2, k) - 2.0 * self.q2(x, r0 + 1, k) + self.q2(x, r0 - half, k);
            (t * norm) as f32
        })
    }
}

/// How much nearer than the ray (a row below) something may be and still be what it meets: a
/// fraction of the ray's depth, and metres (`reflect.wgsl` uses the same).
const RAY_NEAR: f32 = 0.85;
const RAY_SLACK: f32 = 0.3;

/// Whether a pixel is drawn after the sky (props, lights, grass), so the sky is under it.
fn drawn_over_sky(gid: u8) -> bool { gid == id::PROP || gid == id::FIXTURE || gid == id::TUFT }

/// The sky under the props at row `r0` of column `x`, smeared along the column over `spread`
/// rows by five taps weighted 1, 2, 3, 2, 1.
fn sky_under(under: &[[f32; 3]], w: usize, h: usize, x: usize, r0: f32, spread: f32) -> [f32; 3] {
    let mut c = [0.0f32; 3];
    for (k, wt) in [(-1.0f32, 1.0f32), (-0.5, 2.0), (0.0, 3.0), (0.5, 2.0), (1.0, 1.0)] {
        let r = (r0 + k * spread).round().clamp(0.0, h as f32 - 1.0) as usize;
        c = add3(c, scale3(under[r * w + x], wt / 9.0));
    }
    c
}

/// Mirror reflections on glossy floors, traced against the finished frame. For a level camera the
/// ray reflected off a flat floor stays in its pixel's column, and its depth at each row has a
/// closed form: from a floor point at depth d_w seen by an eye h above it, the reflected ray reaches
/// row r at depth 2 d_w / (1 + k), with k = (r - horizon) d_w / (f h). The first row where the frame
/// shows something nearer than the ray is what the floor reflects. A camera-facing card crossed this
/// way is hit exactly where the ray meets it.
///
/// Something much nearer than the ray (by more than `RAY_NEAR` and `RAY_SLACK`) is not a hit: the
/// ray passes behind it. Where the mirrored sky is hidden behind a prop, `under` (the frame as it
/// was before props were drawn) gives the sky there.
fn reflect_pass(ctx: &Ctx, gbuf: &[GPixel], refl: &[Refl], under: Option<&[[f32; 3]]>, hdr: &mut [[f32; 3]]) {
    let v = &ctx.view;
    let (w, h) = (v.width, v.height);
    let (hz, f) = (v.horizon_px, v.focal_px);
    // Rows above the first glossy pixel are only read, so only the rows below are copied (the
    // reflection is added to them while other rows read them).
    let split = refl.chunks(w).position(|r| r.iter().any(|r| r.r >= 0.002)).unwrap_or(h);
    // What the floor reflects is blurred along the column by a triangle of any width; each column's
    // running sums of running sums give that blur in a few lookups, read from the frame as it was
    // before any reflection was added.
    let sums = ColumnBlur::new(hdr, w, h);
    let (_, below) = hdr.split_at_mut(split * w);
    // Per column, the nearest depth in each block of 8 and of 64 rows (stored block by block). A
    // block can only hold the hit if something in it is nearer than the ray at the block's top (the
    // ray only gets further as it climbs), so empty stretches are skipped whole while every
    // candidate row is still tested.
    // All three are stored column by column, so a march up a column reads memory in order.
    let (nb8, nb64) = (h.div_ceil(8), h.div_ceil(64));
    let mut depth = vec![f32::INFINITY; w * h];
    let mut min8 = vec![f32::INFINITY; w * nb8];
    let mut min64 = vec![f32::INFINITY; w * nb64];
    depth.par_chunks_mut(h).zip(min8.par_chunks_mut(nb8)).zip(min64.par_chunks_mut(nb64)).enumerate().for_each(|(x, ((d, m8), m64))| {
        for r in 0..h {
            let z = gbuf[r * w + x].depth;
            d[r] = z;
            if z < m8[r / 8] { m8[r / 8] = z; }
            if z < m64[r / 64] { m64[r / 64] = z; }
        }
    });
    below.par_chunks_mut(w).enumerate().for_each(|(yb, row)| {
        let y = yb + split;
        for x in 0..w {
            let i = y * w + x;
            let rf = refl[i];
            if rf.r < 0.002 { continue; }
            let g = &gbuf[i];
            if g.id != id::GROUND && g.id != id::CHASM { continue; }
            let p = v.to_cam(g.x, g.y, g.d);
            let eye = (-p[1]).max(0.05);
            let dw = g.depth;
            let yc = y as f32 + 0.5;
            // Ripples tilt the surface: the reflection wobbles sideways, more the further it reaches.
            let xs = (x as f32 + rf.sx * (yc - hz) * 0.5).round().clamp(0.0, w as f32 - 1.0) as usize;
            // The ray's depth at row r is 2 dw / (1 + k(r)), k linear in r (infinite once 1 + k
            // falls to 0.001); "z nearer than the ray" is tested as z (1 + k) <= 2 dw, without dividing.
            let kr = dw / (f * eye);
            let near_ray = |z: f32, r: usize| { let k1 = 1.0 + (r as f32 + 0.5 - hz) * kr; k1 <= 0.001 || z * k1 <= 2.0 * dw };
            // The ray only ever gets further from the camera, so it meets nothing nearer than it
            // was a row below. Something well in front of that (a palm's fronds or a lantern
            // hanging over distant water) is only in the way on screen: the ray passes behind it.
            let on_ray = |z: f32, r: usize| (z + RAY_SLACK) * (1.0 + (r as f32 + 1.5 - hz) * kr) >= 2.0 * RAY_NEAR * dw;
            let (col, c8, c64) = (&depth[xs * h..(xs + 1) * h], &min8[xs * nb8..(xs + 1) * nb8], &min64[xs * nb64..(xs + 1) * nb64]);
            let r_inf = 2.0 * hz - yc;
            let r_min = r_inf.max(0.0).ceil() as usize;
            let mut hit = None;
            let mut r = y as isize - 1;
            while r >= r_min as isize {
                let ru = r as usize;
                let top64 = ru / 64 * 64;
                if top64 >= r_min && !near_ray(c64[ru / 64], top64) { r = top64 as isize - 1; continue; }
                let top8 = ru / 8 * 8;
                if top8 >= r_min && !near_ray(c8[ru / 8], top8) { r = top8 as isize - 1; continue; }
                let z = col[ru];
                if z.is_finite() && near_ray(z, ru) && on_ray(z, ru) { hit = Some(ru); break; }
                r -= 1;
            }
            let tap = |r0: f32| -> [f32; 3] {
                // A rough surface smears what it reflects along the column, the more the further
                // the reflected thing is from the surface. Past the top of what was rendered, the
                // top row repeats.
                let spread = rf.rough * (yc - r0).max(0.0) * 0.25;
                sums.triangle(xs, r0.round() as i64, spread.round() as i64)
            };
            let sky_row = (r_inf as usize).min(h - 1) * w + xs;
            let col = match hit {
                Some(rh) => tap(rh as f32),
                None if r_inf >= 0.0 && gbuf[sky_row].id == id::NONE => {
                    // Nothing in the way: the sky, mirrored with its moon, stars and clouds. Fade to
                    // the plain sky colour where the mirror row leaves the top of the frame.
                    let k = smoothstep(0.0, 16.0, r_inf);
                    mix3(rf.env, tap(r_inf), k)
                }
                None if r_inf >= 0.0 && drawn_over_sky(gbuf[sky_row].id) && under.is_some() => {
                    // A prop stands in front of the mirrored sky on screen: the sky as it was
                    // before the props were drawn, so the moon's path on the water runs on unbroken.
                    let k = smoothstep(0.0, 16.0, r_inf);
                    mix3(rf.env, sky_under(under.unwrap(), w, h, xs, r_inf, rf.rough * (yc - r_inf) * 0.25), k)
                }
                None => rf.env,
            };
            row[x] = add3(row[x], scale3(sub3(col, rf.env), rf.r));
        }
    });
}

// ── Shadows ────────────────────────────────────────────────────────────────

/// A billboard's two shadows, worked out once: the contact shadow (a soft ellipse on the ground
/// at its foot, `axis` along its width) and the sun's (the flat picture it is drawn as, at the
/// angle it stands: `c0` its top left corner, `across` and `down` its edges, `n` its normal),
/// each with the pixels it can fall on. A run of fence is one too: a flat picture along its line,
/// its posts, rails and pickets worked out where the sun's ray meets it.
struct Caster<'b> {
    pic: Pic<'b>, shadow: f32,
    foot: [f32; 2], axis: [f32; 2], ra: f32, rb: f32, contact: Option<(usize, usize, usize, usize)>,
    c0: [f32; 3], across: [f32; 3], down: [f32; 3], ia: f32, idn: f32, n: [f32; 3], nl: f32,
    sun: Option<(usize, usize, usize, usize)>,
}

/// What a caster's shadow is the shape of.
enum Pic<'b> { Sprite(&'b Billboard), Fence(FenceCut) }

/// One run of fence as its shadow sees it: from `-pw/2` to `len + pw/2` along its line (u 0..1)
/// and from `ht + 0.06` down to the ground (v 0..1), the parts `fences` builds it from.
#[derive(Clone, Copy)]
struct FenceCut { style: u32, len: f32, ht: f32, posts: u32, n: u32, pw: f32 }

impl FenceCut {
    fn top(&self) -> f32 { self.ht + 0.06 }

    /// Its packed shape for the GPU (f[19..22] and f[37..39] of a caster).
    fn bits(&self) -> u32 { self.style | (self.posts.min(1023) << 2) | (self.n.min(65535) << 12) }

    /// How much of it stands at (u, v), averaged over a box `fu` x `fv` wide.
    fn alpha(&self, u: f32, v: f32, fu: f32, fv: f32) -> f32 {
        let span = self.len + self.pw;
        let (s, y) = (u * span - 0.5 * self.pw, (1.0 - v) * self.top());
        let (hs, hy) = (0.5 * fu * span, 0.5 * fv * self.top());
        let ht = self.ht;
        // Posts at each end and between, as tall as the fence and a little more.
        let post = fence_comb(s, hs, self.len / self.posts.max(1) as f32, self.pw, 0.0, -0.5 * self.pw, self.len + 0.5 * self.pw) * fence_box(0.0, ht + 0.06, y, hy);
        let along = fence_box(0.0, self.len, s, hs);
        let (upright, across) = match self.style {
            // Pickets with a narrower point on top, on two rails.
            0 => {
                let p = self.len / self.n.max(1) as f32;
                let body = fence_comb(s, hs, p, 0.074, 0.5 * p, 0.0, self.len) * fence_box(0.04, ht - 0.05, y, hy);
                let tip = fence_comb(s, hs, p, 0.036, 0.5 * p, 0.0, self.len) * fence_box(ht - 0.05, ht, y, hy);
                (body + tip, along * (fence_box(0.22 * ht, 0.22 * ht + 0.07, y, hy) + fence_box(0.72 * ht, 0.72 * ht + 0.07, y, hy)))
            }
            // Three rails.
            1 => (0.0, along * (fence_box(0.28 * ht - 0.09, 0.28 * ht, y, hy) + fence_box(0.6 * ht - 0.09, 0.6 * ht, y, hy) + fence_box(0.9 * ht - 0.09, 0.9 * ht, y, hy))),
            // Boards, capped.
            _ => (0.0, along * fence_box(0.02, ht + 0.02, y, hy)),
        };
        let up = 1.0 - (1.0 - post.min(1.0)) * (1.0 - upright.min(1.0));
        1.0 - (1.0 - up) * (1.0 - across.min(1.0))
    }
}

/// How much of [x - h, x + h] lies in [lo, hi] (whether x does, for h near 0).
fn fence_box(lo: f32, hi: f32, x: f32, h: f32) -> f32 {
    if h < 1e-5 { return (x >= lo && x < hi) as u32 as f32; }
    ((x + h).min(hi) - (x - h).max(lo)).max(0.0) / (2.0 * h)
}

/// How much of [x - h, x + h] (within [lo, hi]) is covered by bars `w` wide every `p`, one
/// centred on `c`.
fn fence_comb(x: f32, h: f32, p: f32, w: f32, c: f32, lo: f32, hi: f32) -> f32 {
    // Covered length from `c - p/2` to x.
    let cum = |x: f32| {
        let r = x - c + 0.5 * p;
        let k = (r / p).floor();
        k * w + (r - k * p - 0.5 * (p - w)).clamp(0.0, w)
    };
    if h < 1e-5 {
        if x < lo || x >= hi { return 0.0; }
        let r = (x - c + 0.5 * p).rem_euclid(p);
        return (r >= 0.5 * (p - w) && r < 0.5 * (p + w)) as u32 as f32;
    }
    let (a, b) = ((x - h).max(lo), (x + h).min(hi));
    if b <= a { return 0.0; }
    (cum(b) - cum(a)) / (2.0 * h)
}

/// The sun's direction when it casts shadows here (a low sun's shadows held to six heights
/// long), and every caster of world `ctx`.
fn shadow_casters<'b>(ctx: &Ctx, bills: &'b [Billboard]) -> (Option<[f32; 3]>, Vec<Caster<'b>>) {
    let v = &ctx.view;
    let (w, h) = (v.width, v.height);
    let sun = ctx.sky_lights.first().map(|s| s.dir).filter(|d| d[1] > 0.05 && !ctx.scene.ceiling.enabled).map(|l| {
        let lh = (l[0] * l[0] + l[2] * l[2]).sqrt();
        let k = if lh > 6.0 * l[1] { 6.0 * l[1] / lh } else { 1.0 };
        [l[0] * k, l[1], l[2] * k]
    });
    let cross = |a: [f32; 3], b: [f32; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    // The pixels a caster's shadows can fall on: around its foot, and its picture projected along
    // the sun onto the ground.
    let reach = |c0: [f32; 3], across: [f32; 3], down: [f32; 3], n: [f32; 3], (foot, axis, ra, rb): ([f32; 2], [f32; 2], f32, f32), grounded: bool| {
        let (aa, dd) = (dot3(across, across), dot3(down, down));
        let bottom = [c0[0] + down[0], c0[1] + down[1], c0[2] + down[2]];
        let mut bbox = BBox::new();
        let mut whole = false;
        if grounded {
            for (sa, sb) in [(-1.0f32, -1.0f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                let (x, d) = (foot[0] + axis[0] * ra * sa - axis[1] * rb * sb, foot[1] + axis[1] * ra * sa + axis[0] * rb * sb);
                match v.world_to_px(x, 0.0, d) { Some(p) => bbox.add(Some(p)), None => whole = true }
            }
        }
        let contact = if !grounded { None } else if whole { Some((0, 0, w - 1, h - 1)) } else { bbox.clip(w, h) };
        let sun_box = sun.and_then(|l| {
            let nl = dot3(n, l);
            if nl.abs() < 1e-6 * (aa * dd).sqrt() { return None; }
            let mut bbox = BBox::new();
            let mut whole = false;
            for c in [c0, [c0[0] + across[0], c0[1] + across[1], c0[2] + across[2]], bottom, [bottom[0] + across[0], bottom[1] + across[1], bottom[2] + across[2]]] {
                let t = c[1].max(0.0) / l[1];
                for (x, d) in [(c[0], c[2]), (c[0] - l[0] * t, c[2] - l[2] * t)] {
                    match v.world_to_px(x, 0.0, d) { Some(p) => bbox.add(Some(p)), None => whole = true }
                }
            }
            if whole { Some((0, (v.horizon_px.max(0.0) as usize).min(h - 1), w - 1, h - 1)) } else { bbox.clip(w, h) }
        });
        (contact, sun_box)
    };
    let mut casters: Vec<Caster> = bills.iter().filter(|b| b.shadow > 0.0 && b.realm == ctx.realm && b.tube.is_none()).filter_map(|b| {
        let [bx, _by, bd] = b.world;
        // The picture as it stands: a panel where it is; a card turned to the camera is a thing
        // alike from every side (a tree, a rock), so its shadow is cast as if turned to the sun,
        // as a round thing's is, wherever the camera stands.
        let (c0, across, down) = match b.panel {
            Some(pn) => (pn.tl, pn.across, pn.down),
            None => {
                let half_w = b.height * b.sprite.aspect * 0.5;
                let r = match sun {
                    Some(l) if l[0] * l[0] + l[2] * l[2] > 1e-6 => { let k = (l[0] * l[0] + l[2] * l[2]).sqrt(); [l[2] / k, 0.0, -l[0] / k] }
                    _ => { let r = v.unrot_dir([1.0, 0.0, 0.0]); [r[0], 0.0, r[2]] }
                };
                ([bx - r[0] * half_w, b.height, bd - r[2] * half_w], [2.0 * half_w * r[0], 0.0, 2.0 * half_w * r[2]], [0.0, -b.height, 0.0])
            }
        };
        let (aa, dd) = (dot3(across, across), dot3(down, down));
        if aa < 1e-8 || dd < 1e-8 { return None; }
        let n = cross(across, down);
        let width = aa.sqrt();
        // Contact darkening: under a card, or along the foot of a panel that stands on the ground.
        let bottom = [c0[0] + down[0], c0[1] + down[1], c0[2] + down[2]];
        let grounded = bottom[1].min(c0[1] + down[1] + across[1]) < 0.3;
        let (foot, axis, ra, rb) = match b.panel {
            None => {
                let r = v.unrot_dir([1.0, 0.0, 0.0]);
                let k = (r[0] * r[0] + r[2] * r[2]).sqrt().max(1e-6);
                let half_w = 0.5 * width;
                ([bx, bd], [r[0] / k, r[2] / k], half_w * 0.85, half_w * 0.55)
            }
            Some(_) => {
                let hk = (across[0] * across[0] + across[2] * across[2]).sqrt().max(1e-6);
                ([bottom[0] + 0.5 * across[0], bottom[2] + 0.5 * across[2]], [across[0] / hk, across[2] / hk], 0.5 * hk + 0.2, (0.275 * hk).min(0.6))
            }
        };
        let (contact, sun_box) = reach(c0, across, down, n, (foot, axis, ra, rb), grounded);
        Some(Caster { pic: Pic::Sprite(b), shadow: b.shadow, foot, axis, ra, rb, contact, c0, across, down, ia: 1.0 / aa, idn: 1.0 / dd, n, nl: sun.map_or(0.0, |l| dot3(n, l)), sun: sun_box })
    }).collect();
    // Fences: each run on each side, as `fences` builds them. Runs just out of sight still cast
    // onto the ground in sight.
    {
        let margin = 8.0;
        let far = (if v.turned { ctx.d_hi } else { ctx.far.min(120.0) }) + margin;
        let near = (if v.turned { ctx.d_lo } else { NEAR }) - margin;
        for f in ctx.scene.fences.iter().filter(|f| f.enabled && f.height > 0.0 && f.shadow && f.shadow_opacity > 0.0) {
            let sp = snap_to_loop(f.spacing.max(1.0), ctx.loop_len);
            let len = f.length.clamp(0.5, sp);
            let posts = ((len / f.post_spacing.max(0.5)).round() as u32).max(1);
            let style = match f.style { FenceStyle::Picket => 0, FenceStyle::Rails => 1, FenceStyle::Boards => 2 };
            let pw = if style == 1 { 0.12 } else { 0.09 };
            let cut = FenceCut { style, len, ht: f.height, posts, n: (len / 0.14).round().max(1.0) as u32, pw };
            let top = cut.top();
            let k_lo = ((ctx.scroll + near - len - f.offset) / sp).floor() as i64;
            let k_hi = ((ctx.scroll + far - f.offset) / sp).ceil() as i64;
            for k in k_lo..=k_hi {
                let a = k as f32 * sp + f.offset - ctx.scroll;
                let b = a + len;
                if b < near || a > far || ctx.on_bridge(0.5 * (a + b)) { continue; }
                for sign in f.side.signs().iter().copied() {
                    if ctx.cut_away(sign * (v.half_width + f.lateral)) { continue; }
                    let (a0, b0) = (a - 0.5 * pw, b + 0.5 * pw);
                    let (xa, xb) = (sign * (v.path_half_width(a0) + f.lateral), sign * (v.path_half_width(b0) + f.lateral));
                    let (c0, across, down) = ([xa, top, a0], [xb - xa, 0.0, b0 - a0], [0.0, -top, 0.0]);
                    let (aa, dd) = (dot3(across, across), dot3(down, down));
                    let n = cross(across, down);
                    let hk = aa.sqrt();
                    let fa = ([0.5 * (xa + xb), 0.5 * (a0 + b0)], [across[0] / hk, across[2] / hk], 0.5 * hk, 0.3);
                    let (contact, sun_box) = reach(c0, across, down, n, fa, true);
                    let (foot, axis, ra, rb) = fa;
                    casters.push(Caster { pic: Pic::Fence(cut), shadow: f.shadow_opacity.min(1.0), foot, axis, ra, rb, contact, c0, across, down, ia: 1.0 / aa, idn: 1.0 / dd, n, nl: sun.map_or(0.0, |l| dot3(n, l)), sun: sun_box });
                }
            }
        }
    }
    (sun, casters)
}

fn prop_shadows(ctx: &Ctx, bills: &[Billboard], gbuf: &[GPixel], sun_mask: &mut [f32], ao_mask: &mut [f32]) {
    let w = ctx.view.width;
    let (sun, casters) = shadow_casters(ctx, bills);
    if casters.is_empty() { return; }
    // Drawn in bands of rows side by side, each caster in the same order as one after another.
    const BAND: usize = 8;
    sun_mask.par_chunks_mut(w * BAND).zip(ao_mask.par_chunks_mut(w * BAND)).enumerate().for_each(|(k, (sm, am))| {
        let (r0, r1) = (k * BAND, k * BAND + sm.len() / w - 1);
        for c in &casters {
            if let Some((x0, y0, x1, y1)) = c.contact.filter(|r| r.3 >= r0 && r.1 <= r1) {
                for y in y0.max(r0)..=y1.min(r1) {
                    for x in x0..=x1 {
                        let i = y * w + x;
                        let g = &gbuf[i];
                        if g.id != id::GROUND { continue; }
                        let (dx, dd) = (g.x - c.foot[0], g.d - c.foot[1]);
                        let (na, nb) = ((dx * c.axis[0] + dd * c.axis[1]) / c.ra, (dd * c.axis[0] - dx * c.axis[1]) / c.rb);
                        // Under a prop a soft oval; along a fence's foot a soft strip.
                        let r2 = match c.pic { Pic::Fence(_) if na.abs() <= 1.0 => nb * nb, Pic::Fence(_) => 1.0, _ => na * na + nb * nb };
                        if r2 < 1.0 { am[i - r0 * w] *= 1.0 - 0.5 * c.shadow * (1.0 - r2); }
                    }
                }
            }
            let (Some(l), Some((x0, y0, x1, y1))) = (sun, c.sun) else { continue };
            if y1 < r0 || y0 > r1 { continue; }
            // Toward the sun from a bit of ground: where the ray meets the picture's plane, as (u, v)
            // on it. (The ground is at height 0 in path space: hills are the camera's.)
            let on_pic = |gx: f32, gd: f32| {
                let rel = [c.c0[0] - gx, c.c0[1], c.c0[2] - gd];
                let t = dot3(c.n, rel) / c.nl;
                let q = [gx + l[0] * t - c.c0[0], l[1] * t - c.c0[1], gd + l[2] * t - c.c0[2]];
                (t, dot3(q, c.across) * c.ia, dot3(q, c.down) * c.idn)
            };
            for y in y0.max(r0)..=y1.min(r1) {
                for x in x0..=x1 {
                    let i = y * w + x;
                    let g = &gbuf[i];
                    if g.id != id::GROUND { continue; }
                    let (t, mut u, vv) = on_pic(g.x, g.d);
                    if t <= 0.0 { continue; }
                    if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&vv) { continue; }
                    let a = match &c.pic {
                        Pic::Sprite(b) => {
                            if let Some(s) = b.panel.and_then(|p| p.strip) { u = s; } else if b.flip { u = 1.0 - u; }
                            let mut a = b.sprite.sample(u, vv, 192.0, false)[3];
                            // A painted mask says which parts are solid: a flame or a pane of glass casts nothing.
                            if b.sprite.casts { if let Some(gl) = &b.sprite.glow { a *= gl.sample(u, vv, 192.0, false)[1]; } }
                            a
                        }
                        Pic::Fence(fc) => {
                            // Averaged over the ground this pixel covers, so pickets far off do not shimmer.
                            let (fu, fv) = ground_step(gbuf, w, x, y).map_or((0.0, 0.0), |((ax, ad), (bx, bd))| {
                                let (_, ua, va) = on_pic(g.x + ax, g.d + ad);
                                let (_, ub, vb) = on_pic(g.x + bx, g.d + bd);
                                ((ua - u).abs() + (ub - u).abs(), (va - vv).abs() + (vb - vv).abs())
                            });
                            fc.alpha(u, vv, fu.min(1.0), fv.min(1.0))
                        }
                    };
                    if a > 0.01 { let m = &mut sm[i - r0 * w]; *m = m.min(1.0 - c.shadow * a.min(1.0)); }
                }
            }
        }
    });
}

/// How far the ground moves (x, d) from pixel (x, y) to the next pixel across and the next down,
/// from neighbours that are ground too (the one before where the one after is not).
fn ground_step(gbuf: &[GPixel], w: usize, x: usize, y: usize) -> Option<((f32, f32), (f32, f32))> {
    let h = gbuf.len() / w;
    let i = y * w + x;
    let g = &gbuf[i];
    let step = |j: Option<usize>, k: Option<usize>| {
        let pick = |j: usize, s: f32| { let o = &gbuf[j]; (o.id == id::GROUND).then(|| (s * (o.x - g.x), s * (o.d - g.d))) };
        j.and_then(|j| pick(j, 1.0)).or_else(|| k.and_then(|k| pick(k, -1.0)))
    };
    let across = step((x + 1 < w).then(|| i + 1), (x > 0).then(|| i - 1))?;
    let down = step((y + 1 < h).then(|| i + w), (y > 0).then(|| i - w))?;
    Some((across, down))
}

struct BBox { x0: f32, y0: f32, x1: f32, y1: f32, any: bool }
impl BBox {
    fn new() -> BBox { BBox { x0: f32::MAX, y0: f32::MAX, x1: f32::MIN, y1: f32::MIN, any: false } }
    fn add(&mut self, p: Option<[f32; 2]>) {
        if let Some([x, y]) = p { self.x0 = self.x0.min(x); self.y0 = self.y0.min(y); self.x1 = self.x1.max(x); self.y1 = self.y1.max(y); self.any = true; }
    }
    fn clip(&self, w: usize, h: usize) -> Option<(usize, usize, usize, usize)> {
        if !self.any { return None; }
        let x0 = self.x0.floor().max(0.0) as i64;
        let y0 = self.y0.floor().max(0.0) as i64;
        let x1 = (self.x1.ceil() as i64).min(w as i64 - 1);
        let y1 = (self.y1.ceil() as i64).min(h as i64 - 1);
        if x0 > x1 || y0 > y1 { None } else { Some((x0 as usize, y0 as usize, x1 as usize, y1 as usize)) }
    }
}

/// Transmittance through fog banks from path position `scroll` to `z` metres further on.
fn bank_transmittance(b: &crate::scene::FogBanks, loop_len: f32, scroll: f32, z: f32) -> f32 {
    if !b.enabled || b.density <= 0.0 { return 1.0; }
    let sp = snap_to_loop(b.spacing.max(1.0), loop_len);
    let len = b.length.clamp(0.0, sp);
    // Metres of bank before path position x (banks start at offset + k*spacing).
    let cum = |x: f32| { let y = x - b.offset; (y / sp).floor() * len + y.rem_euclid(sp).min(len) };
    (-b.density * (cum(scroll + z) - cum(scroll))).exp()
}

// ── Lightning ──────────────────────────────────────────────────────────────

/// A strike in progress: which one, how bright the scene flash is, and how bright the bolt is.
#[derive(Clone, Copy)]
struct Strike { index: i64, flash: f32, bolt: f32, x: f32 }

/// When each lightning strike begins, as fractions of the loop (sorted). The same moments the renderer
/// flashes at; games use them to play thunder.
pub fn lightning_times(scene: &Scene) -> Vec<f32> {
    let l = &scene.weather.lightning;
    if !l.enabled || l.strikes == 0 || l.intensity <= 0.0 { return Vec::new(); }
    let frames = scene.motion.frames();
    let mut v: Vec<f32> = (0..l.strikes.min(16) as i64).map(|i| strike_phase(l.seed, i, l.strikes.min(16) as i64, frames)).collect();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v
}

fn strike_phase(seed: u32, i: i64, n: i64, frames: u32) -> f32 {
    let p = (i as f32 + 0.15 + 0.7 * hf(seed ^ 0x5A, i)) / n as f32;
    ((p * frames as f32).round() / frames as f32).rem_euclid(1.0)
}

/// Lightning at loop time `tphase`. A strike is several return strokes down one channel, tens of
/// milliseconds apart, each a sharp peak; strike times land on export frames so every peak is seen.
fn lightning_at(l: &crate::scene::Lightning, tphase: f32, loop_seconds: f32, frames: u32) -> Option<Strike> {
    if !l.enabled || l.strikes == 0 || l.intensity <= 0.0 { return None; }
    let n = l.strikes.min(16) as i64;
    let mut best: Option<Strike> = None;
    for i in 0..n {
        let p = strike_phase(l.seed, i, n, frames);
        let dt = (tphase - p).rem_euclid(1.0) * loop_seconds;
        if dt > 1.0 { continue; }
        let strokes = 2 + (hash(l.seed ^ 0x6B, i) % 3) as i64;
        let (mut flash, mut bolt, mut at) = (0.0f32, 0.0f32, 0.0f32);
        for k in 0..strokes {
            if k > 0 { at += 0.07 + 0.12 * hf(l.seed ^ 0x7C, i * 8 + k); }
            let ds = dt - at;
            if ds < 0.0 { break; }
            let amp = if k == 0 { 1.0 } else { 0.45 + 0.45 * hf(l.seed ^ 0x8D, i * 8 + k) };
            flash += amp * (-ds / 0.09).exp();
            bolt = bolt.max(amp * (-ds / 0.05).exp());
        }
        if flash > 0.01 && best.as_ref().map_or(true, |b| flash > b.flash) {
            best = Some(Strike { index: i, flash: flash.min(1.6), bolt, x: 0.12 + 0.76 * hf(bolt_seed(l.seed, i), 0) });
        }
    }
    best
}

fn bolt_seed(seed: u32, index: i64) -> u32 { seed ^ 0x9E3 ^ (index as u32).wrapping_mul(0x9E37_79B9) }

/// The bolt, and the sky lit up behind it. Drawn on sky pixels only.
fn draw_lightning(ctx: &Ctx, s: &Strike, flash: [f32; 3], gbuf: &[GPixel], hdr: &mut [[f32; 3]]) {
    let v = &ctx.view;
    let (w, h) = (v.width, v.height);
    let hy = v.horizon_px.max(2.0);
    // Placed in the frame; a guarded view renders more above and beside it.
    let oy = v.top as f32;
    let fhy = (hy - oy).max(2.0);
    let l = &ctx.scene.weather.lightning;
    // The clouds light up from inside: brighter high in the sky.
    for y in 0..(hy as usize).min(h) {
        let k = 0.55 * (1.0 - ((y as f32 - oy) / fhy).max(0.0)).powf(0.7) + 0.15;
        for x in 0..w {
            if gbuf[y * w + x].id == id::NONE { hdr[y * w + x] = add3(hdr[y * w + x], scale3(flash, k)); }
        }
    }
    if !l.bolts || s.bolt < 0.05 { return; }
    let scale = (h as f32 - oy) / 854.0;
    let segs = bolt_segments(ctx, s);
    let core = rgb_lin([235, 240, 255]);
    let tint = rgb_lin(l.color);
    let glow_r = 7.0 * scale;
    let (mut bx0, mut bx1, mut by1) = (f32::MAX, f32::MIN, 0.0f32);
    for (a, b, _) in &segs { bx0 = bx0.min(a[0].min(b[0])); bx1 = bx1.max(a[0].max(b[0])); by1 = by1.max(a[1].max(b[1])); }
    let (xa, xb) = ((bx0 - glow_r * 3.0).max(0.0) as usize, ((bx1 + glow_r * 3.0) as usize).min(w - 1));
    let yb = ((by1 + glow_r) as usize).min(h - 1);
    let gain = s.bolt * l.intensity.max(0.0);
    let rows: Vec<(usize, Vec<[f32; 3]>)> = (0..=yb).into_par_iter().map(|y| {
        let mut row = vec![[0.0f32; 3]; xb + 1 - xa];
        for x in xa..=xb {
            if gbuf[y * w + x].id != id::NONE { continue; }
            let pt = [x as f32 + 0.5, y as f32 + 0.5];
            let (mut c, mut g) = (0.0f32, 0.0f32);
            for (a, b, wgt) in &segs {
                let ab = [b[0] - a[0], b[1] - a[1]];
                let t = (((pt[0] - a[0]) * ab[0] + (pt[1] - a[1]) * ab[1]) / (ab[0] * ab[0] + ab[1] * ab[1]).max(1e-6)).clamp(0.0, 1.0);
                let d = ((pt[0] - a[0] - ab[0] * t).powi(2) + (pt[1] - a[1] - ab[1] * t).powi(2)).sqrt();
                let width = (0.9 * scale * wgt).max(0.5);
                c = c.max(wgt * (1.0 - smoothstep(width, width + 1.0, d)));
                g = g.max(wgt * (-d / (glow_r * wgt.max(0.5))).exp());
            }
            row[x - xa] = add3(scale3(core, 6.0 * c), scale3(tint, 0.9 * g));
        }
        (y, row)
    }).collect();
    for (y, row) in rows {
        for (i, c) in row.into_iter().enumerate() {
            let p = &mut hdr[y * w + xa + i];
            *p = add3(*p, scale3(c, gain));
        }
    }
}

/// A strike's channel as line segments (from, to, weight): a jagged walk down from the top, with a
/// branch or two.
fn bolt_segments(ctx: &Ctx, s: &Strike) -> Vec<([f32; 2], [f32; 2], f32)> {
    let v = &ctx.view;
    let w = v.width;
    let hy = v.horizon_px.max(2.0);
    let (ox, oy) = (v.left as f32, v.top as f32);
    let (fw, fhy) = (w as f32 - 2.0 * ox, (hy - oy).max(2.0));
    let l = &ctx.scene.weather.lightning;
    let seed = bolt_seed(l.seed, s.index);
    let mut segs: Vec<([f32; 2], [f32; 2], f32)> = Vec::new();
    let x0 = ox + fw * s.x;
    let bottom = fhy * (0.75 + 0.2 * hf(seed, 1));
    let steps = 14;
    let dy = bottom / steps as f32;
    // Above the frame the channel carries on up to the top of what is rendered.
    let mut p = [x0, oy];
    let mut up = (hf(seed, 3) - 0.5) * 0.6;
    for k in 0.. {
        if p[1] <= 0.0 { break; }
        let q = [p[0] + (up + (hf(seed, 300 + k) - 0.5) * 1.6) * dy, p[1] - dy];
        segs.push((q, p, 1.0));
        up = up * 0.7 + (hf(seed, 330 + k) - 0.5) * 0.3;
        p = q;
    }
    let mut p = [x0, oy];
    let mut drift = (hf(seed, 2) - 0.5) * 0.6;
    for k in 0..steps {
        let dx = (drift + (hf(seed, 10 + k) - 0.5) * 1.6) * dy;
        let q = [p[0] + dx, p[1] + dy];
        segs.push((p, q, 1.0));
        if k > 2 && k < steps - 3 && hf(seed, 40 + k) > 0.78 {
            // A fork: thinner, dimmer, dies out partway down.
            let mut b = q;
            let side = if hf(seed, 60 + k) > 0.5 { 1.0 } else { -1.0 };
            for j in 0..(3 + (hash(seed, 80 + k) % 4) as i64) {
                let c = [b[0] + side * dy * (0.5 + hf(seed, 100 + k * 8 + j)), b[1] + dy * (0.6 + 0.4 * hf(seed, 200 + k * 8 + j))];
                segs.push((b, c, 0.45));
                b = c;
            }
        }
        drift = drift * 0.7 + (hf(seed, 30 + k) - 0.5) * 0.3;
        p = q;
    }
    segs
}

// ── Sky ────────────────────────────────────────────────────────────────────

/// The colours stars range between, cool to warm.
pub(super) const STAR_COOL: [f32; 3] = [0.75, 0.85, 1.0];
pub(super) const STAR_WARM: [f32; 3] = [1.0, 0.9, 0.75];

/// One star as drawn: its centre, its width, how many whole pixels out it reaches, its brightness
/// and its colour (0 cool .. 1 warm).
#[derive(Clone, Copy)]
struct Star { x: f32, y: f32, r: f32, ri: i64, b: f32, t: f32 }
/// How much of a star's light its glow holds at bloom 1 (see `star_list`).
const STAR_GLOW: f32 = 1.5;

/// The stars of a world's sky this frame, in drawing order (the CPU and GPU skies both draw these).
/// The shooting stars burning now, in pixels: tail (x, y), head (x, y), brightness, width and
/// colour, as both renderers draw them (packed for the sky pass in this order).
pub(super) fn meteor_list(ctx: &Ctx) -> Vec<[f32; 9]> {
    let sky = &ctx.scene.sky;
    let m = &sky.meteors;
    if !sky.enabled || !m.enabled || m.count == 0 || m.brightness <= 0.0 || sky.tunnel.enabled { return Vec::new(); }
    let v = &ctx.view;
    let hy = v.horizon_px.max(2.0);
    let (ox, oy) = (v.left as f32 + v.sky_dx(), v.top as f32);
    let (fw, fhy) = (v.width as f32 - 2.0 * v.left as f32, (hy - v.top as f32).max(2.0));
    let dur = (m.duration.max(0.05) / ctx.scene.motion.loop_seconds().max(0.1)).min(0.5);
    let base = rgb_lin(m.color);
    let w = (0.7 * (v.height as f32 / 854.0)).max(0.6);
    let mut out = Vec::new();
    for i in 0..m.count.min(64) as i64 {
        let h = |k: u32| hf(m.seed.wrapping_mul(0x9E37) ^ k, i);
        let p = (ctx.tphase - h(0x6E1)).rem_euclid(1.0);
        if p >= dur { continue; }
        let u = p / dur;
        let a = (m.angle + (h(0x6E2) - 0.5) * m.spread).to_radians();
        let (dx, dy) = (a.cos(), -a.sin());
        let len = m.length * fhy * (0.6 + 0.8 * h(0x6E3));
        let (sx, sy) = (ox + (h(0x6E4) * 1.2 - 0.1) * fw, oy + h(0x6E5) * 0.75 * fhy);
        let (hx, hy2) = (sx + dx * len * u, sy + dy * len * u);
        let tail = len * 0.35 * (u / 0.25).min(1.0);
        let env = (std::f32::consts::PI * u).sin().max(0.0).sqrt() * m.brightness * (0.5 + h(0x6E6));
        let tint = if h(0x6E7) < 0.5 { [0.55, 1.0, 0.65] } else { [1.0, 0.65, 0.35] };
        let col = mix3(base, mul3(base, tint), m.tint.clamp(0.0, 1.0) * h(0x6E8));
        out.push([hx - dx * tail, hy2 - dy * tail, hx, hy2, env, w, col[0], col[1], col[2]]);
    }
    out
}

/// One shooting star's light at pixel centre (px, py): a streak brightening to its head, and the
/// head's glow.
pub(super) fn meteor_px(m: &[f32; 9], px: f32, py: f32) -> [f32; 3] {
    let (ax, ay, bx, by, env, w) = (m[0], m[1], m[2], m[3], m[4], m[5]);
    let (vx, vy) = (bx - ax, by - ay);
    let l2 = (vx * vx + vy * vy).max(1e-4);
    let t = (((px - ax) * vx + (py - ay) * vy) / l2).clamp(0.0, 1.0);
    let (qx, qy) = (ax + vx * t - px, ay + vy * t - py);
    let d2 = qx * qx + qy * qy;
    let (hx, hy) = (bx - px, by - py);
    let h2 = hx * hx + hy * hy;
    let k = env * (t * t * (-d2 / (2.0 * w * w)).exp() + 1.5 * (-h2 / (2.0 * 3.2 * w * w)).exp());
    [m[6] * k, m[7] * k, m[8] * k]
}

fn star_list(ctx: &Ctx) -> Vec<Star> {
    let sky = &ctx.scene.sky;
    if !sky.stars.enabled { return Vec::new(); }
    let v = &ctx.view;
    let (w, h) = (v.width, v.height);
    let hy = v.horizon_px.max(2.0);
    let (ox, oy) = (v.left as f32, v.top as f32);
    let (fw, fhy) = (w as f32 - 2.0 * ox, (hy - oy).max(2.0));
    let scale = (h as f32 - oy) / 854.0;
    let tw = sky.stars.twinkle.clamp(0.0, 4.0);
    let n = sky.stars.count.min(6000) as i64;
    // The sky outside the frame has stars too, as many per pixel as the top of the frame.
    let area = fw * fhy * 0.95;
    let extra = |a: f32| (n as f32 * 1.2 * a / area.max(1.0)).round() as i64;
    let (n_top, n_side) = (extra(w as f32 * oy), extra(ox * fhy * 0.95));
    // As the camera turns with a winding path the sky slides sideways (View::sky_dx). The stars
    // are then laid over a strip several frames wide that comes round, wide enough that no
    // star is seen twice in one swing.
    let (sdx, swing) = (v.sky_dx(), v.sky_swing());
    let tiles = if swing > 0.0 { (1 + 2 * (swing / fw.max(1.0)).ceil() as i64).clamp(3, 7) } else { 1 };
    let back = ((tiles - 1) / 2) as f32;
    let round = |x: f32, span: f32, j: i64| if tiles == 1 { x } else { (x + (j as f32 + back) * span + sdx).rem_euclid(tiles as f32 * span) - back * span };
    // Bloom: a bright star is far brighter than a pixel can show, and the lens spreads some
    // of that light round it. (The frame's bloom cannot: a star is too small to carry any.)
    let bloom = ctx.scene.post.bloom.clamp(0.0, 4.0);
    let mut out = Vec::new();
    for j in 0..tiles { for i0 in 0..n + n_top + 2 * n_side {
        let i = i0 + j * 100_003;
        let (sx, sy) = if i0 < n {
            (ox + round(hf(sky.stars.seed ^ 0xA1, i) * fw, fw, j), oy + hf(sky.stars.seed ^ 0xB2, i).powf(1.4) * fhy * 0.95)
        } else if i0 < n + n_top {
            (round(hf(sky.stars.seed ^ 0xA7, i) * w as f32, w as f32, j), hf(sky.stars.seed ^ 0xB8, i) * oy)
        } else {
            // (The strip covers the sides of a sky that slides.)
            if tiles > 1 { continue; }
            let x = hf(sky.stars.seed ^ 0xA7, i) * ox;
            (if (i - n - n_top) % 2 == 0 { x } else { w as f32 - x }, oy + hf(sky.stars.seed ^ 0xB8, i) * fhy * 0.95)
        };
        let freq = 1.0 + (hash(sky.stars.seed ^ 0xC3, i) % 4) as f32;
        let tw_v = 0.5 + 0.5 * (TAU * freq * ctx.tphase + hf(sky.stars.seed ^ 0xD4, i) * TAU).sin();
        // Most stars are faint and a few are bright; each is a soft point (`r` its width), never
        // narrower than a pixel can hold, dimmed instead so its light adds up the same.
        let mag = hf(sky.stars.seed ^ 0xE5, i);
        let b = (0.16 + 1.7 * mag * mag * mag) * (1.0 - tw * 0.25 + tw * 0.25 * tw_v) * 1.4;
        let natural = (sky.stars.size * scale * (0.26 + 0.26 * hf(sky.stars.seed ^ 0xF6, i) + 0.32 * mag)).max(0.05);
        let r = natural.max(0.62);
        let b = b * (natural / r).powi(2).max(0.2);
        let t = hf(sky.stars.seed ^ 0x1F7, i);
        // Its glow: wider for a brighter star, holding STAR_GLOW times the star's own light.
        let gr = r * (3.5 + 3.5 * mag);
        let gb = STAR_GLOW * bloom * b * (r / gr).powi(2);
        for (r, b, glow) in [(r, b, false), (gr, gb, true)] {
            let ri = (2.6 * r).ceil() as i64;
            if (glow && b < 0.003) || (tiles > 1 && (sx < -(ri as f32) || sx > (w as i64 + ri) as f32)) { continue; }
            out.push(Star { x: sx, y: sy, r, ri, b, t });
        }
    }}
    out
}

/// One soft ellipse of a cloud, centred at (x, y), with its share of the cloud's life.
#[derive(Clone, Copy)]
struct Blob { x: f32, y: f32, rx: f32, ry: f32, life: f32 }
/// The clouds this frame: their colour, the sun that lights their edges, and every blob in order.
struct CloudSet { col: [f32; 3], sun_at: Option<(f32, f32, [f32; 3])>, opacity: f32, blobs: Vec<Blob> }

fn cloud_list(ctx: &Ctx) -> Option<CloudSet> {
    let sky = &ctx.scene.sky;
    if !sky.clouds.enabled || sky.clouds.kind != crate::scene::CloudKind::Soft { return None; }
    let v = &ctx.view;
    let w = v.width;
    let hy = v.horizon_px.max(2.0);
    let (ox, oy) = (v.left as f32, v.top as f32);
    let (fw, fhy) = (w as f32 - 2.0 * ox, (hy - oy).max(2.0));
    let cl = &sky.clouds;
    let span = fw * 1.6;
    // A whole number of crossings per loop wraps each cloud back to where it
    // started. Any other drift (0.4x, 1.5x) cannot: a cloud would end the loop
    // somewhere else. Then each cloud lives exactly one loop instead, forming,
    // drifting at the speed asked and dissolving, with the lives staggered so
    // the sky never empties; the last frame still matches the first.
    let drift = cl.drift.max(0.0);
    let whole = (drift - drift.round()).abs() < 1e-3;
    let tint = rgb_lin(cl.tint);
    let lit = add3(scale3(ctx.ambient, 0.6), ctx.sky_lights.iter().fold([0.0; 3], |a, s| add3(a, scale3(s.color, 0.4))));
    let col = mul3(tint, add3(lit, [0.35; 3]));
    // Clouds near the sun light up, most at their thin edges, where light comes through.
    let sdx = v.sky_dx();
    let sun_at = (sky.sun.enabled).then(|| (ox + sdx + sky.sun.pos[0] * fw, oy + sky.sun.pos[1].clamp(0.0, 1.0) * fhy, scale3(rgb_lin(sky.sun.color), 0.6)));
    let mut blobs = Vec::new();
    for i in 0..cl.count.min(200) as i64 {
        let base = hf(cl.seed ^ 0x11, i) * span;
        let speed = drift * (1.0 + (hash(cl.seed ^ 0x22, i) % 2) as f32);
        let (travel, life) = if whole {
            (speed * span * ctx.tphase, 1.0)
        } else {
            let ph = (ctx.tphase + hf(cl.seed ^ 0x77, i)).rem_euclid(1.0);
            // Centred on its spot over its life; fades over a fifth of it at each end.
            (speed * span * (ph - 0.5), smoothstep(0.0, 0.2, ph) * smoothstep(0.0, 0.2, 1.0 - ph))
        };
        if life <= 0.0 { continue; }
        let cx = ox + (base + travel + sdx).rem_euclid(span) - (span - fw) * 0.5;
        let cy = oy + fhy * (0.12 + 0.6 * hf(cl.seed ^ 0x33, i));
        let size = fhy * (0.06 + 0.08 * hf(cl.seed ^ 0x44, i)) * cl.scale;
        let n = 3 + (cl.variation * 4.0) as i64;
        for b in 0..n {
            let t = b as f32 / (n - 1).max(1) as f32;
            let bx = cx + (t - 0.5) * size * (1.6 + cl.variation);
            let by = cy - size * 0.25 * (t * 3.1).sin().abs() * (0.5 + cl.variation);
            let (rx, ry) = (size * (0.55 + 0.35 * hf(cl.seed ^ 0x55, i * 8 + b)), size * (0.32 + 0.2 * hf(cl.seed ^ 0x66, i * 8 + b)));
            blobs.push(Blob { x: bx, y: by, rx, ry, life });
        }
    }
    Some(CloudSet { col, sun_at, opacity: cl.opacity, blobs })
}

/// The moon's large craters for a seed: (x, y, z) on the near hemisphere and a radius (0 for
/// none), many small and a few large. The sky pass gets the same list.
pub(super) fn moon_craters(seed: u32) -> [[f32; 4]; 22] {
    let mut out = [[0.0f32; 4]; 22];
    let s = seed.wrapping_mul(0x9E37_79B9);
    for (k, c) in out.iter_mut().enumerate() {
        let k = k as i64;
        let (u, v) = (hf(0x3A1 ^ s, k) * 1.8 - 0.9, hf(0x3A2 ^ s, k) * 1.8 - 0.9);
        *c = if u * u + v * v > 0.85 { [0.0, 0.0, 0.0, -1.0] } else { [u, v, (1.0 - u * u - v * v).sqrt(), 0.04 + 0.16 * hf(0x3A3 ^ s, k).powi(3)] };
    }
    out
}

/// One crater's look at distance `d` (in its radii) from its centre: (albedo factor, slope of
/// its height outwards, per radius). A bowl with a darker floor, a raised bright rim, a blanket
/// of ejecta round it, and (the large ones) a central peak. `e` softens edges to a pixel, `peak`
/// 0..1 how much of a central peak it has.
fn crater_profile(d: f32, e: f32, peak: f32) -> (f32, f32) {
    let floor = 1.0 - smoothstep(0.78 - e, 0.82 + e, d);
    let rim = smoothstep(0.72 - e, 0.92, d) * (1.0 - smoothstep(1.0, 1.2 + e, d));
    let alb = 1.0 + 0.07 * rim - 0.09 * floor;
    // Height: -0.35 (1 - d^2) inside, a rim ridge at 1, the peak at the middle.
    let qr = (d - 1.0) / 0.17;
    let mut slope = if d < 1.0 { 0.7 * d } else { 0.0 };
    slope += 0.13 * (-2.0 * qr / 0.17) * (-qr * qr).exp();
    let qp = d / 0.16;
    slope += peak * 0.12 * (-2.0 * qp / 0.16) * (-qp * qp).exp();
    (alb, slope)
}

/// The moon's surface at a point `n` of its sphere (unit radius, z toward the viewer, y down):
/// its brightness, and how much the sun (`sun`, its x and z) lights it there with the craters'
/// relief (`relief` 0 flat .. 2 rugged). Soft dark maria, the rays of young craters, `seed`'s
/// field of large craters (`big`, from `moon_craters`) and two finer scales of small ones (fewer
/// on the smooth seas), and fine mottling. Fixed to the moon, so the same at every phase; every
/// edge is at least a pixel wide (`px`, disc units per pixel), so a small moon blurs instead of
/// turning blocky, and relief finer than a pixel fades out.
fn moon_surface(n: [f32; 3], px: f32, sun: [f32; 2], big: &[[f32; 4]; 22], seed: u32, relief: f32) -> (f32, f32) {
    use super::looks::value_noise as vn;
    let [x, y, z] = n;
    // The seas, where they lie on the near side, their shores broken up.
    let warp = 0.7 * (vn(x * 5.0 + 1.7, y * 5.0 + 3.9) - 0.5) + 0.3 * (vn(x * 11.0 + 8.0, y * 11.0 + 2.0) - 0.5) + 0.12 * (vn(x * 23.0 + 4.0, y * 23.0 + 6.0) - 0.5);
    let mut sea = 0.0f32;
    for m in MARIA {
        let d = (((x - m[0]) / m[2]).powi(2) + ((y - m[1]) / m[3]).powi(2)).sqrt() + warp;
        let soft = (px / m[2].min(m[3])).max(0.08);
        sea = sea.max(1.0 - smoothstep(0.82 - soft, 1.0 + soft, d));
    }
    // The seas' own darker and lighter lava flows.
    sea *= 0.8 + 0.3 * vn(x * 9.0 + 5.0, y * 9.0 + 1.0);
    // Highlands bright and rough, seas dark and smooth.
    let mut a = (1.0 - 0.5 * sea) * (0.9 + 0.2 * vn(x * 6.5 + 2.0, y * 6.5 + 5.0)) * (1.0 + 0.06 * (1.0 - sea) * (vn(x * 14.0 + 3.0, y * 14.0 + 7.0) - 0.5));
    // Small craters only show on a moon big enough to hold them.
    let pits = 1.0 - smoothstep(0.02, 0.07, px);
    // Young craters and the bright rays thrown out from them.
    for rc in RAY_CRATERS {
        let (vx, vy) = (x - rc[0], y - rc[1]);
        let dist = (vx * vx + vy * vy).sqrt();
        if dist > rc[2] * 3.0 { continue; }
        let (ux, uy) = (vx / dist.max(1e-4), vy / dist.max(1e-4));
        let streak = smoothstep(0.5, 0.78, vn(ux * 4.5 + 10.0 + rc[0] * 7.0, uy * 4.5 + 20.0));
        let spot = (-(dist / (0.09 * rc[2] + px)).powi(2)).exp();
        a += rc[3] * (0.75 * streak * (-dist / rc[2]).exp() * (1.0 - (-dist / (0.1 * rc[2] + px)).exp()) + spot);
    }
    let (mut gx, mut gy) = (0.0f32, 0.0f32);
    for c in big {
        if c[3] <= 0.0 { continue; }
        let (vx, vy) = (x - c[0], y - c[1]);
        let d = (vx * vx + vy * vy + (z - c[2]) * (z - c[2])).sqrt() / c[3];
        if d > 1.5 { continue; }
        let (alb, sl) = crater_profile(d, (px / c[3]).max(0.06), smoothstep(0.08, 0.16, c[3]));
        a *= 1.0 + pits * (alb - 1.0);
        // Relief only where the crater spans a few pixels.
        let k = sl * (1.0 - smoothstep(0.25, 0.6, px / c[3]));
        let dl = (vx * vx + vy * vy).sqrt().max(1e-4);
        gx += k * vx / dl;
        gy += k * vy / dl;
    }
    // Two finer scales of small craters, one to a cell at most, fewer on the seas.
    let s = seed.wrapping_mul(0x9E37);
    for (cell, salt) in [(0.12f32, 0x3B0u32), (0.055, 0x3C0)] {
        let (ix, iy) = ((x / cell).floor() as i32, (y / cell).floor() as i32);
        let k = salt.wrapping_add(s);
        if space::cell_hash(ix, iy, k) >= 0.3 * (1.0 - 0.7 * sea) { continue; }
        let h = space::cell_hash(ix, iy, k.wrapping_add(3));
        let rad = cell * (0.08 + 0.3 * h * h);
        let cx = (ix as f32 + 0.3 + 0.4 * space::cell_hash(ix, iy, k.wrapping_add(1))) * cell;
        let cy = (iy as f32 + 0.3 + 0.4 * space::cell_hash(ix, iy, k.wrapping_add(2))) * cell;
        let (vx, vy) = (x - cx, y - cy);
        let dl = (vx * vx + vy * vy).sqrt();
        let d = dl / rad;
        if d > 1.5 { continue; }
        let (alb, sl) = crater_profile(d, (px / rad).max(0.06), 0.0);
        let fine = 1.0 - smoothstep(0.012, 0.035, px);
        a *= 1.0 + fine * (alb - 1.0);
        let kk = sl * (1.0 - smoothstep(0.25, 0.6, px / rad));
        gx += kk * vx / dl.max(1e-4);
        gy += kk * vy / dl.max(1e-4);
    }
    // Fine mottling, faded out where it would be smaller than a pixel.
    let grain = (1.0 - smoothstep(0.03, 0.08, px)) * (vn(x * 24.0 + 9.0, y * 24.0) - 0.5);
    a *= 1.0 + 0.16 * grain;
    // The surface tilted by the slopes, lit by the sun.
    let r = 0.28 * relief;
    let (nx, ny, nz) = (x - r * gx, y - r * gy, z);
    let l = (nx * nx + ny * ny + nz * nz).sqrt().max(1e-5);
    (a, (nx * sun[0] + nz * sun[1]) / l)
}

/// The moon's seas as it faces the Earth: (x, y, half width, half height) on its disc, north up
/// (Procellarum, Imbrium, Serenitatis, Tranquillitatis, Crisium, Fecunditatis, Nectaris, Nubium,
/// Humorum, Frigoris, Vaporum).
pub(super) const MARIA: [[f32; 4]; 11] = [
    [-0.55, -0.05, 0.33, 0.52], [-0.2, -0.45, 0.32, 0.28], [0.27, -0.4, 0.2, 0.2], [0.42, -0.1, 0.24, 0.22], [0.78, -0.28, 0.09, 0.12], [0.68, 0.1, 0.12, 0.18], [0.48, 0.27, 0.1, 0.1], [-0.2, 0.36, 0.24, 0.2], [-0.5, 0.4, 0.11, 0.11], [0.0, -0.8, 0.45, 0.06], [0.06, -0.22, 0.1, 0.1],
];
/// Bright young craters: (x, y, how far their rays reach, brightness): Tycho, Copernicus, Kepler,
/// Aristarchus.
pub(super) const RAY_CRATERS: [[f32; 4]; 4] = [[-0.12, 0.69, 0.5, 0.22], [-0.33, -0.15, 0.2, 0.16], [-0.6, -0.13, 0.13, 0.13], [-0.68, -0.39, 0.08, 0.2]];

/// How bright the moon's lit surface is against its colour (it is bright enough to glow a little).
pub(super) const MOON_GAIN: f32 = 1.35;

/// How bright the moon's lit face is against the sky behind it (`top` and `horizon` at its height
/// `pos_y`): a day moon as bright as ever (`MOON_GAIN`); against a night sky less, as a camera
/// would expose for it, so its seas and craters show instead of a white disc.
pub(super) fn moon_gain(sky: &crate::scene::Sky) -> f32 {
    let t = sky.moon.body.pos[1].clamp(0.0, 1.0);
    let c = mix3(rgb_lin(sky.top), rgb_lin(sky.horizon), t);
    let l = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    0.8 + (MOON_GAIN - 0.8) * smoothstep(0.02, 0.25, l)
}
/// How bright the sun's disc is against its colour: far past white, so the lens blooms it and its
/// edge is lost in its own glare.
pub(super) const SUN_GAIN: f32 = 12.0;

/// The glow of the air in front of the moon, `d` of its radii out from its limb.
#[inline]
fn moon_halo(d: f32) -> f32 {
    (0.2 * (-d / 0.3).exp() + 0.06 * (-d / 1.8).exp()) * (1.0 - d / MOON_HALO).max(0.0).powi(2)
}
/// How far the moon's glow reaches, in its radii beyond the limb.
pub(super) const MOON_HALO: f32 = 5.0;

fn draw_sky_bodies(ctx: &Ctx, gbuf: &[GPixel], hdr: &mut [[f32; 3]]) {
    let v = &ctx.view;
    let (w, h) = (v.width, v.height);
    let sky = &ctx.scene.sky;
    let hy = v.horizon_px.max(2.0);
    // Everything is placed in the frame; a guarded view renders more sky above and beside it.
    let (ox, oy) = (v.left as f32, v.top as f32);
    let (fw, fhy) = (w as f32 - 2.0 * ox, (hy - oy).max(2.0));
    let is_sky = |x: i64, y: i64| x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h && (y as f32) < hy && gbuf[y as usize * w + x as usize].id == id::NONE;
    let add = |hdr: &mut [[f32; 3]], x: i64, y: i64, c: [f32; 3], a: f32, additive: bool| {
        if !is_sky(x, y) { return; }
        let p = &mut hdr[y as usize * w + x as usize];
        *p = if additive { add3(*p, scale3(c, a)) } else { mix3(*p, c, a.clamp(0.0, 1.0)) };
    };
    // The moon hides the stars behind it, its dark side too.
    let sdx = v.sky_dx();
    let moon_disc = sky.moon.body.enabled.then(|| (ox + sdx + sky.moon.body.pos[0] * fw, oy + sky.moon.body.pos[1].clamp(0.0, 1.0) * fhy, sky.moon.body.radius * fhy));
    let behind_moon = |x: i64, y: i64| moon_disc.is_some_and(|(cx, cy, r)| (x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2) < r * r);
    space::draw_backdrop(ctx, gbuf, hdr);
    // A sunset's glow, low along the horizon and round the sun.
    if let Some(gl) = clouds::glow(ctx) {
        let rows = (hy.ceil() as usize).min(h);
        hdr[..rows * w].par_chunks_mut(w).enumerate().for_each(|(y, row)| {
            for (x, p) in row.iter_mut().enumerate() {
                if is_sky(x as i64, y as i64) { *p = add3(*p, gl.at(x, y)); }
            }
        });
    }
    for st in star_list(ctx) {
        let (ix, iy, ri) = (st.x.floor() as i64, st.y.floor() as i64, st.ri);
        let col = mix3(STAR_COOL, STAR_WARM, st.t);
        for oy in -ri..=ri { for ox in -ri..=ri {
            let (px, py) = (ix + ox, iy + oy);
            if behind_moon(px, py) { continue; }
            let d2 = (px as f32 + 0.5 - st.x).powi(2) + (py as f32 + 0.5 - st.y).powi(2);
            add(hdr, px, py, col, st.b * (-d2 / (st.r * st.r)).exp(), true);
        }}
    }
    for m in meteor_list(ctx) {
        let r = 4.0 * m[5] + 2.0;
        let (x0, x1) = ((m[0].min(m[2]) - r).floor() as i64, (m[0].max(m[2]) + r).ceil() as i64);
        let (y0, y1) = ((m[1].min(m[3]) - r).floor() as i64, (m[1].max(m[3]) + r).ceil() as i64);
        for py in y0..=y1 { for px in x0..=x1 {
            if behind_moon(px, py) { continue; }
            add(hdr, px, py, meteor_px(&m, px as f32 + 0.5, py as f32 + 0.5), 1.0, true);
        }}
    }
    weather::aurora(ctx, gbuf, hdr);
    if sky.sun.enabled {
        // The sun lights the air round it: the sky is far brighter near it (the aureole), a
        // narrow bright core and a broad faint skirt, both wider in haze or fog. Its disc is
        // darker toward the limb.
        let su = &sky.sun;
        let (cx, cy, r) = (ox + sdx + su.pos[0] * fw, oy + su.pos[1].clamp(0.0, 1.0) * fhy, (su.radius * fhy).max(0.75));
        let c = scale3(rgb_lin(su.color), 0.4 + 0.6 * su.intensity.clamp(0.0, 3.0));
        let fog = ctx.fog.map_or(0.0, |(_, d)| (40.0 / d).min(1.5));
        let spread = 1.0 + 2.5 * ctx.wx.veil + fog;
        let focal = v.focal_px.max(1.0);
        // Seen from space there is no air to light: only the glare round the disc.
        let air = if sky.space.enabled { 0.0 } else { 1.0 };
        hdr.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
            if y as f32 >= hy { return; }
            for (x, p) in row.iter_mut().enumerate() {
                if gbuf[y * w + x].id != id::NONE { continue; }
                let dd = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
                let th = dd / focal;
                // Its glare spills past its edge, so the disc has no outline.
                let glare = 1.1 * (-(dd - r).max(0.0) / (0.4 * r + 1.0)).exp();
                let glow = air * (0.45 * (-th / (0.05 * spread)).exp() + 0.1 * (-th / (0.35 * spread)).exp()) + (1.0 - air) * 0.5 * (-th / 0.01).exp() + glare;
                *p = add3(*p, scale3(c, glow));
                if dd < r + 0.5 {
                    let mu = (1.0 - (dd / r).min(1.0).powi(2)).sqrt();
                    *p = mix3(*p, scale3(c, SUN_GAIN * (0.8 + 0.2 * mu)), (r + 0.5 - dd).clamp(0.0, 1.0));
                }
            }
        });
    }
    if sky.moon.body.enabled {
        let m = &sky.moon;
        let (cx, cy, r) = (ox + sdx + m.body.pos[0] * fw, oy + m.body.pos[1].clamp(0.0, 1.0) * fhy, m.body.radius * fhy);
        let c = rgb_lin(m.body.color);
        let gr = r * (1.0 + MOON_HALO);
        // The moon is a sphere lit by the sun at the phase angle: 0 full (sun behind the viewer),
        // ±1 new; waxing is lit on the right. A point of the disc is lit where its normal faces
        // the sun, which draws the terminator as the real half-ellipse.
        let ph = m.phase.clamp(-1.0, 1.0);
        let angle = ph.abs() * std::f32::consts::PI;
        let sun = [if ph < 0.0 { -angle.sin() } else { angle.sin() }, angle.cos()];
        let lit_fraction = 0.5 * (1.0 + angle.cos());
        let big = moon_craters(m.seed);
        let gain = moon_gain(sky);
        for y in (cy - gr) as i64..=(cy + gr) as i64 { for x in (cx - gr) as i64..=(cx + gr) as i64 {
            let (dx, dy) = ((x as f32 + 0.5 - cx) / r, (y as f32 + 0.5 - cy) / r);
            let dd = (dx * dx + dy * dy).sqrt();
            if dd < 1.0 {
                let nz = (1.0 - dd * dd).max(0.0).sqrt();
                // About a pixel of soft terminator.
                let edge = (1.5 / r.max(1.0)).clamp(0.01, 0.2);
                let mu0 = dx * sun[0] + nz * sun[1];
                // The craters' relief catches the light: rims lit past the line between day and
                // night, the far walls of the bowls in shadow.
                let (alb, mu) = if m.craters { moon_surface([dx, dy, nz], 1.0 / r.max(1.0), sun, &big, m.seed, m.relief.clamp(0.0, 3.0)) } else { (1.0, mu0) };
                let lit = smoothstep(-edge, edge, mu);
                // Dusty ground sends light back the way it came: a full moon is evenly bright to
                // its edge, and a part moon is brightest at its sunward limb.
                let shade = (2.0 * mu.max(0.0) / (mu.max(0.0) + nz).max(1e-3)).min(1.0);
                let crater = shade * alb;
                // Sunlit surface over the sky. The unlit side reflects almost nothing: what shows
                // there is the sky in front of it (the stars behind are hidden above), plus faint
                // earthshine, which only reads against a dark sky.
                if lit > 0.0 { add(hdr, x, y, scale3(c, gain * crater), m.opacity * lit, false); }
                add(hdr, x, y, c, 0.012 * m.opacity * (1.0 - lit), true);
                // The halo is scattered in the air in front of the moon, so it lies over the dark
                // side too (without it, the dark side read as a disc darker than the sky round it).
                // It glows round the lit part only: further into the dark side there is less.
                // Over the lit face it would only wash out the seas.
                add(hdr, x, y, c, lit_fraction * moon_halo(1.2 * (-mu0).max(0.0)) * (1.0 - 0.15 * lit), true);
            } else if dd * r < gr {
                add(hdr, x, y, c, lit_fraction * moon_halo(dd - 1.0 + 1.2 * (-dx / dd * sun[0]).max(0.0)), true);
            }
        }}
    }
    space::draw_planets(ctx, gbuf, hdr);
    blackhole::draw_infall(ctx, gbuf, hdr);
    if let Some(cs) = cloud_list(ctx) {
        for bl in &cs.blobs {
            let (bx, by, rx, ry, life) = (bl.x, bl.y, bl.rx, bl.ry, bl.life);
            for y in (by - ry) as i64..=(by + ry) as i64 { for x in (bx - rx) as i64..=(bx + rx) as i64 {
                let (dx, dy) = ((x as f32 - bx) / rx, (y as f32 - by) / ry);
                let d2 = dx * dx + dy * dy;
                if d2 < 1.0 {
                    let shade = 1.0 - 0.25 * dy.max(0.0);
                    let mut c = scale3(cs.col, shade);
                    if let Some((sx, sy, sc)) = cs.sun_at {
                        let near = (-((x as f32 - sx).powi(2) + (y as f32 - sy).powi(2)) / (0.3 * fhy).powi(2)).exp();
                        c = add3(c, scale3(sc, near * (0.25 + 0.9 * d2)));
                    }
                    add(hdr, x, y, c, cs.opacity * life * (1.0 - d2).powf(1.2) * 0.8, false);
                }
            }}
        }
    }
    let layers = clouds::layers(ctx);
    if !layers.is_empty() {
        let rows = (hy.ceil() as usize).min(h);
        hdr[..rows * w].par_chunks_mut(w).enumerate().for_each(|(y, row)| {
            if (y as f32) >= hy { return; }
            let v = (y as f32 + 0.5 - oy) / fhy;
            for (x, p) in row.iter_mut().enumerate() {
                if gbuf[y * w + x].id != id::NONE { continue; }
                let u = (x as f32 + 0.5 - (ox + sdx)) / fhy;
                for k in &layers {
                    let (c, a) = k.px(u, v);
                    if a > 0.0 { *p = add3(scale3(*p, 1.0 - a), c); }
                }
            }
        });
    }
    weather::rainbow(ctx, gbuf, hdr);
}

// ── Billboards, flames, tufts, particles ───────────────────────────────────

/// A billboard placed on screen and lit, ready to draw any band of rows: cards are set up once,
/// then drawn far to near in bands of rows side by side.
struct Card<'a> {
    b: &'a Billboard,
    w: usize,
    sx: f32, wp: f32, hp: f32, sy_top: f32, sway_px: f32,
    x0: i64, x1: i64, y0: i64, y1: i64,
    light: [f32; 3],
    /// Snow on it: the cap's thickness (a share of its height), the snow's colour as lit, and how
    /// much more lies caught on the upper sides of its clumps (0..1).
    snow: Option<(f32, [f32; 3], f32)>,
    z: f32, nearest: bool,
    fog: (bool, f32, [f32; 3], f32, [f32; 3]),
    /// A card standing in the world (`Billboard::panel`): pixel (X, Y) to its picture is
    /// u = (h0 X + h1 Y + h2) / w, v = (h3 X + h4 Y + h5) / w with w = h6 X + h7 Y + h8, and its
    /// depth there is 1 / (q0 X + q1 Y + q2); `strip` (or below 0) is the column an end wall shows.
    pan: Option<([f32; 9], [f32; 3], f32)>,
    /// A cylinder (`Billboard::tube`), drawn by `tube_at`.
    tube: Option<TubeCard>,
}

/// The mapping from pixels to a world-standing card's picture and depth (see `Card::pan`), for a
/// card whose picture's top left corner is `c0` (camera space) and whose width and height run
/// along `cu` and `cv`. Worked in f64: the per-pixel arithmetic is then the same few f32 steps on
/// the CPU and the GPU.
fn panel_map(v: &View, c0: [f32; 3], cu: [f32; 3], cv: [f32; 3]) -> Option<([f32; 9], [f32; 3])> {
    let d = |a: [f32; 3]| [a[0] as f64, a[1] as f64, a[2] as f64];
    let (c0, cu, cv) = (d(c0), d(cu), d(cv));
    let cross = |a: [f64; 3], b: [f64; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let sc = |a: [f64; 3], k: f64| [a[0] * k, a[1] * k, a[2] * k];
    let sub = |a: [f64; 3], b: [f64; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let n = cross(cu, cv);
    let (a, b) = (cross(cv, n), cross(n, cu));
    let (nc, ua, vb) = (dot(n, c0), dot(cu, a), dot(cv, b));
    if nc.abs() < 1e-9 || ua.abs() < 1e-12 || vb.abs() < 1e-12 { return None; }
    // Over the ray r = ((X - cx) / f, (cy - Y) / f, 1): u = hu.r / n.r, v = hv.r / n.r, 1/z = n.r / n.c0.
    let hu = sc(sub(sc(a, nc), sc(n, dot(c0, a))), 1.0 / ua);
    let hv = sc(sub(sc(b, nc), sc(n, dot(c0, b))), 1.0 / vb);
    let hz = sc(n, 1.0 / nc);
    // Scaled so the denominator is about 1 near the card (only ratios matter).
    let k = 1.0 / dot(n, n).sqrt().max(1e-12);
    let (f, cx, cy) = (v.focal_px as f64, v.center_px as f64, v.horizon_px as f64);
    let px = |h: [f64; 3]| [h[0] / f, -h[1] / f, -h[0] * cx / f + h[1] * cy / f + h[2]];
    let (u3, v3, w3, z3) = (px(sc(hu, k)), px(sc(hv, k)), px(sc(n, k)), px(hz));
    let f32s = |a: [f64; 3]| [a[0] as f32, a[1] as f32, a[2] as f32];
    let (u3, v3, w3) = (f32s(u3), f32s(v3), f32s(w3));
    Some(([u3[0], u3[1], u3[2], v3[0], v3[1], v3[2], w3[0], w3[1], w3[2]], f32s(z3)))
}

/// Smooth value noise for snow on cards (0..1); `cards.wgsl` has the same.
fn snow_noise(x: f32, y: f32) -> f32 {
    let h = |i: i32, j: i32| {
        let mut h = (i as u32).wrapping_mul(0x8da6_b343) ^ (j as u32).wrapping_mul(0xd816_3841) ^ 0x5157u32.wrapping_mul(0x9e37_79b9);
        h ^= h >> 15;
        h = h.wrapping_mul(0x2c1b_3c6d);
        h ^= h >> 12;
        (h & 0xffff) as f32 / 65535.0
    };
    let (xf, yf) = (x.floor(), y.floor());
    let (fx, fy) = (x - xf, y - yf);
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let (i, j) = (xf as i32, yf as i32);
    let a = h(i, j) + (h(i + 1, j) - h(i, j)) * sx;
    let b = h(i, j + 1) + (h(i + 1, j + 1) - h(i, j + 1)) * sx;
    a + (b - a) * sy
}

/// A card's colour at (u, v) under lying snow: a cap along every edge open above; and with
/// `dust`, snow caught on the tops of its clumps all through it: where sky shows a little above,
/// or where a darker gap or outline lies just above a lit clump, in patches, more higher up.
/// `cards.wgsl` repeats it step for step.
#[allow(clippy::too_many_arguments)]
fn snow_on_card(sprite: &Sprite, u: f32, v: f32, hp: f32, cap: f32, dust: f32, lit: [f32; 3], snow: [f32; 3]) -> [f32; 3] {
    let at = |dv: f32| if v - dv < 0.0 { [0.0; 4] } else { sprite.sample(u, v - dv, hp, true) };
    if at(cap)[3] < 0.5 { return snow; }
    if dust <= 0.0 { return lit; }
    let luma = |c: [f32; 4]| 0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2];
    // A step in the card's height, at least a pixel and a half.
    let step = (1.5 / hp.max(1.0)).max(0.012);
    let (here, a1, a2) = (sprite.sample(u, v, hp, true), at(step), at(2.5 * step));
    let open: f32 = if a1[3] < 0.5 { 1.0 } else if a2[3] < 0.5 { 0.7 } else { 0.0 };
    let ledge = if a1[3] >= 0.5 { ((luma(here) - luma(a1)) * 5.0).clamp(0.0, 1.0) } else { 0.0 };
    let patch = (0.5 + 0.5 * (37.0 * u + 23.0 * v + 3.1 * (13.0 * v).sin()).sin()).powf(0.7);
    // Laden: blotches of it lying all over, the more the deeper.
    let blob = smoothstep(0.5, 0.7, 0.65 * snow_noise(5.0 * u, 8.0 * v) + 0.35 * snow_noise(11.0 * u + 3.1, 17.0 * v + 1.7));
    let k = (dust * open.max(ledge) * (0.55 + 0.45 * patch) + dust * dust * (0.75 * blob + 0.2 * patch) * (1.0 - 0.5 * v)).min(1.0);
    mix3(lit, snow, k)
}

impl<'a> Card<'a> {
    fn new(ctx: &Ctx, b: &'a Billboard, crisp: bool) -> Option<Card<'a>> {
        let v = &ctx.view;
        let (w, h) = (v.width, v.height);
        let [x, y, d] = b.world;
        if let Some(pn) = b.panel { return Card::panel(ctx, b, pn, crisp); }
        if let Some(t) = b.tube { return Card::tube(ctx, b, t); }
        let base = v.to_cam(x, y, d);
        if base[2] < NEAR { return None; }
        let [sx, sy_base] = v.project(base)?;
        let [_, sy_top] = v.project(v.to_cam(x, y + b.height, d))?;
        let hp = sy_base - sy_top;
        if hp < 0.5 { return None; }
        let wp = v.px_per_m(base[2]) * b.height * b.sprite.aspect;
        // In the wind the card bends: rows shift sideways, by nothing at the base and most at the top.
        let sway_px = v.px_per_m(base[2]) * b.sway;
        // And its leaves stir a little past its edges.
        let stir = (b.wind.amp + b.wind.lean.abs()) * wp;
        let (x0, x1) = ((sx - wp * 0.5 + sway_px.min(0.0) - stir).floor().max(0.0) as i64, ((sx + wp * 0.5 + sway_px.max(0.0) + stir).ceil() as i64).min(w as i64 - 1));
        let (y0, y1) = (sy_top.floor().max(0.0) as i64, (sy_base.ceil() as i64).min(h as i64 - 1));
        if x0 > x1 || y0 > y1 { return None; }
        let light = ctx.light_except(x, y + b.height * 0.5, d, None, 0, weather::cloud_shade(ctx, x, d), b.own_light);
        // Lying snow settles on the tops of props: a few centimetres under every edge open above.
        let ps = ctx.wx.prop_snow;
        let snow = (ps > 0.05 && b.id == id::PROP && b.emissive < 0.3).then(|| {
            let cap = (v.px_per_m(base[2]) * 0.06 * ps).max(1.0) / hp;
            (cap, mul3([0.80, 0.83, 0.88], light), ctx.wx.prop_dust)
        });
        let light = mul3(light, b.tint);
        let z = base[2];
        let plain = ctx.front.is_none() && ctx.gain == 1.0;
        let (fog_mul, fog_add) = ctx.fog_affine(z);
        Some(Card {
            b, w, sx, wp, hp, sy_top, sway_px, x0, x1, y0, y1, light, snow, z, nearest: b.nearest || crisp,
            fog: (plain, ctx.fog_t(z), ctx.fog_col, fog_mul, fog_add), pan: None, tube: None,
        })
    }

    /// A cylinder (`Billboard::tube`): its axis in camera space, the pixels round it, and the
    /// light falling on it from each side.
    fn tube(ctx: &Ctx, b: &'a Billboard, t: Tube) -> Option<Card<'a>> {
        let v = &ctx.view;
        let (w, h) = (v.width, v.height);
        let (ca, cb) = (v.to_cam(t.a[0], t.a[1], t.a[2]), v.to_cam(t.b[0], t.b[1], t.b[2]));
        if ca[2] + t.r < NEAR && cb[2] + t.r < NEAR { return None; }
        let ax = [cb[0] - ca[0], cb[1] - ca[1], cb[2] - ca[2]];
        let len = dot3(ax, ax).sqrt();
        if len < 1e-4 { return None; }
        let u = scale3(ax, 1.0 / len);
        // Its pixel box: round its ends and their radius, or the whole frame when it reaches
        // behind the camera.
        let (x0, x1, y0, y1) = match (v.project(ca), v.project(cb)) {
            (Some(pa), Some(pb)) if ca[2] - t.r >= NEAR && cb[2] - t.r >= NEAR => {
                let (ra, rb) = (v.px_per_m(ca[2] - t.r) * t.r + 1.0, v.px_per_m(cb[2] - t.r) * t.r + 1.0);
                (((pa[0] - ra).min(pb[0] - rb).floor() as i64).max(0), ((pa[0] + ra).max(pb[0] + rb).ceil() as i64).min(w as i64 - 1),
                 ((pa[1] - ra).min(pb[1] - rb).floor() as i64).max(0), ((pa[1] + ra).max(pb[1] + rb).ceil() as i64).min(h as i64 - 1))
            }
            _ => (0, w as i64 - 1, 0, h as i64 - 1),
        };
        if x0 > x1 || y0 > y1 { return None; }
        let [mx, my, md] = b.world;
        let z = v.to_cam(mx, my, md)[2].max(NEAR);
        let hp = v.px_per_m(z) * 2.0 * t.r;
        if hp < 0.5 { return None; }
        let shade = weather::cloud_shade(ctx, mx, md);
        let mut cube = [[0.0f32; 3]; 6];
        for (i, e) in [[1.0f32, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [-1.0, 0.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, -1.0]].into_iter().enumerate() {
            cube[i] = mul3(ctx.light_except(mx, my, md, Some(v.unrot_dir(e)), 0, shade, b.own_light), b.tint);
        }
        let plain = ctx.front.is_none() && ctx.gain == 1.0;
        let (fog_mul, fog_add) = ctx.fog_affine(z);
        Some(Card {
            b, w, sx: 0.0, wp: 1.0, hp, sy_top: 0.0, sway_px: 0.0, x0, x1, y0, y1, light: cube[1], snow: None, z, nearest: false,
            fog: (plain, ctx.fog_t(z), ctx.fog_col, fog_mul, fog_add), pan: None,
            tube: Some(TubeCard { a: ca, u, len, r: t.r, bark: t.bark, end: t.end, cube, kf: 1.0 / v.focal_px, cx: v.center_px, cy: v.horizon_px }),
        })
    }

    /// A card standing in the world (`Billboard::panel`), seen in perspective.
    fn panel(ctx: &Ctx, b: &'a Billboard, pn: Panel, crisp: bool) -> Option<Card<'a>> {
        let v = &ctx.view;
        let (w, h) = (v.width, v.height);
        let (t, a, dn) = (pn.tl, pn.across, pn.down);
        let cam = |p: [f32; 3]| v.to_cam(p[0], p[1], p[2]);
        let corners = [cam(t), cam([t[0] + a[0], t[1] + a[1], t[2] + a[2]]), cam([t[0] + dn[0], t[1] + dn[1], t[2] + dn[2]]),
            cam([t[0] + a[0] + dn[0], t[1] + a[1] + dn[1], t[2] + a[2] + dn[2]])];
        if corners.iter().all(|c| c[2] < NEAR) { return None; }
        let c0 = corners[0];
        let cu = [corners[1][0] - c0[0], corners[1][1] - c0[1], corners[1][2] - c0[2]];
        let cv = [corners[2][0] - c0[0], corners[2][1] - c0[1], corners[2][2] - c0[2]];
        let (hm, q) = panel_map(v, c0, cu, cv)?;
        // Its pixel box: round its corners, or the whole frame when it passes behind the camera.
        let (x0, x1, y0, y1) = if corners.iter().all(|c| c[2] >= NEAR) {
            let ps: Vec<[f32; 2]> = corners.iter().filter_map(|c| v.project(*c)).collect();
            let (mut a, mut bb, mut c, mut e) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
            for p in &ps { a = a.min(p[0]); bb = bb.max(p[0]); c = c.min(p[1]); e = e.max(p[1]); }
            ((a.floor() as i64).max(0), (bb.ceil() as i64).min(w as i64 - 1), (c.floor() as i64).max(0), (e.ceil() as i64).min(h as i64 - 1))
        } else { (0, w as i64 - 1, 0, h as i64 - 1) };
        if x0 > x1 || y0 > y1 { return None; }
        let [mx, my, md] = [t[0] + 0.5 * (a[0] + dn[0]), t[1] + 0.5 * (a[1] + dn[1]), t[2] + 0.5 * (a[2] + dn[2])];
        let mid = v.to_cam(mx, my, md);
        let z = mid[2].max(NEAR);
        let full = v.px_per_m(z) * b.height;
        if full < 0.5 { return None; }
        // Its picture's detail as squeezed on screen: a face seen at a glancing angle (a roof
        // down the street) takes a coarser level, as its texels crowd into a few pixels.
        let hp = match (v.project(corners[0]), v.project(corners[1]), v.project(corners[2])) {
            (Some(p0), Some(p1), Some(p2)) if corners[..3].iter().all(|c| c[2] >= NEAR) => {
                // The picture's height in pixels along its most squeezed direction: the smaller
                // singular value of the map from the picture (in heights) to the screen.
                let k = 1.0 / b.sprite.aspect.max(1e-3);
                let (a, c) = ([(p1[0] - p0[0]) * k, (p1[1] - p0[1]) * k], [p2[0] - p0[0], p2[1] - p0[1]]);
                let sum = a[0] * a[0] + a[1] * a[1] + c[0] * c[0] + c[1] * c[1];
                let det = (a[0] * c[1] - a[1] * c[0]).abs();
                let big = (0.5 * (sum + (sum * sum - 4.0 * det * det).max(0.0).sqrt())).sqrt();
                (det / big.max(1e-6)).clamp(1.0, full.max(1.0))
            }
            _ => full,
        };
        // Lit as a face turned the way it faces, toward the camera (the underside of an eave in
        // shade, seen from below).
        let n = [a[1] * dn[2] - a[2] * dn[1], a[2] * dn[0] - a[0] * dn[2], a[0] * dn[1] - a[1] * dn[0]];
        let nl = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-9);
        let nc = v.rot_dir(n);
        let k = if nc[0] * mid[0] + nc[1] * mid[1] + nc[2] * mid[2] > 0.0 { -1.0 / nl } else { 1.0 / nl };
        let normal = [n[0] * k, n[1] * k, n[2] * k];
        let light = ctx.light_except(mx, my, md, Some(normal), 0, weather::cloud_shade(ctx, mx, md), b.own_light);
        let ps = ctx.wx.prop_snow;
        let snow = (ps > 0.05 && b.id == id::PROP && b.emissive < 0.3).then(|| {
            let cap = (v.px_per_m(z) * 0.06 * ps).max(1.0) / full;
            (cap, mul3([0.80, 0.83, 0.88], light), ctx.wx.prop_dust)
        });
        let light = mul3(light, b.tint);
        let plain = ctx.front.is_none() && ctx.gain == 1.0;
        let (fog_mul, fog_add) = ctx.fog_affine(z);
        Some(Card {
            b, w, sx: 0.0, wp: 1.0, hp, sy_top: 0.0, sway_px: 0.0, x0, x1, y0, y1, light, snow, z, nearest: b.nearest || crisp,
            fog: (plain, ctx.fog_t(z), ctx.fog_col, fog_mul, fog_add), pan: Some((hm, q, pn.strip.unwrap_or(-1.0))), tube: None,
        })
    }

    /// The card for the GPU's card pass (its sprite levels chosen here, as `draw_rows` samples them).
    #[cfg(feature = "gpu")]
    fn gpu(&self) -> super::gpu::CardIn {
        let b = self.b;
        let (plain, fog_t, fog, fog_mul, fog_add) = self.fog;
        let (snow_on, snow_cap, snow, snow_dust) = self.snow.map_or((0, 0.0, [0.0; 3], 0.0), |(c, s, d)| (1, c, s, d));
        let card = super::gpu::CardGpu {
            sx: self.sx, wp: self.wp, hp: self.hp, sy_top: self.sy_top, sway: self.sway_px, z: self.z,
            x0: self.x0 as i32, x1: self.x1 as i32, y0: self.y0 as i32, y1: self.y1 as i32,
            light: self.light, snow_on, snow_cap, snow, snow_dust,
            nearest: self.nearest as u32, flip: b.flip as u32, plain: plain as u32, fog_t, fog, fog_mul, fog_add,
            emissive: b.emissive, id: b.id as u32, realm: b.realm as u32, world: b.world, source: b.source as u32,
            w_amp: b.wind.amp, w_v: b.wind.v, w_lean: b.wind.lean, w_t1: b.wind.t1, w_t2: b.wind.t2,
            w_c0: b.wind.c0, w_c1: b.wind.c1, w_trunk: b.wind.trunk, w_foot: b.wind.foot, w_kv: b.wind.kv,
            pan: self.pan.is_some() as u32, ph: self.pan.map_or([0.0; 9], |p| p.0), pq: self.pan.map_or([0.0; 3], |p| p.1), strip: self.pan.map_or(-1.0, |p| p.2),
            fade: b.fade,
            ..Default::default()
        };
        let mut card = card;
        if let Some(t) = &self.tube {
            card.tube = 1;
            (card.ta, card.tu, card.tlen, card.trad, card.tbark, card.tend) = (t.a, t.u, t.len, t.r, t.bark, t.end);
            for (i, c) in t.cube.iter().enumerate() { card.cube[3 * i..3 * i + 3].copy_from_slice(c); }
            (card.tkf, card.tcx, card.tcy) = (t.kf, t.cx, t.cy);
        }
        super::gpu::CardIn {
            card, sprite: b.sprite.clone(), lod: b.sprite.lod(self.hp),
            glow: b.sprite.glow.as_ref().map(|g| (g.clone(), g.lod(self.hp))),
        }
    }

    /// Draw the card's rows that fall in a band starting at row `row0`; the slices hold that band.
    fn draw_rows(&self, row0: usize, gbuf: &mut [GPixel], hdr: &mut [[f32; 3]], mut pick: Option<&mut [u16]>) {
        let (b, w) = (self.b, self.w);
        let rows = (gbuf.len() / w) as i64;
        let (ya, yb) = (self.y0.max(row0 as i64), self.y1.min(row0 as i64 + rows - 1));
        let [x, y, d] = b.world;
        let (plain, fog_t, fog_c, fog_mul, fog_add) = self.fog;
        if let Some(t) = &self.tube {
            for py in ya..=yb {
                let fy = py as f32 + 0.5;
                for px in self.x0..=self.x1 {
                    if raster::dissolved(px, py, b.fade) { continue; }
                    let i = (py - row0 as i64) as usize * w + px as usize;
                    let Some((z, a, lit)) = tube_at(t, px as f32 + 0.5, fy) else { continue };
                    if z < NEAR || z >= gbuf[i].depth { continue; }
                    let c = if plain { mix3(fog_c, lit, fog_t) } else { add3(scale3(lit, fog_mul), fog_add) };
                    hdr[i] = mix3(hdr[i], c, a);
                    if a >= 0.5 {
                        gbuf[i] = GPixel { depth: z, id: b.id, x, y, d, realm: b.realm, pad: 0 };
                        if let Some(p) = pick.as_deref_mut() { p[i] = b.source; }
                    }
                }
            }
            return;
        }
        if let Some((hm, q, strip)) = self.pan {
            for py in ya..=yb {
                let fy = py as f32 + 0.5;
                for px in self.x0..=self.x1 {
                    if raster::dissolved(px, py, b.fade) { continue; }
                    let i = (py - row0 as i64) as usize * w + px as usize;
                    let fx = px as f32 + 0.5;
                    let iz = q[0] * fx + q[1] * fy + q[2];
                    if iz <= 0.0 { continue; }
                    let z = 1.0 / iz;
                    if z < NEAR || z >= gbuf[i].depth { continue; }
                    let wd = hm[6] * fx + hm[7] * fy + hm[8];
                    if wd.abs() < 1e-12 { continue; }
                    let mut u = (hm[0] * fx + hm[1] * fy + hm[2]) / wd;
                    let vv = (hm[3] * fx + hm[4] * fy + hm[5]) / wd;
                    if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&vv) { continue; }
                    if strip >= 0.0 { u = strip; } else if b.flip { u = 1.0 - u; }
                    let s = b.sprite.sample(u, vv, self.hp, self.nearest);
                    let a = if self.nearest { if s[3] >= 0.5 { 1.0 } else { 0.0 } } else { s[3] };
                    if a < 0.02 { continue; }
                    let glow = b.emissive + b.sprite.glow.as_ref().map_or(0.0, |g| g.sample(u, vv, self.hp, self.nearest)[0]);
                    let mut lit = add3(mul3([s[0], s[1], s[2]], self.light), scale3([s[0], s[1], s[2]], glow));
                    if let Some((cap, snow_lit, dust)) = self.snow {
                        if a >= 0.5 { lit = snow_on_card(&b.sprite, u, vv, self.hp, cap, dust, lit, snow_lit); }
                    }
                    let c = if plain { mix3(fog_c, lit, fog_t) } else { add3(scale3(lit, fog_mul), fog_add) };
                    hdr[i] = mix3(hdr[i], c, a);
                    if a >= 0.5 {
                        gbuf[i] = GPixel { depth: z, id: b.id, x, y, d, realm: b.realm, pad: 0 };
                        if let Some(p) = pick.as_deref_mut() { p[i] = b.source; }
                    }
                }
            }
            return;
        }
        let bend = |vv: f32| self.sway_px * (1.0 - vv) * (1.0 - vv);
        for py in ya..=yb {
            let vv = (py as f32 + 0.5 - self.sy_top) / self.hp;
            if !(0.0..1.0).contains(&vv) { continue; }
            for px in self.x0..=self.x1 {
                if raster::dissolved(px, py, b.fade) { continue; }
                let i = (py - row0 as i64) as usize * w + px as usize;
                if self.z >= gbuf[i].depth { continue; }
                let mut u = (px as f32 + 0.5 - (self.sx - self.wp * 0.5) - bend(vv)) / self.wp;
                let row = vv;
                let mut vv = vv;
                if b.wind.amp != 0.0 {
                    let (du, dv) = b.wind.at(u, row);
                    u -= du;
                    vv = row - dv;
                    if !(0.0..1.0).contains(&vv) { continue; }
                }
                if !(0.0..1.0).contains(&u) { continue; }
                if b.flip { u = 1.0 - u; }
                let s = b.sprite.sample(u, vv, self.hp, self.nearest);
                let a = if self.nearest { if s[3] >= 0.5 { 1.0 } else { 0.0 } } else { s[3] };
                if a < 0.02 { continue; }
                let glow = b.emissive + b.sprite.glow.as_ref().map_or(0.0, |g| g.sample(u, vv, self.hp, self.nearest)[0]);
                let mut lit = add3(mul3([s[0], s[1], s[2]], self.light), scale3([s[0], s[1], s[2]], glow));
                if let Some((cap, snow_lit, dust)) = self.snow {
                    if a >= 0.5 { lit = snow_on_card(&b.sprite, u, vv, self.hp, cap, dust, lit, snow_lit); }
                }
                let c = if plain { mix3(fog_c, lit, fog_t) } else { add3(scale3(lit, fog_mul), fog_add) };
                hdr[i] = mix3(hdr[i], c, a);
                if a >= 0.5 {
                    gbuf[i] = GPixel { depth: self.z, id: b.id, x, y, d, realm: b.realm, pad: 0 };
                    if let Some(p) = pick.as_deref_mut() { p[i] = b.source; }
                }
            }
        }
    }
}

fn draw_flame(ctx: &Ctx, f: &Flame, out: &mut Vec<Splat>) {
    let v = &ctx.view;
    let c = v.to_cam(f.world[0], f.world[1], f.world[2]);
    if c[2] < NEAR { return; }
    let Some([sx, sy]) = v.project(c) else { return };
    let ppm = v.px_per_m(c[2]);
    let fade = ctx.fog_t(c[2]) * ctx.gain;
    // Halo.
    let gr = (ppm * 1.2 * f.size).max(1.5);
    let glow = scale3(f.glow, 0.8 * fade * f.flicker);
    out.push(Splat { z: c[2], gate: None, shape: Shape::Halo { sx, sy, gr, glow } });
    if f.halo_only { return; }
    // Flame body, or a bright orb.
    let fh = (ppm * 0.16 * f.size * f.flicker).max(1.0);
    let Some(cols) = f.colors else {
        let r = (ppm * 0.05 * f.size).max(0.8);
        out.push(Splat { z: c[2], gate: None, shape: Shape::Orb { sx, sy, r, glow: f.glow, fade } });
        return;
    };
    let lin = cols.map(rgb_lin);
    let (rx, ry) = (fh * 0.55, fh * 1.25);
    let cy = sy - ry * 0.55;
    out.push(Splat { z: c[2], gate: None, shape: Shape::Body { sx, cy, rx, ry, lin, fade, ph: f.phase } });
    // Embers lifting off it and fading as they rise, each round a whole number of times a loop.
    let loop_seconds = ctx.scene.motion.loop_seconds();
    for i in 0..f.embers as i64 {
        let s = f.seed;
        let cyc = cycles(0.45 + 0.5 * hf(s ^ 0xE5, i), loop_seconds);
        let p = (cyc * ctx.tphase + hf(s ^ 0xE6, i)).rem_euclid(1.0);
        let wob = (TAU * (2.0 * p + hf(s ^ 0xE7, i))).sin();
        let ex = sx + fh * ((hf(s ^ 0xE8, i) - 0.5) * 0.7 + 0.35 * wob * p);
        let ey = sy - fh * (0.9 + 4.5 * p);
        let k = (1.0 - p) * (1.0 - p);
        if k < 0.02 { continue; }
        let r = (fh * 0.045).max(0.55);
        out.push(Splat { z: c[2] - 0.01, gate: None, shape: Shape::Dot { sx: ex, sy: ey, r, col: scale3(lin[1], 3.0 * fade), alpha: 0.9 * k } });
    }
}

/// Waterfalls' water. Over the lip it is a smooth, glassy band; falling, the curtain breaks
/// into ribbons of white aerated water (thick in the middle, thin and see-through at the edges)
/// down which clumps of water race, quickening as they fall; at the foot it churns white in a
/// pool, and a plume of spray billows up and drifts, with a rainbow in it when the sun is out.
fn draw_falls(ctx: &Ctx, out: &mut Vec<Splat>) {
    let v = &ctx.view;
    let (sw, sh) = (v.width as i64, v.height as i64);
    let loop_seconds = ctx.scene.motion.loop_seconds();
    let sun = ctx.scene.sky.enabled && ctx.scene.sky.sun.enabled && ctx.scene.sky.sun.pos[1] < 1.0;
    // A soft puff (`Wisp`) of `r` pixels, flattened by `squash`.
    let puff = |out: &mut Vec<Splat>, c: [f32; 3], r: f32, squash: f32, lit: [f32; 3], alpha: f32| {
        if c[2] < NEAR || alpha < 0.005 { return; }
        let Some([sx, sy]) = v.project(c) else { return };
        let (rx, ry) = (r.max(1.0), (r * squash).max(1.0));
        let (x0, x1) = (((sx - rx).floor() as i64).max(0), ((sx + rx).ceil() as i64).min(sw - 1));
        let (y0, y1) = (((sy - ry).floor() as i64).max(0), ((sy + ry).ceil() as i64).min(sh - 1));
        if x0 > x1 || y0 > y1 { return; }
        out.push(Splat { z: c[2], gate: None, shape: Shape::Wisp { sx, sy, rx, ry, x0, x1, y0, y1, lit, alpha } });
    };
    for (at, width, wi) in &ctx.falls {
        let wf = &ctx.scene.waterfalls[*wi];
        let (x0, d) = ctx.view.toward_camera(at[0], at[2], 0.05);
        let top = at[1];
        let width = *width;
        let lit = add3(ctx.light_at(x0, 0.5 * top, d, None, 0, 1.0), [0.1; 3]);
        // Deep water is the water's colour; broken into foam it is nearly white.
        let dz = v.depth_of(x0, d);
        let water = ctx.apply_fog(mul3(rgb_lin(wf.water), lit), dz);
        let white = ctx.apply_fog(mul3(mix3(rgb_lin(wf.water), [1.0; 3], 0.7), lit), dz);
        let glass = ctx.apply_fog(mul3(scale3(rgb_lin(wf.water), 0.55), lit), dz);
        let ppm = v.px_per_m(dz.max(NEAR));
        // A point `m` metres in front of the fall (toward the camera, whichever way it looks).
        let near = |x: f32, y: f32, m: f32| { let (x, d) = v.toward_camera(x, d, m); v.to_cam(x, y, d) };
        let h = |k: u32, i: i64| hf(wf.seed ^ k, i);
        // The curtain, water drawn pixel by pixel (splat::fall_at): it speeds up as it falls
        // (from 1 m/s over the lip), so its noise is carried down in falling time, a whole
        // number of cells a loop so the loop closes.
        let g = 9.81f32;
        let tau = ((1.0 + 2.0 * g * top).sqrt() - 1.0) / g;
        let n_of = |cells: f32| ((cells * loop_seconds / tau.max(0.05)).round().clamp(1.0, 65535.0)) as u32;
        let (n1, n2) = (n_of(7.0), n_of(15.0));
        let ky = [n1 as f32 * tau / loop_seconds, n2 as f32 * tau / loop_seconds];
        let (ct, cf) = (near(x0, top, -0.02), near(x0, 0.0, -0.02));
        if ct[2] >= NEAR {
            if let (Some(pt), Some(pf)) = (v.project(ct), v.project(cf)) {
                let k = splat::FallK {
                    seed: wf.seed.wrapping_mul(0x2545_F491) ^ 0xFA11, ky, n: [n1, n2], t: ctx.tphase.rem_euclid(1.0),
                    deep: scale3(glass, 1.1), foam: scale3(white, 1.12), strength: 1.0, kk: 2.0 * g * top,
                };
                let hw = 0.5 * width * ppm;
                out.push(Splat { z: ct[2], gate: None, shape: Shape::Fall { cx: pt[0], top: pt[1], foot: pf[1], hw_top: hw, hw_foot: hw * 1.18, k } });
            }
        }
        // Drops flung from its frayed edges low down, longer the faster they fall.
        let cyc = cycles(1.3, loop_seconds).max(1.0);
        for i in 0..((width * 4.0).round() as i64).clamp(4, 20) {
            let p = (cyc * ctx.tphase + h(0xD1, i)).rem_euclid(1.0);
            let side = if h(0xD2, i) < 0.5 { -1.0 } else { 1.0 };
            let y = top * (0.55 - 0.55 * p);
            let x = x0 + side * width * (0.5 + 0.1 * h(0xD3, i) + 0.12 * p);
            let len = 0.08 + 0.35 * p;
            let (c0, c1) = (near(x, y + len, 0.05), near(x, y, 0.05));
            if c0[2] < NEAR { continue; }
            let (Some(a0), Some(a1)) = (v.project(c0), v.project(c1)) else { continue };
            splat::streak(out, c0[2], (a0[0], a0[1]), (a1[0], a1[1]), white, 0.25 * (p / 0.1).min(1.0), (0.015 * ppm).max(0.5));
        }
        // A splash crown where it strikes: drops thrown up and out, falling back.
        let crown = cycles(1.1, loop_seconds).max(1.0);
        for i in 0..((width * 14.0).round() as i64).clamp(10, 70) {
            let p = (crown * ctx.tphase + h(0xB1, i)).rem_euclid(1.0);
            let (vx, vy) = ((h(0xB2, i) - 0.5) * 2.2, 1.0 + 1.6 * h(0xB3, i));
            let x = x0 + (h(0xB4, i) - 0.5) * width + vx * p;
            let y = 0.1 + vy * p - 2.4 * p * p;
            if y < 0.0 { continue; }
            puff(out, near(x, y, 0.3 + 0.4 * h(0xB5, i)), ppm * (0.05 + 0.06 * h(0xB6, i)) * (1.0 - 0.5 * p), 1.0, scale3(white, 1.2), 0.75 * (std::f32::consts::PI * p).sin());
        }
        // The foot: the pool, white water churning where the fall strikes it, and the plume of
        // spray billowing up and drifting off.
        let spray = wf.spray.clamp(0.0, 1.0);
        puff(out, near(x0, 0.02, 0.7), ppm * width * 1.25, 0.24, mix3(water, white, 0.3), 0.8);
        let n_churn = ((width * 10.0).round() as i64).clamp(8, 60);
        let pc = cycles(0.8, loop_seconds).max(1.0);
        for i in 0..n_churn {
            let p = (pc * ctx.tphase + h(0xC1, i)).rem_euclid(1.0);
            let x = x0 + (h(0xC2, i) - 0.5) * width * 1.25;
            let y = 0.04 + 0.5 * (p * (1.0 - p)) * 4.0 * h(0xC3, i);
            puff(out, near(x, y, 0.2 + 0.3 * h(0xC5, i)), ppm * width * (0.14 + 0.2 * h(0xC4, i)), 0.75, scale3(white, 1.2), 0.85 * (1.0 - p) * (p / 0.12).min(1.0));
        }
        // Where the curtain strikes the pool, a band of white water.
        if let (Some(l), Some(r)) = (v.project(near(x0 - 0.62 * width, 0.12, 0.2)), v.project(near(x0 + 0.62 * width, 0.12, 0.2))) {
            let z = near(x0, 0.12, 0.2)[2];
            if z >= NEAR { splat::streak(out, z, (l[0], l[1]), (r[0], r[1]), scale3(white, 1.15), 0.7, (0.28 * ppm).max(1.0)); }
        }
        let n_puff = (30.0 * spray).round() as i64;
        let sc = cycles(0.35, loop_seconds).max(1.0);
        for i in 0..n_puff {
            let p = (sc * ctx.tphase + h(0xF5, i)).rem_euclid(1.0);
            let drift = ctx.wx.wind * 0.05 * p * top;
            let x = x0 + (h(0xF6, i) - 0.5) * width * (1.0 + 1.8 * p) + drift;
            let y = 0.2 + top * 0.6 * p * (0.4 + 0.6 * h(0xF7, i));
            let c = near(x, y, 0.4 + 0.9 * p);
            let r = v.px_per_m(c[2].max(NEAR)) * width * (0.3 + 0.6 * p);
            puff(out, c, r, 0.8, ctx.apply_fog(mul3([0.93, 0.96, 1.0], lit), c[2]), 0.3 * spray * (1.0 - p) * (p / 0.1).min(1.0));
        }
        // A haze of fine mist over the lower part of the fall.
        puff(out, near(x0, 0.3 * top, 0.5), ppm * width * 1.6, 0.9, ctx.apply_fog(mul3([0.9, 0.93, 0.97], lit), dz), 0.12 * spray);
        // A rainbow in the spray, low over the foot.
        if sun && wf.rainbow > 0.0 {
            let rad = width * 0.9;
            for b in 0..5 {
                let t = b as f32 / 4.0;
                let spec = [smoothstep(0.45, 0.85, t) + 0.35 * (1.0 - smoothstep(0.0, 0.15, t)), smoothstep(0.2, 0.5, t) * (1.0 - smoothstep(0.7, 0.9, t)), 1.0 - smoothstep(0.25, 0.55, t)];
                let rr = rad * (1.0 + 0.06 * t);
                for k in 0..24 {
                    // Only the part of the bow in front of the spray shows.
                    let f = k as f32 / 23.0;
                    let an = std::f32::consts::PI * (0.2 + 0.6 * f);
                    let fade = (std::f32::consts::PI * f).sin();
                    let (x, y) = (x0 + rr * an.cos(), 0.3 + rr * 0.8 * an.sin());
                    let c = near(x, y, 0.4);
                    if c[2] < NEAR { continue; }
                    let Some([sx, sy]) = v.project(c) else { continue };
                    splat::dot(out, c[2], (sx, sy), (v.px_per_m(c[2]) * rad * 0.06).max(0.8), scale3(spec, 1.4), 0.1 * fade * wf.rainbow.min(1.0));
                }
            }
        }
    }
}

/// White water where a waterway tumbles over a step of rock: a ridge of foam across the
/// channel, water pouring over it, and foam carried away downstream.
fn draw_cascades(ctx: &Ctx, out: &mut Vec<Splat>) {
    let ww = &ctx.scene.path.waterway;
    if !ww.enabled || ww.cascades <= 0.0 { return; }
    let v = &ctx.view;
    let (w, hgt) = (v.width as i64, v.height as i64);
    let loop_seconds = ctx.scene.motion.loop_seconds();
    let size = ww.cascades.min(1.0);
    let sp = snap_to_loop(ww.cascade_spacing.max(6.0), ctx.loop_len);
    let n = (ctx.loop_len / sp).round().max(1.0) as i64;
    // Downstream is towards the camera unless the current runs away.
    let dir = if ww.flow < 0 { 1.0 } else { -1.0 };
    let foam = rgb_lin(ww.foam_color);
    let offset = 0.37 * sp;
    let (lo, hi) = ctx.span(-3.0, ctx.far.min(120.0));
    let k_lo = ((ctx.scroll + lo - offset) / sp).ceil() as i64;
    let k_hi = ((ctx.scroll + hi - offset) / sp).floor() as i64;
    let cyc = cycles(1.1, loop_seconds).max(1.0);
    let fc = cycles(0.35, loop_seconds).max(1.0);
    for k in k_lo..=k_hi {
        let d = k as f32 * sp + offset - ctx.scroll;
        if !v.turned && d < NEAR { continue; }
        let seed = ww.seed ^ hash(0xCA5C, k.rem_euclid(n)) as u32;
        let hw = v.path_half_width(d).max(0.3);
        let ridge = 0.08 + 0.3 * size;
        let lit = add3(ctx.light_at(0.0, 0.2, d, None, 0, 1.0), [0.08; 3]);
        let col = ctx.apply_fog(mul3(foam, lit), v.depth_of(0.0, d).max(NEAR));
        let cols = ((hw * 2.0 * 18.0).round() as i64).clamp(12, 220);
        for i in 0..cols {
            let h = |q: u32| hf(seed ^ q, i);
            let u = (i as f32 + h(1)) / cols as f32;
            // The rock step is not straight across.
            let x = (2.0 * u - 1.0) * hw * 0.97;
            let bend = 0.6 * (u * 5.0 + hf(seed, 0) * 6.0).sin() * size;
            let dd = d + bend;
            let top = ridge * (0.6 + 0.6 * h(2)) * (1.0 - (2.0 * u - 1.0).powi(8));
            // The white water itself: overlapping soft puffs along the step make a band.
            {
                let c = v.to_cam(x, 0.5 * top, dd + dir * 0.15);
                if c[2] >= NEAR {
                    if let Some([sx, sy]) = v.project(c) {
                        let ppm = v.px_per_m(c[2]);
                        let rx = (ppm * 2.4 * hw / cols as f32).max(1.0);
                        let ry = (ppm * (0.6 * top + 0.04)).max(1.0);
                        let (x0, x1) = (((sx - rx).floor() as i64).max(0), ((sx + rx).ceil() as i64).min(w - 1));
                        let (y0, y1) = (((sy - ry).floor() as i64).max(0), ((sy + ry).ceil() as i64).min(hgt - 1));
                        if x0 <= x1 && y0 <= y1 {
                            let tumble = 0.75 + 0.25 * (TAU * (cyc * ctx.tphase + h(8))).sin();
                            out.push(Splat { z: c[2], gate: None, shape: Shape::Wisp { sx, sy, rx, ry, x0, x1, y0, y1, lit: scale3(col, 0.95), alpha: 0.5 * tumble } });
                        }
                    }
                }
            }
            // Water pouring over, here and there: a curve from the lip down to the pool below.
            if i % 2 == 1 {
                let p = (cyc * ctx.tphase + h(3)).rem_euclid(1.0);
                let (ya, yb) = (top * (1.0 - p * p), top * (1.0 - (p + 0.35).min(1.0).powi(2)));
                let (da, db) = (dd + dir * 0.5 * p, dd + dir * 0.5 * (p + 0.35).min(1.0));
                let (c0, c1) = (v.to_cam(x, ya, da), v.to_cam(x + (h(4) - 0.5) * 0.05, yb.max(0.0), db));
                if c0[2] < NEAR || c1[2] < NEAR { continue; }
                let (Some(a), Some(b)) = (v.project(c0), v.project(c1)) else { continue };
                let wpx = (v.px_per_m(c0[2]) * 0.022).max(0.6);
                splat::streak(out, c0[2], (a[0], a[1]), (b[0], b[1]), scale3(col, 1.15 + 0.3 * h(5)), 0.4, wpx);
            }
            // Foam tumbling at the foot, then carried off downstream.
            {
                let p = (fc * ctx.tphase + h(6)).rem_euclid(1.0);
                let (x, y, dz) = (x + (h(7) - 0.5) * 0.3, 0.03 + top * 0.5 * (1.0 - p), dd + dir * (0.4 + 3.5 * size * p));
                let c = v.to_cam(x, y, dz);
                if c[2] < NEAR { continue; }
                let Some([sx, sy]) = v.project(c) else { continue };
                let r = (v.px_per_m(c[2]) * (0.07 + 0.1 * size) * (0.6 + 0.9 * p)).max(1.0);
                let (rx, ry) = (r, r * 0.4);
                let (x0, x1) = (((sx - rx).floor() as i64).max(0), ((sx + rx).ceil() as i64).min(w - 1));
                let (y0, y1) = (((sy - ry).floor() as i64).max(0), ((sy + ry).ceil() as i64).min(hgt - 1));
                if x0 > x1 || y0 > y1 { continue; }
                out.push(Splat { z: c[2], gate: None, shape: Shape::Wisp { sx, sy, rx, ry, x0, x1, y0, y1, lit: col, alpha: 0.5 * (1.0 - p) * (p / 0.1).min(1.0) } });
            }
        }
    }
}

/// The wires of strings of lights: thin dark lines (dots close along them), lit a little by the
/// bulbs and the scene.
fn draw_wires(ctx: &Ctx, out: &mut Vec<Splat>) {
    let v = &ctx.view;
    for p in &ctx.wire_pts {
        let c = v.to_cam(p[0], p[1], p[2]);
        if c[2] < NEAR { continue; }
        let Some([sx, sy]) = v.project(c) else { continue };
        let ppm = v.px_per_m(c[2]);
        let r = (ppm * 0.008).max(0.35);
        let col = ctx.apply_fog(scale3(ctx.light_at(p[0], p[1], p[2], None, 0, 1.0), 0.04), c[2]);
        out.push(Splat { z: c[2], gate: None, shape: Shape::Dot { sx, sy, r, col, alpha: 0.75 } });
    }
}

/// Smoke rising from props: puffs leaving each one in turn, climbing, spreading and drifting with
/// the wind, thinning away as they go; each puff goes round a whole number of times a loop.
fn draw_smoke(ctx: &Ctx, out: &mut Vec<Splat>) {
    let v = &ctx.view;
    let (w, h) = (v.width as i64, v.height as i64);
    let loop_seconds = ctx.scene.motion.loop_seconds();
    const PUFFS: i64 = 9;
    for (at, sm, seed) in &ctx.smokes {
        let cyc = cycles(0.22, loop_seconds);
        let base_col = rgb_lin(sm.color);
        for i in 0..PUFFS {
            let p = (cyc * ctx.tphase + (i as f32 + hf(*seed, i)) / PUFFS as f32).rem_euclid(1.0);
            let rise = sm.rise.max(0.2) * p;
            let wob = 0.25 * sm.width * (TAU * (p * 1.5 + hf(*seed ^ 0x11, i))).sin();
            let drift = ctx.wx.wind * 0.12 * rise;
            let (x, y, d) = (at[0] + wob + drift, at[1] + rise, at[2] + 0.15 * rise);
            let c = v.to_cam(x, y, d);
            if c[2] < NEAR { continue; }
            let Some([sx, sy]) = v.project(c) else { continue };
            let ppm = v.px_per_m(c[2]);
            let r = (ppm * sm.width.max(0.05) * (0.25 + 0.75 * p)).max(1.0);
            let (rx, ry) = (r, 0.8 * r);
            let alpha = sm.amount.clamp(0.0, 1.0) * 0.4 * (1.0 - p) * (p / 0.08).min(1.0) * ctx.fog_t(c[2]);
            if alpha < 0.01 { continue; }
            let lit = mul3(base_col, ctx.light_at(x, y, d, None, 0, 1.0));
            let (x0, x1) = (((sx - rx).floor() as i64).max(0), ((sx + rx).ceil() as i64).min(w - 1));
            let (y0, y1) = (((sy - ry).floor() as i64).max(0), ((sy + ry).ceil() as i64).min(h - 1));
            if x0 > x1 || y0 > y1 { continue; }
            out.push(Splat { z: c[2], gate: None, shape: Shape::Wisp { sx, sy, rx, ry, x0, x1, y0, y1, lit, alpha } });
        }
    }
}

fn draw_tufts(ctx: &Ctx, out: &mut Vec<Splat>) {
    let v = &ctx.view;
    let w = v.width;
    let s = ctx.scene;
    let sp = snap_to_loop(0.22 / s.verge.tuft_density.clamp(0.1, 4.0), ctx.loop_len);
    let n = (ctx.loop_len / sp).round() as i64;
    let range = ctx.far.min(30.0);
    let (lo, hi) = ctx.span(0.3, range);
    let (lo, hi) = if v.turned { (lo.max(-range), hi.min(range)) } else { (lo, hi) };
    let k_lo = ((ctx.scroll + lo) / sp).ceil() as i64;
    let k_hi = ((ctx.scroll + hi) / sp).floor() as i64;
    let base = rgb_lin(s.verge.tuft_color);
    for k in (k_lo..=k_hi).rev() {
        let key = k.rem_euclid(n);
        let d = k as f32 * sp - ctx.scroll;
        for (si, sign) in [-1.0f32, 1.0].into_iter().enumerate() {
            let ikey = key * 2 + si as i64;
            if hf(0x7A, ikey) > 0.85 { continue; }
            if ctx.on_bridge(d) { continue; }
            let x = sign * (ctx.path_edge(d) + (hf(0x7B, ikey) - 0.3) * 0.18);
            if ctx.on_fork(x, d) || !ctx.owns(x, 0.0, d) { continue; }
            let c = v.to_cam(x, 0.0, d);
            if c[2] < NEAR { continue; }
            let Some([sx, sy]) = v.project(c) else { continue };
            let hp = v.px_per_m(c[2]) * s.verge.tuft_height * (0.6 + 0.8 * hf(0x7C, ikey));
            if hp < 0.8 || sy < 0.0 || sx < -hp || sx > w as f32 + hp { continue; }
            let light = ctx.light_at(x, 0.1, d, None, 0, 1.0);
            // Snow lies on the blades; the wind bends them.
            let col = ctx.apply_fog(mul3(mix3(base, [0.80, 0.83, 0.88], 0.6 * ctx.wx.snow), light), c[2]);
            let gust = ctx.wx.sway_at(hf(0x7D, ikey) * TAU) * 2.5;
            let blades = 3 + (hash(0x7D, ikey) % 3) as i64;
            for b in 0..blades {
                let lean = (hf(0x7E, ikey * 8 + b) - 0.5) * 0.9 + sign * 0.15 + gust;
                let bx = sx + (b as f32 - blades as f32 * 0.5) * hp * 0.12;
                let len = hp * (0.6 + 0.4 * hf(0x7F, ikey * 8 + b));
                let steps = len.ceil().max(1.0) as i64;
                out.push(Splat { z: c[2], gate: None, shape: Shape::Blade { bx, sy, lean, len, steps, col } });
            }
        }
    }
}

fn draw_particles(ctx: &Ctx, p: &Particles, loop_seconds: f32, out: &mut Vec<Splat>) {
    let v = &ctx.view;
    let s = ctx.scene;
    let lat = if s.walls.enabled { ctx.wall_x(5.0) } else { v.path_half_width(5.0) + 6.0 };
    let top = if s.ceiling.enabled { s.ceiling.height } else if s.walls.enabled && s.walls.height > 0.0 { s.walls.height.min(8.0) } else { 6.0 };
    let color = rgb_lin(p.color);
    let range = ctx.far.min(36.0);
    // (A turned camera sees them behind where it stands too.)
    let lo = if v.turned { ctx.d_lo.max(-range) } else { 0.0 };
    let hi = if v.turned { ctx.d_hi.min(range) } else { range };
    let copies = ((hi - lo) / ctx.loop_len).ceil() as i64 + 1;
    let speed = p.speed.max(0.0);
    let (rate, size_m, emissive, streak) = match p.kind {
        ParticleKind::Dust => (0.05, 0.012, false, false),
        ParticleKind::Embers => (0.35, 0.018, true, false),
        ParticleKind::Fireflies => (0.08, 0.025, true, false),
        ParticleKind::Rain => (1.6, 0.008, false, true),
        ParticleKind::Snow => (0.12, 0.022, false, false),
        ParticleKind::Leaves => (0.15, 0.045, false, false),
        ParticleKind::Ash => (0.06, 0.016, false, false),
        ParticleKind::Spores => (0.05, 0.016, true, false),
        ParticleKind::Sand => (0.5, 0.007, false, false),
        ParticleKind::Petals => (0.07, 0.028, false, false),
    };
    // Blown sand runs sideways, low over the ground, as short horizontal streaks, faster and
    // the wind's way in a wind.
    let sideways = p.kind == ParticleKind::Sand;
    let wind = ctx.wx.wind;
    let fall = if sideways { cycles(rate * speed.max(0.05) + wind.abs() / (2.0 * lat), loop_seconds) } else { cycles(rate * speed.max(0.05), loop_seconds) };
    let blow = if sideways && wind < 0.0 { -1.0 } else { 1.0 };
    for i in 0..p.count.min(4000) as i64 {
        let seed = p.seed;
        let x0 = (hf(seed ^ 0x101, i) * 2.0 - 1.0) * lat;
        let y0 = hf(seed ^ 0x202, i) * top;
        let d0 = hf(seed ^ 0x303, i) * ctx.loop_len;
        let ph = hf(seed ^ 0x404, i) * TAU;
        let wob = cycles(0.3, loop_seconds);
        let t = ctx.tphase;
        let (dx, y, glow) = match p.kind {
            ParticleKind::Dust | ParticleKind::Ash => (0.15 * (TAU * wob * t + ph).sin(), (y0 - if p.kind == ParticleKind::Ash { fall * t * top } else { 0.1 * (TAU * wob * t + ph).cos() }).rem_euclid(top), 1.0),
            ParticleKind::Embers | ParticleKind::Spores => (0.2 * (TAU * wob * t + ph).sin(), (y0 + fall * t * top).rem_euclid(top), 1.0 - (y0 + fall * t * top).rem_euclid(top) / top),
            ParticleKind::Fireflies => (0.5 * (TAU * wob * t + ph).sin(), (y0 * 0.5 + 0.4 + 0.3 * (TAU * wob * t + ph * 1.3).cos()).max(0.1), (TAU * cycles(0.7, loop_seconds) * t + ph).sin().max(0.0)),
            ParticleKind::Rain => (0.0, (y0 - fall * t * top).rem_euclid(top), 1.0),
            ParticleKind::Snow | ParticleKind::Leaves | ParticleKind::Petals => (0.35 * (TAU * wob * t + ph).sin(), (y0 - fall * t * top).rem_euclid(top), 1.0),
            ParticleKind::Sand => {
                // Across the whole width a whole number of times per loop, hugging the ground.
                let x = (x0 + lat + blow * fall * t * 2.0 * lat).rem_euclid(2.0 * lat) - lat;
                (x - x0, (y0 / top).powi(3) * 1.2 + 0.03 + 0.05 * (TAU * wob * t + ph).sin(), 1.0)
            }
        };
        // Carried by the wind: falling things drift for as long as they have been falling, rising
        // things for as long as they have been rising, and hovering things live a loop at a time,
        // fading in and out, so the loop still closes.
        let (mut dx, mut life) = (dx, 1.0f32);
        if wind != 0.0 && !sideways {
            let k = match p.kind { ParticleKind::Fireflies => 0.2, ParticleKind::Spores => 0.5, ParticleKind::Dust => 0.6, ParticleKind::Embers | ParticleKind::Ash => 0.8, _ => 1.0 };
            let period = loop_seconds / fall;
            match p.kind {
                ParticleKind::Rain | ParticleKind::Snow | ParticleKind::Leaves | ParticleKind::Ash | ParticleKind::Petals => dx += wind * k * (1.0 - y / top) * period,
                ParticleKind::Embers | ParticleKind::Spores => dx += wind * k * (y / top) * period,
                _ => {
                    let lp = (t + hf(seed ^ 0x505, i)).rem_euclid(1.0);
                    dx += wind * k * loop_seconds * 0.5 * (lp - 0.5);
                    life = smoothstep(0.0, 0.15, lp) * smoothstep(0.0, 0.15, 1.0 - lp);
                }
            }
        }
        // Petals turn as they fall, catching the light and showing their edges.
        if p.kind == ParticleKind::Petals { life *= 0.45 + 0.55 * (TAU * cycles(0.8, loop_seconds) * t + ph * 3.0).sin().abs(); }
        if life <= 0.0 { continue; }
        for cpy in 0..copies {
            let d = lo + (d0 - ctx.scroll - lo).rem_euclid(ctx.loop_len) + cpy as f32 * ctx.loop_len;
            if if v.turned { d > hi } else { d < 0.3 || d > range } { continue; }
            // Blown out of the band at one side, back in at the other.
            let x = if sideways { x0 + dx } else { (x0 + dx + lat).rem_euclid(2.0 * lat) - lat };
            if !ctx.owns(x, y, d) { continue; }
            let c = v.to_cam(x, y, d);
            if v.turned && c[2] < 0.3 { continue; }
            let Some([sx, sy]) = v.project(c) else { continue };
            let r = (v.px_per_m(c[2]) * size_m * p.size).max(0.5);
            let fade = ctx.fog_t(c[2]) * ctx.gain;
            // Dust-like motes only show where light catches them, so they are added, not painted.
            let additive = emissive || matches!(p.kind, ParticleKind::Dust | ParticleKind::Ash);
            let lit = if emissive { scale3(color, 2.5 * glow) } else { mul3(color, ctx.light_at(x, y, d, None, 0, 1.0)) };
            let lit = scale3(lit, fade);
            let (len_px, a) = if streak { (v.px_per_m(c[2]) * 0.35, 0.35) } else if sideways { (v.px_per_m(c[2]) * 0.2, 0.5) } else { (0.0, if emissive { 1.0 } else { 0.85 }) };
            let ri = r.ceil() as i64;
            let (dy_len, dx_len) = if sideways { (0, len_px as i64) } else { (len_px as i64, 0) };
            out.push(Splat { z: c[2], gate: None, shape: Shape::Mote { sx: sx as i64, sy: sy as i64, ri, dy_len, dx_len, r, sideways, streak, additive, emissive, lit, a, life } });
        }
    }
}

fn frame_stats(ctx: &Ctx, gbuf: &[GPixel], rgb: &[[f32; 3]], billboards: usize) -> FrameStats {
    const CLASSES: [&str; 11] = ["sky", "void", "chasm", "bridge", "path", "verge", "walls", "ceiling", "props", "fixtures", "grass"];
    let class = |g: &GPixel| -> usize {
        match g.id {
            id::NONE => if ctx.scene.sky.enabled { 0 } else { 1 },
            id::CHASM | id::CLIFF => 2,
            r if id::is_rail(r) => 3,
            id::GROUND | id::RISER => if !ctx.scene.verge.enabled || g.x.abs() < ctx.path_edge(g.d) || ctx.on_fork(g.x, g.d) { 4 } else { 5 },
            id::WALL_L | id::WALL_R => 6,
            id::CEILING => 7,
            id::PROP => 8,
            id::FIXTURE => 9,
            _ => 10,
        }
    };
    // Counted in rows side by side, then summed in order, so the totals do not depend on threads.
    const ROWS: usize = 16;
    let w = ctx.view.width.max(1);
    let parts: Vec<([f64; 11], [f64; 11], f64)> = gbuf.par_chunks(w * ROWS).zip(rgb.par_chunks(w * ROWS)).map(|(gs, cs)| {
        let (mut n, mut l, mut total) = ([0.0f64; 11], [0.0f64; 11], 0.0f64);
        for (g, c) in gs.iter().zip(cs) {
            let luma = (0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]) as f64;
            let k = class(g);
            n[k] += 1.0;
            l[k] += luma;
            total += luma;
        }
        (n, l, total)
    }).collect();
    let (mut n, mut l, mut total) = ([0.0f64; 11], [0.0f64; 11], 0.0f64);
    for (pn, pl, pt) in parts {
        for k in 0..11 { n[k] += pn[k]; l[k] += pl[k]; }
        total += pt;
    }
    let px = gbuf.len().max(1) as f64;
    let seen = || (0..11).filter(|&k| n[k] > 0.0);
    FrameStats {
        coverage: seen().map(|k| (CLASSES[k].to_string(), (n[k] / px) as f32)).collect(),
        luma: seen().map(|k| (CLASSES[k].to_string(), (l[k] / n[k]) as f32)).collect(),
        mean_luma: (total / px) as f32,
        lights: ctx.lights.len(),
        billboards,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preset(n: &str) -> Scene { crate::scene::presets::ALL.iter().find(|(m, _)| *m == n).unwrap().1() }
    fn luma(img: &Image) -> f32 {
        img.rgba.chunks_exact(4).map(|p| 0.3 * p[0] as f32 + 0.55 * p[1] as f32 + 0.15 * p[2] as f32).sum::<f32>() / (img.rgba.len() / 4) as f32
    }

    /// Lights scattered along the path, in clusters, at every height, with the right-hand row out of
    /// step, and props shifted and leaning: all of it comes round with the loop, thins out when
    /// asked, and leaves an even row alone.
    #[test]
    fn scattered_lights_and_props_keep_the_loop() {
        let mut s = preset("Forest Path");
        s.particles.clear();
        s.weather = Weather::default();
        s.fixtures = vec![Fixture {
            kind: FixtureKind::Firefly, mount: Mount::Floating, side: Side::Both, spacing: 4.0, jitter: 1.0, jitter_along: 1.0, height_var: 0.8,
            size_var: 0.5, intensity_var: 0.5, color_var: 0.5, density: 0.8, cluster: 3, stagger: 0.5, ..Fixture::default()
        }];
        for p in &mut s.props { p.jitter_along = 1.0; p.stagger = 0.5; p.lean_var = 0.1; }
        let secs = s.motion.loop_seconds();
        let shot = |s: &Scene, t: f32| WorldRenderer::default().render(s, 3.0, &RenderOptions { size: Some((90, 160)), time: Some(t), ..RenderOptions::default() });
        let (a, b, mid) = (shot(&s, 0.0), shot(&s, secs), shot(&s, 0.4 * secs));
        let apart = |x: &Image, y: &Image| x.rgba.iter().zip(&y.rgba).map(|(p, q)| (*p as f32 - *q as f32).abs()).sum::<f32>() / x.rgba.len() as f32;
        assert!(apart(&a, &b) < 0.05, "a loop later the frame is {} away", apart(&a, &b));
        assert!(apart(&a, &mid) > 1.0);
        // The swarm lights the scene; with no place filled it is dark again.
        let mut none = s.clone();
        none.fixtures[0].density = 0.0;
        assert!(luma(&a) > luma(&shot(&none, 0.0)) + 0.3, "{} lit, {} unlit", luma(&a), luma(&shot(&none, 0.0)));
        // One to a place with nothing random set is the even row it always was.
        let mut even = preset("Stone Dungeon");
        let before = shot(&even, 0.0);
        for f in &mut even.fixtures { f.cluster = 1; f.density = 1.0; f.jitter_along = 0.0; f.stagger = 0.0; }
        assert!(shot(&even, 0.0).rgba == before.rgba);
    }

    /// Set pieces (arches, gates, portals across the path) show from the side too: every preset
    /// that has one shows it somewhere along the loop to a camera turned to look across the path.
    #[test]
    fn set_pieces_show_from_the_side() {
        let mut r = WorldRenderer::default();
        for (name, mk) in crate::scene::presets::ALL.iter() {
            let mut s = mk();
            if !s.set_pieces.iter().any(|p| p.enabled) { continue; }
            s.camera.yaw = -90.0;
            s.camera.side = 5.0;
            s.camera.cutaway = 0.4;
            let len = s.motion.loop_length.max(1.0);
            let seen = (0..8).any(|k| {
                let img = r.render(&s, len * k as f32 / 8.0, &RenderOptions { size: Some((192, 108)), pick: true, ..RenderOptions::default() });
                img.pick.is_some_and(|p| p.ids.iter().any(|&i| i >> 12 == PICK_SET_PIECES >> 12))
            });
            assert!(seen, "{name}: its set piece never shows from the side");
        }
    }

    /// A waterfall seen from the side is as big as it is there, with its spray: the sky over it
    /// stays the sky, not hidden behind spray drawn as if the fall stood where the camera does.
    #[test]
    fn a_waterfall_seen_from_the_side_keeps_its_size() {
        let mut s = preset("Waterfall Glen");
        s.camera.yaw = -90.0;
        s.camera.side = 5.0;
        s.camera.eye_height = 2.0;
        s.camera.zoom = 1.3;
        s.camera.horizon = 0.5;
        let (w, h) = (192usize, 108usize);
        let mut r = WorldRenderer::default();
        let len = s.motion.loop_length.max(1.0);
        for k in 0..12 {
            let img = r.render(&s, len * k as f32 / 12.0, &RenderOptions { size: Some((w as u32, h as u32)), ..RenderOptions::default() });
            // The top of the frame is sky: blue over the glen, whatever stands below it.
            let (mut red, mut blue) = (0.0f32, 0.0f32);
            for i in 0..w * h / 8 { red += img.rgba[i * 4] as f32; blue += img.rgba[i * 4 + 2] as f32; }
            let n = (w * h / 8) as f32;
            assert!(blue / n > red / n + 15.0, "frame {k}: the top of the frame is {} red, {} blue", red / n, blue / n);
        }
    }

    /// A camera turned to look across the path sees the lamps behind where it stands as well as
    /// those ahead: each draws, with its glow, wherever it is in the frame.
    #[test]
    fn a_turned_camera_sees_the_lamps_behind_it() {
        let mut s = preset("Night Road");
        s.particles.clear();
        s.weather = Weather::default();
        s.camera.yaw = -90.0;
        s.camera.side = 5.0;
        let (w, h) = (320usize, 180usize);
        let shot = |s: &Scene| WorldRenderer::default().render(s, 0.0, &RenderOptions { size: Some((w as u32, h as u32)), time: Some(0.0), ..RenderOptions::default() });
        let mut dark = s.clone();
        dark.fixtures.clear();
        let (lit, unlit) = (shot(&s), shot(&dark));
        // The brightest the lamps make each third of the frame, above the ground.
        let thirds: Vec<f32> = (0..3).map(|k| {
            let mut most = 0.0f32;
            for y in 0..h / 2 { for x in k * w / 3..(k + 1) * w / 3 {
                let i = (y * w + x) * 4;
                let l = |im: &Image| (im.rgba[i] as f32 + im.rgba[i + 1] as f32 + im.rgba[i + 2] as f32) / 3.0;
                most = most.max(l(&lit) - l(&unlit));
            } }
            most
        }).collect();
        assert!(thirds.iter().all(|&m| m > 60.0), "the lamps' glow in each third of the frame: {thirds:?}");
    }

    /// Lights told to stand only on bridges, or with a set piece, stand there and nowhere else; every
    /// kind draws, and one that pulses comes round with the loop.
    #[test]
    fn lights_keep_to_their_places_and_every_kind_draws() {
        let shot = |s: &Scene, t: f32| WorldRenderer::default().render(s, 3.0, &RenderOptions { size: Some((90, 160)), time: Some(t), ..RenderOptions::default() });
        let mut s = preset("Night Road");
        s.particles.clear();
        s.weather = Weather::default();
        let lamp = Fixture { mount: Mount::Ground, height: 2.0, spacing: 6.0, lateral: 0.3, ..Fixture::default() };
        let mut dark = s.clone();
        dark.fixtures.clear();
        let dark_shot = shot(&dark, 0.0);
        // No bridge, no set piece: nowhere to stand.
        for place in [LightPlace::Bridges, LightPlace::SetPieces] {
            s.fixtures = vec![Fixture { place, ..lamp.clone() }];
            assert!(shot(&s, 0.0).rgba == dark_shot.rgba, "{place:?}");
        }
        // With an arch every 12 m the lights come with it, however their own spacing is set.
        s.set_pieces = vec![SetPiece { kind: SetPieceKind::Archway, spacing: 12.0, offset: 5.0, ..SetPiece::default() }];
        dark.set_pieces = s.set_pieces.clone();
        s.fixtures = vec![Fixture { place: LightPlace::SetPieces, spacing: 1.7, ..lamp.clone() }];
        let tied = shot(&s, 0.0);
        s.fixtures[0].spacing = 12.0;
        s.fixtures[0].offset = 5.0;
        s.fixtures[0].place = LightPlace::Anywhere;
        assert!(luma(&tied) > luma(&shot(&dark, 0.0)) + 0.2);
        assert!(tied.rgba == shot(&s, 0.0).rgba);
        // On a bridge: lit there, and only there.
        let mut b = preset("Mountain Pass");
        b.particles.clear();
        b.weather = Weather::default();
        b.sky = preset("Night Road").sky;
        b.light = preset("Night Road").light;
        let secs = b.motion.loop_seconds();
        let (on, off) = (0.3 * secs, 0.8 * secs);
        let mut none = b.clone();
        none.fixtures.clear();
        assert!(luma(&shot(&b, on)) > luma(&shot(&none, on)) + 0.2, "lit on the bridge");
        b.fixtures[0].place = LightPlace::OffBridges;
        assert!(luma(&shot(&b, off)) > luma(&shot(&none, off)) + 0.2, "lit off it");
        // Every kind on every post, and the loop closes on them all (a beacon's pulse included).
        s.set_pieces.clear();
        s.fixtures = FixtureKind::ALL.iter().enumerate().map(|(i, &kind)| Fixture { kind, post: LampPost::ALL[i % 5], offset: i as f32 * 0.7, side: Side::Both, ..lamp.clone() }).collect();
        let secs = s.motion.loop_seconds();
        let (a, z) = (shot(&s, 0.0), shot(&s, secs));
        let apart = a.rgba.iter().zip(&z.rgba).map(|(p, q)| (*p as f32 - *q as f32).abs()).sum::<f32>() / a.rgba.len() as f32;
        assert!(apart < 0.05, "a loop later the frame is {apart} away");
        assert!(luma(&a) > luma(&dark_shot) + 1.0);
    }

    /// Props sent round the path on a ring wind and turn with the wormhole, come back to where they
    /// started when the loop does, and the new kinds of space prop all paint.
    #[test]
    fn orbiting_props_turn_with_the_tunnel_and_keep_the_loop() {
        let shot = |s: &Scene, t: f32| WorldRenderer::default().render(s, 3.0, &RenderOptions { size: Some((90, 160)), time: Some(t), ..RenderOptions::default() });
        let apart = |x: &Image, y: &Image| x.rgba.iter().zip(&y.rgba).map(|(p, q)| (*p as f32 - *q as f32).abs()).sum::<f32>() / x.rgba.len() as f32;
        let mut s = preset("Wormhole");
        assert!(s.props.iter().all(|p| p.orbit.enabled && p.orbit.with_tunnel));
        let secs = s.motion.loop_seconds();
        let (a, z) = (shot(&s, 0.0), shot(&s, secs));
        assert!(apart(&a, &z) < 0.05, "a loop later the frame is {} away", apart(&a, &z));
        // Without them the throat is empty; with the ring stopped they stand elsewhere mid-loop.
        let mut bare = s.clone();
        bare.props.clear();
        assert!(apart(&a, &shot(&bare, 0.0)) > 0.5);
        let mid = shot(&s, 0.3 * secs);
        for p in &mut s.props { p.orbit.with_tunnel = false; p.orbit.spin = 0; }
        assert!(apart(&mid, &shot(&s, 0.3 * secs)) > 0.3);
        for kind in PropKind::ALL.into_iter().filter(|k| k.of_space()) {
            let sp = crate::world::sprites::paint_prop(kind, kind.default_tint(), 3);
            assert!(sp.aspect > 0.2 && sp.aspect < 5.0, "{kind:?}");
        }
    }

    /// As the camera turns with a winding path the sky turns with it: the moon stands as far
    /// from its place as the far end of the path does, the frame comes round with the loop, and
    /// the sky over a straight path stays where the scene puts it.
    #[test]
    fn the_sky_turns_with_a_winding_path_and_keeps_the_loop() {
        let (w, h) = (120usize, 214usize);
        let shot = |s: &Scene, d: f32| WorldRenderer::default().render(s, d, &RenderOptions { size: Some((w as _, h as _)), ..RenderOptions::default() });
        let apart = |x: &Image, y: &Image| x.rgba.iter().zip(&y.rgba).map(|(p, q)| (*p as f32 - *q as f32).abs()).sum::<f32>() / x.rgba.len() as f32;
        // Where the moon is: the middle of the bright pixels of the upper sky.
        let moon_x = |im: &Image| {
            let (mut sx, mut n) = (0.0f32, 0.0f32);
            for y in 0..h * 3 / 10 { for x in 0..w {
                let i = (y * w + x) * 4;
                if im.rgba[i] as u32 + im.rgba[i + 1] as u32 + im.rgba[i + 2] as u32 > 560 { sx += x as f32 + 0.5; n += 1.0; }
            }}
            assert!(n > 4.0, "the moon should be in the frame ({n} bright pixels)");
            sx / n
        };
        let mut s = preset("Night Road");
        s.fixtures.clear();
        s.props.clear();
        (s.post.bloom, s.sky.stars.enabled, s.sky.clouds.enabled, s.path.bend) = (0.0, false, false, 0.0);
        let still = moon_x(&shot(&s, 0.0));
        assert!((still - moon_x(&shot(&s, 9.0))).abs() < 0.05, "a straight path's sky stays put");
        s.path.winding = crate::scene::Winding { enabled: true, amplitude: 0.5, length: 24.0, turn: 1.0, irregular: 0.4, phase: 0.1 };
        let len = s.motion.loop_length;
        let (mut least, mut most) = (0.0f32, 0.0f32);
        for d in [0.0, 0.2 * len, 0.45 * len, 0.7 * len] {
            let dx = View::new(&s, w, h).at(d).sky_dx();
            let seen = moon_x(&shot(&s, d)) - still;
            assert!((seen - dx).abs() < 1.0, "at {d} m the moon moved {seen} px, the far end of the path {dx}");
            (least, most) = (least.min(dx), most.max(dx));
        }
        assert!(most - least > 6.0, "the sky should swing across the frame ({least}..{most})");
        let (a, z) = (shot(&s, 0.0), shot(&s, len));
        assert!(apart(&a, &z) < 0.05, "a loop later the frame is {} away", apart(&a, &z));
        // The stars come round with it too, and a camera that does not turn leaves the sky alone.
        s.sky.stars.enabled = true;
        assert!(apart(&shot(&s, 0.0), &shot(&s, len)) < 0.05);
        s.path.winding.turn = 0.0;
        assert_eq!(View::new(&s, w, h).at(0.3 * len).sky_dx(), 0.0);
    }

    /// Bloom reaches a night sky: what outshines a dark sky glows (the moon, the brighter stars),
    /// though none of it is as bright as paper in daylight; a day sky, deep space and a tunnel
    /// keep the world's threshold, and a scene exposed up glows sooner.
    #[test]
    fn bloom_lights_a_night_sky_and_leaves_a_day_sky_as_it_was() {
        use crate::world::post::{sky_threshold, BLOOM_TH};
        for day in ["Forest Path", "Winding Trail", "Wormhole", "Star Bridge"] { assert_eq!(sky_threshold(&preset(day)), BLOOM_TH, "{day}"); }
        let (w, h) = (120usize, 214usize);
        let shot = |s: &Scene| WorldRenderer::default().render(s, 0.0, &RenderOptions { size: Some((w as _, h as _)), ..RenderOptions::default() });
        // The light of the upper sky.
        let sky = |im: &Image| (0..h * 3 / 10 * w).map(|i| im.rgba[i * 4] as f32 + im.rgba[i * 4 + 1] as f32 + im.rgba[i * 4 + 2] as f32).sum::<f32>() / (h * 3 / 10 * w) as f32;
        let mut s = preset("Night Road");
        assert!(sky_threshold(&s) < 0.3, "{}", sky_threshold(&s));
        s.fixtures.clear();
        s.sky.clouds.enabled = false;
        let lit = |s: &Scene, bloom: f32| { let mut s = s.clone(); s.post.bloom = bloom; sky(&shot(&s)) };
        let (off, on) = (lit(&s, 0.0), lit(&s, 1.0));
        assert!(on > off + 1.5, "bloom should light the night sky ({off} -> {on})");
        // The stars alone glow too.
        s.sky.moon.body.enabled = false;
        let (off, on) = (lit(&s, 0.0), lit(&s, 1.0));
        assert!(on > off + 0.3, "bloom should light the stars ({off} -> {on})");
        // Exposed down, less of the sky is bright enough to glow.
        s.sky.moon.body.enabled = true;
        s.sky.stars.enabled = false;
        let gain = |e: f32| { let mut s = s.clone(); s.post.exposure = e; lit(&s, 1.0) / lit(&s, 0.0) };
        assert!(gain(1.0) > gain(0.4), "{} {}", gain(1.0), gain(0.4));
    }

    /// An image with a painted mask: the masked part glows in the dark and lights the ground, the
    /// rest is lit by the scene; and the part marked as casting nothing leaves no shadow.
    #[test]
    fn a_painted_mask_makes_part_of_an_image_glow_light_and_cast_no_shadow() {
        use crate::world::sprites::{mask_path, Mask, MaskPart, Sprite};
        let dir = std::env::temp_dir().join(format!("pf_mask_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("lamp.png");
        // A grey post with a warm pane on top.
        let img = image::RgbaImage::from_fn(16, 48, |_, y| if y < 12 { image::Rgba([255, 190, 90, 255]) } else { image::Rgba([120, 120, 120, 255]) });
        img.save(&file).unwrap();
        assert!(Sprite::load(&file).unwrap().lamp.is_none());
        let mut m = Mask::solid(16, 48);
        for y in 0..12 { for x in 0..16 { m.px[y * 16 + x] = [255, 0]; } }
        m.save(&file).unwrap();
        let sp = Sprite::load(&file).unwrap();
        let lamp = sp.lamp.expect("a lamp from the mask");
        assert!(sp.casts && (lamp.v - 0.125).abs() < 0.01 && (lamp.u - 0.5).abs() < 0.01 && lamp.color[0] > lamp.color[2], "{lamp:?}");
        // By night: with the mask the scene round the prop is lit; without it, dark.
        let mut s = preset("Night Road");
        s.fixtures.clear();
        s.particles.clear();
        s.props = vec![PropLayer { sprite: SpriteRef { path: file.to_string_lossy().into_owned(), ..SpriteRef::default() }, spacing: 6.0, lateral: 0.4, scale: 0.5, jitter: 0.0, ..PropLayer::default() }];
        let shot = |s: &Scene| WorldRenderer::default().render(s, 3.0, &RenderOptions { size: Some((90, 160)), time: Some(0.0), ..RenderOptions::default() });
        let lit = luma(&shot(&s));
        std::fs::remove_file(mask_path(&file)).unwrap();
        let unlit = luma(&shot(&s));
        assert!(lit > unlit + 0.5, "{lit} with the mask, {unlit} without");
        // A mask that marks everything as casting nothing: by day the prop leaves no shadow.
        let mut d = preset("Desert Canyon");
        d.props = vec![PropLayer { sprite: SpriteRef { path: file.to_string_lossy().into_owned(), ..SpriteRef::default() }, spacing: 5.0, lateral: 0.3, jitter: 0.0, shadow_opacity: 1.0, ..PropLayer::default() }];
        d.particles.clear();
        let with_shadow = luma(&shot(&d));
        let mut clear = Mask::solid(16, 48);
        clear.paint(8.0, 24.0, 60.0, MaskPart::Clear);
        clear.save(&file).unwrap();
        assert!(luma(&shot(&d)) > with_shadow + 0.05, "no shadow is lighter than {with_shadow}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The moon's dark side shows the sky in front of it (it used to be painted as a near-black
    /// disc whatever the opacity), it hides the stars behind it, and phase follows its doc:
    /// 0 full, ±1 new, with no jump next to 0.
    #[test]
    fn the_moons_dark_side_is_sky_and_phase_is_continuous() {
        let (w, h) = (120usize, 214usize);
        let luma = |img: &Image, x: f32, y: f32| { let i = (y as usize * w + x as usize) * 4; 0.3 * img.rgba[i] as f32 + 0.55 * img.rgba[i + 1] as f32 + 0.15 * img.rgba[i + 2] as f32 };
        let opts = RenderOptions { size: Some((w as u32, h as u32)), time: Some(0.0), layers: Layers { particles: false, post: false, ..Layers::default() }, ..RenderOptions::default() };
        let mut r = WorldRenderer::default();
        let with_moon = |name: &str, phase: f32| {
            let mut s = preset(name);
            s.sky.clouds.enabled = false;
            // The moon alone: the sun's aureole would put a gradient across it.
            s.sky.sun.enabled = false;
            s.weather = Weather::default();
            s.sky.moon.body.enabled = true;
            s.sky.moon.body.pos = [0.5, 0.3];
            s.sky.moon.body.radius = 0.12;
            s.sky.moon.phase = phase;
            s.sky.moon.opacity = 1.0;
            s
        };
        let s = with_moon("Desert Canyon", 0.5);
        let v = View::new(&s, w, h);
        let (cx, cy, rad) = (0.5 * w as f32, 0.3 * v.horizon_px, 0.12 * v.horizon_px);
        let img = r.render(&s, 0.0, &opts);
        // Waxing half: lit on the right, dark on the left.
        let (dark, lit, beside) = (luma(&img, cx - 0.5 * rad, cy), luma(&img, cx + 0.5 * rad, cy), luma(&img, cx - 1.3 * rad, cy));
        assert!(lit > dark + 20.0, "lit {lit} vs dark {dark}");
        assert!((dark - beside).abs() < 12.0, "the dark side ({dark}) should read as the sky beside it ({beside})");
        // Phase 0.02 is a nearly full moon (it used to draw nearly new).
        let full = r.render(&with_moon("Desert Canyon", 0.0), 0.0, &opts);
        let near = r.render(&with_moon("Desert Canyon", 0.02), 0.0, &opts);
        let d: f32 = full.rgba.iter().zip(&near.rgba).map(|(a, b)| (*a as f32 - *b as f32).abs()).sum::<f32>() / full.rgba.len() as f32;
        assert!(d < 0.5, "phase 0.02 differs from full by {d}");
        // At night no star shows through the dark side: its brightest pixel is no brighter than
        // the faint earthshine and halo across it.
        let s = with_moon("Night Road", 0.6);
        let img = r.render(&s, 0.0, &opts);
        let v = View::new(&s, w, h);
        let (cx, cy, rad) = (0.5 * w as f32, 0.3 * v.horizon_px, 0.12 * v.horizon_px);
        let mut samples = Vec::new();
        for y in (cy - 0.8 * rad) as i32..(cy + 0.8 * rad) as i32 { for x in (cx - 0.9 * rad) as i32..(cx - 0.3 * rad) as i32 { samples.push(luma(&img, x as f32, y as f32)); } }
        let (lo, hi) = samples.iter().fold((f32::MAX, 0.0f32), |(a, b), &l| (a.min(l), b.max(l)));
        assert!(hi - lo < 25.0, "a star shows through the dark side: luma {lo}..{hi}");
    }

    /// Clouds move at any drift, not only whole ones (0.4x used to round to 0 and
    /// stand still), and the loop still closes exactly.
    #[test]
    fn clouds_drift_at_fractional_speeds_and_the_loop_closes() {
        let sky_rows = |img: &Image| img.rgba[..img.rgba.len() * 2 / 5].to_vec();
        let diff = |a: &[u8], b: &[u8]| a.iter().zip(b).map(|(x, y)| (*x as f32 - *y as f32).abs()).sum::<f32>() / a.len() as f32;
        let mut r = WorldRenderer::default();
        for (drift, moves) in [(0.0, false), (0.4, true), (0.45, true), (1.5, true), (1.0, true), (2.0, true)] {
            let mut s = preset("Desert Canyon");
            s.sky.clouds = Clouds { enabled: true, count: 24, drift, opacity: 1.0, ..Clouds::default() };
            let secs = s.motion.loop_seconds();
            let opts = |t: f32| RenderOptions { size: Some((90, 160)), time: Some(t), layers: Layers { particles: false, post: false, ..Layers::default() }, ..RenderOptions::default() };
            let a = r.render(&s, 0.0, &opts(0.0));
            let b = r.render(&s, 0.0, &opts(secs * 0.3));
            let end = r.render(&s, 0.0, &opts(secs));
            let d = diff(&sky_rows(&a), &sky_rows(&b));
            assert_eq!(d > 0.05, moves, "drift {drift}: sky changed by {d} over 0.3 of the loop");
            assert_eq!(a.rgba, end.rgba, "drift {drift}: the loop must close");
        }
    }

    #[test]
    fn lightning_flashes_on_its_frame_and_the_loop_still_closes() {
        let mut s = preset("Night Road");
        s.weather.lightning.enabled = true;
        let times = lightning_times(&s);
        assert_eq!(times.len(), 2);
        let n = s.motion.frames() as f32;
        let (len, secs) = (s.motion.loop_length, s.motion.loop_seconds());
        let opts = |t: f32| RenderOptions { size: Some((120, 214)), time: Some(t), ..RenderOptions::default() };
        let mut r = WorldRenderer::default();
        for p in times {
            // Strikes land on export frames, so the peak is never skipped.
            assert!((p * n - (p * n).round()).abs() < 1e-3, "strike at {p} is between frames");
            let before = luma(&r.render(&s, len * (p - 2.0 / n), &opts(secs * (p - 2.0 / n))));
            let peak = luma(&r.render(&s, len * p, &opts(secs * p)));
            assert!(peak > before * 1.5, "flash {peak} vs {before}");
        }
        let a = r.render(&s, 0.0, &opts(0.0));
        let b = r.render(&s, len, &opts(secs));
        assert_eq!(a.rgba, b.rgba);
    }

    #[test]
    fn fog_banks_thicken_inside_and_repeat_with_the_loop() {
        let b = crate::scene::FogBanks { enabled: true, spacing: 12.0, length: 4.0, density: 0.8, offset: 0.0 };
        let t = |scroll: f32, z: f32| bank_transmittance(&b, 24.0, scroll, z);
        // Standing in clear air 2 m before a bank: the first 2 m are clear, then 4 m of bank.
        assert!((t(10.0, 2.0) - 1.0).abs() < 1e-6);
        assert!((t(10.0, 6.0) - (-0.8f32 * 4.0).exp()).abs() < 1e-5);
        // Two banks within 18 m: 8 m of fog in all.
        assert!((t(10.0, 18.0) - (-0.8f32 * 8.0).exp()).abs() < 1e-5);
        // Inside a bank, 1 m in: 3 m of it left ahead.
        assert!((t(13.0, 10.0) - (-0.8f32 * 3.0).exp()).abs() < 1e-5);
        // The same distance one loop later sees the same fog.
        for z in [0.5, 3.0, 9.0, 30.0] { assert!((t(10.0, z) - t(34.0, z)).abs() < 1e-5); }
    }

    #[test]
    fn stairs_climb_one_flight_per_period_and_the_view_repeats() {
        let st = crate::world::view::StairProfile::new(&crate::scene::Stairs { enabled: true, spacing: 12.0, steps: 8, rise: 0.17, run: 0.32, offset: 3.0, descending: false }, 24.0).unwrap();
        // A flight climbs steps * rise; the eye's ramp ends level with the top step.
        let flight = 8.0 * 0.17;
        assert!((st.ground(3.0 + 12.0 - 0.01) - st.ground(3.0 - 0.01) - flight).abs() < 1e-4);
        assert!((st.ramp(3.0 + 8.0 * 0.32) - st.ground(3.0 + 8.0 * 0.32)).abs() < 1e-4);
        assert_eq!(st.risers(0.0, 24.0).len(), 16);
        // The ground ahead, relative to the camera, is the same one period (and so one loop) later.
        let mut s = preset("Stone Dungeon");
        s.path.stairs.enabled = true;
        for scroll in [0.0, 1.3, 4.1, 7.77] {
            let (a, b) = (View::new(&s, 60, 107).at(scroll), View::new(&s, 60, 107).at(scroll + 12.0));
            for d in [0.5, 2.0, 3.3, 9.0, 30.0] { assert!((a.lift(d) - b.lift(d)).abs() < 1e-3, "lift at {d} from {scroll}"); }
        }
        let opts = RenderOptions { size: Some((120, 214)), time: Some(0.0), ..RenderOptions::default() };
        let mut r = WorldRenderer::default();
        assert_eq!(r.render(&s, 0.0, &opts).rgba, r.render(&s, 24.0, &RenderOptions { time: Some(s.motion.loop_seconds()), ..opts.clone() }).rgba);
    }

    #[test]
    fn stair_risers_leave_no_gaps_in_the_ground() {
        // Open ground with nothing behind it: a missing riser would show void between the steps.
        let mut s = preset("Forest Path");
        s.sky.enabled = false;
        s.props.clear();
        s.fixtures.clear();
        s.set_pieces.clear();
        s.light.fog.enabled = false;
        s.path.stairs = crate::scene::Stairs { enabled: true, ..Default::default() };
        let mut r = WorldRenderer::default();
        for k in 0..12 {
            let opts = RenderOptions { size: Some((90, 160)), layers: Layers { particles: false, post: false, ..Layers::default() }, ..RenderOptions::default() };
            let ctx_view = View::new(&s, 90, 160).at(k as f32 * 0.7);
            let ctx = Ctx {
                scene: &s, view: ctx_view, loop_len: 24.0, tphase: 0.0, scroll: k as f32 * 0.7, far: 320.0, d_lo: 0.0, d_hi: 320.0, x_reach: 0.0,
                path_tex: r.textures.get(&s.path.material), verge_tex: r.textures.get(&s.verge.material),
                wall_tex: r.textures.get(&s.walls.material), ceil_tex: r.textures.get(&s.ceiling.material),
                path_tile: 1.0, verge_tile: 1.0, wall_tile: 1.0, ceil_tile: 1.0,
                bridge: Spans::new(&s.path.bridge, 24.0, &s.fixtures), fork: Forks::new(&s.path.fork, s.path.edge_noise, 24.0), deck_tex: r.textures.get(&s.path.bridge.deck), bottom_tex: r.textures.get(&s.verge.material),
                deck_tile: 1.0, bottom_tile: 1.0, ambient: [0.0; 3],
                sky_lights: vec![], lights: vec![], smokes: vec![], wire_pts: vec![], falls: vec![], fog: None, fog_col: [0.0; 3], void_lin: [0.0; 3], realm: 0, bounds: None, facade: None, opening: None, portal: None, front: None, verge_beyond: None, gain: 1.0, wx: weather::Wx::default(), water: None, cover: Vec::new(),
            };
            let tris = build_geometry(&ctx, &opts.layers);
            let mut gbuf = vec![raster::GPixel::EMPTY; 90 * 160];
            raster::rasterize(&mut gbuf, 90, 160, &tris, &ctx.view);
            for x in [10usize, 45, 80] {
                let col: Vec<u8> = (0..160).map(|y| gbuf[y * 90 + x].id).collect();
                let top = col.iter().position(|&i| i != id::NONE).unwrap();
                assert!(col[top..].iter().all(|&i| i != id::NONE), "gap in column {x} at scroll {}: {:?}", k as f32 * 0.7, &col[top..]);
            }
        }
        let _ = &mut r;
    }

    #[test]
    fn branch_test_by_windows_matches_walking_every_junction() {
        for (angle, spacing, hw, side) in [(5.0f32, 4.0f32, 1.1f32, ForkSide::Alternate), (38.0, 24.0, 1.1, ForkSide::Left), (85.0, 9.0, 0.6, ForkSide::Right), (12.0, 6.0, 1.2, ForkSide::Alternate), (30.0, 8.0, 0.9, ForkSide::Both)] {
            let f = Forks::new(&Fork { enabled: true, spacing, angle, half_width: hw, side, ..Fork::default() }, 0.0, 24.0).unwrap();
            // The original: every junction within reach.
            let walk = |x: f32, w: f32, main_hw: f32| {
                let reach = 300.0 / f.tan;
                let norm = (1.0 + f.tan * f.tan).sqrt();
                f.near(w, reach).any(|(j, side)| {
                    let t = w - j;
                    if t < -f.hw || x * side <= 0.0 { return false; }
                    let x0 = side * main_hw * 0.5;
                    let dist = if t >= 0.0 { ((x - x0) - side * f.tan * t).abs() / norm } else { ((x - x0).powi(2) + t * t).sqrt() };
                    dist < f.hw
                })
            };
            let mut n = 0;
            for i in 0..40_000i64 {
                let x = (hf(1, i) - 0.5) * 120.0;
                let w = hf(2, i) * 200.0 - 50.0;
                let main_hw = 0.5 + hf(3, i) * 2.0;
                let (a, b) = (f.on_branch(x, w, main_hw), walk(x, w, main_hw));
                assert_eq!(a, b, "angle {angle} x {x} w {w} hw {main_hw}");
                n += a as usize;
            }
            assert!(n > 100, "angle {angle}: too few points on a branch to mean anything ({n})");
        }
    }

    #[test]
    fn split_forks_pave_one_road_then_part_round_a_grass_point() {
        for (angle, spacing, hw, side, noise) in [(38.0f32, 24.0f32, 1.1f32, ForkSide::Alternate, 0.15f32), (20.0, 24.0, 0.9, ForkSide::Both, 0.0), (70.0, 8.0, 0.6, ForkSide::Right, 0.3)] {
            let f = Forks::new(&Fork { enabled: true, spacing, angle, half_width: hw, side, style: ForkStyle::Split, ..Fork::default() }, noise, 24.0).unwrap();
            // The windowed search finds the same nearest branch as walking every junction.
            for i in 0..40_000i64 {
                let x = (hf(11, i) - 0.5) * 120.0;
                let w = hf(12, i) * 200.0 - 50.0;
                let main_hw = 0.5 + hf(13, i) * 2.0;
                let r = (main_hw + f.hw) / f.tan;
                let all = (-80..=80).fold(f32::INFINITY, |best, kk| f.branch_sd_at(x, w, kk, r, best));
                let a = f.branch_sd(x, w, main_hw);
                // Far from every branch the window may stop short of the nearest, which no one asks about.
                if all < f.hw + f.fillet() { assert_eq!(a, all, "angle {angle} x {x} w {w} hw {main_hw}"); }
            }
            let main_hw = 1.5;
            let j = f.offset + 3.0 * f.period;
            let side = f.side_of(3);
            let r = (main_hw + f.hw) / f.tan;
            // At the junction the road starts to widen (the rounded corner of the two together);
            // a bend's length on, the branch alone paves ground beyond the main road's edge.
            let x = side * (main_hw + 0.05);
            assert!(smin(x.abs() - main_hw, f.branch_sd(x, j + 0.5, main_hw), f.fillet()) < x.abs() - main_hw - 0.01, "angle {angle}: the road does not start to widen");
            assert!(f.branch_sd(side * (main_hw + 0.2 * f.hw), j + r, main_hw) < 0.0, "angle {angle}: the road does not widen");
            // Further on, between the two roads, a grass point: off both.
            let t = 4.0 * r + 4.0 * f.hw / f.tan;
            let xc = f.tan * (((t * t + r * r).sqrt()) - r);
            let gore = side * 0.5 * (main_hw + xc);
            assert!(xc - main_hw > 2.0 * f.hw + 2.0 * f.noise, "angle {angle}: test point not past the gore");
            assert!(f.branch_sd(gore, j + t, main_hw) > 0.0 && gore.abs() > main_hw + f.noise, "angle {angle}: no grass between the roads");
            // Every loop the same forks, noise and all.
            for i in 0..2_000i64 {
                let x = (hf(14, i) - 0.5) * 30.0;
                let w = hf(15, i) * 24.0;
                let (a, b) = (f.branch_sd(x, w, main_hw), f.branch_sd(x, w + 24.0, main_hw));
                assert!((a.min(5.0) - b.min(5.0)).abs() < 2e-3, "angle {angle}: loop seam at x {x} w {w}: {a} vs {b}");
            }
        }
    }

    #[test]
    fn column_blur_is_the_triangle_average_with_the_ends_repeated() {
        let (w, h) = (3usize, 37usize);
        let src: Vec<[f32; 3]> = (0..w * h).map(|i| [hf(7, i as i64), hf(8, i as i64) * 4.0, (i % 5) as f32]).collect();
        let b = ColumnBlur::new(&src, w, h);
        for x in 0..w {
            for r0 in [-30i64, -3, 0, 1, 17, 36, 40, 80] {
                for half in [0i64, 1, 2, 7, 25, 60] {
                    let mut acc = [0.0f64; 3];
                    for k in -half..=half {
                        let r = (r0 + k).clamp(0, h as i64 - 1) as usize;
                        for c in 0..3 { acc[c] += (half + 1 - k.abs()) as f64 * src[r * w + x][c] as f64; }
                    }
                    let want = acc.map(|v| v / ((half + 1) * (half + 1)) as f64);
                    let got = b.triangle(x, r0, half);
                    for c in 0..3 { assert!((got[c] as f64 - want[c]).abs() < 1e-4, "x {x} r0 {r0} half {half}: {got:?} vs {want:?}"); }
                }
            }
        }
    }

    #[test]
    fn every_railing_is_closed_and_water_sits_at_its_level() {
        let mut s = preset("Forest Path");
        s.sky.enabled = false;
        s.light.fog.enabled = false;
        s.set_pieces.clear();
        s.walls.enabled = false;
        let mut r = WorldRenderer::default();
        let mut frame = |s: &Scene, scroll: f32| {
            let ctx = Ctx {
                scene: s, view: View::new(s, 90, 160).at(scroll), loop_len: 24.0, tphase: 0.0, scroll, far: 320.0, d_lo: 0.0, d_hi: 320.0, x_reach: 0.0,
                path_tex: r.textures.get(&s.path.material), verge_tex: r.textures.get(&s.verge.material),
                wall_tex: r.textures.get(&s.walls.material), ceil_tex: r.textures.get(&s.ceiling.material),
                path_tile: 1.0, verge_tile: 1.0, wall_tile: 1.0, ceil_tile: 1.0,
                bridge: Spans::new(&s.path.bridge, 24.0, &s.fixtures), fork: Forks::new(&s.path.fork, s.path.edge_noise, 24.0), deck_tex: r.textures.get(&s.path.bridge.deck), bottom_tex: r.textures.get(&s.verge.material),
                deck_tile: 1.0, bottom_tile: 1.0, ambient: [0.0; 3],
                sky_lights: vec![], lights: vec![], smokes: vec![], wire_pts: vec![], falls: vec![], fog: None, fog_col: [0.0; 3], void_lin: [0.0; 3], realm: 0, bounds: None, facade: None, opening: None, portal: None, front: None, verge_beyond: None, gain: 1.0, wx: weather::Wx::default(), water: None, cover: Vec::new(),
            };
            let tris = build_geometry(&ctx, &Layers::default());
            let mut gbuf = vec![raster::GPixel::EMPTY; 90 * 160];
            raster::rasterize(&mut gbuf, 90, 160, &tris, &ctx.view);
            gbuf
        };
        for railing in [Railing::Posts, Railing::Parapet, Railing::Balustrade, Railing::Iron, Railing::Rope] {
            for pillars in [true, false] {
                s.path.bridge = Bridge { enabled: true, spacing: 12.0, length: 6.0, offset: 4.0, railing, end_pillars: pillars, ..Bridge::default() };
                let g = frame(&s, 0.0);
                let faces: std::collections::BTreeSet<u8> = g.iter().filter(|p| id::is_rail(p.id)).map(|p| id::rail_parts(p.id).1).collect();
                // Coming up to the bridge, the near end of the railing faces the camera: an open end
                // would show the inner face from behind, or the drop through it.
                assert!(faces.contains(&id::FRONT) && faces.contains(&id::INNER) && faces.contains(&id::TOP), "{railing:?} pillars {pillars}: faces {faces:?}");
            }
        }
        for level in [1.5f32, 3.5] {
            s.path.bridge = Bridge { enabled: true, spacing: 12.0, length: 11.0, offset: -2.0, depth: 30.0, bottom: BridgeBottom::Water, water_level: level, railing: Railing::None, ..Bridge::default() };
            let g = frame(&s, 0.0);
            let water: Vec<f32> = g.iter().filter(|p| p.id == id::CHASM).map(|p| p.y).collect();
            assert!(!water.is_empty(), "no water seen at {level} m");
            assert!(water.iter().all(|&y| (y + level).abs() < 1e-3), "water at {level} m drawn at {:?}", water.first());
        }
    }

    #[test]
    fn bridges_leave_no_gaps_and_nothing_stands_over_the_drop() {
        // Open ground, nothing behind: a crack between the deck, the cliff and the bottom shows void.
        let mut s = preset("Forest Path");
        s.sky.enabled = false;
        s.light.fog.enabled = false;
        s.set_pieces.clear();
        s.path.bridge = Bridge { enabled: true, spacing: 12.0, length: 5.0, offset: 4.0, ..Bridge::default() };
        let mut r = WorldRenderer::default();
        for k in 0..16 {
            let scroll = k as f32 * 0.77;
            let ctx = Ctx {
                scene: &s, view: View::new(&s, 90, 160).at(scroll), loop_len: 24.0, tphase: 0.0, scroll, far: 320.0, d_lo: 0.0, d_hi: 320.0, x_reach: 0.0,
                path_tex: r.textures.get(&s.path.material), verge_tex: r.textures.get(&s.verge.material),
                wall_tex: r.textures.get(&s.walls.material), ceil_tex: r.textures.get(&s.ceiling.material),
                path_tile: 1.0, verge_tile: 1.0, wall_tile: 1.0, ceil_tile: 1.0,
                bridge: Spans::new(&s.path.bridge, 24.0, &s.fixtures), fork: Forks::new(&s.path.fork, s.path.edge_noise, 24.0), deck_tex: r.textures.get(&s.path.bridge.deck), bottom_tex: r.textures.get(&s.verge.material),
                deck_tile: 1.0, bottom_tile: 1.0, ambient: [0.0; 3],
                sky_lights: vec![], lights: vec![], smokes: vec![], wire_pts: vec![], falls: vec![], fog: None, fog_col: [0.0; 3], void_lin: [0.0; 3], realm: 0, bounds: None, facade: None, opening: None, portal: None, front: None, verge_beyond: None, gain: 1.0, wx: weather::Wx::default(), water: None, cover: Vec::new(),
            };
            let tris = build_geometry(&ctx, &Layers::default());
            let mut gbuf = vec![raster::GPixel::EMPTY; 90 * 160];
            raster::rasterize(&mut gbuf, 90, 160, &tris, &ctx.view);
            for x in (0..90).step_by(4) {
                let col: Vec<u8> = (0..160).map(|y| gbuf[y * 90 + x].id).collect();
                let top = col.iter().position(|&i| i != id::NONE).unwrap();
                assert!(col[top..].iter().all(|&i| i != id::NONE), "gap in column {x} at scroll {scroll}: {:?}", &col[top..]);
            }
        }
        // Trees beside the path are left out over the gap; the frame still renders and loops.
        let opts = RenderOptions { size: Some((120, 214)), time: Some(0.0), ..RenderOptions::default() };
        let a = r.render(&s, 0.0, &opts);
        let b = r.render(&s, 24.0, &RenderOptions { time: Some(s.motion.loop_seconds()), ..opts.clone() });
        assert_eq!(a.rgba, b.rgba);
    }

    /// Where a winding path comes to the end of a bridge, the camera sees round under the deck's
    /// end: the gap's end wall runs under it, so the bottom never shows past the gap.
    #[test]
    fn the_gap_ends_in_a_wall_under_the_deck_too() {
        let mut s = preset("Village Side Scroll");
        s.camera.yaw = 0.0;
        s.camera.side = 0.0;
        s.camera.zoom = 1.0;
        s.camera.horizon = 0.4;
        s.path.bridge = Bridge { enabled: true, spacing: 30.0, length: 10.0, offset: 3.0, depth: 6.0, bottom: BridgeBottom::Water, water_level: 3.0, railing: Railing::Iron, ..Bridge::default() };
        let (w, h) = (320usize, 180usize);
        let mut r = WorldRenderer::default();
        for k in 0..12 {
            let scroll = k as f32 * 0.5;
            let ctx = Ctx {
                scene: &s, view: View::new(&s, w, h).at(scroll), loop_len: 24.0, tphase: 0.0, scroll, far: 320.0, d_lo: 0.0, d_hi: 320.0, x_reach: 0.0,
                path_tex: r.textures.get(&s.path.material), verge_tex: r.textures.get(&s.verge.material),
                wall_tex: r.textures.get(&s.walls.material), ceil_tex: r.textures.get(&s.ceiling.material),
                path_tile: 1.0, verge_tile: 1.0, wall_tile: 1.0, ceil_tile: 1.0,
                bridge: Spans::new(&s.path.bridge, s.motion.loop_length, &s.fixtures), fork: None, deck_tex: r.textures.get(&s.path.bridge.deck), bottom_tex: r.textures.get(&s.verge.material),
                deck_tile: 1.0, bottom_tile: 1.0, ambient: [0.0; 3],
                sky_lights: vec![], lights: vec![], smokes: vec![], wire_pts: vec![], falls: vec![], fog: None, fog_col: [0.0; 3], void_lin: [0.0; 3], realm: 0, bounds: None, facade: None, opening: None, portal: None, front: None, verge_beyond: None, gain: 1.0, wx: weather::Wx::default(), water: None, cover: Vec::new(),
            };
            let tris = build_geometry(&ctx, &Layers::default());
            let mut gbuf = vec![raster::GPixel::EMPTY; w * h];
            raster::rasterize(&mut gbuf, w, h, &tris, &ctx.view);
            let past = gbuf.iter().filter(|p| p.id == id::CHASM && !ctx.on_bridge(p.d - 0.05) && !ctx.on_bridge(p.d + 0.05)).count();
            assert_eq!(past, 0, "the gap's bottom seen past its end at scroll {scroll}");
            // Nor any hole through to the sky below the ground's far edge.
            for x in 0..w {
                let col: Vec<u8> = (0..h).map(|y| gbuf[y * w + x].id).collect();
                let Some(top) = col.iter().position(|&i| i != id::NONE) else { continue };
                let holes: Vec<usize> = (top..h).filter(|&y| col[y] == id::NONE).collect();
                assert!(holes.is_empty(), "a hole in column {x} at scroll {scroll}: rows {holes:?}");
            }
        }
    }

    #[test]
    fn side_passages_open_the_wall_without_cracks() {
        let base = preset("Stone Dungeon");
        let mut forked = base.clone();
        forked.path.fork.enabled = true;
        let f = Forks::new(&forked.path.fork, forked.path.edge_noise, 24.0).unwrap();
        let mut r = WorldRenderer::default();
        let gbuf_of = |r: &mut WorldRenderer, s: &Scene, scroll: f32| {
            let ctx = Ctx {
                scene: s, view: View::new(s, 120, 214).at(scroll), loop_len: 24.0, tphase: 0.0, scroll, far: 320.0, d_lo: 0.0, d_hi: 320.0, x_reach: 0.0,
                path_tex: r.textures.get(&s.path.material), verge_tex: r.textures.get(&s.verge.material),
                wall_tex: r.textures.get(&s.walls.material), ceil_tex: r.textures.get(&s.ceiling.material),
                path_tile: 1.0, verge_tile: 1.0, wall_tile: 1.0, ceil_tile: 1.0,
                bridge: None, fork: Forks::new(&s.path.fork, s.path.edge_noise, 24.0), deck_tex: r.textures.get(&s.path.bridge.deck), bottom_tex: r.textures.get(&s.verge.material),
                deck_tile: 1.0, bottom_tile: 1.0, ambient: [0.0; 3],
                sky_lights: vec![], lights: vec![], smokes: vec![], wire_pts: vec![], falls: vec![], fog: None, fog_col: [0.0; 3], void_lin: [0.0; 3], realm: 0, bounds: None, facade: None, opening: None, portal: None, front: None, verge_beyond: None, gain: 1.0, wx: weather::Wx::default(), water: None, cover: Vec::new(),
            };
            let tris = build_geometry(&ctx, &Layers::default());
            let mut gbuf = vec![raster::GPixel::EMPTY; 120 * 214];
            raster::rasterize(&mut gbuf, 120, 214, &tris, &ctx.view);
            gbuf
        };
        let mut checked = 0;
        for k in 0..48 {
            let scroll = k as f32 * 0.5;
            // Only views where the next opening is 3 m or more ahead: then its far post hides the
            // passage's open end, and any new void pixel would be a crack.
            let next = f.openings(scroll, scroll + 40.0).into_iter().map(|(a, _, _)| a - scroll).filter(|&a| a > -2.5).fold(f32::MAX, f32::min);
            if next < 3.0 { continue; }
            let (a, b) = (gbuf_of(&mut r, &base, scroll), gbuf_of(&mut r, &forked, scroll));
            let void = |g: &[raster::GPixel]| g.iter().filter(|p| p.id == id::NONE).count();
            assert_eq!(void(&a), void(&b), "scroll {scroll}: side passages added void pixels");
            // And the wall really is open: the far post (shaded like a cliff) shows.
            if next < 12.0 { assert!(b.iter().any(|p| p.id == id::CLIFF), "scroll {scroll}: no passage visible {next} m ahead"); }
            checked += 1;
        }
        assert!(checked > 20);
        // Across the loop point every fork stays on its side: the fork one loop on is the same fork.
        for (name, fork) in [("Stone Crypt", None), ("Haunted Forest", None), ("Forest Path", Some(Fork { enabled: true, spacing: 12.0, ..Fork::default() }))] {
            let mut s = preset(name);
            if let Some(f) = fork { s.path.fork = f; }
            s.path.fork.enabled = true;
            let f = Forks::new(&s.path.fork, s.path.edge_noise, s.motion.loop_length).unwrap();
            for k in -5..5 { assert_eq!(f.side_of(k), f.side_of(k + f.per_loop), "{name}: fork {k} changes side a loop later"); }
            // With two or more per loop, neighbours do alternate.
            if f.per_loop % 2 == 0 { assert_ne!(f.side_of(0), f.side_of(1)); }
        }
    }

    #[test]
    fn still_water_mirrors_the_sky_and_ripples_keep_the_loop() {
        // A lake with nothing on it: just below the horizon the water shows the sky just above it.
        let mut s = preset("Forest Path");
        s.props.clear();
        s.ground_cover.clear();
        s.fixtures.clear();
        s.particles.clear();
        s.set_pieces.clear();
        s.weather = Weather::default();
        s.light.fog.enabled = false;
        s.verge.tufts = false;
        s.verge.material = Material { pattern: Pattern::Plain, base: [20, 24, 26], gloss: 1.0, ripples: 0.0, ..Material::default() };
        let (w, h) = (90usize, 160usize);
        let opts = RenderOptions { size: Some((w as u32, h as u32)), layers: Layers { post: false, ..Layers::default() }, time: Some(0.0), ..RenderOptions::default() };
        let mut r = WorldRenderer::default();
        let img = r.render(&s, 0.0, &opts);
        let hz = View::new(&s, w, h).horizon_px.round() as usize;
        let px = |x: usize, y: usize| { let p = &img.rgba[(y * w + x) * 4..]; 0.3 * p[0] as f32 + 0.55 * p[1] as f32 + 0.15 * p[2] as f32 };
        let mut checked = 0;
        for x in [2usize, 5, 84, 87] {
            for k in [2usize, 4, 6] {
                let (sky, water) = (px(x, hz - k - 1), px(x, hz + k));
                assert!((water - sky).abs() < sky * 0.15 + 4.0, "column {x}, {k} rows from the horizon: sky {sky} vs water {water}");
                checked += 1;
            }
        }
        assert_eq!(checked, 12);
        // Matte, the same water does not: the test is not passing by accident.
        s.verge.material.gloss = 0.0;
        let matte = r.render(&s, 0.0, &opts);
        let mp = |x: usize, y: usize| { let p = &matte.rgba[(y * w + x) * 4..]; 0.3 * p[0] as f32 + 0.55 * p[1] as f32 + 0.15 * p[2] as f32 };
        assert!(mp(2, hz + 4) < px(2, hz + 4) * 0.6);
        // Ripples move with loop time and the world, so the loop still closes.
        s.verge.material.gloss = 1.0;
        s.verge.material.ripples = 0.6;
        let a = r.render(&s, 0.0, &opts);
        let b = r.render(&s, s.motion.loop_length, &RenderOptions { time: Some(s.motion.loop_seconds()), ..opts.clone() });
        assert_eq!(a.rgba, b.rgba);
    }

    #[test]
    fn a_card_on_still_water_reflects_flipped_about_its_base() {
        // Synthetic frame: a flat mirror floor, green sky, and a red camera-facing card standing on
        // the floor. Exactly: the card's reflection is the card flipped about its base row.
        let mut s = preset("Forest Path");
        // Horizon mid-frame, so the mirrored sky stays inside the frame.
        s.camera.horizon = 0.5;
        let (w, h) = (60usize, 200usize);
        let mut r = WorldRenderer::default();
        let view = View::new(&s, w, h);
        let (hz, f, eye) = (view.horizon_px, view.focal_px, view.eye_height);
        let ctx = Ctx {
            scene: &s, view: view.clone(), loop_len: 24.0, tphase: 0.0, scroll: 0.0, far: 320.0, d_lo: 0.0, d_hi: 320.0, x_reach: 0.0,
            path_tex: r.textures.get(&s.path.material), verge_tex: r.textures.get(&s.verge.material),
            wall_tex: r.textures.get(&s.walls.material), ceil_tex: r.textures.get(&s.ceiling.material),
            path_tile: 1.0, verge_tile: 1.0, wall_tile: 1.0, ceil_tile: 1.0,
            bridge: None, fork: None, deck_tex: r.textures.get(&s.path.bridge.deck), bottom_tex: r.textures.get(&s.verge.material),
            deck_tile: 1.0, bottom_tile: 1.0, ambient: [0.0; 3],
            sky_lights: vec![], lights: vec![], smokes: vec![], wire_pts: vec![], falls: vec![], fog: None, fog_col: [0.0; 3], void_lin: [0.0; 3], realm: 0, bounds: None, facade: None, opening: None, portal: None, front: None, verge_beyond: None, gain: 1.0, wx: weather::Wx::default(), water: None, cover: Vec::new(),
        };
        let (zc, card_h) = (6.0f32, 1.2f32);
        let base_row = hz + f * eye / zc;
        let top_row = hz + f * (eye - card_h) / zc;
        let mut gbuf = vec![raster::GPixel::EMPTY; w * h];
        let mut src = vec![[0.0f32, 1.0, 0.0]; w * h];
        let mut refl = vec![Refl::default(); w * h];
        for y in 0..h {
            let yc = y as f32 + 0.5;
            for x in 0..w {
                let i = y * w + x;
                if (20..40).contains(&x) && yc >= top_row && yc < base_row {
                    gbuf[i] = raster::GPixel { depth: zc, id: id::PROP, x: 0.0, y: eye - (yc - hz) * zc / f, d: zc, realm: 0, pad: 0 };
                    src[i] = [1.0, 0.0, 0.0];
                } else if yc > hz + 0.5 {
                    let d = f * eye / (yc - hz);
                    gbuf[i] = raster::GPixel { depth: d, id: id::GROUND, x: 0.0, y: 0.0, d, realm: 0, pad: 0 };
                    // Shading has already put the assumed sky (blue) into a fully reflective pixel.
                    src[i] = [0.0, 0.0, 1.0];
                    refl[i] = Refl { r: 1.0, env: [0.0, 0.0, 1.0], rough: 0.0, sx: 0.0 };
                }
            }
        }
        let mut out = src.clone();
        reflect_pass(&ctx, &gbuf, &refl, None, &mut out);
        let x = 30;
        for k in 2..((base_row - top_row) as usize - 2) {
            let below = (base_row + k as f32) as usize;
            if below >= h { break; }
            assert!(out[below * w + x][0] > 0.95, "row {below} ({k} below the base) should reflect the card: {:?}", out[below * w + x]);
        }
        // Beyond the card's mirrored height the water reflects the sky instead.
        let past = (base_row + (base_row - top_row) + 6.0) as usize;
        if past < h { assert!(out[past * w + x][1] > 0.95, "row {past} should show the mirrored sky: {:?}", out[past * w + x]); }
        // Beside the card, only sky.
        let side = (base_row + 5.0) as usize;
        assert!(out[side * w + 5][1] > 0.95 && out[side * w + 5][0] < 0.05);
    }

    #[test]
    fn lights_on_bridge_posts_stand_at_the_ends_and_between() {
        let mut b = Bridge { enabled: true, spacing: 24.0, length: 10.0, offset: 2.0, railing: Railing::Balustrade, ..Bridge::default() };
        let lamp = |between: u32| Fixture { kind: FixtureKind::Lantern, mount: Mount::Ground, bridge_posts: true, posts_between: between, ..Fixture::default() };
        let sp = Spans::new(&b, 24.0, &[lamp(1)]).unwrap();
        let p = sp.post_places(2.0, 12.0);
        assert_eq!(p.len(), 3);
        assert!((p[1] - 7.0).abs() < 1e-4, "the middle post is halfway: {p:?}");
        assert!(p[0] > 2.0 && p[2] < 12.0, "the end posts are on the bridge: {p:?}");
        assert_eq!(Spans::new(&b, 24.0, &[lamp(3)]).unwrap().post_places(2.0, 12.0).len(), 5);
        // The railing gains a post for each light between the ends.
        let v = View::new(&preset("Forest Path"), 90, 160);
        let tris = |fx: &[Fixture]| { let mut t = Vec::new(); railing(&v, Spans::new(&b, 24.0, fx).unwrap(), 2.0, 12.0, &[], (NEAR, 300.0), &mut t); t.len() };
        assert!(tris(&[lamp(2)]) > tris(&[]), "posts between the ends add to the railing");
        // The deck has its thickness under it, and a sheet none.
        let slab = |b: &Bridge| { let mut t = Vec::new(); deck_slab(&v, Spans::new(b, 24.0, &[]).unwrap(), 2.0, 12.0, &[], (NEAR, 300.0), &mut t); t };
        let thick = slab(&b);
        assert!(!thick.is_empty() && slab(&Bridge { thickness: 0.0, ..b.clone() }).is_empty());
        assert!(thick.iter().all(|t| t.v.iter().all(|p| p.world[1] <= 1e-4 && p.world[1] >= -0.4 - 1e-4)), "the slab hangs under the deck, as deep as it is thick");
        // In a frame, a light on every post each side of the bridge in view.
        b.end_pillars = false;
        let mut s = preset("Forest Path");
        s.path.bridge = b;
        s.fixtures = vec![lamp(1)];
        let mut r = WorldRenderer::default();
        let view = View::new(&s, 90, 160);
        let (mut ctx, ..) = r.world_ctx(&WorldIn { scene: &s, distance: 0.0, time: None, base_dir: None }, view, 0, None);
        let (mut bills, mut flames) = (Vec::new(), Vec::new());
        r.collect_fixtures(&mut ctx, s.motion.loop_seconds(), &RenderOptions::default(), &mut bills, &mut flames);
        let on_first: Vec<f32> = ctx.lights.iter().map(|l| l.pos[2]).filter(|&d| d > 0.0 && d < 13.0).collect();
        assert_eq!(on_first.len(), 6, "three posts a side on the first bridge: {on_first:?}");
    }

    #[test]
    fn a_snow_cover_whitens_trees_without_any_snow_falling() {
        let mean = |cover: bool| {
            let mut s = preset("Forest Path");
            s.weather.snow_cover = crate::scene::SnowCover { enabled: cover, props: 1.0, ground: 0.0 };
            let img = WorldRenderer::default().render(&s, 3.0, &RenderOptions { size: Some((90, 160)), ..RenderOptions::default() });
            // The crowns fill the top of the frame.
            img.rgba[..90 * 40 * 4].chunks(4).map(|p| p[0] as f32 + p[1] as f32 + p[2] as f32).sum::<f32>() / (90.0 * 40.0 * 3.0)
        };
        let (bare, snowy) = (mean(false), mean(true));
        assert!(snowy > bare + 8.0, "trees under snow {snowy} against bare {bare}");
    }

    #[test]
    fn water_beyond_a_near_prop_does_not_reflect_it() {
        // A red card hangs 3 m ahead high over still water (a palm's fronds, a lantern), green sky
        // behind it. Water further off than the card cannot see it in its mirror: what the card
        // hides on screen there is only the sky, taken from the frame before props were drawn.
        let mut s = preset("Forest Path");
        s.camera.horizon = 0.5;
        let (w, h) = (60usize, 200usize);
        let mut r = WorldRenderer::default();
        let view = View::new(&s, w, h);
        let (hz, f, eye) = (view.horizon_px, view.focal_px, view.eye_height);
        let ctx = Ctx {
            scene: &s, view: view.clone(), loop_len: 24.0, tphase: 0.0, scroll: 0.0, far: 320.0, d_lo: 0.0, d_hi: 320.0, x_reach: 0.0,
            path_tex: r.textures.get(&s.path.material), verge_tex: r.textures.get(&s.verge.material),
            wall_tex: r.textures.get(&s.walls.material), ceil_tex: r.textures.get(&s.ceiling.material),
            path_tile: 1.0, verge_tile: 1.0, wall_tile: 1.0, ceil_tile: 1.0,
            bridge: None, fork: None, deck_tex: r.textures.get(&s.path.bridge.deck), bottom_tex: r.textures.get(&s.verge.material),
            deck_tile: 1.0, bottom_tile: 1.0, ambient: [0.0; 3],
            sky_lights: vec![], lights: vec![], smokes: vec![], wire_pts: vec![], falls: vec![], fog: None, fog_col: [0.0; 3], void_lin: [0.0; 3], realm: 0, bounds: None, facade: None, opening: None, portal: None, front: None, verge_beyond: None, gain: 1.0, wx: weather::Wx::default(), water: None, cover: Vec::new(),
        };
        let zc = 3.0f32;
        let mut gbuf = vec![raster::GPixel::EMPTY; w * h];
        let mut under = vec![[0.0f32, 1.0, 0.0]; w * h];
        let mut refl = vec![Refl::default(); w * h];
        for y in 0..h {
            let yc = y as f32 + 0.5;
            for x in 0..w {
                let i = y * w + x;
                if yc > hz + 0.5 {
                    let d = f * eye / (yc - hz);
                    gbuf[i] = raster::GPixel { depth: d, id: id::GROUND, x: 0.0, y: 0.0, d, realm: 0, pad: 0 };
                    under[i] = [0.0, 0.0, 1.0];
                    refl[i] = Refl { r: 1.0, env: [0.0, 0.0, 1.0], rough: 0.0, sx: 0.0 };
                }
            }
        }
        let mut src = under.clone();
        for y in 0..(hz - 10.0) as usize {
            for x in 20..40 {
                gbuf[y * w + x] = raster::GPixel { depth: zc, id: id::PROP, x: 0.0, y: eye + (hz - y as f32 - 0.5) * zc / f, d: zc, realm: 0, pad: 0 };
                src[y * w + x] = [1.0, 0.0, 0.0];
            }
        }
        let mut out = src.clone();
        reflect_pass(&ctx, &gbuf, &refl, Some(&under), &mut out);
        let x = 30;
        let mut checked = 0;
        for y in (hz + 1.0) as usize..h {
            if gbuf[y * w + x].depth < zc * 1.2 { continue; }
            // Only rows that mirror the sky inside the frame (the rest fade to the assumed sky).
            if 2.0 * hz - (y as f32 + 0.5) < 16.0 { continue; }
            let c = out[y * w + x];
            assert!(c[0] < 0.05 && c[1] > 0.95, "row {y} (water {:.1} m off) should mirror the sky behind the card, not the card: {c:?}", gbuf[y * w + x].depth);
            checked += 1;
        }
        assert!(checked > 10, "too few rows checked: {checked}");
    }

    #[test]
    fn water_reflects_what_stands_above_the_frame() {
        // A 45 m red tower 40 m ahead on a mirror floor: its top is above the frame, but its reflection
        // reaches back into the frame. Only the rows rendered above the frame can supply it; the
        // further away the tower, the nearer the band's full height (set for things at infinity) it needs.
        let mut s = preset("Forest Path");
        s.camera.horizon = 0.3;
        s.path.material.gloss = 1.0;
        s.path.material.ripples = 0.0;
        let (w, h) = (40usize, 200usize);
        let mut r = WorldRenderer::default();
        let frame = View::new(&s, w, h);
        let (gl, gt) = reflection_guard(&s, &frame, &Layers::default());
        assert!(gt > 0 && gl == 0, "guard {gl} x {gt}");
        let sum: f32 = WAVES.iter().map(|w| w.1.cos().abs()).sum();
        assert!(sum <= WAVE_COS_SUM && sum > WAVE_COS_SUM - 1e-3, "WAVE_COS_SUM should be {sum}");
        let (zc, card_h) = (40.0f32, 45.0f32);
        let mut run = |v: View| -> Vec<[f32; 3]> {
            let (bw, bh) = (v.width, v.height);
            let (hz, f, eye) = (v.horizon_px, v.focal_px, v.eye_height);
            let ctx = Ctx {
                scene: &s, view: v, loop_len: 24.0, tphase: 0.0, scroll: 0.0, far: 320.0, d_lo: 0.0, d_hi: 320.0, x_reach: 0.0,
                path_tex: r.textures.get(&s.path.material), verge_tex: r.textures.get(&s.verge.material),
                wall_tex: r.textures.get(&s.walls.material), ceil_tex: r.textures.get(&s.ceiling.material),
                path_tile: 1.0, verge_tile: 1.0, wall_tile: 1.0, ceil_tile: 1.0,
                bridge: None, fork: None, deck_tex: r.textures.get(&s.path.bridge.deck), bottom_tex: r.textures.get(&s.verge.material),
                deck_tile: 1.0, bottom_tile: 1.0, ambient: [0.0; 3],
                sky_lights: vec![], lights: vec![], smokes: vec![], wire_pts: vec![], falls: vec![], fog: None, fog_col: [0.0; 3], void_lin: [0.0; 3], realm: 0, bounds: None, facade: None, opening: None, portal: None, front: None, verge_beyond: None, gain: 1.0, wx: weather::Wx::default(), water: None, cover: Vec::new(),
            };
            let (base_row, top_row) = (hz + f * eye / zc, hz + f * (eye - card_h) / zc);
            let mut gbuf = vec![raster::GPixel::EMPTY; bw * bh];
            let mut out = vec![[0.0f32, 1.0, 0.0]; bw * bh];
            let mut refl = vec![Refl::default(); bw * bh];
            for y in 0..bh {
                let yc = y as f32 + 0.5;
                for x in 0..bw {
                    let i = y * bw + x;
                    if (10..30).contains(&x) && yc >= top_row && yc < base_row {
                        gbuf[i] = raster::GPixel { depth: zc, id: id::PROP, x: 0.0, y: eye - (yc - hz) * zc / f, d: zc, realm: 0, pad: 0 };
                        out[i] = [1.0, 0.0, 0.0];
                    } else if yc > hz + 0.5 {
                        let d = f * eye / (yc - hz);
                        gbuf[i] = raster::GPixel { depth: d, id: id::GROUND, x: 0.0, y: 0.0, d, realm: 0, pad: 0 };
                        out[i] = [0.0, 0.0, 1.0];
                        refl[i] = Refl { r: 1.0, env: [0.0, 0.0, 1.0], rough: 0.0, sx: 0.0 };
                    }
                }
            }
            reflect_pass(&ctx, &gbuf, &refl, None, &mut out);
            crop(&out, bw, v.left, v.top, w, h)
        };
        let guarded = run(frame.guarded(gl, gt));
        let bare = run(frame);
        // The frame's top row mirrors to 2 * base - 0; below it, the part of the card out of shot.
        let base = frame.horizon_px + frame.focal_px * frame.eye_height / zc;
        let from = (2.0 * base) as usize + 2;
        let to = ((frame.horizon_px + frame.focal_px * (frame.eye_height + card_h) / zc) as usize).min(h) - 2;
        assert!(to > from + 8, "rows {from}..{to}");
        for y in from..to {
            assert!(guarded[y * w + 20][0] > 0.95, "row {y} should reflect the card above the frame: {:?}", guarded[y * w + 20]);
            assert!(bare[y * w + 20][0] < 0.05, "without the band above, row {y} cannot see it: {:?}", bare[y * w + 20]);
        }
        // Inside the frame both agree.
        for y in (base as usize + 2)..(from - 4) { assert_eq!(guarded[y * w + 20], bare[y * w + 20], "row {y}"); }
    }

    #[cfg(feature = "gpu")]
    #[test]
    fn gpu_frames_fit_a_default_limits_device() {
        // The studio draws on eframe's device: WebGPU's default limits, no features.
        let Ok(ctx) = super::super::gpu::GpuContext::headless_with(true) else { eprintln!("no GPU here, skipped"); return };
        let mut g = WorldRenderer::default();
        g.set_gpu(Some(Arc::new(super::super::gpu::Gpu::new(ctx).unwrap())));
        let mut c = WorldRenderer::default();
        for name in ["Bog Boardwalk", "Ice Cave"] {
            let s = preset(name);
            let o = RenderOptions { pick: true, stats: true, ..RenderOptions::default() };
            let (a, b) = (c.render(&s, 3.0, &o), g.render(&s, 3.0, &o));
            let worst = a.rgba.iter().zip(&b.rgba).map(|(x, y)| x.abs_diff(*y)).max().unwrap();
            assert!(worst <= 4, "{name}: worst channel difference {worst}");
        }
    }

    #[cfg(feature = "gpu")]
    #[test]
    fn gpu_cards_name_and_place_the_same_pixels() {
        // Pick ids and depth after the GPU's card pass match the CPU's, card for card.
        let Ok(ctx) = super::super::gpu::GpuContext::headless() else { eprintln!("no GPU here, skipped"); return };
        let mut g = WorldRenderer::default();
        g.set_gpu(Some(Arc::new(super::super::gpu::Gpu::new(ctx).unwrap())));
        let mut c = WorldRenderer::default();
        for name in ["Forest Path", "Haunted Forest", "Ruined Castle", "Desert Ruins", "Night Road"] {
            let s = preset(name);
            let o = RenderOptions { pick: true, depth: true, stats: true, ..RenderOptions::default() };
            let (a, b) = (c.render(&s, 3.0, &o), g.render(&s, 3.0, &o));
            let (sa, sb) = (a.stats.clone().unwrap(), b.stats.clone().unwrap());
            assert_eq!(sa.coverage, sb.coverage, "{name}: coverage");
            for ((ka, la), (kb, lb)) in sa.luma.iter().zip(&sb.luma) { assert!(ka == kb && (la - lb).abs() < 0.01, "{name}: {ka} luma {la} vs {lb}"); }
            let (pa, pb) = (a.pick.unwrap().ids, b.pick.unwrap().ids);
            let ids = pa.iter().zip(&pb).filter(|(x, y)| x != y).count();
            let depth = a.depth.unwrap().iter().zip(b.depth.unwrap().iter()).filter(|(x, y)| x != y).count();
            assert!(ids == 0 && depth == 0, "{name}: {ids} pick ids and {depth} depths differ of {}", pa.len());
        }
    }

    #[test]
    fn every_pixel_says_what_part_of_the_scene_it_shows() {
        let s = preset("Forest Path");
        let mut r = WorldRenderer::default();
        let img = r.render(&s, 3.0, &RenderOptions { size: Some((120, 214)), pick: true, ..RenderOptions::default() });
        let p = img.pick.expect("pick asked for");
        assert_eq!((p.width, p.height), (120, 214));
        assert_eq!(pick_section(p.ids[(p.height - 1) * p.width + p.width / 2]), Some("path"));
        assert_eq!(pick_section(p.ids[p.width / 2]), Some("sky"));
        // Pixels a prop layer drew name that layer; without the layer they show something else.
        let li = s.props.iter().position(|l| l.enabled).unwrap();
        let mine: Vec<usize> = (0..p.ids.len()).filter(|&i| pick_item(p.ids[i]) == Some(("props", li))).collect();
        assert!(mine.len() > 50, "{} pixels of props[{li}]", mine.len());
        let mut t = s.clone();
        t.props[li].enabled = false;
        let q = r.render(&t, 3.0, &RenderOptions { size: Some((120, 214)), pick: true, ..RenderOptions::default() }).pick.unwrap();
        assert!(mine.iter().all(|&i| pick_item(q.ids[i]) != Some(("props", li))));
    }

    #[test]
    fn tiny_canvases_odd_tiles_and_big_seeds_render() {
        // The 2.0 renderer panicked on tiny canvases, broke on tile sizes that do not divide its
        // texture, and overflowed on large seeds; none of that may come back.
        let mut r = WorldRenderer::default();
        for (name, make) in crate::scene::presets::ALL {
            let mut s = make();
            s.path.material.tile_size = 0.37;
            s.verge.material.tile_size = 2.9;
            for p in s.props.iter_mut() { p.seed = u32::MAX - 3; }
            for f in s.fixtures.iter_mut() { f.seed = u32::MAX; }
            for (w, h) in [(1u32, 1u32), (2, 3), (17, 9), (5, 400)] {
                let img = r.render(&s, 3.3, &RenderOptions { size: Some((w, h)), ..RenderOptions::default() });
                assert_eq!(img.rgba.len(), (w * h * 4) as usize, "{name} at {w}x{h}");
            }
            let rep = crate::review::check_loop(&mut r, &s, &RenderOptions { size: Some((60, 107)), ..RenderOptions::default() }, 4);
            assert!(rep.exact < 1e-6, "{name}: odd tile sizes still loop ({})", rep.exact);
        }
    }

    #[test]
    fn light_from_an_exit_falls_on_the_passage_floor() {
        use crate::scene::transition::{plan, Transition};
        let (a, b) = (preset("Stone Dungeon"), preset("Forest Path"));
        let opts = RenderOptions { size: Some((120, 214)), layers: Layers { particles: false, ..Layers::default() }, ..RenderOptions::default() };
        let mut r = WorldRenderer::default();
        let floor = |spill: f32, r: &mut WorldRenderer| {
            let c = plan(&a, &b, &Transition { light_spill: spill, ..Transition::default() });
            let w = [WorldIn { scene: &a, distance: 3.0, time: Some(0.0), base_dir: None }, WorldIn { scene: &b, distance: 0.0, time: Some(0.0), base_dir: None }];
            let img = r.render_layout(&w, Layout::Cross { crossing: &c, zb: 4.0 }, &opts);
            // The floor between the camera and the exit: bottom third, middle columns.
            let mut sum = 0.0;
            for y in 150..214 { for x in 40..80 { let p = &img.rgba[(y * 120 + x) * 4..]; sum += p[0] as f32 + p[1] as f32 + p[2] as f32; } }
            sum / (64.0 * 40.0 * 3.0)
        };
        let (dark, lit) = (floor(0.0, &mut r), floor(1.0, &mut r));
        assert!(lit > dark * 1.15 + 2.0, "the exit should light the floor in front of it: {dark:.1} -> {lit:.1}");
    }

    #[test]
    fn props_follow_the_bend_and_the_shadow_toggle() {
        let mut s = preset("Forest Path");
        s.path.bend = 1.5;
        let v = View::new(&s, 120, 214);
        // A point beside the path far ahead is pushed sideways by the bend, as the path is.
        let (a, b) = (v.world_to_px(2.0, 0.0, 30.0).unwrap(), v.world_to_px(0.0, 0.0, 30.0).unwrap());
        assert!((a[0] - b[0] - v.focal_px * 2.0 / 30.0).abs() < 0.01, "props sit relative to the bent path");
        let mut r = WorldRenderer::default();
        let opts = RenderOptions { size: Some((120, 214)), ..RenderOptions::default() };
        let with = r.render(&s, 2.0, &opts).rgba;
        for p in s.props.iter_mut() { p.shadow = false; }
        let without = r.render(&s, 2.0, &opts).rgba;
        assert!(with != without, "turning prop shadows off changes the frame");
    }

    #[test]
    fn the_day_stands_where_the_hour_says() {
        let dn = crate::scene::DayNight::default();
        let at = |h: f32| dn.at(h);
        assert!(at(6.0).altitude.abs() < 1e-4 && at(18.0).altitude.abs() < 1e-4, "the sun is on the horizon at sunrise and sunset");
        assert!((at(12.0).altitude - 1.0).abs() < 1e-4 && (at(0.0).altitude + 1.0).abs() < 1e-4);
        assert_eq!((at(12.0).phase, at(0.0).phase, at(6.3).phase, at(17.8).phase), ("day", "night", "dawn", "dusk"));
        assert!(at(12.0).daylight > 0.99 && at(23.0).daylight < 0.01);
        assert!((at(25.0).hour - 1.0).abs() < 1e-4, "hours wrap round the day");
    }

    #[test]
    fn the_light_follows_the_hour() {
        let mut r = WorldRenderer::default();
        let s = preset("Forest Path");
        let o = RenderOptions { size: Some((90, 160)), time: Some(0.5), ..RenderOptions::default() };
        let sum = |img: &Image| img.rgba.iter().map(|v| *v as f64).sum::<f64>();
        let mut at = |h: f32| { r.set_hour(Some(h)); sum(&r.render(&s, 3.0, &o)) };
        let (noon, dusk, night) = (at(12.0), at(18.25), at(0.0));
        assert!(noon > dusk * 1.3 && dusk > night * 1.2, "noon {noon} > dusk {dusk} > midnight {night}");
        // The lamps are lit by night and out by day; the stars and moon only show at night.
        let bog = preset("Bog Boardwalk");
        let lamps = |h: f32| crate::scene::DayNight::bake(&bog, Some(h)).unwrap().fixtures[0].intensity;
        assert!(lamps(0.0) > 4.0 * lamps(12.0));
        let sky = |h: f32| crate::scene::DayNight::bake(&s, Some(h)).unwrap().sky;
        assert!(sky(0.0).stars.enabled && sky(0.0).moon.body.enabled && !sky(0.0).sun.enabled);
        assert!(!sky(12.0).stars.enabled && !sky(12.0).moon.body.enabled && sky(12.0).sun.enabled);
        // No sky, no day: a dungeon is left as it is.
        assert!(crate::scene::DayNight::bake(&preset("Stone Dungeon"), Some(12.0)).is_none());
        assert!(crate::scene::DayNight::bake(&preset("Star Bridge"), Some(12.0)).is_none(), "nor is space");
    }

    #[test]
    fn days_that_pass_with_the_loop_keep_it_seamless() {
        let mut r = WorldRenderer::default();
        let mut s = preset("Forest Path");
        s.sky.day_night = crate::scene::DayNight { enabled: true, days_per_loop: 1, hour: 5.0, ..crate::scene::DayNight::default() };
        let (len, secs) = (s.motion.loop_length, s.motion.loop_seconds());
        let o = |t: f32| RenderOptions { size: Some((90, 160)), time: Some(t), ..RenderOptions::default() };
        assert_eq!(r.render(&s, 0.0, &o(0.0)).rgba, r.render(&s, len, &o(secs)).rgba, "the loop must close");
        let (a, b) = (r.render(&s, 3.0, &o(secs * 0.1)), r.render(&s, 3.0, &o(secs * 0.4)));
        assert!(a.rgba != b.rgba, "the hour should move with the loop");
    }

    #[test]
    fn shooting_stars_fall_and_keep_the_loop() {
        let mut s = preset("Night Road");
        s.sky.meteors = crate::scene::Meteors { enabled: true, count: 30, brightness: 2.0, ..crate::scene::Meteors::default() };
        let (len, secs) = (s.motion.loop_length, s.motion.loop_seconds());
        let o = |t: f32| RenderOptions { size: Some((90, 160)), time: Some(t), ..RenderOptions::default() };
        let mut r = WorldRenderer::default();
        assert_eq!(r.render(&s, 0.0, &o(0.0)).rgba, r.render(&s, len, &o(secs)).rgba, "the loop must close");
        let with = r.render(&s, 3.0, &o(secs * 0.3)).rgba;
        s.sky.meteors.enabled = false;
        let without = r.render(&s, 3.0, &o(secs * 0.3)).rgba;
        assert!(with != without, "some are falling at any moment of a 30-a-loop shower");
    }

    #[test]
    fn the_moons_seed_and_relief_change_its_face() {
        let mut s = preset("Desert Canyon");
        s.sky.sun.enabled = false;
        s.sky.clouds.enabled = false;
        (s.sky.top, s.sky.horizon) = ([4, 6, 14], [10, 14, 26]);
        s.sky.moon.body = crate::scene::SkyBody { enabled: true, pos: [0.5, 0.4], radius: 0.25, ..s.sky.moon.body };
        s.sky.moon.phase = 0.45;
        let o = RenderOptions { size: Some((120, 214)), time: Some(0.0), ..RenderOptions::default() };
        let mut r = WorldRenderer::default();
        let a = r.render(&s, 0.0, &o).rgba;
        s.sky.moon.seed = 9;
        let b = r.render(&s, 0.0, &o).rgba;
        assert!(a != b, "another seed scatters other craters");
        s.sky.moon.relief = 0.0;
        let c = r.render(&s, 0.0, &o).rgba;
        assert!(b != c, "the craters' relief catches the light");
    }

    #[test]
    fn a_flame_is_hottest_low_in_its_middle_and_licks_at_its_tip() {
        let lin = [[1.0, 1.0, 0.9], [1.0, 0.8, 0.3], [1.0, 0.4, 0.1], [0.6, 0.1, 0.02]];
        let at = |dx: f32, dy: f32, ph: [f32; 2]| splat::flame_at(dx, dy, ph, &lin);
        let core = at(0.0, 0.4, [0.0, 0.0]).unwrap();
        let edge = at(0.55, 0.4, [0.0, 0.0]).unwrap();
        assert!(core.1 > edge.1 * 1.5 && core.0[2] > edge.0[2], "white-hot middle, redder edges");
        assert!(at(0.95, 0.4, [0.0, 0.0]).is_none() && at(0.0, 1.05, [0.0, 0.0]).is_none(), "nothing outside it");
        // Its tip moves as it wavers; its base hardly does.
        let row = |dy: f32, p: f32| (0..64).map(|k| -1.0 + 2.0 * k as f32 / 63.0).map(|x| at(x, dy, [p, 1.3 * p]).map_or(0.0, |c| c.1)).collect::<Vec<f32>>();
        let moved = |dy: f32| row(dy, 0.0).iter().zip(row(dy, 2.0)).map(|(a, b)| (a - b).abs()).sum::<f32>();
        assert!(moved(-0.6) > 3.0 * moved(0.7), "the tip sways ({}) more than the base ({})", moved(-0.6), moved(0.7));
    }

    #[test]
    fn the_seasons_turn_the_trees_and_a_year_can_pass_in_a_loop() {
        let s = preset("Winding Trail");
        let at = |v: f32| { let mut t = s.clone(); t.season = crate::scene::Season { enabled: true, season: v, ..Default::default() }; crate::scene::Season::bake(&t, 0.0).unwrap() };
        let (autumn, winter) = (at(2.0), at(3.0));
        let tree = |sc: &Scene| sc.props.iter().find(|p| matches!(p.kind, PropKind::Tree | PropKind::DeadTree)).unwrap().clone();
        assert!(tree(&autumn).tint[0] > tree(&autumn).tint[1], "autumn leaves turn warm: {:?}", tree(&autumn).tint);
        assert_eq!(tree(&winter).kind, PropKind::DeadTree, "broadleaf trees are bare in winter");
        assert!(winter.weather.snow_cover.enabled && autumn.particles.iter().any(|p| p.kind == ParticleKind::Leaves));
        // A year a loop closes seamlessly.
        let mut y = s.clone();
        y.season = crate::scene::Season { enabled: true, season: 0.0, years_per_loop: 1, ..Default::default() };
        let (len, secs) = (y.motion.loop_length, y.motion.loop_seconds());
        let o = |t: f32| RenderOptions { size: Some((90, 160)), time: Some(t), ..RenderOptions::default() };
        let mut r = WorldRenderer::default();
        assert_eq!(r.render(&y, 0.0, &o(0.0)).rgba, r.render(&y, len, &o(secs)).rgba, "the loop must close");
    }

    #[test]
    fn halos_and_valley_fog_change_the_picture() {
        let o = RenderOptions { size: Some((120, 214)), time: Some(0.5), ..RenderOptions::default() };
        let mut r = WorldRenderer::default();
        let mut s = preset("Night Road");
        let a = r.render(&s, 3.0, &o).rgba;
        s.sky.halo.enabled = true;
        assert!(r.render(&s, 3.0, &o).rgba != a, "a halo rings the moon");
        let mut m = preset("Misty River");
        let near = r.render(&m, 3.0, &o).rgba;
        m.weather.mist.from = 40.0;
        let far = r.render(&m, 3.0, &o).rgba;
        let bottom = |v: &[u8]| v[v.len() * 3 / 4..].iter().map(|x| *x as f32).sum::<f32>();
        assert!((bottom(&near) - bottom(&far)).abs() > 1.0, "the mist clears from the near ground");
    }

    #[test]
    fn waterfalls_and_cascades_pour_and_keep_the_loop() {
        let o = |t: f32| RenderOptions { size: Some((120, 214)), time: Some(t), ..RenderOptions::default() };
        let mut r = WorldRenderer::default();
        for name in ["Waterfall Glen", "River Run"] {
            let mut s = preset(name);
            let (len, secs) = (s.motion.loop_length, s.motion.loop_seconds());
            assert_eq!(r.render(&s, 0.0, &o(0.0)).rgba, r.render(&s, len, &o(secs)).rgba, "{name}: the loop must close");
            let (a, b) = (r.render(&s, 3.0, &o(0.0)).rgba, r.render(&s, 3.0, &o(0.3)).rgba);
            assert!(a != b, "{name}: the water moves while the camera stands still");
            s.waterfalls.clear();
            s.path.waterway.cascades = 0.0;
            let mut differs = false;
            for k in 0..6 {
                let t = len * k as f32 / 6.0;
                differs |= r.render(&preset(name), t, &o(0.3)).rgba != r.render(&s, t, &o(0.3)).rgba;
            }
            assert!(differs, "{name}: its falls show somewhere along the loop");
        }
    }

    #[test]
    fn a_village_smokes_and_its_string_lights_glow() {
        let o = RenderOptions { size: Some((120, 214)), time: Some(0.4), ..RenderOptions::default() };
        let mut r = WorldRenderer::default();
        let s = preset("Village Evening");
        let (len, secs) = (s.motion.loop_length, s.motion.loop_seconds());
        let lo = |t: f32| RenderOptions { size: Some((90, 160)), time: Some(t), ..RenderOptions::default() };
        assert_eq!(r.render(&s, 0.0, &lo(0.0)).rgba, r.render(&s, len, &lo(secs)).rgba, "the loop must close");
        let sum = |v: &[u8]| v.iter().map(|x| *x as f64).sum::<f64>();
        let lit = r.render(&s, 4.0, &o).rgba;
        let mut dark = s.clone();
        dark.fixtures[0].string_lights.enabled = false;
        assert!(sum(&lit) > sum(&r.render(&dark, 4.0, &o).rgba), "the strings of bulbs add light");
        let cottage = &s.props[0];
        let (p, _) = crate::world::propdefs::PropDefs::default().resolve(&s, None, cottage, &mut crate::world::sprites::SpriteCache::default());
        assert!(p.unwrap().alts.iter().all(|a| a.smoke.is_some()), "every build of cottage has a chimney that smokes");
    }

    #[test]
    fn neon_signs_flicker_in_their_own_colour() {
        let o = |t: f32| RenderOptions { size: Some((120, 214)), time: Some(t), ..RenderOptions::default() };
        let mut r = WorldRenderer::default();
        let mut s = preset("Dark Street");
        s.fixtures = vec![Fixture { kind: FixtureKind::Neon, mount: Mount::Wall, spacing: 6.0, flicker: 1.0, ..Fixture::default() }];
        let a = r.render(&s, 2.0, &o(0.2)).rgba;
        s.fixtures[0].tint = [60, 220, 255];
        assert!(r.render(&s, 2.0, &o(0.2)).rgba != a, "the tubes take the fixture's colour");
        let mut off = s.clone();
        off.fixtures.clear();
        assert!(r.render(&s, 2.0, &o(0.2)).rgba != r.render(&off, 2.0, &o(0.2)).rgba, "the signs show");
    }

    #[test]
    fn birds_bats_and_fish_move_and_keep_the_loop() {
        let o = |t: f32| RenderOptions { size: Some((120, 214)), time: Some(t), ..RenderOptions::default() };
        let mut r = WorldRenderer::default();
        let mut s = preset("River Run");
        s.wildlife.birds.enabled = true;
        s.wildlife.fish = crate::scene::Fish { enabled: true, spacing: 3.0, ..crate::scene::Fish::default() };
        let mut night = preset("Night Road");
        night.wildlife.bats = crate::scene::Bats { enabled: true, dusk_only: false, count: 20, ..crate::scene::Bats::default() };
        for s in [s, night] {
            let (len, secs) = (s.motion.loop_length, s.motion.loop_seconds());
            assert_eq!(r.render(&s, 0.0, &o(0.0)).rgba, r.render(&s, len, &o(secs)).rgba, "{}: the loop must close", s.name);
            let mut none = s.clone();
            none.wildlife = crate::scene::Wildlife::default();
            assert!(r.render(&s, 2.0, &o(0.3)).rgba != r.render(&none, 2.0, &o(0.3)).rgba, "{}: they show", s.name);
        }
    }

    #[test]
    fn camera_moves_keep_the_loop_and_the_flight_comes_down_once() {
        let mut r = WorldRenderer::default();
        let mut s = preset("Waterfall Glen");
        s.camera.moves = crate::scene::CameraMoves { bob: 0.05, sway: 0.5, pan: 8.0, tilt: 3.0, tilts: 2, dolly: 4.0, fly_in: 3.0, ..s.camera.moves.clone() };
        let (len, secs) = (s.motion.loop_length, s.motion.loop_seconds());
        let o = |t: f32, intro: Option<f32>| RenderOptions { size: Some((90, 160)), time: Some(t), intro, ..RenderOptions::default() };
        assert_eq!(r.render(&s, 0.0, &o(0.0, None)).rgba, r.render(&s, len, &o(secs, None)).rgba, "the loop must close");
        let still = { let mut t = s.clone(); t.camera.moves = Default::default(); t };
        assert!(r.render(&s, 7.0, &o(2.0, None)).rgba != r.render(&still, 7.0, &o(2.0, None)).rgba, "the camera moves");
        // The flight starts high and ends where the walk's own camera is.
        assert!(r.render(&s, 0.0, &o(0.0, Some(0.0))).rgba != r.render(&s, 0.0, &o(0.0, None)).rgba);
        assert_eq!(r.render(&s, 9.0, &o(3.6, Some(3.5))).rgba, r.render(&s, 9.0, &o(3.6, None)).rgba, "over once it has landed");
        let m = &s.camera.moves;
        let top = m.pose(0.0, len, 0.0, Some(0.0), s.camera.eye_height, s.camera.horizon);
        assert!((top.lift - (m.fly_height - s.camera.eye_height)).abs() < 0.2, "it starts at fly_height: {}", top.lift);
        // A dolly zoom keeps what stands at its focus the same size.
        let d = crate::scene::CameraMoves { dolly: 6.0, dolly_focus: 10.0, ..Default::default() };
        let p = d.pose(0.0, len, 0.5, None, 1.6, 0.3);
        assert!(p.back > 5.9 && (p.zoom / (10.0 + p.back) - 1.0 / 10.0).abs() < 1e-6, "{p:?}");
    }

    #[test]
    fn focus_blurs_away_from_the_sharp_depth_or_band() {
        let mut r = WorldRenderer::default();
        let s = preset("Winding Trail");
        let (w, h) = (120usize, 214usize);
        let o = RenderOptions { size: Some((w as u32, h as u32)), time: Some(1.0), ..RenderOptions::default() };
        let plain = r.render(&s, 3.0, &o).rgba;
        let mut t = s.clone();
        t.post.focus = crate::scene::Focus { enabled: true, mode: crate::scene::FocusMode::TiltShift, blur: 0.02, band: 0.6, band_width: 0.2, ..Default::default() };
        let tilt = r.render(&t, 3.0, &o).rgba;
        let row = |img: &[u8], y: usize| img[y * w * 4..(y + 1) * w * 4].to_vec();
        let mid = (0.6 * h as f32) as usize;
        // (Bloom spreads a little of the blur round it, so the band is within a level or two.)
        let off = |y: usize| row(&tilt, y).iter().zip(row(&plain, y)).map(|(a, b)| a.abs_diff(b) as u32).max().unwrap_or(0);
        assert!(off(mid) <= 2, "the band stays sharp: {}", off(mid));
        assert!(off(8) > 4 && off(h - 4) > 4, "above and below it blur: {} {}", off(8), off(h - 4));
        // A real lens: the sky (far) blurs, and blurring evens out the sky's detail.
        t.post.focus = crate::scene::Focus { enabled: true, mode: crate::scene::FocusMode::Depth, distance: 3.0, blur: 0.02, ..Default::default() };
        let depth = r.render(&t, 3.0, &o).rgba;
        let var = |img: &[u8]| { let v: Vec<f32> = (0..w).map(|x| img[(4 * w + x) * 4] as f32).collect(); let m = v.iter().sum::<f32>() / w as f32; v.iter().map(|a| (a - m).powi(2)).sum::<f32>() };
        assert!(var(&depth) <= var(&plain), "the far distance is softer");
        assert!(depth != plain);
    }

    #[test]
    fn a_turned_camera_keeps_the_loop_and_a_side_view_cuts_away_its_near_side() {
        let mut r = WorldRenderer::default();
        for name in ["Village Side Scroll", "Village Corner"] {
            let s = preset(name);
            let (len, secs) = (s.motion.loop_length, s.motion.loop_seconds());
            let o = |t: f32| RenderOptions { size: Some((96, 96)), time: Some(t), ..RenderOptions::default() };
            assert_eq!(r.render(&s, 0.0, &o(0.0)).rgba, r.render(&s, len, &o(secs)).rgba, "{name}: the loop must close");
            let mut straight = s.clone();
            (straight.camera.yaw, straight.camera.side) = (0.0, 0.0);
            assert!(r.render(&s, 3.0, &o(1.0)).rgba != r.render(&straight, 3.0, &o(1.0)).rgba, "{name}: the turn shows");
        }
        // Seen from the side, nothing on the camera's side of the path stands in the way.
        let s = preset("Village Side Scroll");
        let v = View::new(&s, 96, 54);
        assert!(v.turned && s.camera.side_view());
        let near = s.camera.side.signum();
        // A point on the path ahead of the camera is in front of it; the ground behind it is not.
        assert!(v.to_cam(0.0, 0.0, 0.0)[2] > 4.0 && v.to_cam(near * 12.0, 0.0, 0.0)[2] < 0.0);
        // Looking down the path changes nothing: the view is the one it always was.
        let d = View::new(&preset("Village Evening"), 96, 96);
        assert!(!d.turned && d.to_cam(1.0, 2.0, 7.0)[2] == 7.0);
    }

    #[test]
    fn a_building_facing_the_path_shows_its_side_walls_and_roof_down_the_street() {
        let mut s = preset("Village Evening");
        s.props.retain(|p| p.def == "@settlement#cottage");
        for p in &mut s.props { p.def = "@settlement#cottage_timber".into(); }
        let kit = crate::world::propdefs::PropDefs::default().kit(std::path::Path::new("@settlement")).unwrap();
        let cottage = kit.props["cottage_timber"].clone();
        assert_eq!(cottage.facing, Facing::Path);
        assert!(cottage.faces.len() >= 4);
        let mut flat = s.clone();
        flat.prop_defs.insert("flat".into(), PropDef { faces: Vec::new(), depth: 0.0, ..cottage });
        for p in &mut flat.props { p.def = "flat".into(); }
        let mut r = WorldRenderer::default();
        let o = |t: f32| RenderOptions { size: Some((120, 200)), time: Some(t), ..RenderOptions::default() };
        let (a, b) = (r.render(&s, 3.0, &o(1.0)).rgba, r.render(&flat, 3.0, &o(1.0)).rgba);
        // Its side walls and roof stand where a flat front shows the ground and sky past it.
        let changed = a.chunks(4).zip(b.chunks(4)).filter(|(x, y)| x.iter().zip(y.iter()).any(|(p, q)| p.abs_diff(*q) > 24)).count();
        assert!(changed > 400, "only {changed} pixels show the rest of the building");
        let (len, secs) = (s.motion.loop_length, s.motion.loop_seconds());
        assert_eq!(r.render(&s, 0.0, &o(0.0)).rgba, r.render(&s, len, &o(secs)).rgba, "the loop must close");
    }

    #[test]
    fn a_tube_shows_its_round_side_and_its_sawn_ends_with_a_soft_outline() {
        let lit = [[1.0f32; 3]; 6];
        // A log 2 m long, 0.15 m round, lying across the view 5 m ahead (focal 100 px, centre 50, 50).
        let t = TubeCard { a: [-1.0, 0.0, 5.0], u: [1.0, 0.0, 0.0], len: 2.0, r: 0.15, bark: [0.4, 0.3, 0.2], end: [0.7, 0.6, 0.4], cube: lit, kf: 0.01, cx: 50.0, cy: 50.0 };
        let (z, cover, _) = tube_at(&t, 50.0, 50.0).expect("its middle");
        assert!((z - 4.85).abs() < 1e-3 && cover == 1.0, "its near side, a radius before its axis: {z}");
        assert!(tube_at(&t, 50.0, 50.0 - 4.0).is_none(), "nothing beyond its radius (3 px at 5 m)");
        let edge = tube_at(&t, 50.0, 50.0 - 3.0).map_or(0.0, |h| h.1);
        assert!(edge > 0.0 && edge < 1.0, "its outline is soft: {edge}");
        assert!(tube_at(&t, 50.0 + 25.0, 50.0).is_none(), "nothing past its end");
        // Seen end on, its sawn end: rings, not bark.
        let e = TubeCard { a: [0.0, 0.0, 5.0], u: [0.0, 0.0, 1.0], len: 2.0, ..t };
        let (z, _, c) = tube_at(&e, 50.5, 50.5).expect("its end");
        assert!((z - 5.0).abs() < 1e-4 && c[0] > 0.4, "the near end, end grain: {z} {c:?}");
    }

    #[test]
    fn shadows_fall_away_from_the_sun_from_any_angle() {
        // One tree beside a straight path (on the far side from a side view's camera, whose own
        // side is cut away), a low sun to the right and behind it, no fog.
        let mut s = preset("Forest Path");
        s.path.bend = 0.0;
        s.light.fog.distance = 500.0;
        s.weather = Default::default();
        s.props = vec![PropLayer { side: Side::Left, lateral: 2.0, spacing: 48.0, offset: 14.0, jitter: 0.0, density: 1.0, scale: 1.0, scale_var: 0.0, ..PropLayer::default() }];
        s.sky.sun.pos = [0.78, 0.55];
        s.motion.loop_length = 48.0;
        let mut r = WorldRenderer::default();
        // (Looking down the path the tree is 14 m ahead; seen from the side, the walk is level with it.)
        for (yaw, side, (w, h), at) in [(0.0, 0.0, (120usize, 214usize), 0.0f32), (-90.0, 6.0, (214, 120), 14.0)] {
            let mut t = s.clone();
            (t.camera.yaw, t.camera.side) = (yaw, side);
            let o = RenderOptions { size: Some((w as u32, h as u32)), time: Some(0.0), ..RenderOptions::default() };
            let with = r.render(&t, at, &o).rgba;
            let mut off = t.clone();
            off.props[0].shadow = false;
            let without = r.render(&off, at, &o).rgba;
            // Where the shadow fell: pixels darker with it than without.
            let (mut n, mut sx) = (0usize, 0.0f64);
            for y in 0..h { for x in 0..w {
                let i = (y * w + x) * 4;
                let (a, b) = (with[i] as i32 + with[i + 1] as i32 + with[i + 2] as i32, without[i] as i32 + without[i + 1] as i32 + without[i + 2] as i32);
                if b - a > 12 { n += 1; sx += x as f64; }
            }}
            assert!(n > 20, "yaw {yaw}: the tree casts a shadow ({n} pixels)");
            // The sun is on the right of the picture: the shadow falls to the left of the tree's foot.
            let v = View::new(&t, w, h);
            let foot = v.world_to_px(-(v.path_half_width(14.0 - at) + 2.0), 0.0, 14.0 - at).unwrap();
            assert!(sx / (n as f64) < foot[0] as f64, "yaw {yaw}: the shadow lies left of the tree at {foot:?}, away from the sun");
        }
    }

    #[test]
    fn a_world_sky_stays_put_as_the_camera_turns() {
        // Seen down the path, then turned to look across it: with the sky in the world the sun
        // shines from the same way and the whole sky slides across the picture; with it on the
        // camera it stays in the picture and its light turns with the camera.
        // (A straight path: a winding one turns the camera down the path, not across it.)
        let mut s = preset("Village Evening");
        s.path.winding.enabled = false;
        s.sky.day_night.enabled = false;
        s.sky.sun.pos = [0.7, 0.4];
        let ahead = sky_light_dir(&View::new(&s, 480, 270), s.sky.sun.pos);
        for yaw in [-90.0f32, -40.0, 25.0, 90.0] {
            let mut t = s.clone();
            (t.camera.yaw, t.camera.side, t.camera.world_sky) = (yaw, 6.0 * yaw.signum(), true);
            let v = View::new(&t, 480, 270);
            let dir = sky_light_dir(&v, t.sky.sun.pos);
            assert!((0..3).all(|k| (dir[k] - ahead[k]).abs() < 1e-4), "yaw {yaw}: the sun shines from where it stands ({dir:?} vs {ahead:?})");
            assert!((v.sky_dx() + yaw.to_radians() / 2.4 * 480.0).abs() < 1e-2, "yaw {yaw}: and the sky slides across the picture ({})", v.sky_dx());
            t.camera.world_sky = false;
            let v = View::new(&t, 480, 270);
            assert_eq!(v.sky_dx(), 0.0);
            let turned = sky_light_dir(&v, t.sky.sun.pos);
            assert!((turned[0] - ahead[0]).abs() > 0.1 || (turned[2] - ahead[2]).abs() > 0.1, "yaw {yaw}: on the camera, the light turns with it");
        }
    }

    #[test]
    fn a_swing_comes_round_into_the_scenes_own_view() {
        use crate::scene::{Swing, SwingView};
        // From behind the walker round to a side-scroller's view (the camera 5 m to the right
        // looking left across the path).
        let mut s = preset("Village Side Scroll");
        s.camera.moves.swing_in = Swing { seconds: 3.0, hold: 0.5, from: SwingView { back: 5.0, ..SwingView::default() }, ..Swing::default() };
        let sw = &s.camera.moves.swing_in;
        assert_eq!(sw.length(), 3.5);
        let at = |t: f32| sw.pose(&s.camera, t);
        // Held on the first view, then coming round, then the scene's own camera.
        let a = at(0.2).unwrap();
        assert_eq!((a.camera.yaw, a.camera.side, a.back), (0.0, 0.0, 5.0));
        assert_eq!((a.turning.side_k, a.turning.straight, a.turning.face_k), (0.0, 0.0, 0.0));
        assert!(at(3.5).is_none() && Swing::default().pose(&s.camera, 0.0).is_none());
        let end = sw.between(&sw.from, &SwingView::of(&s.camera), &s.camera, 1.0);
        assert!((end.camera.yaw - s.camera.yaw).abs() < 1e-3 && (end.camera.side - s.camera.side).abs() < 1e-3 && end.back.abs() < 1e-3);
        assert_eq!((end.turning.side_k, end.turning.straight, end.turning.face_k), (1.0, 1.0, 1.0));
        // Round the walker: all the way it looks at the spot on the path both views look at, 5 m
        // away; the near side is gone before the camera is halfway round.
        let mut last = 0.0f32;
        for k in 1..20 {
            let p = at(0.5 + 3.0 * k as f32 / 20.0).unwrap();
            let (yaw, x, d) = (p.camera.yaw.to_radians(), p.camera.side, -p.back);
            assert!((x + 5.0 * yaw.sin()).abs() < 1e-3 && (d + 5.0 * yaw.cos()).abs() < 1e-3, "{k}: on the circle, looking at its middle");
            assert!(p.camera.yaw <= last + 1e-4, "{k}: turning one way");
            last = p.camera.yaw;
            if p.camera.yaw > -45.0 && p.camera.yaw < -40.0 { assert!(p.turning.side_k > 0.9, "{k}: the near side gone by halfway"); }
        }
        // Straight across instead: from one place to the other.
        let straight = Swing { orbit: false, ..sw.clone() }.between(&sw.from, &SwingView::of(&s.camera), &s.camera, 0.5);
        assert!((straight.camera.side - 2.5).abs() < 1e-3 && (straight.back - 2.5).abs() < 1e-3);
    }

    #[test]
    fn a_swing_ends_on_the_scene_itself_and_draws_alike_on_both_sides_of_the_turn() {
        use crate::scene::Swing;
        let mut s = preset("Village Side Scroll");
        s.camera.moves.swing_in = Swing { seconds: 2.0, hold: 0.0, bars: 0.1, ..Swing::default() };
        let mut r = WorldRenderer::default();
        let o = |intro: Option<f32>| RenderOptions { size: Some((160, 90)), time: Some(1.0), intro, ..RenderOptions::default() };
        // Once over, the scene as it always is; before, the camera elsewhere and the bars on.
        assert_eq!(r.render(&s, 5.0, &o(Some(2.5))).rgba, r.render(&s, 5.0, &o(None)).rgba);
        let mid = r.render(&s, 5.0, &o(Some(0.8)));
        assert_ne!(mid.rgba, r.render(&s, 5.0, &o(None)).rgba);
        let row = |img: &Image, y: usize| img.rgba[y * 160 * 4..(y + 1) * 160 * 4].chunks_exact(4).all(|p| p[..3] == [0, 0, 0]);
        assert!(row(&mid, 0) && row(&mid, 89) && !row(&mid, 45), "letterbox bars over the top and bottom");
    }

    #[test]
    fn a_transition_swings_out_of_one_view_crosses_and_swings_into_the_next() {
        use crate::journey::{CrossWalk, Place};
        use crate::scene::transition::{plan, Transition};
        // Down the forest path, then into the village seen from the side.
        let mut a = preset("Forest Path");
        a.canvas = preset("Village Side Scroll").canvas;
        let b = preset("Village Side Scroll");
        let c = plan(&a, &b, &Transition::default());
        assert_eq!(c.swing_out, 0.0, "the forest looks down the path already");
        assert!((c.swing_in - 2.5 * b.motion.speed).abs() < 1e-3, "{}", c.swing_in);
        assert!(c.notes.iter().any(|n| n.contains("swings round")), "{:?}", c.notes);
        let w = CrossWalk::new(c.clone(), 0.0, 0.0);
        let mut r = WorldRenderer::default();
        let o = RenderOptions { size: Some((160, 90)), ..RenderOptions::default() };
        let (pa, pb) = (Place { scene: &a, dir: None }, Place { scene: &b, dir: None });
        let at = |r: &mut WorldRenderer, travel: f32| w.frame(r, pa, pb, travel, 1.0, 1.0, &o).rgba;
        let len = w.length();
        // It ends on the village's own side view; just before, it is still coming round.
        let end = r.render(&b, w.at(len).2, &RenderOptions { time: Some(1.0), ..o.clone() }).rgba;
        assert!(at(&mut r, len) == end);
        assert!(at(&mut r, len - 0.5 * c.swing_in) != end);
        // Where the crossing hands over to the swing, nothing jumps: the camera looks down the
        // village's path either way.
        let edge = len - c.swing_in;
        let (x, y) = (at(&mut r, edge - 1e-3), at(&mut r, edge + 1e-3));
        let diff = x.iter().zip(&y).map(|(p, q)| p.abs_diff(*q) as f64).sum::<f64>() / x.len() as f64;
        assert!(diff < 1.0, "a jump at the handover ({diff})");
        // The other way round: out of the side view first.
        let back = plan(&b, &a, &Transition::default());
        assert!(back.swing_out > 0.0 && back.swing_in == 0.0);
        // And with the swing turned off, the views crossfade as before.
        let off = plan(&a, &b, &Transition { swing: crate::scene::Swing { seconds: -1.0, ..Default::default() }, ..Transition::default() });
        assert_eq!((off.swing_out, off.swing_in), (0.0, 0.0));
    }

    #[test]
    fn a_dissolve_leaves_out_its_share_of_pixels_evenly() {
        for k in [0.0f32, 0.1, 0.25, 0.5, 0.8, 1.0] {
            let gone = (0..4).flat_map(|y| (0..4).map(move |x| (x, y))).filter(|&(x, y)| raster::dissolved(x, y, k)).count();
            assert_eq!(gone, (16.0 * k).round() as usize, "{k}");
            // Every 4 x 4 block alike, wherever it is.
            assert!((0..4).all(|y| (0..4).all(|x| raster::dissolved(x, y, k) == raster::dissolved(x + 8, y + 12, k))));
        }
    }

    #[test]
    fn fences_cast_shadows_away_from_the_sun() {
        // Picket fences along both sides of a straight path, a low sun to the right and ahead, and
        // no ambient light, so only the sun's shadows show (not the soft darkening at their feet).
        let mut s = preset("Forest Path");
        s.path.bend = 0.0;
        s.light.fog.distance = 500.0;
        s.light.ambient = 0.0;
        s.weather = Default::default();
        s.props.clear();
        s.fences = vec![Fence { side: Side::Both, lateral: 1.3, spacing: 16.0, length: 16.0, ..Fence::default() }];
        s.sky.day_night.enabled = false;
        s.sky.sun.pos = [0.85, 0.8];
        s.motion.loop_length = 48.0;
        let mut r = WorldRenderer::default();
        // (From the side the camera's own side of the path is cut away: only the far fence stands.)
        for (yaw, side, (w, h), signs) in [(0.0, 0.0, (240usize, 427usize), &[-1.0f32, 1.0][..]), (-90.0, 6.0, (427, 240), &[-1.0][..])] {
            let mut t = s.clone();
            (t.camera.yaw, t.camera.side) = (yaw, side);
            let v = View::new(&t, w, h);
            let o = RenderOptions { size: Some((w as u32, h as u32)), time: Some(0.0), ..RenderOptions::default() };
            let with = r.render(&t, 0.0, &o).rgba;
            let mut off = t.clone();
            off.fences[0].shadow = false;
            let without = r.render(&off, 0.0, &o).rgba;
            // Where each fence's foot runs on screen.
            let lines: Vec<Vec<[f32; 2]>> = signs.iter().map(|&k| (1..400).filter_map(|j| { let d = j as f32 * 0.25; v.world_to_px(k * (v.path_half_width(d) + 1.3), 0.0, d) }).collect()).collect();
            // Darker with its shadow than without, and which side of the nearest foot it lies:
            // looking down the path, left of it (away from the sun) or right; from the side, below
            // it (toward the camera, away from the sun beyond) or above.
            let (mut away, mut toward) = (0usize, 0usize);
            for y in 0..h { for x in 0..w {
                let i = (y * w + x) * 4;
                let (a, b) = (with[i] as i32 + with[i + 1] as i32 + with[i + 2] as i32, without[i] as i32 + without[i + 1] as i32 + without[i + 2] as i32);
                if b - a <= 12 { continue; }
                let (px, py) = (x as f32, y as f32);
                let at = |l: &Vec<[f32; 2]>| l.iter().copied().min_by(|p, q| if yaw == 0.0 { (p[1] - py).abs().total_cmp(&(q[1] - py).abs()) } else { (p[0] - px).abs().total_cmp(&(q[0] - px).abs()) });
                let Some(f) = lines.iter().filter_map(at).min_by(|p, q| (p[0] - px).abs().total_cmp(&(q[0] - px).abs())) else { continue };
                if if yaw == 0.0 { px < f[0] } else { py > f[1] } { away += 1 } else { toward += 1 }
            }}
            assert!(away > 40, "yaw {yaw}: the fences cast shadows ({away} pixels away from the sun)");
            assert!(away > 4 * toward, "yaw {yaw}: their shadows fall away from the sun ({away} away, {toward} toward)");
        }
    }
}
