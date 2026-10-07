# GPU port of the v3 renderer — plan

2026-10-07. Status: phases 0-4 and most of 6 built (see "Where it stands" at the end).

## Goal

Render every scene, with any stack of effects, at full quality on the GPU, with frame times that
stop mattering. Rough targets at 480×854:
- the heaviest effect stack under 4 ms a frame (it is 38 ms on the CPU today);
- the studio preview shown straight from a GPU texture, with no copy back to the CPU;
- headroom to spend on quality the CPU could never afford, such as supersampled anti-aliasing.

The CPU renderer stays. It is the reference that the GPU is tested against, and the fallback where
there is no GPU adapter (CI, a game built with `default-features = false`).

## No fork of wgpu

Everything this renderer needs is core WebGPU: compute pipelines, storage buffers, sampled 2D and
3D textures, texture arrays, explicit mip loads, and (later) a render pipeline with several colour
targets. Stock wgpu has all of it on Metal, Vulkan and DX12.

The VR engine forked wgpu for things this renderer will never use:
- fragment density maps (fixed foveated rendering);
- f16 storage capabilities on Adreno;
- robust-access reporting on Quest.

There is one limit worth knowing about, and requesting it is not a fork. The default allows 8
storage buffers per shader stage, and the shading pass reads more inputs than that. Either pack
inputs into fewer buffers, or request the adapter's own limit (Metal reports 31) when creating the
device. Packing is the more portable choice.

Storage *textures* that are both read and written are avoided altogether: whatever a pass both
reads and writes lives in a storage buffer. That sidesteps per-format storage support, which
differs between backends.

## Versions

The pieces that would share a device use different wgpu versions today:

| Crate | wgpu |
|---|---|
| path_forge (v2 GPU code) | 22.1.0 |
| eframe / egui-wgpu 0.29.1 (the studio) | 22.1.0 |
| quartz, wgpu_canvas (synful_quartz) | 25.0.2 |

The studio already runs on eframe's wgpu backend, and its `RenderState` hands over `Arc<Device>`
and `Arc<Queue>`. So a renderer on wgpu 22 could share the studio's device today. It could not
share quartz's, and a game would end up building two copies of wgpu.

**Decision (Phase 0):** move path_forge to wgpu 25, matching quartz. Upgrade eframe/egui to the
release whose egui-wgpu depends on wgpu 25. The cheap check is to bump the version and run
`cargo tree -i wgpu`: exactly one version must be listed. If no egui release lines up, move
quartz and path_forge together to a newer common version instead.

The old v2 GPU code (`gpu_scene.rs`, `gpu_effects.rs`) uses several wgpu-22-only APIs:
`ImageCopy*`, `Maintain::Wait`, the adapter as an `Option`, `entry_point: &str`. Only the v2
`--v2` / `--engine v2` path calls it. Port it in the same step, or retire v2 GPU.

## Architecture

The structure follows the CPU renderer: deferred, per-pixel shading, with scatter passes binned
per tile, as the CPU's 8-row bands already are.

```
CPU per frame:  plan, views, lights, billboards, particles (small: kilobytes)
GPU per frame:
  geometry ── raster (hardware, several colour targets) ──> G-buffer (depth, id, realm, x, y, d)
  shadows (prop casters binned per tile) ──> sun_mask, ao_mask
  shade (+ tile light lists, textures) ──> hdr, refl
  sky, sky bodies, lightning, veil
  cards (billboards), tufts, flames, particles, precipitation (sorted lists, binned per tile)
  reflect (column march + triangle blur)
  mist, light shafts, heat shimmer, lens
  post: bloom, tone map, grade, kuwahara, outline, lens warp, palette and dither, upscale, surface
  ──> studio: texture shown by egui-wgpu (no readback)
  ──> export / CLI / MCP: one readback per frame
```

### Code layout

- `src/world/gpu/mod.rs`: `GpuRenderer` mirrors `WorldRenderer::render` and `render_layout`, and
  returns the same `Image` (pixels, depth, pick, stats) when it reads back.
- `src/world/gpu/*.wgsl`, loaded with `include_str!`, one file per pass, plus `common.wgsl`: views,
  hashes, noise, fog, colour.
