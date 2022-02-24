#pragma once

#include "PermutationManager.hpp"

struct RayTracePermutation : public PermutationManager {
	RayTracePermutation(const GTSL::StringView instance_name) : PermutationManager(instance_name, u8"RayTracePermutation") {
		
	}

	void Initialize(GPipeline* pipeline, ShaderGenerationData& shader_generation_data) override {
		//auto shaderRecordBlockHandle = pipeline->add(closestHitShaderScope, u8"shaderRecordBlock", GPipeline::LanguageElement::ElementType::MEMBER);
		//auto shaderRecordEntry = pipeline->DeclareVariable(shaderRecordBlockHandle, { u8"shaderParametersData*", u8"shaderEntries" });
		//pipeline->add(closestHitShaderScope, u8"surfaceNormal", GPipeline::LanguageElement::ElementType::DISABLED);
		//pipeline->DeclareFunction(closestHitShaderScope, u8"vec2f", u8"GetSurfaceTextureCoordinates", {}, u8"instanceData* instance = pushConstantBlock.rayTrace.instances[gl_InstanceCustomIndexEXT]; u16vec3 indices = instance.IndexBuffer[gl_PrimitiveID].indexTri; vec3f barycenter = GetVertexBarycenter(); return instance.VertexBuffer[indices[0]].TEXTURE_COORDINATES * barycenter.x + instance.VertexBuffer[indices[1]].TEXTURE_COORDINATES * barycenter.y + instance.VertexBuffer[indices[2]].TEXTURE_COORDINATES * barycenter.z;");
	}

	GTSL::StaticVector<Result1, 8> MakeShaderGroups() override {
		//todo: make single float pipeline

		u8"vec3f worldPosition = WorldPositionFromDepth(GetNormalizeFragmentPosition(), Sample(pushConstantBlock.depth, GetFragmentPosition()).r, pushConstantBlock.camera.viewInverse);";
		u8"TraceRay(vec4f(worldPosition, 1.0f), normalize(vec4f(-100, 100, 0, 1) - vec4f(worldPosition, 1.0f)));";
		u8"";

		return {};
	}

	void ProcessShader(GPipeline* pipeline, GTSL::JSONMember shaderGroupJson, GTSL::JSONMember shaderJson, const GTSL::StaticVector<PermutationManager*, 16>& hierarchy, GTSL::StaticVector<Result, 8>& batches) override {
		
	}
private:
};