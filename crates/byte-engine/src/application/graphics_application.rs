use crate::{
	application::{parameters::Parameters, thread::Thread},
	audio::generator::Generator,
	core::{
		channel::{Channel, DefaultChannel},
		factory::{CreateMessage, Factory},
		listener::{DefaultListener, Listener},
		task, Entity, EntityHandle,
	},
	gameplay::{transform::TransformationUpdate, world::DefaultWorld},
	input::{
		input_trigger,
		utils::{register_gamepad_device_class, register_keyboard_device_class, register_mouse_device_class},
		Action,
	},
	inspector::{http::HttpInspectorServer, Inspector},
	physics::dynabit::{self, body::PhysicsBody},
	rendering::{
		lights::{Light, Lights},
		pipelines::{
			simple::{SimpleRenderPass, SimpleSceneManager},
			visibility::VisibilityWorldRenderDomain,
		},
		render_pass::RenderPass,
		render_passes::{aces::AcesToneMapPass, agx::AgxToneMapPass, sky::SkyRenderPass},
		renderable, renderer,
		scene_manager::SceneManager,
		texture_manager::TextureManager,
		RenderableMesh,
	},
	ui::{layout::engine::Render, render_pass::UiRenderPass},
};
use core::time;
use std::{
	net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, UdpSocket},
	sync::Arc,
	thread,
	time::Duration,
};

use artnet_protocol::{ArtCommand, ArtTalkToMe, Output, Poll, PollReply, PortAddress};
use math::Vector2;
use resource_management::{
	asset::{
		asset_manager::AssetManager,
		bema_asset_handler::{BEMAAssetHandler, ProgramGenerator},
		gltf_asset_handler::GLTFAssetHandler,
		png_asset_handler::PNGAssetHandler,
		wav_asset_handler::WAVAssetHandler,
		FileStorageBackend,
	},
	resource::{resource_manager::ResourceManager, RedbStorageBackend},
	resources::material::Material,
};
use smallvec::SmallVec;
use tracing::{debug_span, instrument, span, Level};
use utils::{sync::RwLock, Box, Extent, RGBA};

use crate::{
	audio::audio_system::{AudioSystem, DefaultAudioSystem},
	gameplay::anchor::AnchorSystem,
	input, physics,
	rendering::{
		self, common_shader_generator::CommonShaderGenerator,
		pipelines::visibility::shader_generator::VisibilityShaderGenerator, renderer::Renderer, window::Window,
	},
};

use super::{
	application::{Application, BaseApplication},
	Events, Parameter, Receiver, Sender, Time,
};

/// A graphics application is the base for all applications that use the graphics functionality of the engine.
/// It uses the orchestrated application as a base and adds rendering and windowing functionality.
///
/// # Parameters
/// - `kill-after`: The number of ticks after which the application should be killed. Defaults to None.
/// ## Resources
/// - `resources.path`: The path to the resources directory. Defaults to "./resources".
/// ## Render
/// ### Render > Debug
/// - `render.debug`: Enables validation layers for debugging. Defaults to true on debug builds.
/// - `render.debug.dump`: Enables API dump for debugging. Defaults to false.
/// - `render.debug.extended`: Enables extended validation for debugging. Defaults to false.
pub struct GraphicsApplication {
	application: BaseApplication,

	tick_count: u64,
	start_time: std::time::Instant,
	last_tick_time: std::time::Instant,

	close: bool,

	application_events: (Sender<Events>, Receiver<Events>),

	window_factory: (Factory<Window>, DefaultListener<CreateMessage<Window>>),
	action_factory: Factory<Action>,

	renderable_factory: Factory<EntityHandle<dyn RenderableMesh>>,
	light_factory: Factory<Lights>,
	generator_factory: Factory<Arc<dyn Generator>>,

	world: DefaultWorld,

	input_system: input::InputManager,
	resource_manager: EntityHandle<ResourceManager>,
	renderer: Renderer,

	threads: SmallVec<[Thread; 64]>,

	#[cfg(debug_assertions)]
	ttff: std::time::Duration,
	#[cfg(debug_assertions)]
	min_frame_time: std::time::Duration,
	#[cfg(debug_assertions)]
	max_frame_time: std::time::Duration,

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

