# Changelog

## Unreleased

- Reworked input around two classes: every source event is recorded into an `InputCollector`, and each consumer owns an `InputSink` that declares its actions, pulls them once per tick, and answers `Capture::Captured` to keep the source event from later sinks. `InputEvents`, `ActionProcessor`, and `InputManager` are replaced; `GraphicsApplication::input_system` is replaced by `GraphicsApplication::input`, and manual action queuing is removed in favor of publishing `ActionEvent` values directly.
- A value driven by a record is no longer repeated a second time in the same tick by `TickPolicy::WhileActive` or `TickPolicy::Always`.
- Retained control values live in a dense per-seat, device, and trigger table instead of a hash map, each sink keeps a sorted control-to-action index so a record only visits the actions bound to it, and binding gates are classified when resolved. `InputCollector::has_trigger` is replaced by `InputCollector::trigger`, which returns the handle for recording without a name lookup, and the public `TriggerMapping` alias is removed.
- `InputCollector::end_tick` is removed. Each sink reads from its own position in the queue, and a record is retained and dropped once every sink has pulled past it.
- `process_default_window_input` records into the collector itself and discards the seat on focus loss, minimize, close, and resize, returning whether it did.
- The UI `Engine` owns the pointer drag: `Engine::press`, `Engine::drag_to`, `Engine::release`, and `Engine::drag` replace the standalone `Drag` type, components read the held source through `Context::drag`, and `Engine::cancel` ends the interaction in progress. A changed viewport size cancels from `Engine::evaluate`.
- The UI engine delivers drag gestures to components as events: `Events::DragStarted` and `Events::DragEnded` reach the source, and `Events::Dropped` reaches the surface under the release point and its ancestors with the released source in `UiEvent::source`, before the source's `DragEnded`. `UiEvent` gains that `source` field.
- `EvaluationContext::reparent` moves a retained UI element under another parent as its last child, keeping its id and properties.
- Components spawned inside a mounted UI scope end when that scope is removed, and finished components release their runtime slot. Previously both stayed in the runtime, and components waiting on frames kept running after their elements were gone.

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
