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
		fn update(&mut self) {
			while let Some(message) = self.mesh_receiver.read() {
				let handle = message.handle();

				self.pipeline_manager.request_mesh(handle, message.into_data());
			}
		}

		fn prepare<'a>(
			&'a mut self,
			frame: &mut ghi::implementation::Frame,
			sinks: &[rendering::Sink],
			frame_allocator: &'a bumpalo::Bump,
			alpha: f32,
			time: crate::time::MediaTime,
		) -> Option<SmallVec<[rendering::render_pass::RenderPassReturn<'a>; 16]>> {
			while let Some(message) = self.transforms_listener.read() {
				self.pipeline_manager
					.update_transform(frame, message.handle(), message.transform());
			}

			while let Some(message) = self.mesh_delete_receiver.read() {
				self.pipeline_manager.remove_mesh(message.into_handle());

				// TODO: handle light removal
			}

			self.pipeline_manager.prepare(frame, sinks, frame_allocator, alpha, time)
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
	// Geometry capacity: each parameter overrides one scene-wide geometry buffer's element count.
	let mut geometry_capacity = visibility_pipeline_settings.geometry_capacity();
	for (stream, capacity) in [
		("vertex", &mut geometry_capacity.vertices),
		("vertex-index", &mut geometry_capacity.vertex_indices),
		("triangle", &mut geometry_capacity.triangles),
		("meshlet", &mut geometry_capacity.meshlets),
		("skinning-vertex", &mut geometry_capacity.skinning_vertices),
	] {
		let name = format!("{GEOMETRY_CAPACITY_PARAMETER_PREFIX}{stream}-capacity");
		if let Some(parameter) = application.get_parameter(&name) {
			*capacity = parameter.value().parse::<u32>().unwrap_or_else(|_| {
				panic!(
					"Geometry capacity was not set. The most likely cause is that `{}` for `{name}` is not a whole number below 2^32.",
					parameter.value()
				)
			});
		}
	}
	visibility_pipeline_settings = visibility_pipeline_settings
		.with_geometry_capacity(geometry_capacity)
		.unwrap_or_else(|reason| panic!("{reason}"));
	// Directional shadow coverage: each parameter overrides one part of the default splits.
	let parse_split_parameter = |name: &str| {
		application.get_parameter(name).map(|parameter| {
			parameter.value().parse::<f32>().unwrap_or_else(|_| {
				panic!(
					"Directional shadow setting was not set. The most likely cause is that `{}` for `{name}` is not a number.",
					parameter.value()
				)
			})
		})
	};
	let default_splits = visibility_pipeline_settings.cascade_splits();
	let shadow_distance = parse_split_parameter(DIRECTIONAL_SHADOW_DISTANCE_PARAMETER).unwrap_or(default_splits.distance());
	let split_blend =
		parse_split_parameter(DIRECTIONAL_SHADOW_SPLIT_BLEND_PARAMETER).unwrap_or(default_splits.logarithmic_share());
	visibility_pipeline_settings = visibility_pipeline_settings.with_cascade_splits(
		crate::rendering::csm::CascadeSplits::new(shadow_distance, split_blend).unwrap_or_else(|reason| panic!("{reason}")),
	);
	if let Some(parameter) = application.get_parameter(DIRECTIONAL_SHADOW_FITTING_PARAMETER) {
		visibility_pipeline_settings = visibility_pipeline_settings.with_cascade_fitting(
			parameter.value().parse().unwrap_or_else(|reason| panic!("{reason}")),
		);
	}
	let gtao_configuration = application
		.configuration()
		.register(crate::rendering::pipelines::visibility::GTAO_CONFIGURATION_PREFIX);
	let contact_shadow_configuration = application
		.configuration()
		.register(crate::rendering::pipelines::visibility::CONTACT_SHADOWS_CONFIGURATION_PREFIX);
	for parameter_name in [
		"render.gtao.radius",
		"render.gtao.samples-per-ray",
		"render.gtao.radial-rays",
		"render.contact-shadows.distance",
	] {
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

	let geometry =
		rendering::pipelines::visibility::GeometryHandles::new(&mut context, visibility_pipeline_settings.geometry_capacity());

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
		resource_receiver: DefaultListener<CreateMessage<rendering::Resource>>,
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

		/// Drains resource creation messages so their loads start before any entity needs them.
		fn request_pending_resources(&mut self) {
			while let Some(message) = self.resource_receiver.read() {
				self.visibility_pipeline_manager.request_resource(message.into_data());
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
		fn update(&mut self) {
			self.request_pending_resources();
			self.request_pending_lights();
			self.request_pending_meshes();
			self.request_pending_environments();
		}

		fn step(&mut self) {
			self.visibility_pipeline_manager.step();
		}

		fn prepare<'a>(
			&'a mut self,
			frame: &mut ghi::implementation::Frame,
			sinks: &[rendering::Sink],
			frame_allocator: &'a bumpalo::Bump,
			alpha: f32,
			time: crate::time::MediaTime,
		) -> Option<SmallVec<[rendering::render_pass::RenderPassReturn<'a>; 16]>> {
			self.process_pose_updates();

			self.visibility_pipeline_manager.process_transform_updates(alpha);
			self.process_deletions();

			self.visibility_pipeline_manager
				.prepare(frame, sinks, frame_allocator, alpha, time)
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
		let resource_receiver = application.world().factory::<rendering::Resource>().listener();
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
				contact_shadow_configuration,
				visibility_pipeline_settings,
			),
			cone_light_receiver,
			directional_light_receiver,
			point_light_receiver,
			delete_receiver,
			mesh_receiver,
			resource_receiver,
			pose_receiver,
			environment_receiver,
		};

		renderer.add_pipeline_manager(sm);
	}
}

