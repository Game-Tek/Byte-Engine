use ghi::{
	command_buffer::CommonCommandBufferMode as _,
	context::{Context as _, ContextCreate as _},
	frame::Frame as _,
};
use math::{Point, Radians, ShaderMatrix, UnitVector, inverse};
use maths_rs::{Vec3f, Vec4f};
use utils::Extent;

use crate::{
	core::{
		Entity,
		factory::{CreateMessage, Handle},
		listener::{DefaultListener, Listener},
	},
	gameplay::transform::TransformationUpdate,
	rendering::{
		DirectionalLight, Sink,
		render_pass::{
			RenderPass, RenderPassBuilder, RenderPassReturn, SceneBackgroundTargets, allocate_render_command, simple_compute,
		},
	},
};

const TRANSMITTANCE_LUT_WIDTH: u32 = 256;
const TRANSMITTANCE_LUT_HEIGHT: u32 = 64;
const SKY_VIEW_LUT_SIZE: u32 = 256;
/// Light scattered more than once varies smoothly with altitude and sun angle, so a small LUT holds it.
const MULTIPLE_SCATTERING_LUT_SIZE: u32 = 32;
/// Half-float render targets overflow above 65,504. Even after exposure, the sun disk's physical radiance can pass that,
/// so it's capped at half the limit, which leaves room for passes that add samples together.
const SUN_DISK_MAX_RADIANCE: f32 = 32_768.0;

fn transmittance_lut_extent() -> Extent {
	Extent::rectangle(TRANSMITTANCE_LUT_WIDTH, TRANSMITTANCE_LUT_HEIGHT)
}

fn sky_view_lut_extent() -> Extent {
	Extent::square(SKY_VIEW_LUT_SIZE)
}

fn multiple_scattering_lut_extent() -> Extent {
	Extent::square(MULTIPLE_SCATTERING_LUT_SIZE)
}

