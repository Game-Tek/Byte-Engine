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

/// The [`Renderer`] struct owns graphics queues, render targets, scene pipelines,
/// and per-sink render passes.
///
/// Prefer the setup helpers in [`crate::application::graphics`] unless building
/// a custom headed runtime.
/// For custom composition, create the renderer, call [`Self::set_resource_manager`],
/// add a [`PipelineManager`] and sink-local [`RenderPass`] values, then register
/// windows and cameras before calling [`Self::prepare`] each frame.
/// See the [rendering guide](https://byte-engine.0x44491229.dev/docs/develop/design/rendering)
/// before composing a custom render system, domain, or model.
pub struct Renderer {
	/// The GHI context where all rendering resources and operations are performed.
	context: ghi::implementation::Context,
	/// The GHI device that is used for rendering.
	device: Arc<ghi::implementation::Device>,
	/// The GHI instance that manages devices.
	instance: ghi::implementation::Instance,

	/// The monotonically increasing identity of the next graphics submission frame.
	started_frame_count: u64,

	frame_queue_depth: usize,

	/// Display windows and their swapchains.
	windows: SmallVec<[(ghi::Window, ghi::SwapchainHandle); 16]>,
	/// Sink indices and their camera handles.
	sink_cameras: SmallVec<[(SinkId, Handle); 16]>,
	/// Cameras and their stable handles.
	cameras: SmallVec<[(Handle, Camera, Transform); 16]>,

	render_targets: RenderTargets,
	resource_manager: Option<crate::core::entity::handle::WeakHandle<ResourceManager>>,
	#[cfg(debug_assertions)]
	resource_updates: Option<resource_management::resource::ResourceUpdateListener>,

	render_passes: SmallVec<[RenderPassHarness; 64]>,
	render_passes_by_sink: SmallVec<[(RenderPassId, SinkId); 32]>,
	render_pass_writable_targets: SmallVec<[Vec<(String, ghi::BaseImageHandle)>; 64]>,
	post_scene_render_pass_factories: SmallVec<[Box<RenderPassFactory>; 16]>,
	pending_sink_initializations: SmallVec<[SinkId; 16]>,
	configuration: ConfigurationPort,
	pending_configuration: VecDeque<PendingRenderPassConfiguration>,
	render_pass_states: HashMap<String, RenderPassState>,

	pipeline_managers: SmallVec<[Box<dyn PipelineManager>; 16]>,
	pipeline_manager_resources_by_sink: SmallVec<[(PipelineManagerId, SinkId, Vec<(String, ghi::AccessPolicies)>); 64]>,
	pipeline_compilation_client: crate::rendering::PipelineManagerClient,
	pipeline_compilation_manager: crate::rendering::pipeline_compilation::PipelineManager,
	pipeline_compilation_servers: Vec<crate::rendering::PipelineManagerServer>,

	/// The GHI queue where graphics commands are submitted. The main rendering operations occur on this queue.
	graphics_queue_handle: ghi::QueueHandle,

	render_command_buffer: ghi::CommandBufferHandle,
	resource_upload_command_buffer: ghi::CommandBufferHandle,
	render_finished_synchronizer: ghi::SynchronizerHandle,
	defer_first_frame_sink_setup: bool,
}

