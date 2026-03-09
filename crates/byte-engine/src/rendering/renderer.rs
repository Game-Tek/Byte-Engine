use std::{
	borrow::BorrowMut,
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
	device::{Device as _, DeviceCreate as _},
	frame::Frame as _,
	queue::Queue as _,
	raster_pipeline,
	vulkan::command_buffer,
};
use math::direction_from_orientation;
use resource_management::resource::resource_manager::ResourceManager;
use smallvec::SmallVec;
use utils::{
	hash::{HashMap, HashMapExt},
	sync::RwLock,
	Extent, RGBA,
};

use crate::{
	application::parameters::Parameters,
	camera::Camera,
	core::{
		channel::{Channel, DefaultChannel},
		factory::Handle,
		listener::Listener,
		Entity, EntityHandle,
	},
	gameplay::{transform::TransformationUpdate, Positionable},
	rendering::{
		make_perspective_view_from_camera,
		render_pass::{FramePrepare, RenderPassFunction, RenderPassReturn},
		scene_manager::SceneManager,
		viewport,
		window::Window,
		View, Viewport,
	},
};

use super::{
	render_pass::{RenderPass, RenderPassBuilder},
	texture_manager::TextureManager,
};

use utils::Box;

type RenderPassFactory = dyn for<'a> Fn(&'a mut RenderPassBuilder<'a>) -> Box<dyn RenderPass>;

/// A `Viewport` represents a specific way of looking at the scene, defined by a window.
type ViewportId = usize;
/// A `View` represents a specific way of looking at the scene, defined by a camera.
type ViewId = usize;
/// A `RenderPass` represents a specific rendering task that can be performed on the scene, defined by a render pass factory.
type RenderPassId = usize;

/// The `Renderer` class centralizes the management of the rendering tasks and state.
/// It manages the creation of a Graphics Hardware Interfacec device and orchestrates render passes.
pub struct Renderer {
	device: ghi::implementation::Device, // Place device before instance to ensure proper drop order
	instance: ghi::implementation::Instance,

	started_frame_count: usize,

	frame_queue_depth: usize,

	/// A list of display windows and their associated swapchains.
	windows: SmallVec<[(ghi::Window, ghi::SwapchainHandle); 16]>,
	/// A list of windows (idx) and their associated cameras (Handle).
	views: SmallVec<[(ViewportId, Handle); 16]>,
	/// A list of cameras and their associated handles.
	cameras: SmallVec<[(Handle, Camera); 16]>,

	render_targets: RenderTargets,

	render_passes: SmallVec<[Box<dyn RenderPass>; 64]>,
	render_passes_by_viewport: SmallVec<[(RenderPassId, ViewportId); 32]>,
	post_scene_render_pass_factories: SmallVec<[Box<RenderPassFactory>; 16]>,

	scene_managers: SmallVec<[EntityHandle<dyn SceneManager>; 16]>,

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

		let features = ghi::device::Features::new()
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

		let mut instance = ghi::implementation::Instance::new(features.clone()).unwrap();

		let mut queue_handle = None;

		let mut device = instance
			.create_device(
				features.clone(),
				&mut [(ghi::QueueSelection::new(ghi::CommandBufferType::GRAPHICS), &mut queue_handle)],
			)
			.unwrap();

		let queue_handle = queue_handle.unwrap();

		let render_command_buffer = device.create_command_buffer(Some("Render"), queue_handle);
		let render_finished_synchronizer = device.create_synchronizer(Some("Render Finisished"), true);

		let texture_manager = Arc::new(RwLock::new(TextureManager::new()));

