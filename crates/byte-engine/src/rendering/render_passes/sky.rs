use ghi::{
	command_buffer::{BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, CommonCommandBufferMode as _},
	context::{Context as _, ContextCreate as _},
	frame::Frame as _,
};
use math::{mat::MatInverse as _, ShaderMatrix4, Vector3, Vector4};
use resource_management::{
	resources::material, shader::generator::ShaderGenerationSettings, types::ShaderTypes as ResourceShaderTypes,
};
use utils::{Box, Extent};

use crate::{
	core::Entity,
	rendering::{
		render_pass::{RenderPass, RenderPassBuilder, RenderPassReturn},
		shader_store::{ShaderSourceDefinition, ShaderSourceDescriptor},
		Sink,
	},
};

const SKY_DEPTH_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(
	0,
	ghi::descriptors::DescriptorType::CombinedImageSampler,
	ghi::Stages::COMPUTE,
);
const SKY_MAIN_BINDING: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(1, ghi::descriptors::DescriptorType::StorageImage, ghi::Stages::COMPUTE);
const SKY_PARAMETERS_BINDING: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(2, ghi::descriptors::DescriptorType::StorageBuffer, ghi::Stages::COMPUTE)
		.buffer_read_only(true);

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

/// The `AtmosphereSkyRenderPass` struct renders an atmosphere-only sky into the main color target wherever depth remains at infinity.
pub struct AtmosphereSkyRenderPass {
	pipeline: ghi::PipelineHandle,
	descriptor_set: ghi::DescriptorSetHandle,
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
		let main = render_pass_builder.render_to("main");

		let shader_storage = render_pass_builder.shader_storage();
		let context = render_pass_builder.context();

		let descriptor_set_template = context.create_descriptor_set_template(
			Some("Sky Render Pass Descriptor Set"),
			&[SKY_DEPTH_BINDING, SKY_MAIN_BINDING, SKY_PARAMETERS_BINDING],
		);

		let shader = create_sky_shader(context, shader_storage);

		let pipeline = context.create_compute_pipeline(ghi::pipelines::compute::Builder::new(
			&[descriptor_set_template],
			&[],
			ghi::ShaderParameter::new(&shader, ghi::ShaderTypes::Compute),
		));

		let parameters = context.build_dynamic_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Sky Render Pass Parameters")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);

