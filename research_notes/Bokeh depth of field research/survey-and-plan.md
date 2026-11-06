# Bokeh depth of field: survey and plan for Byte-Engine

Prepared 2026-11-02 for Byte-Engine (Metal on an Apple M4 Mac mini first; Vulkan/DX12 also present). Survey and proposal only, no code.

Revised the same day. Methods that split the scene into discrete planes (near/far layers, depth slices, and a sharp-to-blurred crossfade) are no longer recommended, because their seams show in practice. The core is now a single continuous gather (B3). B2 says what goes wrong with layers.

## How to read this

- **Status** on each source says how it was read this session:
  - *Read*: full text or code.
  - *Partial*: abstract, product docs, or a talk's landing page only.
  - *Memory*: from prior knowledge of the talk or paper; not re-read this session. Check before quoting numbers.
- **(derived)** marks my own arithmetic, inference, or design. It is not a claim any source makes. B3 is mostly derived; treat it as a design to prototype and check against the reference (B8), not as shipped practice.
- Part A is the engine's current state. Part B is the techniques. Part C is the recommendation. Part D is what could not be verified.

---

## Part A. What the engine has today

| Fact | Where | What it means for bokeh |
| --- | --- | --- |
| `Camera` stores `aperture` and `focus_distance`, but nothing reads them. The reference page says "Stored but not read". | `crates/byte-engine/src/rendering/camera.rs`, `docs/reference/rendering.mdx` (Camera table) | The API slot exists. The Reference row must be written first (per the "reference docs are the spec" rule), with units: the field has no unit today. |
| The camera has a vertical field of view but no focal length or sensor size. | `camera.rs` | A thin-lens model needs a focal length. It follows from the field of view once a sensor height exists (see B1). |
| Depth is reverse-Z with finite planes, near 0.1 m and far 100 m, fixed. `projection_matrix` writes z_ndc = α + β/z with α = −n/(f−n), β = nf/(f−n). The sky is where depth is 0. | `crates/math/src/lib.rs:244`, Reference "Atmosphere sky" row | The circle of confusion (CoC) is an affine function of device depth (derived, B1). Two constants per frame, no linearization in the shader. |
| Scene color is `RGBu11u11u10`, linear HDR, no alpha. | Reference "Objects" | The CoC needs its own target (R16F, signed). |
| Post-scene passes declare full-size, divisor-scaled, persistent, and history targets. Passes are "tile" (per-pixel, keeps `main` losslessly compressed) or compute (`simple_compute`, used by bloom). | `render_pass.rs`, `render_passes/bloom.rs`, `motion-blur.pipeline` | A gather, a CoC pyramid, tile radii, and a persistent autofocus value all fit the existing pass model. The gather needs neighbors, so it is compute, not tile. |
| BESL has shared memory, barriers, subgroup operations (ballot, lane index, size), `gather`, and atomics. | `crates/besl/src` | Tile CoC maxima can be subgroup reductions. A highlight scatter can append with atomics. |
| There is no temporal antialiasing. Antialiasing is SMAA 1x. | Reference "Passes" | **Hard constraint.** Unreal's and most modern DOF lean on TAA to hide gather noise and smooth layer seams. Here the gather must be stable and seamless on its own: a fixed sample lattice, not per-frame jitter. |
| Motion blur, bloom, lens flare, height fog, and tone mappers exist as post-scene passes. Lens flare already has an anamorphic `squeeze`. | `render_passes/` | Order: fog → **DOF** → motion blur → lens flare → bloom → tone map. Bloom then spreads already-blurred highlights, as a real lens would. The anamorphic squeeze should be one lens property shared by both passes. |
| Particles are drawn into `main` without writing depth. | Reference "Passes" | A particle gets the CoC of the opaque surface behind it. That is the usual post-DOF limitation; note it in the Reference page. |
| `DebugViews` lets a pass write named debug images. | `debug_views.rs`, motion blur | CoC, gather radius, and coverage should be debug views from day one. |
| The sandbox driving engine design is an isometric RTS. | memory: sandbox apps | Tilt-shift ("miniature") focus is the most likely game use: a ground plane that recedes smoothly through focus. That is the case where discrete planes look worst (B2). |

