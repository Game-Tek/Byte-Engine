# P0 - Correctness and stability

- Fix the Cube test hang and gate renderer/window integration tests so normal test and `cargo llvm-cov` runs complete.
- Make the UDP client and server exchange canonical BETP datagrams. `crates/byte-engine/src/network/client/udp.rs` treats `WouldBlock` as `IoError`, never decodes the receive buffer, and always sends a 1024-byte buffer. A data packet is 1045 bytes, so `write_packet` fails and the socket sends zeros. Handshake packets that fit are padded, and `read_packet` rejects any length other than the exact size. `crates/byte-engine/src/network/server/udp.rs` blocks in `recv`, ignores the datagram, and never inserts a client. `crates/byte-engine/src/network/server/server.rs` still only logs connect and disconnect.
- Deliver in-process channel `Data` only after the session is connected, and apply each reliable payload once. `crates/byte-engine/src/network/server/channel.rs` pushes every `Data` packet into `received` before accept, and reliable sends stay queued for eight attempts. Stop using `client_salt ^ 0x4254_4550` as the connection id.
- Retransmit a lost challenge response. After a matching challenge, the BETP client enters `Connecting` and the next update, including an idle one, becomes `Connected` without sending the response again (`crates/betp/src/client/session.rs`). The server connects only when that response arrives.
- Return a recoverable error from Vulkan `vkQueuePresentKHR` when the surface is out of date. `crates/ghi/src/vulkan/frame.rs` uses `.expect("No present")`, so a resize aborts the process. A second present in the same submit also waits again on a binary semaphore that was already signaled.
- Release or present an acquired swapchain image when its extent is `0` or at least 65535. `crates/byte-engine/src/rendering/renderer/core.rs` drops the `PresentKey` and stores `None`, so a minimized window keeps the image acquired and later acquires fail.
- Reject glTF and FBX URIs that escape the asset root. `resolve_gltf_uri` returns absolute paths and `..` joins unchanged (`crates/resource-management/src/asset/handler/implementations/gltf/io.rs`), and `read_asset_from_source` opens `base_path.join(url)`. On Unix an absolute URI replaces the base. FBX textures take the same path. Environment maps already reject this.
- Make Vulkan `set_frames_in_flight` grow and shrink without sharing resources or panicking. Lowering the count hits `unimplemented!()` in `crates/ghi/src/vulkan/context/traits.rs`. Raising it extends image and synchronizer chains by one node and does not rebuild dynamic buffers, so two sequences can share one `VkBuffer`. The renderer currently stays at 2, which matches context creation.
- Copy retained bytes on GHI buffer resize, and free Vulkan image memory when an image is resized. `crates/ghi/src/vulkan/context/resources.rs` still has the copy todo, asserts when the buffer has staging, and can delete the old `VkBuffer` after only one sequence fence. Image resize leaves the old `VkDeviceMemory` allocated until the context drops. Metal and DX12 also replace a resized buffer without copying the previous CPU contents.
- Rebuild Metal dynamic resources when the frame count changes. `crates/ghi/src/metal/context/resources/allocation.rs` only retires upload slots and resizes `internal_upload_queues`. Descriptor sets, synchronizers, and dynamic buffers stay chained at their old length, so `nth_handle` reuses the last node.
- Fail DX12 pipeline creation when no native pipeline state exists. `crates/ghi/src/dx12/context/pipelines.rs` still returns a handle, and later dispatch records no work when `pipeline_state` is `None`.
- Close the BESL language gaps that SSGI had to work around, then remove the workarounds from `crates/byte-engine/assets/rendering/visibility/ssgi-*.besl`. BESL has no `else`, and assigning one component of a local vector fails with "Accessor is not rooted in a buffer binding". Each gap needs the parser or lexer, the VM, and the MSL, HLSL, and GLSL backends.
- Rebake generated visibility material shaders when the material-evaluation interface changes. After adding SSGI bindings, a sandbox app loaded stale material shaders from its resource cache and panicked with "Missing retained descriptor at resource slot 1056".
- Return a parse error for a zero-length BESL array. `u32[0]` is accepted, then `Node::array` expects a non-zero size in `crates/besl/src/lexer/ast.rs`.
- Emit `vec3u16` raster I/O as a non-interpolated integer on HLSL, GLSL, and MSL. `is_integer_type` lists the 2- and 4-component forms and omits `vec3u16`, so the stage output is interpolated or the backend compile fails.
- Treat a forward BETP sequence gap above 32768 as newer, or reject it, so the receive window cannot stall. `sequence_greater_than` in `crates/betp/src/lib.rs` treats that distance as older, and the packet is dropped without moving `ack`.
- Make Linux audio pause return an error when the device does not support ALSA pause. `crates/ahi/src/os/linux.rs` uses `pcm.pause(true).unwrap()`, and `play` panics for a channel count other than 1 or 2. On Windows, reject a closest match the renderer cannot play, request 32-bit streams as float rather than integer PCM, and treat `CoInitializeEx` returning `S_FALSE` as already initialized. `play` panics outside 16- or 32-bit mono and stereo, and a 32-bit stream is requested as `KSDATAFORMAT_SUBTYPE_PCM` while samples are written as `f32`.
- Migrate the Vulkan and DX12 GHI backends from legacy descriptor templates to retained flat `ResourceSlot` writes and pipeline-derived native layouts.
- Fix the macOS `NSWindow canBecomeKeyWindow` warning.
- Give every BESL specialization member its own constant index on every backend. MSL numbers each specialization's members from 0 (`crates/resource-management/src/shader/besl/backends/msl/emit.rs`), so two specializations both use `function_constant(0)`, while the renderer passes the variant variable index (`crates/byte-engine/src/rendering/pipeline_compilation.rs`). HLSL emits each member as `static const … = 1.0f` (`crates/resource-management/src/shader/besl/backends/hlsl/generate.rs`), so DX12 ignores the values.
- Define texture usage semantics for resources consumed by multiple unknown render passes.
- Give UI element paths an identity that cannot repeat between live scopes. `RetainedTree::begin_frame` clears `path_counts`, and `scope_path` assigns the ordinal from that per-frame counter (`crates/byte-engine/src/ui/layout/retained_tree.rs`). Two mounts with the same name under one parent started on different frames share a path, share element ids, and removing one removes the other's elements. Task ownership already uses `ScopeId` and is unaffected.