impl Renderer {
	/// Creates a renderer from application configuration parameters.
	///
	/// # Parameters
	/// - `render.debug`: Enables validation layers for debugging. Defaults to true on debug builds.
	/// - `render.debug.dump`: Enables API dump for debugging. Defaults to false.
	/// - `render.debug.extended`: Enables extended validation for debugging. Defaults to false.
	/// - `render.debug.labels`: Enables graphics API object labels and command debug groups. Defaults to `render.debug`.
	/// - `render.ghi.features.mesh-shading`: Enables mesh shading features on the graphics context. Defaults to true.
	/// - `render.startup.defer-sink-setup`: Presents the first window frame before constructing sink render pipelines.
	///   Defaults to false.
	///
	/// Next, call [`Self::set_resource_manager`] before adding pipeline managers or
	/// render passes that load resources.
	pub fn new(parameters: &dyn Parameters, configuration: &Configuration) -> Self {
		let settings = Settings::new();

		let settings = if let Some(param) = parameters.get_parameter("render.debug") {
			settings.validation(param.as_bool_simple())
		} else {
			settings
		};

		let settings = if let Some(param) = parameters.get_parameter("render.debug.dump") {
			settings.api_dump(param.as_bool_simple())
		} else {
			settings
		};

		let settings = if let Some(param) = parameters.get_parameter("render.debug.extended") {
			settings.extended_validation(param.as_bool_simple())
		} else {
			settings
		};

		let settings = if let Some(param) = parameters.get_parameter("render.debug.labels") {
			settings.debug_labels(param.as_bool_simple())
		} else {
			let validation = settings.validation;
			settings.debug_labels(validation)
		};

		let settings = if let Some(param) = parameters.get_parameter("render.ghi.features.mesh-shading") {
			settings.mesh_shading(param.as_bool_simple())
		} else {
			settings
		};
		let defer_first_frame_sink_setup = parameters
			.get_parameter("render.startup.defer-sink-setup")
			.map(|parameter| parameter.as_bool_simple())
			.unwrap_or(false);

		let mut features = ghi::device::Features::new()
			.validation(settings.validation)
			.api_dump(settings.api_dump)
			.gpu_validation(settings.extended_validation)
			.debug_labels(settings.debug_labels)
			.debug_log_function(|message| {
				let backtrace = std::backtrace::Backtrace::force_capture().to_string();
				let manifest_dir = env!("CARGO_MANIFEST_DIR");
				let workspace_root = manifest_dir
					.rsplit_once("/crates/")
					.map(|(root, _)| root)
					.unwrap_or(manifest_dir);

				let mut filtered = String::new();
				for line in backtrace.lines() {
					if line.contains(workspace_root) {
						filtered.push_str(line);
						filtered.push('\n');
					}
				}

				if filtered.trim().is_empty() {
					log::error!("{}\n{}", message, backtrace);
				} else {
					log::error!("{}\n{}", message, filtered.trim_end());
				}
			})
			.geometry_shader(false)
			.mesh_shading(settings.mesh_shading);

		let mut instance = match ghi::implementation::Instance::new(features) {
			Ok(instance) => instance,
			Err(error) if settings.validation => {
				log::warn!(
					"Renderer validation was requested but could not be enabled: {error} Falling back to renderer validation disabled. The most likely cause is missing or unsupported platform graphics tooling. See {}.",
					crate::online_docs_url("use/setup/environment")
				);
				features = features
					.validation(false)
					.gpu_validation(false)
					.api_dump(false)
					.debug_labels(false);
				ghi::implementation::Instance::new(features).unwrap()
			}
			Err(error) => panic!("Failed to create GHI instance: {error}"),
		};

		let mut graphics_queue_handle = None;

		let device = instance
			.create_device(
				features,
				&mut [(
					ghi::QueueSelection::new(ghi::types::WorkloadTypes::RASTER),
					&mut graphics_queue_handle,
				)],
			)
			.unwrap();
		let mut context = device.create_context().unwrap();
		let frame_queue_depth = 2;
		context.set_frames_in_flight(frame_queue_depth);
		let pipeline_compilation_server_count = parameters
			.get_parameter("render.pipeline-compilation.threads")
			.and_then(|parameter| parameter.value().parse::<usize>().ok())
			.unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |count| (count.get() / 2).clamp(1, 4)));
		let (pipeline_compilation_client, pipeline_compilation_manager, pipeline_compilation_servers) =
			crate::rendering::pipeline_compilation::PipelineManager::new(&mut context, pipeline_compilation_server_count);

		let graphics_queue_handle = graphics_queue_handle.unwrap();

		let render_command_buffer = context.queue(graphics_queue_handle).create_command_buffer(Some("Render"));
		let resource_upload_command_buffer = context
			.queue(graphics_queue_handle)
			.create_command_buffer(Some("Resource Upload"));
		let render_finished_synchronizer = context.create_synchronizer(Some("Render Finisished"), true);

		Renderer {
			context,
			device: Arc::new(device),
			instance,

			started_frame_count: 0,

			frame_queue_depth: frame_queue_depth as usize,

			windows: SmallVec::with_capacity(16),
			sink_cameras: SmallVec::with_capacity(16),
			cameras: SmallVec::with_capacity(16),

			render_targets: RenderTargets::new(),
			resource_manager: None,
			#[cfg(debug_assertions)]
			resource_updates: None,

			render_passes: SmallVec::with_capacity(64),
			render_passes_by_sink: SmallVec::with_capacity(32),
			render_pass_writable_targets: SmallVec::with_capacity(64),
			post_scene_render_pass_factories: SmallVec::with_capacity(16),
			pending_sink_initializations: SmallVec::with_capacity(16),
			configuration: configuration.register(RENDER_PASS_PARAMETER_PREFIX),
			pending_configuration: VecDeque::new(),
			render_pass_states: HashMap::new(),

			pipeline_managers: SmallVec::with_capacity(8),
			pipeline_manager_resources_by_sink: SmallVec::with_capacity(64),
			pipeline_compilation_client,
			pipeline_compilation_manager,
			pipeline_compilation_servers,

			graphics_queue_handle,

			render_command_buffer,
			resource_upload_command_buffer,
			render_finished_synchronizer,
			defer_first_frame_sink_setup,
		}
	}

	/// Supplies the externally owned resource manager used by render passes to resolve baked shaders.
	///
	/// The renderer retains a weak reference so it does not extend application-owned resource lifetimes.
	/// Debug asset management is installed through the resource manager's one-time initialization seam.
	/// The owner must keep `resource_manager` alive for as long as render passes may load resources.
	/// Connects the renderer to the resource manager used by pipelines and passes.
	///
	/// Next, call [`Self::add_pipeline_manager`] and register sink-local render
	/// passes before creating windows.
	pub fn set_resource_manager(&mut self, resource_manager: &EntityHandle<ResourceManager>) {
		self.resource_manager = Some(resource_manager.weak());
		#[cfg(debug_assertions)]
		{
			self.resource_updates = Some(resource_manager.resource_updates());
		}
		for server in &mut self.pipeline_compilation_servers {
			server.set_resource_manager(resource_manager.clone());
		}
	}

	/// Registers a scene pipeline manager with the renderer.
	///
	/// Next, add post-scene passes with
	/// [`Self::add_post_scene_render_pass_for_all_sinks`] or create a window with
	/// [`Self::create_window`].
	pub fn add_pipeline_manager(&mut self, mut pipeline_manager: impl PipelineManager + 'static) {
		let pipeline_manager_id = self.pipeline_managers.len();
		{
			let sink_swapchains: SmallVec<[(SinkId, ghi::SwapchainHandle); 16]> = self
				.sink_cameras
				.iter()
				.map(|(sink_id, _)| (*sink_id, self.windows[*sink_id].1))
				.collect();
			for (sink_id, swapchain) in sink_swapchains {
				if self.pending_sink_initializations.contains(&sink_id) {
					continue;
				}

				let mut rpb = RenderPassBuilder::new(
					&mut self.context,
					&mut self.render_targets,
					sink_id,
					swapchain,
					self.pipeline_compilation_client.clone(),
				);
				pipeline_manager.create_sink(sink_id, &mut rpb);
				let consumed_resources = rpb
					.consumed_resources
					.iter()
					.map(|(name, access)| ((*name).to_string(), *access))
					.collect();
				self.pipeline_manager_resources_by_sink
					.push((pipeline_manager_id, sink_id, consumed_resources));

				if rpb.consumed_resources.is_empty() {
					log::debug!("No resources consumed by scene manager");
				}
			}
		}

		self.pipeline_managers.push(Box::new(pipeline_manager));
	}

	fn initialize_scene_sink(&mut self, sink_id: SinkId) {
		let swapchain = self.windows[sink_id].1;

		{
			let Renderer {
				context,
				render_targets,
				pipeline_managers,
				pipeline_manager_resources_by_sink,
				pipeline_compilation_client,
				..
			} = self;

			for (pipeline_manager_id, sm) in pipeline_managers.iter_mut().enumerate() {
				let mut rpb = RenderPassBuilder::new(
					context,
					render_targets,
					sink_id,
					swapchain,
					pipeline_compilation_client.clone(),
				);
				sm.create_sink(sink_id, &mut rpb);
				let consumed_resources = rpb
					.consumed_resources
					.iter()
					.map(|(name, access)| ((*name).to_string(), *access))
					.collect();
				pipeline_manager_resources_by_sink.push((pipeline_manager_id, sink_id, consumed_resources));

				if rpb.consumed_resources.is_empty() {
					log::debug!("No resources consumed by scene manager");
				}
			}
		}

		self.add_post_scene_render_passes_for_sink(sink_id);
	}

	fn initialize_pending_sink_resources(&mut self) {
		let pending_sink_initializations = std::mem::take(&mut self.pending_sink_initializations);
		for sink_id in pending_sink_initializations {
			self.initialize_scene_sink(sink_id);
		}
	}

	fn add_render_pass(
		&mut self,
		render_pass: Box<dyn RenderPass>,
		sink_id: SinkId,
		writable_targets: Vec<(String, ghi::BaseImageHandle)>,
	) {
		let render_pass_id = self.render_passes.len();
		self.render_passes
			.push(render_pass_harness_with_state(render_pass, &self.render_pass_states));
		self.render_passes_by_sink.push((render_pass_id, sink_id));
		self.render_pass_writable_targets.push(writable_targets);
	}

	/// Changes the state of every sink-local render pass with the requested stable name.
	///
	/// Returns the number of updated instances. A return value of `0` means that no registered render pass uses
	/// `name`. Pass names come from [`RenderPass::name`].
	pub fn set_render_pass_state(&mut self, name: &str, state: RenderPassState) -> usize {
		self.render_pass_states.insert(name.to_string(), state);
		set_render_pass_state_by_name(&mut self.render_passes, name, state)
	}

	/// Applies queued render-pass configuration after passes exist and before they prepare frame work.
	fn apply_configuration(&mut self) {
		apply_render_pass_configuration(
			&self.configuration,
			&mut self.pending_configuration,
			&mut self.render_pass_states,
			&mut self.render_passes,
		);
	}

	/// Registers a render pass factory that will be instantiated for every current and future sink.
	pub fn add_post_scene_render_pass_for_all_sinks<F>(&mut self, render_pass_factory: F)
	where
		F: for<'builder, 'resources> Fn(&'builder mut RenderPassBuilder<'resources>) -> Box<dyn RenderPass> + 'static,
	{
		let render_pass_factory: Box<RenderPassFactory> = Box::new(render_pass_factory);
		let sink_ids: SmallVec<[usize; 16]> = self.sink_cameras.iter().map(|(sink_id, _)| *sink_id).collect();

		for sink_id in sink_ids {
			let (render_pass, writable_targets) = {
				let swapchain = self.windows[sink_id].1;
				let mut render_pass_builder = RenderPassBuilder::new(
					&mut self.context,
					&mut self.render_targets,
					sink_id,
					swapchain,
					self.pipeline_compilation_client.clone(),
				);
				let render_pass = render_pass_factory(&mut render_pass_builder);
				let writable_targets = render_pass_builder.writable_targets();
				(render_pass, writable_targets)
			};

			self.add_render_pass(render_pass, sink_id, writable_targets);
		}

		self.post_scene_render_pass_factories.push(render_pass_factory);
	}

	/// Instantiates all registered post-scene render pass factories for a given sink.
	fn add_post_scene_render_passes_for_sink(&mut self, sink_id: SinkId) {
		let mut render_passes_for_sink: SmallVec<[(Box<dyn RenderPass>, Vec<(String, ghi::BaseImageHandle)>); 16]> =
			SmallVec::new();

		let swapchain = self.windows[sink_id].1;

		for render_pass_factory in &self.post_scene_render_pass_factories {
			let render_pass = {
				let mut render_pass_builder = RenderPassBuilder::new(
					&mut self.context,
					&mut self.render_targets,
					sink_id,
					swapchain,
					self.pipeline_compilation_client.clone(),
				);
				let render_pass = render_pass_factory(&mut render_pass_builder);
				(render_pass, render_pass_builder.writable_targets())
			};

			render_passes_for_sink.push(render_pass);
		}

		for (render_pass, writable_targets) in render_passes_for_sink {
			self.add_render_pass(render_pass, sink_id, writable_targets);
		}
	}

	pub fn update_windows<'a>(&'a mut self) -> impl Iterator<Item = impl Iterator<Item = ghi::window::Events> + 'a> + 'a {
		self.windows.iter_mut().map(|(window, _)| window.poll())
	}

	/// Prepares a frame by invoking the configured render passes.
	///
	/// The renderer skips execution when no swapchain is available or when any
	/// swapchain surface has a zero-sized dimension.
	pub(crate) fn prepare(
		&'_ mut self,
		transforms_listener: &mut impl Listener<TransformationUpdate>,
		frame_allocator: &bumpalo::Bump,
		screenshot_requests: &[(usize, &crate::inspector::screenshot::ScreenshotCapture)],
	) -> Vec<Result<(u64, ghi::TextureReadback), RendererScreenshotError>> {
		let span = debug_span!(
			"Renderer::prepare",
			frame = self.started_frame_count,
			windows = self.windows.len()
		);
		let _enter = span.enter();

		let Some(_) = self.windows.first() else {
			log::debug!("No swapchains available to present to. Skipping rendering!");
			return screenshot_requests
				.iter()
				.map(|_| Err(RendererScreenshotError::SinkNotFound))
				.collect();
		};
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
				let handle = *message.handle();

				if let Some((camera, transform)) =
					self.cameras
						.iter_mut()
						.find_map(|(h, camera, transform)| if handle == *h { Some((camera, transform)) } else { None })
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
		let resource_upload_command_buffer = self.resource_upload_command_buffer;
		let synchronizer = self.render_finished_synchronizer;
		let wait_for = &[];
		let windows = &self.windows;
		let sink_cameras = &self.sink_cameras;
		let cameras = &self.cameras;
		let render_targets = &self.render_targets;
		let pipeline_managers = &mut self.pipeline_managers;
		let pipeline_compilation_client = &self.pipeline_compilation_client;
		let pipeline_compilation_manager = &mut self.pipeline_compilation_manager;
		#[cfg(debug_assertions)]
		let resource_updates = &self.resource_updates;
		let pipeline_manager_resources_by_sink = &self.pipeline_manager_resources_by_sink;
		let render_passes = &mut self.render_passes;
		let render_passes_by_sink = &self.render_passes_by_sink;
		let frame_allocator = frame_allocator;
		let submitted_frame = self.started_frame_count - 1;
		let mut screenshot_transfers = (0..screenshot_captures.len()).map(|_| None).collect::<Vec<_>>();

		{
			let span = debug_span!("Renderer::queue_execute");
			let _enter = span.enter();
			queue.execute(Some(frame), wait_for, synchronizer, |execution| {
				let completed_graphics_frame = execution.completed_frame();
				let frame_key = execution
					.frame()
					.expect(
						"Frame is required to record renderer uploads. The most likely cause is that Renderer::prepare called Queue::execute without a frame request.",
					)
					.key();
				let mut has_frame_uploads = false;
				for pipeline_manager in pipeline_managers.iter_mut() {
					has_frame_uploads |= pipeline_manager.begin_frame(completed_graphics_frame);
				}
				if has_frame_uploads {
					execution.record(resource_upload_command_buffer, |recording| {
						for pipeline_manager in pipeline_managers.iter_mut() {
							pipeline_manager.record_frame_uploads(frame_key, &mut *recording);
						}
					});
				}
				#[cfg(debug_assertions)]
				if let Some(resource_updates) = resource_updates {
					while let Some(update) = resource_updates.read() {
						pipeline_compilation_client.resource_updated(update.id());
					}
				}
				pipeline_compilation_manager.publish(execution.frame().expect(
					"Frame is required to publish compiled pipelines. The most likely cause is that Renderer::prepare called Queue::execute without a frame request.",
				));

				let (sinks, pipeline_manager_commands, render_pass_commands, present_keys, swapchains) = {
					let span = debug_span!("Renderer::prepare_frame_work");
					let _enter = span.enter();
					let frame = execution.frame().expect(
					"Frame is required to prepare renderer frame work. The most likely cause is that Renderer::render called Queue::execute without a frame request.",
				);
					let swapchains: SmallVec<[Option<(ghi::PresentKey, Extent, ghi::SwapchainHandle)>; 16]> = {
						let span = debug_span!("Renderer::acquire_swapchains", count = windows.len());
						let _enter = span.enter();
						windows
							.iter()
							.map(|(_window, swapchain)| {
								let (present_key, extent) = frame.acquire_swapchain_image(*swapchain);

								if extent.width() == 0 || extent.height() == 0 {
									log::warn!("The extent is too small: {:?}. Rendering will be skipped.", extent);
									return None;
								}

								if extent.width() >= 65535 || extent.height() >= 65535 {
									log::warn!(
										"The extent is too large: {:?}. The renderer only supports dimensions as big as 16 bits. Rendering will be skipped.",
										extent
									);
									return None;
								}

								Some((present_key, extent, *swapchain))
							})
							.collect()
					};

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
							sinks.push(Sink::new(view, extent, *sink_id));
						}
					}

					{
						let span = debug_span!("Renderer::resize_render_targets", sinks = sinks.len());
						let _enter = span.enter();
						for sink in &sinks {
							// Get images for the current sink and render pass and resize them to window extent
							let images = render_targets.get_images_for_sink(sink.index());

							// Resize images to window extent
							for &image in images {
								frame.resize_image(image, sink.extent());
							}
						}
					}

					let pipeline_managers = pipeline_managers.iter_mut().enumerate();

					let pipeline_manager_commands: SmallVec<[(PipelineManagerId, SmallVec<[RenderPassReturn<'_>; 16]>); 16]> = {
						let span = debug_span!("Renderer::prepare_pipeline_managers");
						let _enter = span.enter();
						pipeline_managers
							.filter_map(|(pipeline_manager_id, sm)| {
								sm.prepare(frame, &sinks, frame_allocator).map(|commands| (pipeline_manager_id, commands))
							})
							.collect()
					};

					// A list of render pass commands and their corresponding pass/sink indices.
					let render_pass_commands: SmallVec<[(Option<RenderPassReturn>, RenderPassId, SinkId); 64]> = {
						let span = debug_span!("Renderer::prepare_render_passes");
						let _enter = span.enter();
						render_passes_by_sink
							.iter()
							.filter_map(|(render_pass_id, sink_id)| {
								let render_pass = render_passes.get_mut(*render_pass_id)?;
								let sink = sinks.iter().find(|sink| sink.index() == *sink_id)?;
								Some((render_pass.prepare(frame, sink, frame_allocator), *render_pass_id, sink.index()))
							})
							.collect()
					};

					let present_keys = swapchains
						.iter()
						.filter_map(|sc| sc.as_ref().map(|(pk, ..)| *pk))
						.collect::<SmallVec<[ghi::PresentKey; 16]>>();

					(sinks, pipeline_manager_commands, render_pass_commands, present_keys, swapchains)
				};

				execution.record_with_present_keys(command_buffer, &present_keys, |command_buffer_recording| {
					let span = debug_span!("Renderer::record_commands", sinks = sinks.len());
					let _enter = span.enter();
					{
						let span = debug_span!("Renderer::record_pipeline_manager_commands");
						let _enter = span.enter();
						for (pipeline_manager_id, commands) in pipeline_manager_commands {
							for (command, sink) in commands.into_iter().zip(sinks.iter()) {
								let attachment_infos = render_targets.get_attachment_infos_for_resources(
									sink.index(),
									pipeline_manager_resources_by_sink
										.iter()
										.find_map(|(id, sink_id, resources)| {
											(*id == pipeline_manager_id && *sink_id == sink.index()).then_some(resources.as_slice())
										})
										.unwrap_or(&[]),
								);

								command(&mut *command_buffer_recording, &attachment_infos);
							}
						}
					}

					{
						let span = debug_span!("Renderer::record_render_pass_commands");
						let _enter = span.enter();
						for (command, render_pass_id, sink) in render_pass_commands {
							if let Some(command) = command {
								let attachment_infos = render_targets.get_attachment_infos(sink);
								command(&mut *command_buffer_recording, &attachment_infos);
							}
							for request_index in captures_after_pass(&screenshot_captures, render_pass_id) {
								let Ok(ResolvedScreenshotCapture::AfterPass { image, .. }) = screenshot_captures[request_index]
								else {
									unreachable!();
								};
								screenshot_transfers[request_index] = Some(
									command_buffer_recording
										.transfer_texture(image.into())
										.map_err(RendererScreenshotError::Transfer),
								);
							}
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
							Ok(ResolvedScreenshotCapture::AfterPass { .. }) => None,
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

		screenshot_transfers
			.into_iter()
			.map(|transfer| {
				let handle = transfer.unwrap_or(Err(RendererScreenshotError::SinkUnavailable))?;
				self.context
					.get_image_data(handle)
					.map(|readback| (submitted_frame, readback))
					.map_err(RendererScreenshotError::Transfer)
			})
			.collect()
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
		let ScreenshotCapture::AfterPass { pass, target } = capture else {
			return Ok(ResolvedScreenshotCapture::FinalSwapchain { sink });
		};
		let mut matches = self
			.render_passes_by_sink
			.iter()
			.filter(|(id, pass_sink)| *pass_sink == sink && self.render_passes[*id].name() == pass);
		let Some((pass_id, _)) = matches.next() else {
			return Err(RendererScreenshotError::PassNotFound);
		};
		if matches.next().is_some() {
			return Err(RendererScreenshotError::PassAmbiguous);
		}
		let image = self.render_pass_writable_targets[*pass_id]
			.iter()
			.rev()
			.find_map(|(name, image)| (name == target).then_some(*image))
			.ok_or(RendererScreenshotError::TargetNotWritten)?;
		Ok(ResolvedScreenshotCapture::AfterPass { pass: *pass_id, image })
	}

	pub fn context_mut(&mut self) -> &mut ghi::implementation::Context {
		&mut self.context
	}

	/// Returns a client for requesting renderer-owned asynchronous pipelines.
	pub fn pipeline_manager_client(&self) -> crate::rendering::PipelineManagerClient {
		self.pipeline_compilation_client.clone()
	}

	/// Takes pending compiler servers so an application setup function can start them on owned threads.
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

		let window = ghi::Window::new_with_params(name, extent, "main_window", features);

		match window {
			Ok(window) => {
				let os_handles = window.os_handles();

				let swapchain_handle = self.context.bind_to_window(
					&os_handles,
					ghi::PresentationModes::FIFO,
					extent,
					ghi::Uses::RenderTarget | ghi::Uses::Storage | ghi::Uses::TransferSource,
				);

				let sink_id = self.windows.len();

				let sink_has_camera = if let Some(camera) = camera {
					self.sink_cameras.push((sink_id, *camera));
					true
				} else {
					false
				};

				self.windows.push((window, swapchain_handle));

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
			return;
		}

		self.cameras.push((handle, camera, Transform::default()));
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
		image: ghi::BaseImageHandle,
	},
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RendererScreenshotError {
	SinkNotFound,
	SinkUnavailable,
	PassNotFound,
	PassAmbiguous,
	TargetNotWritten,
	Transfer(ghi::TextureTransferError),
}

/// The `Settings` struct configures a [`Renderer`] during creation.
pub struct Settings {
	/// Controls whether the GHI context enables validation layers.
	validation: bool,
	/// Controls whether the renderer logs parameters sent to the underlying graphics API.
	///
	/// This option requires `validation`.
	api_dump: bool,
	/// Controls whether the graphics API performs additional validation, including
	/// GPU validation.
	///
	/// This option can be expensive and requires `validation`.
	extended_validation: bool,
	/// Controls whether graphics API object labels and command debug groups are emitted.
	debug_labels: bool,
	/// Controls whether the GHI context enables mesh shading.
	mesh_shading: bool,
}

impl Settings {
	/// Creates renderer settings with the engine defaults.
	///
	/// - `validation` is true by default in debug builds and false in release.
	/// - `api_dump` is false by default.
	/// - `extended_validation` is false by default.
	pub fn new() -> Self {
		Self {
			validation: cfg!(debug_assertions),
			api_dump: false,
			extended_validation: false,
			debug_labels: cfg!(debug_assertions),
			mesh_shading: true,
		}
	}

	pub fn validation(mut self, value: bool) -> Self {
		self.validation = value;
		self
	}

	pub fn api_dump(mut self, value: bool) -> Self {
		self.api_dump = value;
		self
	}

	pub fn extended_validation(mut self, value: bool) -> Self {
		self.extended_validation = value;
		self
	}

	pub fn debug_labels(mut self, value: bool) -> Self {
		self.debug_labels = value;
		self
	}

	pub fn mesh_shading(mut self, value: bool) -> Self {
		self.mesh_shading = value;
		self
	}
}

use std::{
	collections::VecDeque,
	io::Write,
	ops::{Deref, DerefMut},
	rc::Rc,
	sync::Arc,
};

use ghi::{
	command_buffer::{
		BoundComputePipelineMode as _, BoundRasterizationPipelineMode as _, CommandBufferRecording,
		RasterizationRenderPassMode as _,
	},
	context::{Context as _, ContextCreate as _},
	device::Device as _,
	frame::Frame as _,
	queue::{Queue as _, QueueExecution as _},
};
use resource_management::resource::resource_manager::ResourceManager;
use smallvec::SmallVec;
use tracing::debug_span;
use utils::Box;
use utils::{
	hash::{HashMap, HashMapExt},
	sync::RwLock,
	Extent, RGBA,
};

use super::{
	configuration::{
		apply_render_pass_configuration, render_pass_harness_with_state, set_render_pass_state_by_name,
		PendingRenderPassConfiguration, RENDER_PASS_PARAMETER_PREFIX,
	},
	targets::RenderTargets,
};
use crate::{
	application::parameters::Parameters,
	configuration::{Configuration, ConfigurationPort},
	core::{
		channel::{Channel, DefaultChannel},
		factory::Handle,
		listener::Listener,
		Entity, EntityHandle,
	},
	gameplay::transform::TransformationUpdate,
	rendering::{
		make_perspective_view_from_camera,
		pipeline_manager::PipelineManager,
		render_pass::{FramePrepare, RenderPassReturn},
		window::{self, Window},
		Camera, Sink, View,
	},
	space::Orientable as _,
};
use crate::{
	gameplay::Transform,
	rendering::render_pass::{RenderPass, RenderPassBuilder, RenderPassHarness, RenderPassState},
};
