/// The `Renderer` class centralizes the management of the rendering tasks and state.
/// It manages the creation of a Graphics Hardware Interfacec device and orchestrates render passes.
pub struct Renderer {
	/// The GHI instance that manages devices.
	instance: ghi::implementation::Instance,
	/// The GHI device that is used for rendering.
	device: Arc<ghi::implementation::Device>,
	/// The GHI context where all rendering resources and operations are performed.
	context: ghi::implementation::Context,

	started_frame_count: usize,

	frame_queue_depth: usize,

	/// A list of display windows and their associated swapchains.
	windows: SmallVec<[(ghi::Window, ghi::SwapchainHandle); 16]>,
	/// A list of sink indices and their associated camera handles.
	sink_cameras: SmallVec<[(SinkId, Handle); 16]>,
	/// A list of cameras and their associated handles.
	cameras: SmallVec<[(Handle, Camera); 16]>,

	render_targets: RenderTargets,

	render_passes: SmallVec<[Box<dyn RenderPass>; 64]>,
	render_passes_by_sink: SmallVec<[(RenderPassId, SinkId); 32]>,
	post_scene_render_pass_factories: SmallVec<[Box<RenderPassFactory>; 16]>,
	pending_swapchain_captures: SmallVec<[SwapchainCapture; 16]>,

	pipeline_managers: SmallVec<[Box<dyn PipelineManager>; 16]>,

