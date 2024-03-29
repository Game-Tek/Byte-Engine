use std::{ops::{DerefMut, Deref}, rc::Rc, sync::RwLock};

use ghi::GraphicsHardwareInterface;
use resource_management::resource::resource_manager::ResourceManager;
use utils::Extent;

use crate::{core::{self, entity::EntityBuilder, listener::{EntitySubscriber, Listener}, orchestrator, Entity, EntityHandle}, ui::render_model::UIRenderModel, utils, window_system::{self, WindowSystem},};

use super::{aces_tonemap_render_pass::AcesToneMapPass, shadow_render_pass::ShadowRenderingPass, ssao_render_pass::ScreenSpaceAmbientOcclusionPass, tonemap_render_pass::ToneMapRenderPass, visibility_model::render_domain::VisibilityWorldRenderDomain, world_render_domain::WorldRenderDomain};

pub struct Renderer {
	ghi: Rc<RwLock<dyn ghi::GraphicsHardwareInterface>>,

	rendered_frame_count: usize,

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
}

impl Renderer {
	pub fn new_as_system<'a>(window_system_handle: EntityHandle<WindowSystem>, resource_manager_handle: EntityHandle<ResourceManager>) -> EntityBuilder<'a, Self> {
		EntityBuilder::new_from_closure_with_parent(move |parent| {
			let ghi_instance = Rc::new(RwLock::new(ghi::create()));

			let result = {
				let mut ghi = ghi_instance.write().unwrap();

				ghi.create_image(Some("result"), Extent::rectangle(1920, 1080), ghi::Formats::RGBA8(ghi::Encodings::UnsignedNormalized), None, ghi::Uses::Storage | ghi::Uses::TransferDestination, ghi::DeviceAccesses::GpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::DYNAMIC)
			};

			let visibility_render_model: EntityHandle<VisibilityWorldRenderDomain> = core::spawn_as_child(parent.clone(), VisibilityWorldRenderDomain::new(ghi_instance.clone(), resource_manager_handle));

			let ui_render_model = core::spawn(UIRenderModel::new_as_system());
			
			let render_command_buffer;
			let render_finished_synchronizer;
			let image_ready;
			let tonemap_render_model;

			{
				let mut ghi = ghi_instance.write().unwrap();

				{
					let result_image = visibility_render_model.map(|e| { let e = e.read_sync(); e.get_result_image() });
					tonemap_render_model = core::spawn(AcesToneMapPass::new_as_system(ghi.deref_mut(), result_image, result));
				}

				render_command_buffer = ghi.create_command_buffer(Some("Render"));
				render_finished_synchronizer = ghi.create_synchronizer(Some("Render Finisished"), true);
				image_ready = ghi.create_synchronizer(Some("Swapchain Available"), false);
			}

			let ao_render_pass = {
				let mut ghi = ghi_instance.write().unwrap();
				let vrm = visibility_render_model.read_sync();
				core::spawn(ScreenSpaceAmbientOcclusionPass::new(ghi.deref_mut(), vrm.get_descriptor_set_template(), vrm.get_view_occlusion_image(), vrm.get_view_depth_image()))
			};

			Renderer {
				ghi: ghi_instance,

				rendered_frame_count: 0,

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
			}
		}).listen_to::<window_system::Window>()
	}

	pub fn render(&mut self,) {
		if self.swapchain_handles.is_empty() { return; }

		let ghi = self.ghi.write().unwrap();

		let swapchain_handle = self.swapchain_handles[0];

		ghi.wait(self.render_finished_synchronizer);

		ghi.start_frame_capture();

		let image_index = ghi.acquire_swapchain_image(swapchain_handle, self.image_ready);

		let mut command_buffer_recording = ghi.create_command_buffer_recording(self.render_command_buffer, Some(self.rendered_frame_count as u32));

		self.visibility_render_model.map(|vis_rp| {
			let mut vis_rp = vis_rp.write_sync();
			vis_rp.render_a(ghi.deref(), command_buffer_recording.as_mut());

			self.ao_render_pass.map(|ao_rp| { // BUG: if visibility_render_model is not used, this will trigger an error, TODO: disable ao_render_pass if visibility_render_model is not used
				let ao_rp = ao_rp.write_sync();
				ao_rp.render(command_buffer_recording.as_mut());
			});

			vis_rp.render_b(ghi.deref(), command_buffer_recording.as_mut());
		});

		self.tonemap_render_model.map(|e| {
			let e = e.read_sync();
			e.render(command_buffer_recording.as_mut());
		});			

		// Copy to swapchain

		command_buffer_recording.copy_to_swapchain(self.result, image_index, swapchain_handle);

		command_buffer_recording.execute(&[self.image_ready], &[self.render_finished_synchronizer], self.render_finished_synchronizer);

		ghi.end_frame_capture();

		ghi.present(image_index, &[swapchain_handle], self.render_finished_synchronizer);

		self.rendered_frame_count += 1;
	}
}

impl EntitySubscriber<window_system::Window> for Renderer {
	fn on_create<'a>(&'a mut self, handle: EntityHandle<window_system::Window>, window: &window_system::Window) -> utils::BoxedFuture<()> {
		let os_handles = self.window_system.map(|e| {
			let e = e.read_sync();
			e.get_os_handles(&handle)
		});

		let mut ghi = self.ghi.write().unwrap();

		let swapchain_handle = ghi.bind_to_window(&os_handles);

		self.swapchain_handles.push(swapchain_handle);

		Box::pin(async move {})
	}
}

impl Entity for Renderer {}