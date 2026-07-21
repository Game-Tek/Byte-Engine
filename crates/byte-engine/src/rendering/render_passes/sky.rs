use ghi::{
	context::{Context as _, ContextCreate as _},
	frame::Frame as _,
};
use math::{mat::MatInverse as _, ShaderMatrix4, Vector3, Vector4};
use utils::Extent;

use crate::{
	core::Entity,
	rendering::{
		render_pass::{simple_compute, RenderPass, RenderPassBuilder, RenderPassReturn},
		Sink,
	},
};

/// The `AtmosphereSkyRenderPassSettings` struct configures the physical atmosphere and sun parameters for the sky pass.
#[derive(Clone, Copy, Debug)]
pub struct AtmosphereSkyRenderPassSettings {
	pub sun_direction: Vector3,
	pub sun_intensity: f32,
	pub sun_angular_radius: f32,
	pub ground_radius: f32,
	pub atmosphere_radius: f32,
	pub rayleigh_scale_height: f32,
	pub mie_scale_height: f32,
	pub mie_anisotropy: f32,
	pub ozone_strength: f32,
	pub skip_below_horizon: bool,
	pub planet_center: Vector3,
}

impl Default for AtmosphereSkyRenderPassSettings {
	fn default() -> Self {
		Self {
			sun_direction: Vector3::new(0.35, 0.85, 0.4),
			sun_intensity: 22.0,
			sun_angular_radius: 0.004675,
			ground_radius: 6_360_000.0,
			atmosphere_radius: 6_460_000.0,
			rayleigh_scale_height: 8_000.0,
			mie_scale_height: 1_200.0,
			mie_anisotropy: 0.76,
			ozone_strength: 1.0,
			skip_below_horizon: true,
			planet_center: Vector3::new(0.0, -6_360_000.0, 0.0),
		}
	}
}

#[repr(C)]
#[derive(Clone, Copy)]
struct SkyShaderData {
	inverse_view_projection: ShaderMatrix4,
	camera_position: [f32; 4],
	sun_direction: [f32; 4],
	planet_center: [f32; 4],
	atmosphere: [f32; 4],
	misc: [f32; 4],
}

/// The `AtmosphereSkyRenderPass` struct places an atmosphere behind scene color wherever opaque depth remains at infinity.
pub struct AtmosphereSkyRenderPass {
	pass: simple_compute::Pass,
	parameters: ghi::DynamicBufferHandle<SkyShaderData>,
	settings: AtmosphereSkyRenderPassSettings,
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
		let pipeline = simple_compute::Pipeline::compile(
			render_pass_builder,
			simple_compute::Descriptor::new("Sky", "byte-engine/rendering/sky.besl", "Sky Render Pass Compute Shader"),
		)
		.expect("Failed to create the sky shader. The most likely cause is an incompatible shader interface.");
		let parameters = render_pass_builder.context().build_dynamic_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Sky Render Pass Parameters")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let sampler = render_pass_builder.context().build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Linear)
				.mip_map_mode(ghi::FilteringModes::Linear)
				.addressing_mode(ghi::SamplerAddressingModes::Clamp),
		);
		let pass = pipeline
			.bind(
				render_pass_builder,
				"Sky Render Pass Descriptor Set",
				&[
					simple_compute::Resource::combined_image_sampler("depth_texture", depth, sampler, ghi::Layouts::Read),
					simple_compute::Resource::image("main_texture", main),
					simple_compute::Resource::buffer("parameters", parameters),
				],
			)
			.expect("Failed to bind the sky resources. The most likely cause is a changed BESL binding contract.");

		Self {
			pass,
			parameters,
			settings,
		}
	}

	/// Updates per-view sky constants from the active camera before dispatch.
	fn write_parameters(&self, frame: &mut ghi::implementation::Frame, sink: &Sink) {
		let view = sink.view();
		let inverse_view_projection = view.view_projection().inverse();
		let inverse_view = view.view().inverse();
		let camera_position = inverse_view * Vector4::new(0.0, 0.0, 0.0, 1.0);
		let sun_direction = math::normalize(self.settings.sun_direction);
		let planet_center = [
			camera_position.x + self.settings.planet_center.x,
			self.settings.planet_center.y,
			camera_position.z + self.settings.planet_center.z,
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
		parameters.sun_direction = [sun_direction.x, sun_direction.y, sun_direction.z, settings.mie_anisotropy];
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
	}
}

impl RenderPass for AtmosphereSkyRenderPass {
	fn prepare<'a>(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		self.write_parameters(frame, sink);
		self.pass.prepare(frame, sink, frame_allocator)
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
	use besl::vm::{DescriptorBindings, ResourceSlot, Value};
	use math::{mat::MatInverse as _, ShaderMatrix4, Vector3};

	use super::simple_compute;
	use crate::rendering::shader_vm_test::{assert_rgba_close, buffer, empty_image, rgba, run_at, texture_2d};

	const SKY_SHADER_BESL: &str = include_str!("../../../assets/rendering/sky.besl");

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

		let settings = super::AtmosphereSkyRenderPassSettings::default();
		let view = crate::rendering::View::new_perspective(
			60.0,
			1.0,
			0.1,
			100.0,
			Vector3::new(0.0, 0.0, 0.0),
			Vector3::new(0.0, 0.0, 1.0),
		);
		let inverse_view_projection = ShaderMatrix4::from(view.view_projection().inverse()).0;
		let sun_direction = math::normalize(settings.sun_direction);
		let parameter_slot = ResourceSlot::new(2);
		let mut parameters = buffer(&program, parameter_slot);
		// Mirror the production upload field-for-field so the VM validates the real atmosphere parameter contract.
		for (name, value) in [
			("camera_position", [0.0, 0.0, 0.0, settings.sun_intensity]),
			(
				"sun_direction",
				[sun_direction.x, sun_direction.y, sun_direction.z, settings.mie_anisotropy],
			),
			(
				"planet_center",
				[
					settings.planet_center.x,
					settings.planet_center.y,
					settings.planet_center.z,
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

		let mut background_depth = texture_2d(1, 1, &[[0.0, 0.0, 0.0, 1.0]]);
		let mut background_target = empty_image(1, 1);
		let mut background_descriptors = DescriptorBindings::new();
		background_descriptors.bind_texture(ResourceSlot::new(0), &mut background_depth);
		background_descriptors.bind_image(ResourceSlot::new(1), &mut background_target);
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
