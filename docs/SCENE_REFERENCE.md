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
| `season` | [Season](#season) | (see the type) | The time of year: foliage, grass and the ground through spring, summer, autumn and winter. |
| `wildlife` | [Wildlife](#wildlife) | (see the type) | Things that move about: birds crossing the sky, bats at dusk, fish leaping from the water. |
| `waterfalls` | list of [Waterfall](#waterfall) | `[]` | Waterfalls beside the path: water pouring down a rock face into foam and spray. |
| `fences` | list of [Fence](#fence) | `[]` | Fences along the path: pickets, rails or boards, joined post to post and following its bends. |
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
| `wedge_m` | number | `0.0` | Length of ground of this world left between the branches past the junction, metres (0 = auto; negative = none: the ground between the branches is the branch worlds' from where they part, each its own half, so what stands beside the branch taken goes on into its world). |
| `path_hold_m` | number | `0.0` | Metres down each branch over which its path keeps this world's look before turning into its own over `blend_m`, in patches (0 = auto; negative = none: each branch's path is its own from the junction). Two differently coloured paths parting at the junction read as a seam; this way the fork is one path that changes once the player is on their way. |
| `steer_m` | number | `6.0` | Metres of walking over which the camera turns onto the chosen branch. |
| `camera_blend_m` | number | `4.0` | Metres either side of the junction over which the camera becomes the chosen world's. |
| `mouth_m` | number | `0.0` | Length of the fork's mouth, metres (0 = auto): over this stretch before the junction the first world's props stand back from the path in a widening funnel, so both branches can be seen parting instead of vanishing behind the trees that line the road. |
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
| `swing` | [Swing](#swing) | (see the type) | When the scenes look from different angles (`camera.yaw`, `camera.side`: a side-scroller and a view down the path), the camera swings: out of the first scene's view to look down the path before the boundary, and round into the second's after it. `seconds` each way (0 = auto, 2.5), with its `hold`, `ease`, `overshoot`, `orbit` and `bars` (`from` is not used: the views are the scenes'). Negative seconds: no swing, the views crossfade at the end. |
| `door` | number × 2 | `[0.0,0.0]` | A door's size where the boundary is a wall between two rooms: half its width and its height, metres. 0 each = as wide and as high as the rooms allow (the smaller room's whole end). A door narrower than the rooms leaves the first room an end wall with it in. |

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

### Bats

Bats flitting about over the path, darting and fluttering.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `false` |  |
| `count` | integer | `8` |  |
| `size` | number | `0.3` | Wingspan, metres. |
| `color` | integer × 3 | `[24,20,22]` |  |
| `dusk_only` | boolean | `true` | Only after dusk when the scene has a day and night. |
| `seed` | integer | `0` |  |

### Birds

Flocks of birds crossing the sky in loose vees, wings beating.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `false` |  |
| `flocks` | integer | `2` | Flocks crossing at once. |
| `count` | integer | `7` | Birds in a flock. |
| `size` | number | `0.025` | Wingspan as a share of the sky's height. |
| `height` | number | `0.35` | How high they fly: 0 the top of the sky .. 1 the horizon. |
| `flap` | number | `3.0` | Wingbeats a second. |
| `crossing` | number | `12.0` | Seconds to cross the frame (rounded so a whole number of crossings fit the loop). |
| `color` | integer × 3 | `[30,30,36]` |  |
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
| `thickness` | number | `0.4` | How thick the deck is, metres: its edge, seen from beside the path. 0 is a sheet. |
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
| `moves` | [CameraMoves](#cameramoves) | (see the type) | How the camera moves besides walking: a bob in its step, slow looks to the side, a dolly zoom, and a flight down from above at the start. |
| `yaw` | number | `0.0` | Which way the camera looks, degrees from straight down the path (-90 .. 90; positive turns right). The whole loop is seen from this angle: 30 looks diagonally across the path; 90 or -90 looks straight across it, a side-scroller's view, the walk carrying the world sideways past (to the left when the camera looks left, at -90). Turned far, the path's bends are straightened and what stands between the camera and the path (walls, the ceiling, props nearer than `cutaway`) is left out, so the path is seen. |
| `side` | number | `0.0` | Where the camera stands, metres sideways from the middle of the path (positive right). A side view wants it well out to the side: 10 m from a path at yaw -90, -10 at 90. |
| `cutaway` | number | `0.4` | In a side view, how far from the camera's side of the path things are left out, metres past the path's edge (props, lights, walls): the cut through the near side of the world. |
| `world_sky` | boolean | `false` | Whether the sky keeps its place in the world as the camera turns (`yaw`): the sun and moon stand where `sky.sun.pos` puts them seen down the path, and turned to the side the view swings away from them, the clouds and stars with them, so a sun ahead of the walk is off to one side of a side-scroller, its light and shadows still coming from where it is (and a swinging camera sweeps across the sky). Off: the sky keeps its place in the picture whichever way the camera looks, and its light turns with it. |

### CameraMoves

Camera moves on top of the walk. Each repeats a whole number of times a loop, so the loop
still closes; the flight in is the exception, played once at the very start (the live
runtime, `pf render --intro`, an export with an intro).

| Field | Type | Default | Meaning |
|---|---|---|---|
| `bob` | number | `0.0` | The head's bob as the walker steps: metres up and down at each footfall (0 none, about 0.04 a walk, 0.1 a run). It follows the distance walked, so it stops when the walk does. |
| `stride` | number | `0.75` | Metres walked in one step. |
| `sway` | number | `0.0` | The head's sway from side to side with the steps, degrees. |
| `pan` | number | `0.0` | A slow look to one side and back: how far, degrees (0 none; negative looks left first). |
| `pans` | integer | `1` | Looks to the side and back per loop (whole). |
| `tilt` | number | `0.0` | A slow look up and down: degrees (positive looks up first). |
| `tilts` | integer | `1` |  |
| `dolly` | number | `0.0` | A dolly zoom: the camera draws back while it zooms in (or the reverse when negative), so what stands `dolly_focus` metres ahead keeps its size while the background swells and shrinks behind it. How far it draws back, metres. |
| `dolly_focus` | number | `8.0` |  |
| `dollies` | integer | `1` | Dolly zooms per loop (whole). |
| `fly_in` | number | `0.0` | The flight in: seconds of coming down from above and over to the walker's eyes (0 none). |
| `fly_height` | number | `18.0` | How high it starts, metres. |
| `swing_in` | [Swing](#swing) | (see the type) | The swing in: played once at the start like the flight in, the camera comes round from another view (behind the walker, say) into the scene's own (a side-scroller's). |

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

### CloudMix

Layers of cloud of several kinds, chosen at random from `seed`: which kinds, how much of the
sky each covers, their size, raggedness and shade. The same seed always gives the same sky;
change it for another. They are drawn over `clouds` and `cloud_layers`, highest first.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `false` |  |
| `layers` | integer | `3` | How many layers to make (1..4). |
| `kinds` | list of [CloudKind](#cloudkind) | `[]` | The kinds to choose from. None ticked: fair-weather kinds (cumulus, towering, stratus, mackerel, cirrus). `Soft` reads as cumulus. |
| `cover` | number | `0.45` | How much of the sky they fill, roughly (0..1); each layer varies round it. |
| `patches` | number | `0.5` | How much each layer keeps to its own parts of the sky (0..1; see `Clouds::patches`). |
| `variety` | number | `0.6` | How much the layers differ from each other in size, cover, raggedness and shade (0..1). |
| `drift` | number | `1.0` | Drifts across the sky per loop, as `Clouds::drift`; higher layers drift more slowly. |
| `opacity` | number | `0.5` |  |
| `tint` | integer × 3 | `[226,230,238]` |  |
| `seed` | integer | `0` |  |

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
| `patches` | number | `0.0` | How much the layer gathers into patches with open sky between them (0..1): 0 spreads it over the whole sky, 1 keeps it to a few parts of it. Layers of different kinds with patches share the sky: heaps over here, a mackerel sky over there. |
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

### DayNight

A day and night (`sky.day_night`). The hour sets everything: the sun's place on its arc and
its colour, the sky's colours (the sunset's `sunset_style` at dusk and dawn, the blue hour,
night), the light and its colour, the fog and mist, the stars and moon, and the lamps. The
scene's own colours keep their hue (`keep_look`) while their brightness follows the hour.
Has no effect under a space sky or a tunnel, or with the sky off.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `false` |  |
| `hour` | number | `12.0` | The hour of the day, 0..24: 6 sunrise, 12 noon, 18 sunset, 0 midnight (with the default `sunrise` and `sunset`). |
| `days_per_loop` | integer | `0` | Whole days that pass in one loop, so a loop runs through dawn, day, dusk and night and closes seamlessly (0: the hour stays put). |
| `sunrise` | number | `6.0` | When the sun rises and sets, hours. |
| `sunset` | number | `18.0` |  |
| `sun_height` | number | `1.15` | How high the sun climbs at noon, as a share of the sky: past 1 it passes overhead, out of the frame (the moon climbs a little lower, and stays in it). |
| `sun_path` | number × 2 | `[0.15,0.85]` | Where across the frame the sun rises and where it sets (0 left .. 1 right). |
| `sunset_style` | [SunsetStyle](#sunsetstyle) | `"Golden"` | The colours of its dawns and dusks. |
| `glow` | number | `1.0` | How bright the dawn and dusk glow is (0..2). |
| `keep_look` | number | `0.6` | How much of the scene's own colours it keeps: 0 the hour's own sky .. 1 the scene's hues, only their brightness following the hour. |
| `night_light` | number | `0.3` | How bright the night's light is against the day's (0..1). |
| `sun` | boolean | `true` | The sun crosses the sky (on even when the scene has no sun of its own). |
| `moon` | boolean | `true` | The moon crosses the night sky. |
| `stars` | boolean | `true` | Stars come out after dusk. |
| `lamps` | boolean | `true` | The lamps (torches, lanterns, street lamps...) light at dusk and go out at dawn, and fireflies come out at night. |
| `real_time` | boolean | `false` | For live wallpapers and games: follow the device's own clock instead of `hour`. |

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

### Facing

Which way a prop's picture faces.

| Value | Meaning |
|---|---|
| `Camera` | Always toward the camera (a tree, a rock, a lamp: alike from every side). |
| `Path` | Always toward the path, standing along it as a building's front does: seen at an angle from down the path, and squarely from the side. |
| `PathWhenTurned` | Toward the camera while it looks down the path, toward the path when it is turned (`camera.yaw`): a building that looks as it always has from the path, and stands in the street when seen from an angle or the side. |

### Fence

A fence beside the path, built post to post as solid pieces that run along its edge and bend
with it, so it reads as one fence from any angle.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `true` |  |
| `style` | [FenceStyle](#fencestyle) | `"Picket"` |  |
| `side` | [Side](#side) | `"Both"` |  |
| `lateral` | number | `1.2` | Metres out from the path's edge. |
| `height` | number | `0.9` | Metres tall. |
| `color` | integer × 3 | `[236,232,220]` | The paint of pickets and boards. (Every painted fence of a scene takes the first one's colour; rails are bare wood, in the colour of a bridge's deck.) |
| `spacing` | number | `24.0` | Runs `length` metres long every `spacing` metres: as long as the spacing, one fence with no break; shorter, with a gap (a gate, a lane) between runs. Both snap to divide the loop. |
| `length` | number | `20.0` |  |
| `offset` | number | `0.0` |  |
| `post_spacing` | number | `2.0` | Metres between posts. |
| `shadow` | boolean | `true` | Whether it casts the sun's shadow on the ground, post by post and picket by picket, with a soft darkening along its foot. |
| `shadow_opacity` | number | `0.6` | How dark that shadow is (0..1), as a prop's. |

### FenceStyle

| Value | Meaning |
|---|---|
| `Picket` | White pickets on two rails between posts. |
| `Rails` | Posts and three bare wooden rails: a field fence. |
| `Boards` | Close boards from the ground up, capped by a rail. |

### Fish

Fish leaping from the water beside or along the path in silver arcs, with splash rings.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `false` |  |
| `spacing` | number | `6.0` | One every so many metres along the path. |
| `lateral` | number × 2 | `[1.5,6.0]` | How far out from the path's middle they leap: from .. to, metres (either side). |
| `leaps` | integer | `1` | Leaps each fish makes per loop. |
| `size` | number | `0.35` | Its length, metres. |
| `color` | integer × 3 | `[200,210,220]` |  |
| `seed` | integer | `0` |  |

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
| `bridge_posts` | boolean | `false` | On bridges, stand on the railing's posts instead of at `spacing`: one on each end post of every bridge, each side (whatever `place` says). |
| `posts_between` | integer | `0` | With `bridge_posts`: how many more posts (each with its light) stand evenly between the end posts: 1 halfway, 2 at the thirds, 3 at the quarters... The railing gains those posts. |
| `string_lights` | [StringLights](#stringlights) | (see the type) | Strings of little bulbs hung between each light and the next along its side, sagging between them: festoon lights for a bridge, a terrace or a market street. |
| `sprite` | [SpriteRef](#spriteref) | (see the type) |  |
| `seed` | integer | `0` |  |

### FixtureKind

One of `Torch` \| `Lantern` \| `Candle` \| `Brazier` \| `Crystal` \| `Firefly` \| `Magic` \| `GreenFire` \| `IceWisp` \| `StreetLamp` \| `PaperLantern` \| `Campfire` \| `Glowshroom` \| `Beacon` \| `Chandelier` \| `Neon`.

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

### Focus

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `false` |  |
| `mode` | [FocusMode](#focusmode) | `"Depth"` |  |
| `distance` | number | `8.0` | Depth: metres ahead that are sharp. |
| `blur` | number | `0.008` | How strongly the rest blurs: the widest blur, as a share of the frame height (0..0.03). |
| `band` | number | `0.62` | Tilt-shift: the sharp band's middle (share of the frame height from the top) and width. |
| `band_width` | number | `0.16` |  |

### FocusMode

| Value | Meaning |
|---|---|
| `Depth` | A real lens: sharp at `distance`, blurring nearer and further. |
| `TiltShift` | A band across the frame sharp and above and below it blurred: a miniature. |

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
| `frame` | number | `0.0` | A side passage's doorway framed in timber (the bridge deck's): jambs and a lintel this wide, metres, standing out from the wall. 0: a bare opening. |

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

### Halo

A halo: the ring of light round the sun or moon seen through thin high cloud (ice crystals
bending its light), reddish on its sharp inner edge, fading out beyond.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `false` |  |
| `sun` | boolean | `true` | Round the sun, round the moon (either or both, when they are up). |
| `moon` | boolean | `true` |  |
| `size` | number | `0.32` | The ring's radius, as a share of the sky's height. |
| `strength` | number | `0.35` |  |

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

### Meteors

Shooting stars: streaks of light crossing the sky, each flaring and fading as it burns up.
Each one falls at its own moment of the loop, so the loop closes seamlessly.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `false` |  |
| `count` | integer | `6` | How many fall in one loop (a shower: 10 or more). |
| `duration` | number | `0.7` | How long each one lasts, seconds. |
| `length` | number | `0.35` | How far each one travels, as a share of the sky's height. |
| `angle` | number | `-35.0` | Which way they fall, degrees (0 to the right, -90 straight down; a shower shares one way). |
| `spread` | number | `30.0` | How much their ways vary, degrees (180 from anywhere). |
| `brightness` | number | `1.0` |  |
| `color` | integer × 3 | `[235,242,255]` |  |
| `tint` | number | `0.4` | How much some burn green or orange. |
| `seed` | integer | `0` |  |

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
| `from` | number | `0.0` | Clear air this many metres ahead before the mist begins: fog lying in the low ground in the distance, a valley filled with it (0: from the camera on). |

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
| `craters` | boolean | `true` | Its seas, craters (with raised rims, bowls and central peaks, catching the sunlight past the line between day and night) and the rays of its young craters. |
| `relief` | number | `1.0` | How deep the craters look: 0 flat markings .. 1 natural .. 2 rugged. |
| `seed` | integer | `0` | Which craters: each seed scatters a different field of them. |

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
| `selective` | number | `0.0` | Selective outline, 0..1: how far each line takes the colour of the thing it outlines (darkened by `dark`) instead of `color`. 1 is the hand-drawn pixel-art line, a deeper shade of the leaf or stone itself; 0 is flat ink. |
| `dark` | number | `0.5` | How dark a selective line is against its object's colour, 0..1 (0.5 halves it). |

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
| `orbit` | [PlanetOrbit](#planetorbit) | (see the type) | Circling another planet of the sky as its moon: it passes in front of it and behind it, and casts its shadow across it as it crosses (an eclipse). |
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
| `Desert` | A dry world of dunes and canyons (`color` the sand, `color2` the rock), thin dust storms and small pale caps at its poles. |

### PlanetOrbit

A planet's orbit round another (`Planet::orbit`).

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `false` |  |
| `around` | integer | `0` | Which planet of the list it circles (0 the first). Its own `pos` is then ignored. |
| `distance` | number | `2.2` | How far out, in the other planet's radii. |
| `tilt` | number | `12.0` | How steeply the orbit is seen: 0 edge-on (straight across, in front then behind) .. 90 face-on. |
| `roll` | number | `-8.0` | The orbit's slope on screen, degrees. |
| `turns` | integer | `1` | Whole orbits per loop (negative the other way; 0 holds it at `phase`). |
| `phase` | number | `0.1` | Where on the orbit it is at the start, 0..1. |

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
| `focus` | [Focus](#focus) | (see the type) | Depth of field: what is nearer or further than the focus blurs, or (tilt-shift) all but a band across the frame does, for the look of a miniature. |
| `keep_dark` | boolean | `false` | Left as dark as it is drawn: a game's visibility floor (`Runtime::set_visibility_floor`) does not lift it. For a place where what hides in the dark is meant to. |

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
| `smoke` | [PropSmoke](#propsmoke) | (see the type) | Smoke rising from it: a chimney, a campfire, a smouldering ruin. |
| `facing` | [Facing](#facing) | `"Camera"` | Which way its picture faces (see `Facing`): the camera (a tree, a rock: the same from every side), or the path (a building's front, a fence, a stall), seen at an angle when the camera is turned. |
| `depth` | number | `0.0` | Facing the path: how deep it is behind its front, metres. Its ends are drawn as walls that deep (from the edges of its picture), so seen at an angle it reads as a solid block; 0 is a flat front. |
| `faces` | list of [PropFace](#propface) |  | Facing the path: the rest of it, as flat faces with pictures of their own (a cottage's side walls and the slopes of its roof, a chimney's sides), drawn in place of plain end walls. |
| `tubes` | list of [PropTube](#proptube) |  | Facing the path: round parts as real cylinders (the tails of logs at a cabin's corners, posts, pipes), drawn true from any angle with their sawn ends. |
| `pick` | list of string |  | Variety: each prop picks one of these definitions instead (names in the same kit, or in the scene's `prop_defs`), so a row of cottages is not one cottage over and over. |

### PropFace

A face of a prop besides its front (`PropDef::faces`): a flat picture standing in the prop's
own space, in metres at scale 1 (the units of its `shape`): `x` across its front, `y` up from
the ground and `z` back from its front, away from the path.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `corners` | number × 3 × 3 | `[[1.0,1.0,0.0],[1.0,1.0,1.0],[1.0,0.0,0.0]]` | Seen from the side it faces: its picture's top left, top right and bottom left corners (`[x, y, z]`; the fourth corner follows). A roof's slope runs from its ridge down to its eave. |
| `shape` | list of [ShapePart](#shapepart) | `[]` | Its picture: parts painted in metres, `x` from its left edge and `y` up from its bottom edge, as wide and tall as the face. |
| `image` | string |  | Or an image stretched over it (relative to the file the definition is in). |
| `mirror` | boolean | `false` | Drawn twice: as given and mirrored across the middle of the front (`x` to `-x`), for the two sides of something symmetric. |

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
| `facing` | [Facing](#facing) or null |  | Which way the props face, in place of their definition's (`PropDef::facing`; built-in kinds face the camera). |
| `depth` | number or null |  | How deep they are behind their fronts, in place of their definition's (`PropDef::depth`). |

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

### PropSmoke

Smoke rising from a prop (`PropDef::smoke`): puffs that climb, spread, drift with the wind and
thin away, round and round with the loop.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `false` |  |
| `at` | number × 2 | `[0.0,2.0]` | Where it leaves the prop, metres at scale 1: `x` sideways from its centre, `y` up. |
| `color` | integer × 3 | `[150,150,155]` |  |
| `amount` | number | `0.5` | How thick (0..1). |
| `rise` | number | `3.0` | How high it climbs before it has thinned away, metres. |
| `width` | number | `0.8` | How wide a puff grows, metres. |
| `back` | number |  | For a prop facing the path: how far back from its front it leaves, metres at scale 1 (a chimney on the ridge). |

### PropTube

A cylinder that is part of a prop (`PropDef::tubes`): from one point to another in the prop's
space (metres at scale 1: `x` across its front, `y` up, `z` back from the path), `radius`
round, its side the colour of bark and its ends sawn: end grain with growth rings.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `from` | number × 3 | `[0.0,0.5,0.0]` |  |
| `to` | number × 3 | `[1.0,0.5,0.0]` |  |
| `radius` | number | `0.15` |  |
| `color` | integer × 3 | `[110,80,54]` |  |
| `end` | integer × 3 | `[180,146,104]` | Its sawn ends. |
| `mirror` | boolean | `false` | Drawn twice: as given and mirrored across the middle of the front (`x` to `-x`). |

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
| `saturation` | number | `1.0` | How colourful: 1 a rainbow; about 0.2 a moonbow, the pale bow the moon makes at night. |

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

### Season

The time of year (`scene.season`), over the scene's own colours (taken as summer's): fresh light
green and blossom in spring; turning gold, orange and red with leaves falling in autumn; bare
broadleaf trees, straw-coloured grass and lying snow in winter. In between, one season turns
into the next.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `false` |  |
| `season` | number | `1.0` | Where in the year: 0 spring, 1 summer, 2 autumn, 3 winter (4 is spring again; fractions are the turn from one to the next). |
| `years_per_loop` | integer | `0` | Whole years that pass in one loop (0 holds the season). |
| `falling` | boolean | `true` | Blossom drifting down in spring, leaves falling in autumn. |
| `snow` | boolean | `true` | Snow lying in winter. |

### SetPiece

A structure spanning the path, repeated along it.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `true` |  |
| `kind` | [SetPieceKind](#setpiecekind) | `"Archway"` |  |
| `spacing` | number | `24.0` | Distance between repeats, metres (snapped to divide the loop; the loop length = once per loop). |
| `offset` | number | `12.0` | Shift along the path, metres. |
| `width` | number | `0.0` | Clear width of the opening, metres. 0 = fit the path (and the gap to the walls). |
| `height` | number | `3.4` | Clear height of the opening, metres. An arch or gate is never lower than the walking camera's eye plus `ARCH_CLEARANCE_M`, so the camera passes under its stone, not through it. |
| `depth` | number | `0.0` | How deep it is along the path, metres: it is built solid from its picture, so it has sides, tops and the underside of its arch seen from any angle. 0 = as deep as its kind usually is (an arch 0.9 m, a gate 1.4 m, a portal's ring 0.35 m, banners 0.12 m). |
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
| `meteors` | [Meteors](#meteors) | (see the type) | Shooting stars streaking across the sky (by night, or in space). |
| `clouds` | [Clouds](#clouds) | (see the type) |  |
| `cloud_layers` | list of [Clouds](#clouds) | `[]` | More layers of cloud over `clouds`, drawn in order (up to three): cirrus high over cumulus, heaps under an overcast. Give layers `patches` to share the sky between kinds. |
| `cloud_mix` | [CloudMix](#cloudmix) | (see the type) | Clouds of several kinds chosen at random from a seed, added over the layers above. |
| `sunset` | [Sunset](#sunset) | (see the type) | A sunset (or sunrise): the sky, a glow along the horizon, the clouds and the light all take the colours of a low sun. |
| `day_night` | [DayNight](#daynight) | (see the type) | A day and night: the sun crosses the sky and sets, the light, sky and fog follow it, the stars and moon come out and the lamps light at dusk. Set the hour, or let the days pass with the loop; a game can set the hour itself (`Runtime::set_hour`). |
| `aurora` | [Aurora](#aurora) | (see the type) | Northern lights: curtains of green and violet light rippling across the sky. |
| `rainbow` | [Rainbow](#rainbow) | (see the type) |  |
| `halo` | [Halo](#halo) | (see the type) | A ring of light round the sun or moon in thin high cloud. |
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

### SnowCover

Snow that has already fallen: laid on trees, rocks and the rest of the scenery (`props`) and on
the ground, walls and railings (`ground`), with no snow falling. Falling snow
(`precipitation`) lays its own; whichever is deeper shows.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `false` |  |
| `props` | number | `0.7` | How much snow lies on trees and the rest of the scenery (0..1): a cap along their tops at 0.2, branches laden with it at 1. |
| `ground` | number | `0.6` | How much lies on the ground, walls and railings (0..1): a dusting at 0.3, deep at 1. |

### Space

The sky seen from space. It sits at infinity, so it stays put while the camera walks.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `false` |  |
| `below` | boolean | `true` | Space all round: where nothing is drawn below the horizon (a bridge's bottomless drop, the gaps beside a floating walkway) shows space too, instead of the void colour. The sky's `top`..`horizon` gradient is mirrored below the horizon. |
| `star_style` | [StarStyle](#starstyle) | `"Realistic"` | How the starfield is drawn. Realistic: a few bright stars among a great many faint ones, coloured by temperature, gathered in clouds and lanes. Classic: the even field of small dots of earlier versions (and of ball_swing_game). |
| `stars` | number | `1.0` | The dense field of faint stars: 0 none .. 1 a rich field .. 2 a crowded one. |
| `star_brightness` | number | `1.0` |  |
| `star_colors` | number | `1.0` | The stars' colours: 1 hot blue-white to cool orange; 0 all the faint blue-white of ball_swing_game's starfield. |
| `star_glow` | number | `0.5` | Realistic stars: the faint glow of stars too many and too far to see one by one. |
| `star_clumps` | number | `0.5` | Realistic stars: how unevenly they gather, 0 evenly .. 1 in clouds and voids. |
| `star_spikes` | number | `0.4` | Realistic stars: the bright stars' diffraction spikes (as a telescope sees them), 0 none. |
| `spike_shape` | [SpikeShape](#spikeshape) | `"Four"` |  |
| `twinkle` | number | `0.0` | How much the stars twinkle (0 steady, as in airless space .. 1 strongly). |
| `twinkle_speed` | integer | `2` | Whole twinkles per loop for the slowest stars (some go twice as fast), so the loop closes. |
| `nebula` | number | `0.6` | Glowing gas clouds: strength 0 .. 2, two colours, and size (1 about a sky's height across). |
| `nebula_colors` | integer × 3 × 2 | `[[190,70,200],[60,120,255]]` |  |
| `nebula_scale` | number | `1.0` |  |
| `nebula_dust` | number | `0.5` | How much dark dust threads the gas and hides the stars behind it. |
| `galaxy` | number | `0.7` | The band of a galaxy seen edge-on, with dark dust lanes and a brighter core. |
| `galaxy_color` | integer × 3 | `[235,225,255]` |  |
| `galaxy_angle` | number | `18.0` | Tilt of the band in degrees (0 level, positive rising to the right). |
| `galaxy_width` | number | `0.22` | Its width, as a share of the sky's height. |
| `galaxy_height` | number | `0.55` | Where the band crosses the middle of the frame, in sky heights above the horizon. |
| `galaxy_core` | number | `0.25` | Where its bright core sits along the band: -1 left .. 1 right. |
| `galaxy_dust` | number | `0.7` | How much its dark dust lanes and rifts hide. |
| `objects` | list of [SpaceObject](#spaceobject) | `[]` | Up to six things far off in the sky: nebulae, galaxies, star clusters and comets. |
| `planets` | list of [Planet](#planet) | `[]` | Up to three planets, drawn in order (later ones in front). |
| `black_hole` | [BlackHole](#blackhole) | (see the type) | A black hole: its shadow, a glowing disk lensed round it, and matter falling in. |
| `seed` | integer | `0` |  |

### SpaceObject

Something far off in the space sky, behind the planets and bent round a black hole like the
stars. `colors` mean something different for each kind (see `SpaceObjectKind`).

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `true` |  |
| `kind` | [SpaceObjectKind](#spaceobjectkind) | `"Emission"` |  |
| `pos` | number × 2 | `[0.5,0.35]` | Centre: x 0..1 across the frame, y 0 (top) .. 1 (horizon). |
| `size` | number | `0.3` | Its radius as a share of the sky's height (a comet: its tail's length). |
| `angle` | number | `0.0` | Its turn on screen, degrees (a comet: where its tail points, 0 right, 90 up). |
| `aspect` | number | `1.0` | How round it is: 1 round .. 0.1 seen almost edge-on. |
| `colors` | integer × 3 × 3 | `[[235,70,110],[90,220,215],[255,180,120]]` |  |
| `brightness` | number | `1.0` |  |
| `spin` | integer | `1` | Whole turns or passes per loop (0 still): a binary's orbit, a pulsar's beams, a belt's drift, a sun's flares, a ship's crossings. Whole numbers keep the loop seamless. |
| `seed` | integer | `0` |  |

### SpaceObjectKind

| Value | Meaning |
|---|---|
| `Emission` | A glowing cloud of gas lit by the young stars inside it, with dark dust pillars and lanes: `colors` its outer gas (hydrogen red), its hot heart (oxygen teal) and the dust's lit edges. |
| `Reflection` | Dust lit blue by a few bright stars inside it: its glow, the stars' halo, its streaks. |
| `Dark` | A cloud of dust hiding the stars behind it: its lit rim, and its faint body. |
| `Planetary` | A dying star's glowing shell round a white dwarf: the outer rim, the inner shell, the halo. |
| `Remnant` | The wisps of an exploded star's shell: two filament colours and the faint fill. |
| `Spiral` | A spiral galaxy: its arms, its core and its pink star-forming knots. |
| `Elliptical` | A smooth elliptical galaxy: its glow, its core. |
| `Cluster` | A star cluster: its stars' tint, its glow. |
| `Comet` | A comet: its curved dust tail, its straight blue ion tail, its glowing head. |
| `Binary` | Two stars circling each other (`spin` orbits a loop), a stream of gas between them: the larger star, the smaller, the stream. |
| `Pulsar` | A spinning dead star sweeping two beams round (`spin` turns a loop) inside the wisps of the star it was: the star, its beams, the wisps. |
| `Belt` | A band of asteroids across the sky, `angle` its slope and `size` its half thickness, drifting along (`spin` passes a loop): its dust, its rocks, their lit sides. |
| `Sun` | A sun seen close: a glowing disc with spots and a darker rim, its corona streaming out and flares licking off its edge (changing `spin` times a loop): disc, corona, flares. |
| `Ship` | A ship far off, crossing the sky `spin` times a loop along `angle` (its length is `size`), its engine glowing and leaving a trail: hull, engine, running lights. |

### SpikeShape

The bright stars' diffraction spikes: four (a cross), six or eight points.

One of `Four` \| `Six` \| `Eight`.

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

### StarStyle

One of `Realistic` \| `Classic`.

### Stop

| Field | Type | Default | Meaning |
|---|---|---|---|
| `scene` | string | `""` | Scene file (relative to the journey) or `preset:<name>`. |
| `next` | [Next](#next) | `"End"` |  |

### StringLights

Bulbs on a wire strung between neighbouring lights of a row (`Fixture::string_lights`).

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `false` |  |
| `bulbs` | integer | `9` | Bulbs between each pair of lights. |
| `sag` | number | `0.45` | How far the wire sags at the middle, metres. |
| `color` | integer × 3 | `[255,214,140]` |  |
| `colors` | list of integer × 3 | `[]` | Colour the bulbs in turn through these instead (empty: all `color`). |
| `brightness` | number | `1.0` |  |
| `size` | number | `1.0` |  |
| `twinkle` | number | `0.2` | How much they twinkle (0 steady). |

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

### Sunset

A sunset over the sky. `amount` takes the sky's colours, the sun, the clouds' tint and the
light towards those of the `style`; the glow lies along the horizon, brightest round the
sun's place across the sky (`sky.sun.pos`, even with the sun itself off, as at dusk); clouds
are lit from below, gold and orange near the horizon and the sun, pink higher up, violet
where they turn away. Place the sun low (`pos` y 0.8-1) for the best of it.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `false` |  |
| `style` | [SunsetStyle](#sunsetstyle) | `"Golden"` |  |
| `amount` | number | `1.0` | How far the sky, sun, clouds and light go towards the sunset's colours (0..1). |
| `glow` | number | `1.0` | How bright the glow along the horizon is (0..2). |
| `height` | number | `0.35` | How high the glow climbs, as a fraction of the sky (0.05..1). |
| `spread` | number | `0.8` | How far round from the sun the glow reaches, in frame widths (0.1..3). |
| `clouds` | number | `1.0` | How much the clouds take its colours (0..1). |
| `light` | number | `0.7` | How much the scene's light takes its colours: a warm low sun and a dusky fill (0..1). |

### SunsetStyle

The colours of a sunset.

| Value | Meaning |
|---|---|
| `Golden` | Gold and amber low down under a clear blue sky, clouds gilded and peach. |
| `Fiery` | Blazing orange and red along the horizon, crimson and magenta clouds, a deep blue sky. |
| `Pastel` | Soft peach and rose, a lavender sky, pink clouds: a hazy summer evening. |
| `Dusk` | Just after the sun has gone: a thin red band at the horizon under violet and indigo, clouds dark with rose undersides. |

### Swing

A camera swing: once, the camera comes round from one view to another, for an intro into a
scene (`CameraMoves::swing_in`) or a game's change of view. Circling the spot on the path both
views look at (`orbit`), that spot stays in the middle of the frame as the camera comes round,
the near side of the world dissolves out of its way, the path straightens and buildings turn
to face it, as a side view has them; with `camera.world_sky` the sun and sky sweep across.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `seconds` | number | `0.0` | Seconds it takes to come round (0: no swing). |
| `hold` | number | `0.5` | Seconds it holds the first view before it moves. |
| `from` | [SwingView](#swingview) | (see the type) | The view it comes from (it ends on the scene's own camera). |
| `ease` | [SwingEase](#swingease) | `"Smooth"` |  |
| `overshoot` | number | `0.0` | How far past the end it swings before settling back, a share of the turn (0 none, up to 0.3): a crane's or a handheld camera's overshoot. |
| `orbit` | boolean | `true` | Circling the spot on the path both views look at (true), or moving straight across from one place to the other, turning as it goes (false). |
| `bars` | number | `0.0` | Letterbox bars over the top and bottom of the frame at the start, a share of its height each (0 none, up to 0.25), sliding away as the swing ends: the game starts. |

### SwingEase

| Value | Meaning |
|---|---|
| `Linear` |  |
| `Smooth` | Gathering speed and settling gently. |
| `Out` | Off at once, settling slowly: a reveal. |
| `In` | Easing off the first view, arriving briskly. |

### SwingView

Where a swing's camera starts: as `Camera` has it, and `back` metres behind the walker (a
third-person view; 0 at the walker's eyes).

| Field | Type | Default | Meaning |
|---|---|---|---|
| `yaw` | number | `0.0` |  |
| `side` | number | `0.0` |  |
| `back` | number | `5.0` |  |
| `eye_height` | number | `2.2` |  |
| `horizon` | number | `0.4` |  |
| `zoom` | number | `1.0` |  |

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
| `thickness` | number | `0.5` | How thick the wall is, metres: its top, its back and its ends show this much stone wherever they are seen (from above, from a turned camera, where a fork or crossing ends it). 0 = a sheet with no thickness. |
| `material` | [Material](#material) | (see the type) |  |

### Waterfall

A waterfall beside the path (`scene.waterfalls`): a rock face with water pouring down it in
streaks, foam and spray boiling up at its foot, and (with the sun out) a little rainbow in the
spray. It repeats every `spacing` metres, like a set piece; its water falls a whole number of
times a loop.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `true` |  |
| `side` | [Side](#side) | `"Left"` |  |
| `lateral` | number | `3.0` | Metres from the path's edge. |
| `spacing` | number | `60.0` |  |
| `offset` | number | `20.0` |  |
| `height` | number | `7.0` | How far it falls, and how wide the water is, metres. |
| `width` | number | `2.2` |  |
| `water` | integer × 3 | `[210,228,240]` |  |
| `rock` | integer × 3 | `[92,88,82]` |  |
| `spray` | number | `0.7` | How much foam and spray at its foot (0..1). |
| `rainbow` | number | `0.5` | The rainbow in its spray when the sun is out (0 none). |
| `facing` | number | `45.0` | Which way its face looks, degrees: 0 back down the path, toward a walker coming, 90 across to the path. It stands where it is whichever way the camera looks. |
| `cliff_width` | number | `0.0` | How wide its rock face is, metres (0: as wide as it is tall), and how far the rock runs back from it. |
| `cliff_depth` | number | `5.0` |  |
| `ridge` | number | `3.0` | A higher shoulder of the mountain behind, this much higher than the fall's rock, with a cascade down from it onto the top (0: none). |
| `stream` | boolean | `true` | The stream running over the top to the lip, where the water comes from. |
| `pool` | number | `1.0` | The pool at its foot, its size as a share of the usual (0: none; it falls on the ground). |
| `seed` | integer | `0` |  |

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
| `cascades` | number | `0.0` | Steps of white water across the channel where it tumbles over rock: 0 none .. 1 a tumbling drop. |
| `cascade_spacing` | number | `24.0` | Metres between them. |
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
| `snow_cover` | [SnowCover](#snowcover) | `{"enabled":false,"props":0.7,"ground":0.6}` | Snow lying on the scenery and the ground without any falling: a winter scene from any scene. |

### Wildlife

Animals on the move (`scene.wildlife`). Each goes round a whole number of times a loop, so the
loop closes; with a day and night, birds fly by day and bats come out at dusk.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `birds` | [Birds](#birds) | (see the type) |  |
| `bats` | [Bats](#bats) | (see the type) |  |
| `fish` | [Fish](#fish) | (see the type) |  |

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

