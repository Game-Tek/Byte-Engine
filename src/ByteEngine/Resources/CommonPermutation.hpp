#pragma once

#include "PermutationManager.hpp"
#include "ByteEngine/Render/ShaderGenerator.h"
#include "ByteEngine/Render/Types.hpp"

struct CommonPermutation : PermutationManager {
	CommonPermutation(const GTSL::StringView name) : PermutationManager(name, u8"CommonPermutation") {}

	void Initialize(GPipeline* pipeline, ShaderGenerationData& shader_generation_data) override {
		auto descriptorSetBlockHandle = pipeline->DeclareScope(GPipeline::GLOBAL_SCOPE , u8"descriptorSetBlock");
		auto firstDescriptorSetBlockHandle = pipeline->DeclareScope(descriptorSetBlockHandle, u8"descriptorSet");
		pipeline->DeclareVariable(firstDescriptorSetBlockHandle, { u8"texture2D[]", u8"textures" });
		pipeline->DeclareVariable(firstDescriptorSetBlockHandle, { u8"image2D[]", u8"images" });
		pipeline->DeclareVariable(firstDescriptorSetBlockHandle, { u8"sampler", u8"s" });

		pipeline->DeclareStruct(GPipeline::GLOBAL_SCOPE, u8"InstanceData", INSTANCE_DATA);

		pipeline->SetMakeStruct(pipeline->DeclareStruct(GPipeline::GLOBAL_SCOPE, u8"TextureReference", { { u8"uint32", u8"Instance" } }));
		pipeline->SetMakeStruct(pipeline->DeclareStruct(GPipeline::GLOBAL_SCOPE, u8"ImageReference", { { u8"uint32", u8"Instance" } }));
		pipeline->SetMakeStruct(pipeline->DeclareStruct(GPipeline::GLOBAL_SCOPE, u8"IndirectDispatchCommand", INDIRECT_DISPATCH_COMMAND_DATA));

		pipeline->DeclareStruct(GPipeline::GLOBAL_SCOPE, u8"uint32", { { u8"uint32", u8"a"} });
		pipeline->DeclareStruct(GPipeline::GLOBAL_SCOPE, u8"vec2s", { { u8"u16vec2", u8"wh"} });
		pipeline->DeclareStruct(GPipeline::GLOBAL_SCOPE, u8"vec2f", { { u8"vec2f", u8"xy"} });
		pipeline->DeclareStruct(GPipeline::GLOBAL_SCOPE, u8"vec3f", { { u8"vec3f", u8"xyz"} });
		pipeline->DeclareStruct(GPipeline::GLOBAL_SCOPE, u8"vec4f", { { u8"vec4f", u8"xyzw"} });

		pipeline->DeclareFunction(GPipeline::GLOBAL_SCOPE, u8"vec3f", u8"Barycenter", { { u8"vec2f", u8"coords" } }, u8"return vec3(1.0f - coords.x - coords.y, coords.x, coords.y);");
		pipeline->DeclareFunction(GPipeline::GLOBAL_SCOPE, u8"vec3f", u8"Barycenter", { { u8"vec3f", u8"p" }, { u8"vec3f", u8"a" }, { u8"vec3f", u8"b" }, { u8"vec3f", u8"c" } }, u8"vec3f v0 = b - a, v1 = c - a, v2 = p - a; float32 d00 = dot(v0, v0); float32 d01 = dot(v0, v1); float32 d11 = dot(v1, v1); float32 d20 = dot(v2, v0); float32 d21 = dot(v2, v1); float32 invDenom = 1.0f / (d00 * d11 - d01 * d01); v = (d11 * d20 - d01 * d21) * invDenom; w = (d00 * d21 - d01 * d20) * invDenom; return vec3f(1.0f - v - w, v, w);");
		pipeline->DeclareFunction(GPipeline::GLOBAL_SCOPE, u8"vec4f", u8"Sample", { { u8"TextureReference", u8"tex" }, { u8"vec2f", u8"texCoord" } }, u8"return texture(sampler2D(textures[nonuniformEXT(tex.Instance)], s), texCoord);");
		pipeline->DeclareFunction(GPipeline::GLOBAL_SCOPE, u8"vec4f", u8"SampleNormal", { { u8"TextureReference", u8"tex" }, { u8"vec2f", u8"texCoord" } }, u8"return normalize(texture(sampler2D(textures[nonuniformEXT(tex.Instance)], s), texCoord) * 2.0f - 1.0f);");
		pipeline->DeclareFunction(GPipeline::GLOBAL_SCOPE, u8"vec4f", u8"Sample", { { u8"TextureReference", u8"tex" }, { u8"vec2f", u8"texCoord" }, { u8"vec2f", u8"ddx" }, { u8"vec2f", u8"ddy" } }, u8"return textureGrad(sampler2D(textures[nonuniformEXT(tex.Instance)], s), texCoord, ddx, ddy);");
		pipeline->DeclareFunction(GPipeline::GLOBAL_SCOPE, u8"vec4f", u8"Sample", { { u8"TextureReference", u8"tex" }, { u8"uvec2", u8"pos" } }, u8"return texelFetch(sampler2D(textures[nonuniformEXT(tex.Instance)], s), ivec2(pos) % textureSize(sampler2D(textures[nonuniformEXT(tex.Instance)], s), 0), 0);");
		pipeline->DeclareFunction(GPipeline::GLOBAL_SCOPE, u8"vec4u", u8"SampleUint", { { u8"TextureReference", u8"tex" }, { u8"uvec2", u8"pos" } }, u8"return texelFetch(usampler2D(textures[nonuniformEXT(tex.Instance)], s), ivec2(pos), 0);");
		pipeline->DeclareFunction(GPipeline::GLOBAL_SCOPE, u8"vec4f", u8"Sample", { { u8"ImageReference", u8"img" }, { u8"uvec2", u8"pos" } }, u8"return imageLoad(images[nonuniformEXT(img.Instance)], ivec2(pos));");
		pipeline->DeclareFunction(GPipeline::GLOBAL_SCOPE, u8"void", u8"Write", { { u8"ImageReference", u8"img" }, { u8"uvec2", u8"pos" }, { u8"vec4f", u8"value" } }, u8"imageStore(images[nonuniformEXT(img.Instance)], ivec2(pos), value);");
		pipeline->DeclareFunction(GPipeline::GLOBAL_SCOPE, u8"void", u8"Write", { { u8"ImageReference", u8"img" }, { u8"uvec2", u8"pos" }, { u8"float32", u8"value" } }, u8"imageStore(images[nonuniformEXT(img.Instance)], ivec2(pos), vec4f(value));");
		pipeline->DeclareFunction(GPipeline::GLOBAL_SCOPE, u8"float32", u8"X", { { u8"vec4f", u8"vec" } }, u8"return vec.x;");
		pipeline->DeclareFunction(GPipeline::GLOBAL_SCOPE, u8"float32", u8"Y", { { u8"vec4f", u8"vec" } }, u8"return vec.y;");
		pipeline->DeclareFunction(GPipeline::GLOBAL_SCOPE, u8"float32", u8"Z", { { u8"vec4f", u8"vec" } }, u8"return vec.z;");
		pipeline->DeclareFunction(GPipeline::GLOBAL_SCOPE, u8"vec3f", u8"FresnelSchlick", { { u8"float32", u8"cosTheta" }, { u8"vec3f", u8"F0" } }, u8"return F0 + (1.0 - F0) * pow(max(0.0, 1.0 - cosTheta), 5.0);");
		pipeline->DeclareFunction(GPipeline::GLOBAL_SCOPE, u8"vec3f", u8"Normalize", { { u8"vec3f", u8"a" } }, u8"return normalize(a);");
		pipeline->DeclareFunction(GPipeline::GLOBAL_SCOPE, u8"float32", u8"Sigmoid", { { u8"float32", u8"x" } }, u8"return 1.0 / (1.0 + pow(x / (1.0 - x), -3.0));");
		pipeline->DeclareFunction(GPipeline::GLOBAL_SCOPE, u8"vec3f", u8"WorldPositionFromDepth2", { { u8"vec2f", u8"texture_coordinate" }, { u8"float32", u8"depth" }, { u8"matrix4f", u8"inverse_proj_view_matrix" } }, u8"vec4 clipSpacePosition = vec4((texture_coordinate * 2.0) - 1.0, depth, 1.0); vec4f position = inverse_proj_view_matrix * clipSpacePosition; return position.xyz / position.w;");
		pipeline->DeclareFunction(GPipeline::GLOBAL_SCOPE, u8"vec3f", u8"WorldPositionFromDepth", { { u8"vec2f", u8"texture_coordinate" }, { u8"float32", u8"depth" }, { u8"matrix4f", u8"inverse_proj_matrix" }, { u8"matrix4f", u8"inverse_view_matrix" } }, u8"vec2 ndc = (texture_coordinate * 2.0) - 1.0; vec4 clipSpacePosition = vec4(ndc, depth, 1.0); vec4f viewSpacePosition = inverse_proj_matrix * clipSpacePosition; viewSpacePosition /= viewSpacePosition.w; return (inverse_view_matrix * viewSpacePosition).xyz;");
		pipeline->DeclareFunction(GPipeline::GLOBAL_SCOPE, u8"float32", u8"PI", { }, u8"return 3.14159265359f;");
		pipeline->DeclareFunction(GPipeline::GLOBAL_SCOPE, u8"vec2f", u8"SphericalCoordinates", { { u8"vec3f", u8"v" } }, u8"vec2f uv = vec2(atan(v.z, v.x), asin(v.y)); uv *= vec2(0.1591, 0.3183); uv += 0.5; return uv; ");
		pipeline->DeclareFunction(GPipeline::GLOBAL_SCOPE, u8"float32", u8"DistributionGGX", { { u8"vec3f", u8"N"}, { u8"vec3f", u8"H"}, { u8"float32", u8"roughness"} }, u8"float32 a = roughness * roughness; float32 a2 = a * a; float32 NdotH = max(dot(N, H), 0.0); float32 NdotH2 = NdotH * NdotH; float32 num = a2; float32 denom = (NdotH2 * (a2 - 1.0) + 1.0); denom = PI() * denom * denom; return num / denom;");
		pipeline->DeclareFunction(GPipeline::GLOBAL_SCOPE, u8"float32", u8"GeometrySchlickGGX", { { u8"float32", u8"NdotV"}, { u8"float32", u8"roughness"} }, u8"float32 r = (roughness + 1.0); float32 k = (r * r) / 8.0; float32 num = NdotV; float32 denom = NdotV * (1.0 - k) + k; return num / denom;");
		pipeline->DeclareFunction(GPipeline::GLOBAL_SCOPE, u8"float32", u8"GeometrySmith", { { u8"vec3f", u8"N"}, { u8"vec3f", u8"V"}, { u8"vec3f", u8"L"}, { u8"float32", u8"roughness" } }, u8"float32 NdotV = max(dot(N, V), 0.0); float32 NdotL = max(dot(N, L), 0.0); float32 ggx2 = GeometrySchlickGGX(NdotV, roughness); float32 ggx1 = GeometrySchlickGGX(NdotL, roughness); return ggx1 * ggx2;");

		pipeline->DeclareFunction(GPipeline::GLOBAL_SCOPE, u8"float32", u8"LinearizeDepth", { { u8"float32", u8"depth"}, { u8"float32", u8"near"}, { u8"float32", u8"far" } }, u8"return (near * far) / (far + depth * (near - far));");

		pipeline->DeclareFunction(GPipeline::GLOBAL_SCOPE, u8"mat3f", u8"AngleAxis3x3", { { u8"vec3f", u8"axis"}, { u8"float32", u8"angle"} }, 
			u8"float32 c = cos(angle), s = sin(angle); float32 t = 1 - c; float32 x = axis.x; float32 y = axis.y; float z = axis.z; return mat3f(t * x * x + c, t * x * y - s * z,  t * x * z + s * y, t * x * y + s * z, t * y * y + c, t * y * z - s * x, t * x * z - s * y,  t * y * z + s * x,  t * z * z + c);");

		pipeline->DeclareFunction(GPipeline::GLOBAL_SCOPE, u8"vec3f", u8"Perpendicular", { { u8"vec3f", u8"u"} }, u8"vec3f a = abs(u); uint32 xm = ((a.x - a.y)<0 && (a.x - a.z)<0) ? 1 : 0; uint32 ym = (a.y - a.z)<0 ? (1 ^ xm) : 0; uint32 zm = 1 ^ (xm | ym); return cross(u, vec3f(xm, ym, zm));");

		pipeline->DeclareFunction(GPipeline::GLOBAL_SCOPE, u8"uint32", u8"MakeRandomSeed", { { u8"uint32", u8"val0" }, { u8"uint32", u8"val1" } },
			u8"uint32 v0 = val0, v1 = val1, s0 = 0; for (uint n = 0; n < 16; n++) { s0 += 0x9e3779b9; v0 += ((v1 << 4) + 0xa341316c) ^ (v1 + s0) ^ ((v1 >> 5) + 0xc8013ea4); v1 += ((v0 << 4) + 0xad90777d) ^ (v0 + s0) ^ ((v0 >> 5) + 0x7e95761e); } return v0;");

		pipeline->DeclareFunction(GPipeline::GLOBAL_SCOPE, u8"float32", u8"NextRandom", { { u8"inout uint32", u8"s" } },
			u8"s += 1375; return fract(sin(dot(vec2f(uint(s), uint(s) + 7), vec2f(12.9898,78.233))) * 43758.5453123);");

		pipeline->DeclareFunction(GPipeline::GLOBAL_SCOPE, u8"vec2f", u8"MapRectangleToCircle", { { u8"vec2f", u8"rect" } },
			u8"float32 radius = sqrt(rect.x); float32 angle = rect.y * 2 * PI(); return vec2f(radius * cos(angle), radius * sin(angle));");

		pipeline->DeclareFunction(GPipeline::GLOBAL_SCOPE, u8"vec3f", u8"SphereDirection", { { u8"vec2f", u8"rect" }, { u8"vec3f", u8"direction" }, { u8"float32", u8"radius" } },
			u8"vec2f point = MapRectangleToCircle(rect) * radius; vec3f tangent = normalize(cross(direction, vec3f(0, 1, 0))); vec3f bitangent = normalize(cross(tangent, direction)); return normalize(direction + point.x * tangent + point.y * bitangent);");

		pipeline->DeclareFunction(GPipeline::GLOBAL_SCOPE, u8"vec3f", u8"CosineWeightedHemisphereSample", { { u8"vec3f", u8"hitNorm" }, { u8"vec2f", u8"random" } },
			u8"vec3f bitangent = Perpendicualar(hitNorm); vec3f tangent = cross(bitangent, hitNorm); float32 r = sqrt(random.x); float phi = 2.0f * PI() * random.y; return tangent * (r * cos(phi).x) + bitangent * (r * sin(phi)) + hitNorm.xyz * sqrt(1 - random.x);");

		vertexShaderScope = pipeline->DeclareScope(GPipeline::GLOBAL_SCOPE, u8"VertexShader");
		fragmentShaderScope = pipeline->DeclareScope(GPipeline::GLOBAL_SCOPE, u8"FragmentShader");
		computeShaderScope = pipeline->DeclareScope(GPipeline::GLOBAL_SCOPE, u8"ComputeShader");
		rayGenShaderScope = pipeline->DeclareScope(GPipeline::GLOBAL_SCOPE, u8"RayGenShader");
		closestHitShaderScope = pipeline->DeclareScope(GPipeline::GLOBAL_SCOPE, u8"ClosestHitShader");
		anyHitShaderScope = pipeline->DeclareScope(GPipeline::GLOBAL_SCOPE, u8"AnyHitShader");
		missShaderScope = pipeline->DeclareScope(GPipeline::GLOBAL_SCOPE, u8"MissShader");

		pipeline->DeclareFunction(GPipeline::GLOBAL_SCOPE, u8"vec3f", u8"DirectLighting", { {u8"vec3f", u8"light_position"}, {u8"vec3f", u8"camera_position"}, {u8"vec3f", u8"surface_world_position"}, {u8"vec3f", u8"surface_normal"}, {u8"vec3f", u8"light_color"}, {u8"vec3f", u8"albedo"}, {u8"vec3f", u8"F0"}, {u8"float32", u8"roughness"} }, u8R"(
vec3f V = normalize(camera_position - surface_world_position);
vec3f L = normalize(light_position - surface_world_position);
vec3f H = normalize(V + L);
float32 distance = length(light_position - surface_world_position);
float32 attenuation = 1.0f / (distance * distance);
vec3f radiance = light_color * attenuation;
float32 NDF = DistributionGGX(surface_normal, H, roughness);
float32 G = GeometrySmith(surface_normal, V, L, roughness);
vec3f F = FresnelSchlick(max(dot(H, V), 0.0), F0);
vec3f numerator = NDF * G * F;
float32 denominator = 4.0f * max(dot(surface_normal, V), 0.0f) * max(dot(surface_normal, L), 0.0f) + 0.0001f;
vec3f specular = numerator / denominator;
float32 NdotL = max(dot(surface_normal, L), 0.0f);
vec3f kS = F; vec3f kD = vec3f(1.0) - kS; kD *= 1.0 - 0;
return (kD * albedo / PI() + specular) * radiance * NdotL;)");

		pipeline->DeclareFunction(fragmentShaderScope, u8"vec2f", u8"GetFragmentPosition", {}, u8"return gl_FragCoord.xy;");
		pipeline->DeclareFunction(fragmentShaderScope, u8"float32", u8"GetFragmentDepth", {}, u8"return gl_FragCoord.z;");

		pipeline->DeclareVariable(closestHitShaderScope, { u8"vec2f", u8"hitBarycenter" });
		pipeline->DeclareFunction(closestHitShaderScope, u8"vec3f", u8"GetVertexBarycenter", {}, u8"return Barycenter(hitBarycenter);");

		commonScope = pipeline->DeclareScope(GPipeline::GLOBAL_SCOPE, u8"CommonPermutation");

		pipeline->DeclareStruct(commonScope, u8"GlobalData", GLOBAL_DATA);
		pipeline->DeclareStruct(commonScope, u8"ViewData", VIEW_DATA);
		pipeline->DeclareStruct(commonScope, u8"CameraData", CAMERA_DATA);

		pipeline->DeclareVariable(fragmentShaderScope, { u8"vec4f", u8"Color" });
		pipeline->DeclareVariable(fragmentShaderScope, { u8"vec4f", u8"Normal" });

		auto glPositionHandle = pipeline->DeclareVariable(vertexShaderScope, { u8"vec4f", u8"gl_Position" });
		pipeline->AddMemberDeductionGuide(vertexShaderScope, u8"vertexPosition", { glPositionHandle });

		pipeline->DeclareFunction(fragmentShaderScope, u8"vec2f", u8"GetSurfaceTextureCoordinates", {}, u8"return vertexTextureCoordinates;");
		pipeline->DeclareFunction(fragmentShaderScope, u8"vec3f", u8"GetSurfaceWorldSpacePosition", {}, u8"return worldSpacePosition;");
		pipeline->DeclareFunction(fragmentShaderScope, u8"vec3f", u8"GetSurfaceWorldSpaceNormal", {}, u8"return worldSpaceNormal;");
		pipeline->DeclareFunction(fragmentShaderScope, u8"vec3f", u8"GetSurfaceViewSpacePosition", {}, u8"return viewSpacePosition;");
		pipeline->DeclareFunction(fragmentShaderScope, u8"vec4f", u8"GetSurfaceViewSpaceNormal", {}, u8"return vec4(viewSpaceNormal, 0);");

		pipeline->DeclareFunction(vertexShaderScope, u8"vec4f", u8"GetVertexPosition", {}, u8"return vec4(POSITION, 1);");
		pipeline->DeclareFunction(vertexShaderScope, u8"vec4f", u8"GetVertexNormal", {}, u8"return vec4(NORMAL, 0);");
		pipeline->DeclareFunction(vertexShaderScope, u8"vec2f", u8"GetVertexTextureCoordinates", {}, u8"return TEXTURE_COORDINATES;");

		pipeline->DeclareFunction(computeShaderScope, u8"uvec3", u8"GetThreadIndex", {}, u8"return gl_LocalInvocationID;");
		pipeline->DeclareFunction(computeShaderScope, u8"uvec3", u8"GetWorkGroupIndex", {}, u8"return gl_WorkGroupID;");
		pipeline->DeclareFunction(computeShaderScope, u8"uvec3", u8"GetGlobalIndex", {}, u8"return gl_GlobalInvocationID;");
		pipeline->DeclareFunction(computeShaderScope, u8"uvec3", u8"GetWorkGroupExtent", {}, u8"return gl_WorkGroupSize;");
		pipeline->DeclareFunction(computeShaderScope, u8"uvec3", u8"GetGlobalExtent", {}, u8"return gl_WorkGroupSize * gl_NumWorkGroups;");

		pipeline->DeclareFunction(computeShaderScope, u8"vec3f", u8"GetNormalizedGlobalIndex", {}, u8"return (vec3f(GetGlobalIndex()) + vec3f(0.5f)) / vec3f(GetGlobalExtent());");
		
		pipeline->DeclareFunction(rayGenShaderScope, u8"vec2u", u8"GetFragmentPosition", {}, u8" return gl_LaunchIDEXT.xy;");
		pipeline->DeclareFunction(rayGenShaderScope, u8"vec2f", u8"GetNormalizedFragmentPosition", {}, u8"vec2f pixelCenter = vec2f(gl_LaunchIDEXT.xy) + vec2f(0.5f); return pixelCenter / vec2f(gl_LaunchSizeEXT.xy);");

		computeRenderPassScope = pipeline->DeclareScope(commonScope, u8"ComputeRenderPass");
		pipeline->DeclareStruct(computeRenderPassScope, u8"RenderPassData", { { u8"ImageReference", u8"Albedo" } });

		auto pushConstantBlockHandle = pipeline->DeclareScope(computeRenderPassScope, u8"pushConstantBlock");
		pipeline->DeclareVariable(pushConstantBlockHandle, { u8"GlobalData*", u8"global" });
		pipeline->DeclareVariable(pushConstantBlockHandle, { u8"RenderPassData*", u8"renderPass" });
		pipeline->DeclareFunction(computeRenderPassScope, u8"vec2u", u8"GetPixelPosition", {}, u8"return GetGlobalIndex().xy;");
		pipeline->DeclareFunction(computeRenderPassScope, u8"vec4f", u8"ACES", { { u8"vec4f", u8"x" } }, u8"const float a = 2.51; const float b = 0.03; const float c = 2.43; const float d = 0.59; const float e = 0.14; return (x * (a * x + b)) / (x * (c * x + d) + e);");
		pipeline->DeclareFunction(computeRenderPassScope, u8"vec4f", u8"Filmic", { { u8"vec4f", u8"x" } }, u8"vec3 X = max(vec3(0.0), vec3f(x) - vec3f(0.004)); vec3 result = (X * (6.2 * X + 0.5)) / (X * (6.2 * X + 1.7) + 0.06); return vec4f(pow(result, vec3(2.2)), x.a); ");
	}

	GPipeline::ElementHandle commonScope, computeRenderPassScope;
	GPipeline::ElementHandle vertexShaderScope, fragmentShaderScope, computeShaderScope, rayGenShaderScope, closestHitShaderScope, anyHitShaderScope, missShaderScope;
};