- No shader lives in an inline string: the old 1,565-line `SCENE_SHADER` string showed where that
  ends.
- One `GpuContext` per device: borrowed from eframe in the studio, created headless for pf, MCP and
  exports. It holds caches for pipelines, textures (keyed like `TextureCache`), sprite atlas pages
  and per-size frame resources. The v2 code reallocated per call; this one must not.
- **Every pipeline is created in a test.** WGSL is validated only when a pipeline is created, so a
  shader error otherwise first shows up at run time (see the `wgsl-runtime-validation` memory).

### Data

| Thing | On the GPU |
|---|---|
| G-buffer | Raster targets (Rgba32Float x, y, d, depth plus R32Uint id/realm), copied into storage buffers for the compute passes. |
| Textures | One 2D array per frame (path, verge, wall, ceiling, deck, bottom, facade, rail), 192² with all 7 mip levels uploaded. Sampled with `textureLoad` at the CPU's own lod rule, `floor(log2(footprint · side))`, and the same wrap, so results match. |
| Sprites | Atlas pages with each sprite's mips packed beside it, plus the glow mask. A card carries its atlas rects. Bilinear sampling with alpha weighting is ported as is. |
| Lights | Storage buffer of point lights and sky lights. Tile lists built on the GPU, the same test as `tile_lights_at`. |
| Scene and world | One uniform block per world: view, fog, fog banks, `Wx`, materials, bridge spans, forks, bounds, opening, portal. Arrays of 3 cover crossings and forks. |
| Grades, palettes | 3D textures (the `.cube` grade, the 32³ or 64³ palette LUT); the Bayer matrix as a constant. |

## Parity: how "the same picture" is checked

The CPU renderer is deterministic: the golden check used for the speed work (render every preset,
compare the PNG bytes) proved each optimisation exact. The GPU adds a second harness:
- `pf parity --engine gpu` renders every preset at two loop times, plus the weather scenes, on both
  engines. It reports the mean and 99.9th-percentile difference and the maximum. A per-pass mode
  compares intermediate buffers (G-buffer, hdr after shading, after reflections, after the air), so
  a fault is placed in a pass, not a frame.
- **Gate:** mean ≤ 0.5/255 and 99.9% of pixels within 4/255. Anything larger has a named cause
  before it is accepted.
- **Loop closure stays exact on the GPU.** Frame 0 and frame N have the same inputs, so `pf seam`
  must report byte-identical frames on one device.
- **Integer hashes are emulated exactly.** `hash()` multiplies in u64 and WGSL has no 64-bit
  integers. So `common.wgsl` builds the 64-bit multiply from 32-bit halves, and every noise
  pattern (path edges, puddles, snow, mist banks, rain rings, frost) lands on the same cells as on
  the CPU. Swapping in a cheaper GPU hash would move every pattern and break the comparison.
- **Floats will not match to the bit:** `exp`, `pow`, `sin` and FMA differ between CPU and GPU.
  The tolerance above absorbs that, and the per-pass mode shows where it grows.

## Passes that need a new design, not a transcription

| Pass | CPU form | GPU form |
|---|---|---|
| Reflection blur | `ColumnBlur`: f64 running sums of running sums (WGSL has no f64) | Column sums kept relative to 64-row blocks, so f32 has the precision. Tested against the CPU triangle within 1e-3. |
| Reflection march | One pixel walks up its column through 8/64-row minimum blocks | Same march, one thread per pixel. Column-major depth and minimum tables built in a prior pass (the CPU now does the same). |
| Kuwahara | f64 summed-area tables | Direct window sums in workgroup memory (the radius is small), or block-relative f32 tables as above. |
| `Forks::on_branch` | Walks every junction within reach: about 860 iterations a pixel with a shallow fork angle | Closed form for the nearest junction. This is also a CPU speed fix, so make it first on the CPU, where the golden check proves it. |
| Cards, particles, precipitation, drips, sand, wisps, flames, tufts, prop shadows | Drawn in order into 8-row bands | Lists sorted on the CPU (hundreds of items), binned per 16×16 tile on the GPU. Each tile's workgroup draws its items in order: the CPU's own algorithm. Deterministic, and blends in the same order. Fragment-shader storage writes would race where cards overlap, so they are not used. |
| Pick ids, frame stats | Full-frame buffers read every frame | Stats reduced on the GPU (11 classes). Pick ids stay on the GPU and the one pixel under a click is read back. |
| Shading's light list when there are few lights | Allocates a `Vec` per pixel (`all`) | Fixed loop over the light buffer. Remove the CPU allocation too. |

