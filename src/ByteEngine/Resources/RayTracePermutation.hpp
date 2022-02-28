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
		sg.ShaderGroupJSON = u8"{ \"name\":\"DirectionalShadow\", \"instances\":[{ \"name\":\"unique\", \"parameters\":[] }], \"execution\":\"windowExtent\" }";

		auto directionalShadowScope = pipeline->DeclareScope(GPipeline::GLOBAL_SCOPE, u8"DirectionalShadow");
		auto payloadBlockHandle = pipeline->DeclareScope(directionalShadowScope, u8"payloadBlock");
		pipeline->DeclareVariable(payloadBlockHandle, { u8"float32", u8"payload" });

		pipeline->DeclareStruct(directionalShadowScope, u8"RenderPassData", { { u8"ImageReference", u8"color" }, { u8"TextureReference", u8"depth"} });

		pipeline->SetMakeStruct(pipeline->DeclareStruct(directionalShadowScope, u8"TraceRayParameterData", { { u8"uint64", u8"AccelerationStructure"}, {u8"uint32", u8"RayFlags"}, {u8"uint32", u8"SBTRecordOffset"}, {u8"uint32", u8"SBTRecordStride"}, {u8"uint32", u8"MissIndex"}, {u8"float32", u8"tMin"}, {u8"float32", u8"tMax"} }));

		pipeline->DeclareStruct(directionalShadowScope, u8"RayTraceData", { { u8"TraceRayParameterData", u8"traceRayParameters"}, { u8"InstanceData*", u8"instances" } });

		pipeline->DeclareFunction(directionalShadowScope, u8"void", u8"TraceRay", { { u8"vec4f", u8"origin" }, { u8"vec4f", u8"direction" }, { u8"uint32", u8"rayFlags" } }, u8"TraceRayParameterData r = pushConstantBlock.rayTrace.traceRayParameters; traceRayEXT(accelerationStructureEXT(r.AccelerationStructure), r.RayFlags | rayFlags, 0xff, r.SBTRecordOffset, r.SBTRecordStride, r.MissIndex, vec3f(origin), r.tMin, vec3f(direction), r.tMax, 0);");

		{
			auto pushConstantBlockHandle = pipeline->DeclareScope(directionalShadowScope, u8"pushConstantBlock");
			pipeline->DeclareVariable(pushConstantBlockHandle, { u8"GlobalData*", u8"global" });
			pipeline->DeclareVariable(pushConstantBlockHandle, { u8"RenderPassData*", u8"renderPass" });
			pipeline->DeclareVariable(pushConstantBlockHandle, { u8"CameraData*", u8"camera" });
			pipeline->DeclareVariable(pushConstantBlockHandle, { u8"RayTraceData*", u8"rayTrace" });
		}


		{
			auto& s = sg.Shaders.EmplaceBack();
			auto& b = s.EmplaceBack();

			auto shaderHandle = pipeline->DeclareShader(directionalShadowScope, u8"rayGen");
			auto mainFunctionHandle = pipeline->DeclareFunction(shaderHandle, u8"void", u8"main");

			b.TargetSemantics = GAL::ShaderType::RAY_GEN;
			b.Scopes.EmplaceBack(GPipeline::GLOBAL_SCOPE);
			b.Scopes.EmplaceBack(commonPermutation->commonScope);
			b.Scopes.EmplaceBack(directionalShadowScope);
			b.Scopes.EmplaceBack(commonPermutation->rayGenShaderScope);
			b.Scopes.EmplaceBack(shaderHandle);
			tokenizeCode(u8"vec3f worldPosition = WorldPositionFromDepth(GetNormalizedFragmentPosition(), Sample(pushConstantBlock.renderPass.depth, GetFragmentPosition()).r, pushConstantBlock.camera.viewInverse); TraceRay(vec4f(worldPosition, 1.0f), normalize(vec4f(-100, 100, 0, 1) - vec4f(worldPosition, 1.0f)), gl_RayFlagsTerminateOnFirstHitEXT); float colorMultiplier; if (payload == -1.0f) { colorMultiplier = 1.0f; } else { colorMultiplier = 0.1f; } Write(pushConstantBlock.renderPass.color, GetFragmentPosition(), Sample(pushConstantBlock.renderPass.color, GetFragmentPosition()) * colorMultiplier);", pipeline->GetFunctionTokens(mainFunctionHandle));

		}

		{
			auto& s = sg.Shaders.EmplaceBack();
			auto& b = s.EmplaceBack();

			auto shaderHandle = pipeline->DeclareShader(directionalShadowScope, u8"closestHit");
			auto mainFunctionHandle = pipeline->DeclareFunction(shaderHandle, u8"void", u8"main");

			b.TargetSemantics = GAL::ShaderType::CLOSEST_HIT;
			b.Scopes.EmplaceBack(GPipeline::GLOBAL_SCOPE);
			b.Scopes.EmplaceBack(commonPermutation->commonScope);
			b.Scopes.EmplaceBack(directionalShadowScope);
			b.Scopes.EmplaceBack(commonPermutation->closestHitShaderScope);
			b.Scopes.EmplaceBack(shaderHandle);
			tokenizeCode(u8"payload = gl_HitTEXT;", pipeline->GetFunctionTokens(mainFunctionHandle));
		}

		{
			auto& s = sg.Shaders.EmplaceBack();
			auto& b = s.EmplaceBack();

			auto shaderHandle = pipeline->DeclareShader(directionalShadowScope, u8"miss");
			auto mainFunctionHandle = pipeline->DeclareFunction(shaderHandle, u8"void", u8"main");

			b.TargetSemantics = GAL::ShaderType::MISS;
			b.Scopes.EmplaceBack(GPipeline::GLOBAL_SCOPE);
			b.Scopes.EmplaceBack(commonPermutation->commonScope);
			b.Scopes.EmplaceBack(directionalShadowScope);
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