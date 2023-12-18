use std::ops::{DerefMut, Deref};

use crate::{orchestrator::{self, EntityHandle}, window_system::{self, WindowSystem}, Extent, resource_manager::resource_manager::ResourceManager};

use super::{visibility_model::render_domain::VisibilityWorldRenderDomain, render_system::{self, RenderSystem}, aces_tonemap_render_pass::AcesToneMapPass, tonemap_render_pass::ToneMapRenderPass, world_render_domain::WorldRenderDomain};

pub struct Renderer {
	rendered_frame_count: usize,

	swapchain_handles: Vec<render_system::SwapchainHandle>,

	render_command_buffer: render_system::CommandBufferHandle,
	render_finished_synchronizer: render_system::SynchronizerHandle,
	image_ready: render_system::SynchronizerHandle,

	result: render_system::ImageHandle,

	render_system: EntityHandle<dyn render_system::RenderSystem>,
	window_system: EntityHandle<window_system::WindowSystem>,

	visibility_render_model: orchestrator::EntityHandle<VisibilityWorldRenderDomain>,
	tonemap_render_model: orchestrator::EntityHandle<AcesToneMapPass>,
}

impl Renderer {
	pub fn new_as_system(render_system_handle: EntityHandle<dyn RenderSystem>, window_system_handle: EntityHandle<WindowSystem>, resource_manager_handle: EntityHandle<ResourceManager>) -> orchestrator::EntityReturn<'static, Self> {
		orchestrator::EntityReturn::new_from_function(move |orchestrator| {
			let result = {
				let render_system = orchestrator.get_entity(&render_system_handle);
				let mut render_system = render_system.get_mut();

				render_system.create_image(Some("result"), Extent::plane(1920, 1080), render_system::Formats::RGBAu8, None, render_system::Uses::Storage | render_system::Uses::TransferDestination, render_system::DeviceAccesses::GpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::DYNAMIC)
			};

			let visibility_render_model = orchestrator.spawn_entity(VisibilityWorldRenderDomain::new(render_system_handle.clone(), resource_manager_handle)).unwrap();

			let render_system = orchestrator.get_entity(&render_system_handle);
			let mut render_system = render_system.get_mut();
			
			let tonemap_render_model = {
				let visibility_render_model = orchestrator.get_entity(&visibility_render_model);
				let visibility_render_model = visibility_render_model.get();
				orchestrator.spawn_entity(AcesToneMapPass::new_as_system(render_system.deref_mut(), visibility_render_model.get_result_image(), result)).unwrap()
			};

			let render_command_buffer = render_system.create_command_buffer(Some("Render"));
			let render_finished_synchronizer = render_system.create_synchronizer(Some("Render Finisished"), true);
			let image_ready = render_system.create_synchronizer(Some("Swapchain Available"), false);

			Renderer {
				rendered_frame_count: 0,

				swapchain_handles: vec![],

				render_command_buffer,
				render_finished_synchronizer,
				image_ready,

				result,

				render_system: render_system_handle,
				window_system: window_system_handle,

				visibility_render_model,
				tonemap_render_model,
			}
		}).add_listener::<window_system::Window>()
	}

	pub fn render(&mut self, orchestrator: orchestrator::OrchestratorReference) {
		if self.swapchain_handles.is_empty() { return; }

		let render_system = orchestrator.get_entity(&self.render_system);
		let render_system = render_system.get();

		let swapchain_handle = self.swapchain_handles[0];

		render_system.wait(self.render_finished_synchronizer);

		render_system.start_frame_capture();

		let image_index = render_system.acquire_swapchain_image(swapchain_handle, self.image_ready);

		let mut command_buffer_recording = render_system.create_command_buffer_recording(self.render_command_buffer, Some(self.rendered_frame_count as u32));

		let visibility_render_model = orchestrator.get_entity(&self.visibility_render_model);
		let mut visibility_render_model = visibility_render_model.get_mut();

		visibility_render_model.render(&orchestrator, render_system.deref(), command_buffer_recording.as_mut());

		let tonemap_render_model = orchestrator.get_entity(&self.tonemap_render_model);
		let tonemap_render_model = tonemap_render_model.get();

		tonemap_render_model.render(command_buffer_recording.as_mut());

		// Copy to swapchain

		command_buffer_recording.copy_to_swapchain(self.result, image_index, swapchain_handle);

		command_buffer_recording.execute(&[self.image_ready], &[self.render_finished_synchronizer], self.render_finished_synchronizer);

		render_system.end_frame_capture();

		render_system.present(image_index, &[swapchain_handle], self.render_finished_synchronizer);

		self.rendered_frame_count += 1;
	}
}

impl orchestrator::EntitySubscriber<window_system::Window> for Renderer {
	fn on_create(&mut self, orchestrator: orchestrator::OrchestratorReference, handle: orchestrator::EntityHandle<window_system::Window>, window: &window_system::Window) {
		let render_system = orchestrator.get_entity(&self.render_system);
		let mut render_system = render_system.get_mut();

		let window_system = orchestrator.get_entity(&self.window_system);
		let mut window_system = window_system.get_mut();

		let swapchain_handle = render_system.bind_to_window(&window_system.get_os_handles(&handle));

		self.swapchain_handles.push(swapchain_handle);
	}

	fn on_update(&mut self, orchestrator: orchestrator::OrchestratorReference, handle: EntityHandle<window_system::Window>, params: &window_system::Window) {
		
	}
}

impl orchestrator::Entity for Renderer {}