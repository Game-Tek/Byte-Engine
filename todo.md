# P0 - Correctness and stability

- Prevent SSGI temporal history overflow after large exposure changes in `ssgi-temporal.besl`. RGB 2 rescaled by a 16-stop increase produces 117964.8 after blending, exceeding RGBA16F's finite range; infinity persists into later frames. Bound or reject non-finite rescaled history and add an exposure-jump regression.

- Fix the Cube test hang and gate renderer/window integration tests so normal test and `cargo llvm-cov` runs complete.
- Give each in-process channel connection its own server salt. `crates/byte-engine/src/network/server/channel.rs` uses `client_salt ^ 0x4254_4550` as the connection id, so the id depends on the client alone.
- Release or present an acquired swapchain image when its extent is `0` or at least 65535. `crates/byte-engine/src/rendering/renderer/core.rs` drops the `PresentKey` and stores `None`, so a minimized window keeps the image acquired and later acquires fail.
- Reject glTF and FBX URIs that escape the asset root. `resolve_gltf_uri` returns absolute paths and `..` joins unchanged (`crates/resource-management/src/asset/handler/implementations/gltf/io.rs`), and `read_asset_from_source` opens `base_path.join(url)`. On Unix an absolute URI replaces the base. FBX textures take the same path. Environment maps already reject this.
- Make Vulkan `set_frames_in_flight` grow and shrink without sharing resources or panicking. Lowering the count hits `unimplemented!()` in `crates/ghi/src/vulkan/context/traits.rs`. Raising it extends image and synchronizer chains by one node and does not rebuild dynamic buffers, so two sequences can share one `VkBuffer`. The renderer currently stays at 2, which matches context creation.
- Copy retained bytes on GHI buffer resize, and free Vulkan image memory when an image is resized. `crates/ghi/src/vulkan/context/resources.rs` still has the copy todo, asserts when the buffer has staging, and can delete the old `VkBuffer` after only one sequence fence. Image resize leaves the old `VkDeviceMemory` allocated until the context drops. Metal and DX12 also replace a resized buffer without copying the previous CPU contents.
- Rebuild Metal dynamic resources when the frame count changes. `set_frames_in_flight` in `crates/ghi/src/metal/context/traits.rs` only retires upload slots and resizes `internal_upload_queues`. Descriptor sets, synchronizers, and dynamic buffers stay chained at their old length, so `nth_handle` reuses the last node.
- Fail DX12 pipeline creation when no native pipeline state exists. `crates/ghi/src/dx12/context/pipelines.rs` still returns a handle, and later dispatch records no work when `pipeline_state` is `None`.
- Add `else` to BESL, then remove the SSGI workarounds for it from `crates/byte-engine/assets/rendering/visibility/ssgi-*.besl`. It needs the parser or lexer, the VM, and the MSL, HLSL, and GLSL backends.
- Remove the GLSL f16 vector conversion helpers (`F16_VECTOR_CONVERSIONS` in `crates/resource-management/src/shader/besl/backends/glsl/generator.rs`) once NVIDIA fixes its Vulkan shader compiler. A vector `OpSpecConstantOp FConvert` from 32- to 16-bit floats crashes pipeline creation on drivers 615.71.09 and 595.44.16; glslang emits it when a specialization constant, such as a material's static color, is narrowed to an f16 vector, so the helpers route the conversion through a function call. Check a new driver with the 15-line repro `e.spvasm` and the `nvrepro` program in `~/tools/nvrepro` on the Linux box; see "Narrow f16 vectors through functions in GLSL" in `docs/develop/besl/compilation.mdx`.
- Return a parse error for a zero-length BESL array. `u32[0]` is accepted, then `Node::array` expects a non-zero size in `crates/besl/src/lexer/ast.rs`.
- Emit `vec3u16` raster I/O as a non-interpolated integer on HLSL, GLSL, and MSL. `is_integer_type` lists the 2- and 4-component forms and omits `vec3u16`, so the stage output is interpolated or the backend compile fails.
- Make Linux audio pause return an error when the device does not support ALSA pause. `crates/ahi/src/os/linux.rs` uses `pcm.pause(true).unwrap()`, and `play` panics for a channel count other than 1 or 2. On Windows, reject a closest match the renderer cannot play, request 32-bit streams as float rather than integer PCM, and treat `CoInitializeEx` returning `S_FALSE` as already initialized. `play` panics outside 16- or 32-bit mono and stereo, and a 32-bit stream is requested as `KSDATAFORMAT_SUBTYPE_PCM` while samples are written as `f32`.
- Give the Vulkan and DX12 GHI backends native per-layout descriptor sets, as Metal has. Both take `ghi::ShaderDescriptorSet` and `ghi::DescriptorSetLayout` now, but `create_shader` flattens the used resources into one flat-slot list (`shader::flatten_used_resources`), `create_descriptor_set` ignores its layout, and each pipeline still materializes its own native descriptors from retained flat-slot writes. Create one native set or descriptor table per `DescriptorSetLayout`, share it between pipelines, check writes against the layout, and match bound sets to a pipeline's layouts by `DescriptorSetLayout::identity` (`crates/ghi/src/metal/command_buffer/encoding.rs` is the reference). The GLSL and HLSL BESL backends still emit one flat set or space; they need a set or space per layout, in the order `evaluation::collect_set_nodes` returns, which is also the order of `ShaderInterface::sets`. Vulkan set numbers are shared by a pipeline's stages, so either number sets per pipeline and patch the SPIR-V decorations, or require stages to list their layers in one order. The signature changes in `crates/ghi/src/vulkan` and `crates/ghi/src/dx12` were written without building either backend; compile and run both first.
- Fix the macOS `NSWindow canBecomeKeyWindow` warning.
- Stop the visibility pipeline from panicking past 1,024 resident primitives. `assert!(active_index < MAX_INSTANCES)` in `crates/byte-engine/src/rendering/pipelines/visibility/manager.rs` counts every resident primitive of every ready renderable before culling, so a level of a few hundred multi-primitive props crashes. Cap or grow the instance buffer and report what didn't fit, as lights do.
- Build byte-engine without the `headed` feature again. `crates/byte-engine/src/gameplay/pathfinding/navigation.rs` imports `rendering::debug` unconditionally. Then check that a headless spawner ignores the rendering level kinds.
- Define texture usage semantics for resources consumed by multiple unknown render passes.
- Give UI element paths an identity that cannot repeat between live scopes. `RetainedTree::begin_frame` clears `path_counts`, and `scope_path` assigns the ordinal from that per-frame counter (`crates/byte-engine/src/ui/layout/retained_tree.rs`). Two mounts with the same name under one parent started on different frames share a path, share element ids, and removing one removes the other's elements. Task ownership already uses `ScopeId` and is unaffected.
- Upload only the bytes a frame wrote to a Vulkan dynamic buffer. `Frame::get_mut_dynamic_buffer_slice` in `crates/ghi/src/vulkan/frame.rs` queues a copy of the frame copy's whole staging buffer into its device buffer. The UI primitive, mask, curve, and band buffers are dynamic (`crates/byte-engine/src/ui/render_pass.rs`), so a frame that writes them uploads about 14.7 MB of primitives plus about 4 MB of masks, curves, and bands per sink. Map host-to-device dynamic copies directly so they need no staging copy, or give the UI a smaller per-frame footprint instead, such as a ring suballocation of one buffer that holds only the live primitive count, which also removes the extra 14.7 MB per sink that Metal keeps for the second frame's copy.
- Implement the loader-context contract on DX12 so Windows builds again. `crates/ghi/src/lib.rs` requires `Context`, `DetachedImage`, and `SharedBuffer` to be `Send`, but the DX12 context holds raw `*mut u8` mappings and an `IDxcCompiler3` (`crates/ghi/src/dx12/context.rs`). `FactoryImage` is a parameter bag that interning turns into a committed resource (`crates/ghi/src/dx12/frame.rs`); replace it with `export_image` that moves the `ID3D12Resource`, its CPU copy, and its `image_states` entry, and `intern_image` that adopts them. Add `share_buffer` and `import_buffer`, delete the factory image and sampler path and its test in `crates/ghi/src/dx12/mod.rs`, and make device-wide idle waits cover every context's queues. A `TRANSFER` selection maps to a `DIRECT` queue today, which is enough for an independent loading timeline. Verify with `cargo check -p byte-engine-ghi --target x86_64-pc-windows-msvc` and the GPU tests on Windows.

