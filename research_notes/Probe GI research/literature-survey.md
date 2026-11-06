# Ray-traced irradiance probes (DDGI family): literature and production survey

Prepared 2026-10-30 for Byte-Engine (Metal on an Apple M4 Mac mini first; Vulkan/DX12 also present). Survey only, no code.

## How to read this

- **Status** on each entry says how the source was read:
  - *Read*: full text, slide notes or shader code.
  - *Partial*: abstract, listing, docs page or code-derived defaults only.
  - *Secondary*: a third party's description.
- **(derived)** marks my own arithmetic or inference. It is not a claim the source makes.
- Short code expressions from SDK sources are given as formulas. Prose quotes are kept under 15 words.
- Entry IDs such as [D1] and [P1] are used in Part B (Design implications).
- Parts C (numbers at a glance) and D (what could not be verified) follow Part B.

---

## Part A. Annotated bibliography

### A1. DDGI and its production lineage

#### [D1] Majercik, Guertin, Nowrouzezahrai, McGuire. *Dynamic Diffuse Global Illumination with Ray-Traced Irradiance Fields*. JCGT 8(2), 2019.
URL: https://jcgt.org/published/0008/02/01/ (PDF: https://jcgt.org/published/0008/02/01/paper-lowres.pdf). Status: Read.
- **Layout.** Each probe stores:
  - irradiance as an 8×8 octahedral tile (the text says R11G11B10F; the timing table used RGB10A2);
  - mean distance and mean squared distance as a 16×16 RG16F tile.

  All tiles are packed into one 2D atlas with duplicated 1-texel gutters, so hardware bilinear filtering works. Tiles are padded to 4×4 write boundaries. Probes sit on a uniform grid with power-of-two counts per axis, and each axis can have its own spacing.
- **Placement.** Use 1–2 m spacing at human scale. Each room-like space should hold at least one full 8-probe cage. The authors also showed the grid can be rotated or translated, as long as shaded points stay inside it.
- **Update.** Each frame, every probe traces n rays from a spherical Fibonacci pattern. The pattern gets a new random rotation every frame. All rays launch in one batch, and the hits go into a G-buffer-like surfel buffer.
  - The hits are shaded with the same routine as screen pixels, including direct light (variance shadow maps).
  - Indirect light at a hit comes from the previous frame's probes, so bounces accumulate over frames. Screen-space AO is applied to camera pixels only, not to probe hits.
- **Blend.** Per texel: new = lerp(old, max(0, texelDir·rayDir) · rayRadiance, α), with hysteresis α between 0.85 and 0.98.
  - The depth moments use a sharper cosine-power lobe.
  - Texels whose lobe weight is below 0.001 are skipped.
- **Leak-resistant interpolant.** Four terms are combined with trilinear weights:
  - a soft backface weight;
  - a perceptual down-weighting of very low irradiance (below about 5% of range);
  - a Chebyshev (variance-shadow-map) visibility test against the stored moments;
  - an offset of the shading point along the normal and view vectors.

  An ablation figure shows each term removing part of the leak in a closed room. The visibility tile is 16×16 RG16F, where the light-field probes that DDGI builds on used 128×128×6 cube maps.
- **Formats.** RGB5A1 needed α = 0.8 and flickered. The other formats were similar at α = 0.95. Without float range, RGB10A2 and RGB8 came out too dark, which is why RTXGI adds gamma encoding [D3]. R11G11B10F has a green tint because it cannot store exact grays, but 11-bit floats matched 16F quality at 45% less storage.
- **Cost (RTX 2080 Ti, 32×8×32 probes, 64 rays/probe, every probe every frame):**

  | Pass | ms |
  |---|---|
  | Ray generation | 0.1 |
  | Ray cast | 0.8 |
  | Shading the hits | 0.4 |
  | Probe update | 0.7 (0.3 color + 0.4 depth) |
  | Sampling in the deferred pass | 0.5 |
  | **Total** | **2.6** |

  Throughput was 1.5–1.66 GRays/s. That run is about 524k probe rays per frame (derived).
- **Findings and limits.**
  - Probe count matters more than angular resolution. Too few probes leaves subtle leaks.
  - Feeding the previous frame's probes back in makes indirect light visibly "flow" after sharp visibility changes.
  - At probe hits, glossy energy is folded into diffuse.
  - The authors left adaptive probe selection (for example culling by farthest surface) as future work.

#### [D2] Majercik, Marrs, Spjut, McGuire. *Scaling Probe-Based Real-Time Dynamic Global Illumination for Production*. JCGT 10(2), 2021.
URL: https://jcgt.org/published/0010/02/01/ (arXiv 2009.10796, read via https://ar5iv.labs.arxiv.org/html/2009.10796). Status: Read (arXiv text; timing tables are images and were not extracted).
- **One self-shadow bias parameter.** bias = (0.2·n + 0.8·ω_o) · 0.75 · D · B, where D is the minimum axial probe spacing and B = 0.3 by default.
  - Raise B when there are fewer rays per probe (noisier moments).
  - Too large a B causes light leaks.
  - At 1 m spacing the default offset is 0.225 m (derived).
- **Backface rays.** A ray that hits a backface stores irradiance 0, and its depth is shortened by 80%. Depth is not zeroed, because that would wrongly darken probes that sit near real surfaces.
- **Perceptual encoding.** Irradiance is raised to 1/5 before hysteresis blending. When sampling, each of the 8 probe values is raised to 2.5, the values are blended, and the result is squared. This makes light-to-dark transitions look linear and reduces flicker from single bright rays.
- **Faster lighting transitions.**
  - Per texel, irradiance only:
    - a change above 25% of the maximum lowers hysteresis by 0.15;
    - a change above 80% sets it to 0.
  - Per scene event:
    - small light change: irradiance hysteresis −15% for 4 frames;
    - large light change: −50% for 10 frames;
    - large geometry change: irradiance −50% for 10 frames and visibility −50% for 7 frames.
  - TAA history must be relaxed by the same rule, or it adds its own lag. Visibility hysteresis is not lowered per texel because visibility is too unstable.
- **Probe relocation.** If a close backface exists and more than 25% of rays hit backfaces, the probe moves through the closest backface, capped at an offset limit × spacing. A probe hugging a surface is nudged away. The optimizer runs for about 5 iterations during classification, then stops so probes do not oscillate. Only static geometry drives relocation.
- **Probe states.** Off, Sleeping, Newly Awake, Newly Vigilant, Awake, Vigilant.
  - A probe with no geometry within one spacing goes to sleep.
  - It wakes when it falls inside a dynamic object's AABB grown by one cell plus the bias.
  - "Newly" probes can converge fast with hysteresis 0.
  - Initialization traces distance only for 5 frames.
  - Sleeping probes gave a 30–50% average speedup.
- **Large worlds.** The paper uses nested volumes in one texture: camera-locked, city-scale and level-scale.
  - The shader walks volumes from densest to sparsest and stops once the weights sum to 1.
  - Each volume fades linearly to zero across its last grid cell.
  - A camera-locked volume tightens its fade by one cell to avoid pops when it moves.
  - A tracking window spawns a new plane ahead of the camera once the camera is more than one spacing from the center (circular buffer).
- **Other.**
  - Moving the update to coherent compute was about 3× faster than pixel shaders with stencil.
  - Irradiance is sampled inline in the deferred shader, with no separate gather pass.
  - Second and later glossy bounces reuse probe irradiance at maximum roughness. The paper notes a 2π unit difference between irradiance and radiance in that reuse.

#### [D3] NVIDIA RTXGI-DDGI SDK (RTXGI v1.x).
URL: https://github.com/NVIDIAGameWorks/RTXGI-DDGI. Status: Read. Files read:
- `docs/DDGIVolume.md`;
- `rtxgi-sdk/include/rtxgi/ddgi/DDGIVolume.h`;
- `rtxgi-sdk/shaders/ddgi/` `ProbeBlendingCS.hlsl`, `Irradiance.hlsl`, `ProbeRelocationCS.hlsl`, `ProbeClassificationCS.hlsl`, `include/ProbeRayCommon.hlsl`;
- `samples/test-harness/shaders/ddgi/ProbeTraceRGS.hlsl`.
- **Defaults (DDGIVolume.h).**

  | Field | Default |
  |---|---|
  | probeNumRays | 256 |
  | probeHysteresis | 0.97 |
  | probeDistanceExponent | 50 |
  | probeIrradianceEncodingGamma | 5 |
  | probeIrradianceThreshold | 0.25 |
  | probeBrightnessThreshold | 0.10 |
  | probeRandomRayBackfaceThreshold | 0.1 |
  | probeFixedRayBackfaceThreshold | 0.25 |
  | probeViewBias | 0.1 world units |
  | probeNormalBias | 0.1 world units |
  | probeMinFrontfaceDistance | 1.0 |
  | Fixed rays (RTXGI_DDGI_NUM_FIXED_RAYS, from the docs) | 32 |

  The docs warn that the default view bias probably will not suit your content. They also suggest one probe every 2–3 m.
- **Resources.**
  - **RayData:** one row per probe and one column per ray, storing radiance and hit distance. A miss stores distance 1e27. A backface hit stores −0.2·t.
  - **Irradiance:** octahedral interior plus a 1-texel border, in a texture array with one slice per horizontal plane of probes. The docs example uses a 6×6 interior, so 8×8 tiles.
  - **Distance:** the same layout, storing the mean and mean square. The code accumulates both moments.
  - **ProbeData:** one texel per probe. xyz holds the relocation offset divided by spacing; w holds the state.
  - **Variability:** per-texel coefficient of variation, reduced to one volume-wide average that estimates convergence.
  - Practical limit: about 128 probes per axis.
