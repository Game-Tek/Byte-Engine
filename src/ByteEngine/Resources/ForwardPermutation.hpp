#pragma once

#include "PermutationManager.hpp"

struct ForwardRenderPassPermutation : PermutationManager {
	ForwardRenderPassPermutation(const GTSL::StringView instance_name) : PermutationManager(instance_name, u8"ForwardRenderPassPermutation") {
		AddTag(u8"Forward");

		AddSupportedDomain(u8"World");
	}

	void Initialize(GPipeline* pipeline, ShaderGenerationData& shader_generation_data) override {
		forwardScopeHandle = pipeline->DeclareScope(shader_generation_data.Scopes.back(), u8"ForwardScope");

		{
			auto vertexBlock = pipeline->DeclareScope(forwardScopeHandle, u8"vertex");
			pipeline->DeclareVariable(vertexBlock, { u8"vec3f", u8"POSITION" });
			pipeline->DeclareVariable(vertexBlock, { u8"vec3f", u8"NORMAL" });
			pipeline->DeclareVariable(vertexBlock, { u8"vec3f", u8"TANGENT" });
			pipeline->DeclareVariable(vertexBlock, { u8"vec3f", u8"BITANGENT" });
			pipeline->DeclareVariable(vertexBlock, { u8"vec2f", u8"TEXTURE_COORDINATES" });
		}

		pipeline->DeclareStruct(forwardScopeHandle, u8"renderPassData", { { u8"ImageReference", u8"Color" }, {u8"ImageReference", u8"Normal" }, { u8"ImageReference", u8"Depth"} });

		shader_generation_data.Scopes.EmplaceBack(forwardScopeHandle);

		pipeline->SetMakeStruct(pipeline->DeclareStruct(forwardScopeHandle, u8"PointLightData", { { u8"vec3f", u8"position" }, {u8"float32", u8"radius"} }));
		pipeline->DeclareStruct(forwardScopeHandle, u8"LightingData", { {u8"uint32", u8"pointLightsLength"},  {u8"PointLightData[4]", u8"pointLights"} });

		pushConstantBlockHandle = pipeline->DeclareScope(forwardScopeHandle, u8"pushConstantBlock");
		pipeline->DeclareVariable(pushConstantBlockHandle, { u8"globalData*", u8"global" });
		pipeline->DeclareVariable(pushConstantBlockHandle, { u8"cameraData*", u8"camera" });
		pipeline->DeclareVariable(pushConstantBlockHandle, { u8"renderPassData*", u8"renderPass" });
		pipeline->DeclareVariable(pushConstantBlockHandle, { u8"LightingData*", u8"lightingData" });
		pipeline->DeclareVariable(pushConstantBlockHandle, { u8"instanceData*", u8"instances" });
		shaderParametersHandle = pipeline->DeclareVariable(pushConstantBlockHandle, { u8"shaderParametersData*", u8"shaderParameters" });

		pipeline->DeclareRawFunction(forwardScopeHandle, u8"mat4f", u8"GetInstancePosition", {}, u8"return mat4(pushConstantBlock.instances[gl_InstanceIndex].ModelMatrix);");

		pipeline->DeclareStruct(forwardScopeHandle, u8"instanceData", { { u8"mat4x3f", u8"ModelMatrix" }, { u8"uint32", u8"vertexBufferOffset" }, { u8"uint32", u8"indexBufferOffset" }, { u8"uint64", u8"padding" } });

		{
			auto fragmentOutputBlockHandle = pipeline->DeclareScope(forwardScopeHandle, u8"fragmentOutputBlock");
			auto outColorHandle = pipeline->DeclareVariable(fragmentOutputBlockHandle, { u8"vec4f", u8"out_Color" });
			auto outNormalHandle = pipeline->DeclareVariable(fragmentOutputBlockHandle, { u8"vec4f", u8"out_Normal" });
			pipeline->AddMemberDeductionGuide(forwardScopeHandle, u8"surfaceColor", { outColorHandle });
		}

		CommonPermutation* common_permutation = Find<CommonPermutation>(u8"CommonPermutation", shader_generation_data.Hierarchy);

		if (common_permutation) {
			pipeline->DeclareFunction(forwardScopeHandle, u8"vec3f", u8"GetCameraPosition", {}, u8"return vec3f(pushConstantBlock.camera.worldPosition);");

			auto vertexSurfaceInterface = pipeline->DeclareScope(forwardScopeHandle, u8"vertexSurfaceInterface");
			auto vertexTextureCoordinatesHandle = pipeline->DeclareVariable(vertexSurfaceInterface, { u8"vec2f", u8"vertexTextureCoordinates" });
			pipeline->AddMemberDeductionGuide(common_permutation->vertexShaderScope, u8"vertexTextureCoordinates", { { vertexSurfaceInterface }, { vertexTextureCoordinatesHandle } });
			auto vertexViewSpacePositionHandle = pipeline->DeclareVariable(vertexSurfaceInterface, { u8"vec3f", u8"viewSpacePosition" });
			pipeline->AddMemberDeductionGuide(common_permutation->vertexShaderScope, u8"vertexViewSpacePosition", { { vertexSurfaceInterface }, { vertexViewSpacePositionHandle } });
			auto vertexViewSpaceNormalHandle = pipeline->DeclareVariable(vertexSurfaceInterface, { u8"vec3f", u8"viewSpaceNormal" });
			pipeline->AddMemberDeductionGuide(common_permutation->vertexShaderScope, u8"vertexViewSpaceNormal", { { vertexSurfaceInterface }, { vertexViewSpaceNormalHandle } });
			pipeline->DeclareVariable(vertexSurfaceInterface, { u8"vec3f", u8"worldSpacePosition" });
			pipeline->DeclareVariable(vertexSurfaceInterface, { u8"vec3f", u8"worldSpaceNormal" });
		}
		else {
			BE_LOG_ERROR(u8"Needed CommonPermutation to setup state but not found in hierarchy.")
		}
	}

