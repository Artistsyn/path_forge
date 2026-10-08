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
        self.compute_with(module, layout, entry, true)
    }
    /// `multi` false: the pipeline for frames of one world, with everything only several worlds
    /// need compiled out (the `MULTI` override in common.wgsl).
    fn compute_with(&self, module: &wgpu::ShaderModule, layout: &wgpu::PipelineLayout, entry: &str, multi: bool) -> Result<wgpu::ComputePipeline, String> {
        self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let constants = [("MULTI", if multi { 1.0 } else { 0.0 })];
        let p = self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(entry), layout: Some(layout), module, entry_point: Some(entry),
            compilation_options: wgpu::PipelineCompilationOptions { constants: &constants, ..Default::default() }, cache: None,
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
    pub zero: u32, pub fk_style: u32, pub fk_noise: f32, pub tile_cap: u32,
    // A frame of several worlds: which one this is (its pixels carry it as their realm), where its
    // textures and lights start in the shared buffers, and whether the frame has several.
    pub realm: u32, pub tex_base: u32, pub light_base: u32, pub multi: u32,
    // View::split: the boundary, the path beyond it, and each side's stairs.
    pub sp_on: u32, pub sp_zb: f32, pub sp_hw_b: f32, pub sp_flare_b: f32, pub sp_taper: f32,
    pub sa_on: u32, pub sa_period: f32, pub sa_run: f32, pub sa_rise: f32, pub sa_steps: f32, pub sa_offset: f32, pub sa_scroll: f32,
    pub sb_on: u32, pub sb_period: f32, pub sb_run: f32, pub sb_rise: f32, pub sb_steps: f32, pub sb_offset: f32, pub sb_scroll: f32,
    // View::shear of every world in the frame (a soft patch moves a pixel from one to another).
    pub sh_z0: [f32; 3], pub sh_k: [f32; 3],
    // FrontAir: the first world's air in front of the boundary.
    pub fa_on: u32, pub fa_zb: f32, pub fa_fog_on: u32, pub fa_fog_dist: f32, pub fa_col: [f32; 3], pub fa_gain: f32,
    // Portal: light from the other world through the opening (corners in camera space).
    pub pt_on: u32, pub pt_zb: f32, pub pt_front: u32, pub pt_rad: [f32; 3], pub pt_c: [f32; 12],
    // Opening: the face round a threshold's opening, and its texture's tile size.
    pub op_on: u32, pub op_hw: f32, pub op_top: f32, pub op_arch: f32, pub op_rim: f32, pub op_seed: u32, pub op_outer_hw: f32, pub op_outer_top: f32, pub op_hill: f32,
    pub fac_tile: f32,
    // Ctx::verge_beyond, and the first world's region past it (Bounds::region_of): four half-planes.
    pub vb_on: u32, pub vb_z: f32, pub wedge: [f32; 12],
    // Path edge lights (render::EdgeK).
    pub el_on: u32, pub el_col: [f32; 3], pub el_hw: f32, pub el_inset: f32, pub el_period: f32, pub el_phase: f32,
    /// The path as a waterway (render::water::WaterK).
    pub ww_on: u32, pub ww_shift: f32, pub ww_foam: f32, pub ww_foam_col: [f32; 3], pub ww_foam_w: f32,
    pub ww_lap_ph: f32, pub ww_lap_k: f32, pub ww_k1: f32, pub ww_k2: f32, pub ww_wet: f32,
    pub ww_kind: u32, pub ww_dens: f32, pub ww_fcol: [f32; 3], pub ww_fsize: f32, pub ww_period: f32, pub ww_cs: f32, pub ww_seed: u32,
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
    pub au_on: u32, pub au_a: f32, pub au_m: f32, pub au_cx: f32,
    pub au_unit: f32, pub au_lo: [f32; 3], pub au_hi: [f32; 3], pub au_acc: [f32; 3],
    pub au_foot: f32, pub au_arc: f32, pub au_tall: f32, pub au_rays: f32, pub au_waves: f32,
    pub au_pres_lo: f32, pub au_pres_hi: f32, pub au_edge: f32, pub au_k: f32, pub au_so: f32,
    pub sun_on: u32, pub s_x: f32, pub s_y: f32, pub s_r: f32, pub s_c: [f32; 3], pub s_spread: f32, pub s_focal: f32,
    pub moon_on: u32, pub m_c: [f32; 3], pub m_gr: f32, pub m_sx: f32, pub m_sy: f32, pub m_litf: f32, pub m_op: f32, pub m_craters: u32,
    pub cl_on: u32, pub cl_col: [f32; 3], pub cl_sun: u32, pub cl_sx: f32, pub cl_sy: f32, pub cl_sc: [f32; 3],
    pub cl_op: f32, pub cl_bins: u32, pub cl_prims: u32,
    pub rb_on: u32, pub rb_cx: f32, pub rb_cy: f32, pub rb_rad: f32, pub rb_band: f32, pub rb_rad2: f32, pub rb_band2: f32, pub rb_k: f32, pub rb_double: u32,
    pub fl_on: u32, pub fl: [f32; 3],
    pub bolt_on: u32, pub b_gain: f32, pub b_core: [f32; 3], pub b_tint: [f32; 3],
    pub b_glow: f32, pub b_scale: f32, pub b_xa: u32, pub b_xb: u32, pub b_yb: u32, pub b_nsegs: u32, pub b_prims: u32,
    pub veil: f32, pub bank_on: u32, pub bank_t: f32, pub bank: [f32; 3],
    pub sp_on: u32, pub sp_below: u32, pub sp_scale: f32, pub sp_stars: f32, pub sp_sb: f32, pub sp_cx: f32, pub sp_unit: f32, pub sp_top: f32,
    pub sp_st: [f32; 3], pub sp_sh: [f32; 3],
    pub sp_neb: f32, pub sp_n1: [f32; 3], pub sp_n2: [f32; 3], pub sp_neb_f: f32,
    pub sp_gal: f32, pub sp_gc: [f32; 3], pub sp_gal_c: f32, pub sp_gal_s: f32, pub sp_gal_w: f32, pub sp_gal_h: f32, pub sp_gal_core: f32,
    pub sp_so: f32, pub pl_n: u32, pub pl_prims: u32, pub s_air: f32, pub bh_on: u32,
    pub tn_on: u32, pub tn_kind: f32, pub tn_vx: f32, pub tn_vy: f32, pub tn_focal: f32, pub tn_radius: f32, pub tn_travel: f32,
    pub tn_spin: f32, pub tn_twist: f32, pub tn_loop: f32, pub tn_lanes: f32, pub tn_period: f32, pub tn_nx: f32, pub tn_ny: f32,
    pub tn_c0: [f32; 3], pub tn_c1: [f32; 3], pub tn_c2: [f32; 3], pub tn_k: f32, pub tn_core: f32, pub tn_scale: f32, pub tn_fade: f32, pub bh_prims: u32,
    pub mode: u32, pub w_fork: u32, pub w_base: f32,
    pub w_center: f32, pub w_focal: f32, pub w_split: f32, pub w_near: f32, pub w_keep: [f32; 2], pub sp_star_var: f32, pub pad5: u32,
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

/// Per-tile triangle lists in `bin_tiles`' layout, each triangle listed only in the tiles it can
/// draw into (`raster::tiles_touched`), in triangle order (depth ties go to the first).
/// Triangles marked in `skip` are listed nowhere.
pub(crate) fn bin_triangles(w: usize, h: usize, prepared: &[crate::world::raster::Prepared], skip: &[bool]) -> Vec<u32> {
    use rayon::prelude::*;
    let (cols, rows) = (w.div_ceil(16), h.div_ceil(16));
    let touched: Vec<Vec<u32>> = prepared.par_iter().zip(skip).map(|(t, &s)| {
        let mut v = Vec::new();
        if !s { crate::world::raster::tiles_touched(t, cols, &mut v); }
        v
    }).collect();
    let mut count = vec![0u32; cols * rows + 1];
    for v in &touched { for &k in v { count[k as usize + 1] += 1; } }
    for k in 1..count.len() { count[k] += count[k - 1]; }
    let mut out = count.clone();
    out.resize(cols * rows + 1 + count[cols * rows] as usize, 0);
    let mut fill = count;
    for (i, v) in touched.iter().enumerate() {
        for &k in v {
            out[cols * rows + 1 + fill[k as usize] as usize] = i as u32;
            fill[k as usize] += 1;
        }
    }
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

/// A triangle set up on the CPU (raster::Prepared), as geom.wgsl's `Tri` reads it.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct TriGpu { pub f: [f32; 36] }

impl TriGpu {
    pub fn new(t: &super::raster::Prepared) -> TriGpu {
        let mut f = [0.0f32; 36];
        for (k, q) in t.p.iter().enumerate() { f[k * 2] = q[0]; f[k * 2 + 1] = q[1]; }
        f[6..9].copy_from_slice(&t.inv_z);
        for (k, w) in t.w_over_z.iter().enumerate() { f[9 + k * 3..12 + k * 3].copy_from_slice(w); }
        for (k, v) in [t.min_x, t.max_x, t.min_y, t.max_y].into_iter().enumerate() { f[18 + k] = f32::from_bits(v as u32); }
        f[22] = 1.0 / t.area;
        f[23] = f32::from_bits(t.id as u32 | (t.realm as u32) << 8);
        for (k, e) in t.edges().iter().enumerate() { f[24 + k * 3..27 + k * 3].copy_from_slice(e); }
        TriGpu { f }
    }
}

/// A billboard's shadow footprints (render::prop_shadows), as geom.wgsl's `Caster` reads it; the
/// sprite level's place in the sprite store is filled in by `Frame::geometry`.
#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct CasterGpu { pub f: [f32; 24] }
pub(crate) struct CasterIn { pub c: CasterGpu, pub sprite: Arc<super::sprites::Sprite>, pub lod: usize }

/// geom.wgsl's `Geom`.
#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct GeomParams {
    pub tile_cols: u32, pub cast_bins: u32, pub sun_on: u32, pub n_lights: u32,
    pub sun: [f32; 3], pub clouds: f32,
    pub cloud_travel: f32, pub cloud_seed: u32, pub tiles_on: u32, pub cap: u32,
    pub small_off: u32, pub n_small: u32, pub pad: [u32; 2],
}

/// What the geometry pass draws: the triangles and their per-tile lists, the shadow casters and
/// theirs, the sun (when it casts), cloud shade (amount, travel, seed), and the lamps to sort into
/// tiles (None: shading looks at every lamp).
pub(crate) struct GeomIn {
    pub tris: Vec<TriGpu>, pub tri_bins: Vec<u32>,
    /// Triangles rasterised one thread each instead of from the tile lists (`raster::SMALL_TRI`).
    pub small: Vec<u32>,
    pub casters: Vec<CasterIn>, pub cast_bins: Vec<u32>,
    pub sun: Option<[f32; 3]>, pub clouds: Option<(f32, f32, u32)>,
    pub lamps: Option<Vec<[f32; 8]>>,
}

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
    /// Path, verge, wall, ceiling, deck, bottom, facade.
    pub textures: [&'a Arc<Texture>; 7],
    /// Point lights: pos (camera space), radius, colour.
    pub lights: Vec<[f32; 8]>,
    /// Sky lights: direction, colour.
    pub sky: Vec<[f32; 8]>,
    /// Per-tile light lists (16 x 16 tiles), when used.
    pub tiles: Option<Vec<Vec<u16>>>,
}


struct Sized {
    w: usize, h: usize,
    gbuf: wgpu::Buffer, masks: wgpu::Buffer, uvs: wgpu::Buffer, hdr: wgpu::Buffer, refl: wgpu::Buffer,
    pick: wgpu::Buffer,
    /// The raster's meeting place: per pixel the least depth bits, small winner, big winner.
    racc: wgpu::Buffer,
    /// The reflection pass's per-column tables.
    q2s: wgpu::Buffer, mins: wgpu::Buffer, ends: wgpu::Buffer, blocks: wgpu::Buffer,
    read: wgpu::Buffer,
}

struct SkyPass { layout: wgpu::BindGroupLayout, pipe: [wgpu::ComputePipeline; 2], params: wgpu::Buffer, prims: Mutex<Option<wgpu::Buffer>>, bins: Mutex<Option<wgpu::Buffer>> }

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

struct ShaftPass { layout: wgpu::BindGroupLayout, mask: wgpu::ComputePipeline, add: wgpu::ComputePipeline, apply: wgpu::ComputePipeline, params: wgpu::Buffer, lists: Mutex<Option<wgpu::Buffer>>, cells: Mutex<Option<wgpu::Buffer>>, lamps: Mutex<Option<wgpu::Buffer>> }

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
    pub t_cube: u32, pub t_drops: u32, pub lens_tphase: f32, pub pad2: u32,
    // Heat haze per world of the frame (each pixel its own world's): the clock of world 0 (whose
    // strength and cycles are sh_strength, sh_c1, sh_c2), then worlds 1 and 2's.
    pub sh_t0: f32, pub sh_s: [f32; 2], pub sh_a: [f32; 2], pub sh_b: [f32; 2], pub sh_t: [f32; 2], pub pad3: [u32; 3],
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

struct GeomPass {
    layout: wgpu::BindGroupLayout, raster_layout: wgpu::BindGroupLayout,
    small_z: wgpu::ComputePipeline, small_pick: wgpu::ComputePipeline, resolve: wgpu::ComputePipeline,
    raster: wgpu::ComputePipeline, shadow: wgpu::ComputePipeline, tiles: wgpu::ComputePipeline,
    params: wgpu::Buffer, tris: Mutex<Option<wgpu::Buffer>>, bins: Mutex<Option<wgpu::Buffer>>, casters: Mutex<Option<wgpu::Buffer>>,
    lamps: Mutex<Option<wgpu::Buffer>>, lists: Mutex<Option<wgpu::Buffer>>,
}
struct ReflectPass { layout: wgpu::BindGroupLayout, colsum_a: wgpu::ComputePipeline, colsum_b: wgpu::ComputePipeline, reflect: wgpu::ComputePipeline }

struct ShadePass {
    layout: wgpu::BindGroupLayout,
    /// [one world, several worlds] each.
    uvs: [wgpu::ComputePipeline; 2],
    shade: [wgpu::ComputePipeline; 2],
    soft: wgpu::ComputePipeline,
    mist: [wgpu::ComputePipeline; 2],
    /// One `AirParams` per world, AIR_STRIDE apart.
    air: wgpu::Buffer,
}

/// The most worlds one frame draws (a fork: the road and its two branches).
pub(crate) const MAX_WORLDS: usize = 3;
const AIR_STRIDE: u64 = 256;

/// The air's settings for the mist pass; `Air` in shade.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct AirParams {
    pub mist_on: u32, pub density: f32, pub top: f32, pub patch: f32,
    pub travel: f32, pub seed: u32, pub mist: [f32; 3], pub pad: [u32; 3],
}

