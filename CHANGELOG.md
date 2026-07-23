# Changelog

## 0.1.0 - 2026-07-23

- Published `byte-engine`, `beld`, and the engine's internal support crates.
- Scoped internal package names under `byte-engine-*` while preserving existing Rust crate import names.
- Tightened the documented `byte-engine` API surface by hiding renderer and layout implementation modules.
- Fixed public rustdoc links and added a crate-level usage example.
- Added public facade re-exports across UI, rendering, gameplay, physics, audio, and networking modules.
- Verified `byte-engine` with strict missing-docs rustdoc linting.
- Stabilized BEMA material asset tests that previously shared shader compiler state under parallel test execution.
