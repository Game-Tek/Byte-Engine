//! Frame orchestration and ownership of graphics hardware resources.
//!
//! Applications register windows, pipeline managers, and post-scene
//! [`crate::rendering::RenderFeature`] values with [`Renderer`]. The graphics
//! application owns frame timing and calls the renderer in lifecycle order.

#![expect(
	clippy::disallowed_types,
	reason = "Not given one owner yet; see \"Give the remaining shared state one owner\" in todo.md"
)]

/// The `Registration` struct is one feature the application registered, with the pass state every pass it builds
/// shares. Its index in the renderer's list is its [`RenderFeatureId`].
struct Registration {
	/// The feature's name, which `render.pass.<name>` sets the state by.
	name: &'static str,
	state: RenderPassState,
	/// Which sinks get a pass; a scene background reaches every sink whose scene pipeline asks for it.
	scope: SinkScope,
	kind: RegistrationKind,
}

/// The `RegistrationKind` enum is what a registration builds for each sink.
enum RegistrationKind {
	/// A pass that records its own commands.
	Pass(Box<dyn InstalledPasses>),
	/// A pixel step, which the renderer records alone or fused with the steps registered next to it.
	Step {
		/// The step's own `tile` pipeline, which fusions name it by.
		pipeline: &'static str,
		steps: Box<dyn InstalledSteps>,
	},
	/// The scene background, which scene pipelines record; see [`Renderer::set_scene_background`].
	Background(Box<dyn InstalledBackground>),
}

impl Registration {
	/// Reports whether the registration records after the scene pipelines of every sink in its scope, so it takes part
	/// in the plan every sink with a camera follows.
	fn is_post_scene(&self) -> bool {
		match self.kind {
			RegistrationKind::Pass(_) => matches!(self.scope, SinkScope::All | SinkScope::Windows),
			RegistrationKind::Step { .. } => true,
			RegistrationKind::Background(_) => false,
		}
	}

	/// Returns what the registration installed, which the renderer updates, asks for frames, and drops sinks from the
	/// same way whatever its kind.
	fn installed(&mut self) -> &mut dyn InstalledFeature {
		match &mut self.kind {
			RegistrationKind::Pass(passes) => &mut **passes,
			RegistrationKind::Step { steps, .. } => &mut **steps,
			RegistrationKind::Background(background) => &mut **background,
		}
	}

	/// Returns the installed passes, which only a pass registration has.
	fn passes(&mut self) -> &mut dyn InstalledPasses {
		match &mut self.kind {
			RegistrationKind::Pass(passes) => &mut **passes,
			RegistrationKind::Step { .. } | RegistrationKind::Background(_) => {
				unreachable!("Only pass registrations build passes.")
			}
		}
	}

	/// Returns the installed passes while the registration is an enabled pass registration, for the questions the
	/// renderer asks only enabled passes.
	fn enabled_passes(&self) -> Option<&dyn InstalledPasses> {
		match &self.kind {
			RegistrationKind::Pass(passes) if self.state == RenderPassState::Enabled => Some(&**passes),
			_ => None,
		}
	}

	/// Returns the installed steps, which only a step registration has.
	fn steps(&mut self) -> &mut dyn InstalledSteps {
		match &mut self.kind {
			RegistrationKind::Step { steps, .. } => &mut **steps,
			RegistrationKind::Pass(_) | RegistrationKind::Background(_) => {
				unreachable!("Only step registrations build steps.")
			}
		}
	}

	/// Returns the installed scene background, which only the background registration has.
	fn background(&mut self) -> &mut dyn InstalledBackground {
		match &mut self.kind {
			RegistrationKind::Background(background) => &mut **background,
			RegistrationKind::Pass(_) | RegistrationKind::Step { .. } => {
				unreachable!("Only the background registration builds backgrounds.")
			}
		}
	}
}

/// Returns where each post-scene node of a sink stands relative to its terminal node, in order. `overlays` says, for
/// each node, whether it draws over `main` instead of replacing it, and `scene_main` whether a scene pipeline of the
/// sink wrote a `main`.
///
/// The last node that replaces `main` is terminal, and the overlays after it draw over the final target. With no such
/// node, overlays draw over the copy of the scene's `main`. A sink with neither has no `main` for an overlay to draw
/// over, so its last node is terminal and starts the picture itself.
fn main_places(overlays: &[bool], scene_main: bool) -> SmallVec<[MainPlace; 16]> {
	let replacers = overlays.iter().rposition(|overlay| !overlay).map_or(0, |last| last + 1);
	if replacers == 0 && !scene_main {
		let last = overlays.len().saturating_sub(1);
		return (0..overlays.len())
			.map(|node| {
				if node == last {
					MainPlace::Terminal
				} else {
					MainPlace::Before
				}
			})
			.collect();
	}
	(0..overlays.len())
		.map(|node| match (node + 1).cmp(&replacers) {
			std::cmp::Ordering::Less => MainPlace::Before,
			std::cmp::Ordering::Equal => MainPlace::Terminal,
			std::cmp::Ordering::Greater => MainPlace::Over,
		})
		.collect()
}

/// Returns a registration of the feature named `name`, enabled, that builds `kind` for the sinks `scope` names.
fn registration(name: &'static str, scope: SinkScope, kind: RegistrationKind) -> Registration {
	Registration {
		name,
		state: RenderPassState::Enabled,
		scope,
		kind,
	}
}

/// Returns every registration's name with its state, for the configuration that sets them by name.
fn registration_states(registrations: &mut [Registration]) -> impl Iterator<Item = (&'static str, &mut RenderPassState)> {
	registrations
		.iter_mut()
		.map(|registration| (registration.name, &mut registration.state))
}

/// The `PlannedWork` enum is one post-scene node a sink builds, with the registrations it records.
enum PlannedWork<'p> {
	/// The pass registration at this index.
	Pass(usize),
	/// The step registrations at these indices, in order, recorded as one pass with the pipeline `pipeline`.
	Run { steps: SmallVec<[usize; 4]>, pipeline: &'p str },
}

/// The `FusionCatalog` enum tracks the fusions the renderer groups pixel steps with, read once when the graph closes.
enum FusionCatalog {
	/// Not read yet. The first window starts reading it.
	Unread,
	/// Read on a thread of its own, which sends the fusions when done.
	Loading(std::sync::mpsc::Receiver<Vec<Fusion>>),
	/// The passes every sink records, grouped with the fusions read.
	Planned(Vec<Planned>),
}
type SinkId = usize;
/// Identifies one node of a sink's graph that a post-scene registration built, by its index in the renderer's list.
type PassNodeId = usize;
type PipelineManagerId = usize;

/// How one sink takes part in the frame being prepared.
#[derive(Clone, Copy)]
enum SinkFrame {
	/// The frame renders into this acquired swapchain image and presents it.
	Acquired(ghi::PresentKey, Extent, ghi::SwapchainHandle),
	/// No part of the window can be seen, so the frame skips it without acquiring, unless a screenshot captures it.
	Hidden,
	/// The window had no usable swapchain image, so the frame skips it.
	Unusable,
	/// The sink was removed, so no frame renders it again.
	Removed,
	/// The sink renders into this render image at this extent, which needs no acquisition and presents nothing.
	Image(Extent, ghi::ImageHandle),
}

/// The `SceneLighting` struct keeps the sun and fog of one scene, which every sink of the scene carries to its passes.
#[derive(Default)]
struct SceneLighting {
	/// The scene's newest directional light; see [`Renderer::create_directional_light`].
	sun_light: Option<(Handle, DirectionalLight)>,
	/// The scene's newest fog; see [`Renderer::create_fog`].
	fog: Option<ExponentialHeightFog>,
}

/// The `SinkTarget` enum names where one sink's graph ends, so windows and render images share every other part of a
/// sink: its scene, its passes, and its render targets.
enum SinkTarget {
	Window {
		/// The scene the window shows, which is always the world.
		scene: SceneId,
		/// The native window, which the renderer's [`ghi::window::App`] owns.
		window: ghi::window::WindowId,
		swapchain: ghi::SwapchainHandle,
		/// Whether the latest acquisition warned that the window cannot render.
		warned: bool,
		/// The handle the application created the window with.
		handle: Handle,
		/// The features the application gave the window.
		features: window::Features,
	},
	Image {
		/// The handle the application created the render image with.
		handle: Handle,
		/// The scene the render image shows.
		scene: SceneId,
		image: ghi::ImageHandle,
		/// The extent the image renders at, in pixels.
		extent: Extent,
	},
}

impl SinkTarget {
	/// The handle the application created the window or render image with.
	fn handle(&self) -> Handle {
		match self {
			SinkTarget::Window { handle, .. } | SinkTarget::Image { handle, .. } => *handle,
		}
	}

	/// The scene the sink shows.
	fn scene(&self) -> SceneId {
		match self {
			SinkTarget::Window { scene, .. } | SinkTarget::Image { scene, .. } => *scene,
		}
	}
}

/// The scene sink `sink_id` of `sinks` shows. A removed sink shows nothing, so it sorts with the world.
fn scene_of(sinks: &[Option<SinkTarget>], sink_id: SinkId) -> SceneId {
	sinks[sink_id].as_ref().map_or(SceneId::WORLD, SinkTarget::scene)
}

/// The `NodeWork` enum is what one post-scene node of a sink records.
enum NodeWork {
	/// The sink's pass of the pass registration at this index.
	Pass(usize),
	/// A run of pixel steps, which records the steps' registrations as one tile pass.
	Run(PixelRun),
}

impl NodeWork {
	/// Returns the name the work reports under: its registration's, or its run's.
	fn name(&self, registrations: &[Registration]) -> &'static str {
		match self {
			Self::Pass(index) => registrations[*index].name,
			Self::Run(run) => run.name(),
		}
	}
}

/// The `PassNode` struct keeps one post-scene node of a sink's graph with what the renderer needs to record and capture
/// it.
struct PassNode {
	work: NodeWork,
	sink: SinkId,
	/// The step of the sink's frame the node records as: [`RenderNode::Over`] for an overlay that draws over the
	/// sink's final target, which records after the node or copy that fills that target.
	node: RenderNode,
	/// Every name and alias the node wrote when it was built, so screenshots can find its outputs.
	writable_targets: Vec<(String, ghi::ImageOrSwapchain)>,
	/// The copy that forwards `main` into the node's replacement in a frame the node records nothing. It is `None`
	/// when the node replaced no `main`, and when its replacement is placed as the incoming `main`'s image instead; see
	/// [`RenderTargets::forward_main`].
	main_copy: Option<ImageBypassPass>,
	/// The GPU counter around the node's commands and the metric its time is reported under.
	counter: Option<GpuCounter>,
	/// The targets the node reads, which scene pipelines write for the sink while the node is enabled.
	reads: crate::rendering::TargetSet,
	/// The registry index of the debug views the node declared, which its sink carries while it prepares.
	debug_views: Option<usize>,
	/// Whether the node is the sink's upscaling pass or comes after it, so it renders at the sink's output extent; see
	/// [`Sink::at_output`].
	at_output: bool,
}

impl PassNode {
	/// Returns the name the node reports under and screenshots find it by.
	fn name(&self, registrations: &[Registration]) -> &'static str {
		self.work.name(registrations)
	}

	/// Returns the state that decides whether the node applies its effect: its registration's, or, for a run, enabled
	/// while any of its steps is.
	fn state(&self, registrations: &[Registration]) -> RenderPassState {
		match &self.work {
			NodeWork::Pass(index) => registrations[*index].state,
			NodeWork::Run(run)
				if run
					.steps()
					.iter()
					.any(|step| registrations[*step].state == RenderPassState::Enabled) =>
			{
				RenderPassState::Enabled
			}
			NodeWork::Run(_) => RenderPassState::Bypassed,
		}
	}

	/// Returns the node's installed passes while they are enabled; a run of pixel steps has none.
	fn enabled_passes<'r>(&self, registrations: &'r [Registration]) -> Option<&'r dyn InstalledPasses> {
		match &self.work {
			NodeWork::Pass(index) => registrations[*index].enabled_passes(),
			NodeWork::Run(_) => None,
		}
	}

	/// Reports whether the node is enabled and its pass asks for lens samples; see [`RenderPass::samples_lens`].
	fn samples_lens(&self, registrations: &[Registration]) -> bool {
		self.enabled_passes(registrations)
			.is_some_and(|passes| passes.samples_lens(self.sink))
	}

	/// Reports whether the node is enabled and its pass asks for jitter; see [`RenderPass::jitters_view`].
	fn jitters_view(&self, registrations: &[Registration]) -> bool {
		self.enabled_passes(registrations)
			.is_some_and(|passes| passes.jitters_view(self.sink))
	}

	/// Returns the render scale the node's pass asks for while it is enabled, and `1` while it is bypassed, so a
	/// bypassed upscaler's `main` is copied between targets of one extent; see [`RenderPass::render_scale`].
	fn render_scale(&self, registrations: &[Registration]) -> f32 {
		self.enabled_passes(registrations)
			.map_or(1.0, |passes| passes.render_scale(self.sink))
	}

	/// Prepares the node's command and, when the node records nothing and its replacement for `main` can't be the
	/// incoming `main`'s image, the copy that forwards `main` into the replacement.
	fn prepare<'a>(
		&mut self,
		registrations: &mut [Registration],
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> [Option<RenderPassReturn<'a>>; 2] {
		let command = match &mut self.work {
			NodeWork::Pass(index) => {
				let registration = &mut registrations[*index];
				let state = registration.state;
				registration.passes().prepare(state, frame, sink, frame_allocator)
			}
			NodeWork::Run(run) => {
				// Every step prepares, so a skipped one still drains its inputs; only enabled, active steps apply.
				let mut mask = 0;
				for (bit, step) in run.steps().iter().enumerate() {
					let registration = &mut registrations[*step];
					let state = registration.state;
					if registration.steps().prepare(frame, sink) && state == RenderPassState::Enabled {
						mask |= 1 << bit;
					}
				}
				run.prepare(mask, frame, sink, frame_allocator)
			}
		};
		// A node that records nothing leaves `main` unchanged, whatever the reason: bypassed, nothing to apply, or a
		// pipeline that isn't ready.
		let forward = match command {
			Some(_) => None,
			None => self
				.main_copy
				.as_mut()
				.and_then(|copy| copy.prepare(frame, sink, frame_allocator)),
		};
		[command, forward]
	}
}

