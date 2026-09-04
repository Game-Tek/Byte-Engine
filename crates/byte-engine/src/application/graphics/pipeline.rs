//! Graphics pipeline and render-pass setup.

use super::*;
use crate::rendering::{ConeLight, DirectionalLight, PointLight};

/// Installs the retained wireframe debug scene after the render passes registered before this call.
///
/// Call this after tone mapping, every other image effect, and any overlay it
/// should cover to keep debug colors unchanged and on top. The pass also supports
/// registration before tone mapping when post-processed, scene-linear debug
/// colors are useful. In either position, depth-aware messages read the scene
/// pipeline's `depth` image without modifying it. Register this pass before
/// creating the first window. Next, call [`Factory::create`] once for each
/// [`rendering::DebugMesh`], [`Factory::derive`] to replace it, and
/// [`DefaultWorld::delete`] to remove it.
pub fn setup_debug_mesh_render_pass(application: &mut GraphicsApplication) -> Factory<rendering::DebugMesh> {
	defaults::setup_default_pipeline_compilation(application);
	let factory = application.world().factory::<rendering::DebugMesh>();
	// Register future-only lifecycle listeners before returning the producer factory.
	let listener = factory.listener();
	let delete_listener = application.world().deletions_listener();
	let scene = std::rc::Rc::new(std::cell::RefCell::new(rendering::DebugSceneManager::new(
		&mut application.renderer.context_mut(),
		listener,
		delete_listener,
	)));

	application
		.renderer
		.add_post_scene_render_pass_for_all_sinks(move |render_pass_builder| {
			Box::new(rendering::DebugMeshRenderPass::new(
				render_pass_builder,
				std::rc::Rc::clone(&scene),
			))
		});

	factory
}

/// Installs the simple scene pipeline and registers its asynchronous mesh-loading worker.
///
/// This setup is the smallest end-to-end implementation of
/// [`rendering::loading`]. It creates one mapped staging arena, one loader lane,
/// and a Simple-owned store whose position and index streams intentionally
/// differ from Visibility storage. The supplied callback owns task placement so
/// this function does not impose an executor or thread policy on the
/// application. Simple's shaders use the renderer's asynchronous pipeline
/// compilation servers; this setup never waits for shader resources.
///
/// Add the supplied task to a queue from [`defaults::build_deferred_tasks_queue`],
/// then start that queue with [`defaults::launch_deferred_tasks_thread`] after
/// every subsystem has registered its work. At shutdown, join that task before
/// dropping the renderer and mapped upload buffer.
pub fn setup_simple_render_pipeline(
	application: &mut GraphicsApplication,
	spawn_loading_task: impl FnOnce(std::boxed::Box<dyn FnOnce(&compio::runtime::Runtime) + Send>),
) {
	defaults::setup_default_pipeline_compilation(application);
	let listener = application.world().factory::<RenderableMesh>().listener();
	let delete_listener = application.world().deletions_listener();
	let transforms_listener = application.world().transforms_channel().listener();
	let application_resources = application.resource_manager.clone();

	let renderer = &mut application.renderer;
	let pipeline_compiler = renderer.pipeline_manager_client();
	let mut context = renderer.context_mut();
	let (upload_buffer, upload_staging, upload_staging_worker) = rendering::resource_loading::UploadStagingArena::create::<
		{ rendering::pipelines::simple::resource_manager::ASYNC_UPLOAD_BUFFER_BYTE_COUNT },
	>(&mut context, "Simple Async Upload Buffer");
	let resource_store = std::sync::Arc::new(std::sync::Mutex::new(
		rendering::pipelines::simple::resource_manager::SimpleResourceStore::new(&mut context, upload_buffer),
	));
	drop(context);
	let (simple_loader, simple_loader_lanes) = rendering::pipelines::simple::resource_manager::SimpleLoader::spawn(
		&renderer.shared_context(),
		renderer.graphics_queue(),
		application_resources,
		upload_staging,
		resource_store.clone(),
	);

	spawn_loading_task(std::boxed::Box::new(move |runtime| {
		runtime.spawn(upload_staging_worker.run()).detach();
		for lane in simple_loader_lanes {
			runtime.spawn(lane.run()).detach();
		}
	}));

	struct CustomPipelineManager {
		pipeline_manager: SimplePipelineManager,
		mesh_receiver: DefaultListener<CreateMessage<RenderableMesh>>,
		mesh_delete_receiver: DefaultListener<DeleteMessage>,
		transforms_listener: DefaultListener<TransformationUpdate>,
	}

	impl PipelineManager for CustomPipelineManager {
		fn prepare<'a>(
			&'a mut self,
			frame: &mut ghi::implementation::Frame,
			sinks: &[rendering::Sink],
			frame_allocator: &'a bumpalo::Bump,
		) -> Option<SmallVec<[rendering::render_pass::RenderPassReturn<'a>; 16]>> {
			while let Some(message) = self.mesh_receiver.read() {
				let handle = message.handle();

				self.pipeline_manager.request_mesh(frame, handle, message.into_data());
			}

			while let Some(message) = self.transforms_listener.read() {
				self.pipeline_manager
					.update_transform(frame, message.handle(), message.transform());
			}

			while let Some(message) = self.mesh_delete_receiver.read() {
				self.pipeline_manager.remove_mesh(message.into_handle());

				// TODO: handle light removal
			}

			self.pipeline_manager.prepare(frame, sinks, frame_allocator)
		}

		fn create_sink(&mut self, sink_id: usize, render_pass_builder: &mut rendering::render_pass::RenderPassBuilder) {
			self.pipeline_manager.create_sink(sink_id, render_pass_builder);
		}
	}

	let sm = {
		CustomPipelineManager {
			pipeline_manager: SimplePipelineManager::new(
				&mut renderer.context_mut(),
				pipeline_compiler,
				simple_loader,
				resource_store,
			),
			mesh_receiver: listener,
			mesh_delete_receiver: delete_listener,
			transforms_listener,
		}
	};

	renderer.add_pipeline_manager(sm);
}