	/// The GHI queue where graphics commands are submitted. The main rendering operations occur on this queue.
	graphics_queue_handle: ghi::QueueHandle,
	/// The GHI queue where transfer commands are submitted. Async transfer operations occur on this queue.
	pub transfer_queue_handle: ghi::QueueHandle,

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
	/// - `render.ghi.features.mesh-shading`: Enables mesh shading features on the graphics context. Defaults to true.
	pub fn new(parameters: &dyn Parameters) -> Self {
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

		let mut features = ghi::device::Features::new()
			.validation(settings.validation)
			.api_dump(settings.api_dump)
			.gpu_validation(settings.extended_validation)
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
					"Renderer validation was requested but could not be enabled: {error} Falling back to renderer validation disabled."
				);
				features = features.validation(false).gpu_validation(false).api_dump(false);
				ghi::implementation::Instance::new(features).unwrap()
			}
			Err(error) => panic!("Failed to create GHI instance: {error}"),
		};

		let mut graphics_queue_handle = None;
		let mut transfer_queue_handle = None;

		let device = instance
			.create_device(
				features.clone(),
				&mut [
					(
						ghi::QueueSelection::new(ghi::types::WorkloadTypes::RASTER),
						&mut graphics_queue_handle,
					),
					(
						ghi::QueueSelection::new(ghi::types::WorkloadTypes::TRANSFER),
						&mut transfer_queue_handle,
					),
				],
			)
			.unwrap();
		let mut context = device.create_context().unwrap();

		let graphics_queue_handle = graphics_queue_handle.unwrap();
		let transfer_queue_handle = transfer_queue_handle.unwrap();

		let render_command_buffer = context
			.queue_reference(graphics_queue_handle)
			.create_command_buffer(Some("Render"));
		let render_finished_synchronizer = context.create_synchronizer(Some("Render Finisished"), true);

		Renderer {
			context,
			device: Arc::new(device),
			instance,

			started_frame_count: 0,

			frame_queue_depth: 2,

			windows: SmallVec::with_capacity(16),
			sink_cameras: SmallVec::with_capacity(16),
			cameras: SmallVec::with_capacity(16),

			render_targets: RenderTargets::new(),

			render_passes: SmallVec::with_capacity(64),
			render_passes_by_sink: SmallVec::with_capacity(32),
			post_scene_render_pass_factories: SmallVec::with_capacity(16),
			pending_swapchain_captures: SmallVec::with_capacity(16),

			pipeline_managers: SmallVec::with_capacity(8),

			graphics_queue_handle,
			transfer_queue_handle,

			render_command_buffer,
			render_finished_synchronizer,
		}
	}

	pub fn add_pipeline_manager(&mut self, mut pipeline_manager: impl PipelineManager + 'static) {
		{
			let sink_swapchains: SmallVec<[(SinkId, ghi::SwapchainHandle); 16]> = self
				.sink_cameras
				.iter()
				.map(|(sink_id, _)| (*sink_id, self.windows[*sink_id].1))
				.collect();
			for (sink_id, swapchain) in sink_swapchains {
				let mut rpb = RenderPassBuilder::new(&mut self.context, &mut self.render_targets, sink_id, swapchain);

				pipeline_manager.create_sink(sink_id, &mut rpb);

				if rpb.consumed_resources.len() == 0 {
					log::debug!("No resources consumed by scene manager");
				}
			}
		}

		self.pipeline_managers.push(Box::new(pipeline_manager));
	}

	fn add_render_pass(&mut self, render_pass: Box<dyn RenderPass>, sink_id: SinkId) {
		let render_pass_id = self.render_passes.len();
		self.render_passes.push(render_pass);
		self.render_passes_by_sink.push((render_pass_id, sink_id));
	}

	/// Registers a render pass factory that will be instantiated for every current and future sink.
	pub fn add_post_scene_render_pass_for_all_sinks<F>(&mut self, render_pass_factory: F)
	where
		F: for<'a> Fn(&'a mut RenderPassBuilder<'a>) -> Box<dyn RenderPass> + 'static,
	{
		let render_pass_factory: Box<RenderPassFactory> = Box::new(render_pass_factory);
		let sink_ids: SmallVec<[usize; 16]> = self.sink_cameras.iter().map(|(sink_id, _)| *sink_id).collect();

		for sink_id in sink_ids {
			let render_pass = {
				let swapchain = self.windows[sink_id].1;
				let mut render_pass_builder =
					RenderPassBuilder::new(&mut self.context, &mut self.render_targets, sink_id, swapchain);
				render_pass_factory(&mut render_pass_builder)
			};

			self.add_render_pass(render_pass, sink_id);
		}

		self.post_scene_render_pass_factories.push(render_pass_factory);
	}

	/// Instantiates all registered post-scene render pass factories for a given sink.
	fn add_post_scene_render_passes_for_sink(&mut self, sink_id: SinkId) {
		let mut render_passes_for_sink: SmallVec<[Box<dyn RenderPass>; 16]> = SmallVec::new();

		let swapchain = self.windows[sink_id].1;

		for render_pass_factory in &self.post_scene_render_pass_factories {
			let render_pass = {
				let mut render_pass_builder =
					RenderPassBuilder::new(&mut self.context, &mut self.render_targets, sink_id, swapchain);
				render_pass_factory(&mut render_pass_builder)
			};

			render_passes_for_sink.push(render_pass);
		}

		for render_pass in render_passes_for_sink {
			self.add_render_pass(render_pass, sink_id);
		}
	}

	pub fn update_windows<'a>(&'a mut self) -> impl Iterator<Item = impl Iterator<Item = ghi::window::Events> + 'a> + 'a {
		self.windows.iter_mut().map(|(window, _)| window.poll())
	}

	/// Schedules copying a sink's swapchain image into a buffer during the next prepared frame.
	pub fn capture_swapchain_to_buffer(
		&mut self,
		sink_id: SinkId,
		destination_buffer: impl Into<ghi::BaseBufferHandle>,
		destination_offset: usize,
		destination_bytes_per_row: usize,
		destination_bytes_per_image: usize,
	) {
		self.pending_swapchain_captures.push(SwapchainCapture {
			sink_id,
			destination_buffer: destination_buffer.into(),
			destination_offset,
			destination_bytes_per_row,
			destination_bytes_per_image,
		});
	}

	/// This function prepares a frame by invoking multiple render passes.
	/// If no swapchains are available no rendering/execution will be performed.
	/// If some swapchain surface is 0 sized along some dimension no rendering/execution will be performed.
	pub fn prepare(&'_ mut self, transforms_listener: &mut impl Listener<TransformationUpdate>) {
		let Some(_) = self.windows.first() else {
			log::debug!("No swapchains available to present to. Skipping rendering!");
			return;
		};

		self.context.start_frame_capture();

		let mut transforms_listener = transforms_listener.to_vec();

		transforms_listener.retain(|message| {
			let handle = message.handle().clone();

			if let Some(camera) = self
				.cameras
				.iter_mut()
				.find_map(|(h, camera)| if handle == *h { Some(camera) } else { None })
			{
				camera.set_position(message.transform().get_position());
				camera.set_orientation(message.transform().get_orientation());
				false
			} else {
				true
			}
		});

		let mut queue = self.context.queue(self.graphics_queue_handle);
		let frame = ghi::queue::FrameRequest {
			index: self.started_frame_count as u32,
			synchronizer: self.render_finished_synchronizer,
		};

		self.started_frame_count += 1;

		let command_buffer = self.render_command_buffer;
		let synchronizer = self.render_finished_synchronizer;
		let wait_for = &[];
		let windows = &self.windows;
		let sink_cameras = &self.sink_cameras;
		let cameras = &self.cameras;
		let render_targets = &self.render_targets;
		let pipeline_managers = &mut self.pipeline_managers;
		let render_passes = &mut self.render_passes;
		let render_passes_by_sink = &self.render_passes_by_sink;
		let pending_swapchain_captures = self.pending_swapchain_captures.drain(..).collect::<SmallVec<[_; 16]>>();

		queue.execute(Some(frame), wait_for, synchronizer, |execution| {
			let completed_graphics_frame = execution.completed_frame();

			let (sinks, pipeline_manager_commands, render_pass_commands, present_keys, swapchain_capture_copies) = {
				let frame = execution.frame().expect(
					"Frame is required to prepare renderer frame work. The most likely cause is that Renderer::render called Queue::execute without a frame request.",
				);
				let swapchains: SmallVec<[Option<(ghi::PresentKey, Extent, ghi::SwapchainHandle)>; 16]> = windows
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
					.collect();

				let mut sinks: SmallVec<[Sink; 16]> = SmallVec::new();

				for (sink_id, camera_handle) in sink_cameras.iter() {
					let Some((_present_key, extent, _swapchain)) = swapchains[*sink_id] else {
						continue;
					};

					let Some(camera) = cameras
						.iter()
						.find_map(|(handle, camera)| if handle == camera_handle { Some(camera) } else { None })
					else {
						continue;
					};

					let view = make_perspective_view_from_camera(&camera, extent);
					sinks.push(Sink::new(view, extent, *sink_id));
				}

				for sink in &sinks {
					// Get images for the current sink and render pass and resize them to window extent
					let images = render_targets.get_images_for_sink(sink.index());

					// Resize images to window extent
					for &image in images {
						frame.resize_image(image.into(), sink.extent());
					}
				}

				let pipeline_managers = pipeline_managers.iter_mut();

				let pipeline_manager_commands: SmallVec<[Vec<Box<dyn RenderPassFunction>>; 16]> =
					pipeline_managers.filter_map(|sm| sm.prepare(frame, &sinks)).collect();

				// A list of render pass commands and their corresponding sink index
				let render_pass_commands: SmallVec<[(RenderPassReturn, SinkId); 64]> = render_passes_by_sink
					.iter()
					.filter_map(|(render_pass_id, sink_id)| {
						if let Some(render_pass) = render_passes.get_mut(*render_pass_id) {
							if let Some(sink) = sinks.iter().find(|sink| sink.index() == *sink_id) {
								if let Some(command) = render_pass.prepare(frame, sink) {
									return Some((command, sink.index()));
								}
							}
						}
						None
					})
					.collect();

				let present_keys = swapchains
					.iter()
					.filter_map(|sc| sc.as_ref().map(|(pk, ..)| *pk))
					.collect::<SmallVec<[ghi::PresentKey; 16]>>();

				let swapchain_capture_copies = pending_swapchain_captures
					.iter()
					.filter_map(|capture| {
						let Some(Some((_present_key, _extent, swapchain))) = swapchains.get(capture.sink_id) else {
							return None;
						};

						Some(ghi::ImageBufferCopyDescriptor::swapchain(
							*swapchain,
							capture.destination_buffer,
							capture.destination_offset,
							capture.destination_bytes_per_row,
							capture.destination_bytes_per_image,
						))
					})
					.collect::<SmallVec<[ghi::ImageBufferCopyDescriptor; 16]>>();

				(sinks, pipeline_manager_commands, render_pass_commands, present_keys, swapchain_capture_copies)
			};

			execution.record_with_present_keys(command_buffer, &present_keys, |command_buffer_recording| {
				for commands in pipeline_manager_commands {
					for (command, sink) in commands.into_iter().zip(sinks.iter()) {
						let attachment_infos = render_targets.get_attachment_infos(sink.index());

						(&command)(&mut *command_buffer_recording, &attachment_infos);
					}
				}

				for (command, sink) in render_pass_commands {
					let attachment_infos = render_targets.get_attachment_infos(sink);
					(&command)(&mut *command_buffer_recording, &attachment_infos);
				}

				if !swapchain_capture_copies.is_empty() {
					command_buffer_recording.copy_images_to_buffer(&swapchain_capture_copies);
				}
			});

			present_keys
		});
	}

	pub fn context_mut(&mut self) -> &mut ghi::implementation::Context {
		&mut self.context
	}

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
					ghi::Uses::RenderTarget | ghi::Uses::Storage,
				);

				let sink_id = self.windows.len();

				let sink_has_camera = if let Some(camera) = camera {
					self.sink_cameras.push((sink_id, camera.clone()));
					true
				} else {
					false
				};

				if sink_has_camera {
					let pipeline_managers = self.pipeline_managers.iter_mut();

					for sm in pipeline_managers {
						let mut rpb =
							RenderPassBuilder::new(&mut self.context, &mut self.render_targets, sink_id, swapchain_handle);

						sm.create_sink(sink_id, &mut rpb);

						if rpb.consumed_resources.len() == 0 {
							log::debug!("No resources consumed by scene manager");
						}
					}
				}

				self.windows.push((window, swapchain_handle));

				if sink_has_camera {
					self.add_post_scene_render_passes_for_sink(sink_id);
				}
			}
			Err(msg) => {
				log::error!("Failed to create GHI window: {}", msg);
			}
		}
	}

	pub fn create_camera(&mut self, handle: Handle, camera: Camera) {
		if let Some((_, existing_camera)) = self
			.cameras
			.iter_mut()
			.find(|(existing_handle, _)| *existing_handle == handle)
		{
			*existing_camera = camera;
			return;
		}

		self.cameras.push((handle, camera));
	}
}

