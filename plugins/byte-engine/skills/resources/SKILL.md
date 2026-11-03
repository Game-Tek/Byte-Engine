---
name: resources
description: "Bake Byte Engine assets into resources with BELD and inspect the resource store efficiently: when a bake is needed at all, which BELD binary to run, dry runs, intercepting a bake's intermediate products such as the BESL and MSL each shader compiles from and the metallib it compiles to, scriptable queries for what fills the store, stale, failed, duplicate, uncompressed, or unreachable resources, reading one resource's model or payload data, and why a resource doesn't load. Use when the user asks to bake, rebake, or package assets, asks what an asset turns into, wants to see the shader source a material or shader generates or the compiled shader binary, or wants to find or debug resources in a Byte Engine project."
---

# Bake and inspect Byte Engine resources

BELD bakes the source assets in `assets/` into the resource store in `resources/` and reads that store back.
Every application builds its own BELD. This skill writes the command as `beld`; run it as the application's
binary (see [Run the application's BELD](#run-the-applications-beld)).

## Decide whether you need to bake

- **Debug builds: don't bake before a run.** A debug application installs an asset manager that bakes a missing
  or stale resource the first time the application requests it. Edit the asset and run the application.
- **Release builds: bake first.** A release application reads only the baked store and can't process assets.
  After changing a source asset, run `beld bake`; BELD rebakes only what changed.
- **Use `beld bake --force`** after updating the engine or BELD, or after changing a render feature or
  `--texture-compression`. Those changes don't change source file versions, so a plain bake skips them.
- **Bake on the target platform.** Shader payloads are native to the platform (SPIR-V, DXIL, or Metal), so a
  store baked on another platform isn't portable.

```bash
beld --source "$PWD/assets" --destination "$PWD/resources" bake
```

Pass absolute paths and run from the application directory. BELD reads `config.json` from the working directory;
pass `--config <path>` for another one.

## Run the application's BELD

An application adds its own BELD binary, which starts from the application's parameters, registers its asset
handlers, and builds from its `Cargo.lock`, so it bakes exactly what a debug run of the application bakes. Run it
from the application directory:

```bash
cargo run --release --bin beld -- --source "$PWD/assets" --destination "$PWD/resources" bake
```

If the application has no `beld` binary (`cargo run --bin beld` says there is no such target), tell the user and
point them to https://byte-engine.0x44491229.dev/docs/use/resources/add-beld.md instead of falling back to
another BELD. A `beld` on `PATH` was built from some other engine revision with the default handlers only: it
can fail to read the store, bake different resources, or have a different command line.

If you see `resource-management signature '…' does not match this engine's expected signature`, the store was
baked by another engine revision. Rebake with the application's BELD once the user agrees to replace the store.

Drop `--release` to investigate a store. A debug BELD records bake traces and which resources use which. A
release BELD doesn't, so it has no traces, `refs`, `used-by`, or `total`, and rejects `is:unreachable` and
`is:dangling`.

## Inspect the store efficiently

Never use `beld browse` from an agent; it needs an interactive terminal. `beld query` takes the same filter,
group, and sort and prints the result. Logs go to standard error, so `2>/dev/null` (or `RUST_LOG=warn`) leaves
clean output on standard output.

Work from coarse to fine:

1. Get the shape of the store without listing resources. `--limit 0` keeps the counts and sizes:

   ```bash
   beld query --group class --limit 0
   ```

2. Narrow it with a filter, and cap the list:

   ```bash
   beld query --limit 10 class=Image 'stored>1MiB'
   ```

3. Switch to JSON when you need fields to process:

   ```bash
   beld query --format json --limit 20 -- class=Image -is:compressed 2>/dev/null | jq '.resources[] | {id, stored}'
   ```

4. Read one resource in full:

   ```bash
   beld inspect material.bema --format json 2>/dev/null | jq '.resource'
   ```

5. Add `--payload typed` only when you need the data itself, and slice it with `jq`. Payloads can be large:

   ```bash
   beld inspect scene.glb --format json --payload typed 2>/dev/null \
     | jq '.streams[] | select(.name == "Vertex.Position") | .data[:3]'
   ```

Quote terms that contain `>`, `<`, `*`, or `?`. Put options first, then `--`, before a term that starts with
`-`. A term that doesn't parse exits with code 2 and prints nothing.

| To find | Run |
| --- | --- |
| Which source files produce the most data | `beld query --group source --limit 5` |
| Large textures stored uncompressed | `beld query -- class=Image 'stored>1MiB' -is:compressed` |
| Identical payloads stored twice | `beld query --group hash is:duplicate` |
| What the next bake processes again | `beld query is:stale` |
| Bakes that failed or warned (debug BELD) | `beld query --format json is:failed` and `beld query has:warnings` |
| Images in one format | `beld query class=Image model.format=BC7` |
| Recently baked resources | `beld query 'baked<1h'` |
| Any value in a resource's model | `beld query --group model.<path>`, such as `model.extent.width` |
| What one resource pulls in (debug BELD) | `beld query --format json <id>` and read `refs` and `total` |

## See what an asset bakes into, without touching the store

A dry run bakes the asset and everything it needs in memory, prints it, and writes nothing under the destination:

```bash
beld --source "$PWD/assets" bake --dry-run --format json character.glb 2>/dev/null \
  | jq '.resources[] | {id, class}, .failed'
```

Quote IDs with a fragment, such as `'character.fbx#animations/Walk'`, so the shell keeps the `#`. Animation clips
print their keyframes under `resource.tracks` without `--payload`.

## See a bake's intermediate products, without instrumenting the application

A bake builds intermediate products it normally throws away, such as the BESL program each shader compiles from, the
platform source that BESL lowers to, and the binary the platform compiler produces. Add `--intercept <pattern>` to a
dry run to print them, and `--intercept-out <dir>` to write each one to a file instead. Don't add logging,
debug prints, or file dumps to the engine or the application to get these; intercept them.

Each product is named `<resource ID>.<extension>`, after the resource it belongs to and its format:

| Product | Name | When you need it |
| --- | --- | --- |
| BESL a shader compiles from | `<shader ID>.besl` | What a material's graph or a standalone shader became after the renderer's program generator or transform; reading the program behind a shader compile error. |
| Platform source the BESL lowered to | `<shader ID>.msl` on macOS, `.hlsl` on Windows, `.glsl` on Linux | What the platform compiler received; a shader that compiles but renders or performs differently than its BESL suggests. |
| Compiled binary | `<shader ID>.metallib` on macOS, `.spv` on Linux; none on Windows, where the payload is the HLSL | The exact bytes the store keeps; register, spill, and occupancy statistics, `xcrun metal-source --extract`, and GPU tools that open a metallib. |

Shader IDs are `<material>.bema#shader` and `#hit-shader` for `.bema` materials (a variant uses its parent
material's shader), `<container>#shaders/<key>` for glTF and FBX materials, and `<system>.particles#shaders/<stage>`
for particles. A standalone shader's ID is its `.besl` asset, so its products are `<shader>.besl.besl`,
`<shader>.besl.msl`, and `<shader>.besl.metallib`. Baking a `.pipeline` bakes the standalone shaders it lists, such as
`byte_engine/rendering/visibility/shadow-mesh.besl.msl` for `cone-shadow.pipeline`.

```bash
# The BESL every shader of a material compiles from
beld --source "$PWD/assets" bake --dry-run --intercept '*.besl' materials/wood.bema 2>/dev/null

# The BESL beside the MSL it lowered to, for one glTF's generated materials
beld --source "$PWD/assets" bake --dry-run --intercept 'character.glb#*.besl' --intercept 'character.glb#*.msl' \
  character.glb 2>/dev/null

# Every product a bake builds, by name only
beld --source "$PWD/assets" bake --dry-run --intercept '*' --format json character.glb 2>/dev/null \
  | jq -r '.intercepted[].name'

# Every shader binary of a pass as files, named after their shaders, for pipeline statistics or other Metal tools
beld --source "$PWD/assets" bake --dry-run --intercept '*.metallib' --intercept-out shaders \
  byte_engine/rendering/visibility/gtao.pipeline 2>/dev/null
# prints shaders/byte_engine/rendering/visibility/gtao.besl.metallib

# One product's text, to save or diff
beld --source "$PWD/assets" bake --dry-run --intercept '*.msl' --format json materials/wood.bema 2>/dev/null \
  | jq -r '.intercepted[] | select(.name == "materials/wood.bema#shader.msl") | .content' > wood.msl
```

- Patterns match the whole name, ignoring case. `*` also matches `/` and `#`. Quote them so the shell doesn't
  expand them.
- The human format prints each product under a `// <name>` line and nothing else, so it pipes cleanly; a binary
  product prints only its size there. The JSON format is the usual dry-run document with `intercepted` added; each
  entry has `name`, `encoding` (`utf-8` or `base64`), and `content`.
- `--intercept-out <dir>` writes each product to `<dir>/<name>`, where each `/` in the name is a directory, and the
  human format prints one written path per line instead. Use it for binaries, and for many products at once instead
  of extracting them from JSON. Existing files are overwritten. Never `grep` the store's `*-raw` payload files to
  find a shader's metallib; intercept it by name.
- A dry run bakes into an empty store, so every product is built. A pattern that matches nothing logs a warning on
  standard error and still exits with code 0.
- BELD reads the project's `config.json`, so the programs match the render features the application uses. Pass
  `--config` or set `BE_RENDER__*` variables to see another configuration's shaders.
- The BESL is written back from the program the bake built. Declarations engine code adds with no BESL syntax print
  as comments (`// specialization …`, `// intrinsic …`), so the text reads as the program but may
  not parse back. The MSL leaves out the precompiled prelude the Metal compiler includes.
- To see one glTF or FBX material's shader, bake that material alone. A container's generated materials are named
  `<container>#materials/<material>.variant`. Unnamed glTF materials are `material_<index>`, and FBX materials are
  `<index>_<name>`. Shader products are named by a hash of their source (`#shaders/<hash>`), so the material is the
  handle. List a container's materials, then intercept one:

  ```bash
  beld --source "$PWD/assets" bake --dry-run --format json sponza/Sponza.gltf 2>/dev/null \
    | jq -r '.resources[] | select(.class == "Variant") | .id'
  beld --source "$PWD/assets" bake --dry-run --intercept '*.msl' 'sponza/Sponza.gltf#materials/material_3.variant' \
    2>/dev/null
  ```

  A material that a sidecar overrides isn't generated; bake the override's own ID. Without the `.variant` suffix,
  the ID names no resource and the bake fails.
- Bake with debug views off when you read shaders for performance. A debug BELD builds them into every shader. Pass
  `--config` with `{"render":{"debug-views":{"enabled":false}}}`, or set `BE_RENDER__DEBUG_VIEWS__ENABLED=false`.
- When `render.material-evaluation` is `uber`, materials compile no shader of their own. A dry run then bakes the
  assembled shaders the materials feed, which appear as
  `byte_engine/rendering/visibility/material-evaluation-<phase>.uber#shaders/<key>.besl` and `.msl`. Set
  `BE_RENDER__MATERIAL_EVALUATION=pipelines` to see one shader per material instead.

## Find what a bake failed on

`beld bake` keeps going after a failed asset and exits with code 1 at the end. Find the failures and read why:

```bash
beld query --format json is:failed 2>/dev/null | jq '.resources[] | {id, trace}'
beld inspect <failed id> --format json
```

A failed asset still has a trace, with `resource` set to `null`.

## Find resources nothing uses

This needs a debug BELD and `resources.roots` in `config.json`: glob patterns of the IDs the application loads by
name, where `*` also matches `/`. Without roots, `is:unreachable` is an error.

```bash
beld query is:root --limit 0                      # check the patterns match what you expect
beld query --group source --limit 0 is:unreachable # one line per source file to review
beld query is:dangling                            # resources that use an ID the store doesn't have
```

## Commands that change or remove resources

Ask the user before running `beld delete`, `beld clear`, `beld wipe`, or `beld bake --prune`. They remove
resources, and `clear` and `wipe` remove the whole store. To stop baking an asset, add it to `resources.exclude`
in `config.json`; an exclusion doesn't delete a resource that is already baked.

## When a resource doesn't load in a debug build

Check these in order:

1. The source asset path and extension, and that an asset handler for it is installed.
2. That the assets it depends on bake too: `beld bake --dry-run --format json <id>` shows the whole tree and
   every failure.
3. That the store wasn't cleared by a signature change. The store is wiped when the resource code changes.
4. That the requested resource type matches the class stored for that ID: `beld query <id>`.

## Exit codes

0 is success. 1 means a command or at least one asset failed, the store couldn't be opened, or its revision or
storage mode doesn't match. 2 means invalid arguments, including a filter that doesn't parse.

## Reference

Read these pages for the exact rules. Each is served as Markdown:

- Every BELD command, filter term, field, flag, and output shape: https://byte-engine.0x44491229.dev/docs/api/beld.md
- Using BELD: https://byte-engine.0x44491229.dev/docs/use/resources/beld.md
- Release bakes, roots, and excluded assets: https://byte-engine.0x44491229.dev/docs/use/resources/bake.md
- On-demand baking in debug builds: https://byte-engine.0x44491229.dev/docs/use/resources/debug-loading.md
- Giving an application its own BELD: https://byte-engine.0x44491229.dev/docs/use/resources/add-beld.md
- Index of every page: https://byte-engine.0x44491229.dev/llms.txt