# P1 - Runtime performance

## Frame and rendering

- Measure Quarry mesh loading and LOD residency against a stable baked resource store after the instance-lifecycle startup fix. Check competition between mesh refinements and full texture chains in the four shared loader lanes, the material-readiness gate on LOD demand, and the repeated reads and conversion from one-level-at-a-time refinement. Deferred at the user's request; do not change loading or residency policy as part of the instance-lifecycle fix.

- Reuse the fetched taps when ray-traced indirect diffuse reconstruction retries with the triangle-plane normal, without weakening either normal or tangent-plane rejection. The covered-facet fix raises material evaluation from 2.0058125 to 2.1334795 ms (+6.36%) on Sponza's close lion view at 1920 × 1080 on an Apple M4; frame time rises 0.84%, within the runs' spread, so it does not yet meet `AGENTS.md`'s no-regression bar. Keep the three `surfel_reconstruction_*` regressions and repeat the native A/B/B/A timing and square/wide captures after optimizing.
- Use Metal's native sampler reduction on Apple10 GPUs. A BESL `sampler` with `reduction: min` or `max` always reduces in shader code on Metal (`_besl_downsample_*` in `crates/resource-management/src/shader/besl/backends/msl/emit.rs`), because a baked shader doesn't know its GPU family. On Apple10 a `constexpr sampler(..., reduction::minimum)` reads one texel instead of four at levels above 0, which occlusion culling does for every meshlet. Bake an Apple10 variant, or pick the reduction at pipeline creation with a function constant, and A/B the culling and sun-visibility stage counters.
- Elide redundant barriers in the Vulkan and DX12 resource trackers as the Metal tracker now does. `MetalResourceTracker::plan` in `crates/ghi/src/metal/synchronization.rs` logs every encoder barrier it hands out, stamps each state with the log length when it was recorded, and skips a same-encoder hazard that a logged barrier already orders: one whose `after` stages hold the state's stages, whose `before` stages hold the use's stages, and, for a read of a write, that carried `Device` visibility. The Vulkan tracker's `vkCmdPipelineBarrier` and the DX12 tracker's `ResourceBarrier` are issued per hazard the same way, so the interleaved screen-space passes in `crates/byte-engine/src/rendering/pipelines/visibility/render_pass.rs` pay a barrier per stage there. Vulkan needs the equivalent with execution plus memory dependencies, where a logged barrier covers a hazard when its source stages and access hold the state's and its destination stages and access hold the use's; DX12 needs it for UAV barriers, where transitions still change state and cannot be skipped. Neither can be verified on this Mac; test with the tracker unit tests ported from the Metal ones, then a GPU run on each platform.
- Reuse the slots that `export_image` empties. `ResourceCollection::take` (`crates/ghi/src/frame_resources.rs`) leaves the slot empty and never hands its index out again, so the loader context's image list grows by one entry per loaded image for as long as an application streams. Add a free list with generation-checked handles when image destruction or texture streaming lands.
- Check, on a driver that exposes `VK_EXT_descriptor_heap`, the barrier `Context::expect_shared_buffer_writes` adds to the first use of each shared geometry buffer in a frame. NVIDIA's 615.71 driver on an RTX 3060 exposes the extension, and there `cargo nextest run -p byte-engine-ghi --test rendering --ignore-default-filter` passes with validation, `cargo nextest run -p byte-engine --test loading --ignore-default-filter` passes, and the `retained_work` benchmark runs. Mesa 26.1 RADV still lacks the extension, so no test that creates a pipeline runs there.
- Give GHI factory shaders a retention contract. `Factory.shaders` only grows on Metal (`crates/ghi/src/metal/factory.rs`) and Vulkan, and each development rebake of a shader adds a new native library or module without freeing the old one. On Metal, a closest-hit shader also keeps a binary function for each set of specialization constants a ray tracing pipeline used it with (`Shader::binary_functions` in `crates/ghi/src/metal/pipeline.rs`). `PipelineManagerServer` reuses one handle per prepared shader, so steady-state growth is bounded by the distinct shaders; add `destroy_shader` or content-addressed factory shaders when shaders stream or rebake often.
- Add allocation instrumentation and budgets for steady-state frames, GHI recording, audio callbacks, and BELD peak memory before large optimization work.
- Extend the existing shared frame allocator into GHI for frame-local scratch allocation.
- Add borrowed GHI frame allocators with two lifetime classes: CPU scratch reset after submission and retained frame-slot storage reset only after its `FrameKey` completes.
- Route backend recording scratch through those allocators: Vulkan semaphore/copy/barrier data, Metal resource/binding/attachment/push-constant data, and DX12 pipeline/binding/queue data.
- Keep Metal finished-command-buffer updates, native object ownership, readback state, and completion data out of CPU scratch.
- Change GHI collection-returning APIs such as `transfer_textures` to accept caller-provided storage or allocator-aware output.
- Prefer `SmallVec` for normally tiny GHI collections such as swapchains, command buffers, descriptor sets, present drawables, and semaphores.
- Replace Vulkan command recording's full device state-map clones with immutable base state plus recording-local changes, and clean up the transition implementation. Recordings already plan into vectors they reuse (`PlanningStorage` in `crates/ghi/src/vulkan/command_buffer/transitions.rs`), but every batch that records barriers still allocates its image and buffer barrier vectors, and `PlannedTransitions::update_buffer_state` allocates a new range vector for each buffer it touches.
- Replace render-target linear lookups with direct `(SinkId, ResourceId) -> ImageIndex` maps and per-sink image lists.
- Replace the temporary simple and visibility pipeline renderable-transform maps with proper retained component storage.
- Precompute render-pass resource access and attachment templates instead of rebuilding hash maps and vectors each frame.
- Remove material names from non-debug visibility builds.
- Add reusable or allocator-aware listener draining.
- Replace generated meshlet membership scans with fixed-capacity local storage and a generation-tagged global-to-local lookup, or consistently use meshopt.
- Coalesce visibility instance render calls into one dispatch over a compact instance and meshlet work list.
- Sort visibility and transparent work by camera distance where required.
- Select mesh detail levels per view in `meshlet-task.besl` instead of once per instance on the CPU (`VisibilityPipelineManager::rebuild_active_instances`, `crates/byte-engine/src/rendering/pipelines/visibility/lod.rs`). Shadow views draw the main camera's level today, so a caster far from the camera casts a coarse shadow even where that shadow lands near the camera, and a distant cascade draws more detail than its texels show. Per-view selection needs each primitive's level table on the GPU and work items sized for the largest level.
- Write exponential height fog in place into `main` instead of into a replacement. `ExponentialHeightFogRenderPass` (`crates/byte-engine/src/rendering/render_passes/height_fog.rs`) calls `create_main_render_target`, so with no `ExponentialHeightFog` created, or with the pass bypassed, it still copies the whole frame through `ImageBypassPass` to forward `main`. That costs as much memory traffic as fogging it, plus one full-resolution target per sink. Bind `main` with `render_to("main")` the way particles do (`crates/byte-engine/src/rendering/particles/manager.rs`), read it with `image_load` in `crates/byte-engine/assets/rendering/height-fog.besl`, and record nothing when there is no fog. Delete the `main_copy` field and the custom `bypass`. First confirm that the scene color format, `RGBu11u11u10`, supports read-write storage-image access on Metal, Vulkan, and DX12; if a backend doesn't, keep the replacement on that backend. Check that the inspector can still capture the fogged `main` by name, then verify in the Sponza model viewer that the fog looks unchanged and that removing the fog costs no full-screen pass.

