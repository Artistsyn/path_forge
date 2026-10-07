---
name: pathforge
description: Design, critique and export endless looping first-person path backgrounds for 2D games with PathForge (Path of Kings style portrait runners). Use when the user wants a walking-path / corridor / road background, wants to restyle, judge or export a PathForge scene, or needs depth layers, sprite sheets, encounter or biome-transition clips, or camera data for placing enemies on the path. Not for editing PathForge's own Rust code, side-scrolling parallax or top-down maps.
metadata:
  version: "1.0"
  tools: "pf_mcp MCP server (pf_* tools) or the pf CLI"
---

# PathForge design loop: brief -> scene -> look -> export

PathForge renders a camera walking forward along a path, as a loop that repeats without a seam. Use it
for the background of a portrait runner, a road a party walks along, or a corridor between fights.

## How you reach it

- **MCP (preferred):** the `pf_mcp` server exposes `pf_*` tools that return images, so you can see
  what you made. Relative paths resolve under `PATH_FORGE_HOME` (default: Documents/PathForge;
  shown in every result).
- **CLI fallback:** `pf` does the same work from a shell, writing PNGs you can open:
  `pf presets`, `pf dump --preset NAME > scene.json`, `pf render --scene F -t 0.25 -o a.png`,
  `pf sheet --scene F --frames 8 -o sheet.png`, `pf styles --scene F -o styles.png`,
  `pf seam --scene F`, `pf export --scene F --formats webp,sheet --out DIR`,
  `pf animcheck --file DIR/x.webp --scene F`, `pf kit --kit @wayside -o kit.png`. Edit the JSON
  (scenes and kit.json files) yourself between runs.
- **Studio:** `path_forge scene.json` opens the desktop editor. It reloads the file when it
  changes, so the user can watch every edit you make, and can keep editing alongside you.

## The world model (read this once)

- Everything is in **metres**: `x` sideways from the path centre (right positive), `y` height above
  the ground, `d` distance ahead of the camera.
- One loop covers `motion.loop_length` metres at `motion.speed` m/s. Every repeating thing
  (torches, props, set pieces, fog banks, texture tiles) is snapped to divide the loop, so loops
  are seamless by construction. If a loop check fails, report it: do not hide it.
- Time runs separately from distance: flames, weather and particles keep moving even when the
  camera stands still (encounter clips rely on this).
- Scenes are JSON. Enum values are **PascalCase**: `"Torch"`, `"StoneBlock"`, `"DeadTree"`,
  `{"Named": "PICO-8"}`, `"Bayer4"`. Unknown fields are rejected with their path; check
  `pf_schema {"section": "Fixture"}` (or `"SetPiece"`, `"Particles"`, `"Weather"`...) for every field
  with units and ranges.

## 1. Brief (write it down before touching tools)

Biome and mood; walled corridor or open path; time of day and weather; light rhythm (a torch every
N m?); what frames the path (trees, pillars, arches, gates); the game's 2D style; and the output
target: canvas size, fps, format, and whether characters must walk *between* scenery (then you need
depth layers or a depth map).

## 2. Start from the nearest preset

- `pf_compare {"presets": ["*"], "scale": 0.15, "columns": 8}` shows every preset at once;
  `pf_presets` lists them with one-line summaries.
- `pf_new_scene {"path": "scenes/<name>.json", "preset": "<closest>"}` copies it and returns a preview.
- Stuck for ideas, or the user wants options? `pf_remix {"path": ..., "keep": ["camera", "path"]}`
  shows variations where the kept parts stay and the rest come from other presets (each with its
  seed); `"save_as"` with the chosen `seed` writes one. Still look and refine after: a remix is a
  starting point, not a finished scene.

## 3. Shape the world: small edits, look after each

- `pf_edit_scene {"path": ..., "merge": {...}}` for object fields; `ops` for list items:
  `{"op": "set", "pointer": "/fixtures/0/spacing", "value": 6}`,
  `{"op": "append", "pointer": "/props", "value": {"kind": "DeadTree", "lateral": 1.5}}`.
