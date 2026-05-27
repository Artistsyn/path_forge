# PathForge GPU Parity + Export Performance Plan (2026-05-27)

## Goals
1. Restore exact visual parity with CPU renderer for all presets.
2. Keep GPU path enabled only when parity is verified.
3. Make exports fast enough for practical iteration.
4. Remove loop seams from randomized props/shading.

## What is already fixed in this pass
- Safety: GPU scene parity gate is conservative (CPU fallback for correctness).
- Seam randomization: prop random hashes now use loop-wrapped seed indices under seamless lock.
- Export speed: composite CPU export now uses one render pass per frame/sample instead of five layer passes.

## Current GPU parity gaps (source-audited)
1. Floor and wall tile sampling
- CPU: true tile texture sampling with orientation and scale.
- GPU: procedural hash-based color pattern.
- Effect: path/wall appearance mismatch.

2. Curve-aware floor sampling
- CPU: per-pixel curve-adjusted depth sampling path.
- GPU: simplified row model.
- Effect: curvature and perspective mismatch on many presets.

3. Prop rendering richness
- CPU: full prop geometry variants, row spawning, sprite billboards, jitter/shadow richness.
- GPU: compact silhouette renderer.
- Effect: tree shape/type/shading mismatch.

4. Atmosphere richness
- CPU: richer per-layer effects and debris/motes interactions.
- GPU: reduced model.

5. Post and lighting calibration
- CPU and GPU use different approximations.
- Effect: brightness and contrast mismatch in edge presets.

## Implementation phases

### Phase 1: Deterministic loop foundations on GPU
- Add loop uniforms: loop_s and seamless_lock to GPU params.
- Add WGSL helper to wrap instance keys by periodic slot count.
- Use wrapped keys for all prop and atmo random selections.
- Acceptance: first and last loop frame hashes match for deterministic props under seamless lock.

### Phase 2: Texture parity for floor and walls
- Bind floor/wall texture atlases to GPU scene pipeline.
- Replace procedural floor/wall color with texture sampling parity math.
- Match CPU tex_scale and 90-degree rotate flags.
- Acceptance: CPU vs GPU mean absolute error under threshold on canonical presets.

### Phase 3: Prop parity path
- Add prop type-specific shape logic parity (or sprite-first path with atlas).
- Add GPU sprite atlas upload + per-instance sprite index.
- Match CPU path_follow, edge placement, and shadow controls.
- Acceptance: tree lines and silhouette clusters visually match CPU in test presets.

### Phase 4: Atmosphere parity path
- Add debris/motes and atmo light contribution parity toggles.
- Keep additive glow behavior numerically close to CPU output.
- Acceptance: atmo-heavy presets match scene mood and glow rhythm.

### Phase 5: Export throughput
- Keep current one-pass CPU composite optimization.
- Re-enable GPU scene for parity-approved combinations.
- Add readback staging ring buffer and frame pipelining for GPU export.
- Add optional reduced smoothing for sky-only and low-motion cases.
- Acceptance: composite export throughput significantly better than previous 5-pass baseline.

## Validation protocol
1. Build preset corpus: default + test1 + stress scenes.
2. Render CPU and GPU frames for same scroll/global_t samples.
3. Compute per-frame image difference metrics.
4. Gate GPU enablement only when preset features pass thresholds.
5. Keep CPU fallback as hard safety.

## Immediate next coding steps
1. Implement Phase 1 uniforms and wrapped random key logic in GPU scene WGSL.
2. Implement floor/wall texture bindings and sampling parity in GPU scene.
3. Add per-feature parity flags so GPU can be enabled incrementally without regressions.