/// Installs the retained UI render pass fed by UI render messages from `ui`.
///
/// Register this pass before publishing renders that every sink must observe.
/// The source subscribes immediately and retains the latest render for sinks
/// initialized later. `font` is the file text is drawn with, or `None` for a system font;
/// pass the same file to [`crate::ui::Engine::with_font`] so layout and drawing agree.
pub fn setup_ui_render_pass(application: &mut GraphicsApplication, ui: &Factory<Render>, font: Option<&std::path::Path>) {
	let font = font.map(std::path::Path::to_path_buf);
	defaults::setup_default_pipeline_compilation(application);
	UiRenderPass::request_pipelines(&application.renderer.pipeline_manager_client());
	let source = std::rc::Rc::new(std::cell::RefCell::new(UiRenderSource::new(ui.listener())));
	let renderer = &mut application.renderer;

	renderer.add_post_scene_render_pass_for_all_sinks(move |render_pass_builder| {
		/// The `CustomRenderPass` struct connects a sink to the shared retained UI source.
		struct CustomRenderPass {
			source: std::rc::Rc<std::cell::RefCell<UiRenderSource>>,
			revision: u64,
			render_pass: UiRenderPass,
		}

		impl CustomRenderPass {
			/// Adopts the latest submitted render once per sink, including while bypassed.
			fn update(&mut self) {
				if let Some(render) = self.source.borrow_mut().latest(&mut self.revision) {
					self.render_pass.update(render);
				}
			}
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
				self.update();

				self.render_pass.prepare(frame, sink, frame_allocator)
			}

			fn bypass<'a>(
				&mut self,
				frame: &mut ghi::implementation::Frame,
				sink: &rendering::Sink,
				frame_allocator: &'a bumpalo::Bump,
			) -> Option<rendering::render_pass::RenderPassReturn<'a>> {
				self.update();

				self.render_pass.bypass(frame, sink, frame_allocator)
			}

			fn needs_frame(&mut self) -> bool {
				self.update();

				self.render_pass.needs_frame()
			}
		}

		Box::new(CustomRenderPass {
			source: std::rc::Rc::clone(&source),
			revision: 0,
			render_pass: UiRenderPass::new(render_pass_builder, font.as_deref()),
		})
	});
}

/// The `UiRenderSource` struct retains submitted UI independently of sink startup.
///
/// It keeps the UI in drawable form rather than the submitted [`Render`], so the UI engine gets its render buffers
/// back and rewrites them in place for the next change. Subscribe during setup, then give each sink its own revision
/// for [`Self::latest`].
struct UiRenderSource {
	listener: DefaultListener<CreateMessage<Render>>,
	adopted: AdoptedRender,
	revision: u64,
}

