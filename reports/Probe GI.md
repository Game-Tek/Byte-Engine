# Ray-traced probe GI

**Byte-Engine now has a third global illumination technique: `render.gi.technique = probes`, a DDGI-style volume of
ray-traced irradiance probes around the camera.** It runs end to end in the model viewer on the M4. Lighting Sponza, it
settles within 50 frames of the scene appearing, and its light changes by 0.1–0.2 % between captures four frames apart
(surfel GI: 0.3 %). Its mean indirect light is about 16 % above surfel GI's, and the medians differ by 7 %. Unlike
surfel GI, it also bounces point and cone lights, so the night scene gets indirect light where surfel GI gets none. It
costs 1.34 ms at the default 65,536 rays a frame against surfel GI's 0.88 ms, or 0.91 ms at 32,768 rays. It is
committed on `dev` as `34fcc38b5`, after a cleanup pass that shares the hit sort, the light table's layout, and the
normal reconstruction with surfel GI and SSGI. That pass measured neutral, and it left your own uncommitted work
untouched and uncommitted.

The design follows the literature survey in `research_notes/Probe GI research/literature-survey.md` (35 sources, from
DDGI 2019 and 2021 through RTXGI, idTech 8, and Frostbite). The closest shipped match is idTech 8: trace a bare hit
record, then shade the hits in a separate, material-sorted pass.

## 1. Try it

```bash
cd ~/development/sandbox/model_viewer
cargo run --bin model-viewer -- --resources.path=/tmp/probe-res --render.probe-gi.enabled=true --render.gi.technique=probes
```

Use a fresh resources directory, because the bake changes. Add
`--render.debug.view.VisibilityPipelineManager.probes=true` to draw the probes as spheres, or `.probe-light=true` for
the light the probes give each pixel. `render.probe-gi.ray-budget` (default 65,536) and `render.probe-gi.spacing`
(default 1 m) size it at startup.

| Images | Left | Middle | Right |
| --- | --- | --- | --- |
| `Probe GI/final-compare.png` | Probe GI | Surfel GI | GI off: the environment lights every surface as if nothing hid the sky |
| `Probe GI/gi-compare.png` | Probe GI's indirect light alone | Surfel GI's | |
| `Probe GI/views.png` | `probes` view: active probes lit, idle blue, buried red | `probe-light` view | |
| `Probe GI/startup-compare.png` | 100 frames after load, before the restart fix | The same, after | |
| `Probe GI/move-compare.png` | 4 frames after a 10 m jump, before the transient fixes | The same, after | |
| `Probe GI/sun-compare.png` | Before turning the sun | 15 frames after | 900 frames after |
| `Probe GI/night-compare.png` | Night, probe GI's indirect light (×2000) | Surfel GI's: none | |

![Final frames: probe GI, surfel GI, GI off](Probe%20GI/final-compare.png)
![Indirect light: probe GI, surfel GI](Probe%20GI/gi-compare.png)

## 2. How it works

| Part | What it does |
| --- | --- |
| Volume | 4 cascades of 24 × 12 × 24 probes around the first sink's camera, 1, 2, 4, and 8 m apart by default. Each window snaps to its grid, and a grid point keeps its slot while the window moves, wrapping toroidally, so a camera move only resets the slabs it enters. 27,648 probes. |
| Probe data | An 8 × 8 RGBA16F octahedral irradiance tile (6 × 6 plus border: irradiance/π and hit fraction) and a 16 × 16 RG16F distance tile (mean and mean square), in two atlases of 13.5 and 27 MiB. A 48-byte record holds the relocation offset, state, coordinate, update count, and last update frame. |
| `probe-schedule` (0.10 ms) | One thread per probe resets the slots the windows moved past, then buckets every probe by priority into a 64-bucket histogram. Unset probes come first, then the log of frames since the last update times an importance: 8 when a pixel read the probe in the last 8 frames, 1 for other active probes, 0.125 for idle and buried ones. One thread finds the cut, and a gather pass lists the winners in an order that changes every frame. |
| `probe-trace` (0.78 ms) | 128 rays per listed probe along a spherical Fibonacci pattern with a fresh random rotation each frame, traced inline. Each hit stores a bare record, which is counting-sorted by material. A ray-generation pass then `invoke`s each hit's material closest-hit shader, the same machinery surfel GI uses, now shared. The hit reflects the suns, the point and cone lights (shadow rays, no IES profile), and the volume's own light from the last update, for unlimited bounces at 0.95 energy. A miss brings back the environment. |
| `probe-update` (0.33 ms) | One 256-thread workgroup per probe: 36 lanes blend irradiance texels (cosine-weighted) and 196 lanes blend distance texels (cos⁵⁰), then the lanes fix the borders. One lane relocates and classifies the probe. Hysteresis is 0.97, ramping in as a running mean over the first 32 updates. A probe whose mean ray light differs tenfold from its map's restarts its average; see section 4. |
| Resolve (+0.41 ms in `screen-space`) | Per sink, at half resolution, each pixel rebuilds its surface from the depth pyramid. It biases its point by (0.2 n + 0.8 v) × 0.225 × spacing and weighs the 8 active probes around it: trilinear, a wrapped backface term, cubed Chebyshev visibility, and a crush below 0.2. Cascades blend over 2 probes, and unset probes hand their share to the next cascade. The resolve writes `SSGI History` and `SSGI Normals`, the images material evaluation already reads, and marks the probes it read so the schedule favors them. |