/// Installs the visibility-buffer PBR scene pipeline and its async upload worker.
///
/// Visibility gives meshes, materials, and textures independent loader lanes.
/// Mesh loaders append parallel geometry streams and request discovered
/// materials. Material loaders assign table slots and request discovered
/// textures. Texture loaders finish CPU or native GPU-I/O transfers before
/// publishing residency. These choices belong to Visibility; they are not
/// requirements of the shared loader.
///
/// The supplied callback must run the staging worker and every loader lane on
/// application-owned async tasks. Join those tasks before renderer shutdown so
/// no worker retains a mapping or detached GHI factory after its context is
/// dropped.
///
/// Next, create an [`Environment`] through
/// [`DefaultWorld::factory`] to select the HDR image used for ambient and
/// specular reflections.
// Keep the cross-layer setup sequence contiguous so listeners, workers, mappings, and renderer ownership remain ordered.
#[allow(clippy::too_many_lines)]
pub fn setup_pbr_visibility_shading_render_pipeline(
	application: &mut GraphicsApplication,
	spawn_loading_task: impl FnOnce(std::boxed::Box<dyn FnOnce(&compio::runtime::Runtime) + Send>),
) {
	defaults::setup_default_pipeline_compilation(application);
	let mut visibility_pipeline_settings = VisibilityPipelineSettings::default();
	if let Some(parameter) = application.get_parameter(CONE_SHADOW_MAP_POOL_CAPACITY_PARAMETER) {
		let capacity = parameter.value().parse::<usize>().unwrap_or_else(|_| {
			panic!(
				"Cone shadow map pool capacity was not set. The most likely cause is that `{}` is not a whole number.",
				parameter.value()
			)
		});
		visibility_pipeline_settings = visibility_pipeline_settings
			.with_cone_shadow_map_pool_capacity(capacity)
			.unwrap_or_else(|reason| panic!("{reason}"));
	}
	if let Some(parameter) = application.get_parameter(POINT_SHADOW_MAP_POOL_CAPACITY_PARAMETER) {
		let capacity = parameter.value().parse::<usize>().unwrap_or_else(|_| {
			panic!(
				"Point shadow map pool capacity was not set. The most likely cause is that `{}` is not a whole number.",
				parameter.value()
			)
		});
		visibility_pipeline_settings = visibility_pipeline_settings
			.with_point_shadow_map_pool_capacity(capacity)
			.unwrap_or_else(|reason| panic!("{reason}"));
	}
	let gtao_configuration = application
		.configuration()
		.register(crate::rendering::pipelines::visibility::GTAO_CONFIGURATION_PREFIX);
	for parameter_name in ["render.gtao.radius", "render.gtao.samples-per-ray", "render.gtao.radial-rays"] {
		if let Some(parameter) = application.get_parameter(parameter_name) {
			application.configuration().update(parameter.name(), parameter.value());
		}
	}

	let application_resource_manager = application.resource_manager.clone();
	let renderer = &mut application.renderer;
	let pipeline_manager = renderer.pipeline_manager_client();
	let mut context = renderer.context_mut();
	let material_pipeline_config = rendering::pipelines::visibility::MaterialPipelineConfig::new(
		vec![ghi::pipelines::PushConstantRange::new(0, 8)],
		pipeline_manager.clone(),
	);

	let (upload_buffer, upload_staging, upload_staging_worker) = rendering::resource_loading::UploadStagingArena::create::<
		{ rendering::pipelines::visibility::ASYNC_UPLOAD_BUFFER_BYTE_COUNT },
	>(&mut context, "Renderer Async Upload Buffer");

	let geometry = rendering::pipelines::visibility::GeometryHandles::new(&mut context);

	drop(context);

	let shared_context = renderer.shared_context();
	let graphics_queue = renderer.graphics_queue();

	let (visibility_loader, visibility_loader_lanes) = rendering::pipelines::visibility::spawn_loader(
		&shared_context,
		graphics_queue,
		application_resource_manager,
		upload_staging,
		upload_buffer,
		geometry,
		material_pipeline_config,
	);

	spawn_loading_task(std::boxed::Box::new(move |runtime| {
		runtime.spawn(upload_staging_worker.run()).detach();
		for lane in visibility_loader_lanes {
			runtime.spawn(lane.run()).detach();
		}
	}));

	struct CustomPipelineManager {
		cone_light_receiver: DefaultListener<CreateMessage<ConeLight>>,
		directional_light_receiver: DefaultListener<CreateMessage<DirectionalLight>>,
		point_light_receiver: DefaultListener<CreateMessage<PointLight>>,
		delete_receiver: DefaultListener<DeleteMessage>,
		mesh_receiver: DefaultListener<CreateMessage<RenderableMesh>>,
		pose_receiver: DefaultListener<UpdatePose>,
		environment_receiver: DefaultListener<CreateMessage<Environment>>,
		visibility_pipeline_manager: VisibilityPipelineManager,
	}

	impl CustomPipelineManager {
		/// Drains light creation messages into the visibility scene.
		fn request_pending_lights(&mut self) {
			// Concrete routes let application-defined creation stay strongly typed.
			// The visibility scene erases each value only at its storage boundary.
			while let Some(message) = self.cone_light_receiver.read() {
				let handle = message.handle();
				self.visibility_pipeline_manager
					.create_light(handle, message.into_data().into());
			}

			while let Some(message) = self.directional_light_receiver.read() {
				let handle = message.handle();
				self.visibility_pipeline_manager
					.create_light(handle, message.into_data().into());
			}

			while let Some(message) = self.point_light_receiver.read() {
				let handle = message.handle();
				self.visibility_pipeline_manager
					.create_light(handle, message.into_data().into());
			}
		}

		/// Drains renderable creation messages into the visibility resource request path.
		fn request_pending_meshes(&mut self) {
			while let Some(message) = self.mesh_receiver.read() {
				let handle = message.handle();
				self.visibility_pipeline_manager.request_mesh(handle, message.into_data());
			}
		}

		/// Drains pending deletion messages.
		fn process_deletions(&mut self) {
			while let Some(message) = self.delete_receiver.read() {
				let handle = message.into_handle();
				self.visibility_pipeline_manager.remove_light(handle);
				self.visibility_pipeline_manager.remove_mesh(handle);
			}
		}

		/// Applies application-authored skeleton poses to the visibility scene.
		fn process_pose_updates(&mut self) {
			while let Some(message) = self.pose_receiver.read() {
				self.visibility_pipeline_manager
					.update_pose(message.handle(), message.global_matrices());
			}
		}

		/// Drains environment creation commands into the visibility resource request path.
		fn request_pending_environments(&mut self) {
			while let Some(message) = self.environment_receiver.read() {
				self.visibility_pipeline_manager.create_environment(message.into_data());
			}
		}
	}

	impl PipelineManager for CustomPipelineManager {
		fn prepare<'a>(
			&'a mut self,
			frame: &mut ghi::implementation::Frame,
			sinks: &[rendering::Sink],
			frame_allocator: &'a bumpalo::Bump,
		) -> Option<SmallVec<[rendering::render_pass::RenderPassReturn<'a>; 16]>> {
			self.request_pending_lights();
			self.request_pending_meshes();
			self.request_pending_environments();
			self.process_pose_updates();

			self.visibility_pipeline_manager.process_transform_updates();
			self.process_deletions();

			self.visibility_pipeline_manager.prepare(frame, sinks, frame_allocator)
		}

		fn create_sink(&mut self, sink_id: usize, render_pass_builder: &mut rendering::render_pass::RenderPassBuilder) {
			self.visibility_pipeline_manager.create_sink(sink_id, render_pass_builder);
		}
	}

	{
		let cone_light_receiver = application.world().factory::<ConeLight>().listener();
		let directional_light_receiver = application.world().factory::<DirectionalLight>().listener();
		let point_light_receiver = application.world().factory::<PointLight>().listener();
		let delete_receiver = application.world().deletions_listener();
		let mesh_receiver = application.world().factory::<RenderableMesh>().listener();
		let transforms_listener = application.world().transforms_channel().listener();
		let pose_receiver = application.world().poses_channel().listener();
		let environment_receiver = application.world().factory::<Environment>().listener();

		let renderer = &mut application.renderer;
		let sm = CustomPipelineManager {
			visibility_pipeline_manager: VisibilityPipelineManager::new(
				&mut renderer.context_mut(),
				geometry,
				visibility_loader,
				pipeline_manager,
				transforms_listener,
				gtao_configuration,
				visibility_pipeline_settings,
			),
			cone_light_receiver,
			directional_light_receiver,
			point_light_receiver,
			delete_receiver,
			mesh_receiver,
			pose_receiver,
			environment_receiver,
		};

		renderer.add_pipeline_manager(sm);
	}
}

