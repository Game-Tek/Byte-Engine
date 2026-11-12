---
name: visibility-perf
description: "Cut GPU frame time in a real-time renderer: how to measure so a number means something, and the shader and pass techniques that reduced run time in practice (per-stage counters, hoisting work out of low-occupancy kernels, batching fetches, conservative pyramid probes, gathers, grid-anchored tent filters, half-resolution tracing with edge-aware upsampling, pass fusion, clear removal, read-once reductions, several reads per thread combined with subgroup shuffles, a thread's chain of atomics split across lanes). Use when the user asks what is slow in a render pipeline, why frame time rose after a change, where to start optimizing, how to A/B a rendering change, or to continue frame-time work; for Byte-Engine's visibility renderer also read reference/byte-engine.md. Not for gameplay CPU work, asset-bake, or startup performance."
---

## Method

1. **Attribute before you optimize.** Put a GPU timestamp counter around every pass and read the per-stage times
   in-engine. Estimates from reading shaders were wrong by 2x in both directions; the counters were right.
   Engines often ship with GPU counters off: confirm the first sample of a run reports per-stage GPU times before
   you start timing, not after a batch of runs that measured only CPU spans.
2. **Pick the technique by what bounds the stage.** Latency-bound (few threads, dependent fetches): hoist, batch,
   fuse, gather. Bandwidth-bound (cost tracks bytes read): shrink the data, read it once, skip it with a probe.
   Fixed-overhead-bound (tiny dispatches, clears, encoder switches): remove passes, overlap them. Before you call a
   pass bandwidth-bound, divide the bytes it reads by its time and compare that with a plain buffer read of the same
   size: render targets are usually stored losslessly compressed, so a cost that scales with the texel count can
   be thread count, not bytes. See "Check compression before blaming bandwidth" under measurement traps.
3. **Change one thing, prove it exact, measure in one clock state.** An optimization that claims the same image
   must show zero differing texels on the intermediate target it writes, at more than one pinned view. Compare
   stage times only between runs in the same GPU performance state, back to back, against the same engine HEAD.
4. **Stop after each step with the numbers**, before and after, and the image diff. Small exact steps compound;
   a large speculative step that moves two stages at once cannot be attributed when it disappoints.

## Techniques that reduced run time

Each technique gives the rule, how to recognize the opportunity, how to do it, how to verify it, and the evidence
from the Byte-Engine visibility renderer (Sponza, 1024x1024, Apple M4) where it was applied. Ordered by payoff.

### Instrument every pass with GPU counters

**Rule.** Per-pass GPU time, exposed at runtime, is the only attribution that survives clock changes and hidden
windows. **Recognize.** You are about to guess which stage is slow. **Do.** Wrap each pass's commands in a
start/end timestamp pair, publish `stage.<name>` next to the frame time, and keep the counters cheap enough to stay
on. Add temporary sub-counters when one stage covers two kernels. **Verify.** The stages sum to the scene time
within a few percent; the remainder is gaps between passes. **Evidence.** The first breakdown put contact shadows
at 1.25 ms where the shader-reading estimate said half that, and the shadow raster at 2.94 ms; every later win
was chosen from that table.

### Hoist latency-bound work out of low-occupancy kernels

**Rule.** A kernel with a large register footprint runs few threads, so nothing hides its fetch latency; work that
needs many dependent fetches is cheaper in a dedicated kernel with high occupancy, even at the same instruction
count. **Recognize.** A sampling helper inside a fat shader (material evaluation, a megakernel) costs more than its
fetch count suggests, or the shader's cost grows with features that most pixels do not use. **Do.** Move the work to
its own full-screen pass that writes one small image, and have the fat kernel fetch that image once. Rebuild
whatever the pass needs (position, normal, receiver plane) from depth and the G-buffer. **Verify.** The fat kernel
drops by more than the new pass costs; the output image is identical. **Evidence.** Moving the directional shadow
resolve out of material evaluation cut it from 3.33 to 1.80 ms; the resolve cost 1.28 ms as a pass against 1.47 ms
inline, and every later shadow optimization then acted on a 2.3 ms stage with its own counter.

### Keep dead code out of fat kernels