		let sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Linear)
				.mip_map_mode(ghi::FilteringModes::Linear)
				.addressing_mode(ghi::SamplerAddressingModes::Clamp),
		);

		let descriptor_set = context.create_descriptor_set(Some("Sky Render Pass Descriptor Set"), &descriptor_set_template);
		let _ = context.create_descriptor_binding(
			descriptor_set,
			ghi::BindingConstructor::combined_image_sampler(&SKY_DEPTH_BINDING, depth, sampler, ghi::Layouts::Read),
		);
		let _ = context.create_descriptor_binding(descriptor_set, ghi::BindingConstructor::image(&SKY_MAIN_BINDING, main));
		let _ = context.create_descriptor_binding(
			descriptor_set,
			ghi::BindingConstructor::buffer(&SKY_PARAMETERS_BINDING, parameters.into()),
		);

		Self {
			pipeline,
			descriptor_set,
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

fn create_sky_shader(
	context: &mut ghi::implementation::Context,
	shader_storage: Option<&dyn resource_management::resource::StorageBackend>,
) -> ghi::ShaderHandle {
	crate::rendering::shader_store::create_shader(
		context,
		shader_storage,
		&ShaderSourceDescriptor {
			id: "byte-engine/rendering/sky",
			name: "Sky Render Pass Compute Shader",
			stage: ResourceShaderTypes::Compute,
			source: ShaderSourceDefinition::Besl {
				settings: ShaderGenerationSettings::compute(Extent::new(8, 8, 1))
					.name("Sky Render Pass Compute Shader".to_string()),
				main_node: create_sky_program(),
			},
			interface: material::ShaderInterface {
				workgroup_size: Some((8, 8, 1)),
				bindings: vec![
					material::Binding::new(0, 0, true, false),
					material::Binding::new(0, 1, false, true),
					material::Binding::new(0, 2, true, false),
				],
			},
		},
	)
	.expect("Failed to create the sky shader. The most likely cause is an incompatible shader interface.")
}

fn create_sky_program() -> besl::NodeReference {
	let mut root = besl::Node::root();
	let vec4f = root.get_child("vec4f").expect("vec4f type not found in BESL root");
	let mat4f = root.get_child("mat4f").expect("mat4f type not found in BESL root");

	root.add_child(
		besl::Node::binding(
			"depth_texture",
			besl::BindingTypes::CombinedImageSampler { format: String::new() },
			0,
			0,
			true,
			false,
		)
		.into(),
	);
	root.add_child(
		besl::Node::binding(
			"main_texture",
			besl::BindingTypes::Image { format: String::new() },
			0,
			1,
			false,
			true,
		)
		.into(),
	);
	root.add_child(
		besl::Node::binding(
			"parameters",
			besl::BindingTypes::Buffer {
				members: vec![
					besl::Node::member("inverse_view_projection", mat4f).into(),
					besl::Node::member("camera_position", vec4f.clone()).into(),
					besl::Node::member("sun_direction", vec4f.clone()).into(),
					besl::Node::member("planet_center", vec4f.clone()).into(),
					besl::Node::member("atmosphere", vec4f.clone()).into(),
					besl::Node::member("misc", vec4f).into(),
				],
			},
			0,
			2,
			true,
			false,
		)
		.into(),
	);

	let program = besl::compile_to_besl(SKY_SHADER_BESL, Some(root))
		.expect("Failed to compile the sky BESL shader. The most likely cause is invalid BESL syntax.");
	program
		.get_main()
		.expect("Failed to find the sky BESL entry point. The most likely cause is that the BESL program did not define main.")
}

impl RenderPass for AtmosphereSkyRenderPass {
	fn prepare<'a>(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		self.write_parameters(frame, sink);

		let pipeline = self.pipeline;
		let descriptor_set = self.descriptor_set;
		let extent = sink.extent();

		Some(crate::rendering::render_pass::allocate_render_command(
			frame_allocator,
			move |command_buffer, _| {
				command_buffer.region(
					|label| label.write_str("Sky"),
					|command_buffer| {
						let pipeline = command_buffer.bind_compute_pipeline(pipeline);
						pipeline.bind_descriptor_sets(&[descriptor_set]);
						pipeline.dispatch(ghi::DispatchExtent::new(extent, Extent::new(8, 8, 1)));
					},
				);
			},
		))
	}
}

const SKY_SHADER_BESL: &str = r#"
get_camera_position: fn () -> vec3f {
	return vec3f(parameters.camera_position.x, parameters.camera_position.y, parameters.camera_position.z);
}

get_sun_direction: fn () -> vec3f {
	return vec3f(parameters.sun_direction.x, parameters.sun_direction.y, parameters.sun_direction.z);
}

get_planet_center: fn () -> vec3f {
	return vec3f(parameters.planet_center.x, parameters.planet_center.y, parameters.planet_center.z);
}

get_ground_radius: fn () -> f32 { return parameters.atmosphere.x; }
get_atmosphere_radius: fn () -> f32 { return parameters.atmosphere.y; }
get_rayleigh_scale_height: fn () -> f32 { return parameters.atmosphere.z; }
get_mie_scale_height: fn () -> f32 { return parameters.atmosphere.w; }
get_mie_g: fn () -> f32 { return parameters.sun_direction.w; }
get_sun_intensity: fn () -> f32 { return parameters.camera_position.w; }
get_sun_angular_radius: fn () -> f32 { return parameters.planet_center.w; }
get_ozone_strength: fn () -> f32 { return parameters.misc.x; }

should_skip_below_horizon: fn () -> bool {
	return parameters.misc.y > 0.5;
}