		Renderer {
			instance,
			device,

			started_frame_count: 0,

			frame_queue_depth: 2,

			windows: SmallVec::with_capacity(16),
			views: SmallVec::with_capacity(16),
			cameras: SmallVec::with_capacity(16),

			render_targets: RenderTargets::new(),

			render_passes: SmallVec::with_capacity(64),
			render_passes_by_viewport: SmallVec::with_capacity(32),
			post_scene_render_pass_factories: SmallVec::with_capacity(16),

			scene_managers: SmallVec::with_capacity(8),

			render_command_buffer,
			render_finished_synchronizer,
		}
	}

	pub fn add_scene_manager(&mut self, handle: EntityHandle<dyn SceneManager>) {
		{
			for (view_id, _) in self.windows.iter().enumerate() {
				let mut rpb = RenderPassBuilder::new(&mut self.device, &mut self.render_targets, view_id);

				scene_manager.create_view(view_id, &mut rpb);

				if rpb.consumed_resources.len() == 0 {
					log::debug!("No resources consumed by scene manager");
				}
			}
		}

		self.scene_managers.push(handle);
	}

	fn add_render_pass(&mut self, render_pass: Box<dyn RenderPass>, viewport_id: ViewportId) {
		let render_pass_id = self.render_passes.len();
		self.render_passes.push(render_pass);
		self.render_passes_by_viewport.push((render_pass_id, viewport_id));
	}

	/// Registers a render pass factory that will be instantiated for every current and future view.
	pub fn add_post_scene_render_pass_for_all_views<F>(&mut self, render_pass_factory: F)
	where
		F: for<'a> Fn(&'a mut RenderPassBuilder<'a>) -> Box<dyn RenderPass> + 'static,
	{
		let render_pass_factory: Box<RenderPassFactory> = Box::new(render_pass_factory);
		let view_ids: SmallVec<[usize; 16]> = self.views.iter().map(|(view_id, _)| *view_id).collect();

		for view_id in view_ids {
			let render_pass = {
				let mut render_pass_builder = RenderPassBuilder::new(&mut self.device, &mut self.render_targets, view_id);
				render_pass_factory(&mut render_pass_builder)
			};

			self.add_render_pass(render_pass, view_id);
		}

		self.post_scene_render_pass_factories.push(render_pass_factory);
	}

	/// Instantiates all registered post-scene render pass factories for a given view.
	fn add_post_scene_render_passes_for_viewport(&mut self, viewport_id: ViewportId) {
		let mut render_passes_for_view: SmallVec<[Box<dyn RenderPass>; 16]> = SmallVec::new();

		for render_pass_factory in &self.post_scene_render_pass_factories {
			let render_pass = {
				let mut render_pass_builder = RenderPassBuilder::new(&mut self.device, &mut self.render_targets, viewport_id);
				render_pass_factory(&mut render_pass_builder)
			};

			render_passes_for_view.push(render_pass);
		}

		for render_pass in render_passes_for_view {
			self.add_render_pass(render_pass, viewport_id);
		}
	}

	pub fn update_windows<'a>(&'a mut self) -> impl Iterator<Item = impl Iterator<Item = ghi::Events> + 'a> + 'a {
		self.windows.iter_mut().map(|(window, _)| window.poll())
	}

	/// This function prepares a frame by invoking multiple render passes.
	/// If no swapchains are available no rendering/execution will be performed.
	/// If some swapchain surface is 0 sized along some dimension no rendering/execution will be performed.
	pub fn prepare(&'_ mut self, transforms_listener: &mut impl Listener<TransformationUpdate>) {
		let Some(_) = self.windows.first() else {
			log::debug!("No swapchains available to present to. Skipping rendering!");
			return;
		};

		let device = &mut self.device;

		device.start_frame_capture();

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

		let mut frame = device.start_frame(self.started_frame_count as u32, self.render_finished_synchronizer);

		self.started_frame_count += 1;

		let swapchains: SmallVec<[Option<(ghi::PresentKey, Extent, ghi::SwapchainHandle)>; 16]> = self
			.windows
			.iter()
			.map(|(window, swapchain)| {
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

		let views = self.views.iter();

		let viewports: SmallVec<[Viewport; 16]> = views
			.filter_map(|(index, view)| {
				let (present_key, extent, swapchain) = swapchains[*index]?;

				let camera = self
					.cameras
					.iter()
					.find_map(|(handle, camera)| if handle == view { Some(camera) } else { None })?;

				let view = make_perspective_view_from_camera(&camera, extent);

				let viewport = Viewport::new(view, extent, *index);

				Some(viewport)
			})
			.collect();

		for viewport in &viewports {
			// Get images for the current view and render pass and resize them to window extent
			let images = self.render_targets.get_images_for_view(viewport.index());

			// Resize images to window extent
			for &image in images {
				frame.resize_image(image, viewport.extent());
			}
		}

		let scene_managers = self.scene_managers.iter_mut();

		let scene_manager_commands: SmallVec<[Vec<Box<dyn RenderPassFunction>>; 16]> = scene_managers
			.filter_map(|sm| {
				let mut sm = sm.write();
				sm.prepare(&mut frame, &viewports)
			})
			.collect();

		// A list of render pass commands and their corresponding viewport index
		let render_pass_commands: SmallVec<[(RenderPassReturn, ViewportId); 64]> = self
			.render_passes_by_viewport
			.iter_mut()
			.filter_map(|(render_pass_id, viewport_id)| {
				if let Some(render_pass) = self.render_passes.get_mut(*render_pass_id) {
					if let Some(viewport) = viewports.iter().find(|vp| vp.index() == *viewport_id) {
						if let Some(command) = render_pass.prepare(&mut frame, viewport) {
							return Some((command, viewport.index()));
						}
					}
				}
				None
			})
			.collect();

		let swapchain_targets: SmallVec<[(ViewportId, ghi::PresentKey, ghi::SwapchainHandle); 16]> = viewports
			.iter()
			.filter_map(|viewport| {
				let (present_key, _, swapchain_handle) = swapchains.get(viewport.index())?.as_ref()?;
				Some((viewport.index(), *present_key, *swapchain_handle))
			})
			.collect();

		let present_keys = swapchains
			.iter()
			.filter_map(|sc| sc.as_ref().map(|(pk, ..)| *pk))
			.collect::<SmallVec<[ghi::PresentKey; 16]>>();

		let execute = {
			let viewports = &viewports;
			let render_targets = &self.render_targets;
			let swapchain_targets = &swapchain_targets;

			move |e| {
				for commands in scene_manager_commands.into_iter() {
					for (mut command, viewport) in commands.iter().zip(viewports.iter()) {
						let attachment_infos = render_targets.get_attachment_infos(viewport.index());

						(command.borrow_mut())(e, &attachment_infos);
					}
				}

				for (mut command, viewport) in render_pass_commands.into_iter() {
					let attachment_infos = render_targets.get_attachment_infos(viewport);
					(command.borrow_mut())(e, &attachment_infos);
				}

				for (view_id, present_key, swapchain_handle) in swapchain_targets.iter().copied() {
					let source = render_targets.get_image("result", view_id);
					e.copy_to_swapchain(*source, present_key, swapchain_handle);
				}
			}
		};

		let command_buffer = self.render_command_buffer;
		let synchronizer = self.render_finished_synchronizer;

		let mut command_buffer_recording = frame.create_command_buffer_recording(command_buffer);
		execute(&mut command_buffer_recording);

		let command_buffer = command_buffer_recording.end(&present_keys);
		frame.execute(command_buffer, synchronizer);
	}

	pub fn device_mut(&mut self) -> &mut ghi::implementation::Device {
		&mut self.device
	}

	pub fn create_window(&mut self, window: Window) {
		let name = window.name();
		let extent = window.extent();
		let camera = window.camera();

		let window = ghi::Window::new_with_params(name, extent, "main_window");

		match window {
			Ok(window) => {
				let os_handles = window.os_handles();

				let device = &mut self.device;

				let swapchain_handle =
					device.bind_to_window(&os_handles, ghi::PresentationModes::FIFO, extent, ghi::Uses::BlitDestination);

				let viewport_id = self.windows.len();

				let view_id = if let Some(camera) = camera {
					let view_id = self.views.len();
					self.views.push((view_id, camera.clone()));
					Some(view_id)
				} else {
					None
				};

				let main = device.build_image(
					ghi::image::Builder::new(
						ghi::Formats::RGBA16F,
						ghi::Uses::Storage
							| ghi::Uses::TransferSource
							| ghi::Uses::BlitDestination
							| ghi::Uses::RenderTarget
							| ghi::Uses::InputAttachment,
					)
					.name("main")
					.use_case(ghi::UseCases::DYNAMIC),
				);

				let result = device.build_image(
					ghi::image::Builder::new(ghi::Formats::RGBA8UNORM, ghi::Uses::Storage | ghi::Uses::BlitSource)
						.name("result")
						.use_case(ghi::UseCases::DYNAMIC),
				);

				self.render_targets
					.insert("main".to_string(), viewport_id, main, ghi::Formats::RGBA16F);

				self.render_targets
					.insert("result".to_string(), viewport_id, result, ghi::Formats::RGBA8UNORM);

				if let Some(view_id) = view_id {
					let depth = device.build_image(
						ghi::image::Builder::new(ghi::Formats::Depth32, ghi::Uses::RenderTarget | ghi::Uses::Image)
							.name("depth")
							.use_case(ghi::UseCases::DYNAMIC),
					);

					self.render_targets
						.insert("depth".to_string(), viewport_id, depth, ghi::Formats::Depth32);

					{
						let scene_managers = self.scene_managers.iter();

						for sm in scene_managers {
							let mut rpb = RenderPassBuilder::new(&mut self.device, &mut self.render_targets, viewport_id);

							sm.write().create_view(view_id, &mut rpb);

							if rpb.consumed_resources.len() == 0 {
								log::debug!("No resources consumed by scene manager");
							}
						}
					}
				}

				self.add_post_scene_render_passes_for_viewport(viewport_id);

				self.windows.push((window, swapchain_handle));
			}
			Err(msg) => {
				log::error!("Failed to create GHI window: {}", msg);
			}
		}
	}

	pub fn create_camera(&mut self, handle: Handle, camera: Camera) {
		self.cameras.push((handle, camera));
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

struct CopyToSwapchainRenderPass {
	source_texture_handle: ghi::ImageHandle,
	present_key: ghi::PresentKey,
	swapchain_handle: ghi::SwapchainHandle,
}

impl RenderPass for CopyToSwapchainRenderPass {
	fn prepare(&mut self, frame: &mut ghi::implementation::Frame, viewport: &Viewport) -> Option<RenderPassReturn> {
		let view_id = viewport.index();

		let source_texture_handle = self.source_texture_handle;
		let present_key = self.present_key;
		let swapchain_handle = self.swapchain_handle;

		let command = Box::new(
			move |e: &mut ghi::implementation::CommandBufferRecording, a: &[ghi::AttachmentInformation]| {
				e.copy_to_swapchain(source_texture_handle, present_key, swapchain_handle);
			},
		);

		Some(command)
	}
}

pub struct RenderTargets {
	images: Vec<(ghi::ImageHandle, ghi::Formats)>,
	/// Maps names to image indices.
	by_name: Vec<(String, usize)>,
	/// Maps view indices to image indices and access policies, making attachments.
	by_view_index: Vec<(usize, (usize, ghi::AccessPolicies))>,
}

impl RenderTargets {
	pub fn new() -> Self {
		Self {
			images: Vec::with_capacity(32),
			by_name: Vec::with_capacity(32),
			by_view_index: Vec::with_capacity(32),
		}
	}

	/// Inserts a new render target image, associated to a view index.
	/// Returns the index of the image in the internal storage.
	pub fn insert(&mut self, name: String, view: usize, image: ghi::ImageHandle, format: ghi::Formats) -> usize {
		if let Some(_) = self.get_image_index(&name) {
			log::debug!("An image by that name already exists");
			panic!("An image by that name already exists");
		};

		if let Some(_) = self.get_attachment_index(&name, view) {
			log::debug!("Attachment is already used in the render pass");
			panic!("Attachment is already used in the render pass");
		}

		let index = self.images.len();
		self.images.push((image, format));
		self.by_name.push((name, index));
		self.by_view_index.push((view, (index, ghi::AccessPolicies::WRITE)));

		index
	}

	pub fn read_from(&mut self, name: &str, view_id: usize) {
		if let Some(_) = self.get_attachment_index(name, view_id) {
			log::debug!("Attachment is already used in the render pass");
			return;
		}

		let Some(index) = self.get_image_index(name) else {
			log::debug!("An image by that name does not exists");
			return;
		};

		self.by_view_index.push((view_id, (index, ghi::AccessPolicies::READ)));
	}

	pub fn write_to(&mut self, name: &str, view_id: usize) {
		if let Some(_) = self.get_attachment_index(name, view_id) {
			log::debug!("Attachment is already used in the render pass");
			return;
		}

		let Some(index) = self.get_image_index(name) else {
			log::debug!("An image by that name does not exists");
			return;
		};

		self.by_view_index.push((view_id, (index, ghi::AccessPolicies::WRITE)));
	}

	pub fn get(&self, name: &str) -> Option<&(ghi::ImageHandle, ghi::Formats)> {
		if let Some((_, index)) = self.by_name.iter().find(|(n, _)| n == name) {
			self.images.get(*index)
		} else {
			None
		}
	}

	pub fn get_attachment_infos(&self, view: usize) -> Vec<ghi::AttachmentInformation> {
		let result_index = self
			.by_name
			.iter()
			.find(|(name, _)| name == "result")
			.map(|(_, index)| *index);

		let attachments = self
			.by_view_index
			.iter()
			.filter_map(|(v, (i, ap))| {
				let is_result_attachment = result_index == Some(*i);

				if *v == view && !is_result_attachment {
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

				ghi::AttachmentInformation::new(*image, *format, ghi::Layouts::RenderTarget, clear_value, load, store)
			});

		attachments.collect()
	}

	fn get_image(&self, name: &str, view_id: usize) -> &ghi::ImageHandle {
		let index = self.get_attachment_index(name, view_id).unwrap();
		&self.images.get(index).unwrap().0
	}

	fn get_image_index(&self, name: &str) -> Option<usize> {
		self.by_name.iter().find(|(n, _)| n == name).map(|(_, i)| *i)
	}

	fn get_attachment_index(&self, name: &str, view_id: usize) -> Option<usize> {
		let image_index = self.get_image_index(name)?;

		self.by_view_index
			.iter()
			.find_map(|(v, (i, _))| if *v == view_id && *i == image_index { Some(*i) } else { None })
	}

	fn get_images_for_view<'a>(&'a self, index: usize) -> impl Iterator<Item = &'a ghi::ImageHandle> {
		self.by_view_index.iter().filter_map(move |(v, (i, _))| {
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
		assert!(rt.by_view_index.is_empty());
	}

	#[test]
	fn test_insert_and_get() {
		let mut rt = RenderTargets::new();
		let image = unsafe { std::mem::transmute::<u64, ghi::ImageHandle>(1) };
		let format = ghi::Formats::RGBA8UNORM;
		let index = rt.insert("test".to_string(), 0, image, format);
		assert_eq!(index, 0);
		let retrieved = rt.get("test");
		assert!(retrieved.is_some());
		assert_eq!(rt.get("nonexistent"), None);
	}

	#[test]
	fn test_insert_multiple() {
		let mut rt = RenderTargets::new();
		let image1 = unsafe { std::mem::transmute::<u64, ghi::ImageHandle>(1) };
		let format1 = ghi::Formats::RGBA8UNORM;
		let image2 = unsafe { std::mem::transmute::<u64, ghi::ImageHandle>(2) };
		let format2 = ghi::Formats::Depth32;

		rt.insert("color".to_string(), 0, image1, format1);
		rt.insert("depth".to_string(), 0, image2, format2);

		assert!(rt.get("color").is_some());
		assert!(rt.get("depth").is_some());
	}

	#[test]
	fn test_get_attachment_infos() {
		let mut rt = RenderTargets::new();
		let image1 = unsafe { std::mem::transmute::<u64, ghi::ImageHandle>(1) };
		let format1 = ghi::Formats::RGBA8UNORM;
		let image2 = unsafe { std::mem::transmute::<u64, ghi::ImageHandle>(2) };
		let format2 = ghi::Formats::Depth32;

		rt.insert("color".to_string(), 0, image1, format1);
		rt.insert("depth".to_string(), 0, image2, format2);
		rt.insert(
			"other".to_string(),
			1,
			unsafe { std::mem::transmute::<u64, ghi::ImageHandle>(3) },
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
}