/// A GPU timing counter paired with the metric the renderer publishes its durations to.
type GpuCounter = (ghi::CounterHandle, MetricId);

/// Returns the point of the unit disc a sink's view samples the lens from on frame `index`.
///
/// Consecutive frames walk Roberts' R2 sequence, which covers the disc evenly from any starting frame, so an average
/// that restarts mid-sequence converges as fast as one from frame 0. The square root spreads the points evenly over
/// the disc's area.
fn lens_sample(index: u64) -> [f32; 2] {
	// The R2 sequence's steps are one over the plastic number and its square.
	let u = (0.5 + index as f64 * 0.754_877_666_246_692_7).fract();
	let v = (0.5 + index as f64 * 0.569_840_290_998_053_2).fract();
	let radius = u.sqrt();
	let (sine, cosine) = (std::f64::consts::TAU * v).sin_cos();
	[(radius * cosine) as f32, (radius * sine) as f32]
}

/// Returns the sub-pixel offset, in pixels, a jittered sink's view moves by on frame `index`; see
/// [`Sink::with_jitter`].
///
/// Consecutive frames walk the first ⌈8 / `render_scale`²⌉ points of the Halton (2, 3) sequence, skipping its point
/// 0, which sits on a corner, and centered on the render pixel. Eight points cover the pixel evenly well within the
/// frames a temporal blend weighs; a render pixel smaller than an output pixel needs as many points per output pixel,
/// which is FSR's published rule.
fn jitter_offset(index: u64, render_scale: f32) -> [f32; 2] {
	let phases = (8.0 / (render_scale * render_scale)).ceil() as u64;
	let point = index % phases + 1;
	[radical_inverse(point, 2) - 0.5, radical_inverse(point, 3) - 0.5]
}

/// Returns `index` with its digits in `base` mirrored around the radix point, the `index`th point of the van der
/// Corput sequence in that base.
fn radical_inverse(mut index: u64, base: u64) -> f32 {
	let (mut result, mut scale) = (0.0, 1.0 / base as f32);
	while index > 0 {
		result += (index % base) as f32 * scale;
		index /= base;
		scale /= base as f32;
	}
	result
}

/// The [`Renderer`] struct owns graphics queues, render targets, scene pipelines,
/// and per-sink render passes.
///
/// [`crate::application::graphics::GraphicsApplication::new`] creates the renderer, connects its resource manager,
/// and starts its pipeline compilation servers. Prefer the setup helpers in [`crate::application::graphics`] to
/// compose it. For custom composition, add a [`PipelineManager`] and sink-local [`RenderPass`] values, then register
/// windows and cameras.
pub struct Renderer {
	/// The monotonically increasing identity of the next graphics submission frame.
	started_frame_count: u64,

	/// The frame index the acquisitions belong to and, per window adopted so far, how the window takes part in it.
	acquisitions: (u64, SmallVec<[SinkFrame; 16]>),
	/// The minimum time between presented frames applied to every window; `None` presents on every refresh.
	present_interval: Option<std::time::Duration>,
	/// Whether no one watches the windows: they open in the background and render whether or not they can be seen.
	unattended: bool,
	/// The player's motion preference, which every sink carries to its passes; see [`Self::set_motion`].
	motion: crate::accessibility::MotionPreference,
	/// The swapchain image count requested for every window; `None` keeps the backend default.
	swapchain_images: Option<u8>,
	/// Sink indices and their camera handles.
	sink_cameras: SmallVec<[(SinkId, Handle); 16]>,
	/// Cameras and their stable handles. Each one is placed at its handle's shown transform.
	cameras: SmallVec<[(Handle, Camera); 16]>,
	/// The sun and fog of every scene, by [`SceneId`]; the world's comes first.
	scenes: SmallVec<[SceneLighting; 4]>,

	render_targets: RenderTargets,
	#[cfg(debug_assertions)]
	resource_updates: Option<resource_management::resource::ResourceUpdateListener>,

	/// Every sink's post-scene nodes, in the order each sink records them. A node's index is its [`PassNodeId`].
	pass_nodes: SmallVec<[PassNode; 64]>,
	/// Every registered feature, in registration order, which is the order every sink records the post-scene ones. A
	/// registration's index is its [`RenderFeatureId`]. Passes registered for one window build on the first frame that
	/// finds their window open.
	registrations: SmallVec<[Registration; 16]>,
	/// The sinks of windows without a camera whose own passes are built, which closes their graphs.
	window_graphs: SmallVec<[SinkId; 16]>,
	/// The fusions runs of pixel steps use, and the passes they group the registrations into.
	fusions: FusionCatalog,
	/// Whether runs of pixel steps use fusions; `render.fusion`.
	fusion_enabled: bool,
	/// Where the fusion catalog is read from; `None` until [`Self::set_resource_manager`], which leaves every step on its
	/// own.
	resource_manager: Option<EntityHandle<ResourceManager>>,
	/// The registration of the scene background, when the application set one; see [`Self::set_scene_background`].
	scene_background: Option<usize>,
	scene_presentation_copies: SmallVec<[(SinkId, ImageBypassPass); 16]>,
	pending_sink_initializations: SmallVec<[SinkId; 16]>,
	configuration: ConfigurationPort,
	pending_configuration: VecDeque<PendingRenderPassConfiguration>,
	/// Which debug views exist and which pool slot each shown one holds; see [`crate::rendering::debug_views`].
	debug_views: DebugViewRegistry,
	/// The registry index of the debug views each scene pipeline declared, by the pipeline's id.
	scene_debug_views: SmallVec<[Option<usize>; 16]>,
	debug_view_settings: DebugViewSettings,
	/// The port `render.debug.view.<pass>.<view>` arrives on.
	debug_view_configuration: ConfigurationPort,
	pending_debug_view_configuration: VecDeque<PendingDebugView>,
	/// The pass that tiles each sink's shown debug views over its presented image, when the build has debug views.
	debug_view_presents: SmallVec<[(SinkId, DebugViewPresentPass); 16]>,

	/// Every scene pipeline with the scene it draws. A manager's index is its [`PipelineManagerId`].
	pipeline_managers: SmallVec<[(SceneId, Box<dyn PipelineManager>); 16]>,
	pipeline_compilation_client: crate::rendering::PipelineManagerClient,
	pipeline_compilation_manager: crate::rendering::pipeline_compilation::PipelineManager,

	/// The GHI queue where graphics commands are submitted. The main rendering operations occur on this queue.
	graphics_queue_handle: ghi::QueueHandle,

	render_command_buffer: ghi::CommandBufferHandle,
	render_finished_synchronizer: ghi::SynchronizerHandle,
	defer_first_frame_sink_setup: bool,
	/// Whether renderer state changed in a way that the last presented frame does not show.
	redraw_requested: bool,

	/// Where the renderer reports the GPU time of completed frames.
	metrics: Arc<Metrics>,
	/// The counter around every command of a frame, reported as `frame`.
	frame_counter: Option<GpuCounter>,
	/// The counter around each pipeline manager's scene commands on each sink, reported as `scene.<manager>`.
	scene_counters: SmallVec<[(PipelineManagerId, SinkId, GpuCounter); 16]>,
	/// The counters scene pipelines and render passes created around their own stages, reported as `stage.<name>`.
	stage_counters: SmallVec<[GpuCounter; 32]>,

	/// Where the compiled pipeline cache is saved when the renderer drops. `None` leaves caching to the GPU driver.
	pipeline_cache_path: Option<std::path::PathBuf>,

	/// The GHI context where all rendering resources and operations are performed. Only the render thread uses it.
	/// This field drops after renderer subsystems so they finish pending GPU work before their resources are destroyed.
	context: ghi::implementation::Context,

	/// Where each sink's graph ends, a window or a render image, by sink. A removed sink leaves `None`, so the other
	/// sinks keep their ids. Declared after `context`, because destroying a swapchain's surface may still talk to its
	/// window.
	sinks: SmallVec<[Option<SinkTarget>; 16]>,
	/// The sink of the first window opened, the main window, which stays its sink after it closes.
	main_window: Option<SinkId>,
	/// The windowing connection that pumps every window's events. Declared after `sinks` so they drop first.
	app: Option<ghi::window::App>,
}

impl Drop for Renderer {
	/// Saves the pipelines this run compiled, so the next run creates them from the cache.
	fn drop(&mut self) {
		let Some(path) = &self.pipeline_cache_path else {
			return;
		};
		if let Err(error) = self.context.pipeline_cache_data(|data| save_pipeline_cache(path, data)) {
			log::warn!(
				"Failed to save the pipeline cache at {}: {error}. The most likely cause is that the directory does not exist or is not writable. The next run will compile its pipelines again.",
				path.display()
			);
		}
	}
}

/// Brings the pipeline cache file up to date with one write of only the bytes it lacks, or with no disk access when
/// nothing was compiled.
///
/// The cache is an append-only log, so new pipelines are appended to the file that was loaded. A file the device
/// kept nothing of is overwritten. A write that is interrupted only costs the pipelines it was writing: loading
/// keeps the records before the cut and the next save replaces the file.
fn save_pipeline_cache(path: &std::path::Path, data: ghi::pipelines::CacheData) -> std::io::Result<()> {
	use std::io::Write as _;

	if data.added.is_empty() {
		return Ok(());
	}
	let mut options = std::fs::OpenOptions::new();
	if data.loaded.is_empty() {
		options.write(true).create(true).truncate(true);
	} else {
		options.append(true);
	}
	options.open(path)?.write_all(data.added)
}

