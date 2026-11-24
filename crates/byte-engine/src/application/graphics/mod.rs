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
//! Follow the [sample project guide](/docs/use/sample-project)
//! for the complete application setup sequence.

/// The [`GraphicsApplication`] struct owns the headed runtime and coordinates
/// windows, input, worlds, resources, audio workers, and rendering.
///
/// Use [`default_setup`] for the conventional engine stack. Use
/// [`setup_default_window`], [`setup_default_input`], and the render-pass setup
/// functions independently when an application needs explicit composition.
/// After setup, call [`Self::do_loop`] to run the application, or
/// [`Self::tick_with`] when application code must run during each tick. Call
/// [`Self::run_lanes`] from it when part of that code should run on every lane of the application's alley.
///
/// # Configuration
/// - `kill-after`: Closes the application after this number of ticks. The default is `None`.
/// - `inspector.address`: Serves the HTTP inspector on this IPv4 or IPv6 address only, such as `0.0.0.0` to reach it from another machine. The default is both loopback addresses. An address that isn't loopback is logged as a warning, because anyone who reaches the inspector can close the application or send it messages.
/// - `inspector.port`: Serves the HTTP inspector on this port. The default is `6680`, or a free port the system picks, logged as a warning, when another application uses it. A port set here that is in use stops startup.
/// - `console.enabled`: Takes over the terminal that started the application with a full-screen console: the log, and a command line for commands such as `set render.lod.force 2`. The console starts only when standard input and standard output are both terminals; set this to `false` when the application reads standard input or writes standard output itself. See the [console reference](/docs/reference/console). The default is `true`.
/// - `metrics-out`: Writes the performance metrics the inspector serves to this file when the application closes: the `GET /metrics` summary as JSON, or every retained tick as NDJSON, one `GET /metrics/frames` row per line, when the path ends in `.ndjson`. Combine it with `kill-after` to measure a run without polling. The default is `None`.
/// - `debug.streams`: Writes every entity data stream that world-spawned [`DebugDataStream`](crate::inspector::DebugDataStream) requests to `log`, `stdout`, `stderr`, or the file at this path, such as `/dev/fd/3`, as NDJSON. The default is `None`, which opens no stream unless the console runs; the console reads streams, so they open for it with no destination.
/// - `debug.streams.changes-only`: Writes a data stream record only when the system's key for it changed, leaving out records where only progress like a phase moved. The default is `false`.
/// - `max-frame-rate`: Caps presentation at this many frames per second; frames land on even refreshes. The default is uncapped.
/// - `simulation-rate`: Steps simulation this many times per second, independent of how often frames are presented. [`Self::tick_stepped_with`] paces its steps on it and defaults to `60`. [`Self::tick_with`] simulates once per frame and panics when this rate disagrees with the rate frames are presented at.
/// - `unattended`: Runs the application for no one at the desktop, such as an agent debugging it through the inspector, at this many frames per second. Windows open without being shown or taking focus, every tick renders every window whether or not it can be seen, the loop paces itself to the rate, and the clock advances by exactly one frame period per tick, so runs repeat. The rate is required; `max-frame-rate` and `render-on-demand` are ignored. The default is `None`, an attended run.
/// - `render-on-demand`: Renders only when something changed instead of on every tick. Window changes, UI renders, and screenshot requests ask for frames; call [`Renderer::request_redraw`] after changing the scene. Idle ticks keep running at the refresh rate of the fastest display showing a window, or the `max-frame-rate` pace when that is slower, so events are still handled. Once the frame is shown and no UI component waits for a frame, the loop waits in the window system until an event, a UI timer, or a [`crate::application::LoopWaker`] wakes it; call [`crate::application::LoopWaker::wake`] from threads that change state the loop must see. The default is `false`.
/// - `assets.path`: Selects the debug-build asset directory. Relative overrides use the current working directory. In development, the default is `assets` under `CARGO_MANIFEST_DIR` when available, then beside the executable.
/// - `resources.path`: Selects the resource directory. Relative overrides use the current working directory. In development, the default uses `CARGO_MANIFEST_DIR` when available; otherwise, it is beside the executable.
/// - `render.debug`: Enables validation layers. The default is `true` in debug builds.
/// - `render.debug.dump`: Enables graphics API logging. The default is `false`.
/// - `render.debug.extended`: Enables extended validation. The default is `false`.
/// - `render.pipeline-cache.path`: Keeps compiled pipelines in this file, so later runs create their pipelines without compiling shaders. The file is read at startup, and the pipelines compiled since are appended to it when the application closes; a run that compiles none does not touch it. Use one file per GPU when applications that run on different GPUs at the same time would otherwise share it. Data from another GPU or driver version is ignored. Relative paths use the current working directory. The default is none, which leaves pipeline caching to the GPU driver.
/// - `messages.capacity`: Sets the bytes of message storage shared by every typed route, in whole chunks of [`crate::core::message_bus::CHUNK_BYTES`]. A publisher waits when every chunk is held by unread messages. The default is `67108864`, which is 64 MiB.
/// - `render.pass.<name>`: Selects `enabled` or `bypassed` for the named render pass.
/// - `render.debug.view.<pass>.<view>`: Shows (`true`) or hides (`false`) one debug view of the named render pass or scene pipeline manager, such as `render.debug.view.motion-blur.velocity`. Shown views tile over the presented image, each in one of `render.debug.targets` slots; a view is refused when every slot is taken. `GET /debug-views` on the inspector lists the views. The default is hidden.
/// - `render.debug.targets`: Sets how many debug views can be shown at once, from `1` to `15`. The default is `4`.
/// - `render.debug-views.enabled`: Builds debug views into the project. Like `render.gtao.enabled`, the startup value decides whether their code is baked into the shaders; a build without them refuses every `render.debug.view.*`. The default is `true` in debug builds and `false` in release builds.
/// - `render.gtao.enabled`: Runs GTAO, which darkens environment light in creases and corners. Both screen-space occlusion sources can run alone, together, or not at all; together, each pixel takes the stronger occlusion. The startup value also decides whether GTAO is built in: set it to `false` in `config.json` and BELD bakes shaders without GTAO code and skips its pipelines, and GTAO cannot be turned on at runtime. See [`crate::rendering::pipelines::visibility::VisibilityFeatures`]. The default is `true`.
/// - `render.gtao.radius`: Sets the GTAO world-space search radius. The default is `1.0`.
/// - `render.gtao.samples-per-ray`: Sets the GTAO samples along each ray. The default is `4`.
/// - `render.gtao.radial-rays`: Sets the even number of GTAO ray directions. The default is `6`.
/// - `render.gi.enabled`: Lights indirect diffuse light with the technique `render.gi.technique` selects. Off, the environment lights it alone. The default is `true`.
/// - `render.gi.technique`: Selects the global illumination technique: `ssgi`, `surfels`, `probes`, or `baked`. A technique left out of the baked shaders is rejected with a warning. The default is the first built in of `ssgi`, `probes`, `surfels`, and `baked`.
/// - `render.ssgi.enabled`: Builds screen-space global illumination, which adds light bounced off visible surfaces and darkens environment light that nearby geometry blocks, into the baked shaders, so `render.gi.technique` can select it. It is read once, at startup. The default is `true`.
/// - `render.surfel-gi.enabled`: Builds ray-traced surfel global illumination into the baked shaders, so `render.gi.technique` can select it. It is read once, at startup. The default is `false`.
/// - `render.baked-gi.enabled`: Builds baked lightmaps in: lightmap UVs in mesh bakes, the lightmap bake in BELD, and lightmap reads in the baked shaders, so `render.gi.technique` can select `baked`. It is read once, at startup. The default is `false`.
/// - `render.contact-shadows.enabled`: Traces short rays toward the sun to shadow where objects touch, such as under a foot on a floor. Like `render.gtao.enabled`, its startup value also decides whether contact shadows are built into the baked shaders. The default is `true`.
/// - `render.contact-shadows.distance`: Sets the world-space reach of each contact-shadow ray. The default is `0.1`.
/// - `render.lights.directional.max`: Caps how many directional lights the project uses. Set it to `1` when the scene has at most one directional light, which builds the `single_directional_light` shader feature in. No shader reads that feature yet, so the setting changes nothing drawn. The default is `4`.
/// - `render.bloom.pixel-weights`: Weighs each pixel of a bloom prefilter block by its brightness before averaging the block (`true`), or reads each block with one bilinear tap (`false`), which costs less but lets a cluster of bright pixels glow more. The default is `false`.
/// - `render.lod.bias`: Scales the on-screen error, one pixel by default, that a mesh detail level may show before a finer one is drawn. Values above `1` draw coarser levels sooner, and `0` allows no error. Levels follow the first window's camera, and shadows draw the same level. The default is `1.0`.
/// - `render.lod.force`: Draws this detail level for every mesh instead of choosing by distance, where `0` is full detail; a mesh with fewer levels draws its coarsest one. Use it to compare levels or to measure what they save. The default is `off`.
/// - `render.shadow-maps.budget-mb`: Sets the shadow-map memory, in mebibytes, that every shadow-casting light shares. The first directional light takes its cascades first, cone and point lights then share what remains by how much of the screen they light, and other directional lights take what is left. One sun takes 32 MiB at the default resolution and four times that per doubling of `render.directional-shadows.resolution`; a cone light takes 32 KiB to 8 MiB and a point light 192 KiB to 48 MiB, by how large it looks in the largest sink that sees it. Maps allocate on first use; the default is `128`.
/// - `render.shadow-maps.update-texels`: Sets how many shadow-map texels each frame draws. Cascades 0 and 1 of every shadowed sun draw every frame and count first; the other cascades, cone maps, and point cube faces then draw by need within what's left, so a smaller budget spreads their updates over more frames. Texels a frame leaves unspent carry over to the next, so a budget smaller than one map still draws it every few frames. The default is `10485760`, two and a half cascades of one sun at 2,048 texels per side: cascades 2 and 3 take turns every other frame. Set `16777216` to draw all four every frame.
/// - `render.directional-shadows.resolution`: Sets the directional shadow cascades' resolution in texels per side, a multiple of 16. Halving it quarters the shadow raster and depth pyramid work, and each sun's cost from the shadow-map budget. The default is `2048`.
///
/// See the [sample project guide](/docs/use/sample-project)
/// for a complete `GraphicsApplication` setup.
pub struct GraphicsApplication {
	application: BaseApplication,
	window_events: DefaultChannel<ghi::window::Event>,

	tick_count: u64,
	start_time: std::time::Instant,
	/// Accumulated frame deltas; kept separate from the wall clock so it stays consistent with paced deltas.
	elapsed: MediaTime,
	last_tick_instant: std::time::Instant,
	/// The display time of the last presented frame the clock has consumed, when the swapchain reported one.
	last_present_time: Option<std::time::Instant>,
	/// The minimum time between presented frames; the `max-frame-rate` parameter when given.
	present_interval: Option<std::time::Duration>,
	/// The fixed simulation step; the `simulation-rate` parameter when given.
	simulation_step: Option<MediaTime>,
	/// Simulated time that has not yet reached a whole fixed step.
	simulation_pending: MediaTime,
	/// The time the fixed steps run so far have consumed.
	simulation_elapsed: MediaTime,
	/// How far the frame lies between the two most recent steps; `1` when simulation runs once per frame.
	simulation_alpha: f32,
	/// The simulation time frames show: one step behind the latest step in the stepped loop, and the latest step
	/// when simulation runs once per frame.
	display_time: MediaTime,
	/// The frame period the loop sleeps for when no window presents; see [`skipped_frame_pace`].
	skipped_frame_pace: std::time::Duration,
	/// The value of `elapsed` when `last_present_time` was consumed; later presented times land relative to it.
	elapsed_at_last_present: MediaTime,
	/// Whether frames are rendered only when something changed; the `render-on-demand` parameter.
	render_on_demand: bool,
	/// The frame period of an unattended run; the `unattended` parameter when given.
	unattended: Option<std::time::Duration>,
	/// When the next unattended tick may start.
	next_frame_at: std::time::Instant,
	/// Whether the last tick rendered because something changed, so this tick expects to render again.
	rendering_active: bool,
	/// Wakes this loop when state it cannot observe through window events changes.
	waker: LoopWaker,
	/// Whether the window system's waker reached [`Self::waker`].
	platform_waker_set: bool,
	/// The earliest tick the application asked for; see [`Self::schedule_tick`].
	requested_tick: Option<std::time::Instant>,

