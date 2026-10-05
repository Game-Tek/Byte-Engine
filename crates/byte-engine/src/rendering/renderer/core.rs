//! Frame orchestration and ownership of graphics hardware resources.
//!
//! Applications register windows, pipeline managers, and post-scene
//! [`crate::rendering::RenderPass`] values with [`Renderer`]. The graphics
//! application owns frame timing and calls the renderer in lifecycle order.

type RenderPassFactory = dyn for<'builder, 'resources> Fn(&'builder mut RenderPassBuilder<'resources>) -> Box<dyn RenderPass>;
type SinkId = usize;
/// Identifies a render pass created by a render-pass factory.
type RenderPassId = usize;
type PipelineManagerId = usize;

/// How one window takes part in the frame being prepared.
#[derive(Clone, Copy)]
enum WindowFrame {
	/// The frame renders into this acquired swapchain image and presents it.
	Acquired(ghi::PresentKey, Extent, ghi::SwapchainHandle),
	/// No part of the window can be seen, so the frame skips it without acquiring, unless a screenshot captures it.
	Hidden,
	/// The window had no usable swapchain image, so the frame skips it.
	Unusable,
}

/// The `SinkPass` struct keeps one sink-local post-scene pass with what the renderer needs to record and capture it.
struct SinkPass {
	harness: RenderPassHarness,
	sink: SinkId,
	/// Every name and alias the pass wrote when it was built, so screenshots can find its outputs.
	writable_targets: Vec<(String, ghi::ImageOrSwapchain)>,
	/// The copy that keeps `main` flowing while the pass is bypassed, when the pass replaced `main`.
	main_copy: Option<ImageBypassPass>,
	/// The GPU counter around the pass's commands and the metric its time is reported under.
	counter: Option<GpuCounter>,
}

/// A GPU timing counter paired with the metric the renderer publishes its durations to.
type GpuCounter = (ghi::CounterHandle, MetricId);

impl SinkPass {
	/// Prepares the pass's command and, while it is bypassed, the copy that forwards `main` after it.
	fn prepare<'a>(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> [Option<RenderPassReturn<'a>>; 2] {
		let command = self.harness.prepare(frame, sink, frame_allocator);
		let forward = match self.harness.state() {
			RenderPassState::Enabled => None,
			RenderPassState::Bypassed => self
				.main_copy
				.as_mut()
				.and_then(|copy| copy.prepare(frame, sink, frame_allocator)),
		};
		[command, forward]
	}
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

	/// Display windows, their swapchains, and whether the latest acquisition warned that the window cannot render.
	windows: SmallVec<[(ghi::Window, ghi::SwapchainHandle, bool); 16]>,
	/// The windowing connection that pumps every window's events. Declared after `windows` so they drop first.
	app: Option<ghi::window::App>,
	/// The frame index the acquisitions belong to and, per window adopted so far, how the window takes part in it.
	acquisitions: (u64, SmallVec<[WindowFrame; 16]>),
	/// The minimum time between presented frames applied to every window; `None` presents on every refresh.
	present_interval: Option<std::time::Duration>,
	/// Sink indices and their camera handles.
	sink_cameras: SmallVec<[(SinkId, Handle); 16]>,
	/// Cameras and their stable handles.
	cameras: SmallVec<[(Handle, Camera, Transform); 16]>,

	render_targets: RenderTargets,
	#[cfg(debug_assertions)]
	resource_updates: Option<resource_management::resource::ResourceUpdateListener>,

	/// Every sink's post-scene passes, in the order each sink records them. A pass's index is its [`RenderPassId`].
	render_passes: SmallVec<[SinkPass; 64]>,
	post_scene_render_pass_factories: SmallVec<[Box<RenderPassFactory>; 16]>,
	scene_background_factory: Option<Box<crate::rendering::render_pass::SceneBackgroundFactory>>,
	scene_backgrounds: SmallVec<[crate::rendering::render_pass::SceneBackground; 16]>,
	scene_presentation_copies: SmallVec<[(SinkId, ImageBypassPass); 16]>,
	pending_sink_initializations: SmallVec<[SinkId; 16]>,
	configuration: ConfigurationPort,
	pending_configuration: VecDeque<PendingRenderPassConfiguration>,
	/// The state every pass with one name shares, including scene backgrounds and passes created later.
	render_pass_states: RenderPassStates,

	pipeline_managers: SmallVec<[Box<dyn PipelineManager>; 16]>,
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

	/// The GHI context where all rendering resources and operations are performed. Only the render thread uses it.
	/// This field drops last so renderer subsystems finish pending GPU work before their resources are destroyed.
	context: ghi::implementation::Context,
}

