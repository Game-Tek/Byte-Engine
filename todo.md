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
- Rebake generated visibility material shaders when the layout layers they stack, or the generator, change. A sandbox app that loads stale material shaders from its resource cache now panics on the first material dispatch with "Missing descriptor set for layout", which names the layer; the bake cache only notices changes to resource-management itself. Until then, delete the app's `resources` folder after changing `crates/byte-engine/assets/rendering/visibility/layouts` or `shader_generator`.
- Return a parse error for a zero-length BESL array. `u32[0]` is accepted, then `Node::array` expects a non-zero size in `crates/besl/src/lexer/ast.rs`.
- Emit `vec3u16` raster I/O as a non-interpolated integer on HLSL, GLSL, and MSL. `is_integer_type` lists the 2- and 4-component forms and omits `vec3u16`, so the stage output is interpolated or the backend compile fails.
- Make Linux audio pause return an error when the device does not support ALSA pause. `crates/ahi/src/os/linux.rs` uses `pcm.pause(true).unwrap()`, and `play` panics for a channel count other than 1 or 2. On Windows, reject a closest match the renderer cannot play, request 32-bit streams as float rather than integer PCM, and treat `CoInitializeEx` returning `S_FALSE` as already initialized. `play` panics outside 16- or 32-bit mono and stereo, and a 32-bit stream is requested as `KSDATAFORMAT_SUBTYPE_PCM` while samples are written as `f32`.
- Give the Vulkan and DX12 GHI backends native per-layout descriptor sets, as Metal has. Both take `ghi::ShaderDescriptorSet` and `ghi::DescriptorSetLayout` now, but `create_shader` flattens the used resources into one flat-slot list (`shader::flatten_used_resources`), `create_descriptor_set` ignores its layout, and each pipeline still materializes its own native descriptors from retained flat-slot writes. Create one native set or descriptor table per `DescriptorSetLayout`, share it between pipelines, check writes against the layout, and match bound sets to a pipeline's layouts by `DescriptorSetLayout::identity` (`crates/ghi/src/metal/command_buffer/encoding.rs` is the reference). The GLSL and HLSL BESL backends still emit one flat set or space; they need a set or space per layout, in the order `evaluation::collect_set_nodes` returns, which is also the order of `ShaderInterface::sets`. Vulkan set numbers are shared by a pipeline's stages, so either number sets per pipeline and patch the SPIR-V decorations, or require stages to list their layers in one order. The signature changes in `crates/ghi/src/vulkan` and `crates/ghi/src/dx12` were written without building either backend; compile and run both first.
- Fix the macOS `NSWindow canBecomeKeyWindow` warning.
- Define texture usage semantics for resources consumed by multiple unknown render passes.
- Give UI element paths an identity that cannot repeat between live scopes. `RetainedTree::begin_frame` clears `path_counts`, and `scope_path` assigns the ordinal from that per-frame counter (`crates/byte-engine/src/ui/layout/retained_tree.rs`). Two mounts with the same name under one parent started on different frames share a path, share element ids, and removing one removes the other's elements. Task ownership already uses `ScopeId` and is unaffected.
- Upload only the bytes a frame wrote to a Vulkan dynamic buffer. `Frame::get_mut_dynamic_buffer_slice` in `crates/ghi/src/vulkan/frame.rs` queues a copy of the frame copy's whole staging buffer into its device buffer. The UI primitive, mask, curve, and band buffers are dynamic (`crates/byte-engine/src/ui/render_pass.rs`), so a frame that writes them uploads about 14.7 MB of primitives plus about 4 MB of masks, curves, and bands per sink. Map host-to-device dynamic copies directly so they need no staging copy, or give the UI a smaller per-frame footprint instead, such as a ring suballocation of one buffer that holds only the live primitive count, which also removes the extra 14.7 MB per sink that Metal keeps for the second frame's copy.
- Implement the loader-context contract on DX12 so Windows builds again. `crates/ghi/src/lib.rs` requires `Context`, `DetachedImage`, and `SharedBuffer` to be `Send`, but the DX12 context holds raw `*mut u8` mappings and an `IDxcCompiler3` (`crates/ghi/src/dx12/context.rs`). `FactoryImage` is a parameter bag that interning turns into a committed resource (`crates/ghi/src/dx12/frame.rs`); replace it with `export_image` that moves the `ID3D12Resource`, its CPU copy, and its `image_states` entry, and `intern_image` that adopts them. Add `share_buffer` and `import_buffer`, delete the factory image and sampler path and its test in `crates/ghi/src/dx12/mod.rs`, and make device-wide idle waits cover every context's queues. A `TRANSFER` selection maps to a `DIRECT` queue today, which is enough for an independent loading timeline. Verify with `cargo check -p byte-engine-ghi --target x86_64-pc-windows-msvc` and the GPU tests on Windows.