**Rule.** Register allocation is the maximum over all paths, so code a pixel never executes still taxes every pixel
when the branch that skips it is a runtime value. **Recognize.** A uniform flag (blend mode, feature toggle) selects
between paths inside one shader. **Do.** Specialize at generation or compile time, or put the rare path in another
pass or another pipeline variant. Never add a new path to the fat kernel for a rare case. **Verify.** Occupancy or
the fat kernel's stage time, measured with the rare path present and absent. **Evidence.** Adding a gather-based
tent to the transparent-only shadow path of the material shader cost opaque material evaluation 0.027 ms though no
opaque pixel ran it; the same code in the dedicated pass was free.

### Batch independent fetches ahead of the math that consumes them

**Rule.** Texture reads overlap only while they are in flight together; a read, a branch, then more reads serialize.
**Recognize.** A helper written as fetch, early-out, fetch, weight, especially inside a low-occupancy kernel.
**Do.** Read every texel the helper may need into locals or small arrays first, then compute weights and early-outs
in a second loop. **Verify.** Same output, lower stage time. **Evidence.** Folding a four-texel edge-aware upsample
into material evaluation first lost 0.70 ms: twelve fetches per pixel serialized behind an early-out. Fetching all
twelve first turned the same fold into a 0.2 to 0.4 ms net win. In the surfel GI passes, issuing the four reads of
each of the temporal accumulation's four bilinear history taps before weighing any took it from 0.230 to 0.192 ms,
and reading each spatial-filter tap's depth, weights, history, and normal before its validity tests took that pass
from 0.123 to 0.113 ms (A, B, A, B, both pairs agreeing). **Watch.** Batch only fetches that will all be consumed.
Prefetching an SSGI ray march four steps at a time made the trace 0.08 ms slower, because most rays end within a few
steps and the rest of each batch was wasted bandwidth. Batching costs registers, so it can lose in a kernel that
registers already bound: reading four cell-list entries, then the four records they name, before weighing any, made
the register-heavy surfel resolve about 10 % slower (0.98 to 1.09 ms), and it was reverted. A read that becomes
unconditional must be harmless when its result is dropped: clamp an out-of-image tap's coordinate for the read and
drop the tap with a flag; derive each bilinear tap's coordinate from its own clamp, because one plus a clamped base
lands on the wrong texel at the image's edge; and add a texel only where its weight is positive, because a zero
weight times a NaN that an unused texel held is still NaN.

### Prove the common case with conservative probes, filter only the rest

**Rule.** When most pixels have a trivial answer, a few samples of a conservative summary of the data decide them
and only the rest pays for the full filter. **Recognize.** An expensive per-pixel filter whose result is fully one
value over most of the screen (fully lit, fully shadowed, fully occluded, no hit). **Do.** Build a min/max pyramid
or similar bounds over the source; sample it at the footprint's corners so the samples cover every texel the filter
could read; prove the whole footprint one way or the other with a margin that keeps a sloped receiver from proving
itself. **Verify.** The probe never changes a pixel the full filter would have resolved differently; image diff is
zero. **Evidence.** Four max-reduction samples prove a receiver fully lit, four min-reduction samples prove it in the
umbra: sun visibility 2.32 to 1.23 ms, frame 8.92 to 8.04 ms, at a pyramid cost of 0.30 to 0.52 ms.

### Read once, reduce many

**Rule.** When two passes or two branches reduce the same texels, read them once and write both results.
**Recognize.** A second reduction (min next to max, average next to variance) would re-read a large source, or a
small pass re-reads a full-screen image another pass just reduced. **Do.** Compute both reductions from the same
loaded quad or tile in one pass; fold a whole-screen reduction into the pass that already walks the source, and reset
its accumulator in the consumer instead of clearing it with a blit. **Verify.** The second output costs only its
write and its arithmetic; the frame loses the removed dispatch and encoder boundaries too. **Evidence.** The cascade
pyramid's minimum channel cost 0.13 ms over a max-only build where a second pass would have re-read 34 MB of D16.
Folding the shadow receiver-bounds reduction into the depth pyramid pass took the two from 0.21 to 0.14 ms and the
frame from 8.07 to 7.83 ms, most of it from the dropped clear, dispatch, and encoder splits. **Watch.** When the
host pass has many small workgroups, do not let every thread hit the same shared atomics: 256 threads on six
addresses cost more than the pass saved. Write per-thread results to plain shared memory, let a few lanes combine
them, and merge once; keep array indices static so the arrays stay in registers; and keep the barrier between
zeroing shared accumulators and the first merge, which the BESL VM flagged as a race before the GPU could.