/// Textures kept on the GPU (see `Gpu::texel_buffer`): each held texture and where its levels
/// start in the texel data (offset in floats, side), `used` of `cap` words taken.
struct TexStore { buf: wgpu::Buffer, cap: u64, used: u64, held: Vec<(Arc<Texture>, Vec<(u32, u32)>)> }

impl TexStore {
    /// Three floats a texel, or four when it glows (its glow after its colour).
    fn stride(t: &Arc<Texture>) -> u64 { if t.emit.is_empty() { 3 } else { 4 } }
    fn words(t: &Arc<Texture>) -> u64 { t.levels.iter().take(8).map(|(_, l)| l.len() as u64 * Self::stride(t)).sum() }
    fn find(&self, t: &Arc<Texture>) -> Option<usize> { self.held.iter().position(|h| Arc::ptr_eq(&h.0, t)) }
    fn add(&mut self, queue: &wgpu::Queue, t: &Arc<Texture>) -> usize {
        let mut levels = Vec::new();
        for (l, (side, texels)) in t.levels.iter().take(8).enumerate() {
            let data: Vec<f32> = match t.emit.get(l) {
                Some(e) => texels.iter().zip(e).flat_map(|(c, g)| [c[0], c[1], c[2], *g]).collect(),
                None => texels.iter().flat_map(|c| c.iter().copied()).collect(),
            };
            queue.write_buffer(&self.buf, (544 + self.used) * 4, bytemuck::cast_slice(&data));
            levels.push((self.used as u32, *side as u32));
            self.used += data.len() as u64;
        }
        self.held.push((t.clone(), levels));
        self.held.len() - 1
    }
}

