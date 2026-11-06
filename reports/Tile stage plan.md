# A BESL `Tile` stage for screen passes

**Every screen pass in the engine is a compute shader that writes a storage image, so every target it writes gets
`MTLTextureUsageShaderWrite`, which Apple lists as a reason not to compress a texture.** This plan adds a BESL
`Tile` stage. You write it like a compute shader: thread values from `StageInput`, workgroup storage, and barriers.
Its outputs, though, are declared attachments rather than storage images. The MSL backend lowers it to a Metal tile
function that a render pass dispatches once per tile. Its outputs become imageblock color attachments, so their
targets need only `RenderTarget` usage and stay eligible for lossless compression. The GLSL and HLSL backends lower
the same source to a compute kernel. The baked `Shader` records both the stage the source declares and the native
stage its artifact holds. The GHI uses that record to build the right pipeline, dispatch it the right way, and refuse
a mismatch between a target's usage and the shader that writes it.

Phase 0 has to prove the gain on the M4 before anything lands in the engine. An earlier measurement here kept a D16
target compressed even with `shaderWrite` set (memory note `byte-engine-shadow-pyramid-bound`). Apple's own pages
disagree on whether the rule covers Apple silicon Macs. The general article lists `shaderWrite` as disqualifying on
Apple5 and later ([Optimizing texture data](https://developer.apple.com/documentation/metal/optimizing-texture-data)),
while the `shaderWrite` page scopes it to "iOS devices with GPU family 5"
([shaderWrite](https://developer.apple.com/documentation/metal/mtltextureusage/shaderwrite)). A published tile
encoder measured 1.21x over compute on an M4 MacBook Pro and 1.4–1.65x on iPhones
([Ludicon, 2026](https://www.ludicon.com/castano/blog/2026/09/encoding-render-targets-for-free-with-tile-shaders/)).
That result is promising but comes from a different workload.

## 1. What the code does today

| Fact | Where |
| --- | --- |
| `Uses::Storage` adds `ShaderWrite`; `RenderTarget`, `DepthStencil`, and `Clear` add `RenderTarget`. | `crates/ghi/src/metal/mod.rs:128-145` |
| Every scaled render target already gets `RenderTarget` through `Uses::Clear`, so dropping `Storage` alone removes `ShaderWrite`. | `crates/byte-engine/src/rendering/render_pass.rs:415-422` |
| Bloom output and pyramid levels are `Storage \| Image`; so are lens flare, motion blur, SMAA, LUT, image transforms (ACES, AgX, sRGB), height fog, and sky. | `render_passes/bloom.rs:104,112`, `lens_flare.rs:117-132`, `motion_blur.rs:162`, `smaa.rs:78-83`, `image_transform.rs:59-62`, `height_fog.rs:93` |
| The bypass copy that forwards `main` while a pass is off is a compute storage write, so a `main` replacement needs `Storage` until the blit converts too. | `render_passes/blit.rs:16-42`, `assets/rendering/blit/image.besl` |
| The swapchain is created `RenderTarget \| Storage \| TransferSource` with `framebufferOnly = false`. | `renderer/core.rs:1102-1108`, `ghi/src/metal/context/traits.rs:147` |
| Nothing in GHI, BESL, or the docs supports tile functions. `objc2-metal` 0.3.2 enables `MTL4TileRenderPipeline` by default, and the render pass descriptor exposes `tileWidth`, `tileHeight`, `threadgroupMemoryLength`, and `imageblockSampleLength`. | `crates/ghi/Cargo.toml:62` |
| A `dispatch` inside an open render pass silently ends the pass, and `create_raster_pipeline` panics without a vertex or mesh shader. Tile work needs its own path. | `ghi/src/metal/command_buffer/recording.rs:484-503`, `ghi/src/metal/pipeline.rs:798-802` |
| BESL already lowers a stage to a different native stage: `RayGeneration` becomes an MSL compute kernel and `ClosestHit` a visible function. The Metal backend knows this only implicitly. | `docs/develop/besl/compilation.mdx` ("Generate ray tracing shaders") |

The bloom work in the working tree has the extract and the upsample composite write a 2x2 block per lane, with 8x8
and 16x16 workgroups and up to 1.6 KiB of workgroup storage. The stage must keep those optimizations, so it can't be
limited to one pixel per thread.

## 2. The stage as authors see it

`.besl.bead`:

```json
{ "stage": "Tile", "tile": 16, "workgroup": [16, 16] }
```

- `tile` is the edge of a square tile in pixels: `16` or `32`.
- `workgroup` is the threads per tile. It defaults to `[tile, tile]`, one pixel per thread. `[8, 8]` with a 16-pixel
  tile gives each thread a 2x2 block, as the bloom extract uses now.

Downsample, converted:

```rust
source_texture: descriptor<{ type: Texture2D, binding: 0, access: read }>;
result: tile_output<vec4f, 0>;

main: fn (input: StageInput) -> void {
	let coord: vec2u = input.thread_id;
	let uv: vec2f = (vec2f(f32(coord.x), f32(coord.y)) + vec2f(0.5, 0.5))
		/ vec2f(f32(input.extent.x), f32(input.extent.y));
	// ... the same 13 taps ...
	write(result, coord, vec4f(averaged.x, averaged.y, averaged.z, 1.0));
}
```

What changes from compute:

- **Outputs are `tile_output<TYPE, LOCATION>` declarations**, like the existing mesh `output<..>` and
  `vertex_output<..>` declarations. `LOCATION` is the color attachment, from 0 to 7.
- **`write(output, pixel, value)` takes a global pixel coordinate inside the invocation's own tile.** The stage drops
  writes outside the outputs' extent by itself, so shaders don't call `guard_image_bounds` on outputs.
- **New `StageInput` fields:**
  - `thread_id` (`vec2u`) keeps its compute meaning: tile position × threads per tile + thread position. Ported code
    keeps its coordinate math.
  - `thread_idx` (`u32`) is the row-major index inside the tile on every backend.
  - `tile_position` (`vec2u`) is the tile's position in the grid of tiles.
  - `extent` (`vec2u`) is the outputs' size. A Metal tile function can't query its attachments' size, so
    `image_size(output)` has no lowering.

## 3. Rules the stage enforces

These are the constraints of a Metal tile function, plus the ones the compute fallback needs to give the same
results. `validate_stage` in `backends/stage_rules.rs` checks most of them before any generator runs, and each error
names the rule and the fix, as the ray tracing rules do.

| Rule | Why | Checked by |
| --- | --- | --- |
| The shader declares 1–8 `tile_output`s with unique locations, of type `f32`, `vec2f`, `vec4f`, their `f16` forms, `u32`, or `vec4u`. | Outputs are color attachments of one render pass. | Lexer and stage rules |
| `write` to an output stays inside the invocation's tile. | An imageblock holds only the tile. Compute could write anywhere, which wouldn't match. | VM (error); backends mask the write |
| No `guard_image_bounds`, `image_size`, or `image_load` on an output. | They need a texture the tile function doesn't have. Early returns would also skip barriers. | Stage rules |
| `tile` is 16 or 32, square; `workgroup` is square, depth 1, divides `tile` by 1, 2, or 4, and holds at most 256 threads. | Metal tile sizes, and some GPU families require square tile dispatches ([dispatchThreadsPerTile](https://developer.apple.com/documentation/metal/mtl4rendercommandencoder/dispatchthreadspertile(_:))). 256 keeps the fallback inside common workgroup limits. | `.besl.bead` parsing, schema |
| Of the subgroup intrinsics, only `subgroup_shuffle_xor_f32` is allowed, with a literal mask of 1, 2, or 3. | Tile-function SIMD lanes are quad-ordered, unlike compute threadgroups (memory note `byte-engine-shadow-pyramid-bound`). Only the 2x2 quad can mean the same pixels on every backend: mask 1 is the horizontal neighbor, 2 the vertical, and 3 the diagonal. Ballots, broadcasts, and `subgroup_lane_index` expose lane order, so they're rejected. | Stage rules |
| No `discard`, `sample`, `fwidth`, ray queries, or `invoke`. | A tile function is a kernel: no derivatives and no fragment. Ray queries stay out of the first version. | Stage rules |
| Workgroup storage is allowed. | Tile functions have threadgroup memory. | `validate_workgroup_storage_stage` (`generator.rs:257-267`) |
| Workgroup bytes plus imageblock bytes per tile fit the device's tile memory. | Imageblock and threadgroup memory share tile memory. Formats are known only at pipeline creation. | GHI, from `imageblockMemoryLengthForDimensions` |
| Storage-image writes are allowed, but never to an image the same dispatch writes as an output. | Motion blur writes its debug-view targets as storage images. An attachment can't also be a storage target in its own pass. | GHI at dispatch |

## 4. Lowering

### MSL: a tile function

```metal
struct _besl_tile_outputs { float4 result [[color(0)]]; };

kernel void besl_main(
	imageblock<_besl_tile_outputs, imageblock_layout_implicit> tile_block,
	ushort2 local [[thread_position_in_threadgroup]],
	uint2 tile_position [[threadgroup_position_in_grid]],
	constant PushConstant& push_constant [[buffer(15)]],  // carries the hidden extent
	constant _set0& set0 [[buffer(16)]])
{
	uint2 gid = tile_position * THREADS_PER_TILE + uint2(local);
	// write(result, pixel, value):
	ushort2 p = ushort2(pixel - tile_position * TILE);
	if (all(p < TILE) && all(pixel < extent)) {
		_besl_tile_outputs o = tile_block.read(p); o.result = value; tile_block.write(o, p);
	}
}
```

- Today a compute-path `Output` node lowers to a global `[[color(n)]]` declaration, which isn't valid in a kernel
  (`msl/raster.rs:483-484`, `msl/emit.rs:281-299`). Use a new `TileOutput` node instead of reusing `Output`.
- **Extent:** the generator appends a hidden `uint2` to the push constant and records its offset in the provenance.
  The GHI writes the render area there at dispatch. If Phase 0 finds that `threads_per_grid` gives the exact extent
  in a tile function, the hidden field goes away.
- **Workgroup storage:** compute declares it inside the kernel body (`msl/bindings.rs:271-289`). If tile functions
  reject body-declared threadgroup arrays (Phase 0, question 4), lower it as `threadgroup T* [[threadgroup(i)]]`
  arguments. Their lengths and offsets go in the provenance so the GHI can call `setThreadgroupMemoryLength`.
- Helpers that write outputs take `tile_block` and the tile position as trailing hidden parameters, the way compute
  helpers take `gid` (`msl/bindings.rs:618-648`).

### GLSL and HLSL: a compute kernel

- **Workgroup:** a one-dimensional workgroup of `workgroup.x * workgroup.y` threads.
- **Thread positions:** `thread_position` comes from the local index in quad order. Lanes `4k` to `4k+3` form one
  2x2 quad, so masks 1, 2, and 3 address the same neighbors as on Metal.
- **Outputs:** formatless storage images in one trailing descriptor set the GHI owns, with the binding equal to the
  location. `write` becomes `imageStore` under the same tile and extent mask. `extent` is `imageSize` of output 0.
- Per the memory note `byte-engine-no-cross-platform-checks`, neither backend can be compiled on this Mac. Flag both
  as unverified.

### VM

The VM runs each tile as a workgroup through `run_workgroup` (`besl/src/vm/execution.rs:148+`). It uses the same
quad lane order, binds outputs as images by location, and fails on a write outside the invocation's tile. The VM is
the reference that the GPU test compares against.

## 5. Stage provenance in the baked shader

"Provenance" already means source-asset tracking in the bake (`asset/handler/asset_handler.rs:89`), so this plan
names the new field `native` to keep the two apart. Rename it if you'd rather use the word.

```rust
// crates/resource-management/src/resources/material.rs
pub struct Shader {
	pub id: String,
	/// The stage the source declares in its `.besl.bead`.
	pub stage: ShaderTypes, // gains `Tile`
	/// What the platform artifact runs as, and what the runtime must provide for it.
	pub native: NativeStage,
	pub interface: ShaderInterface,
	pub artifact: ShaderArtifact, // unchanged
}

pub struct ShaderInterface {
	pub workgroup_size: Option<(u32, u32, u32)>, // threads per tile for `Tile`
	pub sets: Vec<SetInterface>,
	pub ray_payload: Option<RayPayload>,
	/// The tile and the outputs of a `Tile` shader; `None` for every other stage.
	pub tile: Option<TileInterface>,
}

pub struct TileInterface {
	pub size: u32,                 // pixels along each edge
	pub outputs: Vec<TileOutput>,  // in location order
	pub workgroup_bytes: u32,
}

pub struct TileOutput {
	pub name: String,     // the BESL name, which render passes bind by
	pub location: u32,
	pub value: TileValue, // f32, vec2f, vec4f, f16 forms, u32, vec4u
	pub loads: bool,      // false until output reads exist (Phase 6)
}

pub enum NativeStage {
	/// The artifact runs as `Shader::stage`.
	Declared,
	/// Metal ray generation kernel and closest-hit visible function: today's implicit contracts, made explicit.
	MetalRayGenerationKernel,
	MetalVisibleFunction,
	/// A Metal tile function, dispatched per tile in a render pass. Outputs are imageblock color attachments, so their
	/// images need `RenderTarget` usage and never `ShaderWrite`.
	MetalTileFunction {
		extent_offset: u32,                   // hidden uint2 in the push constant
		threadgroup: Vec<ThreadgroupArgument>, // empty when workgroup storage is body-declared
	},
	/// A compute kernel standing in for `Tile`. Outputs are storage images in set `output_set`, binding = location.
	ComputeTile { output_set: u32 },
}
```

**Who writes it.** `finalize_platform_shader_artifact` (`shader/artifact.rs:16-36`) already maps the language to the
artifact. It gains the native stage, which the lowering reports: MSL returns the extent offset and the threadgroup
arguments, and GLSL and HLSL return the output set. `PreparedBeslShader::compile` (`besl.rs:448-502`) stores it.
`cache_key` already hashes the stage and the lowered source (`besl.rs:428-441`), so baked shaders refresh. Every
stored `Shader` changes shape, so rebake with `--force` after this lands.

**Who reads it.**

1. **`prepare_shader`** (`pipeline_compilation.rs:671-701`) carries `native` and `tile` into `PreparedShader` and
   converts them to a GHI-side `ghi::TileLowering`. GHI doesn't depend on resource-management.
2. **GHI pipeline creation** accepts only the lowering its backend runs. Metal takes `MetalTileFunction`, and Vulkan
   and DX12 take `ComputeTile`. Anything else fails with the shader's name, the cause (baked for another platform or
   by an older generator), and the fix (rebake with `--force`).
3. **Target usage.** Image usage is fixed when the render pass builder creates the image, before the shader loads,
   so the target can't take its usage from the shader. Instead, the render pass declares outputs with a new
   `ghi::Uses::TileOutput`. Each backend maps that flag to the usage its own lowering needs: `RenderTarget` on Metal
   and `STORAGE` on Vulkan and DX12. At dispatch, the GHI checks every output image against `native`. An image
   without `TileOutput`, or an output that would need `ShaderWrite`, fails with the target's name and the flag to
   add. This check is what guarantees compressed targets on Metal.
4. **Recording.** `start_tile_pass` uses `tile.size`, `threadgroup`, and the output locations to configure the render
   pass. `dispatch_tiles` writes the extent at `extent_offset`. The fallback binds outputs into `output_set`.

## 6. GHI and renderer changes

### GHI types

- `ShaderTypes::Tile` (`ghi/src/types.rs:574`).
- `Stages::TILE` (`types.rs:102-132`), mapped to `MTLStages::Tile` (`metal/synchronization.rs:294-311`) and to
  `COMPUTE` on Vulkan and DX12.
- `Uses::TileOutput`, mapped in `texture_usage_from_uses` (`metal/mod.rs:128`).

### `ghi::pipelines::tile::Builder`

`Builder::new(push_constant_ranges, shader, outputs: &[Formats], lowering)`, created through
`Device::create_tile_pipeline` (`ghi/src/device.rs:32-44`).

- **Metal** builds an `MTL4TileRenderPipelineDescriptor` with the tile function, the color formats in location order,
  and `threadgroupSizeMatchesTileSize` when the workgroup equals the tile. It compiles through the same
  `MTL4Compiler` entry as raster (`metal/pipeline.rs:739-833`) and becomes `PipelineState::Tile`
  (`pipeline.rs:421-435`). `create_shader` must also read the threadgroup size for `Tile` (`pipeline.rs:702-707`).
- **Vulkan and DX12** build a compute pipeline plus the hidden output set layout.

### Recording

The API mirrors `start_render_pass` (`ghi/src/command_buffer.rs:102-108`):

```rust
recording.start_tile_pass(extent, &outputs, &pipeline)   // -> &mut impl TilePassMode
	.bind_tile_pipeline(&pipeline)                        // -> BoundTilePipelineMode
	.bind_descriptor_sets(..).write_push_constant(..)
	.dispatch_tiles();
recording.end_tile_pass();
```

**Metal**

- `start_tile_pass` creates an `MTL4RenderPassDescriptor` with the outputs as color attachments.
  - Load: `DontCare` unless `loads`. Store: `Store`.
  - `setTileWidth` and `setTileHeight` from `tile.size`, and `setThreadgroupMemoryLength`.
  - A tile function can't change the pass's tile size, so each pass takes the pipeline it runs.
- `dispatch_tiles` calls `dispatchThreadsPerTile(workgroup)`.
- **Argument tables:** add a `Tile` entry to `ArgumentTableStage` and `CommandArgumentTables`
  (`metal/command_buffer.rs:257-296`), bind sets to it in `apply_argument_buffers` (`encoding.rs:110-171`), and flush
  push constants to it (`recording.rs:871-910`).
- **Hazard tracking:** attachment uses are recorded at `MTLStages::Fragment` today (`operations.rs:447-472`). Phase 0,
  question 6, decides which stage covers a tile pass's attachment stores.

**Vulkan and DX12**

- `start_tile_pass` only remembers the outputs. `dispatch_tiles` writes the output set and dispatches
  `ceil(extent / tile)` workgroups. Unverified locally.

### Renderer

- **Pipeline asset:** `PipelineKind::Tile { shader, push_constants }` (`resources/pipeline.rs:11-34`) and its handler.
  Output formats aren't in the file, because `main`'s format depends on the pass's position: a terminal pass writes
  the `BGRAsRGB` swapchain (`render_pass.rs:505-514`). Passes request tile pipelines with their formats, and the
  pipeline manager keys them by `(id, formats)`.
- **Pipeline manager:** `compile_tile` next to `compile_compute` (`pipeline_compilation.rs:308-370`), `intern_tile_pipeline` in
  `publish` (`:968-1010`), and `Tile` in `shader_type_to_ghi` (`:624-634`).
- **Pass helper:** `render_pass/simple_tile.rs`, mirroring `simple_compute.rs`. It provides `Pipeline::compile` with
  output targets, `bind` by set name, and `ReadyPass::record` = start → bind → dispatch → end. Outputs bind by their
  BESL `tile_output` name, the way descriptors do.
- **Targets:** passes create outputs with `Uses::TileOutput | Uses::Image` instead of `Storage | Image`.

## 7. Phases

### Phase 0: prove it on the M4

No engine changes. A scratchpad Swift harness runs hand-written MSL, following the memory note
`byte-engine-shadow-pyramid-bound`. It runs the bloom 13-tap downsample from 1920x1080 through five levels in
`RG11B10Float`, plus one consumer pass that reads every level. It compares four variants:

- **a.** compute writing `ShaderWrite` textures (today)
- **b.** a tile function writing `RenderTarget`-only attachments
- **c.** a full-screen-triangle fragment pass (control)
- **d.** variant b with `allowGPUOptimizedContents = false`, which isolates compression from encoder effects

The harness also answers:

1. Does the M4 compress a texture that compute writes?
2. Does a render pass with only a tile dispatch, `DontCare` load, and no draws run every tile, including partial
   edge tiles?
3. What is the SIMD lane-to-pixel order in a tile function? Do masks 1, 2, and 3 reach the horizontal, vertical, and
   diagonal neighbors?
4. Can a tile function declare threadgroup arrays in its body, or must it take `[[threadgroup(n)]]` arguments plus
   pass and encoder lengths?
5. Which implicit imageblock member types work for `RG11B10Float`, `RGBA16Float`, `R8Unorm`, and `BGRA8Unorm_sRGB`?
   How do you write one member of a multi-output imageblock?
6. Which `MTLStages` must a Metal 4 barrier name so that a tile pass's attachment stores precede a later sampled
   read?
7. Does `threads_per_grid` give the exact render extent?
8. Which tile sizes and threads-per-tile limits does the M4 accept?

**Exit:** variant b beats variant a by enough to matter on the chain plus its consumer, and the answers to questions
2–6 fit the design. Otherwise, stop, or keep only the encoder and fusion benefits and say so. Use one short A/B (memory
note `keep-benchmarking-short`).

### Phase 1: the stage in BESL and MSL

- `Stages::Tile { tile, local_size }` (`shader/generator.rs:78`), wired into every exhaustive match the BESL survey
  found:
  - `stage_rules.rs:183-193`
  - `msl/emit.rs:528-536`
  - `msl/generate.rs:164-168,194-230`
  - `glsl/generator.rs:67,120,139,696`
  - `hlsl/generate.rs:503-513`
  - `hlsl_shader_compiler.rs:185-197` (`cs_6_x`)
  - `platform.rs:101-106`, where a GLSL extent must be reported for `Tile` or `besl.rs:483` fails on Linux
- Bead parsing (`besl.rs:196-237`) and the schema (`assets/schemas/besl.bead.schema.json`).
- `tile_output` and `TileOutput`, the `write` overload, and the new `StageInput` fields (`besl/src/lexer/entry.rs:421-434`).
- The stage rules from §3, the MSL tile lowering, and the VM tiles.

**Exit:** a tile shader runs in the VM, Metal compiles the generated MSL, and every rule has a rejection test.

### Phase 2: provenance and the other backends

- `NativeStage`, `TileInterface`, and `ShaderTypes::Tile` in resource-management. Fill them in the bake.
- The GLSL and HLSL compute lowering.

**Exit:** bake tests assert `native` for each platform's lowering.

### Phase 3: GHI tile passes

- Metal first, then the Vulkan and DX12 fallback, flagged unverified.

**Exit:**

- A GHI rendering test writes an image created with `TileOutput` (no `Storage`) through a tile pass and reads back
  the expected pixels.
- The GHI rejects a `Storage`-less image under a compute-lowered tile shader.

### Phase 4: bloom

Start after your current bloom changes are committed. Convert these together:

- Extract (tile 16, workgroup 8), downsample and upsample (tile 16, workgroup 16), upsample composite (tile 32,
  workgroup 16), and composite.
- **The blit bypass**, or the bloom output keeps `Storage`.
- **Lens flare**, because it shares `bloom/extract`, `downsample`, and `composite` (`lens_flare.rs:151-163`).

Port the existing bloom VM tests to tile execution.

**Measure:** run an A/B against a HEAD worktree, reading `pass.bloom` p50 from the inspector's `/metrics` with forced
frames (memory notes `byte-engine-post-pass-measurement` and `byte-engine-frame-pacing-measurement`). Judge short
passes by medians and the scene total. Attribute the gain to compression with a temporary
`allowGPUOptimizedContents = false` on the outputs.

**Exit:** bloom and lens flare captures match the compute baseline within tolerance, and the measured win holds.

### Phase 5: other passes

1. Image transforms (ACES, AgX, sRGB)
2. LUT and color grading
3. Motion blur, keeping its debug-view storage writes
4. SMAA
5. Height fog

The screen-space stages in the visibility pipeline (SSGI, GTAO, contact shadows) come later, one at a time, each
with its own A/B.

### Phase 6: follow-ups

- **Output reads (`load`)**: they set `loads` and use `LoadAction::Load`. This lets height fog write `main` in place,
  which is what the height fog item in `todo.md` asks for, without needing read-write storage support for
  `RGBu11u11u10`.
- **Several dispatches per tile pass:** chained same-size passes keep intermediates in tile memory, for example
  composite → tone map.
- **Swapchain:** `framebufferOnly = true` once nothing writes the swapchain through compute. Inspector capture of the
  final output must still work.
- **Scene pass:** append tile dispatches to the scene's own raster pass.

## 8. Tests

The tests exercise contiguous seams, per `AGENTS.md`.

| Test | Guards |
| --- | --- |
| `generated_tile_stage_compiles_with_metal` | The tile function signature, imageblock outputs, hidden extent, and helper parameters; Metal compiles it. |
| `tile_outputs_lower_to_imageblock_color_attachments` | Location order and the masked read-modify-write. |
| `tile_stage_rules_reject_*` (one per rule) and `lowering_reports_the_stage_rule_a_shader_breaks` extended | §3. |
| `tile_shader_settings_require_square_tiles_and_dividing_workgroups` | Bead parsing. |
| VM: `tile_writes_outside_the_invocation_tile_fail`, `tile_quad_shuffles_reach_quad_neighbors` | VM semantics. |
| `besl` handler: `tile_shaders_record_their_native_stage` | The provenance per platform. |
| GHI rendering: `tile_pass_writes_render_target_only_image` | Usage, recording, and hazards on Metal. |
| `generated_tile_shaders_match_the_vm` (GPU, macOS, `gpu-processing`) | The baked tile function writes what the VM writes, including edge tiles and quad shuffles. |
| Bloom and lens flare VM tests ported | The converted shaders. |

## 9. Documentation

Sections follow the table in `AGENTS.md`. Run `bun run check:style` and `bun run build` in `docs-site`.

- **Reference** (`docs/reference/besl/interface.mdx`):
  - A "Tile entry points" section.
  - `Tile` rows in "Stage input values" and "Standalone stage metadata".
  - A `Tile` row in "Stage and backend support": GLSL and HLSL as a compute shader, MSL as a tile function.
- **Reference** (`docs/reference/besl/intrinsics.mdx`): a "Tile stage" section covering `write` to outputs and the
  quad shuffle rule.
- **Contribute:**
  - A "Generate tile shaders" section in `docs/develop/besl/compilation.mdx`, with a BESL → MSL table, rules, and a
    tests table, like the ray tracing section.
  - A new design note, `docs/develop/rendering/tile-passes.mdx`: how Metal runs a tile pass, the rules for outputs and
    usage, the native-stage contract, other backends, and the tests that guard it.
- **Use:** writing a screen pass as a tile shader.
- **`CHANGELOG.md`.**

## 10. Risks and open questions

**Risks**

- **The M4 may already compress compute-written targets.** Then the win shrinks to encoder switches and fusion.
  Phase 0 decides this before any engine work.
- **Tile functions trade occupancy for locality.** 16–64 threads per tile were hard to schedule in the Ludicon tests,
  and a tile fusion prototype here once cost 0.036 ms more than raster. Keep workgroups at 256 threads where the
  shader allows.
- **The read-modify-write of multi-output imageblocks** may cost more than a direct member write (Phase 0,
  question 5).

**Decisions to confirm**

1. Stage name: `Tile` (recommended, since its rules are tile rules) or `Screen`.
2. `write` coordinates: global pixels (recommended, since compute code ports unchanged) or in-tile coordinates.
3. Storage-image writes to non-output images inside tile shaders: allowed (recommended, since motion blur needs them
   for debug views) or rejected.
