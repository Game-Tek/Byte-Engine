//! Graphics pipeline and render-pass setup.

use super::*;
use crate::rendering::DirectionalLight;

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
	let factory = application.world().factory::<rendering::DebugMesh>();
	// Register future-only lifecycle listeners before returning the producer factory.
	let listener = factory.listener();
	let delete_listener = application.world().deletions_listener();
	let scene = std::rc::Rc::new(std::cell::RefCell::new(rendering::DebugSceneManager::new(
		application.renderer.context_mut(),
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
/// [`rendering::loading`]. It creates one loader lane and a Simple-owned store
/// whose position and index streams intentionally differ from Visibility
/// storage. The application's [`Loader`](rendering::loading::Loader) uploads the
/// lane's meshes. Simple's shaders use the renderer's asynchronous pipeline
/// compilation servers; this setup never waits for shader resources.
///
/// The lane runs on the loading thread, so start that thread with
/// [`defaults::launch_deferred_tasks_thread`] after every subsystem has
/// registered its work. That thread also runs the loader.
pub fn setup_simple_render_pipeline(application: &mut GraphicsApplication) {
	let application_resources = application.resource_manager.clone();

	let (loader, renderer) = application.loader_and_renderer_mut();
	let pipeline_compiler = renderer.pipeline_manager_client();
	let resource_store = std::sync::Arc::new(std::sync::Mutex::new(
		rendering::pipelines::simple::resource_manager::SimpleResourceStore::new(renderer.context_mut()),
	));
	let (simple_loader, simple_loader_lanes) = rendering::pipelines::simple::resource_manager::SimpleLoader::spawn(
		loader,
		renderer.context_mut(),
		application_resources,
		resource_store.clone(),
	);

	let pipeline_manager = SimplePipelineManager::new(
		application.renderer.context_mut(),
		&application.world,
		pipeline_compiler,
		simple_loader,
		&resource_store,
	);
	application.renderer.add_pipeline_manager(pipeline_manager);
	run_on_loading_thread(application, simple_loader_lanes.into_iter().map(|lane| lane.run()));
}

/// Installs the visibility-buffer PBR scene pipeline and its loader lanes.
///
/// Visibility loads meshes, materials, and textures through one lane pool. A
/// mesh load requests its materials as soon as it reads their names, and a
/// material load requests its textures the same way, so all three load at
/// once. Mesh loads append parallel geometry streams, material loads assign
/// table slots, and texture loads finish CPU or native GPU-I/O uploads before
/// publishing residency. These choices belong to Visibility; they are not
/// requirements of the shared loader.
///
/// Every loader lane runs on the loading thread, which
/// [`defaults::launch_deferred_tasks_thread`] starts together with the
/// application's [`Loader`](rendering::loading::Loader).
///
/// Next, create an [`Environment`] through
/// [`DefaultWorld::factory`] to select the HDR image used for ambient and
/// specular reflections.
pub fn setup_pbr_visibility_shading_render_pipeline(application: &mut GraphicsApplication) {
	use crate::rendering::pipelines::visibility::{
		CONTACT_SHADOWS_CONFIGURATION_PREFIX, GTAO_CONFIGURATION_PREFIX, SSGI_CONFIGURATION_PREFIX,
	};

	let visibility_pipeline_settings = visibility_pipeline_settings(application);
	// Each port subscribes before its startup parameters are queued, so it reports every one of them.
	let [gtao_configuration, ssgi_configuration, contact_shadow_configuration] = [
		GTAO_CONFIGURATION_PREFIX,
		SSGI_CONFIGURATION_PREFIX,
		CONTACT_SHADOWS_CONFIGURATION_PREFIX,
	]
	.map(|prefix| {
		let port = application.configuration.register(prefix);
		super::queue_startup_parameters(application.application.parameters(), &application.configuration, prefix);
		port
	});

	let application_resource_manager = application.resource_manager.clone();
	let (loader, renderer) = application.loader_and_renderer_mut();
	let pipeline_manager = renderer.pipeline_manager_client();

	let geometry = rendering::pipelines::visibility::GeometryHandles::new(
		renderer.context_mut(),
		visibility_pipeline_settings.geometry_capacity(),
	);

	let (visibility_loader, visibility_loader_lanes) = rendering::pipelines::visibility::spawn_loader(
		loader,
		renderer.context_mut(),
		application_resource_manager,
		&geometry,
		pipeline_manager.clone(),
	);

	run_on_loading_thread(application, visibility_loader_lanes.into_iter().map(|lane| lane.run()));

	let visibility_pipeline_manager = VisibilityPipelineManager::new(
		application.renderer.context_mut(),
		&application.world,
		geometry,
		visibility_loader,
		pipeline_manager,
		gtao_configuration,
		ssgi_configuration,
		contact_shadow_configuration,
		visibility_pipeline_settings,
	);
	application.renderer.add_pipeline_manager(visibility_pipeline_manager);
}

/// Installs the GPU particle runtime on top of the scene pipeline registered before it.
///
/// Call this after the scene pipeline setup, such as [`setup_pbr_visibility_shading_render_pipeline`], and before
/// [`defaults::launch_deferred_tasks_thread`], which runs the lane that loads particle systems. Particles add their
/// light to the scene's `main` color before post-processing, so bloom and tone mapping treat them like any other
/// emitter. A system costs nothing on frames without live particles.
///
/// Next, create a [`rendering::ParticleEmitter`] naming a `.particles` asset, with a [`crate::gameplay::Transform`],
/// through [`DefaultWorld::create`].
pub fn setup_particles(application: &mut GraphicsApplication) {
	let resources = application.resource_manager_handle();
	let (loader, renderer) = application.loader_and_renderer_mut();
	let pipeline_manager = renderer.pipeline_manager_client();
	// Systems are small records read once each, so one lane keeps up.
	let (systems_loader, lanes) =
		rendering::loading::spawn(loader, rendering::particles::ParticleSystemLoader { resources }, 1, 16);
	let particle_manager = rendering::particles::ParticleManager::new(&application.world, pipeline_manager, systems_loader);
	application.renderer.add_pipeline_manager(particle_manager);
	run_on_loading_thread(application, lanes.into_iter().map(|lane| lane.run()));
}

/// Runs every loader lane as a task on the loading thread that [`defaults::launch_deferred_tasks_thread`] starts.
fn run_on_loading_thread<F: Future<Output = ()> + 'static>(
	application: &mut GraphicsApplication,
	lanes: impl IntoIterator<Item = F> + Send + 'static,
) {
	application.add_deferred_task(move |runtime| lanes.into_iter().for_each(|lane| runtime.spawn(lane).detach()));
}

/// Resolves the visibility pipeline's startup parameters, panicking on any value it cannot use.
fn visibility_pipeline_settings(application: &GraphicsApplication) -> VisibilityPipelineSettings {
	// Every setting below rejects a value it cannot parse instead of silently keeping its default.
	fn parse<T: std::str::FromStr>(application: &GraphicsApplication, name: &str) -> Option<T>
	where
		T::Err: std::fmt::Display,
	{
		application
			.get_parameter(name)
			.map(|parameter| parameter.parse().unwrap_or_else(|error| panic!("{error}")))
	}
	let mut settings = VisibilityPipelineSettings::default();
	if let Some(capacity) = parse(application, CONE_SHADOW_MAP_POOL_CAPACITY_PARAMETER) {
		settings = settings
			.with_cone_shadow_map_pool_capacity(capacity)
			.unwrap_or_else(|reason| panic!("{reason}"));
	}
	if let Some(capacity) = parse(application, POINT_SHADOW_MAP_POOL_CAPACITY_PARAMETER) {
		settings = settings
			.with_point_shadow_map_pool_capacity(capacity)
			.unwrap_or_else(|reason| panic!("{reason}"));
	}
	// Geometry capacity: each parameter overrides one scene-wide geometry buffer's element count.
	let mut geometry_capacity = settings.geometry_capacity();
	for (stream, capacity) in [
		("vertex", &mut geometry_capacity.vertices),
		("vertex-index", &mut geometry_capacity.vertex_indices),
		("triangle", &mut geometry_capacity.triangles),
		("meshlet", &mut geometry_capacity.meshlets),
		("skinning-vertex", &mut geometry_capacity.skinning_vertices),
	] {
		if let Some(value) = parse(application, &format!("{GEOMETRY_CAPACITY_PARAMETER_PREFIX}{stream}-capacity")) {
			*capacity = value;
		}
	}
	settings = settings
		.with_geometry_capacity(geometry_capacity)
		.unwrap_or_else(|reason| panic!("{reason}"));
	// Directional shadow coverage: each parameter overrides one part of the default splits.
	let default_splits = settings.cascade_splits();
	let shadow_distance = parse(application, DIRECTIONAL_SHADOW_DISTANCE_PARAMETER).unwrap_or(default_splits.distance());
	let split_blend =
		parse(application, DIRECTIONAL_SHADOW_SPLIT_BLEND_PARAMETER).unwrap_or(default_splits.logarithmic_share());
	settings = settings.with_cascade_splits(
		crate::rendering::csm::CascadeSplits::new(shadow_distance, split_blend).unwrap_or_else(|reason| panic!("{reason}")),
	);
	if let Some(fitting) = parse(application, DIRECTIONAL_SHADOW_FITTING_PARAMETER) {
		settings = settings.with_cascade_fitting(fitting);
	}
	if let Some(resolution) = parse(application, DIRECTIONAL_SHADOW_RESOLUTION_PARAMETER) {
		settings = settings
			.with_directional_shadow_map_resolution(resolution)
			.unwrap_or_else(|reason| panic!("{reason}"));
	}
	settings
}

/// Installs the retained UI render pass fed by UI render messages from `ui`.
///
/// Register this pass before publishing renders that every sink must observe.
/// The source subscribes immediately and retains the latest render for sinks
/// initialized later. `font` is the file text is drawn with, or `None` for a system font;
/// pass the same file to [`crate::ui::Engine::with_font`] so layout and drawing agree.
pub fn setup_ui_render_pass(application: &mut GraphicsApplication, ui: &Factory<Render>, font: Option<&std::path::Path>) {
	let font = font.map(std::path::Path::to_path_buf);
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
	use crate::ui::{Context, ElementContext, Engine, Size};

	/// Creates an engine whose root renders on every evaluation, so each test controls what it publishes.
	fn rendering_engine() -> Engine {
		let mut engine = Engine::new();
		engine.mount(async move |ctx| {
			let _root = ctx.element("root").container(|c| c).await;
			loop {
				ctx.render().await;
			}
		});
		engine
	}

	#[test]
	fn republished_unchanged_render_is_not_adopted_again() {
		let factory = Factory::new();
		let mut source = UiRenderSource::new(factory.listener());
		let mut engine = rendering_engine();
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
		let mut engine = rendering_engine();
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
		let mut engine = rendering_engine();
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
		while let Some(message) = self.listener.read() {
			newest = Some(message.into_data());
		}
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

/// Installs the AGX tonemapping pass for post-scene color mapping.
///
/// Register it before creating a window; use `render.pass.agx` to enable or bypass it at runtime.
pub fn setup_agx_tonemap_render_pass(application: &mut GraphicsApplication) {
	setup_image_transform_render_pass(application, &rendering::render_passes::agx::TONE_MAPPING);
}

/// Installs display sRGB encoding without applying a tone-mapping curve.
///
/// Use this as the final post-scene pass for SDR scenes whose colors already
/// fit in the display range. Omit this setup when a tone mapper or color-grading
/// pass already produces display-encoded output. Register it before creating a
/// window; use `render.pass.srgb-display` to enable or bypass it at runtime.
pub fn setup_srgb_display_render_pass(application: &mut GraphicsApplication) {
	setup_image_transform_render_pass(application, &rendering::render_passes::srgb_display::ENCODING);
}

/// Installs the ACES v1 tonemapping pass for post-scene color mapping.
///
/// Register it before creating a window; use `render.pass.aces` to enable or bypass it at runtime.
pub fn setup_aces_tonemap_render_pass(application: &mut GraphicsApplication) {
	setup_image_transform_render_pass(application, &rendering::render_passes::aces::TONE_MAPPING);
}

/// Starts one image transform's shaders and installs the transform for every render sink.
fn setup_image_transform_render_pass(
	application: &mut GraphicsApplication,
	configuration: &'static rendering::render_passes::image_transform::Configuration,
) {
	ImageTransformPass::request_pipelines(&application.renderer.pipeline_manager_client(), configuration);
	application
		.renderer
		.add_post_scene_render_pass_for_all_sinks(move |render_pass_builder| {
			Box::new(ImageTransformPass::new(render_pass_builder, configuration))
		});
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
	setup_lut_workflow_render_pass(application, lut, LutWorkflow::Aces);
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
	setup_lut_workflow_render_pass(application, lut, LutWorkflow::DaVinciWideGamut);
}

/// Installs a 3D LUT grading pass from asynchronously prepared resource data.
///
/// Load the resource once with
/// [`crate::rendering::render_passes::lut::PreparedLut::load`] on
/// application-owned asynchronous work. Each sink receives its own copy of the
/// bytes, which its pass drops after the first upload. Call this after passes that
/// produce the HDR `main` target and before tone mapping.
pub fn setup_lut_render_pass(application: &mut GraphicsApplication, lut: crate::rendering::render_passes::lut::PreparedLut) {
	setup_lut_workflow_render_pass(application, lut, LutWorkflow::Creative);
}

/// Installs one LUT workflow for every render sink.
fn setup_lut_workflow_render_pass(
	application: &mut GraphicsApplication,
	lut: crate::rendering::render_passes::lut::PreparedLut,
	workflow: LutWorkflow,
) {
	application
		.renderer
		.add_post_scene_render_pass_for_all_sinks(move |render_pass_builder| {
			Box::new(LutPass::new(render_pass_builder, workflow, lut.clone()))
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

/// Installs exponential height fog for every current and future render sink.
///
/// The fog hides distant surfaces and the horizon behind mist that thins with altitude, lit by the sky and the
/// newest [`DirectionalLight`]. Call it after the scene pipeline and [`setup_particles`], and before
/// [`setup_lens_flare_render_pass`], [`setup_bloom_render_pass`], and tone mapping, so bright light glows through the
/// fog. Use `render.pass.exponential-height-fog` to enable or bypass it at runtime.
///
/// The pass draws nothing until a fog exists. Next, create an [`rendering::ExponentialHeightFog`] through
/// [`DefaultWorld::factory`]. Like the atmosphere sky, each window's fog subscribes to fogs and lights when the
/// renderer adopts the window, so create them in a tick after the one that creates the window.
pub fn setup_exponential_height_fog_render_pass(application: &mut GraphicsApplication) {
	// Keep producer handles in the sink factory instead of template listeners, which would retain unread broadcast messages.
	let fog_factory = application.world().factory::<rendering::ExponentialHeightFog>();
	let light_factory = application.world().factory::<DirectionalLight>();
	let transform_channel = application.world().transforms_channel().clone();

	application
		.renderer
		.add_post_scene_render_pass_for_all_sinks(move |render_pass_builder| {
			Box::new(rendering::render_passes::height_fog::ExponentialHeightFogRenderPass::new(
				render_pass_builder,
				fog_factory.listener(),
				light_factory.listener(),
				transform_channel.listener(),
			))
		});
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