fn should_rebuild_sky_view(transmittance_valid: bool, cached_camera_height: Option<u32>, camera_height: u32) -> bool {
	!transmittance_valid || cached_camera_height != Some(camera_height)
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
/// The sun's brightness, color, and disk size come from the scene's [`DirectionalLight`], not from these settings.
#[derive(Clone, Copy, Debug)]
pub struct AtmosphereSkyRenderPassSettings {
	pub sun_direction: UnitVector,
	pub ground_radius: f32,
	pub atmosphere_radius: f32,
	pub rayleigh_scale_height: f32,
	pub mie_scale_height: f32,
	pub mie_anisotropy: f32,
	pub ozone_strength: f32,
	/// The fraction of sunlight the planet's surface reflects back into the atmosphere. Multiple scattering adds that
	/// light to the sky.
	pub ground_albedo: f32,
	pub skip_below_horizon: bool,
	pub planet_center: Point,
}

impl Default for AtmosphereSkyRenderPassSettings {
	fn default() -> Self {
		Self {
			sun_direction: math::Vector::new(0.35, 0.85, 0.4)
				.normalized()
				.expect("default sun direction is nonzero"),
			ground_radius: 6_360_000.0,
			atmosphere_radius: 6_460_000.0,
			rayleigh_scale_height: 8_000.0,
			mie_scale_height: 1_200.0,
			mie_anisotropy: 0.76,
			ozone_strength: 1.0,
			ground_albedo: 0.3,
			skip_below_horizon: true,
			planet_center: Point::new(0.0, -6_360_000.0, 0.0),
		}
	}
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SkyShaderData {
	inverse_view_projection: ShaderMatrix,
	camera_position: [f32; 4],
	sun_direction: [f32; 4],
	planet_center: [f32; 4],
	atmosphere: [f32; 4],
	misc: [f32; 4],
	sun_illuminance: [f32; 4],
	sun_disk_radiance: [f32; 4],
}

/// The `AtmosphereSkyRenderPass` struct fills scene color with an atmosphere wherever opaque depth remains at infinity.
///
/// It is a scene background: scene pipelines record it after opaque surfaces and before transparent ones, so
/// transparent surfaces composite over the sky and scene color needs no coverage channel.
pub struct AtmosphereSkyRenderPass {
	transmittance_pass: simple_compute::Pass,
	multiple_scattering_pass: simple_compute::Pass,
	sky_view_pass: simple_compute::Pass,
	composite_pass: simple_compute::Pass,
	parameters: ghi::DynamicBufferHandle<SkyShaderData>,
	settings: AtmosphereSkyRenderPassSettings,
	directional_lights: DefaultListener<CreateMessage<DirectionalLight>>,
	transform_listener: DefaultListener<TransformationUpdate>,
	directional_light: Option<Handle>,
	/// The RGB illuminance in lux of the newest directional light. It stays black until a light is created.
	sun_illuminance: Vec3f,
	/// The angular radius of the newest directional light's disk.
	sun_angular_radius: Radians,
	transmittance_valid: bool,
	sky_view_camera_height: Option<u32>,
}

impl Entity for AtmosphereSkyRenderPass {}

impl AtmosphereSkyRenderPass {
	/// Creates a sky pass with default atmosphere settings and world-light listeners.
	///
	/// The newest directional light is the sky's sun: its illuminance sets the sky's brightness and color, and its
	/// transform sets the sun's direction. Publish that light's transform next so the sky can use its orientation.
	/// Without a directional light, the sky stays black. The sky writes `targets.color` in place wherever
	/// `targets.depth` holds no surface and leaves every other pixel untouched.
	pub fn new(
		render_pass_builder: &mut RenderPassBuilder,
		targets: SceneBackgroundTargets,
		directional_lights: DefaultListener<CreateMessage<DirectionalLight>>,
		transform_listener: DefaultListener<TransformationUpdate>,
	) -> Self {
		let settings = AtmosphereSkyRenderPassSettings::default();
		let SceneBackgroundTargets { color, depth } = targets;
		let transmittance_pipeline = simple_compute::Pipeline::compile(
			render_pass_builder,
			simple_compute::Descriptor::new("Sky Transmittance LUT", "byte-engine/rendering/sky-transmittance.pipeline"),
		)
		.expect("Failed to create the sky transmittance shader. The most likely cause is an incompatible shader interface.");
		let multiple_scattering_pipeline = simple_compute::Pipeline::compile(
			render_pass_builder,
			simple_compute::Descriptor::new(
				"Sky Multiple Scattering LUT",
				"byte-engine/rendering/sky-multiple-scattering.pipeline",
			),
		)
		.expect(
			"Failed to create the sky multiple-scattering shader. The most likely cause is an incompatible shader interface.",
		);
		let sky_view_pipeline = simple_compute::Pipeline::compile(
			render_pass_builder,
			simple_compute::Descriptor::new("Sky View LUT", "byte-engine/rendering/sky-view.pipeline"),
		)
		.expect("Failed to create the sky-view shader. The most likely cause is an incompatible shader interface.");
		let composite_pipeline = simple_compute::Pipeline::compile(
			render_pass_builder,
			simple_compute::Descriptor::new("Sky Composite", "byte-engine/rendering/sky.pipeline"),
		)
		.expect("Failed to create the sky shader. The most likely cause is an incompatible shader interface.");
		let context = render_pass_builder.context();
		let parameters = context.build_dynamic_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Sky Render Pass Parameters")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let transmittance_lut = context.build_image(
			ghi::image::Builder::new(crate::rendering::SCENE_COLOR_FORMAT, ghi::Uses::Image | ghi::Uses::Storage)
				.name("Sky Transmittance LUT")
				.extent(transmittance_lut_extent())
				.device_accesses(ghi::DeviceAccesses::DeviceOnly),
		);
		let multiple_scattering_lut = context.build_image(
			ghi::image::Builder::new(crate::rendering::SCENE_COLOR_FORMAT, ghi::Uses::Image | ghi::Uses::Storage)
				.name("Sky Multiple Scattering LUT")
				.extent(multiple_scattering_lut_extent())
				.device_accesses(ghi::DeviceAccesses::DeviceOnly),
		);
		let sky_view_lut = context.build_image(
			ghi::image::Builder::new(crate::rendering::SCENE_COLOR_FORMAT, ghi::Uses::Image | ghi::Uses::Storage)
				.name("Sky View LUT")
				.extent(sky_view_lut_extent())
				.device_accesses(ghi::DeviceAccesses::DeviceOnly),
		);
		let sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Linear)
				.mip_map_mode(ghi::FilteringModes::Linear)
				.addressing_mode(ghi::SamplerAddressingModes::Clamp),
		);
		let transmittance_pass = transmittance_pipeline
			.bind(
				render_pass_builder,
				"Sky Transmittance LUT Descriptor Set",
				&[
					simple_compute::Resource::image("transmittance_lut", transmittance_lut),
					simple_compute::Resource::buffer("parameters", parameters),
				],
			)
			.expect("Failed to bind sky transmittance resources. The most likely cause is a changed BESL binding contract.");
		let multiple_scattering_pass = multiple_scattering_pipeline
			.bind(
				render_pass_builder,
				"Sky Multiple Scattering LUT Descriptor Set",
				&[
					simple_compute::Resource::combined_image_sampler(
						"transmittance_lut",
						transmittance_lut,
						sampler,
						ghi::Layouts::Read,
					),
					simple_compute::Resource::image("multiple_scattering_lut", multiple_scattering_lut),
					simple_compute::Resource::buffer("parameters", parameters),
				],
			)
			.expect(
				"Failed to bind sky multiple-scattering resources. The most likely cause is a changed BESL binding contract.",
			);
		let sky_view_pass = sky_view_pipeline
			.bind(
				render_pass_builder,
				"Sky View LUT Descriptor Set",
				&[
					simple_compute::Resource::combined_image_sampler(
						"transmittance_lut",
						transmittance_lut,
						sampler,
						ghi::Layouts::Read,
					),
					simple_compute::Resource::image("sky_view_lut", sky_view_lut),
					simple_compute::Resource::buffer("parameters", parameters),
					simple_compute::Resource::combined_image_sampler(
						"multiple_scattering_lut",
						multiple_scattering_lut,
						sampler,
						ghi::Layouts::Read,
					),
				],
			)
			.expect("Failed to bind sky-view resources. The most likely cause is a changed BESL binding contract.");
		let composite_pass = composite_pipeline
			.bind(
				render_pass_builder,
				"Sky Render Pass Descriptor Set",
				&[
					simple_compute::Resource::combined_image_sampler("depth_texture", depth, sampler, ghi::Layouts::Read),
					simple_compute::Resource::image("result", color),
					simple_compute::Resource::combined_image_sampler("sky_view_lut", sky_view_lut, sampler, ghi::Layouts::Read),
					simple_compute::Resource::combined_image_sampler(
						"transmittance_lut",
						transmittance_lut,
						sampler,
						ghi::Layouts::Read,
					),
					simple_compute::Resource::buffer("parameters", parameters),
				],
			)
			.expect("Failed to bind the sky resources. The most likely cause is a changed BESL binding contract.");

		Self {
			transmittance_pass,
			multiple_scattering_pass,
			sky_view_pass,
			composite_pass,
			parameters,
			settings,
			directional_lights,
			transform_listener,
			directional_light: None,
			sun_illuminance: Vec3f::new(0.0, 0.0, 0.0),
			sun_angular_radius: DirectionalLight::SUN_ANGULAR_RADIUS,
			transmittance_valid: false,
			sky_view_camera_height: None,
		}
	}

	/// Adopts the newest directional light as the sun and applies its latest illuminance, disk size, and orientation to the sky.
	fn update_sun(&mut self) {
		while let Some(message) = self.directional_lights.read() {
			self.directional_light = Some(message.handle());
			self.sun_illuminance = message.data().color;
			self.sun_angular_radius = message.data().angular_radius;
		}

		while let Some(message) = self.transform_listener.read() {
			if self.directional_light == Some(message.handle()) {
				// Directional-light orientation points along ray travel; the atmosphere needs the direction toward the sun.
				self.settings.sun_direction = -math::direction_from_orientation(message.transform().get_orientation());
				self.sky_view_camera_height = None;
			}
		}
	}

	/// Updates per-view sky constants from the active camera before dispatch and returns the camera height.
	fn write_parameters(&self, frame: &mut ghi::implementation::Frame, sink: &Sink) -> f32 {
		let data = sky_shader_data(&self.settings, self.sun_illuminance, self.sun_angular_radius, sink);
		*frame.get_mut_dynamic_buffer_slice(self.parameters) = data;
		data.camera_position[1]
	}
}