/// The GPU side of a `WorldRenderer`: pipelines, cached textures, and buffers for the frame size.
pub struct Gpu {
    pub ctx: Arc<GpuContext>,
    geom: GeomPass,
    shade: ShadePass,
    sky: SkyPass,
    cards: CardPass,
    splat: SplatPass,
    reflect: ReflectPass,
    shafts: ShaftPass,
    post: PostPass,
    ts: Option<Timestamps>,
    /// One `WorldParams` per world of the frame, `wstride` apart (bound with a dynamic offset).
    params: wgpu::Buffer,
    wstride: u64,
    sized: Mutex<Option<Sized>>,
    /// Texel buffer and the textures it holds (by pointer).
    textures: Mutex<Option<TexStore>>,
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
/// The world's parameters, at the dynamic offset of the world a dispatch draws.
fn world_uniform(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding, visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: true, min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<WorldParams>() as u64) },
        count: None,
    }
}
fn air_uniform(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding, visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: true, min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<AirParams>() as u64) },
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
            entries: &[world_uniform(0), storage(1, true), storage(2, true), storage(3, true), storage(4, true), storage(5, true), storage(6, false), storage(7, false), storage(8, false), air_uniform(9)],
        });
        let pl = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("shade"), bind_group_layouts: &[&layout], push_constant_ranges: &[] });
        let m = ctx.module("shade.wgsl", include_str!("shade.wgsl"))?;
        let air = d.create_buffer(&wgpu::BufferDescriptor {
            label: Some("air params"), size: AIR_STRIDE * MAX_WORLDS as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false,
        });
        let both = |e: &str| -> Result<[wgpu::ComputePipeline; 2], String> { Ok([ctx.compute_with(&m, &pl, e, false)?, ctx.compute_with(&m, &pl, e, true)?]) };
        let shade = ShadePass { uvs: both("uvs_main")?, shade: both("shade_main")?, soft: ctx.compute(&m, &pl, "soft_main")?, mist: both("mist_main")?, layout, air };
        let geom = {
            let layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("geom"),
                entries: &[world_uniform(0), storage(1, false), storage(2, true), storage(3, true), storage(4, false), storage(5, true), storage(6, true), storage(7, false), storage(8, true), uniform(9)],
            });
            let pl = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("geom"), bind_group_layouts: &[&layout], push_constant_ranges: &[] });
            let m = ctx.module("geom.wgsl", include_str!("geom.wgsl"))?;
            // The raster passes on their own: the shadow and tile passes already use the eight
            // storage buffers a stage may bind by default.
            let raster_layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("raster"),
                entries: &[world_uniform(0), storage(1, false), storage(2, true), storage(3, true), uniform(9), storage(10, false)],
            });
            let rpl = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("raster"), bind_group_layouts: &[&raster_layout], push_constant_ranges: &[] });
            let params = d.create_buffer(&wgpu::BufferDescriptor {
                label: Some("geom params"), size: std::mem::size_of::<GeomParams>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false,
            });
            GeomPass {
                raster: ctx.compute_with(&m, &rpl, "raster_main", false)?, small_z: ctx.compute_with(&m, &rpl, "small_z_main", false)?,
                small_pick: ctx.compute_with(&m, &rpl, "small_pick_main", false)?, resolve: ctx.compute_with(&m, &rpl, "resolve_main", false)?,
                raster_layout, shadow: ctx.compute_with(&m, &pl, "shadow_main", false)?, tiles: ctx.compute_with(&m, &pl, "tiles_main", false)?,
                layout, params, tris: Mutex::new(None), bins: Mutex::new(None), casters: Mutex::new(None), lamps: Mutex::new(None), lists: Mutex::new(None),
            }
        };
        let sky = {
            let layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("sky"), entries: &[world_uniform(0), uniform(1), storage(2, true), storage(3, false), storage(4, true), storage(5, true)],
            });
            let pl = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("sky"), bind_group_layouts: &[&layout], push_constant_ranges: &[] });
            let m = ctx.module("sky.wgsl", include_str!("sky.wgsl"))?;
            let pipe = [ctx.compute_with(&m, &pl, "sky_main", false)?, ctx.compute_with(&m, &pl, "sky_main", true)?];
            let params = d.create_buffer(&wgpu::BufferDescriptor {
                label: Some("sky params"), size: std::mem::size_of::<SkyParams>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false,
            });
            SkyPass { layout, pipe, params, prims: Mutex::new(None), bins: Mutex::new(None) }
        };
        let cards = {
            let layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("cards"), entries: &[world_uniform(0), storage(1, false), storage(2, false), storage(3, true), storage(4, true), storage(5, true), storage(6, false), uniform(7)],
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
                label: Some("splats"), entries: &[world_uniform(0), storage(1, false), storage(2, false), storage(3, true), storage(4, true), uniform(5)],
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
                label: Some("reflect"), entries: &[world_uniform(0), storage(1, true), storage(2, false), storage(3, true), storage(4, false), storage(5, false), storage(6, false), storage(7, false)],
            });
            let pl = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("reflect"), bind_group_layouts: &[&layout], push_constant_ranges: &[] });
            let m = ctx.module("reflect.wgsl", include_str!("reflect.wgsl"))?;
            ReflectPass { colsum_a: ctx.compute(&m, &pl, "colsum_a")?, colsum_b: ctx.compute(&m, &pl, "colsum_b")?, reflect: ctx.compute(&m, &pl, "reflect_main")?, layout }
        };
        let shafts = {
            let layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("shafts"), entries: &[world_uniform(0), storage(1, true), storage(2, false), storage(3, true), storage(4, true), storage(5, false), uniform(6)],
            });
            let pl = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("shafts"), bind_group_layouts: &[&layout], push_constant_ranges: &[] });
            let m = ctx.module("shafts.wgsl", include_str!("shafts.wgsl"))?;
            let params = d.create_buffer(&wgpu::BufferDescriptor {
                label: Some("shaft params"), size: std::mem::size_of::<ShaftParams>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false,
            });
            ShaftPass {
                mask: ctx.compute(&m, &pl, "shaft_mask")?, add: ctx.compute(&m, &pl, "shaft_add")?, apply: ctx.compute(&m, &pl, "shaft_apply")?,
                layout, params, lists: Mutex::new(None), cells: Mutex::new(None), lamps: Mutex::new(None),
            }
        };
        let post = {
            let layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("post"), entries: &[world_uniform(0), uniform(1), storage(2, true), storage(3, true), storage(4, true), storage(5, false), storage(6, true), storage(7, false)],
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
        let align = d.limits().min_uniform_buffer_offset_alignment.max(16) as u64;
        let wstride = (std::mem::size_of::<WorldParams>() as u64).next_multiple_of(align);
        assert!(AIR_STRIDE % align == 0 && std::mem::size_of::<AirParams>() as u64 <= AIR_STRIDE);
        let params = d.create_buffer(&wgpu::BufferDescriptor {
            label: Some("world params"), size: wstride * MAX_WORLDS as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false,
        });
        let ts = d.features().contains(wgpu::Features::TIMESTAMP_QUERY).then(|| Timestamps {
            set: d.create_query_set(&wgpu::QuerySetDescriptor { label: Some("pass times"), ty: wgpu::QueryType::Timestamp, count: TS_PASSES * 2 }),
            resolve: d.create_buffer(&wgpu::BufferDescriptor { label: Some("pass times"), size: (TS_PASSES * 16) as u64, usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false }),
            read: d.create_buffer(&wgpu::BufferDescriptor { label: Some("pass times read"), size: (TS_PASSES * 16) as u64, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }),
        });
        Ok(Gpu { ctx, geom, shade, sky, cards, splat, reflect, shafts, post, ts, params, wstride, sized: Mutex::new(None), textures: Mutex::new(None), lights: Mutex::new(None), tiles: Mutex::new(None) })
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
                // Sun mask, ambient-occlusion mask, then (several worlds) the soft patch edges.
                masks: self.buffer("masks", n * 16, st),
                uvs: self.buffer("uvs", n * 12, st),
                hdr: self.buffer("hdr", n * 12, st),
                refl: self.buffer("refl", n * 24, st),
                pick: self.buffer("pick", n * 4, st),
                racc: self.buffer("raster meet", n * 12, U::STORAGE),
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

    fn world_binding(&self) -> wgpu::BindingResource<'_> {
        wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &self.params, offset: 0, size: wgpu::BufferSize::new(std::mem::size_of::<WorldParams>() as u64) })
    }

    /// Every world's textures in one texel buffer: a header of (offset, side) per slot (8 per
    /// world, 7 used) and level, and the level counts, then the textures' levels as linear RGB.
    /// Textures stay in the buffer once added (held, so their pointers stay theirs), so frames that
    /// change which worlds they draw only rewrite the header.
    fn texel_buffer(&self, worlds: &[[&Arc<Texture>; 7]]) -> wgpu::Buffer {
        let mut guard = self.textures.lock().unwrap();
        let fits = |st: &TexStore| worlds.iter().flatten().map(|t| if st.find(t).is_some() { 0 } else { TexStore::words(t) }).sum::<u64>() + st.used <= st.cap;
        if !guard.as_ref().is_some_and(|st| fits(st) && st.held.len() < 64) {
            // Start again with room for these and more: everything in it is uploaded anew.
            let keep: Vec<Arc<Texture>> = guard.as_ref().filter(|st| st.held.len() < 64).map(|st| st.held.iter().map(|h| h.0.clone()).collect()).unwrap_or_default();
            let need: u64 = keep.iter().chain(worlds.iter().flatten().map(|t| *t)).map(TexStore::words).sum::<u64>();
            let cap = (need * 2).max(1 << 20);
            let buf = self.buffer("texels", (544 + cap) * 4, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST);
            let mut st = TexStore { buf, cap, used: 0, held: Vec::new() };
            for t in &keep { st.add(&self.ctx.queue, t); }
            *guard = Some(st);
        }
        let st = guard.as_mut().unwrap();
        let mut header = vec![0u32; 544];
        for (slot, t) in worlds.iter().enumerate().flat_map(|(r, w)| w.iter().enumerate().map(move |(k, t)| (r * 8 + k, *t))) {
            let k = match st.find(t) { Some(k) => k, None => st.add(&self.ctx.queue, t) };
            // The level count, 0x100 when texels carry their glow (four floats each), and 0x200 when
            // read between texels (Texture::smooth).
            header[512 + slot] = t.levels.len().min(8) as u32 | if t.emit.is_empty() { 0 } else { 0x100 } | if t.smooth { 0x200 } else { 0 };
            for (l, &(off, side)) in st.held[k].1.iter().enumerate() {
                header[(slot * 8 + l) * 2] = off;
                header[(slot * 8 + l) * 2 + 1] = side;
            }
        }
        self.ctx.queue.write_buffer(&st.buf, 0, bytemuck::cast_slice(&header));
        st.buf.clone()
    }

    /// Start a frame on the GPU: its buffers, and each world's parameters (one for most frames).
    pub(crate) fn frame(&self, worlds: &[WorldParams]) -> Frame<'_> {
        assert!(!worlds.is_empty() && worlds.len() <= MAX_WORLDS);
        let (w, h) = (worlds[0].width as usize, worlds[0].height as usize);
        let sz = self.sized(w, h);
        for (r, p) in worlds.iter().enumerate() { self.ctx.queue.write_buffer(&self.params, r as u64 * self.wstride, bytemuck::bytes_of(p)); }
        Frame { g: self, sz, w, h, worlds: worlds.len(), here: 0, look: 0, enc: None, profile: false, passes: Vec::new(), shade_bg: None, soft: false, gpu_tiles: None, last: None }
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
    /// How many worlds the frame draws; the one whose air the shafts light, and whose look post takes.
    worlds: usize,
    pub here: usize,
    pub look: usize,
    enc: Option<wgpu::CommandEncoder>,
    /// Time each pass on the GPU (where the adapter can); `passes` names them in order.
    pub profile: bool,
    passes: Vec<&'static str>,
    shade_bg: Option<wgpu::BindGroup>,
    /// Whether `put_soft` gave this frame soft patch edges.
    soft: bool,
    /// Per-tile lamp lists made by the geometry pass, for shading to read.
    gpu_tiles: Option<wgpu::Buffer>,
    last: Option<wgpu::SubmissionIndex>,
}

