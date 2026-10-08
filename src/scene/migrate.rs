//! Best-effort conversion of PathForge 2.0 settings into a v3 scene.
//!
//! 2.0 mixed screen pixels and two different world scales, so there is no exact mapping. The aim
//! is that an old preset opens recognisably (same materials, colours, lights and timing) and
//! then gets retuned in metres.

use super::*;
use crate::settings::{self as v2, PathForgeSettings};

/// One 2.0 scroll unit ("tile") in metres.
const TILE_M: f32 = 4.0;

fn pattern_from_floor(p: &v2::FloorPattern) -> Pattern {
    match p {
        v2::FloorPattern::Cobblestone => Pattern::Cobblestone, v2::FloorPattern::Brick => Pattern::Brick,
        v2::FloorPattern::StoneBlock => Pattern::StoneBlock, v2::FloorPattern::Sand => Pattern::Sand,
        v2::FloorPattern::Dirt => Pattern::Dirt, v2::FloorPattern::Grass => Pattern::Grass,
    }
}

fn pattern_from_wall(p: &v2::WallPattern) -> Pattern {
    match p {
        v2::WallPattern::StoneBlock => Pattern::StoneBlock, v2::WallPattern::Brick => Pattern::Brick,
        v2::WallPattern::Bark => Pattern::Bark, v2::WallPattern::RockFace => Pattern::RockFace,
        v2::WallPattern::Hedge => Pattern::Hedge, v2::WallPattern::Cobblestone => Pattern::Cobblestone,
    }
}

fn sprite(path: &str, pool: &str, pool_on: bool, flip_x: bool) -> SpriteRef {
    let pool = if pool_on {
        pool.split([';', '\n', '\r']).map(str::trim).filter(|p| !p.is_empty()).map(str::to_owned).collect()
    } else {
        Vec::new()
    };
    SpriteRef { path: path.trim().to_owned(), pool, flip_x, pixelated: false }
}

fn side_from_mount(m: &v2::MountSide) -> Side {
    match m { v2::MountSide::Both => Side::Both, v2::MountSide::Left => Side::Left, v2::MountSide::Right => Side::Right, v2::MountSide::Center => Side::Center }
}

