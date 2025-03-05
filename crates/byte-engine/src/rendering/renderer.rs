use core::entity::DomainType;
use std::{io::Write, ops::{Deref, DerefMut}, rc::Rc, sync::Arc};

use ghi::{GraphicsHardwareInterface, CommandBufferRecordable, BoundComputePipelineMode};
use resource_management::resource::resource_manager::ResourceManager;
use utils::{sync::RwLock, Extent};

use crate::{core::{self, entity::EntityBuilder, listener::{EntitySubscriber, Listener}, orchestrator, Entity, EntityHandle}, ui::render_model::UIRenderModel, utils, window_system::{self, WindowSystem},};

use super::{aces_tonemap_render_pass::AcesToneMapPass, background_render_pass::BackgroundRenderingPass, fog_render_pass::FogRenderPass, render_pass::{BlitPass, RenderPass}, shadow_render_pass::ShadowRenderingPass, ssao_render_pass::ScreenSpaceAmbientOcclusionPass, ssgi_render_pass::SSGIRenderPass, texture_manager::TextureManager, tonemap_render_pass::ToneMapRenderPass, visibility_model::render_domain::VisibilityWorldRenderDomain, world_render_domain::WorldRenderDomain};

pub struct Renderer {
	ghi: Rc<RwLock<ghi::GHI>>,

	rendered_frame_count: usize,
	frame_queue_depth: usize,

	swapchain_handles: Vec<ghi::SwapchainHandle>,

	render_command_buffer: ghi::CommandBufferHandle,
	render_finished_synchronizer: ghi::SynchronizerHandle,
	image_ready: ghi::SynchronizerHandle,

	result: ghi::ImageHandle,
	accumulation_map: ghi::ImageHandle,

	window_system: EntityHandle<window_system::WindowSystem>,

	root_render_pass: RootRenderPass,

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

			let result;
			let accumulation_map;
			let depth_sampler;
			
			{
				let mut ghi = ghi_instance.write();

				result = ghi.create_image(Some("result"), extent, ghi::Formats::RGBA8(ghi::Encodings::UnsignedNormalized), ghi::Uses::Storage | ghi::Uses::TransferDestination, ghi::DeviceAccesses::GpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::DYNAMIC, 1);
				accumulation_map = ghi.create_image(Some("accumulate_map"), extent, ghi::Formats::RGBA16(ghi::Encodings::UnsignedNormalized), ghi::Uses::Storage | ghi::Uses::TransferSource | ghi::Uses::BlitDestination, ghi::DeviceAccesses::GpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::DYNAMIC, 1);
				depth_sampler = ghi.build_sampler(ghi::sampler::Builder::new().addressing_mode(ghi::SamplerAddressingModes::Border {}).reduction_mode(ghi::SamplingReductionModes::Min).filtering_mode(ghi::FilteringModes::Closest));
			};