### Keep several reads in flight per thread; combine lanes with shuffles, not barriers

**Rule.** A reduction that gives each thread one read, then synchronizes a small workgroup and lets a few lanes
finish, is bound by its workgroup launches, its barrier, and its serial tail, not by the bytes it reads. Give each
thread several independent reads, reduce them in registers, and combine the few threads that share an output with
subgroup shuffles. **Recognize.** Thousands of tiny workgroups, one load per thread, a barrier, then an
`if (lane < k)` loop; or a pass that costs the same when its input is all clear values. **Do.** Give each output's
threads consecutive local indices that start at a multiple of their count, so they share a subgroup on every GPU; issue
every read before reducing; shuffle with masks below the count; let one of them write. **Verify.** Zero differing
texels on the output, and a VM test whose fixture puts each output's extremes in different threads' reads, so it
fails without the shuffles. **Evidence.** The directional shadow pyramid, four gathers per thread and two
`subgroup_shuffle_xor_f32` calls per cell instead of one gather per thread, a barrier, and two lanes reducing
sixteen values: at the default 4096-texel cascades, in screenshot-forced low-clock frames, 1.97 to 0.58 ms and the
frame 10.41 to 9.08 ms, with the shadow raster, the regime key, within 1.2 %; at 2048, 0.41 to about 0.1 ms and the
scene 6.02 to 5.69 ms. The `Sun Visibility` target stayed bit-identical, and the engine's baked kernels wrote
identical cells on Sponza's shadow content in a standalone replica, where they took 0.259 and 0.109 ms at the high
clock state (1.068 and 0.417 ms at 4096). In that replica, one cell per thread with sixteen gathers and no
cross-lane traffic reached 0.124 ms, per-texel reads instead of gathers took 0.64 ms, and running either kernel
alongside an ALU-bound dispatch gained nothing (0.6 to 3.5 %). **Watch.** At about 0.1 ms the pass is short enough
that single frames of its stage counter misattribute time (minimums of 0.008 ms appeared); judge it by medians and
by the scene total.

### Split a thread's chain of atomics across lanes

**Rule.** Apple GPUs wait for a thread's outstanding memory operations, atomics included, at every branch, so a loop
that issues an atomic per iteration pays a round trip per iteration however many other threads run beside it. Give
each item several threads that each take every n-th iteration, and the chain each thread waits through is n times
shorter. **Recognize.** A pass with one thread per item (a particle, a surfel, a light) whose inner loop issues an
atomic or a dependent read for every cell it visits, and whose time follows the longest walk, not the bytes.
**Do.** Dispatch n threads per item on consecutive lanes aligned to n, and stride the walk by lane. Let the first lane
do the work that must happen once per item and hand its verdict to the others with xor shuffles; add up per-lane
partial results, such as a histogram that picks a cut-off, with xor shuffles, so every lane makes the same decision;
and keep every lane running to the last shuffle, with flags instead of early returns. **Verify.** A VM test of the
output that does not depend on the order the atomics land in, and the stage time. **Evidence.** The surfel grid's
count and scatter passes at four lanes per surfel, in the fast clock state: the grid stage 0.31 to 0.16 ms, count
0.090 to 0.041 ms, and scatter 0.105 to 0.039 ms, which put the grid below its 0.20 ms from before the kajiya review
changes. **Watch.** Only four lanes were measured. Every lane repeats the item's own reads, so a wider split pays
more setup for a shorter chain.

### Trace at lower resolution, rebuild with an edge-aware upsample

