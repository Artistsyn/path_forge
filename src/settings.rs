use serde::{Deserialize, Serialize};

// ── Canvas dimensions (defaults; runtime uses CanvasSettings) ──────────────
pub const CANVAS_W: usize = 480;
pub const CANVAS_H: usize = 854;
pub const TILE: usize = 32;
pub const CX: usize = CANVAS_W / 2;

/// Runtime canvas size.  Landscape mode swaps w/h.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CanvasSettings {
    pub base_w:    u32,   // portrait width  (default 480)
    pub base_h:    u32,   // portrait height (default 854)
    pub landscape: bool,
}
impl CanvasSettings {
    pub fn w(&self) -> usize { if self.landscape { self.base_h as usize } else { self.base_w as usize } }
    pub fn h(&self) -> usize { if self.landscape { self.base_w as usize } else { self.base_h as usize } }
    pub fn cx(&self) -> f32  { self.w() as f32 * 0.5 }
}
impl Default for CanvasSettings {
    fn default() -> Self { Self { base_w: 480, base_h: 854, landscape: false } }
}

// ── Colour helpers ─────────────────────────────────────────────────────────
pub fn hex_to_rgb(hex: &str) -> [u8; 3] {
    let h = hex.trim_start_matches('#');
    let v = u32::from_str_radix(h, 16).unwrap_or(0);
    [((v >> 16) & 0xff) as u8, ((v >> 8) & 0xff) as u8, (v & 0xff) as u8]
}

// ── Enumerations ───────────────────────────────────────────────────────────
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum FloorPattern { Cobblestone, Brick, StoneBlock, Sand, Dirt, Grass }

