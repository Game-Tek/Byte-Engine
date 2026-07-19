# P0 - Correctness and stability

- Fix the Cube test hang and gate renderer/window integration tests so normal test and `cargo llvm-cov` runs complete.
- Fix visibility rendering to use scene instance indices instead of loaded mesh indices in `crates/byte-engine/src/rendering/pipelines/visibility/render_pass.rs`.
- Fix texture/material upload ordering that causes black-object flashes, including correct synchronization before rendering starts.
- Support applications with no audio endpoint.
- Make Linux audio pause tolerate devices without ALSA pause support, make Windows format negotiation return an error instead of panicking, and implement or document Windows pause behavior.
- Rebuild Metal dynamic resources correctly when swapchain frame counts change in `crates/ghi/src/metal/context.rs`.
- Support Vulkan frame-count reductions and replace unimplemented internal-handle translation with explicit handling or a recoverable error.
- Migrate the Vulkan and DX12 GHI backends from legacy descriptor templates to retained flat `ResourceSlot` writes and pipeline-derived native layouts.
- Fix the macOS `NSWindow canBecomeKeyWindow` warning.
- Define texture usage semantics for resources consumed by multiple unknown render passes.
- Remove completed audio sources instead of retaining and revisiting them in `crates/byte-engine/src/audio/audio_system.rs`.

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
- Precompute render-pass resource access and attachment templates instead of rebuilding hash maps and vectors each frame.
- Remove material names from non-debug visibility builds.
- Add reusable or allocator-aware listener draining.
- Replace generated meshlet membership scans with fixed-capacity local storage and a generation-tagged global-to-local lookup, or consistently use meshopt.
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

## Metal-specific

- Batch pending Metal buffer and texture uploads into one transfer command buffer and blit encoder.
- Avoid cloning Metal texture staging data and pipeline state during upload and descriptor binding.
- Simplify Metal frame-chain handle deduplication if frame counts grow beyond the current small fixed count.

# P1 - Bake and asset performance

- Make BELD bake concurrency and initial arena capacity configurable or memory-aware instead of reserving sixteen 32 MiB arenas.
- Replace asset-handler tuple results with an allocator-backed or borrowed `BakedAsset` payload that storage can consume before arena reset.
- Redesign `MeshProcessor` as a two-pass packer that computes offsets and writes directly into one final allocation.
- Let glTF parsing borrow GLB and external BIN data instead of copying whole buffers.
- Flatten glTF traversal into one caller-owned primitive record buffer rather than separate tree and primitive collections.
- Bake each unique glTF material once, resolve primitive material references concurrently, and reuse generated resources.
- Extract independent glTF primitive attributes concurrently before constructing the mesh source.
- Store glTF texture dependencies concurrently while preserving variable order.
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
- Wire real platform input seats across X11, Wayland, Win32, and the byte-engine input manager.

## Engine systems

- Implement sampled UI colors, the remaining UI layout branch, primitive style access, and non-box bounding boxes.
- Implement server-side client entity lifecycle and replace the temporary UDP client identity strategy.
- Build the CPU animation graph, evaluate imported glTF and FBX clips into `VisibilitySceneManager::write_skinned_pose`, apply retained rigid primitive nodes, and provide animation-safe bounds so posed meshlet culling can be re-enabled.

## Shader behavior

- Implement Metal interpolation and metadata-driven push-constant mapping without hardcoded backend conventions.

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
- Fix or replace ignored Vulkan WSI and ray-tracing tests.
- Add targeted GHI backend tests or fakes for device, context, resource, and command lifecycle behavior.
- Add focused window tests, including macOS keyboard consumption, cursor visibility, and confinement.
- Replace ignored glTF, WAV, and PNG tests with committed or generated fixtures.
- Fix ignored asset-manager dependency-injection and BESL member-lexer tests.
- Test asset path handling.
- Add an in-process UDP client connection test and server lifecycle coverage.
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

- Split each large backend context implementation into resources, descriptors, pipelines, synchronization, transfers, and acceleration-structure modules while keeping the public context type in `context/mod.rs`.
- Move GHI handles, resource descriptions, and behavioral traits into their existing domain modules instead of declaring most contracts in `graphics_hardware_interface.rs`.
- Reduce `graphics_hardware_interface.rs` to compatibility re-exports or remove it after callers migrate to domain modules.

## Cross-cutting layout

- Keep every `#[cfg(test)] mod tests` at the end of its production module.
- Split production files once they contain multiple independently testable responsibilities, rather than using line count alone.
- Prefer operation-specific context structs for cohesive long argument lists; do not replace them with broad service-locator-style bags.
- Keep tests in the new owning submodules instead of retaining large centralized test sections.
- Avoid creating additional crates until module-level splits show a stable dependency boundary that needs independent compilation or ownership.
- Prioritize the visibility pipeline refactor first because it combines the greatest file size, dependency breadth, duplicated shader contracts, and constructor complexity.
