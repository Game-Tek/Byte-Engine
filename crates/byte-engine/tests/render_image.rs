//! Renders a sink into a render image instead of a window on the active native GHI backend.
//!
//! These tests need a GPU, so the default nextest filter skips this binary. Run them with
//! `cargo nextest run -p byte-engine --test render_image --ignore-default-filter`.

#![cfg(feature = "headed")]

use std::sync::{Arc, Mutex};

use byte_engine::{
	application::{Parameter, graphics::GraphicsApplication},
	core::{Creator as _, channel::Channel as _, listener::Listener},
	rendering::{
		Camera, DirectionalLight, LightColor, PhotometricIntensity, PipelineManager, RenderImage, RenderImageReady, SceneId,
		SetRenderImageExtent, ShownTransforms, Sink,
		render_pass::{
			EnvironmentMaps, RenderFeature, RenderPass, RenderPassBuilder, RenderPassReturn, SceneBackground,
			SceneBackgroundTargets, SinkScope, allocate_render_command,
		},
	},
};
use ghi::{
	command_buffer::{CommandBufferRecording as _, RasterizationRenderPassMode as _},
	context::{Context as _, ContextCreate as _},
};
use utils::{Extent, RGBA};

/// The color the terminal pass fills the sink's final target with: pure green, which reads back the same in any
/// channel order.
const FILL: RGBA = RGBA {
	r: 0.0,
	g: 1.0,
	b: 0.0,
	a: 1.0,
};

/// The color a pass registered for windows alone fills with: pure red, which a render image must never show.
const WINDOW_FILL: RGBA = RGBA {
	r: 1.0,
	g: 0.0,
	b: 0.0,
	a: 1.0,
};

/// The `FillPass` struct ends a sink's graph by filling its final target with a color, [`FILL`] unless it says
/// otherwise, so a readback shows whether the graph wrote the render image.
struct FillPass {
	attachment: ghi::AttachmentInformation,
}

impl FillPass {
	fn filling(builder: &mut RenderPassBuilder, color: RGBA) -> Self {
		let target = builder
			.create_main_render_target(ghi::image::Builder::new(ghi::Formats::BGRAu8, ghi::Uses::RenderTarget).name("fill"));
		Self {
			attachment: ghi::AttachmentInformation::new(
				target,
				ghi::Layouts::RenderTarget,
				ghi::LoadOp::Clear(ghi::ClearValue::Color(color)),
				ghi::StoreOp::Store,
			),
		}
	}
}

/// The `Fill` struct registers a [`FillPass`] that fills with `color` for every sink it reaches.
struct Fill {
	color: RGBA,
}

impl RenderFeature for Fill {
	type Pass = FillPass;

	fn name(&self) -> &'static str {
		"fill"
	}

	fn create_pass(&mut self, builder: &mut RenderPassBuilder) -> FillPass {
		FillPass::filling(builder, self.color)
	}
}

impl RenderPass<Fill> for FillPass {
	fn prepare<'a>(
		&mut self,
		_feature: &mut Fill,
		_frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		let extent = sink.extent();
		let attachments = [self.attachment];
		Some(allocate_render_command(frame_allocator, move |recording| {
			recording.start_render_pass(extent, &attachments).end_render_pass();
		}))
	}
}

/// Creates an application named `name` with a resource store and an inspector port of its own, since every test runs
/// in a process of its own and both open in one process at a time.
fn application(name: &str) -> GraphicsApplication {
	let resources = std::env::temp_dir().join(format!("byte-engine-{name}-{}", std::process::id()));
	std::fs::create_dir_all(&resources).expect("The test's resource directory could not be created.");
	// Port 0 has the system pick a free inspector port as it binds, so tests running at once never race for one.
	let parameters = [
		Parameter::new("resources.path", &resources.to_string_lossy()),
		Parameter::new("inspector.port", "0"),
	];
	GraphicsApplication::new(name, &parameters)
}

/// Ticks `application` until the renderer names the image of a render image, and returns it.
fn wait_for_image(application: &mut GraphicsApplication, ready: &mut impl Listener<RenderImageReady>) -> RenderImageReady {
	for _ in 0..16 {
		application.tick();
		if let Some(ready) = ready.read() {
			return ready;
		}
	}
	panic!(
		"The renderer never answered with `RenderImageReady`. The most likely cause is that render images are not drained before the frame."
	);
}

/// Copies `image` to the CPU and returns its extent and the first pixel's bytes.
fn read_back(application: &mut GraphicsApplication, image: ghi::ImageHandle) -> (Extent, [u8; 4]) {
	let renderer = application.renderer_mut();
	let queue = renderer.graphics_queue();
	let context = renderer.context_mut();
	let transfer = context.record(queue, Some("Render image readback"), |recording| {
		recording.transfer_texture(image.into()).expect(
			"Render image transfer failed. The most likely cause is that the image was not created as a transfer source.",
		)
	});
	let readback = context
		.get_image_data(transfer)
		.expect("Render image readback failed. The most likely cause is that the transfer was not recorded by this context.");
	assert_eq!(readback.format, ghi::Formats::BGRAu8);
	let pixel = readback.bytes[..4].try_into().expect("A BGRA pixel has four bytes.");
	(readback.extent, pixel)
}