# P1 - Runtime performance

## Frame and rendering

- Elide redundant barriers in the Vulkan and DX12 resource trackers as the Metal tracker now does. `MetalResourceTracker::plan` in `crates/ghi/src/metal/synchronization.rs` logs every encoder barrier it hands out, stamps each state with the log length when it was recorded, and skips a same-encoder hazard that a logged barrier already orders: one whose `after` stages hold the state's stages, whose `before` stages hold the use's stages, and, for a read of a write, that carried `Device` visibility. The Vulkan tracker's `vkCmdPipelineBarrier` and the DX12 tracker's `ResourceBarrier` are issued per hazard the same way, so the interleaved screen-space passes in `crates/byte-engine/src/rendering/pipelines/visibility/render_pass.rs` pay a barrier per stage there. Vulkan needs the equivalent with execution plus memory dependencies, where a logged barrier covers a hazard when its source stages and access hold the state's and its destination stages and access hold the use's; DX12 needs it for UAV barriers, where transitions still change state and cannot be skipped. Neither can be verified on this Mac; test with the tracker unit tests ported from the Metal ones, then a GPU run on each platform.
- Reuse the slots that `export_image` empties. `ResourceCollection::take` (`crates/ghi/src/frame_resources.rs`) leaves the slot empty and never hands its index out again, so the loader context's image list grows by one entry per loaded image for as long as an application streams. Add a free list with generation-checked handles when image destruction or texture streaming lands.
- Compile DX12 pipelines on the compiler threads instead of when the render thread interns them. DX12 interning compiles every factory shader and pipeline state (`crates/ghi/src/dx12/frame.rs`), so `PipelineManager::publish` can stall a frame. Metal and Vulkan factories already build finished pipelines; Vulkan's `build_raster_pipeline` (`crates/ghi/src/vulkan/context/pipelines.rs`) is the pattern to follow.
- GPU-test the Vulkan backend on a driver that exposes `VK_EXT_descriptor_heap`. The loader-context contract (`crates/ghi/src/vulkan/factory.rs`), the per-frame dynamic-buffer copies, and the factory-built raster pipelines (`Frame::intern_raster_pipeline`) compile, and the rendering and loading tests that bind no pipeline pass on RADV. Every test that creates a pipeline needs the extension, which Mesa 26.1 RADV lacks. Run `cargo nextest run -p byte-engine-ghi --test rendering --ignore-default-filter`, `cargo nextest run -p byte-engine --test loading --ignore-default-filter`, and the `retained_work` benchmark there. Check in particular the barrier `Context::expect_shared_buffer_writes` adds to the first use of each shared geometry buffer in a frame.
- Give GHI factory shaders a retention contract. `Factory.shaders` only grows on Metal (`crates/ghi/src/metal/factory.rs`) and Vulkan, and each development rebake of a shader adds a new native library or module without freeing the old one. On Metal, a closest-hit shader also keeps a binary function for each set of specialization constants a ray tracing pipeline used it with (`Shader::binary_functions` in `crates/ghi/src/metal/pipeline.rs`). `PipelineManagerServer` reuses one handle per prepared shader, so steady-state growth is bounded by the distinct shaders; add `destroy_shader` or content-addressed factory shaders when shaders stream or rebake often.
- Add allocation instrumentation and budgets for steady-state frames, GHI recording, audio callbacks, and BELD peak memory before large optimization work.
- Extend the existing shared frame allocator into GHI for frame-local scratch allocation.
- Add borrowed GHI frame allocators with two lifetime classes: CPU scratch reset after submission and retained frame-slot storage reset only after its `FrameKey` completes.
- Route backend recording scratch through those allocators: Vulkan semaphore/copy/barrier data, Metal resource/binding/attachment/push-constant data, and DX12 pipeline/binding/queue data.
- Keep Metal finished-command-buffer updates, native object ownership, readback state, and completion data out of CPU scratch.
- Change GHI collection-returning APIs such as `transfer_textures` to accept caller-provided storage or allocator-aware output.
- Prefer `SmallVec` for normally tiny GHI collections such as swapchains, command buffers, descriptor sets, present drawables, and semaphores.
- Replace Vulkan command recording's full device state-map clones with immutable base state plus recording-local changes, and clean up the transition implementation.
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

