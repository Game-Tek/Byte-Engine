pub(super) const GPU_IBL_GLSL: &str = r#"#version 460
#pragma shader_stage(compute)

layout(local_size_x = 64, local_size_y = 1, local_size_z = 1) in;

layout(set = 0, binding = 0) uniform sampler2D source_atlas;
layout(rgba16f, set = 0, binding = 1) uniform writeonly image2D output_atlas;

layout(push_constant) uniform PushConstants {
	uint source_width;
	uint source_height;
	uint source_level_count;
	uint output_face_size;
	uint output_y_offset;
	uint mode;
	float roughness;
	float source_row_angle_step;
	float source_solid_angle_scale;
	uint _padding0;
	uint _padding1;
	uint _padding2;
	uvec4 source_level_y_offsets[4];
} push_constants;

const float PI = 3.14159265358979323846;
const float TAU = 6.28318530717958647692;
const float MIN_POSITIVE = 1.17549435e-38;
const uint SAMPLE_COUNT = 1024u;

vec3 normalize_direction(vec3 vector) {
	float length_squared = dot(vector, vector);
	if (length_squared > 0.0 && !isnan(length_squared) && !isinf(length_squared)) {
		return vector * inversesqrt(length_squared);
	}
	return vec3(1.0, 0.0, 0.0);
}

void orthonormal_basis(vec3 normal, out vec3 tangent, out vec3 bitangent) {
	float sign_value = normal.z >= 0.0 ? 1.0 : -1.0;
	float a = -1.0 / (sign_value + normal.z);
	float b = normal.x * normal.y * a;
	tangent = vec3(1.0 + sign_value * normal.x * normal.x * a, sign_value * b, -sign_value * normal.x);
	bitangent = vec3(b, sign_value + normal.y * normal.y * a, -normal.y);
}

vec3 tangent_to_world(vec3 local_direction, vec3 tangent, vec3 bitangent, vec3 normal) {
	return tangent * local_direction.x + bitangent * local_direction.y + normal * local_direction.z;
}

// The API face order and signs must stay aligned with the native cubemap upload layout.
vec3 cubemap_texel_direction(uint face, uint x, uint y, uint face_size) {
	float u = 2.0 * (float(x) + 0.5) / float(face_size) - 1.0;
	float v = 2.0 * (float(y) + 0.5) / float(face_size) - 1.0;
	vec3 direction;
	switch (face) {
		case 0u: direction = vec3(1.0, -v, -u); break;
		case 1u: direction = vec3(-1.0, -v, u); break;
		case 2u: direction = vec3(u, 1.0, v); break;
		case 3u: direction = vec3(u, -1.0, -v); break;
		case 4u: direction = vec3(u, -v, 1.0); break;
		case 5u: direction = vec3(-u, -v, -1.0); break;
		default: direction = vec3(1.0, 0.0, 0.0); break;
	}
	return normalize_direction(direction);
}

uint source_level_dimension(uint base_dimension, uint level) {
	return max(base_dimension >> level, 1u);
}

// Four packed vectors keep the push-constant layout identical across GLSL, MSL, and HLSL.
uint source_level_y_offset(uint level) {
	return push_constants.source_level_y_offsets[level >> 2u][level & 3u];
}

vec3 lerp_radiance(vec3 a, vec3 b, float amount) {
	return a + (b - a) * amount;
}

vec2 project_lat_long(vec3 direction) {
	// Every caller constructs a unit direction; avoiding another normalization removes one reciprocal square root per sample.
	// Longitude is undefined at an exact pole, so use a stable seam coordinate there.
	float u = 0.5;
	if (direction.x != 0.0 || direction.z != 0.0) {
		u = atan(direction.z, direction.x) / TAU + 0.5;
	}
	float v = 0.5 - asin(clamp(direction.y, -1.0, 1.0)) / PI;
	return vec2(u, v);
}

