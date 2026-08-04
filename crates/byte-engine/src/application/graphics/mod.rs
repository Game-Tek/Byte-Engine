//! Headed application runtime and graphics setup entry points.
//!
//! Construct [`GraphicsApplication`], configure it with either [`default_setup`]
//! or selected setup functions, then run its loop. The `triangle` example uses
//! the complete default stack; the `window` example creates only a window.
//!
//! Rendering setup remains in this module because it coordinates the world,
//! renderer, and application factories. General startup defaults and external
//! adapters are kept behind the setup functions re-exported from this module.
//!
//! Follow the [sample project guide](https://byte-engine.0x44491229.dev/docs/use/sample-project)
//! for the complete application setup sequence.

// Bound ready work while the temporary Compio runtime still shares the application thread.
const ASYNC_TASK_POLL_BUDGET_PER_TICK: usize = 8;

/// The [`GraphicsApplication`] struct owns the headed runtime and coordinates
/// windows, input, worlds, resources, audio workers, and rendering.
///
/// Use [`default_setup`] for the conventional engine stack. Use
/// [`setup_default_window`], [`setup_default_input`], and the render-pass setup
/// functions independently when an application needs explicit composition.
/// After setup, call [`Self::do_loop`] to run the application, or
/// [`Self::tick_with`] when application code must run during each tick.
///
/// # Configuration
/// - `kill-after`: Closes the application after this number of ticks. The default is `None`.
/// - `resources.path`: Selects the resource directory. The default is `./resources`.
/// - `render.debug`: Enables validation layers. The default is `true` in debug builds.
/// - `render.debug.dump`: Enables graphics API logging. The default is `false`.
/// - `render.debug.extended`: Enables extended validation. The default is `false`.
/// - `render.pass.<name>`: Selects `enabled` or `bypassed` for the named render pass.
/// - `render.gtao.radius`: Sets the GTAO world-space search radius. The default is `1.0`.
/// - `render.gtao.samples-per-ray`: Sets the GTAO samples along each ray. The default is `6`.
/// - `render.gtao.radial-rays`: Sets the even number of GTAO ray directions. The default is `8`.
///
/// See the [sample project guide](https://byte-engine.0x44491229.dev/docs/use/sample-project)
/// for a complete `GraphicsApplication` setup.
pub struct GraphicsApplication {
	application: BaseApplication,

	tick_count: u64,
	start_time: std::time::Instant,
	last_tick_time: MediaTime,

	close: bool,

	application_events: (Sender<Events>, Receiver<Events>),
	http_inspector: HttpInspectorServer,
	configuration: Configuration,

	window_factory: (Factory<Window>, DefaultListener<CreateMessage<Window>>),
	action_factory: Factory<Action>,

	generator_factory: Factory<Arc<dyn Generator>>,

	world_factory: Factory<DefaultWorld>,
	world: DefaultWorld,
	cameras_listener: DefaultListener<crate::core::factory::CreateMessage<Camera>>,
	renderer_transforms_listener: DefaultListener<TransformationUpdate>,

	input_system: input::InputManager,
	gamepad_system: Option<input::gamepad::GamepadSystem>,
	gamepad_device_class_handle: Option<input::device_class::DeviceClassHandle>,
	resource_manager: EntityHandle<ResourceManager>,
	renderer: Renderer,

	threads: SmallVec<[Thread; 64]>,

	#[cfg(debug_assertions)]
	ttff: MediaTime,
	#[cfg(debug_assertions)]
	min_frame_time: MediaTime,
	#[cfg(debug_assertions)]
	max_frame_time: MediaTime,

	#[cfg(debug_assertions)]
	kill_after: Option<u64>,
}