impl<'g> Frame<'g> {
    fn n(&self) -> usize { self.w * self.h }
    /// The dynamic offset of world r's parameters.
    fn wo(&self, r: usize) -> u32 { (r as u64 * self.g.wstride) as u32 }
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
        let b = self.bufs().gbuf.clone();
        self.write(&b, 0, bytemuck::cast_slice(gbuf));
    }
    pub fn get_gbuf(&mut self, gbuf: &mut [super::raster::GPixel]) {
        let b = self.bufs().gbuf.clone();
        let bytes = self.read(&b, (self.n() * 20) as u64);
        bytemuck::cast_slice_mut::<_, u8>(gbuf).copy_from_slice(&bytes);
    }
    pub fn put_masks(&mut self, sun_mask: &[f32], ao_mask: &[f32]) {
        let (b, n) = (self.bufs().masks.clone(), self.n());
        self.write(&b, 0, bytemuck::cast_slice(sun_mask));
        self.g.ctx.queue.write_buffer(&b, (n * 4) as u64, bytemuck::cast_slice(ao_mask));
    }
    /// Soft patch edges between worlds (render::shade's `soft`), when the frame has any.
    pub fn put_soft(&mut self, soft: &[(u8, f32)]) {
        if soft.is_empty() { self.soft = false; return; }
        let words: Vec<u32> = soft.iter().flat_map(|&(o, k)| [o as u32, k.to_bits()]).collect();
        let (b, n) = (self.bufs().masks.clone(), self.n());
        self.write(&b, (n * 8) as u64, bytemuck::cast_slice(&words));
        self.soft = true;
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
            if let Some(g) = gbuf { bytemuck::cast_slice_mut::<_, u8>(g).copy_from_slice(part(0)); }
            if let Some(h) = hdr { bytemuck::cast_slice_mut::<_, u8>(h).copy_from_slice(part(1)); }
            if let Some(r) = refl { bytemuck::cast_slice_mut::<_, u8>(r).copy_from_slice(part(2)); }
            if let Some(p) = pick {
                let src: &[u32] = bytemuck::cast_slice(part(3));
                p.par_iter_mut().zip(src.par_iter()).for_each(|(a, b)| *a = *b as u16);
            }
        });
    }

    /// The sprite store's texels on the GPU, uploaded again if sprites were added since.
    fn sprite_texels(&mut self, st: &mut SpriteStore) -> wgpu::Buffer {
        if st.dirty || st.buf.is_none() {
            let mut t = st.texels.clone();
            if t.is_empty() { t.push([0.0; 4]); }
            let b = self.g.buffer("sprites", (t.len() * 16) as u64, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST);
            self.submit();
            self.g.ctx.queue.write_buffer(&b, 0, bytemuck::cast_slice(&t));
            st.buf = Some(b);
            st.dirty = false;
        }
        st.buf.clone().unwrap()
    }

    /// The G-buffer drawn from triangles, then the shadow masks and per-tile lamp lists from it
    /// (a frame of one world; its parameters say whether shading reads those lists).
    pub fn geometry(&mut self, gi: &GeomIn) {
        let g = self.g;
        let mut cs: Vec<CasterGpu> = Vec::with_capacity(gi.casters.len().max(1));
        let texels = {
            let mut st = g.cards.store.lock().unwrap();
            for c in &gi.casters {
                let mut cd = c.c;
                let (w, h, off) = st.level(&c.sprite, c.lod);
                (cd.f[19], cd.f[20], cd.f[21]) = (f32::from_bits(off), f32::from_bits(w), f32::from_bits(h));
                cs.push(cd);
            }
            self.sprite_texels(&mut st)
        };
        if cs.is_empty() { cs.push(CasterGpu::default()); }
        let mut tris = gi.tris.clone();
        if tris.is_empty() { tris.push(TriGpu { f: [0.0; 36] }); }
        let mut bins = gi.tri_bins.clone();
        let cast_off = bins.len() as u32;
        bins.extend_from_slice(&gi.cast_bins);
        let small_off = bins.len() as u32;
        bins.extend_from_slice(&gi.small);
        let tc = (self.w as u32).div_ceil(16);
        let ntiles = (tc * (self.h as u32).div_ceil(16)) as usize;
        let lamps = gi.lamps.clone().unwrap_or_default();
        let cap = lamps.len() as u32;
        let mut p = GeomParams { tile_cols: tc, cast_bins: cast_off, n_lights: cap, cap, tiles_on: gi.lamps.is_some() as u32, small_off, n_small: gi.small.len() as u32, ..Default::default() };
        if let Some(l) = gi.sun { (p.sun_on, p.sun) = (1, l); }
        if let Some((k, t, seed)) = gi.clouds { (p.clouds, p.cloud_travel, p.cloud_seed) = (k, t, seed); }
        self.submit();
        g.ctx.queue.write_buffer(&g.geom.params, 0, bytemuck::bytes_of(&p));
        let tb = g.upload(&g.geom.tris, "triangles", bytemuck::cast_slice(&tris));
        let bb = g.upload(&g.geom.bins, "geom bins", bytemuck::cast_slice(&bins));
        let cb = g.upload(&g.geom.casters, "casters", bytemuck::cast_slice(&cs));
        let mut lw: Vec<f32> = lamps.iter().flat_map(|l| l.iter().copied()).collect();
        if lw.is_empty() { lw.push(0.0); }
        let lb = g.upload(&g.geom.lamps, "geom lamps", bytemuck::cast_slice(&lw));
        let lists = {
            let mut l = g.geom.lists.lock().unwrap();
            let need = (ntiles as u64 * (cap as u64 + 1) * 4).max(16);
            if !l.as_ref().is_some_and(|b| b.size() >= need) { *l = Some(g.buffer("tile lamps", need, wgpu::BufferUsages::STORAGE)); }
            l.clone().unwrap()
        };
        let sz = self.bufs();
        let bg = g.ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("geom"), layout: &g.geom.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: g.world_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: sz.gbuf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: tb.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: bb.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: sz.masks.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: cb.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 6, resource: texels.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 7, resource: lists.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 8, resource: lb.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 9, resource: g.geom.params.as_entire_binding() },
            ],
        });
        let rbg = g.ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("raster"), layout: &g.geom.raster_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: g.world_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: sz.gbuf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: tb.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: bb.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 9, resource: g.geom.params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 10, resource: sz.racc.as_entire_binding() },
            ],
        });
        let (gx, gy) = self.groups();
        let tiles = gi.lamps.is_some();
        let th = (self.h as u32).div_ceil(16);
        // Big triangles per pixel, then (when there are small ones) their depth, their winner,
        // and the pixels they won.
        let sg = (gi.small.len() as u32).div_ceil(64);
        let small = (sg.min(65535), sg.div_ceil(65535));
        let raster = [
            ("g.raster", &g.geom.raster, (gx, gy)),
            ("g.raster.z", &g.geom.small_z, small),
            ("g.raster.pick", &g.geom.small_pick, small),
            ("g.raster.resolve", &g.geom.resolve, (gx, gy)),
        ];
        for (name, pipe, (x, y)) in raster.into_iter().take(if sg > 0 { 4 } else { 1 }) {
            let mut cp = self.pass(name);
            cp.set_bind_group(0, &rbg, &[0]);
            cp.set_pipeline(pipe);
            cp.dispatch_workgroups(x, y, 1);
        }
        {
            let mut cp = self.pass("g.shadows");
            cp.set_bind_group(0, &bg, &[0]);
            cp.set_pipeline(&g.geom.shadow);
            cp.dispatch_workgroups(gx, gy, 1);
        }
        if tiles {
            let mut cp = self.pass("g.tiles");
            cp.set_bind_group(0, &bg, &[0]);
            cp.set_pipeline(&g.geom.tiles);
            cp.dispatch_workgroups(tc, th, 1);
        }
        self.gpu_tiles = tiles.then_some(lists);
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
            self.sprite_texels(&mut st)
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
                wgpu::BindGroupEntry { binding: 0, resource: g.world_binding() },
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
        p.set_bind_group(0, &bg, &[0]);
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
                wgpu::BindGroupEntry { binding: 0, resource: g.world_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: gb.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: hd.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: sp.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: bn.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: g.splat.params[slot].as_entire_binding() },
            ],
        });
        let (gx, gy) = self.groups();
        let mut p = self.pass(if slot == 0 { "g.blades" } else { "g.splats" });
        p.set_bind_group(0, &bg, &[0]);
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
                wgpu::BindGroupEntry { binding: 0, resource: g.world_binding() },
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
            p.set_bind_group(0, &bg, &[0]);
            p.set_pipeline(&g.reflect.colsum_a);
            p.dispatch_workgroups(cols, blks, 1);
            p.set_pipeline(&g.reflect.colsum_b);
            p.dispatch_workgroups(cols, 1, 1);
        }
        let mut p = self.pass("g.reflect");
        p.set_bind_group(0, &bg, &[0]);
        p.set_pipeline(&g.reflect.reflect);
        p.dispatch_workgroups(gx, gy, 1);
    }

    /// Mist over the ground (hdr in place), each world's over its own pixels (None: no mist in
    /// that world). Needs `shade` to have run this frame (its lights).
    pub fn mist(&mut self, airs: &[Option<AirParams>]) {
        let g = self.g;
        let Some(bg) = self.shade_bg.clone() else { return };
        if airs.iter().all(|a| a.is_none()) { return; }
        self.submit();
        for (r, a) in airs.iter().enumerate() {
            if let Some(a) = a { g.ctx.queue.write_buffer(&g.shade.air, r as u64 * AIR_STRIDE, bytemuck::bytes_of(a)); }
        }
        let (gx, gy) = self.groups();
        let offs: Vec<(u32, u32)> = airs.iter().enumerate().filter(|(_, a)| a.is_some()).map(|(r, _)| (self.wo(r), (r as u64 * AIR_STRIDE) as u32)).collect();
        let k = (self.worlds > 1) as usize;
        let mut p = self.pass("g.mist");
        p.set_pipeline(&g.shade.mist[k]);
        for (wo, ao) in offs {
            p.set_bind_group(0, &bg, &[wo, ao]);
            p.dispatch_workgroups(gx, gy, 1);
        }
    }

    /// Light shafts (hdr in place), in the air of world `here`. `lists`: per tile of 16 x 16 cells,
    /// the lamps (indices into `lamps`: every world's point lights) that can light it.
    pub fn shafts(&mut self, sp: &ShaftParams, lists: &[Vec<u16>], lamps: &[[f32; 8]]) {
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
        let lights = g.upload(&g.shafts.lamps, "shaft lamps", bytemuck::cast_slice(lamps));
        let sz = self.bufs();
        let bg = g.ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shafts"), layout: &g.shafts.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: g.world_binding() },
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
        let wo = self.wo(self.here);
        let mut p = self.pass("g.shafts");
        p.set_bind_group(0, &bg, &[wo]);
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
                wgpu::BindGroupEntry { binding: 0, resource: g.world_binding() },
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
            let wo = self.wo(self.look);
            let mut cp = self.pass("g.post");
            cp.set_bind_group(0, &bg, &[wo]);
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

    /// The sky over every pixel where nothing is drawn (hdr in place), as world `r` sees it.
    pub fn sky(&mut self, input: &SkyInput, r: usize) {
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
                wgpu::BindGroupEntry { binding: 0, resource: g.world_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: g.sky.params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: sz.gbuf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: sz.hdr.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: prims.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: bins.as_entire_binding() },
            ],
        });
        let (gx, gy) = self.groups();
        let (wo, k) = (self.wo(r), (self.worlds > 1) as usize);
        let mut p = self.pass("g.sky");
        p.set_bind_group(0, &bg, &[wo]);
        p.set_pipeline(&g.sky.pipe[k]);
        p.dispatch_workgroups(gx, gy, 1);
    }

    /// render::shade: the G-buffer and masks (already on the GPU) to colour and reflection data,
    /// each world over its own pixels (inputs in world order; their lights in this order are what
    /// `light_base` counts), then the soft patch edges between them.
    pub fn shade(&mut self, inputs: &[ShadeInput]) {
        let g = self.g;
        let texs: Vec<[&Arc<Texture>; 7]> = inputs.iter().map(|i| i.textures).collect();
        let texels = g.texel_buffer(&texs);
        let mut lw: Vec<f32> = inputs.iter().flat_map(|i| i.lights.iter().chain(i.sky.iter())).flat_map(|l| l.iter().copied()).collect();
        if lw.is_empty() { lw.push(0.0); }
        self.submit();
        let lights = g.upload(&g.lights, "lights", bytemuck::cast_slice(&lw));
        let mut tw: Vec<u32> = Vec::new();
        if let Some(t) = inputs.first().and_then(|i| i.tiles.as_ref()) {
            let mut off = 0u32;
            for l in t { tw.push(off); off += l.len() as u32; }
            tw.push(off);
            for l in t { tw.extend(l.iter().map(|&k| k as u32)); }
        }
        if tw.is_empty() { tw.push(0); }
        let tiles = match self.gpu_tiles.clone() { Some(t) => t, None => g.upload(&g.tiles, "tiles", bytemuck::cast_slice(&tw)) };
        let sz = self.bufs();
        let bg = g.ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shade"), layout: &g.shade.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: g.world_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: sz.gbuf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: texels.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: lights.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: tiles.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: sz.masks.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 6, resource: sz.uvs.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 7, resource: sz.hdr.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 8, resource: sz.refl.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 9, resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &g.shade.air, offset: 0, size: wgpu::BufferSize::new(std::mem::size_of::<AirParams>() as u64) }) },
            ],
        });
        self.shade_bg = Some(bg.clone());
        let (gx, gy) = self.groups();
        let offs: Vec<u32> = (0..self.worlds).map(|r| self.wo(r)).collect();
        let soft = self.soft;
        let k = (self.worlds > 1) as usize;
        let mut p = self.pass("g.shade");
        p.set_pipeline(&g.shade.uvs[k]);
        for &o in &offs { p.set_bind_group(0, &bg, &[o, 0]); p.dispatch_workgroups(gx, gy, 1); }
        p.set_pipeline(&g.shade.shade[k]);
        for &o in &offs { p.set_bind_group(0, &bg, &[o, 0]); p.dispatch_workgroups(gx, gy, 1); }
        if soft {
            p.set_pipeline(&g.shade.soft);
            for &o in &offs { p.set_bind_group(0, &bg, &[o, 0]); p.dispatch_workgroups(gx, gy, 1); }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_params_match_the_wgsl_struct_size() {
        // A uniform block's size is a multiple of 16; the WGSL struct has the same fields.
        assert_eq!(std::mem::size_of::<WorldParams>() % 16, 0);
        assert_eq!(std::mem::size_of::<WorldParams>(), 4 * 216);
        assert_eq!(std::mem::size_of::<CardGpu>(), 4 * 42);
        assert_eq!(std::mem::size_of::<SplatGpu>(), 4 * 24);
        assert_eq!(std::mem::size_of::<AirParams>(), 48);
        assert_eq!(std::mem::size_of::<ShaftParams>(), 64);
        assert_eq!(std::mem::size_of::<PostParams>(), 384);
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