pub fn from_v2(o: &PathForgeSettings, name: &str) -> Scene {
    let w = o.canvas.w() as f32;
    let h = o.canvas.h() as f32;
    let hy = (o.scene.horizon_y as f32 * h / 768.0).clamp(8.0, h - 8.0);
    let eye = (o.scene.cam_h * 0.8).clamp(0.5, 5.0);
    let zoom = o.scene.focal_mult.clamp(0.3, 4.0);
    // Keep the on-screen path width at the bottom row.
    let bottom_px = o.scene.max_hw * w / 576.0;
    let half_width = (bottom_px * eye / (h - hy)).clamp(0.3, 6.0);
    let tile_size = |tex_scale: f32| (7.0 / tex_scale.max(0.1)).clamp(0.4, 12.0);

    let path = PathShape {
        half_width,
        flare: (1.0 - o.scene.path_power).clamp(0.0, 0.85),
        bend: (o.scene.horizon_curve * 0.5).clamp(-1.0, 1.0),
        hill: 0.0,
        edge_noise: 0.1,
        edge_dark: (o.floor.edge_vignette * 0.5).clamp(0.0, 1.0),
        material: Material {
            pattern: pattern_from_floor(&o.floor.pattern), base: o.floor.base, mortar: o.floor.mortar,
            noise: o.floor.noise, damage: o.floor.damage, seed: o.floor.variation_seed,
            tile_size: tile_size(o.floor.tex_scale), rotate: o.floor.tex_rot_90, brightness: 1.3, ..Material::default()
        },
        stairs: Stairs::default(),
        bridge: Bridge::default(),
        edge_lights: EdgeLights::default(),
        surface: true,
        fork: Fork::default(),
    };

    let walls = Walls {
        enabled: o.walls.enabled,
        gap: (o.walls.l_wx * 0.8 - half_width).max(0.1),
        height: if o.walls.top_coverage >= 0.98 { 0.0 } else { eye * (1.3 + o.walls.top_coverage * 4.0) },
        base_shadow: (1.0 - o.walls.junc_shadow / 60.0).clamp(0.0, 0.9),
        material: Material {
            pattern: pattern_from_wall(&o.walls.pattern), base: o.walls.base, mortar: o.walls.mortar,
            noise: o.walls.noise, damage: o.walls.damage, seed: o.walls.variation_seed,
            tile_size: tile_size(o.walls.tex_scale), rotate: o.walls.tex_rot_90,
            brightness: (o.walls.bright / 1.5).clamp(0.3, 3.0), ..Material::default()
        },
    };

    let sky = Sky {
        enabled: o.sky.enabled, top: o.sky.top, horizon: o.sky.horizon,
        sun: SkyBody {
            enabled: o.sky.sun_enabled, pos: o.sky.sun_pos, radius: o.sky.sun_radius, color: o.sky.sun_color,
            emits_light: o.sky.sun_emits_light, intensity: 1.0,
        },
        moon: Moon {
            body: SkyBody {
                enabled: o.sky.moon_enabled, pos: o.sky.moon_pos, radius: o.sky.moon_radius, color: o.sky.moon_color,
                emits_light: o.sky.moon_emits_light, intensity: 0.35,
            },
            phase: o.sky.moon_phase, opacity: o.sky.moon_alpha.clamp(0.0, 1.0), craters: o.sky.moon_texture_enabled,
        },
        stars: Stars { enabled: o.sky.stars_enabled, count: o.sky.stars_count, size: o.sky.stars_size / 1.4, twinkle: o.sky.stars_twinkle, seed: o.sky.stars_seed },
        clouds: Clouds {
            enabled: o.sky.clouds_enabled, count: o.sky.cloud_count, drift: o.sky.cloud_speed.max(0.0).round().max(1.0),
            scale: o.sky.cloud_scale, opacity: o.sky.cloud_opacity, tint: o.sky.cloud_tint, variation: o.sky.cloud_variation,
            seed: o.sky.cloud_seed, ..Clouds::default()
        },
        ..Sky::default()
    };

    let fog_color = if o.post.fog_enabled { o.post.fog_color } else { o.scene.void_color };
    let light = Lighting {
        ambient: (o.scene.ambient * 0.45).clamp(0.05, 2.0),
        ambient_color: [190, 195, 215],
        void_color: o.scene.void_color,
        fog: Fog { enabled: true, color: fog_color, distance: (60.0 / o.floor.depth_fade.max(0.5)).clamp(8.0, 120.0), match_sky: o.sky.enabled },
        bands: 0,
    };

    let mut fixtures = Vec::new();
    let mut particles = Vec::new();
    for l in &o.atmo.layers {
        if l.n_motes > 0 {
            particles.push(Particles { enabled: l.enabled, count: l.n_motes * 10, ..Particles::default() });
        }
        let kind = match l.atmo_type {
            v2::AtmoType::None => continue,
            v2::AtmoType::Torch => FixtureKind::Torch, v2::AtmoType::Lantern => FixtureKind::Lantern,
            v2::AtmoType::Firefly => FixtureKind::Firefly, v2::AtmoType::Magic => FixtureKind::Magic,
            v2::AtmoType::GreenFire => FixtureKind::GreenFire, v2::AtmoType::Candle => FixtureKind::Candle,
            v2::AtmoType::IceWisp => FixtureKind::IceWisp,
        };
        let mount = match l.mount_surface {
            v2::AttachmentSurface::Wall => Mount::Wall, v2::AttachmentSurface::Floor => Mount::Ground,
            v2::AttachmentSurface::Ceiling => Mount::Ceiling, v2::AttachmentSurface::Floating => Mount::Floating,
        };
        fixtures.push(Fixture {
            enabled: l.enabled, kind, side: side_from_mount(&l.mount_side), mount,
            height: (l.torch_h * 0.85).clamp(0.0, 12.0), spacing: l.torch_spc.max(1) as f32 * TILE_M, offset: 0.0,
            lateral: 0.3, size: (l.torch_scale / 0.068 * l.fx_scale).clamp(0.2, 6.0), light: l.emits_light,
            intensity: 1.0, radius: 6.5, flicker: l.flicker, jitter: l.placement_jitter.min(2.0),
            sprite: sprite(&l.sprite_path, &l.sprite_pool_paths, l.sprite_pool_enabled, l.sprite_flip_x),
            seed: l.variation_seed,
        });
    }

    let props = o.props.items.iter().map(|p| {
        let kind = match p.prop_type {
            v2::PropType::Tree => PropKind::Tree, v2::PropType::PineTree => PropKind::Pine, v2::PropType::Bush => PropKind::Bush,
            v2::PropType::Rock => PropKind::Rock, v2::PropType::Boulder => PropKind::Boulder, v2::PropType::Cactus => PropKind::Cactus,
            v2::PropType::DeadTree => PropKind::DeadTree, v2::PropType::Mushroom => PropKind::Mushroom,
        };
        let side = if p.mirror { Side::Both } else if p.wx < 0.0 { Side::Left } else { Side::Right };
        PropLayer {
            enabled: p.enabled, kind, side, lateral: (p.wx.abs() * 0.8 - half_width + 0.5).max(0.3),
            spacing: (p.z_spacing * TILE_M).max(0.5), offset: p.pos_z * TILE_M, rows: p.tree_row_count.max(1),
            row_spacing: (p.tree_row_spacing * TILE_M).max(1.0), jitter: (p.x_jitter * 2.0).min(3.0), density: 1.0,
            scale: p.scale, scale_var: p.scale_var.min(0.9), tint: p.tint, sink: (p.y_sink * 0.04).min(0.3), float: 0.0, float_var: 0.0,
            shadow: p.casts_shadow, shadow_opacity: (p.shadow_opacity * 0.7).clamp(0.0, 1.0),
            sprite: sprite(&p.sprite_path, &p.sprite_pool_paths, p.sprite_pool_enabled, p.sprite_flip_x),
            seed: p.seed, def: String::new(),
        }
    }).collect();

    let loop_length = o.anim.loop_s.max(1) as f32 * TILE_M;
    Scene {
        version: SCENE_VERSION,
        name: name.to_owned(),
        canvas: Canvas { width: o.canvas.w() as u32, height: o.canvas.h() as u32 },
        camera: Camera { eye_height: eye, horizon: (hy / h).clamp(0.05, 0.95), zoom, lens_curve: o.scene.horizon_curve },
        path,
        verge: Verge {
            enabled: false, tufts: o.scene.grass_enabled, tuft_color: o.scene.grass_color,
            tuft_density: o.scene.grass_density, tuft_height: 0.25 * o.scene.grass_height, ..Verge::default()
        },
        walls,
        ceiling: Ceiling::default(),
        sky,
        light,
        fixtures,
        props, set_pieces: Vec::new(), companions: Vec::new(), weather: Weather::default(), prop_defs: Default::default(),
        particles,
        post: Post {
            exposure: 1.0, contrast: 1.0,
            saturation: if o.post.saturation_enabled { o.post.saturation } else { 1.0 },
            bloom: if o.post.bloom_enabled { o.post.bloom } else { 0.0 },
            vignette: if o.post.vignette_enabled { o.post.vignette } else { 0.0 },
            grain: if o.post.grain_enabled { o.post.grain } else { 0.0 },
            tint: [255, 255, 255],
        },
        style: Style::default(),
        motion: Motion { loop_length, speed: o.anim.play_speed.max(0.01) * loop_length, fps: 24 },
    }
}
