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
/// [`Self::tick_with`] when application code must run during each tick.
///
/// # Configuration
/// - `kill-after`: Closes the application after this number of ticks. The default is `None`.
/// - `max-frame-rate`: Caps presentation at this many frames per second; frames land on even refreshes. The default is uncapped.
/// - `simulation-rate`: Steps simulation this many times per second, independent of how often frames are presented. [`Self::tick_stepped_with`] paces its steps on it and defaults to `60`. [`Self::tick_with`] simulates once per frame and panics when this rate disagrees with the rate frames are presented at.
/// - `render-on-demand`: Renders only when something changed instead of on every tick. Window changes, UI renders, and screenshot requests ask for frames; call [`Renderer::request_redraw`] after changing the scene. Idle ticks keep running at the refresh rate of the fastest display showing a window, or the `max-frame-rate` pace when that is slower, so events are still handled. Once the frame is shown and no UI component waits for a frame, the loop waits in the window system until an event, a UI timer, or a [`crate::application::LoopWaker`] wakes it; call [`crate::application::LoopWaker::wake`] from threads that change state the loop must see. The default is `false`.
/// - `assets-path`: Selects the debug-build asset directory. Relative overrides use the current working directory. In development, the default is `assets` under `CARGO_MANIFEST_DIR` when available, then beside the executable.
/// - `resources.path`: Selects the resource directory. Relative overrides use the current working directory. In development, the default uses `CARGO_MANIFEST_DIR` when available; otherwise, it is beside the executable.
/// - `render.debug`: Enables validation layers. The default is `true` in debug builds.
/// - `render.debug.dump`: Enables graphics API logging. The default is `false`.
/// - `render.debug.extended`: Enables extended validation. The default is `false`.
/// - `messages.max-topics`: Sets the maximum number of typed routes. The default is `64`.
/// - `messages.cells-per-topic`: Sets the fixed payload-cell budget for each typed route. The default is `512`.
/// - `messages.cell-bytes`: Sets the size of each payload cell in bytes. The default is `256`.
/// - `messages.cell-alignment`: Sets the alignment of each payload cell. The default is `64`.
/// - `messages.listeners-per-topic`: Sets the maximum simultaneous listeners on one typed route. The default is `64`.
/// - `render.pass.<name>`: Selects `enabled` or `bypassed` for the named render pass.
/// - `render.gtao.radius`: Sets the GTAO world-space search radius. The default is `1.0`.
/// - `render.gtao.samples-per-ray`: Sets the GTAO samples along each ray. The default is `6`.
/// - `render.gtao.radial-rays`: Sets the even number of GTAO ray directions. The default is `8`.
/// - `render.cone-shadow-map-pool.capacity`: Sets the startup maximum for reusable cone-light shadow maps per sink. Maps allocate on first use; the default capacity is `4`.
/// - `render.point-shadow-map-pool.capacity`: Sets the startup maximum for reusable point-light cube shadow maps per sink. Maps allocate on first use; the default capacity is `4`.
///
/// See the [sample project guide](/docs/use/sample-project)
/// for a complete `GraphicsApplication` setup.
pub struct GraphicsApplication {
	application: BaseApplication,
	message_bus: MessageBus,
	messages: MessageScope,
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
	/// The frame period the loop sleeps for when no window presents; see [`skipped_frame_pace`].
	skipped_frame_pace: std::time::Duration,
	/// The value of `elapsed` when `last_present_time` was consumed; later presented times land relative to it.
	elapsed_at_last_present: MediaTime,
	/// Whether frames are rendered only when something changed; the `render-on-demand` parameter.
	render_on_demand: bool,
	/// Whether the last tick rendered because something changed, so this tick expects to render again.
	rendering_active: bool,
	/// Wakes this loop when state it cannot observe through window events changes.
	waker: LoopWaker,
	/// Whether the window system's waker reached [`Self::waker`].
	platform_waker_set: bool,
	/// The earliest tick the application asked for; see [`Self::schedule_tick`].
	requested_tick: Option<std::time::Instant>,

	close: bool,

	application_events: (DefaultChannel<Events>, DefaultListener<Events>),
	http_inspector: HttpInspectorServer,
	screenshot_broker: std::sync::Arc<crate::inspector::screenshot::ScreenshotBroker>,
	configuration: Configuration,

	window_factory: (Factory<Window>, DefaultListener<CreateMessage<Window>>),

	generator_factory: Factory<std::boxed::Box<dyn Generator>>,