#[test]
fn a_render_image_holds_its_graph_output_and_follows_its_extent() {
	let mut application = application("render-image-test");
	application
		.renderer_mut()
		.add_render_pass(Fill { color: FILL }, SinkScope::All);
	let mut ready = application.messages().channel::<RenderImageReady>().listener();

	let camera: byte_engine::core::factory::Handle = application.world().create(Camera::new()).into();
	let render_image =
		application
			.render_image_factory()
			.create(RenderImage::new(SceneId::WORLD, Extent::rectangle(64, 32), camera));

	let answer = wait_for_image(&mut application, &mut ready);
	assert_eq!(answer.render_image(), render_image);
	// The sink builds its passes once the fusion catalog has loaded on its own thread, a few ticks later.
	let mut frame = (Extent::rectangle(0, 0), [0; 4]);
	for _ in 0..256 {
		application.tick();
		frame = read_back(&mut application, answer.image());
		if frame.1 == [0, 255, 0, 255] {
			break;
		}
	}
	let (extent, pixel) = frame;
	assert_eq!(extent, Extent::rectangle(64, 32));
	assert_eq!(pixel, [0, 255, 0, 255]);

	application
		.messages()
		.channel()
		.send(SetRenderImageExtent::new(render_image, Extent::rectangle(48, 24)));
	application.tick();
	application.tick();
	let (extent, pixel) = read_back(&mut application, answer.image());
	assert_eq!(extent, Extent::rectangle(48, 24));
	assert_eq!(pixel, [0, 255, 0, 255]);
}

#[test]
fn a_render_image_leaves_out_the_passes_registered_for_windows() {
	let mut application = application("render-image-windows-test");
	let renderer = application.renderer_mut();
	renderer.add_render_pass(Fill { color: FILL }, SinkScope::All);
	// Last in every window, as an interface drawn over the game is.
	renderer.add_render_pass(Fill { color: WINDOW_FILL }, SinkScope::Windows);
	let mut ready = application.messages().channel::<RenderImageReady>().listener();

	let camera: byte_engine::core::factory::Handle = application.world().create(Camera::new()).into();
	application
		.render_image_factory()
		.create(RenderImage::new(SceneId::WORLD, Extent::rectangle(16, 16), camera));

	let answer = wait_for_image(&mut application, &mut ready);
	// The pass registered for all sinks becomes the image's last pass, so the image takes its color.
	let mut pixel = [0; 4];
	for _ in 0..256 {
		application.tick();
		pixel = read_back(&mut application, answer.image()).1;
		if pixel != [0; 4] {
			break;
		}
	}
	assert_eq!(
		pixel,
		[0, 255, 0, 255],
		"A pass registered for windows drew into the render image."
	);
}

/// What one scene pipeline saw: the sinks it built state for, and each sink it prepared with whether it carried a sun.
#[derive(Default)]
struct Seen {
	created: Vec<usize>,
	prepared: Vec<(usize, bool)>,
}

/// The `RecordingPipeline` struct stands in for a scene pipeline and records which sinks the renderer hands it, so a
/// test can tell which scene each sink belongs to.
struct RecordingPipeline {
	seen: Arc<Mutex<Seen>>,
}

impl PipelineManager for RecordingPipeline {
	fn prepare<'a>(
		&'a mut self,
		_frame: &mut ghi::implementation::Frame,
		sinks: &[Sink],
		_frame_allocator: &'a bumpalo::Bump,
		_transforms: &ShownTransforms,
		_alley: &mut byte_engine::core::alley::Alley,
		_time: byte_engine::time::MediaTime,
		_background: &mut byte_engine::rendering::SceneBackgroundFrame<'_>,
	) -> smallvec::SmallVec<[(usize, RenderPassReturn<'a>); 16]> {
		let mut seen = self.seen.lock().unwrap();
		seen.prepared
			.extend(sinks.iter().map(|sink| (sink.index(), sink.sun().is_some())));
		smallvec::SmallVec::new()
	}

	fn create_sink(&mut self, sink_id: usize, _render_pass_builder: &mut RenderPassBuilder) {
		self.seen.lock().unwrap().created.push(sink_id);
	}
}

/// Installs a recording scene pipeline for `scene` and returns what it sees.
fn record(application: &mut GraphicsApplication, scene: SceneId) -> Arc<Mutex<Seen>> {
	let seen = Arc::new(Mutex::new(Seen::default()));
	let pipeline = RecordingPipeline { seen: seen.clone() };
	application.renderer_mut().add_pipeline_manager(scene, pipeline);
	seen
}

