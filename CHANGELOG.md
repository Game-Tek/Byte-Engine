# Changelog

## 0.2.0 - 2026-08-20

- Added skeletal animation sampling, blending, inertialization, root motion, animation graphs, and reusable runtime players.
- Added block-based audio graph processing with custom processors, pitch shifting, random and round-robin selection, and sample-accurate playback time.
- Added navigation-mesh pathfinding and funnel-based corridor simplification.
- Expanded GHI allocation and I/O APIs while advancing the Direct3D 12, Metal, and Vulkan backends.
- Made resource loading asynchronous and improved baking with dependency tracking, request coalescing, streaming writes, and queryable storage.
- Added and optimized rendering paths for GTAO, shadows, environment maps, tone mapping, SMAA, visibility, and material evaluation.
- Reorganized public modules and resource APIs. This release contains breaking API changes from `0.1.x`.

## 0.1.1 - 2026-07-23

- Configured docs.rs builds to enable the AES and SSE2 target features required by `gxhash`.

## 0.1.0 - 2026-07-23

- Published `byte-engine`, `beld`, and the engine's internal support crates.
- Scoped internal package names under `byte-engine-*` while preserving existing Rust crate import names.
- Tightened the documented `byte-engine` API surface by hiding renderer and layout implementation modules.
- Fixed public rustdoc links and added a crate-level usage example.
- Added public facade re-exports across UI, rendering, gameplay, physics, audio, and networking modules.
- Verified `byte-engine` with strict missing-docs rustdoc linting.
- Stabilized BEMA material asset tests that previously shared shader compiler state under parallel test execution.
