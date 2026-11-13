use std::{cell::Cell, rc::Rc};

use ghi::{command_buffer::CommonCommandBufferMode as _, context::ContextCreate as _, frame::Frame as _};
use math::{Point, UnitVector, inverse};
use maths_rs::Vec3f;
use utils::Extent;

use crate::{
	configuration::{Accepts, ConfigurationPort, Setting},
	core::Entity,
	rendering::{
		DirectionalLight, Sink, Sun,
		render_pass::{
			RenderPass, RenderPassBuilder, RenderPassReturn, SceneBackgroundTargets, allocate_render_command, simple_compute,
		},
	},
};

const TRANSMITTANCE_LUT_EXTENT: Extent = Extent::rectangle(256, 64);
const SKY_VIEW_LUT_EXTENT: Extent = Extent::square(256);
/// Light scattered more than once varies smoothly with altitude and sun angle, so a small LUT holds it.
const MULTIPLE_SCATTERING_LUT_EXTENT: Extent = Extent::square(32);
/// Half-float render targets overflow above 65,504. Even after exposure, the sun disk's physical radiance can pass that,
/// so it's capped at half the limit, which leaves room for passes that add samples together.
const SUN_DISK_MAX_RADIANCE: f32 = 32_768.0;

/// Returns the top `screen_height` of `extent`'s rows, rounded up, for the sky composite to dispatch over.
///
/// Each composite thread writes its own pixel, so dispatching only these rows launches only the workgroups that
/// cover the sky.
fn sky_composite_extent(extent: Extent, screen_height: f32) -> Extent {
	let rows = (extent.height() as f32 * screen_height.clamp(0.0, 1.0)).ceil() as u32;
	Extent::rectangle(extent.width(), rows)
}

/// Returns the radiance of a sun disk that delivers `illuminance` from a disk of `angular_radius` radians.
///
/// A disk's radiance is its illuminance divided by its solid angle, `π r²`, so the disk brightens with the sun light.
/// Pass exposed illuminance: each channel is capped at [`SUN_DISK_MAX_RADIANCE`] so the disk stays finite in
/// half-float render targets.
fn sun_disk_radiance(illuminance: Vec3f, angular_radius: f32) -> Vec3f {
	let solid_angle = std::f32::consts::PI * angular_radius * angular_radius;
	let radiance = |channel: f32| (channel / solid_angle).min(SUN_DISK_MAX_RADIANCE);
	Vec3f::new(radiance(illuminance.x), radiance(illuminance.y), radiance(illuminance.z))
}

/// The `AtmosphereSkyRenderPassSettings` struct configures the physical atmosphere and sun disk for the sky pass.
///
/// The sun's brightness, color, direction, and disk size come from the scene's [`DirectionalLight`], not from these
/// settings.
#[derive(Clone, Copy, Debug)]
pub struct AtmosphereSkyRenderPassSettings {
	pub ground_radius: f32,
	pub atmosphere_radius: f32,
	pub rayleigh_scale_height: f32,
	pub mie_scale_height: f32,
	pub mie_anisotropy: f32,
	/// How much haze the air holds: the aerosol optical depth looking straight up from the ground. Haze scatters every
	/// color alike and mostly forward, so more of it puts a white glow around the sun and pales the rest of the sky a
	/// little, most near the horizon. The default, `0.05`, is a clear day; `0.02` is very clean air, `0.15` an
	/// average day over land, and `0.4` a hazy day. The `render.sky.haze` parameter changes it at runtime.
	pub haze: f32,
	pub ozone_strength: f32,
	/// The fraction of sunlight the planet's surface reflects back into the atmosphere. Multiple scattering adds that
	/// light to the sky.
	pub ground_albedo: f32,
	pub skip_below_horizon: bool,
	pub planet_center: Point,
	/// The fraction of the screen's height, from the top, that the sky covers, clamped to `0.0..=1.0`. The default,
	/// `0.5`, covers the upper half; use `1.0` for cameras that look up or down. Rows below keep the scene's clear color.
	pub screen_height: f32,
}

impl Default for AtmosphereSkyRenderPassSettings {
	fn default() -> Self {
		Self {
			ground_radius: 6_360_000.0,
			atmosphere_radius: 6_460_000.0,
			rayleigh_scale_height: 8_000.0,
			mie_scale_height: 1_200.0,
			mie_anisotropy: 0.76,
			haze: 0.05,
			ozone_strength: 1.0,
			ground_albedo: 0.3,
			skip_below_horizon: true,
			planet_center: Point::new(0.0, -6_360_000.0, 0.0),
			screen_height: 0.5,
		}
	}
}

impl AtmosphereSkyRenderPassSettings {
	/// The runtime settings, named inside the controls' configuration port.
	pub(crate) const SETTINGS: &'static [Setting<Self>] = &[Setting {
		name: "haze",
		accepts: Accepts::Number { min: 0.0 },
		apply: |settings, read| {
			let haze = read.number();
			Ok(Self {
				haze: haze as f32,
				..settings
			})
		},
	}];
}

/// The `AtmosphereSkyControls` struct shares one set of settings between every sink's [`AtmosphereSkyRenderPass`], so
/// a runtime parameter changes the sky of every window at once.
///
/// Create it with [`Self::new`] from a [`ConfigurationPort`] registered for the prefix the application gives the sky,
/// and hand the same `Rc` to each pass with [`AtmosphereSkyRenderPass::new`].
pub struct AtmosphereSkyControls {
	settings: Cell<AtmosphereSkyRenderPassSettings>,
	configuration: ConfigurationPort,
}

impl AtmosphereSkyControls {
	/// Creates controls that start at `settings` and apply the runtime updates queued on `configuration`.
	pub fn new(settings: AtmosphereSkyRenderPassSettings, configuration: ConfigurationPort) -> Rc<Self> {
		configuration.declare(AtmosphereSkyRenderPassSettings::SETTINGS.iter().map(Setting::parameter));
		Rc::new(Self {
			settings: Cell::new(settings),
			configuration,
		})
	}