# P1 - Runtime performance

## Frame and rendering

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

## UI

- Reuse UI layout, draw-list, geometry, vertex, index, text, relation, and batch storage instead of rebuilding and cloning it each frame.
- Replace UI layout relation and element scans with an ID index and contiguous parent-to-children adjacency storage.
- Replace hit-test `Vec<Vec<usize>>` buckets with contiguous candidate storage plus per-cell ranges.
- Return references or compact handles for UI primitive shapes instead of cloning them.
- Drive multiple animations concurrently using a single animation driver.

## Physics

- Persist broadphase endpoints and implement a true sweep-and-prune active set with insertion sorting for temporal coherence.
- Reuse endpoint, pair, and contact scratch storage based on body count and previous overlap counts.
- Resolve contacts through disjoint mutable body access instead of cloning complete bodies.
- Group contacts by time of impact or use collision substeps so every body is not advanced once per contact.

## Audio and input

- Resolve named input triggers to handles during action registration and index device classes and triggers by name.
- Reuse gamepad event, new-device, and present-path scratch storage; allocate owned HID paths only for confirmed new devices.
- Decide how modifier keys behave across input sinks: a sink that captures `Ctrl` currently claims it, so a later sink's `Ctrl+S` no longer resolves. Consider modifier-only controls that sinks read without claiming.

## Metal-specific

- Batch pending Metal buffer and texture uploads into one transfer command buffer and blit encoder.
- Avoid cloning Metal texture staging data and pipeline state during upload and descriptor binding.
- Simplify Metal frame-chain handle deduplication if frame counts grow beyond the current small fixed count.

# P1 - Bake and asset performance

- Replace asset-handler tuple results with an allocator-backed or borrowed `BakedAsset` payload that storage can consume before arena reset.
- Redesign `MeshProcessor` as a two-pass packer that computes offsets and writes directly into one final allocation.
- Let glTF parsing borrow GLB and external BIN data instead of copying whole buffers.
- Flatten glTF traversal into one caller-owned primitive record buffer rather than separate tree and primitive collections.
- Bake each unique glTF material once, resolve primitive material references concurrently, and reuse generated resources.
- Move generated glTF and FBX material factors (base color, metallic, roughness, emission, normal scale, occlusion strength) out of the BESL source into per-material GPU data, so materials that differ only in factors share one compiled shader. Generated shaders are already shared per graph (`store_generated_brdf_shaders`), but factors are still written into the program as literals. Specialization constants can't carry them yet (see the P0 specialization entry); a factor array in the visibility `Material` struct, filled from variant variables, would.
- Extract independent glTF primitive attributes concurrently before constructing the mesh source.
- Run independent BEMA shader loading/compilation and material/variant variable resolution concurrently while preserving order.
- Compress generated image mip levels concurrently after mip-chain generation.
- Drain BELD's buffered task stream directly instead of collecting unit results.
- Replace formatted mip stream names with typed stream identifiers or prefix-plus-index metadata.