## UI

- Retire completed physics ray queries and responses from the Inspector catalog, and query scene entities before materializing unrelated diagnostics. NPC sight queries still grow the catalog over time. `DefaultInspector::entities` lists only entities with a `transform` property as a temporary patch, which hides entities without one, such as input actions; remove the patch once the catalog retires message handles.
- Reuse UI layout, draw-list, geometry, vertex, index, text, relation, and batch storage instead of rebuilding and cloning it each frame.
- Replace UI layout relation and element scans with an ID index and contiguous parent-to-children adjacency storage.
- Replace hit-test `Vec<Vec<usize>>` buckets with contiguous candidate storage plus per-cell ranges.
- Return references or compact handles for UI primitive shapes instead of cloning them.
- Drive multiple animations concurrently using a single animation driver.

## Physics

- Group contacts by time of impact or use collision substeps so every body is not advanced once per contact.

## Audio and input

- Resolve named input triggers to handles during action registration and index device classes and triggers by name.
- Decide how modifier keys behave across input sinks: a sink that captures `Ctrl` currently claims it, so a later sink's `Ctrl+S` no longer resolves. Consider modifier-only controls that sinks read without claiming.
- Input bindings: add action sets or contexts with priorities beyond one sink per consumer, composites (four keys to one 2D vector), hold and tap interactions, dead-zone processors, per-seat bindings for local multiplayer, and a warning when two actions in one sink share a trigger.
- Input bindings: give a controls screen a way to read the next pressed trigger without keeping an extra sink that must pull every tick.

## Metal-specific

- Patch a descriptor set's resource uses when one entry changes instead of building them again. `set_resource_uses` in `crates/ghi/src/metal/command_buffer/encoding.rs` walks the set and `DescriptorUses::new` sorts the result once per changed set and per way a pipeline uses it. With 1,024 different textures that is about 13 µs on an Apple M4 when the textures are already in handle order, and the sort alone takes 28 µs when they are not, for each material variant that binds the table while textures stream in. `Context::resource_to_descriptor` already says which entries name a resource, so the entries of the resources a write touched can be replaced in the sorted table; share one table per large array between the variants that use it the same way.
- Track a large read-only descriptor table as one access. Every encoder that binds a 1,024-texture table plans and records 1,024 reads, about 12 µs on an Apple M4 (`streamed_texture_in_loaded_table_completed` against `streamed_texture_shared_by_pipelines_completed`). Register the table once per encoder in `MetalResourceTracker` and let writes look their resource up in the registered tables.
- Batch pending Metal buffer and texture uploads into one transfer command buffer and blit encoder.
- Avoid cloning Metal texture staging data and pipeline state during upload and descriptor binding.
- Simplify Metal frame-chain handle deduplication if frame counts grow beyond the current small fixed count.

# P1 - Bake and asset performance

- Free idle offline GPU pyramids. `GPUMipProcessor` in `crates/resource-management/src/resources/mips/gpu.rs` keeps every pyramid it creates because the GHI has no `destroy_image`/`destroy_buffer`; `GpuProcessor::MAX_BATCH` bounds the count per extent. Add GHI resource destruction, then evict least-recently-used idle slots against a byte budget next to the bake memory budget. Then free the staging buffer `create_lookup_texture` (`crates/byte-engine/src/rendering/lookup_textures.rs`) creates for each lookup texture once its copy returns.
- Batch environment-map bakes the way material mips are batched. `GPUIBLProcessor` in `crates/resource-management/src/ibl/gpu.rs` records one workload per request in `GpuProcessor::process`; give it per-request scratch and record a batch in one workload, so several HDR images share one submission and one wait.
- Replace asset-handler tuple results with an allocator-backed or borrowed `BakedAsset` payload that storage can consume before arena reset.
- Redesign `MeshProcessorSession` as a two-pass packer that computes offsets and writes directly into one final allocation.
- Let glTF parsing borrow GLB and external BIN data instead of copying whole buffers.
- Flatten glTF traversal into one caller-owned primitive record buffer rather than separate tree and primitive collections.
- Bake each unique glTF material once, resolve primitive material references concurrently, and reuse generated resources.
- Move generated glTF and FBX material factors (base color, metallic, roughness, emission, normal scale, occlusion strength) out of the BESL source into per-material GPU data, so materials that differ only in factors share one compiled shader. Generated shaders are already shared per graph (`store_generated_shaders`), but factors are still written into the program as literals. Specialization constants can carry them, but each distinct value still compiles its own pipeline; a factor array in the visibility `Material` struct, filled from variant variables, would let them share one.
- Extract independent glTF primitive attributes concurrently before constructing the mesh source.
- Run independent BEMA shader loading/compilation and material/variant variable resolution concurrently while preserving order.
- Make a `.bemx` or `.bema` edit reach the screen faster than about 1 s in debug runs. Each material's shader carries the engine's whole shading program, about 117 KB of MSL, so BESL lowering (about 150 ms, 114 ms with BESL and resource management optimized) and the offline `metal` compile (about 380 ms, 267 ms of it parsing and checking) scale with engine code, not with the material. Compile the engine's shading code once and link each material's evaluation function against it, for example with Metal function linking, so an edit compiles only the material's own code. The watcher's 250 ms debounce adds a quarter of that; keep it, since some editors save in bursts.
- Compress generated image mip levels concurrently after mip-chain generation.
- Drain BELD's buffered task stream directly instead of collecting unit results.
- Replace formatted mip stream names with typed stream identifiers or prefix-plus-index metadata.

# P2 - Platform and feature completeness

## GHI and windows