/// Installs the retained UI render pass fed by UI render messages from `ui`.
///
/// Register this pass before publishing renders that every sink must observe.
pub fn setup_ui_render_pass(application: &mut GraphicsApplication, ui: &Factory<Render>) {
	let ui = ui.clone();
	let renderer = &mut application.renderer;

	renderer.add_post_scene_render_pass_for_all_sinks(move |render_pass_builder| {
		struct CustomRenderPass {
			listener: DefaultListener<CreateMessage<Render>>,
			render_pass: UiRenderPass,
		}

		impl rendering::RenderPass for CustomRenderPass {
			fn name(&self) -> &'static str {
				self.render_pass.name()
			}

			fn prepare<'a>(
				&mut self,
				frame: &mut ghi::implementation::Frame,
				sink: &rendering::Sink,
				frame_allocator: &'a bumpalo::Bump,
			) -> Option<rendering::render_pass::RenderPassReturn<'a>> {
				drain_render_pass_messages(&mut self.listener, |render| self.render_pass.update(render.into_data()));

				self.render_pass.prepare(frame, sink, frame_allocator)
			}

			fn bypass<'a>(
				&mut self,
				frame: &mut ghi::implementation::Frame,
				sink: &rendering::Sink,
				frame_allocator: &'a bumpalo::Bump,
			) -> Option<rendering::render_pass::RenderPassReturn<'a>> {
				drain_render_pass_messages(&mut self.listener, |render| self.render_pass.update(render.into_data()));

				self.render_pass.bypass(frame, sink, frame_allocator)
			}
		}

		Box::new(CustomRenderPass {
			listener: ui.listener(),
			render_pass: UiRenderPass::new(render_pass_builder),
		})
	});
}

