# Material evaluation (pipelines mode) at 4K: bottlenecks and a 21 % cut

Model viewer, Sponza hall view, 3840 x 2036 (the window's title bar takes the rest of 2160), Apple M4, release bake
with debug views off, `--unattended=30`, GPU counters on. Every number is the median of `stage.material-evaluation`
over 200+ frames, counting only frames whose shadow raster (0.53 to 0.60 ms) and early visibility raster (< 1.2 ms)
show no interference from other processes on the GPU. Back-to-back baselines agree within 0.03 ms (5.13 to 5.15 ms);
over hours the machine drifted by up to 4 %, so every comparison below is against a baseline of the same sweep.

## Result

| variant | material evaluation | new pass | scene |
|---|---|---|---|
| baseline | 5.13 / 5.15 ms | — | 21.34 ms |
| indirect diffuse extracted | 4.14 / 4.14 ms (−19.3 %) | 2.36 ms | 22.64 ms |
| extracted + sun fast path | **4.07 / 4.06 ms (−21 %)** | 2.36 ms | 22.59 ms |
| upsample fused into the sun visibility resolve + sun fast path | **4.07 ms (−20.7 %)** | +2.1 ms in `ss-sun` (4.34 → 6.47) | 22.45 ms |

The material pass gets 21 % cheaper; the scene gets 1.25 ms slower, because the extracted upsample costs as much in its
own pass as it did inside material evaluation, plus a second surface rebuild. The lit image matches the baseline
(whole-image sum within 0.4 %, per-region differences inside run-to-run surfel GI noise).

Fusing the upsample into the sun visibility resolve removes the second surface rebuild (that pass already has each
pixel's position and triangle) and recovers only about 0.15 ms of it: the resolve grows by 2.1 ms, so the scene is still
1.1 ms slower than the baseline. The upsample's weighing work costs about 2 ms wherever it runs, and material
evaluation hid part of it under its own fetch latency. The fused image matches the baseline visually and in its
whole-image sum (within 0.6 %).

## Where the 5.14 ms goes (ablations, one region removed at a time)

| removed | material evaluation | saved |
|---|---|---|
| indirect diffuse upsample (12 taps, plane weights, surfel retry, GTAO combine) | 3.97 ms | **1.17 ms** |
| screen-space reflection march (roughness < 0.4) | 4.33 ms | **0.81 ms** |
| the whole light loop (one sun) | 4.46 ms | **0.69 ms** |
| — of which: the cluster lookup | 5.00 ms | 0.14 ms |
| — of which: the sun visibility fetch | 5.03 ms | 0.10 ms |
| — of which: GGX/Fresnel (vs N·L) | 5.04 ms | 0.10 ms |
| — of which: the upsample's surfel retry | 4.90 ms | 0.24 ms |
| local-light code (never runs in this scene) | 5.17 ms | 0 |
| anisotropy 16 → 4 / → 1 | 5.06 / 5.05 ms | 0.08 ms |
| reflections at 16 steps / on depth mip 1 | 4.98 / 4.98 ms | 0.16 ms |

## Cheaper weighing, inside material evaluation

Black Ops 3's upsampler (and Unity HDRP's 2 x 2 bilateral upsample) spends the full bilateral weighing only at
discontinuities. Measured against a baseline in the same sweep:

| variant | material evaluation |
|---|---|
| baseline | 5.15 ms |
| bilinear when the four taps share the pixel's plane and normal (test after all twelve reads) | 5.13 ms |
| the same, forced on every pixel (upper bound, wrong at edges) | 4.62 ms (−10 %) |
| **plane test from the depth gather alone, then one filtered sample of the history** | **4.95 ms (−4 %)** |

The last variant skips the eight normal and light fetches and the weighing on flat regions; its image matches the
baseline within run-to-run noise. It needs the history bound with the linear sampler. The upper bound shows the weighing
itself is worth at most about 0.5 ms here, so cheaper weighing plus the sun fast path and cheaper reflections reach
about −10 % in place, not −20 %; only moving the upsample out of the pass reaches −20 %.

The same depth-only fast path in the fused sun visibility resolve: material evaluation 4.07 ms (−21 %), scene 22.27 ms
against 21.28 ms for the baseline and 22.35 ms for the fused version without it. The fast path saves under 0.1 ms
there, so the fused shape still costs the frame about 1 ms.

## What does not work on the M4, measured

- **Lowering the register peak.** Both Sponza shaders sit at the 96-register cap and spill (48 and 16 bytes). Removing
  the never-taken spot and point shadow filters removes every spill and changes nothing (5.17 ms). Fusing the
  upsample's two weighings into one pass lowered its region from 95 to 87 registers and made the pass 2.5 % slower.
  The M4's dynamic register caching charges only for what runs, so static register counts are not a cost model here.
- **`[[noinline]]`** on the shadow and reflection helpers produces real calls but the same peak and spills.
  **`max_total_threads_per_threadgroup`** hints change nothing.
- **Issuing fetches earlier.** Moving the upsample before the material body: +5 %. Fetching the sun visibility before
  the light loop: +2 to +5 %. The compiler's own schedule is better.
- **Retrying only when every tap is rejected** (not when the light is zero): no change, so the retry fires on real
  rejections.
- **Loading the upsample's 6 x 6 window into workgroup memory** in the extracted pass: 2.44 ms against 2.21 ms per-pixel.
  The taps are not texture-bound; their weighing work costs about 1 ms wherever it runs.

## Hardware counters: why it is slow

A one-frame GPU trace at 4K (captured with `MTL_CAPTURE_ENABLED=1` and `gpucapture`, then replayed and profiled
headlessly with `gpudebug ... profile run --gpu-state high`), with SSGI in place of surfel GI because the capture layer
cannot handle the ray tracing pipeline. The counter time series over material evaluation's 5.3 ms window, against the
prototype indirect diffuse pass and the screen-space passes before it:

| counter | material evaluation | indirect diffuse pass | screen-space passes |
|---|---|---|---|
| occupancy manager target | **29.5 %** | 67.8 % | 100 % |
| kernel occupancy | 29.5 % | 49.3 % | 22.4 % |
| L1 holding registers | **12.8 %** | 1.3 % | 0 % |
| stack reads from L1 (spills that run) | **4.7** | 0 | 0 |
| L1 holding buffers | 46.0 % | 39.6 % | 0.6 % |
| instruction throughput limiter | 45 to 60 % | 85 to 88 % | 35 % |
| texture read limiter | 40 to 55 % | 18 % | 5 % |

Nothing is saturated in material evaluation: the GPU's occupancy manager holds it to under a third of its threads,
because its registers overflow into L1, where the spills it executes and the buffer data of the surface rebuild and
the light table compete. With so few threads, every fetch's latency shows. The prototype pass is the opposite: high
occupancy and bound by its weighing arithmetic. This is why only removing whole regions of executed code moved the
time, and why static register counts of never-executed code did not.

## The extracted pass, split

| pass body | cost |
|---|---|
| instance test and one RGBA16F write | 0.16 ms (measured before the image was sized, so a lower bound) |
| plus the surface rebuild (triangle, meshlet, indices, vertices, transforms) | 1.17 ms |
| plus the 12-tap upsample with its retry and GTAO | 2.36 ms |

## Recommendations, for the user to decide

1. **Extract the indirect diffuse upsample** (the −19 %) only if the material pass itself is the budget. Both shapes
   were measured: as a standalone pass the scene is +1.25 ms, and fused into the sun visibility resolve it is +1.1 ms.
   For the frame, the cheaper path is to make the upsample's weighing cheaper wherever it runs (fewer taps, or weights
   computed once at SSGI resolution and stored), since that work is what costs about 1 to 2 ms.