- **Ray directions.** Fixed rays are never rotated, which keeps relocation and classification stable. The other rays follow a spherical Fibonacci pattern rotated by a fresh random quaternion every frame. Inactive probes trace only the fixed rays.
- **Blend shader.** One thread per texel.
  - Irradiance = Σ(L·cos⁺) / (2·Σcos⁺), with fixed rays skipped. Normalizing by the weight sum rather than the ray count means the stored value is E/(2π), and the sampler multiplies by 2π (derived).
  - The texel is left unchanged if backface hits reach 10% of the rays.
  - Irradiance is gamma-encoded, then the hysteresis rules run:
    - **Empty history:** if the stored value is 0, hysteresis is 0 (covers first frames and cleared planes).
    - **Darkening:** if the max component of (old − new) exceeds 0.25, hysteresis drops by 0.75.
    - **Brightening:** if the luminance of (new − old) exceeds 0.10, the step is scaled by 0.25.
    - **Minimum step:** when darkening, each update moves by at least 1/1024 so low-bit formats still reach black.
  - Reading the code: the hysteresis drop fires only when a texel darkens, and large brightening is damped. This asymmetry cuts fireflies while still letting lights turn off fast (derived). Note the −0.75 here against −0.15 in [D2].
  - Distance: weight = cos⁺^50, d = min(|t|, 1.5·|spacing|), and the (d, d²) moments are lerped with hysteresis.
  - Border texels are copied after a group barrier.
- **Sampling (Irradiance.hlsl).**
  - Biased position: p' = p + n·normalBias − camDir·viewBias.
  - Per-probe weight:
    - trilinear, with each axis factor floored at 0.001;
    - × ((dot(dirToProbe, n) + 1)/2)² + 0.2;
    - × max(0.05, cheb³), where cheb = σ²/(σ² + (r − μ)²) applies only when r > μ, and the sampled moments are scaled by 2;
  - Weight floor 1e−6.
  - Crush: below 0.2, w ← w³/0.04.
  - Decode: raise each probe sample to γ/2, normalize, square, multiply by 2π. The U32 (RGB10A2) format gets an extra ×1.0989.
  - Volume blend weight fades linearly over one spacing outside the volume's extent.
- **Relocation and classification (fixed rays only).**
  - **Relocation, inside geometry:** when backface ratio > 0.25 and a closest backface exists, move through it by t + 0.5·minFrontfaceDistance.
  - **Relocation, too close to a frontface:** move toward the farthest frontface if it lies roughly opposite.
  - **Relocation, otherwise:** relax back toward the cell center.
  - Any candidate offset of 0.45 spacing or more is rejected. Offsets are not smoothed.
  - **Classification:** backface ratio > 0.25 makes the probe inactive. Otherwise it is active only if some frontface hit lies inside its voxel (planes one spacing away).
- **Sample hit shading (ProbeTraceRGS).**
  - Direct diffuse uses all lights with shadow rays.
  - Indirect: min(albedo, 0.9)/π × irradiance sampled from the same volume, biased the same way. That volume still holds the previous frame's data during the trace.
  - A miss stores sky radiance.
  - The sample stores saturate(radiance), clamping HDR to 1. Check this against your exposure convention.
  - Backface hits get no shading.
  - There is an optional NVAPI path with shader execution reordering.
- **Scrolling volumes.** Scroll offsets are computed on the CPU each frame. Planes that wrap are cleared to 0, so the empty-history rule fills them right away. Rotation is ignored for scrolling volumes.

#### [D4] NVIDIA RTXGI 2.x: SHaRC (Spatially Hashed Radiance Cache) and NRC (Neural Radiance Cache).
URLs:
- https://github.com/NVIDIA-RTX/RTXGI
- https://github.com/NVIDIA-RTX/SHARC (`docs/Integration.md`, `include/HashGridCommon.h`)
- NRC paper: https://arxiv.org/abs/2106.12372

Status: Read (README, SHaRC integration guide, hash code); NRC from the abstract only.
- **What changed in 2.x.** DDGI moved to the separate RTXGI-DDGI repository. RTXGI 2.x ships two path-tracing caches:
  - NRC: experimental and needs tensor cores (NVIDIA Turing or newer).
  - SHaRC: runs on any DXR-capable GPU.

  Both cache radiance at path vertices so paths can stop early. Neither is a probe grid.
- **SHaRC key.** The default key is 64 bits:
  - 17 bits per axis of quantized position;
  - 9-bit level;
  - 3-bit normal sign (one bit per axis, bias 1e−3).

  A compact 32-bit key uses 8+8+8 bits, a 5-bit level and 3-bit normal. Hashing is Jenkins, with linear probing inside a fixed bucket.
- **SHaRC level of detail.**
  - level = 0.5·log_base(dist²) + levelBias, clamped to at least 1.
  - voxel size = base^(level − levelBias) / sceneScale.
  - So cells grow with distance from the camera.
- **SHaRC memory.** About 2²² entries are suggested. Each voxel takes 8 B key + 16 B accumulation + 16 B resolved = 40 B, about 160 MiB. With SH directional encoding it is 64 B, about 256 MiB.
- **SHaRC update.**
  - Trace update paths from a small subset of pixels, for example one random pixel per 5×5 block (about 4%).
  - A resolve pass blends with a maximum accumulated-frame count, evicts entries idle longer than staleFrameNumMax, and clears accumulation.
  - Radiance is stored as scaled 32-bit integers so atomic adds can be used.
  - An optional responsive-lighting mode clears accumulation every frame.
- **SHaRC query.**
  - Do not query at the primary hit.
  - Keep tracing until the segment is at least one voxel long.
  - On glossy paths, use the cache only once the cone spread, 2·t·sqrt(0.5·a²/(1 − a²)), exceeds the voxel size.
  - Expect 10–20% occupancy with a static camera.
- **NRC.** A small network trained online during rendering. The paper reports about 2.6 ms at 1080p for training plus queries. As shipped in RTXGI it needs tensor cores, so it is not a Metal option (inference).

#### [D5] Majercik, Müller, Keller, Nowrouzezahrai, McGuire. *Dynamic Diffuse Global Illumination Resampling*. SIGGRAPH 2021 talk; CGF 2022.
URL: https://arxiv.org/abs/2108.05263. Status: Partial (abstract plus quoted snippets).
- DDGI probes are queried one bounce after the camera hit, inside a ReSTIR-style resampling path tracer, much like final gathering in photon mapping.
- The authors report gains over pure path tracing and pure probes at equal time.
- Secondary-vertex lookups exposed light leaks that the original self-shadow bias did not fix.
- They also produced fireflies in concave corners.
- Implication: biases tuned for camera pixels need re-tuning for ray-hit lookups (inference from the reported failure).

#### [D6] Datta, Goli, Zhang. *Adaptive Dynamic Global Illumination*. HPG 2022 poster; arXiv 2301.05125 (2023).
URLs: https://arxiv.org/abs/2301.05125 and https://sayan1an.github.io/pdfs/adgi.pdf. Status: Read (poster PDF).
- **Adaptive sampling.** Rays are not split evenly across probes. Markov chains place samples where radiosity changes over time, using 8 pilot rays per probe (one per octant) inside an extended region of interest. Static and dynamic chains run separately.
- **Running mean with a cap.** Each cache texel is a true running mean up to N_max samples, and an EMA after that:
  - static probes: hysteresis 0.98 (N_max = 63);
  - dynamic probes: 0.91 (N_max = 10).

  Detected change resets N to 16 for fast catch-up.
- **Packed formats.** Irradiance is 26-bit color plus a sample count. Visibility is 13+13 bits plus a count.
- **Baseline.** "Q-DDGI" uses 32 rays per probe, hysteresis 0.94, R11G11B10 plus count, and frustum culling of probes. That is 0.8–1.6 M rays per frame; ADGI uses 0.5–0.9 M.
- **Result.** On Bistro with 192×64×192 probes (about 2.36 M), GI took 15.6 ms against 26.8 ms for Q-DDGI (RTX 2080 Ti, 1080p). It also reacts faster to changes such as a gate opening.

### A2. Newer probe and cache techniques, 2021–2026

#### [N1] AMD GI-1.0 (Boissé et al., AMD technical report 2022; arXiv 2310.19855) and GI-1.1/1.2 (Capsaicin).
URLs:
- https://arxiv.org/abs/2310.19855 (read via https://ar5iv.labs.arxiv.org/html/2310.19855)
- https://gpuopen.com/download/GPUOpen2022_GI1_0.pdf
- https://github.com/GPUOpen-LibrariesAndSDKs/Capsaicin

Status: GI-1.0 Read. GI-1.1/1.2 Partial (descriptions only).
- **Screen cache.**
  - At most one hemispherical probe per 8×8 pixel tile, on a random pixel, stored as an 8×8 octahedral tile.
  - A quarter of the tiles are respawned per frame using Halton jitter.
  - An old probe is reused if its plane distance is under cell_size and its normals agree with a dot product above 0.95.
  - Rays from tiles that reprojected well are reassigned to empty tiles, so the total ray count stays constant.
  - Rays are guided by a CDF built from the luminance of the reprojected radiance.
- **Adaptive cell size.**
  - cell_size = distance_scale × distance to camera.
  - distance_scale = tan(fov_y · proj_size · max(1/H, H/W²)), with proj_size = 8 px. It can be computed on the CPU.