vec3 sample_lat_long_level(uint level, vec2 uv) {
	uint level_width = source_level_dimension(push_constants.source_width, level);
	uint level_height = source_level_dimension(push_constants.source_height, level);
	uint level_y_offset = source_level_y_offset(level);
	float source_x = uv.x * float(level_width) - 0.5;
	int x0_unwrapped = int(floor(source_x));
	float x_fraction = source_x - float(x0_unwrapped);
	uint x0 = x0_unwrapped < 0 ? level_width - 1u : uint(x0_unwrapped);
	uint next_x = x0 + 1u;
	uint x1 = next_x == level_width ? 0u : next_x;

	float source_y = clamp(uv.y * float(level_height) - 0.5, 0.0, float(level_height - 1u));
	uint y0 = uint(floor(source_y));
	uint y1 = min(y0 + 1u, level_height - 1u);
	float y_fraction = source_y - float(y0);

	vec3 top_left = texelFetch(source_atlas, ivec2(x0, level_y_offset + y0), 0).rgb;
	vec3 top = x0 == x1
		? top_left
		: lerp_radiance(top_left, texelFetch(source_atlas, ivec2(x1, level_y_offset + y0), 0).rgb, x_fraction);
	if (y0 == y1) {
		return top;
	}
	vec3 bottom_left = texelFetch(source_atlas, ivec2(x0, level_y_offset + y1), 0).rgb;
	vec3 bottom = x0 == x1
		? bottom_left
		: lerp_radiance(bottom_left, texelFetch(source_atlas, ivec2(x1, level_y_offset + y1), 0).rgb, x_fraction);
	return lerp_radiance(top, bottom, y_fraction);
}

// This is the exact spherical area of the base-level texel row containing the projected direction.
float direction_texel_solid_angle(float v) {
	uint row = uint(clamp(
		floor(v * float(push_constants.source_height)),
		0.0,
		float(push_constants.source_height - 1u)
	));
	return push_constants.source_solid_angle_scale * sin((float(row) + 0.5) * push_constants.source_row_angle_step);
}

vec2 hammersley(uint index) {
	return vec2(
		float(index) / float(SAMPLE_COUNT),
		float(bitfieldReverse(index)) * 2.3283064e-10
	);
}

// GGX half-vector density is converted to light-direction density for source LOD selection.
float ggx_light_pdf(float cos_theta_squared, float alpha_squared) {
	float denominator_term = cos_theta_squared * (alpha_squared - 1.0) + 1.0;
	return max(alpha_squared / max(4.0 * PI * denominator_term * denominator_term, MIN_POSITIVE), MIN_POSITIVE);
}

// The PDF footprint selects a solid-angle-filtered source LOD and blends adjacent atlas levels.
vec3 sample_filtered_direction(vec3 direction, float pdf) {
	vec2 uv = project_lat_long(direction);
	float solid_angle_product = max(
		float(SAMPLE_COUNT) * max(pdf, MIN_POSITIVE) * direction_texel_solid_angle(uv.y),
		MIN_POSITIVE
	);
	uint last_level = max(push_constants.source_level_count, 1u) - 1u;
	float lod = clamp(-0.5 * log2(solid_angle_product), 0.0, float(last_level));
	uint lower_level = uint(floor(lod));
	uint upper_level = min(lower_level + 1u, last_level);
	float blend = lod - float(lower_level);
	vec3 lower = sample_lat_long_level(lower_level, uv);
	if (blend <= 0.0 || lower_level == upper_level) {
		return lower;
	}
	return lerp_radiance(lower, sample_lat_long_level(upper_level, uv), blend);
}

vec3 prefilter_specular(vec3 normal) {
	vec3 tangent;
	vec3 bitangent;
	orthonormal_basis(normal, tangent, bitangent);
	vec3 sum = vec3(0.0);
	float total_weight = 0.0;
	float alpha = push_constants.roughness * push_constants.roughness;
	float alpha_squared = alpha * alpha;

	// The split-sum prefilter uses N = V and weights accepted samples by N dot L.
	for (uint index = 0u; index < SAMPLE_COUNT; ++index) {
		vec2 sample_point = hammersley(index);
		float cos_theta_squared = max(
			(1.0 - sample_point.y) / (1.0 + (alpha_squared - 1.0) * sample_point.y),
			0.0
		);
		float normal_dot_light = 2.0 * cos_theta_squared - 1.0;
		if (normal_dot_light <= 0.0) {
			continue;
		}

		float cos_theta = sqrt(cos_theta_squared);
		float sin_theta = sqrt(max(1.0 - cos_theta_squared, 0.0));
		float angle = TAU * sample_point.x;
		float radial = 2.0 * cos_theta * sin_theta;
		vec3 local_light = vec3(radial * cos(angle), radial * sin(angle), normal_dot_light);
		vec3 light = tangent_to_world(local_light, tangent, bitangent, normal);
		float pdf = ggx_light_pdf(cos_theta_squared, alpha_squared);
		sum += sample_filtered_direction(light, pdf) * normal_dot_light;
		total_weight += normal_dot_light;
	}

	if (total_weight > 0.0) {
		return sum / total_weight;
	}
	return sample_lat_long_level(0u, project_lat_long(normal));
}