	close: bool,

	/// The inspector the console and the HTTP transport serve, which [`Self::inspector`] lends to in-process tools.
	inspector: EntityHandle<DefaultInspector>,
	/// Keeps the HTTP inspector transport listening while the application lives.
	_http_inspector: HttpInspectorServer,
	screenshot_broker: std::sync::Arc<crate::inspector::screenshot::ScreenshotBroker>,
	/// Wakes inspector requests waiting for events after each tick, and tells them when the application closes.
	inspector_events: std::sync::Arc<crate::inspector::EventSignal>,
	configuration: Configuration,

	/// Layers that pull their own sinks from [`Self::input`] before the world's sink, in the order they were added;
	/// see [`Self::add_input_layer`].
	input_layers: Vec<Box<dyn FnMut(&mut input::InputCollector)>>,
	window_factory: (Factory<Window>, DefaultListener<CreateMessage<Window>>),
	/// Feature changes for open windows, which the renderer applies before each frame.
	window_features: DefaultListener<rendering::SetWindowFeatures>,
	/// Pointer shape changes for open windows, which the renderer applies before each frame.
	window_pointers: DefaultListener<rendering::SetWindowPointer>,
	/// Camera changes for open windows, which the renderer applies before each frame.
	window_cameras: DefaultListener<rendering::SetWindowCamera>,
	/// Windows the application closes, which the renderer removes before each frame.
	window_closes: DefaultListener<rendering::CloseWindow>,
	/// Render images the application creates, which the renderer opens before each frame.
	render_image_factory: (
		Factory<rendering::RenderImage>,
		DefaultListener<CreateMessage<rendering::RenderImage>>,
	),
	/// Where the application learns which image the renderer draws each render image into.
	render_images_ready: DefaultChannel<rendering::RenderImageReady>,
	/// Size changes for open render images, which the renderer applies before each frame.
	render_image_extents: DefaultListener<rendering::SetRenderImageExtent>,
	/// Removals on the application's message scope, which stop the render images they name.
	render_image_deletes: DefaultListener<DeleteMessage>,
	/// This tick's window events, kept between ticks so polling allocates nothing.
	polled_window_events: Vec<ghi::window::Event>,
	/// Each open window's size in pixels and display scale, as its last resize reported them, which
	/// [`Self::layout_size`] turns into layout units.
	window_scales: Vec<(ghi::window::WindowId, crate::ui::Size, f32)>,
	/// The `ui.scale` setting, which multiplies every window's display scale, and the route that changes it.
	ui_scale: (f32, crate::configuration::ConfigurationPort),
	/// The text scale in effect.
	text_scale: f32,
	/// The `accessibility.` settings the player chose, and the route that changes them.
	accessibility: (crate::accessibility::Choices, crate::configuration::ConfigurationPort),
	/// The operating system's presentation preferences, as last reported.
	system_preferences: ghi::window::Preferences,

	generator_factory: Factory<std::boxed::Box<dyn Generator>>,

	/// Every scene's scope and the routes the renderer takes its cameras, sun, and fog from, by [`SceneId`]. The
	/// world comes first: entities are created on its scope, and systems that follow them, such as physics, are
	/// created on it by the application code that steps them.
	scenes: Vec<SceneRoutes>,
	/// The transforms every frame shows, sampled after each simulation step and resolved before each frame.
	shown_transforms: ShownTransforms,

	/// Every window and gamepad event ends up here before the world's actions pull it.
	input: input::InputCollector,
	/// The sink for actions declared through the world. It captures nothing.
	actions: input::InputSink,
	gamepad_system: input::gamepad::GamepadSystem,
	gamepad_device_class_handle: Option<input::device::DeviceClassHandle>,
	/// The `input.gamepad.dead-zone.` settings, which every gamepad binding without its own dead zone takes, and the
	/// route that changes them.
	gamepad_dead_zone: (input::DeadZone, crate::configuration::ConfigurationPort),
	resource_manager: EntityHandle<ResourceManager>,
	renderer: Renderer,
	/// The loading timeline and whether it already moved to the loading thread.
	loader: LoaderState,
	/// Loading work registered during setup, run in registration order on the loading thread once
	/// [`defaults::launch_deferred_tasks_thread`] starts it.
	deferred_tasks: Vec<defaults::DeferredTask>,

	threads: SmallVec<[Thread; 64]>,
	/// Persistent lanes shared by initialization and application frame work. `None` while an alley execution's
	/// caller borrows the whole application; see [`Self::run_lanes`].
	alley: Option<crate::core::alley::Alley>,

	/// Where the metrics summary or rows are written when the application closes, until written.
	metrics_out: Option<std::path::PathBuf>,
	/// Observes world-spawned DDS entities and supplies writers when a debug destination is configured.
	streams: crate::inspector::DebugDataStreams,

	kill_after: Option<u64>,

	/// The device every context comes from. Declared last so the renderer's and the loader's contexts drop first.
	graphics_device: rendering::GraphicsDevice,
}

impl Drop for GraphicsApplication {
	fn drop(&mut self) {
		// An application closed from outside a tick still gets its metrics written and its event waiters released.
		self.inspector_events.close();
		self.write_metrics_out();
		// Workers may own transferred views into renderer allocations. Join them
		// before field destruction reaches the renderer and its GHI context.
		self.stop_worker_threads();
	}
}

/// The `LoaderState` enum tracks the application's loader from setup until the loading thread owns it.
enum LoaderState {
	/// No pipeline asked for the loader yet.
	Unused,
	/// Pipelines are adding lanes; the loader waits for the loading thread.
	Pending(rendering::loading::Loader),
	/// The loading thread took the loader and every deferred task, so setup can add neither.
	Launched,
}

/// The `SceneRoutes` struct keeps one scene's scope and the routes its cameras, sun, and fog reach the renderer by.
///
/// They subscribe when the scene is created, so a light created before a sink's passes exist still reaches them.
struct SceneRoutes {
	scope: MessageScope,
	cameras: DefaultListener<CreateMessage<Camera>>,
	/// Directional lights, the newest of which the renderer shows as the scene's sun.
	directional_lights: DefaultListener<CreateMessage<rendering::DirectionalLight>>,
	/// Fogs, the newest of which the renderer shows in the scene.
	fogs: DefaultListener<CreateMessage<rendering::ExponentialHeightFog>>,
}

impl SceneRoutes {
	/// Subscribes to the cameras, suns, and fogs of the scene whose objects are created on `scope`.
	fn new(scope: MessageScope) -> Self {
		Self {
			cameras: scope.factory::<Camera>().listener(),
			directional_lights: scope.factory::<rendering::DirectionalLight>().listener(),
			fogs: scope.factory::<rendering::ExponentialHeightFog>().listener(),
			scope,
		}
	}
}

/// The `Services` struct names the CPU-side services one setup lane of [`GraphicsApplication::new`] builds, so
/// services of the same type cannot trade places.
struct Services {
	input: input::InputCollector,
	actions: input::InputSink,
	/// The world's scene routes.
	world: SceneRoutes,
	shown_transforms: ShownTransforms,
	inspector: EntityHandle<DefaultInspector>,
	http_inspector: HttpInspectorServer,
	/// The console's thread, when the terminal console started; it joins the application's workers.
	console: Option<Thread>,
	/// Where the inspector keeps each debug data stream's latest record.
	retained_streams: std::sync::Arc<crate::inspector::RetainedStreams>,
	screenshot_broker: std::sync::Arc<crate::inspector::screenshot::ScreenshotBroker>,
	inspector_events: std::sync::Arc<crate::inspector::EventSignal>,
	waker: LoopWaker,
	window_factory: Factory<Window>,
	window_factory_listener: DefaultListener<CreateMessage<Window>>,
	window_features: DefaultListener<rendering::SetWindowFeatures>,
	window_pointers: DefaultListener<rendering::SetWindowPointer>,
	window_cameras: DefaultListener<rendering::SetWindowCamera>,
	window_closes: DefaultListener<rendering::CloseWindow>,
	generator_factory: Factory<std::boxed::Box<dyn Generator>>,
	window_events: DefaultChannel<ghi::window::Event>,
}