ray_sphere_discriminant: fn(origin: vec3f, direction: vec3f, center: vec3f, radius: f32) -> f32 {
	let oc: vec3f = origin - center;
	let b: f32 = dot(oc, direction);
	let c: f32 = dot(oc, oc) - radius * radius;
	return b * b - c;
}

ray_sphere_t_min: fn(origin: vec3f, direction: vec3f, center: vec3f, radius: f32) -> f32 {
	let oc: vec3f = origin - center;
	let b: f32 = dot(oc, direction);
	let discriminant: f32 = ray_sphere_discriminant(origin, direction, center, radius);
	return (0.0 - b) - sqrt(discriminant);
}

ray_sphere_t_max: fn(origin: vec3f, direction: vec3f, center: vec3f, radius: f32) -> f32 {
	let oc: vec3f = origin - center;
	let b: f32 = dot(oc, direction);
	let discriminant: f32 = ray_sphere_discriminant(origin, direction, center, radius);
	return (0.0 - b) + sqrt(discriminant);
}

density_profile: fn(sample_position: vec3f) -> vec3f {
	let altitude: f32 = length(sample_position - get_planet_center()) - get_ground_radius();
	if (altitude < 0.0) {
		return vec3f(0.0, 0.0, 0.0);
	}
	let rayleigh: f32 = exp((0.0 - altitude) / get_rayleigh_scale_height());
	let mie: f32 = exp((0.0 - altitude) / get_mie_scale_height());
	let ozone: f32 = max(0.0, 1.0 - abs(altitude - 25000.0) / 15000.0) * get_ozone_strength();
	return vec3f(rayleigh, mie, ozone);
}

extinction_from_density: fn(density: vec3f) -> vec3f {
	let beta_rayleigh: vec3f = vec3f(0.000005802, 0.000013558, 0.000033100);
	let beta_mie: vec3f = vec3f(0.000003996, 0.000003996, 0.000003996);
	let beta_ozone: vec3f = vec3f(0.000000650, 0.000001881, 0.000000085);
	return density.x * beta_rayleigh + density.y * beta_mie + density.z * beta_ozone;
}

interleaved_gradient_noise: fn(pixel: vec2u) -> f32 {
	return fract(52.9829189 * fract(0.06711056 * f32(pixel.x) + 0.00583715 * f32(pixel.y)));
}

sample_distribution: fn(u: f32) -> f32 {
	let clamped: f32 = clamp(u, 0.0, 1.0);
	return clamped * clamped;
}

phase_rayleigh: fn(cosine_theta: f32) -> f32 {
	let pi: f32 = 3.14159265359;
	return (3.0 / (16.0 * pi)) * (1.0 + cosine_theta * cosine_theta);
}

phase_mie: fn(cosine_theta: f32, g: f32) -> f32 {
	let pi: f32 = 3.14159265359;
	let g2: f32 = g * g;
	let denominator: f32 = 1.0 + g2 - 2.0 * g * cosine_theta;
	let denominator_sqrt: f32 = sqrt(max(0.0001, denominator));
	return (3.0 / (8.0 * pi)) * ((1.0 - g2) * (1.0 + cosine_theta * cosine_theta)) / ((2.0 + g2) * max(0.0001, denominator * denominator_sqrt));
}