impl Application for GraphicsApplication {
	fn new(name: &str, parameters: &[Parameter]) -> Self {
		let start_time = std::time::Instant::now();

		let application = BaseApplication::new(name, parameters);

		let resources_path: std::path::PathBuf = application
			.get_parameter("resources.path")
			.map(|p| p.value.clone())
			.unwrap_or_else(|| "resources".into())
			.into();

		// Debug applications can update individual stale assets in their local resource database.
		#[cfg(debug_assertions)]
		let resource_storage = RedbStorageBackend::new_writable(resources_path);
		// Release resource directories are prepared by BELD and keep the resource-management signature it writes today.
		#[cfg(not(debug_assertions))]
		let resource_storage = RedbStorageBackend::new(resources_path);
		let resource_manager = EntityHandle::from(ResourceManager::new(resource_storage));

		let action_factory = Factory::new();

		let input_system = {
			let action_listener = action_factory.listener();
			let event_channel = DefaultChannel::new();

			input::InputManager::new(action_listener, event_channel)
		};
		// HID initialization and first enumeration can block startup on Windows, so gamepads are initialized after
		// the first frame has reached the screen.
		let gamepad_system = None;

		let configuration = Configuration::new();
		let mut renderer = rendering::renderer::Renderer::new(&application, &configuration);
		renderer.set_resource_manager(&resource_manager);
		queue_render_pass_startup_parameters(application.parameters(), &configuration);

		#[cfg(debug_assertions)]
		let kill_after = application
			.get_parameter("kill-after")
			.map(|p| p.value.parse::<u64>().unwrap());

		let tx = Sender::new(16);

		ctrlc::set_handler({
			let tx = tx.clone();
			move || {
				tx.send(Events::Close).unwrap();
			}
		})
		.unwrap();

		let inspector = EntityHandle::from(Inspector::new(tx.clone(), configuration.clone()));
		let http_inspector = HttpInspectorServer::new(inspector);

		let rx = tx.spawn_rx();
		let application_events = (tx, rx);

		let window_factory = Factory::new();
		let window_factory_listener = window_factory.listener();

		let world = DefaultWorld::new();
		let cameras_listener = world.camera_factory().listener();
		let renderer_transforms_listener = world.transforms_channel().listener();

		GraphicsApplication {
			application,

			application_events,
			http_inspector,
			configuration,

			window_factory: (window_factory, window_factory_listener),
			action_factory,

			generator_factory: Factory::new(),

			world_factory: Factory::new(),
			world,
			cameras_listener,
			renderer_transforms_listener,

			input_system,
			gamepad_system,
			gamepad_device_class_handle: None,
			resource_manager,
			renderer,

			threads: SmallVec::new(),

			close: false,

			tick_count: 0,
			start_time,
			last_tick_time: MediaTime::from_std(start_time.elapsed()),

			#[cfg(debug_assertions)]
			ttff: MediaTime::ZERO,
			#[cfg(debug_assertions)]
			min_frame_time: MediaTime::MAX,
			#[cfg(debug_assertions)]
			max_frame_time: MediaTime::ZERO,

			#[cfg(debug_assertions)]
			kill_after,
		}
	}

	fn get_name(&self) -> &str {
		self.application.get_name()
	}

	fn tick(&mut self) -> bool {
		self.tick_with(|_, _| {}).is_some()
	}
}

impl GraphicsApplication {
	/// Returns frame-local storage for temporary allocations during the current tick.
	pub fn frame_allocator(&self) -> &bumpalo::Bump {
		&self.application.frame_allocator
	}

	/// Returns the configuration exchange used to inspect startup update results.
	pub fn configuration(&self) -> &Configuration {
		&self.configuration
	}

