use core::entity::DomainType;
use std::{io::Write, ops::{Deref, DerefMut}, rc::Rc, sync::Arc};

use ghi::{GraphicsHardwareInterface, CommandBufferRecording, BoundComputePipelineMode};
use resource_management::resource::resource_manager::ResourceManager;
use utils::{sync::RwLock, Extent};

use crate::{core::{self, entity::EntityBuilder, listener::{EntitySubscriber, Listener}, orchestrator, Entity, EntityHandle}, ui::render_model::UIRenderModel, utils, window_system::{self, WindowSystem},};

use super::{aces_tonemap_render_pass::AcesToneMapPass, shadow_render_pass::ShadowRenderingPass, ssao_render_pass::ScreenSpaceAmbientOcclusionPass, texture_manager::TextureManager, tonemap_render_pass::ToneMapRenderPass, visibility_model::render_domain::VisibilityWorldRenderDomain, world_render_domain::WorldRenderDomain};

pub struct Renderer {
	ghi: Rc<RwLock<ghi::GHI>>,

	rendered_frame_count: usize,
	frame_queue_depth: usize,

	swapchain_handles: Vec<ghi::SwapchainHandle>,

	render_command_buffer: ghi::CommandBufferHandle,
	render_finished_synchronizer: ghi::SynchronizerHandle,
	image_ready: ghi::SynchronizerHandle,

	result: ghi::ImageHandle,

	window_system: EntityHandle<window_system::WindowSystem>,

	visibility_render_model: EntityHandle<VisibilityWorldRenderDomain>,
	ao_render_pass: EntityHandle<ScreenSpaceAmbientOcclusionPass>,
	ui_render_model: EntityHandle<UIRenderModel>,
	tonemap_render_model: EntityHandle<AcesToneMapPass>,

	extent: Extent,
}

