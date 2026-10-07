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

/// The scene list and index a pick id names: ("fixtures" | "props" | "set_pieces", index).
pub fn pick_item(id: u16) -> Option<(&'static str, usize)> {
    let list = match id >> 12 { 1 => "fixtures", 2 => "props", 3 => "set_pieces", _ => return None };
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
}

struct Flame {
    world: [f32; 3],
    size: f32,
    colors: Option<[Rgb; 4]>,
    glow: [f32; 3],
    flicker: f32,
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
    path_tex: Arc<Texture>,
    verge_tex: Arc<Texture>,
    wall_tex: Arc<Texture>,
    ceil_tex: Arc<Texture>,
    path_tile: f32,
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
    fn on_fork(&self, x: f32, d: f32) -> bool {
        let Some(f) = &self.fork else { return false };
        if self.scene.walls.enabled { x.abs() > self.wall_x(d) - 1e-3 } else { f.on_branch(x, self.scroll + d, self.view.path_half_width(d)) }
    }

    /// Whether `d` metres ahead is on a bridge (the ground beside the path has dropped away).
    fn on_bridge(&self, d: f32) -> bool { self.bridge.is_some_and(|b| b.contains(self.scroll + d)) }

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
        let mut l = self.ambient;
        for s in &self.sky_lights {
            let ndl = match n { Some(n) => dot3(n, s.dir).max(0.0), None => (0.45 - 0.35 * s.dir[2]).clamp(0.15, 0.8) };
            if ndl <= 0.0 { continue; }
            let vis = self.sky_visibility(x, y, d, s.dir, skip) * sun_mask;
            if vis > 0.0 { l = add3(l, scale3(s.color, ndl * vis)); }
        }
        if !self.lights.is_empty() {
            let p = self.view.to_cam(x, y, d);
            for (li, pl) in self.lights.iter().enumerate() {
                if Some(li) == except { continue; }
                let v = [pl.pos[0] - p[0], pl.pos[1] - p[1], pl.pos[2] - p[2]];
                let d2 = dot3(v, v);
                let r2 = pl.radius * pl.radius;
                if d2 >= r2 { continue; }
                let w = 1.0 - d2 / r2;
                let att = w * w / (1.0 + 0.3 * d2);
                let ndl = match n {
                    Some(n) => (dot3(n, v) / d2.sqrt().max(1e-4)).max(0.0) * 0.85 + 0.15,
                    None => 0.8,
                };
                l = add3(l, scale3(pl.color, att * ndl));
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
    /// Render the scene `distance` metres along the path.
    pub fn render(&mut self, scene: &Scene, distance: f32, opts: &RenderOptions) -> Image {
        let w = WorldIn { scene, distance, time: opts.time, base_dir: opts.base_dir.clone() };
        self.render_worlds(std::slice::from_ref(&w), None, opts)
    }

    /// One frame of a transition or a fork: every world drawn together in one picture, each in its
    /// own part of the view, lit and fogged as one place. `worlds` is [current, next] for a
    /// crossing and [current, left, right] for a fork.
    pub fn render_layout(&mut self, worlds: &[WorldIn], layout: Layout, opts: &RenderOptions) -> Image {
        let need = match layout { Layout::Cross { .. } => 2, Layout::Fork { .. } => 3 };
        assert!(worlds.len() >= need, "a {} frame needs {need} worlds", if need == 2 { "crossing" } else { "fork" });
        self.render_worlds(&worlds[..need], Some(layout), opts)
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
        let mut plan = Self::plan(worlds, layout);
        let n = plan.shear.len();
        let layers = opts.layers;
        let look = worlds[plan.look].scene;
        let (out_w, out_h) = opts.size.unwrap_or((look.canvas.width, look.canvas.height));
        let (out_w, out_h) = (out_w.max(1) as usize, out_h.max(1) as usize);
        let px = look.style.pixel_size.clamp(1, 16) as usize;
        let (w, h) = (out_w.div_ceil(px).max(2), out_h.div_ceil(px).max(2));

        // The camera: the first world's, becoming the next world's as it crosses.
        let mut cam = View::new(worlds[0].scene, w, h);
        if plan.next > 0 {
            let vn = View::new(worlds[plan.next].scene, w, h);
            let t = plan.cam_t;
            let mix = |a: f32, b: f32| a + (b - a) * t;
            cam.horizon_px = mix(cam.horizon_px, vn.horizon_px);
            cam.focal_px = mix(cam.focal_px, vn.focal_px);
            cam.eye_height = mix(cam.eye_height, vn.eye_height);
            cam.bend = mix(cam.bend, vn.bend);
            cam.hill = mix(cam.hill, vn.hill);
            cam.near_ground = mix(cam.near_ground, vn.near_ground);
            cam.lens_curve = mix(cam.lens_curve, vn.lens_curve);
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
        let mut gbuf = vec![GPixel::EMPTY; bw * bh];
        raster::rasterize(&mut gbuf, bw, bh, &tris, &ctxs[0].view);
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
        let mut bills: Vec<Billboard> = Vec::new();
        let mut flames: Vec<Flame> = Vec::new();
        for r in 0..n {
            let ls = strikes[r].0;
            let base = worlds[r].base_dir.clone();
            let o = RenderOptions { base_dir: base, ..opts.clone() };
            if layers.fixtures {
                self.collect_fixtures(&mut ctxs[r], ls, &o, &mut bills, &mut flames);
            }
            if layers.props {
                self.collect_props(&mut ctxs[r], &o, &mut bills);
                self.collect_set_pieces(&mut ctxs[r], &o, &mut bills);
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
                    z: z - 0.05, world: [0.0, 0.0, zb - 0.05], height, sprite, flip: false, nearest: false, id: id::PROP,
                    emissive: if kind == SetPieceKind::Portal { 0.9 } else { 0.0 }, shadow: 0.6, own_light: None, source: 0, realm: 0, sway: 0.0,
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

        // Shadow masks on the ground: sun shadows of props, and contact darkening under them.
        let mut sun_mask = vec![1.0f32; bw * bh];
        let mut ao_mask = vec![1.0f32; bw * bh];
        if layers.ground {
            for ctx in &ctxs { prop_shadows(ctx, &bills, &gbuf, &mut sun_mask, &mut ao_mask); }
        }
        // Cloud shadows drifting over the ground and walls.
        if ctxs.iter().any(|c| c.wx.clouds > 0.0) {
            sun_mask.par_iter_mut().zip(gbuf.par_iter()).for_each(|(m, g)| {
                if g.id != id::NONE { *m *= weather::cloud_shade(&ctxs[(g.realm as usize).min(n - 1)], g.x, g.d); }
            });
        }

        // Deferred lighting.
        let mut hdr = vec![[0.0f32; 3]; bw * bh];
        let mut refl = vec![Refl::default(); bw * bh];
        shade(&ctxs, &plan.sky_w, &soft, &gbuf, &sun_mask, &ao_mask, &layers, &mut hdr, &mut refl);
        if layers.sky {
            let skies: Vec<usize> = (0..n).filter(|&r| plan.sky_w.any(r) && worlds[r].scene.sky.enabled).collect();
            let draw = |r: usize, hdr: &mut Vec<[f32; 3]>| {
                draw_sky_bodies(&ctxs[r], &gbuf, hdr);
                if let (_, Some(s), flash) = &strikes[r] { draw_lightning(&ctxs[r], s, *flash, &gbuf, hdr); }
                weather::veil_sky(&ctxs[r], &gbuf, hdr);
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
        let here = if plan.zb > 0.0 { 0 } else { plan.next };
        let bank_t = ctxs[here].bank_t(ctxs[here].far);
        if bank_t < 1.0 {
            let fc = ctxs[here].fog_col;
            hdr.par_iter_mut().zip(gbuf.par_iter()).for_each(|(c, g)| if g.id == id::NONE { *c = mix3(fc, *c, bank_t); });
        }

        // Billboards far to near.
        bills.sort_by(|a, b| b.z.partial_cmp(&a.z).unwrap_or(std::cmp::Ordering::Equal));
        let crisp = px > 1;
        let mut pick = if opts.pick { vec![0u16; bw * bh] } else { Vec::new() };
        for b in &bills {
            draw_billboard(&ctxs[b.realm as usize], b, crisp, &mut gbuf, &mut hdr, if opts.pick { Some(&mut pick[..]) } else { None });
        }
        for ctx in &ctxs {
            if layers.ground && ctx.scene.verge.tufts { draw_tufts(ctx, &mut gbuf, &mut hdr); }
        }
        for f in &flames {
            draw_flame(&ctxs[f.realm as usize], f, &gbuf, &mut hdr);
        }
        if layers.particles {
            for (r, ctx) in ctxs.iter().enumerate() {
                weather::draw_wisps(ctx, &gbuf, &mut hdr);
                for p in ctx.scene.particles.iter().filter(|p| p.enabled && p.count > 0) {
                    draw_particles(ctx, p, strikes[r].0, &gbuf, &mut hdr);
                }
                weather::draw_precip(ctx, &gbuf, &mut hdr);
                weather::draw_drips(ctx, &gbuf, &mut hdr);
                weather::draw_sandstorm(ctx, &gbuf, &mut hdr);
            }
        }

        // Reflections in glossy floors, now that everything they could show is drawn.
        if refl.iter().any(|r| r.r >= 0.002) {
            reflect_pass(&ctxs[0], &gbuf, &refl, &mut hdr);
        }
        // The air: mist over everything low, then light scattered in it.
        if layers.particles {
            weather::apply_mist(&ctxs, &gbuf, &mut hdr);
            weather::light_shafts(&ctxs, if plan.zb > 0.0 { 0 } else { plan.next }, &gbuf, &mut hdr);
        }
        if gl > 0 || gt > 0 {
            hdr = crop(&hdr, bw, gl, gt, w, h);
            gbuf = crop(&gbuf, bw, gl, gt, w, h);
            if opts.pick { pick = crop(&pick, bw, gl, gt, w, h); }
            for c in ctxs.iter_mut() { c.view = c.view.frame(); }
        }
        let pick = opts.pick.then(|| {
            // What no card covers is the surface behind it.
            for (p, g) in pick.iter_mut().zip(&gbuf) {
                if *p != 0 { continue; }
                let ctx = &ctxs[g.realm as usize];
                *p = match g.id {
                    id::NONE => PICK_SKY,
                    id::WALL_L | id::WALL_R => PICK_WALLS,
                    id::CEILING => PICK_CEILING,
                    id::GROUND if ctx.scene.verge.enabled && g.x.abs() > ctx.path_edge(g.d) => PICK_VERGE,
                    _ => PICK_PATH,
                };
            }
            Pick { width: w, height: h, ids: pick }
        });

        // Heat haze and the lens, on the finished frame.
        if layers.post {
            weather::heat_shimmer(&ctxs, &gbuf, &mut hdr);
            if n > 1 {
                weather::lens(&ctxs[0], 1.0 - plan.cam_t, &mut hdr);
                weather::lens(&ctxs[plan.next], plan.cam_t, &mut hdr);
            } else {
                weather::lens(&ctxs[0], 1.0, &mut hdr);
            }
        }

        // Post and style, in the look of one world (post settings blend as the camera crosses).
        let li = plan.look;
        let post_scene;
        let scene: &Scene = if n > 1 {
            let (a, b) = (&worlds[0].scene.post, &worlds[plan.next].scene.post);
            let t = plan.cam_t;
            let mix = |x: f32, y: f32| x + (y - x) * t;
            let mut s = look.clone();
            s.post = Post {
                exposure: mix(a.exposure, b.exposure), contrast: mix(a.contrast, b.contrast), saturation: mix(a.saturation, b.saturation),
                bloom: mix(a.bloom, b.bloom), vignette: mix(a.vignette, b.vignette), grain: mix(a.grain, b.grain),
                tint: [0, 1, 2].map(|k| mix(a.tint[k] as f32, b.tint[k] as f32).round() as u8),
            };
            post_scene = s;
            &post_scene
        } else { look };
        let look_opts = RenderOptions { base_dir: worlds[li].base_dir.clone(), ..opts.clone() };
        let ctx = &ctxs[li];
        let tphase = ctx.tphase;
        let mut rgb = super::post::finish(&ctx.view, scene, tphase, &hdr, layers.post);
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
                let az = (body.pos[0] - 0.5) * 2.4;
                let el = (1.0 - body.pos[1].clamp(0.0, 1.0)) * 1.15 + 0.08;
                let dir = [az.sin() * el.cos(), el.sin(), az.cos() * el.cos()];
                sky_lights.push(DirLight { dir, color: scale3(rgb_lin(body.color), gain * body.intensity.max(0.0)) });
            }
        }
        // Lightning lights the world from above, out where the bolt is: tops and the ground catch it,
        // faces toward the camera stay dark against the bright sky.
        if let Some(s) = &strike {
            let dir = [(s.x - 0.5) * 1.1, 1.0, 0.7];
            let n = dot3(dir, dir).sqrt();
            sky_lights.push(DirLight { dir: [dir[0] / n, dir[1] / n, dir[2] / n], color: flash });
        }

        let ctx = Ctx {
            scene, view, loop_len, tphase, scroll, far,
            path_tex: self.textures.get(&scene.path.material),
            verge_tex: self.textures.get(&scene.verge.material),
            wall_tex: self.textures.get(&scene.walls.material),
            ceil_tex: self.textures.get(&scene.ceiling.material),
            path_tile: snap_to_loop(scene.path.material.tile_size, loop_len),
            verge_tile: snap_to_loop(scene.verge.material.tile_size, loop_len),
            wall_tile: snap_to_loop(scene.walls.material.tile_size, loop_len),
            ceil_tile: snap_to_loop(scene.ceiling.material.tile_size, loop_len),
            bridge: Spans::new(&scene.path.bridge, loop_len),
            fork: Forks::new(&scene.path.fork, loop_len),
            deck_tex: self.textures.get(&scene.path.bridge.deck),
            bottom_tex: match scene.path.bridge.bottom {
                BridgeBottom::Water => self.textures.get(&Material { pattern: Pattern::Plain, base: scene.path.bridge.bottom_color, ..Material::default() }),
                _ => self.textures.get(if scene.verge.enabled { &scene.verge.material } else { &scene.path.material }),
            },
            deck_tile: snap_to_loop(scene.path.bridge.deck.tile_size, loop_len),
            bottom_tile: snap_to_loop(if scene.verge.enabled { scene.verge.material.tile_size } else { scene.path.material.tile_size }, loop_len),
            ambient: add3(scale3(rgb_lin(scene.light.ambient_color), scene.light.ambient.max(0.0)), scale3(flash, 0.12)),
            sky_lights, lights: Vec::new(), fog, fog_col, void_lin: rgb_lin(scene.light.void_color),
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
        for k in 0..4 {
            let img = self.render(&full, full.motion.loop_length * k as f32 / 4.0, &opts);
            for c in img.rgba.chunks_exact(4) {
                acc = add3(acc, rgb_lin([c[0], c[1], c[2]]));
                count += 1.0;
            }
        }
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
            let sp = snap_to_loop(f.spacing, ctx.loop_len);
            let n = (ctx.loop_len / sp).round() as i64;
            let reach = f.radius.max(0.0);
            let k_lo = ((ctx.scroll - reach - f.offset) / sp).ceil() as i64;
            let k_hi = ((ctx.scroll + ctx.far.min(80.0) - f.offset) / sp).floor() as i64;
            let size = f.size.max(0.05);
            let (body_h, flame_frac) = fixture_body(f.kind);
            let body_h = body_h * size;
            let body = if f.sprite.is_set() {
                let path = f.sprite.pool.first().cloned().unwrap_or_else(|| f.sprite.path.clone());
                self.sprites.file(opts.base_dir.as_deref(), &path)
            } else {
                self.sprites.fixture(f.kind)
            };
            let flick_m1 = cycles(2.3, loop_seconds);
            let flick_m2 = cycles(5.1, loop_seconds);
            let seed = f.seed.wrapping_add(fi as u32 * 7919);
            for k in k_lo..=k_hi {
                let key = k.rem_euclid(n);
                let d = k as f32 * sp + f.offset - ctx.scroll;
                for (si, &sign) in f.side.signs().iter().enumerate() {
                    let skey = key * 3 + si as i64;
                    let jx = (hf(seed ^ 0x51, skey) * 2.0 - 1.0) * f.jitter;
                    let jy = (hf(seed ^ 0x77, skey) * 2.0 - 1.0) * f.jitter * 0.5;
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
                    // Over a bridge's gap there is no ground beside the path: a lamp that stood there
                    // hangs on the railing instead, so the rhythm of light carries across.
                    if matches!(f.mount, Mount::Wall) && scene.walls.enabled && sign != 0.0
                        && ctx.fork.and_then(|fk| fk.opening(ctx.scroll + d)) == Some(sign) { continue; }
                    let standing = matches!(f.mount, Mount::Ground) || (matches!(f.mount, Mount::Wall) && !scene.walls.enabled);
                    let x = if standing && x.abs() > hw && ctx.on_bridge(d) { x.signum() * (hw + 0.05) } else { x };
                    if !ctx.owns(x, 0.0, d) { continue; }
                    let flicker = 1.0 + f.flicker * (0.12 * (TAU * flick_m1 * ctx.tphase + ph).sin() + 0.06 * (TAU * flick_m2 * ctx.tphase + ph * 2.3).sin());
                    let glow = rgb_lin(f.kind.glow());
                    if f.light && f.intensity > 0.0 {
                        ctx.lights.push(PointLight {
                            pos: ctx.view.to_cam(x, flame_y, d),
                            color: scale3(glow, f.intensity * 2.0 * flicker),
                            radius: reach.max(0.5),
                        });
                    }
                    if d < NEAR { continue; }
                    // Fixture body, placed so its flame point sits at the flame height.
                    if let Some(sprite) = &body {
                        if body_h > 0.0 {
                            let top = flame_y + body_h * flame_frac;
                            let mut base = top - body_h;
                            if matches!(f.mount, Mount::Ground) && base > 0.05 {
                                bills.push(Billboard { source: PICK_FIXTURES | fi as u16, ..post_billboard(ctx, x, base, d, size) });
                            }
                            if base < 0.0 && matches!(f.mount, Mount::Ground) { base = 0.0; }
                            let z = ctx.view.to_cam(x, base, d)[2];
                            bills.push(Billboard {
                                z, world: [x, base, d], height: body_h, sprite: sprite.clone(),
                                flip: sign > 0.0 && matches!(f.mount, Mount::Wall), nearest: f.sprite.pixelated,
                                id: id::FIXTURE, emissive: if f.kind == FixtureKind::Crystal { 1.2 } else { 0.0 }, shadow: 0.0, own_light: None, source: PICK_FIXTURES | fi as u16, realm: ctx.realm, sway: 0.0 });
                        }
                    }
                    let flame_size = match f.kind { FixtureKind::Candle => 0.45, FixtureKind::Brazier => 1.8, FixtureKind::Firefly => 0.35, FixtureKind::Crystal => 1.4, _ => 1.0 } * size;
                    flames.push(Flame { world: [x, flame_y, d], size: flame_size, colors: f.kind.flame(), glow: scale3(glow, f.intensity.max(0.0)), flicker, realm: ctx.realm });
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
            // Portals light the path a little before they come into view.
            let k_lo = ((ctx.scroll - 6.0 - p.offset) / sp).ceil() as i64;
            let k_hi = ((ctx.scroll + far - p.offset) / sp).floor() as i64;
            let seed = p.seed.wrapping_add(li as u32 * 7_919);
            for k in k_lo..=k_hi {
                let d = k as f32 * sp + p.offset - ctx.scroll;
                let variant = hash(seed ^ 0x5E7, k.rem_euclid(n));
                let ow = if p.width > 0.0 { p.width } else {
                    let side = if scene.walls.enabled { ctx.wall_x(d) - 0.05 } else { ctx.view.path_half_width(d) + 0.6 };
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
                if d < NEAR + 0.2 { continue; }
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
                    z, world: [0.0, 0.0, d], height, sprite, flip: false, nearest: p.sprite.pixelated, id: id::PROP,
                    emissive: if p.kind == SetPieceKind::Portal { 0.9 } else { 0.0 },
                    shadow: p.shadow.clamp(0.0, 1.0), own_light: None, source: PICK_SET_PIECES | li as u16, realm: ctx.realm, sway });
            }
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
            let Some(look) = self.defs.resolve(scene, opts.base_dir.as_deref(), p, &mut self.sprites).0 else { continue };
            let sp = snap_to_loop(p.spacing, ctx.loop_len);
            let n = (ctx.loop_len / sp).round() as i64;
            let k_lo = ((ctx.scroll + NEAR - p.offset) / sp).ceil() as i64;
            let k_hi = ((ctx.scroll + far - p.offset) / sp).floor() as i64;
            let seed = p.seed.wrapping_add(li as u32 * 104_729);
            let hanging = look.hanging && ceiling.is_some();
            let light = look.light.as_ref().filter(|l| l.intensity > 0.0 && l.radius > 0.0);
            for k in k_lo..=k_hi {
                let key = k.rem_euclid(n);
                let d0 = k as f32 * sp + p.offset - ctx.scroll;
                for (si, &sign) in p.side.signs().iter().enumerate() {
                    for row in 0..p.rows.max(1) {
                        let ikey = (key * 4 + si as i64) * 64 + row as i64;
                        if hf(seed ^ 0xD3, ikey) > p.density { continue; }
                        // Rows further out are staggered along the path so they do not line up.
                        let d = d0 + if row > 0 { (hf(seed ^ 0x9A, ikey) - 0.5) * sp * 0.8 } else { 0.0 };
                        if d < NEAR || d > far { continue; }
                        let jit = (hf(seed ^ 0x1F, ikey) * 2.0 - 1.0) * p.jitter;
                        let edge = ctx.view.path_half_width(d);
                        let x = if sign == 0.0 { p.lateral + jit } else { sign * (edge + p.lateral + row as f32 * p.row_spacing + jit) };
                        // Nothing stands beside the deck of a bridge: the ground there has fallen away.
                        if ctx.on_bridge(d) && x.abs() > edge - 0.05 { continue; }
                        // Nor on a branch path, nor in the mouth of a side passage.
                        if ctx.on_fork(x, d) { continue; }
                        if !ctx.owns(x, 0.0, d) { continue; }
                        let sc = p.scale.max(0.01) * (1.0 + (hf(seed ^ 0x5C, ikey) * 2.0 - 1.0) * p.scale_var.clamp(0.0, 0.95));
                        let height = look.height * sc;
                        let variant = hash(seed ^ 0x7E, ikey);
                        let sprite = if look.sprites.is_empty() {
                            self.sprites.prop(look.kind, look.tint, variant)
                        } else {
                            look.sprites[(variant as usize) % look.sprites.len()].clone()
                        };
                        let base = match ceiling { Some(top) if hanging => top - height, _ => -p.sink.clamp(0.0, 0.9) * height };
                        let mut own_light = None;
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
                        let z = ctx.view.to_cam(x, base, d)[2];
                        let sway = if hanging { 0.0 } else { ctx.wx.sway_at(hf(seed ^ 0x3C, ikey) * TAU) * weather::sway_factor(look.kind) * height };
                        bills.push(Billboard {
                            z, world: [x, base, d], height, sprite, sway,
                            flip: if look.from_files { look.flip_x } else { variant & 0x100 != 0 },
                            nearest: look.pixelated, id: id::PROP, emissive: look.glow,
                            shadow: if p.shadow && !hanging { p.shadow_opacity.clamp(0.0, 1.0) } else { 0.0 }, own_light, source: PICK_PROPS | li as u16, realm: ctx.realm });
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

fn post_billboard(ctx: &Ctx, x: f32, top: f32, d: f32, size: f32) -> Billboard {
    thread_local! {
        static POST: Arc<Sprite> = Arc::new(super::sprites::paint_prop(PropKind::Pillar, [36, 32, 30], 0));
    }
    let z = ctx.view.to_cam(x, 0.0, d)[2];
    Billboard {
        z: z + 0.001, world: [x, 0.0, d], height: top, sprite: POST.with(|p| p.clone()), flip: false, nearest: false,
        id: id::FIXTURE, emissive: 0.0, shadow: 0.35 * size.min(1.0), own_light: None, source: 0, realm: ctx.realm, sway: 0.0,
    }
}

// ── Geometry ───────────────────────────────────────────────────────────────

/// Where the bridges are along the world.
#[derive(Clone, Copy, Debug)]
struct Spans { period: f32, len: f32, offset: f32, depth: f32, bottom: BridgeBottom, railing: Railing, rail_h: f32 }

impl Spans {
    fn new(b: &Bridge, loop_len: f32) -> Option<Spans> {
        if !b.enabled || b.length <= 0.0 { return None; }
        let period = snap_to_loop(b.spacing.max(2.0), loop_len);
        Some(Spans {
            period, len: b.length.clamp(0.5, (period - 0.5).max(0.5)), offset: b.offset, depth: b.depth.clamp(0.5, 200.0),
            bottom: b.bottom, railing: b.railing, rail_h: b.rail_height.clamp(0.2, 3.0),
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
struct Forks { period: f32, offset: f32, side: ForkSide, tan: f32, hw: f32, depth: f32, height: f32, per_loop: i64 }

impl Forks {
    fn new(f: &Fork, loop_len: f32) -> Option<Forks> {
        if !f.enabled || f.half_width <= 0.0 { return None; }
        let period = snap_to_loop(f.spacing.max(4.0), loop_len);
        Some(Forks {
            period, offset: f.offset, side: f.side, tan: f.angle.clamp(5.0, 85.0).to_radians().tan(),
            hw: f.half_width.clamp(0.2, period * 0.2), depth: f.depth.clamp(0.5, 100.0), height: f.height.max(0.5),
            per_loop: (loop_len / period).round().max(1.0) as i64,
        })
    }
    fn side_of(&self, k: i64) -> f32 {
        // Sides follow the fork's place in the loop, not its count from the start, or the fork just
        // ahead would change sides when the loop wraps. With an odd number per loop two neighbours
        // share a side at the wrap (one per loop: always the left).
        match self.side { ForkSide::Left => -1.0, ForkSide::Right => 1.0, ForkSide::Alternate => if k.rem_euclid(self.per_loop) % 2 == 0 { -1.0 } else { 1.0 } }
    }
    /// Junctions whose branch could reach world distance w: (where, which side).
    fn near(&self, w: f32, reach: f32) -> impl Iterator<Item = (f32, f32)> + '_ {
        let k0 = ((w - self.offset - reach) / self.period).floor() as i64;
        let k1 = ((w - self.offset + self.hw * 2.0) / self.period).floor() as i64;
        (k0..=k1).map(move |k| (self.offset + k as f32 * self.period, self.side_of(k)))
    }
    /// Whether ground at (x, w) is on a branch path. A branch leaves the main path's edge at its
    /// junction and runs off at the fork angle, with a rounded mouth.
    fn on_branch(&self, x: f32, w: f32, main_hw: f32) -> bool {
        let reach = 300.0 / self.tan;
        let norm = (1.0 + self.tan * self.tan).sqrt();
        self.near(w, reach).any(|(j, side)| {
            let t = w - j;
            if t < -self.hw || x * side <= 0.0 { return false; }
            // The centreline starts inside the main path and heads off at the fork angle.
            let x0 = side * main_hw * 0.5;
            let dist = if t >= 0.0 { ((x - x0) - side * self.tan * t).abs() / norm } else { ((x - x0).powi(2) + t * t).sqrt() };
            dist < self.hw
        })
    }
    /// The side passage (between walls) open at world distance w, if any: its side.
    fn opening(&self, w: f32) -> Option<f32> {
        let k = ((w - self.offset) / self.period).floor() as i64;
        let u = w - self.offset - k as f32 * self.period;
        (u < self.hw * 2.0).then(|| self.side_of(k))
    }
    /// Each side passage overlapping [a, b): (start, end, side), world metres.
    fn openings(&self, a: f32, b: f32) -> Vec<(f32, f32, f32)> {
        let k0 = ((a - self.offset) / self.period).floor() as i64 - 1;
        let k1 = ((b - self.offset) / self.period).ceil() as i64;
        (k0..=k1).map(|k| { let s = self.offset + k as f32 * self.period; (s, s + self.hw * 2.0, self.side_of(k)) }).filter(|&(s, e, _)| e > a && s < b).collect()
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
    let mut ds = vec![NEAR];
    while *ds.last().unwrap() < ctx.far {
        let d = *ds.last().unwrap();
        ds.push(d + (d * 0.05).max(0.05));
    }
    // Bridges: surfaces are cut where each one starts and ends, so a quad is either over the gap or not.
    let spans: Vec<(f32, f32)> = ctx.bridge.map(|b| b.within(v.scroll, v.scroll + ctx.far).into_iter().map(|(s, e)| (s - v.scroll, e - v.scroll)).collect()).unwrap_or_default();
    let walls_on = s.walls.enabled && layers.walls;
    let openings: Vec<(f32, f32, f32)> = match (&ctx.fork, walls_on) {
        (Some(f), true) => f.openings(v.scroll, v.scroll + ctx.far).into_iter().map(|(a, b, side)| (a - v.scroll, b - v.scroll, side)).collect(),
        _ => Vec::new(),
    };
    let cuts: Vec<f32> = spans.iter().flat_map(|&(s, e)| [s, e]).chain(openings.iter().flat_map(|&(a, b, _)| [a, b]))
        .filter(|&c| c > NEAR && c < ctx.far).collect();
    if !cuts.is_empty() {
        ds.retain(|&x| x == NEAR || cuts.iter().all(|&c| (x - c).abs() > 0.01));
        ds.extend(&cuts);
        ds.sort_by(|a, b| a.partial_cmp(b).unwrap());
    }
    // Steps: every surface is cut at each riser, so a quad never spans two step heights. The quad
    // before a riser ends a hair short of it (on the lower step); the riser face fills the gap.
    const STEP_EPS: f32 = 1e-3;
    let risers: Vec<f32> = match &v.stairs {
        Some(st) => st.risers(v.scroll + NEAR + STEP_EPS, v.scroll + ctx.far).into_iter().map(|e| e - v.scroll).collect(),
        None => Vec::new(),
    };
    if !risers.is_empty() {
        // Make room so each riser's two cuts stay exactly as computed: the riser below reuses them.
        ds.retain(|&x| x == NEAR || risers.iter().all(|&r| (x - r).abs() > 0.01 && (x - (r - STEP_EPS)).abs() > 0.01));
        ds.extend(risers.iter().flat_map(|&d| [d - STEP_EPS, d]));
        ds.sort_by(|a, b| a.partial_cmp(b).unwrap());
    }
    let wide = |d: f32| ((v.width as f32 * 0.5) / v.focal_px * d + v.bend_x(d).abs()) * 1.4 + 4.0;
    let vert = |x: f32, y: f32, d: f32| Vert { cam: v.to_cam(x, y, d), world: [x, y, d] };
    let walls = s.walls.enabled && layers.walls;
    let top = ctx.wall_top();
    let ext = |d: f32| {
        if walls { ctx.wall_x(d) } else if s.verge.enabled { wide(d) } else { v.path_half_width(d) + s.path.edge_noise.abs() }
    };
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
            if e > NEAR && e < ctx.far {
                let (h, x) = (v.path_half_width(e), ext(e));
                if x > h + 1e-4 {
                    for sign in [-1.0f32, 1.0] {
                        raster::quad(&mut tris, [vert(sign * h, -b.depth, e), vert(sign * x, -b.depth, e), vert(sign * x, 0.0, e), vert(sign * h, 0.0, e)], id::CLIFF);
                    }
                }
            }
            if b.railing == Railing::Posts {
                let n = ((e - s0) / 1.6).ceil().max(1.0) as usize;
                for k in 0..=n {
                    let p = s0 + (e - s0) * k as f32 / n as f32;
                    if p < NEAR || p > ctx.far { continue; }
                    let h = v.path_half_width(p);
                    for sign in [-1.0f32, 1.0] {
                        let (xa, xb) = (sign * h, sign * (h + 0.1));
                        raster::quad(&mut tris, [vert(xa, 0.0, p), vert(xb, 0.0, p), vert(xb, b.rail_h + 0.08, p), vert(xa, b.rail_h + 0.08, p)], id::RAIL);
                    }
                }
            }
        }
    }
    // Side passages: an alley (open to the sky) without a ceiling, a doorway with one.
    let passage_h = ctx.fork.filter(|_| !openings.is_empty() && s.ceiling.enabled).map(|f| f.height.min(top));
    if let Some(f) = ctx.fork.filter(|_| !openings.is_empty()) {
        let h = passage_h.unwrap_or(top);
        for &(_, e, side) in &openings {
            // The far post of each opening faces the camera; the near one faces away and is never seen.
            if e <= NEAR || e >= ctx.far { continue; }
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
                    let (y, pid) = if span.is_some() { (-b.depth, id::CHASM) } else { (0.0, id::GROUND) };
                    if (e0 > h0 + 1e-4 || e1 > h1 + 1e-4) && !(span.is_some() && b.bottom == BridgeBottom::Void) {
                        for sign in [-1.0f32, 1.0] {
                            raster::quad(&mut tris, [vert(sign * h0, y, d0), vert(sign * e0, y, d0), vert(sign * e1, y, d1), vert(sign * h1, y, d1)], pid);
                        }
                    }
                    if span.is_some() {
                        for sign in [-1.0f32, 1.0] {
                            let (x0, x1) = (sign * h0, sign * h1);
                            match b.railing {
                                Railing::None => {}
                                Railing::Posts => {
                                    // Two rails along the posts' inner faces.
                                    let (r0, r1) = (sign * (h0 + 0.03), sign * (h1 + 0.03));
                                    for (lo, hi) in [(b.rail_h - 0.07, b.rail_h), (b.rail_h * 0.5 - 0.05, b.rail_h * 0.5)] {
                                        raster::quad(&mut tris, [vert(r0, lo, d0), vert(r1, lo, d1), vert(r1, hi, d1), vert(r0, hi, d0)], id::RAIL);
                                    }
                                }
                                Railing::Parapet => {
                                    let (c0, c1) = (sign * (h0 + 0.3), sign * (h1 + 0.3));
                                    raster::quad(&mut tris, [vert(x0, 0.0, d0), vert(x1, 0.0, d1), vert(x1, b.rail_h, d1), vert(x0, b.rail_h, d0)], id::RAIL);
                                    raster::quad(&mut tris, [vert(x0, b.rail_h, d0), vert(x1, b.rail_h, d1), vert(c1, b.rail_h, d1), vert(c0, b.rail_h, d0)], id::RAIL);
                                }
                            }
                        }
                    }
                }
            }
        }
        if walls {
            // Over a gap the walls carry on down to its bottom.
            let foot = span.map_or(0.0, |b| -b.depth);
            // With side passages every wall is also cut at the passage height, so the far post of each
            // opening shares its edge with the wall beside it.
            let ph = passage_h.filter(|&h| h < top - 1e-3);
            let open = ctx.fork.filter(|_| !openings.is_empty()).and_then(|f| f.opening(v.scroll + 0.5 * (d0 + d1)));
            for (sign, wid) in [(-1.0f32, id::WALL_L), (1.0, id::WALL_R)] {
                let (x0, x1) = (sign * ctx.wall_x(d0), sign * ctx.wall_x(d1));
                let f = ctx.fork.unwrap_or(Forks { period: 1.0, offset: 0.0, side: ForkSide::Left, tan: 1.0, hw: 0.0, depth: 0.0, height: 0.0, per_loop: 1 });
                if open == Some(sign) {
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
        if s.ceiling.enabled && layers.walls {
            let ch = s.ceiling.height.max(0.2);
            let ext = |d: f32| if walls { ctx.wall_x(d) } else { wide(d) };
            let (e0, e1) = (ext(d0), ext(d1));
            raster::quad(&mut tris, [vert(-e0, ch, d0), vert(-e1, ch, d1), vert(e1, ch, d1), vert(e0, ch, d0)], id::CEILING);
        }
    }
    tris
}

// ── Shading ────────────────────────────────────────────────────────────────

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
        id::RAIL => Some(match ctx.scene.path.bridge.railing {
            Railing::Parapet if ctx.scene.walls.enabled => (dw / ctx.wall_tile, -g.y / ctx.wall_tile, 2),
            Railing::Parapet => (dw / ctx.path_tile, -g.y / ctx.path_tile, 0),
            _ => ((dw + g.x) / ctx.deck_tile, -g.y / ctx.deck_tile, 4),
        }),
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
#[derive(Clone, Copy, Default)]
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
    for (lambda, ang, ph, cyc) in WAVES {
        let fade = ((lambda / footprint.max(1e-3) - 3.0) / 6.0).clamp(0.0, 1.0);
        if fade <= 0.0 { continue; }
        let k = TAU / lambda;
        let (dx, dz) = (ang.cos(), ang.sin());
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
fn shade_px(ctx: &Ctx, g: &GPixel, gbuf: &[GPixel], i: usize, x: usize, y: usize, sun_mask: &[f32], ao_mask: &[f32], layers: &Layers) -> Option<([f32; 3], Refl)> {
    let (w, h) = (ctx.view.width, ctx.view.height);
    let hy = ctx.view.horizon_px.max(1.0);
    let top = ctx.view.top as f32;
    let sky_t = |row: f32| ((row - top) / (hy - top).max(1.0)).clamp(0.0, 1.0).powf(1.3);
    let (sky_top, sky_hor) = (rgb_lin(ctx.scene.sky.top), rgb_lin(ctx.scene.sky.horizon));
    let mut rf = Refl::default();
    let scene = ctx.scene;
    let (u, v, tex) = surface_uv(ctx, g)?;
    // Footprint from neighbouring pixels on the same surface, like GPU derivatives.
    let mut fp = 0.0f32;
    for j in [if x + 1 < w { i + 1 } else { i - 1 }, if y + 1 < h { i + w } else { i - w }] {
        if gbuf[j].id == g.id && gbuf[j].realm == g.realm {
            if let Some((u2, v2, t2)) = surface_uv(ctx, &gbuf[j]) {
                if t2 == tex { fp = fp.max((u2 - u).abs().max((v2 - v).abs())); }
            }
        }
    }
    let texture = match tex { 0 => &ctx.path_tex, 1 => &ctx.verge_tex, 2 => &ctx.wall_tex, 3 => &ctx.ceil_tex, 4 => &ctx.deck_tex, 6 => &ctx.facade.as_ref().unwrap().0, _ => &ctx.bottom_tex };
    let mut albedo = texture.sample(u, v, fp.min(4.0));
    let mut ao = ao_mask[i];
    let (normal, skip) = match g.id {
        id::GROUND => {
            ao *= passage_dark(ctx, g);
            if tex == 0 && !ctx.on_fork(g.x, g.d) {
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
            ao *= 0.25 + 0.75 * (-b.depth.max(0.0) / 8.0).exp();
            if b.bottom == BridgeBottom::Ground { albedo = mul3(albedo, rgb_lin(b.bottom_color)); }
            ([0.0, 1.0, 0.0], 0)
        }
        id::CLIFF => { ao *= 0.85 * passage_dark(ctx, g); ([0.0, 0.0, -1.0], 0) }
        id::RAIL => ([if g.x > 0.0 { -1.0 } else { 1.0 }, 0.0, 0.0], 0),
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
    // Rain darkens and wets, puddles mirror, snow covers.
    let (albedo, gloss, ripples, rings) = weather::surface(ctx, g, tex, albedo, gloss, ripples);
    let light = ctx.light_at(g.x, g.y, g.d, Some(normal), skip, sun_mask[i]);
    let mut c = mul3(albedo, scale3(light, ao));
    if gloss > 0.001 {
        let p = ctx.view.to_cam(g.x, g.y, g.d);
        let plen = dot3(p, p).sqrt().max(1e-4);
        let v = [-p[0] / plen, -p[1] / plen, -p[2] / plen];
        let eye = (-p[1]).max(0.05);
        let footprint = (p[2] * p[2] / (ctx.view.focal_px * eye)).max(p[2] / ctx.view.focal_px);
        let (sx, sz) = ripple_slope(ctx, g.x, ctx.scroll + g.d, ripples, footprint);
        let (sx, sz) = (sx + rings.0, sz + rings.1);
        let n = { let n = [-sx, 1.0, -sz]; let l = dot3(n, n).sqrt(); [n[0] / l, n[1] / l, n[2] / l] };
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
        for pl in &ctx.lights {
            let lp = [pl.pos[0] - p[0], pl.pos[1] - p[1], pl.pos[2] - p[2]];
            let d2 = dot3(lp, lp);
            let r2 = pl.radius * pl.radius;
            if d2 >= r2 { continue; }
            let wgt = 1.0 - d2 / r2;
            let dist = d2.sqrt().max(1e-4);
            lobe([lp[0] / dist, lp[1] / dist, lp[2] / dist], pl.color, wgt * wgt / (1.0 + 0.3 * d2));
        }
        for sl in &ctx.sky_lights { lobe(sl.dir, sl.color, sun_mask[i]); }
        c = add3(add3(scale3(c, 1.0 - r), scale3(env, r)), spec);
        rf = Refl { r: r * ctx.fog_t(g.depth), env: scale3(env, ctx.gain), rough, sx };
    }
    Some((ctx.apply_fog(c, g.depth), rf))
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
    hdr.par_chunks_mut(w).zip(refl.par_chunks_mut(w)).enumerate().for_each(|(y, (row, rrow))| {
        for x in 0..w {
            let i = y * w + x;
            let g = &gbuf[i];
            let ctx = &ctxs[(g.realm as usize).min(ctxs.len() - 1)];
            let Some((c, rf)) = shade_px(ctx, g, gbuf, i, x, y, sun_mask, ao_mask, layers) else {
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
                    match shade_px(other, &go, gbuf, i, x, y, sun_mask, ao_mask, layers) {
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

/// Mirror reflections on glossy floors, traced against the finished frame. For a level camera the
/// ray reflected off a flat floor stays in its pixel's column, and its depth at each row has a
/// closed form: from a floor point at depth d_w seen by an eye h above it, the reflected ray reaches
/// row r at depth 2 d_w / (1 + k), with k = (r - horizon) d_w / (f h). The first row where the frame
/// shows something nearer than the ray is what the floor reflects. A camera-facing card crossed this
/// way is hit exactly where the ray meets it.
fn reflect_pass(ctx: &Ctx, gbuf: &[GPixel], refl: &[Refl], hdr: &mut [[f32; 3]]) {
    let v = &ctx.view;
    let (w, h) = (v.width, v.height);
    let (hz, f) = (v.horizon_px, v.focal_px);
    // Rows above the first glossy pixel are only read, so only the rows below are copied (the
    // reflection is added to them while other rows read them).
    let split = refl.chunks(w).position(|r| r.iter().any(|r| r.r >= 0.002)).unwrap_or(h);
    let (above, below) = hdr.split_at_mut(split * w);
    let above: &[[f32; 3]] = above;
    let src_below = below.to_vec();
    let src = |r: usize, x: usize| if r < split { above[r * w + x] } else { src_below[(r - split) * w + x] };
    // Per column, the nearest depth in each block of 8 and of 64 rows (stored block by block). A
    // block can only hold the hit if something in it is nearer than the ray at the block's top (the
    // ray only gets further as it climbs), so empty stretches are skipped whole while every
    // candidate row is still tested.
    let (nb8, nb64) = (h.div_ceil(8), h.div_ceil(64));
    let mut min8 = vec![f32::INFINITY; w * nb8];
    min8.par_chunks_mut(w).enumerate().for_each(|(b, m)| {
        for r in b * 8..(b * 8 + 8).min(h) {
            for (x, z) in gbuf[r * w..(r + 1) * w].iter().enumerate() { if z.depth < m[x] { m[x] = z.depth; } }
        }
    });
    let mut min64 = vec![f32::INFINITY; w * nb64];
    min64.par_chunks_mut(w).enumerate().for_each(|(b, m)| {
        for b8 in b * 8..(b * 8 + 8).min(nb8) {
            for (x, z) in min8[b8 * w..(b8 + 1) * w].iter().enumerate() { if *z < m[x] { m[x] = *z; } }
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
            let ray_d = |r: usize| { let k = (r as f32 + 0.5 - hz) * dw / (f * eye); if k <= -0.999 { f32::INFINITY } else { 2.0 * dw / (1.0 + k) } };
            let r_inf = 2.0 * hz - yc;
            let r_min = r_inf.max(0.0).ceil() as usize;
            let mut hit = None;
            let mut r = y as isize - 1;
            while r >= r_min as isize {
                let ru = r as usize;
                let top64 = ru / 64 * 64;
                if top64 >= r_min && min64[ru / 64 * w + xs] > ray_d(top64) { r = top64 as isize - 1; continue; }
                let top8 = ru / 8 * 8;
                if top8 >= r_min && min8[ru / 8 * w + xs] > ray_d(top8) { r = top8 as isize - 1; continue; }
                let z = gbuf[ru * w + xs].depth;
                if z.is_finite() && z <= ray_d(ru) { hit = Some(ru); break; }
                r -= 1;
            }
            let tap = |r0: f32| -> [f32; 3] {
                // A rough surface smears what it reflects along the column, the more the further
                // the reflected thing is from the surface.
                let spread = rf.rough * (yc - r0).max(0.0) * 0.25;
                let n = (spread / 1.5).ceil().clamp(1.0, 12.0) as i32;
                let mut acc = [0.0f32; 3];
                let mut wsum = 0.0;
                for j in -n..=n {
                    let wt = (n + 1 - j.abs()) as f32;
                    // Past the top of what was rendered, the top row repeats.
                    let rr = (r0 + j as f32 * spread / n as f32).round().clamp(0.0, h as f32 - 1.0);
                    acc = add3(acc, scale3(src(rr as usize, xs), wt));
                    wsum += wt;
                }
                if wsum > 0.0 { scale3(acc, 1.0 / wsum) } else { rf.env }
            };
            let col = match hit {
                Some(rh) => tap(rh as f32),
                None if r_inf >= 0.0 && gbuf[(r_inf as usize).min(h - 1) * w + xs].id == id::NONE => {
                    // Nothing in the way: the sky, mirrored with its moon, stars and clouds. Fade to
                    // the plain sky colour where the mirror row leaves the top of the frame.
                    let k = smoothstep(0.0, 16.0, r_inf);
                    mix3(rf.env, tap(r_inf), k)
                }
                None => rf.env,
            };
            row[x] = add3(row[x], scale3(sub3(col, rf.env), rf.r));
        }
    });
}

// ── Shadows ────────────────────────────────────────────────────────────────

fn prop_shadows(ctx: &Ctx, bills: &[Billboard], gbuf: &[GPixel], sun_mask: &mut [f32], ao_mask: &mut [f32]) {
    let v = &ctx.view;
    let (w, h) = (v.width, v.height);
    let sun = ctx.sky_lights.first().map(|s| s.dir).filter(|d| d[1] > 0.05 && !ctx.scene.ceiling.enabled);
    for b in bills.iter().filter(|b| b.shadow > 0.0 && b.realm == ctx.realm) {
        let [bx, _by, bd] = b.world;
        let half_w = b.height * b.sprite.aspect * 0.5;
        // Contact darkening: a soft ellipse on the ground under every prop.
        let (rx, rd) = (half_w * 0.85, half_w * 0.55);
        let mut bbox = BBox::new();
        for (dx, dd) in [(-rx, -rd), (rx, -rd), (rx, rd), (-rx, rd)] {
            bbox.add(v.world_to_px(bx + dx, 0.0, (bd + dd).max(NEAR)));
        }
        if let Some((x0, y0, x1, y1)) = bbox.clip(w, h) {
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let i = y * w + x;
                    let g = &gbuf[i];
                    if g.id != id::GROUND { continue; }
                    let (nx, nd) = ((g.x - bx) / rx, (g.d - bd) / rd);
                    let r2 = nx * nx + nd * nd;
                    if r2 < 1.0 { ao_mask[i] *= 1.0 - 0.5 * b.shadow * (1.0 - r2); }
                }
            }
        }
        // Sun shadow: the sprite's silhouette projected along the sun onto the ground.
        let Some(l) = sun else { continue };
        if l[2].abs() < 0.05 { continue; }
        let len = (b.height / l[1]).min(b.height * 6.0);
        let tip = [-l[0] / l[1] * b.height, -l[2] / l[1] * b.height];
        let scale = len / (b.height / l[1]);
        let tip = [tip[0] * scale, tip[1] * scale];
        let mut bbox = BBox::new();
        for (cx, cd) in [(-half_w, 0.0), (half_w, 0.0), (half_w + tip[0], tip[1]), (-half_w + tip[0], tip[1])] {
            bbox.add(v.world_to_px(bx + cx, 0.0, (bd + cd).max(NEAR)));
        }
        let Some((x0, y0, x1, y1)) = bbox.clip(w, h) else { continue };
        for y in y0..=y1 {
            for x in x0..=x1 {
                let i = y * w + x;
                let g = &gbuf[i];
                if g.id != id::GROUND { continue; }
                // Height on the billboard whose shadow lands here, then the sideways position.
                let hgt = (bd - g.d) * l[1] / l[2] / scale;
                if hgt < 0.0 || hgt > b.height { continue; }
                let off = g.x - bx + hgt * l[0] / l[1] * scale;
                if off.abs() > half_w { continue; }
                let mut u = off / (2.0 * half_w) + 0.5;
                if b.flip { u = 1.0 - u; }
                let a = b.sprite.sample(u, 1.0 - hgt / b.height, 192.0, false)[3];
                if a > 0.01 { sun_mask[i] = sun_mask[i].min(1.0 - b.shadow * a.min(1.0)); }
            }
        }
    }
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
    let (ox, oy) = (v.left as f32, v.top as f32);
    let (fw, fhy) = (w as f32 - 2.0 * ox, (hy - oy).max(2.0));
    let l = &ctx.scene.weather.lightning;
    // The clouds light up from inside: brighter high in the sky.
    for y in 0..(hy as usize).min(h) {
        let k = 0.55 * (1.0 - ((y as f32 - oy) / fhy).max(0.0)).powf(0.7) + 0.15;
        for x in 0..w {
            if gbuf[y * w + x].id == id::NONE { hdr[y * w + x] = add3(hdr[y * w + x], scale3(flash, k)); }
        }
    }
    if !l.bolts || s.bolt < 0.05 { return; }
    // One channel per strike: a jagged walk down from the top, with a branch or two.
    let seed = bolt_seed(l.seed, s.index);
    let scale = (h as f32 - oy) / 854.0;
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

// ── Sky ────────────────────────────────────────────────────────────────────

/// Brightness of the moon's surface at a point `n` of its sphere (unit radius, z toward the
/// viewer): soft dark maria, round craters with darker floors and brighter rims, and fine
/// mottling. Fixed to the moon, so the same at every phase. Craters are placed and measured on
/// the sphere, so the ones near the limb come out foreshortened; every edge is at least a pixel
/// wide (`px`, disc units per pixel), so a small moon blurs instead of turning blocky.
fn moon_albedo(n: [f32; 3], px: f32) -> f32 {
    use super::looks::value_noise as vn;
    let [x, y, z] = n;
    // Maria: big soft patches, more of them up and to the left, as on the near side.
    let m = 0.55 * vn(x * 2.2 + 3.1, y * 2.2 + 7.4) + 0.3 * vn(x * 4.7 + 1.3, y * 4.7 + 5.2) + 0.15 * vn(x * 9.0 + 4.0, y * 9.0);
    let soft = (2.0 * px).max(0.04);
    let maria = smoothstep(0.55 - soft, 0.6 + soft, m - 0.1 * y - 0.06 * x);
    let mut a = 1.0 - 0.42 * maria;
    for k in 0..22 {
        // A centre on the near hemisphere, and a radius: many small, a few large.
        let (u, v) = (hf(0x3A1, k) * 1.8 - 0.9, hf(0x3A2, k) * 1.8 - 0.9);
        if u * u + v * v > 0.85 { continue; }
        let w = (1.0 - u * u - v * v).sqrt();
        let rad = 0.04 + 0.16 * hf(0x3A3, k).powi(3);
        let d = ((x - u).powi(2) + (y - v).powi(2) + (z - w).powi(2)).sqrt() / rad;
        if d > 1.4 { continue; }
        let e = (px / rad).max(0.06);
        let floor = 1.0 - smoothstep(0.78 - e, 0.82 + e, d);
        let rim = smoothstep(0.72 - e, 0.92, d) * (1.0 - smoothstep(1.0, 1.2 + e, d));
        a *= 1.0 - 0.2 * floor + 0.14 * rim;
    }
    // Fine mottling, faded out where it would be smaller than a pixel.
    let grain = (1.0 - smoothstep(0.03, 0.08, px)) * (vn(x * 24.0 + 9.0, y * 24.0) - 0.5);
    a * (1.0 + 0.16 * grain)
}

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
    let scale = (h as f32 - oy) / 854.0;
    // The moon hides the stars behind it, its dark side too.
    let moon_disc = sky.moon.body.enabled.then(|| (ox + sky.moon.body.pos[0] * fw, oy + sky.moon.body.pos[1].clamp(0.0, 1.0) * fhy, sky.moon.body.radius * fhy));
    let behind_moon = |x: i64, y: i64| moon_disc.is_some_and(|(cx, cy, r)| (x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2) < r * r);
    if sky.stars.enabled {
        let tw = sky.stars.twinkle.clamp(0.0, 4.0);
        let n = sky.stars.count.min(6000) as i64;
        // The sky outside the frame has stars too, as many per pixel as the top of the frame.
        let area = fw * fhy * 0.95;
        let extra = |a: f32| (n as f32 * 1.2 * a / area.max(1.0)).round() as i64;
        let (n_top, n_side) = (extra(w as f32 * oy), extra(ox * fhy * 0.95));
        for i in 0..n + n_top + 2 * n_side {
            let (sx, sy) = if i < n {
                (ox + hf(sky.stars.seed ^ 0xA1, i) * fw, oy + hf(sky.stars.seed ^ 0xB2, i).powf(1.4) * fhy * 0.95)
            } else if i < n + n_top {
                (hf(sky.stars.seed ^ 0xA7, i) * w as f32, hf(sky.stars.seed ^ 0xB8, i) * oy)
            } else {
                let x = hf(sky.stars.seed ^ 0xA7, i) * ox;
                (if (i - n - n_top) % 2 == 0 { x } else { w as f32 - x }, oy + hf(sky.stars.seed ^ 0xB8, i) * fhy * 0.95)
            };
            let (sx, sy) = (sx as i64, sy as i64);
            let freq = 1.0 + (hash(sky.stars.seed ^ 0xC3, i) % 4) as f32;
            let tw_v = 0.5 + 0.5 * (TAU * freq * ctx.tphase + hf(sky.stars.seed ^ 0xD4, i) * TAU).sin();
            let b = (0.35 + 0.65 * hf(sky.stars.seed ^ 0xE5, i)) * (1.0 - tw * 0.25 + tw * 0.25 * tw_v) * 1.4;
            let r = (sky.stars.size * scale * (0.6 + hf(sky.stars.seed ^ 0xF6, i))).max(0.5);
            let ri = r.ceil() as i64;
            for oy in -ri..=ri { for ox in -ri..=ri {
                let dd = ((ox * ox + oy * oy) as f32).sqrt();
                if dd <= r && !behind_moon(sx + ox, sy + oy) { add(hdr, sx + ox, sy + oy, [0.9, 0.92, 1.0], b * (1.0 - dd / (r + 1.0)), true); }
            }}
        }
    }
    weather::aurora(ctx, gbuf, hdr);
    if sky.sun.enabled {
        // The sun lights the air round it: the sky is far brighter near it (the aureole), a
        // narrow bright core and a broad faint skirt, both wider in haze or fog. Its disc is
        // darker toward the limb.
        let su = &sky.sun;
        let (cx, cy, r) = (ox + su.pos[0] * fw, oy + su.pos[1].clamp(0.0, 1.0) * fhy, (su.radius * fhy).max(0.75));
        let c = scale3(rgb_lin(su.color), 0.4 + 0.6 * su.intensity.clamp(0.0, 3.0));
        let fog = ctx.fog.map_or(0.0, |(_, d)| (40.0 / d).min(1.5));
        let spread = 1.0 + 2.5 * ctx.wx.veil + fog;
        let focal = v.focal_px.max(1.0);
        hdr.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
            if y as f32 >= hy { return; }
            for (x, p) in row.iter_mut().enumerate() {
                if gbuf[y * w + x].id != id::NONE { continue; }
                let dd = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
                let th = dd / focal;
                let glow = 0.45 * (-th / (0.05 * spread)).exp() + 0.1 * (-th / (0.35 * spread)).exp();
                *p = add3(*p, scale3(c, glow));
                if dd < r + 0.5 {
                    let mu = (1.0 - (dd / r).min(1.0).powi(2)).sqrt();
                    *p = mix3(*p, scale3(c, 3.0 * (0.6 + 0.4 * mu)), (r + 0.5 - dd).clamp(0.0, 1.0));
                }
            }
        });
    }
    if sky.moon.body.enabled {
        let m = &sky.moon;
        let (cx, cy, r) = (ox + m.body.pos[0] * fw, oy + m.body.pos[1].clamp(0.0, 1.0) * fhy, m.body.radius * fhy);
        let c = rgb_lin(m.body.color);
        let gr = r * 2.6;
        // The moon is a sphere lit by the sun at the phase angle: 0 full (sun behind the viewer),
        // ±1 new; waxing is lit on the right. A point of the disc is lit where its normal faces
        // the sun, which draws the terminator as the real half-ellipse.
        let ph = m.phase.clamp(-1.0, 1.0);
        let angle = ph.abs() * std::f32::consts::PI;
        let sun = [if ph < 0.0 { -angle.sin() } else { angle.sin() }, angle.cos()];
        let lit_fraction = 0.5 * (1.0 + angle.cos());
        for y in (cy - gr) as i64..=(cy + gr) as i64 { for x in (cx - gr) as i64..=(cx + gr) as i64 {
            let (dx, dy) = ((x as f32 + 0.5 - cx) / r, (y as f32 + 0.5 - cy) / r);
            let dd = (dx * dx + dy * dy).sqrt();
            if dd < 1.0 {
                let nz = (1.0 - dd * dd).max(0.0).sqrt();
                // About a pixel of soft terminator.
                let edge = (1.5 / r.max(1.0)).clamp(0.01, 0.2);
                let lit = smoothstep(-edge, edge, dx * sun[0] + nz * sun[1]);
                let crater = if m.craters { moon_albedo([dx, dy, nz], 1.0 / r.max(1.0)) } else { 1.0 };
                // Sunlit surface over the sky. The unlit side reflects almost nothing: what shows
                // there is the sky in front of it (the stars behind are hidden above), plus faint
                // earthshine, which only reads against a dark sky.
                if lit > 0.0 { add(hdr, x, y, scale3(c, 1.15 * crater), m.opacity * lit, false); }
                add(hdr, x, y, c, 0.012 * m.opacity * (1.0 - lit), true);
                // The halo is scattered in the air in front of the moon, so it lies over the dark
                // side too (without it, the dark side read as a disc darker than the sky round it).
                add(hdr, x, y, c, 0.15 * lit_fraction, true);
            } else if dd * r < gr {
                add(hdr, x, y, c, 0.15 * lit_fraction * (1.0 - (dd * r - r) / (gr - r)).powi(2), true);
            }
        }}
    }
    if sky.clouds.enabled {
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
        let sun_at = (sky.sun.enabled).then(|| (ox + sky.sun.pos[0] * fw, oy + sky.sun.pos[1].clamp(0.0, 1.0) * fhy, scale3(rgb_lin(sky.sun.color), 0.6)));
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
            let cx = ox + (base + travel).rem_euclid(span) - (span - fw) * 0.5;
            let cy = oy + fhy * (0.12 + 0.6 * hf(cl.seed ^ 0x33, i));
            let size = fhy * (0.06 + 0.08 * hf(cl.seed ^ 0x44, i)) * cl.scale;
            let blobs = 3 + (cl.variation * 4.0) as i64;
            for b in 0..blobs {
                let t = b as f32 / (blobs - 1).max(1) as f32;
                let bx = cx + (t - 0.5) * size * (1.6 + cl.variation);
                let by = cy - size * 0.25 * (t * 3.1).sin().abs() * (0.5 + cl.variation);
                let (rx, ry) = (size * (0.55 + 0.35 * hf(cl.seed ^ 0x55, i * 8 + b)), size * (0.32 + 0.2 * hf(cl.seed ^ 0x66, i * 8 + b)));
                for y in (by - ry) as i64..=(by + ry) as i64 { for x in (bx - rx) as i64..=(bx + rx) as i64 {
                    let (dx, dy) = ((x as f32 - bx) / rx, (y as f32 - by) / ry);
                    let d2 = dx * dx + dy * dy;
                    if d2 < 1.0 {
                        let shade = 1.0 - 0.25 * dy.max(0.0);
                        let mut c = scale3(col, shade);
                        if let Some((sx, sy, sc)) = sun_at {
                            let near = (-((x as f32 - sx).powi(2) + (y as f32 - sy).powi(2)) / (0.3 * fhy).powi(2)).exp();
                            c = add3(c, scale3(sc, near * (0.25 + 0.9 * d2)));
                        }
                        add(hdr, x, y, c, cl.opacity * life * (1.0 - d2).powf(1.2) * 0.8, false);
                    }
                }}
            }
        }
    }
    weather::rainbow(ctx, gbuf, hdr);
}

// ── Billboards, flames, tufts, particles ───────────────────────────────────

fn draw_billboard(ctx: &Ctx, b: &Billboard, crisp: bool, gbuf: &mut [GPixel], hdr: &mut [[f32; 3]], mut pick: Option<&mut [u16]>) {
    let v = &ctx.view;
    let (w, h) = (v.width, v.height);
    let [x, y, d] = b.world;
    let base = v.to_cam(x, y, d);
    if base[2] < NEAR { return; }
    let Some([sx, sy_base]) = v.project(base) else { return };
    let Some([_, sy_top]) = v.project(v.to_cam(x, y + b.height, d)) else { return };
    let hp = sy_base - sy_top;
    if hp < 0.5 { return; }
    let wp = v.px_per_m(base[2]) * b.height * b.sprite.aspect;
    // In the wind the card bends: rows shift sideways, by nothing at the base and most at the top.
    let sway_px = v.px_per_m(base[2]) * b.sway;
    let bend = |vv: f32| sway_px * (1.0 - vv) * (1.0 - vv);
    let (x0, x1) = ((sx - wp * 0.5 + sway_px.min(0.0)).floor().max(0.0) as i64, ((sx + wp * 0.5 + sway_px.max(0.0)).ceil() as i64).min(w as i64 - 1));
    let (y0, y1) = (sy_top.floor().max(0.0) as i64, (sy_base.ceil() as i64).min(h as i64 - 1));
    if x0 > x1 || y0 > y1 { return; }
    let light = ctx.light_except(x, y + b.height * 0.5, d, None, 0, weather::cloud_shade(ctx, x, d), b.own_light);
    // Lying snow settles on the tops of props: a few centimetres under every edge open above.
    let snow = (ctx.wx.snow > 0.05 && b.id == id::PROP && b.emissive < 0.3).then(|| {
        let cap = (v.px_per_m(base[2]) * 0.06 * ctx.wx.snow).max(1.0) / hp;
        (cap, mul3([0.80, 0.83, 0.88], light))
    });
    let z = base[2];
    let nearest = b.nearest || crisp;
    let fog_t = ctx.fog_t(z);
    let fog_c = ctx.fog_col;
    let plain = ctx.front.is_none() && ctx.gain == 1.0;
    let (fog_mul, fog_add) = ctx.fog_affine(z);
    for py in y0..=y1 {
        let vv = (py as f32 + 0.5 - sy_top) / hp;
        if !(0.0..1.0).contains(&vv) { continue; }
        for px in x0..=x1 {
            let i = py as usize * w + px as usize;
            if z >= gbuf[i].depth { continue; }
            let mut u = (px as f32 + 0.5 - (sx - wp * 0.5) - bend(vv)) / wp;
            if !(0.0..1.0).contains(&u) { continue; }
            if b.flip { u = 1.0 - u; }
            let s = b.sprite.sample(u, vv, hp, nearest);
            let a = if nearest { if s[3] >= 0.5 { 1.0 } else { 0.0 } } else { s[3] };
            if a < 0.02 { continue; }
            let glow = b.emissive + b.sprite.glow.as_ref().map_or(0.0, |g| g.sample(u, vv, hp, nearest)[0]);
            let mut lit = add3(mul3([s[0], s[1], s[2]], light), scale3([s[0], s[1], s[2]], glow));
            if let Some((cap, snow_lit)) = snow {
                if a >= 0.5 && (vv - cap < 0.0 || b.sprite.sample(u, vv - cap, hp, true)[3] < 0.5) { lit = snow_lit; }
            }
            let c = if plain { mix3(fog_c, lit, fog_t) } else { add3(scale3(lit, fog_mul), fog_add) };
            hdr[i] = mix3(hdr[i], c, a);
            if a >= 0.5 {
                gbuf[i] = GPixel { depth: z, id: b.id, x, y, d, realm: b.realm };
                if let Some(p) = pick.as_deref_mut() { p[i] = b.source; }
            }
        }
    }
}

fn draw_flame(ctx: &Ctx, f: &Flame, gbuf: &[GPixel], hdr: &mut [[f32; 3]]) {
    let v = &ctx.view;
    let (w, h) = (v.width, v.height);
    let c = v.to_cam(f.world[0], f.world[1], f.world[2]);
    if c[2] < NEAR { return; }
    let Some([sx, sy]) = v.project(c) else { return };
    let ppm = v.px_per_m(c[2]);
    let fade = ctx.fog_t(c[2]) * ctx.gain;
    // Halo.
    let gr = (ppm * 0.7 * f.size).max(1.5);
    let glow = scale3(f.glow, 0.5 * fade * f.flicker);
    for y in (sy - gr) as i64..=(sy + gr) as i64 {
        for x in (sx - gr) as i64..=(sx + gr) as i64 {
            if x < 0 || y < 0 || x as usize >= w || y as usize >= h { continue; }
            let i = y as usize * w + x as usize;
            if gbuf[i].depth < c[2] - 0.3 { continue; }
            let dd = ((x as f32 + 0.5 - sx).powi(2) + (y as f32 + 0.5 - sy).powi(2)).sqrt() / gr;
            if dd < 1.0 { hdr[i] = add3(hdr[i], scale3(glow, (1.0 - dd).powi(2))); }
        }
    }
    // Flame body, or a bright orb.
    let fh = (ppm * 0.16 * f.size * f.flicker).max(1.0);
    let Some(cols) = f.colors else {
        let r = (ppm * 0.05 * f.size).max(0.8);
        for y in (sy - r) as i64..=(sy + r) as i64 { for x in (sx - r) as i64..=(sx + r) as i64 {
            if x < 0 || y < 0 || x as usize >= w || y as usize >= h { continue; }
            let i = y as usize * w + x as usize;
            if gbuf[i].depth < c[2] - 0.1 { continue; }
            let dd = ((x as f32 + 0.5 - sx).powi(2) + (y as f32 + 0.5 - sy).powi(2)).sqrt() / r;
            if dd < 1.0 { hdr[i] = add3(hdr[i], scale3(f.glow, 3.0 * (1.0 - dd) * fade)); }
        }}
        return;
    };
    let lin = cols.map(rgb_lin);
    let (rx, ry) = (fh * 0.55, fh * 1.25);
    let cy = sy - ry * 0.55;
    for y in (cy - ry) as i64..=(cy + ry) as i64 { for x in (sx - rx) as i64..=(sx + rx) as i64 {
        if x < 0 || y < 0 || x as usize >= w || y as usize >= h { continue; }
        let i = y as usize * w + x as usize;
        if gbuf[i].depth < c[2] - 0.1 { continue; }
        let (dx, dy) = ((x as f32 + 0.5 - sx) / rx, (y as f32 + 0.5 - cy) / ry);
        // Teardrop: narrower toward the tip.
        let taper = 1.0 + (-dy).max(0.0) * 0.8;
        let dd = ((dx * taper).powi(2) + dy * dy).sqrt();
        if dd >= 1.0 { continue; }
        let (col, a) = if dd < 0.35 { (mix3(lin[0], lin[1], dd / 0.35), 1.0) }
            else if dd < 0.7 { (mix3(lin[1], lin[2], (dd - 0.35) / 0.35), 0.85) }
            else { (mix3(lin[2], lin[3], (dd - 0.7) / 0.3), 0.85 * (1.0 - (dd - 0.7) / 0.3)) };
        hdr[i] = add3(hdr[i], scale3(col, a * 2.2 * fade));
    }}
}

fn draw_tufts(ctx: &Ctx, gbuf: &mut [GPixel], hdr: &mut [[f32; 3]]) {
    let v = &ctx.view;
    let (w, h) = (v.width, v.height);
    let s = ctx.scene;
    let sp = snap_to_loop(0.22 / s.verge.tuft_density.clamp(0.1, 4.0), ctx.loop_len);
    let n = (ctx.loop_len / sp).round() as i64;
    let range = ctx.far.min(30.0);
    let k_lo = ((ctx.scroll + 0.3) / sp).ceil() as i64;
    let k_hi = ((ctx.scroll + range) / sp).floor() as i64;
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
                for st in 0..steps {
                    let t = st as f32 / steps as f32;
                    let (px, py) = ((bx + lean * len * t * t * 0.5) as i64, (sy - len * t) as i64);
                    if px < 0 || py < 0 || px as usize >= w || py as usize >= h { continue; }
                    let i = py as usize * w + px as usize;
                    if c[2] >= gbuf[i].depth + 0.05 { continue; }
                    hdr[i] = scale3(col, 0.7 + 0.5 * t);
                    gbuf[i].id = id::TUFT;
                    gbuf[i].depth = c[2];
                }
            }
        }
    }
}

fn draw_particles(ctx: &Ctx, p: &Particles, loop_seconds: f32, gbuf: &[GPixel], hdr: &mut [[f32; 3]]) {
    let v = &ctx.view;
    let (w, h) = (v.width, v.height);
    let s = ctx.scene;
    let lat = if s.walls.enabled { ctx.wall_x(5.0) } else { v.path_half_width(5.0) + 6.0 };
    let top = if s.ceiling.enabled { s.ceiling.height } else if s.walls.enabled && s.walls.height > 0.0 { s.walls.height.min(8.0) } else { 6.0 };
    let color = rgb_lin(p.color);
    let range = ctx.far.min(36.0);
    let copies = (range / ctx.loop_len).ceil() as i64 + 1;
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
            let d = (d0 - ctx.scroll).rem_euclid(ctx.loop_len) + cpy as f32 * ctx.loop_len;
            if d < 0.3 || d > range { continue; }
            // Blown out of the band at one side, back in at the other.
            let x = if sideways { x0 + dx } else { (x0 + dx + lat).rem_euclid(2.0 * lat) - lat };
            if !ctx.owns(x, y, d) { continue; }
            let c = v.to_cam(x, y, d);
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
            for oy in -ri..=(ri + dy_len) {
                for ox in -ri - dx_len..=ri {
                    let (px, py) = (sx as i64 + ox, sy as i64 + oy);
                    if px < 0 || py < 0 || px as usize >= w || py as usize >= h { continue; }
                    let i = py as usize * w + px as usize;
                    if c[2] >= gbuf[i].depth { continue; }
                    let dd = (if sideways { 0.0 } else { (ox * ox) as f32 } + if streak { 0.0 } else { (oy * oy) as f32 }).sqrt() / (r + 0.5);
                    if dd >= 1.0 { continue; }
                    let k = (1.0 - dd) * a * life;
                    hdr[i] = if additive { add3(hdr[i], scale3(lit, k * if emissive { 1.0 } else { 0.6 })) } else { mix3(hdr[i], lit, k) };
                }
            }
        }
    }
}

fn frame_stats(ctx: &Ctx, gbuf: &[GPixel], rgb: &[[f32; 3]], billboards: usize) -> FrameStats {
    let mut count: std::collections::BTreeMap<&'static str, (f32, f32)> = Default::default();
    let mut total_luma = 0.0f32;
    for (g, c) in gbuf.iter().zip(rgb) {
        let luma = 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2];
        total_luma += luma;
        let class = match g.id {
            id::NONE => if ctx.scene.sky.enabled { "sky" } else { "void" },
            id::CHASM | id::CLIFF => "chasm",
            id::RAIL => "bridge",
            id::GROUND | id::RISER => if !ctx.scene.verge.enabled || g.x.abs() < ctx.path_edge(g.d) || ctx.on_fork(g.x, g.d) { "path" } else { "verge" },
            id::WALL_L | id::WALL_R => "walls",
            id::CEILING => "ceiling",
            id::PROP => "props",
            id::FIXTURE => "fixtures",
            _ => "grass",
        };
        let e = count.entry(class).or_insert((0.0, 0.0));
        e.0 += 1.0;
        e.1 += luma;
    }
    let n = gbuf.len().max(1) as f32;
    FrameStats {
        coverage: count.iter().map(|(k, v)| (k.to_string(), v.0 / n)).collect(),
        luma: count.iter().map(|(k, v)| (k.to_string(), v.1 / v.0.max(1.0))).collect(),
        mean_luma: total_luma / n,
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
                scene: &s, view: ctx_view, loop_len: 24.0, tphase: 0.0, scroll: k as f32 * 0.7, far: 320.0,
                path_tex: r.textures.get(&s.path.material), verge_tex: r.textures.get(&s.verge.material),
                wall_tex: r.textures.get(&s.walls.material), ceil_tex: r.textures.get(&s.ceiling.material),
                path_tile: 1.0, verge_tile: 1.0, wall_tile: 1.0, ceil_tile: 1.0,
                bridge: Spans::new(&s.path.bridge, 24.0), fork: Forks::new(&s.path.fork, 24.0), deck_tex: r.textures.get(&s.path.bridge.deck), bottom_tex: r.textures.get(&s.verge.material),
                deck_tile: 1.0, bottom_tile: 1.0, ambient: [0.0; 3],
                sky_lights: vec![], lights: vec![], fog: None, fog_col: [0.0; 3], void_lin: [0.0; 3], realm: 0, bounds: None, facade: None, opening: None, portal: None, front: None, verge_beyond: None, gain: 1.0, wx: weather::Wx::default(),
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
                scene: &s, view: View::new(&s, 90, 160).at(scroll), loop_len: 24.0, tphase: 0.0, scroll, far: 320.0,
                path_tex: r.textures.get(&s.path.material), verge_tex: r.textures.get(&s.verge.material),
                wall_tex: r.textures.get(&s.walls.material), ceil_tex: r.textures.get(&s.ceiling.material),
                path_tile: 1.0, verge_tile: 1.0, wall_tile: 1.0, ceil_tile: 1.0,
                bridge: Spans::new(&s.path.bridge, 24.0), fork: Forks::new(&s.path.fork, 24.0), deck_tex: r.textures.get(&s.path.bridge.deck), bottom_tex: r.textures.get(&s.verge.material),
                deck_tile: 1.0, bottom_tile: 1.0, ambient: [0.0; 3],
                sky_lights: vec![], lights: vec![], fog: None, fog_col: [0.0; 3], void_lin: [0.0; 3], realm: 0, bounds: None, facade: None, opening: None, portal: None, front: None, verge_beyond: None, gain: 1.0, wx: weather::Wx::default(),
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

    #[test]
    fn side_passages_open_the_wall_without_cracks() {
        let base = preset("Stone Dungeon");
        let mut forked = base.clone();
        forked.path.fork.enabled = true;
        let f = Forks::new(&forked.path.fork, 24.0).unwrap();
        let mut r = WorldRenderer::default();
        let gbuf_of = |r: &mut WorldRenderer, s: &Scene, scroll: f32| {
            let ctx = Ctx {
                scene: s, view: View::new(s, 120, 214).at(scroll), loop_len: 24.0, tphase: 0.0, scroll, far: 320.0,
                path_tex: r.textures.get(&s.path.material), verge_tex: r.textures.get(&s.verge.material),
                wall_tex: r.textures.get(&s.walls.material), ceil_tex: r.textures.get(&s.ceiling.material),
                path_tile: 1.0, verge_tile: 1.0, wall_tile: 1.0, ceil_tile: 1.0,
                bridge: None, fork: Forks::new(&s.path.fork, 24.0), deck_tex: r.textures.get(&s.path.bridge.deck), bottom_tex: r.textures.get(&s.verge.material),
                deck_tile: 1.0, bottom_tile: 1.0, ambient: [0.0; 3],
                sky_lights: vec![], lights: vec![], fog: None, fog_col: [0.0; 3], void_lin: [0.0; 3], realm: 0, bounds: None, facade: None, opening: None, portal: None, front: None, verge_beyond: None, gain: 1.0, wx: weather::Wx::default(),
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
            let f = Forks::new(&s.path.fork, s.motion.loop_length).unwrap();
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
            scene: &s, view: view.clone(), loop_len: 24.0, tphase: 0.0, scroll: 0.0, far: 320.0,
            path_tex: r.textures.get(&s.path.material), verge_tex: r.textures.get(&s.verge.material),
            wall_tex: r.textures.get(&s.walls.material), ceil_tex: r.textures.get(&s.ceiling.material),
            path_tile: 1.0, verge_tile: 1.0, wall_tile: 1.0, ceil_tile: 1.0,
            bridge: None, fork: None, deck_tex: r.textures.get(&s.path.bridge.deck), bottom_tex: r.textures.get(&s.verge.material),
            deck_tile: 1.0, bottom_tile: 1.0, ambient: [0.0; 3],
            sky_lights: vec![], lights: vec![], fog: None, fog_col: [0.0; 3], void_lin: [0.0; 3], realm: 0, bounds: None, facade: None, opening: None, portal: None, front: None, verge_beyond: None, gain: 1.0, wx: weather::Wx::default(),
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
                    gbuf[i] = raster::GPixel { depth: zc, id: id::PROP, x: 0.0, y: eye - (yc - hz) * zc / f, d: zc, realm: 0 };
                    src[i] = [1.0, 0.0, 0.0];
                } else if yc > hz + 0.5 {
                    let d = f * eye / (yc - hz);
                    gbuf[i] = raster::GPixel { depth: d, id: id::GROUND, x: 0.0, y: 0.0, d, realm: 0 };
                    // Shading has already put the assumed sky (blue) into a fully reflective pixel.
                    src[i] = [0.0, 0.0, 1.0];
                    refl[i] = Refl { r: 1.0, env: [0.0, 0.0, 1.0], rough: 0.0, sx: 0.0 };
                }
            }
        }
        let mut out = src.clone();
        reflect_pass(&ctx, &gbuf, &refl, &mut out);
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
                scene: &s, view: v, loop_len: 24.0, tphase: 0.0, scroll: 0.0, far: 320.0,
                path_tex: r.textures.get(&s.path.material), verge_tex: r.textures.get(&s.verge.material),
                wall_tex: r.textures.get(&s.walls.material), ceil_tex: r.textures.get(&s.ceiling.material),
                path_tile: 1.0, verge_tile: 1.0, wall_tile: 1.0, ceil_tile: 1.0,
                bridge: None, fork: None, deck_tex: r.textures.get(&s.path.bridge.deck), bottom_tex: r.textures.get(&s.verge.material),
                deck_tile: 1.0, bottom_tile: 1.0, ambient: [0.0; 3],
                sky_lights: vec![], lights: vec![], fog: None, fog_col: [0.0; 3], void_lin: [0.0; 3], realm: 0, bounds: None, facade: None, opening: None, portal: None, front: None, verge_beyond: None, gain: 1.0, wx: weather::Wx::default(),
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
                        gbuf[i] = raster::GPixel { depth: zc, id: id::PROP, x: 0.0, y: eye - (yc - hz) * zc / f, d: zc, realm: 0 };
                        out[i] = [1.0, 0.0, 0.0];
                    } else if yc > hz + 0.5 {
                        let d = f * eye / (yc - hz);
                        gbuf[i] = raster::GPixel { depth: d, id: id::GROUND, x: 0.0, y: 0.0, d, realm: 0 };
                        out[i] = [0.0, 0.0, 1.0];
                        refl[i] = Refl { r: 1.0, env: [0.0, 0.0, 1.0], rough: 0.0, sx: 0.0 };
                    }
                }
            }
            reflect_pass(&ctx, &gbuf, &refl, &mut out);
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
}