impl GraphicsApplication {
	/// Creates the headed runtime with the specified name and configuration parameters.
	///
	/// Parameters may be overridden by the project's `config.json`, then by `BE_*` environment variables, and then by
	/// `--name=value` command-line arguments. The pipeline compilation servers start here, so every setup function can
	/// request pipelines.
	///
	/// Next, call [`default_setup`] or the individual setup functions, then [`Self::do_loop`].
	pub fn new(name: &str, parameters: &[Parameter]) -> Self {
		let start_time = std::time::Instant::now();

		let application = BaseApplication::new(name, parameters);
		let messages = application.messages().clone();
		let world_messages = application.message_bus().new_scope("world");
		let application_events = application.events().clone();

		let resources_path = resolve_application_path(application.get_parameter("resources.path"), "resources");
		// Parameters go to the setup lanes as a slice, which can be shared, where the application cannot.
		let parameters = application.parameters();
		let render_parameters = parameters.scope(RENDER_PARAMETERS);

		let configuration = Configuration::new();
		let metrics = Arc::clone(application.metrics());
		let mut alley = crate::core::alley::Alley::new();
		// The renderer's ports take render-pass states and debug views; they subscribe before the startup parameters
		// are queued below.
		let render_pass_configuration = configuration.register(RENDER_PASS_CONFIGURATION);
		let debug_view_configuration = configuration.register(DEBUG_VIEW_CONFIGURATION);
		let debug_views = Arc::new(rendering::DebugViewCatalog::default());
		let renderer_debug_views = Arc::clone(&debug_views);
		let transforms = world_messages.channel::<TransformationUpdate>();
		let mut storage = None;
		let mut services = None;
		// Storage and CPU services own separate lanes while graphics creation stays on the caller.
		let (graphics_device, mut renderer, pipeline_compilation_servers) = alley
			.join_with_mut(
				(&mut storage, &mut services),
				|lane, (storage, services)| {
					lane.only_one_runs_mut(storage, |storage| {
						*storage = Some(
							ReDBStorageBackend::new_writable_with_settings(
								resources_path.clone(),
								ResourceStorageSettings::new(ResourceStorageMode::Files)
									.image_compression(ResourceGpuCompressionPolicy::native()),
							)
							// The message names the store, the cause, and the fix; startup has no caller to return it to.
							.unwrap_or_else(|error| panic!("{error}")),
						);
					});
					lane.only_one_runs_mut(services, |services| {
						let action_events = world_messages.channel();
						let mut input = input::InputCollector::new();
						let actions = input::InputSink::new(input.add_sink(), action_events.clone())
							.with_declarations(world_messages.factory::<Action>().listener());
						let world = SceneRoutes::new(world_messages.clone());
						let shown_transforms = ShownTransforms::new(&world_messages);

						// Register reflected posts after the shown transforms' future-only listener exists.
						let mut inspector = DefaultInspector::new(
							application_events.clone(),
							configuration.clone(),
							world_messages.clone(),
							Arc::clone(&metrics),
						);
						inspector.set_debug_views(Arc::clone(&debug_views));
						inspector
							.register_message(TRANSFORMATION_UPDATE_MESSAGE_TYPE, transforms.clone())
							.unwrap_or_else(|error| panic!("{error}"));
						// The renderer replaces a camera created again under its handle, so an update is a creation.
						inspector
							.register_message(
								CAMERA_UPDATE_MESSAGE_TYPE,
								world_messages.channel::<rendering::CameraUpdate>(),
							)
							.unwrap_or_else(|error| panic!("{error}"));
						// So does a light, created again under its handle.
						inspector
							.register_message(
								POINT_LIGHT_UPDATE_MESSAGE_TYPE,
								world_messages.channel::<rendering::PointLightUpdate>(),
							)
							.unwrap_or_else(|error| panic!("{error}"));
						inspector
							.register_message(
								CONE_LIGHT_UPDATE_MESSAGE_TYPE,
								world_messages.channel::<rendering::ConeLightUpdate>(),
							)
							.unwrap_or_else(|error| panic!("{error}"));
						inspector
							.register_message(
								DIRECTIONAL_LIGHT_UPDATE_MESSAGE_TYPE,
								world_messages.channel::<rendering::DirectionalLightUpdate>(),
							)
							.unwrap_or_else(|error| panic!("{error}"));
						for message_type in [DELETE_MESSAGE_TYPE, DESTROY_MESSAGE_TYPE] {
							inspector
								.register_message(message_type, world_messages.channel::<DeleteMessage>())
								.unwrap_or_else(|error| panic!("{error}"));
						}
						inspector
							.register_message(TRIGGER_ACTION_MESSAGE_TYPE, action_events)
							.unwrap_or_else(|error| panic!("{error}"));
						let inspector = EntityHandle::from(inspector);
						let screenshot_broker = inspector.screenshot_broker();
						let inspector_events = inspector.event_signal();
						let retained_streams = std::sync::Arc::clone(inspector.streams());
						let waker = LoopWaker::default();
						let console = crate::inspector::console::spawn(
							inspector.clone(),
							waker.clone(),
							application_events.listener(),
							parameters.scope("console"),
						);
						let http_inspector =
							HttpInspectorServer::new(inspector.clone(), waker.clone(), parameters.scope("inspector"));

						let window_factory = messages.factory();
						let window_factory_listener = window_factory.listener();
						let window_features = messages.channel().listener();
						let window_pointers = messages.channel().listener();
						let window_cameras = messages.channel().listener();
						let window_closes = messages.channel().listener();

						let generator_factory = messages.factory();

						let window_events = messages.channel();
						*services = Some(Services {
							input,
							actions,
							world,
							shown_transforms,
							inspector,
							http_inspector,
							console,
							retained_streams,
							screenshot_broker,
							inspector_events,
							waker,
							window_factory,
							window_factory_listener,
							window_features,
							window_pointers,
							window_cameras,
							window_closes,
							generator_factory,
							window_events,
						});
					});
				},
				|| {
					let graphics_device = rendering::GraphicsDevice::new(render_parameters);
					let (renderer, servers) = rendering::renderer::Renderer::new(
						&graphics_device,
						render_parameters,
						render_pass_configuration,
						debug_view_configuration,
						renderer_debug_views,
						Arc::clone(&metrics),
					);
					(graphics_device, renderer, servers)
				},
			)
			.unwrap_or_else(|panic| std::panic::resume_unwind(panic));
		let resource_storage = storage.unwrap();
		let services = services.unwrap();

		// Debug builds keep the store on a host thread, so the asset manager installed later bakes into it.
		#[cfg(debug_assertions)]
		let resource_manager = EntityHandle::from(ResourceManager::hosted(resource_storage));
		#[cfg(not(debug_assertions))]
		let resource_manager = EntityHandle::from(ResourceManager::new(resource_storage));

		renderer.set_resource_manager(&resource_manager);
		// The inspector lists stored resources, and it was created before the store finished opening.
		services.inspector.set_resource_manager(resource_manager.clone());
		// Every server compiles from the first request on, so no setup function has to start them.
		let mut threads = SmallVec::new();
		// The console restores the terminal when it ends, so it is joined on close like every other worker.
		// The console reads debug data streams, so its presence opens them like a `debug.streams` destination does.
		let console_running = services.console.is_some();
		threads.extend(services.console);
		for server in pipeline_compilation_servers {
			let resources = resource_manager.clone();
			threads.push(Thread::new(application.events().listener(), move |mut events| {
				let runtime = defaults::build_single_threaded_async_runtime();

				runtime.enter(|| {
					runtime.spawn(server.run(resources)).detach();
					defaults::drive_runtime(&runtime, || matches!(events.read(), Some(Events::Close)));
				});
			}));
		}
		let positive_rate = |name| {
			application
				.get_parameter(name)
				.and_then(|parameter| parameter.parse::<f64>().ok())
				.filter(|rate| *rate > 0.0)
		};
		let unattended = unattended_period(application.get_parameter("unattended"));
		// An unattended run paces itself, so no presentation cap applies.
		let present_interval = positive_rate("max-frame-rate")
			.filter(|_| unattended.is_none())
			.map(|rate| std::time::Duration::from_secs_f64(1.0 / rate));
		renderer.set_present_interval(present_interval);
		renderer.set_unattended(unattended.is_some());
		let simulation_step = positive_rate("simulation-rate").map(|rate| MediaTime::from_seconds_f64(1.0 / rate));
		let render_on_demand = application
			.get_parameter("render-on-demand")
			.is_some_and(|parameter| parameter.as_bool_simple());
		queue_startup_parameters(application.parameters(), &configuration, RENDER_PASS_CONFIGURATION);
		queue_startup_parameters(application.parameters(), &configuration, DEBUG_VIEW_CONFIGURATION);
		let ui_scale = configuration.register(UI_CONFIGURATION);
		ui_scale.declare(crate::ui::scale::SETTINGS.iter().map(|setting| setting.parameter(1.0)));
		queue_startup_parameters(application.parameters(), &configuration, UI_CONFIGURATION);
		let gamepad_dead_zone = configuration.register(GAMEPAD_DEAD_ZONE_CONFIGURATION);
		gamepad_dead_zone.declare(
			GAMEPAD_DEAD_ZONE_SETTINGS
				.iter()
				.map(|setting| setting.parameter(input::DeadZone::OFF)),
		);
		queue_startup_parameters(application.parameters(), &configuration, GAMEPAD_DEAD_ZONE_CONFIGURATION);
		let accessibility = configuration.register(ACCESSIBILITY_CONFIGURATION);
		accessibility.declare(
			crate::accessibility::SETTINGS
				.iter()
				.map(|setting| setting.parameter(crate::accessibility::Choices::default())),
		);
		queue_startup_parameters(application.parameters(), &configuration, ACCESSIBILITY_CONFIGURATION);

		let kill_after = application
			.get_parameter("kill-after")
			.map(|parameter| parameter.parse::<u64>().unwrap_or_else(|error| panic!("{error}")));
		let metrics_out = application
			.get_parameter("metrics-out")
			.map(|parameter| std::path::PathBuf::from(parameter.value()));
		let destination = application.get_parameter("debug.streams").map(|parameter| parameter.value());
		let streams = (destination.is_some() || console_running).then(|| {
			let changes_only = application
				.get_parameter("debug.streams.changes-only")
				.is_some_and(|parameter| parameter.as_bool_simple());
			let output = crate::inspector::StreamOutput::open(destination, changes_only, start_time, services.retained_streams)
				.unwrap_or_else(|error| {
					panic!(
						"Data stream destination could not be opened. The most likely cause is that the `debug.streams` path '{}' names a directory that does not exist: {error}",
						destination.unwrap_or_default()
					)
				});
			Arc::new(output)
		});

		let streams = crate::inspector::DebugDataStreams::new(&world_messages, streams);

		let render_image_factory = application.messages().factory();
		let render_image_factory_listener = render_image_factory.listener();
		let render_images_ready = application.messages().channel();
		let render_image_extents = application.messages().channel().listener();
		let render_image_deletes = application.messages().channel().listener();

		GraphicsApplication {
			application,
			window_events: services.window_events,

			inspector: services.inspector,
			_http_inspector: services.http_inspector,
			screenshot_broker: services.screenshot_broker,
			inspector_events: services.inspector_events,
			configuration,

			window_factory: (services.window_factory, services.window_factory_listener),
			window_features: services.window_features,
			window_pointers: services.window_pointers,
			window_cameras: services.window_cameras,
			window_closes: services.window_closes,
			render_image_factory: (render_image_factory, render_image_factory_listener),
			render_images_ready,
			render_image_extents,
			render_image_deletes,
			polled_window_events: Vec::new(),
			window_scales: Vec::new(),
			ui_scale: (1.0, ui_scale),
			text_scale: 1.0,
			accessibility: (crate::accessibility::Choices::default(), accessibility),
			system_preferences: ghi::window::Preferences::default(),
			input_layers: Vec::new(),

			generator_factory: services.generator_factory,

			scenes: vec![services.world],
			shown_transforms: services.shown_transforms,

			input: services.input,
			actions: services.actions,
			gamepad_system: input::gamepad::GamepadSystem::new(),
			gamepad_device_class_handle: None,
			gamepad_dead_zone: (input::DeadZone::OFF, gamepad_dead_zone),
			resource_manager,
			renderer,
			loader: LoaderState::Unused,
			deferred_tasks: Vec::with_capacity(8),

			threads,
			alley: Some(alley),

			close: false,

			tick_count: 0,
			start_time,
			elapsed: MediaTime::from_std(start_time.elapsed()),
			last_tick_instant: std::time::Instant::now(),
			last_present_time: None,
			present_interval,
			simulation_step,
			simulation_pending: MediaTime::ZERO,
			simulation_elapsed: MediaTime::ZERO,
			simulation_alpha: 1.0,
			display_time: MediaTime::ZERO,
			skipped_frame_pace: skipped_frame_pace(present_interval, None),
			elapsed_at_last_present: MediaTime::ZERO,
			render_on_demand,
			unattended,
			next_frame_at: std::time::Instant::now(),
			rendering_active: true,
			waker: services.waker,
			platform_waker_set: false,
			requested_tick: None,

			metrics_out,
			streams,

			kill_after,

			graphics_device,
		}
	}

	/// Returns the name the application was created with.
	pub fn get_name(&self) -> &str {
		self.application.get_name()
	}

	/// Runs one graphics tick without application code and returns whether the application keeps running.
	pub fn tick(&mut self) -> bool {
		self.tick_with(|_, _| {}).is_some()
	}

	/// Returns frame-local storage for temporary allocations during the current tick.
	pub fn frame_allocator(&self) -> &bumpalo::Bump {
		&self.application.frame_allocator
	}

	/// Returns how far the frame about to be drawn lies between the two most recent simulation steps, from 0 to 1.
	///
	/// Renderers show an entity that publishes every step blended by this amount; see
	/// [`ShownTransforms`]. Frame code that places anything over the scene, such
	/// as UI that follows such an entity, blends the same way to keep pace with what they draw.
	pub fn simulation_alpha(&self) -> f32 {
		self.simulation_alpha
	}

