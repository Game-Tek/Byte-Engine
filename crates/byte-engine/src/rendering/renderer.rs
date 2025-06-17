use std::{
    borrow::BorrowMut, io::Write, ops::{Deref, DerefMut}, rc::Rc, sync::Arc
};

use ghi::{graphics_hardware_interface::Device as _, raster_pipeline, BoundComputePipelineMode, BoundRasterizationPipelineMode, CommandBufferRecordable, Device, RasterizationRenderPassMode};
use resource_management::resource::resource_manager::ResourceManager;
use utils::{hash::{HashMap, HashMapExt}, sync::RwLock, Extent, RGBA};

use crate::{
    core::{
        entity::EntityBuilder, listener::{CreateEvent, Listener}, Entity, EntityHandle
    }, window_system::{self, WindowSystem}
};

use super::{render_pass::{RenderPass, RenderPassBuilder}, texture_manager::TextureManager,};

/// The `Renderer` class centralizes the management of the rendering tasks and state.
/// It manages the creation of a Graphics Hardware Interfacec device and orchestrates render passes.
pub struct Renderer {
    ghi: Arc<RwLock<ghi::Device>>,

    started_frame_count: usize,
    rendered_frame_count: usize,

    frame_queue_depth: usize,

    swapchain_handles: Vec<ghi::SwapchainHandle>,

    render_command_buffer: ghi::CommandBufferHandle,
    render_finished_synchronizer: ghi::SynchronizerHandle,

    window_system: EntityHandle<window_system::WindowSystem>,

	targets: HashMap<String, ghi::ImageHandle>,

    root_render_pass: RootRenderPass,

    extent: Extent,
}

impl Renderer {
    pub fn new(window_system_handle: EntityHandle<WindowSystem>, resource_manager_handle: EntityHandle<ResourceManager>, settings: Settings) -> Self {
		let ghi_instance = Arc::new(RwLock::new(ghi::create(
			ghi::Features::new()
				.validation(settings.validation)
				.api_dump(false)
				.gpu_validation(false)
				.debug_log_function(|message| {
					log::error!("{}", message);
				})
				.geometry_shader(false)
		).unwrap()));

		let extent = Extent::square(0); // Initialize extent to 0 to allocate memory lazily.

		let render_command_buffer;
		let render_finished_synchronizer;

		let mut targets = HashMap::new();

		{
			let mut ghi = ghi_instance.write();

			let result = ghi.create_image(
				Some("result"),
				extent,
				ghi::Formats::RGBA8(ghi::Encodings::UnsignedNormalized),
				ghi::Uses::Storage | ghi::Uses::TransferDestination,
				ghi::DeviceAccesses::GpuWrite | ghi::DeviceAccesses::GpuRead,
				ghi::UseCases::DYNAMIC,
				1,
			);
			let main = ghi.create_image(
				Some("main"),
				extent,
				ghi::Formats::RGBA16(ghi::Encodings::UnsignedNormalized),
				ghi::Uses::Storage | ghi::Uses::TransferSource | ghi::Uses::BlitDestination | ghi::Uses::RenderTarget,
				ghi::DeviceAccesses::GpuWrite | ghi::DeviceAccesses::GpuRead,
				ghi::UseCases::DYNAMIC,
				1,
			);
			let depth = ghi.create_image(
				Some("depth"),
				extent,
				ghi::Formats::Depth32,
				ghi::Uses::RenderTarget | ghi::Uses::Image,
				ghi::DeviceAccesses::GpuWrite | ghi::DeviceAccesses::GpuRead,
				ghi::UseCases::DYNAMIC,
				1,
			);

			targets.insert("main".to_string(), main);
			targets.insert("depth".to_string(), depth);
			targets.insert("result".to_string(), result);

			render_command_buffer = ghi.create_command_buffer(Some("Render"));
			render_finished_synchronizer = ghi.create_synchronizer(Some("Render Finisished"), true);
		};

		let texture_manager = Arc::new(RwLock::new(TextureManager::new()));

		let mut root_render_pass = RootRenderPass::new();

		root_render_pass.add_image("main".to_string(), targets.get("main").unwrap().clone(), ghi::Formats::RGBA16(ghi::Encodings::UnsignedNormalized), ghi::Layouts::RenderTarget);
		root_render_pass.add_image("depth".to_string(), targets.get("depth").unwrap().clone(), ghi::Formats::Depth32, ghi::Layouts::RenderTarget);
		root_render_pass.add_image("result".to_string(), targets.get("result").unwrap().clone(), ghi::Formats::RGBA8(ghi::Encodings::UnsignedNormalized), ghi::Layouts::RenderTarget);

		Renderer {
			ghi: ghi_instance,

			started_frame_count: 0,
			rendered_frame_count: 0,

			frame_queue_depth: 2,

			swapchain_handles: vec![],

			render_command_buffer,
			render_finished_synchronizer,

			window_system: window_system_handle,

			targets,

			root_render_pass,

			extent,
		}
    }