- Complete DX12 command recording and device support for resources, pipelines, uploads, mesh shading, fences, and submission.
- Implement Vulkan standalone command-buffer execution.
- Share the backend-neutral ray tracing rules between the Metal and Vulkan GHI backends before DX12 copies them a third time. `BuildState`, `stale_handle`, `index_size`, the compaction epoch rule, and about 25 contract checks (geometry buffers, instance counts, one build per frame, hit-group sets, binding table records, launch grids) repeat in `crates/ghi/src/metal/` and `crates/ghi/src/vulkan/` with only the backend name changed. Move them into `crate::rt` and `crate::pipelines::ray_tracing` as plain functions that take the backend name, as `rt::unsupported` does, and call them from both backends. Metal doesn't compile on Linux, so do it on macOS and run the ray tracing tests on both.
- Reuse Vulkan shader binding table buffers and compaction query pools. Each `write_shader_binding_table` creates a dedicated buffer and retires the one it replaces (`crates/ghi/src/vulkan/context/ray_tracing.rs`), and each batch of bottom-level builds creates and destroys a query pool (`crates/ghi/src/vulkan/command_buffer/operations/acceleration_structures.rs`). Surfel GI rewrites its table whenever materials change and builds bottom levels while meshes stream, so keep retired buffers and pools until their frame completes and reuse one that fits.
- Port acceleration structures, ray tracing pipelines, and shader binding tables to DX12. It reports `Context::supports_ray_tracing` as `false` and stops in `rt::unsupported` at every acceleration structure, ray tracing pipeline, binding table, and `trace_rays` call, and its `RayTracingPipeline` type is uninhabited. Its earlier code, which wrote buffer addresses where bottom-level structure addresses belong and hard-coded binding table record sizes, was deleted. Follow the contract in the ray tracing section of `docs/reference/ghi.mdx` and the Metal and Vulkan designs in `docs/develop/rendering/ray-tracing.mdx`: one top-level copy per frame in flight, compaction behind a stable handle, deferred destruction, a hit record of the instance's hit group offset plus the geometry index, which is DX12's `InstanceContributionToHitGroupIndex` with a geometry multiplier of 1, binding table writes that leave traces recorded earlier with their records, and launches over exactly the launch size. Vulkan runs each closest hit as a callable shader that the ray generation shader calls through the binding table after a ray query (`crates/ghi/src/vulkan/ray_tracing.rs`); DX12 state objects take callable shaders too. The HLSL BESL backend then lowers the ray tracing stages and intrinsics as the GLSL backend does, and the ray tracing pipeline builder takes a payload size if DX12 needs one. Verify with the tests in `crates/ghi/tests/rendering/ray_tracing.rs`, which are ignored on Windows.
- Add bottom-level acceleration structures that refit, for skinned meshes. Every bottom-level structure is built once with `PreferFastIntersection` and then compacted (`primitive_descriptor` in `crates/ghi/src/metal/acceleration_structures.rs`). Skinned geometry needs a structure that prefers fast builds, skips compaction, and refits from the skinned positions every frame.
- Compact bottom-level acceleration structures on contexts that never start a frame. Compaction advances only in `start_frame` (`advance_acceleration_structures` in `crates/ghi/src/metal/context/resources/acceleration_structures.rs`), so structures built by recordings outside frames keep their built size until a frame starts on that context.
- Build bottom-level acceleration structures on the loading timeline. Builds run on the render context within a per-frame triangle budget, because acceleration structures cannot move between contexts: that needs export and intern for them, as images have, and a loader queue that can run builds.
- Decide how GHI should handle potentially unused staging buffers.
- Keep resource I/O files open across streaming steps. The loader opens a texture's native file for each load and closes it when that load's reads finish (`finish_reads` in `crates/byte-engine/src/rendering/loading/loader.rs`), so a texture that loads its coarse mips and then its full chain opens the file twice, and finer streaming would open it more. Keep a bounded cache of open `ResourceIoFileHandle`s keyed by path, 128 or so to stay well under the 256-file default, that reuses an open handle and closes the least recently used one only while no read of it is in flight.
- Implement macOS cursor visibility and confinement.
- Wire real platform input seats across X11, Wayland, Win32, and the byte-engine input collector.

## Engine systems

- Skip the render targets of visibility features a project leaves out. With `VisibilityFeatures` turning off SSGI or contact shadows, `create_ssgi_targets` (`crates/byte-engine/src/rendering/pipelines/visibility/render_pass/ssgi.rs`) and the contact trace target in `create_sun_visibility_targets` are still allocated per sink.
- Light the first frames after launch with surfel GI on. Since `08a64beec`, a pixel that no surfel lit and that no light on the screen reaches gets none, so every surface the sun doesn't reach is exactly black for the first 26 to 90 frames (measured in the Sponza model viewer on 2026-11-01, raw `Lit` on the shaded floor; it settles near 0.1 and stays there). Bringing back the environment's unoccluded irradiance as a fallback would return the white flash that commit removed, so find a fill that is safe in enclosed rooms: for example, hold presentation or the GI's contribution until the first surfels have their rays, or weigh a fallback by an occlusion estimate. Measure the launch frames and that commit's 60° gallery turn before and after.
- Decide whether the fitted ACES paths stay. The `aces2-color-grading` pass (`crates/byte-engine/assets/rendering/color-grading/aces2.besl`) now renders the official ACES 2.0 Rec.709-D65 100-nit sRGB-piecewise transform from tables computed offline by `scripts/generate-aces2-tables.py`, and agrees with OpenColorIO within 0.002. The `aces` tone mapper and the `aces-color-grading` pass still use Hill's fit. Retire them, or move them onto the ACES 2.0 transform, and validate against the Academy's golden images, which haven't been run.
- Support self-overlapping and intersecting transparent surfaces with forward per-fragment shading or OIT.
- Implement sampled UI colors, the remaining UI layout branch, primitive style access, and non-box bounding boxes.
- Spawn and despawn a server-side client entity when BETP reports `ClientConnected` and `ClientDisconnected`. `network::Server::drain_events` reports both, and nothing creates an entity for them yet.
- Make the HTTP inspector opt-in. `HttpInspectorServer::new` starts with every headed application and accepts unauthenticated loopback requests. `POST /messages` can move or delete entities and trigger actions, and `DELETE /` closes the process.
- Add a starburst and a lens dirt texture to the screen-space lens flare in `crates/byte-engine/src/rendering/render_passes/lens_flare.rs`. The starburst should rotate with the camera's orientation from `Sink::view`, and the dirt texture should modulate the flare, and optionally the bloom, at composite time. Both need texture assets loaded by the pass.
- Build the CPU animation graph, evaluate imported glTF and FBX clips into `VisibilitySceneManager::write_skinned_pose`, apply retained rigid primitive nodes, and provide animation-safe bounds so posed meshlet culling can be re-enabled.

- Release streamed meshes while ray tracing. `SceneAccelerationStructures` builds a BLAS per mesh and never releases one, so `VisibilityPipelineManager::stream_terrains` (`crates/byte-engine/src/rendering/pipelines/visibility/manager.rs`) releases no terrain node mesh while the pipeline traces rays, and a long flight over a large terrain keeps every node it visited. Destroy a mesh's BLAS once no frame in flight reads it, reuse its hit records, and then drop the early return.
- Pass terrain entities' material parameters to their node renderables. `SetMaterialParameter` on a `RenderableTerrain` entity reaches no node, since each node draws from a handle of its own; forward the entity's parameters to every node instance as `TerrainStreams` adds it, and update the terrain rules in `docs/reference/rendering.mdx#terrains`.
- Give terrains height queries for physics and gameplay, such as placing RTS units on the ground. The `Terrain` resource keeps no heights; store the height field as a payload stream, or a coarser copy of it, and specify how a query interpolates it in `docs/reference/terrain.mdx`.
- Give terrain node meshes lightmap UVs when the bake builds lightmaps in. Each node could use its terrain-wide `UV` scaled to the node's square, since the surface never overlaps itself.

- Give skinned primitives detail levels once animation-safe bounds exist. `MeshProcessorSession` keeps them at full detail because simplification error is measured in the bind pose and the instance sphere is the rest pose, so the renderer could not tell when a coarser level is safe.

## Surfel global illumination

