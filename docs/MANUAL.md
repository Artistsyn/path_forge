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

There are 45 presets: Stone Dungeon, Stone Crypt, Mossy Sewer, Forest Path, Desert Canyon, Night
Road, Magic Cavern, Ice Dungeon, Ruins Path, Dark Street, Mountain Pass, Volcanic Rift, Haunted
Forest, Ruined Castle, Fiery Dungeon, Tower Stair, Bog Boardwalk, Desert Ruins, Ice Cave, Day and
Night, Village Evening, Lily Harbour, Waterfall Glen, Miniature Village, Village Side Scroll, Village Corner, Cabin Woods, Star Bridge, Nebula Crossing, Asteroid Belt,
Hyperspace Run, Wormhole, Starship Flight, Event Horizon, Black Hole Arena, River Run, Canal
Lanterns, Jungle River, Lagoon Swim, Sunset Pier, Frozen Channel, Misty River, Dawn Road, Winding
Trail and Hyperspace Jump.
Every one loops without a seam (`pf seam` checks them all).

## 2. Scenes

A scene is a JSON file (`version: 3`) describing one place. Every field is optional; a missing
field takes its default. Units: lengths in metres, angles in degrees, colours `[r, g, b]` 0..255,
times in seconds.

| Part | What it controls |
|---|---|
| `canvas` | Output size in pixels (default 480 × 854, portrait; each side 64..4096, at most 4096 × 2160 pixels in all). The phone apps preview at the canvas's shape (or, set under Shape, the whole screen) and draw Still, Video and GIF/WebP at it; wallpapers and Live Photos always fill the screen. Their Canvas section stages a new size (typed, turned portrait ⇄ landscape, or picked) until Apply. |
| `camera` | Eye height, horizon position, zoom, lens curve, the angle it looks from (`yaw`, `side`; side-scrollers): see [Camera angle and side-scrollers](#camera-angle-and-side-scrollers), and `moves`: see [Camera moves](#camera-moves). |
| `path` | The road: width, flare, bend (curving left or right), hill (rising or dipping), ragged edges, material and pattern (cobblestone, brick, stone block, sand, dirt, grass, planks, water, ice, and the futuristic panels, grid, hex and circuit, whose lit parts glow with `material.glow`), stairs, bridges, side paths, roads that divide in two or three, side passages that branch off as scenery, and `edge_lights` (glowing lines along both edges, solid or dashed, the dashes running ahead with `flow`). `surface: false` draws no ground at all: the camera flies the route through open space (Starship Flight, Hyperspace Jump), with floating props, companions and the sky all round. |
| `verge` | The ground beside an open path: its material and grass tufts. Off, the path edge drops into void (walls usually cover it). |
| `walls` | Walls either side (a corridor, a canyon, a street) and their material. |
| `ceiling` | A roof at a height, with its material. |
| `sky` | Sky gradient (top and horizon colours), sun (with the bright aureole round it), moon (with phase and craters), stars, clouds (with drift and shadows on the ground), aurora, rainbow, and deep space (starfield, nebulae, a galaxy band, planets with rings). |
| `light` | Ambient light and its colour, the colour of the far distance, fog, and cel-shaded light bands. |
| `fixtures` | Repeating lights: torches, lanterns, candles, braziers, crystals, fireflies, magic, green fire, ice wisps, street lamps, paper lanterns, campfires, glowshrooms, beacons and chandeliers, mounted on walls, the ground or the ceiling, or floating, on posts where there is no wall, scattered or in swarms. See [Lights](#lights). |
| `props` | Layers of repeating objects: trees, pillars, rocks, barrels, banners, things of space (asteroids, satellites, debris, cargo pods, comets), or data-defined props (§5). `float` lifts a layer off the ground (asteroids, drifting islands, lanterns), over a bridge's drop too; `orbit` sends it round the path in a turning spiral. See [Scatter, lean and orbit](#scatter-lean-and-orbit). |
| `ground_cover` | Up to three layers of grass, tall grass, wildflowers or moss growing on the verge, the path or both. See [Ground cover](#ground-cover). |
| `set_pieces` | Structures spanning the path at intervals: archways, gates, banners. |
| `companions` | Things travelling along with the camera at a fixed place in the frame: a ship flying beside the path (drawn, or any image seen from behind via `sprite`), following its bends, bobbing and weaving in whole cycles per loop, its engines lighting what is near. |
| `particles` | Dust, embers, fireflies, rain, snow, leaves, ash, spores, sand, petals. They drift with the wind. |
| `weather` | Rain, snow, sleet and hail with what they do to the ground, drips, wind, sandstorms, mist, light shafts, heat shimmer, drops or frost on the lens, lightning and fog banks. See [Weather and the air](#weather-and-the-air). |
| `prop_defs` | Props described by data, named for the scene's prop layers to use. |
| `post` | Exposure, contrast, saturation, bloom (a tight glow round every flame and the sun, falling off into a wide veil), `flare` (what bright light does in the lens: a `style` of streaks, ghosts or haloes, with `strength`), vignette, grain, tint, and `focus`: depth of field or tilt-shift (see [Camera moves](#camera-moves)). |
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

**Cloud kinds.** `sky.clouds.kind` is `Cumulus` (heaped fair-weather clouds), `Towering`, `Stratus`
(a flat grey sheet), `Mackerel` (ripples), `Cirrus` (high streaks), `Overcast`, `Rain`, `Snow`
(dark, heavy decks that read as weather), `Storm` (a black deck with ragged scud under it) or `Soft`
(the older round puffs). `cover` is how much of the sky they fill, `shade` how dark their undersides
are. Heaped kinds are lit through their depth from the sun's side, so they have a bright edge and a
shaded body. `sky.cloud_layers` adds up to three more layers over the first, each a full set of
cloud settings (cirrus above cumulus, say). On the GPU a cloud layer costs about 1-2.5 ms at
960 x 1708.

**Several kinds of cloud in one sky.** A layer's `patches` (0..1) gathers it into patches with open
sky between them, so layers of different kinds share the sky instead of each covering all of it:
heaps over here, a mackerel sky over there. `sky.cloud_mix` makes such layers for you, at random:
`layers` (1-4) of the `kinds` ticked (none ticked: cumulus, towering, stratus, mackerel and cirrus),
each with its own cover, size, raggedness and shade round `cover`, as different from each other as
`variety` says, in `patches`. The same `seed` always gives the same sky; the dice next to it in the
studio picks another. They are drawn over `clouds` and `cloud_layers`, highest first.

**Snow cover.** `weather.snow_cover` lays snow that has already fallen, with none falling: on
trees, rocks and the rest of the scenery (`props`: a cap along their tops at 0.2, branches laden
with it at 1, caught on the tops of their clumps in patches) and on the ground, walls and railings
(`ground`). Any scene becomes a winter one; falling snow (`weather.precipitation`) lays its own, and
whichever is deeper shows.

**Sunsets.** `sky.sunset` turns the sky into a sunset (or sunrise) of one of four `style`s: `Golden`,
`Fiery`, `Pastel`, or `Dusk` (just after the sun has gone: switch the sun off and keep its place).
`amount` takes the sky's colours, the sun, the clouds' tint and the light towards the style's; a
glow lies along the horizon, brightest under the sun's place across the sky and fading up through
the style's colours (`glow` how bright, `height` how high it climbs, `spread` how far round it
reaches); clouds are lit from below, gold and orange near the horizon and the sun, pink higher up,
violet where they turn away, and darker against the light right by the sun (`clouds` how much);
`light` warms the sun's light and the fill. Put the sun low (`sky.sun.pos` y 0.85-0.95). Water
mirrors all of it. The preset Sunset Pier shows one, with a cloud mix.

**The sun** is a small hot disc inside a wide aureole that brightens the sky round it, rather than a
painted shape; with `post.bloom` and `post.flare` it also glows and flares in the lens.

**Bloom at night.** In the world only light brighter than white paper glows (as exposed: raising
`post.exposure` makes more of it glow). A sky is judged against itself: under a dark sky the moon,
an aurora and bright clouds glow though they are far dimmer than that, and each brighter star gets
a soft glow that grows with `post.bloom`. A day sky, deep space and tunnels keep the world's
threshold. Around 0.3-0.6 is a soft glow; above 1 it is strong.

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
`float_color`. `floating: Ice` drifts floes down it, and `frozen` (0..1) freezes it over from the
banks inwards: a shelf of cracked, snow-dusted ice with open water left in the middle, until at 1
it is closed from bank to bank (Frozen Channel). `cascades` (0..1) adds steps of white water across
the channel every `cascade_spacing` metres, where it tumbles over rock and carries foam away
downstream (River Run). Rain rings the water instead of wetting it, and snow does not lie on it. For a
swimmer, put `camera.eye_height` near 0.3 and the horizon low (Lagoon Swim); for a boat, 1-1.3 m.
Light shafts fill the whole air under an open sky, so leave them off unless trees or walls break
the sun up.

**Winding paths** (`path.winding`). With `enabled` the path swings from side to side as it goes:
`amplitude` metres either way, a bend every `length` metres (snapped so the loop closes), `turn`
for how much the camera turns to follow it (0 looks straight ahead while the path snakes across the
view, 1 keeps the path under the camera), `irregular` to make the bends uneven, `phase` to slide
them along. Everything beside the path follows it. See Winding Trail.
The sky turns with the camera: the sun, moon, stars, clouds, aurora, planets and rainbow slide
across the frame by as much as the camera has turned, out of the frame on a sharp bend and back
again, and the light of the sun and moon swings round with them, so shadows turn too. A steady
`path.bend` does not turn the sky (the view of a steady curve never changes, and a sky that kept
turning could not come back to where it started each loop).

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
at each end of every railing: stone for parapets, balustrades and iron, wood for posts and rope;
a stone pillar stands on a plinth. Every railing is built as solid pieces, with faces on all the
sides the camera can see (a side view sees the far ends too). `thickness` (0.4 m by default) is the
deck's depth, its edge showing in the deck's material from beside the path; 0 makes it a sheet. In the
studio, switching between Iron and Rope also switches `rail_color`, unless you have chosen a colour yourself.

### Camera angle and side-scrollers

`camera.yaw` turns the camera for the whole loop, in degrees from straight down the path (-90 ..
90, positive to the right), and `camera.side` sets where it stands, metres sideways from the
middle of the path. Everything is drawn from that angle (the ground, walls, bridges, fences,
props, lights, weather), the loop still closes, and the GPU draws the same frame as the CPU.

- **An angle on the path**: yaw 20-35 with the camera at the path's edge (`side` -1 for a turn
  to the right) looks along the path diagonally (preset Village Corner).
- **A side-scroller**: yaw -90 with the camera 4-10 m out to the right (`side` 5) looks straight
  across the path; the walk carries the world past from right to left, with true parallax (near
  things pass faster than far ones), seamlessly. Yaw 90 with `side` -5 looks the other way. A
  landscape canvas (854 x 480) suits it (preset Village Side Scroll). Seen from the side the
  path's bends are straightened, set pieces that span the path (arches, gates) stand face on
  where the path goes through them, as a side-scroller shows a gate, and the wall,
  ceiling, fences, lights and props on the camera's side are cut away beyond `camera.cutaway`
  metres past the path's edge, so the walking lane stays in view. Raise `eye_height` and lower
  `horizon` to see more of the lane.
- **Where the sun stands**: by default the sky keeps its place in the picture whichever way the
  camera looks (the sun where `sky.sun.pos` puts it), and its light and shadows turn with the
  camera. Turn on `camera.world_sky` and the sky keeps its place in the world instead: `pos` is
  where the sun and moon stand seen down the path, and turned to the side the view swings away
  from them, the clouds and stars with them, so a sun ahead of the walk lies off to one side of a
  side-scroller, its light and shadows coming from where it is. Two scenes of one place, one down
  the path and one from the side, then share one sun.
- **A swing in** (`camera.moves.swing_in`): played once at the start, like the flight in, the
  camera comes round from another view into the scene's own: from behind the walker (`from`:
  `yaw`, `side`, `back` metres behind, `eye_height`, `horizon`, `zoom`) round to a side-scroller's
  view, or the other way. It takes `seconds` after a `hold`, eased (`Smooth`, `Out` for a reveal,
  `In`, `Linear`), with an `overshoot` like a crane's if you like. With `orbit` the camera circles
  the spot on the path both views look at, so that spot (where a game's hero stands) stays in the
  middle of the frame; without, it moves straight across. As it comes round to the side the near
  side of the world (props, lamps, fences, the wall and ceiling) dissolves out of its way in an
  ordered pattern, before the camera reaches it, the path's bends straighten and buildings that
  face the path once turned come round to it; going the other way all of that comes back once
  the camera has passed. `bars` puts letterbox bars over the top and bottom that slide away as it
  ends. With `camera.world_sky` the sun and the sky sweep across as the camera turns. The game's
  runtime reports the seconds left (`intro_left` in its state) so it can hold the player's
  controls until the swing has landed, and places things on screen by the swinging camera.

**Props from an angle.** A prop's `facing` says which way its picture faces. `Camera` (the
default, and every built-in kind) keeps it turned to the camera: right for trees, rocks and lamps,
which look alike from every side. `Path` stands it along the path like a building's front, so from
an angle it is seen obliquely and from the side squarely; `PathWhenTurned` faces the camera while it
looks down the path and the path once `yaw` turns it, so a scene looks as it always has head-on and
its buildings line the street from any angle. With `depth` (metres behind the front) the picture's
ends are drawn as plain side walls that deep, so a building seen from an angle reads as a solid
block, not a card. Set both on a prop definition, or per layer (`props.*.facing`, `props.*.depth`)
over it.

**Faces.** For the rest of a building as it really looks, a definition lists its `faces`: flat
pictures standing in the prop's own space (metres at scale 1, `x` across its front, `y` up, `z`
back from the path), each placed by its picture's top left, top right and bottom left `corners`
as seen from the side it faces, and painted from `shape` parts (metres from its bottom left
corner) or an `image`. `mirror: true` draws it again on the other side (`x` to `-x`). A cottage's
faces are its long side walls with their windows, the two slopes of its roof running back from
the gable and its chimney's sides; they replace the plain end walls. The settlement kit's
cottages, log cabins, market stall, cart and woodpile have them and face the path (`Path`), so even looking
straight down a village street you see their side walls and roofs in perspective, as in a real
street. A layer with `facing: PathWhenTurned` brings back the flat front that faces the camera
until the camera turns.

**Tubes.** Round parts are better as `tubes`: real cylinders, each `from` one point `to` another
in the prop's space with a `radius`, a bark `color` and an `end` colour for its sawn ends (with
growth rings), `mirror` for both sides. They are drawn true from any angle, their outlines soft,
shaded round across their height by the same curve as the kit's painted logs, so a tail continues
its wall log seamlessly. The log cabins' corners are tubes: each wall's logs run on past the corner as tails,
the two walls' a half log apart in height, so the tails stack out both ways by turns.

**Variety.** A definition with `pick` (a list of other definitions' names, in the same kit or the
scene's `prop_defs`) is drawn as one of them per prop, the same one each time round the loop, so
a street is not one house over and over. `@settlement#cottage` picks among five builds: timber
framed (`cottage_timber`), stone (`cottage_stone`), whitewashed under thatch (`cottage_thatch`),
two storeys of brick (`cottage_brick`) and weatherboarded (`cottage_board`); `@settlement#log_cabin`
among a small cabin with a stone chimney up its side (`log_cabin_small`), one with a front porch
(`log_cabin_porch`) and one with a turf roof (`log_cabin_sod`). Name one of them to use just that
build. The preset Cabin Woods walks a woodland trail past log cabins at dusk. A smoke plume's
`back` sets how far behind the front it rises (a chimney on the ridge). To look down
on a scene, raise `eye_height` and move `horizon` up: verticals stay vertical.

### Camera moves

`camera.moves` moves the camera on top of the walk. Everything here repeats a whole number of
times a loop, so loops still close, except the flight in, which plays once.

| Field | What it does |
|---|---|
| `bob`, `stride`, `sway` | The head bobbing at each step (metres; 0.03-0.05 a walk, 0.1 a run), the length of a step, and a sway from side to side with the steps (degrees). It follows the distance walked, so it stops when the walk does. |
| `pan`, `pans` | Looking slowly to the side and back: degrees (negative looks left first), and how many times a loop. The sky turns with the rest. |
| `tilt`, `tilts` | Looking slowly up and down. |
| `dolly`, `dolly_focus`, `dollies` | A dolly zoom: the camera draws back `dolly` metres while zooming in, so what stands `dolly_focus` metres ahead keeps its size while the background swells and shrinks round it (negative pushes in and zooms out). |
| `fly_in`, `fly_height` | The flight in: over `fly_in` seconds the camera comes down from `fly_height` metres, looking down, to the walker's eyes. It plays at the start of the live runtime (`restart_intro` plays it again) and with `pf render --intro` / `pf sheet --intro`; loops leave it out, `pf export --clip intro` writes it as a clip of its own. Best under an open sky. |
| `swing_in` | The swing in: the camera comes round from another view (`from`) into the scene's own, once at the start (see Camera angles). Played like the flight in; `pf export --clip intro` writes it as a clip that ends where the loop begins. |

`post.focus` is depth of field: with `mode: Depth` what stands `distance` metres ahead is sharp
and nearer and further things blur (the far distance and sky the most), up to `blur` (the widest
blur, as a share of the frame height, 0.005-0.015). Lights and stars become soft discs. With
`mode: TiltShift` a band across the frame (`band` its middle, from the top, `band_width` its
width) stays sharp and the rest blurs, which with a raised camera and some extra saturation
makes the scene look like a model (preset Miniature Village). The vignette and grain are in
`post` too. Waterfall Glen bobs gently, looks round once a loop and flies in at the start.

### Lights

`fixtures.*.kind` sets what the light is; the rest sets where it stands and how it varies.

| Field | What it does |
|---|---|
| `post` | What holds it up where there is no wall: `Auto` (a street lamp on a pole, a paper lantern from a crook, a wall light with no wall on a pole, anything else raised on a pedestal), `Off`, `Pole`, `Crook` (a tall post with an arm the light hangs from) or `Pedestal`. |
| `place` | `Anywhere`, `Bridges` (only on the path's bridges), `OffBridges`, or `SetPieces`: one at each repeat of `set_pieces[set_piece]`, at that set piece's spacing, with `offset` measured from it (torches on every arch). |
| `bridge_posts`, `posts_between` | On the path's bridges, stand on the railing's posts instead of at `spacing`: one on each end post of every bridge, each side, and `posts_between` more evenly between them (1 halfway, 2 at the thirds, 3 at the quarters...); the railing gains those posts. The light stands above the post's top (higher if `height` says so). |
| `tint` | A colour multiplied over the light and its flame. |
| `jitter`, `jitter_along`, `height_var` | Random placement: sideways (metres), along the path (as a share of the spacing) and in height (metres). |
| `size_var`, `intensity_var`, `color_var` | Each one a little larger or smaller, brighter or dimmer, warmer or cooler. |
| `density`, `cluster` | The share of places that hold a light, and how many stand at each (1..8). A cluster with jitter is a swarm: fireflies at every height. |
| `stagger` | With lights on both sides, how far the right-hand row is out of step (0.5 = left and right take turns). |
| `string_lights` | Festoon lights strung between the lights' posts (or brackets), sagging in a curve: `bulbs` per span, `sag` (metres at the middle), `color` or a repeating list of `colors`, `brightness`, `size`, and `twinkle`. The wires are drawn too (Village Evening). |

`kind: Neon` is a neon sign for city streets: bent tubes on a dark board that glow in the
fixture's `tint` and buzz with `flicker`. Flames sway, lick at their tips and throw embers; a
brazier is a forged basket glowing from the coals.

A beacon pulses; a chandelier carries five flames; glass, paper and glowing caps are lit from
inside. A post is not lit by its own lamp, so it reads dark against the glow.

### Scatter, lean and orbit

Prop layers have `jitter` (sideways), `jitter_along` (along the path, as a share of the spacing),
`stagger` (the right-hand side out of step with the left), `scale_var`, `tint_var` and `lean_var`
(each one bowing a little to one side: 0.04 is a natural stand of trees). The presets' trees, bushes
and rocks stand unevenly and lean; pillars and obelisks stand in their rows.

`orbit` sets a layer round the path instead of beside it: each prop on a ring of `radius` (spread
by `radius_var`) about the path's line at `height` (0 = eye height, the middle of a tunnel), the row
winding round it `twist` whole turns per loop length and the ring turning `spin` whole turns per
loop. With `with_tunnel` (the default) both come from `sky.tunnel`, so the props spiral with a
wormhole. `side: Both` makes two strands and `rows` more; `stray` loosens them. Props hanging in a
tunnel are lit by its walls. See Wormhole.

### Ground cover

`ground_cover` holds up to three layers, each a `kind` (`Grass`, `TallGrass`, `Wildflowers`,
`Moss`) growing `on` the `Verge`, the `Path` or `Both`, with `density`, `height`, two colours
(`color` at the foot, `color2` at the tips or the flower heads), `patchiness` and a `seed`. Blades
and flowers stand up from the ground and are seen through one another, near ones over far ones;
they sway with the wind. Moss is a soft cushion pattern on the surface. On the GPU one grass layer
costs about 1.5 ms at 480 x 854 and a second about 0.5 ms more.

### Weather and the air

Every effect here works in any scene, can be combined with the others, and repeats exactly with
the loop. Everything moving runs on loop time, so when the walk stops (an encounter) the rain keeps
falling. Each is off until `enabled`. Field by field, see
[SCENE_REFERENCE.md](SCENE_REFERENCE.md#weather).

| Effect | Where | What it does |
|---|---|---|
| Rain, snow, sleet, hail | `weather.precipitation` | Falls wherever the sky is open (not under a ceiling), at `intensity` from a few drops to a downpour or blizzard, slanted by the wind. Rain and sleet make the ground wet (darker and glossy, so lamps streak in it), leave puddles that mirror the scene with rings where drops land, and splash. Snow and sleet lie on the ground, the verges, the tops of walls and the tops of props, with a trodden track down the path. `haze` adds curtains of rain in the distance or a blizzard's whiteout. |
| Drips | `weather.drips` | Beads form on the ceiling (or the tops of the walls), fall and splash, from the same spots every time. For caves, sewers and crypts. |
| Wind | `weather.wind` | `speed` in m/s (negative blows left) with `gusts`. Rain and snow slant and drift, particles are carried, and trees, reeds, palms, grass and banners bend and sway (`sway`); leaves stir within a tree's crown and a willow's strands swing and trail downwind while trunks stand (`flutter`); rocks, pillars and crates stay put. |
| Sandstorm | `weather.sandstorm` | Sand streaming past, a sand-coloured haze that hides the distance, the sun dimmed to a disc. Follows the wind's direction. |
| Mist | `weather.mist` | Mist lying below `height` metres, drifting in banks (`patchiness`), with wisps rising. Each pixel takes as much mist as its line of sight passes through, so the ground near the camera stays clear and the distance goes white. Keep `density` near 0.1-0.2. `soft` 1 makes it thickest at the ground and thin smoothly to nothing at `height` (morning ground fog, dew lifting off a field), with no flat top. `over: Path` keeps it over the path only (the water of a river, a road), fading out `spread` metres past the edges, and the wisps then lift off the path like steam. `glow` brightens it looking towards the sun, as mist does lit from behind (Misty River, Dawn Road). `from` leaves that many metres of clear air before it begins: fog lying in the low ground ahead, a valley filled with it. |
| Light shafts | `weather.light_shafts` | `sun`: rays streaming past trees, walls and arches near the sun. `lamps`: glowing haloes in the air round every light. Both are stronger in fog, mist and haze. |
| Cloud shadows | `sky.clouds.shadows` | Shadows drifting over the ground and walls, with the wind if there is one. They need a sun that gives light, and work with the clouds themselves switched off. |
| Heat shimmer | `weather.heat_shimmer` | The distance wavers just above the horizon. Deserts, lava fields. |
| Aurora | `sky.aurora` | A band of curtains across a night sky: a thin bright folding edge (`low`), rays rising from it, a violet veil above (`high`) and pink knots (`accent`), each part fading in and out on its own. `coverage`, `rays`, `waves`, `arc`, `tall`, `edge` shape it; `ground_glow` tints the ground. |
| Seasons | `season` | The time of year over the scene's own colours (taken as summer's): `season` 0 spring (fresh light green, blossom drifting down), 1 summer, 2 autumn (trees turning gold, orange and red, grass golden, leaves falling), 3 winter (broadleaf trees bare, grass straw, snow lying with `snow`, cooler light); fractions turn one into the next. `years_per_loop` runs whole years through a loop. `falling` adds the blossom and leaves. |
| Halo | `sky.halo` | The ring round the sun or moon (`sun`, `moon`) seen through thin high cloud: sharp and reddish on its inside, fading outwards; `size` its radius, `strength`. |
| Day and night | `sky.day_night` | The hour of the day (`hour`, 0..24) sets everything at once: the sun on its arc from `sun_path[0]` to `sun_path[1]` (as high as `sun_height`; past 1 it passes overhead, out of the frame) and its colour, the sky (dawns and dusks in `sunset_style`, with their glow, then the blue hour and night), the light and its colour, the fog and mist, the stars coming out after dusk, the moon crossing the night, and the lamps lighting at nightfall (`lamps`; fireflies only at night). The scene keeps its own hues (`keep_look`) while their brightness follows the hour, so a night scene stays moody by day and a sunny one goes dark at night (`night_light`). `days_per_loop` runs whole days through each loop, seamlessly (preset Day and Night); `real_time` follows the phone's clock (the Android wallpaper redraws as the hours pass). Games set the hour themselves (§12). `sunrise`/`sunset` move the day. No effect under space or a tunnel, or with the sky off. `pf render --hour H` draws any scene at hour H. |
| Deep space | `sky.space` | A sky with no air. `star_style` Realistic (the default) draws stars as a sky photograph shows them: a few bright ones among a great many faint ones, coloured by temperature (most yellow-white, fewer orange or blue; `star_colors` how strongly), gathered in clouds and voids (`star_clumps`) over the faint glow of stars too far to see one by one (`star_glow`), the brightest with diffraction `star_spikes` (`spike_shape` Four, Six or Eight). `twinkle` makes them shimmer, `twinkle_speed` whole times per loop. Classic keeps the even field of small dots of earlier versions. Glowing gas (`nebula`, two `nebula_colors`, `nebula_scale`) drifts in folded wisps and bright filaments, threaded with dark dust that hides the stars behind it (`nebula_dust`); the galaxy band (`galaxy_*`) is the glow of countless stars, mottled with star clouds, warmer at its core and split by dark rifts and lanes (`galaxy_dust`). Up to six `objects` (below) and up to three `planets`. With `below` on, space also fills everything below the horizon where nothing is drawn, such as a bridge's bottomless drop (`path.bridge.bottom: Void`), so a walkway floats in space. The sun loses its glow in the air and keeps only a tight glare. |
| Objects | `sky.space.objects[]` | Things far off in the sky, behind the planets and bent round a black hole like the stars: `kind` Emission (a stellar nursery: glowing gas, a hot heart, dark dust pillars and young stars), Reflection (blue dust lit by a few bright stars), Dark (a dust cloud blotting out the stars, its edge lit), Planetary (a dying star's glowing ring round a white dwarf), Remnant (the filaments of an exploded star's shell), Spiral and Elliptical galaxies, Cluster (a ball of stars) Comet (a glowing head, a curved dust tail and a straight blue ion tail, pointing along `angle`), Binary (two stars circling each other, gas streaming between them), Pulsar (a dead star sweeping two beams round inside its wisps), Belt (a band of asteroids right across the sky, `size` its half thickness, drifting along), Sun (a sun seen close: a spotted disc darker to its rim, its corona, flares licking off its edge) and Ship (a far ship crossing the sky along `angle`, its engine glowing and trailing, wingtip lights blinking). `pos`, `size`, `angle`, `aspect` (1 round .. 0.1 edge-on), three `colors` (each kind's meaning is in the reference), `brightness`, `spin` (whole orbits, turns, drifts or crossings per loop), `seed`. Nebula Crossing and Asteroid Belt show several. |
| Shooting stars | `sky.meteors` | Streaks of light crossing the sky, flaring and fading as they burn up, by night or in space: `count` a loop (a shower: 10 or more), `duration`, `length`, `angle` and `spread` of their ways, `brightness`, `color`, `tint` (some burn green or orange), `seed`. Each falls at its own moment of the loop, so the loop closes; with a day and night they show only after dark. |
| Planets | `sky.space.planets[]` | `kind` Rocky, Gas (bands and a storm), Earth (oceans, land, ice caps, cloud, city lights on the night side), Ice, Lava (glowing cracks) or Desert (dunes, canyons, dust storms, small pale caps); `orbit` makes one a moon of another (`around` which, `distance`, `tilt`, `roll`, whole `turns` a loop, `phase`): it passes in front of its planet and behind it, and casts its shadow across it as it crosses; `pos`, `radius`, two colours, `atmosphere` glow round the rim, `light_angle` and `night` for where the sunlight falls, `tilt`, `spin` in whole turns per loop, and `rings` (near half in front, far half behind, the planet's shadow across them). |
| Black hole | `sky.space.black_hole` | A black hole that bends the light round it: the stars, nebulae and galaxy behind are lensed into arcs, and the far side of its accretion disk shows over the top and under the bottom of the dark shadow, edged by a thin photon ring. `pos`, `size` (the shadow's radius), `tilt` (0 face-on, about 85 the classic edge-on look) and `roll`; the disk's `disk_inner`/`disk_outer` (in horizon radii), three `disk_colors` (hot inner to cool outer), `brightness`, `doppler` (the side turning towards you brighter and bluer), `spin` (whole turns of its inner edge per loop; the outer part turns slower); `lensing`; `infall` streaks of gas spiralling in from beyond the disk's edge, and `nodes` shaped like ball_swing_game's hook nodes (`node_size` as a share of the frame's width, `node_colors`): each floats in slowly and level from its side at play depth, untouched until it nears the middle, then is pulled back towards the hole `distance` times further off, shrinking in perspective to a speck and crawling on screen as it recedes (which is what says the hole is far away), settling into the disk's plane, reddening and fading at the rim. Every trip rolls its own side, height, size and pace, with pauses between (`infall_speed` trips per loop, `infall_color`); `light` on the scene. Planets are not lensed, so keep them clear of it (Event Horizon, Starship Flight, Black Hole Arena). `sky.space.star_colors` 0 makes every star the faint blue-white of the game's starfield. |
| Tunnel | `sky.tunnel` | A tube all round the path in place of the sky, filling everything not drawn (below the horizon too): `kind` Hyperspace (streaks of starlight rushing past) or Wormhole (swirling bands spiralling to a bright throat). `radius`, three `colors` (walls, streaks or bands, the far end's light), `rush` (extra loop lengths per loop), `twist` and `spin` (whole turns), `density`, `core`, `light` (tints the path). Pair it with a bridge over a void, as Hyperspace Run and Wormhole do. |
| Rainbow | `sky.rainbow` | An arc with a fainter, reversed second bow. `size` 1 spans the frame: a real 42-degree bow is wider than a portrait view. `saturation` about 0.2 makes a moonbow: the pale bow the moon casts at night. |
| On the lens | `weather.lens` | `Drops`: rain on a lens or windscreen: fine beads that bend the view, heavy drops that gather and run down in wavering trails leaving beads behind, and a haze where the glass is wet (`size`, `trails`, `blur`). `Frost`: crystals creeping in from the edges. |
| Lightning | `weather.lightning` | Flashes that light the scene from the sky, and bolts where the sky shows. The export metadata gives each strike's frame, for thunder. |
| Fog banks | `weather.fog_banks` | Thick stretches of fog the camera walks into and out of. |
| Wildlife | `wildlife` | `birds`: flocks in V formation crossing the sky, wings flapping (`flocks`, `count`, `size`, `height`, `flap`, `crossing` seconds; not at night). `bats`: flitting low over the path at dusk (`dusk_only`) or all night. `fish`: leaping from the water beside the path and splashing back with a ring (`spacing`, `lateral` range, `leaps` a loop). Deer, stags, rabbits and herons stand in the kit `@wildlife`. |
| Waterfalls | `waterfalls[]` | Streams pouring over a rock face beside the path, the water drawn pixel by pixel: a dark glassy tongue over the lip, then a curtain of fine white threads that speed up as they fall (patterns carried down in falling time, so they stretch lower down), translucent where thin and fraying at the edges, with a splash crown, churn and a drifting plume of spray at the foot: `side`, `lateral`, `spacing`, `offset`, `height` and `width` in metres, `water` and `rock` colours, foam and `spray` at the foot and a `rainbow` in it when the sun is out (Waterfall Glen). |
| Fences | `fences[]` | Fences along the path, built post to post as solid pieces that follow its edge and bends (not cards), so they join up from any angle: `style` Picket (pickets on two rails, painted `color`), Rails (posts and three bare wooden rails, in the bridge deck's wood) or Boards (close boards under a capping rail); `side`, `lateral` metres out from the path's edge, `height`, `post_spacing`. Runs are `length` metres long every `spacing` metres: as long as the spacing it is one fence without a break, shorter leaves gaps for gates (Village Evening puts one in front of each cottage). Painted fences of a scene share the first one's colour. Each casts the sun's shadow on the ground, post by post and picket by picket, from any camera angle (`shadow`, `shadow_opacity`). |

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

The window has an outliner on the left (the scene's parts, its lights, prop layers, set pieces,
particles and ground cover), a live preview in the middle, an inspector on the right generated from
the scene format, and a timeline along the bottom.

- **Adding and removing.** `+` beside a list adds an item; the bin on a row removes it, and Clear
  empties the list. The inspector's heading has Remove, Duplicate and move up / down for the
  selected item, and Delete (or Backspace) removes it. Undo brings anything back. Lists inside a
  part (extra cloud layers, planets) have their own `+ Add` and Remove in the inspector.

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
- **File:** New from preset… (the gallery), Open… ⌘O, Save ⌘S, Save as… ⇧⌘S, Save as preset…,
  Export… ⌘E, Show in Finder, Gather files into the scene folder, Pack into folder… (§14).
- **Presets of your own.** Save as preset… keeps the scene, with copies of the files it uses, in
  `<home>/presets/<name>/`. The gallery shows yours first, each with a delete button that asks
  before deleting. `pf preset-save --scene FILE --name NAME` does the same from a terminal, and
  `--preset NAME` then finds it everywhere a preset is named.
- **Glow & shadow mask…** (beside any image field) opens the mask editor (§5).
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
The built-in kits are drawn from shapes and need no files:

- `@wayside`: lantern posts, hanging lanterns, glowing mushrooms, signposts, fences, barrels, crates;
- `@settlement`: a cottage with lit windows and a smoking chimney, a well, a market stall, picket
  fences, hay bales, a cart and a woodpile (Village Evening);
- `@waterside`: rowing boats, lily pads, mooring posts and a blinking buoy (Lily Harbour);
- `@wildlife`: deer (standing or grazing), a stag, a rabbit and a heron.

A definition's `smoke` (`at` within the image, `color`, `amount`, `rise`, `width`) gives it a
plume rising and drifting with the wind: a chimney, a campfire.

```bash
pf kit --kit @settlement -o settlement.png
```

**What glows and what casts a shadow.** Props, lights and set pieces are flat images (PNG with
transparency) stood upright; nothing needs to be 3D. An image can have a mask beside it,
`<name>.mask.png`, the same size: red is how much a texel glows, green whether it casts a shadow.
Glowing parts light themselves, give off light in their own colour from the middle of the glowing
part, and cast no shadow; solid parts take the scene's light and cast one; parts marked as neither
(smoke, glass) do neither. In the studio, Glow & shadow mask… beside an image field opens an editor
with a brush for each, a brush size, and Mark the light parts (everything above a lightness
glows). An image used as a light takes its flame's place and colour from the mask and only the halo
is drawn over it; used as a prop it lights the ground round it. `pf mask --image FILE --glow-from
0.7` writes a mask from lightness, `--remove` takes it away; `pf_import_sprite` takes
`mask_glow_from`. Masks travel with their images when a scene is packed or kept as a preset.

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

**Intro clips.** A scene with a swing in or flight in (`camera.moves.swing_in`, `fly_in`) exports
its intro as a clip of its own that ends where the loop begins:

```bash
pf export --scene village.json -o exports --formats webp --clip intro
```

Play `<name>_intro` once, then the loop from its frame 0.

**Splicing clips into one file.** To share a sequence, or play it where clips cannot be
sequenced, join exported clips one after another (`*N` plays a clip N times):

```bash
pf splice --clips exports/village_intro.webp,exports/village.webp*2 -o village_intro.webp
pf splice --clips forest.webp,forest_to_village.webp,village.webp*3 -o journey.gif --once
```

It reads gif, webp and folders of png frames (`--formats png`), writes a gif or a webp
(`--quality` for webp, `--once` to play once instead of looping), and prints the colour step
across each join next to the clips' own steps, warning where a join jumps (clips that were not
exported to follow each other). GIFs joined into a gif keep their palettes and pixels exactly.
MCP `pf_splice` does the same.

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

**Between views.** When the two scenes look from different angles (a side-scroller and a view
down the path), the camera swings: out of the first scene's view to look down the path before the
boundary, across it, and round into the second scene's view after it, circling the walker and
dissolving the near side of the world out of its way as an intro's swing does. `swing` sets how
(`seconds` each way, 0 = 2.5; `hold`, `ease`, `overshoot`, `orbit`, `bars`); negative seconds
crossfade the views at the end instead. Give both scenes one canvas: the frame cannot change
shape partway through.
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
let s = rt.state();                 // scene, stop, progress, choose_within, speed, end, day

// Day and night, from the game's own clock:
rt.set_hour(Some(19.5));            // dusk on every scene with an open sky
rt.set_day_length(600.0);           // or let a day pass every 10 minutes of step time
if rt.day().is_some_and(|d| d.daylight < 0.2) { /* night: spawn the bats */ }
rt.restart_intro();                 // fly in / swing in again (camera.moves), as at the start
if rt.state().intro_left.is_some() { /* hold the player's controls until the camera lands */ }
rt.swing_to(SwingView { yaw: -90.0, side: 6.0, ..rt.view_now() }, Swing::default())?; // round to the side
```

| Method | Purpose |
|---|---|
| `new`, `from_file`, `from_json`, `from_journey` | Open a scene or a journey. |
| `render(distance, time, w, h)`, `render_into` | Draw one frame (RGBA8, rows top to bottom). |
| `step(dt, speed)`, `frame(w, h)` | Let the runtime walk; draw where it stands (with the camera's flight in at the start, `restart_intro` to play it again; C `pf_runtime_restart_intro`). |
| `go()` | Walk on to the journey's next stop: a transition, or the approach to a fork. |
| `transition_to(scene, dir, transition)`, `fork(left, right, fork)` | Start a transition or fork to any scene at run time. |
| `choose(branch)` | Take a branch of the fork ahead. Returns false when there is none or it is too late. |
| `finish()` | Jump to the end of what is under way. |
| `state()` | Where the walk stands: `scene`, `distance`, `time`, `stop`, `progress` (0..1 through a transition), `choose_within` (metres left to choose), `speed`, `end`, `intro_left` and `swing_left` (seconds left of the intro or a swing). |
| `swing_to(view, swing)`, `swing(spec)`, `view_now()` | Swing the camera from where it is (even partway through another swing) to another view (`yaw`, `side`, `eye_height`, `horizon`, `zoom`), the way `swing` says; the view stays once it lands. `swing` takes a name (`home`: the scene's own camera; `ahead`; `side`; `side_left`) or JSON as the C function does. The camera's view now. C `pf_runtime_swing_to` and `pf_runtime_view` (interface 3). |
| `set_hour(Some(h))`, `set_hour(None)` | Set the hour of the day (0..24) on every scene drawn (transitions and forks too), turning `sky.day_night` on with the scene's own settings; None gives each scene its own hour back. |
| `set_day_length(seconds)`, `hour()` | Let a day pass every `seconds` of `step` time (0 holds the hour); the hour now. |
| `day()` | Where the day stands: `hour`, `altitude` (sun height, -1..1), `is_day`, `daylight` (0 night .. 1 day), `phase` ("night", "dawn", "day", "dusk"); None without an open sky. Also in `state().day`. |
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
`pf_runtime_choose`, `pf_runtime_state` (JSON), `pf_runtime_project_now`, `pf_runtime_set_hour`,
`pf_runtime_set_day_length`, `pf_runtime_day`. Return codes and
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
| Looking | `presets [--mine]`, `dump`, `render`, `sheet`, `gallery`, `styles`, `kit`, `sprites` (every painted prop and light on one sheet), `schema [--markdown]` |
| Checking | `seam`, `bench`, `animcheck`, `animsheet`, `gifdiff` |
| Exporting | `export` (loops, encounters, transitions, forks) |
| Transitions, forks, journeys | `transition`, `journey` |
| Projects and the skill | `project`, `assets`, `pack`, `preset-save`, `mask`, `skill` |
| Sharing | `share` (a scene as a share code; `--qr out.png` writes its QR code), `unshare --code PF1... [-o scene.json]` |

**Share codes.** `pf share` turns a scene into one line of text starting `PF1.`, short enough
for a message or a QR code: each part of the scene is written as what it changes from the
nearest preset's, compressed (a preset with a few edits is a few dozen characters; a remix, a few
hundred). `pf unshare` (or the phone apps' Open a code) gives the scene back exactly. Codes name
the presets they build on, so they open in later versions of PathForge too.

`pf` with no arguments lists the commands; the top of `src/bin/pf.rs` gives every command with all
its options.

## 16. AI agents: MCP server and skill

`pf_mcp` is an MCP server on stdio. Its tools return images, so an agent can design a scene,
look at it, and export it:

| Tool | Purpose |
|---|---|
| `pf_presets`, `pf_schema`, `pf_palettes`, `pf_styles` | What there is to start from (`pf_presets` lists your own presets too). |
| `pf_save_preset` | Keep a scene as a preset of your own. |
| `pf_new_scene`, `pf_get_scene`, `pf_edit_scene` | Create, read and edit scene files (merge patches and pointer edits, validated, atomic, with a revision check). |
| `pf_render`, `pf_contact_sheet`, `pf_compare`, `pf_analyze`, `pf_check_loop`, `pf_camera` | Look at and measure a scene. |
| `pf_export`, `pf_splice` | Export (§6); join exported clips into one animation (§8). |
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
