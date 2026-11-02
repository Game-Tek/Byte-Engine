# Byte-Engine visibility renderer: the particulars

The generic method and techniques live in `SKILL.md`. This file holds what is specific to Byte-Engine and the model
viewer, as of 2026-10-06, with the surfel GI notes as of 2026-10-30.

## Where we are

Target: the model viewer (`~/development/sandbox/model_viewer`) rendering Sponza at 1024x1024 on the Mac mini M4,
toward and below a 10 ms frame. The paced frame started at 12.7 ms on 2026-10-03 and is 7.80 ms in the same GPU
state; in the GPU's high state the build before the last two steps was 4.39 ms. Numbers are the pinned hall view with
`--render.directional-shadows.resolution=2048` unless stated. They include the courtyard's rain, splashes, and mist,
which the viewer has left out by default since 2026-10-27; pass `--viewer.rain=true` to compare against them.

Current breakdown (low GPU state, `stage.*` medians in ms):

| stage | ms | notes |
|---|---|---|
| material-evaluation | 1.81 | per-material indirect dispatches of one generated shader; low occupancy |
| ssgi | 1.46 | trace 0.79 (16 steps + 4 bisections, half res) and temporal 0.67, measured with a temporary split |
| shadow-maps | 1.23 | 4 cascades rasterized as layers of one D16 array; the regime key |
| sun-visibility | 1.14 | contact trace (half res) fused with the directional shadow resolve |
| shadow-depth-pyramid | 0.10 | max and min 8x8 cells per cascade, four gathers per thread and two shuffles per cell; the D16 array is read compressed, so issuing gathers bounds it (0.41 before 2026-10-06) |
| visibility-early | 0.44 | meshlet raster |
| particles | 0.34 | the demo's rain and splashes, real content |
| material-prepasses | 0.30 | count, offset, pixel mapping |
| depth-pyramid | 0.14 | linear depth pyramid plus the shadow receiver bounds, in 16x16 workgroups |
| cascade-fit | 0.004 | four threads; the bounds come from the depth pyramid pass |
| everything else | 0.26 | fog, ACES, occlusion pyramid, background, late raster, clusters |

The stages summed to about 7.1 ms against a 7.24 ms scene time before the pyramid change, which took about 0.3 ms
off both; the gaps between stages shrank from 0.25 to about 0.1 ms when the bounds pass, its clear, and their
encoder boundaries went away.

What each technique in `SKILL.md` did here, in order of application: stage counters (2026-10-04); the history clear
removed into the pixel-mapping prepass (12.67 to 12.46 ms); `render.directional-shadows.resolution` (raster 2.94 to
1.22, pyramid 1.30 to 0.30); contact shadows traced at half resolution on depth pyramid mip zero with a fused
plane-aware upsample (trace 1.75 to 0.43); the SSGI upscale folded into material evaluation with batched fetches
(frame 9.35 to 9.15); the directional shadow resolve extracted into `sun-visibility.besl` (material evaluation 3.33
to 1.80, frame 8.92); lit and umbra probes on the max and min cascade pyramids (sun visibility 2.32 to 1.23, frame
8.04); `gather` in the pyramid build, the spacing-one tent, and the blocker search (sun visibility 1.20 to 1.14);
the receiver bounds folded into the depth pyramid pass with the fit resetting them (frame 8.07 to 7.83); the sun
pass's cascade constants on four lanes and one receiver projection per cascade (exact, 0.004 ms). The occlusion pyramid builds in
two dispatches through workgroup memory instead of nine, stores 16-bit floats rounded toward the far plane, and
dropped from 0.089 to 0.065 ms with culling counts unchanged. The cascade depth pyramids store
16-bit unorm cells (`StorageImage<r16>`, `Formats::R16UNORM`): exact, half the memory, no time change. Since
2026-10-06 the cascade pyramid gathers four quads per thread and combines each cell's four threads with two
`subgroup_shuffle_xor_f32` calls, instead of one gather per thread, a barrier, and a two-lane tail: pyramid 0.41 to
about 0.10 ms and scene 6.02 to 5.69 ms in forced low-clock frames; at the default 4096 cascades, pyramid 1.97 to
0.58 ms and frame 10.41 to 9.08 ms. Cells and `Sun Visibility` are identical. The old kernel was not
bandwidth-bound: the M4 keeps the shadow map losslessly compressed (see "Check compression before blaming bandwidth"
in `SKILL.md`). Tried and reverted: prefetching the SSGI march four steps at a time (trace 0.79 to 0.87 ms, most rays
end early).

