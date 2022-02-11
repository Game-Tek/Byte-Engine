#pragma once

#include "PermutationManager.hpp"

struct CommonPermutation : PermutationManager {
	CommonPermutation(const GTSL::StringView name) : PermutationManager(name, u8"CommonPermutation") {}

	void Initialize(GPipeline* pipeline, ShaderGenerationData& shader_generation_data) override {
		auto descriptorSetBlockHandle = pipeline->DeclareScope(GPipeline::ElementHandle(), u8"descriptorSetBlock");
		auto firstDescriptorSetBlockHandle = pipeline->DeclareScope(descriptorSetBlockHandle, u8"descriptorSet");
		pipeline->DeclareVariable(firstDescriptorSetBlockHandle, { u8"texture2D[]", u8"textures" });
		pipeline->DeclareVariable(firstDescriptorSetBlockHandle, { u8"image2D[]", u8"images" });
		pipeline->DeclareVariable(firstDescriptorSetBlockHandle, { u8"sampler", u8"s" });

		pipeline->SetMakeStruct(pipeline->DeclareStruct({}, u8"TextureReference", { { u8"uint32", u8"Instance" } }));
		pipeline->SetMakeStruct(pipeline->DeclareStruct({}, u8"ImageReference", { { u8"uint32", u8"Instance" } }));
		pipeline->SetMakeStruct(pipeline->DeclareStruct({}, u8"IndirectDispatchCommand", { { u8"uint32", u8"x" }, { u8"uint32", u8"y" }, { u8"uint32", u8"z" } }));

		pipeline->DeclareStruct({}, u8"uint32", { { u8"uint32", u8"a"} });
		pipeline->DeclareStruct({}, u8"vec2s", { { u8"u16vec2", u8"wh"} });
		pipeline->DeclareStruct({}, u8"vec2f", { { u8"vec2f", u8"xy"} });
		pipeline->DeclareStruct({}, u8"vec3f", { { u8"vec3f", u8"xyz"} });
		pipeline->DeclareStruct({}, u8"vec4f", { { u8"vec4f", u8"xyzw"} });

		pipeline->DeclareFunction({}, u8"vec3f", u8"Barycenter", { { u8"vec2f", u8"coords" } }, u8"return vec3(1.0f - coords.x - coords.y, coords.x, coords.y);");
		pipeline->DeclareFunction({}, u8"vec3f", u8"Barycenter", { { u8"vec3f", u8"p" }, { u8"vec3f", u8"a" }, { u8"vec3f", u8"b" }, { u8"vec3f", u8"c" } }, u8"vec3f v0 = b - a, v1 = c - a, v2 = p - a; float32 d00 = dot(v0, v0); float32 d01 = dot(v0, v1); float32 d11 = dot(v1, v1); float32 d20 = dot(v2, v0); float32 d21 = dot(v2, v1); float32 invDenom = 1.0f / (d00 * d11 - d01 * d01); v = (d11 * d20 - d01 * d21) * invDenom; w = (d00 * d21 - d01 * d20) * invDenom; return vec3f(1.0f - v - w, v, w);");
		pipeline->DeclareFunction({}, u8"vec4f", u8"Sample", { { u8"TextureReference", u8"tex" }, { u8"vec2f", u8"texCoord" } }, u8"return texture(sampler2D(textures[nonuniformEXT(tex.Instance)], s), texCoord);");
		pipeline->DeclareFunction({}, u8"vec4f", u8"Sample", { { u8"TextureReference", u8"tex" }, { u8"vec2f", u8"texCoord" }, { u8"vec2f", u8"ddx" }, { u8"vec2f", u8"ddy" } }, u8"return textureGrad(sampler2D(textures[nonuniformEXT(tex.Instance)], s), texCoord, ddx, ddy);");
		pipeline->DeclareFunction({}, u8"vec4f", u8"Sample", { { u8"TextureReference", u8"tex" }, { u8"uvec2", u8"pos" } }, u8"return texelFetch(sampler2D(textures[nonuniformEXT(tex.Instance)], s), ivec2(pos), 0);");
		pipeline->DeclareFunction({}, u8"vec4u", u8"SampleUint", { { u8"TextureReference", u8"tex" }, { u8"uvec2", u8"pos" } }, u8"return texelFetch(usampler2D(textures[nonuniformEXT(tex.Instance)], s), ivec2(pos), 0);");
		pipeline->DeclareFunction({}, u8"vec4f", u8"Sample", { { u8"ImageReference", u8"img" }, { u8"uvec2", u8"pos" } }, u8"return imageLoad(images[nonuniformEXT(img.Instance)], ivec2(pos));");
		pipeline->DeclareFunction({}, u8"void", u8"Write", { { u8"ImageReference", u8"img" }, { u8"uvec2", u8"pos" }, { u8"vec4f", u8"value" } }, u8"imageStore(images[nonuniformEXT(img.Instance)], ivec2(pos), value);");
		pipeline->DeclareFunction({}, u8"float32", u8"X", { { u8"vec4f", u8"vec" } }, u8"return vec.x;");
		pipeline->DeclareFunction({}, u8"float32", u8"Y", { { u8"vec4f", u8"vec" } }, u8"return vec.y;");
		pipeline->DeclareFunction({}, u8"float32", u8"Z", { { u8"vec4f", u8"vec" } }, u8"return vec.z;");
		pipeline->DeclareFunction({}, u8"vec3f", u8"FresnelSchlick", { { u8"float32", u8"cosTheta" }, { u8"vec3f", u8"F0" } }, u8"return F0 + (1.0 - F0) * pow(max(0.0, 1.0 - cosTheta), 5.0);");
		pipeline->DeclareFunction({}, u8"vec3f", u8"Normalize", { { u8"vec3f", u8"a" } }, u8"return normalize(a);");
		pipeline->DeclareFunction({}, u8"float32", u8"Sigmoid", { { u8"float32", u8"x" } }, u8"return 1.0 / (1.0 + pow(x / (1.0 - x), -3.0));");
		pipeline->DeclareFunction({}, u8"vec3f", u8"WorldPositionFromDepth", { { u8"vec2f", u8"texture_coordinate" }, { u8"float32", u8"depth_from_depth_buffer" }, { u8"mat4f", u8"inverse_projection_matrix" } }, u8"vec4 p = inverse_projection_matrix * vec4(vec3(texture_coordinate * 2.0 - vec2(1.0), depth_from_depth_buffer), 1.0); return p.xyz / p.w;\n");
		pipeline->DeclareFunction({}, u8"float32", u8"PI", { }, u8"return 3.14159265359f;");
		pipeline->DeclareFunction({}, u8"vec2f", u8"SphericalCoordinates", { { u8"vec3f", u8"v" } }, u8"vec2f uv = vec2(atan(v.z, v.x), asin(v.y)); uv *= vec2(0.1591, 0.3183); uv += 0.5; return uv; ");
		pipeline->DeclareFunction({}, u8"float32", u8"DistributionGGX", { { u8"vec3f", u8"N"}, { u8"vec3f", u8"H"}, { u8"float32", u8"roughness"} }, u8"float32 a = roughness * roughness; float32 a2 = a * a; float32 NdotH = max(dot(N, H), 0.0); float32 NdotH2 = NdotH * NdotH; float32 num = a2; float32 denom = (NdotH2 * (a2 - 1.0) + 1.0); denom = PI() * denom * denom; return num / denom;");
		pipeline->DeclareFunction({}, u8"float32", u8"GeometrySchlickGGX", { { u8"float32", u8"NdotV"}, { u8"float32", u8"roughness"} }, u8"float32 r = (roughness + 1.0); float32 k = (r * r) / 8.0; float32 num = NdotV; float32 denom = NdotV * (1.0 - k) + k; return num / denom;");
		pipeline->DeclareFunction({}, u8"float32", u8"GeometrySmith", { { u8"vec3f", u8"N"}, { u8"vec3f", u8"V"}, { u8"vec3f", u8"L"}, { u8"float32", u8"roughness" } }, u8"float32 NdotV = max(dot(N, V), 0.0); float32 NdotL = max(dot(N, L), 0.0); float32 ggx2 = GeometrySchlickGGX(NdotV, roughness); float32 ggx1 = GeometrySchlickGGX(NdotL, roughness); return ggx1 * ggx2;");

		pipeline->DeclareFunction({}, u8"float32", u8"LinearizeDepth", { { u8"float32", u8"depth"}, { u8"float32", u8"near"}, { u8"float32", u8"far" } }, u8"return (near * far) / (far + depth * (near - far));");

		vertexShaderScope = pipeline->DeclareScope(GPipeline::ElementHandle(), u8"VertexShader");
		fragmentShaderScope = pipeline->DeclareScope(GPipeline::ElementHandle(), u8"FragmentShader");
		computeShaderScope = pipeline->DeclareScope(GPipeline::ElementHandle(), u8"ComputeShader");
		rayGenShaderScope = pipeline->DeclareScope(GPipeline::ElementHandle(), u8"RayGenShader");
		closestHitShaderScope = pipeline->DeclareScope(GPipeline::ElementHandle(), u8"ClosestHitShader");
		anyHitShaderScope = pipeline->DeclareScope(GPipeline::ElementHandle(), u8"AnyHitShader");
		missShaderScope = pipeline->DeclareScope(GPipeline::ElementHandle(), u8"MissShader");

		pipeline->DeclareFunction(GPipeline::ElementHandle(), u8"vec3f", u8"light", { {u8"vec3f", u8"light_position"}, {u8"vec3f", u8"camera_position"}, {u8"vec3f", u8"surface_world_position"}, {u8"vec3f", u8"surface_normal"}, {u8"vec3f", u8"light_color"}, {u8"vec3f", u8"V"}, {u8"vec3f", u8"color"}, {u8"vec3f", u8"F0"}, {u8"float32", u8"roughness"} }, u8"vec3f L = normalize(light_position - surface_world_position); vec3f H = normalize(V + L); float32 distance = length(light_position - surface_world_position); float32 attenuation = 1.0f / (distance * distance); vec3f radiance = light_color * attenuation; float32 NDF = DistributionGGX(surface_normal, H, roughness); float32 G = GeometrySmith(surface_normal, V, L, roughness); vec3f F = FresnelSchlick(max(dot(H, V), 0.0), F0); vec3f kS = F; vec3f kD = vec3f(1.0) - kS; kD *= 1.0 - 0; vec3f numerator = NDF * G * F; float32 denominator = 4.0f * max(dot(surface_normal, V), 0.0f) * max(dot(surface_normal, L), 0.0f) + 0.0001f; vec3f specular = numerator / denominator; float32 NdotL = max(dot(surface_normal, L), 0.0f); return (kD * color / PI() + specular) * radiance * NdotL;");

		pipeline->DeclareFunction(fragmentShaderScope, u8"vec2f", u8"GetFragmentPosition", {}, u8"return gl_FragCoord.xy;");
		pipeline->DeclareFunction(fragmentShaderScope, u8"float32", u8"GetFragmentDepth", {}, u8"return gl_FragCoord.z;");

		pipeline->DeclareVariable(closestHitShaderScope, { u8"vec2f", u8"hitBarycenter" });
		pipeline->DeclareFunction(closestHitShaderScope, u8"vec3f", u8"GetVertexBarycenter", {}, u8"return Barycenter(hitBarycenter);");

		commonScope = pipeline->DeclareScope({}, u8"Common");
		shader_generation_data.Scopes.EmplaceBack(commonScope);

		pipeline->DeclareStruct(commonScope, u8"globalData", { { u8"uint32", u8"frameIndex" }, {u8"float32", u8"time"} });
		pipeline->DeclareStruct(commonScope, u8"cameraData", { { u8"mat4f", u8"view" }, {u8"mat4f", u8"proj"}, {u8"mat4f", u8"viewInverse"}, {u8"mat4f", u8"projInverse"}, {u8"mat4f", u8"vp"}, {u8"vec4f", u8"worldPosition"}, { u8"float32", u8"near" }, { u8"float32", u8"far" }, { u8"u16vec2", u8"extent" } });

		pipeline->DeclareVariable(fragmentShaderScope, { u8"vec4f", u8"Color" });
		pipeline->DeclareVariable(fragmentShaderScope, { u8"vec4f", u8"Normal" });

		auto glPositionHandle = pipeline->DeclareVariable(vertexShaderScope, { u8"vec4f", u8"gl_Position" });
		pipeline->AddMemberDeductionGuide(vertexShaderScope, u8"vertexPosition", { glPositionHandle });

		pipeline->DeclareStruct(rayGenShaderScope, u8"traceRayParameterData", { { u8"uint64", u8"AccelerationStructure"}, {u8"uint32", u8"RayFlags"}, {u8"uint32", u8"SBTRecordOffset"}, {u8"uint32", u8"SBTRecordStride"}, {u8"uint32", u8"MissIndex"}, {u8"float32", u8"tMin"}, {u8"float32", u8"tMax"} });
		pipeline->DeclareStruct(missShaderScope, u8"traceRayParameterData", { { u8"uint64", u8"AccelerationStructure"}, {u8"uint32", u8"RayFlags"}, {u8"uint32", u8"SBTRecordOffset"}, {u8"uint32", u8"SBTRecordStride"}, {u8"uint32", u8"MissIndex"}, {u8"float32", u8"tMin"}, {u8"float32", u8"tMax"} });
		pipeline->DeclareStruct(closestHitShaderScope, u8"traceRayParameterData", { { u8"uint64", u8"AccelerationStructure"}, {u8"uint32", u8"RayFlags"}, {u8"uint32", u8"SBTRecordOffset"}, {u8"uint32", u8"SBTRecordStride"}, {u8"uint32", u8"MissIndex"}, {u8"float32", u8"tMin"}, {u8"float32", u8"tMax"} });

		pipeline->DeclareStruct(rayGenShaderScope, u8"rayTraceData", { { u8"traceRayParameterData", u8"traceRayParameters"}, { u8"uint64", u8"instances" } });
		pipeline->DeclareStruct(missShaderScope, u8"rayTraceData", { { u8"traceRayParameterData", u8"traceRayParameters"}, { u8"uint64", u8"instances" } });
		pipeline->DeclareStruct(closestHitShaderScope, u8"rayTraceData", { { u8"traceRayParameterData", u8"traceRayParameters"}, { u8"instanceData*", u8"instances" } });

		pipeline->DeclareFunction(fragmentShaderScope, u8"vec2f", u8"GetSurfaceTextureCoordinates", {}, u8"return vertexIn.vertexTextureCoordinates;");
		pipeline->DeclareFunction(fragmentShaderScope, u8"mat4f", u8"GetInverseProjectionMatrix", {}, u8"return pushConstantBlock.camera.projInverse;");
		pipeline->DeclareFunction(fragmentShaderScope, u8"vec3f", u8"GetSurfaceWorldSpacePosition", {}, u8"return vertexIn.worldSpacePosition;");
		pipeline->DeclareFunction(fragmentShaderScope, u8"vec3f", u8"GetSurfaceWorldSpaceNormal", {}, u8"return vertexIn.worldSpaceNormal;");
		pipeline->DeclareFunction(fragmentShaderScope, u8"vec3f", u8"GetSurfaceViewSpacePosition", {}, u8"return vertexIn.viewSpacePosition;");
		pipeline->DeclareFunction(fragmentShaderScope, u8"vec4f", u8"GetSurfaceViewSpaceNormal", {}, u8"return vec4(vertexIn.viewSpaceNormal, 0);");

		pipeline->DeclareFunction(vertexShaderScope, u8"vec4f", u8"GetVertexPosition", {}, u8"return vec4(POSITION, 1);");
		pipeline->DeclareFunction(vertexShaderScope, u8"vec4f", u8"GetVertexNormal", {}, u8"return vec4(NORMAL, 0);");
		pipeline->DeclareFunction(vertexShaderScope, u8"vec2f", u8"GetVertexTextureCoordinates", {}, u8"return TEXTURE_COORDINATES;");
		pipeline->DeclareFunction(vertexShaderScope, u8"mat4f", u8"GetCameraViewMatrix", {}, u8"return pushConstantBlock.camera.view;");
		pipeline->DeclareFunction(vertexShaderScope, u8"mat4f", u8"GetCameraProjectionMatrix", {}, u8"return pushConstantBlock.camera.proj;");

		pipeline->DeclareFunction(computeShaderScope, u8"uvec3", u8"GetThreadIndex", {}, u8"return gl_LocalInvocationID;");
		pipeline->DeclareFunction(computeShaderScope, u8"uvec3", u8"GetWorkGroupIndex", {}, u8"return gl_WorkGroupID;");
		pipeline->DeclareFunction(computeShaderScope, u8"uvec3", u8"GetGlobalIndex", {}, u8"return gl_GlobalInvocationID;");
		pipeline->DeclareFunction(computeShaderScope, u8"uvec3", u8"GetWorkGroupExtent", {}, u8"return gl_WorkGroupSize;");
		pipeline->DeclareFunction(computeShaderScope, u8"uvec3", u8"GetGlobalExtent", {}, u8"return gl_WorkGroupSize * gl_NumWorkGroups;");

		pipeline->DeclareFunction(computeShaderScope, u8"vec310f", u8"GetNormalizedGlobalIndex", {}, u8"return vec3f(GetGlobalIndex()) / vec3f(GetGlobalExtent());");

		pipeline->DeclareFunction(rayGenShaderScope, u8"mat4f", u8"GetInverseViewMatrix", {}, u8"return pushConstantBlock.camera.viewInverse;");
		pipeline->DeclareFunction(rayGenShaderScope, u8"mat4f", u8"GetInverseProjectionMatrix", {}, u8"return pushConstantBlock.camera.projInverse;");
		pipeline->DeclareFunction(rayGenShaderScope, u8"void", u8"TraceRay", { { u8"vec4f", u8"origin" }, { u8"vec4f", u8"direction" } }, u8"traceRayParameterData r = pushConstantBlock.rayTrace.traceRayParameters; traceRayEXT(accelerationStructureEXT(r.AccelerationStructure), r.RayFlags, 0xff, r.SBTRecordOffset, r.SBTRecordStride, r.MissIndex, vec3f(origin), r.tMin, vec3f(direction), r.tMax, 0);");
		pipeline->DeclareFunction(rayGenShaderScope, u8"vec2u", u8"GetFragmentPosition", {}, u8" return gl_LaunchIDEXT.xy;");
		pipeline->DeclareFunction(rayGenShaderScope, u8"vec2f", u8"GetFragmentNormalizedPosition", {}, u8"vec2f pixelCenter = 1vec2f(gl_LaunchIDEXT.xy) + vec2f(0.5f); return pixelCenter / vec2f(gl_LaunchSizeEXT.xy - 1);");

		//auto shaderRecordBlockHandle = pipeline->add(closestHitShaderScope, u8"shaderRecordBlock", GPipeline::LanguageElement::ElementType::MEMBER);
		//auto shaderRecordEntry = pipeline->DeclareVariable(shaderRecordBlockHandle, { u8"shaderParametersData*", u8"shaderEntries" });
		//pipeline->add(closestHitShaderScope, u8"surfaceNormal", GPipeline::LanguageElement::ElementType::DISABLED);
		//pipeline->DeclareFunction(closestHitShaderScope, u8"vec2f", u8"GetSurfaceTextureCoordinates", {}, u8"instanceData* instance = pushConstantBlock.rayTrace.instances[gl_InstanceCustomIndexEXT]; u16vec3 indices = instance.IndexBuffer[gl_PrimitiveID].indexTri; vec3f barycenter = GetVertexBarycenter(); return instance.VertexBuffer[indices[0]].TEXTURE_COORDINATES * barycenter.x + instance.VertexBuffer[indices[1]].TEXTURE_COORDINATES * barycenter.y + instance.VertexBuffer[indices[2]].TEXTURE_COORDINATES * barycenter.z;");

		computeRenderPassScope = pipeline->DeclareScope(commonScope, u8"ComputeRenderPass");
		pipeline->DeclareStruct(computeRenderPassScope, u8"renderPassData", { { u8"ImageReference", u8"Color" } });

		auto pushConstantBlockHandle = pipeline->DeclareScope(computeRenderPassScope, u8"pushConstantBlock");
		pipeline->DeclareVariable(pushConstantBlockHandle, { u8"globalData*", u8"global" });
		pipeline->DeclareVariable(pushConstantBlockHandle, { u8"renderPassData*", u8"renderPass" });
		pipeline->DeclareFunction(computeRenderPassScope, u8"vec2u", u8"GetPixelPosition", {}, u8"return GetGlobalIndex().xy;");

		AddSupportedDomain(u8"Screen");
	}

