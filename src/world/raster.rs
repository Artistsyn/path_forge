//! Geometry pass: rasterises world triangles into a G-buffer of depth, surface id and the world
//! position each pixel sees. Shading happens afterwards per pixel, so every surface is lit by the
//! same code and the buffer also serves outlines, shadows and depth layers.

use rayon::prelude::*;

/// Surface ids stored per pixel.
pub mod id {
    pub const NONE: u8 = 0;
    pub const GROUND: u8 = 1;
    pub const WALL_L: u8 = 2;
    pub const WALL_R: u8 = 3;
    pub const CEILING: u8 = 4;
    /// The vertical face of a step (path or verge material).
    pub const RISER: u8 = 5;
    /// The bottom of a bridge's gap.
    pub const CHASM: u8 = 6;
    /// The far wall of a bridge's gap, facing the camera.
    pub const CLIFF: u8 = 7;
    pub const PROP: u8 = 10;
    pub const FIXTURE: u8 = 11;
    pub const TUFT: u8 = 12;
    /// The face of the threshold between two worlds (a hillside, an end wall), facing the camera.
    pub const FACADE: u8 = 13;
    /// Bridge railings: posts, rails, parapets, pillars. One id per material and face, so each
    /// face is lit with its own normal: `RAIL + 4 * material + face`.
    pub const RAIL: u8 = 20;
    const RAIL_END: u8 = RAIL + 4 * 3;
    /// Railing materials: stone (the walls, or the path), wood (the deck), and `rail_color`.
    pub const STONE: u8 = 0;
    pub const WOOD: u8 = 1;
    pub const PAINT: u8 = 2;
    /// Railing faces: toward the path, away from it, the top, and the end facing the camera.
    pub const INNER: u8 = 0;
    pub const OUTER: u8 = 1;
    pub const TOP: u8 = 2;
    pub const FRONT: u8 = 3;
    pub const fn rail(material: u8, face: u8) -> u8 { RAIL + 4 * material + face }
    pub const fn is_rail(id: u8) -> bool { id >= RAIL && id < RAIL_END }
    /// (material, face) of a railing id.
    pub const fn rail_parts(id: u8) -> (u8, u8) { ((id - RAIL) / 4, (id - RAIL) % 4) }
}

/// Laid out as the GPU passes read it (`GPix` in common.wgsl: depth, x, y, d, then id | realm << 8
/// as one word), so the G-buffer goes to the GPU and back as it is.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
#[cfg_attr(feature = "gpu", derive(bytemuck::Pod, bytemuck::Zeroable))]
pub struct GPixel {
    /// Camera-space depth (z), metres. f32::INFINITY where nothing was drawn.
    pub depth: f32,
    /// World position seen: x sideways from the centreline, y height, d distance ahead.
    pub x: f32,
    pub y: f32,
    pub d: f32,
    pub id: u8,
    /// Which world the pixel shows in a transition: 0 this side of the boundary, 1 beyond it.
    pub realm: u8,
    /// Always 0 (the rest of the GPU's id word).
    pub pad: u16,
}

impl GPixel {
    pub const EMPTY: GPixel = GPixel { depth: f32::INFINITY, id: id::NONE, x: 0.0, y: 0.0, d: 0.0, realm: 0, pad: 0 };
}

/// A triangle vertex in camera space, carrying the world position it came from.
#[derive(Clone, Copy, Debug)]
pub struct Vert {
    pub cam: [f32; 3],
    pub world: [f32; 3],
}

#[derive(Clone, Copy, Debug)]
pub struct Tri {
    pub v: [Vert; 3],
    pub id: u8,
    pub realm: u8,
}

/// A triangle set up for rasterising: on screen, with what is interpolated, and its pixel box.
pub(crate) struct Prepared {
    pub p: [[f32; 2]; 3],
    pub inv_z: [f32; 3],
    pub w_over_z: [[f32; 3]; 3],
    pub min_y: i32,
    pub max_y: i32,
    pub min_x: i32,
    pub max_x: i32,
    pub area: f32,
    pub id: u8,
    pub realm: u8,
}

impl Prepared {
    /// Each edge (a-b, b-c, c-a) as `rasterize` reads it: (ex, ey, the opposite vertex's side).
    pub fn edges(&self) -> [[f32; 3]; 3] {
        let [a, b, c] = self.p;
        [(a, b, c), (b, c, a), (c, a, b)].map(|(e0, e1, opp)| {
            let (ex, ey) = (e1[0] - e0[0], e1[1] - e0[1]);
            [ex, ey, ex * (opp[1] - e0[1]) - ey * (opp[0] - e0[0])]
        })
    }
}

/// A triangle whose box is under this many pixels each way is rasterised on the GPU by one
/// thread of its own instead of from the tile lists (gpu/geom.wgsl's raster_main).
pub(crate) const SMALL_TRI: i32 = 32;