## Phases

Each phase ends green on the parity gate for what it covers, with the rest still drawn on the CPU.
That works because the CPU's buffers can be uploaded mid-frame. Every step can ship, and the
studio gets faster with each one.

0. **Foundations.**
   - wgpu version alignment and egui upgrade; `GpuContext`; WGSL module layout.
   - The pipeline-creation test; the parity harness running CPU against CPU.
   - Timestamp queries per pass where the backend has them, feeding the same `--stages` output.
   - CPU fixes first, proven by the golden check: closed-form `on_branch`; no per-pixel allocation
     in shading.
1. **Shading on the GPU.**
   - The G-buffer is uploaded from the CPU rasteriser.
   - `shade_px` with `surface_uv`, `light_among` with tile lists, gloss, ripples, `weather::surface`
     (wet, puddles, rain rings, snow), fog and banks.
   - The texture array with explicit mips; the u64 hash emulation and noise.
   - Gate: hdr after shading.
   - Shading is the largest pass (5–17 ms on the CPU), so this phase alone roughly halves heavy
     frames.
2. **Reflections and the air.** Reflect (march and blur), mist with air light, light shafts (sun
   march and lamp haloes with cone culling), heat shimmer, lens drops and frost.
3. **Sky and post.**
   - Sky base, sun aureole, moon, stars, clouds with sunlit edges, aurora, rainbow, lightning,
     veil, fog over the sky.
   - Bloom, tone map, saturation, contrast, vignette, grain, kuwahara, grade, outline, lens warp,
     ramp levels, quantize with dither, upscale, paper and scanlines.
4. **Scatter passes.** The sprite atlas; cards with sway, snow caps, glow and pick ids; prop
   shadows; tufts; flames; particles; precipitation, splashes and curtains; drips; sandstorm;
   wisps.
5. **Geometry on the GPU, and several worlds.**
   - Hardware raster into the G-buffer. The triangles are still built on the CPU (small).
   - Facade, realms, `clip_region` half-planes, soft edges between worlds, per-world adaptation.
     These make crossings, forks and journeys GPU-rendered.
   - Gate: the transition and fork tests and the junction pixel-equality tests in `export.rs`.
6. **Switch over.**
   - The studio shows the GPU texture directly and keeps the render-ahead cache.
   - Exports, `pf` and MCP use the GPU when there is an adapter, and the metadata names the engine.
   - `--engine cpu` forces the reference renderer.
   - The runtime crate gets an optional `gpu` feature, so a quartz game can hand over its own
     device. With the versions aligned, it is the same device the game draws with.
   - Docs: manual, skill, README.
7. **Spend the headroom.**
   - Supersampled anti-aliasing for thin rails, bars and distant edges.
   - Higher internal resolution for exports.
   - Anything else the CPU budget forced away.
   - Each is a scene or export option, so the parity gate keeps comparing like with like.

## Sizes, to set expectations

These are the CPU code each phase ports, from the survey:

| Phase | CPU lines it ports |
|---|---|
| 1 | shading ~280, plus fog and helpers ~100, plus `weather::surface` ~100 |
| 2 | reflect ~150, air ~280 |
| 3 | sky ~400, post and looks ~450 |
| 4 | scatter passes ~600 |
| 5 | raster and realms ~400 |

Expect the WGSL to be about the same size, plus `common.wgsl`. The infrastructure (context,
caches, atlas, parity harness, binning) is the part with no CPU counterpart.

## Risks

- **egui/eframe upgrade churn** in the studio: API renames. These are mechanical, but touch many
  files.
- **Sprite atlas packing:** file sprites can be any size, so a page budget and a fallback for
  sprites larger than a page are needed.
