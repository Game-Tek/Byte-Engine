# Byte-Engine

📚 **Docs:** <https://byte-engine.0x44491229.dev/docs>

Byte-Engine is a composable Rust game engine for applications that need graphics,
input, audio, physics, resources, networking, and retained UI in one runtime.

It is designed as a set of small engine layers rather than one opaque framework:
use the public `byte-engine` facade for normal applications, or work with the
lower-level crates when you need direct access to rendering, audio, resources,
shader processing, transport, math, or utilities.

> [!WARNING]
> **Status:** Byte-Engine is early and API-breaking changes are expected. The
> workspace is preparing its first public package flow, so source checkout usage
> is currently the most reliable path.

## ✨ Highlights

- 🖼️ Platform graphics paths for Vulkan/Linux, Metal/macOS, and Direct3D 12/Windows.
- 🔊 Platform audio interfaces for Linux, macOS, and Windows.
- 🎮 Action-based input so application code listens for intent like `Move`, `Fire`,
  or `Confirm` instead of raw device buttons everywhere.
- 📨 Actor-style system boundaries built around compact messages, factories,
  handles, and listeners.
- 📦 Asset pipeline that separates authored assets from runtime-ready resources.
- 🛠️ `beld`, a workspace CLI for baking, listing, querying, inspecting, and deleting
  resources.
- 🧪 BESL, the Byte Engine Shader Language, for shader parsing, reflection, material
  integration, and backend shader generation.
- 🧩 Early retained, async-friendly UI primitives that run inside the engine render
  loop.

## ✅ Requirements

Byte-Engine currently targets nightly Rust and uses unstable language features.
The repository pins the toolchain in [`rust-toolchain.toml`](rust-toolchain.toml):

```text
nightly-2026-05-31
```

Install Rust with `rustup`; Cargo will use the pinned nightly automatically when
run inside the checkout.

Platform requirements:

| Platform | Required setup |
| --- | --- |
| Linux | Vulkan development packages, Wayland/X11 packages, ALSA, CMake |
| macOS | Xcode command line tools or full Xcode, plus CMake and `pkg-config` |
| Windows | Visual Studio Build Tools with the MSVC C++ toolchain and Windows SDK |

On Linux, run `bash install_dependencies.sh` from the repository root to install the same dependency set used by CI and devcontainers.

Hardware expectations include a GPU suitable for the active backend, AVX2 on x64
platforms, and at least 4 GB of RAM. See the full setup docs for details:

- [Requirements](https://byte-engine.0x44491229.dev/docs/requirements)
- [Environment setup](https://byte-engine.0x44491229.dev/docs/use/setup/environment)
- [macOS setup](https://byte-engine.0x44491229.dev/docs/use/setup/environment/macos)
- [Linux setup](https://byte-engine.0x44491229.dev/docs/use/setup/environment/linux)
- [Windows setup](https://byte-engine.0x44491229.dev/docs/use/setup/environment/windows)

## 🚀 Quick start from source

```sh
git clone https://github.com/Game-Tek/Byte-Engine.git
cd Byte-Engine
cargo check -p byte-engine
```

Run a small smoke example:

```sh
cargo run -p byte-engine --example window
```

Run the default graphics setup example:

```sh
cargo run -p byte-engine --example triangle
```

Most smoke examples set `kill-after=60`, so they exit automatically after about a
minute.

## 🧱 Minimal headed application

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

While the package workflow is being finalized, consume the engine from a checkout
or Git dependency:

```toml
[dependencies]
byte-engine = { git = "https://github.com/Game-Tek/Byte-Engine", package = "byte-engine" }
```

For local development, a path dependency is usually faster:

```toml
[dependencies]
byte-engine = { path = "../Byte-Engine/crates/byte-engine" }
```

## 🗺️ Workspace map

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

The crates are published under the `byte-engine-*` namespace where needed so
crates.io can resolve internal layers independently. Rust import names stay short
inside the codebase, such as `ghi`, `ahi`, `besl`, `math`, and `utils`.

## 🎯 Examples

Examples live in [`crates/byte-engine/examples`](crates/byte-engine/examples).
Run them from the workspace root:

```sh
cargo run -p byte-engine --example <name>
```

Useful starting points:

| Example | What it exercises |
| --- | --- |
| `none` | Creates and runs a minimal graphics application without user setup. |
| `window` | Creates a window through the default window setup. |
| `triangle` | Runs the default headed graphics setup. |
| `cube` | Placeholder smoke path for a 3D cube scene. |
| `sandbox` | Placeholder smoke path for physics sandbox work. |
| `sound` | Audio synthesizer smoke path. |
| `replication` | Early networking/replication smoke path. |

## 📦 Asset and resource workflow

Byte-Engine separates authored files from runtime-ready resources:

- **Assets** are source files such as PNG, JPEG, glTF/GLB, FBX, WAV, OGG, LUT files,
  `.bema` material declarations, and BESL shader sources.
- **Resources** are processed engine data with metadata such as format, hash,
  size, image extent, mesh layout, shader reflection data, and binary payloads.
- The resource manager can read an existing resource from storage or ask an asset
  handler to bake it when debug/development loading is enabled.

Use `beld` from the workspace to inspect and manage resources:

```sh
cargo run -p beld -- --source assets --destination resources bake texture.png scene.glb character.fbx
cargo run -p beld -- --destination resources list
cargo run -p beld -- --destination resources inspect texture.png
cargo run -p beld -- --destination resources query Material group=opaque --format json
```

See [Asset and resource management](https://byte-engine.0x44491229.dev/docs/develop/design/resource-management)
for the design notes.

## 🧪 BESL: Byte Engine Shader Language

BESL is Byte-Engine's shader language. It exists so shader code can participate
in material processing, resource reflection, render-model code generation, and
platform shader generation.

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

Backend coverage is still evolving, but the project already contains GLSL, MSL,
HLSL, and platform-generator paths. See the [BESL reference](https://byte-engine.0x44491229.dev/docs/reference/besl)
for more details.

## 🏗️ Architecture notes worth reading

The repository contains design docs that explain the engine direction and are
useful before making larger changes:

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

## 🧰 Development commands

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