- **Pitfall:** a merge that names a list (`"props": [...]`) *replaces the whole list* and resets
  every field you left out. Use ops to change one item; the tool warns when a merge replaced a list.
- Pass `revision` (from `pf_get_scene`) when the user may also be editing in the studio, and
  `dry_run: true` to see what an edit would change without writing it.
- Props for biomes: Tree, Pine, Bush, Rock, Boulder, Cactus, DeadTree, Mushroom, Pillar,
  Gravestone, Crystal, Stalagmite, Reeds, Willow, Palm, Obelisk, Icicle (hangs from the ceiling or
  wall tops), IceSpike. A layer without a `tint` takes its kind's own colour. Particles: Dust,
  Embers, Fireflies, Rain, Snow, Leaves, Ash, Spores, Sand (blown low and sideways) and Petals;
  set a particle layer's `color` to its kind's own when you change its kind (the studio does).
- **Props as data (kits):** when no built-in kind fits, describe the prop instead of settling.
  A prop layer with `"def": "<kit folder>#<name>"` (or a name in the scene's `prop_defs`) draws
  that definition: an image, a `pool` of images or folders of PNGs (one picked per prop), or a
  `shape` drawn from parts in metres (`Rect`, `Ellipse`, `Poly`, `Line`; `x` from the centre, `y` up
  from the ground; a part with `glow` lights itself, like lantern glass). A definition also sets
  `height` (metres), `anchor` ("Ground" or "Ceiling") and a `light` it gives off (lamps, glowing
  mushrooms). The layer still decides side, spacing, density and scale.
  - Look first: `pf_kit {"kit": "@wayside"}` shows the built-in kit (lantern_post, hanging_lantern,
    glowcaps, signpost, fence, barrel, crate), each prop standing beside a path.
  - Make your own: `pf_kit {"kit": "kits/<biome>", "props": {"<name>": {...}}}` writes
    kits/<biome>/kit.json (`"from": "@wayside"` starts from the built-in props). Scenes anywhere
    below the folder holding `kits/` find it as `"kits/<biome>#<name>"`.
  - A definition that cannot be found draws nothing, and pf_analyze / pf_edit_scene say why.
- **Sprites from an image generator:** generate each prop as a side view on a transparent
  background (PixelLab: `create_map_object` with `view: "side"`, then `get_map_object` for its
  URL), then `pf_import_sprite {"kit": "kits/<biome>", "file": "<name>/<name>_1.png", "url": ...,
  "def": "<name>", "height": <metres>}`. Import several images into one prop for variety.
  Transparent margins are cut away when drawn, so a sprite stands on its lowest visible pixel. Size
  props in real metres (a lamp post about 2.5, reeds 1.5, a stump 0.8), and look at them with pf_kit
  in the scene's own light before placing them.
- Order of work: camera (horizon about 0.25-0.35, eye height 1.6) -> path width and material ->
  walls or verges -> lighting (low ambient indoors; fixtures carry the mood) -> props ->
  set pieces -> particles and weather -> post.
- **Set pieces** span the path and pass overhead: `Archway`, `RuinedArch`, `Gate`, `Banners`,
  `Portal` (`set_pieces[]`: `spacing`, `offset`, `width` 0 = fit the path, `height`, `tint`,
  `accent`). Offset them from the lights by half a spacing, or a torch sits inside every arch.
- **Stairs:** `path.stairs` (`spacing` between flights, `steps`, `rise` 0.15-0.2 m, `run`
  0.25-0.35 m, `descending`). The walk climbs or descends for ever and the loop still closes;
  props, lights and set pieces stand on the steps. Going down, steps hide behind their own edges
  from more than a few metres back: that is how real stairs look, so show descents close up.
  `pf_camera` accounts for the steps (`stairs_note` in the camera data gives the formulas).
- **Bridges:** `path.bridge` (`spacing`, `length`, `depth` of the drop, `bottom` "Ground" /
  "Water" / "Void", `deck` material (planks by default), `railing` "Posts" / "Parapet" / "None").
  The ground beside the path falls away and walls carry on down into the gap; trees and rocks
  stop at the edge and standing lamps move onto the railing. In a walled corridor the walls must
  stand back from the path (`walls.gap` 1.5 m or more) or there is no gap to cross; pf_analyze warns.
- **Forks:** `path.fork` (`spacing`, `side` "Left" / "Right" / "Alternate", `angle`,
  `half_width`). On open ground a branch path splits off and runs away; between walls it becomes
  a side passage (a doorway under a ceiling, an alley without one). The walk stays on the main
  path. Alternating sides needs an even number of forks per loop; pf_analyze says when it is odd.
- **Wet, icy and watery floors:** every material has `gloss` (0 matte, 0.3-0.5 wet stone, about
  0.6 ice, 1 still water) and `ripples` (0 glassy to 1 choppy). Reflections are traced against the
  rendered world, including what stands above and beside the frame (a canopy, a ceiling, lamps
  overhead), so lamps, trees, the moon and clouds show in them, and lamps lay glints on rough wet
  stone. Glossy scenes render that extra margin, so they cost about a quarter more per frame. Pattern "Water" on the verge makes a lake or swamp; "Ice" a frozen floor. A wet road
  in the rain (`gloss` 0.45, `ripples` 0.08) is the cheapest big upgrade to a night street.
- **Weather and the air** (every part works in any scene and repeats with the loop; all are
  off until `enabled`):
  - `weather.precipitation`: `kind` Rain / Snow / Sleet / Hail, `intensity` 0.1 drizzle to 1
    downpour or blizzard. It falls only where the sky is open (not under a ceiling) and changes the
    ground: rain and sleet make it `wetness` (darker, glossy, lamps streak in it), `puddles` (still
    water with rings where drops land), `splashes`; snow and sleet lie as `cover` on the ground,
    verges, wall tops and the tops of props, with a trodden `track` down the path; `haze` is
    curtains of rain far off or a blizzard's whiteout. Wet ground costs what any glossy floor
    costs (reflections, about 2x a dry frame); falling drops and snow cost almost nothing. For a
    night street: Rain 0.6, wetness 0.9, puddles 0.35, plus lightning.
  - `weather.wind`: `speed` m/s (negative blows left), `gusts`, `sway`. Rain and snow slant and
    drift, particles are carried, trees, reeds, grass and banners bend (rocks and pillars do not).
  - `weather.drips`: water falling from the ceiling (or wall tops) in caves, sewers, crypts.
  - `weather.sandstorm`: streaming sand, a sand-coloured haze that hides the distance, the sun a
    dim disc. Pair with wind.
  - `weather.mist`: low mist (`height` m, `density` per metre, `patchiness`, `wisps` rising).
    Keep `density` around 0.1-0.2: it thickens with distance, so near ground stays clear.
  - `weather.light_shafts`: `sun` rays streaming past trees, walls and arches near a sun in the
    sky (stronger with fog or mist), and `lamps` haloes in the air round every light.
  - `sky.clouds.shadows`: cloud shadows drifting over the ground (needs a light-giving sun).
  - `weather.heat_shimmer`: the distance wavers above the horizon (deserts, lava).
  - `sky.aurora` (curtains in a night sky: `low`/`high` colours, `height`, `speed`) and
    `sky.rainbow` (`x` across the sky, `size` 1 spans the frame, `double`).
  - `weather.lens`: `kind` Drops (rain landing on and running down the lens) or Frost (creeping
    in from the edges).
  - `weather.lightning` (`strikes` per loop, `intensity`, `color`, `bolts`) flashes the scene and
    draws bolts; export metadata lists each strike's frame for thunder. `weather.fog_banks` are
    thick stretches of fog the camera walks through.

## 4. Look before you claim (every few edits)

- `pf_contact_sheet {"path": ..., "frames": 6}` shows motion and repetition. A single frame hides
  props that bury the path near the camera and lights that bunch up.
- `pf_analyze {"path": ...}`, then act on its warnings: path too small or too dark, path and verge
  the same brightness, props or fixtures never visible, props sharing spacing and offset with the
  lights, blown-out surfaces.
- Short events (a lightning strike) can fall between evenly spaced sheet frames: render the frame
  the summary names, or `pf_render` with `t` at that fraction of the loop.
- A sheet whose step equals the texture tile shows the same ground pattern in every frame. That is
  a strobe effect, not a frozen texture.
- The bar to meet (Path of Kings):
  - the walkable path is the clearest thing on screen;
  - light comes in a readable rhythm, not evenly everywhere;
  - scenery frames the path and never covers its centre near the camera;
  - silhouettes read at phone size (look at scale 0.3);
  - the far end fades into fog or darkness, so the horizon stays calm across the loop.

## 5. Style

- `pf_styles {"path": ...}` renders the scene in every look side by side. Apply one with
  `pf_edit_scene {"path": ..., "style_preset": "Dark Fantasy Pixel"}` (the Path of Kings feel);
  others: "16-bit Console", "Game Boy", "PICO-8", "Painted Storybook", "Toon", "Retro CRT", "Noir",
  "Clean".
- A look replaces `style`, `post` and `light.bands` only. Re-apply exposure tweaks after it.
- Fine-tune with `style.pixel_size`, `style.palette` (`{"Auto": 32}` picks colours once per loop;
  `{"Named": ...}` from `pf_palettes`), `dither`, `outline` (ink stops at 14 m by design), `paint`
  (2-8, painterly), `grade` (a built-in name or a `.cube` file), `paper`, `scanlines`.

## 6. Verify the loop

`pf_check_loop {"path": ...}` (CLI: `pf seam --scene F`) must report `seamless: true` (exact
difference about 0) and a wrap no bigger than about twice an ordinary frame step. If it fails,
something has a period that does not divide the loop: say so and find it.

## 7. Export for the game

`pf_export {"path": ..., "formats": [...]}`. Choose by need:

| Need | Format |
|---|---|
| Full-colour animation | `"webp"` (auto quality 90: a third of the GIF size, less error) |
| Pixel art | `"webp"` (auto lossless: exact, about a third of the GIF); `"gif"` only when a platform demands it |
| Engine sprites | `"sheet"` (pages up to 4096 px, one TexturePacker atlas per page) |
| Characters between scenery | `"layers"` (near/mid/far RGBA bands, `layer_bands` in metres) and/or `"depth"` (16-bit: metres ahead = 0.25 * 65535 / value, 0 = sky) |
| Frames for a custom pipeline | `"png"` |

Clips beyond the plain loop (`clip`):

- `"encounter"`: stop / idle / go clips that join the loop exactly, for halting to fight
  (`slow_m` braking distance, `start_frame` where the stop begins).
- `"transition"` and `"fork"`: see section 7b. Design those with pf_transition, not pf_export.

Always hand the game the metadata `<name>.json`. It gives `metres_per_frame` (to follow a
character, show frame `floor(distance / metres_per_frame) mod frames`; on a timer, advance at
`fps`), the camera and its projection formulas, and lightning strike frames.
`pf_camera {"path": ..., "points": [{"x": 0, "y": 0, "d": 8}], "rows": [600]}` answers "where on
screen is an enemy 8 m ahead" and "how far ahead is this screen row".

A game can also skip exported frames and render the scene live, every frame, at any speed and size
(the hero stops for a fight and the flames keep moving): `path_forge::runtime::Runtime` in Rust, or
the C library `path_forge_runtime` (`runtime/include/path_forge.h`) from other engines. Hand over
the packed scene folder; the runtime's `project` and `ground_distance` answer the same questions as
pf_camera. Suggest it when the game needs variable speed or several resolutions.

## 7b. Walk on: transitions, forks and journeys

Games rarely stay in one place. PathForge renders the walk from one scene into the next in one
pass (both worlds in the same frame, meeting at a boundary on the road) and plans what stands at
that boundary from the two scenes themselves. You design it with `pf_transition`; you do not
choose the details unless the plan's warnings say to.

- **Transition:** `pf_transition {"path": A, "to_path": B}` (or `to_preset`). The plan picks the
  threshold from the scenes: roofed into open is a `Doorway` (the exit glows and its light falls on
  the last metres of the passage, the eye adapts as it does walking out of a cellar); open into
  roofed is a `Doorway` cut in a face of the second scene's stone, or a `CaveMouth` in a rock face
  when it is rocky; open into open is a soft `Open` band whose edge is noise, not a line. It sets
  how far ahead the boundary first shows from both scenes' fog, and blends the camera either side.
  Read the notes (why) and the warnings (what will not look right: a style change, fps that differ,
  a second scene with no fog to hide its first appearance). Override with `transition`
  (`threshold`, `approach_m`, `blend_m`, `light_spill`, `adaptation`, `camera_blend_m`,
  `facade_height`, `marker`, `style`; pf_schema section Transition) only to answer a warning or the
  brief.
- **Fork:** give `left_path`/`left_preset` and `right_path`/`right_preset` instead. The path parts
  into a Y; both branches are in view on the way up, the game chooses before the junction, and the
  camera turns onto the chosen branch while the other swings out of view. `take` picks the branch
  the preview walks. Adjust with `fork` (`angle`, `approach_m`, `blend_m`, `wedge_m`, `steer_m`,
  `default_branch`). Forks read best from open ground into worlds under a similar sky; the plan
  warns when the first scene is roofed or a branch is lit very differently.
- **Look:** every call returns a contact sheet of the walk. Check the boundary's first frame (does
  it come out of fog, not pop?), the frame at the threshold, and the last frame (it must be the
  second scene's loop frame 0; the export metadata says so).
- **Export:** add `"export": {"formats": ["webp"], "out_dir": ..., "entries": 4}`. A transition
  writes `<name>.webp` + `.json`; `entries` > 1 writes clips that start at evenly spaced loop frames,
  so the game starts the walk from the loop frame it is on. A fork writes `_approach`, `_left` and
  `_right` clips: play the approach, then the chosen branch, then that branch's loop.
- **Journey:** a whole game's places in one file: `pf_journey {"op": "write", "path":
  "game.journey.json", "journey": {"name": ..., "start": "crypt", "stops": {"crypt": {"scene":
  "scenes/crypt.json", "next": {"Go": {"to": "forest", "transition": {}}}}, "forest": {"scene":
  "preset:Forest Path", "next": {"Fork": {"left": "pass", "right": "canyon", "fork": {}}}}, ...}}}`.
  `op read` lists its problems (missing scenes, dead ends), `op preview` shows every stop and one
  frame of each walk between them, `op export` writes every clip plus `<name>.journey.json` mapping
  stops to files.
- **Live:** the runtime walks the same journey itself: `Runtime::from_journey`, `step(dt, speed)`,
  `go()` at a Go stop, `choose(Branch)` before a fork's junction (`state().choose_within` says how
  many metres are left to choose), `transition_to`/`fork` for one-off walks. C:
  `pf_runtime_open_journey`, `pf_runtime_step`, `pf_runtime_go`, `pf_runtime_choose`,
  `pf_runtime_state` (JSON). CLI: `pf transition --scene A --to-preset NAME -o sheet.png` (or
  `--left-*`/`--right-*`, `--take`), `pf journey --file F --out DIR`.
- **Quartz games:** the `quartz_path_forge` crate's `PathForgePlugin` plays a scene or journey on a
  game object (live, or the png export), driven by `RunPlugin "path_forge"` (`next`,
  `choose:left|right`, `stop`, `walk`, `speed:`); in quartz_forge it is a background type
  (`qf_path_forge_background_contract`).
- **Studio:** Edit > Transition / fork… shows the same plan with a live preview to scrub or play,
  and exports the clips.

## Keep a project together

Keep each game's backgrounds in one folder: `scenes/`, `assets/` (images, .cube grades), `kits/`,
`exports/` (`pf project --new DIR` makes one). Scenes name files relative to themselves, so the
folder can move or be committed with the game. Before handing a scene over, `pf_pack {"path": ...}`
copies anything it uses from elsewhere into its folder and makes the paths relative (`out_dir`
packs into a new folder instead); pf_analyze lists any file a scene names that is missing.

## 8. Report

Lead with the outcome. Attach the contact sheet, list the exported files with their sizes, quote
the loop check, and name any analyze warnings you chose to keep and why.