	/// Returns the configuration exchange used to inspect startup update results.
	pub fn configuration(&self) -> &Configuration {
		&self.configuration
	}

	/// Advances the application clock by one frame.
	///
	/// Presented times form the display clock: when the swapchain reports a newer one, `elapsed` lands on the
	/// point that display interval reaches, so deltas follow the display cadence instead of CPU scheduling noise.
	/// A swapchain that schedules frames ahead reports the time this tick's frame will be shown instead; successive
	/// targets measure the same intervals, and since a target lies ahead of the previous tick it is never stale.
	/// Without a newer presented time (startup, skipped frames, a window that cannot be seen, or a report that arrives
	/// late) the tick advances by the wall-clock interval; the next presented time discounts that advance instead of
	/// charging it twice.
	///
	/// After the loop waited for events, `elapsed` still advances by the time spent waiting, but the delta is capped
	/// at one frame so systems stepping by it do not jump across the idle stretch. Any other interval is capped at
	/// [`STALL_FRAMES`] paces the same way, so a stall the loop did not wait through, such as a sleep or a presentation
	/// the system held for a second, cannot advance a clip or a body by that whole second in one tick.
	fn sample_frame_time(&mut self, present_time: Option<std::time::Instant>, waited: bool) -> Time {
		// An unattended run advances by exactly one frame period, whatever the wall clock did, so runs repeat and a
		// slow machine runs slower than real time instead of skipping time.
		if let Some(period) = self.unattended {
			let delta = MediaTime::from_std(period);
			self.elapsed += delta;
			return Time {
				elapsed: self.elapsed,
				delta,
			};
		}
		let now = std::time::Instant::now();
		// A presented time from more than a frame before the previous tick shows a frame from before a stretch in which
		// no tick read one, such as while the window was hidden or the loop idled. Anchoring the clock to it would fold
		// that whole stretch into the delta of the next presented time, so it counts as none.
		let present_time =
			present_time.filter(|present_time| *present_time + self.skipped_frame_pace >= self.last_tick_instant);
		let interval = match (present_time, self.last_present_time) {
			(Some(current), Some(previous)) if current > previous => {
				let target = self.elapsed_at_last_present + MediaTime::from_std(current - previous);
				target.max(self.elapsed) - self.elapsed
			}
			_ => MediaTime::from_std(now - self.last_tick_instant),
		};
		let (delta, advance) = frame_delta(interval, MediaTime::from_std(self.skipped_frame_pace), waited);
		self.last_tick_instant = now;
		self.elapsed += advance;
		if present_time.is_some() && present_time != self.last_present_time {
			self.last_present_time = present_time;
			self.elapsed_at_last_present = self.elapsed;
		}
		Time {
			elapsed: self.elapsed,
			delta,
		}
	}

	/// Closes the window created with `window`: the main window quits the application on the next tick, and any other
	/// closes alone.
	fn close_window(&mut self, window: crate::core::factory::Handle) {
		if self.renderer.main_window() == Some(window) {
			self.application.events().send(Events::Close);
		} else {
			self.renderer.remove_window(window);
		}
	}

	/// The native window id of the open window created with `window`, which [`ghi::window::Event::Window`] events on
	/// the message channel name it by. `None` until the renderer opened the window, the tick after it's created.
	pub fn window_id(&self, window: crate::core::factory::Handle) -> Option<ghi::window::WindowId> {
		self.renderer.window_id(window)
	}

	/// The size a UI drawn in the window `id` lays out at: the window's pixels divided by its [`Self::ui_scale`], so
	/// the UI keeps its physical size on every display. `None` until the window reported its size.
	///
	/// Pass it to [`crate::ui::layout::engine::Engine::evaluate`] each frame, since it changes as the window resizes,
	/// moves to another display, or `ui.scale` changes. Window events name the window by `id`, and [`Self::window_id`]
	/// gives the id of a window you created. See [Scale](/docs/reference/gui#scale).
	pub fn layout_size(&self, id: ghi::window::WindowId) -> Option<crate::ui::Size> {
		let (pixels, scale) = self.window_pixels(id)?;
		Some(crate::ui::scale::layout_size(pixels, scale))
	}

	/// The pixels per layout unit of a UI drawn in the window `id`: its display scale times `ui.scale`. Multiply a
	/// layout size by it to size something in pixels, such as a render image a UI shows at its own size.
	pub fn ui_scale(&self, id: ghi::window::WindowId) -> Option<f32> {
		self.window_pixels(id).map(|(_, scale)| scale)
	}

	/// The size in pixels of the window `id` and its UI scale, from its last resize.
	fn window_pixels(&self, id: ghi::window::WindowId) -> Option<(crate::ui::Size, f32)> {
		let (_, pixels, scale) = self.window_scales.iter().find(|(window, ..)| *window == id)?;
		Some((*pixels, scale * self.ui_scale.0))
	}

	/// Routes window input events and reports whether the platform or any window requested close.
	///
	/// Window state changes request a frame, including a window becoming visible again, since frames skip windows
	/// that cannot be seen. Input events do not: their consumers publish what changed.
	fn process_window_events(&mut self, wait: ghi::window::Wait) -> bool {
		let _span = debug_span!("GraphicsApplication::process_window_events").entered();
		let mut close = false;
		let mut redraw = false;
		let mut display_changed = false;
		// A new `ui.scale` lays every UI out again, as a resize does.
		let (scale, port) = &mut self.ui_scale;
		let before = *scale;
		port.drain(scale, crate::ui::scale::SETTINGS);
		redraw |= *scale != before;
		let (choices, port) = &mut self.accessibility;
		port.drain(choices, crate::accessibility::SETTINGS);
		let (dead_zone, port) = &mut self.gamepad_dead_zone;
		port.drain(dead_zone, GAMEPAD_DEAD_ZONE_SETTINGS);
		if let Some(gamepad) = self.gamepad_device_class_handle {
			// A dead zone that changes nothing is left out, so stick values skip it.
			self.input
				.set_dead_zone(gamepad, (*dead_zone != input::DeadZone::OFF).then_some(*dead_zone));
		}
		// The events are taken out of the renderer first, since a closed window is removed from it while they're read.
		let mut events = std::mem::take(&mut self.polled_window_events);
		events.extend(self.renderer.poll_windows(wait));
		for event in events.drain(..) {
			match &event {
				ghi::window::Event::App(ghi::window::AppEvents::Quit) => close = true,
				// Dialog results only matter to whoever opened the dialog, which reads them from the window events.
				ghi::window::Event::App(ghi::window::AppEvents::Dialog { .. }) => {}
				ghi::window::Event::App(ghi::window::AppEvents::Preferences(preferences)) => {
					self.system_preferences = *preferences;
				}
				&ghi::window::Event::Window { window: id, event } => {
					let window = self.renderer.window_handle(id);
					if matches!(event, ghi::window::Events::Close) {
						// The main window's close button quits; another window's closes that window alone.
						match window {
							Some(window) if self.renderer.main_window() != Some(window) => self.renderer.remove_window(window),
							_ => close = true,
						}
					}
					match event {
						ghi::window::Events::Resize { width, height, scale } => {
							let pixels = crate::ui::Size::new(width, height);
							match self.window_scales.iter_mut().find(|(window, ..)| *window == id) {
								Some(entry) => *entry = (id, pixels, scale),
								None => self.window_scales.push((id, pixels, scale)),
							}
						}
						ghi::window::Events::Close => self.window_scales.retain(|(window, ..)| *window != id),
						_ => {}
					}
					display_changed |= matches!(event, ghi::window::Events::DisplayChanged { .. });
					redraw |= matches!(
						event,
						ghi::window::Events::Resize { .. }
							| ghi::window::Events::FocusChanged(_)
							| ghi::window::Events::Minimize
							| ghi::window::Events::Maximize
							| ghi::window::Events::DisplayChanged { .. }
							| ghi::window::Events::VisibilityChanged(true)
					);
					if process_default_window_input(&mut self.input, event, window) {
						self.actions.cancel_seat(input::SeatHandle::stub());
					}
				}
			}
			self.window_events.send(event);
		}
		self.polled_window_events = events;
		// A new `accessibility.` setting, or a new system preference one follows, changes how UIs animate, whether
		// motion blur draws, and how large text is.
		let choices = self.accessibility.0;
		let motion = crate::accessibility::motion_in_effect(choices.motion, &self.system_preferences);
		redraw |= self.renderer.motion() != motion;
		self.renderer.set_motion(motion);
		let text_scale = crate::accessibility::text_scale_in_effect(choices.text_scale, &self.system_preferences);
		redraw |= std::mem::replace(&mut self.text_scale, text_scale) != text_scale;
		if redraw {
			self.renderer.request_redraw();
		}
		if display_changed {
			self.skipped_frame_pace = skipped_frame_pace(self.present_interval, self.renderer.refresh_interval());
		}
		close
	}

	/// Handles window events until every visible window's display can take a frame, and reports whether any window
	/// requested close.
	///
	/// This is where a rendering tick waits for the display. Waiting in the event call lets the application handle
	/// events, such as being brought to the front, however long the display holds back, as it can for a window that
	/// another window just covered. A window that stops being visible during the wait stops being waited for. After
	/// [`STALL_FRAMES`] frame periods, a stall, the tick goes on, and windows still without an image skip the frame.
	fn wait_for_displays(&mut self) -> bool {
		let _span = debug_span!("GraphicsApplication::wait_for_displays").entered();
		let deadline = std::time::Instant::now() + self.skipped_frame_pace * STALL_FRAMES as u32;
		let mut close = false;
		while !close && !self.renderer.displays_ready() && std::time::Instant::now() < deadline {
			close |= self.process_window_events(ghi::window::Wait::Until(deadline));
		}
		close
	}

	/// Chooses how long this tick may wait for window events, and marks the loop as waiting when it may.
	///
	/// Only an idle on-demand loop with a window system connection waits. Pair a waiting result with
	/// [`crate::application::waker::end_wait`] once the wait returned.
	fn event_wait(&mut self) -> ghi::window::Wait {
		if self.kill_after.is_some() {
			// Smoke runs count ticks, so they keep ticking while idle.
			return ghi::window::Wait::Immediate;
		}
		// Unattended runs pace themselves in the tick, so they never wait for events either.
		if !self.render_on_demand || self.rendering_active || self.unattended.is_some() {
			return ghi::window::Wait::Immediate;
		}
		if !self.platform_waker_set {
			let Some(platform_waker) = self.renderer.app_waker() else {
				// Without a window there is no event queue to wait in.
				return ghi::window::Wait::Immediate;
			};
			self.waker.set_platform_waker(move || platform_waker.wake());
			self.platform_waker_set = true;
			// Connecting controllers report nothing through windows, so the event queue also wakes on device changes.
			if let Some(monitor) = self.gamepad_system.monitor()
				&& let Err(error) = self.renderer.wake_on_hid_changes(monitor)
			{
				log::warn!("{error}");
			}
		}

		let woken = self.waker.begin_wait();
		let now = std::time::Instant::now();
		// Gamepads report nothing through the window system, so they are polled once per frame when they need it.
		let poll_at = self.gamepad_system.needs_polling().then(|| now + self.skipped_frame_pace);
		let wait = idle_wait(woken, self.requested_tick.take(), poll_at, now);
		if wait == ghi::window::Wait::Immediate {
			self.waker.end_wait();
		}
		wait
	}

