//! Scene format v3: one metric world model shared by the renderer, the editor, exporters and MCP.
//!
//! Units are metres and seconds. The camera stands on the path and looks down it; the world
//! scrolls toward the camera by `distance` metres. Everything that repeats (texture tiles,
//! fixture and prop spacing, particle fields, sky motion) is snapped so a whole number of
//! repeats fits in `motion.loop_length`, which makes every loop seamless by construction.

pub mod presets;
pub mod form;
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
    /// What grows on the ground: grass, tall grass and wildflowers standing on it, moss lying on
    /// it; over the verge, the path or both. Up to three layers.
    pub ground_cover: Vec<GroundCover>,
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
    /// The path winding left and right as it goes, over and over.
    pub winding: Winding,
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
    /// How much of the water is frozen over (0..1): ice grows out from the banks, leaving a
    /// narrowing channel of open water down the middle, until at 1 it is ice from bank to bank.
    /// The ice is cracked, with snow lying on it in patches, and takes `foam_color`.
    pub frozen: f32,
    pub seed: u32,
}
impl Default for Waterway {
    fn default() -> Self {
        Waterway {
            enabled: false, flow: 2, foam: 0.5, foam_color: [214, 226, 224], foam_width: 0.35, lap: 3, wet: 0.25,
            floating: Floating::None, float_density: 0.3, float_color: [70, 110, 52], float_size: 0.5, frozen: 0.0, seed: 0,
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
    /// Ice floes: flat straight-sided plates, snow on top and bare blue ice round the rim.
    Ice,
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

/// A path that winds: it swings to one side and back to the other as it goes, and everything
/// beside it (walls, verge, props, lights) follows. The bends come towards the camera as it
/// walks; a whole number of them fits the loop.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Winding {
    pub enabled: bool,
    /// How far the path swings to each side, metres.
    pub amplitude: f32,
    /// Distance from one bend to the next the same way, metres (snapped so the loop closes).
    pub length: f32,
    /// How much the view turns with the path (0..1). At 1 the camera always looks along the
    /// path under it, so the bends ahead swing well across the frame; at 0 it looks straight
    /// on and the path slides from side to side under it. The sky turns with the camera: the
    /// sun, moon, stars, clouds and aurora slide across the frame by as much as it has turned
    /// (and out of it on a sharp bend), and the sun's and moon's light swings round with them.
    pub turn: f32,
    /// How uneven the bends are (0..1): 0 an even wave, more makes some sharper, some gentler.
    pub irregular: f32,
    /// Where in a bend the loop starts (0..1 of a whole bend).
    pub phase: f32,
}
impl Default for Winding {
    fn default() -> Self { Winding { enabled: false, amplitude: 2.0, length: 40.0, turn: 0.6, irregular: 0.3, phase: 0.0 } }
}

impl Default for PathShape {
    fn default() -> Self {
        Self {
            half_width: 1.1, flare: 0.0, bend: 0.0, winding: Winding::default(), hill: 0.0, edge_noise: 0.12, edge_dark: 0.35,
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

/// What a layer of ground cover is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum CoverKind {
    /// Short grass: a lawn, a mown verge.
    #[default]
    Grass,
    /// Long grass in tall clumps: a meadow, a field gone wild.
    TallGrass,
    /// Flowers on thin stems, in the flower colour, a related one and white.
    Wildflowers,
    /// Moss lying on the ground in soft cushions (no height).
    Moss,
}

/// Where a layer of ground cover grows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum CoverOn {
    #[default]
    Verge,
    Path,
    Both,
}

/// One layer of ground cover. It stands on the ground itself (not at the path's edge only, as
/// `verge.tufts` do): each blade has its place, hides what is behind it and leans in the wind.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct GroundCover {
    pub enabled: bool,
    pub kind: CoverKind,
    pub on: CoverOn,
    /// How much of the ground it takes (0..1).
    pub density: f32,
    /// How tall it stands, metres (grass about 0.12, tall grass 0.45, flowers 0.3; not moss).
    pub height: f32,
    /// Its colour: blades and stems, or the moss.
    pub color: Rgb,
    /// Grass: the colour of the tips. Wildflowers: the flowers. Moss: its second shade.
    pub color2: Rgb,
    /// 0 grows evenly; towards 1 it grows in patches with bare ground between.
    pub patchiness: f32,
    pub seed: u32,
}
impl Default for GroundCover {
    fn default() -> Self {
        GroundCover { enabled: true, kind: CoverKind::Grass, on: CoverOn::Verge, density: 0.8, height: 0.12, color: [40, 92, 30], color2: [112, 150, 52], patchiness: 0.2, seed: 0 }
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
    /// More layers of cloud over `clouds`, drawn in order (up to three): cirrus high over
    /// cumulus, heaps under an overcast. Give layers `patches` to share the sky between kinds.
    pub cloud_layers: Vec<Clouds>,
    /// Clouds of several kinds chosen at random from a seed, added over the layers above.
    pub cloud_mix: CloudMix,
    /// A sunset (or sunrise): the sky, a glow along the horizon, the clouds and the light all
    /// take the colours of a low sun.
    pub sunset: Sunset,
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
            sun: SkyBody::default(), moon: Moon::default(), stars: Stars::default(), clouds: Clouds::default(), cloud_layers: Vec::new(),
            cloud_mix: CloudMix::default(), sunset: Sunset::default(),
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
        Self { enabled: false, pos: [0.72, 0.35], radius: 0.045, color: [255, 236, 190], emits_light: true, intensity: 1.0 }
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

/// What kind of cloud a layer is. Whatever the kind, cloud that rain or snow is falling from
/// (`weather.precipitation`) is drawn heavier and darker.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
pub enum CloudKind {
    /// Fair-weather heaps standing in rows on level bases: bright billowing tops, grey beneath.
    #[default]
    Cumulus,
    /// Tall heaps building up from the horizon, dark under them: the sky before a storm.
    Towering,
    /// Long low bands lying along the horizon.
    Stratus,
    /// A high sheet broken into ripples and small flakes: a mackerel sky.
    Mackerel,
    /// Thin bright streaks very high up, drawn out towards the horizon.
    Cirrus,
    /// A grey deck over the whole sky, lighter and darker in patches.
    Overcast,
    /// Rain cloud: a low dark deck, heavy-bellied and ragged underneath.
    Rain,
    /// Snow cloud: an even, pale, heavy sky with hardly a feature in it.
    Snow,
    /// Storm cloud: a black deck with ragged scud driving along under it.
    Storm,
    /// Soft round puffs with no detail, as PathForge drew every cloud before; suits flat and
    /// pixel-art styles. (The main layer only: in `cloud_layers` it reads as cumulus.)
    Soft,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Clouds {
    pub enabled: bool,
    pub kind: CloudKind,
    /// How much of the sky they fill (0..1). (`Soft` clouds are counted instead: `count`.)
    pub cover: f32,
    /// How many `Soft` clouds.
    pub count: u32,
    /// Drifts across the sky per loop. Whole numbers wrap each cloud round the sky; any other
    /// value (0.4, 1.5) keeps its exact speed, with each cloud forming and dissolving over one
    /// loop so the loop stays seamless.
    pub drift: f32,
    pub scale: f32,
    pub opacity: f32,
    pub tint: Rgb,
    /// How ragged and billowing their outlines are (0..1).
    pub variation: f32,
    /// How dark their thick parts and undersides are (0..1).
    pub shade: f32,
    /// Shadows of clouds drifting over the ground (0..1; needs a light-giving sun). They drift
    /// with the wind when there is one, and work with the clouds themselves switched off too.
    pub shadows: f32,
    /// How much the layer gathers into patches with open sky between them (0..1): 0 spreads it
    /// over the whole sky, 1 keeps it to a few parts of it. Layers of different kinds with
    /// patches share the sky: heaps over here, a mackerel sky over there.
    pub patches: f32,
    pub seed: u32,
}
impl Default for Clouds {
    fn default() -> Self {
        Self { enabled: false, kind: CloudKind::Cumulus, cover: 0.45, count: 10, drift: 1.0, scale: 1.0, opacity: 0.5, tint: [226, 230, 238], variation: 0.55, shade: 0.55, shadows: 0.0, patches: 0.0, seed: 0 }
    }
}

/// Layers of cloud of several kinds, chosen at random from `seed`: which kinds, how much of the
/// sky each covers, their size, raggedness and shade. The same seed always gives the same sky;
/// change it for another. They are drawn over `clouds` and `cloud_layers`, highest first.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct CloudMix {
    pub enabled: bool,
    /// How many layers to make (1..4).
    pub layers: u32,
    /// The kinds to choose from. None ticked: fair-weather kinds (cumulus, towering, stratus,
    /// mackerel, cirrus). `Soft` reads as cumulus.
    pub kinds: Vec<CloudKind>,
    /// How much of the sky they fill, roughly (0..1); each layer varies round it.
    pub cover: f32,
    /// How much each layer keeps to its own parts of the sky (0..1; see `Clouds::patches`).
    pub patches: f32,
    /// How much the layers differ from each other in size, cover, raggedness and shade (0..1).
    pub variety: f32,
    /// Drifts across the sky per loop, as `Clouds::drift`; higher layers drift more slowly.
    pub drift: f32,
    pub opacity: f32,
    pub tint: Rgb,
    pub seed: u32,
}
impl Default for CloudMix {
    fn default() -> Self {
        Self { enabled: false, layers: 3, kinds: Vec::new(), cover: 0.45, patches: 0.5, variety: 0.6, drift: 1.0, opacity: 0.5, tint: [226, 230, 238], seed: 0 }
    }
}

impl CloudMix {
    /// The layers the mix makes, in drawing order (highest first).
    pub fn layers(&self) -> Vec<Clouds> {
        if !self.enabled || self.layers == 0 { return Vec::new(); }
        let fair = [CloudKind::Cumulus, CloudKind::Towering, CloudKind::Stratus, CloudKind::Mackerel, CloudKind::Cirrus];
        let pool: Vec<CloudKind> = if self.kinds.is_empty() { fair.to_vec() } else { self.kinds.clone() };
        // A small hash of (seed, layer, salt) to 0..1.
        let h = |i: u32, salt: u32| {
            let mut x = self.seed.wrapping_mul(0x9E37_79B9) ^ i.wrapping_mul(0x85EB_CA6B) ^ salt.wrapping_mul(0xC2B2_AE35);
            x ^= x >> 16; x = x.wrapping_mul(0x7FEB_352D); x ^= x >> 15; x = x.wrapping_mul(0x846C_A68B); x ^= x >> 16;
            x as f32 / u32::MAX as f32
        };
        let n = self.layers.clamp(1, 4);
        let var = self.variety.clamp(0.0, 1.0);
        // Each kind once before any comes again, in an order the seed shuffles.
        let mut order: Vec<usize> = (0..pool.len()).collect();
        order.sort_by(|&a, &b| h(a as u32, 0xA1).total_cmp(&h(b as u32, 0xA1)));
        let mut out: Vec<Clouds> = (0..n).map(|i| {
            let kind = pool[order[i as usize % order.len()]];
            let spread = |salt: u32| 1.0 + var * (h(i, salt) - 0.5);
            Clouds {
                enabled: true, kind,
                cover: (self.cover * spread(0xB2) * 1.1).clamp(0.05, 1.0),
                drift: self.drift * if matches!(kind, CloudKind::Cirrus | CloudKind::Mackerel) { 0.5 } else { 1.0 },
                scale: (spread(0xC3) * spread(0xC4)).clamp(0.4, 2.5),
                opacity: (self.opacity * (1.0 + 0.4 * var * (h(i, 0xD5) - 0.5))).clamp(0.0, 1.0),
                tint: self.tint,
                variation: (0.55 * spread(0xE6)).clamp(0.0, 1.0),
                shade: (0.55 * spread(0xF7)).clamp(0.0, 1.0),
                patches: self.patches.clamp(0.0, 1.0),
                seed: self.seed.wrapping_mul(31).wrapping_add(i * 7919 + 17),
                ..Clouds::default()
            }
        }).collect();
        // Highest first, so lower cloud is drawn over it.
        let height = |k: CloudKind| match k { CloudKind::Cirrus => 0, CloudKind::Mackerel => 1, CloudKind::Overcast | CloudKind::Snow => 2, CloudKind::Stratus | CloudKind::Rain | CloudKind::Storm => 3, _ => 4 };
        out.sort_by_key(|c| height(c.kind));
        out
    }
}

/// The colours of a sunset.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum SunsetStyle {
    /// Gold and amber low down under a clear blue sky, clouds gilded and peach.
    #[default]
    Golden,
    /// Blazing orange and red along the horizon, crimson and magenta clouds, a deep blue sky.
    Fiery,
    /// Soft peach and rose, a lavender sky, pink clouds: a hazy summer evening.
    Pastel,
    /// Just after the sun has gone: a thin red band at the horizon under violet and indigo,
    /// clouds dark with rose undersides.
    Dusk,
}

/// A sunset over the sky. `amount` takes the sky's colours, the sun, the clouds' tint and the
/// light towards those of the `style`; the glow lies along the horizon, brightest round the
/// sun's place across the sky (`sky.sun.pos`, even with the sun itself off, as at dusk); clouds
/// are lit from below, gold and orange near the horizon and the sun, pink higher up, violet
/// where they turn away. Place the sun low (`pos` y 0.8-1) for the best of it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Sunset {
    pub enabled: bool,
    pub style: SunsetStyle,
    /// How far the sky, sun, clouds and light go towards the sunset's colours (0..1).
    pub amount: f32,
    /// How bright the glow along the horizon is (0..2).
    pub glow: f32,
    /// How high the glow climbs, as a fraction of the sky (0.05..1).
    pub height: f32,
    /// How far round from the sun the glow reaches, in frame widths (0.1..3).
    pub spread: f32,
    /// How much the clouds take its colours (0..1).
    pub clouds: f32,
    /// How much the scene's light takes its colours: a warm low sun and a dusky fill (0..1).
    pub light: f32,
    /// Set on the copy of a scene whose colours have been taken towards the sunset's, so it is
    /// done once.
    #[serde(skip)]
    #[schemars(skip)]
    pub baked: bool,
}
impl Default for Sunset {
    fn default() -> Self {
        Self { enabled: false, style: SunsetStyle::Golden, amount: 1.0, glow: 1.0, height: 0.35, spread: 0.8, clouds: 1.0, light: 0.7, baked: false }
    }
}

/// A sunset style's colours (sRGB).
#[derive(Clone, Copy, Debug)]
pub struct SunsetColors {
    pub top: Rgb,
    pub horizon: Rgb,
    /// The glow at the horizon, a third of the way up it, and at its top.
    pub glow: [Rgb; 3],
    pub sun: Rgb,
    /// Clouds lit low down and near the sun, lit high up, and turned away from it.
    pub cloud_low: Rgb,
    pub cloud_high: Rgb,
    pub cloud_dark: Rgb,
    /// The light that fills the shadows, and how bright it is against the scene's.
    pub ambient: Rgb,
    pub ambient_k: f32,
}

impl SunsetStyle {
    pub fn colors(self) -> SunsetColors {
        match self {
            SunsetStyle::Golden => SunsetColors {
                top: [58, 96, 160], horizon: [255, 190, 120], glow: [[255, 200, 112], [255, 140, 76], [196, 120, 150]], sun: [255, 206, 140],
                cloud_low: [255, 192, 118], cloud_high: [250, 178, 160], cloud_dark: [104, 88, 124], ambient: [255, 204, 160], ambient_k: 0.9,
            },
            SunsetStyle::Fiery => SunsetColors {
                top: [34, 46, 108], horizon: [255, 128, 52], glow: [[255, 156, 54], [236, 72, 40], [168, 50, 96]], sun: [255, 168, 88],
                cloud_low: [255, 146, 64], cloud_high: [242, 112, 108], cloud_dark: [74, 44, 82], ambient: [255, 160, 120], ambient_k: 0.8,
            },
            SunsetStyle::Pastel => SunsetColors {
                top: [112, 124, 186], horizon: [255, 200, 172], glow: [[255, 214, 174], [252, 172, 180], [206, 164, 214]], sun: [255, 224, 196],
                cloud_low: [255, 204, 178], cloud_high: [244, 186, 206], cloud_dark: [146, 128, 168], ambient: [240, 206, 214], ambient_k: 0.95,
            },
            SunsetStyle::Dusk => SunsetColors {
                top: [22, 28, 68], horizon: [226, 112, 72], glow: [[238, 120, 58], [188, 66, 92], [96, 58, 124]], sun: [255, 140, 92],
                cloud_low: [196, 96, 80], cloud_high: [172, 100, 136], cloud_dark: [40, 34, 66], ambient: [150, 124, 168], ambient_k: 0.6,
            },
        }
    }
}

impl Sunset {
    /// Whether it changes anything.
    pub fn on(&self) -> bool { self.enabled && self.amount > 0.0 }

    /// A copy of `scene` with its colours taken towards the sunset's (sky, sun, cloud tints,
    /// light, fog and mist), or None when it has no sunset or this is already such a copy. What
    /// colours cannot say (the glow, clouds lit from below) the renderer draws from `sky.sunset`.
    pub fn bake(scene: &Scene) -> Option<Scene> {
        let ss = &scene.sky.sunset;
        if !ss.on() || ss.baked || !scene.sky.enabled { return None; }
        let mut s = scene.clone();
        let (a, pal) = (ss.amount.clamp(0.0, 1.0), ss.style.colors());
        let mix = |c: Rgb, to: Rgb, k: f32| -> Rgb { [0, 1, 2].map(|i| (c[i] as f32 + (to[i] as f32 - c[i] as f32) * k).round().clamp(0.0, 255.0) as u8) };
        let sky = &mut s.sky;
        sky.top = mix(sky.top, pal.top, a);
        sky.horizon = mix(sky.horizon, pal.horizon, a);
        sky.sun.color = mix(sky.sun.color, pal.sun, a);
        // The clouds' own colour goes warm and pale; the light on them comes from the renderer.
        let ck = a * ss.clouds.clamp(0.0, 1.0);
        let warm = mix([255, 255, 255], pal.cloud_low, 0.35);
        sky.clouds.tint = mix(sky.clouds.tint, warm, ck);
        for c in sky.cloud_layers.iter_mut() { c.tint = mix(c.tint, warm, ck); }
        sky.cloud_mix.tint = mix(sky.cloud_mix.tint, warm, ck);
        sky.sunset.baked = true;
        let lk = a * ss.light.clamp(0.0, 1.0);
        s.light.ambient_color = mix(s.light.ambient_color, pal.ambient, lk);
        s.light.ambient *= 1.0 + (pal.ambient_k - 1.0) * lk;
        if !s.light.fog.match_sky { s.light.fog.color = mix(s.light.fog.color, pal.horizon, 0.5 * lk); }
        s.weather.mist.color = mix(s.weather.mist.color, mix(pal.horizon, pal.ambient, 0.5), 0.6 * lk);
        Some(s)
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
pub enum FixtureKind { Torch, Lantern, Candle, Brazier, Crystal, Firefly, Magic, GreenFire, IceWisp, StreetLamp, PaperLantern, Campfire, Glowshroom, Beacon, Chandelier }

impl FixtureKind {
    pub const ALL: [FixtureKind; 15] = [
        FixtureKind::Torch, FixtureKind::Lantern, FixtureKind::Candle, FixtureKind::Brazier, FixtureKind::Crystal,
        FixtureKind::Firefly, FixtureKind::Magic, FixtureKind::GreenFire, FixtureKind::IceWisp,
        FixtureKind::StreetLamp, FixtureKind::PaperLantern, FixtureKind::Campfire, FixtureKind::Glowshroom, FixtureKind::Beacon,
        FixtureKind::Chandelier,
    ];
    pub fn name(self) -> &'static str {
        match self {
            FixtureKind::Torch => "Torch", FixtureKind::Lantern => "Lantern", FixtureKind::Candle => "Candle",
            FixtureKind::Brazier => "Brazier", FixtureKind::Crystal => "Crystal", FixtureKind::Firefly => "Firefly",
            FixtureKind::Magic => "Magic", FixtureKind::GreenFire => "Green Fire", FixtureKind::IceWisp => "Ice Wisp",
            FixtureKind::StreetLamp => "Street Lamp", FixtureKind::PaperLantern => "Paper Lantern", FixtureKind::Campfire => "Campfire",
            FixtureKind::Glowshroom => "Glowshroom", FixtureKind::Beacon => "Beacon", FixtureKind::Chandelier => "Chandelier",
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
            FixtureKind::StreetLamp => [255, 214, 150], FixtureKind::PaperLantern => [255, 120, 60],
            FixtureKind::Campfire => [255, 135, 45], FixtureKind::Glowshroom => [90, 240, 200],
            FixtureKind::Beacon => [255, 60, 40], FixtureKind::Chandelier => [255, 180, 90],
        }
    }
    /// Flame gradient from core to edge, or None for a soft orb.
    pub fn flame(self) -> Option<[Rgb; 4]> {
        match self {
            FixtureKind::Torch | FixtureKind::Brazier | FixtureKind::Campfire => Some([[255, 255, 210], [255, 205, 60], [255, 110, 15], [200, 50, 5]]),
            FixtureKind::Lantern => Some([[255, 255, 235], [255, 225, 150], [255, 185, 80], [215, 130, 30]]),
            FixtureKind::Candle | FixtureKind::Chandelier => Some([[255, 245, 200], [255, 190, 70], [255, 110, 25], [200, 55, 5]]),
            FixtureKind::Magic => Some([[230, 210, 255], [150, 100, 255], [90, 50, 230], [45, 15, 190]]),
            FixtureKind::GreenFire => Some([[220, 255, 210], [100, 235, 60], [25, 170, 15], [5, 95, 5]]),
            FixtureKind::IceWisp => Some([[225, 245, 255], [130, 210, 255], [50, 150, 230], [15, 85, 190]]),
            FixtureKind::Crystal | FixtureKind::Firefly | FixtureKind::StreetLamp | FixtureKind::PaperLantern | FixtureKind::Glowshroom | FixtureKind::Beacon => None,
        }
    }
    /// What holds it up where there is no wall to hang it on, when `post` is left to choose.
    pub fn natural_post(self) -> LampPost {
        match self {
            FixtureKind::StreetLamp => LampPost::Pole,
            FixtureKind::PaperLantern => LampPost::Crook,
            _ => LampPost::Pedestal,
        }
    }
}

/// What a light stands on or hangs from where there is no wall for it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum LampPost {
    /// The kind's own: a street lamp on a pole, a paper lantern from a crook, a wall light with no
    /// wall on a pole, anything else set above the ground on a stone pedestal.
    #[default]
    Auto,
    /// Nothing: the light hangs in the air.
    Off,
    /// A slim iron pole: under a lamp, or beside a torch or lantern at its bracket.
    Pole,
    /// A tall post with an arm curling over towards the path, the light hanging from its end.
    Crook,
    /// A stone pedestal under it.
    Pedestal,
}
impl LampPost {
    pub const ALL: [LampPost; 5] = [LampPost::Auto, LampPost::Off, LampPost::Pole, LampPost::Crook, LampPost::Pedestal];
    pub fn name(self) -> &'static str {
        match self { LampPost::Auto => "Auto", LampPost::Off => "Off", LampPost::Pole => "Pole", LampPost::Crook => "Crook", LampPost::Pedestal => "Pedestal" }
    }
}

/// Where along the path a row of lights is allowed to stand.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum LightPlace {
    #[default]
    Anywhere,
    /// Only on the path's bridges (`path.bridge`).
    Bridges,
    /// Everywhere but on the bridges.
    OffBridges,
    /// One at each repeat of a set piece (`set_piece` says which): torches on every arch, a lantern
    /// at every gate. The set piece's spacing replaces `spacing`, and `offset` is measured from it.
    SetPieces,
}
impl LightPlace {
    pub const ALL: [LightPlace; 4] = [LightPlace::Anywhere, LightPlace::Bridges, LightPlace::OffBridges, LightPlace::SetPieces];
    pub fn name(self) -> &'static str {
        match self { LightPlace::Anywhere => "Anywhere", LightPlace::Bridges => "Bridges", LightPlace::OffBridges => "Off Bridges", LightPlace::SetPieces => "Set Pieces" }
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
    /// How much the wind stirs leaves and hanging strands (0..1): a tree's crown ripples, a
    /// willow's strands swing and trail downwind, while trunks stay put.
    pub flutter: f32,
    pub seed: u32,
}
impl Default for Wind {
    fn default() -> Self { Wind { enabled: false, speed: 3.0, gusts: 0.4, sway: 0.5, flutter: 0.6, seed: 0 } }
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
    /// How it thins with height: 0 an even layer up to `height`, 1 thickest at the ground and
    /// thinning smoothly to nothing at `height` (ground fog, dew burning off in the morning sun).
    pub soft: f32,
    /// Where it lies: everywhere, or only over the path (a river's water, a road) and `spread`
    /// metres past its edges, fading out across them.
    pub over: MistOver,
    /// How far past the path's edges mist `over` the path reaches, metres.
    pub spread: f32,
    /// How much brighter it glows looking towards the sun, as mist does lit from behind (0..2).
    pub glow: f32,
}

