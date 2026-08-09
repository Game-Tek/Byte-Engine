use ghi::{
	command_buffer::CommonCommandBufferMode as _,
	context::{Context as _, ContextCreate as _},
	frame::Frame as _,
};
use math::{inverse, Point, ShaderMatrix, UnitVector};
use maths_rs::Vec4f;
use utils::Extent;

use crate::{
	core::Entity,
	rendering::{
		render_pass::{allocate_render_command, simple_compute, RenderPass, RenderPassBuilder, RenderPassReturn},
		Sink,
	},
};

const TRANSMITTANCE_LUT_WIDTH: u32 = 256;
const TRANSMITTANCE_LUT_HEIGHT: u32 = 64;
const SKY_VIEW_LUT_SIZE: u32 = 256;

fn transmittance_lut_extent() -> Extent {
	Extent::rectangle(TRANSMITTANCE_LUT_WIDTH, TRANSMITTANCE_LUT_HEIGHT)
}

fn sky_view_lut_extent() -> Extent {
	Extent::square(SKY_VIEW_LUT_SIZE)
}

fn should_rebuild_sky_view(transmittance_valid: bool, cached_camera_height: Option<u32>, camera_height: u32) -> bool {
	!transmittance_valid || cached_camera_height != Some(camera_height)
}

/// The `AtmosphereSkyRenderPassSettings` struct configures the physical atmosphere and sun parameters for the sky pass.
#[derive(Clone, Copy, Debug)]
pub struct AtmosphereSkyRenderPassSettings {
	pub sun_direction: UnitVector,
	pub sun_intensity: f32,
	pub sun_angular_radius: f32,
	pub ground_radius: f32,
	pub atmosphere_radius: f32,
	pub rayleigh_scale_height: f32,
	pub mie_scale_height: f32,
	pub mie_anisotropy: f32,
	pub ozone_strength: f32,
	pub skip_below_horizon: bool,
	pub planet_center: Point,
}

impl Default for AtmosphereSkyRenderPassSettings {
	fn default() -> Self {
		Self {
			sun_direction: math::Vector::new(0.35, 0.85, 0.4)
				.normalized()
				.expect("default sun direction is nonzero"),
			sun_intensity: 22.0,
			sun_angular_radius: 0.004675,
			ground_radius: 6_360_000.0,
			atmosphere_radius: 6_460_000.0,
			rayleigh_scale_height: 8_000.0,
			mie_scale_height: 1_200.0,
			mie_anisotropy: 0.76,
			ozone_strength: 1.0,
			skip_below_horizon: true,
			planet_center: Point::new(0.0, -6_360_000.0, 0.0),
		}
	}
}

#[repr(C)]
#[derive(Clone, Copy)]
struct SkyShaderData {
	inverse_view_projection: ShaderMatrix,
	camera_position: [f32; 4],
	sun_direction: [f32; 4],
	planet_center: [f32; 4],
	atmosphere: [f32; 4],
	misc: [f32; 4],
}

/// The `AtmosphereSkyRenderPass` struct places an atmosphere behind scene color wherever opaque depth remains at infinity.
pub struct AtmosphereSkyRenderPass {
	transmittance_pass: simple_compute::Pass,
	sky_view_pass: simple_compute::Pass,
	composite_pass: simple_compute::Pass,
	parameters: ghi::DynamicBufferHandle<SkyShaderData>,
	settings: AtmosphereSkyRenderPassSettings,
	transmittance_valid: bool,
	sky_view_camera_height: Option<u32>,
}

impl Entity for AtmosphereSkyRenderPass {}

impl AtmosphereSkyRenderPass {
	/// Creates a sky pass with physically plausible default atmosphere settings.
	pub fn new(render_pass_builder: &mut RenderPassBuilder) -> Self {
		Self::with_settings(render_pass_builder, AtmosphereSkyRenderPassSettings::default())
	}

