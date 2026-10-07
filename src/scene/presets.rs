//! Built-in scenes. Each is a complete v3 scene in metres; tune them with `pf sheet` and `pf seam`.

use super::*;

fn mat(pattern: Pattern, base: Rgb, mortar: Rgb, tile_size: f32) -> Material {
    Material { pattern, base, mortar, tile_size, ..Material::default() }
}

fn torches(kind: FixtureKind, height: f32, spacing: f32) -> Fixture {
    Fixture { kind, height, spacing, radius: 6.5, ..Fixture::default() }
}

fn props(kind: PropKind, lateral: f32, spacing: f32, scale: f32, seed: u32) -> PropLayer {
    PropLayer { kind, lateral, spacing, scale, tint: kind.default_tint(), seed, ..PropLayer::default() }
}

fn dust(kind: ParticleKind, count: u32) -> Particles {
    Particles { kind, count, color: kind.default_color(), ..Particles::default() }
}

fn night_sky(top: Rgb, horizon: Rgb) -> Sky {
    Sky {
        enabled: true, top, horizon,
        sun: SkyBody::default(),
        moon: Moon { body: SkyBody { enabled: true, ..Moon::default().body }, ..Moon::default() },
        stars: Stars { enabled: true, ..Stars::default() },
        clouds: Clouds { enabled: true, opacity: 0.22, tint: [120, 130, 160], ..Clouds::default() },
        ..Sky::default()
    }
}

fn day_sky(top: Rgb, horizon: Rgb, sun_pos: [f32; 2]) -> Sky {
    Sky {
        enabled: true, top, horizon,
        sun: SkyBody { enabled: true, pos: sun_pos, ..SkyBody::default() },
        moon: Moon::default(),
        stars: Stars::default(),
        clouds: Clouds { enabled: true, ..Clouds::default() },
        ..Sky::default()
    }
}

fn no_sky(void: Rgb) -> Sky { Sky { enabled: false, top: void, horizon: void, ..Sky::default() } }

fn indoor_light(ambient: f32, void: Rgb, fog: f32) -> Lighting {
    Lighting {
        ambient, ambient_color: [150, 160, 190], void_color: void,
        fog: Fog { enabled: true, color: void, distance: fog, match_sky: false }, bands: 0,
    }
}

fn outdoor_light(ambient: f32, fog_color: Rgb, fog: f32) -> Lighting {
    Lighting {
        ambient, ambient_color: [190, 200, 225], void_color: fog_color,
        fog: Fog { enabled: true, color: fog_color, distance: fog, match_sky: true }, bands: 0,
    }
}

fn scene(name: &str) -> Scene {
    Scene {
        version: SCENE_VERSION, name: name.to_owned(), canvas: Canvas::default(), camera: Camera::default(),
        path: PathShape::default(), verge: Verge::default(), walls: Walls::default(), ceiling: Ceiling::default(),
        sky: no_sky([0, 0, 0]), light: indoor_light(0.25, [0, 0, 0], 20.0), fixtures: vec![], props: vec![], set_pieces: vec![], weather: Weather::default(), prop_defs: Default::default(),
        particles: vec![], post: Post::default(), style: Style::default(), motion: Motion::default(),
    }
}

pub fn stone_dungeon() -> Scene {
    let mut s = scene("Stone Dungeon");
    s.path.material = mat(Pattern::Cobblestone, [92, 80, 66], [30, 25, 20], 2.4);
    s.walls.material = mat(Pattern::StoneBlock, [74, 64, 54], [28, 24, 20], 2.8);
    s.ceiling = Ceiling { enabled: true, height: 4.2, material: mat(Pattern::StoneBlock, [52, 46, 40], [22, 19, 16], 2.8) };
    s.fixtures = vec![torches(FixtureKind::Torch, 2.1, 6.0)];
    s.particles = vec![dust(ParticleKind::Dust, 90)];
    s
}

