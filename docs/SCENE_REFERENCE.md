# PathForge scene reference

Generated from the code by `pf schema --markdown`; do not edit by hand (a test compares this file with the code). Every field is optional: a missing field takes its default. Enum values are written as shown (`"Torch"`, `{"Named": "PICO-8"}`). Lengths are metres, angles degrees, colours `[r, g, b]` 0..255, times seconds.

See [MANUAL.md](MANUAL.md) for what each part does and how to use it.

### Scene

| Field | Type | Default | Meaning |
|---|---|---|---|
| `version` | integer | `3` |  |
| `name` | string | `"Stone Dungeon"` |  |
| `canvas` | [Canvas](#canvas) | `{"width":480,"height":854}` |  |
| `camera` | [Camera](#camera) | (see the type) |  |
| `path` | [PathShape](#pathshape) | (see the type) |  |
| `verge` | [Verge](#verge) | (see the type) |  |
| `walls` | [Walls](#walls) | (see the type) |  |
| `ceiling` | [Ceiling](#ceiling) | (see the type) |  |
| `sky` | [Sky](#sky) | (see the type) |  |
| `light` | [Lighting](#lighting) | (see the type) |  |
| `fixtures` | list of [Fixture](#fixture) | (see the type) |  |
| `props` | list of [PropLayer](#proplayer) | `[]` |  |
| `set_pieces` | list of [SetPiece](#setpiece) | `[]` | Structures that span the path at intervals: archways, gates, banners. |
| `particles` | list of [Particles](#particles) | (see the type) |  |
| `weather` | [Weather](#weather) | (see the type) | Lightning and fog banks. |
| `prop_defs` | map of name → [PropDef](#propdef) |  | Props described by data, by name: a prop layer uses one with `def: "<name>"`. Kits (folders with a kit.json) hold more, used as `def: "<folder>#<name>"`. |
| `post` | [Post](#post) | (see the type) |  |
| `style` | [Style](#style) | (see the type) |  |
| `motion` | [Motion](#motion) | `{"loop_length":24.0,"speed":4.0,"fps":24}` |  |

## Types

### ForkChoice

A fork in the path where the player chooses: the path splits into a left and a right branch,
each running into its own world. Every 0 or Auto is decided by `plan_fork`.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `angle` | number | `0.0` | Angle between each branch and the path, degrees (0 = auto: wide enough that the branch not taken leaves the view once the camera turns). |
| `approach_m` | number | `0.0` | How far ahead the junction is when the branches first show, metres (0 = auto). |
| `overshoot_m` | number | `1.0` | Metres walked past the junction at the end of an exported clip. |
| `blend_m` | number | `0.0` | Band over which each branch turns from this world into its own, metres (0 = auto). |
| `wedge_m` | number | `0.0` | Length of ground of this world left between the branches past the junction, metres (0 = auto). |
| `steer_m` | number | `6.0` | Metres of walking over which the camera turns onto the chosen branch. |
| `camera_blend_m` | number | `4.0` | Metres either side of the junction over which the camera becomes the chosen world's. |
| `default_branch` | [Branch](#branch) | `"Left"` | The branch taken when the game has not chosen by the time the camera reaches the junction. |
| `adaptation` | number | `1.0` | How much the eye adapts across (0..1). |
| `style` | [StyleFrom](#stylefrom) | `"Auto"` |  |
| `seed` | integer | `11` |  |

### Journey

A map of places a game walks through: each stop is a scene, and each says what comes next
(another stop through a transition, a fork where the player chooses, or the end). Scene names
are files relative to the journey file, or `preset:<name>`.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `version` | integer | `0` |  |
| `name` | string | `""` |  |
| `start` | string | `""` | The stop the walk begins at. |
| `stops` | map of name → [Stop](#stop) | `{}` |  |

### Transition

How one scene gives way to the next. Every `Auto` or 0 is decided by `plan` from the two scenes.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `threshold` | [Threshold](#threshold) | `"Auto"` | What stands at the boundary. Auto: an open blend between open places, a doorway between rooms, a cave or tunnel mouth between open ground and a roofed place. |
| `approach_m` | number | `0.0` | How far ahead the boundary is when the next world first shows, metres. 0 = auto (where the view fades into fog or distance). |
| `overshoot_m` | number | `1.0` | Metres walked after the boundary passes under the camera (the end of an exported clip). |
| `blend_m` | number | `0.0` | Width of the band where the ground, walls and props of two open worlds mix, metres. 0 = auto. |
| `facade` | [Material](#material) or null | `null` | Material of the face around the opening (a hillside, an end wall, a building front). None = auto: the first world's walls, else the second's. |
| `facade_height` | number | `0.0` | Height of that face, metres, where the first world has no roof to bound it. 0 = auto. |
| `light_spill` | number | `1.0` | How strongly light falls through the opening from one world onto the other (0..2). |
| `adaptation` | number | `1.0` | How much the eye adapts across the boundary (0..1): from a dark tunnel the exit glares, from daylight a cave mouth is black, and both settle as the camera passes through. |
| `camera_blend_m` | number | `4.0` | Metres either side of the boundary over which the camera (eye height, horizon, zoom, bend) changes from one scene's to the other's. |
| `style` | [StyleFrom](#stylefrom) | `"Auto"` | Whose look (pixel size, palette, outline, grade) the frames take. |
| `marker` | [Marker](#marker) | `"Auto"` | A structure at the boundary: an arch, a gate, a portal ring. Auto adds one where the threshold asks for it (a gate, a portal). |
| `seed` | integer | `7` |  |

### Anchor

Where a prop is fixed.

| Value | Meaning |
|---|---|
| `Ground` | Stands on the ground. |
| `Ceiling` | Hangs from the ceiling, or from the top of the walls; stands when there is neither. |

### Branch

One of `Left` \| `Right`.

### Bridge

Bridges: every `spacing` metres the ground beside the path drops away for `length` metres, and
the path crosses on a deck. Walls carry on down into the gap.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `false` |  |
| `spacing` | number | `24.0` | Distance from one bridge to the next, metres (snapped to divide the loop). |
| `length` | number | `8.0` | Length of each bridge, metres. |
| `offset` | number | `6.0` | Where along the loop the first bridge starts, metres. |
| `depth` | number | `10.0` | How far down the bottom of the gap is, metres. |
| `bottom` | [BridgeBottom](#bridgebottom) | `"Ground"` | What is down there. |
| `bottom_color` | integer × 3 | `[150,150,150]` | Colour of water, or of ground far below (multiplied with the verge or path material). |
| `deck` | [Material](#material) | (see the type) | The deck: planks, stone... |
| `railing` | [Railing](#railing) | `"Posts"` |  |
| `rail_height` | number | `1.0` | Railing height, metres. |

### BridgeBottom

| Value | Meaning |
|---|---|
| `Ground` | The verge (or path) material, far below. |
| `Water` | Still water. |
| `Void` | Nothing: a bottomless drop into the void colour. |

### Camera

| Field | Type | Default | Meaning |
|---|---|---|---|
| `eye_height` | number | `1.600000023841858` | Eye height above the path, metres. |
| `horizon` | number | `0.2199999988079071` | Horizon position as a fraction of the canvas height from the top (0.1..0.9). |
| `zoom` | number | `1.0` | Zoom: 1.0 puts the ground at `eye_height` metres ahead on the bottom row. Higher = narrower view. |
| `lens_curve` | number | `0.0` | Screen-space lens bend of the horizon (-1..1); 0 is a flat horizon. |

### Canvas

Output size in pixels. Portrait by default, like a phone held upright.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `width` | integer | `480` |  |
| `height` | integer | `854` |  |

### Ceiling

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `false` |  |
| `height` | number | `3.5999999046325684` |  |
| `material` | [Material](#material) | (see the type) |  |

### Clouds

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `false` |  |
| `count` | integer | `10` |  |
| `drift` | number | `1.0` | Drifts across the sky per loop. Whole numbers wrap each cloud round the sky; any other value (0.4, 1.5) keeps its exact speed, with each cloud forming and dissolving over one loop so the loop stays seamless. |
| `scale` | number | `1.0` |  |
| `opacity` | number | `0.4000000059604645` |  |
| `tint` | integer × 3 | `[226,230,238]` |  |
| `variation` | number | `0.550000011920929` |  |
| `seed` | integer | `0` |  |

### Dither

One of `None` \| `Bayer2` \| `Bayer4` \| `Bayer8`.

### Fixture

A repeating light source along the path: torches, lanterns, fireflies, crystals.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `true` |  |
| `kind` | [FixtureKind](#fixturekind) | `"Torch"` |  |
| `side` | [Side](#side) | `"Both"` |  |
| `mount` | [Mount](#mount) | `"Wall"` |  |
| `height` | number | `2.0999999046325684` | Height of the flame or orb above the path, metres. |
| `spacing` | number | `6.0` | Distance between fixtures along the path, metres (snapped to divide the loop). |
| `offset` | number | `0.0` | Shift of the whole row along the path, metres. |
| `lateral` | number | `0.30000001192092896` | For ground and floating mounts: distance outside the path edge, metres (negative = over the path). |
| `size` | number | `1.0` | Size multiplier for the fixture and flame. |
| `light` | boolean | `true` |  |
| `intensity` | number | `1.0` |  |
| `radius` | number | `6.0` | Reach of the light, metres. |
| `flicker` | number | `1.0` |  |
| `jitter` | number | `0.0` | Random placement wobble, metres. |
| `sprite` | [SpriteRef](#spriteref) | (see the type) |  |
| `seed` | integer | `0` |  |

### FixtureKind

One of `Torch` \| `Lantern` \| `Candle` \| `Brazier` \| `Crystal` \| `Firefly` \| `Magic` \| `GreenFire` \| `IceWisp`.

### Fog

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `true` |  |
| `color` | integer × 3 | `[0,0,0]` |  |
| `distance` | number | `26.0` | Distance at which about two-thirds of a surface is hidden, metres. |
| `match_sky` | boolean | `false` | Use the sky's horizon colour instead of `color` when the sky is on. |

### FogBanks

Patches of thick fog along the path that the camera walks into and out of.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `false` |  |
| `spacing` | number | `12.0` | Distance between banks, metres (snapped to divide the loop). |
| `length` | number | `4.0` | Length of each bank along the path, metres. |
| `density` | number | `0.5` | Thickness inside a bank: optical depth per metre (0.3 = light mist, 1.5 = a wall of fog). |
| `offset` | number | `0.0` |  |

### Fork

Forks. On open ground a branch path splits off at `angle` and runs away into the distance; between
walls the fork is a side passage: an opening in the wall with a dark passage behind it. The walk
itself stays on the main path.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `false` |  |
| `spacing` | number | `24.0` | Distance from one fork to the next, metres (snapped to divide the loop). |
| `offset` | number | `14.0` | Where along the loop the first fork is, metres. |
| `side` | [ForkSide](#forkside) | `"Alternate"` |  |
| `angle` | number | `38.0` | Angle of a branch path from the main path, degrees (open ground). |
| `half_width` | number | `1.100000023841858` | Half the width of a branch path, or of a side passage's opening, metres. |
| `depth` | number | `7.0` | How deep a side passage goes before it is lost in the dark, metres (between walls). |
| `height` | number | `2.5999999046325684` | Height of a side passage, metres (between walls). |

### ForkSide

One of `Left` \| `Right` \| `Alternate`.

### Lighting

| Field | Type | Default | Meaning |
|---|---|---|---|
| `ambient` | number | `0.6000000238418579` | Ambient light level (0..2). |
| `ambient_color` | integer × 3 | `[200,205,220]` |  |
| `void_color` | integer × 3 | `[0,0,0]` | Colour of the far distance and of anything outside the world (dungeon darkness). |
| `fog` | [Fog](#fog) | (see the type) | Distance fog. |
| `bands` | integer | `0` | Light bands: 0 = smooth light, 2..8 = cel-shaded steps. |

### Lightning

Lightning strikes: a flash that lights the whole scene, and a bolt in the sky.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `false` |  |
| `strikes` | integer | `2` | Strikes per loop. |
| `intensity` | number | `1.5` | Brightness of the flash. |
| `color` | integer × 3 | `[205,215,255]` |  |
| `bolts` | boolean | `true` | Draw the bolt in the sky (needs the sky visible). |
| `seed` | integer | `0` |  |

### Marker

One of `Auto` \| `None` \| `Archway` \| `RuinedArch` \| `Gate` \| `Banners` \| `Portal`.

### Material

A tiling surface material generated procedurally.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `pattern` | [Pattern](#pattern) | `"Cobblestone"` |  |
| `base` | integer × 3 | `[80,72,62]` |  |
| `mortar` | integer × 3 | `[34,30,26]` |  |
| `noise` | integer | `10` |  |
| `damage` | number | `0.20000000298023224` |  |
| `seed` | integer | `0` |  |
| `tile_size` | number | `2.4000000953674316` | Metres covered by one repeat of the texture. |
| `rotate` | boolean | `false` |  |
| `brightness` | number | `1.0` | Albedo multiplier. |
| `gloss` | number | `0.0` | How mirror-like a floor is: 0 matte, about 0.3-0.5 wet stone, 0.6 ice, 1 still water. Reflections are traced against the finished frame, so lamps, trees and the sky show in it. |
| `ripples` | number | `0.0` | Ripples (water) or roughness (wet stone) that break reflections up: 0 glassy .. 1 choppy. |

### Moon

| Field | Type | Default | Meaning |
|---|---|---|---|
| `body` | [SkyBody](#skybody) | (see the type) |  |
| `phase` | number | `0.0` | -1 waning .. 0 full .. 1 waxing. |
| `opacity` | number | `0.8999999761581421` |  |
| `craters` | boolean | `true` |  |

### Motion

| Field | Type | Default | Meaning |
|---|---|---|---|
| `loop_length` | number | `24.0` | World distance covered by one loop, metres. Every repeat in the scene is snapped to divide it. |
| `speed` | number | `4.0` | Walking speed for previews and timed exports, metres per second. |
| `fps` | integer | `24` | Frames per second for timed exports. |

### Mount

One of `Wall` \| `Ground` \| `Ceiling` \| `Floating`.

### Next

| Value | Meaning |
|---|---|
| `End` | The last stop. |
| `{"Go": object}` | On to another stop, through a transition. |
| `{"Fork": object}` | A fork: the player chooses the left or the right stop. |

### Outline

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `false` |  |
| `color` | integer × 3 | `[12,10,14]` |  |
| `objects_only` | boolean | `true` | Outline props and fixtures only (true) or also wall and ground edges (false). |

### Palette

| Value | Meaning |
|---|---|
| `Full` | No colour restriction. |
| `{"Named": string}` | A built-in palette by name (see `world::palette::NAMED`). |
| `{"Auto": integer}` | The best `n` colours for this scene, chosen per loop. |
| `{"Custom": list of integer × 3}` |  |

### ParticleKind

One of `Dust` \| `Embers` \| `Fireflies` \| `Rain` \| `Snow` \| `Leaves` \| `Ash` \| `Spores` \| `Sand`.

### Particles

Small moving things in the air: dust, embers, rain, snow, leaves.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `true` |  |
| `kind` | [ParticleKind](#particlekind) | `"Dust"` |  |
| `count` | integer | `120` | Particles per loop length of world. |
| `color` | integer × 3 | `[200,190,170]` |  |
| `size` | number | `1.0` |  |
| `speed` | number | `1.0` | Speed multiplier for the particle's own motion (fall, drift, rise). |
| `seed` | integer | `0` |  |

### PathShape

| Field | Type | Default | Meaning |
|---|---|---|---|
| `half_width` | number | `1.100000023841858` | Half the path width near the camera, metres. |
| `flare` | number | `0.0` | 0 = straight edges in true perspective; up to 0.9 widens the path into the distance (stylised). |
| `bend` | number | `0.0` | Lateral curve of the road ahead (-1 left .. 1 right). |
| `hill` | number | `0.0` | Vertical curve of the road ahead (-1 dips away .. 1 rises). |
| `edge_noise` | number | `0.11999999731779099` | How ragged the path edge is, metres. |
| `edge_dark` | number | `0.3499999940395355` | Darkening toward the path edge (0..1). |
| `material` | [Material](#material) | (see the type) |  |
| `stairs` | [Stairs](#stairs) | (see the type) | Flights of steps the walk climbs (or descends) for ever. |
| `bridge` | [Bridge](#bridge) | (see the type) | Stretches where the ground beside the path falls away and the path crosses on a bridge. |
| `fork` | [Fork](#fork) | (see the type) | Side paths that branch off (open ground), or side passages (between walls). |

### Pattern

One of `Cobblestone` \| `Brick` \| `StoneBlock` \| `Sand` \| `Dirt` \| `Grass` \| `Bark` \| `RockFace` \| `Hedge` \| `Planks` \| `Water` \| `Ice` \| `Plain`.

### Post

| Field | Type | Default | Meaning |
|---|---|---|---|
| `exposure` | number | `1.0` |  |
| `contrast` | number | `1.0` |  |
| `saturation` | number | `1.0` |  |
| `bloom` | number | `0.3499999940395355` | Glow around bright light (0..1). |
| `vignette` | number | `0.3499999940395355` |  |
| `grain` | number | `0.0` |  |
| `tint` | integer × 3 | `[255,255,255]` | Colour multiplied over the final image. |

### PropDef

A prop made from data: an image, several images or folders of them, or a shape drawn from
parts. Lives in `scene.prop_defs` or in a kit (a folder holding kit.json and its images).

| Field | Type | Default | Meaning |
|---|---|---|---|
| `description` | string |  | What it is, in a few words (shown when listing a kit). |
| `sprite` | [SpriteRef](#spriteref) | (see the type) | Images: `path`, or `pool` (PNG files, or folders of them; one is picked per prop). Paths are relative to the file the definition is in. |
| `shape` | list of [ShapePart](#shapepart) | `[]` | Or a shape: parts painted in order, in metres, `x` sideways from the centre and `y` up from the ground (0). |
| `kind` | [PropKind](#propkind) | `"Rock"` | The built-in prop drawn when there is neither an image nor a shape; its height is also the default for images. |
| `tint` | integer × 3 or null | `null` | Colour of the built-in prop; left out, the kind's own. |
| `height` | number | `0.0` | Height in metres at scale 1: 0 = the shape's own height, or the kind's. |
| `anchor` | [Anchor](#anchor) | `"Ground"` |  |
| `glow` | number | `0.0` | Lit from within, 0..1: 0 takes only the scene's light, 1 shows its own colours in the dark. |
| `glow_from` | number | `0.0` | For images: only texels at least this light (0..1) glow, like the flame in a lantern; 0 = the whole prop glows by `glow`. |
| `light` | [PropLight](#proplight) | (see the type) | Light it gives off, like a lamp. |
| `trim` | boolean | `true` | Cut away transparent margins of images, so a sprite stands on its lowest visible pixel. |

### PropKind

One of `Tree` \| `Pine` \| `Bush` \| `Rock` \| `Boulder` \| `Cactus` \| `DeadTree` \| `Mushroom` \| `Pillar` \| `Gravestone` \| `Crystal` \| `Stalagmite` \| `Reeds` \| `Willow` \| `Palm` \| `Obelisk` \| `Icicle` \| `IceSpike`.

### PropLayer

A repeating row (or several rows) of props beside the path.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `true` |  |
| `kind` | [PropKind](#propkind) | `"Tree"` |  |
| `side` | [Side](#side) | `"Both"` |  |
| `lateral` | number | `1.5` | Distance from the path edge to the prop's centre, metres (negative = on the path). |
| `spacing` | number | `6.0` | Distance between props along the path, metres (snapped to divide the loop). |
| `offset` | number | `0.0` |  |
| `rows` | integer | `1` | Extra rows further from the path. |
| `row_spacing` | number | `3.0` |  |
| `jitter` | number | `0.5` | Random sideways placement, metres. |
| `density` | number | `1.0` | Share of slots that hold a prop (0..1), for natural gaps. |
| `scale` | number | `1.0` |  |
| `scale_var` | number | `0.20000000298023224` |  |
| `tint` | integer × 3 | `[40,110,34]` | Colour of procedural props. Left out of a scene file, it is the kind's own colour. |
| `sink` | number | `0.019999999552965164` | How far the base sinks into the ground, as a fraction of height. |
| `shadow` | boolean | `true` |  |
| `shadow_opacity` | number | `0.6000000238418579` |  |
| `sprite` | [SpriteRef](#spriteref) | (see the type) |  |
| `seed` | integer | `1` |  |
| `def` | string |  | A prop described by data instead of `kind`: the name of one in `prop_defs`, or `"<kit folder>#<name>"` for one in a kit (relative to the scene file). Placement (side, spacing, scale...) still comes from this layer. |

### PropLight

Light a prop gives off.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `false` |  |
| `color` | integer × 3 | `[255,190,110]` |  |
| `intensity` | number | `0.800000011920929` |  |
| `radius` | number | `4.0` | Reach, metres. |
| `at` | number | `0.800000011920929` | Where the light sits, as a fraction of the prop's height from its base. |
| `flicker` | number | `0.0` | 0 steady, 1 like a flame. |

### Railing

| Value | Meaning |
|---|---|
| `None` |  |
| `Posts` | Wooden posts with two rails, in the deck material. |
| `Parapet` | A low solid wall, in the wall material (or the path material without walls). |

### SetPiece

A structure spanning the path, repeated along it.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `true` |  |
| `kind` | [SetPieceKind](#setpiecekind) | `"Archway"` |  |
| `spacing` | number | `24.0` | Distance between repeats, metres (snapped to divide the loop; the loop length = once per loop). |
| `offset` | number | `12.0` | Shift along the path, metres. |
| `width` | number | `0.0` | Clear width of the opening, metres. 0 = fit the path (and the gap to the walls). |
| `height` | number | `3.4000000953674316` | Clear height of the opening, metres. |
| `tint` | integer × 3 | `[118,110,100]` | Stone or wood colour. |
| `accent` | integer × 3 | `[150,30,36]` | Cloth, trim or glow colour. |
| `shadow` | number | `0.6000000238418579` | Sun shadow strength, 0..1. |
| `sprite` | [SpriteRef](#spriteref) | (see the type) | An image used instead of the painted structure; it is stretched to span the opening. |
| `seed` | integer | `0` |  |

### SetPieceKind

| Value | Meaning |
|---|---|
| `Archway` | Stone piers and a round arch with a keystone. |
| `RuinedArch` | An archway with its crown fallen in. |
| `Gate` | A gatehouse: heavy piers, a lintel and a raised portcullis. |
| `Banners` | A beam across the path with cloth banners hanging from it. |
| `Portal` | A glowing ring over the path. |

### Shape

A shape in metres: `x` sideways from the centre, `y` up from the ground.

| Value | Meaning |
|---|---|
| `{"Ellipse": object}` |  |
| `{"Rect": object}` |  |
| `{"Poly": object}` | A filled polygon (even-odd). |
| `{"Line": object}` | A straight stroke `width` metres wide. |

### ShapePart

One part of a drawn prop.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `shape` | [Shape](#shape) | (see the type) |  |
| `color` | integer × 3 | `[128,128,128]` |  |
| `shade` | number | `0.5` | Rounding light from the upper left: 0 flat, 1 strong. |
| `cut` | boolean | `false` | Cut a hole through what is painted so far instead of painting. |
| `glow` | number | `0.0` | Lit from within, like lantern glass or a glowing cap: 0 none, 1 its own colour at full strength in the dark, more for something bright. |

### Side

One of `Both` \| `Left` \| `Right` \| `Center`.

### Sky

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `true` |  |
| `top` | integer × 3 | `[70,110,170]` |  |
| `horizon` | integer × 3 | `[200,190,160]` |  |
| `sun` | [SkyBody](#skybody) | (see the type) |  |
| `moon` | [Moon](#moon) | (see the type) |  |
| `stars` | [Stars](#stars) | (see the type) |  |
| `clouds` | [Clouds](#clouds) | (see the type) |  |

### SkyBody

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `false` |  |
| `pos` | number × 2 | `[0.7200000286102295,0.3499999940395355]` | Position in the sky: x 0..1 across, y 0 (top) .. 1 (horizon). |
| `radius` | number | `0.07999999821186066` | Radius as a fraction of the sky height. |
| `color` | integer × 3 | `[255,236,190]` |  |
| `emits_light` | boolean | `true` | Lights the world and casts shadows. |
| `intensity` | number | `1.0` |  |

### SpriteRef

An image file used in place of a procedural shape. Paths are relative to the scene file.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `path` | string | `""` | One image, or empty. |
| `pool` | list of string | `[]` | Several images to pick from at random per instance (overrides `path` when not empty). |
| `flip_x` | boolean | `false` |  |
| `pixelated` | boolean | `false` | Pixel art: sample nearest instead of smoothing. |

### Stairs

Flights of steps along the path, each followed by a landing. The camera climbs them like a
walker (smoothly over each flight), and the loop still closes: the view one flight on is the same.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `false` |  |
| `spacing` | number | `12.0` | Distance from one flight to the next, metres (snapped to divide the loop). |
| `steps` | integer | `8` | Steps per flight. |
| `rise` | number | `0.17000000178813934` | Height of each step, metres (real stairs: 0.15-0.2). |
| `run` | number | `0.3199999928474426` | Depth of each step, metres (real stairs: 0.25-0.35). |
| `offset` | number | `3.0` | Where along the loop the first flight starts, metres. |
| `descending` | boolean | `false` | Walk down the steps instead of up. |

### Stars

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `false` |  |
| `count` | integer | `140` |  |
| `size` | number | `1.0` |  |
| `twinkle` | number | `0.5` |  |
| `seed` | integer | `0` |  |

### Stop

| Field | Type | Default | Meaning |
|---|---|---|---|
| `scene` | string | `""` | Scene file (relative to the journey) or `preset:<name>`. |
| `next` | [Next](#next) | `"End"` |  |

### Style

| Field | Type | Default | Meaning |
|---|---|---|---|
| `pixel_size` | integer | `1` | Size of one art pixel in output pixels: 1 = full resolution, 3 = render at a third and scale up crisply. |
| `palette` | [Palette](#palette) | `"Full"` |  |
| `dither` | [Dither](#dither) | `"None"` |  |
| `dither_strength` | number | `0.5` | 0..1 |
| `outline` | [Outline](#outline) | (see the type) |  |
| `paint` | number | `0.0` | Painterly brush size in art pixels (Kuwahara filter): flat strokes that keep edges. 0 = off, 2..8 typical. |
| `grade` | string | `""` | Colour grade: a built-in name (Warm, Cool, Teal Orange, Faded, Night, Sepia, Vivid) or a .cube LUT file (relative to the scene file). Empty = none. |
| `grade_strength` | number | `1.0` | How much of the grade to apply, 0..1. |
| `paper` | number | `0.0` | Paper or canvas texture over the image, 0..1 (static, so loops stay seamless). |
| `scanlines` | number | `0.0` | CRT scanlines, 0..1. |

### StyleFrom

| Value | Meaning |
|---|---|
| `First`, `Second` |  |
| `Auto` | The look of the world the camera is in (it switches as the camera crosses). |

### Threshold

| Value | Meaning |
|---|---|
| `Auto` |  |
| `Open` | No structure: the ground, walls and props of the two worlds mix over a band. |
| `Doorway` | A clean rectangular opening (a door, a tunnel cut square). |
| `CaveMouth` | A rough, rounded opening in rock, in a hillside when the first world is open. |
| `Gate` | A doorway with a gatehouse standing in it. |
| `Portal` | A glowing ring; the worlds meet sharply inside it. |

### Verge

Ground beside the path. Disabled = void beyond the path edge (walls usually cover it).

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `false` |  |
| `material` | [Material](#material) | (see the type) |  |
| `tufts` | boolean | `false` | Grass tufts along the path edge. |
| `tuft_color` | integer × 3 | `[44,100,30]` |  |
| `tuft_density` | number | `1.0` |  |
| `tuft_height` | number | `0.25` |  |

### Walls

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `true` |  |
| `gap` | number | `0.25` | Gap between the path edge and the wall, metres. |
| `height` | number | `4.5` | Wall height, metres. 0 = taller than anything the camera can see. |
| `base_shadow` | number | `0.550000011920929` | Darkening where the wall meets the ground (0..1). |
| `material` | [Material](#material) | (see the type) |  |

### Weather

Weather events. Both repeat exactly with the loop.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `lightning` | [Lightning](#lightning) | (see the type) |  |
| `fog_banks` | [FogBanks](#fogbanks) | (see the type) |  |

