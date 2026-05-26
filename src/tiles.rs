/// Procedural tile generator — direct Rust port of genTile() from PathForgeLive.jsx.
/// Returns TILE×TILE×3 bytes of RGB pixel data.
use crate::settings::{FloorPattern, WallPattern};

const BASE_TILE: usize = 32;
const TILE_TEX: usize = 192;

#[inline]
fn s(v: usize) -> usize { (v * TILE_TEX) / BASE_TILE }

#[inline]
fn sw(v: usize) -> usize { ((v * TILE_TEX) / BASE_TILE).max(1) }

// ── XorShift32 — matches JSX seededRng() exactly ─────────────────────────
pub fn seeded_rng(seed: u32) -> impl FnMut() -> f32 {
    let mut s = seed ^ 0xdeadbeef;
    move || {
        s ^= s.wrapping_shl(13);
        s ^= s.wrapping_shr(17);
        s ^= s.wrapping_shl(5);
        s as f64 as f32 / 4_294_967_296.0
    }
}

// ── Tile building helpers ──────────────────────────────────────────────────
fn set_px(data: &mut [u8], x: usize, y: usize, c: [u8; 3]) {
    if x < TILE_TEX && y < TILE_TEX {
        let i = (y * TILE_TEX + x) * 3;
        data[i] = c[0]; data[i+1] = c[1]; data[i+2] = c[2];
    }
}
fn row(data: &mut [u8], y: usize, c: [u8; 3]) {
    for x in 0..TILE_TEX { set_px(data, x, y, c); }
}
fn fill(data: &mut [u8], x0: usize, y0: usize, w: usize, h: usize, c: [u8; 3]) {
    for y in y0..y0+h { for x in x0..x0+w { set_px(data, x, y, c); } }
}

#[inline]
fn tint(c: [u8; 3], delta: i32) -> [u8; 3] {
    [
        (c[0] as i32 + delta).clamp(0, 255) as u8,
        (c[1] as i32 + delta).clamp(0, 255) as u8,
        (c[2] as i32 + delta).clamp(0, 255) as u8,
    ]
}

#[inline]
fn hash_u32(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846ca68b);
    x ^= x >> 16;
    x
}

// ── Generic entry point ───────────────────────────────────────────────────
pub fn gen_floor_tile(pattern: &FloorPattern, base: [u8; 3], mortar: [u8; 3], noise: u32, damage: f32, seed: u32) -> Vec<u8> {
    let name = match pattern {
        FloorPattern::Cobblestone => "Cobblestone",
        FloorPattern::Brick       => "Brick",
        FloorPattern::StoneBlock  => "Stone Block",
        FloorPattern::Sand        => "Sand",
        FloorPattern::Dirt        => "Dirt",
        FloorPattern::Grass       => "Grass",
    };
    gen_tile(name, base, mortar, noise, damage, seed)
}

pub fn gen_wall_tile(pattern: &WallPattern, base: [u8; 3], mortar: [u8; 3], noise: u32, damage: f32, seed: u32) -> Vec<u8> {
    let name = match pattern {
        WallPattern::StoneBlock  => "Stone Block",
        WallPattern::Brick       => "Brick",
        WallPattern::Bark        => "Bark",
        WallPattern::RockFace    => "Rock Face",
        WallPattern::Hedge       => "Hedge",
        WallPattern::Cobblestone => "Cobblestone",
    };
    gen_tile(name, base, mortar, noise, damage, seed)
}