What I believe is new compared with DDGI and RTXGI:
- The GPU priority scheduler, with pixel feedback: seen marks drive importance, with no CPU readback in the loop.
- Coverage-weighted cascade hand-off for unset probes.
- A probe-wide restart test, which works at any scale of light, where RTXGI's per-texel thresholds act in gamma space.
- Log-scale debug views that show indirect light even in shade.

Material evaluation is untouched; probe GI writes the same images SSGI and surfel GI write.

## 3. Results on Sponza

All numbers are from the M4 at 1024 × 1024, on frames forced one at a time through the inspector (the screen was
locked). The GPU then sits in a low clock state, so absolute times run high; compare them with each other. Each time is
the GPU median of about 400 frames.

| Configuration | Scene GPU ms | Added by GI | Frame-wide stages | `screen-space` |
| --- | --- | --- | --- | --- |
| GI off (probe GI built in) | 3.885 | | | 0.917 |
| Probe GI, 65,536 rays | 5.222 | +1.34 | 1.21 (0.10 + 0.78 + 0.33) | 1.327 |
| Probe GI, 32,768 rays | 4.793 | +0.91 | 0.80 (0.10 + 0.52 + 0.18) | 1.317 |
| GI off (surfel GI built in) | 3.867 | | | 0.916 |
| Surfel GI | 4.748 | +0.88 | 0.59 (0.19 + 0.33 + 0.06) | 1.500 |

The frame-wide work overlaps the raster passes: the stages sum to more than what they add. Probe GI's resolve is
cheaper than surfel GI's (+0.41 against +0.58 ms), but it traces 14 times more rays (65,536 against 4,720).

| Quality | Probe GI | Surfel GI |
| --- | --- | --- |
| Mean indirect light (pre-exposed irradiance/π), p50, p10 | 0.0083, 0.0030, 0.0005 | 0.0072, 0.0028, 0.0004 |
| Mean change between captures 4 frames apart | 0.12–0.24 % | 0.3 % |
| Frames from the scene appearing to a steady mean | ≤ 50 | ≤ 100 |
| Sun turned: share of the change left after 5 / 15 / 60 / 120 / 240 / 480 frames | 69 / 51 / 31 / 20 / 9 / 0 % | 97 / 66 / 19 / 9 / 6 / 3 % |
| Night scene (cone and point lights only), mean indirect light | 0.0002 | 0 |

The two techniques are independent estimators of the same light, so their agreement is the main correctness evidence I
have; there's no path-traced reference in the engine. Probe GI is smoother: surfel GI shows its typical patches on the
curtains and vaults. It is also flatter, because probes don't resolve detail below their spacing. In the light view, the
vaults and arcades darken where they should, and I saw no light leaking through Sponza's walls.

![Probe debug views](Probe%20GI/views.png)
![Night: probe GI's indirect light, surfel GI's](Probe%20GI/night-compare.png)

## 4. Three problems the measurements found

1. **Startup sky.** Probe light started 3× too bright (p10 15×) and took over 1,000 frames to settle (see
   `Probe GI/startup-compare.png`). The probes update from the first frame, before the BLAS builds and material hit
   groups exist, so every ray missed and brought back sky. The 32-update running mean, then hysteresis 0.97, kept that
   sky for seconds. Removing multi-bounce, the coarse-cascade fallback at hits, and my per-texel rules each left it in
   place. The fix is general rather than a startup special case: a probe whose rays' mean luminance differs tenfold
   from its irradiance map's mean restarts its average. Now it settles within 50 frames and stays flat through 1,400.
   It also catches lights switching and level loads.
2. **Change detection that amplified noise.** My first version lowered hysteresis to 0.75 whenever a texel changed by
   25 % (DDGI 2021's rule). With 128 rays, a texel that sees a small sunlit patch has about 45 % noise per update, so
   the rule fired on noise and the light shimmered 0.7–1.8 % between captures. RTXGI's asymmetric version (fast
   darkening, damped brightening) cut the shimmer but biased dark texels darker. The probe-wide restart replaced both:
   noise moves single texels tenfold, but seldom the mean of all 128 rays.