			let texture_manager = Arc::new(utils::r#async::RwLock::new(TextureManager::new()));

			let visibility_render_model: EntityHandle<VisibilityWorldRenderDomain> = core::spawn_as_child(parent.clone(), VisibilityWorldRenderDomain::new(ghi_instance.clone(), resource_manager_handle.clone(), texture_manager.clone())).await;

			let render_command_buffer;
			let render_finished_synchronizer;
			let image_ready;

			let diffuse = {
				let vrm = visibility_render_model.read_sync();
				vrm.get_diffuse()
			};

			let ao_render_pass = {
				let vrm = visibility_render_model.read_sync();
				core::spawn(ScreenSpaceAmbientOcclusionPass::new(ghi_instance.clone(), resource_manager_handle.clone(), texture_manager.clone(), vrm.get_descriptor_set_template(), vrm.get_view_occlusion_image(), vrm.get_view_depth_image()).await).await
			};

			let ssgi_render_pass = {
				let vrm = visibility_render_model.read_sync();
				core::spawn(SSGIRenderPass::new(ghi_instance.clone(), resource_manager_handle.clone(), texture_manager.clone(), vrm.get_descriptor_set_template(), (vrm.get_view_depth_image(), depth_sampler), vrm.get_diffuse(),).await).await
			};

			let background_render_pass: EntityHandle<BackgroundRenderingPass> = {
				let vrm = visibility_render_model.read_sync();
				let mut ghi = ghi_instance.write();
				core::spawn(BackgroundRenderingPass::new(&mut ghi, vrm.get_views_buffer(), vrm.get_view_depth_image(), accumulation_map)).await
			};

			let fog_render_pass = {
				let vrm = visibility_render_model.read_sync();
				let result_image = vrm.get_diffuse();
				let mut ghi = ghi_instance.write();
				core::spawn(FogRenderPass::new(&mut ghi, &vrm.get_descriptor_set_template(), vrm.get_view_depth_image(), result_image)).await
			};

			let tonemap_render_model = {
				let mut ghi = ghi_instance.write();

				let tonemap_render_model: EntityHandle<AcesToneMapPass> = core::spawn(AcesToneMapPass::new(ghi.deref_mut(), accumulation_map, result)).await;

				render_command_buffer = ghi.create_command_buffer(Some("Render"));
				render_finished_synchronizer = ghi.create_synchronizer(Some("Render Finisished"), true);
				image_ready = ghi.create_synchronizer(Some("Swapchain Available"), false);

				tonemap_render_model
			};

			let mut root_render_pass = RootRenderPass::new();

			visibility_render_model.write_sync().add_render_pass(ao_render_pass);

			root_render_pass.add_render_pass(visibility_render_model);

			root_render_pass.add_render_pass(core::spawn(BlitPass::new(diffuse, accumulation_map)).await);

			if true {
				root_render_pass.add_render_pass(ssgi_render_pass);
			}
			
			root_render_pass.add_render_pass(background_render_pass);
			// root_render_pass.add_render_pass(fog_render_pass);
			root_render_pass.add_render_pass(tonemap_render_model);

			Renderer {
				ghi: ghi_instance,

				rendered_frame_count: 0,
				frame_queue_depth: 2,

				swapchain_handles: vec![],

				render_command_buffer,
				render_finished_synchronizer,
				image_ready,

				result,
				accumulation_map,

				window_system: window_system_handle,

				root_render_pass,

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

		let extent = extent.unwrap_or(Extent::rectangle(1920, 1080));

		assert!(extent.width() <= 65535 && extent.height() <= 65535, "The extent is too large: {:?}. The renderer only supports dimensions as big as 16 bits.", extent);

		drop(ghi);

		if extent != self.extent {
			let mut ghi = self.ghi.write();

			ghi.resize_image(self.result, extent);
			ghi.resize_image(self.accumulation_map, extent);

			self.root_render_pass.resize(&mut ghi, extent);

			self.extent = extent;
		}

		let mut ghi = self.ghi.write();

		self.root_render_pass.prepare(&mut ghi, extent);

		let mut command_buffer_recording = ghi.create_command_buffer_recording(self.render_command_buffer, Some(self.rendered_frame_count as u32));

		self.root_render_pass.record(&mut command_buffer_recording, extent);

		// Copy to swapchain

		command_buffer_recording.copy_to_swapchain(self.result, present_key, swapchain_handle);

		command_buffer_recording.execute(&[self.image_ready,], &[self.render_finished_synchronizer], self.render_finished_synchronizer);

		ghi.end_frame_capture();

		ghi.present(modulo_frame_index, present_key, &[swapchain_handle], self.render_finished_synchronizer);

		self.rendered_frame_count += 1;
	}
}

impl EntitySubscriber<window_system::Window> for Renderer {
	fn on_create<'a>(&'a mut self, handle: EntityHandle<window_system::Window>, window: &window_system::Window) -> utils::BoxedFuture<()> {
		let os_handles = self.window_system.map(|e| {
			let e = e.read_sync();
			e.get_os_handles(&handle)
		});

		let mut ghi = self.ghi.write();

		let swapchain_handle = ghi.bind_to_window(&os_handles, ghi::PresentationModes::FIFO, Extent::rectangle(1920, 1080));

		self.swapchain_handles.push(swapchain_handle);

		Box::pin(async move {})
	}
}

impl Entity for Renderer {}

struct RootRenderPass {
	render_passes: Vec<EntityHandle<dyn RenderPass>>,
}

impl RootRenderPass {
	pub fn new() -> Self {
		Self {
			render_passes: vec![],
		}
	}
}


impl RenderPass for RootRenderPass {
	fn add_render_pass(&mut self, render_pass: EntityHandle<dyn RenderPass>) {
		self.render_passes.push(render_pass);
	}

	fn prepare(&self, ghi: &mut ghi::GHI, extent: Extent) {
		for render_pass in &self.render_passes {
			render_pass.sync_get_mut(|e| {
				e.prepare(ghi, extent);
			});
		}
	}

	fn record(&self, command_buffer_recording: &mut ghi::CommandBufferRecording, extent: Extent) {
		for render_pass in &self.render_passes {
			render_pass.sync_get_mut(|e| {
				e.record(command_buffer_recording, extent);
			});
		}
	}

	fn resize(&self, ghi: &mut ghi::GHI, extent: Extent) {
		for render_pass in &self.render_passes {
			render_pass.sync_get_mut(|e| {
				e.resize(ghi, extent);
			});
		}
	}
}