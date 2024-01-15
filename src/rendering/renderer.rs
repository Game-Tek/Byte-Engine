use std::{ops::{DerefMut, Deref}, rc::Rc, sync::RwLock};

use crate::{core::{self, orchestrator::{self,}, Entity, EntityHandle, listener::{Listener, EntitySubscriber}, entity::EntityBuilder}, window_system::{self, WindowSystem}, Extent, resource_management::resource_manager::ResourceManager, ghi::{self, GraphicsHardwareInterface}, ui::render_model::UIRenderModel};

use super::{visibility_model::render_domain::VisibilityWorldRenderDomain, aces_tonemap_render_pass::AcesToneMapPass, tonemap_render_pass::ToneMapRenderPass, world_render_domain::WorldRenderDomain};

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
	ui_render_model: EntityHandle<UIRenderModel>,
	tonemap_render_model: EntityHandle<AcesToneMapPass>,
}

impl Renderer {
	pub fn new_as_system<'a>(listener: &'a mut impl Listener, window_system_handle: EntityHandle<WindowSystem>, resource_manager_handle: EntityHandle<ResourceManager>) -> EntityBuilder<'a, Self> {
		EntityBuilder::new_from_function(|| {
			let ghi_instance = Rc::new(RwLock::new(ghi::create()));

			let result = {
				let mut ghi = ghi_instance.write().unwrap();

				ghi.create_image(Some("result"), Extent::plane(1920, 1080), ghi::Formats::RGBAu8, None, ghi::Uses::Storage | ghi::Uses::TransferDestination, ghi::DeviceAccesses::GpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::DYNAMIC)
			};

			let visibility_render_model: EntityHandle<VisibilityWorldRenderDomain> = core::spawn(VisibilityWorldRenderDomain::new(listener, ghi_instance.clone(), resource_manager_handle));
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

			Renderer {
				ghi: ghi_instance,

				rendered_frame_count: 0,

				swapchain_handles: vec![],

				render_command_buffer,
				render_finished_synchronizer,
				image_ready,

				result,

				window_system: window_system_handle,

				visibility_render_model,
				ui_render_model,
				tonemap_render_model,
			}
		}).listen_to::<window_system::Window>(listener)
	}

	pub fn render(&mut self,) {
		if self.swapchain_handles.is_empty() { return; }

		let ghi = self.ghi.write().unwrap();

		let swapchain_handle = self.swapchain_handles[0];

		ghi.wait(self.render_finished_synchronizer);

		ghi.start_frame_capture();

		let image_index = ghi.acquire_swapchain_image(swapchain_handle, self.image_ready);

		let mut command_buffer_recording = ghi.create_command_buffer_recording(self.render_command_buffer, Some(self.rendered_frame_count as u32));

		self.visibility_render_model.map(|e| {
			let mut e = e.write_sync();
			e.render(ghi.deref(), command_buffer_recording.as_mut())
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
	async fn on_create<'a>(&'a mut self, handle: EntityHandle<window_system::Window>, window: &window_system::Window) {
		let os_handles = self.window_system.map(|e| {
			let e = e.read_sync();
			e.get_os_handles(&handle)
		});

		let mut ghi = self.ghi.write().unwrap();

		let swapchain_handle = ghi.bind_to_window(&os_handles);

		self.swapchain_handles.push(swapchain_handle);
	}

	async fn on_update(&'static mut self, handle: EntityHandle<window_system::Window>, params: &window_system::Window) {
		
	}
}

impl Entity for Renderer {}