impl Renderer {
	/// Creates a renderer that records and presents on `device`'s graphics queue, and returns it with the pipeline
	/// compilation servers that compile its pipelines. Run each server with [`PipelineManagerServer::run`] on a thread
	/// that owns it.
	///
	/// # Parameters
	/// - `render.startup.defer-sink-setup`: Presents the first window frame before constructing sink render pipelines.
	///   Defaults to false.
	/// - `render.pipeline-compilation.threads`: Sets how many threads compile pipelines. Defaults to half the
	///   available cores, between one and four.
	///
	/// GPU times of every frame, scene pipeline, post-scene pass, and the stages those create with
	/// [`RenderPassBuilder::create_gpu_counter`] are reported to `metrics` once each frame completes, under `frame`,
	/// `scene.<manager>`, `pass.<name>`, and `stage.<name>`, with `@<sink>` appended for sinks after the first.
	///
	/// Next, add a scene pipeline with [`Self::add_pipeline_manager`].
	pub fn new(
		device: &crate::rendering::GraphicsDevice,
		parameters: &dyn Parameters,
		configuration: &Configuration,
		metrics: Arc<Metrics>,
	) -> (Self, Vec<PipelineManagerServer>) {
		let defer_first_frame_sink_setup = parameters
			.get_parameter("render.startup.defer-sink-setup")
			.map(|parameter| parameter.as_bool_simple())
			.unwrap_or(false);

		let mut context = device.create_context();
		context.set_frames_in_flight(2);
		let pipeline_compilation_server_count = parameters
			.get_parameter("render.pipeline-compilation.threads")
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

			windows: SmallVec::with_capacity(16),
			app: None,
			acquisitions: (0, SmallVec::with_capacity(16)),
			present_interval: None,
			sink_cameras: SmallVec::with_capacity(16),
			cameras: SmallVec::with_capacity(16),

			render_targets: RenderTargets::new(),
			#[cfg(debug_assertions)]
			resource_updates: None,

			render_passes: SmallVec::with_capacity(64),
			post_scene_render_pass_factories: SmallVec::with_capacity(16),
			scene_background_factory: None,
			scene_backgrounds: SmallVec::new(),
			scene_presentation_copies: SmallVec::with_capacity(16),
			pending_sink_initializations: SmallVec::with_capacity(16),
			configuration: configuration.register(RENDER_PASS_PARAMETER_PREFIX),
			pending_configuration: VecDeque::new(),
			render_pass_states: RenderPassStates::default(),

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

	/// Reports the GPU time every counter measured in `frame`, now that the frame completed.
	fn publish_gpu_metrics(&self, frame: u64) {
		let counters = self
			.frame_counter
			.iter()
			.chain(self.scene_counters.iter().map(|(_, _, counter)| counter))
			.chain(self.stage_counters.iter())
			.chain(
				self.render_passes
					.iter()
					.filter_map(|render_pass| render_pass.counter.as_ref()),
			);
		for (counter, metric) in counters {
			if let Some(duration) = self.context.counter_duration(*counter) {
				self.metrics.set_gpu(frame, *metric, duration);
			}
		}
	}

	/// Connects the renderer to the resource manager whose development rebakes rebuild its pipelines.
	///
	/// In debug builds the renderer listens for replaced resources and recompiles the pipelines built from them.
	/// Release builds keep no connection. Call it in any order relative to adding pipeline managers and render
	/// passes. [`crate::application::graphics::GraphicsApplication::new`] calls it for you.
	#[cfg_attr(not(debug_assertions), allow(unused_variables))]
	pub fn set_resource_manager(&mut self, resource_manager: &EntityHandle<ResourceManager>) {
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
	/// [`Self::add_post_scene_render_pass_for_all_sinks`] or create a window with
	/// [`Self::create_window`].
	pub fn add_pipeline_manager(&mut self, mut pipeline_manager: impl PipelineManager + 'static) {
		let pipeline_manager_id = self.pipeline_managers.len();
		// Sinks that already exist get the manager's state now; deferred sinks get it when they initialize.
		let sinks: SmallVec<[SinkId; 16]> = self
			.sink_cameras
			.iter()
			.map(|(sink_id, _)| *sink_id)
			.filter(|sink_id| !self.pending_sink_initializations.contains(sink_id))
			.collect();
		for sink_id in sinks {
			self.create_manager_sink(&mut pipeline_manager, pipeline_manager_id, sink_id);
		}

		self.pipeline_managers.push(Box::new(pipeline_manager));
	}

	/// Starts the builder for one node of a sink's graph; `final_output` marks the sink's terminal post-scene pass.
	///
	/// Only scene nodes get the scene background factory, because only their backgrounds reach
	/// `self.scene_backgrounds`, where the renderer asks them for frames.
	fn sink_builder(&mut self, sink_id: SinkId, node: RenderNode, final_output: bool) -> RenderPassBuilder<'_> {
		let scene_background_factory = match node {
			RenderNode::Scene(_) => self.scene_background_factory.as_deref(),
			RenderNode::Pass(_) | RenderNode::Presentation => None,
		};
		RenderPassBuilder::new(
			&mut self.context,
			&mut self.render_targets,
			&mut self.render_pass_states,
			sink_id,
			self.windows[sink_id].1,
			self.pipeline_compilation_client.clone(),
			scene_background_factory,
			node,
			final_output,
		)
	}

	/// Lets one scene pipeline manager create its persistent state for a sink and records the targets it uses.
	fn create_manager_sink(
		&mut self,
		pipeline_manager: &mut dyn PipelineManager,
		pipeline_manager_id: PipelineManagerId,
		sink_id: SinkId,
	) {
		let mut builder = self.sink_builder(sink_id, RenderNode::Scene(pipeline_manager_id), false);
		pipeline_manager.create_sink(sink_id, &mut builder);
		builder.record_node();
		let backgrounds = builder.take_scene_backgrounds();
		let stage_counters = builder.take_gpu_counters();
		self.adopt_stage_counters(stage_counters, sink_id);
		self.scene_backgrounds.extend(backgrounds);
		if let Some(counter) = self.create_gpu_counter("scene", pipeline_manager.name(), sink_id) {
			self.scene_counters.push((pipeline_manager_id, sink_id, counter));
		}
	}

	fn initialize_scene_sink(&mut self, sink_id: SinkId) {
		// Every manager builds through `self`, so the list steps aside while they do.
		let mut pipeline_managers = std::mem::take(&mut self.pipeline_managers);
		for (pipeline_manager_id, pipeline_manager) in pipeline_managers.iter_mut().enumerate() {
			self.create_manager_sink(&mut **pipeline_manager, pipeline_manager_id, sink_id);
		}
		self.pipeline_managers = pipeline_managers;

		self.add_post_scene_render_passes_for_sink(sink_id);
	}

	/// Changes the state of every sink-local render pass with the requested stable name.
	///
	/// Returns the number of updated instances. A return value of `0` means that no registered render pass uses
	/// `name`. Pass names come from [`RenderPass::name`].
	pub fn set_render_pass_state(&mut self, name: &str, state: RenderPassState) -> usize {
		self.redraw_requested = true;
		set_render_pass_state(&mut self.render_pass_states, name, state)
	}

	/// Registers the scene background that every future sink's scene pipeline records between opaque and
	/// transparent surfaces, such as an atmosphere sky.
	///
	/// Register it before creating a window. Its [`RenderPass::name`] controls it like a post-scene pass. A later
	/// registration replaces an earlier one.
	pub fn set_scene_background_for_all_sinks<F>(&mut self, factory: F)
	where
		F: for<'builder, 'resources> Fn(
				&'builder mut RenderPassBuilder<'resources>,
				crate::rendering::render_pass::SceneBackgroundTargets,
			) -> Box<dyn RenderPass>
			+ 'static,
	{
		assert!(
			self.windows.is_empty(),
			"Render graph is already closed. The most likely cause is that a scene background was registered after creating a window. Register it before creating the first window."
		);
		self.scene_background_factory = Some(Box::new(factory));
	}

	/// Registers a render pass factory that will be instantiated for every future sink.
	///
	/// Register every pass before creating a window. Closing the graph at that
	/// boundary lets the renderer give the terminal pass the swapchain directly.
	pub fn add_post_scene_render_pass_for_all_sinks<F>(&mut self, render_pass_factory: F)
	where
		F: for<'builder, 'resources> Fn(&'builder mut RenderPassBuilder<'resources>) -> Box<dyn RenderPass> + 'static,
	{
		assert!(
			self.windows.is_empty(),
			"Render graph is already closed. The most likely cause is that a post-scene render pass was registered after creating a window. Register every pass before creating the first window."
		);
		self.post_scene_render_pass_factories.push(Box::new(render_pass_factory));
	}

	/// Instantiates all registered post-scene render pass factories for a given sink.
	fn add_post_scene_render_passes_for_sink(&mut self, sink_id: SinkId) {
		// Every pass builds through `self`, so the factories step aside while they run.
		let factories = std::mem::take(&mut self.post_scene_render_pass_factories);
		let final_factory_index = factories.len().checked_sub(1);
		for (factory_index, factory) in factories.iter().enumerate() {
			let final_output = Some(factory_index) == final_factory_index;
			let render_pass_id = self.render_passes.len();
			let mut builder = self.sink_builder(sink_id, RenderNode::Pass(render_pass_id), final_output);
			let render_pass = factory(&mut builder);
			assert!(
				!final_output || builder.writes_final_output(),
				"Final render pass has no presentation output. The most likely cause is that it created or aliased an ordinary image instead of calling `RenderPassBuilder::create_main_render_target`."
			);
			builder.record_node();
			let writable_targets = builder.writable_targets();
			let main_copy = builder.take_main_copy();
			let stage_counters = builder.take_gpu_counters();
			self.adopt_stage_counters(stage_counters, sink_id);
			let counter = self.create_gpu_counter("pass", render_pass.name(), sink_id);
			self.render_passes.push(SinkPass {
				harness: RenderPassHarness::new(render_pass, &mut self.render_pass_states),
				sink: sink_id,
				writable_targets,
				main_copy,
				counter,
			});
		}
		self.post_scene_render_pass_factories = factories;

		// A scene-only graph has no terminal pass that can receive the swapchain directly. An empty graph also has
		// no scene color to copy, so presenting the acquired swapchain is the complete frame operation in that case.
		if final_factory_index.is_none() && self.render_targets.get("main", sink_id).is_some() {
			let swapchain = self.windows[sink_id].1;
			let mut builder = self.sink_builder(sink_id, RenderNode::Presentation, false);
			let source = builder.read_from("main");
			let copy = ImageBypassPass::new(&mut builder, source, swapchain);
			builder.record_node();
			self.scene_presentation_copies.push((sink_id, copy));
		}
	}

	/// Waits as `wait` allows for the first event, then drains pending application and window events. Match
	/// [`ghi::window::Event::Window`] ids against the windows created with [`Self::create_window`].
	///
	/// Without a window there is no event queue, so the call returns at once whatever `wait` says.
	pub fn poll_windows(&mut self, wait: ghi::window::Wait) -> impl Iterator<Item = ghi::window::Event> + '_ {
		self.app.iter_mut().flat_map(move |app| app.poll(wait))
	}