vec3 convolve_diffuse_irradiance(vec3 normal) {
	vec3 tangent;
	vec3 bitangent;
	orthonormal_basis(normal, tangent, bitangent);
	vec3 sum = vec3(0.0);

	// Cosine-weighted sampling reduces irradiance divided by pi to an arithmetic mean.
	for (uint index = 0u; index < SAMPLE_COUNT; ++index) {
		vec2 sample_point = hammersley(index);
		float radius = sqrt(sample_point.x);
		float angle = TAU * sample_point.y;
		vec3 local_direction = vec3(
			radius * cos(angle),
			radius * sin(angle),
			sqrt(max(1.0 - sample_point.x, 0.0))
		);
		vec3 direction = tangent_to_world(local_direction, tangent, bitangent, normal);
		float pdf = local_direction.z / PI;
		sum += sample_filtered_direction(direction, pdf);
	}

	return sum / float(SAMPLE_COUNT);
}

float finite_or_zero(float value) {
	return (isnan(value) || isinf(value)) ? 0.0 : value;
}

vec3 sanitize_radiance(vec3 radiance) {
	return vec3(
		finite_or_zero(radiance.r),
		finite_or_zero(radiance.g),
		finite_or_zero(radiance.b)
	);
}

void generate_environment_map() {
	uint output_index = gl_GlobalInvocationID.x;
	uint face_pixel_count = push_constants.output_face_size * push_constants.output_face_size;
	if (output_index >= face_pixel_count * 6u) {
		return;
	}
	uint face = output_index / face_pixel_count;
	uint face_pixel = output_index - face * face_pixel_count;
	uint y = face_pixel / push_constants.output_face_size;
	uint x = face_pixel - y * push_constants.output_face_size;

	vec3 normal = cubemap_texel_direction(face, x, y, push_constants.output_face_size);
	vec3 radiance = vec3(0.0);
	if (push_constants.mode == 0u) {
		radiance = sample_lat_long_level(0u, project_lat_long(normal));
	} else if (push_constants.mode == 1u) {
		radiance = prefilter_specular(normal);
	} else if (push_constants.mode == 2u) {
		radiance = convolve_diffuse_irradiance(normal);
	}

	ivec2 output_coordinate = ivec2(
		x,
		push_constants.output_y_offset + face * push_constants.output_face_size + y
	);
	imageStore(output_atlas, output_coordinate, vec4(sanitize_radiance(radiance), 1.0));
}

void main() {
	generate_environment_map();
}
"#;

pub(super) const GPU_IBL_MSL: &str = r#"#include <metal_stdlib>
using namespace metal;

// #pragma shader_stage(compute)
// besl-threadgroup-size:64,1,1

struct Resources {
	texture2d<float, access::sample> source_atlas [[id(0)]];
	sampler source_atlas_sampler [[id(1)]];
	texture2d<float, access::write> output_atlas [[id(2)]];
};

struct PushConstants {
	uint source_width;
	uint source_height;
	uint source_level_count;
	uint output_face_size;
	uint output_y_offset;
	uint mode;
	float roughness;
	float source_row_angle_step;
	float source_solid_angle_scale;
	uint _padding0;
	uint _padding1;
	uint _padding2;
	uint4 source_level_y_offsets[4];
};

constant float PI = 3.14159265358979323846f;
constant float TAU = 6.28318530717958647692f;
constant float MIN_POSITIVE = 1.17549435e-38f;
constant uint SAMPLE_COUNT = 1024u;

float3 normalize_direction(float3 vector) {
	float length_squared = dot(vector, vector);
	if (length_squared > 0.0f && isfinite(length_squared)) {
		return vector * rsqrt(length_squared);
	}
	return float3(1.0f, 0.0f, 0.0f);
}

struct OrthonormalBasis {
	float3 tangent;
	float3 bitangent;
};