/// The 16 x 16 tiles a prepared triangle can draw into, pushed to `out` in increasing order: its
/// rows' spans as `raster_prepared` solves them, widened by a pixel each way so a last-bit
/// difference in the GPU's arithmetic can never lose a pixel. A thin triangle that crosses many
/// tiles' boxes without covering their pixels (a far railing bar) is listed in none of them.
pub(crate) fn tiles_touched(t: &Prepared, cols: usize, out: &mut Vec<u32>) {
    let [a, b, c] = t.p;
    let edges = [(a, b, c), (b, c, a), (c, a, b)].map(|(e0, e1, opp)| {
        let (ex, ey) = (e1[0] - e0[0], e1[1] - e0[1]);
        (e0, ex, ey, ex * (opp[1] - e0[1]) - ey * (opp[0] - e0[0]))
    });
    for ty in (t.min_y / 16)..=(t.max_y / 16) {
        let (mut lo, mut hi) = (i32::MAX, i32::MIN);
        for y in t.min_y.max(ty * 16)..=t.max_y.min(ty * 16 + 15) {
            let py = y as f32 + 0.5;
            let (mut xl, mut xr) = (t.min_x as f32, t.max_x as f32 + 1.0);
            for &(e0, ex, ey, s_opp) in &edges {
                // A horizontal edge only ever empties a whole row: keep the row (conservative).
                if ey.abs() < 1e-12 { continue; }
                let x_cross = (ex * (py - e0[1]) + ey * e0[0]) / ey;
                if (ey > 0.0) == (s_opp > 0.0) { xr = xr.min(x_cross); } else { xl = xl.max(x_cross); }
            }
            let (xl, xr) = (xl - 1.0, xr + 1.0);
            if !(xl < xr) { continue; }
            let xs = ((xl - 0.5).ceil() as i32).max(t.min_x);
            let xe = ((xr - 0.5).floor() as i32).min(t.max_x);
            if xs <= xe { lo = lo.min(xs); hi = hi.max(xe); }
        }
        if lo <= hi {
            for tx in (lo / 16)..=(hi / 16) { out.push(ty as u32 * cols as u32 + tx as u32); }
        }
    }
}

/// Rasterise triangles (all vertices in front of the camera) into `gbuf` with depth testing.
pub fn rasterize(gbuf: &mut [GPixel], w: usize, h: usize, tris: &[Tri], view: &super::view::View) {
    let prepared = prepare(w, h, tris, view);
    raster_prepared(gbuf, w, &prepared);
}

/// Set up every triangle that shows (`rasterize`'s first half; the GPU rasterises from these too).
pub(crate) fn prepare(w: usize, h: usize, tris: &[Tri], view: &super::view::View) -> Vec<Prepared> {
    tris.par_iter().filter_map(|t| {
        let mut p = [[0.0f32; 2]; 3];
        for i in 0..3 { p[i] = view.project(t.v[i].cam)?; }
        let area = (p[1][0] - p[0][0]) * (p[2][1] - p[0][1]) - (p[2][0] - p[0][0]) * (p[1][1] - p[0][1]);
        if area.abs() < 1e-6 { return None; }
        let min_x = p.iter().map(|q| q[0]).fold(f32::INFINITY, f32::min).floor().max(0.0) as i32;
        let max_x = p.iter().map(|q| q[0]).fold(f32::NEG_INFINITY, f32::max).ceil().min(w as f32 - 1.0) as i32;
        let min_y = p.iter().map(|q| q[1]).fold(f32::INFINITY, f32::min).floor().max(0.0) as i32;
        let max_y = p.iter().map(|q| q[1]).fold(f32::NEG_INFINITY, f32::max).ceil().min(h as f32 - 1.0) as i32;
        if min_x > max_x || min_y > max_y { return None; }
        let inv_z = [1.0 / t.v[0].cam[2], 1.0 / t.v[1].cam[2], 1.0 / t.v[2].cam[2]];
        let w_over_z = [0, 1, 2].map(|i| [t.v[i].world[0] * inv_z[i], t.v[i].world[1] * inv_z[i], t.v[i].world[2] * inv_z[i]]);
        Some(Prepared { p, inv_z, w_over_z, min_y, max_y, min_x, max_x, area, id: t.id, realm: t.realm })
    }).collect()
}