struct Attachment {
	name: String,
	image: ghi::BaseImageHandle,
}

/// The `SwapchainCapture` struct exists to defer one swapchain-to-buffer capture request until the next frame.
#[derive(Clone, Copy)]
struct SwapchainCapture {
	sink_id: SinkId,
	destination_buffer: ghi::BaseBufferHandle,
	destination_offset: usize,
	destination_bytes_per_row: usize,
	destination_bytes_per_image: usize,
}

/// This struct holds the settings to configure a `Renderer` during it's creation.
pub struct Settings {
	/// Controls whether validation layers will be enabled or not on the GHI context.
	validation: bool,
	/// Controls whether to enable or not writing out the parameters sent to the underlaying graphics API. Depends on `validation` being enabled.
	api_dump: bool,
	/// Controls wheter to enable or not some extra (bbut expensive) validation for the graphics API. This can include GPU validation. Depends on `validation` being enabled.
	extended_validation: bool,
	/// Controls whether to enable or not mesh shading on the GHI context.
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

pub struct RenderTargets {
	images: Vec<(ghi::BaseImageHandle, ghi::Formats)>,
	/// Maps a sink-scoped name to an image index.
	by_name: Vec<(usize, String, usize)>,
	/// Maps sink indices to image indices and access policies, making attachments.
	by_sink_index: Vec<(usize, (usize, ghi::AccessPolicies))>,
}

impl RenderTargets {
	pub fn new() -> Self {
		Self {
			images: Vec::with_capacity(32),
			by_name: Vec::with_capacity(32),
			by_sink_index: Vec::with_capacity(32),
		}
	}