OrthonormalBasis orthonormal_basis(float3 normal) {
	float sign_value = normal.z >= 0.0f ? 1.0f : -1.0f;
	float a = -1.0f / (sign_value + normal.z);
	float b = normal.x * normal.y * a;
	return OrthonormalBasis {
		float3(1.0f + sign_value * normal.x * normal.x * a, sign_value * b, -sign_value * normal.x),
		float3(b, sign_value + normal.y * normal.y * a, -normal.y)
	};
}

float3 tangent_to_world(float3 local_direction, OrthonormalBasis basis, float3 normal) {
	return basis.tangent * local_direction.x + basis.bitangent * local_direction.y + normal * local_direction.z;
}

// The API face order and signs must stay aligned with the native cubemap upload layout.
float3 cubemap_texel_direction(uint face, uint x, uint y, uint face_size) {
	float u = 2.0f * (float(x) + 0.5f) / float(face_size) - 1.0f;
	float v = 2.0f * (float(y) + 0.5f) / float(face_size) - 1.0f;
	float3 direction;
	switch (face) {
		case 0u: direction = float3(1.0f, -v, -u); break;
		case 1u: direction = float3(-1.0f, -v, u); break;
		case 2u: direction = float3(u, 1.0f, v); break;
		case 3u: direction = float3(u, -1.0f, -v); break;
		case 4u: direction = float3(u, -v, 1.0f); break;
		case 5u: direction = float3(-u, -v, -1.0f); break;
		default: direction = float3(1.0f, 0.0f, 0.0f); break;
	}
	return normalize_direction(direction);
}

uint source_level_dimension(uint base_dimension, uint level) {
	return max(base_dimension >> level, 1u);
}

// Four packed vectors keep the push-constant layout identical across GLSL, MSL, and HLSL.
uint source_level_y_offset(constant PushConstants& push_constants, uint level) {
	return push_constants.source_level_y_offsets[level >> 2u][level & 3u];
}

float3 lerp_radiance(float3 a, float3 b, float amount) {
	return a + (b - a) * amount;
}

float2 project_lat_long(float3 direction) {
	// Every caller constructs a unit direction; avoiding another normalization removes one reciprocal square root per sample.
	// Longitude is undefined at an exact pole, so use a stable seam coordinate there.
	float u = 0.5f;
	if (direction.x != 0.0f || direction.z != 0.0f) {
		u = atan2(direction.z, direction.x) / TAU + 0.5f;
	}
	float v = 0.5f - asin(clamp(direction.y, -1.0f, 1.0f)) / PI;
	return float2(u, v);
}

float3 sample_lat_long_level(
	texture2d<float, access::sample> source_atlas,
	constant PushConstants& push_constants,
	uint level,
	float2 uv
) {
	uint level_width = source_level_dimension(push_constants.source_width, level);
	uint level_height = source_level_dimension(push_constants.source_height, level);
	uint level_y_offset = source_level_y_offset(push_constants, level);
	float source_x = uv.x * float(level_width) - 0.5f;
	int x0_unwrapped = int(floor(source_x));
	float x_fraction = source_x - float(x0_unwrapped);
	uint x0 = x0_unwrapped < 0 ? level_width - 1u : uint(x0_unwrapped);
	uint next_x = x0 + 1u;
	uint x1 = next_x == level_width ? 0u : next_x;

	float source_y = clamp(uv.y * float(level_height) - 0.5f, 0.0f, float(level_height - 1u));
	uint y0 = uint(floor(source_y));
	uint y1 = min(y0 + 1u, level_height - 1u);
	float y_fraction = source_y - float(y0);

	float3 top_left = source_atlas.read(uint2(x0, level_y_offset + y0)).rgb;
	float3 top = x0 == x1
		? top_left
		: lerp_radiance(top_left, source_atlas.read(uint2(x1, level_y_offset + y0)).rgb, x_fraction);
	if (y0 == y1) {
		return top;
	}
	float3 bottom_left = source_atlas.read(uint2(x0, level_y_offset + y1)).rgb;
	float3 bottom = x0 == x1
		? bottom_left
		: lerp_radiance(bottom_left, source_atlas.read(uint2(x1, level_y_offset + y1)).rgb, x_fraction);
	return lerp_radiance(top, bottom, y_fraction);
}

// This is the exact spherical area of the base-level texel row containing the projected direction.
float direction_texel_solid_angle(float v, constant PushConstants& push_constants) {
	uint row = uint(clamp(
		floor(v * float(push_constants.source_height)),
		0.0f,
		float(push_constants.source_height - 1u)
	));
	return push_constants.source_solid_angle_scale * sin((float(row) + 0.5f) * push_constants.source_row_angle_step);
}