	/// Returns the handle that interrupts a waiting [`Self::poll_windows`], once a window connected the renderer
	/// to the window system.
	pub(crate) fn app_waker(&self) -> Option<ghi::window::AppWaker> {
		self.app.as_ref().map(ghi::window::App::waker)
	}

	/// Caps the presentation rate by setting the minimum time between presented frames on every window,
	/// including windows created later. `None` removes the cap so frames present on every refresh.
	pub fn set_present_interval(&mut self, interval: Option<std::time::Duration>) {
		self.present_interval = interval;
		for (_window, swapchain, _) in &self.windows {
			self.context.set_present_interval(*swapchain, interval);
		}
	}

	/// Returns the shortest refresh interval among the displays showing a window, when any platform reports one.
	///
	/// The fastest display decides so that no window handles its events later than its display could show them.
	pub fn refresh_interval(&self) -> Option<std::time::Duration> {
		self.windows.iter().filter_map(|(window, ..)| window.refresh_interval()).min()
	}

	/// Acquires the swapchain image of every window for the next frame and returns the display time of the
	/// primary window's most recently presented image, when the backend reports it.
	///
	/// Call this at the start of a tick so simulation runs after the presentation engine releases an image. Each
	/// frame waits for the display once: here when the backend's acquisition waits for a free image, or at
	/// submission on Metal, which takes its drawable after committing the frame. Either way the wait ends the
	/// previous tick or starts this one, so simulation still follows the display.
	/// A window that cannot be seen is skipped without acquiring: whatever the frame presented there would never be
	/// shown, and on Metal the presentation engine stops pacing the loop for it. Pass the sinks a screenshot captures
	/// as `captured` so those windows render anyway.
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
		let undecided = |acquisitions: &[WindowFrame], index: SinkId| match acquisitions.get(index) {
			None => true,
			Some(window_frame) => matches!(window_frame, WindowFrame::Hidden) && captured(index),
		};
		if !(0..self.windows.len()).any(|index| undecided(&self.acquisitions.1, index)) {
			return None;
		}

