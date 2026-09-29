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

/// The `SinkPass` struct keeps one sink-local post-scene pass with what the renderer needs to record and capture it.
struct SinkPass {
	harness: RenderPassHarness,
	sink: SinkId,
	/// Every name and alias the pass wrote when it was built, so screenshots can find its outputs.
	writable_targets: Vec<(String, ghi::ImageOrSwapchain)>,
	/// The copy that keeps `main` flowing while the pass is bypassed, when the pass replaced `main`.
	main_copy: Option<ImageBypassPass>,
}

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
	/// The frame index the acquisitions belong to and, per window, the acquired image or `None` when its extent is unusable.
	acquisitions: (u64, SmallVec<[Option<(ghi::PresentKey, Extent, ghi::SwapchainHandle)>; 16]>),
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
	pipeline_compilation_servers: Vec<crate::rendering::PipelineManagerServer>,

	/// The GHI queue where graphics commands are submitted. The main rendering operations occur on this queue.
	graphics_queue_handle: ghi::QueueHandle,

	render_command_buffer: ghi::CommandBufferHandle,
	render_finished_synchronizer: ghi::SynchronizerHandle,
	defer_first_frame_sink_setup: bool,
	/// Whether renderer state changed in a way that the last presented frame does not show.
	redraw_requested: bool,

	/// The GHI context where all rendering resources and operations are performed. Only the render thread uses it.
	/// This field drops last so renderer subsystems finish pending GPU work before their resources are destroyed.
	context: ghi::implementation::Context,
}