	void ProcessShader(GPipeline* pipeline, GTSL::JSONMember shader_group_json, GTSL::JSONMember shader_json, GTSL::StaticVector<PermutationManager*, 16> hierarchy, GTSL::StaticVector<Result, 8>& batches) override {
		GTSL::StaticVector<StructElement, 8> shaderParameters;

		if (auto parameters = shader_group_json[u8"parameters"]) {
			for (auto p : parameters) {
				if (auto def = p[u8"defaultValue"]) {
					shaderParameters.EmplaceBack(p[u8"type"], p[u8"name"], def);
				}
				else {
					shaderParameters.EmplaceBack(p[u8"type"], p[u8"name"], u8"");
				}
			}
		}

		auto shaderScope = pipeline->DeclareShader(forwardScopeHandle, shader_json[u8"name"]);
		auto mainFunctionHandle = pipeline->DeclareFunction(shaderScope, u8"void", u8"main");

		{ //add deduction guides for reaching shader parameters
			auto shaderParametersStructHandle = pipeline->DeclareStruct(shaderScope, u8"shaderParametersData", shaderParameters);

			for (auto& e : shaderParameters) {
				pipeline->AddMemberDeductionGuide(shaderScope, e.Name, { pushConstantBlockHandle, shaderParametersHandle, pipeline->GetElementHandle(shaderParametersStructHandle, e.Name) });
			}
		}

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

		auto& main = pipeline->GetFunction({ shaderScope }, u8"main");

		switch (Hash(shader_group_json[u8"domain"])) {
		case GTSL::Hash(u8"World"): {
			auto& batch = batches.EmplaceBack();

			batch.Tags = GetTagList();
			batch.Scopes.EmplaceBack(GPipeline::ElementHandle());

			CommonPermutation* common_permutation = Find<CommonPermutation>(u8"CommonPermutation", hierarchy);
			batch.Scopes.EmplaceBack(common_permutation->commonScope);
			batch.Scopes.EmplaceBack(forwardScopeHandle);

			switch (Hash(shader_json[u8"class"])) {
			case GTSL::Hash(u8"Vertex"): {
				batch.TargetSemantics = GAL::ShaderType::VERTEX;
				batch.Scopes.EmplaceBack(common_permutation->vertexShaderScope);
				batch.Scopes.EmplaceBack(shaderScope);

				tokenizeCode(u8"vertexTextureCoordinates = GetVertexTextureCoordinates(); vertexSurfaceInterface.worldSpacePosition = vec3f(GetInstancePosition() * GetVertexPosition()); vertexSurfaceInterface.worldSpaceNormal = vec3f(GetInstancePosition() * GetVertexNormal());", main.Tokens, GetPersistentAllocator());
				tokenizeCode(shader_json[u8"code"], main.Tokens, GetPersistentAllocator());

				//todo: analyze if we need to emit compute shader
				break;
			}
			case GTSL::Hash(u8"Surface"): {
				batch.TargetSemantics = GAL::ShaderType::FRAGMENT;
				batch.Scopes.EmplaceBack(common_permutation->fragmentShaderScope);
				batch.Scopes.EmplaceBack(shaderScope);

				tokenizeCode(u8"float32 surfaceRoughness = 1.0f; vec4f surfaceNormal = vec4f(0, 0, -1, 0);", main.Tokens, GetPersistentAllocator());
				tokenizeCode(shader_json[u8"code"], main.Tokens, GetPersistentAllocator());
				tokenizeCode(u8"vec4f BE_COLOR_0 = surfaceColor; surfaceColor = vec4f(0); for(uint32 i = 0; i < pushConstantBlock.lightingData.pointLightsLength; ++i) { PointLightData l = pushConstantBlock.lightingData.pointLights[i]; surfaceColor += vec4f(light(l.position, GetCameraPosition(), GetSurfaceWorldSpacePosition(), GetSurfaceWorldSpaceNormal(), vec3f(1) * l.radius, normalize(GetCameraPosition() - GetSurfaceWorldSpacePosition()), vec3f(BE_COLOR_0), vec3f(0.04f), surfaceRoughness), 0.1); }", main.Tokens, GetPersistentAllocator());

				break;
			}
			case GTSL::Hash(u8"Miss"): {
				batch.TargetSemantics = GAL::ShaderType::COMPUTE;
				batch.Scopes.EmplaceBack(common_permutation->computeShaderScope);
				batch.Scopes.EmplaceBack(shaderScope);
				//todo: emit compute shader for raster
				break;
			}
			default: {
				batches.PopBack(); //remove added batch as no shader was created
				BE_LOG_ERROR(u8"Can't utilize this shader class in this domain.")
			}
			}

			break;
		}
		}
	}

	GPipeline::ElementHandle forwardScopeHandle, pushConstantBlockHandle, shaderParametersHandle;
};