## Surfel GI

`--render.gi.technique=surfels --render.surfel-gi.enabled=true` turns it on. The kajiya review changes of 2026-10-30
raised its cost on Sponza from 1.34 to 1.96 ms over GI off; the design page
`docs/develop/rendering/visibility/surfel-gi.mdx` keeps the per-pass table. What brought it back down, each step
measured in one clock state:

- The grid's count and scatter run four threads per surfel (`SURFEL_GRID_LANES` in `layouts/surfel-gi.besl` and
  `render_pass/surfel_gi.rs`), each taking every fourth row of the surfel's cells. The count's first lane runs the
  surfel economy and hands its verdict to the others with two shuffles, and the lanes add up their distance
  histograms with shuffles, so all four pick the same listing (grid 0.31 to 0.16 ms in the fast state).
- The temporal accumulation is a helper in `layouts/surfel-temporal.besl` that the resolve calls, once its four
  bilinear taps' reads were batched (0.230 to 0.192 ms batched, then 0.642 plus 0.192 to 0.726 ms fused).
- The spatial filter issues each tap's four reads before testing them (0.123 to 0.113 ms).
- Tried and reverted: batching the resolve's cell-list reads, four entries and their records at a time (0.98 to
  1.09 ms; it raised the resolve from 81 to 93 registers a thread, against a cap of 96), and moving the history blend
  behind the spawn votes (81 to 71 registers, but the A/B pairs moved -6 % and +4 %).

Final A/B on 2026-10-30, every run in one clock state (shadow raster 1.08 to 1.09 ms), against GI off at 5.05 ms of
scene: the old surfel GI added 1.32 to 1.33 ms, the reviewed one 1.74 to 1.81 ms (1.96 before the steps above). Of
the remaining 0.44 ms, the resolve with its history blend takes 0.19, the spatial filter 0.13, the integrate pass
and its hit spawns 0.04, the validation 0.03, and the grid 0.03. What is left changes what the passes compute, so
it is the user's decision: validate less often, skip the spatial filter or its fallback for confident pixels, or
read fewer than 64 surfels a pixel.

To time single dispatches inside a stage, add temporary handles to `StageCounters` with
`builder.create_gpu_counter("tmp-...")` and nest them inside the stage's counter:
`c.counter(handle, |c| record_compute_stages(c, None, &stages[a..b]))`. Remove them before handing the change over.

## Before you measure

Check each of these once per session, before the first build or run:

- **The target directory resolves.** `~/development/sandbox/target` is a symlink into mbx's cache, and mbx deletes
  whole target directories when disk runs low. If `test -e ~/development/sandbox/target` fails, remove the dangling
  link (`rm ~/development/sandbox/target`) and build again. Don't point the sandbox at the engine's `target/`: the
  sandbox builds with `target-cpu=native`, so the two workspaces rebuild each other's artifacts.
- **The bake matches a release run.** A debug BELD bakes debug views into every shader. Bake with a config that turns
  them off and run with GPU counters on, which release builds leave off:

  ```bash
  cd ~/development/sandbox/model_viewer
  echo '{"render":{"debug-views":{"enabled":false}}}' > <scratch>/release.json
  ../target/release/model-viewer-beld --source "$PWD/assets" --destination <store> --config <scratch>/release.json bake
  ../target/release/model-viewer --resources.path=<store> --render.ghi.features.counters=true --inspector.port=<port>
  ```

- **The window presents.** Front it and sample only presented ticks. On a hidden window or locked screen, force
  frames from about six parallel `GET /screenshots?sink=0&format=raw` clients, never one at a time: serial frames
  give temporal passes no history, so they skip work and their stage times are wrong (see "Determinism" below).