	/// Adopts newly connected gamepads and records their trigger values into the collector.
	fn process_gamepad_events(&mut self) {
		let _span = debug_span!("GraphicsApplication::process_gamepad_events").entered();
		self.gamepad_system.poll(&mut self.input, self.gamepad_device_class_handle);
	}

	/// Adopts newly created windows, cameras, the cameras windows show, directional lights, and fogs before renderer
	/// preparation.
	fn prepare_renderer_state(&mut self) {
		let _span = debug_span!("GraphicsApplication::prepare_renderer_state").entered();
		while let Some(message) = self.window_factory.1.read() {
			self.renderer.create_window(message.handle(), message.into_data());
		}
		while let Some(change) = self.window_features.read() {
			self.renderer.set_window_features(change.window(), change.features());
		}
		while let Some(change) = self.window_pointers.read() {
			self.renderer.set_window_pointer(change.window(), change.pointer());
		}
		while let Some(close) = self.window_closes.read() {
			self.close_window(close.window());
		}
		while let Some(message) = self.render_image_factory.1.read() {
			let handle = message.handle();
			let image = self.renderer.create_render_image(handle, message.into_data());
			self.render_images_ready.send(rendering::RenderImageReady::new(handle, image));
		}
		while let Some(change) = self.render_image_extents.read() {
			self.renderer.set_render_image_extent(change.render_image(), change.extent());
		}
		while let Some(delete) = self.render_image_deletes.read() {
			self.renderer.remove_render_image(*delete.handle());
		}
		for (index, scene) in self.scenes.iter_mut().enumerate() {
			let id = SceneId::new(index);
			while let Some(message) = scene.cameras.read() {
				self.renderer.create_camera(message.handle(), message.into_data());
			}
			while let Some(message) = scene.directional_lights.read() {
				self.renderer
					.create_directional_light(id, message.handle(), message.into_data());
			}
			while let Some(message) = scene.fogs.read() {
				self.renderer.create_fog(id, message.into_data());
			}
		}
		while let Some(change) = self.window_cameras.read() {
			self.renderer.set_window_camera(change.window(), change.camera());
		}
	}

	/// Renders one frame, completes every screenshot request with its readbacks, and returns the frame's identity.
	///
	/// Every capture of every request is read from this one frame. Transports encode the readbacks on their own
	/// threads, so encoding never delays the next frame.
	fn render_frame(
		&mut self,
		requests: Vec<crate::inspector::screenshot::ScreenshotRequest>,
		time: crate::application::Time,
	) -> u64 {
		let _span = debug_span!("GraphicsApplication::render_frame").entered();
		let captures = requests
			.iter()
			.flat_map(|request| request.captures.iter().map(|selection| (selection.sink, &selection.capture)))
			.collect::<Vec<_>>();
		self.shown_transforms.resolve(self.display_time);
		let alley = self.alley.as_mut().expect(
			"Alley is not available to render. The most likely cause is that a frame rendered from a lane of a run_lanes execution, which holds the lanes.",
		);
		let (frame, results) = self.renderer.prepare(
			&self.shown_transforms,
			&self.application.frame_allocator,
			alley,
			&captures,
			time.elapsed(),
			time.delta(),
		);
		// Results follow the flattened capture order, so each request takes the next run of its own length.
		let mut results = results.into_iter();
		for request in requests {
			let captures = results
				.by_ref()
				.take(request.captures.len())
				.map(|result| result.map_err(crate::inspector::screenshot::ScreenshotError::from))
				.collect();
			request.complete(crate::inspector::screenshot::Screenshots { frame, captures });
		}
		frame
	}

	/// Panics when a configured fixed simulation rate cannot be met by simulating once per presented frame.
	///
	/// [`Self::tick_with`] ties the two rates together, so a `simulation-rate` that disagrees with the rate frames
	/// reach the screen at is a loop the application cannot run. The check repeats every tick because a window
	/// moved to another display changes the refresh rate mid-run, and because the first window arrives after
	/// startup. An unknown presentation cadence cannot be shown to disagree, so it passes.
	fn assert_tied_simulation_rate(&self) {
		let Some(step) = self.simulation_step else {
			return;
		};
		// Frames arrive at the unattended rate, or otherwise no faster than the slower of the cap and the display.
		let Some(present_interval) = self
			.unattended
			.or_else(|| self.present_interval.max(self.renderer.refresh_interval()))
		else {
			return;
		};
		assert!(
			rates_are_tied(step, MediaTime::from_std(present_interval)),
			"A simulation rate of {:.2} Hz cannot be met by a loop that presents {:.2} frames per second. The most likely cause is a simulation-rate parameter under GraphicsApplication::tick_with, which simulates once per frame; call GraphicsApplication::tick_stepped_with instead.",
			1.0 / step.as_seconds_f64(),
			1.0 / present_interval.as_secs_f64(),
		);
	}

	/// Runs the whole fixed steps this frame owes and schedules the tick the next one is due in.
	fn run_simulation_steps<S: FnMut(&mut Self, Time)>(&mut self, delta: MediaTime, simulate: &mut S) {
		let step = self.simulation_step.unwrap_or(DEFAULT_SIMULATION_STEP);
		self.simulation_pending += delta;
		let (steps, remainder) = simulation_steps(self.simulation_pending, step, MAX_SIMULATION_STEPS_PER_FRAME);
		self.simulation_pending = remainder;
		for _ in 0..steps {
			let _span = debug_span!("GraphicsApplication::simulation_step").entered();
			self.simulation_elapsed += step;
			let time = Time::new(self.simulation_elapsed, step);
			simulate(self, time);
			// The step's state is complete once the callback has stepped its systems.
			self.shown_transforms.step(time);
		}
		// Frames show the world one step behind, moving from the previous step's state to the latest one's as
		// the pending time fills the step.
		self.simulation_alpha = self.simulation_pending.as_seconds_f32() / step.as_seconds_f32();
		self.display_time = self.simulation_elapsed - step + self.simulation_pending;
		// An idle on-demand loop waits in the window system, so the next step has to be one of the deadlines it
		// waits for. Frames the loop runs anyway reach the step sooner and cost nothing here.
		self.schedule_tick(Some(std::time::Instant::now() + (step - self.simulation_pending).to_std()));
	}

	/// Runs one graphics tick and lets application code update state before rendering.
	///
	/// Simulation advances once per tick, by the frame delta. A `simulation-rate` that disagrees with the rate
	/// frames are presented at panics here; use [`Self::tick_stepped_with`] to run the two rates apart.
	pub fn tick_with<R, F: FnOnce(&mut Self, Time) -> R>(&mut self, f: F) -> Option<R> {
		self.assert_tied_simulation_rate();
		self.tick_internal(|application, time| {
			let result = debug_span!("GraphicsApplication::user_tick").in_scope(|| f(application, time));
			// The step's state is complete once the callback has stepped its systems.
			application.shown_transforms.step(time);
			application.simulation_alpha = 1.0;
			application.display_time = time.elapsed();
			result
		})
	}

	/// Runs `lanes` once on every lane of the application's alley, from a tick callback, and returns when every
	/// lane has finished.
	///
	/// `lanes` receives each lane's [`ConcreteLane`](crate::core::alley::ConcreteLane), the context for lane
	/// indices, single-runner sections, and collectives such as
	/// [`ConcreteLane::each_shared`](crate::core::alley::ConcreteLane::each_shared). What the lanes publish,
	/// such as created bodies and their transforms, is complete when this returns, so a system stepped after it,
	/// such as [`Physics::step`](crate::physics::Physics::step), sees it whole instead of racing it. A lane panic
	/// resumes here.
	///
	/// The lanes cannot borrow the application; take the handles they publish through beforehand, such as
	/// [`MessageScope::factory`], and share other state through `Sync` values. Executions cannot nest: calling
	/// this from a lane, or while [`Self::alley_mut`] is borrowed, panics.
	pub fn run_lanes<L>(&mut self, lanes: L)
	where
		L: crate::core::alley::AlleyFunction<()>,
	{
		// The lanes leave the application for as long as the execution runs, so a caller cannot borrow it from
		// a lane.
		let mut alley = self.alley.take().expect(
			"Alley is already executing. The most likely cause is that run_lanes was called from a lane of another execution.",
		);
		let executed = alley.execute(lanes);
		// Return the lanes before a panic resumes, so the application drops with them in place.
		self.alley = Some(alley);
		executed.unwrap_or_else(|panic| std::panic::resume_unwind(panic));
	}

	/// Runs one graphics tick whose simulation advances in fixed steps, independent of the frame rate.
	///
	/// `simulate` runs zero or more times before the frame, each time with a [`Time`] whose delta is the fixed
	/// step from `simulation-rate` and whose elapsed value counts the steps run so far. Step the systems that
	/// simulate, such as [`Physics::step`](crate::physics::Physics::step), from it. `frame` then runs once with
	/// the frame's own display-derived [`Time`], for the work that belongs to what is about to be drawn: the UI,
	/// cameras, and animation.
	///
	/// A frame that owes more steps than the loop runs in one tick drops the outstanding time instead of running
	/// it later. Frames show the world one step behind, moving each entity between the samples its steps published,
	/// so a simulation rate below the frame rate still moves every frame; see
	/// [shown transforms](/docs/reference/rendering#shown-transforms).
	pub fn tick_stepped_with<R, S: FnMut(&mut Self, Time), F: FnOnce(&mut Self, Time) -> R>(
		&mut self,
		mut simulate: S,
		frame: F,
	) -> Option<R> {
		self.tick_internal(move |application, time| {
			application.run_simulation_steps(time.delta, &mut simulate);
			let _span = debug_span!("GraphicsApplication::user_tick").entered();
			frame(application, time)
		})
	}

	/// Runs the tick every entry point shares, calling `run` where application updates belong, then
	/// closes the tick's metrics row once the tick's own span has ended so its time lands in this row.
	fn tick_internal<R, F: FnOnce(&mut Self, Time) -> R>(&mut self, run: F) -> Option<R> {
		let (result, clock) = self.tick_span(run);
		self.application.metrics().end_tick(clock);
		// Raised after the row exists, so a woken request reads this tick and the configuration states it settled.
		if self.close {
			self.inspector_events.close();
			self.write_metrics_out();
		} else {
			self.inspector_events.notify();
		}
		result
	}

	/// Writes the `metrics-out` file once, with every tick that has ended so far.
	fn write_metrics_out(&mut self) {
		let Some(path) = self.metrics_out.take() else {
			return;
		};
		let metrics = self.application.metrics();
		if let Err(error) = write_metrics(&path, metrics, &metrics.summary(None, false)) {
			log::error!(
				"Metrics could not be written to {}. The most likely cause is that the directory does not exist or is not writable: {error}",
				path.display()
			);
		}
	}