	/// Creates a sky pass with caller-supplied atmosphere settings.
	pub fn with_settings(render_pass_builder: &mut RenderPassBuilder, settings: AtmosphereSkyRenderPassSettings) -> Self {
		let depth = render_pass_builder.read_from("depth");
		let _main_read = render_pass_builder.read_from("main");
		let main = render_pass_builder.render_to("main");
		let transmittance_pipeline = simple_compute::Pipeline::compile(
			render_pass_builder,
			simple_compute::Descriptor::new("Sky Transmittance LUT", "byte-engine/rendering/sky-transmittance.pipeline"),
		)
		.expect("Failed to create the sky transmittance shader. The most likely cause is an incompatible shader interface.");
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
			ghi::image::Builder::new(ghi::Formats::RGBA16F, ghi::Uses::Image | ghi::Uses::Storage)
				.name("Sky Transmittance LUT")
				.extent(transmittance_lut_extent())
				.device_accesses(ghi::DeviceAccesses::DeviceOnly),
		);
		let sky_view_lut = context.build_image(
			ghi::image::Builder::new(ghi::Formats::RGBA16F, ghi::Uses::Image | ghi::Uses::Storage)
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
				],
			)
			.expect("Failed to bind sky-view resources. The most likely cause is a changed BESL binding contract.");
		let composite_pass = composite_pipeline
			.bind(
				render_pass_builder,
				"Sky Render Pass Descriptor Set",
				&[
					simple_compute::Resource::combined_image_sampler("depth_texture", depth, sampler, ghi::Layouts::Read),
					simple_compute::Resource::image("main_texture", main),
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
			sky_view_pass,
			composite_pass,
			parameters,
			settings,
			transmittance_valid: false,
			sky_view_camera_height: None,
		}
	}

	/// Updates per-view sky constants from the active camera before dispatch.
	fn write_parameters(&self, frame: &mut ghi::implementation::Frame, sink: &Sink) -> f32 {
		let view = sink.view();
		let inverse_view_projection = inverse(view.view_projection());
		let inverse_view = inverse(view.view());
		let camera_position = inverse_view * Vec4f::new(0.0, 0.0, 0.0, 1.0);
		let sun_direction = self.settings.sun_direction;
		let planet_center = self.settings.planet_center.into_maths();
		let planet_center = [
			planet_center.x,
			planet_center.y,
			planet_center.z,
			self.settings.sun_angular_radius,
		];
		let parameters = frame.get_mut_dynamic_buffer_slice(self.parameters);
		let settings = self.settings;

		parameters.inverse_view_projection = inverse_view_projection.into();
		parameters.camera_position = [
			camera_position.x,
			camera_position.y,
			camera_position.z,
			settings.sun_intensity,
		];
		parameters.sun_direction = [
			sun_direction.x(),
			sun_direction.y(),
			sun_direction.z(),
			settings.mie_anisotropy,
		];
		parameters.planet_center = planet_center;
		parameters.atmosphere = [
			settings.ground_radius,
			settings.atmosphere_radius,
			settings.rayleigh_scale_height,
			settings.mie_scale_height,
		];
		parameters.misc = [
			settings.ozone_strength,
			if settings.skip_below_horizon { 1.0 } else { 0.0 },
			0.0,
			0.0,
		];

		camera_position.y
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
		let transmittance_pass = self.transmittance_pass.ready(frame)?;
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
		let sky_view_extent = sky_view_lut_extent();

		Some(allocate_render_command(frame_allocator, move |command_buffer, _| {
			command_buffer.region(
				|label| label.write_str("Sky"),
				|command_buffer| {
					if rebuild_transmittance {
						transmittance_pass.record(command_buffer, transmittance_extent);
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
		_frame: &mut ghi::implementation::Frame,
		_sink: &Sink,
		_frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		None
	}
}

#[cfg(test)]
mod tests {
	use besl::vm::{Buffer, DescriptorBindings, ResourceSlot, Value};
	use math::{inverse, Point, ShaderMatrix, UnitVector};

	use super::simple_compute;
	use crate::rendering::shader_vm_test::{assert_rgba_close, buffer, empty_image, rgba, run_at, texture_2d};

	const SKY_SHADER_BESL: &str = include_str!("../../../assets/rendering/sky.besl");
	const SKY_TRANSMITTANCE_SHADER_BESL: &str = include_str!("../../../assets/rendering/sky-transmittance.besl");
	const SKY_VIEW_SHADER_BESL: &str = include_str!("../../../assets/rendering/sky-view.besl");

	/// Builds the production sky parameter layout with deterministic default atmosphere values.
	fn default_parameters(program: &besl::vm::ExecutableProgram, parameter_slot: ResourceSlot) -> Buffer {
		let settings = super::AtmosphereSkyRenderPassSettings::default();
		let view = crate::rendering::View::new_perspective(60.0, 1.0, 0.1, 100.0, Point::origin(), UnitVector::z_axis());
		let inverse_view_projection = ShaderMatrix::from(inverse(view.view_projection())).0;
		let sun_direction = settings.sun_direction;
		let mut parameters = buffer(program, parameter_slot);
		// Mirror the production upload field-for-field so every LUT test validates the real buffer contract.
		for (name, value) in [
			("camera_position", [0.0, 0.0, 0.0, settings.sun_intensity]),
			(
				"sun_direction",
				[
					sun_direction.x(),
					sun_direction.y(),
					sun_direction.z(),
					settings.mie_anisotropy,
				],
			),
			(
				"planet_center",
				[
					settings.planet_center.x(),
					settings.planet_center.y(),
					settings.planet_center.z(),
					settings.sun_angular_radius,
				],
			),
			(
				"atmosphere",
				[
					settings.ground_radius,
					settings.atmosphere_radius,
					settings.rayleigh_scale_height,
					settings.mie_scale_height,
				],
			),
			(
				"misc",
				[
					settings.ozone_strength,
					if settings.skip_below_horizon { 1.0 } else { 0.0 },
					0.0,
					0.0,
				],
			),
		] {
			parameters
				.write(name, Value::Vec4F(value))
				.expect("Failed to initialize sky parameters. The most likely cause is a changed production buffer layout.");
		}
		parameters
			.write("inverse_view_projection", Value::Mat4F(inverse_view_projection))
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
		let mut parameters = default_parameters(&program, parameter_slot);
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

	/// Verifies the sky-view LUT consumes transmittance and produces finite HDR scattering.
	#[test]
	fn sky_view_besl_vm_integrates_scattering_from_transmittance() {
		let program = crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(SKY_VIEW_SHADER_BESL));
		let parameter_slot = ResourceSlot::new(2);
		let mut parameters = default_parameters(&program, parameter_slot);
		let mut transmittance = texture_2d(1, 1, &[[1.0, 1.0, 1.0, 1.0]]);
		let mut output = empty_image(1, 1);
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_texture(ResourceSlot::new(0), &mut transmittance);
		descriptors.bind_image(ResourceSlot::new(1), &mut output);
		descriptors.bind_buffer(parameter_slot, &mut parameters);
		run_at(&program, &mut descriptors, [0, 0]);
		drop(descriptors);

		let scattering = rgba(&output, [0, 0]);
		assert_finite_nonnegative_color(scattering, "sky-view");
		assert!(
			scattering[..3].iter().any(|channel| *channel > 0.0),
			"Empty sky-view VM output. The most likely cause is an invalid atmosphere interval: {scattering:?}"
		);
		assert_rgba_close([0.0, 0.0, 0.0, scattering[3]], [0.0, 0.0, 0.0, 1.0], 1e-6);
	}

	/// Verifies foreground preservation and a finite default atmosphere result through the VM.
	#[test]
	fn sky_besl_vm_preserves_foreground_and_renders_a_bounded_default_background() {
		let program = crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(SKY_SHADER_BESL));
		let sentinel = [0.2, 0.3, 0.4, 0.5];
		let mut foreground_depth = texture_2d(1, 1, &[[0.5, 0.0, 0.0, 1.0]]);
		let mut foreground_target = texture_2d(1, 1, &[sentinel]);
		let mut foreground_descriptors = DescriptorBindings::new();
		foreground_descriptors.bind_texture(ResourceSlot::new(0), &mut foreground_depth);
		foreground_descriptors.bind_image(ResourceSlot::new(1), &mut foreground_target);
		run_at(&program, &mut foreground_descriptors, [0, 0]);
		drop(foreground_descriptors);
		assert_rgba_close(rgba(&foreground_target, [0, 0]), sentinel, 0.0);

		let parameter_slot = ResourceSlot::new(4);
		let mut parameters = default_parameters(&program, parameter_slot);
		let sky_scattering = [0.2, 0.3, 0.4, 1.0];
		let mut sky_view = texture_2d(1, 1, &[sky_scattering]);
		let mut transmittance = texture_2d(1, 1, &[[1.0, 1.0, 1.0, 1.0]]);

		let mut background_depth = texture_2d(1, 1, &[[0.0, 0.0, 0.0, 1.0]]);
		let mut background_target = empty_image(1, 1);
		let mut background_descriptors = DescriptorBindings::new();
		background_descriptors.bind_texture(ResourceSlot::new(0), &mut background_depth);
		background_descriptors.bind_image(ResourceSlot::new(1), &mut background_target);
		background_descriptors.bind_texture(ResourceSlot::new(2), &mut sky_view);
		background_descriptors.bind_texture(ResourceSlot::new(3), &mut transmittance);
		background_descriptors.bind_buffer(parameter_slot, &mut parameters);
		run_at(&program, &mut background_descriptors, [0, 0]);
		drop(background_descriptors);

		let background = rgba(&background_target, [0, 0]);
		assert!(
			background[..3]
				.iter()
				.all(|channel| channel.is_finite() && (0.0..=1.0).contains(channel)),
			"Invalid sky VM output. The most likely cause is unstable atmosphere integration: {background:?}"
		);
		assert!(
			background[..3].iter().any(|channel| *channel > 0.0),
			"Empty sky VM output. The most likely cause is an invalid view ray or atmosphere intersection: {background:?}"
		);
		assert_rgba_close([0.0, 0.0, 0.0, background[3]], [0.0, 0.0, 0.0, 1.0], 1e-6);

		// Visibility stores transparent-only pixels premultiplied, so the post-scene sky must fill the remaining coverage.
		let transparent_foreground = [0.1, 0.05, 0.02, 0.25];
		let mut transparent_depth = texture_2d(1, 1, &[[0.0, 0.0, 0.0, 1.0]]);
		let mut transparent_target = texture_2d(1, 1, &[transparent_foreground]);
		let mut transparent_descriptors = DescriptorBindings::new();
		transparent_descriptors.bind_texture(ResourceSlot::new(0), &mut transparent_depth);
		transparent_descriptors.bind_image(ResourceSlot::new(1), &mut transparent_target);
		transparent_descriptors.bind_texture(ResourceSlot::new(2), &mut sky_view);
		transparent_descriptors.bind_texture(ResourceSlot::new(3), &mut transmittance);
		transparent_descriptors.bind_buffer(parameter_slot, &mut parameters);
		run_at(&program, &mut transparent_descriptors, [0, 0]);
		drop(transparent_descriptors);

		let remaining_alpha = 1.0 - transparent_foreground[3];
		assert_rgba_close(
			rgba(&transparent_target, [0, 0]),
			[
				transparent_foreground[0] + background[0] * remaining_alpha,
				transparent_foreground[1] + background[1] * remaining_alpha,
				transparent_foreground[2] + background[2] * remaining_alpha,
				1.0,
			],
			1e-5,
		);
	}
}
