//! Scene format v3: one metric world model shared by the renderer, the editor, exporters and MCP.
//!
//! Units are metres and seconds. The camera stands on the path and looks down it; the world
//! scrolls toward the camera by `distance` metres. Everything that repeats (texture tiles,
//! fixture and prop spacing, particle fields, sky motion) is snapped so a whole number of
//! repeats fits in `motion.loop_length`, which makes every loop seamless by construction.

pub mod presets;
pub mod migrate;
pub mod styles;
pub mod remix;
pub mod transition;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const SCENE_VERSION: u32 = 3;

pub type Rgb = [u8; 3];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Scene {
    pub version: u32,
    pub name: String,
    pub canvas: Canvas,
    pub camera: Camera,
    pub path: PathShape,
    pub verge: Verge,
    pub walls: Walls,
    pub ceiling: Ceiling,
    pub sky: Sky,
    pub light: Lighting,
    pub fixtures: Vec<Fixture>,
    pub props: Vec<PropLayer>,
    /// Structures that span the path at intervals: archways, gates, banners.
    pub set_pieces: Vec<SetPiece>,
    /// Things travelling along with the camera: a ship flying beside the path, a drone, a bird.
    pub companions: Vec<Companion>,
    pub particles: Vec<Particles>,
    /// Lightning and fog banks.
    pub weather: Weather,
    /// Props described by data, by name: a prop layer uses one with `def: "<name>"`. Kits (folders
    /// with a kit.json) hold more, used as `def: "<folder>#<name>"`.
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub prop_defs: std::collections::BTreeMap<String, PropDef>,
    pub post: Post,
    pub style: Style,
    pub motion: Motion,
}

impl Default for Scene {
    fn default() -> Self { presets::stone_dungeon() }
}

/// Output size in pixels. Portrait by default, like a phone held upright.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Canvas {
    pub width: u32,
    pub height: u32,
}
impl Default for Canvas {
    fn default() -> Self { Self { width: 480, height: 854 } }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Camera {
    /// Eye height above the path, metres.
    pub eye_height: f32,
    /// Horizon position as a fraction of the canvas height from the top (0.1..0.9).
    pub horizon: f32,
    /// Zoom: 1.0 puts the ground at `eye_height` metres ahead on the bottom row. Higher = narrower view.
    pub zoom: f32,
    /// Screen-space lens bend of the horizon (-1..1); 0 is a flat horizon.
    pub lens_curve: f32,
}
impl Default for Camera {
    fn default() -> Self { Self { eye_height: 1.6, horizon: 0.22, zoom: 1.0, lens_curve: 0.0 } }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct PathShape {
    /// Half the path width near the camera, metres.
    pub half_width: f32,
    /// 0 = straight edges in true perspective; up to 0.9 widens the path into the distance (stylised).
    pub flare: f32,
    /// Lateral curve of the road ahead (-1 left .. 1 right).
    pub bend: f32,
    /// Vertical curve of the road ahead (-1 dips away .. 1 rises).
    pub hill: f32,
    /// How ragged the path edge is, metres.
    pub edge_noise: f32,
    /// Darkening toward the path edge (0..1).
    pub edge_dark: f32,
    pub material: Material,
    /// Flights of steps the walk climbs (or descends) for ever.
    pub stairs: Stairs,
    /// Stretches where the ground beside the path falls away and the path crosses on a bridge.
    pub bridge: Bridge,
    /// Side paths that branch off (open ground), or side passages (between walls).
    pub fork: Fork,
    /// Glowing lines along both edges of the path (and a bridge's deck): a lit walkway.
    pub edge_lights: EdgeLights,
    /// Draw the path at all. Off, there is no ground, verge or bridge: the camera flies the route
    /// through open air or space (a ship's flight path), and with `sky.space.below` or a tunnel
    /// the sky fills everything round it.
    pub surface: bool,
    /// The path as a waterway, for boats and swimmers (give `material` the Water pattern).
    pub waterway: Waterway,
}

/// The path as a waterway, for games where the player steers a boat or swims: a current, foam
/// lapping at the banks or walls, the bank wet where the water reaches it, and things floating on
/// the surface. Give `path.material` the Water pattern (gloss 1, some ripples); with no verge the
/// water runs to the walls, as a canal's does.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Waterway {
    pub enabled: bool,
    /// The current, towards the camera: whole tiles of the water's pattern carried past per loop
    /// (0 still water, negative flows away). Floating things drift with it.
    pub flow: i32,
    /// Foam where the water laps the banks or walls: 0 none .. 1.
    pub foam: f32,
    pub foam_color: Rgb,
    /// How far out from the bank the foam reaches, metres.
    pub foam_width: f32,
    /// Times the water laps up the bank in one loop (whole).
    pub lap: u32,
    /// How far up the bank (or the walls) the water wets it, darker and glistening, metres.
    pub wet: f32,
    /// What floats on the water, drifting with the current.
    pub floating: Floating,
    /// How much of the water they cover, 0 .. 1.
    pub float_density: f32,
    pub float_color: Rgb,
    /// How big each one is, metres across.
    pub float_size: f32,
    pub seed: u32,
}
impl Default for Waterway {
    fn default() -> Self {
        Waterway {
            enabled: false, flow: 2, foam: 0.5, foam_color: [214, 226, 224], foam_width: 0.35, lap: 3, wet: 0.25,
            floating: Floating::None, float_density: 0.3, float_color: [70, 110, 52], float_size: 0.5, seed: 0,
        }
    }
}

/// Things floating on a waterway.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Floating {
    #[default]
    None,
    /// Clumps of foam.
    Foam,
    /// Fallen leaves.
    Leaves,
    /// Lily pads, a few in flower.
    LilyPads,
}

/// Forks. On open ground a branch path splits off at `angle` and runs away into the distance; between
/// walls the fork is a side passage: an opening in the wall with a dark passage behind it. The walk
/// itself stays on the main path. `style` chooses how a branch leaves on open ground: through a
/// gap in the path's edge, or by the road itself dividing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Fork {
    pub enabled: bool,
    /// Distance from one fork to the next, metres (snapped to divide the loop).
    pub spacing: f32,
    /// Where along the loop the first fork is, metres.
    pub offset: f32,
    pub side: ForkSide,
    /// How a branch leaves the road on open ground (between walls a fork is always a side passage).
    pub style: ForkStyle,
    /// Angle of a branch path from the main path, degrees (open ground).
    pub angle: f32,
    /// Half the width of a branch path, or of a side passage's opening, metres.
    pub half_width: f32,
    /// How deep a side passage goes before it is lost in the dark, metres (between walls).
    pub depth: f32,
    /// Height of a side passage, metres (between walls).
    pub height: f32,
}
impl Default for Fork {
    fn default() -> Self { Fork { enabled: false, spacing: 24.0, offset: 14.0, side: ForkSide::Alternate, style: ForkStyle::Side, angle: 38.0, half_width: 1.1, depth: 7.0, height: 2.6 } }
}

/// Which side a branch leaves on: one side, alternating from fork to fork, or both at once (the
/// road divides three ways with `ForkStyle::Split`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum ForkSide { Left, Right, Alternate, Both }

/// How a branch leaves the road on open ground.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum ForkStyle {
    /// A side path through a gap in the road's edge.
    #[default]
    Side,
    /// The road itself divides: it widens, stays one paved surface for a stretch, then parts round
    /// a grass point, each path running on alone.
    Split,
}