**Rule.** Ray marching costs per pixel per step, so marching at half resolution quarters it, and a joint-bilateral
upsample that weights source texels by distance and by the pixel's own surface keeps edges sharp. **Recognize.** Any
screen-space trace (contact shadows, SSGI, SSR, AO) at full resolution. **Do.** March a half-resolution linear-depth
image you already have (a depth pyramid's mip zero); upsample with weights from the pixel's tangent plane and
normal, not depth alone, because touching surfaces share a depth. **Verify.** No bleeding across depth edges in the
target; stage time. **Evidence.** The contact-shadow trace went from 1.75 to 0.43 ms and the filter plus upsample
from 0.73 to 0.67 ms.

### Expose a resolution knob for stages whose cost tracks texel count

**Rule.** When a stage's cost tracks a texture's texel count, its resolution is the cheapest lever, and it should be
a parameter so the quality trade stays visible. **Recognize.** Raster, pyramid, or blur cost scales with a map size
that was picked once and never revisited. **Do.** Add the parameter, measure the stages it feeds, look at the
result. **Evidence.** Shadow cascades at 2048 instead of 4096: raster 2.94 to 1.22 ms, pyramid 1.30 to 0.30 ms,
visually identical at interior views because the cascades are fitted to visible receivers.

### Fuse passes over the same pixels, after batching

**Rule.** Two passes over the same pixels can be one, removing an intermediate image, a dispatch, and an encoder
boundary, provided the folded work issues its fetches up front. **Recognize.** A small pass writes a full-resolution
image that exactly one later pass reads once per pixel. **Do.** Move the small pass's body into the consumer as a
helper; batch its fetches; delete the image and its bindings. **Verify.** A stand-in body with one fetch costs
nothing, so carrying code and bindings is free; measure the real body. **Evidence.** The contact-shadow filter fused
with the shadow resolve; the SSGI upscale folded into material evaluation for a 0.2 to 0.4 ms win once batched. The
surfel temporal accumulation became a helper the surfel resolve calls: 0.642 plus 0.192 ms became 0.726 ms, and the
screen-space block went from 1.889 to 1.797 ms, by dropping a dispatch, four reads per pixel of what the resolve had
just written, and two full-screen images. **Watch.** The folded body adds to the host's register peak wherever the
host's own values are still live: the fused resolve went from 68 to 81 registers a thread. Moving the history blend
behind the resolve's spawn votes, where fewer of those values were live, took it to 71, but two A/B pairs disagreed in
sign (-6 % and +4 %), so it was reverted: a lower static count is not a result until the stage time moves.

### Let an existing full-screen pass write the default instead of clearing

**Rule.** A clear is a pass with its own encoder; a pass that already visits every pixel can write the clear value
where nothing else will. **Recognize.** A clear precedes a prepass or mapping pass that touches every pixel.
**Do.** Write the default at pixels the pass leaves uncovered; make sure that covers every pixel the clear did,
including modes without a sky or horizon. **Evidence.** Frame 12.67 to 12.46 ms, and one fewer encoder split.

### Gather texel quads instead of fetching texels

**Rule.** A gather returns the four texels around a corner in one instruction; a filter whose taps are adjacent
texels reads a 4x4 block with four gathers instead of sixteen fetches, exactly. **Recognize.** A fixed-footprint
filter, a blocker search, or a pyramid build over adjacent texels in a latency-bound kernel. **Do.** Address the
corner shared by each 2x2 quad; use the platform's gather order (x, y: second row left and right; z, w: first row
right and left); keep the per-texel compare and weights. **Verify.** Zero differing texels on the target.
**Watch.** Gather reads mip zero and one channel; it needs a sampler, so function parameters that carry textures
must carry one; whether a min/max reduction sampler applies to gathers is platform-specific, so bind such an image
a second time with a plain sampler for gathers; a kernel that issues one read per thread and then synchronizes
gains little, because its shape bounds it, not its texture instructions. **Evidence.** Blocker search 1.20 to
1.14 ms; tent filter 0.02 ms; pyramid build 0.52 to 0.485 ms, little because that kernel gave each thread one gather
followed by a barrier and a serial two-lane tail (see "Keep several reads in flight per thread").

### Grid-anchored tent filters, width from spacing rather than tap count

**Rule.** A 4x4 tent anchored to the texel grid with per-axis weights 1 - f, 2 - f, 1 + f, f sums to four, so it
needs no renormalization, produces smooth noise-free ramps, and stays still frame to frame when the map snaps to
texels; penumbra width comes from tap spacing, so cost is constant for any width. **Recognize.** Poisson or random
taps that need temporal filtering, or a filter whose cost grows with its radius. **Do.** Blend the two nearest
spacings for continuous width; at spacing one the taps are adjacent, so they gather. **Evidence.** The directional
shadow filter; a single continuous-spacing tent would halve the blended case and is still untried.

### Evaluate linear quantities incrementally inside tap loops

**Rule.** A plane's depth at each tap is the first tap's depth plus whole steps of the gradient, and per-axis weights
are computed once and multiplied, not recomputed per tap. **Recognize.** A dot product or a weight expression inside
the inner loop whose inputs change linearly. **Evidence.** Free, and what keeps the gather paths readable.

### Hoist uniform work to a few lanes, and expect little