	/// Runs one graphics tick and lets application code update state before rendering.
	pub fn tick_with<R, F: FnOnce(&mut Self, Time) -> R>(&mut self, f: F) -> Option<R> {
		let span = debug_span!("GraphicsApplication::tick");
		let _enter = span.enter();

		let now = std::time::Instant::now();
		// Sample the monotonic clock once, then keep application time entirely in
		// media ticks so elapsed time is exactly the sum of observed frame deltas.
		let elapsed = MediaTime::from_std(now.duration_since(self.start_time));
		let dt = elapsed - self.last_tick_time;
		self.last_tick_time = elapsed;
		let tick_count = self.tick_count;

		let mut close = false;

		{
			let span = debug_span!("GraphicsApplication::reset_frame_allocator");
			let _enter = span.enter();
			self.application.frame_allocator.reset();
		}

		{
			let span = debug_span!("GraphicsApplication::process_window_events");
			let _enter = span.enter();
			let renderer = &mut self.renderer;
			let input_system = &mut self.input_system;

			for window_events in renderer.update_windows() {
				for event in window_events {
					if let ghi::window::Events::Close = event {
						close = true;
					}

					if let Some((seat_handle, device_handle, input_source_action, value)) =
						process_default_window_input(input_system, event)
					{
						input_system.record_trigger_value_for_device(seat_handle, device_handle, input_source_action, value);
					}
				}
			}
		}

		{
			let span = debug_span!("GraphicsApplication::process_application_events");
			let _enter = span.enter();
			if let Ok(e) = self.application_events.1.try_recv() {
				match e {
					Events::Close => {
						close = true;
					}
				}
			}
		}

		{
			let span = debug_span!("GraphicsApplication::process_gamepad_events");
			let _enter = span.enter();
			if self.tick_count > 0 && self.gamepad_system.is_none() {
				self.gamepad_system = input::gamepad::GamepadSystem::new()
					.map_err(|error| log::warn!("{}", error))
					.ok();
			}
			if self.tick_count > 0 {
				if let Some(gamepad_system) = &mut self.gamepad_system {
					let (new_devices, events) = gamepad_system.poll();

					if let Some(gamepad_device_class_handle) = self.gamepad_device_class_handle {
						for (path, kind, device) in new_devices {
							// Each physical HID device gets its own input-system device so actions can
							// preserve player/device identity instead of collapsing into one gamepad.
							let device_handle = self.input_system.create_device(&gamepad_device_class_handle);
							gamepad_system.add_device(path, kind, device, device_handle);
						}
					} else if !new_devices.is_empty() {
						log::warn!(
							"Detected HID gamepad before the Gamepad device class was registered. The most likely cause is that setup_default_input was not called. See {}.",
							crate::online_docs_url("develop/design/input-handling")
						);
					}

					for event in events {
						log::debug!(
							target: "byte_engine::input::events",
							"Forwarding HID gamepad event: device={:?}, trigger={:?}, value={:?}",
							event.device_handle(),
							event.trigger(),
							event.value()
						);
						self.input_system.record_trigger_value_for_device(
							input::SeatHandle::stub(),
							event.device_handle(),
							event.trigger(),
							event.value(),
						);
					}
				}
			}
		}

		let time = Time { elapsed, delta: dt };

		{
			let span = debug_span!("GraphicsApplication::update_input");
			let _enter = span.enter();
			self.input_system.update(&self.application.frame_allocator);
		}

		let mut physics_transforms_listener = self.world.transforms_channel().listener();

		let result = {
			let span = debug_span!("GraphicsApplication::user_tick");
			let _enter = span.enter();
			f(self, time)
		};

		{
			let span = debug_span!("GraphicsApplication::update_world");
			let _enter = span.enter();
			self.world
				.update(time, &mut physics_transforms_listener, &mut self.application.frame_allocator);
		}

		{
			let span = debug_span!("GraphicsApplication::prepare_renderer_state");
			let _enter = span.enter();
			let camera_messages = self.world.camera_factory_mut().drain_created_before_listener();

			let window_listener = &mut self.window_factory.1;

			while let Some(message) = window_listener.read() {
				self.renderer.create_window(message.into_data());
			}

			for message in camera_messages {
				self.renderer.create_camera(*message.handle(), message.into_data());
			}

			while let Some(message) = self.cameras_listener.read() {
				self.renderer.create_camera(*message.handle(), message.into_data());
			}
		}

		{
			let span = debug_span!("GraphicsApplication::render_frame");
			let _enter = span.enter();
			let frame_allocator = &self.application.frame_allocator;
			self.renderer.prepare(&mut self.renderer_transforms_listener, frame_allocator);
		}

		{
			let span = debug_span!("GraphicsApplication::flush_world_deletions");
			let _enter = span.enter();
			self.world.flush_deletions();
		}

		self.tick_count += 1;

		#[cfg(debug_assertions)]
		{
			if self.tick_count == 1 {
				self.ttff = MediaTime::from_std(self.start_time.elapsed());
			}

			if let Some(kill_after) = self.kill_after {
				if self.tick_count >= kill_after {
					close = true;
				}
			}

			{
				self.min_frame_time = self.min_frame_time.min(dt);
				self.max_frame_time = self.max_frame_time.max(dt);
			}
		}

		if close {
			self.close();
			None
		} else {
			Some(result)
		}
	}

	/// Drains application-side lifecycle events, then signals and joins every
	/// application worker.
	fn stop_worker_threads(&mut self) {
		if self.threads.is_empty() {
			return;
		}

		while self.application_events.1.try_recv().is_ok() {}
		let _ = self.application_events.0.blocking_send(Events::Close);
		self.threads.drain(..).for_each(|thread| {
			let _ = thread.join();
		});
	}