2. **Sun fast path** (the remaining −2 %, exact). Directional lights reach every pixel, so the light table can list them
   first, the clusters only local lights, and material evaluation can visit the suns in a uniform loop before the
   clustered one. Measured here by skipping the cluster lookup, which is exact for this scene's one sun.
3. **Reflections** are the next largest region (0.81 ms). Fewer steps or the coarser mip each buy only 3 % and change
   the image; a hierarchical trace on a deeper pyramid is the structural option.

## Prototype

In a scratch worktree, not in this checkout: `BE_ABLATE` (comma-separated) rewrites the generator's BESL at bake time
for each ablation; `assets/rendering/visibility/indirect-diffuse.besl` is generated from the material shader's own
helpers; `render_pass.rs` sizes its RGBA16F image with the sink, records it after the screen-space block with its own
`stage.indirect-diffuse` counter, and binds it at 1064 of the material evaluation layer. Tools used: the skill's
`pipeline_statistics.m`, extended to specialize the `transparent` constant, for register and spill counts of the
intercepted MSL. GPU trace capture with surfel GI on fails: the capture layer throws `unrecognized selector` on the
surfel GI hit-shader pipeline's MTL4 dynamic linking descriptor, an Apple tooling bug. With
`--render.gi.technique=ssgi --render.surfel-gi.enabled=false` (and a bake with `surfel-gi` off) the capture works.
`gpudebug`'s per-command and per-shader rankings came back empty for these MTL4 command buffers; the encoder timeline
and the counter time series work, and `fetch <counter> --out <file>` writes each series as JSON.