		let _span = debug_span!(
			"Renderer::acquire_swapchains",
			frame = self.started_frame_count,
			windows = self.windows.len()
		)
		.entered();

		let frame = ghi::queue::FrameRequest::new(self.started_frame_count, self.render_finished_synchronizer);
		let mut present_time = None;

		for (index, (window, swapchain, warned)) in self.windows.iter_mut().enumerate() {
			if !undecided(&self.acquisitions.1, index) {
				continue;
			}
			if !captured(index) && !window.is_visible() {
				self.acquisitions.1.push(WindowFrame::Hidden);
				continue;
			}

			let acquisition = self.context.acquire_swapchain_image(frame, *swapchain);
			if index == 0 {
				present_time = acquisition.as_ref().and_then(|acquisition| acquisition.present_time());
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

			let window_frame = match acquisition.filter(|_| problem.is_none()) {
				Some(acquisition) => WindowFrame::Acquired(acquisition.present_key(), acquisition.extent(), *swapchain),
				None => WindowFrame::Unusable,
			};
			match self.acquisitions.1.get_mut(index) {
				Some(decided) => *decided = window_frame,
				None => self.acquisitions.1.push(window_frame),
			}
		}

		present_time
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
		let mut needs_frame = self.redraw_requested || !self.pending_sink_initializations.is_empty();
		// Ask every pass so each adopts its inputs this tick, even when an earlier one already answered.
		for render_pass in &mut self.render_passes {
			needs_frame |= render_pass.harness.needs_frame();
		}
		for background in &self.scene_backgrounds {
			needs_frame |= background.needs_frame();
		}
		needs_frame
	}