/// Bridges: every `spacing` metres the ground beside the path drops away for `length` metres, and
/// the path crosses on a deck. Walls carry on down into the gap.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Bridge {
    pub enabled: bool,
    /// Distance from one bridge to the next, metres (snapped to divide the loop).
    pub spacing: f32,
    /// Length of each bridge, metres.
    pub length: f32,
    /// Where along the loop the first bridge starts, metres.
    pub offset: f32,
    /// How far down the bottom of the gap is, metres.
    pub depth: f32,
    /// What is down there.
    pub bottom: BridgeBottom,
    /// With a Water bottom: how far below the deck the water lies, metres (never deeper than `depth`).
    pub water_level: f32,
    /// Colour of water, or of ground far below (multiplied with the verge or path material).
    pub bottom_color: Rgb,
    /// The deck: planks, stone...
    pub deck: Material,
    pub railing: Railing,
    /// Railing height, metres.
    pub rail_height: f32,
    /// A taller pillar or newel post where each railing starts and ends.
    pub end_pillars: bool,
    /// Colour of iron bars, or of rope.
    pub rail_color: Rgb,
}
impl Default for Bridge {
    fn default() -> Self {
        Bridge {
            enabled: false, spacing: 24.0, length: 8.0, offset: 6.0, depth: 10.0, bottom: BridgeBottom::Ground, bottom_color: [150, 150, 150],
            deck: Material { pattern: Pattern::Planks, base: [104, 78, 54], mortar: [30, 22, 16], noise: 8, damage: 0.15, seed: 0, tile_size: 1.2, rotate: false, brightness: 1.0, ..Material::default() },
            railing: Railing::Posts, rail_height: 1.0, end_pillars: true, rail_color: Railing::Iron.default_color().unwrap(), water_level: 10.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum BridgeBottom {
    /// The verge (or path) material, far below.
    Ground,
    /// Still water.
    Water,
    /// Nothing: a bottomless drop into the void colour.
    Void,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Railing {
    None,
    /// Wooden posts with two rails, in the deck material.
    Posts,
    /// A low solid wall with a coping, in the wall material (or the path material without walls).
    Parapet,
    /// Stone balusters on a plinth under a handrail, in the wall (or path) material.
    Balustrade,
    /// Iron bars between rails, on a stone kerb, in `rail_color`.
    Iron,
    /// Ropes sagging between wooden posts, in `rail_color`.
    Rope,
}

impl Railing {
    /// The colour `rail_color` takes for this railing when it is picked, if it uses one.
    pub fn default_color(self) -> Option<Rgb> {
        match self {
            Railing::Iron => Some([34, 35, 38]),
            Railing::Rope => Some([150, 124, 86]),
            _ => None,
        }
    }
}

/// Flights of steps along the path, each followed by a landing. The camera climbs them like a
/// walker (smoothly over each flight), and the loop still closes: the view one flight on is the same.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Stairs {
    pub enabled: bool,
    /// Distance from one flight to the next, metres (snapped to divide the loop).
    pub spacing: f32,
    /// Steps per flight.
    pub steps: u32,
    /// Height of each step, metres (real stairs: 0.15-0.2).
    pub rise: f32,
    /// Depth of each step, metres (real stairs: 0.25-0.35).
    pub run: f32,
    /// Where along the loop the first flight starts, metres.
    pub offset: f32,
    /// Walk down the steps instead of up.
    pub descending: bool,
}
impl Default for Stairs {
    fn default() -> Self { Stairs { enabled: false, spacing: 12.0, steps: 8, rise: 0.17, run: 0.32, offset: 3.0, descending: false } }
}
/// Lines of light set into the path along both its edges. They glow, and light the floor round
/// them; far away they widen to a pixel and dim to match, so they do not flicker.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct EdgeLights {
    pub enabled: bool,
    pub color: Rgb,
    /// Brightness: 1 bright.
    pub strength: f32,
    /// Width of each line, metres.
    pub width: f32,
    /// How far in from the path's edge, metres.
    pub inset: f32,
    /// Dashes: one dash and one gap every this many metres (snapped to divide the loop); 0 an
    /// unbroken line.
    pub dash: f32,
    /// How many dashes the lights run ahead in one loop, on top of the walk (negative: toward the
    /// camera). Whole numbers keep the loop seamless.
    pub flow: i32,
}
impl Default for EdgeLights {
    fn default() -> Self { EdgeLights { enabled: false, color: [80, 210, 255], strength: 1.0, width: 0.05, inset: 0.08, dash: 0.0, flow: 0 } }
}

impl Default for PathShape {
    fn default() -> Self {
        Self {
            half_width: 1.1, flare: 0.0, bend: 0.0, hill: 0.0, edge_noise: 0.12, edge_dark: 0.35,
            material: Material::default(), stairs: Stairs::default(), bridge: Bridge::default(), fork: Fork::default(),
            edge_lights: EdgeLights::default(), surface: true, waterway: Waterway::default(),
        }
    }
}

/// Ground beside the path. Disabled = void beyond the path edge (walls usually cover it).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Verge {
    pub enabled: bool,
    pub material: Material,
    /// Grass tufts along the path edge.
    pub tufts: bool,
    pub tuft_color: Rgb,
    pub tuft_density: f32,
    pub tuft_height: f32,
}
impl Default for Verge {
    fn default() -> Self {
        Self {
            enabled: false,
            material: Material { pattern: Pattern::Grass, base: [42, 70, 30], mortar: [26, 46, 18], ..Material::default() },
            tufts: false, tuft_color: [44, 100, 30], tuft_density: 1.0, tuft_height: 0.25,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Walls {
    pub enabled: bool,
    /// Gap between the path edge and the wall, metres.
    pub gap: f32,
    /// Wall height, metres. 0 = taller than anything the camera can see.
    pub height: f32,
    /// Darkening where the wall meets the ground (0..1).
    pub base_shadow: f32,
    pub material: Material,
}
impl Default for Walls {
    fn default() -> Self {
        Self {
            enabled: true, gap: 0.25, height: 4.5, base_shadow: 0.55,
            material: Material { pattern: Pattern::StoneBlock, ..Material::default() },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Ceiling {
    pub enabled: bool,
    pub height: f32,
    pub material: Material,
}
impl Default for Ceiling {
    fn default() -> Self {
        Self { enabled: false, height: 3.6, material: Material { pattern: Pattern::StoneBlock, ..Material::default() } }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub enum Pattern { Cobblestone, Brick, StoneBlock, Sand, Dirt, Grass, Bark, RockFace, Hedge, Planks, Water, Ice, Plain, Panels, Grid, Hex, Circuit }

impl Pattern {
    pub const ALL: [Pattern; 17] = [
        Pattern::Cobblestone, Pattern::Brick, Pattern::StoneBlock, Pattern::Sand, Pattern::Dirt,
        Pattern::Grass, Pattern::Bark, Pattern::RockFace, Pattern::Hedge, Pattern::Planks, Pattern::Water, Pattern::Ice, Pattern::Plain,
        Pattern::Panels, Pattern::Grid, Pattern::Hex, Pattern::Circuit,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Pattern::Cobblestone => "Cobblestone", Pattern::Brick => "Brick", Pattern::StoneBlock => "Stone Block",
            Pattern::Sand => "Sand", Pattern::Dirt => "Dirt", Pattern::Grass => "Grass", Pattern::Bark => "Bark",
            Pattern::RockFace => "Rock Face", Pattern::Hedge => "Hedge", Pattern::Planks => "Planks", Pattern::Water => "Water", Pattern::Ice => "Ice", Pattern::Plain => "Plain",
            Pattern::Panels => "Panels", Pattern::Grid => "Grid", Pattern::Hex => "Hex", Pattern::Circuit => "Circuit",
        }
    }
}

/// A tiling surface material generated procedurally.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Material {
    pub pattern: Pattern,
    pub base: Rgb,
    pub mortar: Rgb,
    pub noise: u32,
    pub damage: f32,
    pub seed: u32,
    /// Metres covered by one repeat of the texture.
    pub tile_size: f32,
    pub rotate: bool,
    /// Albedo multiplier.
    pub brightness: f32,
    /// How mirror-like a floor is: 0 matte, about 0.3-0.5 wet stone, 0.6 ice, 1 still water.
    /// Reflections are traced against the finished frame, so lamps, trees and the sky show in it.
    pub gloss: f32,
    /// Ripples (water) or roughness (wet stone) that break reflections up: 0 glassy .. 1 choppy.
    pub ripples: f32,
    /// How brightly the lit parts of a futuristic pattern shine (Panels' light bars, Grid lines,
    /// Hex seams, Circuit traces, in the `mortar` colour): 0 they are only inlays, 1 bright.
    pub glow: f32,
}
impl Default for Material {
    fn default() -> Self {
        Self {
            pattern: Pattern::Cobblestone, base: [80, 72, 62], mortar: [34, 30, 26], noise: 10, damage: 0.2,
            seed: 0, tile_size: 2.4, rotate: false, brightness: 1.0, gloss: 0.0, ripples: 0.0, glow: 0.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Sky {
    pub enabled: bool,
    pub top: Rgb,
    pub horizon: Rgb,
    pub sun: SkyBody,
    pub moon: Moon,
    pub stars: Stars,
    pub clouds: Clouds,
    /// Northern lights: curtains of green and violet light rippling across the sky.
    pub aurora: Aurora,
    pub rainbow: Rainbow,
    /// Deep space: a dense starfield, nebulae, a galaxy band and planets.
    pub space: Space,
    /// A tunnel all round the path in place of the sky: hyperspace or a wormhole.
    pub tunnel: Tunnel,
}
impl Default for Sky {
    fn default() -> Self {
        Self {
            enabled: true, top: [70, 110, 170], horizon: [200, 190, 160],
            sun: SkyBody::default(), moon: Moon::default(), stars: Stars::default(), clouds: Clouds::default(),
            aurora: Aurora::default(), rainbow: Rainbow::default(), space: Space::default(), tunnel: Tunnel::default(),
        }
    }
}

/// A tube all round the path, filling everything the world does not draw (sky, and below the
/// horizon too): the walk runs down its middle and its walls rush past. It hides the rest of the
/// sky. Use it with a path over a drop (`path.bridge`, `bottom: Void`) so it shows below as well.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Tunnel {
    pub enabled: bool,
    pub kind: TunnelKind,
    /// The tube's radius, metres: small for a tight tunnel, large for an open one.
    pub radius: f32,
    /// The walls' deep colour, the streaks or bands, and the light at the far end.
    pub colors: [Rgb; 3],
    pub intensity: f32,
    /// How much faster than the walk the walls rush past: extra loop lengths per loop (whole).
    pub rush: u32,
    /// Whole turns the pattern winds round the tube along one loop length (a corkscrew).
    pub twist: i32,
    /// Whole turns the whole tunnel spins in one loop.
    pub spin: i32,
    /// Hyperspace: how many streaks; Wormhole: how fine the bands. 1 is natural.
    pub density: f32,
    /// Brightness of the light at the far end.
    pub core: f32,
    /// How much it lights the path in its colour (0 none).
    pub light: f32,
    pub seed: u32,
}
impl Default for Tunnel {
    fn default() -> Self {
        Tunnel {
            enabled: false, kind: TunnelKind::Hyperspace, radius: 6.0, colors: [[6, 10, 30], [170, 210, 255], [235, 245, 255]],
            intensity: 1.0, rush: 3, twist: 0, spin: 0, density: 1.0, core: 1.0, light: 0.5, seed: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum TunnelKind {
    /// Streaks of starlight stretched past at light speed.
    #[default]
    Hyperspace,
    /// Swirling bands of glowing gas spiralling down a throat to a bright far end.
    Wormhole,
}

/// The sky seen from space. It sits at infinity, so it stays put while the camera walks.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Space {
    pub enabled: bool,
    /// Space all round: where nothing is drawn below the horizon (a bridge's bottomless drop, the
    /// gaps beside a floating walkway) shows space too, instead of the void colour. The sky's
    /// `top`..`horizon` gradient is mirrored below the horizon.
    pub below: bool,
    /// The dense field of faint stars: 0 none .. 1 a rich field .. 2 a crowded one.
    pub stars: f32,
    pub star_brightness: f32,
    /// The stars' colours: 1 hot blue-white to cool orange; 0 all the faint blue-white of
    /// ball_swing_game's starfield.
    pub star_colors: f32,
    /// Glowing gas clouds: strength 0 .. 2, two colours, and size (1 about a sky's height across).
    pub nebula: f32,
    pub nebula_colors: [Rgb; 2],
    pub nebula_scale: f32,
    /// The band of a galaxy seen edge-on, with dark dust lanes and a brighter core.
    pub galaxy: f32,
    pub galaxy_color: Rgb,
    /// Tilt of the band in degrees (0 level, positive rising to the right).
    pub galaxy_angle: f32,
    /// Its width, as a share of the sky's height.
    pub galaxy_width: f32,
    /// Where the band crosses the middle of the frame, in sky heights above the horizon.
    pub galaxy_height: f32,
    /// Where its bright core sits along the band: -1 left .. 1 right.
    pub galaxy_core: f32,
    /// Up to three planets, drawn in order (later ones in front).
    pub planets: Vec<Planet>,
    /// A black hole: its shadow, a glowing disk lensed round it, and matter falling in.
    pub black_hole: BlackHole,
    pub seed: u32,
}
impl Default for Space {
    fn default() -> Self {
        Space {
            enabled: false, below: true, stars: 1.0, star_brightness: 1.0, star_colors: 1.0,
            nebula: 0.6, nebula_colors: [[190, 70, 200], [60, 120, 255]], nebula_scale: 1.0,
            galaxy: 0.7, galaxy_color: [235, 225, 255], galaxy_angle: 18.0, galaxy_width: 0.22, galaxy_height: 0.55, galaxy_core: 0.25,
            planets: Vec::new(), black_hole: BlackHole::default(), seed: 0,
        }
    }
}

/// A black hole in the space sky. Its gravity bends light: the stars and nebulae behind it are
/// lensed into arcs round it, and the far side of its accretion disk shows over the top and under
/// the bottom of the dark shadow, ringed by a thin bright photon ring. The disk's side turning
/// towards you is brighter and bluer. Matter drifts in from both sides of the frame, speeds up,
/// swirls into the disk's plane and falls in, stretching and reddening. Planets are not lensed:
/// keep them clear of it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct BlackHole {
    pub enabled: bool,
    /// Centre: x 0..1 across the frame, y 0 (top) .. 1 (horizon).
    pub pos: [f32; 2],
    /// Radius of the dark shadow, as a share of the sky's height.
    pub size: f32,
    /// How edge-on the disk is seen, degrees: 0 face-on, 90 edge-on.
    pub tilt: f32,
    /// The disk's roll on screen, degrees (positive turns it anticlockwise).
    pub roll: f32,
    /// The disk's inner and outer edge, in horizon radii (3 is the innermost stable orbit).
    pub disk_inner: f32,
    pub disk_outer: f32,
    /// The disk's colour from hot (inner) through warm to cool (outer).
    pub disk_colors: [Rgb; 3],
    pub brightness: f32,
    /// How much brighter and bluer the side turning towards you is: 0 even .. 1 physical.
    pub doppler: f32,
    /// Turns the disk's inner edge makes in one loop (the outer part turns slower). Whole turns
    /// keep the loop seamless; negative spins it the other way.
    pub spin: i32,
    /// How strongly it bends the light behind it: 1 natural, 0 none.
    pub lensing: f32,
    /// How many streaks of matter are falling in at once, from both sides (0 none, at most 48).
    pub infall: u32,
    /// Whole trips each streak makes per loop, at least (some make one more).
    pub infall_speed: u32,
    pub infall_color: Rgb,
    /// How many nodes like ball_swing_game's hook nodes fall in too (0 none): smaller and dimmer
    /// than the real ones, shrinking as they recede and stretched along their way near the end.
    pub nodes: u32,
    /// A node's width as it comes in, as a share of the frame's width (the game's real nodes are
    /// about 0.03).
    pub node_size: f32,
    /// The nodes' dark body and their two glowing rings.
    pub node_colors: [Rgb; 3],
    /// How many times further away the hole is than the nodes start (they float in at play
    /// depth, then are pulled back to it, shrinking with distance).
    pub distance: f32,
    /// How much its disk lights the scene, in its warm colour (0 none).
    pub light: f32,
    pub seed: u32,
}
impl Default for BlackHole {
    fn default() -> Self {
        BlackHole {
            enabled: false, pos: [0.5, 0.45], size: 0.12, tilt: 84.0, roll: -8.0, disk_inner: 3.0, disk_outer: 11.0,
            disk_colors: [[255, 246, 228], [255, 176, 96], [190, 64, 26]], brightness: 1.0, doppler: 0.9, spin: 4,
            lensing: 1.0, infall: 18, infall_speed: 1, infall_color: [255, 214, 170],
            nodes: 0, node_size: 0.018, node_colors: [[22, 34, 66], [90, 230, 210], [156, 126, 250]], distance: 30.0, light: 0.5, seed: 0,
        }
    }
}

/// What a planet's surface is made of.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum PlanetKind {
    /// Cratered rock: `color` lowlands, `color2` highlands.
    #[default]
    Rocky,
    /// A gas giant in bands of `color` and `color2`, with a storm.
    Gas,
    /// Oceans (`color`), land (`color2`), ice caps and white cloud; city lights on the night side.
    Earth,
    /// Pale ice (`color`) with bright cracks (`color2`).
    Ice,
    /// Dark crust (`color`) split by glowing lava (`color2`), which glows on the night side too.
    Lava,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Planet {
    pub enabled: bool,
    pub kind: PlanetKind,
    /// Centre: x 0..1 across the frame, y 0 (top) .. 1 (horizon); past 1 it sits below the
    /// horizon, which shows only with `space.below`.
    pub pos: [f32; 2],
    /// Radius as a share of the sky's height.
    pub radius: f32,
    pub color: Rgb,
    pub color2: Rgb,
    /// The glow of its air round the rim, and how strong (0 airless).
    pub atmosphere: Rgb,
    pub atmosphere_strength: f32,
    /// Where its sunlight comes from across the frame, in degrees: 0 from the right, 90 from above.
    pub light_angle: f32,
    /// How much of the face is in night: 0 fully lit .. 0.5 half .. 1 a thin crescent.
    pub night: f32,
    /// Tilt of its axis (and its bands), degrees.
    pub tilt: f32,
    /// Whole turns it spins in one loop (0 still; it must be whole for the loop to close).
    pub spin: i32,
    /// Earth and gas giants: how much white cloud.
    pub clouds: f32,
    /// Earth: city lights on the night side.
    pub city_lights: f32,
    pub rings: Rings,
    pub seed: u32,
}
impl Default for Planet {
    fn default() -> Self {
        Planet {
            enabled: true, kind: PlanetKind::Rocky, pos: [0.7, 0.4], radius: 0.25,
            color: [150, 130, 115], color2: [205, 190, 170], atmosphere: [120, 170, 255], atmosphere_strength: 0.0,
            light_angle: 30.0, night: 0.4, tilt: 15.0, spin: 0, clouds: 0.5, city_lights: 0.0,
            rings: Rings::default(), seed: 0,
        }
    }
}

/// A ring system round a planet, its near half in front of the planet and its far half behind.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Rings {
    pub enabled: bool,
    /// Inner and outer edge, in planet radii.
    pub inner: f32,
    pub outer: f32,
    /// How far open they are seen: 0 edge-on .. 1 face-on.
    pub open: f32,
    /// Tilt of the ring plane across the frame, degrees.
    pub angle: f32,
    pub color: Rgb,
    pub opacity: f32,
}
impl Default for Rings {
    fn default() -> Self { Rings { enabled: false, inner: 1.35, outer: 2.3, open: 0.25, angle: -12.0, color: [220, 200, 170], opacity: 0.8 } }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Aurora {
    pub enabled: bool,
    pub intensity: f32,
    /// Colour of the bright lower edge, the glow over it and the rays rising from it.
    pub low: Rgb,
    /// Colour of the diffuse veil high above the edge.
    pub high: Rgb,
    /// Colour of the bright knots that flare along the edge.
    pub accent: Rgb,
    /// Where the lower edge hangs: 0 high in the sky .. 1 down at the horizon.
    pub height: f32,
    /// How far the veil rises above the edge (1 = about the height of the sky).
    pub tall: f32,
    /// How fast the light drifts and flows along the band.
    pub speed: f32,
    /// How much of the band is lit at once: 0 a few patches .. 1 nearly all of it.
    pub coverage: f32,
    /// Fine vertical rays in the curtain: 0 a smooth glow .. 1 natural .. 2 strongly rayed.
    pub rays: f32,
    /// How much the lower edge swells and folds: 0 a straight band.
    pub waves: f32,
    /// How much the band bows up in the middle, as the far arc of the auroral oval does.
    pub arc: f32,
    /// Brightness of the thin lower edge against the rest.
    pub edge: f32,
    /// How much the aurora lights the ground green (a faint, even glow).
    pub ground_glow: f32,
    pub seed: u32,
}
impl Default for Aurora {
    fn default() -> Self {
        Aurora {
            enabled: false, intensity: 1.0, low: [70, 255, 150], high: [225, 130, 255], accent: [255, 170, 245],
            height: 0.6, tall: 1.0, speed: 1.0, coverage: 0.5, rays: 1.0, waves: 0.5, arc: 1.0, edge: 1.0,
            ground_glow: 0.5, seed: 0,
        }
    }
}

/// A rainbow: an arc round the point opposite the sun, with a faint second bow outside it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Rainbow {
    pub enabled: bool,
    pub intensity: f32,
    /// Where its centre is across the sky (0 left .. 1 right).
    pub x: f32,
    /// Size: 1 spans the frame's width (a real 42-degree bow is wider than a portrait view).
    pub size: f32,
    /// Draw the fainter second bow outside the first.
    pub double: bool,
}
impl Default for Rainbow {
    fn default() -> Self { Rainbow { enabled: false, intensity: 0.5, x: 0.5, size: 1.0, double: true } }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct SkyBody {
    pub enabled: bool,
    /// Position in the sky: x 0..1 across, y 0 (top) .. 1 (horizon).
    pub pos: [f32; 2],
    /// Radius as a fraction of the sky height.
    pub radius: f32,
    pub color: Rgb,
    /// Lights the world and casts shadows.
    pub emits_light: bool,
    pub intensity: f32,
}
impl Default for SkyBody {
    fn default() -> Self {
        Self { enabled: false, pos: [0.72, 0.35], radius: 0.08, color: [255, 236, 190], emits_light: true, intensity: 1.0 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Moon {
    pub body: SkyBody,
    /// -1 waning .. 0 full .. 1 waxing.
    pub phase: f32,
    pub opacity: f32,
    pub craters: bool,
}
impl Default for Moon {
    fn default() -> Self {
        Self {
            body: SkyBody { pos: [0.25, 0.3], radius: 0.06, color: [222, 230, 255], intensity: 0.35, ..SkyBody::default() },
            phase: 0.0, opacity: 0.9, craters: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Stars {
    pub enabled: bool,
    pub count: u32,
    pub size: f32,
    pub twinkle: f32,
    pub seed: u32,
}
impl Default for Stars {
    fn default() -> Self { Self { enabled: false, count: 140, size: 1.0, twinkle: 0.5, seed: 0 } }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Clouds {
    pub enabled: bool,
    pub count: u32,
    /// Drifts across the sky per loop. Whole numbers wrap each cloud round the sky; any other
    /// value (0.4, 1.5) keeps its exact speed, with each cloud forming and dissolving over one
    /// loop so the loop stays seamless.
    pub drift: f32,
    pub scale: f32,
    pub opacity: f32,
    pub tint: Rgb,
    pub variation: f32,
    /// Shadows of clouds drifting over the ground (0..1; needs a light-giving sun). They drift
    /// with the wind when there is one, and work with the clouds themselves switched off too.
    pub shadows: f32,
    pub seed: u32,
}
impl Default for Clouds {
    fn default() -> Self {
        Self { enabled: false, count: 10, drift: 1.0, scale: 1.0, opacity: 0.4, tint: [226, 230, 238], variation: 0.55, shadows: 0.0, seed: 0 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Lighting {
    /// Ambient light level (0..2).
    pub ambient: f32,
    pub ambient_color: Rgb,
    /// Colour of the far distance and of anything outside the world (dungeon darkness).
    pub void_color: Rgb,
    /// Distance fog.
    pub fog: Fog,
    /// Light bands: 0 = smooth light, 2..8 = cel-shaded steps.
    pub bands: u32,
}
impl Default for Lighting {
    fn default() -> Self {
        Self { ambient: 0.6, ambient_color: [200, 205, 220], void_color: [0, 0, 0], fog: Fog::default(), bands: 0 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Fog {
    pub enabled: bool,
    pub color: Rgb,
    /// Distance at which about two-thirds of a surface is hidden, metres.
    pub distance: f32,
    /// Use the sky's horizon colour instead of `color` when the sky is on.
    pub match_sky: bool,
}
impl Default for Fog {
    fn default() -> Self { Self { enabled: true, color: [0, 0, 0], distance: 26.0, match_sky: false } }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum FixtureKind { Torch, Lantern, Candle, Brazier, Crystal, Firefly, Magic, GreenFire, IceWisp }

impl FixtureKind {
    pub const ALL: [FixtureKind; 9] = [
        FixtureKind::Torch, FixtureKind::Lantern, FixtureKind::Candle, FixtureKind::Brazier, FixtureKind::Crystal,
        FixtureKind::Firefly, FixtureKind::Magic, FixtureKind::GreenFire, FixtureKind::IceWisp,
    ];
    pub fn name(self) -> &'static str {
        match self {
            FixtureKind::Torch => "Torch", FixtureKind::Lantern => "Lantern", FixtureKind::Candle => "Candle",
            FixtureKind::Brazier => "Brazier", FixtureKind::Crystal => "Crystal", FixtureKind::Firefly => "Firefly",
            FixtureKind::Magic => "Magic", FixtureKind::GreenFire => "Green Fire", FixtureKind::IceWisp => "Ice Wisp",
        }
    }
    /// Light colour.
    pub fn glow(self) -> Rgb {
        match self {
            FixtureKind::Torch => [255, 150, 60], FixtureKind::Lantern => [255, 200, 120],
            FixtureKind::Candle => [255, 175, 80], FixtureKind::Brazier => [255, 130, 40],
            FixtureKind::Crystal => [120, 200, 255], FixtureKind::Firefly => [150, 255, 90],
            FixtureKind::Magic => [150, 90, 255], FixtureKind::GreenFire => [80, 255, 70],
            FixtureKind::IceWisp => [110, 210, 255],
        }
    }
    /// Flame gradient from core to edge, or None for a soft orb.
    pub fn flame(self) -> Option<[Rgb; 4]> {
        match self {
            FixtureKind::Torch | FixtureKind::Brazier => Some([[255, 255, 210], [255, 205, 60], [255, 110, 15], [200, 50, 5]]),
            FixtureKind::Lantern => Some([[255, 255, 235], [255, 225, 150], [255, 185, 80], [215, 130, 30]]),
            FixtureKind::Candle => Some([[255, 245, 200], [255, 190, 70], [255, 110, 25], [200, 55, 5]]),
            FixtureKind::Magic => Some([[230, 210, 255], [150, 100, 255], [90, 50, 230], [45, 15, 190]]),
            FixtureKind::GreenFire => Some([[220, 255, 210], [100, 235, 60], [25, 170, 15], [5, 95, 5]]),
            FixtureKind::IceWisp => Some([[225, 245, 255], [130, 210, 255], [50, 150, 230], [15, 85, 190]]),
            FixtureKind::Crystal | FixtureKind::Firefly => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Side { Both, Left, Right, Center }
impl Side {
    pub const ALL: [Side; 4] = [Side::Both, Side::Left, Side::Right, Side::Center];
    pub fn name(self) -> &'static str {
        match self { Side::Both => "Both", Side::Left => "Left", Side::Right => "Right", Side::Center => "Center" }
    }
    /// Signs of the lateral offset for each instance: -1 left, 1 right, 0 centre.
    pub fn signs(self) -> &'static [f32] {
        match self { Side::Both => &[-1.0, 1.0], Side::Left => &[-1.0], Side::Right => &[1.0], Side::Center => &[0.0] }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Mount { Wall, Ground, Ceiling, Floating }
impl Mount {
    pub const ALL: [Mount; 4] = [Mount::Wall, Mount::Ground, Mount::Ceiling, Mount::Floating];
    pub fn name(self) -> &'static str {
        match self { Mount::Wall => "Wall", Mount::Ground => "Ground", Mount::Ceiling => "Ceiling", Mount::Floating => "Floating" }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub enum SetPieceKind {
    /// Stone piers and a round arch with a keystone.
    Archway,
    /// An archway with its crown fallen in.
    RuinedArch,
    /// A gatehouse: heavy piers, a lintel and a raised portcullis.
    Gate,
    /// A beam across the path with cloth banners hanging from it.
    Banners,
    /// A glowing ring over the path.
    Portal,
}
impl SetPieceKind {
    pub const ALL: [SetPieceKind; 5] = [SetPieceKind::Archway, SetPieceKind::RuinedArch, SetPieceKind::Gate, SetPieceKind::Banners, SetPieceKind::Portal];
    pub fn name(self) -> &'static str {
        match self { SetPieceKind::Archway => "Archway", SetPieceKind::RuinedArch => "Ruined Arch", SetPieceKind::Gate => "Gate", SetPieceKind::Banners => "Banners", SetPieceKind::Portal => "Portal" }
    }
}

/// A structure spanning the path, repeated along it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct SetPiece {
    pub enabled: bool,
    pub kind: SetPieceKind,
    /// Distance between repeats, metres (snapped to divide the loop; the loop length = once per loop).
    pub spacing: f32,
    /// Shift along the path, metres.
    pub offset: f32,
    /// Clear width of the opening, metres. 0 = fit the path (and the gap to the walls).
    pub width: f32,
    /// Clear height of the opening, metres.
    pub height: f32,
    /// Stone or wood colour.
    pub tint: Rgb,
    /// Cloth, trim or glow colour.
    pub accent: Rgb,
    /// Sun shadow strength, 0..1.
    pub shadow: f32,
    /// An image used instead of the painted structure; it is stretched to span the opening.
    pub sprite: SpriteRef,
    pub seed: u32,
}
impl Default for SetPiece {
    fn default() -> Self {
        SetPiece {
            enabled: true, kind: SetPieceKind::Archway, spacing: 24.0, offset: 12.0, width: 0.0, height: 3.4,
            tint: [118, 110, 100], accent: [150, 30, 36], shadow: 0.6, sprite: SpriteRef::default(), seed: 0,
        }
    }
}

/// Something travelling along with the camera at the walk's own speed, so it keeps its place in
/// the frame: a ship flying beside the path. It follows the path round bends, bobs and weaves in
/// whole cycles per loop, and its engines light what is near.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Companion {
    pub enabled: bool,
    /// Where it flies, metres: sideways from the path's centre (right positive), height above the
    /// path, and distance ahead of the camera.
    pub offset: [f32; 3],
    /// Its wingspan, metres (with a sprite: its height).
    pub size: f32,
    /// The drawn ship's hull, trim and engine-glow colours.
    pub color: Rgb,
    pub accent: Rgb,
    pub engine: Rgb,
    /// How far it rises and falls (metres), and how many times per loop.
    pub bob: f32,
    pub bob_cycles: u32,
    /// How far it drifts sideways (metres), and how many times per loop.
    pub weave: f32,
    pub weave_cycles: u32,
    /// How strongly its engines light what is near (0 not at all).
    pub light: f32,
    /// An image used instead of the drawn ship (seen from behind, as it flies ahead).
    pub sprite: SpriteRef,
    /// With a sprite: texels at least this light (0..1) glow, as engines and windows do (0 none).
    pub glow_from: f32,
    pub seed: u32,
}
impl Default for Companion {
    fn default() -> Self {
        Companion {
            enabled: true, offset: [-2.4, 2.4, 10.0], size: 3.2, color: [128, 134, 146], accent: [56, 104, 180], engine: [110, 180, 255],
            bob: 0.25, bob_cycles: 2, weave: 0.4, weave_cycles: 1, light: 1.0, sprite: SpriteRef::default(), glow_from: 0.8, seed: 0,
        }
    }
}

/// Weather and the air: rain and snow, wind, storms, mist, light in the air, heat haze, the lens.
/// Every part works in any scene and repeats exactly with the loop.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default)]
pub struct Weather {
    pub lightning: Lightning,
    pub fog_banks: FogBanks,
    /// Rain, snow, sleet or hail falling everywhere open to the sky, and what it does to the ground.
    pub precipitation: Precipitation,
    /// Water dripping from the ceiling (or the tops of the walls where there is none).
    pub drips: Drips,
    /// Wind: slants rain and snow, carries particles sideways, sways plants and banners.
    pub wind: Wind,
    /// A sandstorm: sand streaming past, a sand-coloured haze, the sun dimmed to a disc.
    pub sandstorm: Sandstorm,
    /// Mist lying on the ground, and wisps rising from it.
    pub mist: Mist,
    /// Light made visible by the air: shafts from the sun past trees and walls, haloes round lamps.
    pub light_shafts: LightShafts,
    /// Heat haze: the air shimmers over the ground in the distance.
    pub heat_shimmer: HeatShimmer,
    /// Raindrops or frost on the camera's lens.
    pub lens: Lens,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
pub enum PrecipKind {
    #[default]
    Rain,
    Snow,
    /// Wet snow mixed with rain.
    Sleet,
    Hail,
}

/// Rain, snow, sleet or hail. It falls wherever the sky is open (not under a ceiling).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Precipitation {
    pub enabled: bool,
    pub kind: PrecipKind,
    /// How hard it falls: 0.1 a few drops or flakes, 0.5 steady, 1 a downpour or a blizzard.
    pub intensity: f32,
    /// Size of the drops, flakes or hailstones.
    pub size: f32,
    /// Colour multiplied over the drops or flakes (white = their natural colour in the scene's light).
    pub tint: Rgb,
    /// Rain and sleet: how wet the ground gets, darker and glossy (0..1).
    pub wetness: f32,
    /// Rain and sleet: puddles on the path and verge that mirror the scene, with rings where drops land (0..1 = how much ground they cover).
    pub puddles: f32,
    /// Rain, sleet and hail: splashes where drops land (0..1).
    pub splashes: f32,
    /// Snow and sleet: snow lying on the ground, the verges, the tops of walls and props (0..1).
    pub cover: f32,
    /// A trodden track down the middle of the path in lying snow (0 = untouched .. 1 = trodden to slush).
    pub track: f32,
    /// Haze of a heavy fall: curtains of rain in the distance, a blizzard's whiteout (0..1).
    pub haze: f32,
    pub seed: u32,
}
impl Default for Precipitation {
    fn default() -> Self {
        Precipitation {
            enabled: false, kind: PrecipKind::Rain, intensity: 0.5, size: 1.0, tint: [255, 255, 255], wetness: 0.8,
            puddles: 0.3, splashes: 0.6, cover: 0.8, track: 0.6, haze: 0.5, seed: 0,
        }
    }
}

/// Drops falling from the ceiling, or from the tops of the walls where there is no ceiling.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Drips {
    pub enabled: bool,
    /// Drips per metre of path per loop.
    pub rate: f32,
    pub color: Rgb,
    pub seed: u32,
}
impl Default for Drips {
    fn default() -> Self { Drips { enabled: false, rate: 1.0, color: [190, 205, 220], seed: 0 } }
}

/// Wind across the path.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Wind {
    pub enabled: bool,
    /// Wind speed, metres per second: positive blows to the right, negative to the left.
    pub speed: f32,
    /// How much it rises and falls in gusts (0 steady .. 1 squally).
    pub gusts: f32,
    /// How much trees, reeds, grass and banners sway (0..1).
    pub sway: f32,
    pub seed: u32,
}
impl Default for Wind {
    fn default() -> Self { Wind { enabled: false, speed: 3.0, gusts: 0.4, sway: 0.5, seed: 0 } }
}

/// A sandstorm: sand streaming past on the wind (to the right unless the wind says otherwise),
/// a sand-coloured haze that hides the distance, and the sun dimmed to a disc.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Sandstorm {
    pub enabled: bool,
    /// 0.2 blowing sand .. 1 a wall of sand.
    pub intensity: f32,
    pub color: Rgb,
    pub seed: u32,
}
impl Default for Sandstorm {
    fn default() -> Self { Sandstorm { enabled: false, intensity: 0.5, color: [214, 172, 116], seed: 0 } }
}

/// Mist lying low over the ground, drifting, with wisps rising from it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Mist {
    pub enabled: bool,
    /// Height of the mist above the ground, metres.
    pub height: f32,
    /// Thickness: optical depth per metre inside it (0.2 thin .. 1.5 thick).
    pub density: f32,
    pub color: Rgb,
    /// How patchy it is (0 even .. 1 in drifting banks).
    pub patchiness: f32,
    /// Wisps rising from it (0..1).
    pub wisps: f32,
    pub seed: u32,
}
impl Default for Mist {
    fn default() -> Self { Mist { enabled: false, height: 0.8, density: 0.25, color: [205, 210, 220], patchiness: 0.6, wisps: 0.4, seed: 0 } }
}

/// Light scattered by the air. Stronger in fog, mist, haze and dust.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct LightShafts {
    pub enabled: bool,
    /// Rays from the sun past trees, walls and arches (needs the sun in the sky).
    pub sun: f32,
    /// Glowing haloes in the air round lamps, torches and other lights.
    pub lamps: f32,
}
impl Default for LightShafts {
    fn default() -> Self { LightShafts { enabled: false, sun: 0.6, lamps: 0.4 } }
}

/// Heat haze: the picture wavers over the distant ground and just above the horizon.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct HeatShimmer {
    pub enabled: bool,
    /// How far the picture wavers (0..1).
    pub strength: f32,
    /// How fast it wavers.
    pub speed: f32,
}
impl Default for HeatShimmer {
    fn default() -> Self { HeatShimmer { enabled: false, strength: 0.5, speed: 1.0 } }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
pub enum LensKind {
    /// Raindrops that land on the lens, linger and run down.
    #[default]
    Drops,
    /// Frost creeping in from the edges.
    Frost,
}

/// Something on the camera's lens.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Lens {
    pub enabled: bool,
    pub kind: LensKind,
    /// How many drops, or how far the frost reaches in (0..1).
    pub amount: f32,
    pub seed: u32,
}
impl Default for Lens {
    fn default() -> Self { Lens { enabled: false, kind: LensKind::Drops, amount: 0.5, seed: 0 } }
}

/// Lightning strikes: a flash that lights the whole scene, and a bolt in the sky.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Lightning {
    pub enabled: bool,
    /// Strikes per loop.
    pub strikes: u32,
    /// Brightness of the flash.
    pub intensity: f32,
    pub color: Rgb,
    /// Draw the bolt in the sky (needs the sky visible).
    pub bolts: bool,
    pub seed: u32,
}
impl Default for Lightning {
    fn default() -> Self { Lightning { enabled: false, strikes: 2, intensity: 1.5, color: [205, 215, 255], bolts: true, seed: 0 } }
}

/// Patches of thick fog along the path that the camera walks into and out of.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct FogBanks {
    pub enabled: bool,
    /// Distance between banks, metres (snapped to divide the loop).
    pub spacing: f32,
    /// Length of each bank along the path, metres.
    pub length: f32,
    /// Thickness inside a bank: optical depth per metre (0.3 = light mist, 1.5 = a wall of fog).
    pub density: f32,
    pub offset: f32,
}
impl Default for FogBanks {
    fn default() -> Self { FogBanks { enabled: false, spacing: 12.0, length: 4.0, density: 0.5, offset: 0.0 } }
}

/// A repeating light source along the path: torches, lanterns, fireflies, crystals.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Fixture {
    pub enabled: bool,
    pub kind: FixtureKind,
    pub side: Side,
    pub mount: Mount,
    /// Height of the flame or orb above the path, metres.
    pub height: f32,
    /// Distance between fixtures along the path, metres (snapped to divide the loop).
    pub spacing: f32,
    /// Shift of the whole row along the path, metres.
    pub offset: f32,
    /// For ground and floating mounts: distance outside the path edge, metres (negative = over the path).
    pub lateral: f32,
    /// Size multiplier for the fixture and flame.
    pub size: f32,
    pub light: bool,
    pub intensity: f32,
    /// Reach of the light, metres.
    pub radius: f32,
    pub flicker: f32,
    /// Random placement wobble, metres.
    pub jitter: f32,
    pub sprite: SpriteRef,
    pub seed: u32,
}
impl Default for Fixture {
    fn default() -> Self {
        Self {
            enabled: true, kind: FixtureKind::Torch, side: Side::Both, mount: Mount::Wall, height: 2.1, spacing: 6.0,
            offset: 0.0, lateral: 0.3, size: 1.0, light: true, intensity: 1.0, radius: 6.0, flicker: 1.0, jitter: 0.0,
            sprite: SpriteRef::default(), seed: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum PropKind { Tree, Pine, Bush, Rock, Boulder, Cactus, DeadTree, Mushroom, Pillar, Gravestone, Crystal, Stalagmite, Reeds, Willow, Palm, Obelisk, Icicle, IceSpike }

impl PropKind {
    pub const ALL: [PropKind; 18] = [
        PropKind::Tree, PropKind::Pine, PropKind::Bush, PropKind::Rock, PropKind::Boulder, PropKind::Cactus,
        PropKind::DeadTree, PropKind::Mushroom, PropKind::Pillar, PropKind::Gravestone, PropKind::Crystal,
        PropKind::Stalagmite, PropKind::Reeds, PropKind::Willow, PropKind::Palm, PropKind::Obelisk,
        PropKind::Icicle, PropKind::IceSpike,
    ];
    pub fn name(self) -> &'static str {
        match self {
            PropKind::Tree => "Tree", PropKind::Pine => "Pine", PropKind::Bush => "Bush", PropKind::Rock => "Rock",
            PropKind::Boulder => "Boulder", PropKind::Cactus => "Cactus", PropKind::DeadTree => "Dead Tree",
            PropKind::Mushroom => "Mushroom", PropKind::Pillar => "Pillar", PropKind::Gravestone => "Gravestone",
            PropKind::Crystal => "Crystal", PropKind::Stalagmite => "Stalagmite", PropKind::Reeds => "Reeds",
            PropKind::Willow => "Willow", PropKind::Palm => "Palm", PropKind::Obelisk => "Obelisk",
            PropKind::Icicle => "Icicle", PropKind::IceSpike => "Ice Spike",
        }
    }
    /// Hangs from the ceiling (or the top of the walls) instead of standing on the ground.
    pub fn hangs(self) -> bool { self == PropKind::Icicle }
    /// How much it lights itself (crystals and ice glow faintly).
    pub fn glow(self) -> f32 { match self { PropKind::Crystal => 0.6, PropKind::IceSpike | PropKind::Icicle => 0.12, _ => 0.0 } }
    /// Natural height in metres at scale 1.
    pub fn height_m(self) -> f32 {
        match self {
            PropKind::Tree => 6.0, PropKind::Pine => 7.0, PropKind::Bush => 1.1, PropKind::Rock => 0.6,
            PropKind::Boulder => 1.6, PropKind::Cactus => 2.6, PropKind::DeadTree => 5.0, PropKind::Mushroom => 0.7,
            PropKind::Pillar => 4.0, PropKind::Gravestone => 1.0, PropKind::Crystal => 1.3, PropKind::Stalagmite => 1.8,
            PropKind::Reeds => 1.5, PropKind::Willow => 6.5, PropKind::Palm => 7.0, PropKind::Obelisk => 5.0,
            PropKind::Icicle => 1.1, PropKind::IceSpike => 1.7,
        }
    }
    pub fn default_tint(self) -> Rgb {
        match self {
            PropKind::Tree => [40, 110, 34], PropKind::Pine => [26, 86, 40], PropKind::Bush => [38, 100, 30],
            PropKind::Rock => [110, 104, 96], PropKind::Boulder => [96, 92, 86], PropKind::Cactus => [52, 130, 50],
            PropKind::DeadTree => [86, 70, 52], PropKind::Mushroom => [200, 50, 46], PropKind::Pillar => [120, 112, 100],
            PropKind::Gravestone => [104, 104, 110], PropKind::Crystal => [120, 190, 255], PropKind::Stalagmite => [96, 84, 74],
            PropKind::Reeds => [104, 112, 58], PropKind::Willow => [70, 96, 52], PropKind::Palm => [64, 118, 46],
            PropKind::Obelisk => [180, 156, 116], PropKind::Icicle => [196, 228, 246], PropKind::IceSpike => [168, 214, 240],
        }
    }
}

/// A repeating row (or several rows) of props beside the path.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct PropLayer {
    pub enabled: bool,
    pub kind: PropKind,
    pub side: Side,
    /// Distance from the path edge to the prop's centre, metres (negative = on the path).
    pub lateral: f32,
    /// Distance between props along the path, metres (snapped to divide the loop).
    pub spacing: f32,
    pub offset: f32,
    /// Extra rows further from the path.
    pub rows: u32,
    pub row_spacing: f32,
    /// Random sideways placement, metres.
    pub jitter: f32,
    /// Share of slots that hold a prop (0..1), for natural gaps.
    pub density: f32,
    pub scale: f32,
    pub scale_var: f32,
    /// Colour of procedural props. Left out of a scene file, it is the kind's own colour.
    pub tint: Rgb,
    /// How far the base sinks into the ground, as a fraction of height.
    pub sink: f32,
    /// Floating props (asteroids, drifting islands, lanterns): height above the ground, metres,
    /// and a random spread either way. Floating props hang over a bridge's drop too, and cast no
    /// sun shadow.
    pub float: f32,
    pub float_var: f32,
    pub shadow: bool,
    pub shadow_opacity: f32,
    pub sprite: SpriteRef,
    pub seed: u32,
    /// A prop described by data instead of `kind`: the name of one in `prop_defs`, or
    /// `"<kit folder>#<name>"` for one in a kit (relative to the scene file). Placement (side,
    /// spacing, scale...) still comes from this layer.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub def: String,
}
impl Default for PropLayer {
    fn default() -> Self {
        Self {
            enabled: true, kind: PropKind::Tree, side: Side::Both, lateral: 1.5, spacing: 6.0, offset: 0.0, rows: 1,
            row_spacing: 3.0, jitter: 0.5, density: 1.0, scale: 1.0, scale_var: 0.2, tint: PropKind::Tree.default_tint(),
            sink: 0.02, float: 0.0, float_var: 0.0, shadow: true, shadow_opacity: 0.6, sprite: SpriteRef::default(), seed: 1, def: String::new(),
        }
    }
}

/// An image file used in place of a procedural shape. Paths are relative to the scene file.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct SpriteRef {
    /// One image, or empty.
    pub path: String,
    /// Several images to pick from at random per instance (overrides `path` when not empty).
    pub pool: Vec<String>,
    pub flip_x: bool,
    /// Pixel art: sample nearest instead of smoothing.
    pub pixelated: bool,
}
impl SpriteRef {
    pub fn is_set(&self) -> bool { !self.path.trim().is_empty() || self.pool.iter().any(|p| !p.trim().is_empty()) }
}

/// A prop made from data: an image, several images or folders of them, or a shape drawn from
/// parts. Lives in `scene.prop_defs` or in a kit (a folder holding kit.json and its images).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct PropDef {
    /// What it is, in a few words (shown when listing a kit).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Images: `path`, or `pool` (PNG files, or folders of them; one is picked per prop). Paths are
    /// relative to the file the definition is in.
    pub sprite: SpriteRef,
    /// Or a shape: parts painted in order, in metres, `x` sideways from the centre and `y` up from
    /// the ground (0).
    pub shape: Vec<ShapePart>,
    /// The built-in prop drawn when there is neither an image nor a shape; its height is also the
    /// default for images.
    pub kind: PropKind,
    /// Colour of the built-in prop; left out, the kind's own.
    pub tint: Option<Rgb>,
    /// Height in metres at scale 1: 0 = the shape's own height, or the kind's.
    pub height: f32,
    pub anchor: Anchor,
    /// Lit from within, 0..1: 0 takes only the scene's light, 1 shows its own colours in the dark.
    pub glow: f32,
    /// For images: only texels at least this light (0..1) glow, like the flame in a lantern;
    /// 0 = the whole prop glows by `glow`.
    pub glow_from: f32,
    /// Light it gives off, like a lamp.
    pub light: PropLight,
    /// Cut away transparent margins of images, so a sprite stands on its lowest visible pixel.
    pub trim: bool,
}
impl Default for PropDef {
    fn default() -> Self {
        Self {
            description: String::new(), sprite: SpriteRef::default(), shape: Vec::new(), kind: PropKind::Rock, tint: None,
            height: 0.0, anchor: Anchor::Ground, glow: 0.0, glow_from: 0.0, light: PropLight::default(), trim: true,
        }
    }
}

/// Where a prop is fixed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Anchor {
    /// Stands on the ground.
    #[default]
    Ground,
    /// Hangs from the ceiling, or from the top of the walls; stands when there is neither.
    Ceiling,
}

/// Light a prop gives off.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct PropLight {
    pub enabled: bool,
    pub color: Rgb,
    pub intensity: f32,
    /// Reach, metres.
    pub radius: f32,
    /// Where the light sits, as a fraction of the prop's height from its base.
    pub at: f32,
    /// 0 steady, 1 like a flame.
    pub flicker: f32,
}
impl Default for PropLight {
    fn default() -> Self { Self { enabled: false, color: [255, 190, 110], intensity: 0.8, radius: 4.0, at: 0.8, flicker: 0.0 } }
}

/// One part of a drawn prop.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ShapePart {
    pub shape: Shape,
    pub color: Rgb,
    /// Rounding light from the upper left: 0 flat, 1 strong.
    pub shade: f32,
    /// Cut a hole through what is painted so far instead of painting.
    pub cut: bool,
    /// Lit from within, like lantern glass or a glowing cap: 0 none, 1 its own colour at full
    /// strength in the dark, more for something bright.
    pub glow: f32,
}
impl Default for ShapePart {
    fn default() -> Self { Self { shape: Shape::Rect { min: [-0.1, 0.0], max: [0.1, 1.0] }, color: [128, 128, 128], shade: 0.5, cut: false, glow: 0.0 } }
}

/// A shape in metres: `x` sideways from the centre, `y` up from the ground.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub enum Shape {
    Ellipse { center: [f32; 2], radius: [f32; 2] },
    Rect { min: [f32; 2], max: [f32; 2] },
    /// A filled polygon (even-odd).
    Poly { points: Vec<[f32; 2]> },
    /// A straight stroke `width` metres wide.
    Line { from: [f32; 2], to: [f32; 2], width: f32 },
}

/// A kit: props for a biome or a theme, kept in a folder as kit.json beside their images, and used
/// by any scene as `def: "<folder>#<name>"`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Kit {
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub props: std::collections::BTreeMap<String, PropDef>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum ParticleKind { Dust, Embers, Fireflies, Rain, Snow, Leaves, Ash, Spores, Sand, Petals }
impl ParticleKind {
    pub const ALL: [ParticleKind; 10] = [
        ParticleKind::Dust, ParticleKind::Embers, ParticleKind::Fireflies, ParticleKind::Rain, ParticleKind::Snow,
        ParticleKind::Leaves, ParticleKind::Ash, ParticleKind::Spores, ParticleKind::Sand, ParticleKind::Petals,
    ];
    pub fn name(self) -> &'static str {
        match self {
            ParticleKind::Dust => "Dust", ParticleKind::Embers => "Embers", ParticleKind::Fireflies => "Fireflies",
            ParticleKind::Rain => "Rain", ParticleKind::Snow => "Snow", ParticleKind::Leaves => "Leaves",
            ParticleKind::Ash => "Ash", ParticleKind::Spores => "Spores", ParticleKind::Sand => "Sand", ParticleKind::Petals => "Petals",
        }
    }
    pub fn default_color(self) -> Rgb {
        match self {
            ParticleKind::Dust => [200, 190, 170], ParticleKind::Embers => [255, 140, 40],
            ParticleKind::Fireflies => [170, 255, 100], ParticleKind::Rain => [160, 180, 210],
            ParticleKind::Snow => [240, 245, 255], ParticleKind::Leaves => [170, 110, 40],
            ParticleKind::Ash => [120, 116, 112], ParticleKind::Spores => [170, 140, 255], ParticleKind::Sand => [222, 190, 140],
            ParticleKind::Petals => [255, 190, 210],
        }
    }
}

/// Small moving things in the air: dust, embers, rain, snow, leaves, petals. They drift with the wind
/// when there is one. For weather that soaks or covers the ground, use `weather.precipitation`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Particles {
    pub enabled: bool,
    pub kind: ParticleKind,
    /// Particles per loop length of world.
    pub count: u32,
    pub color: Rgb,
    pub size: f32,
    /// Speed multiplier for the particle's own motion (fall, drift, rise).
    pub speed: f32,
    pub seed: u32,
}
impl Default for Particles {
    fn default() -> Self {
        Self { enabled: true, kind: ParticleKind::Dust, count: 120, color: ParticleKind::Dust.default_color(), size: 1.0, speed: 1.0, seed: 0 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Post {
    pub exposure: f32,
    pub contrast: f32,
    pub saturation: f32,
    /// Glow around bright light (0..1).
    pub bloom: f32,
    pub vignette: f32,
    pub grain: f32,
    /// Colour multiplied over the final image.
    pub tint: Rgb,
}
impl Default for Post {
    fn default() -> Self {
        Self { exposure: 1.0, contrast: 1.0, saturation: 1.0, bloom: 0.35, vignette: 0.35, grain: 0.0, tint: [255, 255, 255] }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Style {
    /// Size of one art pixel in output pixels: 1 = full resolution, 3 = render at a third and scale up crisply.
    pub pixel_size: u32,
    pub palette: Palette,
    pub dither: Dither,
    /// 0..1
    pub dither_strength: f32,
    pub outline: Outline,
    /// Painterly brush size in art pixels (Kuwahara filter): flat strokes that keep edges. 0 = off, 2..8 typical.
    pub paint: f32,
    /// Colour grade: a built-in name (Warm, Cool, Teal Orange, Faded, Night, Sepia, Vivid) or a .cube LUT file
    /// (relative to the scene file). Empty = none.
    pub grade: String,
    /// How much of the grade to apply, 0..1.
    pub grade_strength: f32,
    /// Paper or canvas texture over the image, 0..1 (static, so loops stay seamless).
    pub paper: f32,
    /// CRT scanlines, 0..1.
    pub scanlines: f32,
}
impl Default for Style {
    fn default() -> Self {
        Self {
            pixel_size: 1, palette: Palette::Full, dither: Dither::None, dither_strength: 0.5, outline: Outline::default(),
            paint: 0.0, grade: String::new(), grade_strength: 1.0, paper: 0.0, scanlines: 0.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub enum Palette {
    /// No colour restriction.
    Full,
    /// A built-in palette by name (see `world::palette::NAMED`).
    Named(String),
    /// The best `n` colours for this scene, chosen per loop.
    Auto(u32),
    Custom(Vec<Rgb>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Dither { None, Bayer2, Bayer4, Bayer8 }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Outline {
    pub enabled: bool,
    pub color: Rgb,
    /// Outline props and fixtures only (true) or also wall and ground edges (false).
    pub objects_only: bool,
}
impl Default for Outline {
    fn default() -> Self { Self { enabled: false, color: [12, 10, 14], objects_only: true } }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Motion {
    /// World distance covered by one loop, metres. Every repeat in the scene is snapped to divide it.
    pub loop_length: f32,
    /// Walking speed for previews and timed exports, metres per second.
    pub speed: f32,
    /// Frames per second for timed exports.
    pub fps: u32,
}
impl Default for Motion {
    fn default() -> Self { Self { loop_length: 24.0, speed: 4.0, fps: 24 } }
}

impl Motion {
    pub fn loop_seconds(&self) -> f32 { self.loop_length.max(0.1) / self.speed.max(0.01) }
    /// Frames in one loop at `fps`, at least 2.
    pub fn frames(&self) -> u32 { ((self.loop_seconds() * self.fps.max(1) as f32).round() as u32).max(2) }
}

impl Scene {
    /// Parse a scene from JSON, accepting v3 files and older PathForge 2.0 settings files.
    pub fn from_json(text: &str) -> Result<Scene, String> {
        let mut v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if v.get("version").and_then(|x| x.as_u64()).unwrap_or(0) >= 3 {
            fill_kind_defaults(&mut v);
            serde_json::from_value(v).map_err(|e| e.to_string())
        } else {
            let old: crate::settings::PathForgeSettings = serde_json::from_value(v).map_err(|e| format!("not a v3 scene or a 2.0 settings file: {e}"))?;
            Ok(migrate::from_v2(&old, "Imported"))
        }
    }
}

/// Fill in what a scene file leaves to the kind of a thing: a prop layer that names its `kind` but
/// no `tint` takes that kind's colour (a struct-wide default cannot, since it does not know the kind).
pub fn fill_kind_defaults(v: &mut serde_json::Value) {
    if let Some(props) = v.get_mut("props").and_then(|p| p.as_array_mut()) {
        for p in props.iter_mut().filter_map(|p| p.as_object_mut()) {
            if p.contains_key("tint") { continue; }
            let Some(kind) = p.get("kind").and_then(|k| serde_json::from_value::<PropKind>(k.clone()).ok()) else { continue };
            p.insert("tint".into(), serde_json::json!(kind.default_tint()));
        }
    }
}

/// Short stable fingerprint of a file's text (FNV-1a), for "changed since I read it" checks.
pub fn revision(text: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.as_bytes() { h ^= *b as u64; h = h.wrapping_mul(0x100_0000_01b3); }
    format!("{h:016x}")
}

/// Read a scene file; returns the scene and the revision of the text read.
pub fn load_file(path: &std::path::Path) -> Result<(Scene, String), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let scene = Scene::from_json(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok((scene, revision(&text)))
}

/// Write a scene file atomically (temporary file, then rename); returns the new revision.
pub fn save_file(path: &std::path::Path, scene: &Scene) -> Result<String, String> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let text = serde_json::to_string_pretty(scene).map_err(|e| e.to_string())? + "\n";
    let tmp = path.with_extension("json.part");
    std::fs::write(&tmp, &text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(revision(&text))
}