- **World cache.**
  - A hash grid keyed on quantized position and quantized ray direction. Quantization coarsens with distance, which gives radiance levels of detail.
  - One extra key bit records whether the ray was shorter than the cell. This is their light-leak fix.
  - Cells are grouped into 8×8 tiles projected along the dominant axis, with box-filtered mips. Updates blend with an EMA.
  - Each entry has a decay counter for eviction.
  - Screen-probe rays read this world cache at their hit points.
- **Reconstruction.**
  - Probes are projected to L2 SH (3 bands) before per-pixel interpolation.
  - Each pixel uses the nearest probe in each of 4 directions, with depth and normal weights. Probes farther than cell_size get weight 0.
  - If every weight is 0, the pixel falls back to equal weights and is flagged for the denoiser.
  - Near-field detail comes from a short-radius horizon-based screen-space GI pass.
- **Temporal.**
  - Screen probes blend with a biased hysteresis: a factor from the normalized difference between new and reconstructed radiance, clamped to 0.95 and squared. Empty cells are filled with the average of populated ones.
  - A persistent LRU side cache of evicted probes keeps thin geometry from wobbling over time.
- **Cost at 1080p, ¼ spp, direct plus indirect:** 1.9–3.4 ms total on RX 6900 XT or RTX 3080 (99k to 9M triangles). Ray tracing itself was only 0.13–0.68 ms of that.
- **Stated limits.**
  - Multi-bounce only comes from surfaces visible in the previous frame.
  - Probe blur loses small features.
  - Thin occluders can still leak.
  - Disocclusions are noisy under fast motion.
- **GI-1.1 and GI-1.2.** GI-1.1 adds glossy reflections (SIGGRAPH Asia 2023 paper): interpolate screen probes for rough surfaces, and ray trace into the hash-grid cache for smooth ones. GI-1.2 adds multi-bounce indirect light. Details were not read.

#### [N2] Lumen (Unreal Engine 5).
Sources:
- Wright, *Radiance Caching for Real-Time GI* (SIGGRAPH 2021 Advances): https://advances.realtimerendering.com/s2021/index.html
- Wright, Narkowicz, Kelly, *Lumen* (SIGGRAPH 2022 Advances): https://advances.realtimerendering.com/s2022/SIGGRAPH2022-Advances-Lumen-Wright%20et%20al.pdf
- Epic docs: https://dev.epicgames.com/documentation/en-us/unreal-engine/lumen-technical-details-in-unreal-engine
- UE 5.4.4 CVar defaults taken from source: https://indxzero.github.io/ue544cvarwiki/articles/r.lumen.screenprobegather.downsamplefactor and sibling pages

Status: Partial. The 2022 slides were not fetched (over 10 MB). The numbers below are code defaults, not talk claims.
- **Screen probes.**
  - One probe per 16 px tile (clamped 4–64). Scalability presets use 32, 16 and 8 px.
  - Adaptive extra probes go down to 4 px, with an allocation fraction of 0.5.
  - The tracing octahedron is 8×8, so 64 traces per probe, with importance sampling on by default.
  - In reference mode a probe traces 1024 uniform rays with no filtering, importance sampling or radiance cache.
- **World radiance cache.**
  - 4 clipmaps, each a 48³ grid. The first clipmap spans 2500 cm (code default). The growth rate between clipmaps was not verified.
  - Each probe is 32×32 radiance texels (16×16 on High), so 1024 rays per probe at full resolution.
  - The trace budget is 300 probes per frame (1000 on Cinematic), scaled by a lighting-update-speed factor of 0.5–4.
- **Near and far split.** Epic describes tracing many more rays for distant light through the world-space cache, while screen probes use short rays. An Epic staff forum post says world probes are placed only near surfaces that have screen probes (search snippet; not read directly).
- **Trace order.** Screen traces run first. A ray that finds no hit, or passes behind a surface, falls back to software (distance field) or hardware ray tracing. Hits are lit from the surface cache, which is updated over several frames and gives unlimited bounces.
- **Budget and scale.**
  - The Epic preset is about 8 ms for GI and reflections at 1080p internal on consoles (30 fps target). The High preset targets 60 fps.
  - The scene covers 200 m by default and up to about 1 km with far field.
- **Failure modes from the docs.**
  - Walls thinner than about 10 cm leak.
  - Caves leak when the trace distance is too short.
  - Lighting can pop in during fast camera moves.
  - A "lighting update speed" setting trades responsiveness against cost.

#### [N3] Radiance cascades.
Sources:
- Sannikov, *Radiance Cascades* (unsubmitted JCGT-format paper): https://github.com/Raikiri/RadianceCascadesPaper (`RadianceCascades.tex` read)
- Osborne & Sannikov 2024: https://arxiv.org/abs/2408.14425 (HTML read)
- Freeman, Sannikov, Margel, *Holographic Radiance Cascades* (2D), 2025: https://arxiv.org/abs/2505.02041 (abstract)
- Freeman, Sannikov, Pitt-Francis, *Split Radiance Cascades*, 2026: https://arxiv.org/abs/2607.20384 (abstract only; PDF over 10 MB)

Status: Read / Partial.
- **Penumbra condition.** To resolve light at distance D, the linear step must be at most about D and the angular step at most about 1/D. So near-field light needs fine spatial and coarse angular sampling, and far-field light the reverse.
- **Cascade i.** Probe spacing grows ~2^i and angular step shrinks ~1/2^i. Cascade i stores the radiance interval [t_i, t_{i+1}], with t_i ~ 2^i (or 2^(αi) with branching factor α).
- **Merging.** Intervals combine like volume compositing: L_ac = L_ab + β_ab·L_bc and β_ac = β_ab·β_bc. Diffuse output merges from the top cascade down to cascade 0, then integrates cascade 0's few directions.
- **Memory.** In full world-space 3D, memory per cascade falls by about 2× per level (total < 2·M_0), but cascade 0 alone is roughly a full voxelization of the scene. For surface-attached 3D variants, memory per cascade is constant.
- **Known artifacts.**
  - Ringing: up to about 10% worst case, typically a few percent.
  - A "bilinear fix" removes ringing but costs 4× rays on every cascade except the last.
  - Occluders smaller than the probe spacing still leak.
  - Energy is not conserved where cascades overlap a light source.
- **Shipped use.** Path of Exile 2 uses a 2.5D screen-space variant: about 3 ms on a GTX 1050, with 4 rays per raymarch and a 128-bit occlusion bitmask per texel. It misses off-screen light and occluders.
  - A variant with screen-attached probes and world-space intervals catches off-screen light but took 30–50 ms on an RTX 3060.
  - Sannikov estimates hardware RT would cost about 10× more than screen raymarching.
- **Split Radiance Cascades (2026).**
  - World-space probes live in a sparse hashmap.
  - "Ray splitting" builds the intervals from rays traced from visible surfaces, weighted by hit distance.
  - The authors say it captures soft penumbrae that DDGI misses when the penumbra is smaller than DDGI's probe spacing.
  - Timings and layouts were not read.

#### [N4] Unity Adaptive Probe Volumes (APV), HDRP/URP 17 and Unity 6.
URLs:
- https://docs.unity3d.com/Packages/com.unity.render-pipelines.high-definition@17.0/manual/probevolumes-concept.html
- https://docs.unity3d.com/Packages/com.unity.render-pipelines.high-definition@17.0/manual/probevolumes-fixissues.html
- https://docs.unity.com/en-us/engine/current/manual/lighting-overview/lighting/probevolumes/options-override-reference
- `ProbeVolumesOptions.cs` in https://github.com/Unity-Technologies/Graphics (core package)

Status: Read. APV is baked, but its leak tooling transfers to dynamic probes.
- **Bricks.** 4×4×4 probes per brick. Spacing levels are 1, 3, 9 and 27 m by default, chosen by local geometry density. Pixels sample their 8 nearest probes.
- **Probe data.** L1 or L2 SH. A per-probe validity texture packs one validity bit per neighbor in the probe's 8-neighborhood.
- **Runtime defaults.**
  - Normal bias 0.05 m and view bias 0.1 m. Both can optionally scale with the minimum probe spacing in a brick.
  - Sampling noise 0.1, animated under TAA, hides seams between subdivision levels.
- **Leak-reduction modes.**
  - Performance: warps the sample coordinates so invalid probes stop contributing; one APV fetch.
  - Quality (default): takes 1–3 fetches.
- **Validity.** A probe is invalid when too many of its rays hit backfaces (threshold is configurable).
- **Bake-time fixes.**
  - Virtual Offset moves an invalid probe's capture point out of geometry. Controls: search distance multiplier, geometry bias, ray-origin bias.
  - Dilation fills invalid probes from valid neighbors. Controls: search radius, iterations, optional squared-distance weights.
- **Authoring fixes.**
  - Rendering-layer masks (up to 4) restrict which probes a renderer may sample.
  - Probe Adjustment Volumes can invalidate or override probes locally.
  - The docs say to make walls closer to the local probe spacing in thickness.
- **Fallback for SSGI.** HDRP screen-space GI falls back to APV (via a fallback-hierarchy mask) when a screen-space ray misses. A "mixed" mode tries screen-space raymarching first, then hardware RT.

#### [N5] EA SEED / Frostbite GIBS (Global Illumination Based on Surfels). SIGGRAPH 2021 Advances (Halén, Brinck et al.). Included for contrast; the shipped details are in [P2].
URL: https://www.ea.com/seed/news/siggraph21-global-illumination-surfels. Status: Partial (abstract; the 16 MB slides were not read).
- **Surfels.** Spawned from the G-buffer as surfaces come into view, they persist and follow their surfaces. Surfel count is capped and allocated up front.
- **Structure.** Surfels are re-binned into a spatial acceleration structure every frame.
- **Techniques.** Hardware RT with ray guiding, ray binning and spatial filters. Handles skinned meshes and transparency through companion probes.
- **Contrast with probes.** Surfels follow the camera's view and decouple ray rate from shading rate. They have the cold-start problem probes do not ([P2] fixes it with probes).