#[cfg(test)]
mod ui_source_tests {
	use super::*;
	use crate::ui::{Container, Context, ElementContext, Engine, Size};

	#[test]
	fn republished_unchanged_render_is_not_adopted_again() {
		let factory = Factory::new();
		let mut source = UiRenderSource::new(factory.listener());
		let mut engine = Engine::new();
		engine.mount(async move |ctx| {
			let _root = ctx.element("root").container(|c| c).await;
			loop {
				ctx.render().await;
			}
		});
		let allocator = bumpalo::Bump::new();
		let mut publish = |size| {
			engine.evaluate(Size::new(size, size), &allocator);
			let render = engine.render();
			factory.create(render.clone());
			render.revision()
		};
		publish(100);
		let mut sink = 0;
		assert!(source.latest(&mut sink).is_some());
		// The same tree at the same size yields the same revision.
		publish(100);
		assert!(source.latest(&mut sink).is_none());
		let resized = publish(120);
		assert_eq!(source.latest(&mut sink).unwrap().revision(), Some(resized));
	}

	#[test]
	fn adopted_ui_returns_render_buffers_to_the_engine() {
		let factory = Factory::new();
		let mut source = UiRenderSource::new(factory.listener());
		let mut engine = Engine::new();
		engine.mount(async move |ctx| {
			let _root = ctx.element("root").container(|c| c).await;
			loop {
				ctx.render().await;
			}
		});
		let allocator = bumpalo::Bump::new();
		let mut publish = |size| {
			engine.evaluate(Size::new(size, size), &allocator);
			let render = engine.render();
			factory.create(render.clone());
			std::ptr::from_ref::<crate::ui::layout::engine::RenderContents>(render)
		};
		let first = publish(100);
		let mut sink = 0;
		assert!(source.latest(&mut sink).is_some());
		// Once the source has adopted the render, nothing else holds it, so the next change is written in place.
		assert_eq!(publish(120), first);
	}

	#[test]
	fn submitted_ui_reaches_late_sinks_without_republication() {
		let factory = Factory::new();
		let mut source = UiRenderSource::new(factory.listener());
		let mut engine = Engine::new();
		engine.mount(async move |ctx| {
			let _root = ctx.element("root").container(|c| c).await;
			loop {
				ctx.render().await;
			}
		});
		let allocator = bumpalo::Bump::new();
		let mut publish = |size| {
			engine.evaluate(Size::new(size, size), &allocator);
			let render = engine.render();
			factory.create(render.clone());
			render.revision()
		};
		// No sink exists when the first render is submitted.
		let submitted = publish(100);
		let mut first = 0;
		assert_eq!(source.latest(&mut first).unwrap().revision(), Some(submitted));
		let mut late = 0;
		assert_eq!(source.latest(&mut late).unwrap().revision(), Some(submitted));
		assert!(source.latest(&mut first).is_none());
		publish(150);
		let newest = publish(200);
		assert_eq!(source.latest(&mut first).unwrap().revision(), Some(newest));
		assert_eq!(source.latest(&mut late).unwrap().revision(), Some(newest));
	}
}

impl UiRenderSource {
	fn new(listener: DefaultListener<CreateMessage<Render>>) -> Self {
		Self {
			listener,
			adopted: AdoptedRender::default(),
			revision: 0,
		}
	}

