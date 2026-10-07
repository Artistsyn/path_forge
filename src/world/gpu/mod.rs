//! The v3 renderer's passes on the GPU (wgpu compute), pass by pass. Each pass reads and writes
//! the same buffers the CPU pass does, laid out the same way, so a frame can run any mix of GPU
//! and CPU passes and every GPU pass can be checked against its CPU twin (`pf parity --engine gpu`).
//! Stock wgpu, core WebGPU features and default limits only.
//! Shaders live in `*.wgsl` beside this file; `common.wgsl` is prepended to each.

use super::sprites::Sprite;
use std::sync::{Arc, Mutex};
use super::texture::Texture;

/// A device and queue: created headless (pf, MCP, exports) or borrowed from the studio's window.
pub struct GpuContext {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    /// Adapter name and backend, for reports.
    pub info: String,
}

/// The process's own device, for renderers that are not handed one: made on first use. None
/// where there is no adapter, or when `PF_ENGINE=cpu` asks for the reference renderer.
pub fn shared() -> Option<Arc<GpuContext>> {
    static CTX: std::sync::OnceLock<Option<Arc<GpuContext>>> = std::sync::OnceLock::new();
    CTX.get_or_init(|| {
        if std::env::var("PF_ENGINE").is_ok_and(|e| e.eq_ignore_ascii_case("cpu")) { return None; }
        GpuContext::headless().map_err(|e| eprintln!("GPU unavailable, rendering on the CPU: {e}")).ok()
    }).clone()
}

impl GpuContext {
    /// A device of its own, no window. None (with the reason) where there is no usable adapter.
    pub fn headless() -> Result<Arc<GpuContext>, String> { Self::headless_with(false) }

    /// `headless`, with WebGPU's default limits and no features when `plain` (as eframe makes
    /// the studio's device), to check everything fits there.
    pub fn headless_with(plain: bool) -> Result<Arc<GpuContext>, String> {
        pollster::block_on(async {
            let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor { backends: wgpu::Backends::PRIMARY, ..Default::default() });
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, compatible_surface: None, force_fallback_adapter: false })
                .await
                .map_err(|e| format!("no GPU adapter: {e}"))?;
            let ai = adapter.get_info();
            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: Some("path_forge"),
                    // Pass timings for `--stages`, where the adapter has them.
                    required_features: if plain { wgpu::Features::empty() } else { adapter.features() & wgpu::Features::TIMESTAMP_QUERY },
                    // Big frames need big buffers: ask for what the adapter has (still stock wgpu).
                    required_limits: if plain { wgpu::Limits::default() } else { wgpu::Limits { max_storage_buffer_binding_size: adapter.limits().max_storage_buffer_binding_size, max_buffer_size: adapter.limits().max_buffer_size, ..wgpu::Limits::default() } },
                    memory_hints: wgpu::MemoryHints::Performance,
                    trace: wgpu::Trace::Off,
                })
                .await
                .map_err(|e| format!("GPU device: {e}"))?;
            Ok(Arc::new(GpuContext { device, queue, info: format!("{} ({:?})", ai.name, ai.backend) }))
        })
    }

    /// Use a device someone else made (the studio's window).
    pub fn from_device(device: wgpu::Device, queue: wgpu::Queue, info: String) -> Arc<GpuContext> {
        Arc::new(GpuContext { device, queue, info })
    }

    /// A shader module: `common.wgsl` followed by the pass's own source. Validation errors are
    /// caught and returned (they are only reported when a pipeline is made).
    fn module(&self, label: &str, src: &str) -> Result<wgpu::ShaderModule, String> {
        let source = format!("{}\n{}", include_str!("common.wgsl"), src);
        self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let m = self.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(label), source: wgpu::ShaderSource::Wgsl(source.into()) });
        match pollster::block_on(self.device.pop_error_scope()) {
            Some(e) => Err(format!("{label}: {e}")),
            None => Ok(m),
        }
    }

    fn compute(&self, module: &wgpu::ShaderModule, layout: &wgpu::PipelineLayout, entry: &str) -> Result<wgpu::ComputePipeline, String> {
        self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let p = self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(entry), layout: Some(layout), module, entry_point: Some(entry), compilation_options: Default::default(), cache: None,
        });
        match pollster::block_on(self.device.pop_error_scope()) {
            Some(e) => Err(format!("{entry}: {e}")),
            None => Ok(p),
        }
    }
}

/// One world's parameters for the GPU passes; `World` in common.wgsl, field for field.
#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct WorldParams {
    pub width: u32, pub height: u32, pub horizon_px: f32, pub center_px: f32,
    pub focal_px: f32, pub eye_height: f32, pub bend: f32, pub hill: f32,
    pub half_width: f32, pub flare: f32, pub near_ground: f32, pub scroll: f32,
    pub top: f32, pub left: f32,
    pub stairs_on: u32, pub st_period: f32, pub st_run: f32, pub st_rise: f32, pub st_steps: f32, pub st_offset: f32,
    pub loop_len: f32, pub tphase: f32, pub far: f32, pub gain: f32,
    pub fog_on: u32, pub fog_dist: f32, pub fog: [f32; 3],
    pub void: [f32; 3],
    pub ambient: [f32; 3],
    pub sky_top: [f32; 3],
    pub sky_hor: [f32; 3],
    pub sky_on: u32, pub match_sky: u32, pub env_fog: u32,
    pub edge_noise: f32, pub edge_dark: f32, pub verge_on: u32, pub walls_on: u32,
    pub walls_gap: f32, pub base_shadow: f32, pub wall_top: f32, pub ceiling_on: u32,
    pub bands: u32,
    pub path_tile: f32, pub verge_tile: f32, pub wall_tile: f32, pub ceil_tile: f32, pub deck_tile: f32, pub bottom_tile: f32,
    pub rotate: u32,
    pub path_gloss: f32, pub path_rip: f32, pub verge_gloss: f32, pub verge_rip: f32, pub deck_gloss: f32, pub deck_rip: f32,
    pub bridge_on: u32, pub br_period: f32, pub br_len: f32, pub br_offset: f32, pub br_floor: f32, pub br_bottom: u32,
    pub bottom: [f32; 3],
    pub deck_base: [f32; 3],
    pub rail: [f32; 3],
    pub fork_on: u32, pub fk_period: f32, pub fk_offset: f32, pub fk_side: u32, pub fk_tan: f32, pub fk_hw: f32, pub fk_per_loop: u32,
    pub fb_on: u32, pub fb_density: f32, pub fb_spacing: f32, pub fb_length: f32, pub fb_offset: f32,
    pub wx_wet: f32, pub wx_puddles: f32, pub wx_snow: f32, pub wx_track: f32, pub wx_rings: f32,
    pub precip_seed: u32, pub loop_seconds: f32,
    pub n_lights: u32, pub n_sky: u32, pub tiles_on: u32, pub tile_cols: u32,
    pub zero: u32, pub fk_style: u32, pub fk_noise: f32, pub pad: u32,
}

/// The sky pass's parameters; `Sky` in sky.wgsl, field for field.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct SkyParams {
    pub craters: [[f32; 4]; 22],
    pub hy: f32, pub ox: f32, pub oy: f32, pub fw: f32,
    pub fhy: f32, pub tile_cols: u32, pub pad0: u32, pub pad1: u32,
    pub stars_on: u32, pub star_bins: u32, pub star_prims: u32, pub md_on: u32,
    pub md: [f32; 3],
    pub au_on: u32, pub au_c: [f32; 3], pub au_t: f32,
    pub au_lo: [f32; 3], pub au_hi: [f32; 3],
    pub au_seed: u32, pub au_p1: f32, pub au_p2: f32, pub au_height: f32, pub au_k: f32,
    pub sun_on: u32, pub s_x: f32, pub s_y: f32, pub s_r: f32, pub s_c: [f32; 3], pub s_spread: f32, pub s_focal: f32,
    pub moon_on: u32, pub m_c: [f32; 3], pub m_gr: f32, pub m_sx: f32, pub m_sy: f32, pub m_litf: f32, pub m_op: f32, pub m_craters: u32,
    pub cl_on: u32, pub cl_col: [f32; 3], pub cl_sun: u32, pub cl_sx: f32, pub cl_sy: f32, pub cl_sc: [f32; 3],
    pub cl_op: f32, pub cl_bins: u32, pub cl_prims: u32,
    pub rb_on: u32, pub rb_cx: f32, pub rb_cy: f32, pub rb_rad: f32, pub rb_band: f32, pub rb_rad2: f32, pub rb_band2: f32, pub rb_k: f32, pub rb_double: u32,
    pub fl_on: u32, pub fl: [f32; 3],
    pub bolt_on: u32, pub b_gain: f32, pub b_core: [f32; 3], pub b_tint: [f32; 3],
    pub b_glow: f32, pub b_scale: f32, pub b_xa: u32, pub b_xb: u32, pub b_yb: u32, pub b_nsegs: u32, pub b_prims: u32,
    pub veil: f32, pub bank_on: u32, pub bank_t: f32, pub bank: [f32; 3],
    pub pad: [u32; 3],
}