#### [N6] Iwanicki. *Neural Light Grid: Modernizing Irradiance Volumes with Machine Learning*. SIGGRAPH 2024 Advances (Activision).
URL: https://advances.realtimerendering.com/s2024/content/Iwanicki/Advances_SIGGRAPH_2024_Neural_LightGrid.pdf. Status: Read (speaker notes). Precomputed, not ray-traced at runtime.
- **Framing.** Treat probe reconstruction as minimizing error. The per-probe weighting functions should stop at walls, ignore small obstacles and wrap around large ones.
- **Building the weights.** For each probe:
  - points visible from the probe get trilinear weights;
  - 2 diffusion passes spread weight to points with no line of sight;
  - visibility is tested only against the closest 32 neighbors, using BVH-vs-BVH culling.
- **Runtime.** Each weighting function is a tiny MLP: xyz input, 2 hidden layers of 8 units, LeakyReLU. A generator network predicts the weights; direct optimization ran at about 60–80 functions per second.
- **Probes inside geometry.** Instead of moving the probe, cluster its support points by color and use the largest connected component.
- **Shipped.** Call of Duty: Modern Warfare 3 and Warzone. About 33% less memory than their previous tetrahedral system.
- **On DDGI.** DDGI needs about 1 KB of unique depth per probe. At very low probe density, DDGI-style Chebyshev visibility shows discontinuities.

#### [N7] Coomans et al. *Real-time Rendering with a Neural Irradiance Volume*. arXiv 2602.12949 (CGF 45(2), 2026).
URL: https://arxiv.org/abs/2602.12949. Status: Read (HTML). Contrast only, since it is baked offline.
- **Model.** A 4-layer MLP maps (position, normal) to irradiance. Larger variants use a hash-grid encoding.
- **Cost.** 0.19–1.35 ms at 1080p on an RTX 4090, about 4× slower on an RTX 2080 Ti.
- **Claims.**
  - It learns contact shadows and avoids leaks without explicit visibility.
  - Probes leak unless ray-traced visibility is added.
  - 0.25 MB against 35.5 MB for an open-source DDGI setup on Sponza.
- **Limits.** No online updates yet, and few dynamic lights.

#### [N8] Tatzgern et al. *Radiance Caching with On-Surface Caches for Real-Time GI*. HPG 2024.
URL: https://highperformancegraphics.org/slides24/hpg24_oscgi.pdf. Status: Read (slides).
- **Caches.** Radiance is cached in texture space on triangle "islands". Primary caches are allocated from the visibility buffer. Secondary caches cover about 30×30 cm and are quantized so many rays reuse them.
- **Lighting.** Secondary caches sample direct light with ReSTIR, and sample each other for multi-bounce over time.
- **Artifact and fix.** At lower cache resolution, rays can start from bad triangles. Decoupled triangle IDs fix this.
- **Relevance.** A visibility-buffer-native alternative to world-space probes, aimed at sharing caches across viewers.

#### [N9] Embark *kajiya* (Rust renderer), GI overview.
URL: https://github.com/EmbarkStudios/kajiya/blob/main/docs/gi-overview.md. Status: Read.
- **Cache layout.** 12 camera-aligned 32³ clipmaps with a dense indirection array. Up to 65,536 entries are pre-allocated, typically fewer than 16k in use. Entries are freed a few frames after their last use.
- **Allocation.** No entry is requested if irradiance can be reprojected from screen space.
- **Leak control by rank.** Entries visible on screen get rank 1, and entries spawned from rank N get N+1. A cell accepts votes only from equal or lower rank, which stops outside rays from filling inside cells. Lookups are also offset along the normal.
- **Rays per entry per frame.** 4 for the main trace (G-buffer plus sun shadow), 4 for validation, 16 short accessibility rays. Results are stored as L1 SH.
- **Cost.** 0.55 ms at 1080p on an RX 6800 XT, about 128k cache rays plus 384k shadow rays.
- **Reported problems.** The cache flickers, thin geometry leaks, and indirect shadows are blurry.

#### [N10] Bevy *Solari* world cache (Rust, wgpu), Bevy 0.17–0.19.
URLs: https://jms55.github.io/posts/2025-09-20-solari-bevy-0-17 and https://jms55.github.io/posts/2026-04-12-solari-bevy-0-19. Status: Read.
- **Key.** Spatial hash of quantized position, quantized geometric normal and an LOD factor. A checksum detects collisions, and probing has a bounded number of steps.
- **Per frame.**
  - A life value decays each frame, and an access resets it.
  - Active cells are compacted with a prefix sum.
  - Each cell traces 1 RIS light sample plus 1 cosine ray, and the cosine ray reads the cache at its hit.
  - Blending: 1/min(n + 1, cap).
- **Scaling.**
  - On Bistro, sampling took 2.09 ms (RTX 3080, 1600×900).
  - Version 0.19 caps updates at 40k random cells per frame: 2.65 ms → 0.47 ms.
  - The finest cell shrank from 0.25 m to 0.15 m.