impl FloorPattern {
    pub fn all() -> &'static [FloorPattern] {
        &[Self::Cobblestone, Self::Brick, Self::StoneBlock, Self::Sand, Self::Dirt, Self::Grass]
    }
    pub fn name(&self) -> &'static str {
        match self {
            Self::Cobblestone => "Cobblestone",
            Self::Brick       => "Brick",
            Self::StoneBlock  => "Stone Block",
            Self::Sand        => "Sand",
            Self::Dirt        => "Dirt",
            Self::Grass       => "Grass",
        }
    }
    pub fn gen_seed_offset(&self) -> u32 {
        match self { Self::Cobblestone => 0, Self::Brick => 1,
            Self::StoneBlock => 2, Self::Sand => 3, Self::Dirt => 4, Self::Grass => 5 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum WallPattern { StoneBlock, Brick, Bark, RockFace, Hedge, Cobblestone }

impl WallPattern {
    pub fn all() -> &'static [WallPattern] {
        &[Self::StoneBlock, Self::Brick, Self::Bark, Self::RockFace, Self::Hedge, Self::Cobblestone]
    }
    pub fn name(&self) -> &'static str {
        match self {
            Self::StoneBlock  => "Stone Block",
            Self::Brick       => "Brick",
            Self::Bark        => "Bark",
            Self::RockFace    => "Rock Face",
            Self::Hedge       => "Hedge",
            Self::Cobblestone => "Cobblestone",
        }
    }
    pub fn gen_seed_offset(&self) -> u32 {
        match self { Self::StoneBlock => 0, Self::Brick => 1, Self::Bark => 2,
            Self::RockFace => 3, Self::Hedge => 4, Self::Cobblestone => 5 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum AtmoType { None, Torch, Lantern, Firefly, Magic, GreenFire, Candle, IceWisp }

impl AtmoType {
    pub fn all() -> &'static [AtmoType] {
        &[Self::None, Self::Torch, Self::Lantern, Self::Firefly,
          Self::Magic, Self::GreenFire, Self::Candle, Self::IceWisp]
    }
    pub fn name(&self) -> &'static str {
        match self {
            Self::None      => "None",
            Self::Torch     => "Torch",
            Self::Lantern   => "Lantern",
            Self::Firefly   => "Firefly",
            Self::Magic     => "Magic",
            Self::GreenFire => "Green Fire",
            Self::Candle    => "Candle",
            Self::IceWisp   => "Ice Wisp",
        }
    }
    pub fn glow_color(&self) -> Option<[u8; 3]> {
        match self {
            Self::None      => None,
            Self::Torch     => Some([215, 100,  10]),
            Self::Lantern   => Some([220, 185, 100]),
            Self::Firefly   => Some([ 70, 220,  50]),
            Self::Magic     => Some([100,  50, 255]),
            Self::GreenFire => Some([ 30, 180,  20]),
            Self::Candle    => Some([255, 160,  40]),
            Self::IceWisp   => Some([ 80, 200, 255]),
        }
    }
    pub fn flame_colors(&self) -> Option<[[u8; 3]; 4]> {
        match self {
            Self::Torch     => Some([[255,255,200],[255,200, 35],[255,105, 10],[210, 55,  5]]),
            Self::Lantern   => Some([[255,255,230],[255,225,150],[255,185, 80],[215,130, 30]]),
            Self::Magic     => Some([[200,180,255],[130, 80,255],[ 80, 40,220],[ 40, 10,180]]),
            Self::GreenFire => Some([[200,255,200],[ 80,220, 50],[ 20,160, 10],[  5, 90,  5]]),
            Self::Candle    => Some([[255,240,190],[255,180, 60],[255,100, 20],[200, 50,  5]]),
            Self::IceWisp   => Some([[200,240,255],[100,200,255],[ 40,140,220],[ 10, 80,180]]),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum AttachmentSurface { Floating, Wall, Floor, Ceiling }

impl AttachmentSurface {
    pub fn all() -> &'static [AttachmentSurface] {
        &[Self::Floating, Self::Wall, Self::Floor, Self::Ceiling]
    }
    pub fn name(&self) -> &'static str {
        match self {
            Self::Floating => "Floating",
            Self::Wall => "Wall",
            Self::Floor => "Floor",
            Self::Ceiling => "Ceiling",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum MountSide { Both, Left, Right, Center }

impl MountSide {
    pub fn all() -> &'static [MountSide] {
        &[Self::Both, Self::Left, Self::Right, Self::Center]
    }
    pub fn name(&self) -> &'static str {
        match self {
            Self::Both => "Both",
            Self::Left => "Left",
            Self::Right => "Right",
            Self::Center => "Center",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum LightingPreset {
    Balanced,
    GoldenHour,
    HighNoon,
    NightNeon,
}

impl LightingPreset {
    pub fn all() -> &'static [LightingPreset] {
        &[Self::Balanced, Self::GoldenHour, Self::HighNoon, Self::NightNeon]
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Balanced => "Balanced",
            Self::GoldenHour => "Golden Hour",
            Self::HighNoon => "High Noon",
            Self::NightNeon => "Night Neon",
        }
    }
}

fn default_lighting_preset() -> LightingPreset { LightingPreset::Balanced }
fn default_atmo_light_influence() -> f32 { 0.35 }
fn default_atmo_tint_influence() -> f32 { 0.28 }

// ── Settings structs ───────────────────────────────────────────────────────
#[derive(Clone, Serialize, Deserialize)]
pub struct SceneSettings {
    pub horizon_y:    u32,     // 80..260 (scales with 576×768 default)
    #[serde(default = "default_horizon_curve")]
    pub horizon_curve: f32,    // -1.0..1.0 vertical lens-like world bend
    #[serde(default = "default_curve_top_weight")]
    pub curve_top_weight: f32, // 0.0..2.0 top perspective bend weight
    #[serde(default = "default_curve_bottom_weight")]
    pub curve_bottom_weight: f32, // 0.0..2.0 bottom perspective bend weight
    pub max_hw:       f32,     // 30..176
    pub cam_h:        f32,     // 0.5..4.0
    pub focal_mult:   f32,     // 0.5..2.0
    pub path_power:   f32,     // 0.25..2.5
    pub void_color:   [u8; 3],
    #[serde(default)]
    pub grass_enabled: bool,
    #[serde(default = "default_grass_color")]
    pub grass_color:  [u8; 3],
    #[serde(default = "default_grass_density")]
    pub grass_density: f32,
    #[serde(default = "default_grass_height")]
    pub grass_height: f32,
    #[serde(default = "default_grass_upright")]
    pub grass_upright: f32,
    #[serde(default = "default_ambient")]
    pub ambient:      f32,     // 0.0..1.0  (floor/wall base brightness scale)
    #[serde(default = "default_lighting_preset")]
    pub lighting_preset: LightingPreset,
    #[serde(default = "default_atmo_light_influence")]
    pub atmo_light_influence: f32,
    #[serde(default = "default_atmo_tint_influence")]
    pub atmo_tint_influence: f32,
}
fn default_grass_color() -> [u8; 3] { [28, 90, 18] }
fn default_grass_density() -> f32 { 1.0 }
fn default_grass_height() -> f32 { 1.0 }
fn default_grass_upright() -> f32 { 0.8 }
fn default_ambient() -> f32 { 1.0 }
fn default_horizon_curve() -> f32 { 0.0 }
fn default_curve_top_weight() -> f32 { 1.0 }
fn default_curve_bottom_weight() -> f32 { 1.0 }

#[derive(Clone, Serialize, Deserialize)]
pub struct FloorSettings {
    pub depth_fade:    f32,     // 0.5..10.0
    pub edge_vignette: f32,     // 0.0..1.0
    pub noise:         u32,     // 0..30
    #[serde(default = "default_tex_scale")]
    pub tex_scale:     f32,     // 0.25..8.0 (higher = denser/smaller material detail)
    #[serde(default = "default_tex_rot_90")]
    pub tex_rot_90:    bool,    // rotate material mapping by 90deg
    #[serde(default = "default_damage")]
    pub damage:        f32,     // 0.0..1.0 random cracks/chips amount
    #[serde(default)]
    pub variation_seed:u32,     // material variation seed
    pub pattern:       FloorPattern,
    pub base:          [u8; 3],
    pub mortar:        [u8; 3],
}
fn default_tex_scale() -> f32 { 3.0 }
fn default_tex_rot_90() -> bool { false }
fn default_damage() -> f32 { 0.24 }

#[derive(Clone, Serialize, Deserialize)]
pub struct WallSettings {
    #[serde(default = "bool_true")]
    pub enabled:     bool,
    #[serde(default = "default_wall_top_coverage")]
    pub top_coverage: f32,  // 0.0..1.0 (1.0 = walls extend to top of frame)
    pub l_wx:        f32,   // 0.3..2.5 — wall distance from centre
    pub bright:      f32,   // 0.4..5.0
    pub junc_shadow: f32,   // 2..60
    pub fade_rows:   u32,   // 4..60
    pub noise:       u32,   // 0..25
    #[serde(default = "default_tex_scale")]
    pub tex_scale:   f32,   // 0.25..8.0 (higher = denser/smaller material detail)
    #[serde(default = "default_tex_rot_90")]
    pub tex_rot_90:  bool,  // rotate material mapping by 90deg
    #[serde(default = "default_damage")]
    pub damage:      f32,   // 0.0..1.0 random cracks/chips amount
    #[serde(default)]
    pub variation_seed: u32,
    pub pattern:     WallPattern,
    pub base:        [u8; 3],
    pub mortar:      [u8; 3],
}
fn bool_true() -> bool { true }
fn default_wall_top_coverage() -> f32 { 0.18 }

/// Sky gradient rendered above the horizon line (new in PathForge 2.0).
#[derive(Clone, Serialize, Deserialize)]
pub struct SkySettings {
    pub enabled:  bool,
    pub top:      [u8; 3],   // colour at top of screen
    pub horizon:  [u8; 3],   // colour at horizon
    #[serde(default)]
    pub sun_enabled: bool,
    #[serde(default = "bool_true")]
    pub sun_emits_light: bool,
    #[serde(default = "default_sun_pos")]
    pub sun_pos: [f32; 2],    // normalized x,y in sky area
    #[serde(default)]
    pub sun_z: f32,           // -1..1 horizon depth bias (positive = longer shadows)
    #[serde(default = "default_sun_radius")]
    pub sun_radius: f32,
    #[serde(default = "default_sun_color")]
    pub sun_color: [u8; 3],
    #[serde(default)]
    pub moon_enabled: bool,
    #[serde(default = "bool_true")]
    pub moon_emits_light: bool,
    #[serde(default = "default_moon_pos")]
    pub moon_pos: [f32; 2],
    #[serde(default)]
    pub moon_z: f32,          // -1..1 horizon depth bias (positive = longer shadows)
    #[serde(default = "default_moon_radius")]
    pub moon_radius: f32,
    #[serde(default = "default_moon_color")]
    pub moon_color: [u8; 3],
    #[serde(default = "default_moon_phase")]
    pub moon_phase: f32,      // -1..1 (negative=waning, positive=waxing)
    #[serde(default = "default_moon_alpha")]
    pub moon_alpha: f32,      // 0..1 moon transparency
    #[serde(default)]
    pub moon_texture_enabled: bool,
    #[serde(default = "default_moon_texture_scale")]
    pub moon_texture_scale: f32,
    #[serde(default)]
    pub stars_enabled: bool,
    #[serde(default = "default_stars_count")]
    pub stars_count: u32,
    #[serde(default)]
    pub stars_seed: u32,
    #[serde(default = "default_stars_size")]
    pub stars_size: f32,
    #[serde(default = "default_stars_twinkle")]
    pub stars_twinkle: f32,
    #[serde(default)]
    pub clouds_enabled: bool,
    #[serde(default = "default_cloud_count")]
    pub cloud_count: u32,
    #[serde(default)]
    pub cloud_seed: u32,
    #[serde(default = "default_cloud_speed")]
    pub cloud_speed: f32,
    #[serde(default = "default_cloud_scale")]
    pub cloud_scale: f32,
    #[serde(default = "default_cloud_opacity")]
    pub cloud_opacity: f32,
    #[serde(default = "default_cloud_tint")]
    pub cloud_tint: [u8; 3],
    #[serde(default = "default_cloud_variation")]
    pub cloud_variation: f32,
}
fn default_sun_pos() -> [f32; 2] { [0.78, 0.22] }
fn default_sun_radius() -> f32 { 0.09 }
fn default_sun_color() -> [u8; 3] { [255, 235, 180] }
fn default_moon_pos() -> [f32; 2] { [0.22, 0.25] }
fn default_moon_radius() -> f32 { 0.07 }
fn default_moon_color() -> [u8; 3] { [225, 232, 255] }
fn default_moon_phase() -> f32 { 0.0 }
fn default_moon_alpha() -> f32 { 0.88 }
fn default_moon_texture_scale() -> f32 { 1.0 }
fn default_stars_count() -> u32 { 120 }
fn default_stars_size() -> f32 { 1.4 }
fn default_stars_twinkle() -> f32 { 0.5 }
fn default_cloud_count() -> u32 { 16 }
fn default_cloud_speed() -> f32 { 0.35 }
fn default_cloud_scale() -> f32 { 1.0 }
fn default_cloud_opacity() -> f32 { 0.35 }
fn default_cloud_tint() -> [u8; 3] { [218, 224, 232] }
fn default_cloud_variation() -> f32 { 0.55 }
fn default_attachment_wall() -> AttachmentSurface { AttachmentSurface::Wall }
fn default_attachment_floating() -> AttachmentSurface { AttachmentSurface::Floating }
fn default_mount_side_both() -> MountSide { MountSide::Both }
fn default_mount_side_center() -> MountSide { MountSide::Center }

// ── Atmosphere layer (one entry in the Vec) ──────────────────────────────
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AtmoLayer {
    pub enabled:     bool,
    #[serde(default = "bool_true")]
    pub emits_light: bool,
    #[serde(default = "bool_true")]
    pub casts_shadow: bool,
    pub atmo_type:   AtmoType,
    pub torch_h:     f32,   // 0.3..5.0
    pub torch_spc:   u32,   // 1..8
    pub torch_scale: f32,   // 0.01..0.20
    pub n_motes:     u32,   // 0..25
    pub n_debris:    u32,   // 0..10
    #[serde(default = "default_atmo_jitter")]
    pub placement_jitter: f32, // 0..1 side-to-side/random depth variance
    #[serde(default = "default_atmo_flicker")]
    pub flicker:     f32,   // 0..2 flame pulse amount
    #[serde(default = "default_atmo_scale")]
    pub fx_scale:    f32,   // 0.2..3.0 glow/debris/motes scale
    #[serde(default = "default_attachment_wall")]
    pub mount_surface: AttachmentSurface,
    #[serde(default = "default_mount_side_both")]
    pub mount_side: MountSide,
    #[serde(default)]
    pub sprite_path: String,
    #[serde(default)]
    pub sprite_pool_paths: String,
    #[serde(default)]
    pub sprite_pool_enabled: bool,
    #[serde(default = "default_sprite_scale")]
    pub sprite_scale: f32,
    #[serde(default)]
    pub sprite_flip_x: bool,
    #[serde(default)]
    pub sprite_flip_y: bool,
    #[serde(default)]
    pub sprite_rot_deg: f32,
    #[serde(default)]
    pub sprite_offset_x: f32,
    #[serde(default)]
    pub sprite_offset_y: f32,
    #[serde(default)]
    pub variation_seed: u32,
}
fn default_atmo_jitter() -> f32 { 0.18 }
fn default_atmo_flicker() -> f32 { 1.0 }
fn default_atmo_scale() -> f32 { 1.0 }
fn default_sprite_scale() -> f32 { 1.0 }

impl AtmoLayer {
    pub fn new(t: AtmoType) -> Self {
        Self {
            enabled: true,
            emits_light: true,
            casts_shadow: true,
            atmo_type: t,
            torch_h: 2.4,
            torch_spc: 4,
            torch_scale: 0.068,
            n_motes: 8,
            n_debris: 4,
            placement_jitter: 0.18,
            flicker: 1.0,
            fx_scale: 1.0,
            mount_surface: AttachmentSurface::Wall,
            mount_side: MountSide::Both,
            sprite_path: String::new(),
            sprite_pool_paths: String::new(),
            sprite_pool_enabled: false,
            sprite_scale: 1.0,
            sprite_flip_x: false,
            sprite_flip_y: false,
            sprite_rot_deg: 0.0,
            sprite_offset_x: 0.0,
            sprite_offset_y: 0.0,
            variation_seed: 0,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct AtmoSettings {
    pub layers: Vec<AtmoLayer>,
}

impl Default for AtmoSettings {
    fn default() -> Self { Self { layers: vec![AtmoLayer::new(AtmoType::Torch)] } }
}

// ── Props ──────────────────────────────────────────────────────────────────
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PropType { Tree, PineTree, Bush, Rock, Boulder, Cactus, DeadTree, Mushroom }

impl PropType {
    pub fn all() -> &'static [PropType] {
        &[Self::Tree, Self::PineTree, Self::Bush, Self::Rock,
          Self::Boulder, Self::Cactus, Self::DeadTree, Self::Mushroom]
    }
    pub fn name(&self) -> &'static str {
        match self {
            Self::Tree     => "Tree",
            Self::PineTree => "Pine Tree",
            Self::Bush     => "Bush",
            Self::Rock     => "Rock",
            Self::Boulder  => "Boulder",
            Self::Cactus   => "Cactus",
            Self::DeadTree => "Dead Tree",
            Self::Mushroom => "Mushroom",
        }
    }
    pub fn default_tint(&self) -> [u8; 3] {
        match self {
            Self::Tree     => [25, 90, 18],
            Self::PineTree => [18, 70, 22],
            Self::Bush     => [22, 78, 16],
            Self::Rock     => [78, 72, 64],
            Self::Boulder  => [62, 58, 52],
            Self::Cactus   => [28, 95, 24],
            Self::DeadTree => [52, 42, 28],
            Self::Mushroom => [185, 38, 38],
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PropInstance {
    pub enabled:   bool,
    #[serde(default)]
    pub emits_light: bool,
    #[serde(default = "bool_true")]
    pub casts_shadow: bool,
    pub prop_type: PropType,
    #[serde(default)]
    pub pos_x:      f32,
    #[serde(default)]
    pub pos_y:      f32,
    #[serde(default)]
    pub pos_z:      f32,
    #[serde(default)]
    pub pixel_hitbox_enabled: bool,
    #[serde(default = "default_attachment_floating")]
    pub mount_surface: AttachmentSurface,
    #[serde(default = "default_mount_side_center")]
    pub mount_side: MountSide,
    pub wx:        f32,   // world-space lateral offset (positive = right)
    pub mirror:    bool,  // also place at -wx
    pub z_spacing: f32,   // world-Z gap between instances (should divide loop_s)
    #[serde(default = "default_prop_start_wz")]
    pub start_wz:  f32,   // nearest distance at which props start rendering
    #[serde(default = "default_prop_end_wz")]
    pub end_wz:    f32,   // farthest draw distance for props
    pub scale:     f32,   // size multiplier
    #[serde(default = "default_axis_scale")]
    pub width_scale: f32,
    #[serde(default = "default_axis_scale")]
    pub height_scale: f32,
    pub tint:      [u8; 3],
    /// Random scale variation ± this fraction (0 = uniform, 0.3 = ±30%)
    #[serde(default = "default_scale_var")]
    pub scale_var: f32,
    /// Lateral jitter: random x offset as fraction of focal*wx/wz
    #[serde(default = "default_x_jitter")]
    pub x_jitter:  f32,
    #[serde(default = "bool_true")]
    pub x_jitter_enabled: bool,
    #[serde(default)]
    pub y_jitter:  f32,
    #[serde(default)]
    pub y_jitter_enabled: bool,
    #[serde(default)]
    pub width_var: f32,
    #[serde(default)]
    pub height_var: f32,
    /// 0..1 blend between world-space placement and path-edge screen-space following.
    #[serde(default = "default_path_follow")]
    pub path_follow: f32,
    /// Additional screen-space gap from edge when path-follow is active.
    #[serde(default = "default_edge_gap")]
    pub edge_gap: f32,
    /// Base sink: how far into the ground the prop base extends (0=float, 1=grounded)
    #[serde(default = "default_y_sink")]
    pub y_sink:    f32,
    /// Blend strength for props emerging from ground
    #[serde(default = "default_ground_blend")]
    pub ground_blend: f32,
    /// Ground shadow size multiplier
    #[serde(default = "default_shadow_size")]
    pub shadow_size: f32,
    /// Ground shadow cast length multiplier
    #[serde(default = "default_shadow_length")]
    pub shadow_length: f32,
    /// Manual shadow cast direction (-1 left, +1 right)
    #[serde(default)]
    pub shadow_dir: f32,
    /// Blend factor to follow sun/moon direction for cast shadows
    #[serde(default = "default_shadow_follow_light")]
    pub shadow_follow_light: f32,
    /// Final shadow darkness/intensity multiplier
    #[serde(default = "default_shadow_opacity")]
    pub shadow_opacity: f32,
    /// Shadow edge softness (higher = softer penumbra)
    #[serde(default = "default_shadow_softness")]
    pub shadow_softness: f32,
    #[serde(default)]
    pub sprite_path: String,
    #[serde(default)]
    pub sprite_pool_paths: String,
    #[serde(default)]
    pub sprite_pool_enabled: bool,
    #[serde(default = "default_sprite_scale")]
    pub sprite_scale: f32,
    #[serde(default)]
    pub sprite_flip_x: bool,
    #[serde(default)]
    pub sprite_flip_y: bool,
    #[serde(default)]
    pub sprite_rot_deg: f32,
    #[serde(default)]
    pub sprite_offset_x: f32,
    #[serde(default)]
    pub sprite_offset_y: f32,
    /// 0..1 mix between selected tree type and other tree silhouettes
    #[serde(default)]
    pub tree_style_mix: f32,
    /// -1..1 tree style distribution bias (negative=dead, positive=pine)
    #[serde(default)]
    pub tree_style_bias: f32,
    /// Number of lateral tree rows to spawn for tree-like props
    #[serde(default = "default_tree_row_count")]
    pub tree_row_count: u32,
    /// Lateral spacing between extra tree rows
    #[serde(default = "default_tree_row_spacing")]
    pub tree_row_spacing: f32,
    /// Random per-row lateral offset
    #[serde(default = "default_tree_row_jitter")]
    pub tree_row_jitter: f32,
    /// Instance randomization seed
    #[serde(default)]
    pub seed:      u32,
}
fn default_scale_var() -> f32 { 0.22 }
fn default_x_jitter()  -> f32 { 0.18 }
fn default_y_sink()    -> f32 { 0.55 }
fn default_ground_blend() -> f32 { 0.35 }
fn default_shadow_size() -> f32 { 1.0 }
fn default_shadow_length() -> f32 { 1.0 }
fn default_shadow_follow_light() -> f32 { 0.65 }
fn default_shadow_opacity() -> f32 { 0.82 }
fn default_shadow_softness() -> f32 { 1.0 }
fn default_prop_start_wz() -> f32 { 0.15 }
fn default_prop_end_wz() -> f32 { 30.0 }
fn default_axis_scale() -> f32 { 1.0 }
fn default_path_follow() -> f32 { 0.65 }
fn default_edge_gap() -> f32 { 0.18 }
fn default_tree_row_count() -> u32 { 1 }
fn default_tree_row_spacing() -> f32 { 0.75 }
fn default_tree_row_jitter() -> f32 { 0.25 }

impl PropInstance {
    pub fn new(t: PropType) -> Self {
        let tint = t.default_tint();
        Self { enabled: true, emits_light: false, casts_shadow: true, prop_type: t, pos_x: 0.0, pos_y: 0.0, pos_z: 0.0, pixel_hitbox_enabled: false, mount_surface: AttachmentSurface::Floating, mount_side: MountSide::Center, wx: 1.4, mirror: true, z_spacing: 4.0,
                             start_wz: default_prop_start_wz(), end_wz: default_prop_end_wz(),
                             scale: 1.0, width_scale: 1.0, height_scale: 1.0,
                             tint, scale_var: 0.22, x_jitter: 0.18, x_jitter_enabled: true,
                             y_jitter: 0.0, y_jitter_enabled: false, width_var: 0.0, height_var: 0.0,
               path_follow: default_path_follow(), edge_gap: default_edge_gap(), y_sink: 0.55,
               ground_blend: default_ground_blend(), shadow_size: default_shadow_size(),
               shadow_length: default_shadow_length(), shadow_dir: 0.0,
               shadow_follow_light: default_shadow_follow_light(),
               shadow_opacity: default_shadow_opacity(), shadow_softness: default_shadow_softness(),
                             sprite_path: String::new(), sprite_scale: 1.0,
                             sprite_pool_paths: String::new(), sprite_pool_enabled: false,
                             sprite_flip_x: false, sprite_flip_y: false, sprite_rot_deg: 0.0,
                             sprite_offset_x: 0.0, sprite_offset_y: 0.0,
               tree_style_mix: 0.25, tree_style_bias: 0.0,
               tree_row_count: default_tree_row_count(), tree_row_spacing: default_tree_row_spacing(),
               tree_row_jitter: default_tree_row_jitter(), seed: 42 }
    }
}

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct PropsSettings {
    pub items: Vec<PropInstance>,
}

// ── Post-process ───────────────────────────────────────────────────────────
#[derive(Clone, Serialize, Deserialize)]
pub struct PostSettings {
    #[serde(default = "bool_true")]
    pub vignette_enabled: bool,
    pub vignette:    f32,    // 0.0..1.0
    pub fog_enabled: bool,
    pub fog_color:   [u8; 3],
    pub fog_density: f32,    // 0.0..1.0
    #[serde(default = "bool_true")]
    pub bloom_enabled: bool,
    /// Bloom intensity 0=off, 1=full
    #[serde(default)]
    pub bloom:       f32,
    #[serde(default = "bool_true")]
    pub grain_enabled: bool,
    /// Film grain intensity 0=off, 1=heavy
    #[serde(default)]
    pub grain:       f32,
    #[serde(default = "bool_true")]
    pub saturation_enabled: bool,
    /// Colour saturation: 0=greyscale, 1=normal, 2=vivid
    #[serde(default = "default_saturation")]
    pub saturation:  f32,
    #[serde(default = "bool_true")]
    pub realtime_lighting_enabled: bool,
    #[serde(default = "bool_true")]
    pub realtime_shadows_enabled: bool,
}
fn default_saturation() -> f32 { 1.0 }

impl Default for PostSettings {
    fn default() -> Self {
        Self {
            vignette_enabled: true,
            vignette: 0.0,
            fog_enabled: false,
            fog_color: [40, 35, 30],
            fog_density: 0.3,
            bloom_enabled: true,
            bloom: 0.0,
            grain_enabled: true,
            grain: 0.0,
            saturation_enabled: true,
            saturation: 1.0,
            realtime_lighting_enabled: true,
            realtime_shadows_enabled: true,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct AnimSettings {
    pub loop_s:    u32,   // 1..8  (keep integer for seamless GIF)
    pub play_speed: f32,  // 0.1..5.0
    #[serde(default = "default_gif_cycles")]
    pub gif_cycles: u32,  // extra seamless loop passes for export
    #[serde(default = "default_seamless_lock")]
    pub seamless_lock: bool,
}

fn default_gif_cycles() -> u32 { 1 }
fn default_seamless_lock() -> bool { true }

#[derive(Clone, Serialize, Deserialize)]
pub struct PathForgeSettings {
    #[serde(default)]
    pub canvas: CanvasSettings,
    pub scene: SceneSettings,
    pub floor: FloorSettings,
    pub walls: WallSettings,
    pub sky:   SkySettings,
    pub atmo:  AtmoSettings,
    pub props: PropsSettings,
    pub post:  PostSettings,
    pub anim:  AnimSettings,
}

impl Default for PathForgeSettings {
    fn default() -> Self { presets::stone_dungeon() }
}

// ── Presets ────────────────────────────────────────────────────────────────
pub mod presets {
    use super::*;

    // ── Small helpers to keep preset bodies compact ────────────────────────
    fn sky_off(void: [u8;3]) -> SkySettings {
        SkySettings {
            enabled: false,
            top: void,
            horizon: void,
            sun_enabled: false,
            sun_emits_light: true,
            sun_pos: default_sun_pos(),
            sun_z: 0.0,
            sun_radius: default_sun_radius(),
            sun_color: default_sun_color(),
            moon_enabled: false,
            moon_emits_light: true,
            moon_pos: default_moon_pos(),
            moon_z: 0.0,
            moon_radius: default_moon_radius(),
            moon_color: default_moon_color(),
            moon_phase: default_moon_phase(),
            moon_alpha: default_moon_alpha(),
            moon_texture_enabled: false,
            moon_texture_scale: default_moon_texture_scale(),
            stars_enabled: false,
            stars_count: default_stars_count(),
            stars_seed: 0,
            stars_size: default_stars_size(),
            stars_twinkle: default_stars_twinkle(),
            clouds_enabled: false,
            cloud_count: default_cloud_count(),
            cloud_seed: 0,
            cloud_speed: default_cloud_speed(),
            cloud_scale: default_cloud_scale(),
            cloud_opacity: default_cloud_opacity(),
            cloud_tint: default_cloud_tint(),
            cloud_variation: default_cloud_variation(),
        }
    }
    fn sky_on(top: [u8;3], hor: [u8;3]) -> SkySettings {
        SkySettings {
            enabled: true,
            top,
            horizon: hor,
            sun_enabled: true,
            sun_emits_light: true,
            sun_pos: default_sun_pos(),
            sun_z: 0.0,
            sun_radius: default_sun_radius(),
            sun_color: default_sun_color(),
            moon_enabled: false,
            moon_emits_light: true,
            moon_pos: default_moon_pos(),
            moon_z: 0.0,
            moon_radius: default_moon_radius(),
            moon_color: default_moon_color(),
            moon_phase: default_moon_phase(),
            moon_alpha: default_moon_alpha(),
            moon_texture_enabled: false,
            moon_texture_scale: default_moon_texture_scale(),
            stars_enabled: true,
            stars_count: default_stars_count(),
            stars_seed: 0,
            stars_size: default_stars_size(),
            stars_twinkle: default_stars_twinkle(),
            clouds_enabled: true,
            cloud_count: default_cloud_count(),
            cloud_seed: 0,
            cloud_speed: default_cloud_speed(),
            cloud_scale: default_cloud_scale(),
            cloud_opacity: default_cloud_opacity(),
            cloud_tint: default_cloud_tint(),
            cloud_variation: default_cloud_variation(),
        }
    }
    fn atmo1(t: AtmoType, th: f32, ts: u32, tsc: f32, nm: u32, nd: u32) -> AtmoSettings {
        AtmoSettings { layers: vec![AtmoLayer {
            enabled: true,
            emits_light: true,
            casts_shadow: true,
            atmo_type: t,
            torch_h: th,
            torch_spc: ts,
            torch_scale: tsc,
            n_motes: nm,
            n_debris: nd,
            placement_jitter: default_atmo_jitter(),
            flicker: default_atmo_flicker(),
            fx_scale: default_atmo_scale(),
            mount_surface: AttachmentSurface::Wall,
            mount_side: MountSide::Both,
            sprite_path: String::new(),
            sprite_pool_paths: String::new(),
            sprite_pool_enabled: false,
            sprite_scale: 1.0,
            sprite_flip_x: false,
            sprite_flip_y: false,
            sprite_rot_deg: 0.0,
            sprite_offset_x: 0.0,
            sprite_offset_y: 0.0,
            variation_seed: 0,
        }] }
    }
    fn no_props() -> PropsSettings { PropsSettings { items: vec![] } }
    fn pi(pt: PropType, wx: f32, mirror: bool, zs: f32, sc: f32, seed: u32) -> PropInstance {
        let tint = pt.default_tint();
        PropInstance { enabled: true, emits_light: false, casts_shadow: true, prop_type: pt, pos_x: 0.0, pos_y: 0.0, pos_z: 0.0, pixel_hitbox_enabled: false, mount_surface: AttachmentSurface::Floating, mount_side: MountSide::Center, wx, mirror, z_spacing: zs,
                       start_wz: default_prop_start_wz(), end_wz: default_prop_end_wz(),
                       scale: sc, width_scale: 1.0, height_scale: 1.0, tint,
                       scale_var: 0.22, x_jitter: 0.18, x_jitter_enabled: true,
                       y_jitter: 0.0, y_jitter_enabled: false, width_var: 0.0, height_var: 0.0,
                       path_follow: default_path_follow(), edge_gap: default_edge_gap(),
                       y_sink: 0.55, ground_blend: default_ground_blend(),
                       shadow_size: default_shadow_size(), shadow_length: default_shadow_length(),
                       shadow_dir: 0.0, shadow_follow_light: default_shadow_follow_light(),
                       shadow_opacity: default_shadow_opacity(), shadow_softness: default_shadow_softness(),
                       sprite_path: String::new(), sprite_pool_paths: String::new(), sprite_pool_enabled: false,
                       sprite_scale: 1.0,
                       sprite_flip_x: false, sprite_flip_y: false, sprite_rot_deg: 0.0,
                       sprite_offset_x: 0.0, sprite_offset_y: 0.0,
                       tree_style_mix: 0.25, tree_style_bias: 0.0,
                       tree_row_count: default_tree_row_count(),
                       tree_row_spacing: default_tree_row_spacing(),
                       tree_row_jitter: default_tree_row_jitter(), seed }
    }
    fn prop1(pt: PropType, wx: f32, mirror: bool, zs: f32, sc: f32) -> PropsSettings {
        PropsSettings { items: vec![pi(pt, wx, mirror, zs, sc, 42)] }
    }
    fn prop2(pt1: PropType, wx1: f32, m1: bool, sc1: f32,
             pt2: PropType, wx2: f32, m2: bool, sc2: f32, ls: u32) -> PropsSettings {
        let zs1 = ls as f32;
        let zs2 = (ls / 2).max(1) as f32;
        PropsSettings { items: vec![
            pi(pt1, wx1, m1, zs1, sc1, 42),
            pi(pt2, wx2, m2, zs2, sc2, 99),
        ]}
    }
    fn post(v: f32, fog: bool, fc: [u8;3], fd: f32) -> PostSettings {
        PostSettings {
            vignette_enabled: true,
            vignette: v,
            fog_enabled: fog,
            fog_color: fc,
            fog_density: fd,
            bloom_enabled: true,
            bloom: 0.0,
            grain_enabled: true,
            grain: 0.0,
            saturation_enabled: true,
            saturation: 1.0,
            realtime_lighting_enabled: true,
            realtime_shadows_enabled: true,
        }
    }
    fn sc_indoor(hy: u32, hw: f32, ch: f32, fm: f32, pp: f32, vc: [u8;3]) -> SceneSettings {
        SceneSettings { horizon_y: hy, horizon_curve: default_horizon_curve(), curve_top_weight: default_curve_top_weight(), curve_bottom_weight: default_curve_bottom_weight(), max_hw: hw, cam_h: ch, focal_mult: fm, path_power: pp,
                        void_color: vc, grass_enabled: false, grass_color: [28,90,18], grass_density: default_grass_density(),
                        grass_height: default_grass_height(), grass_upright: default_grass_upright(), ambient: 1.0,
                        lighting_preset: default_lighting_preset(),
                        atmo_light_influence: default_atmo_light_influence(),
                        atmo_tint_influence: default_atmo_tint_influence() }
    }
    fn sc_outdoor(hy: u32, hw: f32, ch: f32, fm: f32, pp: f32, vc: [u8;3]) -> SceneSettings {
        SceneSettings { horizon_y: hy, horizon_curve: default_horizon_curve(), curve_top_weight: default_curve_top_weight(), curve_bottom_weight: default_curve_bottom_weight(), max_hw: hw, cam_h: ch, focal_mult: fm, path_power: pp,
                        void_color: vc, grass_enabled: true, grass_color: [28,90,18], grass_density: default_grass_density(),
                        grass_height: default_grass_height(), grass_upright: default_grass_upright(), ambient: 1.0,
                        lighting_preset: default_lighting_preset(),
                        atmo_light_influence: default_atmo_light_influence(),
                        atmo_tint_influence: default_atmo_tint_influence() }
    }
    fn wall(en: bool, lx: f32, br: f32, js: f32, fr: u32, ns: u32, pat: WallPattern, base: [u8;3], mortar: [u8;3]) -> WallSettings {
        WallSettings { enabled: en, top_coverage: default_wall_top_coverage(), l_wx: lx, bright: br, junc_shadow: js, fade_rows: fr,
                       noise: ns, tex_scale: default_tex_scale(), tex_rot_90: default_tex_rot_90(),
                       damage: default_damage(), variation_seed: 0, pattern: pat, base, mortar }
    }

    // ── Presets ────────────────────────────────────────────────────────────
    pub fn stone_dungeon() -> PathForgeSettings { PathForgeSettings {
        canvas: CanvasSettings::default(),
        scene: sc_indoor(164, 128.0, 1.8, 1.0, 0.5,  [0,0,0]),
        floor: FloorSettings { depth_fade:3.8, edge_vignette:0.88, noise:13, tex_scale:3.0, tex_rot_90:false, damage:0.28, variation_seed:0, pattern:FloorPattern::Cobblestone, base:hex_to_rgb("#3a332a"), mortar:hex_to_rgb("#16120f") },
        walls: wall(true, 0.9, 1.8, 22.0, 22, 10, WallPattern::StoneBlock, hex_to_rgb("#28221c"), hex_to_rgb("#11100b")),
        sky:   sky_off([0,0,0]),
        atmo:  atmo1(AtmoType::Torch,    2.4, 4, 0.068, 10, 6),
        props: no_props(),
        post:  post(0.55, false, [30,25,20], 0.3),
        anim:  AnimSettings { loop_s:4, play_speed:1.0, gif_cycles: 1, seamless_lock: true },
    }}

    pub fn stone_crypt() -> PathForgeSettings { PathForgeSettings {
        canvas: CanvasSettings::default(),
        scene: sc_indoor(150, 100.0, 1.5, 0.9, 0.4,  [2,0,4]),
        floor: FloorSettings { depth_fade:5.0, edge_vignette:0.95, noise:8,  tex_scale:3.4, tex_rot_90:false, damage:0.30, variation_seed:0, pattern:FloorPattern::Brick,       base:hex_to_rgb("#2c2823"), mortar:hex_to_rgb("#100e0c") },
        walls: wall(true, 0.7, 1.2, 14.0, 15, 7, WallPattern::Brick, hex_to_rgb("#201c18"), hex_to_rgb("#0d0c0a")),
        sky:   sky_off([2,0,4]),
        atmo:  atmo1(AtmoType::Candle,   2.0, 4, 0.050,  6, 4),
        props: no_props(),
        post:  post(0.65, false, [20,18,16], 0.3),
        anim:  AnimSettings { loop_s:4, play_speed:0.7, gif_cycles: 1, seamless_lock: true },
    }}

    pub fn mossy_sewer() -> PathForgeSettings { PathForgeSettings {
        canvas: CanvasSettings::default(),
        scene: sc_indoor(170,  90.0, 1.4, 0.95, 0.35, [0,3,0]),
        floor: FloorSettings { depth_fade:4.5, edge_vignette:0.92, noise:18, tex_scale:3.2, tex_rot_90:false, damage:0.24, variation_seed:0, pattern:FloorPattern::Cobblestone, base:hex_to_rgb("#232a1e"), mortar:hex_to_rgb("#0f140c") },
        walls: wall(true, 0.6, 1.4, 18.0, 20, 15, WallPattern::StoneBlock, hex_to_rgb("#1c2618"), hex_to_rgb("#0c1209")),
        sky:   sky_off([0,3,0]),
        atmo:  atmo1(AtmoType::Firefly,  2.0, 4, 0.055, 14, 5),
        props: no_props(),
        post:  post(0.30, true, [10,18,8], 0.15),
        anim:  AnimSettings { loop_s:4, play_speed:0.8, gif_cycles: 1, seamless_lock: true },
    }}

    pub fn forest_path() -> PathForgeSettings { PathForgeSettings {
        canvas: CanvasSettings::default(),
        scene: sc_outdoor(160, 140.0, 2.0, 1.1, 0.6,  [8,14,4]),
        floor: FloorSettings { depth_fade:2.5, edge_vignette:0.75, noise:20, tex_scale:2.6, tex_rot_90:false, damage:0.12, variation_seed:0, pattern:FloorPattern::Grass, base:hex_to_rgb("#2a3419"), mortar:hex_to_rgb("#1c2412") },
        walls: wall(true, 1.1, 2.2, 28.0, 30, 18, WallPattern::Hedge, hex_to_rgb("#263417"), hex_to_rgb("#16200f")),
        sky:   sky_on([25,55,10], [55,85,30]),
        atmo:  atmo1(AtmoType::Firefly,  3.0, 4, 0.040, 18, 8),
        props: prop2(PropType::Tree, 1.5, true, 1.0, PropType::Bush, 1.1, true, 0.7, 4),
        post:  post(0.20, true, [10,18,8], 0.35),
        anim:  AnimSettings { loop_s:4, play_speed:1.2, gif_cycles: 1, seamless_lock: true },
    }}

    pub fn desert_canyon() -> PathForgeSettings { PathForgeSettings {
        canvas: CanvasSettings::default(),
        scene: sc_outdoor(156, 136.0, 2.2, 1.0, 0.55, [40,28,10]),
        floor: FloorSettings { depth_fade:2.0, edge_vignette:0.70, noise:16, tex_scale:2.2, tex_rot_90:false, damage:0.08, variation_seed:0, pattern:FloorPattern::Sand,  base:hex_to_rgb("#a58250"), mortar:hex_to_rgb("#826437") },
        walls: wall(true, 1.0, 2.5, 30.0, 25, 14, WallPattern::RockFace, hex_to_rgb("#966e41"), hex_to_rgb("#73552d")),
        sky:   sky_on([110,140,180], [200,170,100]),
        atmo:  atmo1(AtmoType::None,     3.2, 6, 0.035,  8, 7),
        props: prop2(PropType::Cactus, 1.4, true, 1.0, PropType::Rock, 1.9, true, 0.8, 6),
        post:  post(0.15, true, [100,80,40], 0.28),
        anim:  AnimSettings { loop_s:6, play_speed:1.5, gif_cycles: 1, seamless_lock: true },
    }}

    pub fn night_road() -> PathForgeSettings { PathForgeSettings {
        canvas: CanvasSettings::default(),
        scene: sc_outdoor(152, 144.0, 1.6, 1.05, 0.5,  [4,4,14]),
        floor: FloorSettings { depth_fade:3.0, edge_vignette:0.82, noise:10, tex_scale:3.6, tex_rot_90:false, damage:0.26, variation_seed:0, pattern:FloorPattern::Brick,       base:hex_to_rgb("#1e1e26"), mortar:hex_to_rgb("#121219") },
        walls: wall(true, 0.95, 1.5, 20.0, 18, 8, WallPattern::StoneBlock, hex_to_rgb("#161620"), hex_to_rgb("#0c0c14")),
        sky:   sky_on([4,5,18], [14,10,30]),
        atmo:  atmo1(AtmoType::Lantern,  2.8, 4, 0.060, 12, 3),
        props: no_props(),
        post:  post(0.60, true, [6,6,12], 0.20),
        anim:  AnimSettings { loop_s:4, play_speed:1.5, gif_cycles: 1, seamless_lock: true },
    }}

    pub fn magic_cavern() -> PathForgeSettings { PathForgeSettings {
        canvas: CanvasSettings::default(),
        scene: sc_indoor(160, 120.0, 1.7, 1.0, 0.45, [0,0,8]),
        floor: FloorSettings { depth_fade:4.0, edge_vignette:0.90, noise:12, tex_scale:3.2, tex_rot_90:false, damage:0.24, variation_seed:0, pattern:FloorPattern::Cobblestone, base:hex_to_rgb("#1a0e2a"), mortar:hex_to_rgb("#0e081a") },
        walls: wall(true, 0.85, 1.6, 20.0, 20, 12, WallPattern::StoneBlock, hex_to_rgb("#120820"), hex_to_rgb("#08041a")),
        sky:   sky_off([0,0,8]),
        atmo:  atmo1(AtmoType::Magic,    2.3, 4, 0.070, 16, 4),
        props: prop1(PropType::Mushroom, 1.2, true, 4.0, 0.8),
        post:  post(0.50, false, [0,0,8], 0.3),
        anim:  AnimSettings { loop_s:4, play_speed:0.9, gif_cycles: 1, seamless_lock: true },
    }}

    pub fn ice_dungeon() -> PathForgeSettings { PathForgeSettings {
        canvas: CanvasSettings::default(),
        scene: sc_indoor(164, 124.0, 1.8, 1.0, 0.48, [2,8,16]),
        floor: FloorSettings { depth_fade:4.2, edge_vignette:0.86, noise:9,  tex_scale:2.8, tex_rot_90:false, damage:0.32, variation_seed:0, pattern:FloorPattern::StoneBlock,  base:hex_to_rgb("#1e2e38"), mortar:hex_to_rgb("#0e1820") },
        walls: wall(true, 0.88, 2.0, 18.0, 22, 8, WallPattern::Brick, hex_to_rgb("#1a2a34"), hex_to_rgb("#0c1820")),
        sky:   sky_off([2,8,16]),
        atmo:  atmo1(AtmoType::IceWisp,  2.5, 4, 0.065,  8, 3),
        props: no_props(),
        post:  post(0.35, true, [8,12,20], 0.20),
        anim:  AnimSettings { loop_s:4, play_speed:0.85, gif_cycles: 1, seamless_lock: true },
    }}

    pub fn ruins_path() -> PathForgeSettings { PathForgeSettings {
        canvas: CanvasSettings::default(),
        scene: sc_outdoor(176, 144.0, 2.2, 1.1, 0.55, [10,14,18]),
        floor: FloorSettings { depth_fade:2.2, edge_vignette:0.65, noise:20, tex_scale:2.8, tex_rot_90:false, damage:0.22, variation_seed:0, pattern:FloorPattern::Cobblestone, base:hex_to_rgb("#8a7556"), mortar:hex_to_rgb("#4e4032") },
        walls: wall(true, 1.2, 3.0, 32.0, 35, 16, WallPattern::StoneBlock, hex_to_rgb("#756248"), hex_to_rgb("#42362a")),
        sky:   sky_on([55,85,130], [140,150,120]),
        atmo:  atmo1(AtmoType::None,     3.0, 4, 0.035,  6, 10),
        props: prop2(PropType::DeadTree, 1.5, true, 1.0, PropType::Rock, 2.2, true, 0.7, 4),
        post:  post(0.20, true, [80,75,65], 0.22),
        anim:  AnimSettings { loop_s:4, play_speed:1.3, gif_cycles: 1, seamless_lock: true },
    }}

    pub fn dark_street() -> PathForgeSettings { PathForgeSettings {
        canvas: CanvasSettings::default(),
        scene: sc_indoor(168, 132.0, 1.9, 1.05, 0.52, [6,6,8]),
        floor: FloorSettings { depth_fade:3.5, edge_vignette:0.80, noise:8,  tex_scale:3.6, tex_rot_90:false, damage:0.34, variation_seed:0, pattern:FloorPattern::Brick,       base:hex_to_rgb("#1a1a1e"), mortar:hex_to_rgb("#0c0c10") },
        walls: wall(true, 1.0, 1.8, 24.0, 28, 8, WallPattern::StoneBlock, hex_to_rgb("#161618"), hex_to_rgb("#0c0c0e")),
        sky:   sky_on([4,6,16], [16,12,24]),
        atmo:  atmo1(AtmoType::Lantern,  2.8, 4, 0.070, 14, 4),
        props: no_props(),
        post:  post(0.65, true, [8,6,8], 0.18),
        anim:  AnimSettings { loop_s:4, play_speed:1.2, gif_cycles: 1, seamless_lock: true },
    }}

    pub fn mountain_pass() -> PathForgeSettings { PathForgeSettings {
        canvas: CanvasSettings::default(),
        scene: sc_outdoor(160, 130.0, 2.5, 1.0, 0.5,  [50,55,70]),
        floor: FloorSettings { depth_fade:2.8, edge_vignette:0.72, noise:22, tex_scale:2.2, tex_rot_90:false, damage:0.10, variation_seed:0, pattern:FloorPattern::Sand,  base:hex_to_rgb("#786860"), mortar:hex_to_rgb("#504540") },
        walls: wall(true, 1.15, 2.2, 28.0, 30, 20, WallPattern::RockFace, hex_to_rgb("#5a5048"), hex_to_rgb("#3c3430")),
        sky:   sky_on([100,120,165], [165,160,145]),
        atmo:  atmo1(AtmoType::None,     3.0, 6, 0.030,  4, 8),
        props: prop2(PropType::Boulder, 1.8, true, 1.0, PropType::Rock, 2.3, true, 0.65, 6),
        post:  post(0.15, true, [100,105,115], 0.25),
        anim:  AnimSettings { loop_s:6, play_speed:1.4, gif_cycles: 1, seamless_lock: true },
    }}

    pub fn volcanic_rift() -> PathForgeSettings { PathForgeSettings {
        canvas: CanvasSettings::default(),
        scene: sc_indoor(172, 116.0, 1.8, 1.0, 0.50, [20,4,2]),
        floor: FloorSettings { depth_fade:3.5, edge_vignette:0.88, noise:14, tex_scale:2.8, tex_rot_90:false, damage:0.36, variation_seed:0, pattern:FloorPattern::StoneBlock,  base:hex_to_rgb("#1e0c08"), mortar:hex_to_rgb("#400808") },
        walls: wall(true, 0.85, 2.0, 22.0, 22, 18, WallPattern::RockFace, hex_to_rgb("#2a1008"), hex_to_rgb("#180604")),
        sky:   sky_off([20,4,2]),
        atmo:  atmo1(AtmoType::GreenFire, 2.2, 4, 0.080, 18, 5),
        props: prop1(PropType::Rock, 1.6, true, 4.0, 0.9),
        post:  post(0.55, false, [30,10,5], 0.3),
        anim:  AnimSettings { loop_s:4, play_speed:1.1, gif_cycles: 1, seamless_lock: true },
    }}

    pub const ALL: &'static [(&'static str, fn() -> PathForgeSettings)] = &[
        ("Stone Dungeon",  stone_dungeon),
        ("Stone Crypt",    stone_crypt),
        ("Mossy Sewer",    mossy_sewer),
        ("Forest Path",    forest_path),
        ("Desert Canyon",  desert_canyon),
        ("Night Road",     night_road),
        ("Magic Cavern",   magic_cavern),
        ("Ice Dungeon",    ice_dungeon),
        ("Ruins Path",     ruins_path),
        ("Dark Street",    dark_street),
        ("Mountain Pass",  mountain_pass),
        ("Volcanic Rift",  volcanic_rift),
    ];
}