- Light a frame with surfel GI on Vulkan in an application, and compare it with Metal. The visibility and surfel shaders compile for Vulkan, GHI traces rays and draws task and mesh pipelines there with validation on an RTX 3060, and the `cube` example starts with `--render.ghi.features.mesh-shading=true --render.surfel-gi.enabled=true` and presents frames without a validation error. That example bakes no assets and draws no scene, so no visibility or surfel pass has run on Vulkan yet. Bake the engine's assets on Linux with `render.surfel-gi.enabled` set to `true` (`beld bake --force`), render Sponza in the model viewer, check the `VisibilityPipelineManager.surfels` and `VisibilityPipelineManager.surfel-coverage` debug views, and fix what validation reports.
- Let rays hit skinned meshes, and let surfels follow moving instances. The top level places only unskinned geometry (`crates/byte-engine/src/rendering/pipelines/visibility/acceleration_structures.rs`), so characters neither block bounce light nor reflect it, and surfels keep the world position they spawned at. `surfel-validate` votes out a surfel whose surface moved away within 8 frames, and `surfel-integrate` one that geometry now covers, so the light on a moving object lags behind it while new surfels spawn and converge. Skinned meshes need the refitting bottom levels above; moving instances need surfels stored relative to their instance, which the grid then lists at the instance's current transform.
- Read only the lights that matter at a surfel ray's hit. `surfel_direct_light` in `crates/byte-engine/assets/rendering/visibility/surfel-shade.besl`, like `probe_direct_light`, loops over the whole light table and traces a shadow ray for each light within reach, so the shade pass's cost grows with the lights around the hits. Read the light clusters at the hit, or keep a short list of the brightest lights per grid cell, or pick one light per hit by its expected contribution and weigh it by that probability. Then run the model viewer's night scene, which surfel GI left black before hits read the light table; only the VM test `surfel_shade_lights_a_hit_with_the_scenes_lights_unless_something_hides_them` covers point and cone lights so far.
- Test alpha against masked primitives in surfel rays, with any-hit functions. Masked and blended primitives stay out of the bottom levels, so rays pass through foliage and glass. Add one generic any-hit function that reads the material's coverage texture and alpha cutoff, run it through an intersection function buffer at the same record indices as the hit groups, and mark the geometries without one opaque. Rebuild a mesh's bottom level when a material's alpha mode changes on reload; the build reads the modes only once.
- Record the frame-wide surfel stages on an asynchronous compute queue. The first sink records them before its raster passes, so the GPU runs them alongside those passes, which saved 0.27 ms on Sponza, but only until a later barrier waits for all earlier compute work; on a queue of their own, the grid, the trace, and the integrate pass could overlap any pass that doesn't read their results.
- Give every sink its own surfel grid, or center the grid between the sinks' cameras. The grid follows the first sink's camera (`SurfelGiParameters::grid_origin`), so a second sink far from it reads coarse cascades or no surfels at all.
- Rename `SSGI History` and `SSGI Normals` to names that cover both sources of indirect diffuse light, now that surfel GI writes them too, and update the targets table in `docs/reference/rendering.mdx`.
- Cut the rest of the surfel resolve's radial depth test, about 0.3 ms of `screen-space` on Sponza at 1024 × 1024 on an Apple M4. Each surfel's clearance (`surfel_moment_clearance` in `crates/byte-engine/assets/rendering/visibility/layouts/surfel_gi.besl`) is the minimum over its 16 cells, so a wall in any direction makes every pixel around the surfel map its direction and read four cells. On the stage E7b shaders, the three extra cell reads cost 0.15 ms and the four-moment solve 0.04 ms; the rest is each pixel and surfel pair's setup and first read. Try clearances per quadrant of the surfel's plane, where most of the pixels it lights lie, or loading each tile's surfels and their cells into workgroup memory.
- Prototype a final gather for surfel GI and evaluate sample reuse, the review items deferred from the kajiya comparison. Trace one short ray per half-resolution pixel, or per pixel of a checkerboard, that reads the surfels where it hits, and compare its noise and cost with the surfel lookup at the same total. Then try reusing the surfels' ray samples between neighbors that face the same way, as kajiya's restir-meets-surfel branch reuses its rays through reservoirs.
- Bring back the first frames of surfel GI's response to a large light change. When the sun turns on Sponza, 84% of the change remains after 15 frames, against 66% before the temporal pass and the ray classes, though the tail settles sooner: 2% after 120 frames against 9%. The light also starts about 4% below its final mean 25 frames after the view appears. Feed each surfel's inconsistency, or its priority class, into the history's confidence cut (`surfel_temporal_accumulate` in `crates/byte-engine/assets/rendering/visibility/layouts/surfel_temporal.besl`), or shorten `SURFEL_MEMORY_FRAMES` while a surfel's inconsistency stays high, and measure with raw `SSGI History` captures at a pinned view.
- Snap each surfel grid cascade to whole cells of its own. The grid origin snaps to whole cells of cascade 0 (`SurfelGiParameters::grid_origin`), so a coarser cascade's cells move by a fraction of themselves as the camera walks, and which of its cells list a surfel changes with them. The irradiance cache of kajiya scrolls each cascade by whole cells of that cascade.
- Tune surfel GI's defaults on scenes larger than Sponza: the surfel capacity, the ray budget, the 64 surfels a pixel reads, the priority class's three quarters of the budget, and the shares of the pool that surfels spawned by hits may take (`SURFEL_BOUNCED_SHARE`, `SURFEL_HIT_SPAWN_SHARE` in `crates/byte-engine/assets/rendering/visibility/layouts/surfel_gi.besl`). Sponza settles at about 1,500 surfels, about 400 of them spawned by hits, and young surfels keep the default budget of 16,384 rays a frame full; the default capacity of 32,768 surfels takes about 24 MiB.
- Trace webgiya's depth probe rays: one ray for a quarter of the surfels each frame, in a depth cell picked in turn, which updates only the radial depth. Guided rays visit dark directions rarely, so their depth cells go stale while the scene changes.
- Light enclosed rooms evenly and sooner. In Sponza's gallery, most diffuse light is bounce light spread from the few surfels whose rays found the sunlit courtyard, about one ray in several thousand: with no surfel light at hits, 88% of the deep gallery goes black. Its light spans a factor of 6 to 9 between the 10th and 90th percentile of pixels, and three guided runs read a third of an unguided run's level 600 frames after the view was pinned (1.7–1.9e-5 against 5.6e-5 pre-exposed), while runs captured later all read 4–6e-5. The build-up also dominates what a turn shows: after a 60° snap turn into the gallery, the revealed pixels' `SSGI History` reads 0.85–0.9 of its level 1,200 frames later, with a mean |log2| error of about 0.55 over frames 10–45, 0.4 at frame 60, 0.14 at 230, and 0.04 at 460, while young surfels' bright spots (above four times the later level) cover only 0.5% of the pixels. Measure the build-up at matched surfel ages first, then try sharing estimates between neighboring surfels that face the same way, and more rays for surfels whose light comes from rare bright hits; the 512-ray warm-up of young surfels is a first step.

## Baked lightmaps

The basic lightmap bake (`crates/byte-engine/src/rendering/lightmaps`) leaves these for later:

- Read authored second UV sets, glTF `TEXCOORD_1` and FBX `UVSet1`, instead of always unwrapping.
- Replace the dominant-axis unwrapper with an xatlas-quality one: chart growth by normal cone, seam-aware packing, and per-chart texel density.
- Pack the atlas tighter. Sponza fills about a quarter of its 2,048² image, because the row packer's square rounds up to the next power of two; try non-square images or a skyline packer.
- Store directional lightmaps (L1 spherical harmonics) so normal maps shape baked light.
- Denoise the traced light (an edge-aware filter over position and normal), so fewer samples give smooth light.
- Sort hits by material before shading them in the bake, as surfel GI does, instead of shading in ray order.
- Split a level into several atlas pages when it doesn't fit `max_extent`, instead of shrinking the texel density.
- Light dynamic objects in lightmapped scenes, for example with probes baked from the same paths.
- Let masked materials occlude the bake's rays with an any-hit alpha test.
- Apply IES profiles to point and cone lights in the bake.
- Bake more than the visibility pipeline's 1,024 primitives; the bake traces only the first 1,024.
- Stop Sponza's masked foliage from glowing white under `render.gi.technique=baked`. Its many tiny charts each get the 16-texel minimum and the bake leaves masked surfaces out of the traced geometry; find which texels go bright (likely texels whose nearest triangle point lies outside the plant) and either fix them or leave masked primitives unlit.
- Reduce the per-texel noise lightmaps show on large floors at 128 samples, before or with the denoiser above.
- Learn ray guides with one invocation per texel instead of per group. Learning batches run about 1.6 times longer than guided ones because a group's texels trace in turn; float atomics in BESL (or a per-texel cell buffer) would remove that, and then more paths could learn.
- Guide more than the first bounce, and weight the learning and guided paths' estimates by their variance instead of averaging them; at three bounces the later, cosine-weighted bounces hold most of the remaining noise.
- Importance-sample the environment at each hit (an environment CDF with MIS), so sharp skies and small openings stop needing the first bounce's guide to find them.

