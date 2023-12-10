use crate::{orchestrator, window_system, Extent};

use super::{visibility_model::render_domain::VisibilityWorldRenderDomain, render_system::{self, RenderSystem}, aces_tonemap_render_pass::AcesToneMapPass, tonemap_render_pass::ToneMapRenderPass, world_render_domain::WorldRenderDomain};

pub struct Renderer {
	rendered_frame_count: usize,

	swapchain_handles: Vec<render_system::SwapchainHandle>,

	render_command_buffer: render_system::CommandBufferHandle,
	render_finished_synchronizer: render_system::SynchronizerHandle,
	image_ready: render_system::SynchronizerHandle,

	result: render_system::ImageHandle,

	visibility_render_model: orchestrator::EntityHandle<VisibilityWorldRenderDomain>,
	tonemap_render_model: orchestrator::EntityHandle<AcesToneMapPass>,
}

impl Renderer {
	pub fn new_as_system() -> orchestrator::EntityReturn<'static, Self> {
		orchestrator::EntityReturn::new_from_function(move |orchestrator| {
			let render_system = orchestrator.get_by_class::<render_system::RenderSystemImplementation>();
			let mut render_system = render_system.get_mut();
			let render_system = render_system.downcast_mut::<render_system::RenderSystemImplementation>().unwrap();

			let result = render_system.create_image(Some("result"), Extent::plane(1920, 1080), render_system::Formats::RGBAu8, None, render_system::Uses::Storage | render_system::Uses::TransferDestination, render_system::DeviceAccesses::GpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::DYNAMIC);

			let visibility_render_model = orchestrator.spawn_entity(VisibilityWorldRenderDomain::new(render_system)).unwrap();
			
			let tonemap_render_model = {
				let visibility_render_model = orchestrator.get_entity(&visibility_render_model);
				let mut visibility_render_model = visibility_render_model.get_mut();
				let visibility_render_model = visibility_render_model.downcast_mut::<VisibilityWorldRenderDomain>().unwrap();
				orchestrator.spawn_entity(AcesToneMapPass::new_as_system(render_system, visibility_render_model.get_result_image(), result)).unwrap()
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

				visibility_render_model,
				tonemap_render_model,
			}
		}).add_listener::<window_system::Window>()
	}

	pub fn render(&mut self, orchestrator: orchestrator::OrchestratorReference) {
		if self.swapchain_handles.is_empty() { return; }

		let swapchain_handle = self.swapchain_handles[0];

		let render_system = orchestrator.get_by_class::<render_system::RenderSystemImplementation>();
		let mut render_system = render_system.get_mut();
		let render_system = render_system.downcast_mut::<render_system::RenderSystemImplementation>().unwrap();

		render_system.wait(self.render_finished_synchronizer);

		render_system.start_frame_capture();

		let image_index = render_system.acquire_swapchain_image(swapchain_handle, self.image_ready);

		let mut command_buffer_recording = render_system.create_command_buffer_recording(self.render_command_buffer, Some(self.rendered_frame_count as u32));

		let visibility_render_model = orchestrator.get_entity(&self.visibility_render_model);
		let mut visibility_render_model = visibility_render_model.get_mut();
		let visibility_render_model = visibility_render_model.downcast_mut::<VisibilityWorldRenderDomain>().unwrap();

		visibility_render_model.render(&orchestrator, render_system, command_buffer_recording.as_mut());

		let tonemap_render_model = orchestrator.get_entity(&self.tonemap_render_model);
		let mut tonemap_render_model = tonemap_render_model.get_mut();
		let tonemap_render_model = tonemap_render_model.downcast_mut::<AcesToneMapPass>().unwrap();

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
		let render_system = orchestrator.get_by_class::<render_system::RenderSystemImplementation>();
		let mut render_system = render_system.get_mut();
		let render_system = render_system.downcast_mut::<render_system::RenderSystemImplementation>().unwrap();

		let window_system = orchestrator.get_by_class::<window_system::WindowSystem>();
		let mut window_system = window_system.get_mut();
		let window_system = window_system.downcast_mut::<window_system::WindowSystem>().unwrap();

		let swapchain_handle = render_system.bind_to_window(&window_system.get_os_handles(&handle));

		self.swapchain_handles.push(swapchain_handle);
	}
}

impl orchestrator::Entity for Renderer {}