float2 hammersley(uint index) {
	return float2(
		float(index) / float(SAMPLE_COUNT),
		float(reverse_bits(index)) * 2.3283064e-10f
	);
}

// GGX half-vector density is converted to light-direction density for source LOD selection.
float ggx_light_pdf(float cos_theta_squared, float alpha_squared) {
	float denominator_term = cos_theta_squared * (alpha_squared - 1.0f) + 1.0f;
	return max(alpha_squared / max(4.0f * PI * denominator_term * denominator_term, MIN_POSITIVE), MIN_POSITIVE);
}

// The PDF footprint selects a solid-angle-filtered source LOD and blends adjacent atlas levels.
float3 sample_filtered_direction(
	texture2d<float, access::sample> source_atlas,
	constant PushConstants& push_constants,
	float3 direction,
	float pdf
) {
	float2 uv = project_lat_long(direction);
	float solid_angle_product = max(
		float(SAMPLE_COUNT) * max(pdf, MIN_POSITIVE) * direction_texel_solid_angle(uv.y, push_constants),
		MIN_POSITIVE
	);
	uint last_level = max(push_constants.source_level_count, 1u) - 1u;
	float lod = clamp(-0.5f * log2(solid_angle_product), 0.0f, float(last_level));
	uint lower_level = uint(floor(lod));
	uint upper_level = min(lower_level + 1u, last_level);
	float blend = lod - float(lower_level);
	float3 lower = sample_lat_long_level(source_atlas, push_constants, lower_level, uv);
	if (blend <= 0.0f || lower_level == upper_level) {
		return lower;
	}
	return lerp_radiance(lower, sample_lat_long_level(source_atlas, push_constants, upper_level, uv), blend);
}

float3 prefilter_specular(
	texture2d<float, access::sample> source_atlas,
	constant PushConstants& push_constants,
	float3 normal
) {
	OrthonormalBasis basis = orthonormal_basis(normal);
	float3 sum = float3(0.0f);
	float total_weight = 0.0f;
	float alpha = push_constants.roughness * push_constants.roughness;
	float alpha_squared = alpha * alpha;

	// The split-sum prefilter uses N = V and weights accepted samples by N dot L.
	for (uint index = 0u; index < SAMPLE_COUNT; ++index) {
		float2 sample_point = hammersley(index);
		float cos_theta_squared = max(
			(1.0f - sample_point.y) / (1.0f + (alpha_squared - 1.0f) * sample_point.y),
			0.0f
		);
		float normal_dot_light = 2.0f * cos_theta_squared - 1.0f;
		if (normal_dot_light <= 0.0f) {
			continue;
		}

		float cos_theta = sqrt(cos_theta_squared);
		float sin_theta = sqrt(max(1.0f - cos_theta_squared, 0.0f));
		float angle = TAU * sample_point.x;
		float radial = 2.0f * cos_theta * sin_theta;
		float3 local_light = float3(radial * cos(angle), radial * sin(angle), normal_dot_light);
		float3 light = tangent_to_world(local_light, basis, normal);
		float pdf = ggx_light_pdf(cos_theta_squared, alpha_squared);
		sum += sample_filtered_direction(source_atlas, push_constants, light, pdf) * normal_dot_light;
		total_weight += normal_dot_light;
	}

	if (total_weight > 0.0f) {
		return sum / total_weight;
	}
	return sample_lat_long_level(source_atlas, push_constants, 0u, project_lat_long(normal));
}

float3 convolve_diffuse_irradiance(
	texture2d<float, access::sample> source_atlas,
	constant PushConstants& push_constants,
	float3 normal
) {
	OrthonormalBasis basis = orthonormal_basis(normal);
	float3 sum = float3(0.0f);

	// Cosine-weighted sampling reduces irradiance divided by pi to an arithmetic mean.
	for (uint index = 0u; index < SAMPLE_COUNT; ++index) {
		float2 sample_point = hammersley(index);
		float radius = sqrt(sample_point.x);
		float angle = TAU * sample_point.y;
		float3 local_direction = float3(
			radius * cos(angle),
			radius * sin(angle),
			sqrt(max(1.0f - sample_point.x, 0.0f))
		);
		float3 direction = tangent_to_world(local_direction, basis, normal);
		float pdf = local_direction.z / PI;
		sum += sample_filtered_direction(source_atlas, push_constants, direction, pdf);
	}

	return sum / float(SAMPLE_COUNT);
}