	/// Flags the application for closing.
	pub fn close(&mut self) {
		self.close = true;

		self.stop_worker_threads();

		#[cfg(debug_assertions)]
		log::debug!(
			"Run stats:\n\tElapsed time: {:#?}\n\tAverage frame time: {:#?}\n\tMin frame time: {:#?}\n\tMax frame time: {:#?}\n\tTime to first frame: {:#?}",
			MediaTime::from_std(self.start_time.elapsed()),
			MediaTime::from_std(self.start_time.elapsed()) / self.tick_count as i64,
			self.min_frame_time,
			self.max_frame_time,
			self.ttff
		);
	}

	/// Returns the input manager that owns devices, triggers, and action state.
	pub fn input_system(&self) -> &input::InputManager {
		&self.input_system
	}

	/// Returns the renderer used by setup functions and advanced render integrations.
	pub fn renderer(&self) -> &Renderer {
		&self.renderer
	}

	/// Returns the factory used to request new windows.
	pub fn window_factory(&self) -> &Factory<Window> {
		&self.window_factory.0
	}

	/// Returns mutable access to the factory used to request new windows.
	pub fn window_factory_mut(&mut self) -> &mut Factory<Window> {
		&mut self.window_factory.0
	}

	/// Returns the factory used to register input actions.
	pub fn action_factory(&self) -> &Factory<Action> {
		&self.action_factory
	}

	/// Returns mutable access to the factory used to register input actions.
	pub fn action_factory_mut(&mut self) -> &mut Factory<Action> {
		&mut self.action_factory
	}

	/// Returns the factory used to create additional worlds.
	pub fn world_factory(&self) -> &Factory<DefaultWorld> {
		&self.world_factory
	}

	/// Returns mutable access to the factory used to create additional worlds.
	pub fn world_factory_mut(&mut self) -> &mut Factory<DefaultWorld> {
		&mut self.world_factory
	}

	/// Returns the default world updated by the graphics application loop.
	pub fn world(&self) -> &DefaultWorld {
		&self.world
	}

	/// Returns mutable access to the default world updated by the graphics application loop.
	pub fn world_mut(&mut self) -> &mut DefaultWorld {
		&mut self.world
	}

	/// Returns the audio generator factory used by default audio setup.
	pub fn generator_factory(&self) -> &Factory<Arc<dyn Generator>> {
		&self.generator_factory
	}

	/// Returns mutable access to the audio generator factory used by default audio setup.
	pub fn generator_factory_mut(&mut self) -> &mut Factory<Arc<dyn Generator>> {
		&mut self.generator_factory
	}

	/// Runs ticks until the application is closed.
	pub fn do_loop(&mut self) {
		while !self.close {
			self.tick();
		}
	}

	/// Runs ticks with an application callback until the application is closed.
	pub fn do_loop_with<F: FnOnce(&mut Self, Time) + Copy>(&mut self, f: F) {
		while !self.close {
			self.tick_with(f);
		}
	}

	/// Returns the resource manager shared by rendering and asset setup.
	pub fn resource_manager(&self) -> &ResourceManager {
		&self.resource_manager
	}

	/// Returns shared ownership of the resource manager for application-owned async systems.
	///
	/// Use this handle when constructing loaders such as
	/// [`crate::animation::graph::AnimationPool`]. Next, spawn the returned
	/// worker on the application's chosen async runtime.
	pub fn resource_manager_handle(&self) -> EntityHandle<ResourceManager> {
		self.resource_manager.clone()
	}
}

impl Parameters for GraphicsApplication {
	fn get_parameter(&self, name: &str) -> Option<&Parameter> {
		self.application.get_parameter(name)
	}
}

/// Converts resolved render-pass startup parameters into asynchronous configuration events.
fn queue_render_pass_startup_parameters(parameters: &[Parameter], configuration: &Configuration) {
	for parameter in parameters {
		if parameter.name().starts_with(RENDER_PASS_PARAMETER_PREFIX) {
			configuration.update(parameter.name(), parameter.value());
		}
	}
}

const RENDER_PASS_PARAMETER_PREFIX: &str = "render.pass.";

