# PathForge

Endless, seamlessly looping first-person path backgrounds for 2D games: the road a hero walks in a
portrait runner like *Path of Kings*, a corridor between fights, a road a party travels along.

You describe a world in metres (path, walls or verges, stairs, bridges, forks and side passages,
lights, props, arches and gates, weather, sky), and PathForge renders a camera walking through it
as a loop with no seam. It exports the loop
as animated WebP, GIF, APNG, PNG frames or sprite sheets, plus depth maps, near/mid/far layers,
encounter clips (stop, idle, go) and transitions into the next biome, all with metadata that tells
the game where things are on screen.

## Binaries

| | |
|---|---|
| `path_forge [scene.json]` | The studio: a live preview, an inspector generated from the scene schema, a timeline, a preset gallery and an export window. It reloads the file when it changes on disk. |
| `pf` | Headless CLI: `presets`, `dump`, `render`, `sheet`, `gallery`, `styles`, `kit`, `schema`, `seam`, `bench`, `animcheck`, `animsheet`, `gifdiff`, `export`, `transition`, `journey`, `project`, `assets`, `pack`, `skill`. |
| `pf_mcp` | MCP server on stdio with 20 `pf_*` tools that return images, so an AI agent can design, look at and export scenes. |

```bash
cargo build --release
```

## Documentation

- [docs/MANUAL.md](docs/MANUAL.md): the full manual. Scenes, the studio, looks, kits, every export
  format and its metadata, encounter clips, how to make transitions, forks and journeys, the live
  runtime (Rust and C), Quartz games, projects, the CLI and the MCP server.
- [docs/SCENE_REFERENCE.md](docs/SCENE_REFERENCE.md): every scene, transition, fork and journey
  field with its type, default and meaning, generated from the code (`pf schema --markdown`; a test
  keeps it current).

## Export formats

| Format | What you get |
|---|---|
| `webp` | Animated WebP, full colour, lossy or lossless, far smaller than GIF |
| `gif` | 256-colour GIF (the scene's palette, or the best colours across the loop) |
| `apng` | Animated PNG, lossless |
| `png` | A folder of frames |
| `sheet` | Sprite sheet pages plus a TexturePacker/Aseprite-style atlas |
| `depth` | 16-bit depth per frame, for hiding game objects behind scenery |
| `layers` | Near/mid/far transparent layers per frame, for parallax |

Every export also writes `<name>.json`: frame count, fps, metres per frame, lightning times, the
camera projection that maps world metres to screen pixels, and the scene. Animated WebP plays
slowly in Safari, Preview and Quick Look on macOS, because Apple's decoder replays every earlier
frame. Chrome, Firefox, Discord and game engines play it at its real speed. See the manual,
§7.

## Weather and the air

Rain, snow, sleet and hail that wet the ground, fill puddles that mirror the scene and settle as
snow on the ground and on props; dripping ceilings; wind that slants the rain and sways trees and
banners; sandstorms; low drifting mist; sun shafts through trees and haloes round lamps; cloud
shadows; heat shimmer; aurora and rainbows; raindrops or frost on the lens. Any scene can use any of
them, and every one repeats seamlessly with the loop (manual §2, Weather and the air).

## Props as data: kits

Beyond the built-in prop kinds, a prop can be described by data: an image, a pool of images or
folders of PNGs, or a shape drawn from parts in metres, with its height, whether it stands or hangs,
how it glows (a lantern's glass, a flame's lightest pixels) and the light it gives off. Definitions
live in a scene's `prop_defs` or in a **kit**: a folder holding `kit.json` beside its images, used
from any scene below it as `"def": "kits/<name>#<prop>"`. The built-in kit `@wayside` (lantern
posts, hanging lanterns, glowing mushrooms, signposts, fences, barrels, crates) is drawn from shapes
and needs no files.

```bash
pf kit --kit @wayside -o wayside.png
```

Sprites made with an image generator go straight into a kit: `pf_import_sprite` takes a URL (for
example a PixelLab download link), a file or base64 PNG, saves it in the kit and adds it to a prop.
Transparent margins are cut away when drawn, so every sprite stands on the ground.

## Projects

PathForge keeps work in `~/Documents/PathForge` unless `PATH_FORGE_HOME` says otherwise (never the
folder it was started from). A project folder holds `scenes/`, `assets/`, `kits/` and `exports/`,
and scenes name their files relative to themselves:

```bash
pf project --new ~/Games/MyRunner/backgrounds --preset "Haunted Forest"
```

`pf assets --scene F` lists every file a scene uses and whether it is there; `pf pack --scene F`
copies anything from elsewhere into the scene's folder and makes the paths relative (`--out DIR`
packs into a new folder). The studio does the same from File > Gather files / Pack into folder,
and its image and grade pickers copy files from elsewhere into the scene's `assets/`.