	GTSL::StaticVector<Result1, 8> MakeShaderGroups() override { return {}; }

	void ProcessShader(GPipeline* pipeline, GTSL::JSONMember shaderGroupJson, GTSL::JSONMember shader_json, GTSL::StaticVector<PermutationManager*, 16> hierarchy, GTSL::StaticVector<Result, 8>& batches) override {
		if (shaderGroupJson[u8"domain"].GetStringView() == u8"Screen") {
			if (shader_json[u8"class"].GetStringView() == u8"Compute") {
				auto shaderScope = pipeline->DeclareScope(computeRenderPassScope, shader_json[u8"name"]);
				auto mainFunctionHandle = pipeline->DeclareFunction(shaderScope, u8"void", u8"main");
				auto& main = pipeline->GetFunction({ shaderScope }, u8"main");

				tokenizeCode(u8"vec4f color = Sample(pushConstantBlock.renderPass.Color, GetPixelPosition());", main.Tokens, GetPersistentAllocator()); //insert variable shader will use to store color
				tokenizeCode(shader_json[u8"code"], main.Tokens, GetPersistentAllocator());
				tokenizeCode(u8"Write(pushConstantBlock.renderPass.Color, GetPixelPosition(), color);", main.Tokens, GetPersistentAllocator()); //store final "color" value to image

				if (auto res = shader_json[u8"localSize"]) {
					pipeline->DeclareVariable(shaderScope, { u8"uint16", u8"group_size_x", res[0].GetStringView() });
					pipeline->DeclareVariable(shaderScope, { u8"uint16", u8"group_size_y", res[1].GetStringView() });
					pipeline->DeclareVariable(shaderScope, { u8"uint16", u8"group_size_z", res[2].GetStringView() });
				}
				else {
					pipeline->DeclareVariable(shaderScope, { u8"uint16", u8"group_size_x", u8"1" });
					pipeline->DeclareVariable(shaderScope, { u8"uint16", u8"group_size_y", u8"1" });
					pipeline->DeclareVariable(shaderScope, { u8"uint16", u8"group_size_z", u8"1" });
				}

				auto& batch = batches.EmplaceBack();
				batch.TargetSemantics = GAL::ShaderType::COMPUTE;
				batch.Scopes.PushBack({ {}, commonScope, computeShaderScope, computeRenderPassScope, shaderScope });
			}
		}
	}

	GPipeline::ElementHandle commonScope, computeRenderPassScope;
	GPipeline::ElementHandle vertexShaderScope, fragmentShaderScope, computeShaderScope, rayGenShaderScope, closestHitShaderScope, anyHitShaderScope, missShaderScope;
};