- **Float drift in long chains** (fog over far distances, adaptation in crossings). The per-pass
  parity mode exists so this is found where it starts.
- **GPU timing on a busy machine:** compare passes within one trace (as on the Quest).

## Where it stands (2026-10-08)

Built, and drawn on the GPU for every frame that shows one world:

| Pass | WGSL | Notes |
|---|---|---|
| Shading, mist | `shade.wgsl` | Mist loops every lamp in index order (the CPU's tile lists come from a G-buffer the GPU has since changed); out-of-reach lamps add exactly nothing, so the sum is the same. |
| Sky, sky bodies, lightning, veil, fog bank | `sky.wgsl` | Stars and cloud blobs binned per 16 x 16 tile in drawing order. |
| Billboards | `cards.wgsl` | Sprites in one texel store keyed by `Arc` pointer; mip level chosen on the CPU (`Sprite::lod`). Pick ids and depth match the CPU exactly. |
| Tufts, flames, wisps, particles, precipitation, splashes, curtains, drips, sand | `splat.wgsl` | Every pass now emits `render::splat::Splat` shapes; one list drawn by either engine. Blades (which write the G-buffer) in their own dispatch; splash gates read it after. |
| Reflections | `reflect.wgsl` | `ColumnBlur` as double-float (hi + lo) running sums: exact for these sums, as f64 is. Blocked scan (64-row blocks). Skipped when nothing in the frame can reflect. |
| Light shafts | `shafts.wgsl` | Setup shared with the CPU (`weather::shaft_setup`). |
| Crop, pick, heat shimmer, lens drops and frost, bloom, tone map, Kuwahara, grade, outline, lens warp, ramp levels, palette and dither, stats, upscale, paper and scanlines | `post.wgsl` | One scratch buffer at offsets the CPU plans per frame; tables (sRGB thresholds, palette LUT, `.cube`) cached by pointer. Only the output bytes come back (plus pick, depth, stats when asked). |

Still on the CPU: rasterisation and prop shadows (about 1.5 ms), and every frame of two
worlds (crossings, forks, journeys). Those frames draw on the CPU even with a GPU renderer; the
joins of transition and fork clips are frames of one world, so they still match the loops
exactly on either engine (tested).

**Parity** (`pf parity --engine gpu`): every preset at two times, max difference 1/255; 31
weather scenes and an all-sky stress scene, max 1-6; 19 style variants pass (palettes snap an
isolated pixel to the next colour, p99.9 = 0).

**Fast math.** wgpu-hal 25 builds every Metal library with `CompileOptions::new()`, so fast math
is on and there is no switch short of a fork. Division (`div`) is corrected with an fma, and
`rnd(x)` (an OR with a uniform that is always zero) stops a product being fused into an add or a
sum being reassociated wherever a threshold or a floor follows. With those, alpha ties at exactly
0.5 land on the same side as on the CPU.

**Where it is used.** The studio preview (each worker has its own buffers on eframe's device),
`pf`, exports and MCP use the GPU when there is an adapter. `--engine cpu` (or `PF_ENGINE=cpu`)
forces the reference renderer; export metadata says which drew the frames. The game runtime
stays on the CPU unless the game calls `Runtime::use_gpu` (optionally with its own device).

**Timings** (480 x 854, M4, per frame): Bog Boardwalk 27.9 -> 12.7 ms, rain 27.5 -> 13.2,
Ice Cave 23.9 -> 11.9, Haunted Forest 17.0 -> 10.1, Forest Path 11.9 -> 9.6. The GPU passes
take 3.5-6 ms of that; the rest is the CPU's raster, shadows and buffer preparation before the
GPU starts, and the wait. `pf bench --stages` lists GPU pass times (`g.*`, timestamp queries).
Studio: rain plays 24/24 frames on time at ~30 ms per worker (CPU preview ~51 ms).

**Forks that divide the road** (`path.fork.style: "Split"`, `side: "Both"`) were added on both
engines together: `Forks::branch_sd` on the CPU, `branch_sd` in `common.wgsl`, parity max 1/255.

Next: frames of two worlds on the GPU (phase 5), the raster, zero-copy display in the studio.