## Metal-specific

- Patch a descriptor set's resource uses when one entry changes instead of building them again. `set_resource_uses` in `crates/ghi/src/metal/command_buffer/encoding.rs` walks the set and `DescriptorUses::new` sorts the result once per changed set and per way a pipeline uses it. With 1,024 different textures that is about 13 µs on an Apple M4 when the textures are already in handle order, and the sort alone takes 28 µs when they are not, for each material variant that binds the table while textures stream in. `Context::resource_to_descriptor` already says which entries name a resource, so the entries of the resources a write touched can be replaced in the sorted table; share one table per large array between the variants that use it the same way.
- Track a large read-only descriptor table as one access. Every encoder that binds a 1,024-texture table plans and records 1,024 reads, about 12 µs on an Apple M4 (`streamed_texture_in_loaded_table_completed` against `streamed_texture_shared_by_pipelines_completed`). Register the table once per encoder in `MetalResourceTracker` and let writes look their resource up in the registered tables.
- Batch pending Metal buffer and texture uploads into one transfer command buffer and blit encoder.
- Avoid cloning Metal texture staging data and pipeline state during upload and descriptor binding.
- Simplify Metal frame-chain handle deduplication if frame counts grow beyond the current small fixed count.

# P1 - Bake and asset performance

- Free idle offline GPU pyramids. `GPUMipProcessor` in `crates/resource-management/src/resources/mips/gpu.rs` keeps every pyramid it creates because the GHI has no `destroy_image`/`destroy_buffer`; `MAX_IN_FLIGHT` bounds the count per extent. Add GHI resource destruction, then evict least-recently-used idle slots against a byte budget next to the bake memory budget.
- Pipeline environment-map bakes the way material mips are pipelined. `GPUIBLProcessor` in `crates/resource-management/src/ibl/gpu.rs` completes each request inline in `GpuProcessor::submit`; give it per-request scratch and finish requests in `poll` so several HDR images overlap on the GPU.
- Replace the 1 ms completion poll in `crates/resource-management/src/gpu_worker.rs` with a backend wake only if profiling shows it matters: a `Context::wait_for_any_synchronizer` with a timeout (Vulkan `vkWaitForFences` wait-any, DX12 `SetEventOnCompletion` with real events, Metal a condvar pulsed from the commit feedback handler).
- Replace asset-handler tuple results with an allocator-backed or borrowed `BakedAsset` payload that storage can consume before arena reset.
- Redesign `MeshProcessorSession` as a two-pass packer that computes offsets and writes directly into one final allocation.
- Let glTF parsing borrow GLB and external BIN data instead of copying whole buffers.
- Flatten glTF traversal into one caller-owned primitive record buffer rather than separate tree and primitive collections.
- Bake each unique glTF material once, resolve primitive material references concurrently, and reuse generated resources.
- Move generated glTF and FBX material factors (base color, metallic, roughness, emission, normal scale, occlusion strength) out of the BESL source into per-material GPU data, so materials that differ only in factors share one compiled shader. Generated shaders are already shared per graph (`store_generated_shaders`), but factors are still written into the program as literals. Specialization constants can carry them, but each distinct value still compiles its own pipeline; a factor array in the visibility `Material` struct, filled from variant variables, would let them share one.
- Extract independent glTF primitive attributes concurrently before constructing the mesh source.
- Run independent BEMA shader loading/compilation and material/variant variable resolution concurrently while preserving order.
- Compress generated image mip levels concurrently after mip-chain generation.
- Drain BELD's buffered task stream directly instead of collecting unit results.
- Replace formatted mip stream names with typed stream identifiers or prefix-plus-index metadata.

# P2 - Platform and feature completeness

## GHI and windows