	pub fn alias(&mut self, sink_id: usize, orig: &str, alias: &str) {
		if let Some(index) = self.get_image_index(orig, sink_id) {
			self.by_name.push((sink_id, alias.to_string(), index));
		}
	}

	/// Inserts a new render target image, associated to a sink index.
	/// Returns the index of the image in the internal storage.
	pub fn insert(&mut self, name: String, sink_id: usize, image: ghi::BaseImageHandle, format: ghi::Formats) -> usize {
		if let Some(_) = self.get_image_index(&name, sink_id) {
			log::debug!("An image by that name already exists");
			panic!("An image by that name already exists");
		};

		if let Some(_) = self.get_attachment_index(&name, sink_id) {
			log::debug!("Attachment is already used in the render pass");
			panic!("Attachment is already used in the render pass");
		}

		let index = self.images.len();
		self.images.push((image, format));
		self.by_name.push((sink_id, name, index));
		self.by_sink_index.push((sink_id, (index, ghi::AccessPolicies::WRITE)));

		index
	}

	pub fn read_from(&mut self, name: &str, sink_id: usize) {
		if let Some(_) = self.get_attachment_index(name, sink_id) {
			log::debug!("Attachment is already used in the render pass");
			return;
		}

		let Some(index) = self.get_image_index(name, sink_id) else {
			log::debug!("An image by that name does not exists");
			return;
		};

		self.by_sink_index.push((sink_id, (index, ghi::AccessPolicies::READ)));
	}

