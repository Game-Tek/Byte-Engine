#pragma once

#include "PermutationManager.hpp"

struct RayTracePermutation : public PermutationManager {
	RayTracePermutation(const GTSL::StringView instance_name) : PermutationManager(instance_name, u8"RayTracePermutation") {
		
	}

	void Initialize(GPipeline* pipeline, ShaderGenerationData& shader_generation_data) override {
		auto rayTracePermutationScope = pipeline->DeclareScope(GPipeline::GLOBAL_SCOPE, u8"RayTracePermutation");

		pipeline->DeclareStruct(rayTracePermutationScope, u8"TraceRayParameterData", TRACE_RAY_PARAMETER_DATA);
		pipeline->DeclareFunction(rayTracePermutationScope, u8"void", u8"TraceRay", { { u8"vec4f", u8"origin" }, { u8"vec4f", u8"direction" }, { u8"uint32", u8"rayFlags" } }, u8"TraceRayParameterData* r = pushConstantBlock.rayTrace; traceRayEXT(accelerationStructureEXT(r.accelerationStructure), r.rayFlags | rayFlags, 0xff, r.recordOffset, r.recordStride, r.missIndex, vec3f(origin), r.tMin, vec3f(direction), r.tMax, 0);");

		pipeline->DeclareStruct(rayTracePermutationScope, u8"RenderPassData", { { u8"ImageReference", u8"Color" }, { u8"TextureReference", u8"Position" }, { u8"TextureReference", u8"Depth" } });
	}
private:
};