/// The sky pass's input: parameters, primitives (stars, blobs, bolt segments; 6 floats each) and
/// the per-tile lists of stars and of blobs.
pub(crate) struct SkyInput { pub params: SkyParams, pub prims: Vec<f32>, pub bins: Vec<u32> }

/// Per-tile lists (16 x 16 tiles over a w x h buffer) of the boxes that touch each tile, in the
/// boxes' order: (tiles + 1) offsets, then indices. Boxes are inclusive pixel ranges.
pub(crate) fn bin_tiles(w: usize, h: usize, boxes: &[(i64, i64, i64, i64)]) -> Vec<u32> {
    let (cols, rows) = (w.div_ceil(16), h.div_ceil(16));
    let mut lists: Vec<Vec<u32>> = vec![Vec::new(); cols * rows];
    for (k, &(x0, y0, x1, y1)) in boxes.iter().enumerate() {
        if x1 < 0 || y1 < 0 || x0 >= w as i64 || y0 >= h as i64 || x1 < x0 || y1 < y0 { continue; }
        let (tx0, tx1) = (x0.max(0) as usize / 16, (x1 as usize).min(w - 1) / 16);
        let (ty0, ty1) = (y0.max(0) as usize / 16, (y1 as usize).min(h - 1) / 16);
        for ty in ty0..=ty1 { for tx in tx0..=tx1 { lists[ty * cols + tx].push(k as u32); } }
    }
    let mut out = Vec::with_capacity(lists.len() + 1);
    let mut off = 0u32;
    for l in &lists { out.push(off); off += l.len() as u32; }
    out.push(off);
    for l in lists { out.extend(l); }
    out
}

/// One billboard for the card pass; `Card` in cards.wgsl, field for field. The sprite levels
/// (`sw`..`goff`) are filled in by `Frame::cards` from the sprite store.
#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct CardGpu {
    pub sx: f32, pub wp: f32, pub hp: f32, pub sy_top: f32, pub sway: f32, pub z: f32,
    pub x0: i32, pub x1: i32, pub y0: i32, pub y1: i32,
    pub light: [f32; 3],
    pub snow_on: u32, pub snow_cap: f32, pub snow: [f32; 3],
    pub nearest: u32, pub flip: u32, pub plain: u32, pub fog_t: f32,
    pub fog: [f32; 3], pub fog_mul: f32, pub fog_add: [f32; 3],
    pub emissive: f32,
    pub sw: u32, pub sh: u32, pub soff: u32, pub gw: u32, pub gh: u32, pub goff: u32,
    pub id: u32, pub realm: u32, pub world: [f32; 3], pub source: u32,
}

/// A card and the sprite levels it samples (chosen on the CPU, as `Sprite::sample` would).
pub(crate) struct CardIn { pub card: CardGpu, pub sprite: Arc<Sprite>, pub lod: usize, pub glow: Option<(Arc<Sprite>, usize)> }

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CardPassParams { tile_cols: u32, pick: u32, n_cards: u32, pad: u32 }

/// Every sprite level the cards have used, as RGBA texels in one buffer; sprites are kept alive
/// here so their pointers stay unique. Grows when a new sprite appears.
#[derive(Default)]
struct SpriteStore {
    held: Vec<Arc<Sprite>>,
    /// Pointer -> first texel of each level.
    at: std::collections::HashMap<usize, Vec<u32>>,
    texels: Vec<[f32; 4]>,
    buf: Option<wgpu::Buffer>,
    dirty: bool,
}

impl SpriteStore {
    fn level(&mut self, s: &Arc<Sprite>, lod: usize) -> (u32, u32, u32) {
        let key = Arc::as_ptr(s) as usize;
        if !self.at.contains_key(&key) {
            let mut offs = Vec::with_capacity(s.levels.len());
            for (_, _, data) in &s.levels { offs.push(self.texels.len() as u32); self.texels.extend_from_slice(data); }
            self.at.insert(key, offs);
            self.held.push(s.clone());
            self.dirty = true;
        }
        let (w, h, _) = &s.levels[lod];
        (*w as u32, *h as u32, self.at[&key][lod])
    }
}

/// One splat for the splat pass; `Splat` in splat.wgsl (kind, depth, gate pixel and depth, then
/// the shape's numbers, integers as their bits).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct SplatGpu { pub kind: u32, pub z: f32, pub gx: i32, pub gy: i32, pub gcz: f32, pub f: [f32; 19] }

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SplatPassParams { tile_cols: u32, n: u32, pad: [u32; 2] }

/// Download targets: any of the frame's buffers, fetched in one wait.
#[derive(Default)]
pub(crate) struct Down<'a> {
    pub gbuf: Option<&'a mut [super::raster::GPixel]>,
    pub hdr: Option<&'a mut [[f32; 3]]>,
    pub refl: Option<&'a mut [[f32; 6]]>,
    pub pick: Option<&'a mut [u16]>,
}

/// What the shading pass reads besides the G-buffer and masks.
pub(crate) struct ShadeInput<'a> {
    /// Path, verge, wall, ceiling, deck, bottom.
    pub textures: [&'a Arc<Texture>; 6],
    /// Point lights: pos (camera space), radius, colour.
    pub lights: Vec<[f32; 8]>,
    /// Sky lights: direction, colour.
    pub sky: Vec<[f32; 8]>,
    /// Per-tile light lists (16 x 16 tiles), when used.
    pub tiles: Option<Vec<Vec<u16>>>,
}

/// The CPU G-buffer pixel packed as the GPU reads it.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct GPixGpu { pub depth: f32, pub x: f32, pub y: f32, pub d: f32, pub idr: u32 }

struct Sized {
    w: usize, h: usize,
    gbuf: wgpu::Buffer, masks: wgpu::Buffer, uvs: wgpu::Buffer, hdr: wgpu::Buffer, refl: wgpu::Buffer,
    pick: wgpu::Buffer,
    /// The reflection pass's per-column tables.
    q2s: wgpu::Buffer, mins: wgpu::Buffer, ends: wgpu::Buffer, blocks: wgpu::Buffer,
    read: wgpu::Buffer,
}

struct SkyPass { layout: wgpu::BindGroupLayout, pipe: wgpu::ComputePipeline, params: wgpu::Buffer, prims: Mutex<Option<wgpu::Buffer>>, bins: Mutex<Option<wgpu::Buffer>> }

struct CardPass { layout: wgpu::BindGroupLayout, pipe: wgpu::ComputePipeline, params: wgpu::Buffer, cards: Mutex<Option<wgpu::Buffer>>, bins: Mutex<Option<wgpu::Buffer>>, store: Mutex<SpriteStore> }

struct SplatPass { layout: wgpu::BindGroupLayout, pipe: wgpu::ComputePipeline, params: [wgpu::Buffer; 2], splats: [Mutex<Option<wgpu::Buffer>>; 2], bins: [Mutex<Option<wgpu::Buffer>>; 2] }

/// Timestamp queries: a pair per pass (start, end), read back when the frame is profiled.
struct Timestamps { set: wgpu::QuerySet, resolve: wgpu::Buffer, read: wgpu::Buffer }
const TS_PASSES: u32 = 16;