	/// Runs one tick inside its span and reports what the tick's metrics row records about it.
	fn tick_span<R, F: FnOnce(&mut Self, Time) -> R>(&mut self, run: F) -> (Option<R>, TickClock) {
		let _span = debug_span!("GraphicsApplication::tick").entered();

		self.application.message_bus().begin_tick();
		debug_span!("GraphicsApplication::reset_frame_allocator").in_scope(|| self.application.frame_allocator.reset());
		let wait = self.event_wait();
		let waited = wait != ghi::window::Wait::Immediate;
		let mut close = self.process_window_events(wait);
		if waited {
			self.waker.end_wait();
		}
		close |= matches!(self.application.next_event(), Some(Events::Close));

		// Existing windows pace simulation on the display; newly published windows are adopted after the
		// callback so their scene can request resources before native window setup begins.
		// An acquired image must be presented, so on-demand rendering only hoists the acquisition while frames keep
		// coming; the first frame after an idle stretch acquires when it renders.
		// Windows that cannot be seen acquire nothing, so a tick that only renders for them sleeps below.
		let present_time = if self.rendering_active {
			close |= self.wait_for_displays();
			self.renderer.acquire_swapchain_images(|_| false)
		} else {
			None
		};
		if let Some(period) = self.unattended {
			// Unattended runs render every window and nothing waits on a display, so the loop holds their rate here.
			std::thread::sleep(self.next_frame_at.saturating_duration_since(std::time::Instant::now()));
			self.next_frame_at = next_frame_deadline(self.next_frame_at, period, std::time::Instant::now());
		} else if self.tick_count > 0 && !waited && !self.renderer.presents_this_frame() {
			// The first tick has no previous frame to pace. Later ticks that neither present nor wait for
			// events sleep here so a windowless or unchanged application does not spin a core.
			std::thread::sleep(self.skipped_frame_pace);
		}
		let time = self.sample_frame_time(present_time, waited);

		self.process_gamepad_events();

		// Layers pull first, so the world's sink only resolves the input they passed. It pulls last and captures
		// nothing.
		debug_span!("GraphicsApplication::update_input").in_scope(|| {
			for layer in &mut self.input_layers {
				layer(&mut self.input);
			}
			self.actions.pull(&mut self.input, |_| input::Capture::Passed)
		});

		self.streams.update();
		let result = run(self, time);
		self.streams.update();

		// Transforms the frame code wrote are placements; adopting them every tick keeps the route from holding
		// back its publishers while nothing renders.
		self.shown_transforms.adopt();
		self.renderer.update(&self.shown_transforms);
		self.prepare_renderer_state();
		let screenshot_requests = self.screenshot_broker.drain();
		// Ask the renderer even when rendering anyway so passes adopt this tick's inputs before deciding next tick.
		let changed = self.renderer.needs_frame() || !screenshot_requests.is_empty();
		self.rendering_active = changed || !self.render_on_demand || self.unattended.is_some();
		let frame = (self.rendering_active || self.renderer.presents_this_frame())
			.then(|| self.render_frame(screenshot_requests, time));
		// Rendering may acquire more than the tick's start did, such as a hidden window a screenshot captures.
		let presented = frame.is_some_and(|frame| self.renderer.frame_acquired(frame));

		let clock = TickClock {
			tick: self.tick_count,
			frame,
			time: self.start_time.elapsed(),
			delta: time.delta.to_std(),
			presented,
		};
		self.tick_count += 1;
		self.streams.set_tick(self.tick_count);

		if let Some(kill_after) = self.kill_after
			&& self.tick_count >= kill_after
		{
			close = true;
		}

		if close {
			self.close();
			(None, clock)
		} else {
			(Some(result), clock)
		}
	}

	/// Drains application-side lifecycle events, then signals and joins every
	/// application worker.
	fn stop_worker_threads(&mut self) {
		if self.threads.is_empty() {
			return;
		}

		while self.application.next_event().is_some() {}
		self.application.events().send(Events::Close);
		self.threads.drain(..).for_each(|thread| {
			let _ = thread.join();
		});
	}

	/// Flags the application for closing.
	pub fn close(&mut self) {
		self.close = true;

		self.stop_worker_threads();

		let summary = self.application.metrics().summary(None, false);
		if let Some(delta) = &summary.delta {
			log::debug!(
				"Run stats over the last {} ticks: frame delta mean {:.3} ms, min {:.3} ms, max {:.3} ms, p99 {:.3} ms; time to first frame {:?}.",
				summary.ticks,
				delta.mean,
				delta.min,
				delta.max,
				delta.p99,
				summary.time_to_first_frame,
			);
		}
	}

	/// Returns a waker that makes this application run its next tick from any thread.
	///
	/// Hand it to whatever changes state the loop cannot observe through window events, such as a worker thread or
	/// a UI engine through `Engine::set_waker`. It only matters with `render-on-demand`, where an idle loop waits
	/// in the window system.
	pub fn waker(&self) -> LoopWaker {
		self.waker.clone()
	}

	/// Asks the loop to run a tick at `at`, or as soon as it can when `at` has passed.
	///
	/// Systems the loop drives itself, such as a UI engine, report their next evaluation this way: pass
	/// `Engine::next_tick` from the tick callback. Without a request, an idle `render-on-demand` loop waits for
	/// events and wakes. The earliest request wins, and each tick starts without one.
	pub fn schedule_tick(&mut self, at: Option<std::time::Instant>) {
		self.requested_tick = self.requested_tick.into_iter().chain(at).min();
	}

	/// Borrows the persistent lanes for independent application or frame work.
	///
	/// Use [`crate::core::alley::Alley::join`] for caller-bound work or
	/// [`crate::core::alley::Alley::execute_with_mut`] for resources with stable lane ownership.
	/// Every dispatch finishes before returning to the application's next phase.
	///
	/// Systems that step on the lanes, such as [`Physics::step`](crate::physics::Physics::step), take them from
	/// here.
	pub fn alley_mut(&mut self) -> &mut crate::core::alley::Alley {
		self.alley.as_mut().expect(
			"Alley is not available. The most likely cause is that alley_mut was called from a lane of a run_lanes execution, which holds the lanes.",
		)
	}

	/// Returns the collector that owns the registered devices and their control values.
	///
	/// Declared [`Action`] values are published through the world's
	/// [`ActionEvent`](input::ActionEvent) channel.
	pub fn input(&self) -> &input::InputCollector {
		&self.input
	}

	/// Returns the collector with the sink that resolves the world's declared [`Action`] values.
	///
	/// Use the sink to change what drives those actions: apply a player's saved
	/// [`Bindings`](input::Bindings) with [`InputSink::apply`](input::InputSink::apply),
	/// replace one action with [`InputSink::rebind`](input::InputSink::rebind), and take
	/// the replacements to save with [`InputSink::bindings`](input::InputSink::bindings).
	/// The collector is mutable so you can add a sink of your own, such as one a
	/// controls screen reads presses through.
	pub fn actions_mut(&mut self) -> (&mut input::InputCollector, &mut input::InputSink) {
		(&mut self.input, &mut self.actions)
	}

	/// Adds a layer of input that takes its share before the world's [`Action`] values resolve, such as a tool drawn
	/// over the game that keeps the clicks it handles.
	///
	/// Each tick, the application calls `layer` with [`Self::input`], in the order the layers were added, then
	/// resolves the world's actions from the input the layers passed. In it, pull a sink you added to the collector
	/// through [`Self::actions_mut`] with [`InputSink::pull`](input::InputSink::pull), and capture what it handles.
	/// Next, cancel the world's held actions with [`InputSink::cancel_seat`](input::InputSink::cancel_seat) when the
	/// layer starts taking input a held action was using.
	pub fn add_input_layer(&mut self, layer: impl FnMut(&mut input::InputCollector) + 'static) {
		self.input_layers.push(Box::new(layer));
	}

	/// Returns the renderer used by setup functions and advanced render integrations.
	pub fn renderer(&self) -> &Renderer {
		&self.renderer
	}

	/// Returns mutable renderer access for application-defined pipeline setup.
	///
	/// Use this during startup to create renderer-owned GHI resources, such as the samplers a pipeline binds
	/// loaded images with. Then call [`Renderer::add_pipeline_manager`] before the application starts rendering.
	/// For asynchronous loading, create lanes from [`Self::loader_mut`].
	pub fn renderer_mut(&mut self) -> &mut Renderer {
		&mut self.renderer
	}

	/// Returns the loader that uploads resources for every pipeline, creating it on first use.
	///
	/// Create a pipeline's lanes with [`rendering::loading::spawn`] during setup. The loader then starts on the
	/// loading thread with [`defaults::launch_deferred_tasks_thread`].
	///
	/// # Panics
	///
	/// Panics after the loading thread started, because a lane created then would never be served.
	pub fn loader_mut(&mut self) -> &mut rendering::loading::Loader {
		self.loader_and_renderer_mut().0
	}

	/// Returns the loader and the renderer together, for setup that shares render buffers with the loader.
	pub(crate) fn loader_and_renderer_mut(&mut self) -> (&mut rendering::loading::Loader, &mut Renderer) {
		if let LoaderState::Unused = self.loader {
			self.loader = LoaderState::Pending(rendering::loading::Loader::new(
				&self.graphics_device,
				self.application.parameters().scope("render.loading"),
			));
		}
		let LoaderState::Pending(loader) = &mut self.loader else {
			panic!(
				"Loader setup failed. The most likely cause is that a pipeline was set up after `launch_deferred_tasks_thread` started the loading thread. Set up every pipeline first."
			);
		};
		(loader, &mut self.renderer)
	}

	/// Queues loading work that spawns its futures on the loading thread's runtime.
	///
	/// Tasks run in the order they were added, after the loader starts, once
	/// [`defaults::launch_deferred_tasks_thread`] moves them to the loading thread.
	///
	/// # Panics
	///
	/// Panics after the loading thread started, because a task added then would never run.
	pub fn add_deferred_task(&mut self, task: impl FnOnce(&compio::runtime::Runtime) + Send + 'static) {
		assert!(
			!matches!(self.loader, LoaderState::Launched),
			"Deferred task was not queued. The most likely cause is that it was added after `launch_deferred_tasks_thread` started the loading thread. Add every task first."
		);
		self.deferred_tasks.push(std::boxed::Box::new(task));
	}

	/// Takes the loader and every deferred task so the loading thread can run them. Setup afterwards can add
	/// neither lanes nor tasks.
	pub(crate) fn take_loading_work(&mut self) -> (Option<rendering::loading::Loader>, Vec<defaults::DeferredTask>) {
		let loader = match std::mem::replace(&mut self.loader, LoaderState::Launched) {
			LoaderState::Pending(loader) => Some(loader),
			LoaderState::Unused | LoaderState::Launched => None,
		};
		(loader, std::mem::take(&mut self.deferred_tasks))
	}

	/// Creates a scene isolated from the world, such as a preview stage, and returns its id.
	///
	/// Objects created on its scope, [`Self::scene_messages`], reach only its own scene pipelines, and its sun and fog
	/// light only its sinks. `name` names the scope in diagnostics. Next, install its scene pipelines with the id, such
	/// as [`setup_pbr_visibility_shading_render_pipeline`], and show it with a [`rendering::RenderImage`]. See
	/// [scenes](/docs/reference/rendering#scenes).
	pub fn create_scene(&mut self, name: &str) -> SceneId {
		let scope = self.application.message_bus().new_scope(name);
		let id = self.renderer.create_scene();
		assert_eq!(
			id.index(),
			self.scenes.len(),
			"The renderer and the application number scenes alike."
		);
		self.shown_transforms.follow(&scope);
		self.scenes.push(SceneRoutes::new(scope));
		id
	}

	/// The scope objects of `scene` are created on.
	///
	/// # Panics
	///
	/// Panics when `scene` is not the world or a scene [`Self::create_scene`] returned.
	pub fn scene_messages(&self, scene: SceneId) -> &MessageScope {
		&self
			.scenes
			.get(scene.index())
			.expect("Scene is unknown. The most likely cause is a scene id from another application. Pass `SceneId::WORLD` or the id of a scene `create_scene` returned.")
			.scope
	}

	/// Returns the factory used to request new windows.
	pub fn window_factory(&self) -> &Factory<Window> {
		&self.window_factory.0
	}

