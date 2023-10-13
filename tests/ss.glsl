#pragma shader_stage(compute)
#extension GL_EXT_shader_16bit_storage : enable
#extension GL_EXT_shader_explicit_arithmetic_types_int8 : enable
#extension GL_EXT_shader_explicit_arithmetic_types_int16 : enable
#extension GL_EXT_shader_explicit_arithmetic_types_int64 : enable
#extension GL_EXT_nonuniform_qualifier : enable
#extension GL_EXT_scalar_block_layout : enable
#extension GL_EXT_buffer_reference : enable
#extension GL_EXT_buffer_reference2 : enable
#extension GL_EXT_shader_image_load_formatted : enable
#extension GL_KHR_shader_subgroup_basic : enable
#extension GL_KHR_shader_subgroup_arithmetic  : enable
#extension GL_KHR_shader_subgroup_ballot : enable
#extension GL_KHR_shader_subgroup_shuffle : enable
layout(row_major) uniform; layout(row_major) buffer;
layout(set=0,binding=0,scalar) buffer MaterialCount {
        uint material_count[];
};
layout(set=0,binding=1,scalar) buffer MaterialOffset {
        uint material_offset[];
};
layout(set=0,binding=4,scalar) buffer PixelMapping {
        u16vec2 pixel_mapping[];
};
layout(set=0, binding=6, r8ui) uniform readonly uimage2D vertex_id;
layout(set=0, binding=7, r8ui) uniform readonly uimage2D instance_id;
layout(set=1, binding=0, rgba16) uniform image2D out_albedo;

struct Mesh {
        mat4 model;
        uint material_id;
};

layout(set=1, binding=1, scalar) buffer readonly MeshBuffer {
        Mesh meshes[];
};

layout(set=1, binding=2, scalar) buffer readonly Positions {
        vec3 positions[];
};

layout(set=1, binding=3, scalar) buffer readonly Normals {
        vec3 normals[];
};

const float PI = 3.14159265359;

float DistributionGGX(vec3 N, vec3 H, float roughness) {
    float a      = roughness*roughness;
    float a2     = a*a;
    float NdotH  = max(dot(N, H), 0.0);
    float NdotH2 = NdotH*NdotH;

    float num   = a2;
    float denom = (NdotH2 * (a2 - 1.0) + 1.0);
    denom = PI * denom * denom;

    return num / denom;
}

float GeometrySchlickGGX(float NdotV, float roughness) {
    float r = (roughness + 1.0);
    float k = (r*r) / 8.0;

    float num   = NdotV;
    float denom = NdotV * (1.0 - k) + k;

    return num / denom;
}

float GeometrySmith(vec3 N, vec3 V, vec3 L, float roughness) {
    float NdotV = max(dot(N, V), 0.0);
    float NdotL = max(dot(N, L), 0.0);
    float ggx2  = GeometrySchlickGGX(NdotV, roughness);
    float ggx1  = GeometrySchlickGGX(NdotL, roughness);

    return ggx1 * ggx2;
}

vec4 get_debug_color(uint i) {
        vec4 colors[16] = vec4[16](
                vec4(0.16863, 0.40392, 0.77647, 1),
                vec4(0.32941, 0.76863, 0.21961, 1),
                vec4(0.81961, 0.16078, 0.67451, 1),
                vec4(0.96863, 0.98824, 0.45490, 1),
                vec4(0.75294, 0.09020, 0.75686, 1),
                vec4(0.30588, 0.95686, 0.54510, 1),
                vec4(0.66667, 0.06667, 0.75686, 1),
                vec4(0.78824, 0.91765, 0.27451, 1),
                vec4(0.40980, 0.12745, 0.48627, 1),
                vec4(0.89804, 0.28235, 0.20784, 1),
                vec4(0.93725, 0.67843, 0.33725, 1),
                vec4(0.95294, 0.96863, 0.00392, 1),
                vec4(1.00000, 0.27843, 0.67843, 1),
                vec4(0.29020, 0.90980, 0.56863, 1),
                vec4(0.30980, 0.70980, 0.27059, 1),
                vec4(0.69804, 0.16078, 0.39216, 1)
        );

        return colors[i % 16];
}
layout(constant_id=0) const float be_variable_color_r = 1.0;layout(constant_id=1) const float be_variable_color_g = 0.0;layout(constant_id=2) const float be_variable_color_b = 0.0;layout(constant_id=3) const float be_variable_color_a = 1.0;const vec4 be_variable_color = vec4(be_variable_color_r, be_variable_color_g, be_variable_color_b, be_variable_color_a);

layout(scalar, buffer_reference) buffer CameraData {
        mat4 view;
        mat4 projection_matrix;
        mat4 view_projection;
};

layout(push_constant, scalar) uniform PushConstant {
        CameraData camera;
        layout(offset=16) uint material_id;
} pc;
vec3 fresnelSchlick(float cosTheta, vec3 F0) {
                        return F0 + (1.0 - F0) * pow(clamp(1.0 - cosTheta, 0.0, 1.0), 5.0);
                }layout(local_size_x=32) in;

void main() {
	if (gl_GlobalInvocationID.x >= material_count[pc.material_id]) { return; }

	uint offset = material_offset[pc.material_id];
	u16vec2 be_pixel_xy = pixel_mapping[offset + gl_GlobalInvocationID.x];
	ivec2 be_pixel_coordinate = ivec2(be_pixel_xy.x, be_pixel_xy.y);
	uint be_vertex_id = imageLoad(vertex_id, be_pixel_coordinate).r;
	uint be_instance_id = imageLoad(instance_id, be_pixel_coordinate).r;

	Mesh mesh = meshes[be_instance_id];

	vec3 BE_VERTEX_POSITION = vec3(mesh.model * vec4(positions[be_vertex_id], 0.0f));
	vec3 BE_VERTEX_NORMAL = vec3(mesh.model * vec4(normals[be_vertex_id], 0.0f));

	vec3 N = normalize(BE_VERTEX_NORMAL);
	vec3 V = normalize(pc.camera.view[3].xyz - BE_VERTEX_POSITION);

	vec3 Lo = vec3(0.0);

	for (uint i = 0; i < 1; ++i) {
			vec3 light_pos = vec3(0, 2, 0);
			vec3 L = normalize(light_pos - BE_VERTEX_POSITION);
			vec3 H = normalize(V + L);

			float distance = length(light_pos - BE_VERTEX_POSITION);
			float attenuation = 1.0 / (distance * distance);
			vec3 light_color = vec3(1.0);
			vec3 radiance = light_color * attenuation;

			vec3 BE_ALBEDO = vec3(be_variable_color);
			vec3 BE_METALLIC = vec3(vec3(0.0));
			float BE_ROUGHNESS = float(0.5);

			vec3 F0 = vec3(0.04);
			F0 = mix(F0, BE_ALBEDO, BE_METALLIC);
			vec3 F = fresnelSchlick(max(dot(H, V), 0.0), F0);

			float NDF = DistributionGGX(N, H, BE_ROUGHNESS);
			float G = GeometrySmith(N, V, L, BE_ROUGHNESS);
			vec3 numerator = NDF * G * F;
			float denominator = 4.0 * max(dot(N, V), 0.0) * max(dot(N, L), 0.0) + 0.000001;
			vec3 specular = numerator / denominator;

			vec3 kS = F;
			vec3 kD = vec3(1.0) - kS;

			kD *= 1.0 - BE_METALLIC;

			float NdotL = max(dot(N, L), 0.0);
			Lo += (kD * BE_ALBEDO / PI + specular) * radiance * NdotL;
	}
}