	world: DefaultWorld,
	cameras_listener: DefaultListener<crate::core::factory::CreateMessage<Camera>>,
	physics_transforms_listener: DefaultListener<TransformationUpdate>,
	renderer_transforms_listener: DefaultListener<TransformationUpdate>,

	/// Every window and gamepad event ends up here before the world's actions pull it.
	input: input::InputCollector,
	/// The sink for actions declared through the world. It captures nothing.
	actions: input::InputSink,
	gamepad_system: Option<input::gamepad::GamepadSystem>,
	gamepad_device_class_handle: Option<input::device::DeviceClassHandle>,
	resource_manager: EntityHandle<ResourceManager>,
	renderer: Renderer,

	threads: SmallVec<[Thread; 64]>,
	/// Persistent lanes shared by initialization and application frame work.
	alley: crate::core::alley::Alley,

	#[cfg(debug_assertions)]
	ttff: MediaTime,
	#[cfg(debug_assertions)]
	min_frame_time: MediaTime,
	#[cfg(debug_assertions)]
	max_frame_time: MediaTime,

	#[cfg(debug_assertions)]
	kill_after: Option<u64>,
}

impl Drop for GraphicsApplication {
	fn drop(&mut self) {
		// Workers may own transferred views into renderer allocations. Join them
		// before field destruction reaches the renderer and its GHI context.
		self.stop_worker_threads();
	}
}