/// Core tile generator — port of JSX genTile().
pub fn gen_tile(pattern: &str, base: [u8; 3], mortar: [u8; 3], noise: u32, damage: f32, seed: u32) -> Vec<u8> {
    let mut data = vec![0u8; TILE_TEX * TILE_TEX * 3];
    // Fill with base colour
    for i in 0..TILE_TEX*TILE_TEX {
        data[i*3]   = base[0];
        data[i*3+1] = base[1];
        data[i*3+2] = base[2];
    }

    match pattern {
        "Cobblestone" => {
            let mut rng = seeded_rng(seed.wrapping_add(71));
            let n = 8usize;
            let cell = TILE_TEX as f32 / n as f32;
            let mut centers: Vec<(f32, f32, [u8; 3])> = Vec::with_capacity(n * n);
            for gy in 0..n {
                for gx in 0..n {
                    let jx = (rng() - 0.5) * cell * 0.65;
                    let jy = (rng() - 0.5) * cell * 0.65;
                    let cx = gx as f32 * cell + cell * 0.5 + jx;
                    let cy = gy as f32 * cell + cell * 0.5 + jy;
                    let td = (rng() * 30.0 - 15.0) as i32;
                    centers.push((cx, cy, tint(base, td)));
                }
            }
            let mortar_w = (TILE_TEX as f32 / 86.0).max(1.2);
            for y in 0..TILE_TEX {
                for x in 0..TILE_TEX {
                    let xf = x as f32;
                    let yf = y as f32;
                    let mut d1 = f32::MAX;
                    let mut d2 = f32::MAX;
                    let mut col = base;
                    for (cx, cy, c) in &centers {
                        let dx0 = (xf - *cx).abs();
                        let dy0 = (yf - *cy).abs();
                        let dx = dx0.min(TILE_TEX as f32 - dx0);
                        let dy = dy0.min(TILE_TEX as f32 - dy0);
                        let d = (dx * dx + dy * dy).sqrt();
                        if d < d1 {
                            d2 = d1;
                            d1 = d;
                            col = *c;
                        } else if d < d2 {
                            d2 = d;
                        }
                    }
                    if d2 - d1 < mortar_w {
                        set_px(&mut data, x, y, mortar);
                    } else {
                        // Subtle convex stone shading toward stone centre.
                        let shade = ((mortar_w * 2.0 - d1).max(-8.0).min(8.0)) as i32;
                        set_px(&mut data, x, y, tint(col, shade / 3));
                    }
                }
            }
        }
        "Brick" => {
            let brick_w = (TILE_TEX / 5).max(20);
            let brick_h = (TILE_TEX / 12).max(10);
            let mortar_w = (TILE_TEX / 96).max(1);
            for y in 0..TILE_TEX {
                let row_idx = y / brick_h;
                let y_in = y % brick_h;
                if y_in < mortar_w {
                    row(&mut data, y, mortar);
                    continue;
                }
                let off = if row_idx % 2 == 0 { 0 } else { brick_w / 2 };
                for x in 0..TILE_TEX {
                    let xs = x + off;
                    let col_idx = xs / brick_w;
                    let x_in = xs % brick_w;
                    if x_in < mortar_w {
                        set_px(&mut data, x, y, mortar);
                        continue;
                    }
                    let key = hash_u32((row_idx as u32) * 131 + col_idx as u32 + seed.wrapping_mul(17));
                    let td = ((key & 31) as i32) - 15;
                    let mut bc = tint(base, td);
                    // Brick edge wear + centre highlight.
                    let edge = x_in.min(brick_w - 1 - x_in).min(y_in.min(brick_h - 1 - y_in));
                    if edge < mortar_w + 1 {
                        bc = tint(bc, -10);
                    } else if edge > mortar_w + 4 {
                        bc = tint(bc, 4);
                    }
                    set_px(&mut data, x, y, bc);
                }
            }
        }
        "Stone Block" => {
            let bw = (TILE_TEX / 6).max(24);
            let bh = (TILE_TEX / 5).max(24);
            let mw = (TILE_TEX / 96).max(1);
            for y in 0..TILE_TEX {
                let ry = y % bh;
                if ry < mw {
                    row(&mut data, y, mortar);
                    continue;
                }
                for x in 0..TILE_TEX {
                    let rx = x % bw;
                    if rx < mw {
                        set_px(&mut data, x, y, mortar);
                        continue;
                    }
                    let row_idx = y / bh;
                    let col_idx = x / bw;
                    let key = hash_u32((row_idx as u32) * 257 + col_idx as u32 + seed.wrapping_mul(29));
                    let td = ((key & 23) as i32) - 11;
                    let mut sc = tint(base, td);
                    let edge = rx.min(bw - 1 - rx).min(ry.min(bh - 1 - ry));
                    if edge < mw + 1 { sc = tint(sc, -9); }
                    set_px(&mut data, x, y, sc);
                }
            }
        }
        "Bark" => {
            let mut rng = seeded_rng(seed + 10);
            let mut x = 0usize;
            while x < TILE_TEX {
                x += sw(3 + (rng() * 5.0) as usize);
                if x < TILE_TEX { for yy in 0..TILE_TEX { set_px(&mut data, x, yy, mortar); } }
            }
            let mut y = 0usize;
            while y < TILE_TEX {
                y += sw(4 + (rng() * 6.0) as usize);
                if y < TILE_TEX { row(&mut data, y, mortar); }
            }
        }
        "Rock Face" => {
            let mut rng = seeded_rng(seed + 20);
            for _ in 0..(8 * TILE_TEX / BASE_TILE) {
                let mut x = (rng() * TILE_TEX as f32) as usize;
                let y     = (rng() * TILE_TEX as f32) as usize;
                let dx    = rng() * 7.0 - 3.0;
                let steps = sw(5 + (rng() * 10.0) as usize);
                for s in 0..steps {
                    let nx = (x as i32 + (dx * s as f32 / steps as f32) as i32)
                        .rem_euclid(TILE_TEX as i32) as usize;
                    let ny = (y + s).min(TILE_TEX - 1);
                    set_px(&mut data, nx, ny, mortar);
                    x = nx;
                }
            }
        }
        "Sand" => {
            for y in (0..TILE_TEX).step_by(sw(3)) {
                for x in 0..TILE_TEX {
                    if (x * 7 + y * 3) % 13 < 2 { set_px(&mut data, x, y, mortar); }
                }
            }
        }
        "Dirt" => {
            let mut rng = seeded_rng(seed + 30);
            let count = 40 * TILE_TEX * TILE_TEX / (BASE_TILE * BASE_TILE);
            for _ in 0..count {
                let x = (rng() * TILE_TEX as f32) as usize;
                let y = (rng() * TILE_TEX as f32) as usize;
                let w = sw(1 + (rng() * 3.0) as usize);
                fill(&mut data, x, y, w, 1, mortar);
            }
        }
        "Grass" => {
            let mut rng = seeded_rng(seed + 40);
            let mut x = 0;
            while x < TILE_TEX {
                if rng() > 0.4 {
                    let h = sw(2 + (rng() * 6.0) as usize);
                    let blade = [
                        (base[0] as i32 + 10).clamp(0, 255) as u8,
                        (base[1] as i32 + 28).clamp(0, 255) as u8,
                        base[2],
                    ];
                    fill(&mut data, x, TILE_TEX - h, sw(1), h, blade);
                }
                x += sw(2);
            }
        }
        "Hedge" => {
            let mut rng = seeded_rng(seed + 50);
            let mut y = 0;
            while y < TILE_TEX {
                let mut x = 0;
                while x < TILE_TEX {
                    if rng() > 0.4 {
                        let dab = [
                            (base[0] as i32 + (rng() * 20.0 - 8.0) as i32).clamp(0,255) as u8,
                            (base[1] as i32 + (rng() * 28.0 - 5.0) as i32).clamp(0,255) as u8,
                            base[2],
                        ];
                        fill(&mut data, x, y, sw(2), sw(2), dab);
                    }
                    x += sw(3);
                }
                y += sw(3);
            }
        }
        _ => {}
    }

    apply_damage_overlay(&mut data, base, mortar, damage, seed.wrapping_add(909));

    // Noise pass (luma-preserving) — avoids RGB color-splotch artifacts.
    let mut rng = seeded_rng(seed);
    let px_count = TILE_TEX * TILE_TEX;
    for i in 0..px_count {
        let delta = ((rng() * 2.0 - 1.0) * noise as f32 * 0.45) as i32;
        let p = i * 3;
        data[p]     = (data[p] as i32 + delta).clamp(2, 253) as u8;
        data[p + 1] = (data[p + 1] as i32 + delta).clamp(2, 253) as u8;
        data[p + 2] = (data[p + 2] as i32 + delta).clamp(2, 253) as u8;
    }

    data
}

