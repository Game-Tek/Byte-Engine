#pragma once

#include "PermutationManager.hpp"

#include "ByteEngine/Render/Types.hpp"

struct ForwardRenderPassPermutation : PermutationManager {
	ForwardRenderPassPermutation(const GTSL::StringView instance_name) : PermutationManager(instance_name, u8"ForwardRenderPassPermutation") {
		AddTag(u8"RenderTechnique", u8"Forward");
	}

	void Initialize(GPipeline* pipeline, ShaderGenerationData& shader_generation_data) override {
		forwardScopeHandle = pipeline->DeclareScope(GPipeline::GLOBAL_SCOPE, u8"ForwardRenderingPermutation");

		{
			auto vertexBlock = pipeline->DeclareScope(forwardScopeHandle, u8"vertex");
			pipeline->DeclareVariable(vertexBlock, { u8"vec3f", u8"POSITION" });
			pipeline->DeclareVariable(vertexBlock, { u8"vec3f", u8"NORMAL" });
			pipeline->DeclareVariable(vertexBlock, { u8"vec3f", u8"TANGENT" });
			pipeline->DeclareVariable(vertexBlock, { u8"vec3f", u8"BITANGENT" });
			pipeline->DeclareVariable(vertexBlock, { u8"vec2f", u8"TEXTURE_COORDINATES" });
		}

		forwardRenderPassScopeHandle = pipeline->DeclareStruct(forwardScopeHandle, u8"RenderPassData", { { u8"ImageReference", u8"Color" }, {u8"ImageReference", u8"Normal" }, { u8"TextureReference", u8"Position" }, {u8"ImageReference", u8"Depth"}});

		pipeline->SetMakeStruct(pipeline->DeclareStruct(forwardScopeHandle, u8"PointLightData", POINT_LIGHT_DATA));
		pipeline->DeclareStruct(forwardScopeHandle, u8"LightingData", LIGHTING_DATA);

		AddPushConstantDeclaration(pipeline, forwardScopeHandle, { { u8"GlobalData*", u8"global" }, { u8"RenderPassData*", u8"renderPass" }, { u8"CameraData*", u8"camera" }, { u8"LightingData*", u8"lightingData" }, { u8"InstanceData*", u8"instances" }, { u8"ShaderParametersData*", u8"shaderParameters" } });

		{
			auto fragmentOutputBlockHandle = pipeline->DeclareScope(forwardScopeHandle, u8"fragmentOutputBlock");
			auto outColorHandle = pipeline->DeclareVariable(fragmentOutputBlockHandle, { u8"vec4f", u8"out_Color" });
			auto outNormalHandle = pipeline->DeclareVariable(fragmentOutputBlockHandle, { u8"vec4f", u8"out_Normal" });
			pipeline->DeclareVariable(fragmentOutputBlockHandle, { u8"vec4f", u8"out_WorldPosition" });
			pipeline->AddMemberDeductionGuide(forwardScopeHandle, u8"surfaceColor", { outColorHandle });
		}

		const CommonPermutation* common_permutation = Find<CommonPermutation>(u8"CommonPermutation", shader_generation_data.Hierarchy);

		if (common_permutation) {
			auto vertexSurfaceInterface = pipeline->DeclareScope(forwardScopeHandle, u8"vertexSurfaceInterface");
			auto vertexTextureCoordinatesHandle = pipeline->DeclareVariable(vertexSurfaceInterface, { u8"vec2f", u8"vertexTextureCoordinates" });
			auto vertexViewSpacePositionHandle = pipeline->DeclareVariable(vertexSurfaceInterface, { u8"vec3f", u8"viewSpacePosition" });
			auto vertexViewSpaceNormalHandle = pipeline->DeclareVariable(vertexSurfaceInterface, { u8"vec3f", u8"viewSpaceNormal" });
			pipeline->DeclareVariable(vertexSurfaceInterface, { u8"vec3f", u8"worldSpacePosition" });
			pipeline->DeclareVariable(vertexSurfaceInterface, { u8"mat3f", u8"tbn" });
			pipeline->DeclareVariable(vertexSurfaceInterface, { u8"uint32", u8"_instanceIndex" });
		}
		else {
			BE_LOG_ERROR(u8"Needed CommonPermutation to setup state but not found in hierarchy.")
		}
	}

	GPipeline::ElementHandle forwardScopeHandle, pushConstantBlockHandle, shaderParametersHandle, forwardRenderPassScopeHandle;
};