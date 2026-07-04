# Byte-Engine

Byte-Engine is a composable Rust game engine for applications that need graphics,
input, audio, physics, resources, and retained UI in one runtime.

The main crate is `byte-engine`. The workspace also publishes support crates
under the `byte-engine-*` namespace so crates.io can resolve the engine's
internal layers independently.

## Status

Byte-Engine currently targets nightly Rust. Building the engine requires a
nightly toolchain from 2026-06-01 or more recent. The workspace uses unstable
language features and declares its supported Rust version in the workspace
manifest.

## Documentation

- API documentation: <https://docs.rs/byte-engine>
- Repository: <https://github.com/Game-Tek/Byte-Engine>
- Release notes: [`CHANGELOG.md`](CHANGELOG.md)
- Publish order: [`PUBLISHING.md`](PUBLISHING.md)

## Crates

- `byte-engine`
- `byte-engine-utils`
- `byte-engine-math`
- `byte-engine-ahi`
- `byte-engine-besl-derive`
- `byte-engine-besl`
- `byte-engine-resource-management`
- `byte-engine-ghi`
- `byte-engine-betp`

## License

Byte-Engine is licensed under the MIT license.
