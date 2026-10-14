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
| `ground_cover` | list of [GroundCover](#groundcover) | `[]` | What grows on the ground: grass, tall grass and wildflowers standing on it, moss lying on it; over the verge, the path or both. Up to three layers. |
| `walls` | [Walls](#walls) | (see the type) |  |
| `ceiling` | [Ceiling](#ceiling) | (see the type) |  |
| `sky` | [Sky](#sky) | (see the type) |  |
| `light` | [Lighting](#lighting) | (see the type) |  |
| `fixtures` | list of [Fixture](#fixture) | (see the type) |  |
| `props` | list of [PropLayer](#proplayer) | `[]` |  |
| `set_pieces` | list of [SetPiece](#setpiece) | `[]` | Structures that span the path at intervals: archways, gates, banners. |
| `companions` | list of [Companion](#companion) | `[]` | Things travelling along with the camera: a ship flying beside the path, a drone, a bird. |
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

### Aurora

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `false` |  |
| `intensity` | number | `1.0` |  |
| `low` | integer × 3 | `[70,255,150]` | Colour of the bright lower edge, the glow over it and the rays rising from it. |
| `high` | integer × 3 | `[225,130,255]` | Colour of the diffuse veil high above the edge. |
| `accent` | integer × 3 | `[255,170,245]` | Colour of the bright knots that flare along the edge. |
| `height` | number | `0.6` | Where the lower edge hangs: 0 high in the sky .. 1 down at the horizon. |
| `tall` | number | `1.0` | How far the veil rises above the edge (1 = about the height of the sky). |
| `speed` | number | `1.0` | How fast the light drifts and flows along the band. |
| `coverage` | number | `0.5` | How much of the band is lit at once: 0 a few patches .. 1 nearly all of it. |
| `rays` | number | `1.0` | Fine vertical rays in the curtain: 0 a smooth glow .. 1 natural .. 2 strongly rayed. |
| `waves` | number | `0.5` | How much the lower edge swells and folds: 0 a straight band. |
| `arc` | number | `1.0` | How much the band bows up in the middle, as the far arc of the auroral oval does. |
| `edge` | number | `1.0` | Brightness of the thin lower edge against the rest. |
| `ground_glow` | number | `0.5` | How much the aurora lights the ground green (a faint, even glow). |
| `seed` | integer | `0` |  |

### BlackHole

A black hole in the space sky. Its gravity bends light: the stars and nebulae behind it are
lensed into arcs round it, and the far side of its accretion disk shows over the top and under
the bottom of the dark shadow, ringed by a thin bright photon ring. The disk's side turning
towards you is brighter and bluer. Matter drifts in from both sides of the frame, speeds up,
swirls into the disk's plane and falls in, stretching and reddening. Planets are not lensed:
keep them clear of it.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `false` |  |
| `pos` | number × 2 | `[0.5,0.45]` | Centre: x 0..1 across the frame, y 0 (top) .. 1 (horizon). |
| `size` | number | `0.12` | Radius of the dark shadow, as a share of the sky's height. |
| `tilt` | number | `84.0` | How edge-on the disk is seen, degrees: 0 face-on, 90 edge-on. |
| `roll` | number | `-8.0` | The disk's roll on screen, degrees (positive turns it anticlockwise). |
| `disk_inner` | number | `3.0` | The disk's inner and outer edge, in horizon radii (3 is the innermost stable orbit). |
| `disk_outer` | number | `11.0` |  |
| `disk_colors` | integer × 3 × 3 | `[[255,246,228],[255,176,96],[190,64,26]]` | The disk's colour from hot (inner) through warm to cool (outer). |
| `brightness` | number | `1.0` |  |
| `doppler` | number | `0.9` | How much brighter and bluer the side turning towards you is: 0 even .. 1 physical. |
| `spin` | integer | `4` | Turns the disk's inner edge makes in one loop (the outer part turns slower). Whole turns keep the loop seamless; negative spins it the other way. |
| `lensing` | number | `1.0` | How strongly it bends the light behind it: 1 natural, 0 none. |
| `infall` | integer | `18` | How many streaks of matter are falling in at once, from both sides (0 none, at most 48). |
| `infall_speed` | integer | `1` | Whole trips each streak makes per loop, at least (some make one more). |
| `infall_color` | integer × 3 | `[255,214,170]` |  |
| `nodes` | integer | `0` | How many nodes like ball_swing_game's hook nodes fall in too (0 none): smaller and dimmer than the real ones, shrinking as they recede and stretched along their way near the end. |
| `node_size` | number | `0.018` | A node's width as it comes in, as a share of the frame's width (the game's real nodes are about 0.03). |
| `node_colors` | integer × 3 × 3 | `[[22,34,66],[90,230,210],[156,126,250]]` | The nodes' dark body and their two glowing rings. |
| `distance` | number | `30.0` | How many times further away the hole is than the nodes start (they float in at play depth, then are pulled back to it, shrinking with distance). |
| `light` | number | `0.5` | How much its disk lights the scene, in its warm colour (0 none). |
| `seed` | integer | `0` |  |

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
| `water_level` | number | `10.0` | With a Water bottom: how far below the deck the water lies, metres (never deeper than `depth`). |
| `bottom_color` | integer × 3 | `[150,150,150]` | Colour of water, or of ground far below (multiplied with the verge or path material). |
| `deck` | [Material](#material) | (see the type) | The deck: planks, stone... |
| `railing` | [Railing](#railing) | `"Posts"` |  |
| `rail_height` | number | `1.0` | Railing height, metres. |
| `end_pillars` | boolean | `true` | A taller pillar or newel post where each railing starts and ends. |
| `rail_color` | integer × 3 | `[34,35,38]` | Colour of iron bars, or of rope. |

### BridgeBottom

| Value | Meaning |
|---|---|
| `Ground` | The verge (or path) material, far below. |
| `Water` | Still water. |
| `Void` | Nothing: a bottomless drop into the void colour. |

### Camera

| Field | Type | Default | Meaning |
|---|---|---|---|
| `eye_height` | number | `1.6` | Eye height above the path, metres. |
| `horizon` | number | `0.22` | Horizon position as a fraction of the canvas height from the top (0.1..0.9). |
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
| `height` | number | `3.6` |  |
| `material` | [Material](#material) | (see the type) |  |

### CloudKind

What kind of cloud a layer is. Whatever the kind, cloud that rain or snow is falling from
(`weather.precipitation`) is drawn heavier and darker.

| Value | Meaning |
|---|---|
| `Cumulus` | Fair-weather heaps standing in rows on level bases: bright billowing tops, grey beneath. |
| `Towering` | Tall heaps building up from the horizon, dark under them: the sky before a storm. |
| `Stratus` | Long low bands lying along the horizon. |
| `Mackerel` | A high sheet broken into ripples and small flakes: a mackerel sky. |
| `Cirrus` | Thin bright streaks very high up, drawn out towards the horizon. |
| `Overcast` | A grey deck over the whole sky, lighter and darker in patches. |
| `Rain` | Rain cloud: a low dark deck, heavy-bellied and ragged underneath. |
| `Snow` | Snow cloud: an even, pale, heavy sky with hardly a feature in it. |
| `Storm` | Storm cloud: a black deck with ragged scud driving along under it. |
| `Soft` | Soft round puffs with no detail, as PathForge drew every cloud before; suits flat and pixel-art styles. (The main layer only: in `cloud_layers` it reads as cumulus.) |

### Clouds

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `false` |  |
| `kind` | [CloudKind](#cloudkind) | `"Cumulus"` |  |
| `cover` | number | `0.45` | How much of the sky they fill (0..1). (`Soft` clouds are counted instead: `count`.) |
| `count` | integer | `10` | How many `Soft` clouds. |
| `drift` | number | `1.0` | Drifts across the sky per loop. Whole numbers wrap each cloud round the sky; any other value (0.4, 1.5) keeps its exact speed, with each cloud forming and dissolving over one loop so the loop stays seamless. |
| `scale` | number | `1.0` |  |
| `opacity` | number | `0.5` |  |
| `tint` | integer × 3 | `[226,230,238]` |  |
| `variation` | number | `0.55` | How ragged and billowing their outlines are (0..1). |
| `shade` | number | `0.55` | How dark their thick parts and undersides are (0..1). |
| `shadows` | number | `0.0` | Shadows of clouds drifting over the ground (0..1; needs a light-giving sun). They drift with the wind when there is one, and work with the clouds themselves switched off too. |
| `seed` | integer | `0` |  |

### Companion

Something travelling along with the camera at the walk's own speed, so it keeps its place in
the frame: a ship flying beside the path. It follows the path round bends, bobs and weaves in
whole cycles per loop, and its engines light what is near.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `true` |  |
| `offset` | number × 3 | `[-2.4,2.4,10.0]` | Where it flies, metres: sideways from the path's centre (right positive), height above the path, and distance ahead of the camera. |
| `size` | number | `3.2` | Its wingspan, metres (with a sprite: its height). |
| `color` | integer × 3 | `[128,134,146]` | The drawn ship's hull, trim and engine-glow colours. |
| `accent` | integer × 3 | `[56,104,180]` |  |
| `engine` | integer × 3 | `[110,180,255]` |  |
| `bob` | number | `0.25` | How far it rises and falls (metres), and how many times per loop. |
| `bob_cycles` | integer | `2` |  |
| `weave` | number | `0.4` | How far it drifts sideways (metres), and how many times per loop. |
| `weave_cycles` | integer | `1` |  |
| `light` | number | `1.0` | How strongly its engines light what is near (0 not at all). |
| `sprite` | [SpriteRef](#spriteref) | (see the type) | An image used instead of the drawn ship (seen from behind, as it flies ahead). |
| `glow_from` | number | `0.8` | With a sprite: texels at least this light (0..1) glow, as engines and windows do (0 none). |
| `seed` | integer | `0` |  |

### CoverKind

What a layer of ground cover is.

| Value | Meaning |
|---|---|
| `Grass` | Short grass: a lawn, a mown verge. |
| `TallGrass` | Long grass in tall clumps: a meadow, a field gone wild. |
| `Wildflowers` | Flowers on thin stems, in the flower colour, a related one and white. |
| `Moss` | Moss lying on the ground in soft cushions (no height). |

### CoverOn

Where a layer of ground cover grows.

One of `Verge` \| `Path` \| `Both`.

### Dither

One of `None` \| `Bayer2` \| `Bayer4` \| `Bayer8`.

### Drips

Drops falling from the ceiling, or from the tops of the walls where there is no ceiling.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `false` |  |
| `rate` | number | `1.0` | Drips per metre of path per loop. |
| `color` | integer × 3 | `[190,205,220]` |  |
| `seed` | integer | `0` |  |

### EdgeLights

Lines of light set into the path along both its edges. They glow, and light the floor round
them; far away they widen to a pixel and dim to match, so they do not flicker.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `false` |  |
| `color` | integer × 3 | `[80,210,255]` |  |
| `strength` | number | `1.0` | Brightness: 1 bright. |
| `width` | number | `0.05` | Width of each line, metres. |
| `inset` | number | `0.08` | How far in from the path's edge, metres. |
| `dash` | number | `0.0` | Dashes: one dash and one gap every this many metres (snapped to divide the loop); 0 an unbroken line. |
| `flow` | integer | `0` | How many dashes the lights run ahead in one loop, on top of the walk (negative: toward the camera). Whole numbers keep the loop seamless. |

### Fixture

A repeating light source along the path: torches, lanterns, fireflies, crystals.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `true` |  |
| `kind` | [FixtureKind](#fixturekind) | `"Torch"` |  |
| `side` | [Side](#side) | `"Both"` |  |
| `mount` | [Mount](#mount) | `"Wall"` |  |
| `height` | number | `2.1` | Height of the flame or orb above the path, metres. |
| `spacing` | number | `6.0` | Distance between fixtures along the path, metres (snapped to divide the loop). |
| `offset` | number | `0.0` | Shift of the whole row along the path, metres. |
| `lateral` | number | `0.3` | For ground and floating mounts: distance outside the path edge, metres (negative = over the path). |
| `size` | number | `1.0` | Size multiplier for the fixture and flame. |
| `light` | boolean | `true` |  |
| `intensity` | number | `1.0` |  |
| `radius` | number | `6.0` | Reach of the light, metres. |
| `flicker` | number | `1.0` |  |
| `jitter` | number | `0.0` | Random placement wobble, metres: sideways by this much either way and up or down by half of it. Floating lights also drift about by it. |
| `jitter_along` | number | `0.0` | Random shift along the path, as a fraction of the spacing (0 = an even row, 1 = anywhere between its neighbours). |
| `height_var` | number | `0.0` | Random height, metres either way (on top of `jitter`'s): fireflies and wisps at every height. |
| `size_var` | number | `0.0` | Random size, as a fraction either way (0.3 = from 0.7 to 1.3 of `size`). |
| `intensity_var` | number | `0.0` | Random brightness, as a fraction either way. |
| `color_var` | number | `0.0` | Random colour, 0..1: each one a little warmer or cooler than the next. |
| `density` | number | `1.0` | Fraction of places that hold a light (1 = every place, 0.5 = half of them, at random). |
| `cluster` | integer | `1` | How many stand at each place (1..8). They share the place's random settings, so with `jitter`, `jitter_along` and `height_var` they scatter into a loose swarm. Each is a light of its own: turn `light` off, or the spacing up, when there are many. |
| `stagger` | number | `0.0` | With lights on both sides: how far the right-hand row is shifted along the path, as a fraction of the spacing (0 = in pairs, 0.5 = left and right take turns). |
| `tint` | integer × 3 | `[255,255,255]` | Colour multiplied over the kind's own light and flame (white = as it is). |
| `post` | [LampPost](#lamppost) | `"Auto"` | What it stands on or hangs from where there is no wall (ground mounts, and wall mounts in a scene without walls). |
| `place` | [LightPlace](#lightplace) | `"Anywhere"` | Where along the path the row is allowed to stand. |
| `set_piece` | integer | `0` | For `place: SetPieces`: which of `set_pieces` the lights go with (0 = the first). |
| `sprite` | [SpriteRef](#spriteref) | (see the type) |  |
| `seed` | integer | `0` |  |

### FixtureKind

One of `Torch` \| `Lantern` \| `Candle` \| `Brazier` \| `Crystal` \| `Firefly` \| `Magic` \| `GreenFire` \| `IceWisp` \| `StreetLamp` \| `PaperLantern` \| `Campfire` \| `Glowshroom` \| `Beacon` \| `Chandelier`.

### Flare

Lens flare from the sun and bright lights.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `style` | [FlareStyle](#flarestyle) | `"None"` |  |
| `amount` | number | `0.5` | Strength (0..2). |

### FlareStyle

The look of a lens flare.

| Value | Meaning |
|---|---|
| `None` |  |
| `Star` | Four fine rays from every bright light, as through a star filter or a stopped-down lens. |
| `Starburst` | Six rays. |
| `Anamorphic` | A long horizontal blue streak, as an anamorphic cinema lens makes. |
| `Ghosts` | Faint coloured reflections of the light across the frame from it, and a wide ring. |
| `Cinematic` | The streak and the ghosts together. |

### Floating

Things floating on a waterway.

| Value | Meaning |
|---|---|
| `None` |  |
| `Foam` | Clumps of foam. |
| `Leaves` | Fallen leaves. |
| `LilyPads` | Lily pads, a few in flower. |
| `Ice` | Ice floes: flat straight-sided plates, snow on top and bare blue ice round the rim. |

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
itself stays on the main path. `style` chooses how a branch leaves on open ground: through a
gap in the path's edge, or by the road itself dividing.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `false` |  |
| `spacing` | number | `24.0` | Distance from one fork to the next, metres (snapped to divide the loop). |
| `offset` | number | `14.0` | Where along the loop the first fork is, metres. |
| `side` | [ForkSide](#forkside) | `"Alternate"` |  |
| `style` | [ForkStyle](#forkstyle) | `"Side"` | How a branch leaves the road on open ground (between walls a fork is always a side passage). |
| `angle` | number | `38.0` | Angle of a branch path from the main path, degrees (open ground). |
| `half_width` | number | `1.1` | Half the width of a branch path, or of a side passage's opening, metres. |
| `depth` | number | `7.0` | How deep a side passage goes before it is lost in the dark, metres (between walls). |
| `height` | number | `2.6` | Height of a side passage, metres (between walls). |

### ForkSide

Which side a branch leaves on: one side, alternating from fork to fork, or both at once (the
road divides three ways with `ForkStyle::Split`).

One of `Left` \| `Right` \| `Alternate` \| `Both`.

### ForkStyle

How a branch leaves the road on open ground.

| Value | Meaning |
|---|---|
| `Side` | A side path through a gap in the road's edge. |
| `Split` | The road itself divides: it widens, stays one paved surface for a stretch, then parts round a grass point, each path running on alone. |

### GroundCover

One layer of ground cover. It stands on the ground itself (not at the path's edge only, as
`verge.tufts` do): each blade has its place, hides what is behind it and leans in the wind.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `true` |  |
| `kind` | [CoverKind](#coverkind) | `"Grass"` |  |
| `on` | [CoverOn](#coveron) | `"Verge"` |  |
| `density` | number | `0.8` | How much of the ground it takes (0..1). |
| `height` | number | `0.12` | How tall it stands, metres (grass about 0.12, tall grass 0.45, flowers 0.3; not moss). |
| `color` | integer × 3 | `[40,92,30]` | Its colour: blades and stems, or the moss. |
| `color2` | integer × 3 | `[112,150,52]` | Grass: the colour of the tips. Wildflowers: the flowers. Moss: its second shade. |
| `patchiness` | number | `0.2` | 0 grows evenly; towards 1 it grows in patches with bare ground between. |
| `seed` | integer | `0` |  |

### HeatShimmer

Heat haze: the picture wavers over the distant ground and just above the horizon.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `false` |  |
| `strength` | number | `0.5` | How far the picture wavers (0..1). |
| `speed` | number | `1.0` | How fast it wavers. |

### LampPost

What a light stands on or hangs from where there is no wall for it.

| Value | Meaning |
|---|---|
| `Auto` | The kind's own: a street lamp on a pole, a paper lantern from a crook, a wall light with no wall on a pole, anything else set above the ground on a stone pedestal. |
| `Off` | Nothing: the light hangs in the air. |
| `Pole` | A slim iron pole: under a lamp, or beside a torch or lantern at its bracket. |
| `Crook` | A tall post with an arm curling over towards the path, the light hanging from its end. |
| `Pedestal` | A stone pedestal under it. |

### Lens

Something on the camera's lens.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `false` |  |
| `kind` | [LensKind](#lenskind) | `"Drops"` |  |
| `amount` | number | `0.5` | How much rain is on the glass, or how far the frost reaches in (0..1). |
| `size` | number | `1.0` | Size of the drops (1 = as on a camera lens; larger reads as a windscreen close to). |
| `trails` | number | `0.6` | Share of the heavy drops that run down the glass and leave a trail (0..1). |
| `blur` | number | `0.35` | How far out of focus the wet glass is between the drops and their trails (0 = clear). |
| `seed` | integer | `0` |  |

### LensKind

| Value | Meaning |
|---|---|
| `Drops` | Rain on the glass, as a lens or a windscreen shows it: beads gathering all over it, heavy drops that land, hang and run down leaving wet trails, each drop holding the scene upside down, and the wet glass between them a little out of focus. |
| `Frost` | Frost creeping in from the edges. |

### Lighting

| Field | Type | Default | Meaning |
|---|---|---|---|
| `ambient` | number | `0.6` | Ambient light level (0..2). |
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

### LightPlace

Where along the path a row of lights is allowed to stand.

| Value | Meaning |
|---|---|
| `Anywhere` |  |
| `Bridges` | Only on the path's bridges (`path.bridge`). |
| `OffBridges` | Everywhere but on the bridges. |
| `SetPieces` | One at each repeat of a set piece (`set_piece` says which): torches on every arch, a lantern at every gate. The set piece's spacing replaces `spacing`, and `offset` is measured from it. |

### LightShafts

Light scattered by the air. Stronger in fog, mist, haze and dust.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `false` |  |
| `sun` | number | `0.6` | Rays from the sun past trees, walls and arches (needs the sun in the sky). |
| `lamps` | number | `0.4` | Glowing haloes in the air round lamps, torches and other lights. |

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
| `damage` | number | `0.2` |  |
| `seed` | integer | `0` |  |
| `tile_size` | number | `2.4` | Metres covered by one repeat of the texture. |
| `rotate` | boolean | `false` |  |
| `brightness` | number | `1.0` | Albedo multiplier. |
| `gloss` | number | `0.0` | How mirror-like a floor is: 0 matte, about 0.3-0.5 wet stone, 0.6 ice, 1 still water. Reflections are traced against the finished frame, so lamps, trees and the sky show in it. |
| `ripples` | number | `0.0` | Ripples (water) or roughness (wet stone) that break reflections up: 0 glassy .. 1 choppy. |
| `glow` | number | `0.0` | How brightly the lit parts of a futuristic pattern shine (Panels' light bars, Grid lines, Hex seams, Circuit traces, in the `mortar` colour): 0 they are only inlays, 1 bright. |

### Mist

Mist lying low over the ground, drifting, with wisps rising from it.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `false` |  |
| `height` | number | `0.8` | Height of the mist above the ground, metres. |
| `density` | number | `0.25` | Thickness: optical depth per metre inside it (0.2 thin .. 1.5 thick). |
| `color` | integer × 3 | `[205,210,220]` |  |
| `patchiness` | number | `0.6` | How patchy it is (0 even .. 1 in drifting banks). |
| `wisps` | number | `0.4` | Wisps rising from it (0..1). |
| `seed` | integer | `0` |  |
| `soft` | number | `0.0` | How it thins with height: 0 an even layer up to `height`, 1 thickest at the ground and thinning smoothly to nothing at `height` (ground fog, dew burning off in the morning sun). |
| `over` | [MistOver](#mistover) | `"Everywhere"` | Where it lies: everywhere, or only over the path (a river's water, a road) and `spread` metres past its edges, fading out across them. |
| `spread` | number | `1.0` | How far past the path's edges mist `over` the path reaches, metres. |
| `glow` | number | `0.0` | How much brighter it glows looking towards the sun, as mist does lit from behind (0..2). |

### MistOver

Where mist lies.

| Value | Meaning |
|---|---|
| `Everywhere` |  |
| `Path` | Over the path only: the water of a waterway, or the road. |

### Moon

| Field | Type | Default | Meaning |
|---|---|---|---|
| `body` | [SkyBody](#skybody) | (see the type) |  |
| `phase` | number | `0.0` | -1 waning .. 0 full .. 1 waxing. |
| `opacity` | number | `0.9` |  |
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

### Orbit

Props set round the path instead of beside it: each stands on a ring about the path's line, the
row winding round it like the thread of a screw, the whole of it turning - rocks and wreckage
spiralling down a wormhole's throat. `side: Both` makes two strands opposite each other, `rows`
more strands between; `spacing`, `density` and the rest work as they do for a row.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `false` |  |
| `radius` | number | `4.0` | The ring's radius, metres. |
| `radius_var` | number | `0.8` | Random spread of the radius, metres either way. |
| `height` | number | `0.0` | Height of the ring's centre above the path, metres (0 = the camera's eye height, which is the middle of a tunnel). |
| `twist` | integer | `1` | Whole turns each strand winds round the path along one loop length. |
| `spin` | integer | `1` | Whole turns the ring makes in one loop. |
| `stray` | number | `0.08` | How far each one strays from its strand, in turns either way (0 = a clean spiral, 0.5 = anywhere round the ring). |
| `with_tunnel` | boolean | `true` | Take `twist` and `spin` from `sky.tunnel`, so the props wind and turn with the wormhole. |

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

One of `Dust` \| `Embers` \| `Fireflies` \| `Rain` \| `Snow` \| `Leaves` \| `Ash` \| `Spores` \| `Sand` \| `Petals`.

### Particles

Small moving things in the air: dust, embers, rain, snow, leaves, petals. They drift with the wind
when there is one. For weather that soaks or covers the ground, use `weather.precipitation`.

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
| `half_width` | number | `1.1` | Half the path width near the camera, metres. |
| `flare` | number | `0.0` | 0 = straight edges in true perspective; up to 0.9 widens the path into the distance (stylised). |
| `bend` | number | `0.0` | Lateral curve of the road ahead (-1 left .. 1 right). |
| `winding` | [Winding](#winding) | (see the type) | The path winding left and right as it goes, over and over. |
| `hill` | number | `0.0` | Vertical curve of the road ahead (-1 dips away .. 1 rises). |
| `edge_noise` | number | `0.12` | How ragged the path edge is, metres. |
| `edge_dark` | number | `0.35` | Darkening toward the path edge (0..1). |
| `material` | [Material](#material) | (see the type) |  |
| `stairs` | [Stairs](#stairs) | (see the type) | Flights of steps the walk climbs (or descends) for ever. |
| `bridge` | [Bridge](#bridge) | (see the type) | Stretches where the ground beside the path falls away and the path crosses on a bridge. |
| `fork` | [Fork](#fork) | (see the type) | Side paths that branch off (open ground), or side passages (between walls). |
| `edge_lights` | [EdgeLights](#edgelights) | (see the type) | Glowing lines along both edges of the path (and a bridge's deck): a lit walkway. |
| `surface` | boolean | `true` | Draw the path at all. Off, there is no ground, verge or bridge: the camera flies the route through open air or space (a ship's flight path), and with `sky.space.below` or a tunnel the sky fills everything round it. |
| `waterway` | [Waterway](#waterway) | (see the type) | The path as a waterway, for boats and swimmers (give `material` the Water pattern). |

### Pattern

One of `Cobblestone` \| `Brick` \| `StoneBlock` \| `Sand` \| `Dirt` \| `Grass` \| `Bark` \| `RockFace` \| `Hedge` \| `Planks` \| `Water` \| `Ice` \| `Plain` \| `Panels` \| `Grid` \| `Hex` \| `Circuit`.

### Planet

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `true` |  |
| `kind` | [PlanetKind](#planetkind) | `"Rocky"` |  |
| `pos` | number × 2 | `[0.7,0.4]` | Centre: x 0..1 across the frame, y 0 (top) .. 1 (horizon); past 1 it sits below the horizon, which shows only with `space.below`. |
| `radius` | number | `0.25` | Radius as a share of the sky's height. |
| `color` | integer × 3 | `[150,130,115]` |  |
| `color2` | integer × 3 | `[205,190,170]` |  |
| `atmosphere` | integer × 3 | `[120,170,255]` | The glow of its air round the rim, and how strong (0 airless). |
| `atmosphere_strength` | number | `0.0` |  |
| `light_angle` | number | `30.0` | Where its sunlight comes from across the frame, in degrees: 0 from the right, 90 from above. |
| `night` | number | `0.4` | How much of the face is in night: 0 fully lit .. 0.5 half .. 1 a thin crescent. |
| `tilt` | number | `15.0` | Tilt of its axis (and its bands), degrees. |
| `spin` | integer | `0` | Whole turns it spins in one loop (0 still; it must be whole for the loop to close). |
| `clouds` | number | `0.5` | Earth and gas giants: how much white cloud. |
| `city_lights` | number | `0.0` | Earth: city lights on the night side. |
| `rings` | [Rings](#rings) | (see the type) |  |
| `seed` | integer | `0` |  |

### PlanetKind

What a planet's surface is made of.

| Value | Meaning |
|---|---|
| `Rocky` | Cratered rock: `color` lowlands, `color2` highlands. |
| `Gas` | A gas giant in bands of `color` and `color2`, with a storm. |
| `Earth` | Oceans (`color`), land (`color2`), ice caps and white cloud; city lights on the night side. |
| `Ice` | Pale ice (`color`) with bright cracks (`color2`). |
| `Lava` | Dark crust (`color`) split by glowing lava (`color2`), which glows on the night side too. |

### Post

| Field | Type | Default | Meaning |
|---|---|---|---|
| `exposure` | number | `1.0` |  |
| `contrast` | number | `1.0` |  |
| `saturation` | number | `1.0` |  |
| `bloom` | number | `0.35` | Glow around bright light, as a lens scatters it: a tight glow round every flame, the sun, a glint or a lit panel, falling off into a wide soft veil (0..1; in the world only light brighter than white paper spreads, as exposed, so raising `exposure` makes more of it glow). Under a dark sky the moon, an aurora and the brighter stars glow too. |
| `vignette` | number | `0.35` |  |
| `grain` | number | `0.0` |  |
| `tint` | integer × 3 | `[255,255,255]` | Colour multiplied over the final image. |
| `flare` | [Flare](#flare) | `{"style":"None","amount":0.5}` | What bright light does in the lens besides bloom. |

### Precipitation

Rain, snow, sleet or hail. It falls wherever the sky is open (not under a ceiling).

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `false` |  |
| `kind` | [PrecipKind](#precipkind) | `"Rain"` |  |
| `intensity` | number | `0.5` | How hard it falls: 0.1 a few drops or flakes, 0.5 steady, 1 a downpour or a blizzard. |
| `size` | number | `1.0` | Size of the drops, flakes or hailstones. |
| `tint` | integer × 3 | `[255,255,255]` | Colour multiplied over the drops or flakes (white = their natural colour in the scene's light). |
| `wetness` | number | `0.8` | Rain and sleet: how wet the ground gets, darker and glossy (0..1). |
| `puddles` | number | `0.3` | Rain and sleet: puddles on the path and verge that mirror the scene, with rings where drops land (0..1 = how much ground they cover). |
| `splashes` | number | `0.6` | Rain, sleet and hail: splashes where drops land (0..1). |
| `cover` | number | `0.8` | Snow and sleet: snow lying on the ground, the verges, the tops of walls and props (0..1). |
| `track` | number | `0.6` | A trodden track down the middle of the path in lying snow (0 = untouched .. 1 = trodden to slush). |
| `haze` | number | `0.5` | Haze of a heavy fall: curtains of rain in the distance, a blizzard's whiteout (0..1). |
| `seed` | integer | `0` |  |

### PrecipKind

| Value | Meaning |
|---|---|
| `Rain`, `Snow`, `Hail` |  |
| `Sleet` | Wet snow mixed with rain. |

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

One of `Tree` \| `Pine` \| `Bush` \| `Rock` \| `Boulder` \| `Cactus` \| `DeadTree` \| `Mushroom` \| `Pillar` \| `Gravestone` \| `Crystal` \| `Stalagmite` \| `Reeds` \| `Willow` \| `Palm` \| `Obelisk` \| `Icicle` \| `IceSpike` \| `Asteroid` \| `Satellite` \| `Debris` \| `CargoPod` \| `Comet`.

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
| `jitter_along` | number | `0.0` | Random shift along the path, as a fraction of the spacing (0 = an even row, 1 = anywhere between its neighbours). Rows beyond the first are always shifted by at least 0.8. |
| `stagger` | number | `0.0` | With props on both sides: how far the right-hand side is shifted along the path, as a fraction of the spacing (0 = in pairs, 0.5 = left and right take turns). |
| `density` | number | `1.0` | Share of slots that hold a prop (0..1), for natural gaps. |
| `scale` | number | `1.0` |  |
| `scale_var` | number | `0.2` |  |
| `lean_var` | number | `0.0` | How far each one leans, at random to either side: its top moves by up to this fraction of its height, the trunk bowing over (0.04 is a natural stand of trees; rocks and pillars want 0). |
| `tint_var` | number | `0.12` | How much each one's colour differs from its neighbours' (0 all alike, 0.12 natural, 0.4 strongly): some lighter or darker, some a little warmer or cooler. |
| `tint` | integer × 3 | `[40,110,34]` | Colour of procedural props. Left out of a scene file, it is the kind's own colour. |
| `sink` | number | `0.02` | How far the base sinks into the ground, as a fraction of height. |
| `float` | number | `0.0` | Floating props (asteroids, drifting islands, lanterns): height above the ground, metres, and a random spread either way. Floating props hang over a bridge's drop too, and cast no sun shadow. |
| `float_var` | number | `0.0` |  |
| `orbit` | [Orbit](#orbit) | (see the type) | Set round the path in a ring instead of in rows beside it. |
| `shadow` | boolean | `true` |  |
| `shadow_opacity` | number | `0.6` |  |
| `sprite` | [SpriteRef](#spriteref) | (see the type) |  |
| `seed` | integer | `1` |  |
| `def` | string |  | A prop described by data instead of `kind`: the name of one in `prop_defs`, or `"<kit folder>#<name>"` for one in a kit (relative to the scene file). Placement (side, spacing, scale...) still comes from this layer. |

### PropLight

Light a prop gives off.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `false` |  |
| `color` | integer × 3 | `[255,190,110]` |  |
| `intensity` | number | `0.8` |  |
| `radius` | number | `4.0` | Reach, metres. |
| `at` | number | `0.8` | Where the light sits, as a fraction of the prop's height from its base. |
| `flicker` | number | `0.0` | 0 steady, 1 like a flame. |

### Railing

| Value | Meaning |
|---|---|
| `None` |  |
| `Posts` | Wooden posts with two rails, in the deck material. |
| `Parapet` | A low solid wall with a coping, in the wall material (or the path material without walls). |
| `Balustrade` | Stone balusters on a plinth under a handrail, in the wall (or path) material. |
| `Iron` | Iron bars between rails, on a stone kerb, in `rail_color`. |
| `Rope` | Ropes sagging between wooden posts, in `rail_color`. |

### Rainbow

A rainbow: an arc round the point opposite the sun, with a faint second bow outside it.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `false` |  |
| `intensity` | number | `0.5` |  |
| `x` | number | `0.5` | Where its centre is across the sky (0 left .. 1 right). |
| `size` | number | `1.0` | Size: 1 spans the frame's width (a real 42-degree bow is wider than a portrait view). |
| `double` | boolean | `true` | Draw the fainter second bow outside the first. |

### Rings

A ring system round a planet, its near half in front of the planet and its far half behind.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `false` |  |
| `inner` | number | `1.35` | Inner and outer edge, in planet radii. |
| `outer` | number | `2.3` |  |
| `open` | number | `0.25` | How far open they are seen: 0 edge-on .. 1 face-on. |
| `angle` | number | `-12.0` | Tilt of the ring plane across the frame, degrees. |
| `color` | integer × 3 | `[220,200,170]` |  |
| `opacity` | number | `0.8` |  |

### Sandstorm

A sandstorm: sand streaming past on the wind (to the right unless the wind says otherwise),
a sand-coloured haze that hides the distance, and the sun dimmed to a disc.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `false` |  |
| `intensity` | number | `0.5` | 0.2 blowing sand .. 1 a wall of sand. |
| `color` | integer × 3 | `[214,172,116]` |  |
| `seed` | integer | `0` |  |

### SetPiece

A structure spanning the path, repeated along it.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `true` |  |
| `kind` | [SetPieceKind](#setpiecekind) | `"Archway"` |  |
| `spacing` | number | `24.0` | Distance between repeats, metres (snapped to divide the loop; the loop length = once per loop). |
| `offset` | number | `12.0` | Shift along the path, metres. |
| `width` | number | `0.0` | Clear width of the opening, metres. 0 = fit the path (and the gap to the walls). |
| `height` | number | `3.4` | Clear height of the opening, metres. |
| `tint` | integer × 3 | `[118,110,100]` | Stone or wood colour. |
| `accent` | integer × 3 | `[150,30,36]` | Cloth, trim or glow colour. |
| `shadow` | number | `0.6` | Sun shadow strength, 0..1. |
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
| `shape` | [Shape](#shape) | `{"Rect":{"min":[-0.1,0.0],"max":[0.1,1.0]}}` |  |
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
| `cloud_layers` | list of [Clouds](#clouds) | `[]` | More layers of cloud over `clouds`, drawn in order (up to three): cirrus high over cumulus, heaps under an overcast. |
| `aurora` | [Aurora](#aurora) | (see the type) | Northern lights: curtains of green and violet light rippling across the sky. |
| `rainbow` | [Rainbow](#rainbow) | (see the type) |  |
| `space` | [Space](#space) | (see the type) | Deep space: a dense starfield, nebulae, a galaxy band and planets. |
| `tunnel` | [Tunnel](#tunnel) | (see the type) | A tunnel all round the path in place of the sky: hyperspace or a wormhole. |

### SkyBody

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `false` |  |
| `pos` | number × 2 | `[0.72,0.35]` | Position in the sky: x 0..1 across, y 0 (top) .. 1 (horizon). |
| `radius` | number | `0.045` | Radius as a fraction of the sky height. |
| `color` | integer × 3 | `[255,236,190]` |  |
| `emits_light` | boolean | `true` | Lights the world and casts shadows. |
| `intensity` | number | `1.0` |  |

### Space

The sky seen from space. It sits at infinity, so it stays put while the camera walks.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `false` |  |
| `below` | boolean | `true` | Space all round: where nothing is drawn below the horizon (a bridge's bottomless drop, the gaps beside a floating walkway) shows space too, instead of the void colour. The sky's `top`..`horizon` gradient is mirrored below the horizon. |
| `stars` | number | `1.0` | The dense field of faint stars: 0 none .. 1 a rich field .. 2 a crowded one. |
| `star_brightness` | number | `1.0` |  |
| `star_colors` | number | `1.0` | The stars' colours: 1 hot blue-white to cool orange; 0 all the faint blue-white of ball_swing_game's starfield. |
| `nebula` | number | `0.6` | Glowing gas clouds: strength 0 .. 2, two colours, and size (1 about a sky's height across). |
| `nebula_colors` | integer × 3 × 2 | `[[190,70,200],[60,120,255]]` |  |
| `nebula_scale` | number | `1.0` |  |
| `galaxy` | number | `0.7` | The band of a galaxy seen edge-on, with dark dust lanes and a brighter core. |
| `galaxy_color` | integer × 3 | `[235,225,255]` |  |
| `galaxy_angle` | number | `18.0` | Tilt of the band in degrees (0 level, positive rising to the right). |
| `galaxy_width` | number | `0.22` | Its width, as a share of the sky's height. |
| `galaxy_height` | number | `0.55` | Where the band crosses the middle of the frame, in sky heights above the horizon. |
| `galaxy_core` | number | `0.25` | Where its bright core sits along the band: -1 left .. 1 right. |
| `planets` | list of [Planet](#planet) | `[]` | Up to three planets, drawn in order (later ones in front). |
| `black_hole` | [BlackHole](#blackhole) | (see the type) | A black hole: its shadow, a glowing disk lensed round it, and matter falling in. |
| `seed` | integer | `0` |  |

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
| `rise` | number | `0.17` | Height of each step, metres (real stairs: 0.15-0.2). |
| `run` | number | `0.32` | Depth of each step, metres (real stairs: 0.25-0.35). |
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

### Tunnel

A tube all round the path, filling everything the world does not draw (sky, and below the
horizon too): the walk runs down its middle and its walls rush past. It hides the rest of the
sky. Use it with a path over a drop (`path.bridge`, `bottom: Void`) so it shows below as well.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `false` |  |
| `kind` | [TunnelKind](#tunnelkind) | `"Hyperspace"` |  |
| `radius` | number | `6.0` | The tube's radius, metres: small for a tight tunnel, large for an open one. |
| `colors` | integer × 3 × 3 | `[[6,10,30],[170,210,255],[235,245,255]]` | The walls' deep colour, the streaks or bands, and the light at the far end. |
| `intensity` | number | `1.0` |  |
| `rush` | integer | `3` | How much faster than the walk the walls rush past: extra loop lengths per loop (whole). |
| `twist` | integer | `0` | Whole turns the pattern winds round the tube along one loop length (a corkscrew). |
| `spin` | integer | `0` | Whole turns the whole tunnel spins in one loop. |
| `density` | number | `1.0` | Hyperspace: how many streaks; Wormhole: how fine the bands. 1 is natural. |
| `core` | number | `1.0` | Brightness of the light at the far end. |
| `light` | number | `0.5` | How much it lights the path in its colour (0 none). |
| `seed` | integer | `0` |  |

### TunnelKind

| Value | Meaning |
|---|---|
| `Hyperspace` | Streaks of starlight stretched past at light speed. |
| `Wormhole` | Swirling bands of glowing gas spiralling down a throat to a bright far end. |

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
| `base_shadow` | number | `0.55` | Darkening where the wall meets the ground (0..1). |
| `material` | [Material](#material) | (see the type) |  |

### Waterway

The path as a waterway, for games where the player steers a boat or swims: a current, foam
lapping at the banks or walls, the bank wet where the water reaches it, and things floating on
the surface. Give `path.material` the Water pattern (gloss 1, some ripples); with no verge the
water runs to the walls, as a canal's does.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `false` |  |
| `flow` | integer | `2` | The current, towards the camera: whole tiles of the water's pattern carried past per loop (0 still water, negative flows away). Floating things drift with it. |
| `foam` | number | `0.5` | Foam where the water laps the banks or walls: 0 none .. 1. |
| `foam_color` | integer × 3 | `[214,226,224]` |  |
| `foam_width` | number | `0.35` | How far out from the bank the foam reaches, metres. |
| `lap` | integer | `3` | Times the water laps up the bank in one loop (whole). |
| `wet` | number | `0.25` | How far up the bank (or the walls) the water wets it, darker and glistening, metres. |
| `floating` | [Floating](#floating) | `"None"` | What floats on the water, drifting with the current. |
| `float_density` | number | `0.3` | How much of the water they cover, 0 .. 1. |
| `float_color` | integer × 3 | `[70,110,52]` |  |
| `float_size` | number | `0.5` | How big each one is, metres across. |
| `frozen` | number | `0.0` | How much of the water is frozen over (0..1): ice grows out from the banks, leaving a narrowing channel of open water down the middle, until at 1 it is ice from bank to bank. The ice is cracked, with snow lying on it in patches, and takes `foam_color`. |
| `seed` | integer | `0` |  |

### Weather

Weather and the air: rain and snow, wind, storms, mist, light in the air, heat haze, the lens.
Every part works in any scene and repeats exactly with the loop.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `lightning` | [Lightning](#lightning) | (see the type) |  |
| `fog_banks` | [FogBanks](#fogbanks) | (see the type) |  |
| `precipitation` | [Precipitation](#precipitation) | (see the type) | Rain, snow, sleet or hail falling everywhere open to the sky, and what it does to the ground. |
| `drips` | [Drips](#drips) | (see the type) | Water dripping from the ceiling (or the tops of the walls where there is none). |
| `wind` | [Wind](#wind) | (see the type) | Wind: slants rain and snow, carries particles sideways, sways plants and banners. |
| `sandstorm` | [Sandstorm](#sandstorm) | (see the type) | A sandstorm: sand streaming past, a sand-coloured haze, the sun dimmed to a disc. |
| `mist` | [Mist](#mist) | (see the type) | Mist lying on the ground, and wisps rising from it. |
| `light_shafts` | [LightShafts](#lightshafts) | `{"enabled":false,"sun":0.6,"lamps":0.4}` | Light made visible by the air: shafts from the sun past trees and walls, haloes round lamps. |
| `heat_shimmer` | [HeatShimmer](#heatshimmer) | `{"enabled":false,"strength":0.5,"speed":1.0}` | Heat haze: the air shimmers over the ground in the distance. |
| `lens` | [Lens](#lens) | (see the type) | Raindrops or frost on the camera's lens. |

### Wind

Wind across the path.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `false` |  |
| `speed` | number | `3.0` | Wind speed, metres per second: positive blows to the right, negative to the left. |
| `gusts` | number | `0.4` | How much it rises and falls in gusts (0 steady .. 1 squally). |
| `sway` | number | `0.5` | How much trees, reeds, grass and banners sway (0..1). |
| `flutter` | number | `0.6` | How much the wind stirs leaves and hanging strands (0..1): a tree's crown ripples, a willow's strands swing and trail downwind, while trunks stay put. |
| `seed` | integer | `0` |  |

### Winding

A path that winds: it swings to one side and back to the other as it goes, and everything
beside it (walls, verge, props, lights) follows. The bends come towards the camera as it
walks; a whole number of them fits the loop.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `false` |  |
| `amplitude` | number | `2.0` | How far the path swings to each side, metres. |
| `length` | number | `40.0` | Distance from one bend to the next the same way, metres (snapped so the loop closes). |
| `turn` | number | `0.6` | How much the view turns with the path (0..1). At 1 the camera always looks along the path under it, so the bends ahead swing well across the frame; at 0 it looks straight on and the path slides from side to side under it. The sky turns with the camera: the sun, moon, stars, clouds and aurora slide across the frame by as much as it has turned (and out of it on a sharp bend), and the sun's and moon's light swings round with them. |
| `irregular` | number | `0.3` | How uneven the bends are (0..1): 0 an even wave, more makes some sharper, some gentler. |
| `phase` | number | `0.0` | Where in a bend the loop starts (0..1 of a whole bend). |

