# Changelog

## Unreleased

- Reworked input around two classes: every source event is recorded into an `InputCollector`, and each consumer owns an `InputSink` that declares its actions, pulls them once per tick, and answers `Capture::Captured` to keep the source event from later sinks. `InputEvents`, `ActionProcessor`, and `InputManager` are replaced; `GraphicsApplication::input_system` is replaced by `GraphicsApplication::input`, and manual action queuing is removed in favor of publishing `ActionEvent` values directly.
- A value driven by a record is no longer repeated a second time in the same tick by `TickPolicy::WhileActive` or `TickPolicy::Always`.
- Retained control values live in a dense per-seat, device, and trigger table instead of a hash map, each sink keeps a sorted control-to-action index so a record only visits the actions bound to it, and binding gates are classified when resolved. `InputCollector::has_trigger` is replaced by `InputCollector::trigger`, which returns the handle for recording without a name lookup, and the public `TriggerMapping` alias is removed.
- `InputCollector::end_tick` is removed. Each sink reads from its own position in the queue, and a record is retained and dropped once every sink has pulled past it.
- `process_default_window_input` records into the collector itself and discards the seat on focus loss, minimize, close, and resize, returning whether it did.
- The UI `Engine` owns the pointer drag: `Engine::press`, `Engine::drag_to`, `Engine::release`, and `Engine::drag` replace the standalone `Drag` type, components read the held source through `Context::drag`, and `Engine::cancel` ends the interaction in progress. A changed viewport size cancels from `Engine::evaluate`.
- The UI engine hit-tests pointer gestures itself: `Engine::press`, `Engine::drag_to`, and `Engine::release` take normalized window coordinates, `press` grabs the surface under the pointer from the last evaluated frame, and `Engine::hit` answers what is under a point. `HitTest` is no longer needed to drive a drag.
- The UI engine delivers drag gestures to components as events: the grabbed surface gets `Events::Grabbed` on press, `Events::Dragged` once per evaluation with the offset from the press point in `UiEvent::delta`, and `Events::DragEnded` when the grab ends with the surface it was dropped on in `UiEvent::source`. `Events::Dropped` reaches the surface under the release point and its ancestors with the released source in `UiEvent::source`, before the source's `DragEnded`. `UiEvent` gains that `source` field.
- `EvaluationContext::reparent` moves a retained UI element under another parent as its last child, keeping its id and properties, and `EvaluationContext::adopt` moves another element under this one.
- `Container::absolute_position` is now an offset from the parent's top-left corner rather than the viewport's, so nested widgets keep their internal placement wherever their parent's flow puts them. Containers with `Depth::absolute` still start from the viewport.
- Components spawned inside a mounted UI scope end when that scope is removed, and finished components release their runtime slot. Previously both stayed in the runtime, and components waiting on frames kept running after their elements were gone.
- `Context::element` takes any name that converts into `Cow<'static, str>`, so elements created from runtime data can be named with an owned `String`; the slot is keyed by the string's content.
- `EvaluationContext::remove` removes one UI element with everything declared under it, including the components started there, and reports whether anything was removed. Declaring the same name again on a later frame reuses its id.
- `EvaluationContext::update_curve` edits a retained curve in place. `Curve::path_mut`, `Curve::set_path`, and the new `CurvePath::clear`, `push`, `push_line`, `push_cubic`, and `set_size` methods re-route a path while reusing its segment buffer, and changing only the segments repaints without replaying layout.
- A container's `Transform` scale now reaches the curves and text under it: segment points, stroke widths, and font sizes scale with the inherited transform the way rectangles, hit bounds, and clipping already did.
- The UI engine reports the pointer passing over surfaces: `Events::PointerEntered` and `Events::PointerExited` reach the surface under the pointer and the ancestors that started or stopped containing it, a held drag source is skipped, `DragCapture::over` names the surface beneath a dragged item, and `Engine::hovered` answers at the application level.
- `Curve::hit_testable(width)` makes a curve a pointer target within `width` layout units of its stroke, so wires can be hovered, clicked, and dropped on. `HitTest` retains the same polylines.
- Absolutely positioned containers keep negative layout coordinates instead of being snapped to the viewport's edge; visual transforms preserve them as well.
- `Context::remove` is available to components, so a component can remove its own elements and end itself.
- GHI windows are pumped by one `ghi::window::App` per process instead of per window, matching how Win32, AppKit, and Wayland deliver events. Create windows with `App::create_window` and drain `App::poll`, which yields `ghi::window::Event`: either an application event such as `AppEvents::Quit`, or a window `Events` value tagged with the `WindowId` it targets. Input targets the window with focus. `Window::new`, `Window::poll`, and `Renderer::update_windows` are removed; `Renderer::poll_windows` replaces the latter, and the graphics application publishes `ghi::window::Event` instead of `ghi::window::Events`.
- Multiple GHI windows no longer steal each other's events on macOS, Win32 reports `WM_QUIT` and keeps handling messages sent between polls, and every Wayland window shares one compositor connection. The macOS dock's Quit item now asks the application to close instead of ending the process.
- The `render-on-demand` application parameter renders a frame only when something changed. Window changes, new UI renders, and screenshot requests ask for frames; call `Renderer::request_redraw` after changing the scene. Idle ticks keep handling events at the `max-frame-rate` pace. Render passes report pending output through the new `RenderPass::needs_frame`, which defaults to `false`.
- A UI `update_*` call that writes the values an element already has no longer advances the render revision, so unchanged frames reuse the retained render and on-demand rendering stays idle.
- Ticks that present nothing, such as idle on-demand ticks, now run at the refresh rate of the fastest display showing a window instead of a fixed 60 per second, or at the `max-frame-rate` pace when that is slower. GHI windows report their display's refresh rate through `Window::refresh_interval` and the new `Events::DisplayChanged` event on macOS, Win32, and Wayland, and `Renderer::refresh_interval` returns the fastest one.
- An idle `render-on-demand` application now waits in the window system instead of ticking. It keeps ticking while a frame is being shown, and wakes for window events, inspector requests, and anything that wakes its `LoopWaker`. After a wait, `Time::delta` is capped at one frame while `Time::elapsed` includes the wait.
- Systems the application loop drives report when they next need a tick, wired explicitly: `GraphicsApplication::waker` hands out a `LoopWaker` (which converts into a `std::task::Waker`), and `GraphicsApplication::schedule_tick` takes the instant of the next tick they need. A UI engine answers both with `Engine::set_waker` and `Engine::next_tick`, so animations keep running at the display rate and UI timers fire on time.
- `ghi::window::App::poll` and `Renderer::poll_windows` take a `Wait` that lets them wait for the first event, and `App::waker` returns an `AppWaker` that ends such a wait from any thread.
- Simulation can step at its own rate: `GraphicsApplication::tick_stepped_with` takes a `simulate` callback, run zero or more times per tick with a fixed step from the new `simulation-rate` parameter (default 60), and a `frame` callback run once with the display-derived time. The world and physics advance with the steps. A frame that falls more than eight steps behind drops the outstanding time. `tick_with` keeps simulating once per frame and panics when `simulation-rate` disagrees with the rate frames are presented at.
- Frames show the world interpolated between the two most recent simulation steps, one step behind. `PipelineManager::step` marks the end of a step, transforms read in it are that step's sample, and `PipelineManager::prepare` receives `alpha`, how far the frame lies between the two latest steps. The visibility pipeline moves each renderable between its samples and holds transforms written outside a step where they are; `Transform::interpolate` blends two transforms.
- `.flipbook` assets declare `frames_per_second` and the ordered `images` of a sequence. The standard setup bakes them together with their images into a `Flipbook` resource that owns its solved image references.
- `PipelineManager::prepare` receives `time`, the frame clock's elapsed time, after `alpha`, so managers can drive playback such as flipbooks at the display rate instead of the simulation rate.

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
