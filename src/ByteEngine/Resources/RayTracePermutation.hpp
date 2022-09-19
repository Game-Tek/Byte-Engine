#pragma once

#include "PermutationManager.hpp"

struct RayTracePermutation : public PermutationManager {
	RayTracePermutation(const GTSL::StringView instance_name) : PermutationManager(instance_name, u8"RayTracePermutation") {
		
	}

	void Initialize(GPipeline* pipeline, ShaderGenerationData& shader_generation_data) override {
		auto rayTracePermutationScope = pipeline->DeclareScope(GPipeline::GLOBAL_SCOPE, u8"RayTracePermutation");

		pipeline->DeclareStruct(rayTracePermutationScope, u8"PointLightData", POINT_LIGHT_DATA);
		pipeline->DeclareStruct(rayTracePermutationScope, u8"LightingData", LIGHTING_DATA);
		pipeline->DeclareStruct(rayTracePermutationScope, u8"TraceRayParameterData", TRACE_RAY_PARAMETER_DATA);
		pipeline->DeclareFunction(rayTracePermutationScope, u8"void", u8"TraceRay", { { u8"vec3f", u8"origin" }, { u8"vec3f", u8"direction" }, { u8"uint32", u8"rayFlags" } }, u8"TraceRayParameterData* r = pushConstantBlock.rayTrace; traceRayEXT(accelerationStructureEXT(r.accelerationStructure), r.rayFlags | rayFlags, 0xff, r.recordOffset, r.recordStride, r.missIndex, vec3f(origin), r.tMin, vec3f(direction), r.tMax, 0);");

		pipeline->DeclareFunction(rayTracePermutationScope, u8"void", u8"TraceRayFromAToB", { { u8"vec3f", u8"start" }, { u8"vec3f", u8"end" },{ u8"uint32", u8"rayFlags" } }, u8"vec3f AB = end - start; TraceRayParameterData* r = pushConstantBlock.rayTrace; traceRayEXT(accelerationStructureEXT(r.accelerationStructure), r.rayFlags | rayFlags, 0xff, r.recordOffset, r.recordStride, r.missIndex, start, r.tMin, AB, length(AB), 0);");

		pipeline->DeclareFunction(rayTracePermutationScope, u8"void", u8"MMM", { { u8"vec3f", u8"origin" }, { u8"vec3f", u8"light_position" } }, u8"vec3f toLight = normalize(light_position - origin); vec3f perpL = cross(toLight, vec3f(0,1,0)); if(perpL == 0.0f) { perpL.x = 1.f; } vec3f toLightEdge = normalize((light_position + perpL * light_radius) - origin); float coneAngle = acos(dot(toLight, toLightEdge)) * 2;");

		pipeline->DeclareFunction(rayTracePermutationScope, u8"vec3f", u8"GetConeSample", { { u8"vec3f", u8"direction" }, { u8"float32", u8"cone_angle" } }, u8"float32 cosAngle = cos(cone_angle); float32 z = NextRandom(randSeed) * (1-cosAngle) + cosAngle; float32 phi = NextRandom(randSeed) * 2 * PI(); float32 x = sqrt(1 - z * z) * cos(phi); float32 y = sqrt(1 - z * *) * sin(phi); vec3f north = vec3f(0, 1, 0); vec3f axis = normalize(cross(north, normalize(direction))); float32 angle = acos(dot(normalize(direction), north)); mat3f r = AngleAxis3x3(axis, angle); return r * vec3f(x, y, z);");

		pipeline->DeclareStruct(rayTracePermutationScope, u8"RenderPassData", { { u8"ImageReference", u8"Color" }, { u8"TextureReference", u8"Position" }, { u8"TextureReference", u8"Depth" } });
	}
private:
};