/// The light shafts' settings; `Shafts` in shafts.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct ShaftParams {
    pub mw: u32, pub mh: u32, pub sun_on: u32, pub lamps_on: u32,
    pub sx: f32, pub sy: f32, pub glow_r: f32, pub k_sun: f32,
    pub sun: [f32; 3], pub k_lamp: f32,
    pub tc: u32, pub n_lights: u32, pub hy: f32, pub pad: u32,
}

struct ShaftPass { layout: wgpu::BindGroupLayout, mask: wgpu::ComputePipeline, add: wgpu::ComputePipeline, apply: wgpu::ComputePipeline, params: wgpu::Buffer, lists: Mutex<Option<wgpu::Buffer>>, cells: Mutex<Option<wgpu::Buffer>> }

/// Post and style's settings and buffer plan; `Post` in post.wgsl, field for field.
#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct PostParams {
    pub w: u32, pub h: u32, pub gl: u32, pub gt: u32,
    pub out_w: u32, pub out_h: u32, pub px: u32, pub pick_on: u32,
    pub o_pg: u32, pub o_pk: u32, pub o_q0: u32, pub o_q1: u32,
    pub o_ks: u32, pub o_red: u32, pub qw: u32, pub qh: u32,
    pub h_crop: u32, pub h_shim: u32, pub h_lsrc: u32, pub h_ldst: u32,
    pub h_fin: u32, pub r_fin: u32, pub r_kuw: u32, pub r_mid: u32,
    pub r_wdst: u32, pub r_last: u32, pub hz: f32, pub unit: f32,
    pub sh_on: u32, pub sh_strength: f32, pub sh_c1: f32, pub sh_c2: f32,
    pub lens_kind: u32, pub lens_weight: f32, pub n_drops: u32, pub lens_seed: u32,
    pub th: f32, pub tw: f32, pub aspect: f32, pub pad0: u32,
    pub post_on: u32, pub bloom: f32, pub exposure: f32, pub grain_seed: u32,
    pub tint_r: f32, pub tint_g: f32, pub tint_b: f32, pub saturation: f32,
    pub contrast: f32, pub vignette: f32, pub grain: f32, pub kuw_r: i32,
    pub grade: u32, pub grade_n: u32, pub grade_s: f32, pub ol_on: u32,
    pub ol_objects_only: u32, pub ol_r: f32, pub ol_g: f32, pub ol_b: f32,
    pub lw_on: u32, pub lw_amp: f32, pub lw_den: f32, pub lw_blend_top: f32,
    pub lw_span: f32, pub lw_nearest: u32, pub ramp_on: u32, pub ramp_lo: f32,
    pub ramp_k: f32, pub q_on: u32, pub q_bits: u32, pub q_amp: f32,
    pub dither: u32, pub paper: f32, pub scan: f32, pub stats_on: u32,
    pub sky_enabled: u32, pub verge_enabled: u32, pub t_lut: u32, pub t_col: u32,
    pub t_cube: u32, pub t_drops: u32, pub pad1: u32, pub pad2: u32,
}

/// What post needs besides its settings: the lens drops, the palette and grade tables (cached by
/// `key`), and what to bring back.
pub(crate) struct PostIn<'a> {
    pub params: PostParams,
    pub drops: Vec<[f32; 5]>,
    pub key: usize,
    pub lut: Option<(u32, &'a [u8], Vec<u32>)>,
    pub cube: Option<&'a [[f32; 3]]>,
    pub kuwahara: bool, pub shimmer: bool, pub warp: bool, pub bloom: bool,
    pub want_depth: bool,
}

/// Post's results: the output bytes, and when asked the pick ids, the depth and per-row stats.
pub(crate) struct PostOut { pub rgba: Vec<u8>, pub pick: Option<Vec<u16>>, pub depth: Option<Vec<f32>>, pub stats: Option<Vec<f32>> }

struct PostPass {
    layout: wgpu::BindGroupLayout,
    pipes: std::collections::HashMap<&'static str, wgpu::ComputePipeline>,
    params: wgpu::Buffer,
    scratch: Mutex<Option<wgpu::Buffer>>,
    out: Mutex<Option<wgpu::Buffer>>,
    read: Mutex<Option<wgpu::Buffer>>,
    tables: Mutex<Option<(usize, wgpu::Buffer, u32, u32, u32)>>,
}

struct ReflectPass { layout: wgpu::BindGroupLayout, colsum_a: wgpu::ComputePipeline, colsum_b: wgpu::ComputePipeline, reflect: wgpu::ComputePipeline }

struct ShadePass {
    layout: wgpu::BindGroupLayout,
    uvs: wgpu::ComputePipeline,
    shade: wgpu::ComputePipeline,
    mist: wgpu::ComputePipeline,
    air: wgpu::Buffer,
}

/// The air's settings for the mist pass; `Air` in shade.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct AirParams {
    pub mist_on: u32, pub density: f32, pub top: f32, pub patch: f32,
    pub travel: f32, pub seed: u32, pub mist: [f32; 3], pub pad: [u32; 3],
}

/// The GPU side of a `WorldRenderer`: pipelines, cached textures, and buffers for the frame size.
pub struct Gpu {
    pub ctx: Arc<GpuContext>,
    shade: ShadePass,
    sky: SkyPass,
    cards: CardPass,
    splat: SplatPass,
    reflect: ReflectPass,
    shafts: ShaftPass,
    post: PostPass,
    ts: Option<Timestamps>,
    params: wgpu::Buffer,
    sized: Mutex<Option<Sized>>,
    /// Texel buffer and the textures it holds (by pointer).
    textures: Mutex<Option<(Vec<usize>, wgpu::Buffer)>>,
    lights: Mutex<Option<wgpu::Buffer>>,
    tiles: Mutex<Option<wgpu::Buffer>>,
}

fn storage(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding, visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None },
        count: None,
    }
}
fn uniform(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding, visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
        count: None,
    }
}