float finite_or_zero(float value) {
	return isfinite(value) ? value : 0.0f;
}

float3 sanitize_radiance(float3 radiance) {
	return float3(
		finite_or_zero(radiance.r),
		finite_or_zero(radiance.g),
		finite_or_zero(radiance.b)
	);
}

kernel void generate_environment_map(
	uint3 invocation_id [[thread_position_in_grid]],
	constant PushConstants& push_constants [[buffer(15)]],
	constant Resources& resources [[buffer(16)]]
) {
	uint output_index = invocation_id.x;
	uint face_pixel_count = push_constants.output_face_size * push_constants.output_face_size;
	if (output_index >= face_pixel_count * 6u) {
		return;
	}
	uint face = output_index / face_pixel_count;
	uint face_pixel = output_index - face * face_pixel_count;
	uint y = face_pixel / push_constants.output_face_size;
	uint x = face_pixel - y * push_constants.output_face_size;

	float3 normal = cubemap_texel_direction(face, x, y, push_constants.output_face_size);
	float3 radiance = float3(0.0f);
	if (push_constants.mode == 0u) {
		radiance = sample_lat_long_level(resources.source_atlas, push_constants, 0u, project_lat_long(normal));
	} else if (push_constants.mode == 1u) {
		radiance = prefilter_specular(resources.source_atlas, push_constants, normal);
	} else if (push_constants.mode == 2u) {
		radiance = convolve_diffuse_irradiance(resources.source_atlas, push_constants, normal);
	}

	uint2 output_coordinate = uint2(
		x,
		push_constants.output_y_offset + face * push_constants.output_face_size + y
	);
	resources.output_atlas.write(float4(sanitize_radiance(radiance), 1.0f), output_coordinate);
}
"#;

pub(super) const GPU_IBL_HLSL: &str = r#"Texture2D<float4> source_atlas : register(t0, space0);
SamplerState source_atlas_sampler : register(s0, space0);
RWTexture2D<float4> output_atlas : register(u1, space0);

struct PushConstants {
	uint source_width;
	uint source_height;
	uint source_level_count;
	uint output_face_size;
	uint output_y_offset;
	uint mode;
	float roughness;
	float source_row_angle_step;
	float source_solid_angle_scale;
	uint _padding0;
	uint _padding1;
	uint _padding2;
	uint4 source_level_y_offsets[4];
};

ConstantBuffer<PushConstants> push_constants : register(b0, space0);

static const float PI = 3.14159265358979323846;
static const float TAU = 6.28318530717958647692;
static const float MIN_POSITIVE = 1.17549435e-38;
static const uint SAMPLE_COUNT = 1024u;

float3 normalize_direction(float3 value) {
	float length_squared = dot(value, value);
	if (length_squared > 0.0 && isfinite(length_squared)) {
		return value * rsqrt(length_squared);
	}
	return float3(1.0, 0.0, 0.0);
}

void orthonormal_basis(float3 normal, out float3 tangent, out float3 bitangent) {
	float sign_value = normal.z >= 0.0 ? 1.0 : -1.0;
	float a = -1.0 / (sign_value + normal.z);
	float b = normal.x * normal.y * a;
	tangent = float3(1.0 + sign_value * normal.x * normal.x * a, sign_value * b, -sign_value * normal.x);
	bitangent = float3(b, sign_value + normal.y * normal.y * a, -normal.y);
}

float3 tangent_to_world(float3 local_direction, float3 tangent, float3 bitangent, float3 normal) {
	return tangent * local_direction.x + bitangent * local_direction.y + normal * local_direction.z;
}

// The API face order and signs must stay aligned with the native cubemap upload layout.
float3 cubemap_texel_direction(uint face, uint x, uint y, uint face_size) {
	float u = 2.0 * (float(x) + 0.5) / float(face_size) - 1.0;
	float v = 2.0 * (float(y) + 0.5) / float(face_size) - 1.0;
	float3 direction;
	switch (face) {
		case 0u: direction = float3(1.0, -v, -u); break;
		case 1u: direction = float3(-1.0, -v, u); break;
		case 2u: direction = float3(u, 1.0, v); break;
		case 3u: direction = float3(u, -1.0, -v); break;
		case 4u: direction = float3(u, -v, 1.0); break;
		case 5u: direction = float3(-u, -v, -1.0); break;
		default: direction = float3(1.0, 0.0, 0.0); break;
	}
	return normalize_direction(direction);
}