#[test]
fn a_scene_reaches_only_its_own_render_images() {
	let mut application = application("render-image-scenes-test");
	let preview = application.create_scene("preview");
	let world_pipeline = record(&mut application, SceneId::WORLD);
	let preview_pipeline = record(&mut application, preview);
	let mut ready = application.messages().channel::<RenderImageReady>().listener();

	let sun = DirectionalLight::new(LightColor::kelvin(6000.0), PhotometricIntensity::illuminance(100_000.0)).unwrap();
	let stage = application.scene_messages(preview).clone();
	stage.create(sun);
	let world_camera: byte_engine::core::factory::Handle = application.world().create(Camera::new()).into();
	let preview_camera: byte_engine::core::factory::Handle = stage.create(Camera::new()).into();
	let extent = Extent::rectangle(16, 16);
	let world_image = application
		.render_image_factory()
		.create(RenderImage::new(SceneId::WORLD, extent, world_camera));
	let preview_image = application
		.render_image_factory()
		.create(RenderImage::new(preview, extent, preview_camera));

	// Both answers come in the same tick, in creation order: the world's image is sink 0 and the preview's sink 1.
	let first = wait_for_image(&mut application, &mut ready);
	let second = ready
		.read()
		.expect("The renderer answers every render image created in one tick together.");
	assert_eq!([first.render_image(), second.render_image()], [world_image, preview_image]);
	// Sinks build their scene pipelines once the fusion catalog has loaded on its own thread, a few ticks later.
	let prepared = |seen: &Arc<Mutex<Seen>>| {
		let seen = seen.lock().unwrap();
		!seen.created.is_empty() && !seen.prepared.is_empty()
	};
	for _ in 0..256 {
		if prepared(&world_pipeline) && prepared(&preview_pipeline) {
			break;
		}
		application.tick();
	}

	let world = world_pipeline.lock().unwrap();
	let staged = preview_pipeline.lock().unwrap();
	assert_eq!(world.created, [0]);
	assert_eq!(staged.created, [1]);
	assert!(!world.prepared.is_empty(), "The world's pipeline never prepared a frame.");
	assert!(!staged.prepared.is_empty(), "The preview's pipeline never prepared a frame.");
	assert!(world.prepared.iter().all(|&seen| seen == (0, false)));
	assert!(staged.prepared.iter().all(|&seen| seen == (1, true)));
}

/// The `IdlePass` struct stands in for the passes of a scene background, recording nothing.
struct IdlePass;

/// The `IdleBackground` struct is a scene background that draws nothing and counts the environments it builds, each
/// with two small cubes of its own.
struct IdleBackground {
	built: Arc<Mutex<u32>>,
}

impl SceneBackground for IdleBackground {
	type Pass = IdlePass;
	type Environment = IdlePass;

	fn name(&self) -> &'static str {
		"idle background"
	}

	fn create_pass(&mut self, _builder: &mut RenderPassBuilder, _targets: SceneBackgroundTargets) -> IdlePass {
		IdlePass
	}

	fn create_environment(&mut self, builder: &mut RenderPassBuilder) -> Option<(IdlePass, EnvironmentMaps)> {
		*self.built.lock().unwrap() += 1;
		let mut cube = |name: &str| {
			builder.context().build_image(
				ghi::image::Builder::new(ghi::Formats::RGBA16F, ghi::Uses::Image | ghi::Uses::Storage)
					.name(name)
					.extent(Extent::square(1))
					.cube_compatible(),
			)
		};
		let maps = EnvironmentMaps {
			diffuse: cube("diffuse").into(),
			specular: cube("specular").into(),
		};
		Some((IdlePass, maps))
	}
}

impl RenderPass<IdleBackground> for IdlePass {
	fn prepare<'a>(
		&mut self,
		_feature: &mut IdleBackground,
		_frame: &mut ghi::implementation::Frame,
		_sink: &Sink,
		_frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		None
	}
}

#[test]
fn each_scene_generates_its_own_environment() {
	let mut application = application("render-image-environments-test");
	let preview = application.create_scene("preview");
	let world_pipeline = record(&mut application, SceneId::WORLD);
	let preview_pipeline = record(&mut application, preview);
	// Counts the environments the renderer builds.
	let built = Arc::new(Mutex::new(0));
	let counted = built.clone();
	application
		.renderer_mut()
		.set_scene_background(IdleBackground { built: counted });
	let mut ready = application.messages().channel::<RenderImageReady>().listener();

	// Two sinks show the world and one the preview.
	let stage = application.scene_messages(preview).clone();
	let extent = Extent::rectangle(16, 16);
	for _ in 0..2 {
		let camera: byte_engine::core::factory::Handle = application.world().create(Camera::new()).into();
		application
			.render_image_factory()
			.create(RenderImage::new(SceneId::WORLD, extent, camera));
	}
	let preview_camera: byte_engine::core::factory::Handle = stage.create(Camera::new()).into();
	application
		.render_image_factory()
		.create(RenderImage::new(preview, extent, preview_camera));
	wait_for_image(&mut application, &mut ready);
	for _ in 0..256 {
		if world_pipeline.lock().unwrap().created.len() == 2 && !preview_pipeline.lock().unwrap().created.is_empty() {
			break;
		}
		application.tick();
	}

	assert_eq!(
		*built.lock().unwrap(),
		2,
		"The renderer builds one environment for each scene, not for each sink."
	);
}
