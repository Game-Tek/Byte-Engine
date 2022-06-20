#pragma once

#include "PermutationManager.hpp"

#include "ByteEngine/Render/Types.hpp"

struct ForwardRenderPassPermutation : PermutationManager {
	ForwardRenderPassPermutation(const GTSL::StringView instance_name) : PermutationManager(instance_name, u8"ForwardRenderPassPermutation") {
		AddTag(u8"RenderTechnique", u8"Forward");

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

		forwardRenderPassScopeHandle = AddRenderPassDeclaration(pipeline, u8"ForwardRenderPass", { { u8"ImageReference", u8"Color" }, {u8"ImageReference", u8"Normal" }, { u8"TextureReference", u8"Position" }, {u8"ImageReference", u8"Depth"}});

		shader_generation_data.Scopes.EmplaceBack(forwardScopeHandle);
		shader_generation_data.Scopes.EmplaceBack(forwardRenderPassScopeHandle);

		pipeline->SetMakeStruct(pipeline->DeclareStruct(forwardScopeHandle, u8"PointLightData", POINT_LIGHT_DATA));
		pipeline->DeclareStruct(forwardScopeHandle, u8"LightingData", LIGHTING_DATA);

		pushConstantBlockHandle = pipeline->DeclareScope(forwardScopeHandle, u8"pushConstantBlock");
		pipeline->DeclareVariable(pushConstantBlockHandle, { u8"GlobalData*", u8"global" });
		pipeline->DeclareVariable(pushConstantBlockHandle, { u8"RenderPassData*", u8"renderPass" });
		pipeline->DeclareVariable(pushConstantBlockHandle, { u8"CameraData*", u8"camera" });
		pipeline->DeclareVariable(pushConstantBlockHandle, { u8"LightingData*", u8"lightingData" });
		pipeline->DeclareVariable(pushConstantBlockHandle, { u8"InstanceData*", u8"instances" });
		shaderParametersHandle = pipeline->DeclareVariable(pushConstantBlockHandle, { u8"shaderParametersData*", u8"shaderParameters" });

		pipeline->DeclareFunction(forwardScopeHandle, u8"matrix4f", u8"GetInstancePosition", {}, u8"return matrix4f(pushConstantBlock.instances[gl_InstanceIndex].transform);");

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
			pipeline->DeclareVariable(vertexSurfaceInterface, { u8"vec3f", u8"worldSpaceNormal" });
			pipeline->DeclareVariable(vertexSurfaceInterface, { u8"mat3f", u8"tbn" });
			pipeline->DeclareVariable(vertexSurfaceInterface, { u8"uint32", u8"_instanceIndex" });
		}
		else {
			BE_LOG_ERROR(u8"Needed CommonPermutation to setup state but not found in hierarchy.")
		}
	}

	GTSL::Vector<ShaderGroupDescriptor, BE::TAR> MakeShaderGroups(GPipeline* pipeline, GTSL::Range<const PermutationManager**> hierarchy) override {
		return { GetTransientAllocator() };
	}

	void ProcessShader(GPipeline* pipeline, GTSL::JSONMember shader_group_json, GTSL::JSONMember shader_json, const GTSL::Range<const PermutationManager**> hierarchy, GTSL::StaticVector<ShaderPermutation, 8>& batches) override {
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

			//for (auto& e : shaderParameters) {
			//	pipeline->AddMemberDeductionGuide(shaderScope, e.Name, { pushConstantBlockHandle, shaderParametersHandle, pipeline->GetElementHandle(/shaderParametersStructHandle, /e.Name) });
			//}
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
			batch.Scopes.EmplaceBack(GPipeline::GLOBAL_SCOPE);

			const CommonPermutation* common_permutation = Find<CommonPermutation>(u8"CommonPermutation", hierarchy);
			batch.Scopes.EmplaceBack(common_permutation->commonScope);
			batch.Scopes.EmplaceBack(forwardScopeHandle);
			batch.Scopes.EmplaceBack(forwardRenderPassScopeHandle);

			switch (Hash(shader_json[u8"class"])) { //
			case GTSL::Hash(u8"Vertex"): {
				batch.TargetSemantics = GAL::ShaderType::VERTEX;
				batch.Scopes.EmplaceBack(common_permutation->vertexShaderScope);
				batch.Scopes.EmplaceBack(shaderScope);

				pipeline->AddCodeToFunction(mainFunctionHandle, u8"const matrix4f BE_VIEW_PROJECTION_MATRIX = pushConstantBlock.camera.viewHistory[0].vp;");
				tokenizeCode(u8"vertexTextureCoordinates = GetVertexTextureCoordinates(); worldSpacePosition = vec3f(GetInstancePosition() * GetVertexPosition()); worldSpaceNormal = vec3f(GetInstancePosition() * GetVertexNormal()); _instanceIndex = gl_InstanceIndex;", main.Tokens, GetPersistentAllocator());
				tokenizeCode(shader_json[u8"code"], main.Tokens, GetPersistentAllocator());
				tokenizeCode(u8"tbn = mat3f(normalize(vec3f(GetInstancePosition() * vec4f(TANGENT, 0))), normalize(vec3f(GetInstancePosition() * vec4f(BITANGENT, 0))), normalize(vec3f(GetInstancePosition() * vec4f(NORMAL, 0))));", main.Tokens, GetPersistentAllocator());

				//todo: analyze if we need to emit compute shader
				break;
			}
			case GTSL::Hash(u8"Surface"): {
				batch.TargetSemantics = GAL::ShaderType::FRAGMENT;
				batch.Scopes.EmplaceBack(common_permutation->fragmentShaderScope);
				batch.Scopes.EmplaceBack(shaderScope);

				for (auto& e : shaderParameters) {
					pipeline->AddCodeToFunction(mainFunctionHandle, (GTSL::StaticString<256>(e.Type) & e.Name)+ u8"=" + u8"pushConstantBlock.shaderParameters[pushConstantBlock.instances[_instanceIndex].shaderGroupIndex]." + e.Name + u8";");
				}

				pipeline->AddCodeToFunction(mainFunctionHandle, u8"const matrix4f BE_VIEW_PROJECTION_MATRIX = pushConstantBlock.camera.viewHistory[0].vp;");
				pipeline->AddCodeToFunction(mainFunctionHandle, u8"const matrix4f BE_VIEW_MATRIX = pushConstantBlock.camera.viewHistory[0].view;");
				pipeline->AddCodeToFunction(mainFunctionHandle, u8"const vec3f BE_CAMERA_POSITION = vec3f(BE_VIEW_MATRIX[0][3], BE_VIEW_MATRIX[1][3], BE_VIEW_MATRIX[2][3]);");
				tokenizeCode(u8"float32 surfaceRoughness = 1.0f; vec4f surfaceNormal = vec4f(0, 0, -1, 0);", main.Tokens, GetPersistentAllocator());
				tokenizeCode(shader_json[u8"code"], main.Tokens, GetPersistentAllocator());
				tokenizeCode(u8"surfaceNormal = vec4f(normalize(tbn * surfaceNormal.xyz), 0);", main.Tokens, GetPersistentAllocator());
				tokenizeCode(u8"vec4f BE_COLOR_0 = surfaceColor; surfaceColor = vec4f(0); if(!bool(DEBUG)) { for(uint32 i = 0; i < pushConstantBlock.lightingData.pointLightsLength; ++i) { PointLightData l = pushConstantBlock.lightingData.pointLights[i]; surfaceColor += vec4f(light(l.position, BE_CAMERA_POSITION, GetSurfaceWorldSpacePosition(), surfaceNormal.xyz, l.color * l.intensity, normalize(BE_CAMERA_POSITION - GetSurfaceWorldSpacePosition()), vec3f(BE_COLOR_0), vec3f(0.04f), surfaceRoughness), 1.0f); out_WorldPosition = vec4f(worldSpacePosition, 1); } } else { surfaceColor = vec4f(1.0f); }", main.Tokens, GetPersistentAllocator());
				//tokenizeCode(u8"vec4f BE_COLOR_0 = surfaceColor; surfaceColor = vec4f(0); for(uint32 i = 0; i < pushConstantBlock.lightingData.pointLightsLength; ++i) { PointLightData l = pushConstantBlock.lightingData.pointLights[i]; surfaceColor += vec4f(light(l.position, GetCameraPosition(), GetSurfaceWorldSpacePosition(), GetSurfaceWorldSpaceNormal(), vec3f(1) * l.radius, normalize(GetCameraPosition() - GetSurfaceWorldSpacePosition()), vec3f(BE_COLOR_0), vec3f(0.04f), surfaceRoughness), 0.1); }", main.Tokens, GetPersistentAllocator());

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

	GPipeline::ElementHandle forwardScopeHandle, pushConstantBlockHandle, shaderParametersHandle, forwardRenderPassScopeHandle;
};