impl Gpu {
    /// Every pipeline, made up front: a shader that fails validation fails here, not mid-frame.
    pub fn new(ctx: Arc<GpuContext>) -> Result<Gpu, String> {
        let d = &ctx.device;
        let layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shade"),
            entries: &[uniform(0), storage(1, true), storage(2, true), storage(3, true), storage(4, true), storage(5, true), storage(6, false), storage(7, false), storage(8, false), uniform(9)],
        });
        let pl = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("shade"), bind_group_layouts: &[&layout], push_constant_ranges: &[] });
        let m = ctx.module("shade.wgsl", include_str!("shade.wgsl"))?;
        let air = d.create_buffer(&wgpu::BufferDescriptor {
            label: Some("air params"), size: std::mem::size_of::<AirParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false,
        });
        let shade = ShadePass { uvs: ctx.compute(&m, &pl, "uvs_main")?, shade: ctx.compute(&m, &pl, "shade_main")?, mist: ctx.compute(&m, &pl, "mist_main")?, layout, air };
        let sky = {
            let layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("sky"), entries: &[uniform(0), uniform(1), storage(2, true), storage(3, false), storage(4, true), storage(5, true)],
            });
            let pl = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("sky"), bind_group_layouts: &[&layout], push_constant_ranges: &[] });
            let m = ctx.module("sky.wgsl", include_str!("sky.wgsl"))?;
            let pipe = ctx.compute(&m, &pl, "sky_main")?;
            let params = d.create_buffer(&wgpu::BufferDescriptor {
                label: Some("sky params"), size: std::mem::size_of::<SkyParams>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false,
            });
            SkyPass { layout, pipe, params, prims: Mutex::new(None), bins: Mutex::new(None) }
        };
        let cards = {
            let layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("cards"), entries: &[uniform(0), storage(1, false), storage(2, false), storage(3, true), storage(4, true), storage(5, true), storage(6, false), uniform(7)],
            });
            let pl = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("cards"), bind_group_layouts: &[&layout], push_constant_ranges: &[] });
            let m = ctx.module("cards.wgsl", include_str!("cards.wgsl"))?;
            let pipe = ctx.compute(&m, &pl, "cards_main")?;
            let params = d.create_buffer(&wgpu::BufferDescriptor {
                label: Some("card params"), size: 16, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false,
            });
            CardPass { layout, pipe, params, cards: Mutex::new(None), bins: Mutex::new(None), store: Mutex::new(SpriteStore::default()) }
        };
        let splat = {
            let layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("splats"), entries: &[uniform(0), storage(1, false), storage(2, false), storage(3, true), storage(4, true), uniform(5)],
            });
            let pl = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("splats"), bind_group_layouts: &[&layout], push_constant_ranges: &[] });
            let m = ctx.module("splat.wgsl", include_str!("splat.wgsl"))?;
            let pipe = ctx.compute(&m, &pl, "splat_main")?;
            let pbuf = || d.create_buffer(&wgpu::BufferDescriptor {
                label: Some("splat params"), size: 16, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false,
            });
            SplatPass { layout, pipe, params: [pbuf(), pbuf()], splats: [Mutex::new(None), Mutex::new(None)], bins: [Mutex::new(None), Mutex::new(None)] }
        };
        let reflect = {
            let layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("reflect"), entries: &[uniform(0), storage(1, true), storage(2, false), storage(3, true), storage(4, false), storage(5, false), storage(6, false), storage(7, false)],
            });
            let pl = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("reflect"), bind_group_layouts: &[&layout], push_constant_ranges: &[] });
            let m = ctx.module("reflect.wgsl", include_str!("reflect.wgsl"))?;
            ReflectPass { colsum_a: ctx.compute(&m, &pl, "colsum_a")?, colsum_b: ctx.compute(&m, &pl, "colsum_b")?, reflect: ctx.compute(&m, &pl, "reflect_main")?, layout }
        };
        let shafts = {
            let layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("shafts"), entries: &[uniform(0), storage(1, true), storage(2, false), storage(3, true), storage(4, true), storage(5, false), uniform(6)],
            });
            let pl = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("shafts"), bind_group_layouts: &[&layout], push_constant_ranges: &[] });
            let m = ctx.module("shafts.wgsl", include_str!("shafts.wgsl"))?;
            let params = d.create_buffer(&wgpu::BufferDescriptor {
                label: Some("shaft params"), size: std::mem::size_of::<ShaftParams>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false,
            });
            ShaftPass {
                mask: ctx.compute(&m, &pl, "shaft_mask")?, add: ctx.compute(&m, &pl, "shaft_add")?, apply: ctx.compute(&m, &pl, "shaft_apply")?,
                layout, params, lists: Mutex::new(None), cells: Mutex::new(None),
            }
        };
        let post = {
            let layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("post"), entries: &[uniform(0), uniform(1), storage(2, true), storage(3, true), storage(4, true), storage(5, false), storage(6, true), storage(7, false)],
            });
            let pl = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("post"), bind_group_layouts: &[&layout], push_constant_ranges: &[] });
            let m = ctx.module("post.wgsl", include_str!("post.wgsl"))?;
            let mut pipes = std::collections::HashMap::new();
            for e in ["post_crop", "post_shimmer", "post_frost_rows", "post_frost_mean", "post_lens", "post_bloom_down", "post_blur_h", "post_blur_v",
                      "post_finish", "post_kuw_h", "post_kuw_v", "post_grade_outline", "post_warp", "post_quant", "post_stats", "post_out"] {
                pipes.insert(e, ctx.compute(&m, &pl, e)?);
            }
            let params = d.create_buffer(&wgpu::BufferDescriptor {
                label: Some("post params"), size: std::mem::size_of::<PostParams>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false,
            });
            PostPass { layout, pipes, params, scratch: Mutex::new(None), out: Mutex::new(None), read: Mutex::new(None), tables: Mutex::new(None) }
        };
        let params = d.create_buffer(&wgpu::BufferDescriptor {
            label: Some("world params"), size: std::mem::size_of::<WorldParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false,
        });
        let ts = d.features().contains(wgpu::Features::TIMESTAMP_QUERY).then(|| Timestamps {
            set: d.create_query_set(&wgpu::QuerySetDescriptor { label: Some("pass times"), ty: wgpu::QueryType::Timestamp, count: TS_PASSES * 2 }),
            resolve: d.create_buffer(&wgpu::BufferDescriptor { label: Some("pass times"), size: (TS_PASSES * 16) as u64, usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false }),
            read: d.create_buffer(&wgpu::BufferDescriptor { label: Some("pass times read"), size: (TS_PASSES * 16) as u64, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }),
        });
        Ok(Gpu { ctx, shade, sky, cards, splat, reflect, shafts, post, ts, params, sized: Mutex::new(None), textures: Mutex::new(None), lights: Mutex::new(None), tiles: Mutex::new(None) })
    }

    fn buffer(&self, label: &str, size: u64, usage: wgpu::BufferUsages) -> wgpu::Buffer {
        self.ctx.device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: size.max(16), usage, mapped_at_creation: false })
    }

    fn sized(&self, w: usize, h: usize) -> std::sync::MutexGuard<'_, Option<Sized>> {
        let mut s = self.sized.lock().unwrap();
        if !s.as_ref().is_some_and(|s| s.w == w && s.h == h) {
            let n = (w * h) as u64;
            use wgpu::BufferUsages as U;
            let st = U::STORAGE | U::COPY_DST | U::COPY_SRC;
            *s = Some(Sized {
                w, h,
                gbuf: self.buffer("gbuf", n * 20, st),
                masks: self.buffer("masks", n * 8, st),
                uvs: self.buffer("uvs", n * 12, st),
                hdr: self.buffer("hdr", n * 12, st),
                refl: self.buffer("refl", n * 24, st),
                pick: self.buffer("pick", n * 4, st),
                q2s: self.buffer("column sums", (w * (h + 1) * 3 * 8) as u64, st),
                mins: self.buffer("column minima", (w * (h.div_ceil(8) + h.div_ceil(64)) * 4) as u64, st),
                ends: self.buffer("column ends", (w * 12 * 4) as u64, st),
                blocks: self.buffer("column blocks", (w * h.div_ceil(64) * 3 * 2 * 8) as u64, st),
                read: self.buffer("readback", n * 64, U::MAP_READ | U::COPY_DST),
            });
        }
        s
    }

    /// A storage buffer kept between frames, grown when the data no longer fits.
    fn upload(&self, slot: &Mutex<Option<wgpu::Buffer>>, label: &str, bytes: &[u8]) -> wgpu::Buffer {
        let mut s = slot.lock().unwrap();
        let need = (bytes.len() as u64).max(16).next_multiple_of(4);
        if !s.as_ref().is_some_and(|b| b.size() >= need) {
            *s = Some(self.buffer(label, need.next_power_of_two(), wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST));
        }
        let b = s.as_ref().unwrap().clone();
        if !bytes.is_empty() {
            let mut padded = bytes.to_vec();
            padded.resize(bytes.len().next_multiple_of(4), 0);
            self.ctx.queue.write_buffer(&b, 0, &padded);
        }
        b
    }

    /// The six textures as one texel buffer: a header of (offset, side) per slot and level and the
    /// level counts, then every level's linear RGB. Rebuilt only when a texture changes.
    fn texel_buffer(&self, textures: [&Arc<Texture>; 6]) -> wgpu::Buffer {
        let key: Vec<usize> = textures.iter().map(|t| Arc::as_ptr(t) as usize).collect();
        let mut s = self.textures.lock().unwrap();
        if let Some((k, b)) = s.as_ref() { if *k == key { return b.clone(); } }
        let mut header = vec![0u32; 136];
        let mut data: Vec<f32> = Vec::new();
        for (slot, t) in textures.iter().enumerate() {
            header[128 + slot] = t.levels.len().min(8) as u32;
            for (l, (side, texels)) in t.levels.iter().take(8).enumerate() {
                header[(slot * 8 + l) * 2] = data.len() as u32;
                header[(slot * 8 + l) * 2 + 1] = *side as u32;
                data.extend(texels.iter().flat_map(|c| c.iter().copied()));
            }
        }
        let mut words: Vec<u32> = header;
        words.extend(data.iter().map(|f| f.to_bits()));
        let b = self.buffer("texels", words.len() as u64 * 4, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST);
        self.ctx.queue.write_buffer(&b, 0, bytemuck::cast_slice(&words));
        *s = Some((key, b.clone()));
        b
    }

    /// Start a frame on the GPU: its buffers, and the world's parameters.
    pub(crate) fn frame(&self, params: &WorldParams) -> Frame<'_> {
        let (w, h) = (params.width as usize, params.height as usize);
        let sz = self.sized(w, h);
        self.ctx.queue.write_buffer(&self.params, 0, bytemuck::bytes_of(params));
        Frame { g: self, sz, w, h, enc: None, profile: false, passes: Vec::new(), shade_bg: None, last: None }
    }

        /// Map `buf` and hand its first `len` bytes to `f` (zeros if the map failed).
    /// Waits for submission `after` (this frame's last), not for other frames sharing the device.
    fn read_with(&self, buf: &wgpu::Buffer, len: u64, after: Option<wgpu::SubmissionIndex>, f: impl FnOnce(&[u8])) {
        let slice = buf.slice(..len);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| { let _ = tx.send(r); });
        let _ = self.ctx.device.poll(match after { Some(i) => wgpu::PollType::WaitForSubmissionIndex(i), None => wgpu::PollType::Wait });
        let got = match rx.try_recv() {
            Ok(r) => Ok(r),
            Err(_) => { let _ = self.ctx.device.poll(wgpu::PollType::Wait); rx.recv() }
        };
        if matches!(got, Ok(Ok(()))) {
            f(&slice.get_mapped_range());
            buf.unmap();
        } else {
            f(&vec![0u8; len as usize]);
        }
    }
    fn read(&self, buf: &wgpu::Buffer, len: u64) -> Vec<u8> {
        let slice = buf.slice(..len);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| { let _ = tx.send(r); });
        let _ = self.ctx.device.poll(wgpu::PollType::Wait);
        let ok = matches!(rx.recv(), Ok(Ok(())));
        let out = if ok { slice.get_mapped_range().to_vec() } else { vec![0u8; len as usize] };
        if ok { buf.unmap(); }
        out
    }
}