/// Installs the simple scene pipeline for debugging and prototype rendering.
pub fn setup_simple_render_pipeline(application: &mut GraphicsApplication) {
	let listener = application.world().renderable_factory().listener();
	let delete_listener = application.world().delete_channel().listener();
	let transforms_listener = application.world().transforms_channel().listener();

	let renderer = &mut application.renderer;

	struct CustomPipelineManager {
		pipeline_manager: SimplePipelineManager,
		mesh_receiver: DefaultListener<CreateMessage<EntityHandle<dyn RenderableMesh>>>,
		mesh_delete_receiver: DefaultListener<DeleteMessage>,
		transforms_listener: DefaultListener<TransformationUpdate>,
	}

	impl PipelineManager for CustomPipelineManager {
		fn prepare<'a>(
			&'a mut self,
			frame: &mut ghi::implementation::Frame,
			sinks: &[rendering::Sink],
			frame_allocator: &'a bumpalo::Bump,
		) -> Option<SmallVec<[rendering::render_pass::RenderPassReturn<'a>; 16]>> {
			while let Some(message) = self.mesh_receiver.read() {
				let handle = *message.handle();

				self.pipeline_manager.create_mesh(frame, handle, message.into_data());
			}

			while let Some(message) = self.transforms_listener.read() {
				self.pipeline_manager
					.update_transform(frame, *message.handle(), message.transform().get_matrix());
			}

			while let Some(message) = self.mesh_delete_receiver.read() {
				self.pipeline_manager.remove_mesh(message.into_handle());

				// TODO: handle light removal
			}

			self.pipeline_manager.prepare(frame, sinks, frame_allocator)
		}

		fn create_sink(&mut self, sink_id: usize, render_pass_builder: &mut rendering::render_pass::RenderPassBuilder) {
			self.pipeline_manager.create_sink(sink_id, render_pass_builder);
		}
	}

	let sm = {
		CustomPipelineManager {
			pipeline_manager: SimplePipelineManager::new(renderer.context_mut()),
			mesh_receiver: listener,
			mesh_delete_receiver: delete_listener,
			transforms_listener,
		}
	};

	renderer.add_pipeline_manager(sm);
}