**Rule.** Values that depend only on per-frame constants (a cascade's texels per meter, a projection's scale) can be
computed by four lanes into workgroup memory while the workgroup's tile loads, behind a barrier it already pays for,
and a receiver can be projected once per cascade for every stage that needs it. **Evidence.** Exact, free of
barriers, and worth 0.004 ms of a 1.15 ms pass: in a latency-bound kernel, uniform ALU already hides under the
fetches. Do it for structure, not for speed, and measure before promising more.

### Store derived data no wider than its source

**Rule.** A reduction of fixed-point values is one of those values, so it fits the source's format exactly; a wider
float format buys nothing but bandwidth and memory. **Recognize.** A min/max pyramid or a copy of a 16-bit or 8-bit
image kept in 32-bit float. **Do.** Use the source's unorm format for the derived image and keep every comparison in
float, which the texture unit converts exactly. **Verify.** The consumer's output is byte-identical. **Evidence.**
The cascade depth pyramids moved from R32F to R16UNORM: the target is byte-identical, the images are half the size,
and the pass time did not move, because the kernel's workgroup shape bounded it, not the cells it wrote. **When the
source is wider than the store**, as 32-bit depth into a 16-bit float occlusion pyramid, round toward the safe side
in the shader before the store, so the hardware's round-to-nearest never bites: a culling pyramid rounds toward the
far plane, so it never claims a surface nearer than the one drawn. Without bit casts, `f32(f16(v))` and, when that
is above `v`, scaling by 1023/1024 and converting again lands one or two 16-bit steps below, which the VM test pins.

### Load a tile into workgroup memory for wide filters

**Rule.** When neighboring pixels read overlapping footprints, the workgroup loads its tile once into shared memory
and the taps read that. **Recognize.** A 5x5 or wider filter at full resolution. **Evidence.** The fused contact
filter reads 25 taps per pixel from a shared tile of trace and depth texels.

### Overlap independent passes, and expect the hardware to decide

**Rule.** Independent dispatches recorded without barriers between them may run concurrently, but only fill idle
lanes the first dispatch leaves: a full-screen dispatch of a quarter million threads or more already saturates a
ten-core GPU, so a second one mostly queues behind it. **Recognize.** Several screen-space passes after the same
input, serial. **Do.** Record their first stages together and their second stages together, and make the resource
tracker elide a barrier a global encoder barrier already orders, so the block encodes three barriers instead of
seven; a per-stage timestamp serializes the block, so time it as one. **Verify.** Scene time in one GPU state, and
an identical intermediate target. **Evidence.** Measured on the M4 for the contact trace, GTAO, and SSGI stages:
the 2.6 ms block gained 0.02 ms, within noise, with the target byte-identical. The barrier elision and the
reordering are kept because they cost nothing and remove work; the overlap itself did not materialize. The three
per-pass counters cost nothing measurable either (two warm runs, 2.25 against 2.24 ms), so counters are cheap to
keep on.

## Measurement traps

- **Paced frame times lie about work.** The GPU's performance state swings the same work by 1.6x to 1.8x, and
  vsync-paced frames idle 25 to 40% per refresh, so a reduction of X ms shows as about 1.5X in paced milliseconds and
  never linearly. Use a stage nothing in the change touches as the regime key (a pure raster pass works) and compare
  only runs where it matches within 1%.
- **Run unattended.** Start the viewer with `--unattended=<hz>` (`scripts/measure.sh` does). Every tick then renders
  at that rate in the background, without fronting the window, while the user works or the screen is locked, and the
  clock steps by exactly one frame period, so runs repeat. An attended window behind another or behind a locked screen
  renders nothing and reports meaningless times: frames forced one at a time through it give temporal passes no valid
  history, so they skip their work, and a cold run showed the SSGI trace at a third of its real cost. If a run's
  presented frames equal the captures you sent, it ran attended; start it again with `--unattended`.
- **Prebuild the binaries of an A/B** and save them aside, so the runs can go back to back when the machine is
  available, without a build between them that lets the GPU state or the screen lock change.
- **Walked cameras never repeat** when movement integrates velocity with frame time. Pin the look direction from a
  fixed spawn point and capture several looks.
- **The environment moves.** When a stage you did not touch changes, measure HEAD again before believing the delta.
- **Swapping a sampler mode changed nothing; a mid-run clock change changed everything.** Both looked like results
  until the regime key caught them.