---

## Part B. Techniques

### B1. The thin-lens model (all techniques share this)

With focal length F, f-number N (aperture diameter A = F/N), focus distance S, and depth z along the view axis, the signed CoC diameter on the sensor is

  c(z) = A · F / (S − F) · (1 − S / z)

It is positive behind the focus plane (far field) and negative in front (near field). In pixels, c_px = c · H_px / h_sensor, with F = h_sensor / (2 · tan(fov_y / 2)).

**Engine form (derived).** Reverse-Z gives 1/z = (d − α)/β. Substituting:

  c_px(d) = K · (1 − S / z_f) − K · S · (z_f − z_n) / (z_n · z_f) · d,  with K = A · F / (S − F) · H_px / h_sensor

That is one multiply-add per pixel from depth, with both constants computed on the CPU, the same way `motion_blur.rs` folds per-frame values. Checks: d = 0 gives K(1 − S/z_f); d = 1 gives K(1 − S/z_n).

The CoC is a continuous function of depth. Every step after this should keep it that way: no thresholds on it, no classes of pixels, no blend between two images chosen by it.

**How big the blur gets (derived)**, 1080-pixel-tall image, 24 mm sensor height (full frame), background at infinity, so c_px = K:

| Lens | Field of view | Stop | Focus | Far-field CoC diameter |
| --- | --- | --- | --- | --- |
| The engine's default camera | 45° → F ≈ 29 mm | f/2 | 2 m | ≈ 10 px |
| 50 mm | ≈ 27° | f/1.8 | 2 m | ≈ 32 px |
| 85 mm | ≈ 16° | f/1.4 | 2 m | ≈ 121 px |
| 50 mm, object at 0.5 m in front | ≈ 27° | f/1.8 | 2 m | ≈ −97 px (near field, 3× the far value) |

So a useful far-field radius is about 16–60 px at full resolution. The near field grows without bound and needs a clamp. Unreal clamps both fields to a share of the view width (Memory: about 2.5 % by default; verify). The clamp must be smooth (a soft saturation curve), not a `min`, or the clamp itself becomes a visible plane (derived).

### B2. Layered gather with discrete planes: what production ships, and why it looks wrong

The production standard since about 2013. Lineage: Sousa, CryENGINE 3 (2013); Jimenez, *Call of Duty: Advanced Warfare* (2014); Abadie, Unreal "Diaphragm DOF" (2018), which Blender EEVEE's 2021 DOF rewrite followed. Hammon's CoD4 method and Kraus–Strengert's pyramid method are older layered relatives.

Structure (Memory, consistent with [S1], [S2], [S6]): compute the CoC; downsample to half resolution; classify pixels or tiles into a **near layer** and a **far layer**; gather each layer separately; then composite at full resolution in three steps: lerp sharp → blurred-far by CoC, then put blurred-near over it with an invented alpha.