		let resource_manager = ResourceManager::new(RedbStorageBackend::new(resources_path));

		let action_factory = Factory::new();

		let input_system = {
			let action_listener = action_factory.listener();
			let event_channel = DefaultChannel::new();

			input::InputManager::new(action_listener, event_channel)
		};

		let renderable_factory = Factory::new();

		let renderer = rendering::renderer::Renderer::new(&application);

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

		// let inspector = Inspector::new(tx.clone());
		// HttpInspectorServer::new(inspector);

		let rx = tx.spawn_rx();
		let application_events = (tx, rx);

		let window_factory = Factory::new();
		let window_factory_listener = window_factory.listener();

		let world = DefaultWorld::new();

		GraphicsApplication {
			application,

			application_events,

			window_factory: (window_factory, window_factory_listener),
			action_factory,
			renderable_factory,
			light_factory: Factory::new(),

			generator_factory: Factory::new(),

			world,

			input_system,
			renderer,
			resource_manager: EntityHandle::from(resource_manager),

			threads: SmallVec::new(),

			close: false,

			tick_count: 0,
			start_time,
			last_tick_time: std::time::Instant::now(),

			#[cfg(debug_assertions)]
			ttff: std::time::Duration::ZERO,
			#[cfg(debug_assertions)]
			min_frame_time: std::time::Duration::MAX,
			#[cfg(debug_assertions)]
			max_frame_time: std::time::Duration::ZERO,

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
	pub fn tick_with<R, F: FnOnce(&mut Self, Time) -> R>(&mut self, f: F) -> Option<R> {
		let span = debug_span!("GraphicsApplication::tick");
		let _enter = span.enter();

		let now = std::time::Instant::now();
		let dt = now - self.last_tick_time;
		self.last_tick_time = now;

		let elapsed = self.start_time.elapsed();
		let tick_count = self.tick_count;

		let mut close = false;

		{
			let renderer = &mut self.renderer;
			let input_system = &mut self.input_system;

			for window_events in renderer.update_windows() {
				for event in window_events {
					if let ghi::Events::Close { .. } = event {
						close = true;
					}

					if let Some((device_handle, input_source_action, value)) = process_default_window_input(input_system, event)
					{
						input_system.record_trigger_value_for_device(device_handle, input_source_action, value);
					}
				}
			}
		}

		if let Ok(e) = self.application_events.1.try_recv() {
			match e {
				Events::Close => {
					close = true;
				}
			}
		}

		if close {
			let _ = self.application_events.0.send(Events::Close);
			self.threads.drain(..).for_each(|t| {
				let _ = t.join();
			});
			self.close();
			return None;
		}

		let time = Time { elapsed, delta: dt };

		self.input_system.update();

		let mut cameras_listener = self.world.camera_factory().listener();
		let mut renderer_transforms_listener = self.world.transforms_channel().listener();
		let mut physics_transforms_listener = self.world.transforms_channel().listener();
		let mut light_listener = self.light_factory.listener();

		let result = f(self, time);

		self.world.update(time, &mut physics_transforms_listener);

		{
			let window_listener = &mut self.window_factory.1;

			while let Some(message) = window_listener.read() {
				self.renderer.create_window(message.into_data());
			}

			while let Some(message) = cameras_listener.read() {
				self.renderer.create_camera(message.handle().clone(), message.into_data());
			}

			self.renderer.prepare(&mut renderer_transforms_listener);
		}

		self.tick_count += 1;

		#[cfg(debug_assertions)]
		{
			if self.tick_count == 1 {
				self.ttff = self.start_time.elapsed();
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

		(!close).then(|| result)
	}

	/// Flags the application for closing.
	pub fn close(&mut self) {
		self.close = true;

		#[cfg(debug_assertions)]
		log::debug!(
			"Run stats:\n\tElapsed time: {:#?}\n\tAverage frame time: {:#?}\n\tMin frame time: {:#?}\n\tMax frame time: {:#?}\n\tTime to first frame: {:#?}",
			self.start_time.elapsed(),
			self.start_time.elapsed().div_f32(self.tick_count as f32),
			self.min_frame_time,
			self.max_frame_time,
			self.ttff
		);
	}

	pub fn input_system(&self) -> &input::InputManager {
		&self.input_system
	}

	pub fn renderer(&self) -> &Renderer {
		&self.renderer
	}

	pub fn window_factory(&self) -> &Factory<Window> {
		&self.window_factory.0
	}

	pub fn window_factory_mut(&mut self) -> &mut Factory<Window> {
		&mut self.window_factory.0
	}

	pub fn action_factory(&self) -> &Factory<Action> {
		&self.action_factory
	}

	pub fn action_factory_mut(&mut self) -> &mut Factory<Action> {
		&mut self.action_factory
	}

	pub fn world(&self) -> &DefaultWorld {
		&self.world
	}

	pub fn world_mut(&mut self) -> &mut DefaultWorld {
		&mut self.world
	}

	pub fn renderable_factory(&self) -> &Factory<EntityHandle<dyn RenderableMesh>> {
		&self.renderable_factory
	}

	pub fn renderable_factory_mut(&mut self) -> &mut Factory<EntityHandle<dyn RenderableMesh>> {
		&mut self.renderable_factory
	}

	pub fn light_factory_mut(&mut self) -> &mut Factory<Lights> {
		&mut self.light_factory
	}

	pub fn generator_factory(&self) -> &Factory<Arc<dyn Generator>> {
		&self.generator_factory
	}

	pub fn generator_factory_mut(&mut self) -> &mut Factory<Arc<dyn Generator>> {
		&mut self.generator_factory
	}

	pub fn do_loop(&mut self) {
		while !self.close {
			self.tick();
		}
	}

	pub fn do_loop_with<F: FnOnce(&mut Self, Time) + Copy>(&mut self, f: F) {
		while !self.close {
			self.tick_with(f);
		}
	}

	pub fn resource_manager(&self) -> &ResourceManager {
		&self.resource_manager
	}
}

impl Parameters for GraphicsApplication {
	fn get_parameter(&self, name: &str) -> Option<&Parameter> {
		self.application.get_parameter(name)
	}
}

/// Performs a default setup of the application.
/// This includes setting up mouse, keyboard and gamepad input devices,
/// as well as setting up the resource manager with default asset handlers.
/// It also sets up the audio system with default audio devices.
/// It also sets up the renderer with a default render pipeline.
/// The default render pipeline includes a visibility shader generator and a PBR visibility shading render pipeline.
/// The default render pipeline also includes a tone mapping pass.
/// A window is created with the application name.
pub fn default_setup(application: &mut GraphicsApplication) {
	{
		let generator = {
			let visibility_shader_generation =
				VisibilityShaderGenerator::new(false, false, false, false, false, false, true, false);
			visibility_shader_generation
		};

		setup_default_resource_and_asset_management(application, generator);
	}

	setup_default_input(application);

	setup_default_audio(application);

	setup_pbr_visibility_shading_render_pipeline(application);

	setup_default_window(application);
}

/// Creates a new window under the root space with the application name and an extent of 1920x1080.
pub fn setup_default_window(application: &mut GraphicsApplication) {
	application
		.window_factory
		.0
		.create(Window::new(application.get_name(), Extent::rectangle(1920, 1080)));
}

/// Sets up the default resource and asset management for the application.
/// This includes setting up the resource manager with default asset handlers.
/// The default asset handlers include:
/// - MaterialAssetHandler
/// - MeshAssetHandler
/// - ImageAssetHandler
/// - AudioAssetHandler
///
/// The default material asset handler is set up with a shader generator.
/// The shader generator is passed as a parameter to this function.
/// The resources folder path is taken from the `resources-path` parameter and defaults to `resources`.
/// The assets folder path is taken from the `assets-path` parameter and defaults to `assets`.
pub fn setup_default_resource_and_asset_management(
	application: &mut GraphicsApplication,
	generator: impl ProgramGenerator + 'static,
) {
	let assets_path: std::path::PathBuf = application
		.get_parameter("assets-path")
		.map(|p| p.value.clone())
		.unwrap_or_else(|| "assets".into())
		.into();

	let resource_manager = &mut application.resource_manager;

	let storage_backend = FileStorageBackend::new(assets_path.clone());

	let mut asset_manager = AssetManager::new(storage_backend);

	let mut material_asset_handler = BEMAAssetHandler::new();
	material_asset_handler.set_shader_generator(generator);
	asset_manager.add_asset_handler(material_asset_handler);

	asset_manager.add_asset_handler(GLTFAssetHandler::new());
	asset_manager.add_asset_handler(PNGAssetHandler::new());
	asset_manager.add_asset_handler(WAVAssetHandler::new());

	resource_manager
		.try_map_mut(|resource_manager| {
			resource_manager.set_asset_manager(asset_manager);
		})
		.expect("Failed to set up resource manager. Application cannot run without a resource manager.");
}

/// Sets up a default input system for the application.
/// This includes setting up mouse, keyboard and gamepad input devices.
pub fn setup_default_input(application: &mut GraphicsApplication) {
	let input_system = &mut application.input_system;

	let mouse_device_class_handle = register_mouse_device_class(input_system);
	let keyboard_device_class_handle = register_keyboard_device_class(input_system);
	let gamepad_device_class_handle = register_gamepad_device_class(input_system);

	input_system.create_device(&mouse_device_class_handle);
	input_system.create_device(&keyboard_device_class_handle);
	input_system.create_device(&gamepad_device_class_handle);
}

pub fn setup_simple_render_pipeline(application: &mut GraphicsApplication) {
	let listener = application.renderable_factory().listener();
	let transforms_listener = application.world().transforms_channel().listener();

	let renderer = &mut application.renderer;

	struct CustomSceneManager {
		scene_manager: SimpleSceneManager,
		mesh_receiver: DefaultListener<CreateMessage<EntityHandle<dyn RenderableMesh>>>,
		transforms_listener: DefaultListener<TransformationUpdate>,
	}

	impl SceneManager for CustomSceneManager {
		fn prepare(
			&mut self,
			frame: &mut ghi::implementation::Frame,
			viewports: &[rendering::Viewport],
		) -> Option<Vec<Box<dyn rendering::render_pass::RenderPassFunction>>> {
			while let Some(message) = self.mesh_receiver.read() {
				let handle = message.handle().clone();

				self.scene_manager.create_mesh(frame, handle, message.into_data());
			}

			while let Some(message) = self.transforms_listener.read() {
				self.scene_manager
					.update_transform(frame, *message.handle(), message.transform().get_matrix());
			}

			self.scene_manager.prepare(frame, viewports)
		}

		fn create_view(&mut self, id: usize, render_pass_builder: &mut rendering::render_pass::RenderPassBuilder) {
			self.scene_manager.create_view(id, render_pass_builder);
		}
	}

	let sm = {
		let texture_manager = Arc::new(RwLock::new(TextureManager::new()));
		CustomSceneManager {
			scene_manager: SimpleSceneManager::new(renderer.device_mut()),
			mesh_receiver: listener,
			transforms_listener,
		}
	};

	renderer.add_scene_manager(sm);
}

pub fn setup_pbr_visibility_shading_render_pipeline(application: &mut GraphicsApplication) {
	let renderer = &mut application.renderer;

	struct CustomSceneManager {
		light_receiver: DefaultListener<CreateMessage<Lights>>,
		mesh_receiver: DefaultListener<CreateMessage<EntityHandle<dyn RenderableMesh>>>,
		visibility_world_render_domain: VisibilityWorldRenderDomain,
	}

	impl SceneManager for CustomSceneManager {
		fn prepare(
			&mut self,
			frame: &mut ghi::implementation::Frame,
			viewports: &[rendering::Viewport],
		) -> Option<Vec<Box<dyn rendering::render_pass::RenderPassFunction>>> {
			while let Some(message) = self.light_receiver.read() {
				self.visibility_world_render_domain.create_light(message.into_data());
			}

			while let Some(message) = self.mesh_receiver.read() {
				self.visibility_world_render_domain
					.create_renderable_mesh(frame, message.into_data());
			}

			self.visibility_world_render_domain.prepare(frame, viewports)
		}

		fn create_view(&mut self, id: usize, render_pass_builder: &mut rendering::render_pass::RenderPassBuilder) {
			self.visibility_world_render_domain.create_view(id, render_pass_builder);
		}
	}

	let sm = {
		let texture_manager = TextureManager::new();
		CustomSceneManager {
			visibility_world_render_domain: VisibilityWorldRenderDomain::new(
				renderer.device_mut(),
				texture_manager,
				application.resource_manager.clone(),
			),
			light_receiver: application.light_factory.listener(),
			mesh_receiver: application.renderable_factory.listener(),
		}
	};

	renderer.add_scene_manager(sm);
}

pub fn setup_ui_render_pass(application: &mut GraphicsApplication, ui: DefaultListener<CreateMessage<Render>>) {
	let renderer = &mut application.renderer;
	let ui_channel = ui.clone_channel();

	renderer.add_post_scene_render_pass_for_all_views(move |render_pass_builder| {
		struct CustomRenderPass {
			listener: DefaultListener<CreateMessage<Render>>,
			render_pass: UiRenderPass,
		}

		impl rendering::RenderPass for CustomRenderPass {
			fn prepare(
				&mut self,
				frame: &mut ghi::implementation::Frame,
				viewport: &rendering::Viewport,
			) -> Option<rendering::render_pass::RenderPassReturn> {
				while let Some(render) = self.listener.read() {
					self.render_pass.update(render.into_data());
				}

				self.render_pass.prepare(frame, viewport)
			}
		}

		println!("Adding UI render pass");

		Box::new(CustomRenderPass {
			// Spawn only the listeners that are actively consumed by render passes.
			listener: ui_channel.listener(),
			render_pass: UiRenderPass::new(render_pass_builder),
		})
	});
}

pub fn setup_agx_tonemap_render_pass(application: &mut GraphicsApplication) {
	let renderable_mesh_factory = application.renderable_factory_mut();
	let listener = renderable_mesh_factory.listener();

	let renderer = &mut application.renderer;

	renderer.add_post_scene_render_pass_for_all_views(|render_pass_builder| Box::new(AgxToneMapPass::new(render_pass_builder)));
}

pub fn setup_sky_render_pass(application: &mut GraphicsApplication) {
	let renderer = &mut application.renderer;

	renderer.add_post_scene_render_pass_for_all_views(|render_pass_builder| Box::new(SkyRenderPass::new(render_pass_builder)));
}

pub fn setup_default_audio(application: &mut GraphicsApplication) {
	application
		.threads
		.push(Thread::new(application.application_events.0.spawn_rx(), {
			let resource_manager = &mut application.resource_manager;
			let mut generators_listener = application.generator_factory.listener();

			move |mut rx| {
				let Ok(mut audio_system) = DefaultAudioSystem::try_new()
					.map_err(|e| format!("Failed to spawn audio system. No audio will play. Reason: {}", e))
					.warn()
				else {
					return;
				};

				let span = debug_span!("Render audio");
				let _ = span.enter();

				'a: loop {
					if let Ok(event) = rx.try_recv() {
						match event {
							Events::Close => {
								break 'a;
							}
						}
					}

					while let Some(message) = generators_listener.read() {
						audio_system.create_generator(message.into_data());
					}

					if !audio_system.render_available() {
						break 'a; // Audio rendering can no longer be performed.
					}
				}

				log::debug!("Exiting audio thread");
			}
		}));
}

pub fn setup_default_dmx(application: &mut GraphicsApplication, mut rx: DefaultListener<RGBA>) {
	let bind_address = parse_artnet_ipv4_parameter(application.get_parameter("artnet.bind-address"), Ipv4Addr::UNSPECIFIED);
	let poll_target = parse_artnet_ipv4_parameter(application.get_parameter("artnet.poll-target"), Ipv4Addr::BROADCAST);

	application
		.threads
		.push(Thread::new(application.application_events.0.spawn_rx(), move |mut arx| {
			const ARTNET_PORT: u16 = 6454;

			let socket = UdpSocket::bind((bind_address, ARTNET_PORT)).unwrap();
			let target = (poll_target, ARTNET_PORT);
			socket.set_broadcast(true).unwrap();

			log::debug!(
				"Created Art-Net socket on {}:{} and polling {}:{}.",
				bind_address,
				ARTNET_PORT,
				poll_target,
				ARTNET_PORT
			);

			loop {
				while let Ok(m) = arx.try_recv() {
					match m {
						Events::Close => return,
					}
				}

				let buff = ArtCommand::Poll(Poll {
					talk_to_me: ArtTalkToMe::EMIT_CHANGES,
					diagnostics_priority: 0,
					..Poll::default()
				})
				.write_to_buffer()
				.unwrap();
				socket.send_to(&buff, &target).unwrap();
				log::debug!("Sent poll packet!");

				socket.set_read_timeout(Some(Duration::from_millis(500))).unwrap();

				// let mut buffer = [0u8; 1024];
				// let (length, addr) = match socket.recv_from(&mut buffer) {
				// 	Ok(message) => message,
				// 	Err(error) => {
				// 		log::warn!(
				// 			"Failed to receive an Art-Net packet. The most likely cause is that the network socket is no longer readable: {error}"
				// 		);
				// 		continue;
				// 	}
				// };
				// let command = match ArtCommand::from_buffer(&buffer[..length]) {
				// 	Ok(command) => command,
				// 	Err(error) => {
				// 		log::warn!(
				// 			"Failed to decode an Art-Net packet. The most likely cause is that the network device sent a malformed or unsupported payload: {error}"
				// 		);
				// 		continue;
				// 	}
				// };

				// log::debug!("Received {:?}", command);

				while let Some(c) = rx.read() {
					let f_to_u8 = |v: f32| {
						(v * 255f32).clamp(0f32, 255f32) as u8
					};

					let data: [u8; 7] = [f_to_u8(c.r), f_to_u8(c.g), f_to_u8(c.b), 0x00, 0x00, 0x00, 0x00];

					let command = ArtCommand::Output(Output {
						data: data.to_vec().into(),
						port_address: 0.into(), // If not a reply to a poll use 0 for basic devices
						..Output::default()
					});
					let bytes = match command.write_to_buffer() {
						Ok(bytes) => bytes,
						Err(error) => {
							log::warn!(
								"Failed to serialize an Art-Net output packet. The most likely cause is that the DMX payload or universe is invalid: {error}"
							);
							continue;
						}
					};

					if let Err(error) = socket.send_to(&bytes, target) {
						log::warn!(
							"Failed to send an Art-Net output packet. The most likely cause is that the node address is unreachable from this host: {error}"
						);
					}
				}


				// match command {
				// 	ArtCommand::Poll(_) => {
				// 		// Ignore our own discovery broadcast and any other controller polls.
				// 	}
				// 	ArtCommand::PollReply(reply) => {
				// 		let target = resolve_artnet_node_address(addr, &reply);
				// 		let port_address = resolve_artnet_port_address(&reply);
				// 		let command = ArtCommand::Output(Output {
				// 			// port_address,
				// 			data: DMX_DATA.to_vec().into(),
				// 			..Output::default()
				// 		});
				// 		let bytes = match command.write_to_buffer() {
				// 			Ok(bytes) => bytes,
				// 			Err(error) => {
				// 				log::warn!(
				// 					"Failed to serialize an Art-Net output packet. The most likely cause is that the DMX payload or universe is invalid: {error}"
				// 				);
				// 				continue;
				// 			}
				// 		};

				// 		if let Err(error) = socket.send_to(&bytes, target) {
				// 			log::warn!(
				// 				"Failed to send an Art-Net output packet. The most likely cause is that the node address is unreachable from this host: {error}"
				// 			);
				// 		}
				// 	}
				// 	_ => {}
				// }
			}
		}));
}

// Parses an optional Art-Net IPv4 parameter and falls back to the provided default value.
fn parse_artnet_ipv4_parameter(parameter: Option<&Parameter>, default: Ipv4Addr) -> Ipv4Addr {
	let Some(parameter) = parameter else {
		return default;
	};

	match parameter.value().parse::<Ipv4Addr>() {
		Ok(address) => address,
		Err(error) => {
			log::warn!(
				"Invalid Art-Net IPv4 address parameter `{}`. The most likely cause is that the configured value is not a valid IPv4 address: {}",
				parameter.name(),
				error
			);
			default
		}
	}
}

// Resolves the node address advertised by the ArtPollReply and falls back to the packet source.
fn resolve_artnet_node_address(source: SocketAddr, reply: &PollReply) -> SocketAddrV4 {
	if !reply.address.is_unspecified() {
		return SocketAddrV4::new(reply.address, reply.port);
	}

	match source {
		SocketAddr::V4(address) => SocketAddrV4::new(*address.ip(), reply.port),
		SocketAddr::V6(_) => SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, reply.port),
	}
}

// Resolves the first advertised Art-Net output universe from the node discovery reply.
fn resolve_artnet_port_address(reply: &PollReply) -> PortAddress {
	let value = (u16::from(reply.port_address[0] & 0x7F) << 8)
		| (u16::from(reply.port_address[1] & 0x0F) << 4)
		| u16::from(reply.swout[0] & 0x0F);

	PortAddress::try_from(value).unwrap_or_else(|_| Output::default().port_address)
}

pub fn process_default_window_input(
	input_system: &mut input::InputManager,
	event: ghi::Events,
) -> Option<(input::DeviceHandle, input::input_manager::TriggerReference, input::Value)> {
	let mouse_device_handle = input_system
		.get_devices_by_class_name("Mouse")
		.unwrap()
		.get(0)
		.unwrap()
		.clone();
	let keyboard_device_handle = input_system
		.get_devices_by_class_name("Keyboard")
		.unwrap()
		.get(0)
		.unwrap()
		.clone();

	let r = match event {
		ghi::window::Events::Button { pressed, button } => match button {
			ghi::window::input::MouseKeys::Left => (
				mouse_device_handle,
				input::input_manager::TriggerReference::Name("Mouse.LeftButton"),
				input::Value::Bool(pressed),
			),
			ghi::window::input::MouseKeys::Right => (
				mouse_device_handle,
				input::input_manager::TriggerReference::Name("Mouse.RightButton"),
				input::Value::Bool(pressed),
			),
			ghi::window::input::MouseKeys::ScrollUp => (
				mouse_device_handle,
				input::input_manager::TriggerReference::Name("Mouse.Scroll"),
				input::Value::Float(1f32),
			),
			ghi::window::input::MouseKeys::ScrollDown => (
				mouse_device_handle,
				input::input_manager::TriggerReference::Name("Mouse.Scroll"),
				input::Value::Float(-1f32),
			),
			ghi::window::input::MouseKeys::Middle => (
				mouse_device_handle,
				input::input_manager::TriggerReference::Name("Mouse.MiddleButton"),
				input::Value::Bool(pressed),
			),
		},
		ghi::window::Events::MousePosition { x, y, time: _ } => {
			let vec = Vector2::new(x, y);
			(
				mouse_device_handle,
				input::input_manager::TriggerReference::Name("Mouse.Position"),
				input::Value::Vector2(vec),
			)
		}
		ghi::window::Events::MouseMove { dx, dy, time: _ } => {
			let vec = Vector2::new(dx, dy);
			(
				mouse_device_handle,
				input::input_manager::TriggerReference::Name("Mouse.Movement"),
				input::Value::Vector2(vec),
			)
		}
		ghi::window::Events::Key { pressed, key } => match key {
			ghi::window::input::Keys::W => (
				keyboard_device_handle,
				input::input_manager::TriggerReference::Name("Keyboard.W"),
				input::Value::Bool(pressed),
			),
			ghi::window::input::Keys::S => (
				keyboard_device_handle,
				input::input_manager::TriggerReference::Name("Keyboard.S"),
				input::Value::Bool(pressed),
			),
			ghi::window::input::Keys::A => (
				keyboard_device_handle,
				input::input_manager::TriggerReference::Name("Keyboard.A"),
				input::Value::Bool(pressed),
			),
			ghi::window::input::Keys::D => (
				keyboard_device_handle,
				input::input_manager::TriggerReference::Name("Keyboard.D"),
				input::Value::Bool(pressed),
			),
			ghi::window::input::Keys::Space => (
				keyboard_device_handle,
				input::input_manager::TriggerReference::Name("Keyboard.Space"),
				input::Value::Bool(pressed),
			),
			ghi::window::input::Keys::Escape => (
				keyboard_device_handle,
				input::input_manager::TriggerReference::Name("Keyboard.Escape"),
				input::Value::Bool(pressed),
			),
			_ => {
				return None;
			}
		},
		_ => {
			return None;
		}
	};

	Some(r)
}

trait LogResult {
	fn warn(self) -> Self;
}

impl<T> LogResult for Result<T, &'static str> {
	fn warn(self) -> Self {
		match &self {
			Err(error) => {
				log::warn!("{}", error);
			}
			_ => {}
		}

		self
	}
}