impl Renderer {
	pub fn new_as_system<'a>(window_system_handle: EntityHandle<WindowSystem>, resource_manager_handle: EntityHandle<ResourceManager>) -> EntityBuilder<'a, Self> {
		EntityBuilder::new_from_async_function_with_parent(async move |parent: DomainType| {
			let enable_validation = std::env::vars().find(|(k, _)| k == "BE_RENDER_DEBUG").is_some() || true;

			let ghi_instance = Rc::new(RwLock::new(ghi::create(ghi::Features::new().validation(enable_validation).api_dump(false).gpu_validation(false).debug_log_function(|message| {
				log::error!("{}", message);
			}))));

			let extent = Extent::square(0); // Initialize extent to 0 to allocate memory lazily.

			let result = {
				let mut ghi = ghi_instance.write();

				ghi.create_image(Some("result"), extent, ghi::Formats::RGBA8(ghi::Encodings::UnsignedNormalized), ghi::Uses::Storage | ghi::Uses::TransferDestination, ghi::DeviceAccesses::GpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::DYNAMIC)
			};

			let texture_manager = Arc::new(utils::r#async::RwLock::new(TextureManager::new()));

			let visibility_render_model: EntityHandle<VisibilityWorldRenderDomain> = core::spawn_as_child(parent.clone(), VisibilityWorldRenderDomain::new(ghi_instance.clone(), resource_manager_handle.clone(), texture_manager.clone())).await;

			let ui_render_model = core::spawn(UIRenderModel::new_as_system()).await;

			let render_command_buffer;
			let render_finished_synchronizer;
			let image_ready;
			let tonemap_render_model;

			{
				let mut ghi = ghi_instance.write();

				{
					let result_image = visibility_render_model.map(|e| { let e = e.read_sync(); e.get_result_image() });
					tonemap_render_model = core::spawn(AcesToneMapPass::new_as_system(ghi.deref_mut(), result_image, result)).await;
				}

				render_command_buffer = ghi.create_command_buffer(Some("Render"));
				render_finished_synchronizer = ghi.create_synchronizer(Some("Render Finisished"), true);
				image_ready = ghi.create_synchronizer(Some("Swapchain Available"), false);
			}

			let ao_render_pass = {
				let vrm = visibility_render_model.read_sync();
				core::spawn(ScreenSpaceAmbientOcclusionPass::new(ghi_instance.clone(), resource_manager_handle, texture_manager.clone(), vrm.get_descriptor_set_template(), vrm.get_view_occlusion_image(), vrm.get_view_depth_image()).await).await
			};

			Renderer {
				ghi: ghi_instance,

				rendered_frame_count: 0,
				frame_queue_depth: 2,

				swapchain_handles: vec![],

				render_command_buffer,
				render_finished_synchronizer,
				image_ready,

				result,

				window_system: window_system_handle,

				ao_render_pass,

				visibility_render_model,
				ui_render_model,
				tonemap_render_model,

				extent,
			}
		}).listen_to::<window_system::Window>()
	}

	pub fn render(&mut self,) {
		if self.swapchain_handles.is_empty() { return; }

		let modulo_frame_index = (self.rendered_frame_count % self.frame_queue_depth) as u32;

		let mut ghi = self.ghi.write();

		let swapchain_handle = self.swapchain_handles[0];

		ghi.wait(modulo_frame_index, self.render_finished_synchronizer);

		ghi.start_frame_capture();

		let (present_key, extent) = ghi.acquire_swapchain_image(modulo_frame_index, swapchain_handle, self.image_ready);

		assert!(extent.width() <= 65535 && extent.height() <= 65535, "The extent is too large: {:?}. The renderer only supports dimensions as big as 16 bits.", extent);

		drop(ghi);

		if extent != self.extent {
			{
				let mut ghi = self.ghi.write();
				ghi.resize_image(self.result, extent);
			}

			self.visibility_render_model.sync_get_mut(|e| {
				e.resize(extent);
			});

			self.ao_render_pass.sync_get_mut(|e| {
				let mut ghi = self.ghi.write();
				e.resize(ghi.deref_mut(), extent);
			});

			self.tonemap_render_model.sync_get_mut(|e| {
				e.resize(extent);
			});

			self.extent = extent;
		}

		let mut ghi = self.ghi.write();

		self.visibility_render_model.sync_get_mut(|vis_rp| {
			if let Some(_) = vis_rp.prepare(&mut ghi, extent, modulo_frame_index) {

			}
		});

		let mut command_buffer_recording = ghi.create_command_buffer_recording(self.render_command_buffer, Some(self.rendered_frame_count as u32));

		self.visibility_render_model.sync_get_mut(|vis_rp| {
			if let Some(_) = vis_rp.render_a(&mut command_buffer_recording, extent, modulo_frame_index) {
				self.ao_render_pass.map(|ao_rp| {
					let ao_rp = ao_rp.write_sync();
					ao_rp.render(&mut command_buffer_recording, extent);
				});

				vis_rp.render_b(&mut command_buffer_recording);
			}
		});

		self.tonemap_render_model.map(|e| {
			let e = e.read_sync();
			e.render(&mut command_buffer_recording, extent);
		});

		// Copy to swapchain

		command_buffer_recording.copy_to_swapchain(self.result, present_key, swapchain_handle);

		command_buffer_recording.execute(&[self.image_ready,], &[self.render_finished_synchronizer], self.render_finished_synchronizer);

		ghi.end_frame_capture();

		ghi.present(modulo_frame_index, present_key, &[swapchain_handle], self.render_finished_synchronizer);

		self.rendered_frame_count += 1;
	}

	pub fn resize(&mut self, extent: Extent) {
		self.extent = extent;

		self.visibility_render_model.sync_get_mut(|e| {
			e.resize(extent);
		});

		self.tonemap_render_model.sync_get_mut(|e| {
			e.resize(extent);
		});
	}
}

impl EntitySubscriber<window_system::Window> for Renderer {
	fn on_create<'a>(&'a mut self, handle: EntityHandle<window_system::Window>, window: &window_system::Window) -> utils::BoxedFuture<()> {
		let os_handles = self.window_system.map(|e| {
			let e = e.read_sync();
			e.get_os_handles(&handle)
		});

		let mut ghi = self.ghi.write();

		let swapchain_handle = ghi.bind_to_window(&os_handles, ghi::PresentationModes::FIFO);

		self.swapchain_handles.push(swapchain_handle);

		Box::pin(async move {})
	}
}

impl Entity for Renderer {}