## Shader behavior

- Implement Metal interpolation and metadata-driven push-constant mapping without hardcoded backend conventions.
- Add a BEMX `surface.displacement` input that moves vertices, so water, cloth or terrain can ripple. The visibility mesh, shadow, material-evaluation prefix and BLAS stages all read the resident vertex streams, so evaluate the displacement once per frame in a compute pass that writes an instance-owned vertex range, the way skinning does, and point every stage at that range. Deferred from the water-material plan: the slate quarry's still pit doesn't need it.

## MaterialX conversion

- Reduce MaterialX scattering closures onto a BEMX `Surface`, so a `<surface>` node built from `oren_nayar_diffuse_bsdf`, `dielectric_bsdf`, `conductor_bsdf`, `layer` and `mix` converts instead of reporting an unsupported shading model. This is most of the MaterialX pbrlib test suite and the pbrlib-native way to write a surface.
- Add BEMX nodes for the MaterialX procedural and colour-conversion nodes the standard library defines with source code rather than node graphs, and map them in `materialx::bemx`: `noise2d`, `noise3d`, `fractal3d`, `cellnoise2d`, `worleynoise2d`, `flake2d`, `flake3d`, `hsvtorgb`, `rgbtohsv` and `blackbody`.

# P2 - BESL architecture

- Add explicit interpolation syntax and a generic texture/sampler resource model.
- Complete MSL lowering for all declared intrinsics and reconcile `fetch_u32`, `image_atomic_or`, and `image_load_u32`.
- Add 3D compute built-ins and address-space semantics.
- Dispatch the mesh grid after an early `return` in HLSL task shaders. `emit_hlsl_task_entry` in `crates/resource-management/src/shader/besl/backends/hlsl/generate.rs` emits the statements of `main` inline and then `DispatchMesh`, so the `return` that `meshlet-task.besl` takes for an instance no view sees skips the call, which DXIL requires every thread group to make. Run BESL's `main` as a function and dispatch after it returns, as the GLSL generator does in `crates/resource-management/src/shader/besl/backends/glsl/generator/mesh_shading.rs`, then compile `meshlet-task.besl` as an amplification shader on Windows.
- Add missing control flow, boolean and numeric types, typed textures, matrix shapes, and structured array support.
- Represent threadgroup size, matrix layout, function constants, and other MSL compile options in shader metadata.
- Analyze each BESL graph once with visited-node tracking and keyed binding deduplication, sharing results across generation, reflection, and opacity evaluation.
- Write GLSL/MSL directly into capacity-estimated output strings instead of creating repeated temporary formatted strings.
- Let two BESL modules declare the same struct or constant name, and the same function with the same parameter types. `check_unique_names` in `crates/besl/src/modules.rs` rejects them because backends emit every declaration by its bare name. Give each module declaration an emitted name built from its module path when it would collide, keeping the bare name for root declarations that reflection and engine bindings match by name, then drop the rule from the Modules section of `docs/reference/besl/language.mdx`.
- Fold the descriptors, push constant block, and tile targets a BESL shader declares itself into an implicit layer, so every set comes from a layer and `collect_set_nodes` in `crates/resource-management/src/shader/besl/evaluation/reflection.rs` orders only layers. Today they form the shader's own set, last.
- Number a BESL layer's bindings implicitly, in declaration order, once the GLSL and HLSL backends emit a set or space per layout (see the Vulkan and DX12 per-layout sets item). `binding` numbers stay explicit and unique across a shader's layers only because those backends emit one flat set.
- Give the engine's shaders signatures where several share a parameter list. The migrated shaders list their layers in `main`'s parameters; the material evaluation and hit shaders, which the generator builds, are the first candidates (`MATERIAL_EVALUATION_LAYERS` in `crates/byte-engine/src/rendering/pipelines/visibility/shader_generator.rs`).
- Add `pub` visibility to BESL modules. Every module declaration is reachable by path today, so a module can't keep helpers private.

# P3 - Tests and tooling

- Add one smoke rendering path per supported backend to CI.
- Test rendering a frame with no elements.
- Fix or replace the ignored Vulkan WSI tests in `crates/ghi/tests/rendering.rs`.
- Add targeted GHI backend tests or fakes for device, context, resource, and command lifecycle behavior.
- Add focused window tests, including macOS keyboard consumption, cursor visibility, and confinement.
- Test that glTF and FBX URI resolution stays inside the asset root, including absolute paths and `..` segments.
- Extend BETP and raw-datagram fuzzing past canonical packets. The current targets do not cover the UDP adapters, a lost challenge response, or two clients on one socket.
- Add UI tests for sampled colors, remaining layout behavior, primitive styles, and non-box bounds.
- Delete the stage G0 ray tracing spikes in `crates/ghi/src/metal/ray_tracing_spikes.rs` once benchmarks of the surfel trace and shade passes measure the same costs: binary function compiles, linking, divergent hit group calls, and sorting hits by hit group.

# P3 - Conditional optimizations

- Avoid formatted shader-stage strings while hashing shader descriptors if shader-cache profiling identifies meaningful churn.

# P2 - Architecture and module cohesion

## Visibility rendering

- Give `PipelineManager` the shape render passes have: a scene feature that holds what every sink shares and builds one pass per sink, which borrows the feature, as `RenderFeature` and `RenderPass<F>` do (`crates/byte-engine/src/rendering/render_pass.rs`). Today each manager keeps its sinks' state inside itself. Deferred from the render feature split; see [render features and passes](/docs/develop/rendering/render-features).

- Split `rendering/pipelines/visibility/mod.rs` into focused modules for bindings, limits, GPU data layouts, and visibility, shadow, and material shader sources.
- Split `rendering/pipelines/visibility/render_pass.rs` so visibility, shadow, material-count, material-offset, pixel-mapping, GTAO, and material-evaluation passes each own their resources, preparation logic, and tests.
- Split the visibility resource manager into request/completion protocol, worker lifecycle, resource loading, texture upload, pipeline compilation, and reusable resource-state modules.
- Move visibility production definitions and imports before test modules so tests remain the final section of each source module.
- Replace high-argument visibility pipeline and render-pass construction with operation-specific configuration and resource structs.

## Application and input

- Move input trigger and evaluation documentation beside the implementation that owns those rules.
- Move UI timer deadlines from the process-wide list in `crates/byte-engine/src/ui/timer.rs` into the runtime a `WaitFuture` is polled in, found through its task context, so `Engine::next_tick` reads only its own state and `timer::wait` keeps its signature.

## Assets and resource processing