uint source_level_dimension(uint base_dimension, uint level) {
	return max(base_dimension >> level, 1u);
}

// Four packed vectors keep the push-constant layout identical across GLSL, MSL, and HLSL.
uint source_level_y_offset(uint level) {
	return push_constants.source_level_y_offsets[level >> 2u][level & 3u];
}

float3 lerp_radiance(float3 a, float3 b, float amount) {
	return a + (b - a) * amount;
}

float2 project_lat_long(float3 direction) {
	// Every caller constructs a unit direction; avoiding another normalization removes one reciprocal square root per sample.
	// Longitude is undefined at an exact pole, so use a stable seam coordinate there.
	float u = 0.5;
	if (direction.x != 0.0 || direction.z != 0.0) {
		u = atan2(direction.z, direction.x) / TAU + 0.5;
	}
	float v = 0.5 - asin(clamp(direction.y, -1.0, 1.0)) / PI;
	return float2(u, v);
}

float3 sample_lat_long_level(uint level, float2 uv) {
	uint level_width = source_level_dimension(push_constants.source_width, level);
	uint level_height = source_level_dimension(push_constants.source_height, level);
	uint level_y_offset = source_level_y_offset(level);
	float source_x = uv.x * float(level_width) - 0.5;
	int x0_unwrapped = int(floor(source_x));
	float x_fraction = source_x - float(x0_unwrapped);
	uint x0 = x0_unwrapped < 0 ? level_width - 1u : uint(x0_unwrapped);
	uint next_x = x0 + 1u;
	uint x1 = next_x == level_width ? 0u : next_x;

	float source_y = clamp(uv.y * float(level_height) - 0.5, 0.0, float(level_height - 1u));
	uint y0 = uint(floor(source_y));
	uint y1 = min(y0 + 1u, level_height - 1u);
	float y_fraction = source_y - float(y0);

	float3 top_left = source_atlas[uint2(x0, level_y_offset + y0)].rgb;
	float3 top = x0 == x1
		? top_left
		: lerp_radiance(top_left, source_atlas[uint2(x1, level_y_offset + y0)].rgb, x_fraction);
	if (y0 == y1) {
		return top;
	}
	float3 bottom_left = source_atlas[uint2(x0, level_y_offset + y1)].rgb;
	float3 bottom = x0 == x1
		? bottom_left
		: lerp_radiance(bottom_left, source_atlas[uint2(x1, level_y_offset + y1)].rgb, x_fraction);
	return lerp_radiance(top, bottom, y_fraction);
}

// This is the exact spherical area of the base-level texel row containing the projected direction.
float direction_texel_solid_angle(float v) {
	uint row = uint(clamp(
		floor(v * float(push_constants.source_height)),
		0.0,
		float(push_constants.source_height - 1u)
	));
	return push_constants.source_solid_angle_scale * sin((float(row) + 0.5) * push_constants.source_row_angle_step);
}

float2 hammersley(uint index) {
	return float2(
		float(index) / float(SAMPLE_COUNT),
		float(reversebits(index)) * 2.3283064e-10
	);
}

// GGX half-vector density is converted to light-direction density for source LOD selection.
float ggx_light_pdf(float cos_theta_squared, float alpha_squared) {
	float denominator_term = cos_theta_squared * (alpha_squared - 1.0) + 1.0;
	return max(alpha_squared / max(4.0 * PI * denominator_term * denominator_term, MIN_POSITIVE), MIN_POSITIVE);
}

// The PDF footprint selects a solid-angle-filtered source LOD and blends adjacent atlas levels.
float3 sample_filtered_direction(float3 direction, float pdf) {
	float2 uv = project_lat_long(direction);
	float solid_angle_product = max(
		float(SAMPLE_COUNT) * max(pdf, MIN_POSITIVE) * direction_texel_solid_angle(uv.y),
		MIN_POSITIVE
	);
	uint last_level = max(push_constants.source_level_count, 1u) - 1u;
	float lod = clamp(-0.5 * log2(solid_angle_product), 0.0, float(last_level));
	uint lower_level = uint(floor(lod));
	uint upper_level = min(lower_level + 1u, last_level);
	float blend = lod - float(lower_level);
	float3 lower = sample_lat_long_level(lower_level, uv);
	if (blend <= 0.0 || lower_level == upper_level) {
		return lower;
	}
	return lerp_radiance(lower, sample_lat_long_level(upper_level, uv), blend);
}