	/// Applies the queued runtime updates and returns the settings every pass draws with this frame.
	pub fn settings(&self) -> AtmosphereSkyRenderPassSettings {
		let mut settings = self.settings.get();
		self.configuration
			.drain(&mut settings, AtmosphereSkyRenderPassSettings::SETTINGS);
		self.settings.set(settings);
		settings
	}
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SkyShaderData {
	inverse_view_projection: ghi::pod::Mat4f,
	camera_position: [f32; 4],
	sun_direction: [f32; 4],
	planet_center: [f32; 4],
	atmosphere: [f32; 4],
	misc: [f32; 4],
	haze: [f32; 4],
	sun_illuminance: [f32; 4],
	sun_disk_radiance: [f32; 4],
}

/// The `AtmosphereTables` struct holds the lookup tables one view of the atmosphere reads: the transmittance and
/// multiple-scattering tables, which depend on the air alone, and the sky-view table, which depends on where the view
/// stands and where the sun is. Each user rebuilds them only when their inputs change.
///
/// The sky pass keeps one for its sink's camera, and the sky environment one for the view it captures.
struct AtmosphereTables {
	transmittance_pass: simple_compute::Pass,
	multiple_scattering_pass: simple_compute::Pass,
	sky_view_pass: simple_compute::Pass,
	transmittance_lut: ghi::ImageHandle,
	sky_view_lut: ghi::ImageHandle,
	sampler: ghi::SamplerHandle,
	/// The constants every table pass reads, and the user's own passes with them; the user uploads them each frame.
	parameters: ghi::DynamicBufferHandle<SkyShaderData>,
	/// The GPU counters of the transmittance, multiple scattering, and sky view stages.
	counters: [ghi::CounterHandle; 3],
	/// The haze the transmittance and multiple-scattering tables were built for, or `None` before their first build.
	tables_haze: Option<f32>,
	/// The camera height, as bits, and the direction toward the sun that the sky-view table was built for, or `None`
	/// before its first build.
	sky_view_key: Option<(u32, [f32; 3])>,
}

/// The `ReadyTables` struct is one frame's recording state for [`AtmosphereTables`]: the compiled passes and which
/// tables the frame rebuilds.
#[derive(Clone, Copy)]
struct ReadyTables {
	transmittance_pass: simple_compute::ReadyPass,
	multiple_scattering_pass: simple_compute::ReadyPass,
	sky_view_pass: simple_compute::ReadyPass,
	counters: [ghi::CounterHandle; 3],
	rebuild_transmittance: bool,
	rebuild_sky_view: bool,
}

impl AtmosphereTables {
	/// Creates the tables, with a constants buffer named `parameters_name`, and names their counters
	/// `stage.<counter_prefix>-<table>`.
	fn new(render_pass_builder: &mut RenderPassBuilder, parameters_name: &str, counter_prefix: &str) -> Self {
		let [transmittance_pipeline, multiple_scattering_pipeline, sky_view_pipeline] = [
			("Sky Transmittance LUT", "byte_engine/rendering/sky-transmittance.pipeline"),
			(
				"Sky Multiple Scattering LUT",
				"byte_engine/rendering/sky-multiple-scattering.pipeline",
			),
			("Sky View LUT", "byte_engine/rendering/sky-view.pipeline"),
		]
		.map(|(label, pipeline_id)| {
			simple_compute::Pipeline::compile(render_pass_builder, simple_compute::Descriptor::new(label, pipeline_id))
		});
		let counters = ["transmittance", "multiple-scattering", "view"]
			.map(|name| render_pass_builder.create_gpu_counter(&format!("{counter_prefix}-{name}")));
		let context = render_pass_builder.context();
		let parameters = context.build_dynamic_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name(parameters_name)
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let [transmittance_lut, multiple_scattering_lut, sky_view_lut] = [
			("Sky Transmittance LUT", TRANSMITTANCE_LUT_EXTENT),
			("Sky Multiple Scattering LUT", MULTIPLE_SCATTERING_LUT_EXTENT),
			("Sky View LUT", SKY_VIEW_LUT_EXTENT),
		]
		.map(|(name, extent)| {
			context.build_image(
				ghi::image::Builder::new(crate::rendering::SCENE_COLOR_FORMAT, ghi::Uses::Image | ghi::Uses::Storage)
					.name(name)
					.extent(extent)
					.device_accesses(ghi::DeviceAccesses::DeviceOnly),
			)
		});
		let sampler = context.build_sampler(ghi::sampler::Builder::new());
		// The multiple-scattering and sky-view passes, and the sky's composite, sample the transmittance LUT.
		let transmittance = simple_compute::Resource::combined_image_sampler(
			"transmittance_lut",
			transmittance_lut,
			sampler,
			ghi::Layouts::Read,
		);
		let transmittance_pass = transmittance_pipeline.bind(
			"Sky Transmittance LUT Descriptor Set",
			&[
				simple_compute::Resource::image("transmittance_lut", transmittance_lut),
				simple_compute::Resource::buffer("parameters", parameters),
			],
		);
		let multiple_scattering_pass = multiple_scattering_pipeline.bind(
			"Sky Multiple Scattering LUT Descriptor Set",
			&[
				transmittance,
				simple_compute::Resource::image("multiple_scattering_lut", multiple_scattering_lut),
				simple_compute::Resource::buffer("parameters", parameters),
			],
		);
		let sky_view_pass = sky_view_pipeline.bind(
			"Sky View LUT Descriptor Set",
			&[
				transmittance,
				simple_compute::Resource::image("sky_view_lut", sky_view_lut),
				simple_compute::Resource::buffer("parameters", parameters),
				simple_compute::Resource::combined_image_sampler(
					"multiple_scattering_lut",
					multiple_scattering_lut,
					sampler,
					ghi::Layouts::Read,
				),
			],
		);
		Self {
			transmittance_pass,
			multiple_scattering_pass,
			sky_view_pass,
			transmittance_lut,
			sky_view_lut,
			sampler,
			parameters,
			counters,
			tables_haze: None,
			sky_view_key: None,
		}
	}