/// A boardwalk through a night swamp: still water on both sides that mirrors the lanterns, reeds and
/// willows standing in it, banks of mist and fireflies.
pub fn bog_boardwalk() -> Scene {
    let mut s = scene("Bog Boardwalk");
    s.camera.horizon = 0.3;
    s.path.half_width = 0.85;
    s.path.edge_noise = 0.0;
    s.path.edge_dark = 0.25;
    s.path.material = Material { gloss: 0.2, ripples: 0.05, ..mat(Pattern::Planks, [92, 74, 54], [26, 20, 14], 1.2) };
    s.path.material.rotate = false;
    s.verge = Verge {
        enabled: true, tufts: false,
        material: Material { gloss: 1.0, ripples: 0.35, noise: 2, damage: 0.0, ..mat(Pattern::Water, [26, 36, 30], [16, 22, 16], 4.0) },
        ..Verge::default()
    };
    s.walls.enabled = false;
    s.sky = night_sky([10, 18, 30], [70, 92, 96]);
    s.sky.moon.body.pos = [0.3, 0.42];
    s.sky.moon.body.radius = 0.07;
    s.light = outdoor_light(0.24, [56, 74, 72], 34.0);
    s.light.ambient_color = [150, 190, 170];
    s.props = vec![
        PropLayer { density: 0.45, rows: 2, row_spacing: 2.2, jitter: 0.8, tint: [84, 100, 56], ..props(PropKind::Reeds, 1.1, 3.0, 0.9, 43) },
        PropLayer { density: 0.7, rows: 2, row_spacing: 6.0, jitter: 1.5, ..props(PropKind::Willow, 4.5, 11.0, 1.0, 47) },
        PropLayer { density: 0.6, jitter: 2.0, ..props(PropKind::DeadTree, 7.5, 13.0, 1.2, 53) },
    ];
    s.fixtures = vec![Fixture { kind: FixtureKind::Lantern, mount: Mount::Ground, lateral: 0.05, height: 1.5, spacing: 8.0, radius: 7.0, intensity: 1.1, side: Side::Both, ..Fixture::default() }];
    s.particles = vec![dust(ParticleKind::Fireflies, 60), dust(ParticleKind::Spores, 30)];
    s.weather.fog_banks = FogBanks { enabled: true, spacing: 12.0, length: 4.0, density: 0.22, offset: 6.0 };
    s.weather.mist = Mist { enabled: true, height: 0.5, density: 0.14, color: [150, 180, 170], patchiness: 0.6, wisps: 0.6, ..Mist::default() };
    s.motion.speed = 3.2;
    s
}

/// Sandstone ruins half buried in a desert: obelisks, broken columns, palms, a ruined arch, and sand
/// blowing low across the road.
pub fn desert_ruins() -> Scene {
    let mut s = scene("Desert Ruins");
    s.camera.horizon = 0.3;
    s.path.half_width = 1.4;
    s.path.edge_noise = 0.5;
    s.path.material = Material { damage: 0.65, ..mat(Pattern::StoneBlock, [196, 168, 120], [150, 122, 82], 2.4) };
    s.verge = Verge { enabled: true, tufts: false, material: mat(Pattern::Sand, [212, 178, 124], [180, 146, 96], 3.0), ..Verge::default() };
    s.walls.enabled = false;
    s.sky = day_sky([88, 146, 214], [238, 220, 182], [0.7, 0.18]);
    s.light = outdoor_light(0.42, [228, 204, 162], 130.0);
    s.light.ambient_color = [210, 196, 180];
    s.post.contrast = 1.12;
    s.props = vec![
        PropLayer { density: 0.6, jitter: 0.6, ..props(PropKind::Pillar, 1.0, 9.0, 1.0, 59) },
        PropLayer { density: 0.7, jitter: 1.0, ..props(PropKind::Obelisk, 3.2, 18.0, 1.0, 61) },
        PropLayer { density: 0.7, rows: 2, row_spacing: 6.0, jitter: 2.0, ..props(PropKind::Palm, 7.0, 14.0, 1.0, 67) },
        PropLayer { density: 0.8, jitter: 1.0, tint: [176, 146, 104], ..props(PropKind::Rock, 0.5, 5.0, 1.0, 71) },
    ];
    s.props[0].tint = [196, 170, 128];
    s.set_pieces = vec![SetPiece { kind: SetPieceKind::RuinedArch, spacing: 36.0, offset: 20.0, height: 4.2, tint: [200, 172, 128], ..SetPiece::default() }];
    s.particles = vec![Particles { count: 320, ..dust(ParticleKind::Sand, 320) }];
    s.motion = Motion { loop_length: 36.0, speed: 4.5, fps: 24 };
    s
}