march_transmittance: fn(origin: vec3f, direction: vec3f) -> vec3f {
	let atmosphere_discriminant: f32 = ray_sphere_discriminant(origin, direction, get_planet_center(), get_atmosphere_radius());
	if (atmosphere_discriminant < 0.0) {
		return vec3f(1.0, 1.0, 1.0);
	}
	let t_min: f32 = max(0.0, ray_sphere_t_min(origin, direction, get_planet_center(), get_atmosphere_radius()));
	let t_max: f32 = ray_sphere_t_max(origin, direction, get_planet_center(), get_atmosphere_radius());
	let ground_discriminant: f32 = ray_sphere_discriminant(origin, direction, get_planet_center(), get_ground_radius());
	if (ground_discriminant >= 0.0) {
		let ground_t_max: f32 = ray_sphere_t_max(origin, direction, get_planet_center(), get_ground_radius());
		if (ground_t_max > 0.0) {
			return vec3f(0.0, 0.0, 0.0);
		}
	}
	let distance_through_atmosphere: f32 = t_max - t_min;
	let optical_depth: vec3f = vec3f(0.0, 0.0, 0.0);
	for (let i: u32 = 0; i < 4; i = i + 1) {
		let t0: f32 = t_min + distance_through_atmosphere * sample_distribution(f32(i) / 4.0);
		let t1: f32 = t_min + distance_through_atmosphere * sample_distribution(f32(i + 1) / 4.0);
		let t: f32 = 0.5 * (t0 + t1);
		let step_size: f32 = t1 - t0;
		let sample_position: vec3f = origin + direction * t;
		let density: vec3f = density_profile(sample_position);
		optical_depth = optical_depth + density * step_size;
	}
	return exp(vec3f(0.0, 0.0, 0.0) - extinction_from_density(optical_depth));
}

integrate_atmosphere: fn(pixel: vec2u, origin: vec3f, direction: vec3f) -> vec3f {
	let atmosphere_discriminant: f32 = ray_sphere_discriminant(origin, direction, get_planet_center(), get_atmosphere_radius());
	if (atmosphere_discriminant < 0.0) {
		return vec3f(0.0, 0.0, 0.0);
	}
	let atmosphere_t_min: f32 = max(0.0, ray_sphere_t_min(origin, direction, get_planet_center(), get_atmosphere_radius()));
	let atmosphere_t_max: f32 = ray_sphere_t_max(origin, direction, get_planet_center(), get_atmosphere_radius());
	let distance_through_atmosphere: f32 = atmosphere_t_max - atmosphere_t_min;
	let optical_depth: vec3f = vec3f(0.0, 0.0, 0.0);
	let luminance: vec3f = vec3f(0.0, 0.0, 0.0);
	let sun_direction: vec3f = get_sun_direction();
	let cosine_theta: f32 = dot(direction, sun_direction);
	let phase_r: f32 = phase_rayleigh(cosine_theta);
	let phase_m: f32 = phase_mie(cosine_theta, get_mie_g());
	let jitter: f32 = interleaved_gradient_noise(pixel);
	for (let i: u32 = 0; i < 16; i = i + 1) {
		let t0: f32 = atmosphere_t_min + distance_through_atmosphere * sample_distribution(f32(i) / 16.0);
		let t1: f32 = atmosphere_t_min + distance_through_atmosphere * sample_distribution(f32(i + 1) / 16.0);
		let t: f32 = mix(t0, t1, jitter);
		let step_size: f32 = t1 - t0;
		let sample_position: vec3f = origin + direction * t;
		let density: vec3f = density_profile(sample_position);
		optical_depth = optical_depth + density * step_size;
		let transmittance_to_camera: vec3f = exp(vec3f(0.0, 0.0, 0.0) - extinction_from_density(optical_depth));
		let transmittance_to_sun: vec3f = march_transmittance(sample_position, sun_direction);
		let beta_rayleigh: vec3f = vec3f(0.000005802, 0.000013558, 0.000033100);
		let beta_mie: vec3f = vec3f(0.000003996, 0.000003996, 0.000003996);
		let scattering: vec3f = density.x * beta_rayleigh * phase_r + density.y * beta_mie * phase_m;
		luminance = luminance + transmittance_to_camera * transmittance_to_sun * scattering * step_size;
	}
	let sun_disk: f32 = smoothstep(cos(get_sun_angular_radius() * 1.4), cos(get_sun_angular_radius()), cosine_theta);
	let sun_radiance: vec3f = vec3f(0.0, 0.0, 0.0);
	if (sun_disk > 0.0) {
		let sun_transmittance: vec3f = march_transmittance(origin, sun_direction);
		sun_radiance = sun_disk * sun_transmittance * vec3f(20.0, 18.0, 16.0);
	}
	let color: vec3f = luminance * get_sun_intensity() + sun_radiance;
	return color / (vec3f(1.0, 1.0, 1.0) + color);
}