impl<T> LogResult for Result<T, String> {
	fn warn(self) -> Self {
		match &self {
			Err(error) => {
				log::warn!("{}", error);
			}
			_ => {}
		}

		self
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn build_default_input_manager() -> input::InputManager {
		let action_channel = DefaultChannel::new();
		let action_listener = action_channel.listener();
		let event_channel = DefaultChannel::new();

		let mut input_manager = input::InputManager::new(action_listener, event_channel);

		let mouse_device_class_handle = register_mouse_device_class(&mut input_manager);
		let keyboard_device_class_handle = register_keyboard_device_class(&mut input_manager);

		input_manager.create_device(&mouse_device_class_handle);
		input_manager.create_device(&keyboard_device_class_handle);

		input_manager
	}

	#[test]
	fn maps_mouse_move_to_mouse_movement_trigger() {
		let mut input_manager = build_default_input_manager();
		let expected_mouse_device_handle = input_manager.get_devices_by_class_name("Mouse").unwrap()[0];

		let result = process_default_window_input(
			&mut input_manager,
			ghi::window::Events::MouseMove {
				dx: 0.25,
				dy: -0.5,
				time: 1,
			},
		)
		.unwrap();

		assert_eq!(result.0, expected_mouse_device_handle);
		assert!(matches!(
			result.1,
			input::input_manager::TriggerReference::Name("Mouse.Movement")
		));
		assert_eq!(result.2, input::Value::Vector2(Vector2::new(0.25, -0.5)));
	}

	#[test]
	fn resolve_artnet_node_address_prefers_the_reply_address() {
		let mut reply = PollReply::default();
		reply.address = Ipv4Addr::new(2, 0, 0, 10);
		reply.port = 6454;

		let resolved = resolve_artnet_node_address(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(2, 0, 0, 20), 6454)), &reply);