float3 prefilter_specular(float3 normal) {
	float3 tangent;
	float3 bitangent;
	orthonormal_basis(normal, tangent, bitangent);
	float3 sum = float3(0.0, 0.0, 0.0);
	float total_weight = 0.0;
	float alpha = push_constants.roughness * push_constants.roughness;
	float alpha_squared = alpha * alpha;

	// The split-sum prefilter uses N = V and weights accepted samples by N dot L.
	for (uint index = 0u; index < SAMPLE_COUNT; ++index) {
		float2 sample_point = hammersley(index);
		float cos_theta_squared = max(
			(1.0 - sample_point.y) / (1.0 + (alpha_squared - 1.0) * sample_point.y),
			0.0
		);
		float normal_dot_light = 2.0 * cos_theta_squared - 1.0;
		if (normal_dot_light <= 0.0) {
			continue;
		}

		float cos_theta = sqrt(cos_theta_squared);
		float sin_theta = sqrt(max(1.0 - cos_theta_squared, 0.0));
		float angle = TAU * sample_point.x;
		float radial = 2.0 * cos_theta * sin_theta;
		float3 local_light = float3(radial * cos(angle), radial * sin(angle), normal_dot_light);
		float3 light = tangent_to_world(local_light, tangent, bitangent, normal);
		float pdf = ggx_light_pdf(cos_theta_squared, alpha_squared);
		sum += sample_filtered_direction(light, pdf) * normal_dot_light;
		total_weight += normal_dot_light;
	}

	if (total_weight > 0.0) {
		return sum / total_weight;
	}
	return sample_lat_long_level(0u, project_lat_long(normal));
}

float3 convolve_diffuse_irradiance(float3 normal) {
	float3 tangent;
	float3 bitangent;
	orthonormal_basis(normal, tangent, bitangent);
	float3 sum = float3(0.0, 0.0, 0.0);

	// Cosine-weighted sampling reduces irradiance divided by pi to an arithmetic mean.
	for (uint index = 0u; index < SAMPLE_COUNT; ++index) {
		float2 sample_point = hammersley(index);
		float radius = sqrt(sample_point.x);
		float angle = TAU * sample_point.y;
		float3 local_direction = float3(
			radius * cos(angle),
			radius * sin(angle),
			sqrt(max(1.0 - sample_point.x, 0.0))
		);
		float3 direction = tangent_to_world(local_direction, tangent, bitangent, normal);
		float pdf = local_direction.z / PI;
		sum += sample_filtered_direction(direction, pdf);
	}

	return sum / float(SAMPLE_COUNT);
}

float finite_or_zero(float value) {
	return isfinite(value) ? value : 0.0;
}

float3 sanitize_radiance(float3 radiance) {
	return float3(
		finite_or_zero(radiance.r),
		finite_or_zero(radiance.g),
		finite_or_zero(radiance.b)
	);
}

[numthreads(64, 1, 1)]
void generate_environment_map(uint3 invocation_id : SV_DispatchThreadID) {
	uint output_index = invocation_id.x;
	uint face_pixel_count = push_constants.output_face_size * push_constants.output_face_size;
	if (output_index >= face_pixel_count * 6u) {
		return;
	}
	uint face = output_index / face_pixel_count;
	uint face_pixel = output_index - face * face_pixel_count;
	uint y = face_pixel / push_constants.output_face_size;
	uint x = face_pixel - y * push_constants.output_face_size;

	float3 normal = cubemap_texel_direction(face, x, y, push_constants.output_face_size);
	float3 radiance = float3(0.0, 0.0, 0.0);
	if (push_constants.mode == 0u) {
		radiance = sample_lat_long_level(0u, project_lat_long(normal));
	} else if (push_constants.mode == 1u) {
		radiance = prefilter_specular(normal);
	} else if (push_constants.mode == 2u) {
		radiance = convolve_diffuse_irradiance(normal);
	}

	uint2 output_coordinate = uint2(
		x,
		push_constants.output_y_offset + face * push_constants.output_face_size + y
	);
	output_atlas[output_coordinate] = float4(sanitize_radiance(radiance), 1.0);
}
"#;