- Complete DX12 command recording and device support for resources, pipelines, uploads, mesh shading, fences, and submission.
- Implement Vulkan standalone command-buffer execution.
- Port acceleration structures, ray tracing pipelines, and shader binding tables to Vulkan and DX12. Both report `Context::supports_ray_tracing` as `false` and stop in `rt::unsupported` at every acceleration structure, ray tracing pipeline, binding table, and `trace_rays` call, and their `RayTracingPipeline` types are uninhabited. Their earlier code, which wrote buffer addresses where bottom-level structure addresses belong and hard-coded binding table record sizes, was deleted. Follow the contract in the ray tracing section of `docs/reference/ghi.mdx` and the Metal design in `docs/develop/rendering/ray-tracing.mdx`: one top-level copy per frame in flight, compaction behind a stable handle, deferred destruction, a hit record of the instance's hit group offset plus the geometry index, which is Vulkan's `instanceShaderBindingTableRecordOffset` and DX12's `InstanceContributionToHitGroupIndex` with a geometry multiplier of 1, binding table writes that leave traces recorded earlier with their records, and launches over exactly the launch size. The GLSL and HLSL BESL backends then lower the ray tracing stages and intrinsics, with `invoke` running hit objects or callables indexed by the record, and the ray tracing pipeline builder takes a payload size. Verify with the tests in `crates/ghi/tests/rendering/ray_tracing.rs`, which are ignored off macOS.
- Add bottom-level acceleration structures that refit, for skinned meshes. Every bottom-level structure is built once with `PreferFastIntersection` and then compacted (`primitive_descriptor` in `crates/ghi/src/metal/acceleration_structures.rs`). Skinned geometry needs a structure that prefers fast builds, skips compaction, and refits from the skinned positions every frame.
- Compact bottom-level acceleration structures on contexts that never start a frame. Compaction advances only in `start_frame` (`advance_acceleration_structures` in `crates/ghi/src/metal/context/resources/acceleration_structures.rs`), so structures built by recordings outside frames keep their built size until a frame starts on that context.
- Build bottom-level acceleration structures on the loading timeline. Builds run on the render context within a per-frame triangle budget, because acceleration structures cannot move between contexts: that needs export and intern for them, as images have, and a loader queue that can run builds.
- Decide how GHI should handle potentially unused staging buffers.
- Implement macOS cursor visibility and confinement.
- Wire real platform input seats across X11, Wayland, Win32, and the byte-engine input collector.

## Engine systems

- Skip the render targets of visibility features a project leaves out. With `VisibilityFeatures` turning off SSGI or contact shadows, `create_ssgi_targets` (`crates/byte-engine/src/rendering/pipelines/visibility/render_pass/ssgi.rs`) and the contact trace target in `create_sun_visibility_targets` are still allocated per sink, and `pixel-mapping.besl` still zeroes the diffuse radiance history only SSGI reads.
- Rebake generated material shaders when the baked `VisibilityFeatures` change, without `beld bake --force`. Source versions record only files, so a material baked with GTAO stays current after `config.json` turns it off. Record the generator's feature set in the material's provenance and compare it in `AssetManager::resource_is_stale`.
- Replace the fitted ACES grading output with the official ACES 2.0 Rec.709-D65 100-nit sRGB-piecewise transform, including AP1-to-AP0 conversion, precomputed hue/gamut tables, complete CAM/JMh tone/chroma/gamut processing, Apache-2.0 attribution, and Academy golden-image validation.
- Support self-overlapping and intersecting transparent surfaces with forward per-fragment shading or OIT.
- Implement sampled UI colors, the remaining UI layout branch, primitive style access, and non-box bounding boxes.
- Spawn and despawn a server-side client entity when BETP reports `ClientConnected` and `ClientDisconnected`. `network::Server::drain_events` reports both, and nothing creates an entity for them yet.
- Make the HTTP inspector opt-in. `HttpInspectorServer::new` starts with every headed application and accepts unauthenticated loopback requests. `POST /messages` can move or delete entities and trigger actions, and `DELETE /` closes the process.
- Add a starburst and a lens dirt texture to the screen-space lens flare in `crates/byte-engine/src/rendering/render_passes/lens_flare.rs`. The starburst should rotate with the camera's orientation from `Sink::view`, and the dirt texture should modulate the flare, and optionally the bloom, at composite time. Both need texture assets loaded by the pass.
- Build the CPU animation graph, evaluate imported glTF and FBX clips into `VisibilitySceneManager::write_skinned_pose`, apply retained rigid primitive nodes, and provide animation-safe bounds so posed meshlet culling can be re-enabled.

