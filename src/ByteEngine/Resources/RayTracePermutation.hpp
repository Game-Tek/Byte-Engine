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

		auto* commonPermutation = Find<CommonPermutation>(u8"CommonPermutation", shader_generation_data.Hierarchy);

		AddScope(GPipeline::GLOBAL_SCOPE);
		AddScope(commonPermutation->commonScope);
	}

	GTSL::Vector<ShaderGroupDescriptor, BE::TAR> MakeShaderGroups(GPipeline* pipeline, GTSL::Range<const PermutationManager**> hierarchy) override {
		GTSL::Vector<ShaderGroupDescriptor, BE::TAR> r(4, GetTransientAllocator());

		auto* commonPermutation = Find<CommonPermutation>(u8"CommonPermutation", hierarchy);

		auto& sg = r.EmplaceBack();
		sg.ShaderGroupJSON = u8"{ \"name\":\"DirectionalShadow\", \"instances\":[{ \"name\":\"unique\", \"parameters\":[] }] }";

		auto directionalShadowScope = pipeline->DeclareScope(GPipeline::GLOBAL_SCOPE, u8"DirectionalShadow");
		auto payloadBlockHandle = pipeline->DeclareScope(directionalShadowScope, u8"payloadBlock");
		pipeline->DeclareVariable(payloadBlockHandle, { u8"float32", u8"payload" });

		pipeline->DeclareStruct(directionalShadowScope, u8"RenderPassData", { { u8"ImageReference", u8"Color" }, { u8"TextureReference", u8"Position" }, {u8"TextureReference", u8"Depth"} });

		pipeline->DeclareStruct(directionalShadowScope, u8"TraceRayParameterData", TRACE_RAY_PARAMETER_DATA);

		pipeline->DeclareFunction(directionalShadowScope, u8"void", u8"TraceRay", { { u8"vec4f", u8"origin" }, { u8"vec4f", u8"direction" }, { u8"uint32", u8"rayFlags" } }, u8"TraceRayParameterData* r = pushConstantBlock.rayTrace; traceRayEXT(accelerationStructureEXT(r.accelerationStructure), r.rayFlags | rayFlags, 0xff, r.recordOffset, r.recordStride, r.missIndex, vec3f(origin), r.tMin, vec3f(direction), r.tMax, 0);");

		AddPushConstantDeclaration(pipeline, directionalShadowScope, { { u8"GlobalData*", u8"global" }, { u8"RenderPassData*", u8"renderPass" }, { u8"CameraData*", u8"camera" }, { u8"InstanceData*", u8"instances" }, { u8"TraceRayParameterData*", u8"rayTrace" } });

		{
			auto& s = sg.Shaders.EmplaceBack();
			auto& b = s.EmplaceBack();

			auto shaderHandle = pipeline->DeclareShader(directionalShadowScope, u8"rayGen");
			auto mainFunctionHandle = pipeline->DeclareFunction(shaderHandle, u8"void", u8"main");

			b.TargetSemantics = GAL::ShaderType::RAY_GEN;
			AddScope(commonPermutation->rayGenShaderScope);
			b.Scopes.EmplaceBack(directionalShadowScope);
			b.Scopes.EmplaceBack(shaderHandle);
			pipeline->AddCodeToFunction(mainFunctionHandle, u8"vec3f worldPosition = vec3f(Sample(pushConstantBlock.renderPass.Position, GetFragmentPosition()));");
			pipeline->AddCodeToFunction(mainFunctionHandle, u8"TraceRay(vec4f(worldPosition, 1.0f), normalize(vec4f(1, 1, 0, 1)), gl_RayFlagsTerminateOnFirstHitEXT);");
			pipeline->AddCodeToFunction(mainFunctionHandle, u8"float colorMultiplier; if (payload == -1.0f)");
			pipeline->AddCodeToFunction(mainFunctionHandle, u8"{ colorMultiplier = 1.0f; } else { colorMultiplier = 0.0f; }");
			pipeline->AddCodeToFunction(mainFunctionHandle, u8"Write(pushConstantBlock.renderPass.Color, GetFragmentPosition(), Sample(pushConstantBlock.renderPass.Color, GetFragmentPosition()) * colorMultiplier);");
			b.Tags.EmplaceBack(u8"Execution", u8"windowExtent");

		}

		{
			auto& s = sg.Shaders.EmplaceBack();
			auto& b = s.EmplaceBack();

			auto shaderHandle = pipeline->DeclareShader(directionalShadowScope, u8"closestHit");
			auto mainFunctionHandle = pipeline->DeclareFunction(shaderHandle, u8"void", u8"main");

			b.TargetSemantics = GAL::ShaderType::CLOSEST_HIT;
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
			b.Scopes.EmplaceBack(directionalShadowScope);
			b.Scopes.EmplaceBack(commonPermutation->missShaderScope);
			b.Scopes.EmplaceBack(shaderHandle);
			tokenizeCode(u8"payload = -1.0f;", pipeline->GetFunctionTokens(mainFunctionHandle));
		}

		return r;
	}

	void ProcessShader(GPipeline* pipeline, GTSL::JSONMember shaderGroupJson, GTSL::JSONMember shaderJson, const GTSL::Range<const PermutationManager**> hierarchy, GTSL::StaticVector<ShaderPermutation, 8>& batches) override {
		
	}
private:
};