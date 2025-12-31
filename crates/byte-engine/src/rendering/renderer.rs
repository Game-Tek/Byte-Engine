use std::{
	borrow::BorrowMut, io::Write, ops::{Deref, DerefMut}, rc::Rc, sync::Arc
};

use ghi::{command_buffer::{BoundComputePipelineMode as _, BoundRasterizationPipelineMode as _, CommandBufferRecordable as _, RasterizationRenderPassMode as _}, device::Device as _, frame::Frame as _, raster_pipeline, vulkan::command_buffer};
use resource_management::resource::resource_manager::ResourceManager;
use utils::{hash::{HashMap, HashMapExt}, sync::RwLock, Extent, RGBA};

use crate::{
	application::parameters::Parameters, core::{
		Entity, EntityHandle, entity::EntityBuilder, listener::{CreateEvent, Listener}
	}, gameplay::space::Spawner, rendering::{View, Viewport, render_pass::{FramePrepare, RenderPassViewCommand}, window::Window}
};

use super::{render_pass::{RenderPass, RenderPassBuilder}, texture_manager::TextureManager,};

/// The `Renderer` class centralizes the management of the rendering tasks and state.
/// It manages the creation of a Graphics Hardware Interfacec device and orchestrates render passes.
pub struct Renderer {
	instance: ghi::Instance,
	device: ghi::Device,

	started_frame_count: usize,

	frame_queue_depth: usize,

	windows: Vec<(ghi::Window, ghi::SwapchainHandle)>,
	views: Vec<(usize, View)>,

	render_targets: HashMap<String, (ghi::ImageHandle, ghi::Formats, i8)>,

	render_passes: Vec<EntityHandle<dyn RenderPass>>,

	render_command_buffer: ghi::CommandBufferHandle,
	render_finished_synchronizer: ghi::SynchronizerHandle,
}

impl Renderer {
	/// Creates a new renderer. Accepts a paramters interface.
	///
	/// # Paramters
	/// - `render.debug`: Enables validation layers for debugging. Defaults to true on debug builds.
	/// - `render.debug.dump`: Enables API dump for debugging. Defaults to false.
	/// - `render.debug.extended`: Enables extended validation for debugging. Defaults to false.
	/// - `render.ghi.features.mesh-shading`: Enables mesh shading features on the graphics device. Defaults to true.
	pub fn new(spawner: &mut impl Spawner, resource_manager_handle: EntityHandle<ResourceManager>, parameters: &dyn Parameters) -> Self {
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

		let settings = if let Some(param) = parameters.get_parameter("render.ghi.features.mesh-shading") {
			settings.mesh_shading(param.as_bool_simple())
		} else {
			settings
		};

		let features = ghi::Features::new()
			.validation(settings.validation)
			.api_dump(settings.api_dump)
			.gpu_validation(settings.extended_validation)
			.debug_log_function(|message| {
				log::error!("{}\n{}", message, std::backtrace::Backtrace::force_capture());
			})
			.geometry_shader(false)
			.mesh_shading(settings.mesh_shading)
		;

		let mut instance = ghi::Instance::new(features.clone()).unwrap();

		let mut queue_handle = None;

		let mut device = instance.create_device(features.clone(), &mut [(ghi::QueueSelection::new(ghi::CommandBufferType::GRAPHICS), &mut queue_handle)]).unwrap();

		let queue_handle = queue_handle.unwrap();

		let render_command_buffer = device.create_command_buffer(Some("Render"), queue_handle);
		let render_finished_synchronizer = device.create_synchronizer(Some("Render Finisished"), true);

		let texture_manager = Arc::new(RwLock::new(TextureManager::new()));

		let root_render_pass = RootRenderPass::new();
		let root_render_pass: EntityHandle<dyn RenderPass> = spawner.spawn(root_render_pass.builder());

		let mut render_passes = Vec::with_capacity(64);

		render_passes.push(root_render_pass);

		Renderer {
			instance,
			device,

			started_frame_count: 0,

			frame_queue_depth: 2,

			windows: Vec::with_capacity(16),
			views: Vec::with_capacity(16),

			render_targets: HashMap::with_capacity(32),

			render_passes,

			render_command_buffer,
			render_finished_synchronizer,
		}
	}