	/// Returns the passes for a frame whose air holds `haze` and whose sky-view table is for `data`, and which of the
	/// tables the frame rebuilds, or `None` while a table's pipeline compiles.
	///
	/// Horizontal movement leaves the camera-to-planet vector nearly unchanged at Earth's scale, so only the camera's
	/// height and the sun's direction rebuild the sky-view table. A change of haze rebuilds every table.
	fn prepare(&mut self, frame: &mut ghi::implementation::Frame, haze: f32, data: &SkyShaderData) -> Option<ReadyTables> {
		let transmittance_pass = self.transmittance_pass.ready(frame)?;
		let multiple_scattering_pass = self.multiple_scattering_pass.ready(frame)?;
		let sky_view_pass = self.sky_view_pass.ready(frame)?;
		let sky_view_key = (
			data.camera_position[1].to_bits(),
			[data.sun_direction[0], data.sun_direction[1], data.sun_direction[2]],
		);
		let rebuild_transmittance = self.tables_haze != Some(haze);
		let rebuild_sky_view = rebuild_transmittance || self.sky_view_key != Some(sky_view_key);
		self.tables_haze = Some(haze);
		self.sky_view_key = Some(sky_view_key);
		Some(ReadyTables {
			transmittance_pass,
			multiple_scattering_pass,
			sky_view_pass,
			counters: self.counters,
			rebuild_transmittance,
			rebuild_sky_view,
		})
	}
}

impl ReadyTables {
	/// Records the rebuilds this frame needs. A table times itself only in frames that rebuild it, so a cached table
	/// reports no time.
	fn record(&self, command_buffer: &mut ghi::implementation::CommandBufferRecording) {
		let [transmittance_counter, multiple_scattering_counter, sky_view_counter] = self.counters;
		// Multiple scattering reads the transmittance LUT and depends only on the atmosphere, so both rebuild together.
		if self.rebuild_transmittance {
			command_buffer.counter(transmittance_counter, |command_buffer| {
				self.transmittance_pass.record(command_buffer, TRANSMITTANCE_LUT_EXTENT)
			});
			command_buffer.counter(multiple_scattering_counter, |command_buffer| {
				self.multiple_scattering_pass
					.record(command_buffer, MULTIPLE_SCATTERING_LUT_EXTENT)
			});
		}
		if self.rebuild_sky_view {
			command_buffer.counter(sky_view_counter, |command_buffer| {
				self.sky_view_pass.record(command_buffer, SKY_VIEW_LUT_EXTENT)
			});
		}
	}
}

/// Returns the sink's sun, or a dark one when the scene has no directional light, which leaves the sky black whatever
/// direction the tables take.
fn sink_sun(sink: &Sink) -> Sun {
	sink.sun().unwrap_or(Sun {
		illuminance: Vec3f::new(0.0, 0.0, 0.0),
		angular_radius: DirectionalLight::SUN_ANGULAR_RADIUS,
		direction: UnitVector::z_axis(),
	})
}

/// The `AtmosphereSkyRenderPass` struct fills scene color with an atmosphere wherever opaque depth remains at infinity.
///
/// It is a scene background: scene pipelines record it after opaque surfaces and before transparent ones, so
/// transparent surfaces composite over the sky and scene color needs no coverage channel.
pub struct AtmosphereSkyRenderPass {
	tables: AtmosphereTables,
	composite_pass: simple_compute::Pass,
	/// The GPU counter of the composite, reported as `stage.sky-composite`.
	composite_counter: ghi::CounterHandle,
	controls: Rc<AtmosphereSkyControls>,
	settings: AtmosphereSkyRenderPassSettings,
}

impl Entity for AtmosphereSkyRenderPass {}

impl AtmosphereSkyRenderPass {
	/// Creates a sky pass that draws with the settings `controls` holds.
	///
	/// The sky's sun is the sink's [`Sun`], the scene's newest directional light: its illuminance sets the sky's
	/// brightness and color, and its transform sets the sun's direction. That illuminance is the sunlight that reaches
	/// the camera, as it is for the surfaces the light shines on, so author it as measured on the ground: dimmer and
	/// warmer as the sun gets low. Without a directional light, the sky stays black. The sky writes `targets.color` in
	/// place wherever `targets.depth` holds no surface, within the top
	/// [`AtmosphereSkyRenderPassSettings::screen_height`] of the screen, and leaves every other pixel untouched.
	pub fn new(
		render_pass_builder: &mut RenderPassBuilder,
		targets: SceneBackgroundTargets,
		controls: Rc<AtmosphereSkyControls>,
	) -> Self {
		let SceneBackgroundTargets { color, depth } = targets;
		let composite_pipeline = simple_compute::Pipeline::compile(
			render_pass_builder,
			simple_compute::Descriptor::new("Sky Composite", "byte_engine/rendering/sky.pipeline"),
		);
		let composite_counter = render_pass_builder.create_gpu_counter("sky-composite");
		let tables = AtmosphereTables::new(render_pass_builder, "Sky Render Pass Parameters", "sky");
		let (sampler, parameters) = (tables.sampler, tables.parameters);
		let composite_pass = composite_pipeline.bind(
			"Sky Render Pass Descriptor Set",
			&[
				simple_compute::Resource::combined_image_sampler("depth_texture", depth, sampler, ghi::Layouts::Read),
				simple_compute::Resource::image("result", color),
				simple_compute::Resource::combined_image_sampler(
					"sky_view_lut",
					tables.sky_view_lut,
					sampler,
					ghi::Layouts::Read,
				),
				simple_compute::Resource::combined_image_sampler(
					"transmittance_lut",
					tables.transmittance_lut,
					sampler,
					ghi::Layouts::Read,
				),
				simple_compute::Resource::buffer("parameters", parameters),
			],
		);

		Self {
			tables,
			composite_pass,
			composite_counter,
			settings: controls.settings(),
			controls,
		}
	}
}

/// Returns the share of the haze's scattering that dims and diffuses light, `1 - g²` for a phase function of
/// anisotropy `g`.
///
/// This is the delta-Eddington approximation (Joseph, Wiscombe and Weinman, 1976): the fraction `g²` of the light a
/// forward-scattering medium scatters goes so nearly straight on that it counts as never scattered, and the rest
/// scatters about evenly. The sky's lookup tables assume even scattering, so they take the haze scaled by this.
fn haze_transport_scale(anisotropy: f32) -> f32 {
	1.0 - anisotropy * anisotropy
}

/// Builds one sink's sky constants from the atmosphere settings and the sun, whose illuminance is the sunlight that
/// reaches the camera.
///
/// The sky is written pre-exposed like the scene: the shaders multiply by the uploaded illuminance and disk radiance,
/// so both include the sink's camera exposure.
fn sky_shader_data(settings: &AtmosphereSkyRenderPassSettings, sun: &Sun, sink: &Sink) -> SkyShaderData {
	let Sun {
		illuminance: sun_illuminance,
		angular_radius: sun_angular_radius,
		direction: sun_direction,
	} = *sun;
	let view = sink.view();
	// The translation column of the inverse view is the camera's world position.
	let camera_position = inverse(view.view()).get_column(3);
	let planet_center = settings.planet_center.into_maths();
	let exposed_illuminance = sun_illuminance * sink.exposure_scale();
	// Haze thins with height like `exp(-altitude / mie_scale_height)`, so its optical depth straight up is its
	// scattering per meter at the ground times that height.
	let haze_scattering = settings.haze / settings.mie_scale_height;
	let disk_radiance = sun_disk_radiance(exposed_illuminance, sun_angular_radius.value());

	SkyShaderData {
		inverse_view_projection: inverse(view.view_projection()).into(),
		camera_position: [camera_position.x, camera_position.y, camera_position.z, 0.0],
		sun_direction: [
			sun_direction.x(),
			sun_direction.y(),
			sun_direction.z(),
			settings.mie_anisotropy,
		],
		planet_center: [planet_center.x, planet_center.y, planet_center.z, sun_angular_radius.value()],
		atmosphere: [
			settings.ground_radius,
			settings.atmosphere_radius,
			settings.rayleigh_scale_height,
			settings.mie_scale_height,
		],
		misc: [
			settings.ozone_strength,
			if settings.skip_below_horizon { 1.0 } else { 0.0 },
			settings.ground_albedo,
			0.0,
		],
		haze: [
			haze_scattering,
			haze_scattering * haze_transport_scale(settings.mie_anisotropy),
			0.0,
			0.0,
		],
		sun_illuminance: [exposed_illuminance.x, exposed_illuminance.y, exposed_illuminance.z, 0.0],
		sun_disk_radiance: [disk_radiance.x, disk_radiance.y, disk_radiance.z, 0.0],
	}
}

impl RenderPass for AtmosphereSkyRenderPass {
	fn name(&self) -> &'static str {
		"atmosphere sky"
	}

	fn prepare<'a>(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		self.settings.haze = self.controls.settings().haze;
		let composite_pass = self.composite_pass.ready(frame)?;
		// Per-view sky constants come from the active camera before dispatch.
		let data = sky_shader_data(&self.settings, &sink_sun(sink), sink);
		let tables = self.tables.prepare(frame, self.settings.haze, &data)?;
		*frame.get_mut_dynamic_buffer_slice(self.tables.parameters) = data;
		let composite_extent = sky_composite_extent(sink.extent(), self.settings.screen_height);
		let composite_counter = self.composite_counter;

		Some(allocate_render_command(frame_allocator, move |command_buffer| {
			command_buffer.region(
				|label| label.write_str("Sky"),
				|command_buffer| {
					tables.record(command_buffer);
					// A sky that covers no rows dispatches nothing.
					if composite_extent.height() > 0 {
						command_buffer.counter(composite_counter, |command_buffer| {
							composite_pass.record(command_buffer, composite_extent)
						});
					}
				},
			);
		}))
	}
}

use crate::rendering::set_layout::CUBE_FACES;

/// The face size, in texels, of the sky environment's radiance and specular cubes: the smallest that holds the eight
/// specular levels the visibility pipeline reads, one per roughness step of 1/7.
const SKY_ENVIRONMENT_FACE_SIZE: u32 = 128;
/// The specular cube's levels, and the radiance cube's, which the prefilter reads: as many as a baked environment has.
const SKY_ENVIRONMENT_LEVELS: u32 = crate::rendering::pipelines::visibility::loader::IBL_SPECULAR_LEVEL_COUNT as u32;
/// The face size of the irradiance cube, as the offline bake makes it.
const SKY_ENVIRONMENT_IRRADIANCE_FACE_SIZE: u32 = 8;
/// The Hammersley samples each specular texel takes, and each irradiance texel. The offline bake takes 1,024; filtered
/// importance sampling reads each sample from the pyramid level its solid angle covers, which keeps fewer smooth.
const SKY_ENVIRONMENT_SPECULAR_SAMPLES: u32 = 64;
const SKY_ENVIRONMENT_DIFFUSE_SAMPLES: u32 = 256;