fn raster_prepared(gbuf: &mut [GPixel], w: usize, prepared: &[Prepared]) {
    const BAND: usize = 8;
    gbuf.par_chunks_mut(w * BAND).enumerate().for_each(|(band, rows)| {
        let y0 = (band * BAND) as i32;
        let y1 = y0 + (rows.len() / w) as i32 - 1;
        for t in prepared {
            if t.max_y < y0 || t.min_y > y1 { continue; }
            let inv_area = 1.0 / t.area;
            let [a, b, c] = t.p;
            for y in t.min_y.max(y0)..=t.max_y.min(y1) {
                let py = y as f32 + 0.5;
                let row = &mut rows[(y - y0) as usize * w..(y - y0 + 1) as usize * w];
                // Solve the span of this row inside the triangle from the three edges.
                let mut xl = t.min_x as f32;
                let mut xr = t.max_x as f32 + 1.0;
                for (e0, e1, opp) in [(a, b, c), (b, c, a), (c, a, b)] {
                    // Edge function E(x) = (e1-e0) x (p-e0), sign must match the opposite vertex.
                    let ex = e1[0] - e0[0];
                    let ey = e1[1] - e0[1];
                    let s_opp = ex * (opp[1] - e0[1]) - ey * (opp[0] - e0[0]);
                    // E(x) = ex*(py-e0y) - ey*(x-e0x) = k - ey*x
                    let k = ex * (py - e0[1]) + ey * e0[0];
                    if ey.abs() < 1e-12 {
                        if (k) * s_opp < 0.0 { xl = f32::INFINITY; break; }
                        continue;
                    }
                    let x_cross = k / ey; // where E = 0
                    // Inside when sign(E) == sign(s_opp); E decreases in x when ey > 0.
                    let inside_left = (ey > 0.0) == (s_opp > 0.0);
                    if inside_left { xr = xr.min(x_cross); } else { xl = xl.max(x_cross); }
                }
                if !(xl < xr) { continue; }
                let xs = (xl - 0.5).ceil().max(t.min_x as f32) as i32;
                let xe = ((xr - 0.5).floor() as i32).min(t.max_x);
                for x in xs..=xe {
                    let px = x as f32 + 0.5;
                    let b0 = ((b[0] - px) * (c[1] - py) - (c[0] - px) * (b[1] - py)) * inv_area;
                    let b1 = ((c[0] - px) * (a[1] - py) - (a[0] - px) * (c[1] - py)) * inv_area;
                    let b2 = 1.0 - b0 - b1;
                    let iz = b0 * t.inv_z[0] + b1 * t.inv_z[1] + b2 * t.inv_z[2];
                    if iz <= 0.0 { continue; }
                    let z = 1.0 / iz;
                    let g = &mut row[x as usize];
                    if z >= g.depth { continue; }
                    let wz = |k: usize| (b0 * t.w_over_z[0][k] + b1 * t.w_over_z[1][k] + b2 * t.w_over_z[2][k]) * z;
                    *g = GPixel { depth: z, id: t.id, x: wz(0), y: wz(1), d: wz(2), realm: t.realm, pad: 0 };
                }
            }
        }
    });
}

/// Two triangles for a quad given in order around its edge.
pub fn quad(out: &mut Vec<Tri>, v: [Vert; 4], id: u8) {
    out.push(Tri { v: [v[0], v[1], v[2]], id, realm: 0 });
    out.push(Tri { v: [v[0], v[2], v[3]], id, realm: 0 });
}

/// A half-plane in camera space: `a * x + b * z + c >= 0` (x right, z ahead).
pub type Half = (f32, f32, f32);

/// The parts of `tris` inside a region made of convex pieces (each the intersection of its
/// half-planes; an empty piece is everything), tagged with `realm`. Cuts are exact: camera space is
/// where the triangles are flat.
pub fn clip_region(tris: Vec<Tri>, pieces: &[Vec<Half>], realm: u8) -> Vec<Tri> {
    let mut out = Vec::with_capacity(tris.len());
    let lerp = |a: &Vert, b: &Vert, t: f32| Vert {
        cam: [0, 1, 2].map(|k| a.cam[k] + (b.cam[k] - a.cam[k]) * t),
        world: [0, 1, 2].map(|k| a.world[k] + (b.world[k] - a.world[k]) * t),
    };
    let side = |v: &Vert, h: &Half| h.0 * v.cam[0] + h.1 * v.cam[2] + h.2;
    let cut = |poly: Vec<Vert>, h: &Half| -> Vec<Vert> {
        let mut res = Vec::with_capacity(poly.len() + 2);
        for i in 0..poly.len() {
            let (a, b) = (&poly[i], &poly[(i + 1) % poly.len()]);
            let (fa, fb) = (side(a, h), side(b, h));
            if fa >= 0.0 { res.push(*a); }
            if (fa >= 0.0) != (fb >= 0.0) { res.push(lerp(a, b, fa / (fa - fb))); }
        }
        res
    };
    for t in tris {
        for piece in pieces {
            let mut poly = t.v.to_vec();
            for h in piece {
                if poly.iter().all(|v| side(v, h) >= 0.0) { continue; }
                if poly.iter().all(|v| side(v, h) < 0.0) { poly.clear(); break; }
                poly = cut(poly, h);
                if poly.len() < 3 { break; }
            }
            for k in 1..poly.len().saturating_sub(1) {
                out.push(Tri { v: [poly[0], poly[k], poly[k + 1]], id: t.id, realm });
            }
        }
    }
    out
}