- Split glTF importing into document loading, mesh extraction, material generation, image loading, and URI resolution modules.
- Introduce a focused glTF import context to replace repeated long parameter lists without creating a generic dependency bag.
- Split image processing into source-format conversion, mip-chain generation, BC compression, and image-layout modules.
- Keep processor tests beside the conversion or compression module they verify.
- Separate mesh source models and normalization from final mesh stream packing in `mesh_processor`.
- Lock only the vertices that mesh primitives share when simplifying detail levels. `lods.lock_borders` locks every open border, which also stops open sheets from simplifying; finding the shared ones needs a pass over all primitives before `MeshProcessorSession` packs the first.
- Let authored detail levels change material, so a Megascans plant's last file, a billboard card with its own `Billboard` material, can be its coarsest level. `lods.authored` ends the chain at a file whose materials differ, because a primitive has one material for every level.
- Weld positions before simplifying detail levels, or use meshopt's `Permissive` option, for meshes whose vertices are split per face. Every vertex of such a mesh is an attribute seam, so `simplify_levels` stalls at once and the mesh keeps full detail only; `Suzanne.gltf` in the model viewer is one.
- Weigh UVs in detail level simplification without letting tiling UVs inflate the stored error. `simplify_levels` passes normals only, so a level can stretch a texture across a flat region and still report no error.

## BESL and shader generation

- Move generic AST traversal and shader emission helpers from resource management into the BESL crate.
- Keep resource management responsible for compiled shader resources, platform compilation, and persistence rather than language-level emission.
- Split the MSL backend into type/expression emission, bindings, raster stages, compute stages, and mesh stages.
- Apply the same shared emitter structure to GLSL and HLSL so backend differences remain explicit and duplicated formatting logic is removed.

## Graphics hardware interface

- Define and enforce Vulkan instance/device/context/factory ownership so contexts and detached resources cannot outlive their native parents. Rendering tests currently keep the primary device alive explicitly; replace this caller-enforced lifetime with a solid ownership model.

- Split each large backend context implementation into resources, descriptors, pipelines, synchronization, transfers, and acceleration-structure modules while keeping the public context type in `context/mod.rs`.
- Move GHI handles, resource descriptions, and behavioral traits into their existing domain modules instead of declaring most contracts in `graphics_hardware_interface.rs`.
- Reduce `graphics_hardware_interface.rs` to compatibility re-exports or remove it after callers migrate to domain modules.
- Honor the swapchain present interval (`Context::set_present_interval`) natively on Vulkan and DX12. Both currently sleep in `pace_present` before acquisition, which is not phase-locked to vblank and can alternate between one and three refresh periods when the slot lands near a boundary. Use `VK_EXT_present_timing` (or `VK_GOOGLE_display_timing`) on Vulkan and a waitable swapchain with `SetMaximumFrameLatency` on DX12, the way Metal uses `presentAfterMinimumDuration`. Verify with `--max-frame-rate=30` on a 60 Hz display: deltas should sit at 33.3 ms as they do on Metal.
- Drive Metal presentation with `CAMetalDisplayLink` (macOS 14+). It hands out the drawable together with a `targetPresentationTimestamp` per frame, so the swapchain acquisition can report the time the frame will be shown instead of only the time the previous one was, and it replaces the manual `presentAfterMinimumDuration` cap with `preferredFrameRateRange` (which also matters on ProMotion displays) and `preferredFrameLatency`. Keep the engine's pull loop: run the link on a dedicated run-loop thread, hand each update over a channel, and let `acquire_swapchain_image` block on that channel instead of `nextDrawable`. Fall back to `nextDrawable` on older systems. `NSView.displayLink(target:selector:)` is the lighter alternative if only the timing is wanted.
- Add cursor visibility, confinement, and pointer lock to the GHI window API when relative mouse input needs them. The unreachable `WindowLike::show_cursor` and `confine_cursor` stubs and the Wayland `zwp_pointer_constraints_v1` state machine were removed with the app-wide event pump. Expose them as `ghi::window::App` operations that take a `WindowId`, since Wayland applies them through the shared connection state and only while the window holds pointer and keyboard focus (AppKit: `NSCursor::hide` and `CGAssociateMouseAndMouseCursorPosition`; Win32: `ShowCursor` and `ClipCursor`).

## Cross-cutting layout

- Keep every `#[cfg(test)] mod tests` at the end of its production module.
- Split production files once they contain multiple independently testable responsibilities, rather than using line count alone.
- Prefer operation-specific context structs for cohesive long argument lists; do not replace them with broad service-locator-style bags.
- Keep tests in the new owning submodules instead of retaining large centralized test sections.
- Avoid creating additional crates until module-level splits show a stable dependency boundary that needs independent compilation or ownership.
- Prioritize the visibility pipeline refactor first because it combines the greatest file size, dependency breadth, duplicated shader contracts, and constructor complexity.

## Levels (deferred)

`.level` files and `gameplay::Spawner` (`crates/byte-engine/src/gameplay/level.rs`) place a flat list of entities. Left out of the first version:

- Per-entity overrides of a prefab instance's payloads. Parameters cover the authored contract today; an override would name a prefab entity by id and replace one kind's payload.
- Respawn prefab instances when their prefab rebakes in debug builds, as for levels below.
- Let the inspector list a prefab's parameters with their defaults, so an editor can show them.
- Place world-space payloads other than `transform` with their spawn: `navigation_area` extents, `merged` mesh placements, and anything else that holds world positions ignores a prefab's placement today.
- Light the prefabs a level places when baking its lightmap. `collect_level` spawns the level alone, so a level with instances fails the lightmap bake.
- Engine kinds the sandbox's prefabs need, such as an animator and a navigation agent, so `third_person`'s NPC can move to a `.prefab`.
- Respawn a level when its resource rebakes in debug builds. `ResourceManager::resource_updates` reports the level's ID; deleting the root and spawning again is correct but loses runtime state.
- Spawn levels larger than the message pool across ticks. A spawn publishes one message per entity and per component in one call; past `messages.capacity` the publisher waits for listeners that run later on the same thread.
- Warn when a spawned kind has no listener. A kind no system follows is published and dropped silently; channels can't report how many listeners they have.
- Give level entities identities that survive across processes, from their file ids, so a server and its clients agree on placed entities without replicating handles.
- Import a level from a glTF or FBX scene's nodes. The glTF handler flattens a whole scene into one mesh resource.
- Let the inspector spawn kinds and list them with their payload shapes, as a base for an editor.
- Add triangle-mesh and heightfield colliders, and convex hulls baked from mesh resources, so a `body` can collide as level geometry.
- Share one `Arc<str>` per distinct mesh ID within a spawn. The `mesh` kind allocates its ID twice per entity, about 40 ns of the 47 ns a mesh costs to spawn.

## Editor resource explorer (deferred)

Cmd+E explores the game's resources: material graphs open on the canvas and textures show in a preview panel. Left out:

- Preview a mesh from Cmd+E. Render images, isolated scenes (`GraphicsApplication::create_scene`), and GPU images in UI `image` elements now exist; what's left is the preview itself: stage the mesh in its own scene, orbit its camera, and show the render image in the explorer. Deferred on 2026-11-16.
- Preview baked textures in builds without sources. Baked images are BC7; decoding needs the BC7 decoder in `resources/mips/bc7.rs` and a way to read a resource's binary streams through the inspector.

## Editor levels (deferred)

- Load the level the palette's New level command creates, in place of the running one, so the editor edits it. The command only writes an empty `.level` source today.
- Name a new level as it's created. Today it's `untitled.level`, numbered when taken.

## Clipboard (deferred)

`ghi::window::App::set_clipboard_text` puts text on the clipboard on macOS only; the editor's Alt-click on a material name uses it. Deferred on 2026-11-17:

