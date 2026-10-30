//! Transitions: how one scene gives way to the next while the camera walks on.
//!
//! Both worlds are one world for the length of a transition: the first is drawn up to a boundary
//! across the path, the second beyond it, and the renderer draws them in a single pass, so light,
//! fog and depth are right where they meet. `Transition` is what an author (or an agent) writes;
//! `plan` reads the two scenes and turns it into a `Crossing` with every choice made, plus notes
//! saying what was chosen and why, and warnings about anything that will not look right.

use super::*;

/// How one scene gives way to the next. Every `Auto` or 0 is decided by `plan` from the two scenes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Transition {
    /// What stands at the boundary. Auto: an open blend between open places, a doorway between
    /// rooms, a cave or tunnel mouth between open ground and a roofed place.
    pub threshold: Threshold,
    /// How far ahead the boundary is when the next world first shows, metres. 0 = auto (where the
    /// view fades into fog or distance).
    pub approach_m: f32,
    /// Metres walked after the boundary passes under the camera (the end of an exported clip).
    pub overshoot_m: f32,
    /// Width of the band where the ground, walls and props of two open worlds mix, metres. 0 = auto.
    pub blend_m: f32,
    /// Material of the face around the opening (a hillside, an end wall, a building front).
    /// None = auto: the first world's walls, else the second's.
    pub facade: Option<Material>,
    /// Height of that face, metres, where the first world has no roof to bound it. 0 = auto.
    pub facade_height: f32,
    /// How strongly light falls through the opening from one world onto the other (0..2).
    pub light_spill: f32,
    /// How much the eye adapts across the boundary (0..1): from a dark tunnel the exit glares, from
    /// daylight a cave mouth is black, and both settle as the camera passes through.
    pub adaptation: f32,
    /// Metres either side of the boundary over which the camera (eye height, horizon, zoom, bend)
    /// changes from one scene's to the other's.
    pub camera_blend_m: f32,
    /// Whose look (pixel size, palette, outline, grade) the frames take.
    pub style: StyleFrom,
    /// A structure at the boundary: an arch, a gate, a portal ring. Auto adds one where the
    /// threshold asks for it (a gate, a portal).
    pub marker: Marker,
    pub seed: u32,
    /// When the scenes look from different angles (`camera.yaw`, `camera.side`: a side-scroller
    /// and a view down the path), the camera swings: out of the first scene's view to look down
    /// the path before the boundary, and round into the second's after it. `seconds` each way
    /// (0 = auto, 2.5), with its `hold`, `ease`, `overshoot`, `orbit` and `bars` (`from` is not
    /// used: the views are the scenes'). Negative seconds: no swing, the views crossfade at the end.
    pub swing: Swing,
    /// A door's size where the boundary is a wall between two rooms: half its width and its
    /// height, metres. 0 each = as wide and as high as the rooms allow (the smaller room's
    /// whole end). A door narrower than the rooms leaves the first room an end wall with it in.
    pub door: [f32; 2],
}
impl Default for Transition {
    fn default() -> Self {
        Transition {
            threshold: Threshold::Auto, approach_m: 0.0, overshoot_m: 1.0, blend_m: 0.0, facade: None, facade_height: 0.0,
            light_spill: 1.0, adaptation: 1.0, camera_blend_m: 4.0, style: StyleFrom::Auto, marker: Marker::Auto, seed: 7,
            swing: Swing { hold: 0.0, ..Swing::default() },
            door: [0.0, 0.0],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Threshold {
    Auto,
    /// No structure: the ground, walls and props of the two worlds mix over a band.
    Open,
    /// A clean rectangular opening (a door, a tunnel cut square).
    Doorway,
    /// A rough, rounded opening in rock, in a hillside when the first world is open.
    CaveMouth,
    /// A doorway with a gatehouse standing in it.
    Gate,
    /// A glowing ring; the worlds meet sharply inside it.
    Portal,
}
impl Threshold {
    pub fn name(self) -> &'static str {
        match self { Threshold::Auto => "Auto", Threshold::Open => "Open", Threshold::Doorway => "Doorway", Threshold::CaveMouth => "Cave mouth", Threshold::Gate => "Gate", Threshold::Portal => "Portal" }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum StyleFrom {
    /// The look of the world the camera is in (it switches as the camera crosses).
    Auto,
    First,
    Second,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Marker { Auto, None, Archway, RuinedArch, Gate, Banners, Portal }

/// Which world's sky shows: through a roofed world's exit only the next sky can be seen, and over
/// the front of a roofed world only the current one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum SkyRule { First, Second, Blend }

/// The face around the opening.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Facade {
    pub material: Material,
    /// Height where nothing else bounds it, metres.
    pub height: f32,
    /// 0 = straight edges (masonry), 1 = ragged rock with a hillside profile.
    pub rough: f32,
    /// 0 = one level top; 1 = a street front: buildings side by side, each its own height, the
    /// ones either side of the opening as tall as the street's walls they carry on from.
    pub skyline: f32,
}

/// A transition with every choice made: what `plan` returns and the renderer draws.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Crossing {
    pub threshold: Threshold,
    pub approach: f32,
    pub overshoot: f32,
    /// Band where open ground mixes, metres (0 = a clean edge at the boundary).
    pub blend: f32,
    /// Metres either side over which the path widths meet.
    pub taper: f32,
    /// The opening's top: 0 flat (a door), 1 a full arch (a cave mouth).
    pub arch: f32,
    /// Raggedness of the opening's rim, 0..1.
    pub rim: f32,
    pub facade: Option<Facade>,
    pub spill: f32,
    pub adaptation: f32,
    pub camera_blend: f32,
    pub style: StyleFrom,
    pub marker: Option<SetPieceKind>,
    pub sky: SkyRule,
    pub seed: u32,
    /// The camera's swings (see `Transition::swing`): metres walked swinging out of the first
    /// scene's view before the crossing and into the second's after it (0: none), and how.
    pub swing_out: f32,
    pub swing_in: f32,
    #[serde(skip)]
    pub swing: Swing,
    /// The door's half width and height, metres; 0 for as large as the rooms allow
    /// (`Transition::door`).
    pub door: [f32; 2],
    /// What was chosen and why, in plain words.
    pub notes: Vec<String>,
    /// Things that will not look right, with what to change.
    pub warnings: Vec<String>,
}

fn roofed(s: &Scene) -> bool { s.ceiling.enabled }

/// How far past a fork's junction a barred branch is shut, metres ahead (`ForkWalk::barred`):
/// where the branches have parted, so the barrier closes one and leaves the other's mouth clear.
pub const BAR_PAST: f32 = 7.0;
/// The height of the wall across a barred branch with no walls of its own, metres.
pub const BAR_OPEN_HEIGHT: f32 = 3.6;

/// What shuts a fork's branch (`ForkWalk::barred`), standing in that branch's own world so it
/// lines up with its path and walls however the fork is drawn.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Barrier {
    /// Wall the branch up across its mouth: between its walls, of their material and as high as
    /// they are (where it has none, a wall of its wall material `BAR_OPEN_HEIGHT` m high, as wide
    /// as its path and a little more).
    pub wall: bool,
    /// A picture of what stands in the way (a door, a gate or a grate set in the wall's foot;
    /// without a wall, debris or a barricade across the path): a PNG, absolute or relative to
    /// the branch scene's folder. Empty for none.
    pub sprite: String,
    /// The picture's height, metres. 0: 3 m in a wall; across a path, as wide as the path.
    pub height: f32,
    /// Opened (a shortcut unbarred, a gate unlocked): the wall stands with a doorway through it
    /// and the picture's two leaves swung back into the passage, and the walk goes through.
    pub open: bool,
    /// Where in the picture the door's (or gate's) leaves are, for swinging them open: x0 y0 x1
    /// y1, fractions of the picture (the rest is its frame).
    pub leaves: [f32; 4],
}
impl Default for Barrier {
    fn default() -> Barrier { Barrier { wall: true, sprite: String::new(), height: 0.0, open: false, leaves: [0.0, 0.0, 1.0, 1.0] } }
}

/// The doorway through an opened barrier's wall (`Barrier::open`): half its width and its
/// height, metres, for a path of half width `path_hw` and a wall `top` high.
pub fn bar_doorway(path_hw: f32, top: f32) -> (f32, f32) {
    ((path_hw * 0.85).clamp(2.0, 3.0), (top - 0.6).min(3.6).max(2.6))
}

/// Whether an open scene is a street between masonry walls too tall to see over: walking into
/// one from anywhere else, its walls must stand in a street front, not rise out of the ground.
fn streeted(s: &Scene) -> bool { !roofed(s) && s.walls.enabled && s.walls.height >= 2.5 && !rocky(&s.walls.material) }

/// Whether a material reads as rock (a cave) rather than masonry.
fn rocky(m: &Material) -> bool { matches!(m.pattern, Pattern::RockFace | Pattern::Ice | Pattern::Dirt | Pattern::Sand) }

/// How far a scene can be seen down the path before fog or distance swallows it, metres.
pub fn visibility(s: &Scene) -> f32 {
    let mut v: f32 = 60.0;
    if s.light.fog.enabled { v = v.min(s.light.fog.distance.max(1.0) * 2.5); }
    if roofed(s) || s.walls.enabled { v = v.min(45.0); }
    v
}

/// Metres past a boundary over which the world beyond turns from the light it was seen in (the
/// fog, sky and light of the world the camera came from) to its own: seen from a night road,
/// a place lit by day is night at its threshold, and the day comes in as the walk goes in.
pub const LIGHT_BLEND_M: f32 = 10.0;

/// Make every choice for a transition from `a` into `b`.
pub fn plan(a: &Scene, b: &Scene, t: &Transition) -> Crossing {
    let mut notes = Vec::new();
    let mut warnings = Vec::new();
    let (an, bn) = (if a.name.is_empty() { "the first scene" } else { a.name.as_str() }, if b.name.is_empty() { "the second scene" } else { b.name.as_str() });
    let (ra, rb) = (roofed(a), roofed(b));

    // Threshold.
    let threshold = match t.threshold {
        Threshold::Auto => {
            let th = match (ra, rb) {
                // Into, out of or between walled streets that differ: a face fills the step, so
                // the walls of the one meet the walls of the other along a building front.
                (false, false) if streeted(a) || streeted(b) => {
                    let same = a.walls.enabled == b.walls.enabled && (a.walls.height - b.walls.height).abs() < 1.0
                        && (a.path.half_width + a.walls.gap - b.path.half_width - b.walls.gap).abs() < 0.3;
                    if same { Threshold::Open } else { Threshold::Doorway }
                }
                (false, false) => Threshold::Open,
                (true, true) => {
                    let same = a.walls.enabled == b.walls.enabled && (a.ceiling.height - b.ceiling.height).abs() < 0.3
                        && (a.path.half_width + a.walls.gap - b.path.half_width - b.walls.gap).abs() < 0.3;
                    if same { Threshold::Open } else if rocky(&a.walls.material) || rocky(&b.walls.material) { Threshold::CaveMouth } else { Threshold::Doorway }
                }
                (false, true) => if rocky(&b.walls.material) { Threshold::CaveMouth } else { Threshold::Doorway },
                (true, false) => if rocky(&a.walls.material) { Threshold::CaveMouth } else { Threshold::Doorway },
            };
            notes.push(match (ra, rb, th) {
                (false, false, Threshold::Doorway) => format!("{} is a street between tall walls: {bn} begins in a street front, the walls of the one meeting the other.",
                    if streeted(b) { bn } else { an }),
                (false, false, _) => format!("{an} and {bn} are both open: their ground, walls and props mix over a band instead of meeting at a line."),
                (true, true, Threshold::Open) => format!("{an} and {bn} are passages of the same size: one runs straight into the other."),
                (true, true, _) => format!("{an} and {bn} are both roofed with different sections: a {} joins them.", th.name().to_lowercase()),
                (false, true, _) => format!("{bn} is roofed and {an} is open: {bn} begins at a {} in a face of {}.", th.name().to_lowercase(), if rocky(&b.walls.material) { "rock" } else { "masonry" }),
                (true, false, _) => format!("{an} is roofed and {bn} is open: {an} ends at a {}, and {bn}'s light falls into it.", if th == Threshold::CaveMouth { "ragged mouth" } else { "square exit" }),
            });
            th
        }
        th => th,
    };

    // Approach: where the next world first shows, at the edge of what can be seen.
    let seen = visibility(a);
    let approach = if t.approach_m > 0.0 { t.approach_m.max(2.0) } else {
        let ap = (seen * 0.8).clamp(12.0, 50.0);
        notes.push(format!("The boundary first shows {ap:.0} m ahead{}.", if a.light.fog.enabled { ", out of the fog" } else { "" }));
        ap
    };
    if !a.light.fog.enabled && !ra && !a.walls.enabled && approach > 20.0 {
        warnings.push(format!("{an} has no fog, so the boundary will be visible as soon as it is {approach:.0} m ahead. Turn on fog in {an}, or shorten approach_m."));
    }

    let blend = if threshold == Threshold::Open {
        if t.blend_m > 0.0 { t.blend_m } else { (approach * 0.2).clamp(3.0, 10.0) }
    } else if threshold == Threshold::Portal { 0.0 } else { 0.0 };
    let taper = if threshold == Threshold::Open { blend } else { 0.0 };
    let (arch, rim) = match threshold { Threshold::CaveMouth => (1.0, 1.0), Threshold::Gate => (0.0, 0.0), _ => (0.0, 0.0) };

    // The face around the opening, when the first world is bigger than the opening.
    let facade = match threshold {
        Threshold::Open | Threshold::Portal => None,
        _ => {
            // Between open places the face is the walled street's own front.
            let front = !ra && !rb && (streeted(a) || streeted(b));
            let material = t.facade.clone().unwrap_or_else(|| {
                if front { return if streeted(b) { b.walls.material.clone() } else { a.walls.material.clone() }; }
                let m = if a.walls.enabled && (ra || threshold != Threshold::CaveMouth) { a.walls.material.clone() }
                    else if b.walls.enabled { b.walls.material.clone() }
                    else { b.path.material.clone() };
                // A cave mouth opens in rock, whatever the passage beyond is built of.
                if threshold == Threshold::CaveMouth && !rocky(&m) { Material { pattern: Pattern::RockFace, tile_size: m.tile_size.max(3.0), damage: 0.0, ..m } } else { m }
            });
            let b_top = if rb { b.ceiling.height } else if b.walls.enabled && b.walls.height > 0.0 { b.walls.height } else { 4.0 };
            let height = if t.facade_height > 0.0 { t.facade_height }
                else if front { if streeted(b) { b.walls.height } else { a.walls.height } }
                else if threshold == Threshold::CaveMouth { (b_top * 2.6).max(b_top + 5.0) } else { (b_top * 1.8).max(b_top + 2.5) };
            // A roofed world's end wall closes it up to its own ceiling, or the next sky shows over it.
            let height = if ra && t.facade_height <= 0.0 { height.max(a.ceiling.height) } else { height };
            if front {
                notes.push(format!("Either side of the way in stand buildings of {}, {height:.0} m at the street and taller and lower beyond.", material.pattern.name().to_lowercase()));
            }
            if !ra && rb {
                notes.push(format!("Around the opening stands a face of {} {height:.0} m high; {an}'s sky shows above it.", material.pattern.name().to_lowercase()));
            }
            Some(Facade { material, height, rough: if threshold == Threshold::CaveMouth { 1.0 } else { 0.0 }, skyline: if front { 1.0 } else { 0.0 } })
        }
    };

    let marker = match t.marker {
        Marker::None => None,
        Marker::Auto => match threshold { Threshold::Gate => Some(SetPieceKind::Gate), Threshold::Portal => Some(SetPieceKind::Portal), _ => None },
        Marker::Archway => Some(SetPieceKind::Archway),
        Marker::RuinedArch => Some(SetPieceKind::RuinedArch),
        Marker::Gate => Some(SetPieceKind::Gate),
        Marker::Banners => Some(SetPieceKind::Banners),
        Marker::Portal => Some(SetPieceKind::Portal),
    };

    let sky = match (ra, rb) {
        (true, _) => SkyRule::Second,
        (false, true) => SkyRule::First,
        _ => SkyRule::Blend,
    };
    if sky == SkyRule::Blend && a.sky.enabled && b.sky.enabled && (a.sky.top != b.sky.top || a.sky.sun.enabled != b.sky.sun.enabled || a.sky.moon.body.enabled != b.sky.moon.body.enabled) {
        notes.push("The skies differ: the sky changes from one to the other as the camera walks through the band.".into());
    }

    let spill = t.light_spill.clamp(0.0, 2.0);
    if spill > 0.0 && (ra || rb) && threshold != Threshold::Open {
        notes.push(format!("Light from {} falls through the opening onto the last metres of {}.", if ra { bn } else { an }, if ra { an } else { bn }));
    }
    let adaptation = t.adaptation.clamp(0.0, 1.0);

    // Looks that cannot blend.
    let sa = &a.style;
    let sb = &b.style;
    let mut diffs = Vec::new();
    if sa.pixel_size != sb.pixel_size { diffs.push(format!("pixel size {} vs {}", sa.pixel_size, sb.pixel_size)); }
    if sa.palette != sb.palette { diffs.push("palette".to_string()); }
    if sa.outline.enabled != sb.outline.enabled { diffs.push("outline".to_string()); }
    if sa.grade != sb.grade { diffs.push("grade".to_string()); }
    if sa.paint != sb.paint { diffs.push("paint".to_string()); }
    let style = match t.style {
        StyleFrom::Auto if !diffs.is_empty() => {
            warnings.push(format!("The two looks differ ({}): the frame switches look as the camera crosses. Give both scenes one style, or set style to First or Second.", diffs.join(", ")));
            StyleFrom::Auto
        }
        s => s,
    };
    if a.motion.fps != b.motion.fps {
        warnings.push(format!("{an} runs at {} fps and {bn} at {} fps; an exported transition uses {bn}'s.", a.motion.fps, b.motion.fps));
    }
    if (a.motion.speed - b.motion.speed).abs() > 0.05 {
        notes.push(format!("The walking speed eases from {:.1} to {:.1} m/s over the transition.", a.motion.speed, b.motion.speed));
    }
    if (a.camera.eye_height - b.camera.eye_height).abs() > 0.05 || (a.camera.horizon - b.camera.horizon).abs() > 0.01 || (a.camera.zoom - b.camera.zoom).abs() > 0.02 {
        notes.push(format!("The camera changes over {:.0} m either side of the boundary (eye height, horizon, zoom).", t.camera_blend_m.max(0.5)));
    }

    // Scenes looking from different angles: the camera swings out of one view and into the other,
    // crossing the boundary looking down the path.
    let (ta, tb) = (a.camera.turned(), b.camera.turned());
    let mut swing = t.swing.clone();
    let (mut swing_out, mut swing_in) = (0.0, 0.0);
    if (ta || tb) && swing.seconds >= 0.0 {
        if swing.seconds == 0.0 { swing.seconds = 2.5; }
        let each = swing.length();
        if ta { swing_out = each * a.motion.speed.max(0.01); }
        if tb { swing_in = each * b.motion.speed.max(0.01); }
        notes.push(match (ta, tb) {
            (true, true) => format!("Both scenes look from an angle: the camera swings out of {an}'s view to look down the path ({:.1} s), crosses, and swings round into {bn}'s ({:.1} s).", each, each),
            (true, false) => format!("{an} looks from an angle: the camera swings out of its view to look down the path ({each:.1} s) before the boundary."),
            _ => format!("{bn} looks from an angle: past the boundary the camera swings round into its view ({each:.1} s)."),
        });
        if a.canvas.width != b.canvas.width || a.canvas.height != b.canvas.height {
            warnings.push(format!("{an} is {}x{} and {bn} {}x{}: a camera swinging between them keeps one frame (an exported transition uses {bn}'s). Give both scenes one canvas.", a.canvas.width, a.canvas.height, b.canvas.width, b.canvas.height));
        }
    } else if ta != tb || (ta && (a.camera.yaw != b.camera.yaw || a.camera.side != b.camera.side)) {
        notes.push("The scenes look from different angles: the views crossfade at the end (give the transition a swing to have the camera come round instead).".into());
    }

    // The walk must carry on past the boundary until the camera, the sky and the light are
    // wholly the second world's, so a clip ends exactly on its loop.
    let camera_blend = t.camera_blend_m.max(0.5);
    // A roofed room keeps its own light (no `LIGHT_BLEND_M` into it), and one camera needs no
    // blending into itself: a door between two such rooms settles as soon as it is passed.
    let views = if a.camera == b.camera { 0.0 } else { camera_blend };
    let light = if rb { 0.0 } else { LIGHT_BLEND_M };
    let settle = views.max(if sky == SkyRule::Blend { blend.max(2.0) } else { 0.0 }).max(blend).max(light) + 1.0;
    let overshoot = t.overshoot_m.max(settle);
    Crossing {
        threshold, approach, overshoot, blend, taper, arch, rim, facade, spill, adaptation,
        camera_blend, style, marker, sky, seed: t.seed, swing_out, swing_in, swing, door: t.door, notes, warnings,
    }
}

/// A scene seen looking straight down the path (its camera's height, horizon and zoom kept): how a
/// transition between scenes looking from different angles crosses the boundary.
pub fn looking_ahead(s: &Scene) -> Scene {
    let mut t = s.clone();
    (t.camera.yaw, t.camera.side) = (0.0, 0.0);
    t
}

/// A fork in the path where the player chooses: the path splits into a left and a right branch,
/// each running into its own world. Every 0 or Auto is decided by `plan_fork`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ForkChoice {
    /// Angle between each branch and the path, degrees (0 = auto: wide enough that the branch not
    /// taken leaves the view once the camera turns).
    pub angle: f32,
    /// How far ahead the junction is when the branches first show, metres (0 = auto).
    pub approach_m: f32,
    /// Metres walked past the junction at the end of an exported clip.
    pub overshoot_m: f32,
    /// Band over which each branch turns from this world into its own, metres (0 = auto).
    pub blend_m: f32,
    /// Length of ground of this world left between the branches past the junction, metres (0 = auto;
    /// negative = none: the ground between the branches is the branch worlds' from where they
    /// part, each its own half, so what stands beside the branch taken goes on into its world).
    pub wedge_m: f32,
    /// Metres down each branch over which its path keeps this world's look before turning into
    /// its own over `blend_m`, in patches (0 = auto; negative = none: each branch's path is its
    /// own from the junction). Two differently coloured paths parting at the junction read as a
    /// seam; this way the fork is one path that changes once the player is on their way.
    pub path_hold_m: f32,
    /// Metres of walking over which the camera turns onto the chosen branch.
    pub steer_m: f32,
    /// Metres either side of the junction over which the camera becomes the chosen world's.
    pub camera_blend_m: f32,
    /// Length of the fork's mouth, metres (0 = auto): over this stretch before the junction the
    /// first world's props stand back from the path in a widening funnel, so both branches can be
    /// seen parting instead of vanishing behind the trees that line the road.
    pub mouth_m: f32,
    /// The branch taken when the game has not chosen by the time the camera reaches the junction.
    pub default_branch: Branch,
    /// How much the eye adapts across (0..1).
    pub adaptation: f32,
    pub style: StyleFrom,
    pub seed: u32,
}
impl Default for ForkChoice {
    fn default() -> Self {
        ForkChoice {
            angle: 0.0, approach_m: 0.0, overshoot_m: 1.0, blend_m: 0.0, wedge_m: 0.0, path_hold_m: 0.0, steer_m: 6.0, camera_blend_m: 4.0, mouth_m: 0.0,
            default_branch: Branch::Left, adaptation: 1.0, style: StyleFrom::Auto, seed: 11,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Branch { Left, Right }
impl Branch {
    /// The world index of a branch in a fork frame (1 = left, 2 = right).
    pub fn realm(self) -> usize { match self { Branch::Left => 1, Branch::Right => 2 } }
    pub fn name(self) -> &'static str { match self { Branch::Left => "left", Branch::Right => "right" } }
}

/// A fork with every choice made.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ForkPlan {
    /// Sideways metres per metre ahead of each branch.
    pub slope: f32,
    pub approach: f32,
    pub overshoot: f32,
    pub blend: f32,
    pub wedge: f32,
    /// Metres down each branch its path keeps the first world's look (negative: none).
    pub hold: f32,
    pub steer: f32,
    pub camera_blend: f32,
    /// Metres before the junction over which the first world's props stand back (the mouth).
    pub mouth: f32,
    pub default_branch: Branch,
    pub adaptation: f32,
    pub style: StyleFrom,
    pub seed: u32,
    pub notes: Vec<String>,
    pub warnings: Vec<String>,
}

/// Make every choice for a fork from `a` into `left` or `right`, for a canvas `width` x `height`.
pub fn plan_fork(a: &Scene, left: &Scene, right: &Scene, f: &ForkChoice) -> ForkPlan {
    let mut notes = Vec::new();
    let mut warnings = Vec::new();
    let an = if a.name.is_empty() { "this scene" } else { a.name.as_str() };
    // The branch not taken must leave the view: past the junction its centre runs off at twice the
    // angle, so the angle must clear half the view's width.
    let (w, h) = (a.canvas.width.max(2) as f32, a.canvas.height.max(2) as f32);
    let horizon = (a.camera.horizon.clamp(0.02, 0.98) * h).round();
    let focal = (h - horizon).max(1.0) * a.camera.zoom.clamp(0.1, 8.0);
    let half_view = w * 0.5 / focal;
    // Both branches must be in view on the way up (each inside half the view's width), and the one
    // not taken must leave it once the camera has turned (it then runs off at twice the angle).
    let slope = if f.angle > 0.0 { f.angle.clamp(4.0, 60.0).to_radians().tan() } else {
        let s = (half_view * 0.62).clamp(0.1, 0.8);
        notes.push(format!("The branches part at {:.0} degrees each way: both are in view on the way up, and the one not taken swings out of view after the turn.", s.atan().to_degrees()));
        s
    };
    if slope * 2.0 < half_view {
        warnings.push(format!("At {:.0} degrees the branch not taken stays in view after the turn; use at least {:.0}.", slope.atan().to_degrees(), (half_view * 0.5).atan().to_degrees().ceil()));
    }
    if slope > half_view {
        warnings.push(format!("At {:.0} degrees the branches leave the view almost at once, so the fork is hard to see coming; use at most {:.0}.", slope.atan().to_degrees(), half_view.atan().to_degrees().floor()));
    }
    if a.ceiling.enabled {
        warnings.push(format!("{an} is roofed: the passage ends at the junction and both branches start in the open. A fork reads best from open ground or a walled street."));
    }
    let approach = if f.approach_m > 0.0 { f.approach_m.max(4.0) } else { (visibility(a) * 0.7).clamp(14.0, 40.0) };
    let blend = if f.blend_m > 0.0 { f.blend_m } else { (approach * 0.25).clamp(3.0, 10.0) };
    let wedge = if f.wedge_m > 0.0 { f.wedge_m } else if f.wedge_m < 0.0 { 0.0 } else { (approach * 0.6).clamp(8.0, 30.0) };
    let hold = if f.path_hold_m != 0.0 { f.path_hold_m } else { (approach * 0.2).clamp(4.0, 10.0) };
    notes.push(format!("The junction first shows {approach:.0} m ahead. Left goes to {}, right to {}; each branch turns into its world over {blend:.0} m, with {an}'s ground between them for {wedge:.0} m.",
        if left.name.is_empty() { "the left scene" } else { left.name.as_str() }, if right.name.is_empty() { "the right scene" } else { right.name.as_str() }));
    notes.push(format!("The game chooses before the junction; the camera turns onto the branch over {:.0} m. If nothing is chosen, it takes the {} branch.", f.steer_m.max(1.0), f.default_branch.name()));
    let lit = |s: &Scene| s.sky.enabled && s.sky.sun.enabled && s.sky.sun.intensity > 0.2;
    if lit(left) != lit(right) || lit(a) != lit(left) || lit(a) != lit(right) {
        warnings.push("The worlds are lit very differently (one by day, another by night): on the way up the darker branch reads as a dark mass beside the bright one. Branches into worlds under the same sky read best.".into());
    }
    for (n, s) in [("left", left), ("right", right)] {
        if s.ceiling.enabled {
            warnings.push(format!("The {n} world ({}) is roofed: a branch runs into it with no entrance. Lead the branch into an open stop first and walk from there into {} through a transition (in a journey: a stop on the branch, then Go).", s.name, s.name));
        }
        if s.style.pixel_size != a.style.pixel_size || s.style.palette != a.style.palette {
            warnings.push(format!("The {n} world's look differs from {an}'s (pixel size or palette): the frame switches look once the camera is on that branch."));
        }
    }
    if hold >= 0.0 {
        notes.push(format!("Each branch's path keeps {an}'s look for {hold:.0} m past the junction and turns into its own world's over the next {blend:.0} m, so the walk goes on until the camera is past that."));
    }
    let camera_blend = f.camera_blend_m.max(0.5);
    // A walk that ended with the old look still on the ground ahead would change it at once.
    let restyled = if hold >= 0.0 { hold + blend + 1.0 } else { 0.0 };
    let mouth = if f.mouth_m > 0.0 { f.mouth_m.min(approach) } else { (approach * 0.35).clamp(6.0, 16.0) };
    notes.push(format!("For the last {mouth:.0} m before the junction {an}'s props stand back from the path in a widening mouth, so the branches show parting."));
    ForkPlan {
        slope, approach, overshoot: f.overshoot_m.max(camera_blend.max(blend) + 1.0).max(restyled), blend, wedge, hold, steer: f.steer_m.max(1.0), camera_blend, mouth,
        default_branch: f.default_branch, adaptation: f.adaptation.clamp(0.0, 1.0), style: f.style, seed: f.seed, notes, warnings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn preset(n: &str) -> Scene { presets::ALL.iter().find(|p| p.0 == n).map(|p| (p.1)()).unwrap() }

    #[test]
    fn the_plan_reads_the_two_scenes() {
        let t = Transition::default();
        let c = plan(&preset("Stone Dungeon"), &preset("Forest Path"), &t);
        assert!(matches!(c.threshold, Threshold::Doorway | Threshold::CaveMouth), "a roofed place opens onto the forest: {:?}", c.threshold);
        assert_eq!(c.sky, SkyRule::Second, "through the exit only the forest sky can be seen");
        assert!(c.facade.is_some());
        // A lofty hall's end wall reaches its vault: no strip of the next sky over the exit.
        let mut nave = preset("Stone Dungeon");
        nave.ceiling.height = 9.0;
        let c = plan(&nave, &preset("Forest Path"), &t);
        assert!(c.facade.as_ref().is_some_and(|f| f.height >= 9.0), "{:?}", c.facade.as_ref().map(|f| f.height));
        let c = plan(&preset("Forest Path"), &preset("Desert Canyon"), &t);
        assert_eq!(c.threshold, Threshold::Open);
        assert!(c.blend > 0.0 && c.facade.is_none());
        let c = plan(&preset("Forest Path"), &preset("Stone Dungeon"), &t);
        assert_eq!(c.sky, SkyRule::First);
        assert!(c.facade.as_ref().is_some_and(|f| f.height > preset("Stone Dungeon").ceiling.height), "a face taller than the passage hides what is behind it");
        assert!(!c.notes.is_empty());
    }

    #[test]
    fn a_walled_street_begins_in_a_street_front_not_two_walls_out_of_the_ground() {
        let t = Transition::default();
        let mut alley = preset("Forest Path");
        alley.walls.enabled = true;
        alley.walls.height = 8.0;
        alley.walls.gap = 0.8;
        alley.walls.material.pattern = Pattern::Brick;
        let lane = preset("Forest Path");
        let c = plan(&lane, &alley, &t);
        assert_eq!(c.threshold, Threshold::Doorway, "{:?}", c.notes);
        let f = c.facade.as_ref().expect("a front either side of the way in");
        assert_eq!((f.height, f.skyline, f.material.pattern), (8.0, 1.0, Pattern::Brick), "as tall as the street's walls, of their brick, with a roofline");
        // Out of it too, and between streets of different widths; the same street goes on open.
        assert_eq!(plan(&alley, &lane, &t).threshold, Threshold::Doorway);
        let mut wider = alley.clone();
        wider.walls.gap = 3.0;
        assert_eq!(plan(&alley, &wider, &t).threshold, Threshold::Doorway);
        assert_eq!(plan(&alley, &alley.clone(), &t).threshold, Threshold::Open);
        // A canyon's rock walls rise from the ground as they are.
        let mut canyon = alley.clone();
        canyon.walls.material.pattern = Pattern::RockFace;
        assert_eq!(plan(&lane, &canyon, &t).threshold, Threshold::Open);
    }
}