# P2 - Platform and feature completeness

## GHI and windows

- Complete DX12 command recording and device support for resources, pipelines, uploads, mesh shading, DXR, shader tables, fences, and submission.
- Implement Vulkan standalone command-buffer execution.
- Implement Metal ray tracing pipelines, acceleration structures, instance data, shader binding tables, and ray dispatch.
- Decide how GHI should handle potentially unused staging buffers.
- Implement macOS cursor visibility and confinement.
- Wire real platform input seats across X11, Wayland, Win32, and the byte-engine input collector.

## Engine systems

- Replace the fitted ACES grading output with the official ACES 2.0 Rec.709-D65 100-nit sRGB-piecewise transform, including AP1-to-AP0 conversion, precomputed hue/gamut tables, complete CAM/JMh tone/chroma/gamut processing, Apache-2.0 attribution, and Academy golden-image validation.
- Support self-overlapping and intersecting transparent surfaces with forward per-fragment shading or OIT.
- Implement sampled UI colors, the remaining UI layout branch, primitive style access, and non-box bounding boxes.
- Spawn and despawn a server-side client entity when BETP reports `ClientConnected` and `ClientDisconnected`. `crates/byte-engine/src/network/server/server.rs` logs those events and leaves the entity empty.
- Make the HTTP inspector opt-in, and keep a busy port 6680 from aborting startup. `HttpInspectorServer::new` starts with every headed application, panics if the port is taken, and accepts unauthenticated loopback requests. `POST /messages` can move or delete entities and trigger actions, and `DELETE /` closes the process.
- Add a starburst and a lens dirt texture to the screen-space lens flare in `crates/byte-engine/src/rendering/render_passes/lens_flare.rs`. The starburst should rotate with the camera's orientation from `Sink::view`, and the dirt texture should modulate the flare, and optionally the bloom, at composite time. Both need texture assets loaded by the pass.
- Build the CPU animation graph, evaluate imported glTF and FBX clips into `VisibilitySceneManager::write_skinned_pose`, apply retained rigid primitive nodes, and provide animation-safe bounds so posed meshlet culling can be re-enabled.

## Shader behavior

- Implement Metal interpolation and metadata-driven push-constant mapping without hardcoded backend conventions.

## MaterialX lowering

- Reduce MaterialX scattering closures onto the renderer's metallic-roughness BRDF, so a `<surface>` node built from `oren_nayar_diffuse_bsdf`, `dielectric_bsdf`, `conductor_bsdf`, `layer` and `mix` lowers instead of reporting an unsupported shading model. This is most of the MaterialX pbrlib test suite and the pbrlib-native way to write a surface.
- Let a MaterialX `<image>` sample with transformed texture coordinates. This needs `sample_material` to take a coordinate argument and the visibility material stage to carry the matching texture-coordinate derivatives, otherwise a tiled or placed image picks the wrong mip level. Until then `place2d`, `tiledimage` and `hextiledimage` are reported as unsupported.
- Add the MaterialX procedural and colour-conversion nodes the standard library defines with source code rather than node graphs: `noise2d`, `noise3d`, `fractal3d`, `cellnoise2d`, `worleynoise2d`, `flake2d`, `flake3d`, `hsvtorgb`, `rgbtohsv` and `blackbody`.

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
- Add an in-process UDP client connection test and server lifecycle coverage. The current UDP adapters discard datagrams, so this test fails until that transport sends and decodes canonical packets.
- Run CI once with the default `headed` and `network` features together. The workflow enables only one of those features per job, and the default nextest filter excludes the native GHI rendering binary.
- Extend BETP and raw-datagram fuzzing past canonical packets. The current targets do not cover the UDP adapters, a lost challenge response, two clients on one socket, or a sequence gap above 32768.
- Add UI tests for sampled colors, remaining layout behavior, primitive styles, and non-box bounds.
- Review and remove or use dead `TestTransport` and `TestSynthesizer` helpers.

# P3 - Conditional optimizations

- Add a free-slot stack and sequence-to-slot index to BETP packet buffering if profiling shows fixed-array scans are significant.
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