	/// Adds a render pass to the renderer's pipeline.
	/// All windows will use this render passes.
	pub fn add_render_pass<T: RenderPass + Entity + 'static>(&mut self, creator: impl Fn(&mut RenderPassBuilder<'_>) -> EntityHandle<T>) {
		let mut render_pass_builder = RenderPassBuilder::new(&mut self.device, &mut self.render_targets);

		let render_pass = creator(&mut render_pass_builder,);

		{
			let render_pass = render_pass.write();

			for _ in &self.windows {
				render_pass.create_view();
			}
		}

		self.render_passes.push(render_pass);
	}

	pub fn update_windows<'a>(&'a mut self) -> impl Iterator<Item = impl Iterator<Item = ghi::Events> + 'a> + 'a {
		self.windows.iter_mut().map(|(window, _)| {
			window.poll()
		})
	}

	/// This function prepares a frame by invoking multiple render passes.
	/// If no swapchains are available no rendering/execution will be performed.
	/// If some swapchain surface is 0 sized along some dimension no rendering/execution will be performed.
	pub fn prepare(&'_ mut self) -> Option<RenderMessage<'_>> {
		let Some(_) = self.windows.first() else {
			log::warn!("No swapchains available to present to. Skipping rendering!");
			return None;
		};

		let device = &mut self.device;

		device.start_frame_capture();

		let mut frame = device.start_frame(self.started_frame_count as u32, self.render_finished_synchronizer);

		self.started_frame_count += 1;

		let mut executions = Vec::with_capacity(8);

		let swapchains = self.windows.iter().map(|(window, swapchain)| {
			let (present_key, extent) = frame.acquire_swapchain_image(*swapchain);

			if extent.width() == 0 || extent.height() == 0 {
				log::warn!("The extent is too small: {:?}. Rendering will be skipped.", extent);
				return None;
			}

			if extent.width() >= 65535 || extent.height() >= 65535 {
				log::warn!("The extent is too large: {:?}. The renderer only supports dimensions as big as 16 bits. Rendering will be skipped.", extent);
				return None;
			}

			Some((present_key, extent, *swapchain))
		}).collect::<Vec<_>>();

		let views = self.views.iter();

		let render_passes = self.render_passes.iter().map(|render_pass| {
			let render_pass = render_pass.read();
			let execute = render_pass.prepare(&mut frame, FramePrepare::new());
			execute
		});

		for (index, view) in views {
			let Some((present_key, extent, swapchain)) = swapchains[*index] else {
				continue;
			};

			let viewport = Viewport::new(*view, extent, *index);

			// We assume every view renders the same set of render passes
			for render_pass in render_passes.clone() {
				executions.push(execute);
			}
		}

		let execute = move |e: &mut ghi::CommandBufferRecording| {
			for execute in executions {
				execute(e);
			}
		};

		RenderMessage::new(frame, self.render_command_buffer, self.render_finished_synchronizer, execute).into()
	}
}

pub struct RenderMessage<'a> {
	frame: ghi::Frame<'a>,
	command_buffer: ghi::CommandBufferHandle,
	synchronizer: ghi::SynchronizerHandle,
	execute: Box<dyn FnOnce(&mut ghi::CommandBufferRecording) + Send + Sync>,
}

impl <'a> RenderMessage<'a> {
	fn new(
		frame: ghi::Frame<'a>,
		command_buffer: ghi::CommandBufferHandle,
		synchronizer: ghi::SynchronizerHandle,
		execute: impl FnOnce(&mut ghi::CommandBufferRecording) + Send + Sync + 'static,
	) -> Self {
		Self {
			frame,
			command_buffer,
			synchronizer,
			execute: Box::new(execute),
		}
	}