	/// Returns the factory used to request new [render images](rendering::RenderImage), which draw a camera into an
	/// image instead of a window.
	///
	/// Next, listen for [`rendering::RenderImageReady`] on [`Self::messages`] to learn the image the renderer draws
	/// into, then show it in the interface.
	pub fn render_image_factory(&self) -> &Factory<rendering::RenderImage> {
		&self.render_image_factory.0
	}

	/// Returns the application-owned namespace used by headed-runtime channels and factories.
	///
	/// Next, call [`MessageScope::channel`] or [`MessageScope::factory`] to add an
	/// application-defined message route without declaring its type at startup.
	/// Subscribe to [`ghi::window::Event`] before creating windows to handle raw
	/// input in your application callback. Events are published in arrival order,
	/// and each window event carries the [`ghi::window::WindowId`] it targets.
	pub fn messages(&self) -> &MessageScope {
		self.application.messages()
	}

	/// Returns the shared fixed-storage message bus for diagnostics and new scopes.
	///
	/// Next, call [`MessageBus::topics`] to inspect registered routes or
	/// [`MessageBus::new_scope`] to isolate routes owned by another subsystem.
	pub fn message_bus(&self) -> &MessageBus {
		self.application.message_bus()
	}

	/// The motion preference in effect: the `accessibility.motion` setting, or under `system` the operating system's.
	///
	/// Pass it to [`crate::ui::layout::engine::Engine::set_motion`] each frame, beside the size from
	/// [`Self::layout_size`], so a UI follows the setting as it changes. Motion blur follows it on its own. See
	/// [Reduced motion](/docs/reference/gui#reduced-motion).
	pub fn motion(&self) -> crate::accessibility::MotionPreference {
		self.renderer.motion()
	}

	/// The text scale in effect: the `accessibility.text-scale` setting, or under `system` the operating system's.
	///
	/// Pass it to [`crate::ui::layout::engine::Engine::set_text_scale`] each frame, beside the size from
	/// [`Self::layout_size`], so a UI's text follows the setting as it changes. See
	/// [Text scale](/docs/reference/gui#text-scale).
	pub fn text_scale(&self) -> f32 {
		self.text_scale
	}

	/// Registers a system's runtime configuration under `prefix` and queues the startup parameters under it.
	///
	/// The port subscribes before the parameters are queued, so it reports every one of them. The system reads
	/// each parameter by its name inside `prefix`.
	pub(crate) fn configure(&self, prefix: &str) -> crate::configuration::ConfigurationPort {
		let port = self.configuration.register(prefix);
		queue_startup_parameters(self.application.parameters(), &self.configuration, prefix);
		port
	}

	/// Returns the resolved startup parameters.
	///
	/// Next, hand a system its subtree with [`Parameters::scope`], such as
	/// `app.parameters().scope("physics")` for [`Physics::new`](crate::physics::Physics::new).
	pub fn parameters(&self) -> &[Parameter] {
		self.application.parameters()
	}

	/// Returns the world scope, where application code creates entities and the systems that follow them.
	///
	/// Next, create entities with [`Creator::create`] and the systems the application needs on it, such as
	/// [`SceneGraph::new`](crate::gameplay::SceneGraph::new) and [`Physics::new`](crate::physics::Physics::new),
	/// and step them from the tick callback in the order the application wants. The scope is `Clone`, so lanes
	/// and systems keep their own copy.
	pub fn world(&self) -> &MessageScope {
		&self.scenes[SceneId::WORLD.index()].scope
	}

	/// Returns the inspector the console and the HTTP inspector serve, for tools that run inside the application,
	/// such as an in-game editor.
	///
	/// List entities with [`DefaultInspector::entities`] and change them with [`DefaultInspector::post_message`]. See
	/// the [inspector reference](/docs/reference/inspector) for the messages it accepts.
	pub fn inspector(&self) -> &EntityHandle<DefaultInspector> {
		&self.inspector
	}

	/// Returns the audio generator factory used by default audio setup.
	pub fn generator_factory(&self) -> &Factory<std::boxed::Box<dyn Generator>> {
		&self.generator_factory
	}