	pub fn write_to(&mut self, name: &str, sink_id: usize) {
		if let Some(_) = self.get_attachment_index(name, sink_id) {
			log::debug!("Attachment is already used in the render pass");
			return;
		}

		let Some(index) = self.get_image_index(name, sink_id) else {
			log::debug!("An image by that name does not exists");
			return;
		};

		self.by_sink_index.push((sink_id, (index, ghi::AccessPolicies::WRITE)));
	}

	pub fn get(&self, name: &str, sink_id: usize) -> Option<&(ghi::BaseImageHandle, ghi::Formats)> {
		self.get_image_index(name, sink_id).and_then(|index| self.images.get(index))
	}

	pub fn get_attachment_infos(&self, sink_id: usize) -> Vec<ghi::AttachmentInformation> {
		let attachments = self
			.by_sink_index
			.iter()
			.filter_map(|(v, (i, ap))| {
				if *v == sink_id {
					let (image, format) = self.images.get(*i)?;
					Some((image, format, ap))
				} else {
					None
				}
			})
			.map(|(image, format, access)| {
				let load = access.intersects(ghi::AccessPolicies::READ);
				let store = access.intersects(ghi::AccessPolicies::WRITE);
				let clear_value = if load {
					ghi::ClearValue::None
				} else {
					ghi::ClearValue::Color(RGBA::black())
				};

				ghi::AttachmentInformation::new(*image, ghi::Layouts::RenderTarget, clear_value, load, store)
				// TODO: contionally pass format
			});

		attachments.collect()
	}

	fn get_image(&self, name: &str, sink_id: usize) -> &ghi::BaseImageHandle {
		let index = self.get_attachment_index(name, sink_id).unwrap();
		&self.images.get(index).unwrap().0
	}

	fn get_image_index(&self, name: &str, sink_id: usize) -> Option<usize> {
		self.by_name
			.iter()
			.rev()
			.find(|(sink, n, _)| *sink == sink_id && n == name)
			.map(|(_, _, i)| *i)
	}

	fn get_attachment_index(&self, name: &str, sink_id: usize) -> Option<usize> {
		let image_index = self.get_image_index(name, sink_id)?;

		self.by_sink_index
			.iter()
			.find_map(|(v, (i, _))| if *v == sink_id && *i == image_index { Some(*i) } else { None })
	}

	fn get_images_for_sink<'a>(&'a self, index: usize) -> impl Iterator<Item = &'a ghi::BaseImageHandle> {
		self.by_sink_index.iter().filter_map(move |(v, (i, _))| {
			if *v != index {
				return None;
			}

			self.images.get(*i).map(|(image, _)| image)
		})
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_render_targets_new() {
		let rt = RenderTargets::new();
		assert!(rt.images.is_empty());
		assert!(rt.by_name.is_empty());
		assert!(rt.by_sink_index.is_empty());
	}

	#[test]
	fn test_insert_and_get() {
		let mut rt = RenderTargets::new();
		let image = unsafe { std::mem::transmute::<u64, ghi::BaseImageHandle>(1) };
		let format = ghi::Formats::RGBA8UNORM;
		let index = rt.insert("test".to_string(), 0, image, format);
		assert_eq!(index, 0);
		let retrieved = rt.get("test", 0);
		assert!(retrieved.is_some());
		assert_eq!(rt.get("nonexistent", 0), None);
	}