/// A cave of ice: a polished frozen floor that mirrors the glowing crystals, icicles overhead, ice
/// spikes along the walls, and a few drifting flakes.
pub fn ice_cave() -> Scene {
    let mut s = scene("Ice Cave");
    s.path.half_width = 1.3;
    s.path.edge_noise = 0.3;
    s.path.material = Material { gloss: 0.55, ripples: 0.04, damage: 0.0, ..mat(Pattern::Ice, [70, 104, 128], [150, 190, 214], 3.0) };
    s.walls = Walls { enabled: true, gap: 0.9, height: 7.0, base_shadow: 0.35, material: mat(Pattern::RockFace, [64, 98, 124], [30, 52, 70], 3.0) };
    s.ceiling = Ceiling { enabled: true, height: 4.6, material: mat(Pattern::RockFace, [46, 72, 94], [22, 38, 52], 3.0) };
    s.light = indoor_light(0.26, [4, 10, 18], 26.0);
    s.light.ambient_color = [140, 190, 235];
    s.props = vec![
        PropLayer { density: 0.9, jitter: 0.5, scale_var: 0.4, ..props(PropKind::Icicle, 0.5, 2.2, 1.0, 73) },
        PropLayer { density: 0.6, jitter: 0.3, ..props(PropKind::IceSpike, 0.35, 5.0, 1.0, 79) },
        PropLayer { density: 0.5, tint: [110, 150, 180], ..props(PropKind::Stalagmite, 0.5, 9.0, 0.9, 83) },
    ];
    s.fixtures = vec![Fixture { kind: FixtureKind::Crystal, mount: Mount::Ground, lateral: 0.35, height: 0.6, spacing: 8.0, radius: 7.5, intensity: 1.3, ..Fixture::default() }];
    s.particles = vec![dust(ParticleKind::Snow, 45)];
    s.post.bloom = 0.45;
    s.motion.speed = 3.4;
    s
}

/// A climb up a castle tower: flights of cut-stone steps, a torch on every landing.
pub fn tower_stair() -> Scene {
    let mut s = scene("Tower Stair");
    s.path.half_width = 0.95;
    s.path.edge_noise = 0.0;
    s.path.material = mat(Pattern::StoneBlock, [96, 86, 74], [34, 29, 24], 1.2);
    s.path.stairs = Stairs { enabled: true, spacing: 8.0, steps: 10, rise: 0.18, run: 0.3, offset: 2.0, descending: false };
    s.walls = Walls { enabled: true, gap: 0.0, height: 6.0, base_shadow: 0.45, material: mat(Pattern::StoneBlock, [70, 62, 54], [26, 22, 18], 1.8) };
    s.ceiling = Ceiling { enabled: true, height: 3.6, material: mat(Pattern::StoneBlock, [50, 44, 38], [20, 17, 14], 1.8) };
    s.light = indoor_light(0.24, [4, 2, 1], 26.0);
    s.fixtures = vec![Fixture { offset: 5.4, radius: 9.0, intensity: 1.5, ..torches(FixtureKind::Torch, 1.9, 8.0) }];
    s.set_pieces = vec![SetPiece { kind: SetPieceKind::Archway, spacing: 24.0, offset: 7.0, height: 2.9, tint: [86, 78, 70], ..SetPiece::default() }];
    s.particles = vec![dust(ParticleKind::Dust, 70)];
    s.motion.speed = 3.0;
    s
}