		assert_eq!(resolved, SocketAddrV4::new(Ipv4Addr::new(2, 0, 0, 10), 6454));
	}

	#[test]
	fn parse_artnet_ipv4_parameter_uses_the_parameter_value() {
		let parameter = Parameter::new("artnet.bind-address", "2.0.0.15");

		let address = parse_artnet_ipv4_parameter(Some(&parameter), Ipv4Addr::UNSPECIFIED);

		assert_eq!(address, Ipv4Addr::new(2, 0, 0, 15));
	}

	#[test]
	fn parse_artnet_ipv4_parameter_falls_back_to_the_default_value() {
		let parameter = Parameter::new("artnet.bind-address", "not-an-ip");

		let address = parse_artnet_ipv4_parameter(Some(&parameter), Ipv4Addr::BROADCAST);

		assert_eq!(address, Ipv4Addr::BROADCAST);
	}

	#[test]
	fn resolve_artnet_port_address_uses_the_reply_universe() {
		let mut reply = PollReply::default();
		reply.port_address = [0x02, 0x03];
		reply.swout[0] = 0x04;

		let port_address = resolve_artnet_port_address(&reply);

		assert_eq!(u16::from(port_address), 0x0234);
	}

	#[test]
	#[ignore] // Renderer broken.
	fn create_graphics_application() {
		let mut app = GraphicsApplication::new("Test", &[]);

		assert!(app.get_name() == "Test");

		let start_time = std::time::Instant::now();

		while !app.close {
			app.tick();

			if start_time.elapsed().as_secs() > 1 {
				app.close();
			}
		}
	}
}