/// The push constant the prefilter shader takes: the level a dispatch fills on every face, the mode, `0` for specular
/// and `1` for irradiance, and the samples per texel.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct EnvironmentDispatch {
	level: u32,
	mode: u32,
	sample_count: u32,
}

/// The `AtmosphereSkyEnvironment` struct generates the scene's environment from the atmosphere sky, so global
/// illumination, reflections, and every other user of the environment see the sky the camera sees.
///
/// [`crate::application::graphics::setup_atmosphere_sky_render_pass`] registers it with
/// [`crate::rendering::Renderer::set_scene_environment`]; scene pipelines light with it while the scene selects no
/// [`crate::rendering::Environment`]. It captures the sky from the first sink's camera height into a radiance cube, in
/// unexposed physical units and without the sun's disk, then prefilters it into the layout a baked environment has. It
/// does all of that in a frame where the sun, the haze, or that height changes, and nothing in the frames between.
pub struct AtmosphereSkyEnvironment {
	tables: AtmosphereTables,
	controls: Rc<AtmosphereSkyControls>,
	settings: AtmosphereSkyRenderPassSettings,
	/// Fills level 0 of the radiance cube. Each pass fills a level of all six faces in one dispatch, a face per
	/// `thread_layer`.
	capture: simple_compute::Pass,
	/// One pass per radiance level after the first, by `level - 1`.
	downsample: [simple_compute::Pass; SKY_ENVIRONMENT_LEVELS as usize - 1],
	/// One pass per specular level.
	specular: [simple_compute::Pass; SKY_ENVIRONMENT_LEVELS as usize],
	/// Fills the irradiance cube.
	diffuse: simple_compute::Pass,
	/// The GPU counters of the capture, with its downsampling, and of the prefilter.
	counters: [ghi::CounterHandle; 2],
	/// The camera height, the sun's direction and illuminance, and the haze, as bits, that the maps hold, or `None`
	/// before their first build.
	key: Option<(u32, [u32; 3], [u32; 3], u32)>,
}

impl Entity for AtmosphereSkyEnvironment {}

impl AtmosphereSkyEnvironment {
	/// Creates the sky environment that draws with the settings `controls` holds, as a scene environment.
	pub fn scene_environment(
		render_pass_builder: &mut RenderPassBuilder,
		controls: Rc<AtmosphereSkyControls>,
	) -> crate::rendering::render_pass::SceneEnvironment {
		let tables = AtmosphereTables::new(render_pass_builder, "Sky Environment Parameters", "sky-environment");
		let [capture_pipeline, downsample_pipeline, prefilter_pipeline] = [
			("Sky Environment Capture", "byte_engine/rendering/sky-environment.pipeline"),
			(
				"Environment Downsample",
				"byte_engine/rendering/environment-downsample.pipeline",
			),
			(
				"Environment Prefilter",
				"byte_engine/rendering/environment-prefilter.pipeline",
			),
		]
		.map(|(label, pipeline_id)| {
			simple_compute::Pipeline::compile(render_pass_builder, simple_compute::Descriptor::new(label, pipeline_id))
		});
		let counters =
			["sky-environment-capture", "sky-environment-prefilter"].map(|name| render_pass_builder.create_gpu_counter(name));
		let context = render_pass_builder.context();
		let mut cube = |name: &str, size: u32, levels: u32| {
			context.build_image(
				ghi::image::Builder::new(ghi::Formats::RGBA16F, ghi::Uses::Image | ghi::Uses::Storage)
					.name(name)
					.extent(Extent::square(size))
					.mip_levels(levels)
					.cube_compatible()
					.device_accesses(ghi::DeviceAccesses::DeviceOnly),
			)
		};
		let radiance = cube("Sky Environment Radiance", SKY_ENVIRONMENT_FACE_SIZE, SKY_ENVIRONMENT_LEVELS);
		let specular = cube("Sky Environment Specular", SKY_ENVIRONMENT_FACE_SIZE, SKY_ENVIRONMENT_LEVELS);
		let irradiance = cube("Sky Environment Irradiance", SKY_ENVIRONMENT_IRRADIANCE_FACE_SIZE, 1);
		// The prefilter reads every radiance level.
		let sampler = context.build_sampler(ghi::sampler::Builder::new().max_lod((SKY_ENVIRONMENT_LEVELS - 1) as f32));
		let radiance_texture =
			simple_compute::Resource::combined_image_sampler("radiance", radiance, sampler, ghi::Layouts::Read);

		let capture = capture_pipeline.bind(
			"Sky Environment Capture Descriptor Set",
			&[
				simple_compute::Resource::combined_image_sampler(
					"sky_view_lut",
					tables.sky_view_lut,
					tables.sampler,
					ghi::Layouts::Read,
				),
				simple_compute::Resource::buffer("parameters", tables.parameters),
				simple_compute::Resource::cube_mip("result", radiance, 0),
			],
		);
		let downsample = std::array::from_fn(|index| {
			let level = index as u32 + 1;
			downsample_pipeline.bind(
				"Environment Downsample Descriptor Set",
				&[
					simple_compute::Resource::cube_mip("source", radiance, level - 1),
					simple_compute::Resource::cube_mip("result", radiance, level),
				],
			)
		});
		let specular_passes = std::array::from_fn(|level| {
			prefilter_pipeline.bind(
				"Environment Prefilter Descriptor Set",
				&[
					radiance_texture,
					simple_compute::Resource::cube_mip("result", specular, level as u32),
				],
			)
		});
		let diffuse = prefilter_pipeline.bind(
			"Environment Prefilter Descriptor Set",
			&[radiance_texture, simple_compute::Resource::cube_mip("result", irradiance, 0)],
		);

		let environment = Self {
			tables,
			settings: controls.settings(),
			controls,
			capture,
			downsample,
			specular: specular_passes,
			diffuse,
			counters,
			key: None,
		};
		crate::rendering::render_pass::SceneEnvironment::new(
			render_pass_builder,
			utils::Box::new(environment),
			irradiance,
			specular,
		)
	}
}

