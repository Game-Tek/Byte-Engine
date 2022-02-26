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

	GTSL::Vector<Result1, BE::TAR> MakeShaderGroups(GPipeline* pipeline, GTSL::Range<const PermutationManager**> hierarchy) override {
		GTSL::Vector<Result1, BE::TAR> r(4, GetTransientAllocator());

		auto* commonPermutation = Find<CommonPermutation>(u8"CommonPermutation", hierarchy);

		//todo: make single float pipeline

		auto& sg = r.EmplaceBack();
		sg.ShaderGroupJSON = u8"{ \"name\":\"DirectionalShadow\" }";

		{
			auto& s = sg.Shaders.EmplaceBack();
			auto& b = s.EmplaceBack();

			auto shaderHandle = pipeline->DeclareShader({}, u8"rayGen");
			auto mainFunctionHandle = pipeline->DeclareFunction(shaderHandle, u8"void", u8"main");

			b.TargetSemantics = GAL::ShaderType::RAY_GEN;
			b.Scopes.EmplaceBack();
			b.Scopes.EmplaceBack(commonPermutation->commonScope);
			b.Scopes.EmplaceBack(commonPermutation->rayGenShaderScope);
			b.Scopes.EmplaceBack(shaderHandle);
			tokenizeCode(u8"vec3f worldPosition = WorldPositionFromDepth(GetNormalizeFragmentPosition(), Sample(pushConstantBlock.depth, GetFragmentPosition()).r, pushConstantBlock.camera.viewInverse); TraceRay(vec4f(worldPosition, 1.0f), normalize(vec4f(-100, 100, 0, 1) - vec4f(worldPosition, 1.0f)), gl_RayFlagsTerminateOnFirstHitEXT); float colorMultiplier; if (payload == -1.0f) { colorMultiplier = 1.0f; } else { colorMultiplier = 0.1f; } Write(pushConstantBlock.renderPass.color, GetFragmentPosition(), Sample(pushConstantBlock.renderPass.color, GetFragmentPosition()) * colorMultiplier);", pipeline->GetFunctionTokens(mainFunctionHandle));

		}

		{
			auto& s = sg.Shaders.EmplaceBack();
			auto& b = s.EmplaceBack();

			auto shaderHandle = pipeline->DeclareShader({}, u8"closestHit");
			auto mainFunctionHandle = pipeline->DeclareFunction(shaderHandle, u8"void", u8"main");

			b.TargetSemantics = GAL::ShaderType::CLOSEST_HIT;
			b.Scopes.EmplaceBack();
			b.Scopes.EmplaceBack(commonPermutation->commonScope);
			b.Scopes.EmplaceBack(commonPermutation->closestHitShaderScope);
			b.Scopes.EmplaceBack(shaderHandle);
			tokenizeCode(u8"payload = gl_HitTEXT;", pipeline->GetFunctionTokens(mainFunctionHandle));
		}

		{
			auto& s = sg.Shaders.EmplaceBack();
			auto& b = s.EmplaceBack();

			auto shaderHandle = pipeline->DeclareShader({}, u8"miss");
			auto mainFunctionHandle = pipeline->DeclareFunction(shaderHandle, u8"void", u8"main");

			b.TargetSemantics = GAL::ShaderType::MISS;
			b.Scopes.EmplaceBack();
			b.Scopes.EmplaceBack(commonPermutation->commonScope);
			b.Scopes.EmplaceBack(commonPermutation->missShaderScope);
			b.Scopes.EmplaceBack(shaderHandle);
			tokenizeCode(u8"payload = -1.0f;", pipeline->GetFunctionTokens(mainFunctionHandle));
		}

		return r;
	}

	void ProcessShader(GPipeline* pipeline, GTSL::JSONMember shaderGroupJson, GTSL::JSONMember shaderJson, const GTSL::Range<const PermutationManager**> hierarchy, GTSL::StaticVector<Result, 8>& batches) override {
		
	}
private:
};