/// Installs the visibility-buffer PBR scene pipeline and its async upload worker.
///
/// Next, create an [`Environment`] through
/// [`DefaultWorld::environment_factory_mut`] to select the HDR image used for
/// ambient and specular reflections.
pub fn setup_pbr_visibility_shading_render_pipeline(
	application: &mut GraphicsApplication,
	spawn_loading_task: impl FnOnce(std::boxed::Box<dyn FnOnce(&compio::runtime::Runtime) + Send>),
) {
	let gtao_configuration = application
		.configuration()
		.register(crate::rendering::pipelines::visibility::render_pass::GTAO_CONFIGURATION_PREFIX);
	for parameter_name in ["render.gtao.radius", "render.gtao.samples-per-ray", "render.gtao.radial-rays"] {
		if let Some(parameter) = application.get_parameter(parameter_name) {
			application.configuration().update(parameter.name(), parameter.value());
		}
	}

	let application_resource_manager = application.resource_manager.clone();
	let visibility_shader_resources = application.resource_manager.clone();
	let renderer = &mut application.renderer;
	let transfer_queue_handle = renderer.transfer_queue_handle;
	let context = renderer.context_mut();
	let mut transfer_queue = context.queue(transfer_queue_handle);
	let transfer_finished_synchronizer = context.create_synchronizer(Some("Async Resource Transfer Synchronizer"), true);
	let transfer_command_buffer = transfer_queue.create_command_buffer(Some("Async Resource Transfer Command Buffer"));

	let upload_buffer: ghi::BufferHandle<
		[u8; rendering::pipelines::visibility::resource_manager::ASYNC_UPLOAD_BUFFER_BYTE_COUNT],
	> = context.build_buffer(
		ghi::buffer::Builder::new(ghi::Uses::TransferSource)
			.name("Renderer Async Upload Buffer")
			.device_accesses(ghi::DeviceAccesses::HostToDevice),
	);

	let (resource_manager_client, resource_manager) =
		VisibilityPipelineResourceManager::spawn(renderer.context_mut(), application_resource_manager);

	spawn_loading_task(std::boxed::Box::new(move |runtime| {
		runtime
			.spawn(resource_manager.run(
				transfer_queue,
				transfer_finished_synchronizer,
				transfer_command_buffer,
				upload_buffer,
			))
			.detach();
	}));

	struct CustomPipelineManager {
		light_receiver: DefaultListener<CreateMessage<Lights>>,
		light_delete_receiver: DefaultListener<DeleteMessage>,
		pending_lights: VecDeque<CreateMessage<Lights>>,
		mesh_receiver: DefaultListener<CreateMessage<EntityHandle<dyn RenderableMesh>>>,
		mesh_delete_receiver: DefaultListener<DeleteMessage>,
		pending_meshes: VecDeque<CreateMessage<EntityHandle<dyn RenderableMesh>>>,
		pose_receiver: DefaultListener<UpdatePose>,
		environment_receiver: DefaultListener<CreateMessage<Environment>>,
		pending_environments: VecDeque<CreateMessage<Environment>>,
		visibility_pipeline_manager: VisibilityPipelineManager,
	}

	impl CustomPipelineManager {
		/// Drains light creation messages into the visibility scene.
		fn request_pending_lights(&mut self) {
			while let Some(message) = self.light_receiver.read() {
				self.pending_lights.push_back(message);
			}

			while let Some(message) = self.pending_lights.pop_front() {
				let handle = *message.handle();
				self.visibility_pipeline_manager.create_light(handle, message.into_data());
			}
		}

		/// Drains renderable creation messages into the visibility resource request path.
		fn request_pending_meshes(&mut self) {
			while let Some(message) = self.mesh_receiver.read() {
				self.pending_meshes.push_back(message);
			}

			while let Some(message) = self.pending_meshes.pop_front() {
				let handle = *message.handle();
				self.visibility_pipeline_manager.request_mesh(handle, message.into_data());
			}
		}

		/// Drains pending deletion messages.
		fn process_deletions(&mut self) {
			while let Some(message) = self.light_delete_receiver.read() {
				self.visibility_pipeline_manager.remove_light(message.into_handle());
			}

			while let Some(message) = self.mesh_delete_receiver.read() {
				self.visibility_pipeline_manager.remove_mesh(message.into_handle());
			}
		}

		/// Applies application-authored skeleton poses to the visibility scene.
		fn process_pose_updates(&mut self) {
			while let Some(message) = self.pose_receiver.read() {
				self.visibility_pipeline_manager
					.update_pose(message.handle(), message.global_matrices());
			}
		}

		/// Drains environment creation commands into the visibility resource request path.
		fn request_pending_environments(&mut self) {
			while let Some(message) = self.environment_receiver.read() {
				self.pending_environments.push_back(message);
			}

			while let Some(message) = self.pending_environments.pop_front() {
				self.visibility_pipeline_manager.create_environment(message.into_data());
			}
		}
	}

	impl PipelineManager for CustomPipelineManager {
		fn prepare<'a>(
			&'a mut self,
			frame: &mut ghi::implementation::Frame,
			sinks: &[rendering::Sink],
			frame_allocator: &'a bumpalo::Bump,
		) -> Option<SmallVec<[rendering::render_pass::RenderPassReturn<'a>; 16]>> {
			self.request_pending_lights();
			self.request_pending_meshes();
			self.request_pending_environments();
			self.process_pose_updates();

			self.process_deletions();

			self.visibility_pipeline_manager.prepare(frame, sinks, frame_allocator)
		}

		fn create_sink(&mut self, sink_id: usize, render_pass_builder: &mut rendering::render_pass::RenderPassBuilder) {
			self.visibility_pipeline_manager.create_sink(sink_id, render_pass_builder);
		}
	}

	{
		let pending_lights = application
			.world_mut()
			.light_factory_mut()
			.drain_created_before_listener()
			.into_iter()
			.collect::<VecDeque<_>>();
		let light_receiver = application.world().light_factory().listener();
		let light_delete_receiver = application.world().delete_channel().listener();
		let pending_meshes = application
			.world_mut()
			.renderable_factory_mut()
			.drain_created_before_listener()
			.into_iter()
			.collect::<VecDeque<_>>();
		let mesh_receiver = application.world().renderable_factory().listener();
		let mesh_delete_receiver = application.world().delete_channel().listener();
		let pose_receiver = application.world().poses_channel().listener();
		let pending_environments = application
			.world_mut()
			.environment_factory_mut()
			.drain_created_before_listener()
			.into_iter()
			.collect::<VecDeque<_>>();
		let environment_receiver = application.world().environment_factory().listener();

		let renderer = &mut application.renderer;

		let sm = CustomPipelineManager {
			visibility_pipeline_manager: VisibilityPipelineManager::new(
				renderer.context_mut(),
				resource_manager_client,
				visibility_shader_resources,
				gtao_configuration,
			),
			light_receiver,
			light_delete_receiver,
			pending_lights,
			mesh_receiver,
			mesh_delete_receiver,
			pending_meshes,
			pose_receiver,
			environment_receiver,
			pending_environments,
		};

		renderer.add_pipeline_manager(sm);
	}
}