	/// Runs ticks until the application is closed.
	pub fn do_loop(&mut self) {
		while !self.close {
			self.tick();
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

/// Converts the resolved startup parameters under `prefix` into asynchronous configuration events.
///
/// Register the port that handles `prefix` first; it then reports each parameter as set or not set.
/// The configuration prefix of render-pass states, each named by its pass, such as `render.pass.bloom`.
const RENDER_PASS_CONFIGURATION: &str = "render.pass.";
/// The prefix of the UI settings: `ui.scale`.
const UI_CONFIGURATION: &str = "ui.";
/// The prefix of the gamepad's dead zone settings: `inner` and `outer`.
const GAMEPAD_DEAD_ZONE_CONFIGURATION: &str = "input.gamepad.dead-zone.";

/// The gamepad's dead zone bounds, each checked against the other so the zone stays valid. See
/// [Dead zones](/docs/reference/input#dead-zones).
const GAMEPAD_DEAD_ZONE_SETTINGS: &[crate::configuration::Setting<input::DeadZone>] = &[
	crate::configuration::Setting {
		name: "inner",
		accepts: crate::configuration::Accepts::Number { min: 0.0 },
		value: |dead_zone| crate::configuration::Read::Number(f64::from(dead_zone.bounds().0)),
		apply: |dead_zone, read| {
			let outer = dead_zone.bounds().1;
			input::DeadZone::new(read.number() as f32, outer).map_err(|_| {
				format!(
					"`input.gamepad.dead-zone.inner` was not set. The most likely cause is a bound at or above the outer one, {outer}. Set it from 0 up to `input.gamepad.dead-zone.outer`, such as 0.15."
				)
			})
		},
	},
	crate::configuration::Setting {
		name: "outer",
		accepts: crate::configuration::Accepts::Number { min: 0.0 },
		value: |dead_zone| crate::configuration::Read::Number(f64::from(dead_zone.bounds().1)),
		apply: |dead_zone, read| {
			let inner = dead_zone.bounds().0;
			input::DeadZone::new(inner, read.number() as f32).map_err(|_| {
				format!(
					"`input.gamepad.dead-zone.outer` was not set. The most likely cause is a bound at or below the inner one, {inner}, or above 1. Set it above `input.gamepad.dead-zone.inner` and at most 1, such as 0.95."
				)
			})
		},
	},
];

/// The prefix of the accessibility settings: `accessibility.motion` and `accessibility.text-scale`.
const ACCESSIBILITY_CONFIGURATION: &str = "accessibility.";
/// The configuration prefix of the debug views, named `<pass>.<view>` inside it.
const DEBUG_VIEW_CONFIGURATION: &str = "render.debug.view.";

pub(crate) fn queue_startup_parameters(parameters: &[Parameter], configuration: &Configuration, prefix: &str) {
	for parameter in parameters {
		if parameter.name().starts_with(prefix) {
			configuration.update(parameter.name(), parameter.value());
		}
	}
}

/// The frame period the loop sleeps for when no window presents and neither `max-frame-rate` nor a display
/// refresh rate is known.
const DEFAULT_SKIPPED_FRAME_PACE: std::time::Duration = std::time::Duration::from_micros(16_667);

/// The most skipped-frame paces one tick's delta spans. A longer interval is a stall: the clock still advances by
/// it, but the delta systems step by is capped here, since simulating through a stall pops poses and tunnels bodies.
const STALL_FRAMES: i64 = 4;

/// Chooses the delta a tick hands to systems and the advance of the application clock from the interval the clock
/// measured, which is the display interval when a frame presented and the wall time since the last tick otherwise.
///
/// The clock always advances by the full interval. The delta is capped at one `pace` after the loop waited for
/// events, since an idle stretch is not simulated, and at [`STALL_FRAMES`] paces otherwise, since a stall is not
/// either; see the [time reference](/docs/reference/time).
fn frame_delta(interval: MediaTime, pace: MediaTime, waited: bool) -> (MediaTime, MediaTime) {
	let cap = if waited { pace } else { pace * STALL_FRAMES };
	(interval.min(cap), interval)
}

/// Chooses how long an idle on-demand tick waits for window events.
///
/// A wake that arrived since the last wait, or a UI that needs a tick now, runs the tick at once. Otherwise the
/// earliest of the UI's next tick and the next input poll bounds the wait, and with neither the loop waits for an
/// event.
fn idle_wait(
	woken: bool,
	ui_tick: Option<std::time::Instant>,
	poll_at: Option<std::time::Instant>,
	now: std::time::Instant,
) -> ghi::window::Wait {
	if woken {
		return ghi::window::Wait::Immediate;
	}
	match ui_tick.into_iter().chain(poll_at).min() {
		None => ghi::window::Wait::Forever,
		Some(deadline) if deadline <= now => ghi::window::Wait::Immediate,
		Some(deadline) => ghi::window::Wait::Until(deadline),
	}
}

/// Reads the frame period of an unattended run from the `unattended` parameter.
///
/// Returns `None` when the parameter is absent. The rate is required, so a bare `--unattended` or a rate that is not
/// a positive number stops startup.
fn unattended_period(parameter: Option<&Parameter>) -> Option<std::time::Duration> {
	parameter.map(|parameter| {
		let rate = parameter.parse::<f64>().ok().filter(|rate| rate.is_finite() && *rate > 0.0);
		let rate = rate.unwrap_or_else(|| {
			panic!(
				"The `unattended` parameter has no valid rendering rate: '{}'. The most likely cause is a bare `--unattended` or a value that is not a positive number. Pass a rate in frames per second, such as `--unattended=30`.",
				parameter.value()
			)
		});
		std::time::Duration::from_secs_f64(1.0 / rate)
	})
}

/// Chooses when the next unattended tick may start, given the deadline the tick starting at `now` waited for.
///
/// Deadlines stay one `period` apart, so oversleeping one tick does not slow the rate. A tick that started more
/// than a period late, after a stall, moves the schedule to `now` instead of running the missed ticks back to back.
fn next_frame_deadline(
	deadline: std::time::Instant,
	period: std::time::Duration,
	now: std::time::Instant,
) -> std::time::Instant {
	(deadline + period).max(now)
}

/// Chooses how long a tick that presents nothing sleeps.
///
/// No frame reaches the screen faster than the display refreshes or than the cap allows, so the slower of the two
/// known intervals wins.
fn skipped_frame_pace(
	present_interval: Option<std::time::Duration>,
	refresh_interval: Option<std::time::Duration>,
) -> std::time::Duration {
	present_interval.max(refresh_interval).unwrap_or(DEFAULT_SKIPPED_FRAME_PACE)
}

/// The fixed step [`GraphicsApplication::tick_stepped_with`] uses when `simulation-rate` is not given.
const DEFAULT_SIMULATION_STEP: MediaTime = MediaTime::from_ticks(crate::time::TICKS_PER_SECOND / 60);

/// The most fixed steps one frame runs before the loop drops the outstanding simulation debt.
const MAX_SIMULATION_STEPS_PER_FRAME: u32 = 8;

/// The reciprocal of the largest relative difference at which two rates still count as tied.
const TIED_RATE_TOLERANCE: i64 = 100;

/// Splits accumulated time into whole fixed steps and the remainder that carries into the next frame.
///
/// The remainder is always what is left inside one step, so the steps a frame does run never drift: a frame that
/// owes more than `max_steps` drops the outstanding debt instead of running it later, which costs simulated time
/// rather than making the next frames longer still.
fn simulation_steps(pending: MediaTime, step: MediaTime, max_steps: u32) -> (u32, MediaTime) {
	debug_assert!(
		step > MediaTime::ZERO,
		"Simulation step must be positive. The most likely cause is a zero or negative simulation rate."
	);
	if pending < step {
		return (0, pending);
	}
	let steps = pending.as_ticks() / step.as_ticks();
	let remainder = MediaTime::from_ticks(pending.as_ticks() % step.as_ticks());
	(steps.min(i64::from(max_steps)) as u32, remainder)
}

/// Returns whether a fixed simulation step matches the interval frames are presented at.
///
/// Reported cadences rarely land on the exact step, so a small relative difference, such as a 59.94 Hz display
/// against a 60 Hz step, still counts as tied.
fn rates_are_tied(step: MediaTime, present_interval: MediaTime) -> bool {
	let difference = step.max(present_interval) - step.min(present_interval);
	difference * TIED_RATE_TOLERANCE <= present_interval
}

mod pipeline;
pub use pipeline::{
	setup_aces_color_grading_render_pass, setup_aces_tonemap_render_pass, setup_aces2_color_grading_render_pass,
	setup_agx_tonemap_render_pass, setup_atmosphere_sky_render_pass, setup_bloom_render_pass,
	setup_color_correction_render_pass, setup_debug_mesh_render_pass, setup_debug_overlay_render_pass,
	setup_depth_of_field_reference_render_pass, setup_depth_of_field_render_pass, setup_dwg_color_grading_render_pass,
	setup_exponential_height_fog_render_pass, setup_film_grain_render_pass, setup_hable_tonemap_render_pass,
	setup_lens_flare_render_pass, setup_lut_render_pass, setup_motion_blur_render_pass, setup_particles,
	setup_pbr_visibility_shading_render_pipeline, setup_simple_render_pipeline, setup_smaa_render_pass,
	setup_srgb_display_render_pass, setup_taa_render_pass, setup_ui_render_pass,
};

use crate::application::LoopWaker;

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn gamepad_dead_zone_bounds_stay_in_order_within_zero_to_one() {
		use crate::configuration::{ConfigurationValue, apply_setting};
		let set = |dead_zone, name, value: f64| {
			apply_setting(
				dead_zone,
				GAMEPAD_DEAD_ZONE_SETTINGS,
				GAMEPAD_DEAD_ZONE_CONFIGURATION,
				name,
				&ConfigurationValue::Float(value),
			)
			.map(|(dead_zone, _)| dead_zone.bounds())
		};

		assert_eq!(set(input::DeadZone::OFF, "inner", 0.15), Ok((0.15, 1.0)));
		let narrow = input::DeadZone::new(0.1, 0.5).unwrap();
		assert_eq!(set(narrow, "outer", 0.9), Ok((0.1, 0.9)));
		assert!(set(narrow, "inner", 0.5).is_err(), "an inner bound at the outer one");
		assert!(set(narrow, "outer", 0.05).is_err(), "an outer bound below the inner one");
		assert!(set(narrow, "outer", 1.5).is_err(), "an outer bound above 1");
	}

	#[test]
	fn idle_wait_runs_now_waits_for_the_earliest_deadline_or_waits_for_events() {
		use ghi::window::Wait;
		let now = std::time::Instant::now();
		let soon = now + std::time::Duration::from_millis(5);
		let later = now + std::time::Duration::from_millis(50);
		assert_eq!(idle_wait(true, None, None, now), Wait::Immediate);
		assert_eq!(idle_wait(false, None, None, now), Wait::Forever);
		assert_eq!(idle_wait(false, Some(now), None, now), Wait::Immediate);
		assert_eq!(idle_wait(false, Some(later), None, now), Wait::Until(later));
		assert_eq!(idle_wait(false, Some(later), Some(soon), now), Wait::Until(soon));
		assert_eq!(idle_wait(false, None, Some(soon), now), Wait::Until(soon));
	}

	#[test]
	fn unattended_period_is_one_frame_at_the_given_rate() {
		assert_eq!(unattended_period(None), None);
		assert_eq!(
			unattended_period(Some(&Parameter::new("unattended", "50"))),
			Some(std::time::Duration::from_millis(20))
		);
	}

	#[test]
	#[should_panic(expected = "rendering rate")]
	fn unattended_without_a_rate_stops_startup() {
		unattended_period(Some(&Parameter::new("unattended", "")));
	}

	#[test]
	#[should_panic(expected = "rendering rate")]
	fn unattended_with_a_non_positive_rate_stops_startup() {
		unattended_period(Some(&Parameter::new("unattended", "0")));
	}

	#[test]
	fn next_frame_deadline_keeps_the_rate_and_moves_after_a_stall() {
		let start = std::time::Instant::now();
		let period = std::time::Duration::from_millis(20);
		let overslept = start + std::time::Duration::from_millis(1);
		let stalled = start + std::time::Duration::from_millis(100);

		// Oversleeping a deadline does not push later deadlines back.
		assert_eq!(next_frame_deadline(start, period, overslept), start + period);
		// A stall restarts the schedule instead of owing the missed ticks.
		assert_eq!(next_frame_deadline(start, period, stalled), stalled);
	}

	#[test]
	fn frame_delta_passes_ordinary_intervals_and_caps_waits_and_stalls() {
		let pace = MediaTime::from_micros(16_667);
		let frame = MediaTime::from_micros(20_000);
		let stall = MediaTime::from_seconds(1);

		// A tick that neither waited nor stalled steps by what it measured.
		assert_eq!(frame_delta(frame, pace, false), (frame, frame));
		// A stall advances the clock in full but steps systems by at most four paces.
		assert_eq!(frame_delta(stall, pace, false), (pace * STALL_FRAMES, stall));
		// An idle wait steps by at most one pace, while the clock still advances by the whole wait.
		assert_eq!(frame_delta(stall, pace, true), (pace, stall));
		assert_eq!(frame_delta(frame, pace, true), (pace, frame));
	}

	#[test]
	fn simulation_steps_accumulate_without_drift() {
		let step = MediaTime::from_frames(1, 60).expect("expected test value");
		let delta = MediaTime::from_frames(1, 144).expect("expected test value");
		let mut pending = MediaTime::ZERO;
		let mut simulated = MediaTime::ZERO;

		for _ in 0..1_000 {
			pending += delta;
			let (steps, remainder) = simulation_steps(pending, step, 8);
			pending = remainder;
			simulated += step * i64::from(steps);
		}

		// Every frame's delta ends up either simulated or still pending, however the frame rate divides the step.
		assert_eq!(simulated + pending, delta * 1_000);
	}

	#[test]
	fn simulation_steps_drop_the_debt_past_the_frame_budget() {
		let step = MediaTime::from_frames(1, 60).expect("expected test value");

		let (steps, remainder) = simulation_steps(step * 20 + step / 2, step, 8);

		assert_eq!(steps, 8);
		assert_eq!(remainder, step / 2);
	}

	#[test]
	fn rates_are_tied_within_a_small_reported_difference() {
		let frame = |rate| MediaTime::from_frames(1, rate).expect("expected test value");

		assert!(rates_are_tied(frame(60), frame(60)));
		assert!(rates_are_tied(frame(60), MediaTime::from_seconds_f64(1.0 / 59.94)));
		assert!(!rates_are_tied(frame(60), frame(30)));
		assert!(!rates_are_tied(frame(60), frame(144)));
	}

	#[test]
	fn startup_parameters_queue_only_configuration_under_the_prefix() {
		let configuration = Configuration::new();
		let port = configuration.register(RENDER_PASS_CONFIGURATION);
		let parameters = [
			Parameter::new("render.pass.bloom", "bypassed"),
			Parameter::new("audio.master.gain", "0.5"),
		];

		queue_startup_parameters(&parameters, &configuration, RENDER_PASS_CONFIGURATION);

		let update = port.read().expect("render-pass startup configuration");

		assert_eq!(update.parameter(), "bloom");
		assert_eq!(update.value(), &crate::configuration::ConfigurationValue::from("bypassed"));
		assert!(port.read().is_none());
		assert_eq!(configuration.events().len(), 1);
	}
}

/// Writes the metrics of a finished run to `path`: every retained tick as NDJSON when the extension is `ndjson`,
/// otherwise the summary as JSON.
fn write_metrics(
	path: &std::path::Path,
	metrics: &crate::metrics::Metrics,
	summary: &crate::metrics::Summary,
) -> std::io::Result<()> {
	use std::io::Write as _;

	let mut file = std::io::BufWriter::new(std::fs::File::create(path)?);
	if path.extension().is_some_and(|extension| extension == "ndjson") {
		for row in metrics.ticks_since(None) {
			serde_json::to_writer(&mut file, &row)?;
			file.write_all(b"\n")?;
		}
	} else {
		serde_json::to_writer(&mut file, summary)?;
	}
	file.flush()
}

use std::sync::Arc;

use resource_management::resource::{
	ReDBStorageBackend, ResourceGpuCompressionPolicy, ResourceStorageMode, ResourceStorageSettings,
	resource_manager::ResourceManager,
};
use smallvec::SmallVec;
use tracing::debug_span;
use utils::Box;

use super::{
	Events, Parameter, Time,
	application::{BaseApplication, resolve_application_path},
};
use crate::{
	application::{parameters::Parameters, thread::Thread},
	audio::generator::Generator,
	configuration::Configuration,
	core::{
		EntityHandle,
		channel::{Channel, DefaultChannel},
		factory::{CreateMessage, Creator, Factory},
		listener::{DefaultListener, Listener},
		message::DeleteMessage,
		message_bus::{MessageBus, MessageScope},
	},
	gameplay::transform::TransformationUpdate,
	input::Action,
	inspector::{
		CAMERA_UPDATE_MESSAGE_TYPE, CONE_LIGHT_UPDATE_MESSAGE_TYPE, DELETE_MESSAGE_TYPE, DESTROY_MESSAGE_TYPE,
		DIRECTIONAL_LIGHT_UPDATE_MESSAGE_TYPE, DefaultInspector, POINT_LIGHT_UPDATE_MESSAGE_TYPE,
		TRANSFORMATION_UPDATE_MESSAGE_TYPE, TRIGGER_ACTION_MESSAGE_TYPE, http::HttpInspectorServer,
	},
	metrics::TickClock,
	rendering::{
		ShownTransforms,
		pipelines::{
			simple::SimplePipelineManager,
			visibility::{RENDER_PARAMETERS, VisibilityPipelineManager, VisibilityPipelineSettings},
		},
		render_passes::{
			bloom::{Bloom, BloomPassSettings},
			lut::{LutGrading, LutWorkflow},
			sky::{AtmosphereSky, AtmosphereSkyRenderPassSettings},
			smaa::Smaa,
		},
	},
	time::MediaTime,
	ui::{
		layout::engine::Render,
		render_pass::{UiOverlay, UiRenderPass},
	},
};
impl Creator<Window> for GraphicsApplication {
	fn publish(&self, handle: crate::core::factory::Handle, window: Window) {
		self.window_factory.0.derive(handle, window);
	}
}

use crate::{
	input,
	rendering::{self, Camera, SceneId, renderer::Renderer, window::Window},
};
// Only debug builds bake on demand, so only they report bakes.
#[cfg(debug_assertions)]
mod asset_stream;
pub mod defaults;
mod integrations;

pub use defaults::{
	configure_default_baking, default_image_processor, default_setup, setup_animation_pool, setup_animation_system,
	setup_default_audio, setup_default_input, setup_default_resource_and_asset_management, setup_default_window,
};
pub use integrations::process_default_window_input;
#[cfg(feature = "dmx")]
pub use integrations::setup_default_dmx;
