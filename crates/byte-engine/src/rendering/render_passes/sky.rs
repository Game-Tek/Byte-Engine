use std::borrow::Borrow;

use ghi::{
	command_buffer::{BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, CommonCommandBufferMode as _},
	device::{Device as _, DeviceCreate as _},
	frame::Frame as _,
};
use math::{mat::MatInverse as _, ShaderMatrix4, Vector3, Vector4};
use resource_management::glsl;
use utils::{Box, Extent};

use crate::{
	core::Entity,
	rendering::{
		render_pass::{RenderPass, RenderPassBuilder, RenderPassReturn},
		Viewport,
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
	ghi::DescriptorSetBindingTemplate::new(2, ghi::descriptors::DescriptorType::StorageBuffer, ghi::Stages::COMPUTE);

/// The `SkyRenderPassSettings` struct configures the physical atmosphere and sun parameters for the sky pass.
#[derive(Clone, Copy, Debug)]
pub struct SkyRenderPassSettings {
	pub sun_direction: Vector3,
	pub sun_intensity: f32,
	pub sun_angular_radius: f32,
	pub ground_radius: f32,
	pub atmosphere_radius: f32,
	pub rayleigh_scale_height: f32,
	pub mie_scale_height: f32,
	pub mie_anisotropy: f32,
	pub ozone_strength: f32,
	pub planet_center: Vector3,
}

impl Default for SkyRenderPassSettings {
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

/// The `SkyRenderPass` struct renders an atmosphere-only sky into the main color target wherever depth remains at infinity.
pub struct SkyRenderPass {
	pipeline: ghi::PipelineHandle,
	descriptor_set: ghi::DescriptorSetHandle,
	parameters: ghi::DynamicBufferHandle<SkyShaderData>,
	settings: SkyRenderPassSettings,
}

impl Entity for SkyRenderPass {}

impl SkyRenderPass {
	/// Creates a sky pass with physically plausible default atmosphere settings.
	pub fn new(render_pass_builder: &mut RenderPassBuilder) -> Self {
		Self::with_settings(render_pass_builder, SkyRenderPassSettings::default())
	}

	/// Creates a sky pass with caller-supplied atmosphere settings.
	pub fn with_settings(render_pass_builder: &mut RenderPassBuilder, settings: SkyRenderPassSettings) -> Self {
		let depth = render_pass_builder.read_from("depth");
		let main = render_pass_builder.render_to("main");

		let device = render_pass_builder.device();

		let descriptor_set_template = device.create_descriptor_set_template(
			Some("Sky Render Pass Descriptor Set"),
			&[SKY_DEPTH_BINDING, SKY_MAIN_BINDING, SKY_PARAMETERS_BINDING],
		);

		let shader = create_sky_shader(device);

		let pipeline = device.create_compute_pipeline(ghi::pipelines::compute::Builder::new(
			&[descriptor_set_template],
			&[],
			ghi::ShaderParameter::new(&shader, ghi::ShaderTypes::Compute),
		));

		let parameters = device.build_dynamic_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Sky Render Pass Parameters")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);

		let sampler = device.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Linear)
				.mip_map_mode(ghi::FilteringModes::Linear)
				.addressing_mode(ghi::SamplerAddressingModes::Clamp),
		);

		let descriptor_set = device.create_descriptor_set(Some("Sky Render Pass Descriptor Set"), &descriptor_set_template);
		let _ = device.create_descriptor_binding(
			descriptor_set,
			ghi::BindingConstructor::combined_image_sampler(&SKY_DEPTH_BINDING, depth, sampler, ghi::Layouts::Read),
		);
		let _ = device.create_descriptor_binding(descriptor_set, ghi::BindingConstructor::image(&SKY_MAIN_BINDING, main));
		let _ = device.create_descriptor_binding(
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
	fn write_parameters(&self, frame: &mut ghi::implementation::Frame, viewport: &Viewport) {
		let view = viewport.view();
		let inverse_view_projection = view.view_projection().inverse();
		let inverse_view = view.view().inverse();
		let camera_position = inverse_view * Vector4::new(0.0, 0.0, 0.0, 1.0);
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
		parameters.sun_direction = [
			settings.sun_direction.x,
			settings.sun_direction.y,
			settings.sun_direction.z,
			settings.mie_anisotropy,
		];
		parameters.planet_center = planet_center;
		parameters.atmosphere = [
			settings.ground_radius,
			settings.atmosphere_radius,
			settings.rayleigh_scale_height,
			settings.mie_scale_height,
		];
		parameters.misc = [settings.ozone_strength, 0.0, 0.0, 0.0];
	}
}

fn create_sky_shader(device: &mut ghi::implementation::Device) -> ghi::ShaderHandle {
	if ghi::implementation::USES_METAL {
		return device
			.create_shader(
				Some("Sky Render Pass Compute Shader"),
				ghi::shader::Sources::MTL {
					source: SKY_SHADER_MSL,
					entry_point: "sky_render_pass",
				},
				ghi::ShaderTypes::Compute,
				[
					SKY_DEPTH_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
					SKY_MAIN_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::WRITE),
					SKY_PARAMETERS_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
				],
			)
			.expect("Failed to create the sky shader. The most likely cause is an incompatible Metal shader interface.");
	}

	let shader_artifact = glsl::compile(SKY_SHADER, "Sky Render Pass")
		.expect("Failed to compile the sky shader. The most likely cause is invalid GLSL syntax in the sky render pass.");

	device
		.create_shader(
			Some("Sky Render Pass Compute Shader"),
			ghi::shader::Sources::SPIRV(shader_artifact.borrow().into()),
			ghi::ShaderTypes::Compute,
			[
				SKY_DEPTH_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
				SKY_MAIN_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::WRITE),
				SKY_PARAMETERS_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
			],
		)
		.expect("Failed to create the sky shader. The most likely cause is an incompatible shader interface.")
}

impl RenderPass for SkyRenderPass {
	fn prepare(&mut self, frame: &mut ghi::implementation::Frame, viewport: &Viewport) -> Option<RenderPassReturn> {
		self.write_parameters(frame, viewport);

		let pipeline = self.pipeline;
		let descriptor_set = self.descriptor_set;
		let extent = viewport.extent();

		Some(Box::new(move |command_buffer, _| {
			command_buffer.region("Sky", |command_buffer| {
				let pipeline = command_buffer.bind_compute_pipeline(pipeline);
				pipeline.bind_descriptor_sets(&[descriptor_set]);
				pipeline.dispatch(ghi::DispatchExtent::new(extent, Extent::new(8, 8, 1)));
			});
		}))
	}
}

const SKY_SHADER: &str = r#"
#version 460 core
#pragma shader_stage(compute)

#extension GL_EXT_shader_image_load_formatted:enable
#extension GL_EXT_scalar_block_layout: enable

layout(row_major) uniform;
layout(row_major) buffer;

layout(set=0, binding=0) uniform sampler2D depth_texture;
layout(set=0, binding=1) uniform image2D main_texture;

struct SkyParameters {
	mat4 inverse_view_projection;
	vec4 camera_position;
	vec4 sun_direction;
	vec4 planet_center;
	vec4 atmosphere;
	vec4 misc;
};

layout(set=0, binding=2, scalar) readonly buffer SkyParametersBuffer {
	SkyParameters parameters;
};

layout(local_size_x=8, local_size_y=8, local_size_z=1) in;

const float PI = 3.14159265359;
const int VIEW_SAMPLE_COUNT = 64;
const int LIGHT_SAMPLE_COUNT = 16;

const vec3 BETA_RAYLEIGH = vec3(5.802e-6, 13.558e-6, 33.100e-6);
const vec3 BETA_MIE = vec3(3.996e-6);
const vec3 BETA_OZONE = vec3(0.650e-6, 1.881e-6, 0.085e-6);

vec3 get_camera_position() {
	return parameters.camera_position.xyz;
}

vec3 get_sun_direction() {
	return normalize(parameters.sun_direction.xyz);
}

vec3 get_planet_center() {
	return parameters.planet_center.xyz;
}

float get_ground_radius() {
	return parameters.atmosphere.x;
}

float get_atmosphere_radius() {
	return parameters.atmosphere.y;
}

float get_rayleigh_scale_height() {
	return parameters.atmosphere.z;
}

float get_mie_scale_height() {
	return parameters.atmosphere.w;
}

float get_mie_g() {
	return parameters.sun_direction.w;
}

float get_sun_intensity() {
	return parameters.camera_position.w;
}

float get_sun_angular_radius() {
	return parameters.planet_center.w;
}

float get_ozone_strength() {
	return parameters.misc.x;
}

bool ray_sphere_intersection(vec3 origin, vec3 direction, vec3 center, float radius, out float t_min, out float t_max) {
	vec3 oc = origin - center;
	float b = dot(oc, direction);
	float c = dot(oc, oc) - radius * radius;
	float discriminant = b * b - c;

	if (discriminant < 0.0) {
		return false;
	}

	float root = sqrt(discriminant);
	t_min = -b - root;
	t_max = -b + root;
	return true;
}

vec3 density_profile(vec3 sample_position) {
	float altitude = length(sample_position - get_planet_center()) - get_ground_radius();

	if (altitude < 0.0) {
		return vec3(0.0);
	}

	float rayleigh = exp(-altitude / get_rayleigh_scale_height());
	float mie = exp(-altitude / get_mie_scale_height());
	float ozone = max(0.0, 1.0 - abs(altitude - 25000.0) / 15000.0) * get_ozone_strength();

	return vec3(rayleigh, mie, ozone);
}

vec3 extinction_from_density(vec3 density) {
	return density.x * BETA_RAYLEIGH + density.y * BETA_MIE + density.z * BETA_OZONE;
}

float interleaved_gradient_noise(ivec2 pixel) {
	return fract(52.9829189 * fract(0.06711056 * float(pixel.x) + 0.00583715 * float(pixel.y)));
}

float sample_distribution(float u) {
	u = clamp(u, 0.0, 1.0);
	return u * u;
}

float phase_rayleigh(float cosine_theta) {
	return (3.0 / (16.0 * PI)) * (1.0 + cosine_theta * cosine_theta);
}

float phase_mie(float cosine_theta, float g) {
	float g2 = g * g;
	float denominator = 1.0 + g2 - 2.0 * g * cosine_theta;
	return (3.0 / (8.0 * PI)) * ((1.0 - g2) * (1.0 + cosine_theta * cosine_theta)) /
		((2.0 + g2) * max(1e-4, pow(denominator, 1.5)));
}

vec3 march_transmittance(vec3 origin, vec3 direction) {
	float t_min;
	float t_max;

	if (!ray_sphere_intersection(origin, direction, get_planet_center(), get_atmosphere_radius(), t_min, t_max)) {
		return vec3(1.0);
	}

	t_min = max(0.0, t_min);
	float distance_through_atmosphere = t_max - t_min;
	vec3 optical_depth = vec3(0.0);

	for (int i = 0; i < LIGHT_SAMPLE_COUNT; ++i) {
		float t0 = t_min + distance_through_atmosphere * sample_distribution(float(i) / float(LIGHT_SAMPLE_COUNT));
		float t1 = t_min + distance_through_atmosphere * sample_distribution(float(i + 1) / float(LIGHT_SAMPLE_COUNT));
		float t = 0.5 * (t0 + t1);
		float step_size = t1 - t0;
		vec3 sample_position = origin + direction * t;
		vec3 density = density_profile(sample_position);
		optical_depth += density * step_size;

		float ground_t_min;
		float ground_t_max;
		if (ray_sphere_intersection(sample_position, direction, get_planet_center(), get_ground_radius(), ground_t_min, ground_t_max) && ground_t_max > 0.0) {
			return vec3(0.0);
		}
	}

	return exp(-(
		optical_depth.x * BETA_RAYLEIGH +
		optical_depth.y * BETA_MIE +
		optical_depth.z * BETA_OZONE
	));
}

vec3 integrate_atmosphere(vec3 origin, vec3 direction) {
	float atmosphere_t_min;
	float atmosphere_t_max;

	if (!ray_sphere_intersection(origin, direction, get_planet_center(), get_atmosphere_radius(), atmosphere_t_min, atmosphere_t_max)) {
		return vec3(0.0);
	}

	atmosphere_t_min = max(0.0, atmosphere_t_min);
	float distance_through_atmosphere = atmosphere_t_max - atmosphere_t_min;
	vec3 optical_depth = vec3(0.0);
	vec3 luminance = vec3(0.0);
	float cosine_theta = dot(direction, get_sun_direction());
	float phase_r = phase_rayleigh(cosine_theta);
	float phase_m = phase_mie(cosine_theta, get_mie_g());
	float jitter = interleaved_gradient_noise(ivec2(gl_GlobalInvocationID.xy));

	for (int i = 0; i < VIEW_SAMPLE_COUNT; ++i) {
		float t0 = atmosphere_t_min + distance_through_atmosphere * sample_distribution(float(i) / float(VIEW_SAMPLE_COUNT));
		float t1 = atmosphere_t_min + distance_through_atmosphere * sample_distribution(float(i + 1) / float(VIEW_SAMPLE_COUNT));
		float t = mix(t0, t1, jitter);
		float step_size = t1 - t0;
		vec3 sample_position = origin + direction * t;
		vec3 density = density_profile(sample_position);
		optical_depth += density * step_size;

		vec3 transmittance_to_camera = exp(-extinction_from_density(optical_depth));
		vec3 transmittance_to_sun = march_transmittance(sample_position, get_sun_direction());
		vec3 scattering =
			density.x * BETA_RAYLEIGH * phase_r +
			density.y * BETA_MIE * phase_m;

		luminance += transmittance_to_camera * transmittance_to_sun * scattering * step_size;
	}

	vec3 sun_transmittance = march_transmittance(origin, get_sun_direction());
	float sun_disk = smoothstep(cos(get_sun_angular_radius() * 1.4), cos(get_sun_angular_radius()), cosine_theta);
	vec3 sun_radiance = sun_disk * sun_transmittance * vec3(20.0, 18.0, 16.0);

	vec3 color = luminance * get_sun_intensity() + sun_radiance;
	return color / (vec3(1.0) + color);
}

vec3 reconstruct_view_direction(ivec2 pixel, ivec2 extent) {
	vec2 uv = (vec2(pixel) + vec2(0.5)) / vec2(extent);
	// Match make_raster_ndc_from_pixel_coordinates: Y-flip for negative viewport height
	vec2 ndc = vec2(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
	// Unproject a point on the far plane (z=0 in reversed-Z)
	vec4 world = parameters.inverse_view_projection * vec4(ndc, 0.0, 1.0);
	return normalize(world.xyz / world.w - get_camera_position());
}

void main() {
	ivec2 pixel = ivec2(gl_GlobalInvocationID.xy);
	ivec2 extent = imageSize(main_texture);

	if (pixel.x >= extent.x || pixel.y >= extent.y) {
		return;
	}

	float depth = texelFetch(depth_texture, pixel, 0).r;
	if (depth > 1e-6) {
		return;
	}

	vec3 direction = reconstruct_view_direction(pixel, extent);
	vec3 sky = integrate_atmosphere(get_camera_position(), direction);

imageStore(main_texture, pixel, vec4(sky, 1.0));
}
"#;

const SKY_SHADER_MSL: &str = r#"
#include <metal_stdlib>
using namespace metal;

// besl-threadgroup-size: 8, 8, 1

struct SkyParameters {
	float4x4 inverse_view_projection;
	float4 camera_position;
	float4 sun_direction;
	float4 planet_center;
	float4 atmosphere;
	float4 misc;
};

struct SkySet0 {
	texture2d<float> depth_texture [[id(0)]];
	sampler depth_texture_sampler [[id(1)]];
	texture2d<float, access::write> main_texture [[id(2)]];
	device SkyParameters* parameters [[id(3)]];
};

constant float PI = 3.14159265359;
constant int VIEW_SAMPLE_COUNT = 64;
constant int LIGHT_SAMPLE_COUNT = 16;

constant float3 BETA_RAYLEIGH = float3(5.802e-6, 13.558e-6, 33.100e-6);
constant float3 BETA_MIE = float3(3.996e-6);
constant float3 BETA_OZONE = float3(0.650e-6, 1.881e-6, 0.085e-6);

float3 get_camera_position(const device SkyParameters& parameters) {
	return parameters.camera_position.xyz;
}

float3 get_sun_direction(const device SkyParameters& parameters) {
	return normalize(parameters.sun_direction.xyz);
}

float3 get_planet_center(const device SkyParameters& parameters) {
	return parameters.planet_center.xyz;
}

float get_ground_radius(const device SkyParameters& parameters) {
	return parameters.atmosphere.x;
}

float get_atmosphere_radius(const device SkyParameters& parameters) {
	return parameters.atmosphere.y;
}

float get_rayleigh_scale_height(const device SkyParameters& parameters) {
	return parameters.atmosphere.z;
}

float get_mie_scale_height(const device SkyParameters& parameters) {
	return parameters.atmosphere.w;
}

float get_mie_g(const device SkyParameters& parameters) {
	return parameters.sun_direction.w;
}

float get_sun_intensity(const device SkyParameters& parameters) {
	return parameters.camera_position.w;
}

float get_sun_angular_radius(const device SkyParameters& parameters) {
	return parameters.planet_center.w;
}

float get_ozone_strength(const device SkyParameters& parameters) {
	return parameters.misc.x;
}

bool ray_sphere_intersection(float3 origin, float3 direction, float3 center, float radius, thread float& t_min, thread float& t_max) {
	float3 oc = origin - center;
	float b = dot(oc, direction);
	float c = dot(oc, oc) - radius * radius;
	float discriminant = b * b - c;

	if (discriminant < 0.0) {
		return false;
	}

	float root = sqrt(discriminant);
	t_min = -b - root;
	t_max = -b + root;
	return true;
}

float3 density_profile(const device SkyParameters& parameters, float3 sample_position) {
	float altitude = length(sample_position - get_planet_center(parameters)) - get_ground_radius(parameters);

	if (altitude < 0.0) {
		return float3(0.0);
	}

	float rayleigh = exp(-altitude / get_rayleigh_scale_height(parameters));
	float mie = exp(-altitude / get_mie_scale_height(parameters));
	float ozone = max(0.0, 1.0 - abs(altitude - 25000.0) / 15000.0) * get_ozone_strength(parameters);

	return float3(rayleigh, mie, ozone);
}

float3 extinction_from_density(float3 density) {
	return density.x * BETA_RAYLEIGH + density.y * BETA_MIE + density.z * BETA_OZONE;
}

float interleaved_gradient_noise(int2 pixel) {
	return fract(52.9829189 * fract(0.06711056 * float(pixel.x) + 0.00583715 * float(pixel.y)));
}

float sample_distribution(float u) {
	u = clamp(u, 0.0, 1.0);
	return u * u;
}

float phase_rayleigh(float cosine_theta) {
	return (3.0 / (16.0 * PI)) * (1.0 + cosine_theta * cosine_theta);
}

float phase_mie(float cosine_theta, float g) {
	float g2 = g * g;
	float denominator = 1.0 + g2 - 2.0 * g * cosine_theta;
	return (3.0 / (8.0 * PI)) * ((1.0 - g2) * (1.0 + cosine_theta * cosine_theta)) /
		((2.0 + g2) * max(1e-4, pow(denominator, 1.5)));
}

float3 march_transmittance(const device SkyParameters& parameters, float3 origin, float3 direction) {
	float t_min;
	float t_max;

	if (!ray_sphere_intersection(origin, direction, get_planet_center(parameters), get_atmosphere_radius(parameters), t_min, t_max)) {
		return float3(1.0);
	}

	t_min = max(0.0, t_min);
	float distance_through_atmosphere = t_max - t_min;
	float3 optical_depth = float3(0.0);

	for (int i = 0; i < LIGHT_SAMPLE_COUNT; ++i) {
		float t0 = t_min + distance_through_atmosphere * sample_distribution(float(i) / float(LIGHT_SAMPLE_COUNT));
		float t1 = t_min + distance_through_atmosphere * sample_distribution(float(i + 1) / float(LIGHT_SAMPLE_COUNT));
		float t = 0.5 * (t0 + t1);
		float step_size = t1 - t0;
		float3 sample_position = origin + direction * t;
		float3 density = density_profile(parameters, sample_position);
		optical_depth += density * step_size;

		float ground_t_min;
		float ground_t_max;
		if (ray_sphere_intersection(
			sample_position,
			direction,
			get_planet_center(parameters),
			get_ground_radius(parameters),
			ground_t_min,
			ground_t_max
		) && ground_t_max > 0.0) {
			return float3(0.0);
		}
	}

	return exp(-(
		optical_depth.x * BETA_RAYLEIGH +
		optical_depth.y * BETA_MIE +
		optical_depth.z * BETA_OZONE
	));
}

float3 integrate_atmosphere(const device SkyParameters& parameters, int2 pixel, float3 origin, float3 direction) {
	float atmosphere_t_min;
	float atmosphere_t_max;

	if (!ray_sphere_intersection(
		origin,
		direction,
		get_planet_center(parameters),
		get_atmosphere_radius(parameters),
		atmosphere_t_min,
		atmosphere_t_max
	)) {
		return float3(0.0);
	}

	atmosphere_t_min = max(0.0, atmosphere_t_min);
	float distance_through_atmosphere = atmosphere_t_max - atmosphere_t_min;
	float3 optical_depth = float3(0.0);
	float3 luminance = float3(0.0);
	float cosine_theta = dot(direction, get_sun_direction(parameters));
	float phase_r = phase_rayleigh(cosine_theta);
	float phase_m = phase_mie(cosine_theta, get_mie_g(parameters));
	float jitter = interleaved_gradient_noise(pixel);

	for (int i = 0; i < VIEW_SAMPLE_COUNT; ++i) {
		float t0 = atmosphere_t_min + distance_through_atmosphere * sample_distribution(float(i) / float(VIEW_SAMPLE_COUNT));
		float t1 = atmosphere_t_min + distance_through_atmosphere * sample_distribution(float(i + 1) / float(VIEW_SAMPLE_COUNT));
		float t = mix(t0, t1, jitter);
		float step_size = t1 - t0;
		float3 sample_position = origin + direction * t;
		float3 density = density_profile(parameters, sample_position);
		optical_depth += density * step_size;

		float3 transmittance_to_camera = exp(-extinction_from_density(optical_depth));
		float3 transmittance_to_sun = march_transmittance(parameters, sample_position, get_sun_direction(parameters));
		float3 scattering =
			density.x * BETA_RAYLEIGH * phase_r +
			density.y * BETA_MIE * phase_m;

		luminance += transmittance_to_camera * transmittance_to_sun * scattering * step_size;
	}

	float3 sun_transmittance = march_transmittance(parameters, origin, get_sun_direction(parameters));
	float sun_disk = smoothstep(cos(get_sun_angular_radius(parameters) * 1.4), cos(get_sun_angular_radius(parameters)), cosine_theta);
	float3 sun_radiance = sun_disk * sun_transmittance * float3(20.0, 18.0, 16.0);

	float3 color = luminance * get_sun_intensity(parameters) + sun_radiance;
	return color / (float3(1.0) + color);
}

float3 reconstruct_view_direction(const device SkyParameters& parameters, int2 pixel, int2 extent) {
	float2 uv = (float2(pixel) + float2(0.5)) / float2(extent);
	float2 ndc = float2(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
	float4 world = parameters.inverse_view_projection * float4(ndc, 0.0, 1.0);
	return normalize(world.xyz / world.w - get_camera_position(parameters));
}

kernel void sky_render_pass(
	uint2 gid [[thread_position_in_grid]],
	constant SkySet0& set0 [[buffer(16)]]
) {
	int2 pixel = int2(gid);
	int2 extent = int2(set0.main_texture.get_width(), set0.main_texture.get_height());

	if (pixel.x >= extent.x || pixel.y >= extent.y) {
		return;
	}

	float2 depth_uv = (float2(pixel) + 0.5) / float2(extent);
	float depth = set0.depth_texture.sample(set0.depth_texture_sampler, depth_uv, level(0.0)).r;
	if (depth > 1e-6) {
		return;
	}

	const device SkyParameters& parameters = *set0.parameters;
	float3 direction = reconstruct_view_direction(parameters, pixel, extent);
	float3 sky = integrate_atmosphere(parameters, pixel, get_camera_position(parameters), direction);

	set0.main_texture.write(float4(sky, 1.0), gid);
}
"#;

#[cfg(test)]
mod tests {
	#[test]
	fn sky_shader_compiles() {
		resource_management::glsl::compile(super::SKY_SHADER, "Sky Render Pass Test").unwrap();
	}

	#[test]
	fn sky_msl_shader_compiles_for_metal() {
		use ghi::{device::DeviceCreate as _, device::Features};

		if !ghi::implementation::USES_METAL {
			return;
		}

		let mut instance =
			ghi::implementation::Instance::new(Features::new()).expect("Expected a Metal instance for the sky shader test");
		let mut queue = None;
		let mut device = instance
			.create_device(
				Features::new(),
				&mut [(ghi::QueueSelection::new(ghi::types::WorkloadTypes::COMPUTE), &mut queue)],
			)
			.expect("Expected a Metal device for the sky shader test");

		let shader_handle = device.create_shader(
			Some("Sky Render Pass Compute Shader"),
			ghi::shader::Sources::MTL {
				source: super::SKY_SHADER_MSL,
				entry_point: "sky_render_pass",
			},
			ghi::ShaderTypes::Compute,
			[
				super::SKY_DEPTH_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
				super::SKY_MAIN_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::WRITE),
				super::SKY_PARAMETERS_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
			],
		);

		assert!(shader_handle.is_ok(), "Expected the sky MSL source to compile for Metal");
	}
}