	pub fn add_render_pass<T: RenderPass + Entity + Send + Sync + 'static>(&mut self, creator: impl FnOnce(&mut RenderPassBuilder<'_>) -> EntityHandle<T>) {
		let read_attachments = T::get_read_attachments();

		if !read_attachments.iter().all(|a| self.targets.contains_key(*a)) {
			return;
		}

		let mut render_pass_builder = RenderPassBuilder::new(self.ghi.clone());

		let main_image = self.root_render_pass.images.get("main").unwrap().clone();
		let depth_image = self.root_render_pass.images.get("depth").unwrap().clone();
		let result_image = self.root_render_pass.images.get("result").unwrap().clone();

		render_pass_builder.images.insert("main".to_string(), (main_image.0, main_image.1, 0));
		render_pass_builder.images.insert("depth".to_string(), (depth_image.0, depth_image.1, 0));
		render_pass_builder.images.insert("result".to_string(), (result_image.0, result_image.1, 0));

		let render_pass = creator(&mut render_pass_builder,);

		self.root_render_pass.add_render_pass(render_pass, render_pass_builder);
	}

	/// This function prepares a frame by invoking multiple render passes.
	/// If no swapchains are available no rendering/execution will be performed.
	/// If some swapchain surface is 0 sized along some dimension no rendering/execution will be performed.
    pub fn prepare(&mut self) -> Option<RenderMessage> {
        let swapchain_handle = if let Some(&sh) = self.swapchain_handles.first() {
        	sh
        } else {
        	log::warn!("No swapchain available to present to. Skipping rendering!");
        	return None;
        };

        let mut ghi = self.ghi.write();

		let frame_key = ghi.start_frame(self.started_frame_count as u32, self.render_finished_synchronizer);

		self.started_frame_count += 1;

        ghi.start_frame_capture();

        let (present_key, extent) = ghi.acquire_swapchain_image(frame_key, swapchain_handle,);

        drop(ghi);

        assert!(extent.width() <= 65535 && extent.height() <= 65535, "The extent is too large: {:?}. The renderer only supports dimensions as big as 16 bits.", extent);

        if extent.width() == 0 || extent.height() == 0 {
        	log::info!("Swapchain extent is zero in either or both dimension. Skipping rendering!");
         	return None;
        }

        if extent != self.extent {
            let mut ghi = self.ghi.write();

			for (_, image) in self.targets.iter_mut() {
				ghi.resize_image(*image, extent);
			}

            self.extent = extent;
        }

        let mut ghi = self.ghi.write();

		let execute = self.root_render_pass.prepare(&mut ghi, extent);

		RenderMessage::new(swapchain_handle, frame_key, present_key, execute).into()
    }

	/// This function executes the prepared render frame.
    pub fn render(&mut self, message: RenderMessage) {
    	let mut ghi = self.ghi.write();

		let frame_key = message.frame_key;
		let present_key = message.present_key;
		let swapchain_handle = message.swapchain_handle;
		let execute = message.execute;

        let mut command_buffer_recording = ghi.create_command_buffer_recording(
            self.render_command_buffer,
            frame_key.into(),
        );

		command_buffer_recording.sync_buffers(); // Copy/sync all dirty buffers to the GPU.
		command_buffer_recording.sync_textures(); // Copy/sync all dirty textures to the GPU.

        execute(&mut command_buffer_recording);

		let result = self.targets.get("result").unwrap();

        command_buffer_recording.copy_to_swapchain(*result, present_key, swapchain_handle);

        command_buffer_recording.execute(
            &[],
            &[self.render_finished_synchronizer],
            &[present_key],
            self.render_finished_synchronizer,
        );

        ghi.end_frame_capture();

        self.rendered_frame_count += 1;
    }
}