fn blend_px(data: &mut [u8], x: usize, y: usize, c: [u8; 3], a: f32) {
    if x >= TILE_TEX || y >= TILE_TEX { return; }
    let i = (y * TILE_TEX + x) * 3;
    let inv = (1.0 - a).clamp(0.0, 1.0);
    data[i] = (data[i] as f32 * inv + c[0] as f32 * a).clamp(0.0, 255.0) as u8;
    data[i + 1] = (data[i + 1] as f32 * inv + c[1] as f32 * a).clamp(0.0, 255.0) as u8;
    data[i + 2] = (data[i + 2] as f32 * inv + c[2] as f32 * a).clamp(0.0, 255.0) as u8;
}

fn apply_damage_overlay(data: &mut [u8], base: [u8; 3], mortar: [u8; 3], damage: f32, seed: u32) {
    let d = damage.clamp(0.0, 1.0);
    if d <= 0.001 { return; }
    let mut rng = seeded_rng(seed);
    let crack_count = (4.0 + 42.0 * d) as usize;
    let chip_count = (12.0 + 80.0 * d) as usize;
    let crack_col = [
        ((mortar[0] as f32 * 0.85 + base[0] as f32 * 0.15).clamp(0.0, 255.0)) as u8,
        ((mortar[1] as f32 * 0.85 + base[1] as f32 * 0.15).clamp(0.0, 255.0)) as u8,
        ((mortar[2] as f32 * 0.85 + base[2] as f32 * 0.15).clamp(0.0, 255.0)) as u8,
    ];

    for _ in 0..crack_count {
        let mut x = rng() * TILE_TEX as f32;
        let mut y = rng() * TILE_TEX as f32;
        let mut ang = rng() * std::f32::consts::TAU;
        let steps = (8.0 + rng() * (TILE_TEX as f32 * (0.15 + d * 0.85))) as usize;
        let width = if rng() < d * 0.6 { 2 } else { 1 };
        for _ in 0..steps {
            let xi = x.rem_euclid(TILE_TEX as f32) as usize;
            let yi = y.rem_euclid(TILE_TEX as f32) as usize;
            blend_px(data, xi, yi, crack_col, 0.45 + d * 0.4);
            if width > 1 {
                blend_px(data, (xi + 1) % TILE_TEX, yi, crack_col, 0.25 + d * 0.2);
                blend_px(data, xi, (yi + 1) % TILE_TEX, crack_col, 0.25 + d * 0.2);
            }
            ang += (rng() - 0.5) * (0.24 + d * 0.28);
            x += ang.cos() * (0.9 + d * 0.7);
            y += ang.sin() * (0.9 + d * 0.7);
        }
    }

    for _ in 0..chip_count {
        let cx = (rng() * TILE_TEX as f32) as usize;
        let cy = (rng() * TILE_TEX as f32) as usize;
        let rad = (1.0 + rng() * (1.0 + d * 3.2)) as i32;
        let chip = [
            (base[0] as f32 * (0.82 + rng() * 0.24)).clamp(0.0, 255.0) as u8,
            (base[1] as f32 * (0.82 + rng() * 0.24)).clamp(0.0, 255.0) as u8,
            (base[2] as f32 * (0.82 + rng() * 0.24)).clamp(0.0, 255.0) as u8,
        ];
        for oy in -rad..=rad {
            for ox in -rad..=rad {
                if ox * ox + oy * oy > rad * rad { continue; }
                let x = (cx as i32 + ox).rem_euclid(TILE_TEX as i32) as usize;
                let y = (cy as i32 + oy).rem_euclid(TILE_TEX as i32) as usize;
                let a = 0.12 + d * 0.22;
                blend_px(data, x, y, chip, a);
            }
        }
    }
}

// ── Cache key ─────────────────────────────────────────────────────────────
#[derive(PartialEq, Clone)]
pub struct TileKey {
    pub pattern: String,
    pub base:    [u8; 3],
    pub mortar:  [u8; 3],
    pub noise:   u32,
    pub seed:    u32,
}