/// Where mist lies.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum MistOver {
    #[default]
    Everywhere,
    /// Over the path only: the water of a waterway, or the road.
    Path,
}
impl Default for Mist {
    fn default() -> Self { Mist { enabled: false, height: 0.8, density: 0.25, color: [205, 210, 220], patchiness: 0.6, wisps: 0.4, seed: 0, soft: 0.0, over: MistOver::Everywhere, spread: 1.0, glow: 0.0 } }
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
    /// Rain on the glass, as a lens or a windscreen shows it: beads gathering all over it, heavy
    /// drops that land, hang and run down leaving wet trails, each drop holding the scene upside
    /// down, and the wet glass between them a little out of focus.
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
    /// How much rain is on the glass, or how far the frost reaches in (0..1).
    pub amount: f32,
    /// Size of the drops (1 = as on a camera lens; larger reads as a windscreen close to).
    pub size: f32,
    /// Share of the heavy drops that run down the glass and leave a trail (0..1).
    pub trails: f32,
    /// How far out of focus the wet glass is between the drops and their trails (0 = clear).
    pub blur: f32,
    pub seed: u32,
}
impl Default for Lens {
    fn default() -> Self { Lens { enabled: false, kind: LensKind::Drops, amount: 0.5, size: 1.0, trails: 0.6, blur: 0.35, seed: 0 } }
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
    /// Random placement wobble, metres: sideways by this much either way and up or down by half of it.
    /// Floating lights also drift about by it.
    pub jitter: f32,
    /// Random shift along the path, as a fraction of the spacing (0 = an even row, 1 = anywhere
    /// between its neighbours).
    pub jitter_along: f32,
    /// Random height, metres either way (on top of `jitter`'s): fireflies and wisps at every height.
    pub height_var: f32,
    /// Random size, as a fraction either way (0.3 = from 0.7 to 1.3 of `size`).
    pub size_var: f32,
    /// Random brightness, as a fraction either way.
    pub intensity_var: f32,
    /// Random colour, 0..1: each one a little warmer or cooler than the next.
    pub color_var: f32,
    /// Fraction of places that hold a light (1 = every place, 0.5 = half of them, at random).
    pub density: f32,
    /// How many stand at each place (1..8). They share the place's random settings, so with `jitter`,
    /// `jitter_along` and `height_var` they scatter into a loose swarm. Each is a light of its own:
    /// turn `light` off, or the spacing up, when there are many.
    pub cluster: u32,
    /// With lights on both sides: how far the right-hand row is shifted along the path, as a fraction
    /// of the spacing (0 = in pairs, 0.5 = left and right take turns).
    pub stagger: f32,
    /// Colour multiplied over the kind's own light and flame (white = as it is).
    pub tint: Rgb,
    /// What it stands on or hangs from where there is no wall (ground mounts, and wall mounts in a
    /// scene without walls).
    pub post: LampPost,
    /// Where along the path the row is allowed to stand.
    pub place: LightPlace,
    /// For `place: SetPieces`: which of `set_pieces` the lights go with (0 = the first).
    pub set_piece: u32,
    pub sprite: SpriteRef,
    pub seed: u32,
}
impl Default for Fixture {
    fn default() -> Self {
        Self {
            enabled: true, kind: FixtureKind::Torch, side: Side::Both, mount: Mount::Wall, height: 2.1, spacing: 6.0,
            offset: 0.0, lateral: 0.3, size: 1.0, light: true, intensity: 1.0, radius: 6.0, flicker: 1.0, jitter: 0.0,
            jitter_along: 0.0, height_var: 0.0, size_var: 0.0, intensity_var: 0.0, color_var: 0.0, density: 1.0, cluster: 1, stagger: 0.0,
            tint: [255, 255, 255], post: LampPost::Auto, place: LightPlace::Anywhere, set_piece: 0,
            sprite: SpriteRef::default(), seed: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum PropKind { Tree, Pine, Bush, Rock, Boulder, Cactus, DeadTree, Mushroom, Pillar, Gravestone, Crystal, Stalagmite, Reeds, Willow, Palm, Obelisk, Icicle, IceSpike, Asteroid, Satellite, Debris, CargoPod, Comet }

impl PropKind {
    pub const ALL: [PropKind; 23] = [
        PropKind::Tree, PropKind::Pine, PropKind::Bush, PropKind::Rock, PropKind::Boulder, PropKind::Cactus,
        PropKind::DeadTree, PropKind::Mushroom, PropKind::Pillar, PropKind::Gravestone, PropKind::Crystal,
        PropKind::Stalagmite, PropKind::Reeds, PropKind::Willow, PropKind::Palm, PropKind::Obelisk,
        PropKind::Icicle, PropKind::IceSpike,
        PropKind::Asteroid, PropKind::Satellite, PropKind::Debris, PropKind::CargoPod, PropKind::Comet,
    ];
    pub fn name(self) -> &'static str {
        match self {
            PropKind::Tree => "Tree", PropKind::Pine => "Pine", PropKind::Bush => "Bush", PropKind::Rock => "Rock",
            PropKind::Boulder => "Boulder", PropKind::Cactus => "Cactus", PropKind::DeadTree => "Dead Tree",
            PropKind::Mushroom => "Mushroom", PropKind::Pillar => "Pillar", PropKind::Gravestone => "Gravestone",
            PropKind::Crystal => "Crystal", PropKind::Stalagmite => "Stalagmite", PropKind::Reeds => "Reeds",
            PropKind::Willow => "Willow", PropKind::Palm => "Palm", PropKind::Obelisk => "Obelisk",
            PropKind::Icicle => "Icicle", PropKind::IceSpike => "Ice Spike",
            PropKind::Asteroid => "Asteroid", PropKind::Satellite => "Satellite", PropKind::Debris => "Debris",
            PropKind::CargoPod => "Cargo Pod", PropKind::Comet => "Comet",
        }
    }
    /// Hangs from the ceiling (or the top of the walls) instead of standing on the ground.
    pub fn hangs(self) -> bool { self == PropKind::Icicle }
    /// How much it lights itself (crystals and ice glow faintly).
    pub fn glow(self) -> f32 { match self { PropKind::Crystal => 0.6, PropKind::IceSpike | PropKind::Icicle => 0.12, PropKind::Comet => 0.45, _ => 0.0 } }
    /// Things of open space (set `float` to hang them in it, or `orbit` to send them round the path).
    pub fn of_space(self) -> bool { matches!(self, PropKind::Asteroid | PropKind::Satellite | PropKind::Debris | PropKind::CargoPod | PropKind::Comet) }
    /// Natural height in metres at scale 1.
    pub fn height_m(self) -> f32 {
        match self {
            PropKind::Tree => 6.0, PropKind::Pine => 7.0, PropKind::Bush => 1.1, PropKind::Rock => 0.6,
            PropKind::Boulder => 1.6, PropKind::Cactus => 2.6, PropKind::DeadTree => 5.0, PropKind::Mushroom => 0.7,
            PropKind::Pillar => 4.0, PropKind::Gravestone => 1.0, PropKind::Crystal => 1.3, PropKind::Stalagmite => 1.8,
            PropKind::Reeds => 1.5, PropKind::Willow => 6.5, PropKind::Palm => 7.0, PropKind::Obelisk => 5.0,
            PropKind::Icicle => 1.1, PropKind::IceSpike => 1.7,
            PropKind::Asteroid => 1.6, PropKind::Satellite => 2.0, PropKind::Debris => 1.4, PropKind::CargoPod => 1.2, PropKind::Comet => 1.5,
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
            PropKind::Asteroid => [128, 118, 108], PropKind::Satellite => [170, 124, 52], PropKind::Debris => [112, 120, 134],
            PropKind::CargoPod => [150, 108, 64], PropKind::Comet => [190, 220, 255],
        }
    }
}

/// Props set round the path instead of beside it: each stands on a ring about the path's line, the
/// row winding round it like the thread of a screw, the whole of it turning - rocks and wreckage
/// spiralling down a wormhole's throat. `side: Both` makes two strands opposite each other, `rows`
/// more strands between; `spacing`, `density` and the rest work as they do for a row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Orbit {
    pub enabled: bool,
    /// The ring's radius, metres.
    pub radius: f32,
    /// Random spread of the radius, metres either way.
    pub radius_var: f32,
    /// Height of the ring's centre above the path, metres (0 = the camera's eye height, which is the
    /// middle of a tunnel).
    pub height: f32,
    /// Whole turns each strand winds round the path along one loop length.
    pub twist: i32,
    /// Whole turns the ring makes in one loop.
    pub spin: i32,
    /// How far each one strays from its strand, in turns either way (0 = a clean spiral, 0.5 = anywhere round the ring).
    pub stray: f32,
    /// Take `twist` and `spin` from `sky.tunnel`, so the props wind and turn with the wormhole.
    pub with_tunnel: bool,
}
impl Default for Orbit {
    fn default() -> Self { Orbit { enabled: false, radius: 4.0, radius_var: 0.8, height: 0.0, twist: 1, spin: 1, stray: 0.08, with_tunnel: true } }
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
    /// Random shift along the path, as a fraction of the spacing (0 = an even row, 1 = anywhere
    /// between its neighbours). Rows beyond the first are always shifted by at least 0.8.
    pub jitter_along: f32,
    /// With props on both sides: how far the right-hand side is shifted along the path, as a
    /// fraction of the spacing (0 = in pairs, 0.5 = left and right take turns).
    pub stagger: f32,
    /// Share of slots that hold a prop (0..1), for natural gaps.
    pub density: f32,
    pub scale: f32,
    pub scale_var: f32,
    /// How far each one leans, at random to either side: its top moves by up to this fraction of its
    /// height, the trunk bowing over (0.04 is a natural stand of trees; rocks and pillars want 0).
    pub lean_var: f32,
    /// How much each one's colour differs from its neighbours' (0 all alike, 0.12 natural, 0.4
    /// strongly): some lighter or darker, some a little warmer or cooler.
    pub tint_var: f32,
    /// Colour of procedural props. Left out of a scene file, it is the kind's own colour.
    pub tint: Rgb,
    /// How far the base sinks into the ground, as a fraction of height.
    pub sink: f32,
    /// Floating props (asteroids, drifting islands, lanterns): height above the ground, metres,
    /// and a random spread either way. Floating props hang over a bridge's drop too, and cast no
    /// sun shadow.
    pub float: f32,
    pub float_var: f32,
    /// Set round the path in a ring instead of in rows beside it.
    pub orbit: Orbit,
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
            row_spacing: 3.0, jitter: 0.5, jitter_along: 0.0, stagger: 0.0, density: 1.0, scale: 1.0, scale_var: 0.2, lean_var: 0.0, tint_var: 0.12, tint: PropKind::Tree.default_tint(),
            sink: 0.02, float: 0.0, float_var: 0.0, orbit: Orbit::default(), shadow: true, shadow_opacity: 0.6, sprite: SpriteRef::default(), seed: 1, def: String::new(),
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
    /// Glow around bright light, as a lens scatters it: a tight glow round every flame, the sun,
    /// a glint or a lit panel, falling off into a wide soft veil (0..1; in the world only light
    /// brighter than white paper spreads, as exposed, so raising `exposure` makes more of it
    /// glow). Under a dark sky the moon, an aurora and the brighter stars glow too.
    pub bloom: f32,
    pub vignette: f32,
    pub grain: f32,
    /// Colour multiplied over the final image.
    pub tint: Rgb,
    /// What bright light does in the lens besides bloom.
    pub flare: Flare,
}

/// Lens flare from the sun and bright lights.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Flare {
    pub style: FlareStyle,
    /// Strength (0..2).
    pub amount: f32,
}
impl Default for Flare {
    fn default() -> Self { Self { style: FlareStyle::None, amount: 0.5 } }
}

/// The look of a lens flare.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum FlareStyle {
    #[default]
    None,
    /// Four fine rays from every bright light, as through a star filter or a stopped-down lens.
    Star,
    /// Six rays.
    Starburst,
    /// A long horizontal blue streak, as an anamorphic cinema lens makes.
    Anamorphic,
    /// Faint coloured reflections of the light across the frame from it, and a wide ring.
    Ghosts,
    /// The streak and the ghosts together.
    Cinematic,
}
impl FlareStyle {
    /// (rays or the streak, ghosts): which parts this style draws; rays 0 none, 4, 6, or 1 the streak.
    pub fn parts(self) -> (u32, bool) {
        match self {
            FlareStyle::None => (0, false), FlareStyle::Star => (4, false), FlareStyle::Starburst => (6, false),
            FlareStyle::Anamorphic => (1, false), FlareStyle::Ghosts => (0, true), FlareStyle::Cinematic => (1, true),
        }
    }
}
impl Default for Post {
    fn default() -> Self {
        Self { exposure: 1.0, contrast: 1.0, saturation: 1.0, bloom: 0.35, vignette: 0.35, grain: 0.0, tint: [255, 255, 255], flare: Flare::default() }
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