pub fn stone_crypt() -> Scene {
    let mut s = scene("Stone Crypt");
    s.path.half_width = 0.9;
    s.path.material = mat(Pattern::Brick, [78, 72, 64], [26, 23, 20], 2.0);
    s.walls.material = mat(Pattern::Brick, [64, 58, 52], [24, 22, 19], 2.0);
    s.walls.height = 3.4;
    s.light = indoor_light(0.18, [3, 0, 6], 16.0);
    s.fixtures = vec![Fixture { kind: FixtureKind::Candle, height: 1.2, mount: Mount::Ground, lateral: -0.15, spacing: 4.0, radius: 3.5, ..Fixture::default() }];
    s.props = vec![PropLayer { kind: PropKind::Gravestone, lateral: 0.0, spacing: 4.0, offset: 2.0, scale: 0.9, jitter: 0.05, tint: PropKind::Gravestone.default_tint(), ..PropLayer::default() }];
    s.walls.gap = 0.9;
    s.particles = vec![dust(ParticleKind::Dust, 60)];
    s.motion.speed = 2.8;
    s.set_pieces = vec![SetPiece { kind: SetPieceKind::Archway, spacing: 12.0, offset: 6.0, height: 2.8, tint: [84, 76, 68], ..SetPiece::default() }];
    // Catacombs: side passages between the arches.
    s.path.fork = Fork { enabled: true, spacing: 24.0, offset: 1.5, side: ForkSide::Left, half_width: 0.8, height: 2.3, ..Fork::default() };
    s
}

pub fn mossy_sewer() -> Scene {
    let mut s = scene("Mossy Sewer");
    s.path.material = mat(Pattern::Cobblestone, [62, 76, 54], [22, 30, 18], 2.0);
    s.walls.material = mat(Pattern::StoneBlock, [52, 70, 46], [20, 30, 16], 2.4);
    s.ceiling = Ceiling { enabled: true, height: 3.6, material: mat(Pattern::Brick, [40, 52, 36], [16, 22, 12], 2.0) };
    s.light = indoor_light(0.22, [0, 4, 0], 18.0);
    s.light.fog.color = [6, 14, 6];
    s.fixtures = vec![Fixture { kind: FixtureKind::Firefly, mount: Mount::Floating, lateral: -0.6, height: 1.4, spacing: 3.0, radius: 3.0, intensity: 0.7, jitter: 0.6, ..Fixture::default() }];
    s.particles = vec![dust(ParticleKind::Spores, 80)];
    s.weather.drips = Drips { enabled: true, rate: 1.5, color: [150, 190, 150], ..Drips::default() };
    s.motion.speed = 3.2;
    s
}

pub fn forest_path() -> Scene {
    let mut s = scene("Forest Path");
    s.camera.horizon = 0.3;
    s.path.half_width = 1.0;
    s.path.edge_noise = 0.3;
    s.path.material = mat(Pattern::Dirt, [104, 84, 58], [74, 58, 38], 2.0);
    s.verge = Verge { enabled: true, material: mat(Pattern::Grass, [46, 84, 32], [30, 56, 20], 2.0), tufts: true, ..Verge::default() };
    s.walls.enabled = false;
    s.sky = day_sky([74, 128, 190], [196, 214, 200], [0.7, 0.4]);
    s.light = outdoor_light(0.55, [150, 180, 170], 48.0);
    s.props = vec![
        PropLayer { rows: 3, row_spacing: 3.5, jitter: 1.2, density: 0.85, ..props(PropKind::Tree, 1.8, 5.0, 1.0, 7) },
        PropLayer { jitter: 0.4, density: 0.7, ..props(PropKind::Bush, 0.5, 3.0, 1.0, 11) },
    ];
    s.particles = vec![dust(ParticleKind::Leaves, 40)];
    s.weather.wind = Wind { enabled: true, speed: 2.5, gusts: 0.5, sway: 0.5, ..Wind::default() };
    s.weather.light_shafts = LightShafts { enabled: true, sun: 0.7, lamps: 0.0 };
    s.motion.speed = 4.0;
    s
}