impl RenderPass for AtmosphereSkyEnvironment {
	fn name(&self) -> &'static str {
		"atmosphere sky environment"
	}

	/// Prepares the capture and prefilter in a frame whose inputs differ from those the maps hold, and nothing otherwise.
	fn prepare<'a>(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		self.settings.haze = self.controls.settings().haze;
		let sun = sink_sun(sink);
		// The environment holds physical light, as a baked one does: its readers apply the exposure.
		let mut data = sky_shader_data(&self.settings, &sun, &sink.with_exposure_scale(1.0));
		// The sky barely changes over a meter of height, so the capture follows the camera in whole meters and a camera
		// that bobs doesn't regenerate the environment every frame.
		data.camera_position[1] = data.camera_position[1].round();
		let key = (
			data.camera_position[1].to_bits(),
			[data.sun_direction[0], data.sun_direction[1], data.sun_direction[2]].map(f32::to_bits),
			[data.sun_illuminance[0], data.sun_illuminance[1], data.sun_illuminance[2]].map(f32::to_bits),
			self.settings.haze.to_bits(),
		);
		if self.key == Some(key) {
			return None;
		}
		let capture = self.capture.ready(frame)?;
		let downsample = simple_compute::ready_all(&mut self.downsample, frame, frame_allocator)?;
		let specular = simple_compute::ready_all(&mut self.specular, frame, frame_allocator)?;
		let diffuse = self.diffuse.ready(frame)?;
		let tables = self.tables.prepare(frame, self.settings.haze, &data)?;
		*frame.get_mut_dynamic_buffer_slice(self.tables.parameters) = data;
		self.key = Some(key);
		let [capture_counter, prefilter_counter] = self.counters;

		Some(allocate_render_command(frame_allocator, move |command_buffer| {
			command_buffer.region(
				|label| label.write_str("Sky Environment"),
				|command_buffer| {
					tables.record(command_buffer);
					// Each dispatch fills a level of all six faces, a face per layer of its grid.
					let faces = |size: u32| Extent::new(size, size, CUBE_FACES);
					command_buffer.counter(capture_counter, |command_buffer| {
						capture.record(command_buffer, faces(SKY_ENVIRONMENT_FACE_SIZE));
						// Each level averages the one above it, so the levels build in order.
						for (index, pass) in downsample.iter().enumerate() {
							pass.record(command_buffer, faces(SKY_ENVIRONMENT_FACE_SIZE >> (index + 1)));
						}
					});
					command_buffer.counter(prefilter_counter, |command_buffer| {
						for (level, pass) in specular.iter().enumerate() {
							let dispatch = EnvironmentDispatch {
								level: level as u32,
								mode: 0,
								sample_count: SKY_ENVIRONMENT_SPECULAR_SAMPLES,
							};
							pass.record_with_push_constant(command_buffer, faces(SKY_ENVIRONMENT_FACE_SIZE >> level), dispatch);
						}
						let dispatch = EnvironmentDispatch {
							level: 0,
							mode: 1,
							sample_count: SKY_ENVIRONMENT_DIFFUSE_SAMPLES,
						};
						diffuse.record_with_push_constant(
							command_buffer,
							faces(SKY_ENVIRONMENT_IRRADIANCE_FACE_SIZE),
							dispatch,
						);
					});
				},
			);
		}))
	}
}

#[cfg(test)]
mod tests {
	use besl::vm::{Buffer, DescriptorBindings, ResourceSlot, Value};
	use math::{Point, UnitVector};
	use maths_rs::Vec3f;

	use super::simple_compute;
	use crate::configuration::{ConfigurationValue, apply_setting};
	use crate::rendering::shader_vm_test::{assert_rgba_close, buffer, empty_image, rgba, run_at, slot, texture_2d};

	static SKY_SHADER_BESL: std::sync::LazyLock<&str> =
		std::sync::LazyLock::new(|| crate::rendering::shader_vm_test::asset_source("sky.besl"));
	static SKY_VIEW_SHADER_BESL: std::sync::LazyLock<&str> =
		std::sync::LazyLock::new(|| crate::rendering::shader_vm_test::asset_source("sky-view.besl"));
	static SKY_TRANSMITTANCE_SHADER_BESL: std::sync::LazyLock<&str> =
		std::sync::LazyLock::new(|| crate::rendering::shader_vm_test::asset_source("sky-transmittance.besl"));
	static SKY_MULTIPLE_SCATTERING_SHADER_BESL: std::sync::LazyLock<&str> =
		std::sync::LazyLock::new(|| crate::rendering::shader_vm_test::asset_source("sky-multiple-scattering.besl"));

	/// Sunlight used by tests that don't depend on the sun's brightness.
	const TEST_SUN_ILLUMINANCE: [f32; 3] = [10.0, 10.0, 10.0];

	/// Returns the direction toward a sun high in the sky, for tests that don't depend on where the sun stands.
	fn test_sun_direction() -> UnitVector {
		math::Vector::new(0.35, 0.85, 0.4)
			.normalized()
			.expect("The test sun direction is nonzero.")
	}

	/// Returns a sun of `illuminance` lux toward `direction`, with the real sun's disk.
	fn test_sun(illuminance: [f32; 3], direction: UnitVector) -> crate::rendering::Sun {
		crate::rendering::Sun {
			illuminance: Vec3f::new(illuminance[0], illuminance[1], illuminance[2]),
			angular_radius: crate::rendering::DirectionalLight::SUN_ANGULAR_RADIUS,
			direction,
		}
	}

	/// Uploads the production sky constants for `settings`, `sun`, and a camera with `exposure_scale`, so every shader
	/// test reads exactly what the pass writes.
	fn sky_parameters(
		program: &besl::vm::ExecutableProgram,
		parameter_slot: ResourceSlot,
		settings: &super::AtmosphereSkyRenderPassSettings,
		sun: crate::rendering::Sun,
		exposure_scale: f32,
	) -> Buffer {
		let view = crate::rendering::View::new_perspective(
			math::Degrees::new(60.0),
			1.0,
			0.1,
			100.0,
			Point::origin(),
			UnitVector::z_axis(),
		);
		let sink = crate::rendering::Sink::new(view, utils::Extent::square(1), 0).with_exposure_scale(exposure_scale);
		let data = super::sky_shader_data(settings, &sun, &sink);
		let mut parameters = buffer(program, parameter_slot);
		for (name, value) in [
			(
				"inverse_view_projection",
				Value::Mat4F(bytemuck::cast(data.inverse_view_projection)),
			),
			("camera_position", Value::Vec4F(data.camera_position)),
			("sun_direction", Value::Vec4F(data.sun_direction)),
			("planet_center", Value::Vec4F(data.planet_center)),
			("atmosphere", Value::Vec4F(data.atmosphere)),
			("misc", Value::Vec4F(data.misc)),
			("haze", Value::Vec4F(data.haze)),
			("sun_illuminance", Value::Vec4F(data.sun_illuminance)),
			("sun_disk_radiance", Value::Vec4F(data.sun_disk_radiance)),
		] {
			parameters
				.write(name, value)
				.expect("Failed to initialize sky parameters. The most likely cause is a changed production buffer layout.");
		}
		parameters
	}