- **One dispatch is one counter.** Ray-tracing hit shaders run inside the trace dispatch, so no timestamp separates
  closest-hit from the trace. Measure a hit shader by A/B against a stub hit shader, or with Metal System Trace.
- **Register and occupancy numbers come from `scripts/pipeline_statistics.m`**, not from AIR or MSL.
- **Tests need their features.** `cargo nextest run --workspace` turns on `gpu-processing` for every package. A run of
  one package needs it named: `cargo nextest run -p byte-engine-resource-management --features gpu-processing`.
  GPU-backed GHI tests also need `--ignore-default-filter`.
- **Only macOS runs here.** Linux (Vulkan, Wayland) can't be built on this Mac; check edits against the pinned `ash`
  signatures and say they weren't compiled. Windows can be cross-checked: run `cargo fetch --target
  x86_64-pc-windows-msvc` once, then `cargo check -p byte-engine-ghi --target x86_64-pc-windows-msvc`. `dev` already
  has DX12 errors there, so compare the error list before and after the change.
- **Format only the files you changed**, with the `rustfmt` line under "Working agreement with the user".
  `cargo fmt --check` reports drift in files nobody touched, so it isn't a pass or fail gate.

## Measuring here

- Build from the sandbox workspace: `cd ~/development/sandbox && cargo build -p model-viewer`. After a change to a
  shader generator or a BESL backend, delete `~/development/sandbox/model_viewer/resources` before launching, or the
  viewer runs the stale baked shader. An edit to a `.besl` asset rebakes on its own, because baked resources record
  their assets' content hashes, so an A/B can swap one asset file between runs. Launch from the crate directory, or
  assets silently fail to resolve.
- `scripts/measure.sh <name> [viewer params]` does the whole run: launch, front the window for twenty seconds, pin
  the look (`LOOK=lx,ly,lz`, default the hall view from the spawn point), sample `stage.*` by forcing frames with
  screenshots, capture the lit image and `TARGET="<render target>"`, stop the viewer. `REBAKE=1` deletes the
  resources first. `OUT_DIR` defaults to `$TMPDIR/byte-engine-perf`.
- `scripts/pngdiff.py before.png after.png` diffs the first channel of two captures without an image library.
- `scripts/sample_metrics.py [host:port] [rounds] [shots]` is the forced-frame sampler on its own.
- To read the BESL or MSL of any shader without launching the viewer or instrumenting the engine, intercept it from a
  dry-run bake: `cd ~/development/sandbox/model_viewer && <engine>/target/debug/beld --source assets bake --dry-run
  --intercept '*.besl' --intercept '*.msl' <id> 2>/dev/null`, where `<id>` is a `.bema`, a `.glb`, or a
  `byte-engine/rendering/visibility/<pass>.pipeline`. Products are named `<shader ID>.besl` and `<shader ID>.msl`;
  `--intercept '*' --format json | jq -r '.intercepted[].name'` lists them. The MSL is what the Metal compiler
  receives, minus the precompiled prelude. Use it to check what a BESL edit lowered to before baking, measuring, or
  extracting a metallib. The `resources` skill in `plugins/byte-engine` has the details.
- `scripts/pipeline_statistics.m` prints a compute kernel's temporary registers, spills, and instruction counts from
  a baked metallib (build line in its header). To try a change without rebaking, extract the MSL with
  `xcrun metal-source --extract=raw -o=<dir> -f <metallib>`, edit it, and compile it with the command line that the
  extraction's `metal-options.txt` records, which includes the prelude's precompiled header, then run the probe on
  the result. Moving one call in the extracted resolve and recompiling took a minute and predicted the bake's 71
  registers exactly.
- By hand: `GET /metrics?presented=true`, `GET /metrics/frames?since=<tick>&presented=true` (NDJSON with `gpu`
  maps), `GET /screenshots?sink=0[&target=<name>][&previous=true][&format=raw]`, `POST /messages` with
  `{"type":"TriggerAction","target":1,"payload":{"type":"Vector3","value":[lx,ly,lz]}}` to pin the look,
  `GET /entities?name=Move` for the movement action. The inspector listens on `127.0.0.1:6680`. Startup parameters
  override as `--name=value`: `render.directional-shadows.resolution`, `render.ssgi.enabled`, `render.gtao.*`,
  `render.contact-shadows.*`, `kill-after`, `metrics-out`.
- `--resources.path=<dir>` gives a variant its own baked resources. A descriptor-set layout change needs a fresh
  directory; an edit to one `.besl` asset rebakes its pipeline on its own, so an A/B of one asset swaps the file
  between runs and keeps the directory.
- A baseline for asset changes: the viewer bakes from the `.besl` assets in the checkout and rebakes whenever their
  hashes change, so a binary built from HEAD must also run while HEAD's assets are in place. Save your changed and
  untracked files in a tarball, `git checkout HEAD --` the modified files and move the untracked ones aside, build
  and copy the binary out, run it with its own resources directory, then restore from the tarball. Never use
  `git stash`; the user keeps work there.
- GI quality comes from raw captures: `GET /screenshots?sink=0&format=raw&target=SSGI%20History` returns the float
  texels, and `curl -D` saves the `x-byte-engine-width`, `-height`, `-bytes-per-row`, and `-format` headers that
  decode them. To measure how fast the GI follows a change, turn the sun with `POST /messages`
  `{"type":"TransformationUpdate","target":<the DirectionalLight entity from GET /entities>,"payload":{"position":
  [0,0,0],"scale":[1,1,1],"orientation":[x,y,z,w]}}`, a quaternion that turns +Z onto the direction the light
  travels, and capture after 5, 15, 30, 60, and 120 forced frames.
- While the window presents, the shadow raster and material evaluation swing from frame to frame without the surfel
  passes following them (see "A presenting window loads some stages and not others" in `SKILL.md`), so neither is a
  regime key then. The plain medians of back-to-back A, B, A, B runs agreed instead (the spatial pass at 0.123 and
  0.123 against 0.115 and 0.111 ms). `scripts/binned_medians.py --match <stage>` shows whether a stage follows the
  key before you group by it.
- Useful pinned looks from the spawn point: `1,0,0` (hall), `-1,0,0` (back down the hall), `0.2,-0.5,1` (floor and
  courtyard side). The viewer integrates camera velocity with frame time, so walks never repeat.
- To study one kernel outside the engine, bake its pipeline without opening the viewer:
  `cd ~/development/sandbox/model_viewer && <engine>/target/debug/beld --source assets --destination <scratch dir>
  bake byte-engine/rendering/visibility/<pass>.pipeline`. The `*-raw` files that start with `MTLB` are metallibs
  (`strings <file> | grep <binding name>` finds the right one in the viewer's own `resources`), and
  `xcrun metal-source --extract=raw <file>` writes the MSL the engine compiled. A small Swift harness that renders the
  real input (Sponza's triangles from `assets/sponza/Sponza.gltf`, node scale 0.008, the viewer's sun), alternates two
  copies of it so the system level cache cannot serve repeats, and interleaves variants per round gives
  high-clock-state numbers within 1 %; a paced loop (four dispatches, then 14 ms idle) gives the low state.
- Sub-stage numbers without code: `xcrun xctrace record --template 'Metal System Trace' --attach <pid>
  --time-limit 3s --output mv.trace`, then export `metal-gpu-intervals` (rows per encoder, mapped to stages by order
  within a frame) and `gpu-performance-state-intervals` (the clock state per interval).
- The GPU has run in three states on one day (shadow-maps 0.64, 1.22, and 2.09 ms for the same raster). Discard
  pairs whose `stage.shadow-maps` differs by more than 1%, and never compare across states.
- `BINARY=<saved viewer>` runs a copy of the viewer saved from an earlier build, so two variants can be measured back
  to back without rebuilding; `ROUNDS=<n>` raises the forced-frame sampling. A run whose presented frame count equals
  the forced count ran hidden or behind a locked screen (`CGSSessionScreenIsLocked`): its SSGI history is invalid and
  the trace exits early, so it cannot measure the screen-space block.

Every screen-space pass with a multi-tap footprint now loads its tile into workgroup memory (GTAO evaluate, blur,
and upscale, SSGI trace and temporal, sun visibility, pixel mapping, material count and offset, both depth pyramids,
and the occlusion pyramid since 2026-10-05). The passes without it are ray marches, whose rays leave any tile
(contact-shadow trace, the SSGI march itself), per-thread or raster work (cascade fit, light clusters, skinning, the
mesh and fragment shaders), and the generated material shader, whose registers are the constraint.

Done since, 2026-10-05 evening: the sun visibility, GTAO, and SSGI stages record interleaved by rank under one
`stage.screen-space` counter, and the Metal tracker elides encoder barriers a logged barrier already orders. Scene
time moved 0.02 ms in the same state; the GPU did not overlap the full-screen dispatches. The per-pass counters it
replaced had cost nothing measurable, so counter overhead is off the list.

## Image A/Bs and their traps

How to prove a change pixel-identical in the model viewer, and how to measure in hidden or release runs.

### Determinism

- Presented frames never match bit for bit, even between two runs of one build. SSGI's noise scrolls with
  `frame_index % 64`, and its float16 history depends on every earlier frame. Compare per-channel means of raw
  captures instead; two runs of one build agree within about 0.03 %. At a pinned look the final image was
  deterministic run to run within one build, so 8-bit captures of two variants compare directly.
- `format=raw` without a target returns the final BGRAu8 image, not RGBA16F. Add `&target=Lit` for the HDR target.
- Forced frames on a minimized window (System Events `AXMinimized`) are deterministic. Each capture lands about 12
  ticks after the last, so SSGI has no history and its trace writes zeros by design. Hash 160 frames per run and
  compare the sets of distinct hashes. A `POST /screenshots` batch returns its captures from one frame, numbered in
  the `x-byte-engine-frame` header (lowercase).
- Back-to-back GET captures can land on consecutive frames, which makes SSGI history valid again. Keep the capture
  loop slow, or batch the captures with a POST.
- To give temporal passes their history on a locked screen, force frames from about 6 parallel clients looping
  `GET /screenshots?sink=0&format=raw` (raw skips the PNG encode). Each capture then follows 40 to 60 consecutive
  ticks; count them from `/metrics/frames?since=<tick>` rows whose `gpu` map has `screen-space`. One-at-a-time
  forcing leaves `SSGI History` all zeros, so a forced-frame SSGI timing taken that way measured an SSGI that did
  nothing.
- Bloom's intermediate captures (`Bloom Downsample 0`, `Bloom Upsample 1`) differ run to run with identical shaders.
  Only `Bloom Output` is bit-stable across hidden forced-frame runs, so judge by it and diff against a second run of
  the baseline.

### Assets and bakes

- Each viewer run re-bakes the resources whose sources changed. Give every variant its own asset snapshot and pass
  `--assets-path=<snapshot>` to both its bake and its runs: a directory of symlinks to
  `~/development/sandbox/model_viewer/assets/*` plus `byte-engine` pointing at a `cp -Rp` copy of
  `crates/byte-engine/assets` (`-p` keeps mtimes, so only changed shaders re-bake), and a `cp -R` of the resources
  directory per variant. Pair each snapshot with a binary built from the same Rust tree, because set layouts are
  reflected at build time. Without this, a BESL edit made mid-A/B pairs an old binary with new shaders and panics with
  "Missing descriptor set for layout".
- Any edit to `crates/resource-management`, even rustfmt, changes the resource-management signature. A binary with
  another signature deletes the whole resources directory at startup and re-bakes it during the run. Bake with the
  binary that reads the resources, and wait for the bake before capturing.
- Check what the bake compiled before trusting a shader A/B. The precompiled MSL prelude
  (`$TMPDIR/byte-engine-msl-prelude-<key>.pch`) is built lazily, and shaders compiled while it builds go without it.
  Extract each metallib's options with `xcrun metal-source --extract=raw`. Payloads ending in `-cpu-lz4` are raw LZ4
  blocks with no frame header; material-evaluation shaders are stored that way, engine shaders as `-raw`.

### Release runs

- The release `model-viewer` reads only a BELD-baked store. From `~/development/sandbox/model_viewer`, bake with
  `target/release/model-viewer-beld --source "$PWD/assets" --destination <store> --config <config.json> bake`, then
  run `model-viewer --inspector.port=<port> --resources.path=<store>`.
- GPU stage metrics need `--render.ghi.features.counters=true` in release; otherwise `/metrics` holds only CPU spans.
- On a locked screen, frames render only on capture requests and temporal histories reset. Use the parallel
  capture clients above.
- `target/` is managed by mbx, which deletes whole target directories when disk runs low. Copy each built binary out
  of `target/` right after building.
- `TriggerAction` on the `Lookaround` action with a Vector3 direction snaps the view.

### Diagnosis

- Find what lights a dark patch before blaming the pass you suspect. Surfel GI's "blotches" in Sponza's gallery were
  environment reflections leaking through a noisy hit fraction, not diffuse light: the lit image's 32 × 32 blocks
  correlated r = +0.88 with max(0, 1 − A) of raw `SSGI History` and −0.18 with its diffuse light. Capture raw
  `SSGI History` with the lit image, correlate the lit blocks against each channel, and compare two captures of one
  pinned look a few hundred frames apart for drift.
- Pick a regime key the change can't reach. The uber material evaluation A/B removed the prepasses before the
  receiver-fitted cascades, and `stage.shadow-maps` rose a steady 0.05 ms in every pair; `stage.screen-space` was
  the honest key.
- Per-dispatch GPU counters serialize the dispatches they wrap, so instrumented savings overstate the production
  total (−0.027 ms instrumented against −0.018 ms in production). Confirm with uninstrumented binaries.
- Every post-processing image is a storage image (Metal `shaderWrite`), so none is losslessly compressed. A
  compressed copy read about 30 % cheaper, but the copy cost more than the reads saved. Compression needs the writer
  to render (fragment or tile shader) into an image without `Storage`.
- A plain copy pass is memory-bound (about 68 GB/s); several pixels per thread don't speed it up.
- Never queue a measurement chain behind another with `pgrep -f <script>`: the waiting shell's own command line
  matches, the chains overlap and fight over the inspector port. Wait on the first chain's output file.
- To tell a scene or exposure problem from a grading bug, `POST /screenshots` one frame with
  `{"pass":"bloom","target":"Bloom Output","format":"raw"}` (RGBu11u11u10: R bits 0–10 and G 11–21 are 5e6m floats,
  B 22–31 is 5e5m) and the grade's output (`pass=aces2-color-grading`, target `ACES 2.0 Color Grading Output`).
  Average regions of the raw input, push them through `scripts/generate-aces2-tables.py` (import it;
  `OutputTransform.forward_ap0`, `LINEAR_SRGB_TO_ACESCG`) and the `.cube` trilinear (red fastest), and compare with the
  GPU's region means. Look strength 0 separates the LUT from the transform. ACES 2.0 desaturates anything near the
  display peak, so sunlit Sponza at EV100 ≤ 9.5 reads washed out, and surfel GI leaves scene values 0.01–0.1 black
  for the first frames after launch.

## What to try next, ranked

0. **Vulkan and DX12 barrier elision**, tracked in the repo's `todo.md` under runtime performance: the Metal tracker's
   logged-barrier elision has no counterpart there, so the interleaved screen-space passes pay a barrier per stage on
   those backends. Cannot be verified on this Mac.
1. **The `blend` push constant** makes the transparent shadow path live in every material shader; a generation-time
   choice would drop the directional resolve from opaque shaders. The user deferred this on 2026-10-05.
2. **Far cascades at half resolution**: about 0.5 ms across raster and pyramid, but the shadow array needs unequal
   layer footprints and the quality trade is visible.
3. **SSGI trace, hierarchical**: the trace is 0.79 ms of the 1.46; batching its fetches did not help, so its cost is
   not fetch latency. The linear depth pyramid has three levels (half, quarter, eighth of full resolution), too few
   for a hierarchical march over 96 half-res texels to skip much; it would need two more levels from the pyramid
   pass, and the result changes the image. Estimated at most 0.15 ms. The denoising stays as it is, by the user's
   instruction.
4. **One continuous-spacing tent** instead of two blended ones in the sun pass: a visual trade worth under 0.02 ms
   now that the spacing-one tent gathers.
5. **Reduce the cascade cells inside the shadow raster with a Metal tile shader**: a standalone prototype on Sponza
   (the shadow fragment stage copies depth into a memoryless R16 color attachment, a tile function reduces each
   32x32 tile's cells) cost 0.036 ms over the raster, against 0.06 to 0.10 ms for a separate pyramid pass in the same
   loop. It needs tile pipelines and memoryless attachments in GHI and BESL, a fragment stage on every directional
   shadow pipeline (the opaque ones have none), a one-step conservative margin (0.01 % of cells rounded one 16-bit
   step off the stored depth), proof that tile dispatches work in the layered cascade pass, and the compute pass
   kept for Vulkan and DX12. Parked on 2026-10-06.

## Where the code is

- Pass orchestration and stage counters: `crates/byte-engine/src/rendering/pipelines/visibility/render_pass.rs`
  (`StageCounters`, `record_compute_stages`, the `unordered` material group); settings and `ShadowWork` in
  `manager.rs`.
- Passes: `render_pass/sun_visibility.rs` (contact trace plus shadow resolve), `render_pass/shadows.rs` (cascade
  raster, both pyramids, cascade fit), `render_pass/ssgi.rs`, `render_pass/gtao.rs`, `render_pass/materials.rs`.
- Shader assets: `crates/byte-engine/assets/rendering/visibility/*.besl` with `.bead` (stage, workgroup) and
  `.pipeline` beside them: `sun-visibility.besl`, `directional-shadow-depth-pyramid.besl`, `ssgi-trace.besl`,
  `ssgi-temporal.besl`, `directional-shadow-receiver-bounds.besl`, `directional-shadow-cascade-fit.besl`,
  `pixel-mapping.besl`.
- Generated material shader: `pipelines/visibility/shader_generator.rs` (helper list in dependency order) and
  `shader_generator/sources/*.rs` (`shadow.rs`, `material.rs`, `ssgi.rs`); VM tests in `shader_generator/tests.rs`
  (`run_helper_test`), pass VM tests in `pipelines/visibility/tests.rs` (`run_sun_visibility`, `sun_shadow_map`,
  `visibility_assets_lower_to_the_platform_shader_language`).
- BESL: intrinsics in `crates/besl/src/lexer/ast.rs`, VM in `crates/besl/src/vm/`, platform backends in
  `crates/resource-management/src/shader/besl/backends/{msl,hlsl,glsl}` with the shared `NodeEmitter` trait in
  `crates/resource-management/src/shader/generator.rs`.
- Inspector HTTP: `crates/byte-engine/src/inspector/http.rs`. GPU counters: `crates/ghi/src/metal/command_buffer.rs`.
- Surfel GI: `render_pass/surfel_gi.rs` (the pipeline list, the stages and their thread counts), the passes in
  `assets/rendering/visibility/surfel-*.besl` and their shared layers in `layouts/surfel-*.besl`, the layer sets in
  `pipelines/visibility/set_layouts.rs`, and the VM tests in `pipelines/visibility/tests/surfel_gi.rs`, which build
  shaders with the helpers in `rendering/shader_vm_test.rs` (`asset!`, `asset_source!`, `stacked_asset`,
  `stacked_on_layers_of`).
- `sun-visibility.besl` is a copy of the generator's directional shadow helpers plus the umbra probe and the gather
  paths; both headers say so. Keep the two in sync when the shared parts change.

## BESL rules that bite

- Declare a helper before every function that calls it; the asset otherwise fails to lex with
  `FunctionCallParametersDoNotMatchFunctionParameters` and no name. Bisect by truncating the helper list.
- `floor` is scalar only, `round` has `vec2f`; `abs` and `step` are scalar or `vec2f`; no signed integers; no
  includes. Bind a `mat4x3f * vec4f` product to a typed `let` before passing it as `vec3f`. `f32[4]` arrays index
  with a `u32` variable; `texture_size` returns mip zero's extent.
- A pass's own descriptor set uses bindings 1033 and up; the base set holds 0 to 9 (`views` at 0).
- `gather(texture, uv[, layer])` returns the first channel of the quad at mip zero. Texture parameters carry their
  sampler on Metal and HLSL since 2026-10-05, so `sample`, `gather`, and `downsample_*` work inside helpers. Do not
  name a parameter `texture`; HLSL and GLSL reserve it and escape it to `besl_texture`. An image bound with a
  reduction sampler gets a second binding with a plain sampler for gathers; the VM binds a `Texture` once, so the
  harness builds the fixture twice.
- Test a new or copied shader through `visibility_assets_lower_to_the_platform_shader_language` and its VM test
  before launching. Backend changes get a test that compiles through the real platform compiler
  (`compile_natively` for MSL).
- Prefer copying shader code between the pass and the generator over adding generator features, and say so in both
  headers. Code added to the generator's material shader is live in every material.
- `subgroup_shuffle_xor_f32(value, mask)` reads the lane whose index is the caller's XOR `mask`. Every lane must pass
  the same mask, and the VM rejects divergent masks and inactive source lanes. Put the lanes that exchange values on
  consecutive local indices aligned to their count and shuffle with masks below it, so any subgroup of four or more
  holds them. Metal compute threadgroups linearize row-major (lane = x + width * y), but tile-function threads are
  quad-ordered (lanes 0 to 3 are a 2x2), so a mask chosen from 2D thread positions differs between the two.
- Subgroup collectives go in `main`: the VM rejects one inside a called function ("Subgroup collectives inside called
  functions cannot participate in workgroup rendezvous"). Every lane of the subgroup must reach each collective, so no
  lane may return before the last one (`DivergentSubgroupCollective`); carry flags such as `alive` and let a dead
  lane run its loops zero times.
- `subgroup_broadcast_f32` and `subgroup_broadcast_u32` take one source lane for the whole subgroup
  (`DivergentSubgroupBroadcastLane` otherwise). To hand each aligned group of four its first lane's value, let the
  other lanes hold zero and add: `let pair: f32 = v + subgroup_shuffle_xor_f32(v, 1);`, then
  `pair + subgroup_shuffle_xor_f32(pair, 2)`, as `surfel-count.besl` does; when the others hold values of their own,
  shuffle on every lane and select by `lane & 1`, then `lane & 2`. Shuffles carry `f32` only, so move counts through
  `f32`, which holds integers exactly below 2^24.
- `&&` and `||` short-circuit, in the VM and in the statements the lexer hoists out of the right side, so
  `lane == 0 && f()` runs `f` only on lane zero.
- A layout layer whose helpers call another layer's must list that layer before it in its `layout!` set in
  `set_layouts.rs`, or `every_layout_layer_declares_a_descriptor_set` fails to reflect it with `UndeclaredName`.
- A layer has no `main`, so test its helpers through `stacked_on_layers_of(asset, stand_in)`, which stacks a stand-in
  `main` on the layers the asset uses; give the stand-in's own bindings numbers no layer takes.

## Working agreement with the user

Analyze means discuss first. Stage the work in steps, surface decisions that change the result or the scope, stop after
each step with regime-matched numbers and an image diff, and never commit; the user commits. Flag HLSL, GLSL, Vulkan,
and DX12 paths as unverified, since only macOS runs here. Never use `git stash`; the user keeps work there. Format only
files you changed, with `rustfmt --edition 2024 --unstable-features --skip-children <file>`; `core/message_observer.rs`
and `visibility/render_pass/gtao.rs` carry pre-existing drift and must not be formatted. Tests run with
`cargo nextest run --workspace`. Add an Unreleased bullet to `CHANGELOG.md` for every user-visible change, with the
measured numbers.