pub fn desert_canyon() -> Scene {
    let mut s = scene("Desert Canyon");
    s.camera.horizon = 0.32;
    s.path.half_width = 1.6;
    s.path.edge_noise = 0.4;
    s.path.material = mat(Pattern::Sand, [196, 160, 108], [160, 124, 76], 3.0);
    s.verge = Verge { enabled: true, material: mat(Pattern::Sand, [176, 138, 90], [140, 106, 64], 3.0), ..Verge::default() };
    s.walls = Walls { enabled: true, gap: 4.0, height: 14.0, base_shadow: 0.3, material: mat(Pattern::RockFace, [170, 116, 70], [120, 80, 46], 6.0) };
    s.sky = day_sky([96, 150, 210], [236, 206, 150], [0.3, 0.3]);
    s.light = outdoor_light(0.6, [220, 190, 150], 70.0);
    s.props = vec![
        PropLayer { density: 0.6, ..props(PropKind::Cactus, 1.6, 8.0, 1.0, 3) },
        PropLayer { density: 0.8, jitter: 1.0, ..props(PropKind::Rock, 0.8, 4.0, 1.0, 5) },
    ];
    s.motion = Motion { loop_length: 48.0, speed: 6.0, fps: 24 };
    s.weather.heat_shimmer = HeatShimmer { enabled: true, strength: 0.45, ..HeatShimmer::default() };
    s
}

pub fn night_road() -> Scene {
    let mut s = scene("Night Road");
    s.camera.horizon = 0.3;
    s.path.half_width = 1.8;
    s.path.material = mat(Pattern::Brick, [70, 70, 84], [36, 36, 46], 1.6);
    s.verge = Verge { enabled: true, material: mat(Pattern::Grass, [30, 44, 34], [20, 30, 24], 2.0), tufts: true, tuft_color: [34, 60, 40], ..Verge::default() };
    s.walls = Walls { enabled: true, gap: 3.0, height: 1.2, base_shadow: 0.4, material: mat(Pattern::StoneBlock, [64, 64, 76], [30, 30, 40], 1.6) };
    s.sky = night_sky([6, 8, 24], [26, 22, 52]);
    s.light = outdoor_light(0.2, [14, 14, 30], 40.0);
    s.fixtures = vec![Fixture { kind: FixtureKind::Lantern, mount: Mount::Ground, lateral: 0.6, height: 2.6, spacing: 8.0, radius: 9.0, intensity: 1.3, ..Fixture::default() }];
    s.particles = vec![dust(ParticleKind::Fireflies, 30)];
    s.motion.speed = 5.0;
    s
}

pub fn magic_cavern() -> Scene {
    let mut s = scene("Magic Cavern");
    s.path.material = mat(Pattern::Cobblestone, [60, 44, 84], [24, 16, 40], 2.2);
    s.walls = Walls { enabled: true, gap: 1.2, height: 0.0, base_shadow: 0.5, material: mat(Pattern::RockFace, [52, 38, 74], [24, 16, 38], 3.0) };
    s.light = indoor_light(0.2, [4, 0, 12], 20.0);
    s.fixtures = vec![Fixture { kind: FixtureKind::Crystal, mount: Mount::Ground, lateral: 0.4, height: 0.6, spacing: 6.0, radius: 5.0, intensity: 1.2, ..Fixture::default() }];
    s.props = vec![
        PropLayer { density: 0.7, jitter: 0.3, ..props(PropKind::Mushroom, 0.2, 3.0, 1.2, 9) },
        PropLayer { density: 0.8, ..props(PropKind::Stalagmite, 0.7, 4.0, 1.0, 13) },
    ];
    s.particles = vec![dust(ParticleKind::Spores, 120)];
    s.post.bloom = 0.6;
    s.motion.speed = 3.5;
    s.set_pieces = vec![SetPiece { kind: SetPieceKind::Portal, spacing: 24.0, offset: 12.0, width: 2.4, height: 3.2, accent: [150, 90, 255], shadow: 0.0, ..SetPiece::default() }];
    s
}