impl Renderer {
	/// Creates a renderer that records and presents on `device`'s graphics queue, and returns it with the pipeline
	/// compilation servers that compile its pipelines. Run each server with [`PipelineManagerServer::run`] on a thread
	/// that owns it.
	///
	/// `render_pass_configuration` is the port render-pass states arrive on, each named by its pass, and
	/// `debug_view_configuration` the port `render.debug.view.<pass>.<view>` arrives on, with `debug_views` the catalog
	/// the renderer publishes the declared views to for the inspector.
	///
	/// # Parameters
	/// Read from `parameters`, the `render` subtree of the application's:
	/// - `startup.defer-sink-setup`: Presents the first window frame before constructing sink render pipelines.
	///   Defaults to false.
	/// - `debug.targets`: How many debug targets each sink's pool holds, from 1 to 15, which is how many debug views
	///   can be shown at once. Defaults to 4.
	/// - `debug-views.enabled`: Whether this build can show debug views; see
	///   [`crate::rendering::pipelines::visibility::VisibilityFeatures`]. Defaults to on in debug builds only.
	/// - `pipeline-compilation.threads`: Sets how many threads compile pipelines. Defaults to half the
	///   available cores, between one and four.
	/// - `swapchain.images`: Sets how many swapchain images each window requests. Backends clamp it to what
	///   they support (Metal takes 2 or 3). Defaults to the backend's choice for the presentation mode.
	/// - `pipeline-cache.path`: Keeps compiled pipelines in this file, so later runs create them without
	///   compiling shaders. The file is read here, before any pipeline is created. When the renderer drops, the
	///   pipelines compiled since are appended to it, and a run that compiled none does not touch it. Create `device` with the same parameters, because the device decides whether it keeps a cache.
	///   Defaults to none, which leaves pipeline caching to the GPU driver.
	///
	/// GPU times of every frame, scene pipeline, post-scene pass, and the stages those create with
	/// [`RenderPassBuilder::create_gpu_counter`] are reported to `metrics` once each frame completes, under `frame`,
	/// `scene.<manager>`, `pass.<name>`, and `stage.<name>`, with `@<sink>` appended for sinks after the first.
	/// They report only when `device` was created with `ghi.features.counters` on, which is the default on debug
	/// builds only; see [`GraphicsDevice::new`](crate::rendering::GraphicsDevice::new).
	///
	/// Next, add a scene pipeline with [`Self::add_pipeline_manager`].
	pub fn new(
		device: &crate::rendering::GraphicsDevice,
		parameters: impl Parameters + Copy,
		render_pass_configuration: ConfigurationPort,
		debug_view_configuration: ConfigurationPort,
		debug_views: Arc<DebugViewCatalog>,
		metrics: Arc<Metrics>,
	) -> (Self, Vec<PipelineManagerServer>) {
		let debug_view_settings = DebugViewSettings::from_parameters(parameters);
		let defer_first_frame_sink_setup = parameters
			.get_parameter("startup.defer-sink-setup")
			.map(|parameter| parameter.as_bool_simple())
			.unwrap_or(false);
		let fusion_enabled = parameters
			.get_parameter("fusion")
			.map(|parameter| parameter.as_bool_simple())
			.unwrap_or(true);
		let swapchain_images = parameters
			.get_parameter("swapchain.images")
			.and_then(|parameter| parameter.parse::<u8>().ok())
			.filter(|count| *count > 0);

		let mut context = device.create_context();
		context.set_frames_in_flight(2);

		let pipeline_cache_path = parameters
			.get_parameter("pipeline-cache.path")
			.map(|parameter| std::path::PathBuf::from(parameter.value()));
		// The compilation servers below share the device's cache, so it is loaded before they can create pipelines.
		if let Some(path) = &pipeline_cache_path {
			match std::fs::read(path) {
				// The device keeps the file's bytes as the cache itself, so they are read once and never copied.
				Ok(data) => context.load_pipeline_cache(data),
				// The first run has no cache yet.
				Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
				Err(error) => log::warn!(
					"Failed to read the pipeline cache at {}: {error}. The most likely cause is that the file is not readable by this user. Pipelines will be compiled from their shaders.",
					path.display()
				),
			}
		}
		let pipeline_compilation_server_count = parameters
			.get_parameter("pipeline-compilation.threads")
			.and_then(|parameter| parameter.parse::<usize>().ok())
			.unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |count| (count.get() / 2).clamp(1, 4)));
		let (pipeline_compilation_client, pipeline_compilation_manager, pipeline_compilation_servers) =
			crate::rendering::pipeline_compilation::PipelineManager::new(&mut context, pipeline_compilation_server_count);

		let graphics_queue_handle = device.graphics_queue();

		let render_command_buffer = context.queue(graphics_queue_handle).create_command_buffer(Some("Render"));
		let render_finished_synchronizer = context.create_synchronizer(Some("Render Finisished"), true);
		let frame_counter = metrics
			.register(MetricKind::Gpu, "frame")
			.map(|metric| (context.create_counter(Some("Frame")), metric));

		let renderer = Renderer {
			context,

			started_frame_count: 0,

			sinks: SmallVec::with_capacity(16),
			main_window: None,
			app: None,
			acquisitions: (0, SmallVec::with_capacity(16)),
			present_interval: None,
			unattended: false,
			motion: crate::accessibility::MotionPreference::Full,
			swapchain_images,
			sink_cameras: SmallVec::with_capacity(16),
			cameras: SmallVec::with_capacity(16),
			scenes: smallvec::smallvec![SceneLighting::default()],

			render_targets: RenderTargets::new(),
			#[cfg(debug_assertions)]
			resource_updates: None,

			pass_nodes: SmallVec::with_capacity(64),
			registrations: SmallVec::with_capacity(16),
			window_graphs: SmallVec::new(),
			fusions: FusionCatalog::Unread,
			fusion_enabled,
			resource_manager: None,
			scene_background: None,
			scene_presentation_copies: SmallVec::with_capacity(16),
			pending_sink_initializations: SmallVec::with_capacity(16),
			configuration: render_pass_configuration,
			pending_configuration: VecDeque::new(),
			debug_views: DebugViewRegistry::new(debug_view_settings, debug_views),
			scene_debug_views: SmallVec::new(),
			debug_view_settings,
			debug_view_configuration,
			pending_debug_view_configuration: VecDeque::new(),
			debug_view_presents: SmallVec::new(),

			pipeline_managers: SmallVec::with_capacity(8),
			pipeline_compilation_client,
			pipeline_compilation_manager,

			graphics_queue_handle,

			render_command_buffer,
			render_finished_synchronizer,
			defer_first_frame_sink_setup,
			redraw_requested: true,

			metrics,
			frame_counter,
			scene_counters: SmallVec::new(),
			stage_counters: SmallVec::new(),

			pipeline_cache_path,
		};
		(renderer, pipeline_compilation_servers)
	}

	/// Creates the counter that times one node of `sink` and registers the metric it reports to, named by
	/// [`gpu_metric_name`].
	fn create_gpu_counter(&mut self, kind: &str, name: &str, sink_id: SinkId) -> Option<GpuCounter> {
		let metric_name = gpu_metric_name(kind, name, sink_id);
		let metric = self.metrics.register(MetricKind::Gpu, &metric_name)?;
		Some((self.context.create_counter(Some(&metric_name)), metric))
	}

	/// Registers a `stage.<name>` metric for every counter a node of `sink` created through its builder, as
	/// [`RenderPassBuilder::take_gpu_counters`] hands them over, so [`Self::publish_gpu_metrics`] reports their times.
	/// A counter past the metric capacity keeps measuring unreported.
	fn adopt_stage_counters(&mut self, counters: Vec<(String, ghi::CounterHandle)>, sink_id: SinkId) {
		for (name, counter) in counters {
			if let Some(metric) = self
				.metrics
				.register(MetricKind::Gpu, &gpu_metric_name("stage", &name, sink_id))
			{
				self.stage_counters.push((counter, metric));
			}
		}
	}

	/// Registers the debug views a node declared, as [`RenderPassBuilder::take_debug_views`] hands them over, under
	/// `name`, the pass or manager name `render.debug.view.<name>.<view>` selects them by, and returns the index the
	/// node's shown views are read by.
	fn register_debug_views(&mut self, name: &str, views: Option<&'static [&'static str]>) -> Option<usize> {
		let index = self.debug_views.register(name, views?);
		self.debug_view_configuration.declare(self.debug_views.parameters());
		Some(index)
	}

	/// Reports the GPU time every counter measured in `frame`, now that the frame completed.
	fn publish_gpu_metrics(&self, frame: u64) {
		let counters = self
			.frame_counter
			.iter()
			.chain(self.scene_counters.iter().map(|(_, _, counter)| counter))
			.chain(self.stage_counters.iter())
			.chain(self.pass_nodes.iter().filter_map(|node| node.counter.as_ref()));
		for (counter, metric) in counters {
			if let Some(duration) = self.context.counter_duration(*counter) {
				self.metrics.set_gpu(frame, *metric, duration);
			}
		}
		self.metrics.complete_frame(frame);
	}

	/// Connects the renderer to the resource manager it finds fusions in and whose development rebakes rebuild its
	/// pipelines.
	///
	/// When the first window opens, the renderer reads the fusions in the resource store from it. In debug builds it
	/// also listens for replaced resources and recompiles the pipelines built from them. Call it in any order relative
	/// to adding pipeline managers and render passes. [`crate::application::graphics::GraphicsApplication::new`] calls
	/// it for you.
	pub fn set_resource_manager(&mut self, resource_manager: &EntityHandle<ResourceManager>) {
		self.resource_manager = Some(resource_manager.clone());
		#[cfg(debug_assertions)]
		{
			self.resource_updates = Some(resource_manager.resource_updates());
		}
	}

	/// Registers a scene pipeline manager with the renderer.
	///
	/// The renderer calls [`PipelineManager::prepare`] once per frame before it
	/// records scene commands. A manager should drain resident loader events there
	/// before it builds draws.
	///
	/// Next, add post-scene passes with
	/// [`Self::add_render_pass`] or create a window with
	/// [`Self::create_window`].
	pub fn add_pipeline_manager(&mut self, scene: SceneId, mut pipeline_manager: impl PipelineManager + 'static) {
		assert!(
			scene.index() < self.scenes.len(),
			"Scene pipeline names an unknown scene. The most likely cause is a scene id from another renderer. Install scene pipelines for `SceneId::WORLD` or a scene `Renderer::create_scene` returned."
		);
		let pipeline_manager_id = self.pipeline_managers.len();
		// Sinks of the scene that already exist get the manager's state now; deferred sinks get it when they
		// initialize.
		let sinks: SmallVec<[SinkId; 16]> = self
			.sink_cameras
			.iter()
			.map(|(sink_id, _)| *sink_id)
			.filter(|sink_id| !self.pending_sink_initializations.contains(sink_id) && scene_of(&self.sinks, *sink_id) == scene)
			.collect();
		for sink_id in sinks {
			self.create_manager_sink(&mut pipeline_manager, pipeline_manager_id, sink_id);
		}

		self.pipeline_managers.push((scene, Box::new(pipeline_manager)));
	}

	/// Adds a scene with no scene pipelines, sun, or fog, and returns its id.
	///
	/// Next, install its scene pipelines with [`Self::add_pipeline_manager`] and show it with
	/// [`Self::create_render_image`].
	pub fn create_scene(&mut self) -> SceneId {
		self.scenes.push(SceneLighting::default());
		SceneId::new(self.scenes.len() - 1)
	}

	/// Starts the builder for one node of a sink's graph; `place` says where the node stands relative to the sink's
	/// terminal pass.
	fn sink_builder(&mut self, sink_id: SinkId, node: RenderNode, place: MainPlace) -> RenderPassBuilder<'_> {
		let final_target = self.final_target(sink_id);
		RenderPassBuilder::new(
			&mut self.context,
			self.graphics_queue_handle,
			&mut self.render_targets,
			sink_id,
			final_target,
			self.pipeline_compilation_client.clone(),
			node,
			place,
			self.debug_view_settings,
		)
	}

	/// Lets one scene pipeline manager create its persistent state for a sink and records the targets it uses.
	///
	/// The scene background builds its pass for the sink with the manager's builder when the manager asks for one, and
	/// its environment for the sink's scene when the scene has none yet, so their targets and counters belong to the
	/// manager's node.
	fn create_manager_sink(
		&mut self,
		pipeline_manager: &mut dyn PipelineManager,
		pipeline_manager_id: PipelineManagerId,
		sink_id: SinkId,
	) {
		let scene = scene_of(&self.sinks, sink_id);
		let background = self.scene_background;
		// The registrations step aside while the builder borrows `self`.
		let mut registrations = std::mem::take(&mut self.registrations);
		let mut builder = self.sink_builder(sink_id, RenderNode::Scene(pipeline_manager_id), MainPlace::Before);
		pipeline_manager.create_sink(sink_id, &mut builder);
		if let Some(background) = background.map(|index| registrations[index].background()) {
			if builder.asks_for_scene_background() {
				background.create_pass(pipeline_manager_id, sink_id, &mut builder);
			}
			background.create_environment(scene, &mut builder);
		}
		builder.record_node();
		let stage_counters = builder.take_gpu_counters();
		let debug_views = builder.take_debug_views();
		self.registrations = registrations;
		if let Some(index) = self.register_debug_views(pipeline_manager.name(), debug_views) {
			if self.scene_debug_views.len() <= pipeline_manager_id {
				self.scene_debug_views.resize(pipeline_manager_id + 1, None);
			}
			self.scene_debug_views[pipeline_manager_id] = Some(index);
		}
		self.adopt_stage_counters(stage_counters, sink_id);
		if let Some(counter) = self.create_gpu_counter("scene", pipeline_manager.name(), sink_id) {
			self.scene_counters.push((pipeline_manager_id, sink_id, counter));
		}
	}

	fn initialize_scene_sink(&mut self, sink_id: SinkId) {
		// Every manager builds through `self`, so the list steps aside while they do.
		let scene = scene_of(&self.sinks, sink_id);
		let mut pipeline_managers = std::mem::take(&mut self.pipeline_managers);
		for (pipeline_manager_id, (manager_scene, pipeline_manager)) in pipeline_managers.iter_mut().enumerate() {
			if *manager_scene == scene {
				self.create_manager_sink(&mut **pipeline_manager, pipeline_manager_id, sink_id);
			}
		}
		self.pipeline_managers = pipeline_managers;

		self.add_post_scene_render_passes_for_sink(sink_id);
	}

	/// Changes the state of every registration named `name`, and with it the state of every pass they built.
	///
	/// Returns the number of registrations named `name`; `0` means that no registered feature uses it. Names come from
	/// [`RenderFeature::name`].
	pub fn set_render_pass_state(&mut self, name: &str, state: RenderPassState) -> usize {
		self.redraw_requested = true;
		set_render_pass_state(registration_states(&mut self.registrations), name, state)
	}

	/// Asserts that no sink has closed the render graph, so a registration for every sink still reaches every sink.
	fn assert_graph_open(&self, what: &str) {
		assert!(
			self.sinks.is_empty(),
			"Render graph is already closed. The most likely cause is that a {what} was registered after creating a window or render image. Register every pass before creating the first window or render image."
		);
	}

	/// Sets the scene background that every future sink's scene pipeline records between opaque and transparent
	/// surfaces, such as an atmosphere sky, and that may generate the environment each scene lights with.
	///
	/// Set it before creating a window. Its [`RenderFeature::name`] controls it like a post-scene pass. A later call
	/// replaces the feature an earlier one set, and returns the same id.
	pub fn set_scene_background<B: SceneBackground>(&mut self, background: B) -> RenderFeatureId<B> {
		self.assert_graph_open("scene background");
		let registration = registration(
			background.name(),
			SinkScope::All,
			RegistrationKind::Background(Box::new(InstalledSceneBackground::new(background))),
		);
		let index = match self.scene_background {
			Some(index) => {
				self.registrations[index] = registration;
				index
			}
			None => self.push_registration(registration),
		};
		self.scene_background = Some(index);
		RenderFeatureId::new(index)
	}

	/// Registers `feature`, which builds a pass for every sink `scope` names, and returns the id that reaches the
	/// feature with [`Self::render_feature_mut`].
	///
	/// A registration for [`SinkScope::All`] or [`SinkScope::Windows`] takes its place among the post-scene passes, in
	/// registration order, and must be made before the first window or render image is created: closing the graph at
	/// that boundary lets the renderer give the terminal pass the swapchain directly. A render image records the
	/// registrations for all sinks, and the last of those writes its image.
	///
	/// A registration for [`SinkScope::Window`] belongs to the one window created with that handle, which must have no
	/// camera. The window records its passes in the order they were registered, the last writing its swapchain, over
	/// black. Register every pass of a window before the frame after the window is created: that frame builds them and
	/// closes the window's graph.
	pub fn add_render_pass<F: RenderFeature>(&mut self, feature: F, scope: SinkScope) -> RenderFeatureId<F>
	where
		F::Pass: RenderPass<F>,
	{
		match scope {
			SinkScope::All | SinkScope::Windows => self.assert_graph_open("post-scene render pass"),
			SinkScope::Window(window) => {
				let built = self
					.sink_of(window)
					.is_some_and(|sink_id| self.window_graphs.contains(&sink_id));
				assert!(
					!built,
					"Render graph of the window is already closed. The most likely cause is that a pass was registered for a window a frame after the window was created. Register every pass of a window right after creating it."
				);
			}
		}
		let index = self.push_registration(registration(
			feature.name(),
			scope,
			RegistrationKind::Pass(Box::new(Installed::new(feature))),
		));
		RenderFeatureId::new(index)
	}

	/// Registers `feature`, whose pass is a pixel step with its own pipeline `pipeline`, for every future sink.
	///
	/// Register every step before creating a window, like a post-scene pass, in the order of the other passes. When the
	/// first window opens, the renderer groups the steps registered one after another into runs: each run takes the
	/// longest fusion in the resource store whose steps are their pipelines, and records as one pass. The feature
	/// builds the step's resources for one sink; the step reads the run's incoming `main` and must not replace it. See
	/// [pixel steps and fusion](/docs/reference/rendering#pixel-steps-and-fusion).
	pub fn add_pixel_step<F: RenderFeature>(&mut self, pipeline: &'static str, feature: F) -> RenderFeatureId<F>
	where
		F::Pass: PixelStep<F>,
	{
		self.assert_graph_open("pixel step");
		let index = self.push_registration(registration(
			feature.name(),
			SinkScope::All,
			RegistrationKind::Step {
				pipeline,
				steps: Box::new(InstalledStep(Installed::new(feature))),
			},
		));
		RenderFeatureId::new(index)
	}

	/// Returns the feature `id` names, so application code can change state its passes read, such as the editor
	/// gizmo's.
	///
	/// Changing a feature does not request a frame by itself; the feature asks for one with
	/// [`RenderFeature::needs_frame`], or call [`Self::request_redraw`].
	pub fn render_feature_mut<F: 'static>(&mut self, id: RenderFeatureId<F>) -> &mut F {
		self.registrations[id.index].installed().feature_mut().downcast_mut().expect(
			"Render feature has another type. The most likely cause is a feature id from another renderer, or the id of a scene background a later call replaced with another type.",
		)
	}

	/// Adds `registration` after the others and returns its index.
	fn push_registration(&mut self, registration: Registration) -> usize {
		self.registrations.push(registration);
		self.registrations.len() - 1
	}

	/// Starts reading the fusion catalog, once, when the render graph closes.
	///
	/// Without a resource manager or with `render.fusion` off, every pixel step is planned to run alone at once.
	fn start_fusion_catalog(&mut self) {
		if !matches!(self.fusions, FusionCatalog::Unread) {
			return;
		}
		let Some(resources) = self.resource_manager.clone().filter(|_| self.fusion_enabled) else {
			self.fusions = FusionCatalog::Planned(self.plan(&[]));
			return;
		};
		let (sender, receiver) = std::sync::mpsc::channel();
		let spawned = std::thread::Builder::new()
			.name("Fusion Catalog".to_string())
			.spawn(move || {
				let runtime = compio::runtime::Runtime::new().expect(
					"Failed to create the fusion catalog's runtime. The most likely cause is that the system ran out of file descriptors or threads.",
				);
				// The renderer may have closed before the catalog was read.
				let _ = sender.send(runtime.block_on(load_fusions(&resources)));
			});
		self.fusions = match spawned {
			Ok(_) => FusionCatalog::Loading(receiver),
			Err(error) => {
				log::error!(
					"Failed to start reading the fusion catalog: {error}. The most likely cause is that the system ran out of threads. Every pixel step records on its own."
				);
				FusionCatalog::Planned(self.plan(&[]))
			}
		};
	}

	/// Reports whether the fusion catalog is read and the passes are planned, adopting the catalog when it arrived.
	fn fusions_planned(&mut self) -> bool {
		let FusionCatalog::Loading(receiver) = &self.fusions else {
			return matches!(self.fusions, FusionCatalog::Planned(_));
		};
		let fusions = match receiver.try_recv() {
			Ok(fusions) => fusions,
			Err(std::sync::mpsc::TryRecvError::Empty) => return false,
			Err(std::sync::mpsc::TryRecvError::Disconnected) => {
				log::error!(
					"The fusion catalog stopped before it was read. The most likely cause is a panic while reading the resource store, which the log shows. Every pixel step records on its own."
				);
				Vec::new()
			}
		};
		self.fusions = FusionCatalog::Planned(self.plan(&fusions));
		true
	}

	/// Returns the indices of the registrations every sink with a camera records after its scene pipelines, in order.
	fn post_scene_registrations(&self) -> SmallVec<[usize; 16]> {
		(0..self.registrations.len())
			.filter(|index| self.registrations[*index].is_post_scene())
			.collect()
	}

	/// Groups the post-scene registrations into the passes every sink records, with `fusions`. The plan's indices are
	/// positions in [`Self::post_scene_registrations`].
	fn plan(&self, fusions: &[Fusion]) -> Vec<Planned> {
		let registrations = self
			.post_scene_registrations()
			.into_iter()
			.map(|index| match &self.registrations[index].kind {
				RegistrationKind::Step { pipeline, .. } => Some(*pipeline),
				RegistrationKind::Pass(_) | RegistrationKind::Background(_) => None,
			})
			.collect::<SmallVec<[_; 16]>>();
		plan(&registrations, fusions)
	}

	/// Builds one node of a sink's graph doing `work` and adds it after the sink's other nodes; `place` says where the
	/// node stands relative to the sink's terminal node, which must write the final target.
	///
	/// `registrations` are the renderer's, which step aside while the builder borrows `self`.
	fn add_sink_pass(&mut self, registrations: &mut [Registration], sink_id: SinkId, place: MainPlace, work: PlannedWork) {
		let node_id = self.pass_nodes.len();
		// An overlay over the final target records after the node or copy that fills that target.
		let node = match place {
			MainPlace::Over => RenderNode::Over(node_id),
			MainPlace::Before | MainPlace::Terminal => RenderNode::Pass(node_id),
		};
		let mut builder = self.sink_builder(sink_id, node, place);
		let work = match work {
			PlannedWork::Pass(index) => {
				registrations[index].passes().create_pass(sink_id, &mut builder);
				NodeWork::Pass(index)
			}
			PlannedWork::Run { steps, pipeline } => {
				// The run's steps are distinct registrations in order, so each lends its steps at once.
				let mut installed = registrations
					.iter_mut()
					.enumerate()
					.filter(|(index, _)| steps.contains(index))
					.map(|(index, registration)| {
						let name = registration.name;
						(index, name, registration.steps())
					})
					.collect::<SmallVec<[_; 4]>>();
				NodeWork::Run(PixelRun::new(&mut builder, sink_id, &mut installed, pipeline))
			}
		};
		assert!(
			place != MainPlace::Terminal || builder.writes_final_output(),
			"Final render pass has no presentation output. The most likely cause is that it created or aliased an ordinary image instead of calling `RenderPassBuilder::create_main_render_target`."
		);
		builder.record_node();
		let reads = builder.read_targets();
		let writable_targets = builder.writable_targets();
		let main_copy = builder.take_main_copy();
		let stage_counters = builder.take_gpu_counters();
		let debug_views = builder.take_debug_views();
		// The upscaling pass marks its sink while it is built, so it and every later pass render at the output extent.
		let at_output = self.render_targets.is_output(sink_id);
		let name = work.name(registrations);
		let debug_views = self.register_debug_views(name, debug_views);
		self.adopt_stage_counters(stage_counters, sink_id);
		let counter = self.create_gpu_counter("pass", name, sink_id);
		self.pass_nodes.push(PassNode {
			work,
			sink: sink_id,
			node,
			writable_targets,
			main_copy,
			counter,
			reads,
			debug_views,
			at_output,
		});
	}

	/// The sink of an open window without a camera that has passes registered and none built yet.
	fn unbuilt_window_graph(&self) -> Option<SinkId> {
		self.open_windows().find_map(|(sink_id, _, handle)| {
			let unbuilt = !self.window_graphs.contains(&sink_id)
				&& !self.sink_cameras.iter().any(|(camera_sink, _)| *camera_sink == sink_id)
				&& self
					.registrations
					.iter()
					.any(|registration| registration.scope == SinkScope::Window(handle));
			unbuilt.then_some(sink_id)
		})
	}

	/// Builds the passes registered for each open window without a camera, which closes those windows' graphs.
	fn build_window_graphs(&mut self) {
		let mut built = false;
		while let Some(sink_id) = self.unbuilt_window_graph() {
			built = true;
			let window = self.sinks[sink_id]
				.as_ref()
				.expect("Only open windows build graphs.")
				.handle();
			let passes = (0..self.registrations.len())
				.filter(|index| self.registrations[*index].scope == SinkScope::Window(window))
				.collect::<SmallVec<[_; 4]>>();
			let mut registrations = std::mem::take(&mut self.registrations);
			// A window without a camera has no scene, so there is a `main` only after a pass that replaces it.
			let overlays = passes
				.iter()
				.map(|index| registrations[*index].passes().draws_over())
				.collect::<SmallVec<[_; 4]>>();
			let places = main_places(&overlays, false);
			for (index, place) in passes.into_iter().zip(places) {
				self.add_sink_pass(&mut registrations, sink_id, place, PlannedWork::Pass(index));
			}
			self.registrations = registrations;
			self.window_graphs.push(sink_id);
		}
		if built {
			self.declare_render_passes();
		}
	}

	/// Declares a `render.pass.<name>` parameter for every registration, so adapters such as the console list the
	/// passes this application has.
	fn declare_render_passes(&self) {
		declare_render_passes(
			&self.configuration,
			self.registrations
				.iter()
				.map(|registration| (registration.name, registration.state)),
		);
	}

	/// Instantiates all planned post-scene passes for a given sink.
	fn add_post_scene_render_passes_for_sink(&mut self, sink_id: SinkId) {
		let post_scene = self.post_scene_registrations();
		// Every pass builds through `self`, so the registrations and the plan step aside while they run.
		let mut registrations = std::mem::take(&mut self.registrations);
		let FusionCatalog::Planned(planned) = std::mem::replace(&mut self.fusions, FusionCatalog::Unread) else {
			unreachable!("Sinks initialize only after the fusion catalog is planned.");
		};
		// A render image leaves out the passes registered for windows alone, and its last remaining pass writes it.
		let image = matches!(self.sinks[sink_id], Some(SinkTarget::Image { .. }));
		let built = |entry: &&Planned| match entry {
			Planned::Pass(index) => !image || registrations[post_scene[*index]].scope == SinkScope::All,
			Planned::Run { .. } => true,
		};
		let works = planned
			.iter()
			.filter(built)
			.map(|entry| match entry {
				Planned::Pass(index) => PlannedWork::Pass(post_scene[*index]),
				Planned::Run { steps, pipeline } => PlannedWork::Run {
					steps: post_scene[steps.clone()].iter().copied().collect(),
					pipeline,
				},
			})
			.collect::<SmallVec<[_; 16]>>();
		let overlays = works
			.iter()
			.map(|work| match work {
				PlannedWork::Pass(index) => registrations[*index].passes().draws_over(),
				// A run writes one replacement for `main`.
				PlannedWork::Run { .. } => false,
			})
			.collect::<SmallVec<[_; 16]>>();
		// The scene pipelines built before this, so a `main` here is the scene's.
		let scene_main = self.render_targets.get("main", sink_id).is_some();
		let places = main_places(&overlays, scene_main);
		let terminal = places.contains(&MainPlace::Terminal);
		for (work, place) in works.into_iter().zip(places) {
			self.add_sink_pass(&mut registrations, sink_id, place, work);
		}
		self.registrations = registrations;
		self.fusions = FusionCatalog::Planned(planned);
		self.declare_render_passes();

		// A graph whose passes are all overlays, or that has none, has no terminal pass that can receive the final
		// target directly, so the scene's `main` is copied into it, under the overlays. A graph without a scene has no
		// scene color to copy, so presenting the acquired swapchain is the complete frame operation in that case.
		let final_target = self.final_target(sink_id);
		if !terminal && scene_main {
			let mut builder = self.sink_builder(sink_id, RenderNode::Presentation, MainPlace::Before);
			let source = builder.read_from("main");
			let copy = ImageBypassPass::new(&mut builder, source, final_target);
			builder.record_node();
			self.scene_presentation_copies.push((sink_id, copy));
		}

		// The debug views tile over everything, so their pass is the sink's last node.
		if self.debug_view_settings.enabled {
			let mut builder = self.sink_builder(sink_id, RenderNode::DebugView, MainPlace::Before);
			let pool = builder.debug_view_pool(ghi::AccessPolicies::READ);
			let present = DebugViewPresentPass::new(&mut builder, &pool, final_target.into());
			builder.record_node();
			self.debug_view_presents.push((sink_id, present));
		}
	}

	/// Waits as `wait` allows for the first event, then drains pending application and window events. Match
	/// [`ghi::window::Event::Window`] ids against the windows created with [`Self::create_window`].
	///
	/// Without a window there is no event queue, so the call returns at once whatever `wait` says.
	///
	/// Each resize also reaches the window's swapchain, since some surfaces, such as Wayland's, report no size of
	/// their own.
	pub fn poll_windows(&mut self, wait: ghi::window::Wait) -> impl Iterator<Item = ghi::window::Event> + '_ {
		let (context, sinks) = (&mut self.context, &self.sinks);
		self.app.iter_mut().flat_map(move |app| app.poll(wait)).inspect(move |event| {
			let &ghi::window::Event::Window {
				window,
				event: ghi::window::Events::Resize { width, height, .. },
			} = event
			else {
				return;
			};
			let swapchain = sinks.iter().find_map(|target| match target {
				Some(SinkTarget::Window {
					window: open, swapchain, ..
				}) if *open == window => Some(*swapchain),
				_ => None,
			});
			if let Some(swapchain) = swapchain {
				context.set_window_extent(swapchain, Extent::rectangle(width, height));
			}
		})
	}

	/// Opens an operating system dialog, such as a file or color picker, without waiting for the user. The main
	/// window owns it, so it opens attached to that window where the platform supports it.
	///
	/// Next, match [`ghi::window::AppEvents::Dialog`] events against the returned id. They reach the application with
	/// the other [`ghi::window::Event`]s on its message channel, and their [`ghi::window::DialogResult`] carries what
	/// the user chose.
	pub fn open_dialog(&mut self, dialog: &ghi::window::Dialog) -> Result<ghi::window::DialogId, ghi::window::DialogError> {
		// A renderer without windows connects here, as `create_window` does, since dialogs need the window system too.
		let app = match &mut self.app {
			Some(app) => app,
			app => app.insert(ghi::window::App::new("main_window").map_err(ghi::window::DialogError::Os)?),
		};
		let owner = self.main_window.and_then(|sink_id| match &self.sinks[sink_id] {
			Some(SinkTarget::Window { window, .. }) => Some(*window),
			_ => None,
		});
		app.open_dialog(dialog, owner)
	}

	/// Puts `text` on the system clipboard as plain text, so the user can paste it into another application. See
	/// [`ghi::window::App::set_clipboard_text`].
	pub fn set_clipboard_text(&mut self, text: &str) -> Result<(), String> {
		let Some(app) = &mut self.app else {
			return Err("The clipboard was not set. The renderer has no window yet, and it reaches the clipboard through the window system. Copy after the first window opens.".to_owned());
		};
		app.set_clipboard_text(text)
	}

	/// Returns the handle that interrupts a waiting [`Self::poll_windows`], once a window connected the renderer
	/// to the window system.
	pub(crate) fn app_waker(&self) -> Option<ghi::window::AppWaker> {
		self.app.as_ref().map(ghi::window::App::waker)
	}

	/// Makes a waiting [`Self::poll_windows`] return when a HID device connects or disconnects.
	pub(crate) fn wake_on_hid_changes(&mut self, monitor: &ghi::hid::Monitor) -> Result<(), String> {
		self.app.as_mut().map_or(Ok(()), |app| app.wake_on_hid_changes(monitor))
	}

	/// Gives the window created with `handle` the features `features`, applying the ones that change at runtime.
	///
	/// Only [`window::Features::CAPTURE_CURSOR`] changes on an open window; a change to another feature is logged and
	/// left out.
	pub fn set_window_features(&mut self, handle: Handle, features: window::Features) {
		let open = self.sink_of(handle).and_then(|sink_id| match &mut self.sinks[sink_id] {
			Some(SinkTarget::Window { window, features, .. }) => Some((window, features)),
			_ => None,
		});
		let Some((window, current)) = open else {
			log::warn!(
				"Window features were not changed. The most likely cause is a handle that names no open window, or a window the renderer hasn't opened yet. Send the change after the tick that created the window, with the handle its creation returned."
			);
			return;
		};
		let fixed = (features ^ *current) & !window::Features::CAPTURE_CURSOR;
		if !fixed.is_empty() {
			log::warn!(
				"Window features {fixed:?} were not changed. The most likely cause is that only cursor capture changes on an open window. Create the window with these features instead."
			);
		}
		let captures = features.contains(window::Features::CAPTURE_CURSOR);
		if captures != current.contains(window::Features::CAPTURE_CURSOR) {
			if let Some(app) = &mut self.app {
				app.set_cursor_capture(*window, captures);
			}
			current.set(window::Features::CAPTURE_CURSOR, captures);
		}
	}

	/// Makes the window created with `handle` show `pointer` while the pointer is over it. See
	/// [`ghi::window::App::set_pointer`].
	pub fn set_window_pointer(&mut self, handle: Handle, pointer: window::Pointer) {
		let open = self.sink_of(handle).and_then(|sink_id| match &self.sinks[sink_id] {
			Some(SinkTarget::Window { window, .. }) => Some(window),
			_ => None,
		});
		let (Some(window), Some(app)) = (open, &mut self.app) else {
			log::warn!(
				"The window's pointer shape was not changed. The most likely cause is a handle that names no open window, or a window the renderer hasn't opened yet. Send the change after the tick that created the window, with the handle its creation returned."
			);
			return;
		};
		app.set_pointer(*window, pointer);
	}

	/// Makes the window created with `handle` show `camera` from the next frame on, keeping the passes its sink built.
	///
	/// Only a window created with a camera shows a scene, so a window without one is left out with a warning.
	pub fn set_window_camera(&mut self, handle: Handle, camera: Handle) {
		let shown = self
			.sink_of(handle)
			.and_then(|sink_id| self.sink_cameras.iter_mut().find(|(sink, _)| *sink == sink_id));
		let Some((_, shown)) = shown else {
			log::warn!(
				"The window's camera was not changed. The most likely cause is a handle that names no open window, or a window created without a camera. Attach a camera to the window before creating it, and send the change after the tick that created it."
			);
			return;
		};
		*shown = camera;
		self.redraw_requested = true;
	}

	/// Caps the presentation rate by setting the minimum time between presented frames on every window,
	/// including windows created later. `None` removes the cap so frames present on every refresh.
	pub fn set_present_interval(&mut self, interval: Option<std::time::Duration>) {
		self.present_interval = interval;
		for target in self.sinks.iter().flatten() {
			if let SinkTarget::Window { swapchain, .. } = target {
				self.context.set_present_interval(*swapchain, interval);
			}
		}
	}

	/// Makes the renderer serve runs no one watches, such as an agent debugging the application.
	///
	/// Windows created afterwards open in the background ([`ghi::window::Features::BACKGROUND`]) and present without
	/// waiting for a display refresh, and every window renders each frame whether or not it can be seen. The caller
	/// paces frames, as the `unattended` application parameter does in
	/// [`crate::application::graphics::GraphicsApplication`].
	pub fn set_unattended(&mut self, unattended: bool) {
		self.unattended = unattended;
	}

	/// Returns the shortest refresh interval among the displays showing a window, when any platform reports one.
	///
	/// The fastest display decides so that no window handles its events later than its display could show them.
	pub fn refresh_interval(&self) -> Option<std::time::Duration> {
		let app = self.app.as_ref()?;
		self.open_windows()
			.filter_map(|(_, window, _)| app.refresh_interval(window))
			.min()
	}

	/// Reports whether every visible window can take an image without waiting for the display, so the tick can stop
	/// waiting for window events and call [`Self::acquire_swapchain_images`].
	///
	/// See [`ghi::context::Context::swapchain_ready`]. An unattended renderer is always ready, since its windows take
	/// images without a display.
	pub(crate) fn displays_ready(&self) -> bool {
		self.unattended
			|| self.sinks.iter().all(|target| match target {
				Some(SinkTarget::Window { window, swapchain, .. }) => {
					!self.app.as_ref().is_some_and(|app| app.is_visible(*window)) || self.context.swapchain_ready(*swapchain)
				}
				_ => true,
			})
	}

	/// Acquires the swapchain image of every window for the next frame and returns the primary window's display time,
	/// when the backend reports one: the time this frame will be shown when the backend schedules ahead, as Metal
	/// does, otherwise the time its most recent frame was shown.
	///
	/// Call this at the start of a tick so simulation runs after the presentation engine releases an image. Each
	/// frame waits for the display once: on Metal in the window event wait before this call, until
	/// [`Self::displays_ready`], and elsewhere here, when the backend's acquisition waits for a free image. Either way
	/// the wait ends the previous tick or starts this one, so simulation still follows the display.
	/// A window that cannot be seen is skipped without acquiring: whatever the frame presented there would never be
	/// shown, and on Metal the presentation engine stops pacing the loop for it. Pass the sinks a screenshot captures
	/// as `captured` so those windows render anyway. An unattended renderer ([`Self::set_unattended`]) renders every
	/// window.
	///
	/// The call is idempotent for one frame: windows already decided are skipped, so [`Self::prepare`] can call it
	/// to pick up windows adopted later in the tick, hidden windows a screenshot captures, or to acquire when nothing
	/// was hoisted.
	pub(crate) fn acquire_swapchain_images(&mut self, captured: impl Fn(SinkId) -> bool) -> Option<std::time::Instant> {
		if self.acquisitions.0 != self.started_frame_count {
			self.acquisitions = (self.started_frame_count, SmallVec::new());
		}
		// A window still needs deciding when this frame has no entry for it, or when it is hidden and a screenshot
		// captures it.
		let undecided = |acquisitions: &[SinkFrame], index: SinkId| match acquisitions.get(index) {
			None => true,
			Some(sink_frame) => matches!(sink_frame, SinkFrame::Hidden) && captured(index),
		};
		if !(0..self.sinks.len()).any(|index| undecided(&self.acquisitions.1, index)) {
			return None;
		}

		let _span = debug_span!(
			"Renderer::acquire_swapchains",
			frame = self.started_frame_count,
			sinks = self.sinks.len()
		)
		.entered();

		let frame = ghi::queue::FrameRequest::new(self.started_frame_count, self.render_finished_synchronizer);
		let mut present_time = None;
		let app = self.app.as_ref();

		for (index, target) in self.sinks.iter_mut().enumerate() {
			if !undecided(&self.acquisitions.1, index) {
				continue;
			}
			// Only a window can be hidden, so a removed sink or a render image is decided the first time it's seen.
			let (window, swapchain, warned) = match target {
				None => {
					self.acquisitions.1.push(SinkFrame::Removed);
					continue;
				}
				Some(SinkTarget::Image { image, extent, .. }) => {
					self.acquisitions.1.push(SinkFrame::Image(*extent, *image));
					continue;
				}
				Some(SinkTarget::Window {
					window,
					swapchain,
					warned,
					..
				}) => (window, swapchain, warned),
			};
			if !self.unattended && !captured(index) && !app.is_some_and(|app| app.is_visible(*window)) {
				self.acquisitions.1.push(SinkFrame::Hidden);
				continue;
			}

			let acquisition = self.context.acquire_swapchain_image(frame, *swapchain);
			if index == 0 {
				present_time = acquisition
					.as_ref()
					.and_then(|acquisition| acquisition.target_present_time().or_else(|| acquisition.present_time()));
			}

			// A window without a usable image skips the frame. Say so once, and again only after it recovers.
			let extent = acquisition.as_ref().map(|acquisition| acquisition.extent());
			let problem = match extent {
				None => Some("No swapchain image was available"),
				Some(extent) if extent.width() == 0 || extent.height() == 0 => Some("The swapchain extent is too small"),
				Some(extent) if extent.width() >= 65535 || extent.height() >= 65535 => {
					Some("The swapchain extent is too large, since the renderer only supports 16-bit dimensions")
				}
				Some(_) => None,
			};
			crate::rendering::warn_once(warned, problem.is_some(), || {
				format!(
					"{} for window {swapchain:?} ({extent:?}). Rendering will be skipped.",
					problem.unwrap_or_default()
				)
			});

			let sink_frame = match acquisition.filter(|_| problem.is_none()) {
				Some(acquisition) => SinkFrame::Acquired(acquisition.present_key(), acquisition.extent(), *swapchain),
				None => SinkFrame::Unusable,
			};
			match self.acquisitions.1.get_mut(index) {
				Some(decided) => *decided = sink_frame,
				None => self.acquisitions.1.push(sink_frame),
			}
		}

		present_time
	}

	/// Returns the player's motion preference, as [`Self::set_motion`] last set it.
	pub fn motion(&self) -> crate::accessibility::MotionPreference {
		self.motion
	}

	/// Sets the player's motion preference, which every sink carries to its passes from the next frame on, so passes
	/// that draw motion, such as motion blur, draw none while it is reduced.
	pub fn set_motion(&mut self, motion: crate::accessibility::MotionPreference) {
		if self.motion != motion {
			self.motion = motion;
			self.redraw_requested = true;
		}
	}

	/// Asks for a new frame when the application changed something the renderer cannot observe itself.
	///
	/// This only matters with the `render-on-demand` application parameter: scene pipelines do not report their
	/// changes, so call this after moving cameras or scene objects. UI renders and window events request frames
	/// on their own.
	pub fn request_redraw(&mut self) {
		self.redraw_requested = true;
	}

	/// Reports whether the next frame would show something the last presented frame does not.
	///
	/// Render passes adopt their pending inputs while answering, so call this after the tick published them.
	pub(crate) fn needs_frame(&mut self) -> bool {
		let mut needs_frame =
			self.redraw_requested || !self.pending_sink_initializations.is_empty() || self.unbuilt_window_graph().is_some();
		// Ask every registration so each adopts its inputs this tick, even when an earlier one already answered.
		for registration in &mut self.registrations {
			needs_frame |= registration.installed().needs_frame();
		}
		needs_frame
	}

	/// Starts scene resource requests before window setup or frame preparation borrows the context.
	pub(crate) fn update(&mut self, transforms: &ShownTransforms) {
		for (_, pipeline_manager) in &mut self.pipeline_managers {
			pipeline_manager.update(transforms);
		}
	}

	/// Returns whether any window holds an acquired swapchain image for the current frame.
	///
	/// When this is `false` after [`Self::acquire_swapchain_images`], nothing blocked on the presentation engine
	/// and the caller must pace the tick itself. Windows that cannot be seen hold no image.
	pub(crate) fn presents_this_frame(&self) -> bool {
		self.frame_acquired(self.started_frame_count)
	}

	/// Returns whether any window held an acquired swapchain image in `frame`, the current or the last submitted one.
	///
	/// A frame that acquired nothing rendered no sink, so its pass and scene counters measured nothing.
	pub(crate) fn frame_acquired(&self, frame: u64) -> bool {
		self.acquisitions.0 == frame
			&& self
				.acquisitions
				.1
				.iter()
				.any(|sink_frame| matches!(sink_frame, SinkFrame::Acquired(..)))
	}

	/// Prepares a frame by invoking the configured render passes.
	///
	/// The renderer skips every window that has no usable swapchain image, such as a zero-sized one, and every
	/// window that cannot be seen and no screenshot captures. It returns the frame that
	/// every screenshot readback comes from, and one readback result per request
	/// in request order. `frame_interval` is the time since the previous frame; every sink carries it to the
	/// passes that simulate exposure over time.
	// Keep the frame transaction contiguous so recording, presentation, and screenshot transfers stay ordered.
	// Swapchain acquisition happens before this call (see `acquire_swapchain_images`) so the tick can pace on it, or
	// on the drawable wait at submission where the backend defers it.
	#[allow(clippy::excessive_nesting, clippy::too_many_lines)]
	pub(crate) fn prepare(
		&'_ mut self,
		transforms: &ShownTransforms,
		frame_allocator: &bumpalo::Bump,
		alley: &mut crate::core::alley::Alley,
		screenshot_requests: &[(usize, &crate::inspector::screenshot::ScreenshotCapture)],
		time: crate::time::MediaTime,
		frame_interval: crate::time::MediaTime,
	) -> (u64, Vec<Result<ghi::TextureReadback, RendererScreenshotError>>) {
		let _span = debug_span!(
			"Renderer::prepare",
			frame = self.started_frame_count,
			sinks = self.sinks.len()
		)
		.entered();

		let Some(_) = self.sinks.iter().flatten().next() else {
			log::debug!("No window or render image is open. Skipping rendering!");
			let screenshots = screenshot_requests
				.iter()
				.map(|_| Err(RendererScreenshotError::SinkNotFound))
				.collect();
			return (self.started_frame_count, screenshots);
		};
		// Acquire here when nothing was hoisted to the start of the tick, for windows adopted since, and for hidden
		// windows a screenshot captures.
		self.acquire_swapchain_images(|sink| screenshot_requests.iter().any(|(captured, _)| *captured == sink));
		self.redraw_requested = false;

		// Sinks wait for the fusion catalog, and deferred sinks also for the first frame to be presented.
		if (self.started_frame_count > 0 || !self.defer_first_frame_sink_setup)
			&& !self.pending_sink_initializations.is_empty()
			&& self.fusions_planned()
		{
			for sink_id in std::mem::take(&mut self.pending_sink_initializations) {
				self.initialize_scene_sink(sink_id);
			}
		}
		self.build_window_graphs();
		// Queued render-pass configuration applies after passes exist and before they prepare frame work.
		let configuration = &self.configuration;
		let pending_configuration = &mut self.pending_configuration;
		let registrations = &mut self.registrations;
		apply_render_pass_configuration(configuration, pending_configuration, |name, state| {
			set_render_pass_state(registration_states(registrations), name, state)
		});
		if apply_debug_view_configuration(
			&self.debug_view_configuration,
			&mut self.pending_debug_view_configuration,
			&mut self.debug_views,
		) {
			self.redraw_requested = true;
		}
		// Slots a view holds get the sink extent this frame; the rest stay tiny.
		let debug_slots_taken = self.debug_views.taken();
		self.render_targets.set_debug_slots_taken(debug_slots_taken);

		// Resolve names outside command recording so the hot path only compares pass IDs and transfers handles.
		let screenshot_captures = screenshot_requests
			.iter()
			.map(|(sink, capture)| self.resolve_screenshot_capture(*sink, capture))
			.collect::<Vec<_>>();

		let mut queue = self.context.queue(self.graphics_queue_handle);
		let frame =
			ghi::queue::FrameRequest::new_in(self.started_frame_count, self.render_finished_synchronizer, &frame_allocator);

		self.started_frame_count += 1;

		let swapchains = &self.acquisitions.1;
		let submitted_frame = self.started_frame_count - 1;
		let mut screenshot_transfers = (0..screenshot_captures.len()).map(|_| None).collect::<Vec<_>>();
		let scene_counters = &self.scene_counters;
		let frame_counter = self.frame_counter;
		let mut completed_frame = None;

		{
			let _span = debug_span!("Renderer::queue_execute").entered();
			queue.execute(Some(frame), &[], self.render_finished_synchronizer, |execution| {
				completed_frame = execution.completed_frame();
				#[cfg(debug_assertions)]
				if let Some(resource_updates) = &self.resource_updates {
					while let Some(update) = resource_updates.read() {
						self.pipeline_compilation_client.resource_updated(update.id());
					}
				}
				self.pipeline_compilation_manager.publish(execution.frame().expect(
					"Frame is required to publish compiled pipelines. The most likely cause is that Renderer::prepare called Queue::execute without a frame request.",
				));

				let prepare_frame_work = debug_span!("Renderer::prepare_frame_work").entered();
				let frame = execution.frame().expect(
					"Frame is required to prepare renderer frame work. The most likely cause is that Renderer::render called Queue::execute without a frame request.",
				);
				let mut sinks: SmallVec<[Sink; 16]> = SmallVec::new();

				// Every feature brings its shared state up to date before any of its passes answers or prepares, so a
				// setting such as the render scale reaches this frame's sinks.
				for registration in &mut self.registrations {
					registration.installed().update();
				}

				{
					let _span = debug_span!("Renderer::build_sinks", cameras = self.cameras.len()).entered();
					for (sink_id, camera_handle) in self.sink_cameras.iter() {
						let (SinkFrame::Acquired(_, extent, _) | SinkFrame::Image(extent, _)) = swapchains[*sink_id] else {
							continue;
						};
						let Some((_, camera)) = self.cameras.iter().find(|(handle, _)| handle == camera_handle) else {
							continue;
						};

						let transform = transforms.get(*camera_handle).cloned().unwrap_or_default();
						let view = make_perspective_view_from_camera(camera, &transform, extent);
						let sink = Sink::new(view, extent, *sink_id)
							.with_camera(*camera_handle)
							.with_scene(scene_of(&self.sinks, *sink_id))
							.with_exposure_scale(camera.exposure_scale())
							.with_detail_level(camera.detail_level())
							.with_lens(camera.f_number(), camera.focus_distance(), camera.sensor_height())
							.with_frame_interval(frame_interval)
							.with_motion(self.motion);
						let lighting = &self.scenes[scene_of(&self.sinks, *sink_id).index()];
						let sink = match &lighting.sun_light {
							Some((handle, light)) => {
								sink.with_sun(Sun::new(light, &transforms.get(*handle).cloned().unwrap_or_default()))
							}
							None => sink,
						};
						let sink = match lighting.fog {
							Some(fog) => sink.with_fog(fog),
							None => sink,
						};
						let registrations = &mut self.registrations;
						let sink_nodes = self.pass_nodes.iter().filter(|node| node.sink == *sink_id);
						let samples_lens = sink_nodes.clone().any(|node| node.samples_lens(registrations));
						let jitters_view = sink_nodes.clone().any(|node| node.jitters_view(registrations));
						// Scene pipelines write only the targets an enabled pass reads.
						let read_targets = sink_nodes
							.clone()
							.filter(|node| node.state(registrations) == RenderPassState::Enabled)
							.fold(crate::rendering::TargetSet::default(), |targets, node| targets.union(node.reads));
						let sink = sink.with_read_targets(read_targets);
						// The scene renders at the scale the enabled upscaling pass asks for; any other pass asks for 1.
						let render_scale = sink_nodes
							.clone()
							.map(|node| node.render_scale(registrations))
							.fold(1.0f32, f32::min);
						let sink = sink.with_render_scale(render_scale);
						let sink = if samples_lens {
							sink.with_lens_sample(lens_sample(self.started_frame_count))
						} else {
							sink
						};
						sinks.push(if jitters_view {
							sink.with_jitter(jitter_offset(self.started_frame_count, render_scale))
						} else {
							sink
						});
					}
					// A window with its own graph shows no scene, so its sink has no camera and an identity view.
					for sink_id in &self.window_graphs {
						let SinkFrame::Acquired(_present_key, extent, _swapchain) = swapchains[*sink_id] else {
							continue;
						};
						sinks.push(
							Sink::new(View::identity(), extent, *sink_id)
								.with_frame_interval(frame_interval)
								.with_motion(self.motion),
						);
					}
				}
				// Each scene's sinks lie next to each other, in creation order, so every scene pipeline takes the run of
				// its own scene, whose first sink leads its frame-wide work.
				sinks.sort_by_key(|sink| scene_of(&self.sinks, sink.index()).index());

				{
					let _span = debug_span!("Renderer::resize_render_targets", sinks = sinks.len()).entered();
					for sink in &sinks {
						// A render image follows its sink's extent like the sink's own targets do.
						if let SinkFrame::Image(_, image) = swapchains[sink.index()] {
							frame.resize_image(image.into(), sink.output_extent());
						}
						// Targets before the upscaling pass follow the render extent, and the rest the output extent.
						let extents = SinkExtents {
							render: sink.render_extent(),
							output: sink.output_extent(),
						};
						// Resize the sink's images to their extent, divided for reduced-resolution targets.
						for (image, extent) in self.render_targets.get_images_for_sink(sink.index(), extents) {
							frame.resize_image(image, extent);
						}
					}
				}

				// Each manager's sinks, the run of its scene, and its commands, indexed by manager, with the sink every
				// command records for.
				type ManagerCommands<'a> = (std::ops::Range<usize>, SmallVec<[(SinkId, RenderPassReturn<'a>); 16]>);
				let pipeline_manager_commands: SmallVec<[ManagerCommands<'_>; 16]> = {
					let _span = debug_span!("Renderer::prepare_pipeline_managers").entered();
					let registrations = &mut self.registrations;
					let background = self.scene_background;
					let (debug_view_registry, scene_debug_views) = (&self.debug_views, &self.scene_debug_views);
					let sinks = &mut sinks;
					self.pipeline_managers
						.iter_mut()
						.enumerate()
						.map(|(pipeline_manager_id, (scene, pipeline_manager))| {
							let start = sinks.partition_point(|sink| scene_of(&self.sinks, sink.index()).index() < scene.index());
							let end = sinks.partition_point(|sink| scene_of(&self.sinks, sink.index()).index() <= scene.index());
							// The scene background is lent to the manager, which prepares it where it chooses.
							let (installed, state) = match background {
								Some(index) => {
									let registration = &mut registrations[index];
									let state = registration.state;
									(Some(registration.background()), state)
								}
								None => (None, RenderPassState::Enabled),
							};
							let mut scene_background = SceneBackgroundFrame::new(installed, state, pipeline_manager_id, *scene);
							// The manager's sinks carry the debug views it declared, in place, since each pass node sets its own.
							let shown = debug_view_registry.shown(scene_debug_views.get(pipeline_manager_id).copied().flatten());
							for sink in &mut sinks[start..end] {
								*sink = sink.with_debug_views(shown);
							}
							let commands = pipeline_manager.prepare(
								frame,
								&sinks[start..end],
								frame_allocator,
								transforms,
								alley,
								time,
								&mut scene_background,
							);
							(start..end, commands)
						})
						.collect()
				};

				// A list of render pass commands with their pass and sink indices and the counter that times them.
				let render_pass_commands: SmallVec<
					[([Option<RenderPassReturn>; 2], PassNodeId, SinkId, Option<GpuCounter>); 64],
				> = {
					let _span = debug_span!("Renderer::prepare_render_passes").entered();
					let registrations = &mut self.registrations;
					let debug_view_registry = &self.debug_views;
					self.pass_nodes
						.iter_mut()
						.enumerate()
						.filter_map(|(node_id, node)| {
							let sink = sinks.iter().find(|sink| sink.index() == node.sink)?;
							let sink = if node.at_output { sink.at_output() } else { *sink }
								.with_debug_views(debug_view_registry.shown(node.debug_views));
							let counter = node.counter;
							Some((
								node.prepare(registrations, frame, &sink, frame_allocator),
								node_id,
								sink.index(),
								counter,
							))
						})
						.collect()
				};

				// The render targets each node uses first, per sink, so recording can give them new contents.
				let mut first_uses = SmallVec::<[(SinkId, FirstUse); 64]>::new();
				{
					let _span = debug_span!("Renderer::place_render_targets", sinks = sinks.len()).entered();
					for sink in &sinks {
						let extents = SinkExtents {
							render: sink.render_extent(),
							output: sink.output_extent(),
						};
						// The nodes that prepared no command leave `main` as it is, so their replacements for it are
						// placed as the `main` before them. This is why placement waits for every pass to prepare.
						let idle = render_pass_commands
							.iter()
							.filter(|(commands, _, of, _)| *of == sink.index() && commands.iter().all(Option::is_none))
							.map(|(_, node, ..)| self.pass_nodes[*node].node)
							.collect::<SmallVec<[RenderNode; 16]>>();
						// Targets whose uses do not overlap share memory, placed again only when an extent, a use, or
						// the idle nodes change.
						if let Some(plan) = self.render_targets.plan(sink.index(), extents, &idle) {
							frame.place_image_group(plan.group, &plan.members);
							first_uses.extend(plan.first_uses.into_iter().map(|first_use| (sink.index(), first_use)));
						}
					}
				}

				let scene_presentation_commands: SmallVec<[(Option<RenderPassReturn>, SinkId); 16]> = self
					.scene_presentation_copies
					.iter_mut()
					.filter_map(|(sink_id, copy)| {
						let sink = sinks.iter().find(|sink| sink.index() == *sink_id)?;
						Some((copy.prepare(frame, sink, frame_allocator), *sink_id))
					})
					.collect();

				// The tiling of the shown debug views, recorded last, only while a view is shown.
				let debug_view_commands: SmallVec<[(Option<RenderPassReturn>, SinkId); 16]> = if debug_slots_taken == 0 {
					SmallVec::new()
				} else {
					self.debug_view_presents
						.iter_mut()
						.filter_map(|(sink_id, present)| {
							let sink = sinks.iter().find(|sink| sink.index() == *sink_id)?;
							Some((present.prepare(frame, sink, frame_allocator, debug_slots_taken), *sink_id))
						})
						.collect()
				};

				let present_keys = swapchains
					.iter()
					.filter_map(|sink_frame| match sink_frame {
						SinkFrame::Acquired(present_key, ..) => Some(*present_key),
						SinkFrame::Hidden | SinkFrame::Unusable | SinkFrame::Removed | SinkFrame::Image(..) => None,
					})
					.collect::<SmallVec<[ghi::PresentKey; 16]>>();
				drop(prepare_frame_work);

				execution.record_with_present_keys(self.render_command_buffer, &present_keys, |command_buffer_recording| {
					let _span = debug_span!("Renderer::record_commands", sinks = sinks.len()).entered();
					if let Some((counter, _)) = frame_counter {
						command_buffer_recording.start_counter(counter);
					}
					{
						let _span = debug_span!("Renderer::record_pipeline_manager_commands").entered();
						for (pipeline_manager_id, (scene_sinks, commands)) in pipeline_manager_commands.iter().enumerate() {
							for sink in &sinks[scene_sinks.clone()] {
								let command = commands
									.iter()
									.find_map(|(sink_id, command)| (*sink_id == sink.index()).then_some(*command));
								let node = RenderNode::Scene(pipeline_manager_id);
								let counter = scene_counters.iter().find_map(|(manager, sink_id, counter)| {
									(*manager == pipeline_manager_id && *sink_id == sink.index()).then_some(counter.0)
								});
								record_node(command_buffer_recording, &first_uses, sink.index(), node, &[command], counter);
							}
						}
					}

					for (request_index, capture) in screenshot_captures.iter().enumerate() {
						let transfer = match capture {
							Ok(ResolvedScreenshotCapture::AfterScene { target }) => command_buffer_recording.transfer_texture(*target),
							// No pass writes the previous frame's copy, so any point in the frame reads the same data.
							Ok(ResolvedScreenshotCapture::PreviousFrame { target }) => {
								command_buffer_recording.transfer_texture_with_frame((*target).into(), -1)
							}
							_ => continue,
						};
						screenshot_transfers[request_index] = Some(transfer.map_err(RendererScreenshotError::Transfer));
					}

					{
						let _span = debug_span!("Renderer::record_render_pass_commands").entered();
						// Render images finish before any window records its passes, so a window's pass that reads one
						// reads this frame's picture.
						let in_phase = |sink: SinkId, images: bool| matches!(swapchains[sink], SinkFrame::Image(..)) == images;
						// Overlays that draw over a sink's final target record after the node or copy that fills it.
						for (images, over) in [(true, false), (true, true), (false, false), (false, true)] {
							for (commands, render_pass_id, sink, counter) in render_pass_commands
								.iter()
								.filter(|(_, node, sink, _)| in_phase(*sink, images) && self.pass_nodes[*node].node.is_over() == over)
							{
								let node = self.pass_nodes[*render_pass_id].node;
								let counter = counter.as_ref().map(|(counter, _)| *counter);
								record_node(command_buffer_recording, &first_uses, *sink, node, commands, counter);
								for (request_index, capture) in screenshot_captures.iter().enumerate() {
									if let Ok(ResolvedScreenshotCapture::AfterPass { pass, target }) = capture
										&& pass == render_pass_id
									{
										screenshot_transfers[request_index] = Some(
											command_buffer_recording
												.transfer_texture(*target)
												.map_err(RendererScreenshotError::Transfer),
										);
									}
								}
							}
							if !over {
								let copies = scene_presentation_commands.iter().filter(|(_, sink)| in_phase(*sink, images));
								for (command, sink) in copies {
									let node = RenderNode::Presentation;
									record_node(command_buffer_recording, &first_uses, *sink, node, &[*command], None);
								}
								continue;
							}
							for (command, sink) in debug_view_commands.iter().filter(|(_, sink)| in_phase(*sink, images)) {
								let node = RenderNode::DebugView;
								record_node(command_buffer_recording, &first_uses, *sink, node, &[*command], None);
							}
						}
					}

					// Final captures remain after every pass; duplicate requests receive independent transfer handles.
					for (request_index, capture) in screenshot_captures.iter().enumerate() {
						screenshot_transfers[request_index] = Some(match capture {
							Err(error) => Err(*error),
							Ok(ResolvedScreenshotCapture::FinalSwapchain { sink }) => match swapchains.get(*sink) {
								None => Err(RendererScreenshotError::SinkNotFound),
								Some(SinkFrame::Hidden | SinkFrame::Unusable | SinkFrame::Removed) => {
									Err(RendererScreenshotError::SinkUnavailable)
								}
								Some(SinkFrame::Acquired(_present_key, _extent, swapchain)) => command_buffer_recording
									.transfer_texture(ghi::ImageOrSwapchain::Swapchain(*swapchain))
									.map_err(RendererScreenshotError::Transfer),
								Some(SinkFrame::Image(_, image)) => command_buffer_recording
									.transfer_texture((*image).into())
									.map_err(RendererScreenshotError::Transfer),
							},
							Ok(_) => continue,
						});
					}
					if let Some((counter, _)) = frame_counter {
						command_buffer_recording.end_counter(counter);
					}
				});

				present_keys
			});
		}
		if let Some(completed_frame) = completed_frame {
			self.publish_gpu_metrics(completed_frame.frame_index());
		}

		if screenshot_transfers.iter().any(|transfer| matches!(transfer, Some(Ok(_)))) {
			self.context.wait_for_synchronizer(self.render_finished_synchronizer);
		}

		let screenshots = screenshot_transfers
			.into_iter()
			.map(|transfer| {
				let handle = transfer.unwrap_or(Err(RendererScreenshotError::SinkUnavailable))?;
				self.context.get_image_data(handle).map_err(RendererScreenshotError::Transfer)
			})
			.collect();
		(submitted_frame, screenshots)
	}

	/// Resolves a screenshot destination against immutable sink-local pass metadata.
	fn resolve_screenshot_capture(
		&self,
		sink: usize,
		capture: &crate::inspector::screenshot::ScreenshotCapture,
	) -> Result<ResolvedScreenshotCapture, RendererScreenshotError> {
		use crate::inspector::screenshot::ScreenshotCapture;
		if self.sinks.get(sink).is_none_or(Option::is_none) {
			return Err(RendererScreenshotError::SinkNotFound);
		}
		let (pass, target) = match capture {
			ScreenshotCapture::FinalSwapchain => return Ok(ResolvedScreenshotCapture::FinalSwapchain { sink }),
			ScreenshotCapture::SceneTarget { target } => {
				// A target the scene does not use gets its contents later, after its memory served other targets.
				let image = self
					.render_targets
					.get(target, sink)
					.filter(|_| self.render_targets.holds_scene_output(target, sink))
					.map(|(image, _)| image)
					.ok_or(RendererScreenshotError::TargetNotWritten)?;
				return Ok(ResolvedScreenshotCapture::AfterScene { target: image.into() });
			}
			ScreenshotCapture::PreviousSceneTarget { target } => {
				return match self.render_targets.history(target, sink) {
					Some(image) => Ok(ResolvedScreenshotCapture::PreviousFrame { target: image }),
					None if self.render_targets.get(target, sink).is_some() => Err(RendererScreenshotError::TargetHasNoHistory),
					None => Err(RendererScreenshotError::TargetNotWritten),
				};
			}
			ScreenshotCapture::AfterPass { pass, target } => (pass, target),
		};
		let mut matches = self
			.pass_nodes
			.iter()
			.enumerate()
			.filter(|(_, node)| node.sink == sink && node.name(&self.registrations) == pass);
		let Some((pass_id, render_pass)) = matches.next() else {
			return Err(RendererScreenshotError::PassNotFound);
		};
		if matches.next().is_some() {
			return Err(RendererScreenshotError::PassAmbiguous);
		}
		let target = render_pass
			.writable_targets
			.iter()
			.rev()
			.find_map(|(name, image)| (name == target).then_some(*image))
			.ok_or(RendererScreenshotError::TargetNotWritten)?;
		Ok(ResolvedScreenshotCapture::AfterPass { pass: pass_id, target })
	}

	/// Returns the queue frames record on, where setup work records its one-time uploads.
	pub fn graphics_queue(&self) -> ghi::QueueHandle {
		self.graphics_queue_handle
	}

	/// Borrows the render thread's GHI context for setup work that must run before the first frame.
	pub fn context_mut(&mut self) -> &mut ghi::implementation::Context {
		&mut self.context
	}

	/// Returns a client for requesting renderer-owned asynchronous pipelines.
	pub fn pipeline_manager_client(&self) -> crate::rendering::PipelineManagerClient {
		self.pipeline_compilation_client.clone()
	}

	/// Closes the window created with `handle`: destroys its swapchain, closes its native window, and stops rendering
	/// its sink. Other windows keep their sinks. Removing a window that isn't open does nothing.
	///
	/// The GPU waits for the frames in flight first. The sink's passes drop, and its render targets stay allocated until
	/// the renderer drops.
	pub fn remove_window(&mut self, handle: Handle) {
		let Some(sink_id) = self.sink_of(handle) else {
			return;
		};
		let Some(SinkTarget::Window { window, swapchain, .. }) =
			self.sinks[sink_id].take_if(|target| matches!(target, SinkTarget::Window { .. }))
		else {
			return;
		};
		self.forget_sink(sink_id);
		// The swapchain goes before its window, since destroying its surface may still talk to the window.
		self.context.destroy_swapchain(swapchain);
		if let Some(app) = &mut self.app {
			app.destroy_window(window);
		}
	}

	/// Stops rendering the render image created with `handle`. Removing a render image that isn't open does nothing.
	///
	/// The sink's passes drop, and the image and the sink's render targets stay allocated until the renderer drops.
	pub fn remove_render_image(&mut self, handle: Handle) {
		if let Some(sink_id) = self.sink_of(handle)
			&& self.sinks[sink_id]
				.take_if(|target| matches!(target, SinkTarget::Image { .. }))
				.is_some()
		{
			self.forget_sink(sink_id);
		}
	}

	/// Resizes the render image created with `handle` to `extent` pixels from the next frame on.
	///
	/// A handle that names no open render image is logged and left out.
	pub fn set_render_image_extent(&mut self, handle: Handle, extent: Extent) {
		let Some(sink_id) = self.sink_of(handle) else {
			log::warn!(
				"The render image was not resized. The most likely cause is a handle that names no open render image, or one the renderer hasn't opened yet. Send the change after the renderer answered with `RenderImageReady`."
			);
			return;
		};
		let Some(SinkTarget::Image { extent: current, .. }) = &mut self.sinks[sink_id] else {
			log::warn!(
				"The render image was not resized. The most likely cause is a handle that names no open render image, or one the renderer hasn't opened yet. Send the change after the renderer answered with `RenderImageReady`."
			);
			return;
		};
		*current = extent;
		// A frame this tick already decided renders at the new extent too.
		if let Some(SinkFrame::Image(decided, _)) = self.acquisitions.1.get_mut(sink_id) {
			*decided = extent;
		}
		self.redraw_requested = true;
	}

	/// Forgets every reference to the sink `sink_id`, whose target was taken out, so no frame
	/// renders it again.
	fn forget_sink(&mut self, sink_id: SinkId) {
		// A sink decided at the start of this tick must not be rendered or presented.
		if let Some(sink_frame) = self.acquisitions.1.get_mut(sink_id) {
			*sink_frame = SinkFrame::Removed;
		}
		self.sink_cameras.retain(|(sink, _)| *sink != sink_id);
		self.window_graphs.retain(|sink| *sink != sink_id);
		self.pending_sink_initializations.retain(|sink| *sink != sink_id);
		// Every feature drops what it built for the sink, and the window's own registrations reach nothing now.
		for registration in &mut self.registrations {
			registration.installed().remove_sink(sink_id);
		}
		self.redraw_requested = true;
	}

	/// The handle of the open window whose native window is `id`, which window events name.
	pub fn window_handle(&self, id: ghi::window::WindowId) -> Option<Handle> {
		self.open_windows()
			.find_map(|(_, window, handle)| (window == id).then_some(handle))
	}

	/// The native window id of the open window created with `handle`, which its window events carry.
	pub fn window_id(&self, handle: Handle) -> Option<ghi::window::WindowId> {
		self.open_windows()
			.find_map(|(_, window, open)| (open == handle).then_some(window))
	}

	/// The handle of the main window: the first one opened, while it's open.
	pub fn main_window(&self) -> Option<Handle> {
		self.sinks.get(self.main_window?)?.as_ref().map(SinkTarget::handle)
	}

	/// The sink of the open window or render image created with `handle`.
	fn sink_of(&self, handle: Handle) -> Option<SinkId> {
		self.sinks
			.iter()
			.position(|target| target.as_ref().is_some_and(|target| target.handle() == handle))
	}

	/// Every open window with its sink.
	fn open_windows(&self) -> impl Iterator<Item = (SinkId, ghi::window::WindowId, Handle)> {
		self.sinks
			.iter()
			.enumerate()
			.filter_map(|(sink_id, target)| match target.as_ref()? {
				SinkTarget::Window { window, handle, .. } => Some((sink_id, *window, *handle)),
				SinkTarget::Image { .. } => None,
			})
	}

	/// The final target of the open sink `sink_id`, which its terminal pass writes: its window's swapchain or its render
	/// image.
	fn final_target(&self, sink_id: SinkId) -> MainRenderTarget {
		match self.sinks[sink_id].as_ref().expect("Only open sinks build passes.") {
			SinkTarget::Window { swapchain, .. } => {
				MainRenderTarget::new(ghi::ImageOrSwapchain::Swapchain(*swapchain), self.context.swapchain_format())
			}
			SinkTarget::Image { image, .. } => MainRenderTarget::new((*image).into(), self.context.swapchain_format()),
		}
	}

	/// Creates the swapchain and sink state for a window created with `handle`.
	///
	/// Next, create a camera with [`Self::create_camera`] and associate it with the
	/// sink through the application or world integration. Change its features later with
	/// [`Self::set_window_features`].
	pub fn create_window(&mut self, handle: Handle, window: Window) {
		let requested_features = window.features();
		let name = window.name();
		let extent = window.extent();
		let camera = window.camera();

		let mut features = ghi::window::Features::empty();
		features.set(
			ghi::window::Features::DECORATIONS,
			window.features().contains(window::Features::DECORATIONS),
		);
		features.set(
			ghi::window::Features::CAPTURE_CURSOR,
			window.features().contains(window::Features::CAPTURE_CURSOR),
		);
		// An unseen window has no display refresh to wait for, so it presents immediately and the loop paces frames.
		let presentation_mode = if self.unattended {
			features |= ghi::window::Features::BACKGROUND;
			ghi::PresentationModes::Inmediate
		} else {
			ghi::PresentationModes::FIFO
		};

		// Connect on first use so headless renderers never touch the windowing system.
		let app = match &mut self.app {
			Some(app) => Ok(app),
			app => ghi::window::App::new("main_window").map(|connected| app.insert(connected)),
		};
		let window = app.and_then(|app| {
			let window = app.create_window(name, extent, features)?;
			// The window may open at another size than asked, such as on a display of another scale.
			let handles = app.os_handles(window).ok_or("the window closed as it opened")?;
			Ok((window, handles, app.extent(window).unwrap_or(extent)))
		});

		match window {
			Ok((window, handles, window_extent)) => {
				let swapchain_handle = self.context.bind_to_window(
					&handles,
					presentation_mode,
					self.swapchain_images,
					window_extent,
					ghi::Uses::RenderTarget | ghi::Uses::Storage | ghi::Uses::TransferSource,
				);
				if self.present_interval.is_some() {
					self.context.set_present_interval(swapchain_handle, self.present_interval);
				}

				let sink_id = self.sinks.len();
				self.sinks.push(Some(SinkTarget::Window {
					scene: SceneId::WORLD,
					window,
					swapchain: swapchain_handle,
					warned: false,
					handle,
					features: requested_features,
				}));
				self.main_window.get_or_insert(sink_id);
				if let Some(camera) = camera {
					self.open_scene_sink(sink_id, *camera);
				} else {
					// A window without a camera still closes the render graph.
					self.redraw_requested = true;
					self.start_fusion_catalog();
				}
			}
			Err(msg) => {
				log::error!(
					"Failed to create GHI window: {msg}. The most likely cause is missing platform graphics support or an incomplete environment setup. See {}.",
					crate::online_docs_url("use/setup/environment")
				);
			}
		}
	}

	/// Creates the image and sink state for a render image created with `handle`, and returns the image the sink's
	/// graph ends in.
	///
	/// The image has the swapchain format, so the passes that end a sink's graph write either alike, and is as large as
	/// the render image's extent. Every frame draws it before any
	/// window, so a window's pass can read it in the same frame. Next, show the returned image, resize it with
	/// [`Self::set_render_image_extent`], and stop it with [`Self::remove_render_image`].
	pub fn create_render_image(&mut self, handle: Handle, render_image: RenderImage) -> ghi::ImageHandle {
		let extent = render_image.extent();
		// The same uses as a window's swapchain, so every terminal pass writes either alike, and `Image` so a window's
		// pass can sample the picture.
		let image = self.context.build_image(
			ghi::image::Builder::new(
				self.context.swapchain_format(),
				ghi::Uses::RenderTarget | ghi::Uses::Storage | ghi::Uses::TransferSource | ghi::Uses::Image,
			)
			.name("Render image")
			.extent(extent),
		);
		let scene = render_image.scene();
		assert!(
			scene.index() < self.scenes.len(),
			"Render image names an unknown scene. The most likely cause is a scene id from another application. Show `SceneId::WORLD` or a scene `GraphicsApplication::create_scene` returned."
		);
		let sink_id = self.sinks.len();
		self.sinks.push(Some(SinkTarget::Image {
			handle,
			scene,
			image,
			extent,
		}));
		self.open_scene_sink(sink_id, render_image.camera());
		image
	}

	/// Shows `camera`'s scene on the new sink `sink_id` and closes the render graph, which the first sink does.
	fn open_scene_sink(&mut self, sink_id: SinkId, camera: Handle) {
		self.redraw_requested = true;
		// The first sink closes the render graph, so the fusions its steps group with can be read.
		self.start_fusion_catalog();
		self.sink_cameras.push((sink_id, camera));
		if (self.defer_first_frame_sink_setup && self.started_frame_count == 0) || !self.fusions_planned() {
			// The native window and swapchain can be presented before scene pipelines are created; this keeps
			// first-paint latency independent of shader and PSO warmup.
			self.pending_sink_initializations.push(sink_id);
		} else {
			self.initialize_scene_sink(sink_id);
		}
	}

	pub fn create_camera(&mut self, handle: Handle, camera: Camera) {
		if let Some((_, existing_camera)) = self
			.cameras
			.iter_mut()
			.find(|(existing_handle, _)| *existing_handle == handle)
		{
			*existing_camera = camera;
		} else {
			self.cameras.push((handle, camera));
		}
		self.redraw_requested = true;
	}

	/// Makes `light` the sun of `scene`, replacing the previous one: the newest directional light is the sun.
	///
	/// Every sink of the scene then carries it to its passes through [`Sink::sun`], placed with the transform each
	/// frame shows, so a window's sky and fog see the sun however late the window's passes were built.
	pub fn create_directional_light(&mut self, scene: SceneId, handle: Handle, light: DirectionalLight) {
		self.scenes[scene.index()].sun_light = Some((handle, light));
		self.redraw_requested = true;
	}

	/// Makes `fog` the fog of `scene`, replacing the previous one: the newest fog is the one shown.
	///
	/// Every sink of the scene then carries it to its passes through [`Sink::fog`], so a window's fog pass sees it
	/// however late the window's passes were built.
	pub fn create_fog(&mut self, scene: SceneId, fog: ExponentialHeightFog) {
		self.scenes[scene.index()].fog = Some(fog);
		self.redraw_requested = true;
	}
}
/// Names the GPU metric one counter reports to: `<kind>.<name>`, with `@<sink>` appended on every sink after the
/// first so one name stays one column while the usual single-sink application reads plain names.
fn gpu_metric_name(kind: &str, name: &str, sink_id: SinkId) -> String {
	if sink_id == 0 {
		format!("{kind}.{name}")
	} else {
		format!("{kind}.{name}@{sink_id}")
	}
}