/// Drains all pending pass inputs so active and bypassed paths adopt the same application state.
pub(super) fn drain_render_pass_messages<M: Clone + Send + Sync + 'static>(
	listener: &mut DefaultListener<M>,
	mut adopt: impl FnMut(M),
) {
	while let Some(message) = listener.read() {
		adopt(message);
	}
}

/// Installs the AGX tonemapping pass for post-scene color mapping.
pub fn setup_agx_tonemap_render_pass(application: &mut GraphicsApplication) {
	let renderer = &mut application.renderer;

	renderer.add_post_scene_render_pass_for_all_sinks(|render_pass_builder| Box::new(AgxToneMapPass::new(render_pass_builder)));
}

/// Installs the ACES v1 tonemapping pass for post-scene color mapping.
pub fn setup_aces_tonemap_render_pass(application: &mut GraphicsApplication) {
	let renderer = &mut application.renderer;

	renderer
		.add_post_scene_render_pass_for_all_sinks(|render_pass_builder| Box::new(AcesToneMapPass::new(render_pass_builder)));
}

/// Registers a reusable bloom pass that should run before tonemapping.
pub fn setup_bloom_render_pass(application: &mut GraphicsApplication, settings: BloomPassSettings) {
	let renderer = &mut application.renderer;

	renderer.add_post_scene_render_pass_for_all_sinks(move |render_pass_builder| {
		Box::new(BloomPass::with_settings(render_pass_builder, settings))
	});
}