impl Application for GraphicsApplication {
	fn new(name: &str, parameters: &[Parameter]) -> Self {
		let start_time = std::time::Instant::now();

		let application = BaseApplication::new(name, parameters);
		let (message_bus, messages, world_messages) = create_message_bus(&application);
		message_bus.observe().unwrap_or_else(|error| panic!("{error}"));

		let resources_path = resolve_application_directory(application.get_parameter("resources.path"), "resources");

		let configuration = Configuration::new();
		let mut alley = crate::core::alley::Alley::new();
		let world = DefaultWorld::with_messages(world_messages.clone());
		let transforms = world.transforms_channel().clone();
		let mut storage = None;
		let mut services = None;
		// Storage and CPU services own separate lanes while graphics creation stays on the caller.
		let mut renderer = alley
			.join_with_mut(
				(&mut storage, &mut services),
				|lane, (storage, services)| {
					lane.only_one_runs_mut(storage, |storage| {
						*storage = Some(
							ReDBStorageBackend::new_writable_with_settings(
								resources_path.clone(),
								ResourceStorageSettings::new(ResourceStorageMode::Files)
									.image_compression(ResourceGpuCompressionPolicy::MetalIoLz4),
							)
							.unwrap(),
						);
					});
					lane.only_one_runs_mut(services, |services| {
						let action_events = world_messages.channel();
						let mut input = input::InputCollector::new();
						let actions = input::InputSink::new(input.add_sink(), action_events.clone())
							.with_declarations(world_messages.factory::<Action>().listener());
						// Register the application listener before control handlers can publish a
						// close request. Worker listeners join this future-only route during setup.
						let application_events: (DefaultChannel<Events>, DefaultListener<Events>) = {
							let channel = messages.channel();
							let listener = channel.listener();
							(channel, listener)
						};

						ctrlc::set_handler({
							let events = application_events.0.clone();
							move || {
								events.send(Events::Close);
							}
						})
						.unwrap();

						let cameras_listener = world_messages.factory::<Camera>().listener();
						let physics_transforms_listener = transforms.listener();
						let renderer_transforms_listener = transforms.listener();

						// Register reflected posts after the world's initial future-only transform and deletion listeners exist.
						let mut inspector =
							DefaultInspector::new(application_events.0.clone(), configuration.clone(), world_messages.clone());
						inspector
							.register_message(TRANSFORMATION_UPDATE_MESSAGE_TYPE, transforms.clone())
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
						let inspector: EntityHandle<dyn Inspector> = inspector;
						let waker = LoopWaker::default();
						let http_inspector = HttpInspectorServer::new(inspector, waker.clone());

						let window_factory = messages.factory();
						let window_factory_listener = window_factory.listener();

						let generator_factory = messages.factory();

						let window_events = messages.channel();
						*services = Some((
							input,
							actions,
							application_events,
							cameras_listener,
							physics_transforms_listener,
							renderer_transforms_listener,
							http_inspector,
							screenshot_broker,
							waker,
							window_factory,
							window_factory_listener,
							generator_factory,
							window_events,
						));
					});
				},
				|| rendering::renderer::Renderer::new(&application, &configuration),
			)
			.unwrap_or_else(|panic| std::panic::resume_unwind(panic));
		let resource_storage = storage.unwrap();
		let (
			input,
			actions,
			application_events,
			cameras_listener,
			physics_transforms_listener,
			renderer_transforms_listener,
			http_inspector,
			screenshot_broker,
			waker,
			window_factory,
			window_factory_listener,
			generator_factory,
			window_events,
		) = services.unwrap();
		// HID initialization stays deferred until the first presented frame.
		let gamepad_system = None;

		let resource_manager = EntityHandle::from(ResourceManager::new(resource_storage));

		renderer.set_resource_manager(&resource_manager);
		let present_interval = application
			.get_parameter("max-frame-rate")
			.and_then(|parameter| parameter.value.parse::<f64>().ok())
			.filter(|rate| *rate > 0.0)
			.map(|max_frame_rate| std::time::Duration::from_secs_f64(1.0 / max_frame_rate));
		if present_interval.is_some() {
			renderer.set_present_interval(present_interval);
		}
		let simulation_step = application
			.get_parameter("simulation-rate")
			.and_then(|parameter| parameter.value.parse::<f64>().ok())
			.filter(|rate| *rate > 0.0)
			.map(|simulation_rate| MediaTime::from_seconds_f64(1.0 / simulation_rate));
		let render_on_demand = application
			.get_parameter("render-on-demand")
			.is_some_and(|parameter| parameter.as_bool_simple());
		queue_render_pass_startup_parameters(application.parameters(), &configuration);

		#[cfg(debug_assertions)]
		let kill_after = application
			.get_parameter("kill-after")
			.map(|p| p.value.parse::<u64>().unwrap());

		GraphicsApplication {
			application,
			message_bus,
			messages,
			window_events,

			application_events,
			http_inspector,
			screenshot_broker,
			configuration,

			window_factory: (window_factory, window_factory_listener),

			generator_factory,

			world,
			cameras_listener,
			physics_transforms_listener,
			renderer_transforms_listener,

			input,
			actions,
			gamepad_system,
			gamepad_device_class_handle: None,
			resource_manager,
			renderer,

			threads: SmallVec::new(),
			alley,

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
			skipped_frame_pace: skipped_frame_pace(present_interval, None),
			elapsed_at_last_present: MediaTime::ZERO,
			render_on_demand,
			rendering_active: true,
			waker,
			platform_waker_set: false,
			requested_tick: None,

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

	/// Advances the application clock by one frame.
	///
	/// Presented times form the display clock: when the swapchain reports a newer one, `elapsed` lands on the
	/// point that display interval reaches, so deltas follow the display cadence instead of CPU scheduling noise.
	/// Without a newer presented time (startup, skipped frames, or a report that arrives late) the tick advances
	/// by the wall-clock interval; the next presented time discounts that advance instead of charging it twice.
	///
	/// After the loop waited for events, `elapsed` still advances by the time spent waiting, but the delta is capped
	/// at one frame so systems stepping by it do not jump across the idle stretch.
	fn sample_frame_time(&mut self, present_time: Option<std::time::Instant>, waited: bool) -> Time {
		let now = std::time::Instant::now();
		let wall_delta = MediaTime::from_std(now - self.last_tick_instant);
		let (delta, advance) = match (present_time, self.last_present_time) {
			(Some(current), Some(previous)) if current > previous => {
				let target = self.elapsed_at_last_present + MediaTime::from_std(current - previous);
				let delta = if target > self.elapsed {
					target - self.elapsed
				} else {
					MediaTime::ZERO
				};
				(delta, delta)
			}
			_ if waited => (wall_delta.min(MediaTime::from_std(self.skipped_frame_pace)), wall_delta),
			_ => (wall_delta, wall_delta),
		};
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

	/// Routes window input events and reports whether the platform or any window requested close.
	///
	/// Window state changes request a frame. Input events do not: their consumers publish what changed.
	fn process_window_events(&mut self, wait: ghi::window::Wait) -> bool {
		let span = debug_span!("GraphicsApplication::process_window_events");
		let _enter = span.enter();
		let mut close = false;
		let mut redraw = false;
		let mut display_changed = false;
		for event in self.renderer.poll_windows(wait) {
			self.window_events.send(event);
			match event {
				ghi::window::Event::App(ghi::window::AppEvents::Quit) => close = true,
				ghi::window::Event::Window { event, .. } => {
					close |= matches!(event, ghi::window::Events::Close);
					display_changed |= matches!(event, ghi::window::Events::DisplayChanged { .. });
					redraw |= matches!(
						event,
						ghi::window::Events::Resize { .. }
							| ghi::window::Events::FocusChanged(_)
							| ghi::window::Events::Minimize
							| ghi::window::Events::Maximize
							| ghi::window::Events::DisplayChanged { .. }
					);
					if process_default_window_input(&mut self.input, event) {
						self.actions.cancel_seat(input::SeatHandle::stub());
					}
				}
			}
		}
		if redraw {
			self.renderer.request_redraw();
		}
		if display_changed {
			self.skipped_frame_pace = skipped_frame_pace(self.present_interval, self.renderer.refresh_interval());
		}
		close
	}

	/// Chooses how long this tick may wait for window events, and marks the loop as waiting when it may.
	///
	/// Only an idle on-demand loop with a window system connection waits. Pair a waiting result with
	/// [`crate::application::waker::end_wait`] once the wait returned.
	fn event_wait(&mut self) -> ghi::window::Wait {
		#[cfg(debug_assertions)]
		if self.kill_after.is_some() {
			// Smoke runs count ticks, so they keep ticking while idle.
			return ghi::window::Wait::Immediate;
		}
		if !self.render_on_demand || self.rendering_active {
			return ghi::window::Wait::Immediate;
		}
		if !self.platform_waker_set {
			let Some(platform_waker) = self.renderer.app_waker() else {
				// Without a window there is no event queue to wait in.
				return ghi::window::Wait::Immediate;
			};
			self.waker.set_platform_waker(move || platform_waker.wake());
			self.platform_waker_set = true;
		}

		let woken = self.waker.begin_wait();
		let now = std::time::Instant::now();
		// Gamepads report nothing through the window system, so connected ones are polled once per frame.
		let poll_at = self
			.gamepad_system
			.as_ref()
			.filter(|gamepads| gamepads.has_devices())
			.map(|_| now + self.skipped_frame_pace);
		let wait = idle_wait(woken, self.requested_tick.take(), poll_at, now);
		if wait == ghi::window::Wait::Immediate {
			self.waker.end_wait();
		}
		wait
	}

	/// Polls newly connected gamepads and records their trigger values into the collector.
	fn process_gamepad_events(&mut self) {
		let span = debug_span!("GraphicsApplication::process_gamepad_events");
		let _enter = span.enter();
		if self.tick_count > 0 && self.gamepad_system.is_none() {
			self.gamepad_system = input::gamepad::GamepadSystem::new()
				.map_err(|error| log::warn!("{}", error))
				.ok();
		}
		let Some(gamepad_system) = self.gamepad_system.as_mut().filter(|_| self.tick_count > 0) else {
			return;
		};
		let (new_devices, events) = gamepad_system.poll();
		if let Some(device_class) = self.gamepad_device_class_handle {
			for (path, kind, negate_stick_y, device) in new_devices {
				// Keep physical HID identity distinct so player and device routing is preserved.
				let device_handle = self.input.create_device(&device_class);
				gamepad_system.add_device(path, kind, negate_stick_y, device, device_handle);
			}
		} else if !new_devices.is_empty() {
			log::warn!(
				"Detected HID gamepad before the Gamepad device class was registered. The most likely cause is that setup_default_input was not called. See {}.",
				crate::online_docs_url("reference/input")
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
			self.input.record(
				input::SeatHandle::stub(),
				event.device_handle(),
				event.trigger(),
				event.value(),
			);
		}
	}

	/// Adopts newly created windows and cameras before renderer preparation.
	fn prepare_renderer_state(&mut self) {
		let span = debug_span!("GraphicsApplication::prepare_renderer_state");
		let _enter = span.enter();
		while let Some(message) = self.window_factory.1.read() {
			self.renderer.create_window(message.into_data());
		}
		while let Some(message) = self.cameras_listener.read() {
			self.renderer.create_camera(message.handle(), message.into_data());
		}
	}

	/// Renders one frame and completes every screenshot request with its readbacks.
	///
	/// Every capture of every request is read from this one frame. Transports encode the readbacks on their own
	/// threads, so encoding never delays the next frame.
	fn render_frame(&mut self, requests: Vec<crate::inspector::screenshot::ScreenshotRequest>, time: MediaTime) {
		let span = debug_span!("GraphicsApplication::render_frame");
		let _enter = span.enter();
		let captures = requests
			.iter()
			.flat_map(|request| request.captures.iter().map(|selection| (selection.sink, &selection.capture)))
			.collect::<Vec<_>>();
		let (frame, results) = self.renderer.prepare(
			&mut self.renderer_transforms_listener,
			&self.application.frame_allocator,
			&captures,
			self.simulation_alpha,
			time,
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
	}

	/// Drops the transforms physics published since the last time simulation read them.
	///
	/// Physics results return on the shared transform route, so they must be discarded before the route is used to
	/// collect the commands of a new step; otherwise a body's own output arrives back as an authored transform.
	fn drain_physics_transforms(&mut self) {
		while self.physics_transforms_listener.read().is_some() {}
	}

	/// Advances anchors and physics by `time`, then marks the step's end for the renderer.
	fn update_world(&mut self, time: Time) {
		let span = debug_span!("GraphicsApplication::update_world");
		let _enter = span.enter();
		self.world.update(
			time,
			&mut self.physics_transforms_listener,
			&mut self.application.frame_allocator,
		);
		self.renderer.step();
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
		// Frames arrive no faster than the slower of the cap and the display.
		let Some(present_interval) = self.present_interval.max(self.renderer.refresh_interval()) else {
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
			let span = debug_span!("GraphicsApplication::simulation_step");
			let _enter = span.enter();
			self.simulation_elapsed += step;
			let time = Time::new(self.simulation_elapsed, step);
			self.drain_physics_transforms();
			simulate(self, time);
			self.update_world(time);
		}
		// Frames show the world one step behind, moving from the previous step's state to the latest one's as
		// the pending time fills the step.
		self.simulation_alpha = self.simulation_pending.as_seconds_f32() / step.as_seconds_f32();
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
			application.drain_physics_transforms();
			let result = {
				let span = debug_span!("GraphicsApplication::user_tick");
				let _enter = span.enter();
				f(application, time)
			};
			application.update_world(time);
			application.simulation_alpha = 1.0;
			result
		})
	}

	/// Runs one graphics tick whose simulation advances in fixed steps, independent of the frame rate.
	///
	/// `simulate` runs zero or more times before the frame, each time with a [`Time`] whose delta is the fixed
	/// step from `simulation-rate` and whose elapsed value counts the steps run so far. The world, and so physics,
	/// advances with it. `frame` then runs once with the frame's own display-derived [`Time`], for the work that
	/// belongs to what is about to be drawn: the UI, cameras, and animation.
	///
	/// A frame that owes more steps than the loop runs in one tick drops the outstanding time instead of running
	/// it later. Renderers consume the state left by the last step, so a simulation rate below the frame rate
	/// repeats a frame's worth of state until the next step lands.
	pub fn tick_stepped_with<R, S: FnMut(&mut Self, Time), F: FnOnce(&mut Self, Time) -> R>(
		&mut self,
		mut simulate: S,
		frame: F,
	) -> Option<R> {
		self.tick_internal(move |application, time| {
			application.run_simulation_steps(time.delta, &mut simulate);
			let span = debug_span!("GraphicsApplication::user_tick");
			let _enter = span.enter();
			frame(application, time)
		})
	}

	/// Runs the tick every entry point shares, calling `run` where application and world updates belong.
	fn tick_internal<R, F: FnOnce(&mut Self, Time) -> R>(&mut self, run: F) -> Option<R> {
		let span = debug_span!("GraphicsApplication::tick");
		let _enter = span.enter();

		{
			let span = debug_span!("GraphicsApplication::reset_frame_allocator");
			let _enter = span.enter();
			self.application.frame_allocator.reset();
		}
		let wait = self.event_wait();
		let waited = wait != ghi::window::Wait::Immediate;
		let mut close = self.process_window_events(wait);
		if waited {
			self.waker.end_wait();
		}
		close |= matches!(self.application_events.1.read(), Some(Events::Close));

		// Existing windows pace simulation on the display; newly published windows are adopted after the
		// callback so their scene can request resources before native window setup begins.
		// An acquired image must be presented, so on-demand rendering only hoists the acquisition while frames keep
		// coming; the first frame after an idle stretch acquires when it renders.
		let present_time = if self.rendering_active {
			self.renderer.acquire_swapchain_images()
		} else {
			None
		};
		if self.tick_count > 0 && !waited && !self.renderer.presents_this_frame() {
			// The first tick has no previous frame to pace. Later ticks that neither present nor wait for
			// events sleep here so a windowless or unchanged application does not spin a core.
			std::thread::sleep(self.skipped_frame_pace);
		}
		let time = self.sample_frame_time(present_time, waited);
		let dt = time.delta;

		self.process_gamepad_events();

		{
			let span = debug_span!("GraphicsApplication::update_input");
			let _enter = span.enter();
			// The world's sink is the only sink here, so it captures nothing.
			self.actions.pull(&mut self.input, |_| input::Capture::Passed);
		}

		let result = run(self, time);

		self.renderer.update();
		self.prepare_renderer_state();
		let screenshot_requests = self.screenshot_broker.drain();
		// Ask the renderer even when rendering anyway so passes adopt this tick's inputs before deciding next tick.
		let changed = self.renderer.needs_frame() || !screenshot_requests.is_empty();
		self.rendering_active = changed || !self.render_on_demand;
		if self.rendering_active || self.renderer.presents_this_frame() {
			self.render_frame(screenshot_requests, time.elapsed());
		}

		{
			let span = debug_span!("GraphicsApplication::flush_world_deletions");
			let _enter = span.enter();
			self.world.flush_deletions();
		}

		self.tick_count += 1;

		#[cfg(debug_assertions)]
		{
			// A tick without a window submits nothing, so stamp the first tick that reached the screen.
			if self.ttff == MediaTime::ZERO && self.renderer.has_presented() {
				self.ttff = MediaTime::from_std(self.start_time.elapsed());
			}

			if let Some(kill_after) = self.kill_after
				&& self.tick_count >= kill_after
			{
				close = true;
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

		while self.application_events.1.read().is_some() {}
		self.application_events.0.send(Events::Close);
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
	pub fn alley_mut(&mut self) -> &mut crate::core::alley::Alley {
		&mut self.alley
	}

	/// Returns the collector that owns the registered devices and their control values.
	///
	/// Declared [`Action`] values are published through the world's
	/// [`ActionEvent`](input::ActionEvent) channel.
	pub fn input(&self) -> &input::InputCollector {
		&self.input
	}

	/// Returns the renderer used by setup functions and advanced render integrations.
	pub fn renderer(&self) -> &Renderer {
		&self.renderer
	}

	/// Returns mutable renderer access for application-defined pipeline setup.
	///
	/// Use this during startup to create renderer-owned GHI resources. For an
	/// asynchronous resource integration, create the mapped transfer buffer and
	/// [`crate::rendering::resource_loading::UploadStagingArena`] here, keep the
	/// buffer handle in the pipeline manager's upload store, and run the staging
	/// worker and resource servers through application-owned tasks. Then call
	/// [`Renderer::add_pipeline_manager`] before the application starts rendering.
	///
	/// The opposite lifetime is equally important: stop and join those tasks
	/// before this application drops the renderer. The built-in Simple and
	/// Visibility setup functions demonstrate that shutdown ordering.
	pub fn renderer_mut(&mut self) -> &mut Renderer {
		&mut self.renderer
	}

	/// Returns the factory used to request new windows.
	pub fn window_factory(&self) -> &Factory<Window> {
		&self.window_factory.0
	}

	/// Returns the application-owned namespace used by headed-runtime channels and factories.
	///
	/// Next, call [`MessageScope::channel`] or [`MessageScope::factory`] to add an
	/// application-defined message route without declaring its type at startup.
	/// Subscribe to [`ghi::window::Event`] before creating windows to handle raw
	/// input in your application callback. Events are published in arrival order,
	/// and each window event carries the [`ghi::window::WindowId`] it targets.
	pub fn messages(&self) -> &MessageScope {
		&self.messages
	}

	/// Returns the shared fixed-storage message bus for diagnostics and new scopes.
	///
	/// Next, call [`MessageBus::topics`] to inspect registered routes or
	/// [`MessageBus::new_scope`] to isolate routes owned by another subsystem.
	pub fn message_bus(&self) -> &MessageBus {
		&self.message_bus
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
	pub fn generator_factory(&self) -> &Factory<std::boxed::Box<dyn Generator>> {
		&self.generator_factory
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

/// Allocates the application bus and its initial isolated namespaces.
fn create_message_bus(application: &BaseApplication) -> (MessageBus, MessageScope, MessageScope) {
	let message_bus = MessageBus::new(message_bus_config(application)).unwrap_or_else(|error| panic!("{error}"));
	let application_messages = message_bus.new_scope("application");
	let world_messages = message_bus.new_scope("world");
	(message_bus, application_messages, world_messages)
}

/// Resolves the fixed message-storage limits from application startup parameters.
fn message_bus_config(application: &BaseApplication) -> MessageBusConfig {
	let defaults = MessageBusConfig::default();

	MessageBusConfig::new(
		message_bus_limit(application, "messages.max-topics", defaults.max_topics),
		message_bus_limit(application, "messages.cells-per-topic", defaults.cells_per_topic),
		message_bus_limit(application, "messages.cell-bytes", defaults.cell_bytes),
	)
	.with_cell_alignment(message_bus_limit(
		application,
		"messages.cell-alignment",
		defaults.cell_alignment,
	))
	.with_max_listeners_per_topic(message_bus_limit(
		application,
		"messages.listeners-per-topic",
		defaults.max_listeners_per_topic,
	))
}

/// Parses one unsigned message-storage limit while preserving the configured default.
fn message_bus_limit(application: &BaseApplication, name: &str, default: usize) -> usize {
	application
		.get_parameter(name)
		.map(|parameter| {
			parameter.value().parse::<usize>().unwrap_or_else(|error| {
				panic!(
					"Message bus parameter '{name}' is invalid. The most likely cause is that '{}' is not an unsigned integer: {error}",
					parameter.value()
				)
			})
		})
		.unwrap_or(default)
}

/// Resolves an explicit path as supplied while anchoring the development default to its Cargo application.
fn resolve_application_directory(parameter: Option<&Parameter>, default_directory: &str) -> std::path::PathBuf {
	parameter.map(|parameter| parameter.value().into()).unwrap_or_else(|| {
		// Cargo provides the application manifest directory while running development binaries.
		#[cfg(debug_assertions)]
		if let Some(manifest_directory) = std::env::var_os("CARGO_MANIFEST_DIR") {
			return default_application_directory(Some(std::path::Path::new(&manifest_directory)), None, default_directory);
		}

		let executable = std::env::current_exe().unwrap_or_else(|error| {
			panic!(
				"Application directory could not be resolved. The most likely cause is that the current executable path is unavailable: {error}"
			)
		});
		default_application_directory(None, Some(&executable), default_directory)
	})
}

/// Builds a default directory from a Cargo manifest when available, then from the executable.
fn default_application_directory(
	manifest_directory: Option<&std::path::Path>,
	executable: Option<&std::path::Path>,
	directory: &str,
) -> std::path::PathBuf {
	manifest_directory
		.or_else(|| executable.and_then(std::path::Path::parent))
		.unwrap_or_else(|| {
			panic!(
				"Application directory could not be resolved. The most likely cause is that neither a Cargo manifest directory nor an executable parent is available."
			)
		})
		.join(directory)
}
// Bound ready work while the temporary Compio runtime still shares the application thread.
const ASYNC_TASK_POLL_BUDGET_PER_TICK: usize = 8;

/// The frame period the loop sleeps for when no window presents and neither `max-frame-rate` nor a display
/// refresh rate is known.
const DEFAULT_SKIPPED_FRAME_PACE: std::time::Duration = std::time::Duration::from_micros(16_667);

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
use pipeline::drain_render_pass_messages;
pub use pipeline::{
	setup_aces_color_grading_render_pass, setup_aces_tonemap_render_pass, setup_agx_tonemap_render_pass,
	setup_atmosphere_sky_render_pass, setup_bloom_render_pass, setup_debug_mesh_render_pass,
	setup_dwg_color_grading_render_pass, setup_lut_render_pass, setup_pbr_visibility_shading_render_pipeline,
	setup_simple_render_pipeline, setup_smaa_render_pass, setup_srgb_display_render_pass, setup_ui_render_pass,
};

use crate::application::LoopWaker;

#[cfg(test)]
mod tests {
	use super::*;

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
	fn skipped_frame_pace_follows_the_slower_of_the_cap_and_the_display() {
		let ms = std::time::Duration::from_millis;
		assert_eq!(skipped_frame_pace(None, None), DEFAULT_SKIPPED_FRAME_PACE);
		assert_eq!(skipped_frame_pace(None, Some(ms(8))), ms(8));
		assert_eq!(skipped_frame_pace(Some(ms(33)), None), ms(33));
		assert_eq!(skipped_frame_pace(Some(ms(33)), Some(ms(8))), ms(33));
		assert_eq!(skipped_frame_pace(Some(ms(4)), Some(ms(16))), ms(16));
	}

	#[test]
	fn simulation_steps_consume_whole_steps_and_carry_the_remainder() {
		let step = MediaTime::from_frames(1, 60).expect("expected test value");

		assert_eq!(simulation_steps(step / 2, step, 8), (0, step / 2));
		assert_eq!(simulation_steps(step, step, 8), (1, MediaTime::ZERO));
		assert_eq!(simulation_steps(step * 2 + step / 4, step, 8), (2, step / 4));
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
	fn application_directories_prefer_the_cargo_manifest() {
		let manifest = std::path::Path::new("app");
		let executable = std::path::Path::new("target/debug/game");

		assert_eq!(
			default_application_directory(Some(manifest), Some(executable), "resources"),
			std::path::Path::new("app/resources")
		);
	}

	#[test]
	fn application_directories_fall_back_beside_the_executable() {
		let executable = std::path::Path::new("app/target/debug/game");

		assert_eq!(
			default_application_directory(None, Some(executable), "resources"),
			std::path::Path::new("app/target/debug/resources")
		);
	}

	#[test]
	fn explicit_application_directories_remain_working_directory_relative() {
		let parameter = Parameter::new("resources.path", "custom/resources");

		assert_eq!(
			resolve_application_directory(Some(&parameter), "resources"),
			std::path::Path::new("custom/resources")
		);
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
}

use core::time;
use std::thread;

use ghi::{Context as _, ContextCreate as _, Frame as _, Queue as _};
use resource_management::{
	resource::{
		ReDBStorageBackend, ResourceGpuCompressionPolicy, ResourceStorageMode, ResourceStorageSettings,
		resource_manager::ResourceManager,
	},
	resources::material::Material,
};
use smallvec::SmallVec;
use tracing::{Level, debug_span, instrument, span};
use utils::{Box, sync::RwLock};

use super::{
	Events, Parameter, Time,
	application::{Application, BaseApplication},
};
use crate::{
	application::{parameters::Parameters, thread::Thread},
	audio::generator::Generator,
	configuration::Configuration,
	core::{
		Entity, EntityHandle,
		channel::{Channel, DefaultChannel},
		factory::{CreateMessage, Creator, Factory},
		listener::{DefaultListener, Listener},
		message::DeleteMessage,
		message_bus::{MessageBus, MessageBusConfig, MessageScope},
		task,
	},
	gameplay::{transform::TransformationUpdate, world::DefaultWorld},
	ghi::command_buffer::CommandBufferRecording as _,
	input::Action,
	inspector::{
		DELETE_MESSAGE_TYPE, DESTROY_MESSAGE_TYPE, DefaultInspector, Inspector, TRANSFORMATION_UPDATE_MESSAGE_TYPE,
		TRIGGER_ACTION_MESSAGE_TYPE, http::HttpInspectorServer,
	},
	physics::dynabit::{self, body::PhysicsBody},
	rendering::{
		Environment, RenderableMesh, UpdatePose,
		pipeline_manager::PipelineManager,
		pipelines::{
			simple::{SimplePipelineManager, SimpleRenderPass},
			visibility::{
				CONE_SHADOW_MAP_POOL_CAPACITY_PARAMETER, DIRECTIONAL_SHADOW_DISTANCE_PARAMETER,
				DIRECTIONAL_SHADOW_SPLIT_BLEND_PARAMETER, POINT_SHADOW_MAP_POOL_CAPACITY_PARAMETER, VisibilityPipelineManager,
				VisibilityPipelineSettings,
			},
		},
		render_pass::RenderPass,
		render_passes::{
			aces::AcesToneMapPass,
			agx::AgxToneMapPass,
			bloom::{BloomPass, BloomPassSettings},
			color_grading::{ColorGradingPass, ColorGradingWorkflow},
			sky::AtmosphereSkyRenderPass,
			smaa::SmaaPass,
		},
		renderable, renderer,
	},
	time::MediaTime,
	ui::{
		layout::engine::Render,
		render_pass::{AdoptedRender, UiRenderPass},
	},
};
impl Creator<Window> for GraphicsApplication {
	fn publish(&self, handle: Option<crate::core::factory::Handle>, window: Window) -> crate::core::factory::Handle {
		if let Some(handle) = handle {
			self.window_factory.0.derive(handle, window);
			handle
		} else {
			self.window_factory.0.create(window)
		}
	}
}

use crate::{
	gameplay::anchor::AnchorSystem,
	input, physics,
	rendering::{self, Camera, renderer::Renderer, window::Window},
};
pub mod defaults;
mod integrations;

pub use defaults::{
	default_setup, setup_animation_pool, setup_default_audio, setup_default_input, setup_default_pipeline_compilation,
	setup_default_resource_and_asset_management, setup_default_window,
};
pub use integrations::process_default_window_input;
#[cfg(feature = "dmx")]
pub use integrations::setup_default_dmx;