## Embedding in a game (live runtime)

Instead of playing exported frames, a game can render the path itself every frame, at any speed
and resolution. Distance walked and time are separate inputs, so the hero can stop for a fight
while flames and weather keep moving, and the loop never shows a seam. The camera answers where
things are: `project` (a point in metres to screen pixels) and `ground_distance` (a screen row to
metres ahead), at the size you render.

Rust (no GUI or GPU dependencies):

```toml
path_forge = { path = "../path_forge", default-features = false }
```

```rust
let mut rt = path_forge::runtime::Runtime::from_file("backgrounds/scenes/crypt.json")?;
let mut walk = path_forge::runtime::Walk::default();
walk.advance(dt, speed, &rt);
let rgba = rt.render(walk.distance, walk.time, 270, 480);
```

Other engines (C, C++, C# via P/Invoke, GDExtension...) link the C library and use
`runtime/include/path_forge.h`; `runtime/examples/walk.c` is a complete example:

```bash
cargo build --release -p path_forge_runtime
```

Rendering is on the CPU across threads. Measured on an M-series Mac under load: Stone Dungeon
1.3 ms a frame at 135x240 and 11.8 ms at 480x854; a glossy swamp with many sprites 3.8 ms at
135x240 and 13 ms at 270x480. Render small (or with a pixel style) and scale up.

## Transitions, forks and journeys

The walk from one scene into another is rendered in one pass: both worlds share the frame and meet
at a boundary on the road, so nothing cuts or cross-fades. PathForge plans that boundary from the
two scenes: a soft band between open places, a lit doorway out of a dungeon (its light falls on the
passage floor and the eye adapts on the way out), a doorway in a stone face or a cave mouth in a
rock face on the way in, a gate or a portal when asked. It shows the boundary where the fog first
gives it up, blends the camera either side, and warns about what will not look right.

A fork parts the path into two branches; the game chooses before the junction and the camera turns
onto the chosen one. A journey (`*.journey.json`) maps a game's places: stops, and whether each
goes on through a transition, forks, or ends.

```bash
pf transition --preset "Stone Dungeon" --to-preset "Forest Path" -o walk.png
```

```bash
pf transition --preset "Forest Path" --left-preset "Mountain Pass" --right-preset "Desert Canyon" --take right -o fork.png
```

```bash
pf journey --file game.journey.json --out exports/
```

Exports end exactly on the next loop's frame 0 (`--entries N` adds clips that start at N evenly
spaced loop frames); a fork exports an approach plus one clip per branch. The studio designs both
from Edit > Transition / fork…, and agents use `pf_transition` and `pf_journey`. The live runtime
plays the same journey: `Runtime::from_journey`, `step`, `go`, `choose`, `state` (C:
`pf_runtime_open_journey`, `pf_runtime_step`, `pf_runtime_go`, `pf_runtime_choose`,
`pf_runtime_state`, `pf_runtime_transition_to`, `pf_runtime_fork`).

The manual's §9-11 walk through making each one, in the studio and from a shell, and what each
export contains. Quartz games play journeys with the `quartz_path_forge` plugin (manual §13).

## Using PathForge from an AI agent

Register the MCP server with your agent. `PATH_FORGE_HOME` is where relative scene and export
paths resolve:

```json
{
  "mcpServers": {
    "path_forge": {
      "command": "/path/to/path_forge/target/release/pf_mcp",
      "env": { "PATH_FORGE_HOME": "/path/to/your/game/backgrounds" }
    }
  }
}
```

PathForge ships an agent skill, `skills/pathforge/SKILL.md`, that teaches an agent the design
loop: brief, start from a preset, small edits, look before claiming, style, loop check, export
choices, and the pitfalls. It follows the open Agent Skills format, so it works in Claude Code,
Codex, GitHub Copilot, Cursor and other agents that read `SKILL.md`.

- **Over MCP, nothing to install:** `pf_mcp` serves it as the prompt `pathforge` (with an
  optional `brief` argument) and the resource `pathforge://skill/SKILL.md`, and its server
  instructions point agents to it.
- **As an installed skill:** install it into a project (or into your home folder for every
  project). The skill is compiled into `pf`, so it always matches the tools of the build you run.

```bash
pf skill --install .
```

```bash
pf skill --install ~
```

  By default this writes `.claude/skills/pathforge/` (Claude Code) and `.agents/skills/pathforge/`
  (Codex; Copilot, Cursor and others read it too). `--agent all` adds `.github/skills/` and
  `.cursor/skills/`; `--agent claude,cursor` picks specific folders. Rerunning upgrades copies
  nobody has edited and keeps edited ones (`--force yes` replaces them). `pf skill` alone prints the
  skill.

Agents without MCP can follow the same skill through the `pf` CLI, which it documents.