	pub fn render(self) {
		let mut frame = self.frame;
		let command_buffer = self.command_buffer;
		let synchronizer = self.synchronizer;
		let execute = self.execute;

		let mut command_buffer_recording = frame.create_command_buffer_recording(
			command_buffer,
		);

		command_buffer_recording.sync_buffers(); // Copy/sync all dirty buffers to the GPU.
		command_buffer_recording.sync_textures(); // Copy/sync all dirty textures to the GPU.

		execute(&mut command_buffer_recording);

		command_buffer_recording.execute(
			&[],
			&[],
			&[],
			synchronizer,
		);
	}
}

impl Entity for Renderer {
	fn builder(self) -> EntityBuilder<'static, Self> where Self: Sized {
		EntityBuilder::new(self)
			.listen_to::<CreateEvent<Window>>()
	}
}

impl Listener<CreateEvent<Window>> for Renderer {
	fn handle(&mut self, event: &CreateEvent<Window>) {
		let handle = event.handle();
		let window = handle.read();

		let name = window.name();
		let extent = window.extent();

		let window = ghi::Window::new_with_params(name, extent, "main_window");

		match window {
			Ok(window) => {
				let os_handles = window.os_handles();

				let device = &mut self.device;

				let swapchain_handle = device.bind_to_window(
					&os_handles,
					ghi::PresentationModes::FIFO,
					extent,
				);

				self.windows.push((window, swapchain_handle));
			}
			Err(msg) => {
				log::error!("Failed to create GHI window: {}", msg);
			}
		}
	}
}

struct RootRenderPass {
	render_passes: Vec<(EntityHandle<dyn RenderPass>, Vec<String>)>,
	images: HashMap<String, (ghi::ImageHandle, ghi::Formats, ghi::Layouts,)>,
	order: Vec<usize>,
}

impl RootRenderPass {
	pub fn new() -> Self {
		Self {
			render_passes: Vec::with_capacity(32),
			images: HashMap::new(),
			order: Vec::with_capacity(32),
		}
	}

	fn create(render_pass_builder: &mut RenderPassBuilder) -> EntityBuilder<'static, Self> where Self: Sized {
		Self::new().into()
	}

	fn add_image(&mut self, name: String, image: ghi::ImageHandle, format: ghi::Formats, layout: ghi::Layouts) {
		self.images.insert(name, (image, format, layout));
	}

	fn add_render_pass(&mut self, render_pass: EntityHandle<dyn RenderPass>, render_pass_builder: RenderPassBuilder) {
		let index = self.render_passes.len();
		self.render_passes.push((render_pass, render_pass_builder.consumed_resources.iter().map(|e| e.0.to_string()).collect()));
		self.order.push(index);
	}

	/// This function prepares every render pass for rendering.
	/// Usually the preparation step involves writing to buffers, culling drawables, determining what to draw and whether to even draw at all.
	/// Individual render pass prepare's can optionally return render pass execution functions which decide if a render pass gets executed.
	/// This can be because the render pass may be disabled or because some other internal conditions are not satisfied.
	fn prepare(&self, frame: &mut ghi::Frame, extent: Extent, present_key: ghi::PresentKey, swapchain_handle: ghi::SwapchainHandle) -> impl FnOnce(&mut ghi::CommandBufferRecording) + Send + Sync {
		let result = self.get_target("result");

		let commands = self.order.iter().map(|index| {
			let (render_pass, consumed) = &self.render_passes[*index];
			let attachments = consumed.iter().map(|c| {
				let (image, format, layout) = self.images.get(c).unwrap();
				ghi::AttachmentInformation::new(*image, *format, *layout, ghi::ClearValue::Color(RGBA::black()), false, true)
			}).collect::<Vec<_>>();

			let command = render_pass.get_mut(|e| {
				e.prepare(frame)
			});

			(attachments, command)
		}).collect::<Vec<_>>();

		move |c: &mut ghi::CommandBufferRecording<'_>| {
			let Some(result) = result else {
				return;
			};

			for (attachments, command) in commands {
				if let Some(command) = command {
					command(c);
				}
			}

			c.copy_to_swapchain(result, present_key, swapchain_handle);

			c.present(present_key);
		}
	}

	fn does_target_exist(&self, name: &str) -> bool {
		self.images.contains_key(name)
	}

	fn targets(&self) -> impl Iterator<Item = &ghi::ImageHandle> {
		self.images.values().map(|(image, _, _)| image)
	}

	fn get_target(&self, name: &str) -> Option<ghi::ImageHandle> {
		self.images.get(name).map(|(image, _, _)| *image)
	}
}