pub fn ice_dungeon() -> Scene {
    let mut s = scene("Ice Dungeon");
    s.path.material = mat(Pattern::StoneBlock, [96, 130, 150], [40, 60, 76], 2.4);
    s.walls.material = mat(Pattern::Brick, [84, 120, 140], [36, 56, 70], 2.0);
    s.ceiling = Ceiling { enabled: true, height: 4.0, material: mat(Pattern::StoneBlock, [60, 86, 104], [26, 40, 52], 2.4) };
    s.light = indoor_light(0.3, [4, 10, 22], 22.0);
    s.light.ambient_color = [150, 190, 230];
    s.fixtures = vec![torches(FixtureKind::IceWisp, 2.0, 6.0)];
    s.particles = vec![dust(ParticleKind::Snow, 70)];
    s.motion.speed = 3.4;
    s
}

pub fn ruins_path() -> Scene {
    let mut s = scene("Ruins Path");
    s.camera.horizon = 0.3;
    s.path.material = mat(Pattern::Cobblestone, [150, 130, 100], [90, 76, 58], 2.4);
    s.path.material.damage = 0.5;
    s.verge = Verge { enabled: true, material: mat(Pattern::Grass, [74, 96, 52], [52, 70, 36], 2.0), tufts: true, ..Verge::default() };
    s.walls = Walls { enabled: true, gap: 1.8, height: 2.2, base_shadow: 0.4, material: mat(Pattern::StoneBlock, [140, 124, 98], [86, 74, 58], 2.4) };
    s.walls.material.damage = 0.7;
    s.sky = day_sky([84, 124, 176], [206, 206, 186], [0.78, 0.45]);
    s.light = outdoor_light(0.55, [190, 196, 190], 55.0);
    s.props = vec![
        PropLayer { density: 0.5, ..props(PropKind::Pillar, 0.6, 8.0, 1.0, 17) },
        PropLayer { density: 0.7, rows: 2, ..props(PropKind::DeadTree, 4.0, 7.0, 1.0, 19) },
    ];
    s.motion.speed = 4.0;
    s.set_pieces = vec![SetPiece { kind: SetPieceKind::RuinedArch, spacing: 24.0, offset: 8.0, height: 3.6, tint: [176, 160, 132], ..SetPiece::default() }];
    s
}

pub fn dark_street() -> Scene {
    let mut s = scene("Dark Street");
    s.path.half_width = 1.4;
    s.path.material = mat(Pattern::Brick, [64, 64, 70], [28, 28, 32], 1.4);
    s.walls = Walls { enabled: true, gap: 0.8, height: 8.0, base_shadow: 0.5, material: mat(Pattern::Brick, [70, 60, 56], [30, 26, 24], 2.0) };
    s.sky = night_sky([4, 6, 18], [20, 16, 34]);
    s.light = indoor_light(0.18, [6, 6, 12], 30.0);
    s.fixtures = vec![Fixture { kind: FixtureKind::Lantern, height: 3.0, spacing: 8.0, radius: 7.5, intensity: 1.3, ..Fixture::default() }];
    s.particles = vec![];
    s.weather.precipitation = Precipitation { enabled: true, kind: PrecipKind::Rain, intensity: 0.6, wetness: 0.9, puddles: 0.35, ..Precipitation::default() };
    s.weather.lightning = Lightning { enabled: true, strikes: 1, intensity: 1.3, ..Lightning::default() };
    s.motion.speed = 3.6;
    s
}