	/// Runs one sky-view texel under `settings` and a sun toward `sun_direction`, through air whose transmittance toward
	/// the sun is `sun_transmittance` everywhere, with a multiple-scattering LUT holding `higher_orders`.
	fn run_sky_view(
		settings: &super::AtmosphereSkyRenderPassSettings,
		sun_direction: UnitVector,
		sun_transmittance: [f32; 3],
		higher_orders: [f32; 4],
	) -> [f32; 4] {
		let program = crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(*SKY_VIEW_SHADER_BESL));
		let parameter_slot = slot(&program, "parameters");
		let sun = test_sun(TEST_SUN_ILLUMINANCE, sun_direction);
		let mut parameters = sky_parameters(&program, parameter_slot, settings, sun, 1.0);
		let [r, g, b] = sun_transmittance;
		let mut transmittance = texture_2d(1, 1, &[[r, g, b, 1.0]]);
		let mut multiple_scattering = texture_2d(1, 1, &[higher_orders]);
		let mut output = empty_image(1, 1);
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_texture(slot(&program, "transmittance_lut"), &mut transmittance);
		descriptors.bind_image(slot(&program, "sky_view_lut"), &mut output);
		descriptors.bind_buffer(parameter_slot, &mut parameters);
		descriptors.bind_texture(slot(&program, "multiple_scattering_lut"), &mut multiple_scattering);
		run_at(&program, &mut descriptors, [0, 0]);
		drop(descriptors);
		rgba(&output, [0, 0])
	}

	/// Verifies that the sky-view LUT adds the light from the multiple-scattering LUT to single scattering.
	#[test]
	fn sky_view_adds_light_scattered_more_than_once() {
		let single = run_sky_view(&Default::default(), test_sun_direction(), [1.0; 3], [0.0, 0.0, 0.0, 1.0]);
		let multiple = run_sky_view(&Default::default(), test_sun_direction(), [1.0; 3], [0.01, 0.02, 0.04, 1.0]);

		assert!(
			(0..3).all(|channel| multiple[channel] > single[channel]),
			"Multiple scattering added no light. The most likely cause is that the sky-view LUT ignores its multiple-scattering binding: single {single:?}, with multiple scattering {multiple:?}"
		);
	}

	/// Verifies that haze glows around the sun: it scatters sunlight mostly forward, so a view toward the sun
	/// brightens in every color.
	#[test]
	fn haze_glows_around_the_sun() {
		let sky = |haze| {
			let settings = super::AtmosphereSkyRenderPassSettings {
				haze,
				..Default::default()
			};
			// The lone texel of a 1 × 1 sky-view LUT looks along +Z.
			run_sky_view(&settings, UnitVector::z_axis(), [1.0; 3], [0.0, 0.0, 0.0, 1.0])
		};
		let clean = sky(0.0);
		let hazy = sky(0.1);

		assert!(
			(0..3).all(|channel| hazy[channel] > 2.0 * clean[channel]),
			"Haze didn't glow around the sun. The most likely cause is that the sky-view LUT ignores the haze: clean {clean:?}, hazy {hazy:?}"
		);
	}

	/// Builds the transmittance LUT texel for a sun nearly overhead, seen from the table's lowest altitude.
	fn overhead_sun_transmittance(haze: f32, mie_anisotropy: f32) -> [f32; 4] {
		let program =
			crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(*SKY_TRANSMITTANCE_SHADER_BESL));
		let parameter_slot = slot(&program, "parameters");
		let settings = super::AtmosphereSkyRenderPassSettings {
			haze,
			mie_anisotropy,
			..Default::default()
		};
		let sun = test_sun(TEST_SUN_ILLUMINANCE, test_sun_direction());
		let mut parameters = sky_parameters(&program, parameter_slot, &settings, sun, 1.0);
		let extent = super::TRANSMITTANCE_LUT_EXTENT;
		let mut output = empty_image(extent.width(), extent.height());
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_image(slot(&program, "transmittance_lut"), &mut output);
		descriptors.bind_buffer(parameter_slot, &mut parameters);
		// The last column is the highest sun and the first row the lowest altitude.
		let texel = [extent.width() - 1, 0];
		run_at(&program, &mut descriptors, texel);
		drop(descriptors);
		rgba(&output, texel)
	}

	/// Verifies the delta-Eddington scaling: haze that scatters forward with anisotropy `g` dims the sun like evenly
	/// scattering haze `1 - g²` times as thick, because the light it throws forward still arrives with the sun.
	#[test]
	fn forward_scattering_haze_dims_the_sun_like_thinner_even_haze() {
		let clean = overhead_sun_transmittance(0.0, 0.76);
		let forward = overhead_sun_transmittance(0.1, 0.76);
		let even = overhead_sun_transmittance(0.1 * (1.0 - 0.76 * 0.76), 0.0);

		assert_rgba_close(forward, even, 1e-5);
		assert!(
			(0..3).all(|channel| forward[channel] < clean[channel]),
			"Haze didn't dim the sun. The most likely cause is that the transmittance LUT ignores the haze: clean {clean:?}, hazy {forward:?}"
		);
	}

	/// Verifies that the `haze` runtime parameter takes numbers of 0 or more, as numbers or as startup text, and
	/// refuses anything else.
	#[test]
	fn haze_parameter_takes_numbers_of_zero_or_more() {
		let settings = super::AtmosphereSkyRenderPassSettings::default();
		let set = |value: ConfigurationValue| {
			apply_setting(settings, super::AtmosphereSkyRenderPassSettings::SETTINGS, "", "haze", &value)
		};

		for value in [ConfigurationValue::Float(0.25), ConfigurationValue::Text("0.25".to_string())] {
			let (updated, effective) = set(value).expect("a haze of 0.25 is valid");
			assert_eq!(updated.haze, 0.25);
			assert_eq!(effective, ConfigurationValue::Float(0.25));
		}
		assert_eq!(set(ConfigurationValue::Integer(0)).unwrap().0.haze, 0.0);
		assert!(set(ConfigurationValue::Float(-0.1)).is_err());
		assert!(set(ConfigurationValue::Bool(true)).is_err());
		assert!(
			apply_setting(
				settings,
				super::AtmosphereSkyRenderPassSettings::SETTINGS,
				"",
				"fog",
				&ConfigurationValue::Float(0.1)
			)
			.is_err()
		);
	}

	/// Builds the multiple-scattering LUT texel for a sun straight overhead at ground level, under full transmittance.
	fn overhead_sun_multiple_scattering(ground_albedo: f32) -> [f32; 4] {
		let program = crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(
			*SKY_MULTIPLE_SCATTERING_SHADER_BESL,
		));
		let parameter_slot = slot(&program, "parameters");
		let settings = super::AtmosphereSkyRenderPassSettings {
			ground_albedo,
			..super::AtmosphereSkyRenderPassSettings::default()
		};
		let sun = test_sun(TEST_SUN_ILLUMINANCE, test_sun_direction());
		let mut parameters = sky_parameters(&program, parameter_slot, &settings, sun, 1.0);
		let mut transmittance = texture_2d(1, 1, &[[1.0, 1.0, 1.0, 1.0]]);
		let size = super::MULTIPLE_SCATTERING_LUT_EXTENT.width();
		let mut output = empty_image(size, size);
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_texture(slot(&program, "transmittance_lut"), &mut transmittance);
		descriptors.bind_image(slot(&program, "multiple_scattering_lut"), &mut output);
		descriptors.bind_buffer(parameter_slot, &mut parameters);
		// The last column is a sun zenith cosine of 1 and the first row is ground level.
		run_at(&program, &mut descriptors, [size - 1, 0]);
		drop(descriptors);
		rgba(&output, [size - 1, 0])
	}

	/// Verifies the multiple-scattering LUT holds bounded, Rayleigh-blue light that the ground's reflection adds to.
	#[test]
	fn multiple_scattering_lut_is_bounded_and_grows_with_ground_albedo() {
		let dark_ground = overhead_sun_multiple_scattering(0.0);
		let default_ground = overhead_sun_multiple_scattering(0.3);

		for higher_orders in [dark_ground, default_ground] {
			assert!(
				higher_orders[..3]
					.iter()
					.all(|channel| channel.is_finite() && *channel > 0.0 && *channel < 1.0),
				"Out-of-range multiple scattering. The most likely cause is an unstable scattering series: {higher_orders:?}"
			);
			assert!(
				higher_orders[2] > higher_orders[1] && higher_orders[1] > higher_orders[0],
				"Multiple scattering isn't bluest. The most likely cause is swapped Rayleigh coefficients: {higher_orders:?}"
			);
		}
		assert!(
			(0..3).all(|channel| default_ground[channel] > dark_ground[channel]),
			"The ground added no light. The most likely cause is that the LUT ignores the ground albedo: dark {dark_ground:?}, default {default_ground:?}"
		);
	}

	/// Verifies that surfaces keep their color and that the background receives scene-linear HDR sky.
	#[test]
	fn sky_besl_vm_preserves_surfaces_and_writes_hdr_background() {
		let program = crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(*SKY_SHADER_BESL));
		let sentinel = [0.2, 0.3, 0.4, 0.5];
		let mut surface_depth = texture_2d(1, 1, &[[0.5, 0.0, 0.0, 1.0]]);
		let mut surface_color = texture_2d(1, 1, &[sentinel]);
		let mut surface_descriptors = DescriptorBindings::new();
		surface_descriptors.bind_texture(slot(&program, "depth_texture"), &mut surface_depth);
		surface_descriptors.bind_image(slot(&program, "result"), &mut surface_color);
		run_at(&program, &mut surface_descriptors, [0, 0]);
		drop(surface_descriptors);
		assert_rgba_close(rgba(&surface_color, [0, 0]), sentinel, 0.0);

		let background = composite_background([2.0, 3.0, 4.0, 1.0], TEST_SUN_ILLUMINANCE, 1.0);
		assert!(
			background[..3].iter().all(|channel| channel.is_finite() && *channel > 1.0),
			"Clamped sky VM output. The most likely cause is tone mapping before the final scene tonemap: {background:?}"
		);
	}

	/// Composites one background pixel whose view misses the sun disk, over a sky-view LUT of `sky_scattering` per lux,
	/// through air that lets all of the sunlight reach the camera.
	fn composite_background(sky_scattering: [f32; 4], sun_illuminance: [f32; 3], exposure_scale: f32) -> [f32; 4] {
		composite(
			&Default::default(),
			test_sun_direction(),
			sky_scattering,
			[1.0, 1.0, 1.0],
			sun_illuminance,
			exposure_scale,
		)
	}

	/// Composites one background pixel under `settings` and a sun toward `sun_direction`, over a sky-view LUT of
	/// `sky_scattering` per lux, through air whose transmittance from the camera toward the sun is `sun_transmittance`.
	fn composite(
		settings: &super::AtmosphereSkyRenderPassSettings,
		sun_direction: UnitVector,
		sky_scattering: [f32; 4],
		sun_transmittance: [f32; 3],
		sun_illuminance: [f32; 3],
		exposure_scale: f32,
	) -> [f32; 4] {
		let program = crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(*SKY_SHADER_BESL));
		let parameter_slot = slot(&program, "parameters");
		let sun = test_sun(sun_illuminance, sun_direction);
		let mut parameters = sky_parameters(&program, parameter_slot, settings, sun, exposure_scale);
		let mut sky_view = texture_2d(1, 1, &[sky_scattering]);
		let [r, g, b] = sun_transmittance;
		let mut transmittance = texture_2d(1, 1, &[[r, g, b, 1.0]]);
		let mut depth = texture_2d(1, 1, &[[0.0, 0.0, 0.0, 1.0]]);
		let mut result = empty_image(1, 1);
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_texture(slot(&program, "depth_texture"), &mut depth);
		descriptors.bind_image(slot(&program, "result"), &mut result);
		descriptors.bind_texture(slot(&program, "sky_view_lut"), &mut sky_view);
		descriptors.bind_texture(slot(&program, "transmittance_lut"), &mut transmittance);
		descriptors.bind_buffer(parameter_slot, &mut parameters);
		run_at(&program, &mut descriptors, [0, 0]);
		drop(descriptors);
		rgba(&result, [0, 0])
	}

	/// Verifies that the sun light's RGB illuminance sets the sky's brightness and color, and that no light gives no sky.
	#[test]
	fn sky_follows_the_sun_light_illuminance() {
		let sky_scattering = [0.02, 0.03, 0.04, 1.0];

		assert_rgba_close(
			composite_background(sky_scattering, [400.0, 200.0, 100.0], 1.0),
			[8.0, 6.0, 4.0, 1.0],
			1e-4,
		);
		assert_rgba_close(
			composite_background(sky_scattering, [0.0, 0.0, 0.0], 1.0),
			[0.0, 0.0, 0.0, 1.0],
			0.0,
		);
	}

	/// Verifies that the light's illuminance is the sunlight that reaches the camera. Light scattered once reaches the
	/// eye from sunlight the same air dimmed, so it doesn't change with that air. Light scattered more than once is
	/// stored per lux entering the atmosphere, so it brightens by as much as the air dims the sun, at most 16 times.
	#[test]
	fn sky_view_takes_the_light_as_the_sunlight_that_reaches_the_camera() {
		let higher_orders = [0.01, 0.02, 0.04, 1.0];
		let sky = |sun_transmittance, higher_orders| {
			run_sky_view(&Default::default(), test_sun_direction(), sun_transmittance, higher_orders)
		};
		let once = sky([1.0; 3], [0.0, 0.0, 0.0, 1.0]);
		let more_than_once: [f32; 3] = std::array::from_fn(|channel| sky([1.0; 3], higher_orders)[channel] - once[channel]);

		assert_rgba_close(sky([0.5, 0.25, 0.125], [0.0, 0.0, 0.0, 1.0]), once, 1e-5);
		let dimmed = sky([0.5, 0.25, 0.125], higher_orders);
		let blocked = sky([0.0; 3], higher_orders);
		for (channel, gain) in [2.0, 4.0, 8.0].into_iter().enumerate() {
			let expected = once[channel] + gain * more_than_once[channel];
			assert!(
				(dimmed[channel] - expected).abs() <= 1e-4 * expected,
				"channel {channel}: {dimmed:?}, expected {expected}"
			);
			let expected = 16.0 * more_than_once[channel];
			assert!(
				(blocked[channel] - expected).abs() <= 1e-4 * expected,
				"channel {channel}: {blocked:?}, expected {expected}"
			);
		}
	}

	/// Verifies that the sun disk shows the light's own radiance through air that dims the sun, since the light is
	/// already what reaches the camera.
	#[test]
	fn sun_disk_keeps_the_light_radiance_through_dimming_air() {
		// The test camera looks along +Z, so this sun sits at the center of the view.
		let settings = super::AtmosphereSkyRenderPassSettings {
			skip_below_horizon: false,
			..Default::default()
		};
		let illuminance = [0.4, 0.2, 0.1];
		let radius = crate::rendering::DirectionalLight::SUN_ANGULAR_RADIUS.value();
		let radiance = super::sun_disk_radiance(Vec3f::new(illuminance[0], illuminance[1], illuminance[2]), radius);

		let disk = composite(
			&settings,
			UnitVector::z_axis(),
			[0.0, 0.0, 0.0, 1.0],
			[0.5, 0.25, 0.125],
			illuminance,
			1.0,
		);

		assert_rgba_close(disk, [radiance.x, radiance.y, radiance.z, 1.0], 1e-2);
	}

	/// Verifies that the sky is written pre-exposed, like the scene it's composited behind.
	#[test]
	fn sky_is_written_pre_exposed() {
		// A 100,000 lux sun at EV100 15 exposure, 1 / (1.2 * 2^15).
		let exposure_scale = 1.0 / (1.2 * 32_768.0);
		let exposed = composite_background([0.02, 0.03, 0.04, 1.0], [100_000.0; 3], exposure_scale);

		assert_rgba_close(
			exposed,
			[
				2_000.0 * exposure_scale,
				3_000.0 * exposure_scale,
				4_000.0 * exposure_scale,
				1.0,
			],
			1e-6,
		);
	}

	/// Verifies that the sky composite dispatches only the rows its screen height covers, rounding a partial row up.
	#[test]
	fn sky_composite_covers_only_its_screen_height() {
		let extent = utils::Extent::rectangle(1920, 1081);
		assert_eq!(super::sky_composite_extent(extent, 0.5), utils::Extent::rectangle(1920, 541));
		assert_eq!(super::sky_composite_extent(extent, 1.0), extent);
		assert_eq!(super::sky_composite_extent(extent, 2.0), extent);
		assert_eq!(super::sky_composite_extent(extent, 0.0).height(), 0);
	}

	static SKY_ENVIRONMENT_SHADER_BESL: std::sync::LazyLock<&str> =
		std::sync::LazyLock::new(|| crate::rendering::shader_vm_test::asset_source("sky-environment.besl"));
	static ENVIRONMENT_DOWNSAMPLE_SHADER_BESL: std::sync::LazyLock<&str> =
		std::sync::LazyLock::new(|| crate::rendering::shader_vm_test::asset_source("environment-downsample.besl"));
	static ENVIRONMENT_PREFILTER_SHADER_BESL: std::sync::LazyLock<&str> =
		std::sync::LazyLock::new(|| crate::rendering::shader_vm_test::asset_source("environment-prefilter.besl"));

	/// Verifies that the sky environment holds the sky's light: the sky-view LUT's light per lux in a texel's
	/// direction, times the sunlight, unexposed.
	#[test]
	fn sky_environment_holds_the_sky_view_light_times_the_sunlight() {
		let program =
			crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(*SKY_ENVIRONMENT_SHADER_BESL));
		let parameter_slot = slot(&program, "parameters");
		let mut parameters = sky_parameters(
			&program,
			parameter_slot,
			&Default::default(),
			test_sun([400.0, 200.0, 100.0], test_sun_direction()),
			1.0,
		);
		// One texel, so every direction reads it.
		let mut sky_view = texture_2d(1, 1, &[[0.02, 0.03, 0.04, 1.0]]);
		let mut result = empty_image(2, 2);
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_texture(slot(&program, "sky_view_lut"), &mut sky_view);
		descriptors.bind_buffer(parameter_slot, &mut parameters);
		descriptors.bind_image(slot(&program, "result").offset(2), &mut result);
		run_on_face(&program, &mut descriptors, [1, 0], 2);
		drop(descriptors);

		assert_rgba_close(rgba(&result, [1, 0]), [8.0, 6.0, 4.0, 1.0], 1e-3);
	}

	/// Verifies that the environment shaders, which index six-face storage arrays by `thread_layer`, compile with the
	/// platform's shader compiler, since a mismatch between grid coordinate widths only surfaces there.
	#[cfg(target_os = "macos")]
	#[compio::test]
	async fn environment_programs_lower_to_the_platform_shader_language() {
		use resource_management::shader::ShaderGenerationSettings;
		use resource_management::shader::besl::backends::platform::PlatformShaderCompiler;

		for (name, source) in [
			("sky_environment", *SKY_ENVIRONMENT_SHADER_BESL),
			("environment_downsample", *ENVIRONMENT_DOWNSAMPLE_SHADER_BESL),
			("environment_prefilter", *ENVIRONMENT_PREFILTER_SHADER_BESL),
		] {
			let mut root = crate::rendering::shader_vm_test::with_engine_modules(source);
			root.add(crate::rendering::common_shader_generator::common_shader_module().to_vec());
			let root = besl::lex(root).expect("environment shader should link");
			let settings = ShaderGenerationSettings::compute(utils::Extent::rectangle(8, 8)).name(name.to_string());
			PlatformShaderCompiler::new()
				.generate(&settings, &root)
				.await
				.unwrap_or_else(|error| panic!("{name} should compile for the platform shader language: {error}"));
		}
	}

	/// Runs `program` at `texel` of cube face `face`, the dispatch's `thread_layer`.
	fn run_on_face(
		program: &besl::vm::ExecutableProgram,
		descriptors: &mut DescriptorBindings<'_>,
		texel: [u32; 2],
		face: u32,
	) {
		let config = crate::rendering::shader_vm_test::lane_config(0)
			.with_thread_id(texel)
			.with_thread_layer(face);
		program
			.run_main_with_config(descriptors, &config)
			.expect("Failed to run an environment shader in the BESL VM. The most likely cause is an unbound face.");
	}

	/// Verifies that each level of the radiance pyramid is the plain average of the 2 × 2 texels above it.
	#[test]
	fn environment_downsample_averages_each_two_by_two_block() {
		let program = crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(
			*ENVIRONMENT_DOWNSAMPLE_SHADER_BESL,
		));
		let mut source = texture_2d(
			2,
			2,
			&[
				[1.0, 0.0, 4.0, 1.0],
				[3.0, 2.0, 0.0, 1.0],
				[0.0, 4.0, 8.0, 1.0],
				[4.0, 6.0, 0.0, 1.0],
			],
		);
		let mut result = empty_image(1, 1);
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_image(slot(&program, "source").offset(3), &mut source);
		descriptors.bind_image(slot(&program, "result").offset(3), &mut result);
		run_on_face(&program, &mut descriptors, [0, 0], 3);
		drop(descriptors);

		assert_rgba_close(rgba(&result, [0, 0]), [2.0, 3.0, 3.0, 1.0], 1e-6);
	}

	/// Prefilters one texel of face 2 of an environment whose radiance is `radiance` in every direction, in `mode`
	/// (`0` for a specular `level`, `1` for irradiance) with `sample_count` samples.
	fn prefilter_uniform_environment(radiance: [f32; 4], mode: u32, level: u32, sample_count: u32) -> [f32; 4] {
		let program =
			crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(*ENVIRONMENT_PREFILTER_SHADER_BESL));
		// The VM reads a cube at its direction as a volume, so a one-texel volume is a uniform environment.
		let mut environment = besl::vm::Texture::new_3d(1, 1, 1).expect("uniform environment");
		environment.write_3d([0, 0, 0], radiance).expect("uniform environment texel");
		let mut result = empty_image(4, 4);
		let mut push_constant = crate::rendering::shader_vm_test::push_constant_buffer(&program);
		for (member, value) in [("level", level), ("mode", mode), ("sample_count", sample_count)] {
			push_constant
				.write(member, Value::U32(value))
				.expect("prefilter push constant");
		}
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_texture(slot(&program, "radiance"), &mut environment);
		descriptors.bind_image(slot(&program, "result").offset(2), &mut result);
		descriptors.bind_push_constant(&mut push_constant);
		run_on_face(&program, &mut descriptors, [1, 2], 2);
		drop(descriptors);
		rgba(&result, [1, 2])
	}

	/// Verifies that an environment of the same radiance everywhere prefilters to that radiance at every specular level,
	/// and to irradiance over π equal to it, as a lobe or a cosine that integrates to one must.
	#[test]
	fn uniform_environment_prefilters_to_itself() {
		let radiance = [3.0, 2.0, 1.0, 1.0];
		for level in [0, 1, 4, 7] {
			assert_rgba_close(prefilter_uniform_environment(radiance, 0, level, 64), radiance, 1e-3);
		}
		assert_rgba_close(prefilter_uniform_environment(radiance, 1, 0, 256), radiance, 1e-3);
	}

	/// Verifies that the sun disk carries the light's illuminance over its solid angle until the half-float cap.
	#[test]
	fn sun_disk_follows_the_light_until_the_half_float_cap() {
		let radius = crate::rendering::DirectionalLight::SUN_ANGULAR_RADIUS.value();
		let solid_angle = std::f32::consts::PI * radius * radius;
		let dim = super::sun_disk_radiance(Vec3f::new(1.0, 0.5, 0.0), radius);

		assert!((dim.x - 1.0 / solid_angle).abs() <= 1e-2, "dim disk = {dim:?}");
		assert!((dim.y - 0.5 / solid_angle).abs() <= 1e-2, "dim disk = {dim:?}");
		assert_eq!(dim.z, 0.0);

		let noon = super::sun_disk_radiance(Vec3f::new(100_000.0, 100_000.0, 100_000.0), radius);
		assert_eq!(
			[noon.x, noon.y, noon.z],
			[super::SUN_DISK_MAX_RADIANCE; 3],
			"a noon sun must stay finite in half-float targets"
		);
	}
}