	#[test]
	fn test_insert_multiple() {
		let mut rt = RenderTargets::new();
		let image1 = unsafe { std::mem::transmute::<u64, ghi::BaseImageHandle>(1) };
		let format1 = ghi::Formats::RGBA8UNORM;
		let image2 = unsafe { std::mem::transmute::<u64, ghi::BaseImageHandle>(2) };
		let format2 = ghi::Formats::Depth32;

		rt.insert("color".to_string(), 0, image1, format1);
		rt.insert("depth".to_string(), 0, image2, format2);

		assert!(rt.get("color", 0).is_some());
		assert!(rt.get("depth", 0).is_some());
	}

	#[test]
	fn test_get_attachment_infos() {
		let mut rt = RenderTargets::new();
		let image1 = unsafe { std::mem::transmute::<u64, ghi::BaseImageHandle>(1) };
		let format1 = ghi::Formats::RGBA8UNORM;
		let image2 = unsafe { std::mem::transmute::<u64, ghi::BaseImageHandle>(2) };
		let format2 = ghi::Formats::Depth32;

		rt.insert("color".to_string(), 0, image1, format1);
		rt.insert("depth".to_string(), 0, image2, format2);
		rt.insert(
			"other".to_string(),
			1,
			unsafe { std::mem::transmute::<u64, ghi::BaseImageHandle>(3) },
			ghi::Formats::RGBA16UNORM,
		);

		let attachments = rt.get_attachment_infos(0);
		assert_eq!(attachments.len(), 2);

		let attachments_view1 = rt.get_attachment_infos(1);
		assert_eq!(attachments_view1.len(), 1);
	}

	#[test]
	fn test_get_attachment_infos_empty_view() {
		let rt = RenderTargets::new();
		let attachments = rt.get_attachment_infos(0);
		assert!(attachments.is_empty());
	}

	#[test]
	fn test_alias_overrides_previous_mapping() {
		let mut rt = RenderTargets::new();
		let first_image = unsafe { std::mem::transmute::<u64, ghi::BaseImageHandle>(1) };
		let second_image = unsafe { std::mem::transmute::<u64, ghi::BaseImageHandle>(2) };

		rt.insert("first".to_string(), 0, first_image, ghi::Formats::RGBA16UNORM);
		rt.insert("second".to_string(), 0, second_image, ghi::Formats::RGBA16UNORM);
		rt.alias(0, "first", "main");
		rt.alias(0, "second", "main");

		let (image, _) = rt.get("main", 0).expect("main alias should resolve");
		assert_eq!(*image, second_image);
	}

	#[test]
	fn test_insert_same_name_for_different_sinks() {
		let mut rt = RenderTargets::new();
		let image1 = unsafe { std::mem::transmute::<u64, ghi::BaseImageHandle>(1) };
		let image2 = unsafe { std::mem::transmute::<u64, ghi::BaseImageHandle>(2) };

		rt.insert("main".to_string(), 0, image1, ghi::Formats::RGBA16UNORM);
		rt.insert("main".to_string(), 1, image2, ghi::Formats::RGBA16UNORM);

		let (sink0_image, _) = rt.get("main", 0).expect("sink 0 main should resolve");
		let (sink1_image, _) = rt.get("main", 1).expect("sink 1 main should resolve");
		assert_eq!(*sink0_image, image1);
		assert_eq!(*sink1_image, image2);
	}
}

type RenderPassFactory = dyn for<'a> Fn(&'a mut RenderPassBuilder<'a>) -> Box<dyn RenderPass>;

type SinkId = usize;
/// A `RenderPass` represents a specific rendering task that can be performed on the scene, defined by a render pass factory.
type RenderPassId = usize;

use std::{
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
use math::direction_from_orientation;
use resource_management::resource::resource_manager::ResourceManager;
use smallvec::SmallVec;
use utils::Box;
use utils::{
	hash::{HashMap, HashMapExt},
	sync::RwLock,
	Extent, RGBA,
};

use super::render_pass::{RenderPass, RenderPassBuilder};
use crate::{
	application::parameters::Parameters,
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
		render_pass::{FramePrepare, RenderPassFunction, RenderPassReturn},
		window::{self, Window},
		Camera, Sink, View,
	},
	space::{Orientable as _, Positionable as _},
};