/// Records one node's commands after giving the render targets it uses first new contents.
///
/// A node that records commands writes its targets itself, so they are only discarded. A node that records nothing,
/// for example while its pipeline compiles, leaves them cleared, so later nodes never read another target's memory.
/// `counter`, when the node has one, measures the GPU time of its commands.
fn record_node(
	recording: &mut ghi::implementation::CommandBufferRecording,
	first_uses: &[(SinkId, FirstUse)],
	sink: SinkId,
	node: RenderNode,
	commands: &[Option<RenderPassReturn>],
	counter: Option<ghi::CounterHandle>,
) {
	let first_uses = first_uses
		.iter()
		.filter(|(first_use_sink, first_use)| *first_use_sink == sink && first_use.node == node)
		.map(|(_, first_use)| first_use);
	if commands.iter().any(Option::is_some) {
		let images = first_uses.map(|first_use| first_use.image).collect::<SmallVec<[_; 8]>>();
		if !images.is_empty() {
			recording.discard_images(&images);
		}
	} else {
		let clears = first_uses
			.filter_map(|first_use| Some((first_use.image, first_use.clear?)))
			.collect::<SmallVec<[_; 8]>>();
		if !clears.is_empty() {
			recording.clear_images(&clears);
		}
	}
	// The counter times only the node's own commands, not the discards and clears above, and only when it records.
	let record = |recording: &mut ghi::implementation::CommandBufferRecording| {
		for command in commands.iter().flatten() {
			command(recording);
		}
	};
	match counter.filter(|_| commands.iter().any(Option::is_some)) {
		Some(counter) => recording.counter(counter, record),
		None => record(recording),
	}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ResolvedScreenshotCapture {
	FinalSwapchain {
		sink: SinkId,
	},
	AfterPass {
		pass: PassNodeId,
		target: ghi::ImageOrSwapchain,
	},
	/// A scene-pipeline target, read after every scene pipeline has recorded and before any post-scene pass.
	AfterScene {
		target: ghi::ImageOrSwapchain,
	},
	/// The copy of a history target that the previous frame wrote.
	PreviousFrame {
		target: ghi::ImageHandle,
	},
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RendererScreenshotError {
	SinkNotFound,
	SinkUnavailable,
	PassNotFound,
	PassAmbiguous,
	TargetNotWritten,
	TargetHasNoHistory,
	Transfer(ghi::TextureTransferError),
}

use std::{collections::VecDeque, sync::Arc};

use ghi::{
	command_buffer::{CommandBufferRecording, CommonCommandBufferMode as _},
	context::{Context as _, ContextCreate as _},
	frame::Frame as _,
	queue::{Queue as _, QueueExecution as _},
};
use resource_management::resource::resource_manager::ResourceManager;
use smallvec::SmallVec;
use tracing::debug_span;
use utils::{Box, Extent};

use super::{
	configuration::{
		PendingRenderPassConfiguration, apply_render_pass_configuration, declare_render_passes, set_render_pass_state,
	},
	targets::{FirstUse, RenderNode, RenderTargets, SinkExtents},
};
use crate::rendering::{
	debug_views::{DebugViewCatalog, DebugViewRegistry, DebugViewSettings, PendingDebugView, apply_debug_view_configuration},
	render_passes::debug_view::DebugViewPresentPass,
};
use crate::rendering::{
	render_pass::{
		Installed, InstalledBackground, InstalledFeature, InstalledPasses, InstalledSceneBackground, MainPlace,
		MainRenderTarget, RenderFeature, RenderFeatureId, RenderPass, RenderPassBuilder, RenderPassState, SceneBackground,
		SceneBackgroundFrame, SinkScope,
		pixel_steps::{Fusion, InstalledStep, InstalledSteps, PixelRun, PixelStep, Planned, load_fusions, plan},
	},
	render_passes::blit::ImageBypassPass,
};
use crate::{
	application::parameters::Parameters,
	configuration::ConfigurationPort,
	core::{EntityHandle, factory::Handle},
	metrics::{MetricId, MetricKind, Metrics},
	rendering::{
		Camera, DirectionalLight, ExponentialHeightFog, PipelineManagerServer, RenderImage, SceneId, ShownTransforms, Sink,
		Sun, View, make_perspective_view_from_camera,
		pipeline_manager::PipelineManager,
		render_pass::RenderPassReturn,
		window::{self, Window},
	},
};

#[cfg(test)]
mod tests {
	use super::{MainPlace, gpu_metric_name, jitter_offset, lens_sample, main_places};

	/// The last pass that replaces `main` writes the final target and the overlays after it draw over it, as the
	/// [terminal pass](/docs/reference/rendering#pipelines-and-passes) rule gives it.
	#[test]
	fn overlays_after_the_last_replacement_draw_over_the_final_target() {
		use MainPlace::{Before, Over, Terminal};
		// Overlay, pass, pass, overlay, overlay: the first overlay has a pass after it, so it is not at the end.
		let places = main_places(&[true, false, false, true, true], true);
		assert_eq!(places.as_slice(), [Before, Before, Terminal, Over, Over]);
		assert_eq!(main_places(&[false, false], true).as_slice(), [Before, Terminal]);
		// With only overlays, the scene's `main` is copied to the final target and every overlay draws over it.
		assert_eq!(main_places(&[true, true], true).as_slice(), [Over, Over]);
		// A sink with no `main` at all has nothing to draw over, so its last overlay starts the picture.
		assert_eq!(main_places(&[true, true], false).as_slice(), [Before, Terminal]);
		assert!(main_places(&[], true).is_empty());
	}

	/// The offsets repeat every 8 frames, stay inside the pixel, and average near its center, as the
	/// [jitter](/docs/reference/rendering#jitter) specification gives them.
	#[test]
	fn jitter_offsets_cycle_through_the_halton_points_inside_the_pixel() {
		let offsets = (0..8).map(|frame| jitter_offset(frame, 1.0)).collect::<Vec<_>>();
		assert_eq!(offsets[0], [0.0, 1.0 / 3.0 - 0.5]);
		assert_eq!(offsets[1], [-0.25, 2.0 / 3.0 - 0.5]);
		assert!(offsets.iter().all(|[x, y]| x.abs() < 0.5 && y.abs() < 0.5));
		assert!((0..8).all(|frame| jitter_offset(frame, 1.0) == jitter_offset(frame + 8, 1.0)));
		let [sum_x, sum_y] = offsets.iter().fold([0.0, 0.0], |[sx, sy], [x, y]| [sx + x, sy + y]);
		assert!((sum_x / 8.0).abs() < 0.07 && (sum_y / 8.0).abs() < 0.07);
	}

	/// A render pixel half an output pixel wide takes ⌈8 / 0.5²⌉ = 32 offsets, as the
	/// [jitter](/docs/reference/rendering#jitter) specification gives them.
	#[test]
	fn jitter_offsets_take_more_phases_at_a_lower_render_scale() {
		let period = |scale: f32| {
			(1..=64).find(|&n| (0..64).all(|frame| jitter_offset(frame, scale) == jitter_offset(frame + n, scale)))
		};
		assert_eq!(period(1.0), Some(8));
		assert_eq!(period(0.5), Some(32));
	}

	/// The samples stay on the unit disc and cover it evenly from any starting frame: centered, with a quarter of
	/// them within half the radius, as a quarter of the area is.
	#[test]
	fn lens_samples_cover_the_disc_evenly_from_any_frame() {
		for start in [0, 7, 1_000_003] {
			let samples = (start..start + 1024).map(lens_sample).collect::<Vec<_>>();
			assert!(samples.iter().all(|[x, y]| x * x + y * y <= 1.0));
			let count = samples.len() as f32;
			let [mean_x, mean_y] = samples.iter().fold([0.0, 0.0], |[sx, sy], [x, y]| [sx + x, sy + y]);
			assert!(
				(mean_x / count).abs() < 0.01 && (mean_y / count).abs() < 0.01,
				"start {start}"
			);
			let inner = samples.iter().filter(|[x, y]| x * x + y * y < 0.25).count() as f32;
			assert!((inner / count - 0.25).abs() < 0.01, "start {start}: {}", inner / count);
		}
	}

	/// Saves `data` to a fresh path under the system's temporary directory and returns what the file holds
	/// afterwards, or `None` when there is no file. `existing` is what the file holds before the save.
	fn saved(name: &str, existing: Option<&[u8]>, data: ghi::pipelines::CacheData) -> Option<Vec<u8>> {
		let path = std::env::temp_dir().join(format!("byte-engine-{}-{name}.pipelines", std::process::id()));
		if let Some(existing) = existing {
			std::fs::write(&path, existing).unwrap();
		}
		let result = super::save_pipeline_cache(&path, data);
		let contents = std::fs::read(&path).ok();
		let _ = std::fs::remove_file(&path);
		result.unwrap();
		contents
	}

	#[test]
	fn pipeline_cache_save_appends_new_pipelines_to_the_loaded_file() {
		let data = ghi::pipelines::CacheData {
			loaded: b"old",
			added: b"new",
		};
		assert_eq!(saved("append", Some(b"old"), data).as_deref(), Some(&b"oldnew"[..]));
	}

	#[test]
	fn pipeline_cache_save_replaces_a_file_the_device_kept_nothing_of() {
		let data = ghi::pipelines::CacheData {
			loaded: b"",
			added: b"new",
		};
		assert_eq!(saved("replace", Some(b"stale file"), data).as_deref(), Some(&b"new"[..]));
		assert_eq!(saved("create", None, data).as_deref(), Some(&b"new"[..]));
	}

	#[test]
	fn pipeline_cache_save_writes_nothing_when_nothing_was_compiled() {
		let data = ghi::pipelines::CacheData {
			loaded: b"old",
			added: b"",
		};
		assert_eq!(saved("unchanged", None, data), None);
	}

	#[test]
	fn gpu_metrics_are_named_by_kind_and_sink() {
		assert_eq!(
			gpu_metric_name("scene", "VisibilityPipelineManager", 0),
			"scene.VisibilityPipelineManager"
		);
		assert_eq!(gpu_metric_name("stage", "shadow-maps", 0), "stage.shadow-maps");
		assert_eq!(gpu_metric_name("pass", "Bloom", 2), "pass.Bloom@2");
	}
}