**Where the seams come from (derived from the structure; matches the user's experience):**

| Seam | Cause | Where it shows |
| --- | --- | --- |
| Line where a surface crosses the near/far split | One continuous surface is gathered by two different filters with different leak rules, then joined. | A floor or terrain receding through focus: exactly the RTS tilt-shift camera. |
| Focus band edge, or a "sharp halo" ring | The full-resolution sharp image and the half-resolution blurred image are crossfaded by CoC. Halfway through the fade, the pixel is two images on top of each other, not a blur of intermediate size. | Every object entering or leaving focus. Unreal adds a third, full-resolution "slight out of focus" image to hide this (Memory), which adds one more fade. |
| Blocky or ghosted foreground | The near layer spreads by tile-dilated CoC, and its alpha is guessed from coverage counts. | Blurred foreground silhouettes over sharp backgrounds. Jimenez's notes call the tile-max-only version blocky [S2]. |
| Popping | Pixels or tiles change class from frame to frame as depth or focus moves. | Focus pulls and camera motion. Without TAA (Part A), nothing hides it. |

Parts worth keeping in other designs: the per-sample "does its disc reach me" coverage test (scatter as gather), the clamp of a behind-sample's CoC to the center's (B3 explains why that one is physical, not a layer), and tile CoC maxima **used only to bound the search, never to change what a pixel computes**.

**Fit:** don't build this.

### B3. Continuous single-accumulator gather (recommended core; derived design)

One gather, at one resolution, into one accumulator. Every weight is a continuous function of the sample's CoC, depth, and distance. No pixel is assigned to a layer, and no two images are crossfaded. It formalizes Gustafsson's single pass (B6), which is the closest published ancestor.

**What the sum means.** For pixel p, each sample s at distance r stands for a small surface patch whose light the lens spreads over a disc of diameter |CoC_s|. Pixel p receives s's light if that disc reaches p, at a density of one over the disc's area. The terms:

- **Coverage:** κ_s = saturate(|CoC_s| / 2 − r + 0.5). A smooth 1-pixel edge on the disc. It reaches 0 continuously.
- **Density:** ρ_s = 1 / max(π |CoC_s|² / 4, 1). A wide disc spreads its light thin. This term makes out-of-focus highlights keep their energy and take the aperture shape at the right brightness, instead of averaging away as they do in a plain normalized gather.
- **Occlusion, behind the center:** if s is farther than p's surface, p's own disc hides it except around p's blur. Clamp s's effective CoC to min(|CoC_s|, |CoC_p|), blending into the clamp over a short depth range measured in CoC units, not a hard depth test. When p is in focus, its CoC is about 0, so nothing behind leaks onto it. That's the physical result, and it's continuous in p's CoC. Gustafsson's 2× clamp [S6] is a tuned version of the same thing.
- **Occlusion, in front of the center:** unrestricted. A blurred foreground disc partly covers p, which is the translucent foreground edge that layers try to fake.
- **Normalization:** out(p) = Σ C_s κ_s ρ_s / Σ κ_s ρ_s. Where a blurred foreground disc covers p, the result weights foreground and background light by how much of p's lens each reaches. Where the occluded background is truly missing (hidden behind the foreground in the input image), the normalization fills it with the nearest visible light. That is the one invention a post-process can't avoid. It's per pixel and continuous, not a plane.

An optional ordering term (derived): weight each sample by a slowly decreasing function of depth, as weighted blended order-independent transparency does [S14], so that among samples that all cover p, nearer surfaces win smoothly. Add it only if the reference (B8) shows background bleeding through blurred foregrounds.

**The sample lattice must not depend on tiles.** If tiles choose a ring spacing or a sample count from their own maximum CoC, the noise pattern and banding change at every tile edge, which is a grid of planes. The lattice is fixed in absolute pixels, and tiles only decide how far out to stop (derived):

- **Inner rings** at radii 1, 2, 3 px, full resolution: 1 + 8 + 16 + 24 = 49 taps. This covers in-focus to slightly out-of-focus continuously. There is no separate "slight out of focus" image.
- **Outer rings** at geometric radii r_k = 3 · g^k (g ≈ 1.4), each with a fixed number of samples (12–16), rotated by the golden angle from the ring before. Each ring reads a level of a **CoC-aware pyramid** that matches its sample spacing, log2(2π r_k / n), with trilinear filtering between levels, so the level changes continuously with radius. Cost grows with the log of the radius, not its square.
- **Search bound:** a tile's dilated maximum |CoC| says which ring is the last one that could contain a covering sample. Stopping there skips rings whose coverage would be 0 anyway, so the output is the same as gathering every ring. Tiles change the cost, never the image. An assertion in debug builds can check this by gathering one ring past the bound and comparing.

**The pyramid** stores premultiplied quantities, so a coarse texel still answers the coverage test: Σ C·w, Σ w, and the weighted mean and maximum of the CoC per texel (derived). Mixing depths in a coarse texel is this design's main risk: a sharp foreground texel and a blurred background texel averaged together. The per-texel maximum CoC limits it, but this must be checked against B8 on hard depth edges before it ships.

**Noise without TAA:** the lattice is fixed per pixel, so there's no temporal shimmer. Spatial banding from the ring structure goes away in two ways: the outer rings read prefiltered levels instead of point samples, and a final small bilateral or median filter weighted by CoC. Unlike B2's postfilter, this filter runs over the whole image, with a strength proportional to CoC, so in-focus pixels pass through unchanged (derived).

| Strengths | Weaknesses |
| --- | --- |
| No seams by construction: a surface receding through focus gets a blur that grows continuously along it. No class changes, so no popping when focus moves. One resolution, so no crossfade halo. The density term gives correct highlight energy and shape. | Not shipped in this form; it's a design to prototype. The CoC-aware pyramid can leak at hard depth edges. The background hidden behind a blurred foreground is still invented. Full-resolution inner rings cost more than half-resolution layers. |

**Fit:** high. Same pass mechanics as before: compute, divisor-scaled pyramid targets, subgroup tile maxima, debug views.

### B4. Highlight scatter, made continuous

Bright, out-of-focus points are what people call "bokeh". Unreal (Memory) and the earlier Crysis 2 and UE3 sprite methods scatter them: pixels above a brightness threshold with a large CoC are appended to a buffer, removed from the gather, and drawn as aperture-shaped quads.

A threshold is a plane in brightness: a highlight that crosses it pops between "sprite" and "gathered". B3's density term already gives highlights their energy and shape, so the scatter is only needed where the gather's samples are too sparse to draw a crisp edge (large CoC, very bright). Make the split continuous (derived): each pixel sends a fraction σ = smoothstep(brightness) · smoothstep(CoC) of its light to its sprite and leaves 1 − σ in the gather. The sum of the two is always the pixel's light, so a highlight fading in or out never changes brightness.

On overflow, don't drop pixels: a pixel that finds the buffer full gets σ = 0 and keeps all its light in the gather. This keeps the "queues stall, never drop" rule.

**Fit:** medium. Needs an indirect draw or a mesh-shader path from a GPU count. Add it only if B3's large-CoC highlights look soft against the reference.

### B5. Separable aperture filters

- **McIntosh, Riecke, DiPaola 2012 [S4].** Square, hexagonal, and octagonal apertures from 1D blurs along the polygon's edge directions (a hexagon is three rhombi). Cheap, but limited to those shapes. One later reviewer calls the results "incorrect, but visually plausible" for many bokehs ([S10], Partial).
- **Garcia 2017, Circular Separable Convolution [S3].** A disc kernel written as a sum of complex Gaussians. Horizontal then vertical passes, with the image held as complex numbers. Shipped in Madden 15–17, FIFA 17, and PGA Tour. Strengths: large radii at low cost and low bandwidth per tap.

Separability holds only for a constant kernel, so a CoC that varies per pixel breaks it near depth edges (derived). In practice both are paired with layers to handle that, which brings back B2's seams. The complex layers multiply storage: about three RGBA16F targets (derived). Neither handles curved blades or cat's-eye shapes.

**Fit:** low. Don't build.

### B6. Single-pass golden-angle spiral (Gustafsson 2018) [S6], Read

One gather pass with no layers. Samples are on a golden-angle spiral, with the radius growing so samples spread evenly over the disc area. Foreground samples aren't restricted. Background samples' CoC is clamped to 2× the center's. When a sample doesn't reach, the running average is added again, which the author found by accident and can't explain. His half-resolution version adds a sharp-to-blurred composite, which brings back the crossfade.

**Fit:** the right family. B3 is this with the weights derived instead of tuned, a fixed absolute lattice, and a pyramid for the outer rings. It's a good one-day prototype to check the CoC math and debug views. It costs the worst case everywhere.

### B7. Mip or pyramid blur and diffusion

- **Pyramid blend (Hammon, GPU Gems 3 ch. 28 [S7]).** Blend between a few prefiltered blur levels by CoC. Discrete levels and Gaussian bokeh. Rejected.
- **Diffusion DOF (Kass, Lefohn, Owens 2006 [S11]; Memory).** Treats blur as heat diffusion with a per-pixel conductivity, solved with tridiagonal systems along rows and columns. Continuous, with no leaking across depth edges, but Gaussian bokeh only.

**Fit:** neither makes bokeh shapes. Diffusion is the continuous low-end fallback if a cheap tier is ever needed.

### B8. Ground truth: lens sampling

Average many frames rendered from points spread over the aperture. Each frame has its frustum sheared so the focus plane stays fixed. Alternatively, trace thin-lens rays in the visibility pipeline's ray tracing path. The result is exact: real partial occlusion and real hidden background, and continuous by definition.

**Fit:** high, and now required. B3's weights and pyramid are derived, so a reference is the only way to tune them and to catch leaks. Build it as a debug mode that accumulates while the camera is still. It's also a valid production path for a photo mode.

### B9. Per-pixel exact alternatives for the hidden background

Both are continuous because they work per pixel, not by depth slices:

- **Ray-traced partial occlusion (Tan et al. 2022 [S5]).** Post-process DOF everywhere, with rays only where post-processing is wrong: the edges of blurred foreground objects. Needs spatio-temporal reconstruction, which the engine lacks.
- **Ray marching a layered depth image (Lee, Eisemann, Seidel 2010 [S15]; Memory).** Depth peeling gives a few surfaces per pixel, and lens rays are traced through them. These are peels (the first, second, ... surface along each pixel's ray), not depth slices, so no seams. Needs a second visibility layer.

**Fit:** future, if B3's invented background is the remaining visible flaw.

### B10. Lens character and the tilted focus plane

All of these are per-sample weights or position changes inside the B3 gather or the B4 sprite. They add almost no cost, and all are continuous:

| Effect | How |
| --- | --- |
| Blade count, curvature, rotation | Map the ring sample's disc position onto a polygon with rounded edges. Unreal ties curvature to how far the lens is stopped down from its widest stop (Partial, [S8] forum). |
| Cat's eye (optical vignetting) | Intersect the aperture with a second, offset circle that grows toward the image edge. The sample test is one more circle check. |
| Anamorphic | Scale the kernel vertically. Share the squeeze value with lens flare. |
| Soap-bubble or smooth disc (spherical aberration) | A radial weight profile across the disc: brighter or softer rim. |
| Longitudinal chromatic aberration | A slightly different CoC per channel: purple near-field fringes, green far-field fringes. |
| **Tilted focus plane (tilt-shift)** | Replace the constant S with a per-pixel focus distance from a plane, S(pixel) = D / (n · r(pixel)), where r is the pixel's view ray with r.z = 1 (derived). Then 1/S is affine in pixel coordinates, so the cost stays at one multiply-add per pixel. This is the "miniature" look for an isometric RTS camera. Note that 1/(S − F) also varies with S; the exact form needs care. |

**Open mechanism (derived, per the memory rule).** Describe the aperture as data: blade count, curvature, rotation, radial profile, squeeze, and an optional baked aperture image. Don't add an enum of named "bokeh styles". The focus surface is also data: a distance or a plane.

**Miniature caveat (derived).** At RTS distances (tens of meters), a physical aperture gives almost no blur. A miniature look physically means an aperture that is large relative to the scene. Allow f-numbers below the physical limit, and document that they're not realizable on a real lens, rather than adding a non-physical "blur scale".

### B11. Autofocus

Read depth at a focus point (or the minimum depth over a small region) in a one-thread GPU pass. Move the stored focus distance toward it at a bounded rate (continuous function plus bounded rate, per memory). Keep it in a persistent target, so it never needs a CPU readback or a frame of latency. The application gives the focus point; a fixed distance is the same mechanism with the pass turned off.

---

## Part C. Recommendation

**Build B3: one continuous gather with a fixed absolute sample lattice, where tiles only bound the search. Shape bokeh with B10's aperture description. Build B8, the lens-sampling reference, alongside it, because B3's weights are derived and need a ground truth.** Add B4's continuous highlight scatter only if large-CoC highlights look soft against the reference. Don't build anything with near/far layers, depth slices, or a sharp-to-blurred crossfade.

### Phases

1. **Spec first (Reference).** In the Camera table of `docs/reference/rendering.mdx`, define these fields with units, ranges, and defaults:
   - aperture as an f-number;
   - focus distance in meters;
   - a new sensor height in millimeters, defaulting to 24 (full-frame height);
   - the diaphragm (blade count, curvature, rotation).

   Define the CoC formula (B1) and its smooth clamp as the contract, with a pinhole default (aperture 0 or ∞; pick one) that leaves the frame unchanged. Add the pass row and its place in the order.
2. **CoC, debug view, and the reference (B8).** A tile pass writes signed CoC from depth with the two constants. Add a CoC debug view and the accumulation reference. Black-box tests of the formula against the thin-lens equation at near, focus, and far depths.
3. **Inner rings only.** The 49-tap full-resolution gather with B3's coverage, density, and occlusion terms. This alone covers CoC diameters up to about 7 px. The default camera at f/2 already reaches about 10 px (B1), so the outer rings are needed even at the defaults; this phase is for checking the weights, not for shipping. Compare with the reference on a receding plane and a hard depth edge.
4. **Outer rings and the CoC-aware pyramid**, plus tile search bounds with the debug-build check that bounding doesn't change the output.
5. **Diaphragm.** Blades, curvature, rotation, and cat's eye in the sample weights. Add the anamorphic squeeze shared with lens flare.
6. **Tilted focus plane and autofocus (B10, B11)**, driven by the RTS sandbox.
7. **Continuous highlight scatter (B4)**, only if needed.

### Tests (black-box, per AGENTS.md)

- With a pinhole aperture, or every pixel on the focus plane, the output equals the input bit for bit.
- A uniform-color, uniform-depth frame stays the same color at any CoC (no darkening at edges or from weights).
- **No seams:** a textured plane receding through focus. Along it, the measured blur width (for example, the edge spread of a stripe texture per row) is monotonic on each side of the focus line and changes by no more than a set step between neighboring rows. This is the test that catches layer splits, crossfades, and tile-dependent lattices.
- **No popping:** moving the focus distance in small steps changes every output pixel by a small, bounded amount per step.
- One bright pixel in the far field spreads into the aperture shape: same total light (energy is conserved), and the footprint area matches the polygon's area for the given blade count within a tolerance.
- The CoC Reference formula at chosen depths (the spec, not the shader internals).
- The pass's BESL shaders lex and parse (the BESL lexer test seam), not their MSL output.

### Performance

- Cost now follows the blur, not the screen (derived). An in-focus pixel stops after the center tap. The inner rings cost up to 49 taps. Each outer ring adds 12–16 taps, and a 60 px radius needs about 9 rings with g ≈ 1.4: about 49 + 9 × 16 ≈ 190 taps at full resolution, at worst. At 1080p, a frame that is entirely wide blur is about 2.07M pixels × 190 taps ≈ 390M taps. Real frames are mostly in focus or slightly out, and the search bound skips what can't contain a covering sample. Measure before optimizing.
- If wide-blur frames are too slow, compute the outer-ring sum at half resolution and upsample it with edge-aware weights (the `visibility-perf` skill covers this). This is a sum of two parts, not a crossfade between two images, so it adds no seam, as long as the upsampling respects depth edges.
- Measure with the `visibility-perf` skill's method and the interleaved A/B binaries from the benchmark memory. Report per-stage GPU times (CoC, tile bounds, pyramid, gather, filter), not just the total.
- Use fp16 for the pyramid and the accumulators. Read the tile's search bound once per thread group, from shared memory.

### Kernel structure for the gather (derived; build it this way from the start)

The circle-of-confusion and reference shaders have no neighbor reads, so none of this applied to them. The gather is
the first real convolution, and it should be shaped like one:

- **Precompute the lattice.** Ring radii, sample directions, the pyramid level of each outer ring, and the aperture
  shape's weight per sample (blades, curvature, rotation) depend only on settings. Build them on the CPU when the
  settings change, into a constant buffer the shader loops over. No trigonometry, polygon tests, or `log2` per tap.
- **Stage the inner rings in tile memory.** A 16 × 16 workgroup with a 3-pixel halo loads 22 × 22 texels once, several
  reads per thread, issued before any are used. The 49 inner taps then read shared memory. That's about 1.9 texture
  reads per pixel instead of 49.
- **Derive per-texel terms once, at load.** Compute each staged texel's CoC from depth there (one multiply-add, so no
  CoC target for the inner rings). Also compute its density ρ, its color premultiplied by ρ, and its depth-order key.
  The tap loop is then one coverage compare and one multiply-add per tap, with no per-tap division.
- **Batch the outer rings' fetches.** Each outer tap is one bilinear read of a prefiltered pyramid level. Issue a
  whole ring's reads before weighing any of them.
- **Read once, reduce many.** Build the tile's maximum |CoC|, the search bound, with subgroup reductions in the pass
  that already reads depth: the pyramid build. Don't add a separate pass for it.
- **Measure per stage.** Put a counter on the pyramid and on the gather. Prove exact changes with zero differing texels
  on the gather's output before comparing times.

### Open decisions for you

1. Does the f-number accept values below about 0.7 for the miniature look, or should the miniature look be separate from the lens model? I recommend the former (B10).
2. Is a sensor-height field acceptable on `Camera`, or should the camera store focal length and derive the field of view? Field of view is the existing primary control, so I recommend sensor height.
3. Does DOF come before motion blur? I recommend yes; Unreal uses the same order.

---

## Sources

| ID | Source | Status |
| --- | --- | --- |
| S1 | Abadie, *A Life of a Bokeh*, SIGGRAPH 2018 Advances in Real-Time Rendering. Slides linked from Epic's Box (not opened). | Memory; landing info via search |
| S2 | Jimenez, [*Next Generation Post Processing in Call of Duty: Advanced Warfare*](https://www.iryoku.com/next-generation-post-processing-in-call-of-duty-advanced-warfare/), SIGGRAPH 2014. [Slides (PPTX, ~408 MB)](https://www.iryoku.com/downloads/Next-Generation-Post-Processing-in-Call-of-Duty-Advanced-Warfare-v18.pptx). | Partial (page); details Memory |
| S3 | Garcia, [*Circular Separable Convolution Depth of Field*](https://www.ea.com/frostbite/news/circular-separable-convolution-depth-of-field), SIGGRAPH 2017 Talks ([PDF](https://history.siggraph.org/wp-content/uploads/2022/09/2017-Talks-Garcia_Circular-Separable-Convolution-Depth-of-Field.pdf)); GDC 2018. | Partial |
| S4 | McIntosh, Riecke, DiPaola, [*Efficiently Simulating the Bokeh of Polygonal Apertures in a Post-Process Depth of Field Shader*](https://diglib.eg.org/handle/10.1111/v31i6pp1810-1822), CGF 2012. | Partial (abstract) |
| S5 | Tan et al., [*Hybrid DoF*](https://arxiv.org/abs/2210.04981) and [*A Hybrid System for Real-Time Rendering of Depth of Field Effect in Games*](https://arxiv.org/pdf/2210.06158), 2022. | Partial (abstract) |
| S6 | Gustafsson, [*Bokeh depth of field in a single pass*](https://blog.voxagon.se/2018/05/04/bokeh-depth-of-field-in-single-pass.html), 2018. | Read |
| S7 | Hammon, [*Practical Post-Process Depth of Field*](https://developer.nvidia.com/gpugems/gpugems3/part-iv-image-effects/chapter-28-practical-post-process-depth-field), GPU Gems 3 ch. 28. | Partial |
| S8 | Epic, [*Cinematic Depth of Field in Unreal Engine*](https://dev.epicgames.com/documentation/unreal-engine/cinematic-depth-of-field-in-unreal-engine) docs; [UE 4.20 DOF forum thread](https://forums.unrealengine.com/t/ue-4-20-new-depth-of-field-test/111508). | Partial |
| S9 | Sousa, [*Graphics Gems from CryENGINE 3*](https://www.slideshare.net/slideshow/graphics-gems-from-cryengine-3-siggraph-2013/25052583), SIGGRAPH 2013. | Memory |
| S10 | [*Real-time bokeh algorithms, and other convolution tricks*](https://dercuano.github.io/notes/convolution-bokeh.html) (dercuano notes). | Partial (search snippet) |
| S11 | Kass, Lefohn, Owens, *Interactive Depth of Field Using Simulated Diffusion on a GPU*, Pixar tech memo 2006. | Memory |
| S12 | [Erfan-Ahmadi/BokehDepthOfField](https://github.com/Erfan-Ahmadi/BokehDepthOfField) and its [resource list](https://gist.github.com/Erfan-Ahmadi/e27842ce9daa163ec10e28ee1fc72659): reference implementations of several methods above. | Partial |
| S13 | Sheng et al., [*Dr.Bokeh*](https://openaccess.thecvf.com/content/CVPR2024/papers/Sheng_Dr._Bokeh_DiffeRentiable_Occlusion-aware_Bokeh_Rendering_CVPR_2024_paper.pdf), CVPR 2024. Differentiable, offline; not real-time. Listed to rule out. | Partial |
| S14 | McGuire, Bavoil, *Weighted Blended Order-Independent Transparency*, JCGT 2(2), 2013. Source of B3's optional depth-ordering weight. | Memory |
| S15 | Lee, Eisemann, Seidel, *Real-Time Lens Blur Effects and Focus Control*, SIGGRAPH 2010. | Memory |
| S16 | McGuire, [*The Skylanders SWAP Force Depth-of-Field Shader*](https://casual-effects.blogspot.com/2013/09/the-skylanders-swap-force-depth-of.html), 2013. Another layered (near/far) design, with source. | Not read (search result only) |

**Licensing.** Unreal's source (EULA) and Blender EEVEE's (GPL) are for reading the design only. Don't copy code from either.

---

## Part D. What could not be verified

- **B3 is a derived design, not a shipped technique.** Its coverage, density, and occlusion terms follow from the thin-lens model, but the CoC-aware pyramid and the outer-ring lattice are untested. Plan on B8 to tune them, and expect the pyramid's behavior at hard depth edges to need iteration.
- No published real-time method was found that is both single-layer and pyramid-backed. The nearest are Gustafsson's single pass [S6] (no pyramid) and the layered methods (with pyramids or half resolution).
- No 2023–2026 GDC or SIGGRAPH game DOF talk turned up in search. An Advances course page from those years may have one.
- The exact Jimenez tap layout, the Unreal default radius clamps, and the "slight out of focus" image are from memory. They matter less now, since B2 is no longer the plan.
- The Garcia paper's equations and component counts weren't readable from the fetched PDF. The storage estimate in B5 is derived.
- No timings were collected for any method on Apple GPUs.