- Give skinned primitives detail levels once animation-safe bounds exist. `MeshProcessorSession` keeps them at full detail because simplification error is measured in the bind pose and the instance sphere is the rest pose, so the renderer could not tell when a coarser level is safe.

## Surfel global illumination

- Let rays hit skinned meshes, and let surfels follow moving instances. The top level places only unskinned geometry (`crates/byte-engine/src/rendering/pipelines/visibility/acceleration_structures.rs`), so characters neither block bounce light nor reflect it, and surfels keep the world position they spawned at, so the light on a moving object lags behind it until its surfels age out. Skinned meshes need the refitting bottom levels above; moving instances need surfels stored relative to their instance, which the grid then lists at the instance's current transform.
- Light ray hits with point and cone lights. The shade pass (`crates/byte-engine/assets/rendering/visibility/surfel-shade.besl`) lights a hit with the first four suns and with the surfels around it, so a lamp's light reaches surfels only through the surfaces the camera sees. Read the light clusters at the hit, or keep a short list of the brightest local lights, and trace a shadow ray toward each.
- Test alpha against masked primitives in surfel rays, with any-hit functions. Masked and blended primitives stay out of the bottom levels, so rays pass through foliage and glass. Add one generic any-hit function that reads the material's coverage texture and alpha cutoff, run it through an intersection function buffer at the same record indices as the hit groups, and mark the geometries without one opaque. Rebuild a mesh's bottom level when a material's alpha mode changes on reload; the build reads the modes only once.
- Record the frame-wide surfel stages on an asynchronous compute queue. The first sink records them before its raster passes, so the GPU runs them alongside those passes, which saved 0.27 ms on Sponza, but only until a later barrier waits for all earlier compute work; on a queue of their own, the grid, the trace, and the integrate pass could overlap any pass that doesn't read their results.
- Give every sink its own surfel grid, or center the grid between the sinks' cameras. The grid follows the first sink's camera (`SurfelGiParameters::grid_origin`), so a second sink far from it reads coarse cascades or no surfels at all.
- Rename `SSGI History` and `SSGI Normals` to names that cover both sources of indirect diffuse light, now that surfel GI writes them too, and update the targets table in `docs/reference/rendering.mdx`.
- Cut the rest of the surfel resolve's radial depth test, about 0.3 ms of `screen-space` on Sponza at 1024 × 1024 on an Apple M4. Each surfel's clearance (`surfel_moment_clearance` in `crates/byte-engine/assets/rendering/visibility/layouts/surfel-gi.besl`) is the minimum over its 16 cells, so a wall in any direction makes every pixel around the surfel map its direction and read four cells. On the stage E7b shaders, the three extra cell reads cost 0.15 ms and the four-moment solve 0.04 ms; the rest is each pixel and surfel pair's setup and first read. Try clearances per quadrant of the surfel's plane, where most of the pixels it lights lie, or loading each tile's surfels and their cells into workgroup memory.
- Tune surfel GI's defaults on scenes larger than Sponza: the surfel capacity, the ray budget, the 64 surfels a pixel reads, and whether surfels that pixels vote for get their rays first. Sponza settles at about 1,000 surfels and 4,500 rays a frame, far below the defaults of 32,768 surfels and 16,384 rays, whose buffers take about 24 MiB.
- Trace webgiya's depth probe rays: one ray for a quarter of the surfels each frame, in a depth cell picked in turn, which updates only the radial depth. Guided rays visit dark directions rarely, so their depth cells go stale while the scene changes.
- Light enclosed rooms evenly and sooner. In Sponza's gallery, most diffuse light is bounce light spread from the few surfels whose rays found the sunlit courtyard, about one ray in several thousand: with no surfel light at hits, 88% of the deep gallery goes black. Its light spans a factor of 6 to 9 between the 10th and 90th percentile of pixels, and three guided runs read a third of an unguided run's level 600 frames after the view was pinned (1.7–1.9e-5 against 5.6e-5 pre-exposed), while runs captured later all read 4–6e-5. Measure the build-up at matched surfel ages first, then try sharing estimates between neighboring surfels that face the same way, and more rays for surfels whose light comes from rare bright hits.