pub fn mountain_pass() -> Scene {
    let mut s = scene("Mountain Pass");
    s.camera.horizon = 0.34;
    s.path.half_width = 1.3;
    s.path.bend = 0.35;
    s.path.edge_noise = 0.35;
    s.path.material = mat(Pattern::Dirt, [130, 116, 104], [96, 84, 74], 2.4);
    s.verge = Verge { enabled: true, material: mat(Pattern::RockFace, [110, 104, 100], [76, 70, 66], 3.0), ..Verge::default() };
    s.walls.enabled = false;
    s.sky = day_sky([110, 140, 190], [214, 210, 200], [0.62, 0.3]);
    s.light = outdoor_light(0.6, [200, 205, 214], 60.0);
    s.props = vec![
        PropLayer { density: 0.6, jitter: 1.5, rows: 2, row_spacing: 5.0, ..props(PropKind::Boulder, 2.0, 7.0, 1.0, 23) },
        PropLayer { density: 0.6, ..props(PropKind::Pine, 6.0, 6.0, 1.0, 29) },
    ];
    s.particles = vec![];
    s.weather.precipitation = Precipitation { enabled: true, kind: PrecipKind::Snow, intensity: 0.35, cover: 0.35, track: 0.5, haze: 0.4, ..Precipitation::default() };
    s.weather.wind = Wind { enabled: true, speed: 2.5, gusts: 0.5, sway: 0.5, ..Wind::default() };
    s.path.bridge = Bridge { enabled: true, spacing: 42.0, length: 11.0, offset: 16.0, depth: 40.0, bottom_color: [120, 130, 150], ..Bridge::default() };
    s.motion = Motion { loop_length: 42.0, speed: 5.0, fps: 24 };
    s
}

pub fn volcanic_rift() -> Scene {
    let mut s = scene("Volcanic Rift");
    s.path.material = mat(Pattern::StoneBlock, [60, 36, 30], [150, 40, 12], 2.4);
    s.walls = Walls { enabled: true, gap: 1.0, height: 0.0, base_shadow: 0.6, material: mat(Pattern::RockFace, [56, 30, 22], [32, 14, 8], 3.0) };
    s.light = indoor_light(0.2, [30, 6, 2], 24.0);
    s.light.ambient_color = [255, 150, 110];
    s.fixtures = vec![Fixture { kind: FixtureKind::Brazier, mount: Mount::Ground, lateral: 0.3, height: 1.1, spacing: 8.0, radius: 7.0, intensity: 1.4, ..Fixture::default() }];
    s.props = vec![PropLayer { density: 0.7, jitter: 0.4, ..props(PropKind::Stalagmite, 0.4, 4.0, 1.2, 31) }];
    s.particles = vec![dust(ParticleKind::Embers, 140), dust(ParticleKind::Ash, 60)];
    s.post.bloom = 0.55;
    s.motion.speed = 4.0;
    s
}

/// Path of Kings-style biomes: portrait first-person runs through dark fantasy places.
pub fn haunted_forest() -> Scene {
    let mut s = forest_path();
    s.name = "Haunted Forest".into();
    s.path.material = mat(Pattern::Dirt, [70, 60, 56], [44, 38, 36], 2.0);
    s.verge.material = mat(Pattern::Grass, [34, 48, 40], [20, 30, 26], 2.0);
    s.verge.tuft_color = [40, 60, 46];
    s.sky = night_sky([10, 14, 26], [50, 60, 70]);
    s.sky.moon.body.pos = [0.5, 0.35];
    s.sky.moon.body.radius = 0.1;
    s.light = outdoor_light(0.22, [36, 46, 52], 24.0);
    s.props = vec![
        PropLayer { rows: 3, row_spacing: 3.0, jitter: 1.0, density: 0.9, tint: [60, 52, 50], ..props(PropKind::DeadTree, 1.4, 4.0, 1.1, 37) },
        PropLayer { density: 0.4, ..props(PropKind::Gravestone, 0.3, 6.0, 1.0, 41) },
    ];
    s.fixtures = vec![Fixture { kind: FixtureKind::IceWisp, mount: Mount::Floating, lateral: 0.5, height: 1.5, spacing: 8.0, radius: 5.0, jitter: 0.8, ..Fixture::default() }];
    s.particles = vec![dust(ParticleKind::Spores, 50)];
    s.weather.fog_banks = FogBanks { enabled: true, spacing: 12.0, length: 5.0, density: 0.3, offset: 3.0 };
    s.weather.mist = Mist { enabled: true, height: 0.6, density: 0.12, color: [150, 170, 185], patchiness: 0.7, wisps: 0.5, ..Mist::default() };
    s.path.fork = Fork { enabled: true, spacing: 24.0, offset: 9.0, side: ForkSide::Right, angle: 34.0, half_width: 0.75, ..Fork::default() };
    s.post.saturation = 0.8;
    s
}