/// Builds one sink's sky constants from the atmosphere settings, the sun light's RGB illuminance in lux, and its disk's
/// angular radius.
///
/// The sky is written pre-exposed like the scene: the shaders multiply by the uploaded illuminance and disk radiance,
/// so both include the sink's camera exposure.
fn sky_shader_data(
	settings: &AtmosphereSkyRenderPassSettings,
	sun_illuminance: Vec3f,
	sun_angular_radius: Radians,
	sink: &Sink,
) -> SkyShaderData {
	let view = sink.view();
	let inverse_view = inverse(view.view());
	let camera_position = inverse_view * Vec4f::new(0.0, 0.0, 0.0, 1.0);
	let sun_direction = settings.sun_direction;
	let planet_center = settings.planet_center.into_maths();
	let exposed_illuminance = sun_illuminance * sink.exposure_scale();
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
		self.update_sun();
		let transmittance_pass = self.transmittance_pass.ready(frame)?;
		let multiple_scattering_pass = self.multiple_scattering_pass.ready(frame)?;
		let sky_view_pass = self.sky_view_pass.ready(frame)?;
		let composite_pass = self.composite_pass.ready(frame)?;
		let camera_height = self.write_parameters(frame, sink).to_bits();
		let rebuild_transmittance = !self.transmittance_valid;
		// Horizontal movement leaves the camera-to-planet vector unchanged because the planet center follows the camera in X/Z.
		let rebuild_sky_view = should_rebuild_sky_view(self.transmittance_valid, self.sky_view_camera_height, camera_height);
		self.transmittance_valid = true;
		self.sky_view_camera_height = Some(camera_height);

		let extent = sink.extent();
		let transmittance_extent = transmittance_lut_extent();
		let multiple_scattering_extent = multiple_scattering_lut_extent();
		let sky_view_extent = sky_view_lut_extent();

		Some(allocate_render_command(frame_allocator, move |command_buffer, _| {
			command_buffer.region(
				|label| label.write_str("Sky"),
				|command_buffer| {
					// Multiple scattering reads the transmittance LUT and only depends on the atmosphere, not on the sun's
					// angle, so both rebuild together.
					if rebuild_transmittance {
						transmittance_pass.record(command_buffer, transmittance_extent);
						multiple_scattering_pass.record(command_buffer, multiple_scattering_extent);
					}
					if rebuild_sky_view {
						sky_view_pass.record(command_buffer, sky_view_extent);
					}
					composite_pass.record(command_buffer, extent);
				},
			);
		}))
	}

	fn bypass<'a>(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		// Bypassed, the background stays the black the scene cleared it to.
		let _ = (frame, sink, frame_allocator);
		self.update_sun();
		None
	}
}