reconstruct_view_direction: fn(pixel: vec2u, extent: vec2u) -> vec3f {
	let uv: vec2f = (vec2f(f32(pixel.x), f32(pixel.y)) + vec2f(0.5, 0.5)) / vec2f(f32(extent.x), f32(extent.y));
	let ndc: vec2f = vec2f(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
	let world: vec4f = parameters.inverse_view_projection * vec4f(ndc.x, ndc.y, 0.0, 1.0);
	return normalize(vec3f(world.x / world.w, world.y / world.w, world.z / world.w) - get_camera_position());
}

is_below_horizon: fn(origin: vec3f, direction: vec3f) -> bool {
	let local_up: vec3f = normalize(origin - get_planet_center());
	return dot(direction, local_up) < 0.0;
}

main: fn () -> void {
	let pixel: vec2u = thread_id();
	guard_image_bounds(main_texture, pixel);
	let extent: vec2u = image_size(main_texture);
	let depth_uv: vec2f = (vec2f(f32(pixel.x), f32(pixel.y)) + vec2f(0.5, 0.5)) / vec2f(f32(extent.x), f32(extent.y));
	let depth: f32 = texture_lod(depth_texture, depth_uv).x;
	if (depth > 0.000001) {
		return;
	}
	let direction: vec3f = reconstruct_view_direction(pixel, extent);
	if (should_skip_below_horizon()) {
		if (is_below_horizon(get_camera_position(), direction)) {
			return;
		}
	}
	let sky: vec3f = integrate_atmosphere(pixel, get_camera_position(), direction);
	write(main_texture, pixel, vec4f(sky.x, sky.y, sky.z, 1.0));
}
"#;

#[cfg(test)]
mod tests {
	use besl::vm::{DescriptorBindings, DescriptorSlot, Value};
	use math::{mat::MatInverse as _, ShaderMatrix4, Vector3};
	use resource_management::shader::besl::{backends::glsl::GLSLShaderGenerator, backends::msl::MSLShaderGenerator};
	use resource_management::shader::generator::{ShaderGenerationSettings, ShaderGenerator as _};
	use utils::Extent;

	use crate::rendering::shader_vm_test::{assert_rgba_close, buffer, empty_image, rgba, run_at, texture_2d};

	/// Verifies foreground preservation and a finite default atmosphere result through the VM.
	#[test]
	fn sky_besl_vm_preserves_foreground_and_renders_a_bounded_default_background() {
		let program = crate::rendering::shader_vm_test::compile(super::create_sky_program());
		let sentinel = [0.2, 0.3, 0.4, 0.5];
		let mut foreground_depth = texture_2d(1, 1, &[[0.5, 0.0, 0.0, 1.0]]);
		let mut foreground_target = texture_2d(1, 1, &[sentinel]);
		let mut foreground_descriptors = DescriptorBindings::new();
		foreground_descriptors.bind_texture(DescriptorSlot::new(0, 0), &mut foreground_depth);
		foreground_descriptors.bind_image(DescriptorSlot::new(0, 1), &mut foreground_target);
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
		let parameter_slot = DescriptorSlot::new(0, 2);
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
		background_descriptors.bind_texture(DescriptorSlot::new(0, 0), &mut background_depth);
		background_descriptors.bind_image(DescriptorSlot::new(0, 1), &mut background_target);
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
	}

	#[test]
	fn sky_besl_shader_lowers_to_platform_sources() {
		let main_node = super::create_sky_program();
		let settings = ShaderGenerationSettings::compute(Extent::new(8, 8, 1)).name("Sky Render Pass Test".to_string());

		GLSLShaderGenerator::new()
			.generate(&settings, &main_node)
			.expect("Failed to lower sky BESL shader to GLSL.");
		MSLShaderGenerator::new()
			.generate(&settings, &main_node)
			.expect("Failed to lower sky BESL shader to MSL.");
	}
}