pub struct RenderMessage {
    swapchain_handle: ghi::SwapchainHandle,
    frame_key: ghi::FrameKey,
    present_key: ghi::PresentKey,
    execute: Box<dyn FnOnce(&mut ghi::CommandBufferRecording) + Send + Sync>,
}

impl RenderMessage {
    fn new(
        swapchain_handle: ghi::SwapchainHandle,
        frame_key: ghi::FrameKey,
        present_key: ghi::PresentKey,
        execute: impl FnOnce(&mut ghi::CommandBufferRecording) + Send + Sync + 'static,
    ) -> Self {
        Self {
            swapchain_handle,
            frame_key,
            present_key,
            execute: Box::new(execute),
        }
    }
}

impl Listener<CreateEvent<window_system::Window>> for Renderer {
    fn handle(&mut self, event: &CreateEvent<window_system::Window>) {
		let handle = event.handle();

        let os_handles = self.window_system.map(|e| {
            let e = e.read();
            e.get_os_handles(&handle)
        });

        let mut ghi = self.ghi.write();

        let swapchain_handle = ghi.bind_to_window(
            &os_handles,
            ghi::PresentationModes::FIFO,
            Extent::rectangle(1920, 1080),
        );

        self.swapchain_handles.push(swapchain_handle);
    }
}

impl Entity for Renderer {
	fn builder(self) -> EntityBuilder<'static, Self> where Self: Sized {
		EntityBuilder::new(self)
			.listen_to::<CreateEvent<window_system::Window>>()
	}
}

struct RootRenderPass {
    render_passes: Vec<(EntityHandle<dyn RenderPass + Send + Sync>, Vec<String>)>,
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

    fn add_render_pass(&mut self, render_pass: EntityHandle<dyn RenderPass + Send + Sync>, render_pass_builder: RenderPassBuilder) {
		let index = self.render_passes.len();
        self.render_passes.push((render_pass, render_pass_builder.consumed_resources.iter().map(|e| e.0.to_string()).collect()));
		self.order.push(index);
    }

    /// This function prepares every render pass for rendering.
    /// Usually the preparation step involves writing to buffers, culling drawables, determining what to draw and whether to even draw at all.
    /// Individual render pass prepare's can optionally return render pass execution functions which decide if a render pass gets executed.
    /// This can be because the render pass may be disabled or because some other internal conditions are not satisfied.
    fn prepare(&self, ghi: &mut ghi::Device, extent: Extent) -> impl FnOnce(&mut ghi::CommandBufferRecording) + Send + Sync {
		let commands = self.order.iter().map(|index| {
			let (render_pass, consumed) = &self.render_passes[*index];
			let attachments = consumed.iter().map(|c| {
				let (image, format, layout) = self.images.get(c).unwrap();
				ghi::AttachmentInformation::new(*image, *format, *layout, ghi::ClearValue::Color(RGBA::black()), false, true)
			}).collect::<Vec<_>>();

            let command = render_pass.get_mut(|e| {
                e.prepare(ghi, extent)
            });

			(attachments, command)
        }).collect::<Vec<_>>();

		move |c: &mut ghi::CommandBufferRecording<'_>| {
			for (attachments, command) in commands {
				if let Some(command) = command {
					command(c, &attachments);
				}
			}
		}
    }
}

struct Attachment {
	name: String,
	image: ghi::ImageHandle,
}

/// This struct holds the settings to configure a `Renderer` during it's creation.
pub struct Settings {
	/// Controls whether validation layers will be enabled or not on the GHI device.
	validation: bool,
}

impl Settings {
	/// Creates a new `Settings` struct.
	/// - `validation` is true by default in debug builds and false in release.
	pub fn new() -> Self {
		Self {
			validation: cfg!(debug_assertions),
		}
	}

	pub fn validation(mut self, value: bool) -> Self {
		self.validation = true;
		self
	}
}