#[cfg(test)]
mod tests {
	use besl::vm::{Buffer, DescriptorBindings, ResourceSlot, Value};
	use math::{Point, UnitVector};
	use maths_rs::Vec3f;

	use super::simple_compute;
	use crate::rendering::shader_vm_test::{assert_rgba_close, buffer, empty_image, rgba, run_at, texture_2d};

	const SKY_SHADER_BESL: &str = include_str!("../../../assets/rendering/sky.besl");
	const SKY_TRANSMITTANCE_SHADER_BESL: &str = include_str!("../../../assets/rendering/sky-transmittance.besl");
	const SKY_VIEW_SHADER_BESL: &str = include_str!("../../../assets/rendering/sky-view.besl");
	const SKY_MULTIPLE_SCATTERING_SHADER_BESL: &str = include_str!("../../../assets/rendering/sky-multiple-scattering.besl");

	/// Sunlight used by tests that don't depend on the sun's brightness.
	const TEST_SUN_ILLUMINANCE: [f32; 3] = [10.0, 10.0, 10.0];

	/// Uploads the production sky constants for default atmosphere settings, a sun of `sun_illuminance` lux, and a
	/// camera with `exposure_scale`, so every shader test reads exactly what the pass writes.
	fn sky_parameters(
		program: &besl::vm::ExecutableProgram,
		parameter_slot: ResourceSlot,
		sun_illuminance: [f32; 3],
		exposure_scale: f32,
	) -> Buffer {
		sky_parameters_with_settings(
			program,
			parameter_slot,
			&super::AtmosphereSkyRenderPassSettings::default(),
			sun_illuminance,
			exposure_scale,
		)
	}