## Shader behavior

- Implement Metal interpolation and metadata-driven push-constant mapping without hardcoded backend conventions.

## MaterialX conversion

- Reduce MaterialX scattering closures onto a BEMX `Surface`, so a `<surface>` node built from `oren_nayar_diffuse_bsdf`, `dielectric_bsdf`, `conductor_bsdf`, `layer` and `mix` converts instead of reporting an unsupported shading model. This is most of the MaterialX pbrlib test suite and the pbrlib-native way to write a surface.
- Let a MaterialX `<image>` sample with transformed texture coordinates. BEMX image nodes already take a `texcoord` input; the visibility shader generator needs to honour it, sampling with the given coordinate and carrying the matching texture-coordinate derivatives, otherwise a tiled or placed image picks the wrong mip level. Until then `place2d`, `tiledimage` and `hextiledimage` are reported as unsupported.
- Add BEMX nodes for the MaterialX procedural and colour-conversion nodes the standard library defines with source code rather than node graphs, and map them in `materialx::bemx`: `noise2d`, `noise3d`, `fractal3d`, `cellnoise2d`, `worleynoise2d`, `flake2d`, `flake3d`, `hsvtorgb`, `rgbtohsv` and `blackbody`.

# P2 - BESL architecture

- Add explicit interpolation syntax and a generic texture/sampler resource model.
- Complete MSL lowering for all declared intrinsics and reconcile `fetch_u32`, `image_atomic_or`, and `image_load_u32`.
- Add 3D compute built-ins and address-space semantics.
- Add task-payload, workgroup-storage, and task-dispatch lowering for GLSL and HLSL; the external visibility task shaders currently support Metal only.
- Add missing control flow, boolean and numeric types, typed textures, matrix shapes, and structured array support.
- Represent threadgroup size, matrix layout, function constants, and other MSL compile options in shader metadata.
- Analyze each BESL graph once with visited-node tracking and keyed binding deduplication, sharing results across generation, reflection, and opacity evaluation.
- Write GLSL/MSL directly into capacity-estimated output strings instead of creating repeated temporary formatted strings.

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
- Build a level manager on top of `Scene`/`SceneNode` (`crates/byte-engine/src/gameplay/scene.rs`): active-scene switching, loading levels from assets, and optional re-parenting of scene members.

## Ownership cleanup (deferred)

- Replace the process-wide `COUNTER` in `crates/byte-engine/src/core/factory.rs` with an id counter owned by the world or message bus and passed to the factories. Deferred on request during the Rc/Arc and globals cleanup.
- Remove the shared `Arc<AssetManagerState>` in `crates/resource-management/src/asset/manager.rs` (and the dependent file-watcher `Weak`, `in_flight_bakes` Arc, bake-memory Arcs, shared storage backend, material mip generator Arc, and test counters). compio dispatch requires `'static` jobs; the options considered (no dispatcher + owned-data compute pool, coordinator/actor, whole pool inside `std::thread::scope`) were rejected, so a different design is needed.

## GPU counters (deferred)

- Add a pipeline-statistics counter kind to the GHI counters API for Vulkan and DX12: `create_counter` takes a `CounterKind`, `Context::counter_statistics` returns input vertices and primitives plus vertex, geometry, clipper, fragment, compute, task, and mesh invocations and clipper and mesh primitives, gated by `supports_pipeline_statistics`. Vulkan needs the `pipelineStatisticsQuery` feature and `meshShaderQueries` for the mesh counts; DX12 uses `PIPELINE_STATISTICS1` where the device supports it. Metal 4 counter heaps hold only timestamps, so Metal always reports `None`. A statistics span is one query slot whose begin and end must sit in the same command buffer, and on Vulkan inside the same render pass instance or both outside it. Deferred on 2026-10-04 so timing counters shipped first.
- Let counters record in detached recordings (loader contexts, the resource-management GPU worker). Timing counters are frame-only because slots are owned per frame sequence; detached recordings would need per-recording slot pages released on synchronizer completion, like texture readbacks.
- Grow the counter slot pool instead of capping a frame at 512 counters, if a frame ever needs more.
