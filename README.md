# Byte-Engine

📚 **Docs:** <https://byte-engine.0x44491229.dev/docs>

Use Byte-Engine to build Rust applications that need graphics,
input, audio, physics, resources, networking, and retained UI in one runtime.

Start with the public `byte-engine` facade for most applications. Use the
lower-level crates when you need direct access to rendering, audio, resources,
shader processing, transport, math, or utilities. Each crate provides a small,
composable engine layer.

> [!WARNING]
> **Status:** Byte-Engine is early, so expect breaking API changes. The workspace
> is preparing its first public package flow. For now, use a source checkout for
> the most reliable setup.

## ✨ What you can use

- 🖼️ Platform graphics paths for Vulkan/Linux, Metal/macOS, and Direct3D 12/Windows.
- 🔊 Platform audio interfaces for Linux, macOS, and Windows.
- 🎮 Action-based input that lets your application respond to intent such as
  `Move`, `Fire`, or `Confirm` instead of raw device buttons.
- 📨 Actor-style system boundaries built around compact messages, factories,
  handles, and listeners.
- 📦 Asset pipeline that separates authored assets from runtime-ready resources.
- 🛠️ `beld`, a workspace CLI for baking, listing, querying, inspecting, and deleting
  resources.
- 🧪 BESL, the Byte Engine Shader Language, for shader parsing, reflection, material
  integration, and backend shader generation.
- 🧩 Early retained, async-friendly UI primitives that run in the engine render loop.

## ✅ Check the requirements

Byte-Engine currently targets nightly Rust and uses unstable language features.
The repository pins the toolchain in [`rust-toolchain.toml`](rust-toolchain.toml):

```text
nightly-2026-05-31
```

Install Rust with `rustup`. When you run Cargo inside the checkout, it uses the
pinned nightly automatically.

Platform requirements:

| Platform | Required setup |
| --- | --- |
| Linux | Vulkan development packages, Wayland/X11 packages, ALSA, CMake |
| macOS | Xcode command line tools or full Xcode, plus CMake and `pkg-config` |
| Windows | Visual Studio Build Tools with the MSVC C++ toolchain and Windows SDK |

On Linux, run `bash install_dependencies.sh` from the repository root. This
command installs the dependencies used by CI and development containers.

Use a GPU that supports the active backend, AVX2 on x64 platforms, and at least
4 GB of RAM. For complete setup information, see:

- [Requirements](https://byte-engine.0x44491229.dev/docs/requirements)
- [Environment setup](https://byte-engine.0x44491229.dev/docs/use/setup/environment)
- [macOS setup](https://byte-engine.0x44491229.dev/docs/use/setup/environment/macos)
- [Linux setup](https://byte-engine.0x44491229.dev/docs/use/setup/environment/linux)
- [Windows setup](https://byte-engine.0x44491229.dev/docs/use/setup/environment/windows)

## 🚀 Start from source

```sh
git clone https://github.com/Game-Tek/Byte-Engine.git
cd Byte-Engine
cargo check -p byte-engine
```

Run a small smoke test:

```sh
cargo run -p byte-engine --example window
```

Run the default graphics setup example:

```sh
cargo run -p byte-engine --example triangle
```

Most smoke examples set `kill-after=60` and exit automatically after about one minute.

## 🧱 Create a minimal headed application

```rust
use byte_engine::application::{Application, Parameter};
use byte_engine::application::graphics::{default_setup, GraphicsApplication};

fn main() {
    let mut app = GraphicsApplication::new(
        "my-byte-app",
        &[
            // Useful on devices that do not support mesh shading yet.
            Parameter::new("render.ghi.features.mesh-shading", "false"),
        ],
    );

    default_setup(&mut app);
    app.do_loop();
}
```

Until the package workflow is ready, add the engine from a checkout or Git dependency:

```toml
[dependencies]
byte-engine = { git = "https://github.com/Game-Tek/Byte-Engine", package = "byte-engine" }
```

For faster local development, use a path dependency:

```toml
[dependencies]
byte-engine = { path = "../Byte-Engine/crates/byte-engine" }
```

## 🗺️ Explore the workspace

| Path | Purpose |
| --- | --- |
| `crates/byte-engine` | Main public engine crate and application-facing facade. |
| `crates/ghi` | Graphics hardware interface used by the renderer. |
| `crates/ahi` | Audio hardware interface used by engine audio systems. |
| `crates/resource-management` | Asset handling, resource storage, shader resources, and runtime reads. |
| `crates/besl` | Byte Engine Shader Language parser, lexer, semantic graph, and integration. |
| `crates/besl-derive` | Procedural macros for BESL-related structures. |
| `crates/betp` | Byte Engine transport protocol primitives for local and remote sessions. |
| `crates/math` | Shared math aliases and helpers. |
| `crates/utils` | Shared allocation, async, sync, collection, and geometry utilities. |
| `crates/beld` | Workspace asset/resource CLI. It is intentionally `publish = false`. |
| `docs` | Documentation source for setup, usage, reference, and engine design notes. |
| `docs-site` | Documentation site project. |

Where required, crates use the `byte-engine-*` namespace so crates.io can resolve
each internal layer independently. In Rust code, use the shorter import names
such as `ghi`, `ahi`, `besl`, `math`, and `utils`.

## 🎯 Run the examples

Examples live in [`crates/byte-engine/examples`](crates/byte-engine/examples).
Run them from the workspace root:

```sh
cargo run -p byte-engine --example <name>
```

Choose an example based on the system you want to exercise:

| Example | What it exercises |
| --- | --- |
| `none` | Creates and runs a minimal graphics application without user setup. |
| `window` | Creates a window through the default window setup. |
| `triangle` | Runs the default headed graphics setup. |
| `cube` | Placeholder smoke path for a 3D cube scene. |
| `sandbox` | Placeholder smoke path for physics sandbox work. |
| `sound` | Audio synthesizer smoke path. |
| `replication` | Early networking/replication smoke path. |

## 📦 Work with assets and resources

Byte-Engine separates the files you author from runtime-ready resources:

- **Assets** are source files such as PNG, JPEG, glTF/GLB, FBX, WAV, OGG, LUT files,
  `.bema` material declarations, and BESL shader sources.
- **Resources** are processed engine data with metadata such as format, hash,
  size, image extent, mesh layout, shader reflection data, and binary payloads.
- When debug or development loading is enabled, the resource manager can read an
  existing resource or ask an asset handler to bake it.

Use `beld` from the workspace to inspect and manage resources:

```sh
cargo run -p beld -- --source assets --destination resources bake texture.png scene.glb character.fbx
cargo run -p beld -- --destination resources list
cargo run -p beld -- --destination resources inspect texture.png
cargo run -p beld -- --destination resources query Material group=opaque --format json
```

For design information, see
[Asset and resource management](https://byte-engine.0x44491229.dev/docs/develop/design/resource-management).

## 🧪 Write shaders with BESL

Use the Byte Engine Shader Language (BESL) to integrate shader code with material
processing, resource reflection, render-model code generation, and platform
shader generation.

The syntax is Rust-inspired, but BESL is not Rust:

```rust
VertexInput: struct {
    position: vec3f,
    normal: vec3f,
    uv: vec2f,
}

albedo: binding CombinedImageSampler set=0 binding=0 read
uv: input vec2f location=0
color: output vec4f location=0

main: fn () -> void {
    let texel: vec4f = sample(albedo, uv);
    color = texel;
}
```

Backend coverage is still evolving. The project includes GLSL, MSL, HLSL, and
platform-generator paths. For language details, see the
[BESL reference](https://byte-engine.0x44491229.dev/docs/reference/besl).

## 🏗️ Understand the architecture

Before you make a large change, read the design documentation for the affected system:

- [Actor pattern](https://byte-engine.0x44491229.dev/docs/develop/design/actor-pattern): message-passing system
  boundaries, factories, handles, and listeners.
- [Input handling](https://byte-engine.0x44491229.dev/docs/develop/design/input-handling): device triggers,
  seats, actions, value mappings, and tick policies.
- [Rendering](https://byte-engine.0x44491229.dev/docs/develop/design/rendering): render orchestrators,
  render systems, render domains, and render models.
- [UI module](https://byte-engine.0x44491229.dev/docs/develop/design/ui): retained async component primitives
  and UI render flow.
- [Resource management](https://byte-engine.0x44491229.dev/docs/develop/design/resource-management): asset
  baking, resource storage, runtime reads, and `beld`.

## 🧰 Run development checks

### Contributing documentation

Use [ISO 24495-1 plain-language principles](https://www.iso.org/standard/78907.html)
for Rust comments and API documentation. Write for the intended developer, state
the purpose early, use direct language, and organize information so developers
can find and apply it quickly. Keep useful intra-code links to related types,
traits, functions, methods, modules, and concepts, even when a strict
plain-language rewrite would remove them. These links are part of the API's
navigation.

Use the [Microsoft Writing Style Guide](https://learn.microsoft.com/style-guide/)
for user guides and pages in [`docs`](docs). Address the reader as **you**, lead
with the reader's goal, prefer everyday words, keep sentences concise, and make
the next action clear. Preserve technical terms when they are the names that
developers see in the engine or its tools.

Common checks from the workspace root:

```sh
cargo fmt --check
cargo check --workspace
cargo test --workspace
cargo clippy --workspace
cargo doc -p byte-engine --no-deps
```

Publishing-specific verification and crate order are documented in
[`PUBLISHING.md`](PUBLISHING.md).

## 🔗 Documentation and links

- API documentation: <https://docs.rs/byte-engine>
- Repository: <https://github.com/Game-Tek/Byte-Engine>
- Changelog: [`CHANGELOG.md`](CHANGELOG.md)
- Publishing notes: [`PUBLISHING.md`](PUBLISHING.md)
- User documentation: <https://byte-engine.0x44491229.dev/docs>
- Documentation source: [`docs`](docs)

## 📄 License

Byte-Engine is licensed under the [MIT license](LICENSE).