3. **Transients after a camera jump.** The newly scrolled-in probes were unset, so a pixel switched hard to the coarser
   cascade, and the gather filled them in slot order. The result was a vertical seam sweeping across the far wall (see
   `Probe GI/move-compare.png`). Unset probes now hand their trilinear share to the next cascade, and the gather visits
   slots in a per-frame permutation (stride 7919, coprime with 27,648). Four frames after a 10 m jump, the frame now
   shows only mild young-probe noise.

![Startup: before and after the restart](Probe%20GI/startup-compare.png)
![Four frames after a jump: before and after](Probe%20GI/move-compare.png)

## 5. Literature, briefly

The full survey is in `research_notes/Probe GI research/literature-survey.md`. The points that shaped this design:

| Source | What I took |
| --- | --- |
| DDGI 2019 (Majercik et al.) | Octahedral irradiance plus distance moments, the Chebyshev visibility weight, the multi-bounce feedback through the previous volume, spherical Fibonacci rays rotated each frame. |
| DDGI 2021 / RTXGI SDK | The (0.2 n + 0.8 v) × 0.75 × spacing × 0.3 bias, backface distances × 0.2, relocation through the nearest backface within 0.45 spacing, classification, crush below 0.2, camera-locked scrolling windows, hysteresis 0.97. I deviated from their per-texel change rules (section 4). |
| idTech 8 (Sousa, SIGGRAPH 2025) | Trace a bare hit record, shade separately by material, use the previous volume for bounces. They update one of six 16³ cascades a frame at 16–64 rays; I schedule by priority instead. |
| Frostbite GIBS (Apers, SIGGRAPH 2024) | Priority scheduling with low priority for probes that see no geometry (about 6k of 100k probes a frame). Probes as the cache at the end of shortened surfel rays, which saved them about 60 % of surfel time. Shadow maps at hits saved them 34 % of probe tracing. |
| Apple (WWDC22/23, M3 tech talk) | Use the `intersector` with opaque geometry, not intersection queries. The engine's MSL lowering already does. |

Shipped budgets are about 65k–260k probe rays a frame, and the default of 65,536 sits at the low end. No one has
published probe GI timings on Apple GPUs; these are the first numbers I know of.

## 6. Limitations and open questions

- **Metal only.** Probe GI needs ray tracing, which only the Metal backend has. Only the MSL lowering of the probe
  shaders was tested; GLSL and HLSL weren't run, so check them before a Vulkan or Direct3D 12 bake that builds it in.
- **Lights at hits.** Each hit loops over every light in the table, with a shadow ray for each one in reach, so the
  cost grows with the light count. IES profiles are ignored at hits. Surfel GI still lights hits with suns only; the
  same loop would carry over to it.
- **One dark probe.** The probe 1 m ahead of the spawn point stores about 35× less light than its neighbors and draws
  black in the `probes` view (it isn't NaN; I checked the raw target). The pixels around it look right. I suspect
  geometry the rays see but the raster pass doesn't. I didn't find the cause.
- **Banding in one dark vault.** Faint stair-step banding shows in the left gallery's vault in the `probe-light` view.
  It is a classic DDGI interpolation artifact where visibility changes across cells.
- **Debug views.** When surfel GI is also built in, the sink declares the surfel views and the probe views are left
  out, because a pass holds at most 8 views.
- **SSGI looked broken in this tree.** While setting up comparisons, `--render.ssgi.enabled=true
  --render.gi.technique=ssgi` gave an unshadowed image, and `SSGI History` read back all zeros. My changes don't touch
  that path, and your uncommitted Radiance History removal is in the same area. I did not investigate.
- **Fixed sizes.** The grid and cascade count are compile-time constants.

## 7. Next steps, ranked

1. **Probes as surfel GI's far field**, as Frostbite does. Shorten surfel rays and read the probes where they miss.
   This should cut surfel trace time and stabilize new surfels, since both techniques can be built in at once.
2. **Shadow maps at hits** for suns inside the cascades, falling back to shadow rays (Frostbite: −34 % of probe
   tracing), and a world-space light grid so hits skip unreachable lights.
3. **Fewer rays per probe, more probes.** Try 64 rays (idTech 8 uses 16–64), and drop rays for idle probes to RTXGI's
   32 fixed rays.
4. **RGB9E5 irradiance**, which is shader-writable on Apple3 and later, to halve the irradiance atlas without
   R11G11B10F's color shift.
5. Hook light-change events (a light switched, a time-of-day jump) to a temporary hysteresis drop, as DDGI 2021 does.

## Appendix: measurement scripts

The scratchpad scripts were `run_viewer.sh` (launch, wait for the scene, force frames, raw captures, checkpoints,
camera moves), `raw_tool.py` (raw RGBA16F statistics and PNGs without numpy), and `sun_test.py` (turns the sun through
an inspector `TransformationUpdate`). I can move them into the repository if they're useful.