	/// Uploads the production sky constants for `settings`. See [`sky_parameters`].
	fn sky_parameters_with_settings(
		program: &besl::vm::ExecutableProgram,
		parameter_slot: ResourceSlot,
		settings: &super::AtmosphereSkyRenderPassSettings,
		sun_illuminance: [f32; 3],
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
		let data = super::sky_shader_data(
			settings,
			Vec3f::new(sun_illuminance[0], sun_illuminance[1], sun_illuminance[2]),
			crate::rendering::DirectionalLight::SUN_ANGULAR_RADIUS,
			&sink,
		);
		let mut parameters = buffer(program, parameter_slot);
		for (name, value) in [
			("camera_position", data.camera_position),
			("sun_direction", data.sun_direction),
			("planet_center", data.planet_center),
			("atmosphere", data.atmosphere),
			("misc", data.misc),
			("sun_illuminance", data.sun_illuminance),
			("sun_disk_radiance", data.sun_disk_radiance),
		] {
			parameters
				.write(name, Value::Vec4F(value))
				.expect("Failed to initialize sky parameters. The most likely cause is a changed production buffer layout.");
		}
		parameters
			.write("inverse_view_projection", Value::Mat4F(data.inverse_view_projection.0))
			.expect("Failed to initialize the sky matrix. The most likely cause is a changed production buffer layout.");
		parameters
	}

	fn assert_finite_nonnegative_color(color: [f32; 4], name: &str) {
		assert!(
			color[..3].iter().all(|channel| channel.is_finite() && *channel >= 0.0),
			"Invalid {name} VM output. The most likely cause is unstable atmosphere integration: {color:?}"
		);
	}

	#[test]
	fn sky_view_cache_rebuilds_for_initialization_and_height_changes_only() {
		let height = 2.0_f32.to_bits();

		assert!(super::should_rebuild_sky_view(false, None, height));
		assert!(super::should_rebuild_sky_view(true, None, height));
		assert!(super::should_rebuild_sky_view(true, Some(3.0_f32.to_bits()), height));
		assert!(!super::should_rebuild_sky_view(true, Some(height), height));
	}