	/// Starts scene resource requests before window setup or frame preparation borrows the context.
	pub(crate) fn update(&mut self) {
		for pipeline_manager in &mut self.pipeline_managers {
			pipeline_manager.update();
		}
	}

	/// Tells every pipeline manager that one simulation step ended; see [`PipelineManager::step`].
	pub(crate) fn step(&mut self) {
		for pipeline_manager in &mut self.pipeline_managers {
			pipeline_manager.step();
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
				.any(|window_frame| matches!(window_frame, WindowFrame::Acquired(..)))
	}

	/// Prepares a frame by invoking the configured render passes.
	///
	/// The renderer skips every window that has no usable swapchain image, such as a zero-sized one, and every
	/// window that cannot be seen and no screenshot captures. It returns the frame that
	/// every screenshot readback comes from, and one readback result per request
	/// in request order.
	// Keep the frame transaction contiguous so recording, presentation, and screenshot transfers stay ordered.
	// Swapchain acquisition happens before this call (see `acquire_swapchain_images`) so the tick can pace on it, or
	// on the drawable wait at submission where the backend defers it.
	#[allow(clippy::excessive_nesting, clippy::too_many_lines)]
	pub(crate) fn prepare(
		&'_ mut self,
		transforms_listener: &mut impl Listener<TransformationUpdate>,
		frame_allocator: &bumpalo::Bump,
		screenshot_requests: &[(usize, &crate::inspector::screenshot::ScreenshotCapture)],
		alpha: f32,
		time: crate::time::MediaTime,
	) -> (u64, Vec<Result<ghi::TextureReadback, RendererScreenshotError>>) {
		let _span = debug_span!(
			"Renderer::prepare",
			frame = self.started_frame_count,
			windows = self.windows.len()
		)
		.entered();

		let Some(_) = self.windows.first() else {
			log::debug!("No swapchains available to present to. Skipping rendering!");
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

		if self.started_frame_count > 0 {
			for sink_id in std::mem::take(&mut self.pending_sink_initializations) {
				self.initialize_scene_sink(sink_id);
			}
		}
		// Queued render-pass configuration applies after passes exist and before they prepare frame work.
		apply_render_pass_configuration(&self.configuration, &mut self.pending_configuration, &self.render_pass_states);

		// Resolve names outside command recording so the hot path only compares pass IDs and transfers handles.
		let screenshot_captures = screenshot_requests
			.iter()
			.map(|(sink, capture)| self.resolve_screenshot_capture(*sink, capture))
			.collect::<Vec<_>>();

		self.context.start_frame_capture();

		{
			let _span = debug_span!("Renderer::update_camera_transforms").entered();
			while let Some(message) = transforms_listener.read() {
				if let Some((.., transform)) = self.cameras.iter_mut().find(|(handle, ..)| *handle == message.handle()) {
					transform.set_position(message.transform().get_position());
					transform.set_orientation(message.transform().get_orientation());
				}
			}
		}

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

				{
					let _span = debug_span!("Renderer::build_sinks", cameras = self.cameras.len()).entered();
					for (sink_id, camera_handle) in self.sink_cameras.iter() {
						let WindowFrame::Acquired(_present_key, extent, _swapchain) = swapchains[*sink_id] else {
							continue;
						};
						let Some((_, camera, transform)) = self.cameras.iter().find(|(handle, ..)| handle == camera_handle)
						else {
							continue;
						};

						let view = make_perspective_view_from_camera(camera, transform, extent);
						sinks.push(Sink::new(view, extent, *sink_id).with_exposure_scale(camera.exposure_scale()));
					}
				}

				// The render targets each node uses first, per sink, so recording can give them new contents.
				let mut first_uses = SmallVec::<[(SinkId, FirstUse); 64]>::new();
				{
					let _span = debug_span!("Renderer::resize_render_targets", sinks = sinks.len()).entered();
					for sink in &sinks {
						// Resize the sink's images to its extent, divided for reduced-resolution targets.
						for (image, extent) in self.render_targets.get_images_for_sink(sink.index(), sink.extent()) {
							frame.resize_image(image, extent);
						}
						// Targets whose uses do not overlap share memory, placed again only when an extent or use changes.
						if let Some(plan) = self.render_targets.plan(sink.index(), sink.extent()) {
							frame.place_image_group(plan.group, &plan.members);
							first_uses.extend(plan.first_uses.into_iter().map(|first_use| (sink.index(), first_use)));
						}
					}
				}

				// Each manager's commands, indexed by manager, with the sink every command records for.
				let pipeline_manager_commands: SmallVec<[SmallVec<[(SinkId, RenderPassReturn<'_>); 16]>; 16]> = {
					let _span = debug_span!("Renderer::prepare_pipeline_managers").entered();
					self.pipeline_managers
						.iter_mut()
						.map(|pipeline_manager| pipeline_manager.prepare(frame, &sinks, frame_allocator, alpha, time))
						.collect()
				};

				// A list of render pass commands with their pass and sink indices and the counter that times them.
				let render_pass_commands: SmallVec<
					[([Option<RenderPassReturn>; 2], RenderPassId, SinkId, Option<GpuCounter>); 64],
				> = {
					let _span = debug_span!("Renderer::prepare_render_passes").entered();
					self.render_passes
						.iter_mut()
						.enumerate()
						.filter_map(|(render_pass_id, render_pass)| {
							let sink = sinks.iter().find(|sink| sink.index() == render_pass.sink)?;
							let counter = render_pass.counter;
							Some((
								render_pass.prepare(frame, sink, frame_allocator),
								render_pass_id,
								sink.index(),
								counter,
							))
						})
						.collect()
				};

				let scene_presentation_commands: SmallVec<[(Option<RenderPassReturn>, SinkId); 16]> = self
					.scene_presentation_copies
					.iter_mut()
					.filter_map(|(sink_id, copy)| {
						let sink = sinks.iter().find(|sink| sink.index() == *sink_id)?;
						Some((copy.prepare(frame, sink, frame_allocator), *sink_id))
					})
					.collect();

				let present_keys = swapchains
					.iter()
					.filter_map(|window_frame| match window_frame {
						WindowFrame::Acquired(present_key, ..) => Some(*present_key),
						WindowFrame::Hidden | WindowFrame::Unusable => None,
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
						for (pipeline_manager_id, commands) in pipeline_manager_commands.iter().enumerate() {
							for sink in &sinks {
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
								command_buffer_recording.transfer_texture_with_frame(*target, -1)
							}
							_ => continue,
						};
						screenshot_transfers[request_index] = Some(transfer.map_err(RendererScreenshotError::Transfer));
					}

					{
						let _span = debug_span!("Renderer::record_render_pass_commands").entered();
						for (commands, render_pass_id, sink, counter) in render_pass_commands {
							let node = RenderNode::Pass(render_pass_id);
							let counter = counter.map(|(counter, _)| counter);
							record_node(command_buffer_recording, &first_uses, sink, node, &commands, counter);
							for (request_index, capture) in screenshot_captures.iter().enumerate() {
								if let Ok(ResolvedScreenshotCapture::AfterPass { pass, target }) = capture
									&& *pass == render_pass_id
								{
									screenshot_transfers[request_index] = Some(
										command_buffer_recording
											.transfer_texture(*target)
											.map_err(RendererScreenshotError::Transfer),
									);
								}
							}
						}
					}

					for (command, sink_id) in scene_presentation_commands {
						record_node(command_buffer_recording, &first_uses, sink_id, RenderNode::Presentation, &[command], None);
					}

					// Final captures remain after every pass; duplicate requests receive independent transfer handles.
					for (request_index, capture) in screenshot_captures.iter().enumerate() {
						screenshot_transfers[request_index] = Some(match capture {
							Err(error) => Err(*error),
							Ok(ResolvedScreenshotCapture::FinalSwapchain { sink }) => match swapchains.get(*sink) {
								None => Err(RendererScreenshotError::SinkNotFound),
								Some(WindowFrame::Hidden | WindowFrame::Unusable) => Err(RendererScreenshotError::SinkUnavailable),
								Some(WindowFrame::Acquired(_present_key, _extent, swapchain)) => command_buffer_recording
									.transfer_texture(ghi::ImageOrSwapchain::Swapchain(*swapchain))
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
		if sink >= self.windows.len() {
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
					.or_else(|| self.render_targets.history(target, sink).map(Into::into))
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
			.render_passes
			.iter()
			.enumerate()
			.filter(|(_, render_pass)| render_pass.sink == sink && render_pass.harness.name() == pass);
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

	/// Borrows the render thread's GHI context for setup work that must run before the first frame.
	pub fn context_mut(&mut self) -> &mut ghi::implementation::Context {
		&mut self.context
	}

	/// Returns a client for requesting renderer-owned asynchronous pipelines.
	pub fn pipeline_manager_client(&self) -> crate::rendering::PipelineManagerClient {
		self.pipeline_compilation_client.clone()
	}

	/// Creates the swapchain and sink state for a window.
	///
	/// Next, create a camera with [`Self::create_camera`] and associate it with the
	/// sink through the application or world integration.
	pub fn create_window(&mut self, window: Window) {
		let name = window.name();
		let extent = window.extent();
		let camera = window.camera();

		let features = if window.features().contains(window::Features::DECORATIONS) {
			ghi::window::Features::DECORATIONS
		} else {
			ghi::window::Features::empty()
		};

		// Connect on first use so headless renderers never touch the windowing system.
		let app = self.app.take().map_or_else(|| ghi::window::App::new("main_window"), Ok);
		let window = app.and_then(|mut app| {
			let window = app.create_window(name, extent, features);
			self.app = Some(app);
			window
		});

		match window {
			Ok(window) => {
				let swapchain_handle = self.context.bind_to_window(
					&window.os_handles(),
					ghi::PresentationModes::FIFO,
					extent,
					ghi::Uses::RenderTarget | ghi::Uses::Storage | ghi::Uses::TransferSource,
				);
				if self.present_interval.is_some() {
					self.context.set_present_interval(swapchain_handle, self.present_interval);
				}

				let sink_id = self.windows.len();
				self.windows.push((window, swapchain_handle, false));
				self.redraw_requested = true;

				if let Some(camera) = camera {
					self.sink_cameras.push((sink_id, *camera));
					if self.defer_first_frame_sink_setup && self.started_frame_count == 0 {
						// The native window and swapchain can be presented before scene pipelines are created; this keeps
						// first-paint latency independent of shader and PSO warmup.
						self.pending_sink_initializations.push(sink_id);
					} else {
						self.initialize_scene_sink(sink_id);
					}
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

	pub fn create_camera(&mut self, handle: Handle, camera: Camera) {
		if let Some((_, existing_camera, _)) = self
			.cameras
			.iter_mut()
			.find(|(existing_handle, ..)| *existing_handle == handle)
		{
			*existing_camera = camera;
		} else {
			self.cameras.push((handle, camera, Transform::default()));
		}
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
			.map(|first_use| (first_use.image, first_use.clear))
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
		pass: RenderPassId,
		target: ghi::ImageOrSwapchain,
	},
	/// A scene-pipeline target, read after every scene pipeline has recorded and before any post-scene pass.
	AfterScene {
		target: ghi::ImageOrSwapchain,
	},
	/// The copy of a history target that the previous frame wrote.
	PreviousFrame {
		target: ghi::DynamicImageHandle,
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
		PendingRenderPassConfiguration, RENDER_PASS_PARAMETER_PREFIX, apply_render_pass_configuration, set_render_pass_state,
	},
	targets::{FirstUse, RenderNode, RenderTargets},
};
use crate::{
	application::parameters::Parameters,
	configuration::{Configuration, ConfigurationPort},
	core::{EntityHandle, factory::Handle, listener::Listener},
	gameplay::transform::TransformationUpdate,
	metrics::{MetricId, MetricKind, Metrics},
	rendering::{
		Camera, PipelineManagerServer, Sink, make_perspective_view_from_camera,
		pipeline_manager::PipelineManager,
		render_pass::RenderPassReturn,
		window::{self, Window},
	},
};
use crate::{
	gameplay::Transform,
	rendering::{
		render_pass::{RenderPass, RenderPassBuilder, RenderPassHarness, RenderPassState, RenderPassStates},
		render_passes::blit::ImageBypassPass,
	},
};

#[cfg(test)]
mod tests {
	use super::gpu_metric_name;

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