impl Renderer {
	/// Creates a renderer that records and presents on `device`'s graphics queue.
	///
	/// # Parameters
	/// - `render.startup.defer-sink-setup`: Presents the first window frame before constructing sink render pipelines.
	///   Defaults to false.
	/// - `render.pipeline-compilation.threads`: Sets how many threads compile pipelines. Defaults to half the
	///   available cores, between one and four.
	///
	/// Next, add a scene pipeline with [`Self::add_pipeline_manager`].
	pub fn new(device: &crate::rendering::GraphicsDevice, parameters: &dyn Parameters, configuration: &Configuration) -> Self {
		let defer_first_frame_sink_setup = parameters
			.get_parameter("render.startup.defer-sink-setup")
			.map(|parameter| parameter.as_bool_simple())
			.unwrap_or(false);

		let mut context = device.create_context();
		let frame_queue_depth = 2;
		context.set_frames_in_flight(frame_queue_depth);
		let pipeline_compilation_server_count = parameters
			.get_parameter("render.pipeline-compilation.threads")
			.and_then(|parameter| parameter.parse::<usize>().ok())
			.unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |count| (count.get() / 2).clamp(1, 4)));
		let (pipeline_compilation_client, pipeline_compilation_manager, pipeline_compilation_servers) =
			crate::rendering::pipeline_compilation::PipelineManager::new(&mut context, pipeline_compilation_server_count);

		let graphics_queue_handle = device.graphics_queue();

		let render_command_buffer = context.queue(graphics_queue_handle).create_command_buffer(Some("Render"));
		let render_finished_synchronizer = context.create_synchronizer(Some("Render Finisished"), true);

		Renderer {
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
			pipeline_compilation_servers,

			graphics_queue_handle,

			render_command_buffer,
			render_finished_synchronizer,
			defer_first_frame_sink_setup,
			redraw_requested: true,
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
		self.scene_backgrounds.extend(backgrounds);
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

	fn initialize_pending_sink_resources(&mut self) {
		let pending_sink_initializations = std::mem::take(&mut self.pending_sink_initializations);
		for sink_id in pending_sink_initializations {
			self.initialize_scene_sink(sink_id);
		}
	}

	/// Changes the state of every sink-local render pass with the requested stable name.
	///
	/// Returns the number of updated instances. A return value of `0` means that no registered render pass uses
	/// `name`. Pass names come from [`RenderPass::name`].
	pub fn set_render_pass_state(&mut self, name: &str, state: RenderPassState) -> usize {
		self.redraw_requested = true;
		set_render_pass_state(&mut self.render_pass_states, name, state)
	}

	/// Applies queued render-pass configuration after passes exist and before they prepare frame work.
	fn apply_configuration(&mut self) {
		apply_render_pass_configuration(&self.configuration, &mut self.pending_configuration, &self.render_pass_states);
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
			self.render_passes.push(SinkPass {
				harness: RenderPassHarness::new(render_pass, &mut self.render_pass_states),
				sink: sink_id,
				writable_targets,
				main_copy,
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
	/// Call this at the start of a tick so simulation runs after the presentation engine releases an image.
	/// The call is idempotent for one frame: windows already acquired are skipped, so [`Self::prepare`] can
	/// call it to pick up windows adopted later in the tick or to acquire when nothing was hoisted.
	pub(crate) fn acquire_swapchain_images(&mut self) -> Option<std::time::Instant> {
		if self.acquisitions.0 != self.started_frame_count {
			self.acquisitions = (self.started_frame_count, SmallVec::new());
		}
		if self.acquisitions.1.len() == self.windows.len() {
			return None;
		}

		let span = debug_span!(
			"Renderer::acquire_swapchains",
			frame = self.started_frame_count,
			windows = self.windows.len()
		);
		let _enter = span.enter();

		let frame = ghi::queue::FrameRequest::new(self.started_frame_count, self.render_finished_synchronizer);
		let mut present_time = None;

		for (index, (_window, swapchain, warned)) in self.windows.iter_mut().enumerate().skip(self.acquisitions.1.len()) {
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

			self.acquisitions.1.push(
				acquisition
					.filter(|_| problem.is_none())
					.map(|acquisition| (acquisition.present_key(), acquisition.extent(), *swapchain)),
			);
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
	/// and the caller must pace the tick itself.
	pub(crate) fn presents_this_frame(&self) -> bool {
		self.acquisitions.0 == self.started_frame_count && self.acquisitions.1.iter().any(Option::is_some)
	}

	/// Returns whether any frame has been submitted to a window since startup.
	pub(crate) fn has_presented(&self) -> bool {
		self.started_frame_count > 0
	}

	/// Prepares a frame by invoking the configured render passes.
	///
	/// The renderer skips execution when no swapchain is available or when any
	/// swapchain surface has a zero-sized dimension. It returns the frame that
	/// every screenshot readback comes from, and one readback result per request
	/// in request order.
	// Keep the frame transaction contiguous so recording, presentation, and screenshot transfers stay ordered.
	// Swapchain acquisition happens before this call (see `acquire_swapchain_images`) so the tick can pace on it.
	#[allow(clippy::excessive_nesting, clippy::too_many_lines)]
	pub(crate) fn prepare(
		&'_ mut self,
		transforms_listener: &mut impl Listener<TransformationUpdate>,
		frame_allocator: &bumpalo::Bump,
		screenshot_requests: &[(usize, &crate::inspector::screenshot::ScreenshotCapture)],
		alpha: f32,
		time: crate::time::MediaTime,
	) -> (u64, Vec<Result<ghi::TextureReadback, RendererScreenshotError>>) {
		let span = debug_span!(
			"Renderer::prepare",
			frame = self.started_frame_count,
			windows = self.windows.len()
		);
		let _enter = span.enter();

		let Some(_) = self.windows.first() else {
			log::debug!("No swapchains available to present to. Skipping rendering!");
			let screenshots = screenshot_requests
				.iter()
				.map(|_| Err(RendererScreenshotError::SinkNotFound))
				.collect();
			return (self.started_frame_count, screenshots);
		};
		// Acquire here when nothing was hoisted to the start of the tick, or for windows adopted since.
		self.acquire_swapchain_images();
		self.redraw_requested = false;

		if self.started_frame_count > 0 && !self.pending_sink_initializations.is_empty() {
			self.initialize_pending_sink_resources();
		}
		self.apply_configuration();

		// Resolve names outside command recording so the hot path only compares pass IDs and transfers handles.
		let screenshot_captures = screenshot_requests
			.iter()
			.map(|(sink, capture)| self.resolve_screenshot_capture(*sink, capture))
			.collect::<Vec<_>>();

		self.context.start_frame_capture();

		{
			let span = debug_span!("Renderer::update_camera_transforms");
			let _enter = span.enter();
			while let Some(message) = transforms_listener.read() {
				let handle = message.handle();

				if let Some(transform) = self
					.cameras
					.iter_mut()
					.find_map(|(h, _, transform)| if handle == *h { Some(transform) } else { None })
				{
					transform.set_position(message.transform().get_position());
					transform.set_orientation(message.transform().get_orientation());
				}
			}
		}

		let mut queue = self.context.queue(self.graphics_queue_handle);
		let frame =
			ghi::queue::FrameRequest::new_in(self.started_frame_count, self.render_finished_synchronizer, &frame_allocator);

		self.started_frame_count += 1;

		let command_buffer = self.render_command_buffer;
		let synchronizer = self.render_finished_synchronizer;
		let wait_for = &[];
		let swapchains = &self.acquisitions.1;
		let sink_cameras = &self.sink_cameras;
		let cameras = &self.cameras;
		let render_targets = &self.render_targets;
		let pipeline_managers = &mut self.pipeline_managers;
		let pipeline_compilation_client = &self.pipeline_compilation_client;
		let pipeline_compilation_manager = &mut self.pipeline_compilation_manager;
		#[cfg(debug_assertions)]
		let resource_updates = &self.resource_updates;
		let render_passes = &mut self.render_passes;
		let scene_presentation_copies = &mut self.scene_presentation_copies;
		let frame_allocator = frame_allocator;
		let submitted_frame = self.started_frame_count - 1;
		let mut screenshot_transfers = (0..screenshot_captures.len()).map(|_| None).collect::<Vec<_>>();

		{
			let span = debug_span!("Renderer::queue_execute");
			let _enter = span.enter();
			queue.execute(Some(frame), wait_for, synchronizer, |execution| {
				#[cfg(debug_assertions)]
				if let Some(resource_updates) = resource_updates {
					while let Some(update) = resource_updates.read() {
						pipeline_compilation_client.resource_updated(update.id());
					}
				}
				pipeline_compilation_manager.publish(execution.frame().expect(
					"Frame is required to publish compiled pipelines. The most likely cause is that Renderer::prepare called Queue::execute without a frame request.",
				));

				let (
					sinks,
					pipeline_manager_commands,
					render_pass_commands,
					scene_presentation_commands,
					present_keys,
					first_uses,
				) = {
					let span = debug_span!("Renderer::prepare_frame_work");
					let _enter = span.enter();
					let frame = execution.frame().expect(
					"Frame is required to prepare renderer frame work. The most likely cause is that Renderer::render called Queue::execute without a frame request.",
				);
					let mut sinks: SmallVec<[Sink; 16]> = SmallVec::new();

					{
						let span = debug_span!("Renderer::build_sinks", cameras = cameras.len());
						let _enter = span.enter();
						for (sink_id, camera_handle) in sink_cameras.iter() {
							let Some((_present_key, extent, _swapchain)) = swapchains[*sink_id] else {
								continue;
							};

							let Some((camera, transform)) = cameras
								.iter()
								.find_map(|(handle, camera, transform)| if handle == camera_handle { Some((camera, transform)) } else { None })
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
						let span = debug_span!("Renderer::resize_render_targets", sinks = sinks.len());
						let _enter = span.enter();
						for sink in &sinks {
							// Resize the sink's images to its extent, divided for reduced-resolution targets.
							for (image, extent) in render_targets.get_images_for_sink(sink.index(), sink.extent()) {
								frame.resize_image(image, extent);
							}
							// Targets whose uses do not overlap share memory, placed again only when an extent or use changes.
							if let Some(plan) = render_targets.plan(sink.index(), sink.extent()) {
								frame.place_image_group(plan.group, &plan.members);
								first_uses.extend(plan.first_uses.into_iter().map(|first_use| (sink.index(), first_use)));
							}
						}
					}

					// Each manager's commands, indexed by manager, with the sink every command records for.
					let pipeline_manager_commands: SmallVec<[SmallVec<[(SinkId, RenderPassReturn<'_>); 16]>; 16]> = {
						let span = debug_span!("Renderer::prepare_pipeline_managers");
						let _enter = span.enter();
						pipeline_managers
							.iter_mut()
							.map(|pipeline_manager| pipeline_manager.prepare(frame, &sinks, frame_allocator, alpha, time))
							.collect()
					};

					// A list of render pass commands and their corresponding pass/sink indices.
					let render_pass_commands: SmallVec<[([Option<RenderPassReturn>; 2], RenderPassId, SinkId); 64]> = {
						let span = debug_span!("Renderer::prepare_render_passes");
						let _enter = span.enter();
						render_passes
							.iter_mut()
							.enumerate()
							.filter_map(|(render_pass_id, render_pass)| {
								let sink = sinks.iter().find(|sink| sink.index() == render_pass.sink)?;
								Some((render_pass.prepare(frame, sink, frame_allocator), render_pass_id, sink.index()))
							})
							.collect()
					};

					let scene_presentation_commands: SmallVec<[(Option<RenderPassReturn>, SinkId); 16]> = scene_presentation_copies
						.iter_mut()
						.filter_map(|(sink_id, copy)| {
							let sink = sinks.iter().find(|sink| sink.index() == *sink_id)?;
							Some((copy.prepare(frame, sink, frame_allocator), *sink_id))
						})
						.collect();

					let present_keys = swapchains
						.iter()
						.filter_map(|sc| sc.as_ref().map(|(pk, ..)| *pk))
						.collect::<SmallVec<[ghi::PresentKey; 16]>>();

					(
						sinks,
						pipeline_manager_commands,
						render_pass_commands,
						scene_presentation_commands,
						present_keys,
						first_uses,
					)
				};

				execution.record_with_present_keys(command_buffer, &present_keys, |command_buffer_recording| {
					let span = debug_span!("Renderer::record_commands", sinks = sinks.len());
					let _enter = span.enter();
					{
						let span = debug_span!("Renderer::record_pipeline_manager_commands");
						let _enter = span.enter();
						for (pipeline_manager_id, commands) in pipeline_manager_commands.iter().enumerate() {
							for sink in &sinks {
								let command = commands
									.iter()
									.find_map(|(sink_id, command)| (*sink_id == sink.index()).then_some(command));
								initialize_first_uses(
									&mut *command_buffer_recording,
									&first_uses,
									sink.index(),
									RenderNode::Scene(pipeline_manager_id),
									command.is_some(),
								);
								if let Some(command) = command {
									command(&mut *command_buffer_recording);
								}
							}
						}
					}

					for (request_index, capture) in screenshot_captures.iter().enumerate() {
						let transfer = match capture {
							Ok(ResolvedScreenshotCapture::AfterScene { target }) => {
								command_buffer_recording.transfer_texture(*target)
							}
							// No pass writes the previous frame's copy, so any point in the frame reads the same data.
							Ok(ResolvedScreenshotCapture::PreviousFrame { target }) => {
								command_buffer_recording.transfer_texture_with_frame(*target, -1)
							}
							_ => continue,
						};
						screenshot_transfers[request_index] = Some(transfer.map_err(RendererScreenshotError::Transfer));
					}

					{
						let span = debug_span!("Renderer::record_render_pass_commands");
						let _enter = span.enter();
						for (commands, render_pass_id, sink) in render_pass_commands {
							initialize_first_uses(
								&mut *command_buffer_recording,
								&first_uses,
								sink,
								RenderNode::Pass(render_pass_id),
								commands.iter().any(Option::is_some),
							);
							for command in commands.into_iter().flatten() {
								command(&mut *command_buffer_recording);
							}
							for request_index in captures_after_pass(&screenshot_captures, render_pass_id) {
								let Ok(ResolvedScreenshotCapture::AfterPass { target, .. }) = screenshot_captures[request_index]
								else {
									unreachable!();
								};
								screenshot_transfers[request_index] = Some(
									command_buffer_recording
										.transfer_texture(target)
										.map_err(RendererScreenshotError::Transfer),
								);
							}
						}
					}

					for (command, sink_id) in scene_presentation_commands {
						initialize_first_uses(
							&mut *command_buffer_recording,
							&first_uses,
							sink_id,
							RenderNode::Presentation,
							command.is_some(),
						);
						if let Some(command) = command {
							command(&mut *command_buffer_recording);
						}
					}

					// Final captures remain after every pass; duplicate requests receive independent transfer handles.
					for (request_index, capture) in screenshot_captures.iter().enumerate() {
						let transfer = match capture {
							Err(error) => Some(Err(*error)),
							Ok(ResolvedScreenshotCapture::FinalSwapchain { sink }) => Some(match swapchains.get(*sink) {
								None => Err(RendererScreenshotError::SinkNotFound),
								Some(None) => Err(RendererScreenshotError::SinkUnavailable),
								Some(Some((_present_key, _extent, swapchain))) => command_buffer_recording
									.transfer_texture(ghi::ImageOrSwapchain::Swapchain(*swapchain))
									.map_err(RendererScreenshotError::Transfer),
							}),
							Ok(
								ResolvedScreenshotCapture::AfterPass { .. }
								| ResolvedScreenshotCapture::AfterScene { .. }
								| ResolvedScreenshotCapture::PreviousFrame { .. },
							) => None,
						};
						if transfer.is_some() {
							screenshot_transfers[request_index] = transfer;
						}
					}
				});

				present_keys
			});
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

	/// Takes pending compiler servers so the application can start them on owned threads with its resource manager.
	pub(crate) fn take_pipeline_compilation_servers(&mut self) -> Vec<crate::rendering::PipelineManagerServer> {
		std::mem::take(&mut self.pipeline_compilation_servers)
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
		let app = match self.app.take() {
			Some(app) => Ok(app),
			None => ghi::window::App::new("main_window"),
		};
		let window = app.and_then(|mut app| {
			let window = app.create_window(name, extent, features);
			self.app = Some(app);
			window
		});

		match window {
			Ok(window) => {
				let os_handles = window.os_handles();

				let swapchain_handle = {
					let swapchain_handle = self.context.bind_to_window(
						&os_handles,
						ghi::PresentationModes::FIFO,
						extent,
						ghi::Uses::RenderTarget | ghi::Uses::Storage | ghi::Uses::TransferSource,
					);
					if self.present_interval.is_some() {
						self.context.set_present_interval(swapchain_handle, self.present_interval);
					}
					swapchain_handle
				};

				let sink_id = self.windows.len();

				let sink_has_camera = if let Some(camera) = camera {
					self.sink_cameras.push((sink_id, *camera));
					true
				} else {
					false
				};

				self.windows.push((window, swapchain_handle, false));
				self.redraw_requested = true;

				if sink_has_camera {
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
/// Gives the render targets a node uses first new contents before the node records.
///
/// A node that records commands writes its targets itself, so they are only discarded. A node that records nothing,
/// for example while its pipeline compiles, leaves them cleared, so later nodes never read another target's memory.
fn initialize_first_uses(
	recording: &mut ghi::implementation::CommandBufferRecording,
	first_uses: &[(SinkId, FirstUse)],
	sink: SinkId,
	node: RenderNode,
	records: bool,
) {
	let first_uses = first_uses
		.iter()
		.filter(|(first_use_sink, first_use)| *first_use_sink == sink && first_use.node == node)
		.map(|(_, first_use)| first_use);
	if records {
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
}

/// Returns request slots transferred immediately after one prepared pass entry.
pub(super) fn captures_after_pass(
	captures: &[Result<ResolvedScreenshotCapture, RendererScreenshotError>],
	pass: RenderPassId,
) -> impl Iterator<Item = usize> + '_ {
	captures.iter().enumerate().filter_map(move |(index, capture)| {
		matches!(capture, Ok(ResolvedScreenshotCapture::AfterPass { pass: capture_pass, .. }) if *capture_pass == pass)
			.then_some(index)
	})
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

use std::collections::VecDeque;

use ghi::{
	command_buffer::CommandBufferRecording,
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
	rendering::{
		Camera, Sink, make_perspective_view_from_camera,
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