- **One run can straddle two clock states**, and its median then lands between them and matches neither (material
  evaluation's run median of 1.07 ms from frames at 0.85 and 1.18 ms). `scripts/binned_medians.py` groups each run's
  frames by the regime key, and `--at` lines up the same states across runs; compare like groups.
- **A presenting window loads some stages and not others.** When the window presents and the CPU runs ahead, stages
  can absorb other work from frame to frame. In one run, visibility-early read 0.03 or 0.24 ms, the depth pyramid
  0.16 or 0.95 ms, the shadow raster 1.04 to 2.35 ms, and material evaluation 0.62 to 1.58 ms (10th to 90th
  percentile), while the surfel passes held within about 15 % and followed neither (correlation -0.13 and 0.04 with
  the surfel resolve). Neither was a regime key there. Before binning by a key, check that an unchanged stage follows
  it; when none does, the clock most likely held still, and the plain medians of back-to-back runs compare.
- **Small passes drift between runs by more than a small change moves them.** Relative to the regime key, an
  unchanged 0.11 ms surfel pass ranged over 30 % across five runs of one day, because a temporal pass's work depends
  on the population it has converged to. For an effect under about 10 % of a pass, run A, B, A, B back to back from
  the same baked resources, believe it only when both pairs agree, and keep an unchanged stage beside the changed one
  as a control.
- **A stage counter that covers two kernels hides which one costs.** The SSGI counter read 1.46 ms; a temporary split
  showed 0.79 ms of trace and 0.67 ms of denoise, which changed what was worth doing. Split before designing.
- **Each precise timestamp may split a compute encoder**; thirteen stage boundaries are themselves part of the gaps
  between passes. Measure with the counters off once.
- **When the GPU idles, its pass spans describe the CPU.** A Metal System Trace that shows the GPU busy for a few
  percent of the recording means the frame is bound by command recording, and per-pass timestamp spans then include
  gaps and overlap, so a long raster span can be a pass made of many tiny draws. Read the CPU span that records and submits (`Renderer::queue_execute` in Byte-Engine) beside the GPU spans, and
  count the draws each pass records, before profiling shaders.
- **A run that suddenly got fast may draw less.** A store with stale or missing shaders, a material that failed to
  load, or a scene that hasn't streamed in renders a near-empty frame at a fraction of the cost. Read the run's log
  for load warnings and look at the lit image before believing a number.
- **Debug builds inflate CPU-side culling and sorting** far more than GPU work, because the Rust runs unoptimized.
  A debug frame finds where the time goes; confirm a CPU-side span's size in a release build before optimizing it
  further.
- **Check compression before blaming bandwidth.** GPUs store private render targets losslessly compressed by
  default (Apple does for depth and color attachments without shader-write usage), so a pass can read far fewer
  bytes than its texel count. Divide the bytes the pass reads by its time and compare that with a plain buffer read
  of the same size: a pass above that roofline reads compressed data and is bound by its threads, not by memory.
  The shadow depth pyramid was called bandwidth-bound for a week because its cost scaled with the cascade texel
  count, which fit both explanations. Its kernel read the 32 MiB D16 array at 127 GB/s against a 97 GB/s
  buffer-read roofline on the M4 and took the same time on a cleared map; only hashed, incompressible depth, or a
  texture made without GPU-optimized contents, brought every kernel variant down to the roofline (0.34 ms).

## Verification recipe

- Image-diff the intermediate target the change writes, before and after, at two or three pinned views; exact
  changes show zero differing texels, and the lit image differs only by temporal noise.
- Unit-test shader helpers in a CPU interpreter or VM with analytic fixtures (an edge gives 0.5 on the edge and
  0.75 one quarter in; a sloped receiver never shadows itself) so a rewrite is pinned before the GPU sees it.
- Compile the generated or copied shader through the real platform compiler in a test, not only through the
  language front end; the runtime compile error is the slowest place to find it.
- Run A, A, B, B back to back against the same HEAD, and report medians with the regime key beside them. To build a
  baseline, save your own changed files in a tarball and check out HEAD's versions; never use `git stash`, which can
  hold the user's own work.

## Applying this to Byte-Engine

Read `reference/byte-engine.md` in this skill for the current per-stage breakdown, the inspector endpoints and
parameters, the measurement scripts in `scripts/`, the file map, the BESL rules, the ranked next steps, and the
working agreement with the user.