	/// Verifies the production transmittance LUT writes finite optical transmission.
	#[test]
	fn sky_transmittance_besl_vm_writes_bounded_transmission() {
		let program =
			crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(SKY_TRANSMITTANCE_SHADER_BESL));
		let parameter_slot = ResourceSlot::new(1);
		let mut parameters = sky_parameters(&program, parameter_slot, TEST_SUN_ILLUMINANCE, 1.0);
		let mut output = empty_image(1, 1);
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_image(ResourceSlot::new(0), &mut output);
		descriptors.bind_buffer(parameter_slot, &mut parameters);
		run_at(&program, &mut descriptors, [0, 0]);
		drop(descriptors);

		let transmission = rgba(&output, [0, 0]);
		assert_finite_nonnegative_color(transmission, "sky transmittance");

		assert!(
			transmission[..3].iter().all(|channel| *channel <= 1.0),
			"Out-of-range sky transmittance. The most likely cause is an invalid optical-depth sign: {transmission:?}"
		);
		assert_rgba_close([0.0, 0.0, 0.0, transmission[3]], [0.0, 0.0, 0.0, 1.0], 1e-6);
	}

	/// Runs one sky-view texel with full sun transmittance and a multiple-scattering LUT holding `higher_orders`.
	fn run_sky_view(higher_orders: [f32; 4]) -> [f32; 4] {
		let program = crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(SKY_VIEW_SHADER_BESL));
		let parameter_slot = ResourceSlot::new(2);
		let mut parameters = sky_parameters(&program, parameter_slot, TEST_SUN_ILLUMINANCE, 1.0);
		let mut transmittance = texture_2d(1, 1, &[[1.0, 1.0, 1.0, 1.0]]);
		let mut multiple_scattering = texture_2d(1, 1, &[higher_orders]);
		let mut output = empty_image(1, 1);
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_texture(ResourceSlot::new(0), &mut transmittance);
		descriptors.bind_image(ResourceSlot::new(1), &mut output);
		descriptors.bind_buffer(parameter_slot, &mut parameters);
		descriptors.bind_texture(ResourceSlot::new(3), &mut multiple_scattering);
		run_at(&program, &mut descriptors, [0, 0]);
		drop(descriptors);
		rgba(&output, [0, 0])
	}

	/// Verifies the sky-view LUT consumes transmittance and produces finite HDR scattering.
	#[test]
	fn sky_view_besl_vm_integrates_scattering_from_transmittance() {
		let scattering = run_sky_view([0.0, 0.0, 0.0, 1.0]);
		assert_finite_nonnegative_color(scattering, "sky-view");

		assert!(
			scattering[..3].iter().any(|channel| *channel > 0.0),
			"Empty sky-view VM output. The most likely cause is an invalid atmosphere interval: {scattering:?}"
		);
		assert_rgba_close([0.0, 0.0, 0.0, scattering[3]], [0.0, 0.0, 0.0, 1.0], 1e-6);
	}

	/// Verifies that the sky-view LUT adds the light from the multiple-scattering LUT to single scattering.
	#[test]
	fn sky_view_adds_light_scattered_more_than_once() {
		let single = run_sky_view([0.0, 0.0, 0.0, 1.0]);
		let multiple = run_sky_view([0.01, 0.02, 0.04, 1.0]);

		assert!(
			(0..3).all(|channel| multiple[channel] > single[channel]),
			"Multiple scattering added no light. The most likely cause is that the sky-view LUT ignores its multiple-scattering binding: single {single:?}, with multiple scattering {multiple:?}"
		);
	}

	/// Builds the multiple-scattering LUT texel for a sun straight overhead at ground level, under full transmittance.
	fn overhead_sun_multiple_scattering(ground_albedo: f32) -> [f32; 4] {
		let program = crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(
			SKY_MULTIPLE_SCATTERING_SHADER_BESL,
		));
		let parameter_slot = ResourceSlot::new(2);
		let settings = super::AtmosphereSkyRenderPassSettings {
			ground_albedo,
			..super::AtmosphereSkyRenderPassSettings::default()
		};
		let mut parameters = sky_parameters_with_settings(&program, parameter_slot, &settings, TEST_SUN_ILLUMINANCE, 1.0);
		let mut transmittance = texture_2d(1, 1, &[[1.0, 1.0, 1.0, 1.0]]);
		let size = super::MULTIPLE_SCATTERING_LUT_SIZE;
		let mut output = empty_image(size, size);
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_texture(ResourceSlot::new(0), &mut transmittance);
		descriptors.bind_image(ResourceSlot::new(1), &mut output);
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
		let program = crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(SKY_SHADER_BESL));
		let sentinel = [0.2, 0.3, 0.4, 0.5];
		let mut surface_depth = texture_2d(1, 1, &[[0.5, 0.0, 0.0, 1.0]]);
		let mut surface_color = texture_2d(1, 1, &[sentinel]);
		let mut surface_descriptors = DescriptorBindings::new();
		surface_descriptors.bind_texture(ResourceSlot::new(0), &mut surface_depth);
		surface_descriptors.bind_image(ResourceSlot::new(2), &mut surface_color);
		run_at(&program, &mut surface_descriptors, [0, 0]);
		drop(surface_descriptors);
		assert_rgba_close(rgba(&surface_color, [0, 0]), sentinel, 0.0);

		let parameter_slot = ResourceSlot::new(5);
		let mut parameters = sky_parameters(&program, parameter_slot, TEST_SUN_ILLUMINANCE, 1.0);
		let mut sky_view = texture_2d(1, 1, &[[2.0, 3.0, 4.0, 1.0]]);
		let mut transmittance = texture_2d(1, 1, &[[1.0, 1.0, 1.0, 1.0]]);
		let mut background_depth = texture_2d(1, 1, &[[0.0, 0.0, 0.0, 1.0]]);
		let mut background_color = empty_image(1, 1);
		let mut background_descriptors = DescriptorBindings::new();
		background_descriptors.bind_texture(ResourceSlot::new(0), &mut background_depth);
		background_descriptors.bind_image(ResourceSlot::new(2), &mut background_color);
		background_descriptors.bind_texture(ResourceSlot::new(3), &mut sky_view);
		background_descriptors.bind_texture(ResourceSlot::new(4), &mut transmittance);
		background_descriptors.bind_buffer(parameter_slot, &mut parameters);
		run_at(&program, &mut background_descriptors, [0, 0]);
		drop(background_descriptors);

		let background = rgba(&background_color, [0, 0]);
		assert!(
			background[..3].iter().all(|channel| channel.is_finite() && *channel > 1.0),
			"Clamped sky VM output. The most likely cause is tone mapping before the final scene tonemap: {background:?}"
		);
	}

	/// Composites one background pixel whose view misses the sun disk, over a sky-view LUT of `sky_scattering` per lux.
	fn composite_background(sky_scattering: [f32; 4], sun_illuminance: [f32; 3], exposure_scale: f32) -> [f32; 4] {
		let program = crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(SKY_SHADER_BESL));
		let parameter_slot = ResourceSlot::new(5);
		let mut parameters = sky_parameters(&program, parameter_slot, sun_illuminance, exposure_scale);
		let mut sky_view = texture_2d(1, 1, &[sky_scattering]);
		let mut transmittance = texture_2d(1, 1, &[[1.0, 1.0, 1.0, 1.0]]);
		let mut depth = texture_2d(1, 1, &[[0.0, 0.0, 0.0, 1.0]]);
		let mut result = empty_image(1, 1);
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_texture(ResourceSlot::new(0), &mut depth);
		descriptors.bind_image(ResourceSlot::new(2), &mut result);
		descriptors.bind_texture(ResourceSlot::new(3), &mut sky_view);
		descriptors.bind_texture(ResourceSlot::new(4), &mut transmittance);
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