impl RenderPass for RootRenderPass {
	fn create_view(&self) {
		todo!()
	}
}

impl Entity for RootRenderPass {}

struct Attachment {
	name: String,
	image: ghi::ImageHandle,
}

/// This struct holds the settings to configure a `Renderer` during it's creation.
pub struct Settings {
	/// Controls whether validation layers will be enabled or not on the GHI device.
	validation: bool,
	/// Controls whether to enable or not writing out the parameters sent to the underlaying graphics API. Depends on `validation` being enabled.
	api_dump: bool,
	/// Controls wheter to enable or not some extra (bbut expensive) validation for the graphics API. This can include GPU validation. Depends on `validation` being enabled.
	extended_validation: bool,
	/// Controls whether to enable or not mesh shading on the GHI device.
	mesh_shading: bool,
}

impl Settings {
	/// Creates a new `Settings` struct.
	/// - `validation` is true by default in debug builds and false in release.
	/// - `api_dump` is false by default.
	/// - `extended_validation` is false by default.
	pub fn new() -> Self {
		Self {
			validation: cfg!(debug_assertions),
			api_dump: false,
			extended_validation: false,
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

	pub fn mesh_shading(mut self, value: bool) -> Self {
		self.mesh_shading = value;
		self
	}
}

struct FrameBuffer {
	root_render_pass: RwLock<RootRenderPass>,
}

impl FrameBuffer {
	fn new(device: &mut ghi::Device) -> Self {
		let mut root_render_pass = RootRenderPass::new();

		let extent = Extent::square(0); // Initialize extent to 0 to allocate memory lazily.

		let result = device.create_image(
			Some("result"),
			extent,
			ghi::Formats::RGBA8(ghi::Encodings::UnsignedNormalized),
			ghi::Uses::Storage | ghi::Uses::TransferDestination | ghi::Uses::TransferSource,
			ghi::DeviceAccesses::DeviceOnly,
			ghi::UseCases::DYNAMIC,
			None,
		);

		let main = device.create_image(
			Some("main"),
			extent,
			ghi::Formats::RGBA16(ghi::Encodings::UnsignedNormalized),
			ghi::Uses::Storage | ghi::Uses::TransferSource | ghi::Uses::BlitDestination | ghi::Uses::RenderTarget,
			ghi::DeviceAccesses::DeviceOnly,
			ghi::UseCases::DYNAMIC,
			None,
		);

		let depth = device.create_image(
			Some("depth"),
			extent,
			ghi::Formats::Depth32,
			ghi::Uses::RenderTarget | ghi::Uses::Image,
			ghi::DeviceAccesses::DeviceOnly,
			ghi::UseCases::DYNAMIC,
			None,
		);

		root_render_pass.add_image("main".to_string(), main, ghi::Formats::RGBA16(ghi::Encodings::UnsignedNormalized), ghi::Layouts::RenderTarget);
		root_render_pass.add_image("depth".to_string(), depth, ghi::Formats::Depth32, ghi::Layouts::RenderTarget);
		root_render_pass.add_image("result".to_string(), result, ghi::Formats::RGBA8(ghi::Encodings::UnsignedNormalized), ghi::Layouts::RenderTarget);

		let root_render_pass = RwLock::new(root_render_pass);

		Self {
			root_render_pass,
		}
	}
}