- **Leak fix.** Force the finest level when the ray is shorter than the cell (only on ReSTIR GI's first bounce). This reduces but does not remove leaks near corners.
- **Open problem.** Visible popping at LOD boundaries.
- **Tracing.** Inline ray queries, because wgpu lacks RT pipelines.

#### [N11] Critiques of moment-based (Chebyshev) probe visibility.
Status: Secondary or Partial.
- **NVIDIA GDC 2017 light-field-probe slides** (https://developer.download.nvidia.com/assets/gameworks/downloads/regular/GDC17/McGuire2017LightField-GDCSlides.pdf; claims via search snippet, PDF not read): two moments cannot represent a texel that sees two depth peaks, so variance-shadow-map tests leak and are conservative.
- **SDF-DDGI** (arXiv 2007.14394; snippet only): depth-map visibility cannot fully stop leaks through thin objects; per-pixel SDF visibility can.
- **Godot (reduz, PR #86267):** tested DDGI-style octahedral visibility and found it leaked more than SDFGI's occlusion ([P8]).
- **Frostbite [P2]:** needed variance-depth attenuation to stop dark probes under stadium stands from leaking onto the crowd.
- **Correction:** a third-party blog claims RTXGI stores only distance (not distance²). The SDK's blend shader does accumulate both moments [D3].

### A3. Shipped titles, 2022–2026

#### [P1] id Software, idTech 8: *Indiana Jones and the Great Circle* (early iteration) and *DOOM: The Dark Ages*. Sousa, *Fast as Hell: idTech 8 Global Illumination*, SIGGRAPH 2025 Advances.
URL: https://www.advances.realtimerendering.com/s2025/content/SOUSA_SIGGRAPH_2025_Final.pdf. Status: Read (slides and notes). This is the closest published match to Byte-Engine's design.
- **Why runtime GI.** 60 Hz or better on all platforms, with Xbox Series S as the low end. Baked data would have meant 44–110 GB on disk and 16–40+ hours of bake per level.
- **Irradiance volumes.**
  - 6 cascades of 16³ probes (12³ on low spec), plus hand-placed local volumes of 10³.
  - Probes sit at cell centers. No relocation is mentioned.
  - Interleaved updates: each frame updates 1 cascade and 1 local volume.
  - Each probe traces 64, 32 or 16 visibility-only rays per update depending on platform. That is 262k, 131k or 65k rays per cascade update (derived).
- **Hit record (128 bits).** Shader binding table index (23 bits) + part of the primitive ID; instance index (24 bits) + the rest of the primitive ID; barycentrics; hit distance. Materials are shaded later, not in the trace.
- **World radiance cache.**
  - Each hit hashes into a 1D spatial-hash cache: 25 cm cells, doubling with distance (LOD = exp2(floor(log2(1 + d/LOD_DIST)))). Collisions use linear search; the cache is 14 MB.
  - The active cells are listed per shader, and each list is shaded by an async indirect dispatch. Shading reuses the raster lighting and material code, plus the previous frame's irradiance volumes for multi-bounce.
  - About 20k cache entries are shaded per frame and reused for N frames.
- **Volume update.**
  - Thread groups of 8×8 cover up to 64 rays. They read cache results (sky on a miss) and convolve irradiance per texel in groupshared memory, then add the previous frame.
  - The atlas is 2560×1585 with 8×8 tiles (10×10 including borders): RGB9E5 color plus RG16F visibility, in DDGI-style storage.
  - 64 MB (30 MB on low spec), 0.08 ms. A single 8 B/texel atlas is about 31 MB, so 64 MB suggests double buffering (derived).
- **Final gather.**
  - 1 cosine-weighted, blue-noise ray per pixel at ½ or ¼ resolution, with no shading in the gather.
  - Cache order: screen-space (if the hit is in the frustum and not occluded), then the world radiance cache, then the irradiance volumes.
  - The result is stored as 2-band SH (3 × RGBA16F), so each pixel is a small irradiance probe.
  - Then a separable bilateral denoise (normal, depth and hit-distance weights), a 4-tap Poisson bilateral upscale, and a temporal running mean capped at n_max.
- **Other systems.**
  - A world-space light grid: 16³ cells × 8 cascades, first cascade 32 m, at most 64 IDs per cell; 30 MB; about 0.2 ms on console.
  - Transparents and fog read a froxel irradiance volume in 2-band SH (50 MB).
  - Directional occlusion restores fine detail that is missing from the BVH.
  - RGB9E5 is preferred over R11G11B10F, which shifts color and has no pure white.
- **Cost on a heavy scene (mission 4), in ms:**

  | Pass | Series S 900p | Series X 1440p | PS5 1440p | PS5 Pro 1800p | PC 4K (4080) |
  |---|---|---|---|---|---|
  | World sampling | 0.38 | 0.27 | 0.45 | 0.083 | 0.091 |
  | Radiance cache | 0.20 | 0.21 | 0.109 | 0.11 | 0.109 |
  | Irradiance volumes | 0.08 | 0.08 | 0.046 | 0.035 | 0.07 |
  | Final gather | 0.60 | 0.535 | 0.489 | 0.385 | 0.50 |
  | Denoise | 0.208 | 0.18 | 0.266 | 0.22 | 0.52 |
  | Upscale | 0.65 | 0.59 | 0.69 | 0.72 | 0.445 |
  | Total (serial) | ~2.11 | ~1.9 | ~2.05 | ~1.92 | ~1.7 |
  | Total (with async compute) | ~1.71 | ~1.4 | ~1.55 | ~1.82 | n/a |

#### [P2] Frostbite GIBS as shipped: *College Football 25* and *Skate*. Apers, *Shipping Dynamic Global Illumination in Frostbite*, SIGGRAPH 2024 Advances.
URL: https://advances.realtimerendering.com/s2024/content/EA-GIBS2/Apers_Advances-s2024_Shipping-Dynamic-GI.pdf. Status: Read (notes).
- **Probes changed to DDGI style.** The clipmap probes moved from hardware-trilinear SH to octahedral tiles with software interpolation and variance-depth attenuation. The attenuation was needed to stop dark probes under stadium stands leaking onto the crowd.
- **Probe budget.**
  - Up to 3 clipmaps and about 100k probes, within 100 MB, so about 1 KB per probe.
  - Irradiance is 6×6 including the border (4×4 interior, 16 directions). Variance depth is 14×14 including the border, because depth resolution mattered more.
  - Arithmetic: 144 + 784 + 96 B of misc = 1024 B (derived).
- **Ray budget.**
  - 100k probe rays per frame. Inline convolution needs at least 16 rays per probe, so only about 6k probes trace per frame.
  - One clipmap is considered per frame. Moved probes get high priority, and probes that see no nearby geometry get low priority.
  - Hysteresis is driven per probe by a Multi-Scale Mean Estimator (no directional statistics).
  - Tracing 1 ray per probe per frame into a back-buffer atlas was dropped: double-buffering depth and copying cost too much.
- **Probes as a cache for surfels.**
  - Surfel rays were shortened from 1000 m to 20 m and look up the nearest clipmap probe in the ray's direction when they miss.
  - Probes also give newly spawned surfels their first estimate.
  - Together: about 60% less surfel time, and much less frame-to-frame variance.
- **Ray tracing scene.**
  - Only opaque, non-alpha-tested geometry. Vegetation is excluded.
  - One simplified albedo per mesh and material.
  - Hits return distance, geometric normal and material ID.
  - Direct light at hits picks one light by reservoir sampling and uses its shadow map when available. That cut probe tracing time by 34%.
  - Inline tracing (DXR 1.1 style) in compute with indirect arguments. Ray binning no longer paid off, but compaction by ray properties still did.
  - Offline-built BLAS gave about 15% faster traversal; selective rebraiding about 5% more.
- **Other fixes.**
  - Spawn surfels with the vertex-interpolated normal, not one reconstructed from depth.
  - Store a per-surfel ray-offset error bound.
- **Cost.**
  - Skate on Xbox Series X at 1440p (about 100k surfels, about 25k probes in 3 clipmaps with reduced height density, 200k-ray cap): probe updates 0.7 ms including RT, all of GIBS 3.2 → 2.5 ms after optimization.
  - College Football 25 freezes GIBS in its 60 Hz gameplay to fit a 2 ms budget (apply-only), and updates fully in 30 Hz modes.
  - Xbox Series S: all of GIBS, including RT resources, under 500 MB.

#### [P3] Ubisoft Anvil: *Assassin's Creed Shadows* (2025).
Sources:
- Leblanc & Conte, *Ray Tracing the World of Assassin's Creed Shadows*, SIGGRAPH 2025 Advances: https://www.realtimerendering.com/advances/s2025/content/Advances%202025%20-%20Raytracing%20the%20world%20of%20Assassin's%20Creed%20Shadows.pdf (14 MB; not read, only abstract and search snippets)
- Ubisoft tech Q&A: https://www.ubisoft.com/en-us/game/assassins-creed/news/4XbPPtFyQEtIMWrA9xVDmZ/assassins-creed-shadows-tech-qa
- AI summary of a video: https://readray.ai/notes/844807

Status: Partial / Secondary.
- **Design (slides, via snippet).** A per-pixel RT pass for diffuse (and later specular) GI. A probe volume acts as the cache for secondary bounces.
  - Probes are convolved into a filtered radiance cache, used when a ray hits nothing nearby.
  - A diffuse irradiance cache is also convolved from the probes, for indirect diffuse at any point.
- **Named difficulties (abstract).** Translucent geometry, thin walls, small windows, dense vegetation.
- **Shipping split (tech Q&A).** A hybrid: RTGI in the fully dynamic Hideout, and Anvil's baked GI elsewhere in the 60 fps console target. PC offers Hideout-only, diffuse-everywhere and diffuse-plus-specular-everywhere modes. Exact per-mode console behavior was not verified.
- **Secondary, unverified.**
  - Probes about every 2 m near the camera, in sparse cascades.
  - Per-pixel rays start in screen space against the G-buffer, then continue in the BVH.
  - Shadow maps replace shadow rays at hits, and dual-sided shadow maps handle thin shoji screens.
  - A miss falls back to the sky or distant probe caches.
  - The BVH uses simplified geometry and materials.
  - A lag artifact appears when a dynamic light turns off.

#### [P4] Ubisoft Snowdrop: *Avatar: Frontiers of Pandora* (2023) and *Star Wars Outlaws* (2024).
Sources:
- GDC 2024 abstracts: https://schedule.gdconf.com/session/advanced-graphics-summit-raytracing-in-snowdrop-an-optimized-lighting-pipeline-for-consoles/900545, https://gdcvault.com/play/1034426/Advanced-Graphics-Summit-Raytracing-in, https://schedule.gdconf.com/session/ray-tracing-in-snowdrop-scene-representation-and-custom-bvh/900070
- Massive interview: https://massive.se/article/snowdrops-ray-tracing-shines-a-light-on-pandora

Status: Partial (abstracts only).
- **Pipeline.** RTGI and reflections traced from the G-buffer, with screen-space tracing first to reduce world-space rays.
- **Probe system.** Probes light forward-rendered objects and also cache GI and reflections. The talk compares grid-based placement with a newer octree-based placement; details unknown.
- **BVH.** A custom acceleration structure on consoles, DXR on PC.
- **Denoisers.** AMD's reflection denoiser, NVIDIA's RT denoisers and two in-house ones.
- **Why ray tracing.** The interview says the old probe GI ignored small objects. Rays come from surfaces, not infinite bounces.
- **Star Wars Outlaws.** No probe details found. NVIDIA material covers RTXDI and Ray Reconstruction on PC.

#### [P5] Unreal Engine titles: *The Finals* (Embark, 2023) and Lumen titles.
URL: https://www.nvidia.com/en-us/geforce/news/the-finals-closed-beta-dlss-reflex-ray-tracing/. Status: Partial.
- *The Finals*:
  - Ships NVIDIA RTXGI (the DDGI SDK [D3], via the UE plugin) with a "dynamic" mode that reacts to destruction, fog and smoke, plus a static mode.
  - The DDGI identification is inference from RTXGI's probe mode; NVIDIA's page names RTXGI only.
- Lumen ([N2]) is the default dynamic GI in UE5 titles. Individual titles were not surveyed.

#### [P6] Activision: *Call of Duty*.
See [N6]. Status: Read (precomputed only).
- **Neural Light Grid.** Irradiance volumes with learned per-probe weighting functions shipped in MW3 and Warzone.
- **No ray-traced runtime probe GI** was found for Call of Duty. The 2021 *Large-Scale GI at Activision* talk was also precomputed lighting.

#### [P7] Remedy Northlight: *Alan Wake 2* (2023).
URL: https://schedule.gdconf.com/session/alan-wake-2-a-deep-dive-into-path-tracing-technology-presented-by-nvidia/903204. Status: Partial.
- **Public material.** Path-traced lighting on PC (opacity micromaps, shader execution reordering, DLSS Ray Reconstruction), with up to 3 bounces in its High mode.
- **Probe GI.** No public source describes a ray-traced probe system in Northlight (unconfirmed either way).

#### [P8] Godot: SDFGI (4.x) and HDDAGI (draft).
URLs: https://docs.godotengine.org/en/stable/tutorials/3d/global_illumination/using_sdfgi.html and https://github.com/godotengine/godot/pull/86267. Status: Read.
- **SDFGI.** Cascaded signed distance fields with probes.
  - Fewer cascades (4 or fewer) when the camera moves fast.
  - A Y scale of 75% or 50% improves quality and reduces leaks at no cost, an anisotropic grid in the vertical axis.
  - Frames to converge: 25 by default (30 for scenes without fast-moving GI lights).
  - Bounce feedback above 0.5 can run away into extreme brightness, which is the same energy problem as RTXGI's albedo cap of 0.9.
- **HDDAGI (draft PR #86267, December 2023; set back to draft December 2024).**
  - Cascades of 16×16 regions, HDDA ray marching, local updates instead of SDF regeneration.
  - Lights update every 2 frames and inactive probes every 3.
  - An optional neighbor filter accounts for occlusion between probes.
  - 0.93 ms against 2.43 ms for SDFGI on a GTX 1650.
  - Its author found DDGI-style octahedral visibility leaked more.
- **No production DDGI** exists in Godot core (as of the sources read).

#### [P9] *Cyberpunk 2077* and SHaRC.
URL: https://wccftech.com/nvidia-rtxgi-2-0-available-next-frontier-ray-traced-visuals-neural-radiance-cache-spatial-hash-radiance-cache-dynamic-diffuse-global-illumination/. Status: Secondary.
- NVIDIA's RTXGI 2.0 material (as reproduced by Wccftech) showed SHaRC widening GI coverage in a *Cyberpunk 2077* scene at small cost. NVIDIA's current RTXGI page names no games.
- It is not confirmed that SHaRC shipped in Overdrive mode. NRC in Cyberpunk was a 2023 rumor.

### A4. Apple silicon and mobile

#### [A1] Apple tech talk *Explore GPU advancements in M3 and A17 Pro* (2023).
URL: https://developer.apple.com/videos/play/tech-talks/111375/. Status: Read (transcript).
- **Ray tracing unit.** Every shader core has one, with fixed-function traversal and intersection.
  - A reorder stage regroups intersection-function invocations from rays that arrive close in time and space, even across SIMD groups.
  - Ray and payload data pass through on-chip memory, visible as "RT scratch" counters in Xcode.
- **Use the intersector API.**
  - Apple's guidance is to prefer the intersector object API.
  - The intersection query API increases ray-trace scratch traffic and disables the reorder stage (paraphrase of the transcript).
- **Payload.** Keep the ray payload small, for lower latency and possibly higher occupancy.
- **Intersection functions.**
  - Opaque triangle geometry runs no shader code.
  - Custom intersection functions run on the shader cores.
  - Write one function per logical routine, not one uber-function.
- **Dynamic Caching.** Registers, threadgroup, tile, stack and buffer memory share on-chip storage, and occupancy adapts automatically. FP16, FP32 and integer work co-issue only across SIMD groups. Prefer FP16.

#### [A2] WWDC22 *Maximize your Metal ray tracing performance* (session 10105).
URL: https://developer.apple.com/videos/play/wwdc2022/10105/. Status: Read (transcript).
- **Per-primitive data.** Small per-primitive data stored inside the acceleration structure (`primitiveDataBuffer`, stride, element size) is readable from intersection results and queries. It gave 10–16% in an Apple test app, so keep it small.
- **Builds.** Build primitive structures at load time and refit deforming meshes (up to 38% faster refits). Rebuild the instance structure each frame.
- **Parallel builds.** Encode many builds on one encoder with separate scratch buffers: up to 2.8× faster for many small structures. Builds are up to 2.3× faster overall on Apple silicon.
- **Instances.** Each instance a ray enters costs a transform and a switch of structure. Merge static sub-meshes into one primitive structure with per-geometry transforms.
- **Compact vertex formats.** Build straight from half floats, normalized integers or `uint1010102Normalized`. Use transform buffers to undo quantization.
- **Residency.** Allocate structures from heaps and call `useHeap` once instead of many `useResource` calls.
- **GPU-driven.** Ray tracing is allowed in indirect command buffers (`supportRayTracing`).

#### [A3] WWDC23 *Your guide to Metal ray tracing* (session 10128).
URL: https://developer.apple.com/videos/play/wwdc2023/10128/. Status: Read (transcript).
- **Instancing.**
  - Indirect instance acceleration structures let the GPU cull instances and write the instance buffer and count with no CPU readback.
  - Multi-level instancing (`max_levels<N>` on the intersector) allows a static/dynamic split. Three levels reduced build time at a small trace cost.
- **Shader access.** The intersector returns `triangle_barycentric_coord` with the `triangle_data` tag, and the instance-ID chain (`instance_count`, `instance_id[i]`) with instancing.
- **Builds.** Compaction (`writeCompactedSize` then `copyAndCompact`) is especially worth it for primitive structures. Refit can run in place.
- **Where rays run.** Apple silicon can intersect in both compute and render encoders; AMD and Intel Macs in compute only.
- **Tools.** Xcode's acceleration-structure viewer has an AABB-traversal heatmap for spotting expensive regions.

#### [A4] WWDC25 *Go further with Metal 4 games* (session 211).
URL: https://developer.apple.com/videos/play/wwdc2025/211/. Status: Read (transcript).
- **New build options.** "Prefer fast intersection" and "minimize memory", set per build.
- **Intersection function buffers.** These map DXR/Vulkan shader-binding-table conventions to Metal.
  - Set instance and geometry offsets.
  - Set the geometry multiplier and base ID (ray type) in the shader.
  - Every thread of a SIMD group must use the same buffer arguments.
- **MetalFX denoised upscaler (Apple9).**
  - Inputs: noise-free world-space normals (signed format), diffuse albedo, roughness, specular albedo.
  - Optional inputs: specular hit distance and a denoise-strength mask.
  - Relevant only if Byte-Engine adds a noisy per-pixel final gather ([P1]-style). Probe sampling itself is already smooth.

#### [A5] Apple *Metal Feature Set Tables* (dated 2026-05-21).
URL: https://developer.apple.com/metal/Metal-Feature-Set-Tables.pdf. Status: Read.
- **GPU family.** M3, M4 and A17 Pro/A18 are Apple9. M5 and A19 are Apple10.
- **Ray tracing.** Available in compute and render pipelines from Apple6, but hardware-accelerated only from M3 (Apple9) [A1].
- **Apple9 features.**
  - Acceleration structure build options.
  - Intersection function buffers.
  - Direct access to on-chip ray-intersection result storage (Metal API name not identified).
  - Per-component motion.
  - Row-major matrices in acceleration structures.
- **Levels.** At most 32 levels for an intersector against at most 16 for an intersection query.
- **Ray tracing and mesh shading.** Ray tracing in render pipelines does not mix with mesh shading in the same pipeline.
- **Formats.** RGB9E5Float and RG11B10Float have all texture capabilities, including shader **Write**, on Apple3 and later. They cannot use lossy texture compression.

#### [A6] Epic, macOS requirements for Unreal Engine.
URL: https://dev.epicgames.com/documentation/unreal-engine/macos-development-requirements-for-unreal-engine. Status: Read.
- Lumen hardware ray tracing (and MegaLights) is Experimental on Apple silicon M2 and later.
- Software-traced Lumen works from M1.
- No Apple-specific probe-GI performance data is published.

#### [A7] Apple M4 family ray tracing claim.
URL: https://petapixel.com/2024/10/30/apples-m4-m4-pro-and-m4-max-chips-promise-huge-performance-gains-and-ai-prowess. Status: Secondary.
- Apple's M4 Pro/Max launch reportedly claimed a ray tracing engine about 2× faster than M3's.
- The baseline and the exact claim were not verified at Apple's newsroom.

#### [A8] Arm, *Mobile Ray Tracing Demystified* (Vulkanised 2026), and Arm's 2021 developer blog.
URLs: https://vulkan.org/user/pages/09.events/vulkanised-2026/Mobile-Ray-Tracing-Demystified-Iago-CalvoLista-Arm.pptx.pdf and https://developer.arm.com/community/arm-community-blogs/b/mobile-graphics-and-gaming-blog/posts/developing-ray-tracing-content-for-mobile-games. Status: Read (slides text) / Partial (blog).
- **Ray queries first.** Ray query is the default on Mali, and RT pipelines are slow on first-generation RT units. Check for the compiler's "slow ray traversal" flag. All rays in a warp should use the same TLAS.
- **Ray flags.** `TerminateOnFirstHit` gave 42.6% on Bistro shadows (Mali G1). Mark geometry and instances opaque and skip AABBs.
- **Builds.** Batch builds, use FAST_BUILD for TLASes and skinned meshes, and refit rather than rebuild.
- **SSR first, then rays.** Trace screen space first. Compact the failed pixels and bin them by direction (sign or octahedral bins) before launching ray queries.
- **"Ray query visibility buffer".** Trace first and store hit data (instance, primitive, barycentrics). Sort the hits by material, then shade each material with its own indirect compute dispatch. This replaces a divergent uber-shader.
- **Hardware.** RTUv2 (Mali G1) beat RTUv1 (Immortalis G925) on shadows-plus-reflections tests: 68% on Sponza (1.94 M rays) and 36% on Bistro (0.75 M rays).

#### [A9] Cao (Tencent Games), *SmartGI: Global Illumination with Space Voxelization on Mobile*. SIGGRAPH 2024, Moving Mobile Graphics.
URL: https://community.arm.com/cfs-file/__key/communityserver-blogs-components-weblogfiles/00-00-00-20-66/siggraph_5F00_mmg_5F00_2024_5F00_SmartGI0722.pdf. Status: Read.
- **Not hardware RT.** A two-level voxel brick structure, ray-marched with HDDA, plus screen probes.
- **Screen probes.**
  - One per 16×16 pixel block, each with 64 directions.
  - 2×2 probes form a virtual group, and only one per group is traced per frame.
  - History probes are reprojected into the current frame, and a probe mip chain covers misses.
- **Cost.** 28–30 MB for a 512 m radius. About 2 ms on 2023 flagship phones; radiosity costs 0.5 ms off and 1.5 ms on.
- **Why not DDGI on mobile.** Probe GI needs precise ray tracing to place and offset probes, and clipmaps waste memory.

---

## Part B. Design implications (ranked by how much each decision shapes the rest of the design)

Each item gives:
- a recommended default (my synthesis);
- the evidence behind it;
- when to deviate.

Byte-Engine context assumed: an existing SSGI, an RT surfel GI, a visibility-buffer renderer with material-evaluation passes, BVHs, and an M4 (Apple9) as the main target.

### 1. Decide the probes' role in the GI stack first: far-field cache and fallback, not the only diffuse term.

**Default.**
- Probes provide the indirect term:
  - for SSGI misses and off-screen rays;
  - at the end of shortened surfel rays;
  - for consumers outside the deferred path (transparents, fog, particles, hair).
- Near-field detail stays with SSGI or the surfels.

**Evidence.**
- Every recent production system layers a screen-space or near-field term over a world cache:
  - idTech 8's final gather checks a screen-space cache first, then a world radiance cache, then irradiance volumes [P1].
  - Lumen runs screen traces first, then world traces, and splits short screen-probe rays from long world-cache rays [N2].
  - GI-1.0 pairs a horizon-based near-field pass with probes [N1].
  - Snowdrop traces screen space first to cut world rays [P4].
  - HDRP's SSGI falls back to APV on a miss [N4].
  - Arm on mobile traces SSR first, then ray queries [A8].
- Frostbite is the strongest data point for Byte-Engine's surfel GI. Making probes the cache at the end of shortened surfel rays (20 m instead of 1000 m) cut surfel tracing time by about 60% and stabilized it. Probes also cover the surfel cold start [P2].
- DDGI's own paper applies screen-space AO to camera pixels, because probes cannot capture contact-scale occlusion [D1].

**Deviate when** the target needs probes to light every pixel with no near-field term (for example on low-end hardware). Then leak control (item 4) and density (item 6) must be much stronger. Expect walls thinner than the spacing to leak [N2][N4].

### 2. Trace visibility only; shade hits in a separate, material-binned pass that reuses the visibility-buffer machinery.

**Default.**
- The probe trace kernel returns a compact hit record: instance, primitive, barycentrics, t and a front-facing flag. idTech 8 packs SBT, instance and primitive IDs, barycentrics and distance into 128 bits [P1].
- Shade the hits in compute passes binned by material, reusing Byte-Engine's material evaluation with a simplified material (albedo, coarse mip).
- Direct light at a hit:
  - use existing shadow maps where a light has them, and a shadow ray otherwise;
  - pick one light by importance (RIS or reservoirs) when there are many.
- Indirect light at a hit = previous frame's probe irradiance × min(albedo, 0.9)/π.
- A miss returns sky radiance.
- Optionally insert idTech 8's hashed world radiance cache (25 cm cells, distance-based level of detail) so neighboring probes and later frames share hit shading.

**Evidence.**
- **Deferred hit shading:** idTech 8 shades about 20k cache entries per frame and reuses each for several frames [P1]. Arm's "ray query visibility buffer" sorts hits by material for the same reason [A8].
- **Shadow maps at hits:**
  - Frostbite: 34% less probe tracing time [P2].
  - AC Shadows uses shadow maps and dual-sided maps for thin screens (secondary) [P3].
- **Simplified materials and opaque-only BVH:** Frostbite uses one albedo per mesh-material subset and no alpha-tested geometry [P2].
- **Albedo cap:** 0.9 stops energy gain over infinite bounces in RTXGI's sample [D3]. Godot sees runaway brightness above a bounce feedback of 0.5 [P8].
- **Same shading as camera pixels:** DDGI's design; glossy energy at hits is folded into diffuse [D1].
- **Metal:** Apple recommends the intersector over intersection queries on M3/M4 [A1].

**Deviate** to inline shading only for tiny scenes. Inline shading makes the trace kernel's register pressure and material divergence the bottleneck [D1][A8].

### 3. Budget rays per frame globally; amortize across cascades and probe states; size hysteresis with the update interval.

**Default.**
- Probe rays per frame in the range consoles have shipped: about 65k–260k [P1], about 100k [P2].
- Update the near cascade often and the far cascades round-robin, ordered by a priority (moved or newly scrolled probes first, probes with no nearby geometry last) [P2].
- 32 fixed, never-rotated rays drive classification and relocation; the rest follow a per-frame randomly rotated spherical Fibonacci pattern [D3].
- Inactive or sleeping probes trace only the fixed rays, at a low rate [D2][D3].
- Per probe per update: 16–64 rays [P1][P2], against RTXGI's 256 default [D3].

**Evidence.**
- idTech 8 updates 1 of 6 cascades plus 1 local volume each frame, at 64, 32 or 16 rays per probe [P1].
- Frostbite traces only about 6k of 100k probes per frame, with at least 16 rays each so inline convolution works [P2].
- Lumen traces 300 world probes per frame [N2].
- Probe sleeping gave 30–50% [D2].
- Adaptive pilot rays beat uniform rays per probe at the same quality [D6].
- DDGI's reference run of 524k rays per frame (every probe, every frame) is an upper bound [D1].

**Derived consequence.** With an EMA of hysteresis α, the time constant is about 1/(1 − α) updates:
- α = 0.97 → about 33 updates;
- α = 0.94 → about 17;
- α = 0.85 → about 7.

A cascade updated every 6th frame at α = 0.97 therefore responds over about 200 frames (about 3.3 s at 60 Hz). Lower α for cascades updated less often, or (better) use the warm-up and change detection in item 5.

**Measure on M4.** No published M3/M4 throughput exists for incoherent probe rays ([D] list). Ray throughput must be measured before picking 16, 32 or 64 rays.

### 4. Keep DDGI's visibility moments, but treat leaks as a stack of fixes, not one test.

**Default.** The RTXGI weight stack as a baseline [D3]:
- trilinear weights, each axis floored at 0.001;
- wrap-shading weight ((dot + 1)/2)² + 0.2;
- Chebyshev visibility, cubed and floored at 0.05, from 16×16 RG16F (mean, mean²) tiles;
- crush weights below 0.2;
- the bias formula from [D2] (B = 0.3, tuned per spacing);
- backface hits stored as irradiance 0 and distance −0.2·t;
- classification (backface ratio > 0.25 means inactive; a probe with no frontface hit in its voxel is inactive);
- relocation capped at 0.45 spacing with a minimum frontface distance.

Also store a per-probe validity mask, so samplers can skip invalid neighbors the way APV's Quality mode does [N4].

**Evidence.**
- DDGI's ablation shows that the backface weight, the visibility test and the normal bias each remove a distinct class of leak [D1].
- Frostbite had to add variance-depth attenuation in production [P2].
- **Known limits:**
  - two moments fail on bimodal depth (GDC 2017 slides) [N11];
  - Godot's author found octahedral VSM leaked more than SDFGI [P8];
  - Chebyshev visibility is discontinuous at very low density [N6];
  - biases tuned for camera pixels leak at secondary hits [D5].
- **Biases in the wild:**
  - RTXGI: normal 0.1 and view 0.1 world units [D3];
  - APV: normal 0.05 m and view 0.1 m [N4];
  - [D2]'s formula: 0.225 m at 1 m spacing (derived).
- **Cache-side leak fixes:**
  - GI-1.0 and Bevy key or force the finest level when the ray is shorter than the cell [N1][N10];
  - kajiya restricts which rays may populate a cell by rank [N9].
- **Thin walls:**
  - Lumen requires ≥ 10 cm walls [N2];
  - Unity says to make walls closer to the probe spacing [N4];
  - Godot recommends finer vertical cells [P8].
- Use the vertex-interpolated (geometric) normal for bias and spawn decisions, not the normal-mapped one (inferred from Frostbite's surfel finding [P2]).

**Deviate** to SDF or per-pixel visibility only if interiors with thin walls dominate. Then the probes become a far-field term again (item 1) [N11][N3].

### 5. Temporal accumulation: warm up, detect change, encode perceptually, damp fireflies.

**Default.**
- Per-texel running mean 1/n up to a cap (n_max ≈ 30–60), then an EMA (α ≈ 0.94–0.98) [D6].
- Empty or scrolled-in texels take the new value directly (hysteresis 0) [D3].
- Irradiance blended in gamma-5 space [D2][D3].
- Change detection:
  - darkening beyond a threshold lowers hysteresis sharply;
  - brightening steps over a luminance threshold are damped (×0.25) to suppress fireflies;
  - a minimum step of 1/1024 when darkening, for low-precision formats [D3].
- Game-event hooks (light switched, time-of-day jump, destruction) lower hysteresis for a few frames and relax TAA with the same rule [D2].
- Do not lower visibility (depth) hysteresis per texel [D2].

**Evidence.**
- **Hysteresis values used:** DDGI 0.85–0.98 [D1]; RTXGI 0.97 [D3]; ADGI 0.98 static / 0.91 dynamic with resets to n = 16 [D6]; Q-DDGI 0.94 [D6].
- **Change detection in [D2]:** a 25% change lowers hysteresis by 0.15; an 80% change sets it to 0. The SDK uses −0.75 at its 0.25 threshold (in gamma space) [D3].
- **Variance-driven blending:** Frostbite's multi-scale mean estimator per probe [P2] and GI-1.0's biased hysteresis [N1] are alternatives.
- **Running-mean caps elsewhere:** idTech 8 (n_max in the final gather) [P1], SHaRC (max accumulated frames plus stale-frame eviction) [D4], Bevy (count cap) [N10].

**Risk.** Previous-frame bounce feedback makes light "flow" and lag after sharp changes [D1]. Small bright lights (flashlights) ghost even with the heuristics [D2][P3].

### 6. Volume topology: camera-centered clipmap cascades with toroidal scrolling and weight-sum blending.

**Default.**
- 4–6 cascades of 16³ (or 32×16×32 near the camera), each doubling the spacing.
- Toroidal addressing: wrapped planes are cleared and refilled through the empty-history rule [D3].
- Sample from the densest volume to the sparsest; stop once the weights sum to 1; fade over the last cell; tighten camera-locked fades by one cell [D2].
- Allow anisotropic vertical density.

**Evidence.**
- **Cascade sizes in production:**
  - idTech 8: 6 × 16³ (12³ low spec) plus local 10³ volumes [P1];
  - Lumen's radiance cache: 4 × 48³ [N2];
  - Frostbite: 3 clips, ≤ 100k probes, reduced density in height [P2];
  - kajiya: 12 × 32³ [N9];
  - [D2]: camera-locked plus city plus level volumes.
- **Hand-placed local volumes** add detail where it matters (idTech 8 allows up to 100) [P1].
- **Memory (derived):**
  - DDGI-style storage is about 1.28 KB per probe (8×8 × 4 B + 16×16 × 4 B) [D1][D3], so 6 × 16³ ≈ 31 MB single-buffered.
  - Frostbite fits about 1 KB per probe with 6×6 and 14×14 tiles [P2].
  - idTech 8's 64 MB atlas is consistent with double buffering [P1].
- **Vertical density:** Godot recommends finer vertical cells [P8]; Frostbite reduces probe density in height [P2]. Choose per game.

### 7. Probe payload: octahedral irradiance at 6×6 interior (8×8 tile) and depth moments at 14×14 interior (16×16); SH only for per-pixel outputs.

**Default.**
- Irradiance: 6×6 interior plus a 1-texel border, in RGB9E5. RGB9E5 is shader-writable on Apple3+ including M4 [A5], and idTech 8 prefers it [P1]. Alternatively RGB10A2 with gamma-5 encoding plus the U32 correction [D3].
- Visibility: 14×14 interior plus border, RG16F.
- Use 4×4-interior irradiance (16 directions) only when memory or rays are tight.

**Evidence.**
- Probe count matters more than angular resolution [D1].
- Frostbite chose 4×4-interior irradiance so it could keep 12×12-interior depth [P2].
- idTech 8 uses 8×8-interior tiles [P1].
- RTXGI's example uses a 6×6 interior [D3].
- **Format problems:**
  - R11G11B10F tints colors (no exact grays or pure white) [D1][P1];
  - RGB5A1 needs α = 0.8 and flickers [D1].
- **SH is used for:**
  - per-pixel final-gather outputs (idTech 8, L1) [P1];
  - interpolation of screen probes (GI-1.0, L2) [N1];
  - small caches (kajiya, L1) [N9];
  - baked probes (APV, L1/L2) [N4].
- **SH is not used for** per-probe visibility: depth stored in SH rings [N11].

**This decision is cheap to change later** (atlas constants). It ranks below the architectural items.

### 8. Metal and M4 implementation constraints that shape the abstraction.

**Default.**
- On Metal, run the probe trace with the `intersector` object in a compute kernel:
  - opaque-only triangle geometry, so no intersection functions run;
  - no payload;
  - `triangle_data` plus `instancing` tags to get barycentrics and the instance chain.
- On Vulkan/DX12 use inline ray queries, opaque flags, and terminate-on-first-hit for shadow rays [A8].
- If Byte-Engine's cross-API abstraction is shaped like ray queries, keep a Metal path that maps to the intersector rather than `intersection_query` [A1].

**Evidence.**
- Intersection queries add scratch traffic and disable the M3+ reorder stage [A1]. They also cap traversal depth at 16 levels against 32 for the intersector [A5].
- Opaque geometry avoids intersection-function shader work [A1].
- Per-primitive data in the acceleration structure gave 10–16% [A2]. Useful for carrying a material ID or simplified albedo into hit shading.
- Refit deformables, rebuild the TLAS per frame, and run parallel builds with separate scratch buffers [A2].
- GPU-driven indirect instance structures are available [A3].
- Metal 4 adds per-build "prefer fast intersection" and "minimize memory" options on Apple9 [A4][A5].
- No published probe-GI numbers exist for Apple GPUs. Lumen's hardware-RT path on macOS is still Experimental [A6]. The claimed 2× M4 ray tracing speedup is unverified [A7].

**Open API detail.** Backface detection is required for DDGI's backface encoding.
- On `intersection_query` it is `is_committed_triangle_front_facing()` (seen in SPIRV-Cross MSL output).
- Whether intersector results expose a front-facing field was not verified.
- Fallback: compute it from the hit triangle's geometric normal and the ray direction, which the hit record allows anyway.

---

## Part C. Numbers at a glance

| System | Probe or cache layout | Rays per update | Update schedule | Temporal | Measured cost |
|---|---|---|---|---|---|
| DDGI 2019 [D1] | 8×8 irradiance, 16×16 RG16F depth | 64 (largest scenes) | every probe, every frame | α 0.85–0.98 | 2.6 ms at 32×8×32 (2080 Ti) |
| DDGI 2021 / RTXGI [D2][D3] | 6×6+border irradiance, 14×14+border depth | 256 default (32 fixed) | states and sleeping (30–50% gain) | 0.97, γ 5, thresholds | compute update 3× faster than pixel shader |
| idTech 8 [P1] | 8×8 (+border) RGB9E5 + RG16F; 6 × 16³ cascades | 64/32/16 | 1 cascade + 1 local volume per frame | running mean (final gather) | ~2 ms total GI on consoles; volumes 0.035–0.08 ms |
| Frostbite GIBS [P2] | 6×6 irradiance, 14×14 depth (incl. borders); ≤ 100k probes, 3 clips | ≥ 16 | ~6k probes/frame by priority | multi-scale mean estimator | probes 0.7 ms (XSX 1440p); GIBS 2.5 ms total |
| Lumen [N2] | screen: 8×8 per 16 px; world: 4 × 48³, 32² texels | 64 per screen probe; 1024 per world probe | 300 world probes/frame | update-speed knob | ~8 ms GI + reflections, Epic preset, 1080p console |
| GI-1.0 [N1] | 8×8 hemispherical per 8×8 tile; hash-grid world cache | ¼ of tiles per frame | rays reallocated to empty tiles | biased hysteresis (≤ 0.95) | 1.9–3.4 ms at 1080p (6900 XT / 3080) |
| SHaRC [D4] | 2²² × 40 B hash entries | ~4% of pixels | resolve plus stale eviction | max frame count | not stated |
| ADGI [D6] | 26-bit color + count; 13+13-bit depth | pilot 8 per probe plus adaptive | Markov chains | 0.98 / 0.91 with N_max | 15.6 vs 26.8 ms (2.36 M probes, 2080 Ti) |
| kajiya [N9] | 12 × 32³ clipmaps, L1 SH | 4 + 4 + 16 per entry | all live entries | ReSTIR reservoirs | 0.55 ms (6800 XT) |
| Bevy Solari [N10] | hash grid, 0.15 m finest cells | 1 light + 1 cosine per cell | ≤ 40k random cells | 1/min(n, cap) | Bistro: 2.65 → 0.47 ms with the 40k cap (GPU not stated) |

---

## Part D. Not verified, or not found

- **Lumen 2022 slides** (over 10 MB) were not read. Lumen figures are UE 5.4.4 code defaults. Not confirmed:
  - the clipmap growth rate;
  - where screen-probe rays hand over to the radiance cache;
  - that world probes are placed only near screen probes (an Epic forum post seen in a search snippet).
- **AC Shadows slides** (14 MB) were not read. Not verified:
  - probe spacing "about 2 m, cascaded";
  - shadow maps at hits and dual-sided shadow maps;
  - a miss falling back to sky or distant probes.

  These come from an AI summary of a video. Only the cache roles and named difficulties come from the slides, via search snippets.
- **Snowdrop GDC 2024 talks:** only abstracts. The grid against octree probe placement details and the per-pass costs are unknown. *Star Wars Outlaws*: no probe-GI source found.
- **Northlight / *Alan Wake 2*:** no source on ray-traced probe GI.
- **Call of Duty:** no runtime ray-traced probe GI found. The Neural Light Grid is precomputed.
- ***Indiana Jones*:** covered only as "an early iteration" of idTech 8 GI [P1]. The talk Sousa cites, "Vahl2025, Pushing 60hz. Shipping Indiana Jones and The Great Circle", was not found.
- **SHaRC in *Cyberpunk 2077*:** known only from Wccftech's reproduction of NVIDIA material. Not confirmed shipped.
- **Split Radiance Cascades (2026):** only the abstract. No timings, layout or DDGI comparison numbers.
- **GIBS 2021 slides** (16 MB): not read. The details of the multi-scale mean estimator are unverified.
- **DDGI 2021 timing tables** are images and were not extracted. The arXiv text was used, and it may differ slightly from the JCGT version.
- **Apple-specific gaps:**
  - No published DDGI or probe-GI timings on M3/M4 or any Apple GPU, and no Apple guidance specific to probe GI.
  - The M4 "2× ray tracing engine vs M3" claim is from secondary reports only.
  - The Metal API behind "direct access to on-chip ray-intersection result storage" (Apple9) was not identified.
  - Whether `intersector` results expose a front-facing flag was not verified.
- **Unity APV:** default values for the validity threshold, virtual-offset and dilation settings were not found.
- **Light-field-probe GDC 2017 slides** (VSM leaks with bimodal depth) and **SDF-DDGI** (thin-object leaks): from search snippets only.
- **Ramamoorthi & Hanrahan 2001** (9-coefficient irradiance SH): the site was unreachable, so its error figures were not quoted.
- **Derived values:** the conversion of RTXGI's γ-space thresholds to linear behavior, and all items marked (derived), are my arithmetic.