/// One frame's work on the GPU. Passes are recorded in order; the frame's buffers stay on the
/// GPU between them, and the CPU waits only when it reads one back (`get_*`). Uploads made after
/// a pass first send what was recorded, so they never overtake it.
pub(crate) struct Frame<'g> {
    g: &'g Gpu,
    sz: std::sync::MutexGuard<'g, Option<Sized>>,
    w: usize,
    h: usize,
    enc: Option<wgpu::CommandEncoder>,
    /// Time each pass on the GPU (where the adapter can); `passes` names them in order.
    pub profile: bool,
    passes: Vec<&'static str>,
    shade_bg: Option<wgpu::BindGroup>,
    last: Option<wgpu::SubmissionIndex>,
}

impl<'g> Frame<'g> {
    fn n(&self) -> usize { self.w * self.h }
    fn bufs(&self) -> &Sized { self.sz.as_ref().unwrap() }
    fn enc(&mut self) -> &mut wgpu::CommandEncoder {
        let d = &self.g.ctx.device;
        self.enc.get_or_insert_with(|| d.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") }))
    }
    /// Send what has been recorded (without waiting for it).
    fn submit(&mut self) {
        if let Some(e) = self.enc.take() { self.last = Some(self.g.ctx.queue.submit([e.finish()])); }
    }
    fn write(&mut self, buf: &wgpu::Buffer, offset: u64, data: &[u8]) {
        self.submit();
        self.g.ctx.queue.write_buffer(buf, offset, data);
    }
    /// Copy `len` bytes of `src` back to the CPU (waits for everything recorded before).
    fn read(&mut self, src: &wgpu::Buffer, len: u64) -> Vec<u8> {
        let read = self.bufs().read.clone();
        self.enc().copy_buffer_to_buffer(src, 0, &read, 0, len);
        self.submit();
        self.g.read(&read, len)
    }

    pub fn put_gbuf(&mut self, gbuf: &[super::raster::GPixel]) {
        use rayon::prelude::*;
        let packed: Vec<GPixGpu> = gbuf.par_iter().map(|p| GPixGpu { depth: p.depth, x: p.x, y: p.y, d: p.d, idr: p.id as u32 | (p.realm as u32) << 8 }).collect();
        let b = self.bufs().gbuf.clone();
        self.write(&b, 0, bytemuck::cast_slice(&packed));
    }
    pub fn get_gbuf(&mut self, gbuf: &mut [super::raster::GPixel]) {
        use rayon::prelude::*;
        let b = self.bufs().gbuf.clone();
        let bytes = self.read(&b, (self.n() * 20) as u64);
        let packed: &[GPixGpu] = bytemuck::cast_slice(&bytes);
        gbuf.par_iter_mut().zip(packed.par_iter()).for_each(|(p, q)| {
            *p = super::raster::GPixel { depth: q.depth, id: (q.idr & 0xFF) as u8, x: q.x, y: q.y, d: q.d, realm: (q.idr >> 8) as u8 };
        });
    }
    pub fn put_masks(&mut self, sun_mask: &[f32], ao_mask: &[f32]) {
        let (b, n) = (self.bufs().masks.clone(), self.n());
        self.write(&b, 0, bytemuck::cast_slice(sun_mask));
        self.g.ctx.queue.write_buffer(&b, (n * 4) as u64, bytemuck::cast_slice(ao_mask));
    }
    pub fn put_hdr(&mut self, hdr: &[[f32; 3]]) {
        let b = self.bufs().hdr.clone();
        self.write(&b, 0, bytemuck::cast_slice(hdr));
    }
    pub fn get_hdr(&mut self, hdr: &mut [[f32; 3]]) {
        let b = self.bufs().hdr.clone();
        let bytes = self.read(&b, (self.n() * 12) as u64);
        hdr.copy_from_slice(bytemuck::cast_slice(&bytes));
    }
    pub fn put_refl(&mut self, refl: &[[f32; 6]]) {
        let b = self.bufs().refl.clone();
        self.write(&b, 0, bytemuck::cast_slice(refl));
    }
    pub fn get_refl(&mut self, refl: &mut [[f32; 6]]) {
        let b = self.bufs().refl.clone();
        let bytes = self.read(&b, (self.n() * 24) as u64);
        refl.copy_from_slice(bytemuck::cast_slice(&bytes));
    }

    /// Fetch any of the frame's buffers in one wait.
    pub fn download(&mut self, d: Down) {
        let n = self.n() as u64;
        let (sz_g, sz_h, sz_r, sz_p) = { let b = self.bufs(); (b.gbuf.clone(), b.hdr.clone(), b.refl.clone(), b.pick.clone()) };
        let read = self.bufs().read.clone();
        let mut at = 0u64;
        let mut parts: Vec<(u64, u64)> = Vec::new();
        for (on, src, len) in [(d.gbuf.is_some(), &sz_g, n * 20), (d.hdr.is_some(), &sz_h, n * 12), (d.refl.is_some(), &sz_r, n * 24), (d.pick.is_some(), &sz_p, n * 4)] {
            if on { self.enc().copy_buffer_to_buffer(src, 0, &read, at, len); parts.push((at, len)); at += len; } else { parts.push((0, 0)); }
        }
        if at == 0 { return; }
        self.submit();
        let Down { gbuf, hdr, refl, pick } = d;
        self.g.read_with(&read, at, self.last.clone(), |bytes| {
            use rayon::prelude::*;
            let part = |k: usize| { let (o, l) = parts[k]; &bytes[o as usize..(o + l) as usize] };
            if let Some(g) = gbuf {
                let packed: &[GPixGpu] = bytemuck::cast_slice(part(0));
                g.par_iter_mut().zip(packed.par_iter()).for_each(|(p, q)| {
                    *p = super::raster::GPixel { depth: q.depth, id: (q.idr & 0xFF) as u8, x: q.x, y: q.y, d: q.d, realm: (q.idr >> 8) as u8 };
                });
            }
            if let Some(h) = hdr { bytemuck::cast_slice_mut::<_, u8>(h).copy_from_slice(part(1)); }
            if let Some(r) = refl { bytemuck::cast_slice_mut::<_, u8>(r).copy_from_slice(part(2)); }
            if let Some(p) = pick {
                let src: &[u32] = bytemuck::cast_slice(part(3));
                p.par_iter_mut().zip(src.par_iter()).for_each(|(a, b)| *a = *b as u16);
            }
        });
    }

    /// Billboards drawn far to near (hdr, gbuf and, with `pick`, the pick ids, in place).
    pub fn cards(&mut self, list: &[CardIn], bins: &[u32], pick: bool) {
        let g = self.g;
        let mut cs: Vec<CardGpu> = Vec::with_capacity(list.len().max(1));
        let texels = {
            let mut st = g.cards.store.lock().unwrap();
            for c in list {
                let mut cd = c.card;
                (cd.sw, cd.sh, cd.soff) = st.level(&c.sprite, c.lod);
                (cd.gw, cd.gh, cd.goff) = match &c.glow { Some((gs, lod)) => st.level(gs, *lod), None => (1, 1, u32::MAX) };
                cs.push(cd);
            }
            if st.dirty || st.buf.is_none() {
                let mut t = st.texels.clone();
                if t.is_empty() { t.push([0.0; 4]); }
                let b = g.buffer("sprites", (t.len() * 16) as u64, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST);
                self.submit();
                g.ctx.queue.write_buffer(&b, 0, bytemuck::cast_slice(&t));
                st.buf = Some(b);
                st.dirty = false;
            }
            st.buf.clone().unwrap()
        };
        if cs.is_empty() { cs.push(CardGpu::default()); }
        self.submit();
        let pp = CardPassParams { tile_cols: (self.w as u32).div_ceil(16), pick: pick as u32, n_cards: list.len() as u32, pad: 0 };
        g.ctx.queue.write_buffer(&g.cards.params, 0, bytemuck::bytes_of(&pp));
        let cards = g.upload(&g.cards.cards, "cards", bytemuck::cast_slice(&cs));
        let bins = g.upload(&g.cards.bins, "card bins", bytemuck::cast_slice(bins));
        let sz = self.bufs();
        let (gb, hd, pk) = (sz.gbuf.clone(), sz.hdr.clone(), sz.pick.clone());
        let bg = g.ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cards"), layout: &g.cards.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: g.params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: gb.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: hd.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: texels.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: cards.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: bins.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 6, resource: pk.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 7, resource: g.cards.params.as_entire_binding() },
            ],
        });
        let (gx, gy) = self.groups();
        let e = self.enc();
        if pick { e.clear_buffer(&pk, 0, None); }
        let mut p = self.pass("g.cards");
        p.set_bind_group(0, &bg, &[]);
        p.set_pipeline(&g.cards.pipe);
        p.dispatch_workgroups(gx, gy, 1);
    }

    /// Splats drawn in order (hdr, and the G-buffer for blades, in place). `slot` 0 or 1: the two
    /// dispatches of a frame (blades, then the rest) keep separate buffers.
    pub fn splats(&mut self, slot: usize, list: &[SplatGpu], bins: &[u32]) {
        if list.is_empty() { return; }
        let g = self.g;
        self.submit();
        let pp = SplatPassParams { tile_cols: (self.w as u32).div_ceil(16), n: list.len() as u32, pad: [0; 2] };
        g.ctx.queue.write_buffer(&g.splat.params[slot], 0, bytemuck::bytes_of(&pp));
        let sp = g.upload(&g.splat.splats[slot], "splats", bytemuck::cast_slice(list));
        let bn = g.upload(&g.splat.bins[slot], "splat bins", bytemuck::cast_slice(bins));
        let sz = self.bufs();
        let (gb, hd) = (sz.gbuf.clone(), sz.hdr.clone());
        let bg = g.ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("splats"), layout: &g.splat.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: g.params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: gb.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: hd.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: sp.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: bn.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: g.splat.params[slot].as_entire_binding() },
            ],
        });
        let (gx, gy) = self.groups();
        let mut p = self.pass(if slot == 0 { "g.blades" } else { "g.splats" });
        p.set_bind_group(0, &bg, &[]);
        p.set_pipeline(&g.splat.pipe);
        p.dispatch_workgroups(gx, gy, 1);
    }

    /// Mirror reflections on glossy floors (hdr in place, from refl).
    pub fn reflect(&mut self) {
        let g = self.g;
        let sz = self.bufs();
        let bg = g.ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("reflect"), layout: &g.reflect.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: g.params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: sz.gbuf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: sz.hdr.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: sz.refl.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: sz.q2s.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: sz.mins.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 6, resource: sz.ends.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 7, resource: sz.blocks.as_entire_binding() },
            ],
        });
        let (gx, gy) = self.groups();
        let (cols, blks) = ((self.w as u32).div_ceil(64), (self.h as u32).div_ceil(64));
        {
            let mut p = self.pass("g.colsum");
            p.set_bind_group(0, &bg, &[]);
            p.set_pipeline(&g.reflect.colsum_a);
            p.dispatch_workgroups(cols, blks, 1);
            p.set_pipeline(&g.reflect.colsum_b);
            p.dispatch_workgroups(cols, 1, 1);
        }
        let mut p = self.pass("g.reflect");
        p.set_bind_group(0, &bg, &[]);
        p.set_pipeline(&g.reflect.reflect);
        p.dispatch_workgroups(gx, gy, 1);
    }

    /// Mist over the ground (hdr in place). Needs `shade` to have run this frame (its lights).
    pub fn mist(&mut self, air: &AirParams) {
        let g = self.g;
        let Some(bg) = self.shade_bg.clone() else { return };
        self.submit();
        g.ctx.queue.write_buffer(&g.shade.air, 0, bytemuck::bytes_of(air));
        let (gx, gy) = self.groups();
        let mut p = self.pass("g.mist");
        p.set_bind_group(0, &bg, &[]);
        p.set_pipeline(&g.shade.mist);
        p.dispatch_workgroups(gx, gy, 1);
    }

    /// Light shafts (hdr in place). `lists`: per tile of 16 x 16 cells, the lamps that can light it.
    /// Needs `shade` to have run this frame (its lights).
    pub fn shafts(&mut self, sp: &ShaftParams, lists: &[Vec<u16>]) {
        let g = self.g;
        self.submit();
        g.ctx.queue.write_buffer(&g.shafts.params, 0, bytemuck::bytes_of(sp));
        let mut lw: Vec<u32> = Vec::new();
        let mut off = 0u32;
        for l in lists { lw.push(off); off += l.len() as u32; }
        lw.push(off);
        for l in lists { lw.extend(l.iter().map(|&k| k as u32)); }
        let lists = g.upload(&g.shafts.lists, "shaft lists", bytemuck::cast_slice(&lw));
        let cells = {
            let mut c = g.shafts.cells.lock().unwrap();
            let need = (sp.mw * sp.mh * 4 * 4) as u64;
            if !c.as_ref().is_some_and(|b| b.size() >= need) {
                *c = Some(g.buffer("shaft cells", need, wgpu::BufferUsages::STORAGE));
            }
            c.clone().unwrap()
        };
        let lights = g.lights.lock().unwrap().clone().unwrap_or_else(|| g.buffer("no lights", 16, wgpu::BufferUsages::STORAGE));
        let sz = self.bufs();
        let bg = g.ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shafts"), layout: &g.shafts.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: g.params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: sz.gbuf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: sz.hdr.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: lights.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: lists.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: cells.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 6, resource: g.shafts.params.as_entire_binding() },
            ],
        });
        let (gx, gy) = self.groups();
        let (cx, cy) = (sp.mw.div_ceil(8), sp.mh.div_ceil(8));
        let mut p = self.pass("g.shafts");
        p.set_bind_group(0, &bg, &[]);
        p.set_pipeline(&g.shafts.mask);
        p.dispatch_workgroups(cx, cy, 1);
        p.set_pipeline(&g.shafts.add);
        p.dispatch_workgroups(cx, cy, 1);
        p.set_pipeline(&g.shafts.apply);
        p.dispatch_workgroups(gx, gy, 1);
    }

    /// The scratch post needs for a w x h frame, in words (None if it would pass the device's
    /// largest storage binding: post then runs on the CPU).
    pub fn post_words(&self, w: usize, h: usize, kuwahara: bool) -> Option<usize> {
        let n = w * h;
        let (qw, qh) = (w.div_ceil(4), h.div_ceil(4));
        let words = 3 * n * 2 + 5 * n + n + 3 * n * 2 + if kuwahara { 14 * n } else { 0 } + 6 * qw * qh + h * 23 + 1 + 30 * 5 + 16;
        ((words * 4) as u64 <= self.g.ctx.device.limits().max_storage_buffer_binding_size as u64).then_some(words)
    }

    /// Post and style, to output bytes (and pick, depth, stats when asked): the end of the frame.
    pub fn post(&mut self, mut pin: PostIn) -> PostOut {
        let g = self.g;
        let p = &mut pin.params;
        let (w, h) = (p.w as usize, p.h as usize);
        let n = w * h;
        let (qw, qh) = (w.div_ceil(4), h.div_ceil(4));
        // The buffer plan: hdr A, B; cropped G-buffer; pick; rgb A, B; Kuwahara; bloom; reductions; drops.
        let (ha, hb, o_pg, o_pk, ra, rb) = (0, 3 * n, 6 * n, 11 * n, 12 * n, 15 * n);
        let mut at = 18 * n;
        let o_ks = at; if pin.kuwahara { at += 14 * n; }
        let (o_q0, o_q1) = (at, at + 3 * qw * qh); at += 6 * qw * qh;
        let o_red = at; at += h * 23 + 1;
        let t_drops = at; at += pin.drops.len().max(1) * 5;
        let words = at;
        (p.o_pg, p.o_pk, p.o_ks, p.o_q0, p.o_q1, p.o_red, p.t_drops) = (o_pg as u32, o_pk as u32, o_ks as u32, o_q0 as u32, o_q1 as u32, o_red as u32, t_drops as u32);
        (p.qw, p.qh, p.n_drops) = (qw as u32, qh as u32, pin.drops.len() as u32);
        let other = |o: usize| if o == ha { hb } else { ha };
        let other_r = |o: usize| if o == ra { rb } else { ra };
        let mut cur = ha;
        p.h_crop = ha as u32;
        if pin.shimmer { p.h_shim = hb as u32; cur = hb; }
        let lens = p.lens_kind != 0;
        if lens { p.h_lsrc = cur as u32; cur = other(cur); p.h_ldst = cur as u32; }
        p.h_fin = cur as u32;
        p.r_fin = ra as u32;
        let mut cr = ra;
        if pin.kuwahara { p.r_kuw = rb as u32; cr = rb; }
        p.r_mid = cr as u32;
        if pin.warp { cr = other_r(cr); p.r_wdst = cr as u32; }
        p.r_last = cr as u32;
        let params = *p;
        // Tables: the sRGB curve, then the palette and the .cube (rebuilt when `key` changes).
        let (tables, t_lut, t_col, t_cube) = {
            let mut t = g.post.tables.lock().unwrap();
            if !t.as_ref().is_some_and(|t| t.0 == pin.key) {
                let (bucket, starts) = super::texture::srgb_tables();
                let mut words: Vec<u32> = bucket.iter().map(|&b| b as u32).collect();
                words.extend(starts.iter().map(|v| v.to_bits()));
                let t_lut = words.len() as u32;
                let mut t_col = t_lut;
                if let Some((_, lut, cols)) = &pin.lut {
                    words.extend(lut.chunks(4).map(|c| c.iter().enumerate().fold(0u32, |a, (k, &b)| a | (b as u32) << (8 * k))));
                    t_col = words.len() as u32;
                    words.extend(cols.iter().copied());
                }
                let t_cube = words.len() as u32;
                if let Some(cube) = pin.cube { words.extend(cube.iter().flat_map(|c| c.iter().map(|v| v.to_bits()))); }
                let b = g.buffer("post tables", (words.len() * 4) as u64, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST);
                g.ctx.queue.write_buffer(&b, 0, bytemuck::cast_slice(&words));
                *t = Some((pin.key, b, t_lut, t_col, t_cube));
            }
            let t = t.as_ref().unwrap();
            (t.1.clone(), t.2, t.3, t.4)
        };
        let mut params = params;
        (params.t_lut, params.t_col, params.t_cube) = (t_lut, t_col, t_cube);
        let sized = |slot: &Mutex<Option<wgpu::Buffer>>, label: &str, bytes: u64, usage: wgpu::BufferUsages| {
            let mut s = slot.lock().unwrap();
            if !s.as_ref().is_some_and(|b| b.size() >= bytes) { *s = Some(g.buffer(label, bytes, usage)); }
            s.clone().unwrap()
        };
        use wgpu::BufferUsages as U;
        let scratch = sized(&g.post.scratch, "post scratch", (words * 4) as u64, U::STORAGE | U::COPY_DST | U::COPY_SRC);
        let out_bytes = (params.out_w * params.out_h * 4) as u64;
        let outb = sized(&g.post.out, "post out", out_bytes, U::STORAGE | U::COPY_SRC);
        self.submit();
        g.ctx.queue.write_buffer(&g.post.params, 0, bytemuck::bytes_of(&params));
        if !pin.drops.is_empty() { g.ctx.queue.write_buffer(&scratch, (t_drops * 4) as u64, bytemuck::cast_slice(&pin.drops)); }
        let sz = self.bufs();
        let bg = g.ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("post"), layout: &g.post.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: g.params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: g.post.params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: sz.hdr.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: sz.gbuf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: sz.pick.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: scratch.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 6, resource: tables.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 7, resource: outb.as_entire_binding() },
            ],
        });
        let full = ((w as u32).div_ceil(8), (h as u32).div_ceil(8));
        let quarter = ((qw as u32).div_ceil(8), (qh as u32).div_ceil(8));
        let rows = (h as u32).div_ceil(64);
        let mut steps: Vec<(&'static str, (u32, u32))> = vec![("post_crop", full)];
        if pin.shimmer { steps.push(("post_shimmer", full)); }
        if lens {
            if params.lens_kind == 2 { steps.push(("post_frost_rows", (rows, 1))); steps.push(("post_frost_mean", (1, 1))); }
            steps.push(("post_lens", full));
        }
        if pin.bloom {
            steps.push(("post_bloom_down", quarter));
            for _ in 0..2 { steps.push(("post_blur_h", quarter)); steps.push(("post_blur_v", quarter)); }
        }
        steps.push(("post_finish", full));
        if pin.kuwahara { steps.push(("post_kuw_h", full)); steps.push(("post_kuw_v", full)); }
        if params.grade != 0 || params.ol_on != 0 { steps.push(("post_grade_outline", full)); }
        if pin.warp { steps.push(("post_warp", full)); }
        if params.ramp_on != 0 || params.q_on != 0 { steps.push(("post_quant", full)); }
        if params.stats_on != 0 { steps.push(("post_stats", (rows, 1))); }
        steps.push(("post_out", ((params.out_w).div_ceil(8), (params.out_h).div_ceil(8))));
        {
            let mut cp = self.pass("g.post");
            cp.set_bind_group(0, &bg, &[]);
            for (e, (x, y)) in steps {
                cp.set_pipeline(&g.post.pipes[e]);
                cp.dispatch_workgroups(x, y, 1);
            }
        }
        // One readback: the bytes, then the pick ids, the cropped G-buffer (for depth), the stats rows.
        let mut parts: Vec<(u64, u64, u64)> = vec![(0, 0, out_bytes)];
        let mut at = out_bytes;
        let pick_on = params.pick_on != 0;
        let (pk_at, pg_at, st_at) = (at, at + if pick_on { (n * 4) as u64 } else { 0 }, 0u64);
        if pick_on { parts.push(((o_pk * 4) as u64, at, (n * 4) as u64)); at += (n * 4) as u64; }
        let pg_at = if pin.want_depth { let a = at; parts.push(((o_pg * 4) as u64, at, (n * 20) as u64)); at += (n * 20) as u64; a } else { pg_at };
        let st_at = if params.stats_on != 0 { let a = at; parts.push(((o_red * 4) as u64, at, (h * 23 * 4) as u64)); at += (h * 23 * 4) as u64; a } else { st_at };
        let read = sized(&g.post.read, "post readback", at, U::MAP_READ | U::COPY_DST);
        {
            let e = self.enc();
            for (k, &(src, dst, len)) in parts.iter().enumerate() {
                e.copy_buffer_to_buffer(if k == 0 { &outb } else { &scratch }, src, &read, dst, len);
            }
        }
        self.submit();
        let mut out = PostOut { rgba: Vec::new(), pick: None, depth: None, stats: None };
        g.read_with(&read, at, self.last.clone(), |b| {
            out.rgba = b[..out_bytes as usize].to_vec();
            if pick_on { out.pick = Some(bytemuck::cast_slice::<u8, u32>(&b[pk_at as usize..pk_at as usize + n * 4]).iter().map(|&v| v as u16).collect()); }
            if pin.want_depth { out.depth = Some(bytemuck::cast_slice::<u8, f32>(&b[pg_at as usize..pg_at as usize + n * 20]).chunks_exact(5).map(|c| c[0]).collect()); }
            if params.stats_on != 0 { out.stats = Some(bytemuck::cast_slice::<u8, f32>(&b[st_at as usize..st_at as usize + h * 23 * 4]).to_vec()); }
        });
        out
    }

    /// hdr and refl in one wait.
    pub fn get_hdr_refl(&mut self, hdr: &mut [[f32; 3]], refl: &mut [[f32; 6]]) {
        let n = self.n() as u64;
        let (h, r, read) = { let b = self.bufs(); (b.hdr.clone(), b.refl.clone(), b.read.clone()) };
        let e = self.enc();
        e.copy_buffer_to_buffer(&h, 0, &read, 0, n * 12);
        e.copy_buffer_to_buffer(&r, 0, &read, n * 12, n * 24);
        self.submit();
        self.g.read_with(&read, n * 36, self.last.clone(), |bytes| {
            bytemuck::cast_slice_mut::<_, u8>(hdr).copy_from_slice(&bytes[..(n * 12) as usize]);
            bytemuck::cast_slice_mut::<_, u8>(refl).copy_from_slice(&bytes[(n * 12) as usize..]);
        });
    }

    /// A compute pass, timed when profiling.
    fn pass(&mut self, label: &'static str) -> wgpu::ComputePass<'_> {
        let g = self.g;
        let mut tw = None;
        if let Some(ts) = g.ts.as_ref().filter(|_| self.profile && (self.passes.len() as u32) < TS_PASSES) {
            let k = self.passes.len() as u32;
            self.passes.push(label);
            tw = Some(wgpu::ComputePassTimestampWrites { query_set: &ts.set, beginning_of_pass_write_index: Some(2 * k), end_of_pass_write_index: Some(2 * k + 1) });
        }
        self.enc().begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some(label), timestamp_writes: tw })
    }

    /// Each timed pass's GPU time in ms, in order (waits for the frame's work).
    pub fn pass_times(&mut self) -> Vec<(&'static str, f64)> {
        let g = self.g;
        let Some(ts) = g.ts.as_ref() else { return Vec::new() };
        let n = self.passes.len() as u32;
        if n == 0 { return Vec::new(); }
        let e = self.enc();
        e.resolve_query_set(&ts.set, 0..2 * n, &ts.resolve, 0);
        e.copy_buffer_to_buffer(&ts.resolve, 0, &ts.read, 0, (n * 16) as u64);
        self.submit();
        let period = g.ctx.queue.get_timestamp_period() as f64;
        let mut out = Vec::new();
        let names = std::mem::take(&mut self.passes);
        g.read_with(&ts.read, (n * 16) as u64, self.last.clone(), |b| {
            let t: &[u64] = bytemuck::cast_slice(b);
            for (k, name) in names.iter().enumerate() {
                out.push((*name, t[2 * k + 1].saturating_sub(t[2 * k]) as f64 * period / 1e6));
            }
        });
        out
    }

    fn groups(&self) -> (u32, u32) { ((self.w as u32).div_ceil(8), (self.h as u32).div_ceil(8)) }

    /// The sky over every pixel where nothing is drawn (hdr in place).
    pub fn sky(&mut self, input: &SkyInput) {
        let g = self.g;
        self.submit();
        g.ctx.queue.write_buffer(&g.sky.params, 0, bytemuck::bytes_of(&input.params));
        let mut pw = input.prims.clone();
        if pw.is_empty() { pw.push(0.0); }
        let prims = g.upload(&g.sky.prims, "sky prims", bytemuck::cast_slice(&pw));
        let mut bw = input.bins.clone();
        if bw.is_empty() { bw.push(0); }
        let bins = g.upload(&g.sky.bins, "sky bins", bytemuck::cast_slice(&bw));
        let sz = self.bufs();
        let bg = g.ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sky"), layout: &g.sky.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: g.params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: g.sky.params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: sz.gbuf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: sz.hdr.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: prims.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: bins.as_entire_binding() },
            ],
        });
        let (gx, gy) = self.groups();
        let mut p = self.pass("g.sky");
        p.set_bind_group(0, &bg, &[]);
        p.set_pipeline(&g.sky.pipe);
        p.dispatch_workgroups(gx, gy, 1);
    }

    /// render::shade for one world: the G-buffer and masks (already on the GPU) to colour and
    /// reflection data.
    pub fn shade(&mut self, input: &ShadeInput) {
        let g = self.g;
        let texels = g.texel_buffer(input.textures);
        let mut lw: Vec<f32> = input.lights.iter().chain(input.sky.iter()).flat_map(|l| l.iter().copied()).collect();
        if lw.is_empty() { lw.push(0.0); }
        self.submit();
        let lights = g.upload(&g.lights, "lights", bytemuck::cast_slice(&lw));
        let mut tw: Vec<u32> = Vec::new();
        if let Some(t) = &input.tiles {
            let mut off = 0u32;
            for l in t { tw.push(off); off += l.len() as u32; }
            tw.push(off);
            for l in t { tw.extend(l.iter().map(|&k| k as u32)); }
        }
        if tw.is_empty() { tw.push(0); }
        let tiles = g.upload(&g.tiles, "tiles", bytemuck::cast_slice(&tw));
        let sz = self.bufs();
        let bg = g.ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shade"), layout: &g.shade.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: g.params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: sz.gbuf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: texels.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: lights.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: tiles.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: sz.masks.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 6, resource: sz.uvs.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 7, resource: sz.hdr.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 8, resource: sz.refl.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 9, resource: g.shade.air.as_entire_binding() },
            ],
        });
        self.shade_bg = Some(bg.clone());
        let (gx, gy) = self.groups();
        let mut p = self.pass("g.shade");
        p.set_bind_group(0, &bg, &[]);
        p.set_pipeline(&g.shade.uvs);
        p.dispatch_workgroups(gx, gy, 1);
        p.set_pipeline(&g.shade.shade);
        p.dispatch_workgroups(gx, gy, 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_params_match_the_wgsl_struct_size() {
        // A uniform block's size is a multiple of 16; the WGSL struct has the same fields.
        assert_eq!(std::mem::size_of::<WorldParams>() % 16, 0);
        assert_eq!(std::mem::size_of::<WorldParams>(), 4 * 108);
        assert_eq!(std::mem::size_of::<CardGpu>(), 4 * 42);
        assert_eq!(std::mem::size_of::<SplatGpu>(), 4 * 24);
        assert_eq!(std::mem::size_of::<AirParams>(), 48);
        assert_eq!(std::mem::size_of::<ShaftParams>(), 64);
        assert_eq!(std::mem::size_of::<PostParams>(), 336);
        assert_eq!(std::mem::size_of::<SkyParams>() % 16, 0, "SkyParams is {} bytes", std::mem::size_of::<SkyParams>());
    }

    #[test]
    fn every_pipeline_builds() {
        // WGSL is only validated when a pipeline is made: build them all here. Skipped (with a
        // note) where the machine has no GPU adapter.
        match GpuContext::headless() {
            Ok(ctx) => { if let Err(e) = Gpu::new(ctx) { panic!("{e}"); } }
            Err(e) => eprintln!("no GPU here, pipelines not checked: {e}"),
        }
    }
}