pub fn ruined_castle() -> Scene {
    let mut s = scene("Ruined Castle");
    s.camera.horizon = 0.26;
    s.path.half_width = 1.5;
    s.path.material = mat(Pattern::StoneBlock, [120, 112, 104], [60, 56, 52], 2.0);
    s.path.material.damage = 0.45;
    s.walls = Walls { enabled: true, gap: 0.5, height: 7.0, base_shadow: 0.5, material: mat(Pattern::StoneBlock, [110, 102, 96], [56, 52, 48], 2.4) };
    s.walls.material.damage = 0.6;
    s.sky = day_sky([70, 90, 130], [180, 170, 160], [0.25, 0.45]);
    s.sky.clouds.opacity = 0.6;
    s.sky.clouds.tint = [170, 170, 180];
    s.light = outdoor_light(0.45, [150, 150, 160], 45.0);
    s.fixtures = vec![torches(FixtureKind::Torch, 2.6, 8.0)];
    s.props = vec![PropLayer { density: 0.5, ..props(PropKind::Pillar, 0.1, 8.0, 0.9, 43) }];
    s.particles = vec![dust(ParticleKind::Ash, 40)];
    s.props[0].offset = 4.0; // between the torches, not under them
    s.set_pieces = vec![SetPiece { kind: SetPieceKind::Gate, spacing: 24.0, offset: 12.0, height: 3.6, tint: [104, 98, 92], accent: [120, 24, 30], ..SetPiece::default() }];
    s
}

pub fn fiery_dungeon() -> Scene {
    let mut s = stone_dungeon();
    s.name = "Fiery Dungeon".into();
    s.path.material = mat(Pattern::StoneBlock, [76, 50, 40], [170, 60, 16], 2.4);
    s.walls.material = mat(Pattern::Brick, [80, 48, 36], [34, 16, 10], 2.0);
    s.ceiling.material = mat(Pattern::Brick, [56, 34, 26], [26, 12, 8], 2.0);
    s.light = indoor_light(0.25, [24, 4, 0], 22.0);
    s.light.ambient_color = [255, 140, 90];
    s.fixtures = vec![Fixture { intensity: 1.3, ..torches(FixtureKind::Torch, 2.2, 6.0) }];
    s.particles = vec![dust(ParticleKind::Embers, 160)];
    s.post.bloom = 0.6;
    s.set_pieces = vec![SetPiece { kind: SetPieceKind::Archway, spacing: 12.0, offset: 9.0, height: 3.0, tint: [70, 54, 46], ..SetPiece::default() }];
    s
}

pub const ALL: &[(&str, fn() -> Scene)] = &[
    ("Stone Dungeon", stone_dungeon),
    ("Stone Crypt", stone_crypt),
    ("Mossy Sewer", mossy_sewer),
    ("Forest Path", forest_path),
    ("Desert Canyon", desert_canyon),
    ("Night Road", night_road),
    ("Magic Cavern", magic_cavern),
    ("Ice Dungeon", ice_dungeon),
    ("Ruins Path", ruins_path),
    ("Dark Street", dark_street),
    ("Mountain Pass", mountain_pass),
    ("Volcanic Rift", volcanic_rift),
    ("Haunted Forest", haunted_forest),
    ("Ruined Castle", ruined_castle),
    ("Fiery Dungeon", fiery_dungeon),
("Tower Stair", tower_stair),
("Bog Boardwalk", bog_boardwalk),
("Desert Ruins", desert_ruins),
("Ice Cave", ice_cave),
];