/// Installs the retained UI render pass fed by UI render messages.
pub fn setup_ui_render_pass(application: &mut GraphicsApplication, ui: DefaultListener<CreateMessage<Render>>) {
	let renderer = &mut application.renderer;
	let ui_channel = ui.clone_channel();

	renderer.add_post_scene_render_pass_for_all_sinks(move |render_pass_builder| {
		struct CustomRenderPass {
			listener: DefaultListener<CreateMessage<Render>>,
			render_pass: UiRenderPass,
		}

		impl rendering::RenderPass for CustomRenderPass {
			fn name(&self) -> &'static str {
				self.render_pass.name()
			}

			fn prepare<'a>(
				&mut self,
				frame: &mut ghi::implementation::Frame,
				sink: &rendering::Sink,
				frame_allocator: &'a bumpalo::Bump,
			) -> Option<rendering::render_pass::RenderPassReturn<'a>> {
				drain_render_pass_messages(&mut self.listener, |render| self.render_pass.update(render.into_data()));

				self.render_pass.prepare(frame, sink, frame_allocator)
			}

			fn bypass<'a>(
				&mut self,
				frame: &mut ghi::implementation::Frame,
				sink: &rendering::Sink,
				frame_allocator: &'a bumpalo::Bump,
			) -> Option<rendering::render_pass::RenderPassReturn<'a>> {
				drain_render_pass_messages(&mut self.listener, |render| self.render_pass.update(render.into_data()));

				self.render_pass.bypass(frame, sink, frame_allocator)
			}
		}

		Box::new(CustomRenderPass {
			// Spawn only the listeners that are actively consumed by render passes.
			listener: ui_channel.listener(),
			render_pass: UiRenderPass::new(render_pass_builder),
		})
	});
}

/// Drains all pending pass inputs so active and bypassed paths adopt the same application state.
fn drain_render_pass_messages<M: Clone>(listener: &mut DefaultListener<M>, mut adopt: impl FnMut(M)) {
	while let Some(message) = listener.read() {
		adopt(message);
	}
}

/// Installs the AGX tonemapping pass for post-scene color mapping.
pub fn setup_agx_tonemap_render_pass(application: &mut GraphicsApplication) {
	let renderable_mesh_factory = application.world().renderable_factory();
	let listener = renderable_mesh_factory.listener();

	let renderer = &mut application.renderer;

	renderer.add_post_scene_render_pass_for_all_sinks(|render_pass_builder| Box::new(AgxToneMapPass::new(render_pass_builder)));
}

/// Installs the ACES v1 tonemapping pass for post-scene color mapping.
pub fn setup_aces_tonemap_render_pass(application: &mut GraphicsApplication) {
	let renderer = &mut application.renderer;

	renderer
		.add_post_scene_render_pass_for_all_sinks(|render_pass_builder| Box::new(AcesToneMapPass::new(render_pass_builder)));
}

/// Installs the final swapchain blit pass that presents rendered sinks.
pub fn setup_swapchain_blit_render_pass(application: &mut GraphicsApplication) {
	let renderer = &mut application.renderer;

	renderer
		.add_post_scene_render_pass_for_all_sinks(|render_pass_builder| Box::new(SwapchainBlitPass::new(render_pass_builder)));
}

/// Registers a reusable bloom pass that should run before tonemapping.
pub fn setup_bloom_render_pass(application: &mut GraphicsApplication, settings: BloomPassSettings) {
	let renderer = &mut application.renderer;

	renderer.add_post_scene_render_pass_for_all_sinks(move |render_pass_builder| {
		Box::new(BloomPass::with_settings(render_pass_builder, settings))
	});
}

/// Installs spatial SMAA for every current and future render sink.
///
/// Add this pass after tonemapping and before UI or the final presentation blit
/// so edge detection receives display-referred color without softening overlays.
pub fn setup_smaa_render_pass(application: &mut GraphicsApplication) {
	application
		.renderer
		.add_post_scene_render_pass_for_all_sinks(|render_pass_builder| Box::new(SmaaPass::new(render_pass_builder)));
}

