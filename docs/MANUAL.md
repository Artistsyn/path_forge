# PathForge manual

PathForge makes endless, seamlessly looping first-person path backgrounds for 2D games: the road a
hero walks in a portrait runner, a corridor between fights, the trail a party travels along. You
describe a world in metres, and PathForge renders a camera walking through it as a loop with no
seam. You can export the loop as animation files with the metadata a game needs, or render it live
inside a game, and walk on from one place into the next through transitions and forks.

This manual explains what each part does and how to use it. Every field of the scene format, with
its type, default and meaning, is listed in [SCENE_REFERENCE.md](SCENE_REFERENCE.md), which is
generated from the code.

Contents

1. [Getting started](#1-getting-started)
2. [Scenes](#2-scenes)
3. [The studio](#3-the-studio)
4. [Looks: styles, palettes, grades](#4-looks-styles-palettes-grades)
5. [Props, kits and sprites](#5-props-kits-and-sprites)
6. [Exporting](#6-exporting)
7. [Export formats and their metadata](#7-export-formats-and-their-metadata)
8. [Encounter clips](#8-encounter-clips)
9. [Transitions](#9-transitions)
10. [Forks](#10-forks)
11. [Journeys](#11-journeys)
12. [The live runtime](#12-the-live-runtime)
13. [Quartz games and quartz_forge](#13-quartz-games-and-quartz_forge)
14. [Projects and files](#14-projects-and-files)
15. [The `pf` command line](#15-the-pf-command-line)
16. [AI agents: MCP server and skill](#16-ai-agents-mcp-server-and-skill)
17. [Checking quality](#17-checking-quality)
18. [Troubleshooting](#18-troubleshooting)

---

## 1. Getting started

```bash
cargo build --release
```

This builds three programs into `target/release/`:

| Program | What it is |
|---|---|
| `path_forge [scene.json]` | The studio: live preview, inspector, timeline, preset gallery, export window, transition and fork designer. |
| `pf` | The command line: render, check, export, plan transitions and forks, export journeys, manage kits and projects. |
| `pf_mcp` | An MCP server for AI agents (20 tools that return images). |

The quickest way in is to open the studio and pick a preset from File > New from preset…, or to
render one from a shell:

```bash
pf presets
```

```bash
pf render --preset "Forest Path" -o forest.png
```

There are 33 presets: Stone Dungeon, Stone Crypt, Mossy Sewer, Forest Path, Desert Canyon, Night
Road, Magic Cavern, Ice Dungeon, Ruins Path, Dark Street, Mountain Pass, Volcanic Rift, Haunted
Forest, Ruined Castle, Fiery Dungeon, Tower Stair, Bog Boardwalk, Desert Ruins, Ice Cave, Star Bridge,
Hyperspace Run, Wormhole, Starship Flight, Event Horizon, Black Hole Arena, River Run, Canal Lanterns,
Jungle River, Lagoon Swim, Frozen Channel, Misty River, Dawn Road and Hyperspace Jump.
Every one loops without a seam (`pf seam` checks them all).

## 2. Scenes

A scene is a JSON file (`version: 3`) describing one place. Every field is optional; a missing
field takes its default. Units: lengths in metres, angles in degrees, colours `[r, g, b]` 0..255,
times in seconds.

| Part | What it controls |
|---|---|
| `canvas` | Output size in pixels (default 480 × 854, portrait). |
| `camera` | Eye height, horizon position, zoom, lens curve. |
| `path` | The road: width, flare, bend (curving left or right), hill (rising or dipping), ragged edges, material and pattern (cobblestone, brick, stone block, sand, dirt, grass, planks, water, ice, and the futuristic panels, grid, hex and circuit, whose lit parts glow with `material.glow`), stairs, bridges, side paths, roads that divide in two or three, side passages that branch off as scenery, and `edge_lights` (glowing lines along both edges, solid or dashed, the dashes running ahead with `flow`). `surface: false` draws no ground at all: the camera flies the route through open space (Starship Flight, Hyperspace Jump), with floating props, companions and the sky all round. |
| `verge` | The ground beside an open path: its material and grass tufts. Off, the path edge drops into void (walls usually cover it). |
| `walls` | Walls either side (a corridor, a canyon, a street) and their material. |
| `ceiling` | A roof at a height, with its material. |
| `sky` | Sky gradient (top and horizon colours), sun (with the bright aureole round it), moon (with phase and craters), stars, clouds (with drift and shadows on the ground), aurora, rainbow, and deep space (starfield, nebulae, a galaxy band, planets with rings). |
| `light` | Ambient light and its colour, the colour of the far distance, fog, and cel-shaded light bands. |
| `fixtures` | Repeating lights: torches, lanterns, candles, braziers, crystals, fireflies, magic, green fire, ice wisps, mounted on walls, the ground or the ceiling, or floating. |
| `props` | Layers of repeating objects: trees, pillars, rocks, barrels, banners, or data-defined props (§5). `float` lifts a layer off the ground (asteroids, drifting islands, lanterns), over a bridge's drop too. |
| `set_pieces` | Structures spanning the path at intervals: archways, gates, banners. |
| `companions` | Things travelling along with the camera at a fixed place in the frame: a ship flying beside the path (drawn, or any image seen from behind via `sprite`), following its bends, bobbing and weaving in whole cycles per loop, its engines lighting what is near. |
| `particles` | Dust, embers, fireflies, rain, snow, leaves, ash, spores, sand, petals. They drift with the wind. |
| `weather` | Rain, snow, sleet and hail with what they do to the ground, drips, wind, sandstorms, mist, light shafts, heat shimmer, drops or frost on the lens, lightning and fog banks. See [Weather and the air](#weather-and-the-air). |
| `prop_defs` | Props described by data, named for the scene's prop layers to use. |
| `post` | Exposure, contrast, saturation, bloom, vignette, grain, tint. |
| `style` | How the picture is drawn: pixel size, palette, dither, outline, paint, grade (§4). |
| `motion` | Loop length (metres), walking speed (m/s) and fps for exports. |

**The loop.** One loop covers `motion.loop_length` metres of road. Everything that repeats along
the road (fixtures, props, set pieces, stairs, the path pattern) is snapped so that its spacing
divides the loop length, so the last frame leads straight into the first. Everything that moves by
itself (flames, particles, clouds, lightning, fog banks) runs in loop time, so it also returns to
where it started. Loop time is `loop_length / speed` seconds: a 24 m loop at 2.5 m/s lasts 9.6 s.

**Clouds** drift with `sky.clouds.drift`, measured in loops. A whole number (1, 2…) moves each
cloud right round the sky that many times per loop. A fraction (0.4, 0.45…) keeps its exact
speed, and each cloud forms, drifts and dissolves once per loop, so the loop still joins.

**The moon** is drawn as a lit sphere. `phase` runs from −1 to 1: 0 is full, and the lit part
shrinks towards new at either end (negative on the waning side, positive on the waxing side). The unlit part shows the sky and the stars behind it, plus a
faint earthshine.

**Waterways** (`path.waterway`). With `enabled` the path is water to travel on by boat or swimming
(give it a `Water` material with high `gloss`). The current carries the water's pattern towards the
camera at `flow` whole tiles per loop (negative runs away; 0 is still). `foam` laps the banks, or the
walls when there is no verge, in a band `foam_width` metres wide that swells `lap` times a loop, in
`foam_color`; the bank or wall is darkened and glossy where the water wets it, up to `wet` metres
out. `floating` sets what drifts down on the current: `Foam` (patches or ice floes), `Leaves` or
`LilyPads` (a few in flower), at `float_density` per cell, `float_size` metres across, in
`float_color`. Rain rings the water instead of wetting it, and snow does not lie on it. For a
swimmer, put `camera.eye_height` near 0.3 and the horizon low (Lagoon Swim); for a boat, 1-1.3 m.
Light shafts fill the whole air under an open sky, so leave them off unless trees or walls break
the sun up.

**Bridges** (`path.bridge`). Every `spacing` metres the ground beside the path drops away for
`length` metres and the path crosses on a `deck`. Below it is `bottom`: ground `depth` metres down,
water, or a bottomless void. With water, `water_level` sets how far below the deck the surface lies
(it never goes deeper than `depth`), so a bridge can skim a stream or span a gorge. `railing` is one of:

| Railing | What it looks like |
|---|---|
| `Posts` | Wooden posts every 1.6 m with two rails, in the deck material. |
| `Parapet` | A solid low wall with a coping on top, in the wall material (the path material without walls). |
| `Balustrade` | Turned stone balusters on a plinth under a handrail, in the wall (or path) material. |
| `Iron` | Iron bars between two rails on a stone kerb, with a stouter post every few metres, in `rail_color`. |
| `Rope` | Ropes hanging between wooden posts, in `rail_color`. |
| `None` | An open deck. |

`rail_height` sets the height. `end_pillars` (on by default) puts a pillar or newel post with a cap
at each end of every railing: stone for parapets, balustrades and iron, wood for posts and rope.
Every railing is built as solid pieces, with faces on all the sides the camera can see. In the
studio, switching between Iron and Rope also switches `rail_color`, unless you have chosen a colour yourself.

### Weather and the air

Every effect here works in any scene, can be combined with the others, and repeats exactly with
the loop. Everything moving runs on loop time, so when the walk stops (an encounter) the rain keeps
falling. Each is off until `enabled`. Field by field, see
[SCENE_REFERENCE.md](SCENE_REFERENCE.md#weather).

| Effect | Where | What it does |
|---|---|---|
| Rain, snow, sleet, hail | `weather.precipitation` | Falls wherever the sky is open (not under a ceiling), at `intensity` from a few drops to a downpour or blizzard, slanted by the wind. Rain and sleet make the ground wet (darker and glossy, so lamps streak in it), leave puddles that mirror the scene with rings where drops land, and splash. Snow and sleet lie on the ground, the verges, the tops of walls and the tops of props, with a trodden track down the path. `haze` adds curtains of rain in the distance or a blizzard's whiteout. |
| Drips | `weather.drips` | Beads form on the ceiling (or the tops of the walls), fall and splash, from the same spots every time. For caves, sewers and crypts. |
| Wind | `weather.wind` | `speed` in m/s (negative blows left) with `gusts`. Rain and snow slant and drift, particles are carried, and trees, reeds, palms, grass and banners bend and sway (`sway`); rocks, pillars and crates stay put. |
| Sandstorm | `weather.sandstorm` | Sand streaming past, a sand-coloured haze that hides the distance, the sun dimmed to a disc. Follows the wind's direction. |
| Mist | `weather.mist` | Mist lying below `height` metres, drifting in banks (`patchiness`), with wisps rising. Each pixel takes as much mist as its line of sight passes through, so the ground near the camera stays clear and the distance goes white. Keep `density` near 0.1-0.2. `soft` 1 makes it thickest at the ground and thin smoothly to nothing at `height` (morning ground fog, dew lifting off a field), with no flat top. `over: Path` keeps it over the path only (the water of a river, a road), fading out `spread` metres past the edges, and the wisps then lift off the path like steam. `glow` brightens it looking towards the sun, as mist does lit from behind (Misty River, Dawn Road). |
| Light shafts | `weather.light_shafts` | `sun`: rays streaming past trees, walls and arches near the sun. `lamps`: glowing haloes in the air round every light. Both are stronger in fog, mist and haze. |
| Cloud shadows | `sky.clouds.shadows` | Shadows drifting over the ground and walls, with the wind if there is one. They need a sun that gives light, and work with the clouds themselves switched off. |
| Heat shimmer | `weather.heat_shimmer` | The distance wavers just above the horizon. Deserts, lava fields. |
| Aurora | `sky.aurora` | A band of curtains across a night sky: a thin bright folding edge (`low`), rays rising from it, a violet veil above (`high`) and pink knots (`accent`), each part fading in and out on its own. `coverage`, `rays`, `waves`, `arc`, `tall`, `edge` shape it; `ground_glow` tints the ground. |
| Deep space | `sky.space` | A sky with no air: a dense starfield (`stars`), glowing nebulae (`nebula`, two `nebula_colors`), a galaxy band with dust lanes and a bright core (`galaxy_*`), and up to three `planets`. With `below` on, space also fills everything below the horizon where nothing is drawn, such as a bridge's bottomless drop (`path.bridge.bottom: Void`), so a walkway floats in space. The sun loses its glow in the air and keeps only a tight glare. |
| Planets | `sky.space.planets[]` | `kind` Rocky, Gas (bands and a storm), Earth (oceans, land, ice caps, cloud, city lights on the night side), Ice or Lava (glowing cracks); `pos`, `radius`, two colours, `atmosphere` glow round the rim, `light_angle` and `night` for where the sunlight falls, `tilt`, `spin` in whole turns per loop, and `rings` (near half in front, far half behind, the planet's shadow across them). |
| Black hole | `sky.space.black_hole` | A black hole that bends the light round it: the stars, nebulae and galaxy behind are lensed into arcs, and the far side of its accretion disk shows over the top and under the bottom of the dark shadow, edged by a thin photon ring. `pos`, `size` (the shadow's radius), `tilt` (0 face-on, about 85 the classic edge-on look) and `roll`; the disk's `disk_inner`/`disk_outer` (in horizon radii), three `disk_colors` (hot inner to cool outer), `brightness`, `doppler` (the side turning towards you brighter and bluer), `spin` (whole turns of its inner edge per loop; the outer part turns slower); `lensing`; `infall` streaks of gas spiralling in from beyond the disk's edge, and `nodes` shaped like ball_swing_game's hook nodes (`node_size` as a share of the frame's width, `node_colors`): each floats in slowly and level from its side at play depth, untouched until it nears the middle, then is pulled back towards the hole `distance` times further off, shrinking in perspective to a speck and crawling on screen as it recedes (which is what says the hole is far away), settling into the disk's plane, reddening and fading at the rim. Every trip rolls its own side, height, size and pace, with pauses between (`infall_speed` trips per loop, `infall_color`); `light` on the scene. Planets are not lensed, so keep them clear of it (Event Horizon, Starship Flight, Black Hole Arena). `sky.space.star_colors` 0 makes every star the faint blue-white of the game's starfield. |
| Tunnel | `sky.tunnel` | A tube all round the path in place of the sky, filling everything not drawn (below the horizon too): `kind` Hyperspace (streaks of starlight rushing past) or Wormhole (swirling bands spiralling to a bright throat). `radius`, three `colors` (walls, streaks or bands, the far end's light), `rush` (extra loop lengths per loop), `twist` and `spin` (whole turns), `density`, `core`, `light` (tints the path). Pair it with a bridge over a void, as Hyperspace Run and Wormhole do. |
| Rainbow | `sky.rainbow` | An arc with a fainter, reversed second bow. `size` 1 spans the frame: a real 42-degree bow is wider than a portrait view. |
| On the lens | `weather.lens` | `Drops`: raindrops land on the lens, each showing the scene upside down, linger and run down. `Frost`: crystals creeping in from the edges. |
| Lightning | `weather.lightning` | Flashes that light the scene from the sky, and bolts where the sky shows. The export metadata gives each strike's frame, for thunder. |
| Fog banks | `weather.fog_banks` | Thick stretches of fog the camera walks into and out of. |

**Cost.** Falling rain and snow, splashes, mist and most effects cost little. Wet ground and puddles
cost what any glossy floor costs: the reflections roughly double a frame's render time (Dark
Street at 480 x 854: 11.6 ms dry, about 31 ms wet). Lower `wetness` and `puddles`, or render smaller,
if the live runtime needs the time back.

Presets that use them: Dark Street (rain, lightning), Mountain Pass (light snow, wind), Haunted
Forest and Bog Boardwalk (mist), Desert Canyon (heat shimmer), Mossy Sewer (drips), Forest Path
(breeze, sun shafts).

In the studio, switching a particle layer to another kind also switches its colour to that kind's
own, unless you had set the colour yourself.

`pf dump --preset NAME` prints any preset as a full scene to start from. `pf schema` prints the
JSON schema; `pf schema --markdown` prints [SCENE_REFERENCE.md](SCENE_REFERENCE.md). Scenes from
PathForge 2.0 are converted when they are opened.

## 3. The studio

```bash
path_forge
```

```bash
path_forge path/to/scene.json
```

The window has an outliner on the left (the scene's parts, its fixtures, prop layers, set pieces
and particles, with Duplicate and Delete), a live preview in the middle, an inspector on the right
generated from the scene format, and a timeline along the bottom.

- **Inspector.** Every field, with units, ranges and hover help. View > Advanced fields shows the
  rarely used ones too. Image and grade pickers copy files from elsewhere into the scene's
  `assets/` folder.
- **Timeline.** Play and pause (Space), step a frame (← →, ⏮ ⏭), or drag to any frame. It shows
  the frame number, the time within the loop, the metres walked, and how long the last frame took
  to render. The preview walks at the scene's `motion.speed` in real time.
- **Smooth playback.** While playing, the preview renders the frames ahead of the playhead on two
  threads at once and keeps them (up to 512 MB), so after the first pass through the loop it plays
  from memory. Each change to the scene starts the loop's cache again. While you drag a slider,
  only the current frame renders, and the newest finished frame shows meanwhile.
  `PF_STUDIO_STATS=1 path_forge scene.json` prints once a second how many frames were on time.
- **Camera guides** (View > Camera guides, or G) draw the path edges and distance marks.
- **File:** New from preset… (the gallery), Open… ⌘O, Save ⌘S, Save as… ⇧⌘S, Export… ⌘E, Show
  in Finder, Gather files into the scene folder, Pack into folder… (§14).
- **Edit:** undo ⌘Z, redo ⇧⌘Z, Remix… (variations that keep the parts you tick and take the rest
  from other presets), Transition / fork… (§9, §10).
- **Live reload.** The studio watches the scene file. When it changes on disk (another editor, an
  AI agent) it reloads; if you have unsaved edits it asks first: Load theirs or Keep mine.

## 4. Looks: styles, palettes, grades

`style` changes how the scene is drawn without touching the world:

| Field | Effect |
|---|---|
| `pixel_size` | Size of one art pixel. 1 = full resolution; 3 = render at a third and scale up crisply. |
| `palette` | `Full` colour, a named palette (`{"Named": "PICO-8"}`), a custom list, or `{"Auto": N}` colours chosen from the scene. |
| `dither`, `dither_strength` | Ordered (Bayer 2, 4 or 8) dither towards the palette. |
| `outline` | Outlines in a colour, around props and fixtures only or also along wall and ground edges. |
| `paint` | A painterly brush (Kuwahara filter) of the given size in art pixels. |
| `grade`, `grade_strength` | A built-in grade (Warm, Cool, Teal Orange, Faded, Night, Sepia, Vivid) or a `.cube` LUT file. |
| `paper`, `scanlines` | Paper texture; CRT scanlines. |

Named style presets (pixel art, Game Boy, painted storybook, toon, CRT, noir…) set several of these
at once. `pf styles --preset NAME -o styles.png` renders a scene in every style side by side, and
agents apply one with `pf_edit_scene`'s `style_preset`. `pf_palettes` lists the named palettes.

## 5. Props, kits and sprites

Besides the built-in prop kinds, a prop can be described by data:

- an image, a pool of images, or folders of PNGs (several images give variety), or
- a shape drawn from parts in metres (ellipses, rectangles, polygons and lines, each with a colour,
  shading and glow, or cutting a hole),

with its height in metres, whether it stands on the ground or hangs from the ceiling, how it glows
(a lantern's glass, a flame's lightest pixels), and the light it gives off.

Definitions live in a scene's `prop_defs` (used as `"def": "<name>"`), or in a **kit**: a folder
holding `kit.json` beside its images, used from any scene below it as `"def": "kits/<name>#<prop>"`.
The built-in kit `@wayside` (lantern posts, hanging lanterns, glowing mushrooms, signposts, fences,
barrels, crates) is drawn from shapes and needs no files.

```bash
pf kit --kit @wayside -o wayside.png
```

Sprites made with an image generator go straight into a kit. `pf_import_sprite` (MCP) takes a URL
(for example a PixelLab download link), a local file, or base64 PNG data, checks it, saves it in the
kit and adds it to a prop. Transparent margins are cut away when drawn, so every sprite stands on
its lowest visible pixel.

## 6. Exporting

**Studio:** File > Export… (⌘E). Pick formats, quality, the clip (Loop, Encounter or Transition),
frames (from the scene's motion, or a fixed count), size (1×, ¾, ½), folder and name. Before you
export it estimates the file sizes from a few frames encoded the same way, and shows the first
frame in the colours the file will have.

**Command line:**

```bash
pf export --preset "Forest Path" -o exports --formats webp,png,sheet
```

| Option | Meaning |
|---|---|
| `--formats` | Any of `gif,webp,apng,png,sheet,depth,layers` (comma-separated). |
| `--frames N` | Frames in the loop. Default: `loop_seconds × fps` from the scene's motion. |
| `--quality 0..100` | WebP quality. 100 is lossless. Default: 100 for pixel-art palettes, 90 otherwise. |
| `--lossy 0..1` | GIF lossy compression. 0.33 is below what the eye notices on smooth gradients. |
| `--dither on\|off` | GIF dithering. |
| `--name STEM` | File name stem. Default: the scene name. |
| `--clip` | `loop` (default), `encounter` (§8), `transition` (§9) or `fork` (§10). |

**Timing.** Frames are evenly spaced in distance, and the animation lasts exactly one loop:
`loop_length / speed` seconds. The fps setting only decides how many frames cover that time. To
make the walk faster or slower, change `motion.speed`. Per-frame delays are rounded so that they
add up to the exact loop length (for example 33, 34, 33… ms at 30 fps).

## 7. Export formats and their metadata

| Format | Files | Notes |
|---|---|---|
| `webp` | `<name>.webp` | Animated WebP: far smaller than GIF, full colour, lossy or lossless. Every frame is a key frame (sharper on slow gradients). Loops forever. |
| `gif` | `<name>.gif` | 256 colours: the scene's palette if it has one, otherwise the best 255 colours across the whole loop. GIF timing cannot go above 50 fps, so faster exports use every 2nd or 3rd frame (the export says so). |
| `apng` | `<name>.png` | Animated PNG: lossless and full colour, but large. |
| `png` | `<name>_frames/` | One PNG per frame: for engines that load frame sequences. |
| `sheet` | `<name>_sheet.png` + atlas JSON | Sprite sheet(s) with a TexturePacker/Aseprite-style atlas (frame rectangles and durations). Large loops are split into pages of at most the sheet size limit. |
| `depth` | `<name>_depth/` | A 16-bit grey PNG per frame. Value v means the surface is `d = near_m × 65535 / v` metres ahead (`near_m` = 0.25); v = 0 is sky. Used to hide game objects behind scenery: something d_obj metres ahead is hidden wherever d < d_obj. |
| `layers` | `<name>_layers/near/`, `mid/`, `far/` | The frame split by distance (default cuts at 4 m and 12 m) into transparent PNG layers, for parallax effects or drawing characters between layers. |

Every export also writes **`<name>.json`**, which tells a game how to play the loop and where
things are on screen:

| Key | Meaning |
|---|---|
| `frames`, `fps`, `loop_seconds` | Frame count, frame rate, loop duration. |
| `loop_length_m`, `metres_per_frame`, `speed_mps` | The loop in world terms. To follow a character, show frame `floor(distance_walked / metres_per_frame) mod frames`. |
| `files`, `sheet`, `depth`, `layers` | What was written, the sheet layout, the depth encoding, the layer bands. |
| `lightning` | The frame and time where each lightning flash peaks, for syncing thunder. |
| `camera` | The projection: `view` (horizon, centre, focal length in pixels, eye height, bend, hill, path half-width…) and the formulas to map a point x m right of the path, y m up and d m ahead to screen pixels, and the path's edges at any distance. |
| `scene` | The full scene that was rendered. |

### Viewing exports

Chrome, Firefox, Edge, Discord and game engines play animated WebP at its real speed. **Safari,
Preview and Quick Look on macOS do not.** Apple's image decoder rebuilds every frame of an
animated WebP by decoding all the frames before it. Measured on macOS 26.6 with a 480 × 854,
288-frame loop: frame 0 took 9 ms, frame 100 took 0.5 s, and frame 287 took 1.4 s, against 33 ms
per frame at 30 fps. Long animations therefore play slowly or stutter there. Frame flags make no
difference: files written by Google's own `img2webp` behave the same way. Preview may show only
the first frame. Check a WebP in a browser other than Safari, or with `vwebp` (from libwebp), or
compare it with `pf animcheck` (§17). Any export longer than 24 frames notes this.

## 8. Encounter clips

For a game where the hero stops to fight and then walks on:

```bash
pf export --preset "Stone Dungeon" -o exports --formats webp --clip encounter --slow 3
```

This writes three clips. `<name>_stop` eases from walking to a halt over `--slow` metres.
`<name>_idle` loops while standing: flames, particles and weather keep moving. `<name>_go` eases
back into the walk. Every junction matches exactly. `<name>_encounter.json` says when to start
each one: wait for the loop frame given as `start_frame`, play `stop` once, repeat `idle` until the
fight ends, then play `go` and continue the loop from the frame it names.

## 9. Transitions

A transition walks from one scene into another in a single unbroken shot. Both worlds are drawn
in the same frame and meet at a boundary on the road ahead, so nothing cuts or cross-fades: the
next place appears down the road and comes towards you.

**What PathForge decides for you.** Given the two scenes, the planner chooses:

- **The threshold** (what stands at the boundary). Between two open places it is a soft band
  where ground, verges, walls and props mix. Out of a roofed place (a dungeon, a cave) it is a lit
  doorway: daylight falls on the passage floor and the exit glares until the eye adapts. Into a
  roofed place it is a doorway in a stone face, or a cave mouth in a rock face or hillside. A gate
  or a portal can be asked for.
- **Where it first shows**: where the first world's fog or distance first gives it up, so it
  does not pop in.
- **The camera**: eye height, horizon, zoom and bend change from one scene's to the other's over
  a few metres either side of the boundary.
- **The look**: each frame takes the style of the world the camera is in.
- **Warnings** about what will not look right (very different camera heights, a bright world
  seen through a dark one, and so on).

Every value set to `Auto` or 0 in the `Transition` settings is decided by the planner. Set one to
override it: `threshold` (Open, Doorway, CaveMouth, Gate, Portal), `approach_m`, `blend_m`,
`facade` and `facade_height`, `light_spill`, `adaptation`, `camera_blend_m`, `style`, and `marker`
(an Archway, RuinedArch, Gate, Banners or Portal structure at the boundary).
[SCENE_REFERENCE.md](SCENE_REFERENCE.md#transition) lists them all.

**In the studio:** Edit > Transition / fork…, then "Transition into a scene".

1. Choose the scene to walk into (a preset, or File… for a scene file).
2. Read the plan: what PathForge chose and why, with any warnings.
3. Press ▶ to walk it, or drag through it. Change the threshold, structure, look and sliders;
   "PathForge decides" puts a value back on auto.
4. Export clips writes the transition with the formats and folder of the Export window. "Clips
   starting across the loop" (more than 1) writes several clips that begin at evenly spaced loop
   frames, so the game need not wait for frame 0.

**From a shell:**

```bash
pf transition --preset "Stone Dungeon" --to-preset "Forest Path" -o walk.png
```

```bash
pf transition --preset "Forest Path" --to-preset "Stone Crypt" --threshold cave --json
```

The first renders a contact sheet of the walk and prints the plan. `--json` prints the plan as
JSON. `--threshold`, `--marker` and `--approach` override the plan. To export the clip:

```bash
pf export --preset "Stone Dungeon" -o exports --formats webp --clip transition --to-preset "Forest Path" --entries 4
```

**What the export contains.** The clip starts exactly on the first scene's loop frame 0 and ends
exactly on the second scene's loop frame 0. Its `.json` says how to play it: play the first
loop until frame 0 comes round, play the clip once, then continue with the second loop from frame
0. With `--entries N` there are N clips (`_e0`, `_e1`…), each with the loop frame to start it at.

## 10. Forks

A fork parts the road into a left and a right branch, each leading into its own world. The player
chooses before the junction and the camera turns onto the chosen branch. If no choice has been
made by the time the camera reaches the junction, `default_branch` is taken.

The planner decides the angle between the branches (wide enough that the branch not taken leaves
the view once the camera turns), where the junction first shows, how each branch turns into its
own world, and how much ground is left between them. Override any of these in `ForkChoice`
(`angle`, `approach_m`, `blend_m`, `wedge_m`, `steer_m`, `camera_blend_m`, `default_branch`).

**In the studio:** Edit > Transition / fork…, then "Fork: the player chooses". Choose the left
and right scenes, set "If no choice" (the default branch), and use "Preview takes" to walk
either branch. Export clips writes the fork.

**From a shell:**

```bash
pf transition --preset "Forest Path" --left-preset "Mountain Pass" --right-preset "Desert Canyon" --take right -o fork.png
```

```bash
pf export --preset "Forest Path" -o exports --formats webp --clip fork --left-preset "Mountain Pass" --right-preset "Desert Canyon"
```

**What the export contains.** `<name>_approach` runs from the loop's frame 0 to the junction.
`<name>_left` and `<name>_right` each run from the junction into their world, ending on that
world's loop frame 0. The `.json` says how to play them: play the loop until frame 0, play
`_approach` once and ask the player during it, then play the chosen branch and continue with that
world's loop.

## 11. Journeys

A journey maps the places a game walks through. It is a `*.journey.json` file of stops, where each
stop is a scene and says what comes next:

```json
{
  "version": 1,
  "name": "chapter1",
  "start": "crypt",
  "stops": {
    "crypt":  { "scene": "scenes/crypt.json",  "next": { "Go": { "to": "forest", "transition": {} } } },
    "forest": { "scene": "preset:Forest Path", "next": { "Fork": { "left": "pass", "right": "canyon", "fork": {} } } },
    "pass":   { "scene": "preset:Mountain Pass", "next": "End" },
    "canyon": { "scene": "preset:Desert Canyon", "next": "End" }
  }
}
```

Scene names are files relative to the journey file, or `preset:<name>`. `Go` walks on through a
transition (its `transition` takes the settings of §9; `{}` lets PathForge plan it). `Fork` offers
a choice (its `fork` takes the settings of §10). `End` ends the journey.

```bash
pf journey --file chapter1.journey.json
```

```bash
pf journey --file chapter1.journey.json --out exports --formats webp,png --entries 2
```

The first checks the journey and lists its problems (a missing scene, a stop nothing leads to, a
`to` that does not exist). The second exports every stop's loop and every transition and fork
clip, plus `<name>.journey.json`, which maps each stop to its files and says how to play them.
Agents use `pf_journey` (`op`: read, write, preview, export). The live runtime (§12) and the
Quartz plugin (§13) play the journey file directly.

## 12. The live runtime

Instead of playing exported frames, a game can render the path itself every frame, at any speed
and resolution. Distance walked and time are separate inputs, so the hero can stop for a fight
while flames and weather keep moving, and the loop never shows a seam. Rendering is on the CPU
across threads. Measured on an M-series Mac: Stone Dungeon 1.3 ms at 135 × 240 and 11.8 ms at
480 × 854. Render small (or with a pixel style) and scale up.

**Rust** (no GUI or GPU dependencies):

```toml
path_forge = { path = "../path_forge", default-features = false }
```

```rust
use path_forge::runtime::Runtime;

// One scene, driven by your own distance and time:
let mut rt = Runtime::from_file("backgrounds/scenes/crypt.json")?;
let rgba = rt.render(distance, time, 270, 480);

// Or let the runtime walk, through a journey:
let mut rt = Runtime::from_journey("backgrounds/chapter1.journey.json")?;
rt.step(dt, speed);                 // each frame
let rgba = rt.frame(270, 480);
rt.go()?;                           // walk on to where the journey leads (returns the plan)
rt.choose(Branch::Right);           // at a fork, before the junction
let s = rt.state();                 // scene, stop, progress, choose_within, speed, end
```

| Method | Purpose |
|---|---|
| `new`, `from_file`, `from_json`, `from_journey` | Open a scene or a journey. |
| `render(distance, time, w, h)`, `render_into` | Draw one frame (RGBA8, rows top to bottom). |
| `step(dt, speed)`, `frame(w, h)` | Let the runtime walk; draw where it stands. |
| `go()` | Walk on to the journey's next stop: a transition, or the approach to a fork. |
| `transition_to(scene, dir, transition)`, `fork(left, right, fork)` | Start a transition or fork to any scene at run time. |
| `choose(branch)` | Take a branch of the fork ahead. Returns false when there is none or it is too late. |
| `finish()` | Jump to the end of what is under way. |
| `state()` | Where the walk stands: `scene`, `distance`, `time`, `stop`, `progress` (0..1 through a transition), `choose_within` (metres left to choose), `speed`, `end`. |
| `project(…)`, `project_now(x, y, d, w, h)` | A point x m right of the path, y m up and d m ahead → screen pixels. |
| `ground_distance(…)`, `ground_distance_now(row, w, h)` | A screen row → metres ahead. |
| `path_edges`, `view`, `camera_now` | Path edges at a distance; the camera. |
| `Walk::advance(dt, speed, &rt)` | A simple distance and time accumulator for `render`. |

**C and other engines** (C++, C# via P/Invoke, GDExtension, …) link the C library and use
`runtime/include/path_forge.h`; `runtime/examples/walk.c` is a complete example.

```bash
cargo build --release -p path_forge_runtime
```

Functions: `pf_runtime_open`, `pf_runtime_from_json`, `pf_runtime_open_journey`,
`pf_runtime_free`, `pf_runtime_canvas`, `pf_runtime_loop_length`, `pf_runtime_loop_seconds`,
`pf_runtime_render`, `pf_runtime_project`, `pf_runtime_ground_distance`, `pf_runtime_step`,
`pf_runtime_frame`, `pf_runtime_go`, `pf_runtime_transition_to`, `pf_runtime_fork`,
`pf_runtime_choose`, `pf_runtime_state` (JSON), `pf_runtime_project_now`. Return codes and
buffer sizes are documented in the header.

## 13. Quartz games and quartz_forge

`quartz_path_forge` (in `arty/synful_quartz/`) is a Quartz plugin. `PathForgePlugin` keeps a game
object showing the current frame:

- **Live:** `PathForgePlugin::live(scene or journey, object, size)` renders on a worker thread.
- **Frames:** `PathForgePlugin::frames("exports/x.journey.json", object)` plays exported PNG
  frames.

The game drives it with events: `next` (walk on), `choose:left` and `choose:right`, and, in live
mode, `scene:<file>`, `transition:<file>`, `fork:<l>|<r>` and `journey:<file>`. See that crate's
README.

**quartz_forge** (the Quartz editor) can use a PathForge scene or journey as a game's background.
Its preview runs the same runtime, so you can walk a journey through its stops and choose
branches from the editor.

## 14. Projects and files

PathForge keeps work in `~/Documents/PathForge` unless `PATH_FORGE_HOME` says otherwise. A project
folder holds `scenes/`, `assets/`, `kits/` and `exports/`, and scenes name their files relative to
themselves, so a project can be moved or committed with a game.

```bash
pf project --new ~/Games/MyRunner/backgrounds --preset "Haunted Forest"
```

- `pf assets --scene F` lists every file a scene uses (images, image folders, kits, grades) and
  whether each is there.
- `pf pack --scene F` copies anything from elsewhere into the scene's folder and makes the paths
  relative. `--out DIR` packs into a new folder instead. The studio does the same from File >
  Gather files / Pack into folder.

## 15. The `pf` command line

Scenes are given as `--preset NAME` or `--scene FILE`.

| Group | Commands |
|---|---|
| Looking | `presets`, `dump`, `render`, `sheet`, `gallery`, `styles`, `kit`, `schema [--markdown]` |
| Checking | `seam`, `bench`, `animcheck`, `animsheet`, `gifdiff` |
| Exporting | `export` (loops, encounters, transitions, forks) |
| Transitions, forks, journeys | `transition`, `journey` |
| Projects and the skill | `project`, `assets`, `pack`, `skill` |

`pf` with no arguments lists the commands; the top of `src/bin/pf.rs` gives every command with all
its options.

## 16. AI agents: MCP server and skill

`pf_mcp` is an MCP server on stdio. Its tools return images, so an agent can design a scene,
look at it, and export it:

| Tool | Purpose |
|---|---|
| `pf_presets`, `pf_schema`, `pf_palettes`, `pf_styles` | What there is to start from. |
| `pf_new_scene`, `pf_get_scene`, `pf_edit_scene` | Create, read and edit scene files (merge patches and pointer edits, validated, atomic, with a revision check). |
| `pf_render`, `pf_contact_sheet`, `pf_compare`, `pf_analyze`, `pf_check_loop`, `pf_camera` | Look at and measure a scene. |
| `pf_export` | Export (§6). |
| `pf_kit`, `pf_import_sprite`, `pf_pack`, `pf_remix` | Kits, sprites, packing, variations. |
| `pf_transition`, `pf_journey` | Plan and preview transitions and forks; read, write, preview and export journeys. |

Register it with your agent. `PATH_FORGE_HOME` is where relative paths resolve:

```json
{ "mcpServers": { "path_forge": {
    "command": "/path/to/path_forge/target/release/pf_mcp",
    "env": { "PATH_FORGE_HOME": "/path/to/your/game/backgrounds" } } } }
```

The agent skill `skills/pathforge/SKILL.md` teaches the design loop: brief, start from a preset,
small edits, look before claiming, style, loop check, export choices, transitions and forks, and
the pitfalls. `pf_mcp` serves it as the prompt `pathforge` and the resource
`pathforge://skill/SKILL.md`. To install it for agents that read skill folders (Claude Code, Codex,
Copilot, Cursor):

```bash
pf skill --install .
```

## 17. Checking quality

- `pf seam` renders across the loop point and checks that the last frame leads into the first
  with no larger step than any other. With no scene it checks every preset.
- `pf animcheck --preset NAME --file X.webp` compares an exported file frame by frame with fresh
  renders, so you can tell an encoding problem from a scene problem.
- `pf animsheet --file X.webp -o sheet.png` lays out an export's frames, as decoded, in a sheet.
- `pf bench` times rendering. `--stages` adds where each frame's time goes (shading,
  reflections, mist, post…, loudest first; `g.*` are GPU passes). `--studio` renders as the
  studio preview does (with frame stats and click-to-select ids). `pf transition ... --stages`
  does the same for a walk across a transition or fork.
- Rendering runs on the GPU when there is one, the CPU renderer otherwise; `--engine cpu` (or
  `PF_ENGINE=cpu`) forces the CPU, and export metadata names the engine. `pf parity --engine gpu`
  checks the two draw the same picture (every preset, or `--scene FILE`); add `--walks` for
  frames of transitions and forks.
- `pf_analyze` (MCP) warns about scenes that usually look wrong: too dark, blown out, props or
  lights out of view, a path that does not read against its verge.

## 18. Troubleshooting

| Symptom | Cause and fix |
|---|---|
| A WebP shows one frame or plays slowly on a Mac | Safari, Preview and Quick Look (§7, Viewing exports). Use Chrome, Firefox, Discord or `vwebp`. |
| The preview stutters with many effects on | The first pass through the loop renders live; after that it plays from memory. Rain on glossy floors, still water, mist and light shafts cost the most: `pf bench --stages --scene file.json` names the expensive part. |
| The export walks faster or slower than wanted | Change `motion.speed` (m/s). fps only sets how many frames cover the loop. |
| A GIF looks banded | GIF has 256 colours. Use WebP, or a pixel-art palette, or dithering. |
| A GIF exported at 60 fps plays at 30 | GIF timing tops out at 50 fps; PathForge uses every 2nd frame and says so. |
| A scene file names missing images | `pf assets --scene F`, then `pf pack --scene F`. |
| Rain falls but the ground stays dry, or no rain shows | The scene has a ceiling: rain and snow fall only where the sky is open. Use `weather.drips` indoors. |
| Rain made the frame much slower | Wet ground reflects like any glossy floor. Lower `wetness` and `puddles`. |
| A transition looks wrong | Read the plan's warnings in the studio or with `pf transition … --json`, then set the threshold, approach or camera blend yourself. |