/// Installs a fused ACEScg/ACEScct grading and SDR output pass from a prepared LUT.
///
/// The LUT must accept and return ACEScct values. The pass uses an AP1-aware
/// fitted SDR transform; it does not claim ACES reference-transform compliance.
/// Load it with [`crate::rendering::render_passes::lut::PreparedLut::load`] on
/// application-owned asynchronous work before calling this setup function. Call
/// this after scene-linear effects. Later passes consume its remapped `main` output.
pub fn setup_aces_color_grading_render_pass(
	application: &mut GraphicsApplication,
	lut: crate::rendering::render_passes::lut::PreparedLut,
) {
	setup_color_grading_render_pass(application, lut, ColorGradingWorkflow::Aces);
}

/// Installs a fused DaVinci Wide Gamut/Intermediate grading and SDR output pass from a prepared LUT.
///
/// The LUT must accept and return DaVinci Wide Gamut/Intermediate values. The
/// pass uses AgX for SDR display rendering and does not claim parity with
/// DaVinci Resolve color management. Load it with
/// [`crate::rendering::render_passes::lut::PreparedLut::load`] on
/// application-owned asynchronous work before calling this setup function. Call
/// this after scene-linear effects. Later passes consume its remapped `main` output.
pub fn setup_dwg_color_grading_render_pass(
	application: &mut GraphicsApplication,
	lut: crate::rendering::render_passes::lut::PreparedLut,
) {
	setup_color_grading_render_pass(application, lut, ColorGradingWorkflow::DaVinciWideGamut);
}

/// Loads and installs one fixed color-grading workflow for every render sink.
fn setup_color_grading_render_pass(
	application: &mut GraphicsApplication,
	lut: crate::rendering::render_passes::lut::PreparedLut,
	workflow: ColorGradingWorkflow,
) {
	application
		.renderer
		.add_post_scene_render_pass_for_all_sinks(move |render_pass_builder| {
			Box::new(ColorGradingPass::new(render_pass_builder, workflow, lut.clone()))
		});
}

/// Installs a 3D LUT grading pass from asynchronously prepared resource data.
///
/// Load the resource once with
/// [`crate::rendering::render_passes::lut::PreparedLut::load`] on
/// application-owned asynchronous work. Each sink shares those immutable bytes
/// while creating its own renderer-owned image. Call this after passes that
/// produce the HDR `main` target and before tone mapping.
pub fn setup_lut_render_pass(application: &mut GraphicsApplication, lut: crate::rendering::render_passes::lut::PreparedLut) {
	application
		.renderer
		.add_post_scene_render_pass_for_all_sinks(move |render_pass_builder| {
			Box::new(crate::rendering::render_passes::lut::LutRenderPass::new(
				render_pass_builder,
				lut.clone(),
			))
		});
}

/// Installs spatial SMAA for every current and future render sink.
///
/// This adds a self-contained post-scene pass without coalescing it with tone mapping
/// or other independent passes. Call it after the setup that produces the `main` color
/// input you want SMAA to filter, and before any overlay pass you want to keep sharp.
pub fn setup_smaa_render_pass(application: &mut GraphicsApplication) {
	application
		.renderer
		.add_post_scene_render_pass_for_all_sinks(|render_pass_builder| Box::new(SmaaPass::new(render_pass_builder)));
}

/// Installs the atmosphere sky pass used as a post-scene background.
pub fn setup_atmosphere_sky_render_pass(application: &mut GraphicsApplication) {
	// Keep producer handles in the sink factory instead of template listeners, which would retain unread broadcast messages.
	let light_factory = application.world().factory::<DirectionalLight>();
	let transform_channel = application.world().transforms_channel().clone();
	let renderer = &mut application.renderer;

	renderer.add_post_scene_render_pass_for_all_sinks(move |render_pass_builder| {
		Box::new(AtmosphereSkyRenderPass::new(
			render_pass_builder,
			light_factory.listener(),
			transform_channel.listener(),
		))
	});
}