/// Installs the atmosphere sky pass used as a post-scene background.
pub fn setup_atmosphere_sky_render_pass(
	application: &mut GraphicsApplication,
	settings: crate::rendering::render_passes::sky::AtmosphereSkyRenderPassSettings,
) {
	let renderer = &mut application.renderer;

	renderer.add_post_scene_render_pass_for_all_sinks(move |render_pass_builder| {
		Box::new(AtmosphereSkyRenderPass::with_settings(render_pass_builder, settings))
	});
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn bypass_message_drain_adopts_every_pending_value() {
		let channel = DefaultChannel::new();
		let mut listener = channel.listener();
		channel.send(1);
		channel.send(2);
		let mut adopted = Vec::new();

		drain_render_pass_messages(&mut listener, |value| adopted.push(value));

		assert_eq!(adopted, vec![1, 2]);
		assert!(listener.read().is_none());
	}

	#[test]
	fn startup_parameters_queue_only_render_pass_configuration() {
		let configuration = Configuration::new();
		let port = configuration.register(RENDER_PASS_PARAMETER_PREFIX);
		let parameters = [
			Parameter::new("render.pass.bloom", "bypassed"),
			Parameter::new("audio.master.gain", "0.5"),
		];

		queue_render_pass_startup_parameters(&parameters, &configuration);

		let update = port.read().expect("render-pass startup configuration");
		assert_eq!(update.parameter(), "render.pass.bloom");
		assert_eq!(update.value(), &crate::configuration::ConfigurationValue::from("bypassed"));
		assert!(port.read().is_none());
		assert_eq!(configuration.events().len(), 1);
	}

	#[test]
	#[ignore] // Renderer broken.
	fn create_graphics_application() {
		let mut app = GraphicsApplication::new("Test", &[]);

		assert_eq!(app.get_name(), "Test");

		let start_time = std::time::Instant::now();

		while !app.close {
			app.tick();

			if start_time.elapsed().as_secs() > 1 {
				app.close();
			}
		}
	}
}

use core::time;
use std::{collections::VecDeque, sync::Arc, thread};

use ghi::{Context as _, ContextCreate as _, Frame as _, Queue as _};
use resource_management::{
	resource::{resource_manager::ResourceManager, RedbStorageBackend},
	resources::material::Material,
};
use smallvec::SmallVec;
use tracing::{debug_span, instrument, span, Level};
use utils::{sync::RwLock, Box};

use super::{
	application::{Application, BaseApplication},
	Events, Parameter, Receiver, Sender, Time,
};
use crate::{
	application::{parameters::Parameters, thread::Thread},
	audio::generator::Generator,
	configuration::Configuration,
	core::{
		channel::{Channel, DefaultChannel},
		factory::{CreateMessage, Factory},
		listener::{DefaultListener, Listener},
		message::DeleteMessage,
		task, Entity, EntityHandle,
	},
	gameplay::{transform::TransformationUpdate, world::DefaultWorld},
	ghi::command_buffer::CommandBufferRecording as _,
	input::{input_trigger, Action},
	inspector::{http::HttpInspectorServer, Inspector},
	physics::dynabit::{self, body::PhysicsBody},
	rendering::{
		lights::{Light, Lights},
		pipeline_manager::PipelineManager,
		pipelines::{
			simple::{SimplePipelineManager, SimpleRenderPass},
			visibility::{resource_manager::VisibilityPipelineResourceManager, VisibilityPipelineManager},
		},
		render_pass::RenderPass,
		render_passes::{
			aces::AcesToneMapPass,
			agx::AgxToneMapPass,
			blit::SwapchainBlitPass,
			bloom::{BloomPass, BloomPassSettings},
			sky::AtmosphereSkyRenderPass,
			smaa::SmaaPass,
		},
		renderable, renderer, Environment, RenderableMesh, UpdatePose,
	},
	time::MediaTime,
	ui::{layout::engine::Render, render_pass::UiRenderPass},
};
use crate::{
	gameplay::anchor::AnchorSystem,
	input, physics,
	rendering::{self, renderer::Renderer, window::Window, Camera},
};
pub mod defaults;
mod integrations;

pub use defaults::{
	default_setup, setup_default_audio, setup_default_input, setup_default_resource_and_asset_management, setup_default_window,
};
pub use integrations::process_default_window_input;
#[cfg(feature = "dmx")]
pub use integrations::setup_default_dmx;