	/// Returns the newest adopted UI when this sink has not taken it yet.
	fn latest(&mut self, sink_revision: &mut u64) -> Option<&AdoptedRender> {
		// Only the newest pending render is drawn, so older ones are dropped without converting them.
		let mut newest = None;
		drain_render_pass_messages(&mut self.listener, |message| newest = Some(message.into_data()));
		// A republished unchanged render must not make every sink rebuild its draw list. The render drops at the end
		// of this block, which hands the UI engine its buffers back.
		if let Some(render) = newest
			&& self.adopted.adopt(&render)
		{
			self.revision += 1;
		}
		if *sink_revision == self.revision {
			return None;
		}
		*sink_revision = self.revision;
		Some(&self.adopted)
	}
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

/// Installs display sRGB encoding without applying a tone-mapping curve.
///
/// Use this as the final post-scene pass for SDR scenes whose colors already
/// fit in the display range. Omit this setup when a tone mapper or color-grading
/// pass already produces display-encoded output. Register it before creating a
/// window; use `render.pass.srgb-display` to enable or bypass it at runtime.
pub fn setup_srgb_display_render_pass(application: &mut GraphicsApplication) {
	defaults::setup_default_pipeline_compilation(application);
	rendering::render_passes::srgb_display::SrgbDisplayPass::request_pipelines(&application.renderer.pipeline_manager_client());
	application
		.renderer
		.add_post_scene_render_pass_for_all_sinks(|render_pass_builder| {
			Box::new(rendering::render_passes::srgb_display::SrgbDisplayPass::new(
				render_pass_builder,
			))
		});
}

/// Installs the ACES v1 tonemapping pass for post-scene color mapping.
pub fn setup_aces_tonemap_render_pass(application: &mut GraphicsApplication) {
	let renderer = &mut application.renderer;

	renderer
		.add_post_scene_render_pass_for_all_sinks(|render_pass_builder| Box::new(AcesToneMapPass::new(render_pass_builder)));
}

/// Installs an HDR bloom pass for every current and future render sink.
///
/// The pass glows scene-linear light above `settings.threshold` and adds it back onto
/// `main`, so call it after the passes that write scene light, such as the visibility
/// pipeline and the atmosphere sky, and before tone mapping. Later passes consume its
/// remapped `main` output. Use `render.pass.bloom` to enable or bypass it at runtime.
pub fn setup_bloom_render_pass(application: &mut GraphicsApplication, settings: BloomPassSettings) {
	let renderer = &mut application.renderer;

	renderer.add_post_scene_render_pass_for_all_sinks(move |render_pass_builder| {
		Box::new(BloomPass::with_settings(render_pass_builder, settings))
	});
}

/// Installs a screen-space lens flare pass for every current and future render sink.
///
/// The pass casts tinted ghosts and a halo from scene-linear light above `settings.threshold`
/// and adds them onto `main`. Call it after the passes that write scene light and before
/// [`setup_bloom_render_pass`] and tone mapping, so the ghosts glow with the rest of the scene.
/// Later passes consume its remapped `main` output. Use `render.pass.lens-flare` to enable or
/// bypass it at runtime.
pub fn setup_lens_flare_render_pass(
	application: &mut GraphicsApplication,
	settings: rendering::render_passes::lens_flare::LensFlarePassSettings,
) {
	application
		.renderer
		.add_post_scene_render_pass_for_all_sinks(move |render_pass_builder| {
			Box::new(rendering::render_passes::lens_flare::LensFlarePass::with_settings(
				render_pass_builder,
				settings,
			))
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
/// application-owned asynchronous work. Each sink receives its own copy of the
/// bytes, which its pass drops after the first upload. Call this after passes that
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

/// Installs the atmosphere sky as every sink's scene background.
///
/// Scene pipelines draw it after opaque surfaces and before transparent ones, so transparent surfaces blend over
/// the sky. Use `render.pass.atmosphere sky` to enable or bypass it at runtime; bypassed, the background is black.
///
/// The newest [`DirectionalLight`] is the sky's sun: its illuminance sets the sky's brightness and color, and its
/// transform sets the sun's direction. Without one, the sky stays black.
///
/// Each window's sky subscribes to lights when the renderer adopts the window, at the end of the tick that creates it,
/// and doesn't see lights created before then. Create the window in one tick and the light in a later one.
pub fn setup_atmosphere_sky_render_pass(application: &mut GraphicsApplication) {
	// Keep producer handles in the sink factory instead of template listeners, which would retain unread broadcast messages.
	let light_factory = application.world().factory::<DirectionalLight>();
	let transform_channel = application.world().transforms_channel().clone();
	let renderer = &mut application.renderer;

	renderer.set_scene_background_for_all_sinks(move |render_pass_builder, targets| {
		Box::new(AtmosphereSkyRenderPass::new(
			render_pass_builder,
			targets,
			light_factory.listener(),
			transform_channel.listener(),
		))
	});
}