- Windows: `OpenClipboard`, `EmptyClipboard`, then `SetClipboardData(CF_UNICODETEXT)` with a UTF-16 global allocation.
- Wayland: offer a `wl_data_source` with `text/plain;charset=utf-8` through `wl_data_device::set_selection` with the last input serial, and write the text to each `send` request's descriptor from the event loop while it owns the selection.

## Ownership cleanup (deferred)

- Replace the process-wide `COUNTER` in `crates/byte-engine/src/core/factory.rs` with an id counter owned by the world or message bus and passed to the factories. Deferred on request during the Rc/Arc and globals cleanup.

## GPU counters (deferred)

- Add a pipeline-statistics counter kind to the GHI counters API for Vulkan and DX12: `create_counter` takes a `CounterKind`, `Context::counter_statistics` returns input vertices and primitives plus vertex, geometry, clipper, fragment, compute, task, and mesh invocations and clipper and mesh primitives, gated by `supports_pipeline_statistics`. Vulkan needs the `pipelineStatisticsQuery` feature and `meshShaderQueries` for the mesh counts; DX12 uses `PIPELINE_STATISTICS1` where the device supports it. Metal 4 counter heaps hold only timestamps, so Metal always reports `None`. A statistics span is one query slot whose begin and end must sit in the same command buffer, and on Vulkan inside the same render pass instance or both outside it. Deferred on 2026-10-04 so timing counters shipped first.
- Let counters record in detached recordings (loader contexts, the resource-management GPU worker). Timing counters are frame-only because slots are owned per frame sequence; detached recordings would need per-recording slot pages released on synchronizer completion, like texture readbacks.
- Grow the counter slot pool instead of capping a frame at 512 counters, if a frame ever needs more.

## OS dialogs (deferred)

- Route dialogs from byte-engine: let UI and gameplay code request an OS dialog and receive its `ghi::window::DialogResult`. The renderer owns `ghi::window::App` today, so nothing outside it can call `App::open_dialog`.
- Attach Linux portal dialogs to their parent window: export the toplevel with `zxdg_exporter_v2` and pass `wayland:<handle>` as the portal's parent window. Today they open on their own.
- Offer message boxes on Linux, which has no portal for them; `App::open_dialog` returns `DialogError::Unsupported` there.
- Compile and run the Wayland portal backend (`crates/ghi/src/window/os/wayland/dialogs.rs`) on Linux: it was written on the Mac and never compiled.

## Subsurface scattering (deferred)

- Bake local thickness offline so meshes get transmission through ears and nostrils without an authored map: cast rays inward from each vertex or texel at mesh bake, or bake inverted ambient occlusion (Barré-Brisebois and Bouchard, GDC 2011), and expose it to BEMX graphs as a `thickness` source. Deferred on 2026-11-13 in favor of the authored `thickness` input.
- Convert MaterialX `standard_surface` subsurface inputs to BEMX: `subsurface_radius × subsurface_scale` to `subsurface_radius`, gated by `subsurface` weight. `docs/reference/materialx.mdx` still lists them as not converted.
- Dispatch the subsurface pass only over subsurface pixels, through the material pixel lists or tiles marked during material evaluation, instead of over the whole screen, and skip it in frames that draw no subsurface material.
- Shade subsurface materials in surfel and probe GI hits; `material_hit_body` only reads albedo and emission.

## Temporal antialiasing (deferred)

- Raise the reactive mask from particles: their generated draw pipelines write only `main` and `depth`, so particles reproject with the surface behind them. Add a `Reactive` attachment with max blending to the particle draw bake, writing `min(α, 0.9)`.
- Let a material override its reactive value, as FSR does, such as 1 for sparks and UV-scrolling surfaces, through a BEMX output that transparent evaluation writes instead of `min(α, 0.9)`.
- Add FSR 3.1 (FidelityFX SDK) as a `ghi::upscaler` backend for Vulkan and DX12, and an `fsr` technique of the `taa` pass. Vulkan: record it like `blit_image_to_image` in `vulkan/command_buffer/recording.rs` (consume the inputs and output, record the SDK call, consume the post-state), then reset `bound_pipeline`, `descriptor_heaps_bound`, and `descriptor_bindings_dirty`. DX12: move the images into layouts compatible with legacy resource states before the call, update `image_states` to what the SDK leaves, and rebind the descriptor heaps, root signature, and pipeline (`dx12/context/transfers/synchronization.rs`). Deferred on 2026-11-20 so MetalFX shipped first.
- Add DLSS (through NVIDIA Streamline) and XeSS behind the same `ghi::upscaler` interface, picked by GPU vendor on Vulkan and DX12.
- Dynamic resolution: allocate render-extent targets at the largest scale and vary `ghi::upscaler::Inputs::content_extent` per frame from a GPU-time budget. Every pass before the upscaler would then have to render into a viewport of its targets.
- Skinned instances posed for the first time, and instances that reuse a freed slot, read stale previous positions or a stale previous model for one frame. Mark those instances so the velocity stage treats them as standing still.
- Try k-DOP history clipping (Ikkala et al., 2024) if AABB variance clipping still lets ghosts through.

## Virtual shadow maps (deferred)

- Sample the virtual shadow map for transparent receivers and the fog march; both get no sun shadow under `virtual` today. Fog and transparent receivers also need pages marked, or a fallback to the nearest coarser level that holds one.
- Mark pages from every sink's receivers, not only the first sink's, so a second camera's shadows don't depend on what the first one sees.
- Free pages under skinned instances every frame they receive a pose; today a pose leaves the pages drawn with an earlier one.
- Alpha-test masked casters in the page draw; they cast as if opaque.
- Report a metric per page pass inside `stage.virtual-shadow-maps`.
- Read the penumbra filter's texels in 4 × 4 blocks with gathers when a block lies inside one page, as the cascades do; the resolve fetches each texel through its page (about 3 ms against 1.4 ms for cascades in the quarry, debug build).
- Cut the page draw's fixed cost: 1.5 ms in the quarry with a still camera, nearly all the task stage testing every instance against 16 levels. Skip levels with no dirty page, or raise `MAX_TASK_VIEWS` to 16 so one dispatch covers every level.
- Shadow more than one directional light, with page table layers per light.
- Quantize the sun direction that shadows use: a slowly turning sun frees every page each frame, as in the chapter.
- Cone and point lights on the page pool, using the thesis's mip chains.
- Per-level detail-level selection for the page draw.

## UI fit sizing (deferred)

- Minimum and maximum constraints on a fit axis, such as `Sizing::fit().max(..)`, so a menu stops growing and scrolls.
- Move editor panels, menus, and toolbars from pixel sizes to `Sizing::fit()`, and animate their resizes through `logical_size_change`.
- A fused wait for `Watch` changes, so a component can `select!` over a watch and a size change without `.fuse()`.

## UI memory footprint (deferred)

Deferred on 2026-11-19 from a UI CPU and GPU memory report.

- Shrink `UiPrimitive` (`src/ui/render_pass/data.rs`) from 112 bytes: move the gradient axis and end color into a table and pack the kind and mask slot into one `u32` (80 bytes), or store both colors as adjacent FP16 vectors (96 bytes). Keep positions and curve coordinates in f32. Changes the BESL layout in `assets/rendering/ui/layouts/ui.besl`; check visual precision. Lowering the buffer's capacity may be a cheaper first step.
- Write slug glyphs straight into painter order instead of staging them in `UiTextGeometry` and copying them in `geometry.rs`.
- Store each distinct inherited clip mask once in the layout engine's visual state and carry 32-bit indices, keeping mask identity across incremental updates (160 to 72 bytes per tree slot).
- Patch retained damage footprints in place on incremental renders when membership and indices are stable, instead of resizing and refilling the scratch array (`layout/engine.rs`).
