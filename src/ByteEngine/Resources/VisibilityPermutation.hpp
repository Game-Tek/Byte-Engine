#pragma once

#include "PermutationManager.hpp"

struct VisibilityRenderPassPermutation : PermutationManager {
	VisibilityRenderPassPermutation(const GTSL::StringView instance_name) : PermutationManager(instance_name, u8"VisibilityRenderPassPermutation") {
		AddTag(u8"Visibility");

		AddSupportedDomain<VisibilityRenderPassPermutation, &VisibilityRenderPassPermutation::ProcessVisibility>(u8"Visibility");
		AddSupportedDomain(u8"CountPixels");
		AddSupportedDomain(u8"PrefixPass");
		AddSupportedDomain(u8"SelectPixels");
		AddSupportedDomain(u8"World");
	}

	void Initialize(GPipeline* pipeline, ShaderGenerationData& shader_generation_data) override {
		visibilityHandle = pipeline->DeclareScope(shader_generation_data.Scopes.back(), u8"Visibility");

		pipeline->DeclareStruct(visibilityHandle, u8"renderPassData", { { u8"ImageReference", u8"Visibility" }, { u8"ImageReference", u8"Depth"} });

		shader_generation_data.Scopes.EmplaceBack(visibilityHandle);

		pipeline->SetMakeStruct(pipeline->DeclareStruct(visibilityHandle, u8"BarycentricDeriv", { { u8"vec3f", u8"m_lambda" }, { u8"vec3f", u8"m_ddx" }, { u8"vec3f", u8"m_ddy" } }));
		pipeline->DeclareRawFunction(visibilityHandle, u8"BarycentricDeriv", u8"CalcFullBary", { { u8"vec4f", u8"pt0" }, { u8"vec4f", u8"pt1" }, { u8"vec4f", u8"pt2" }, { u8"vec2f", u8"pixelNdc" }, { u8"vec2f", u8"winSize" } }, u8"BarycentricDeriv ret; vec3f invW = vec3f(1) / vec3f(pt0.w, pt1.w, pt2.w); vec2f ndc0 = pt0.xy * invW.x; vec2f ndc1 = pt1.xy * invW.y; vec2f ndc2 = pt2.xy * invW.z; float32 invDet = 1.0f / determinant(mat2f(ndc2 - ndc1, ndc0 - ndc1)); ret.m_ddx = vec3f(ndc1.y - ndc2.y, ndc2.y - ndc0.y, ndc0.y - ndc1.y) * invDet; ret.m_ddy = vec3f(ndc2.x - ndc1.x, ndc0.x - ndc2.x, ndc1.x - ndc0.x) * invDet; vec2f deltaVec = pixelNdc - ndc0; float32 interpInvW = (invW.x + deltaVec.x * dot(invW, ret.m_ddx) + deltaVec.y * dot(invW, ret.m_ddy)); float32 interpW = 1.0f / interpInvW; ret.m_lambda.x = interpW * (invW[0] + deltaVec.x * ret.m_ddx.x * invW[0] + deltaVec.y * ret.m_ddy.x * invW[0]); ret.m_lambda.y = interpW * (0.0f + deltaVec.x * ret.m_ddx.y * invW[1] + deltaVec.y * ret.m_ddy.y * invW[1]); ret.m_lambda.z = interpW * (0.0f + deltaVec.x * ret.m_ddx.z * invW[2] + deltaVec.y * ret.m_ddy.z * invW[2]); ret.m_ddx *= (2.0f / winSize.x); ret.m_ddy *= (2.0f / winSize.y); ret.m_ddy *= -1.0f; return ret;");
		pipeline->DeclareRawFunction(visibilityHandle, u8"vec3f", u8"InterpolateWithDeriv", { { u8"BarycentricDeriv", u8"deriv" }, { u8"vec3f", u8"mergedV" } }, u8"vec3f ret; ret.x = dot(deriv.m_lambda, mergedV); ret.y = dot(deriv.m_ddx * mergedV, vec3f(1, 1, 1)); ret.z = dot(deriv.m_ddy * mergedV, vec3f(1, 1, 1)); return ret;");

		pipeline->SetMakeStruct(pipeline->DeclareStruct(visibilityHandle, u8"PointLightData", { { u8"vec3f", u8"position" }, {u8"float32", u8"radius"} }));
		pipeline->DeclareStruct(visibilityHandle, u8"LightingData", { {u8"uint32", u8"pointLightsLength"},  {u8"PointLightData[4]", u8"pointLights"} });

		pipeline->DeclareStruct(visibilityHandle, u8"VisibilityData", { { u8"vec3f*", u8"positionStream" }, { u8"vec3f*", u8"normalStream" }, { u8"vec3f*", u8"tangentStream" }, { u8"vec3f*", u8"bitangentStream" }, { u8"vec2f*", u8"textureCoordinatesStream" }, {u8"uint32", u8"shaderGroupLength"},  {u8"uint32*", u8"shaderGroupUseCount"}, {u8"uint32*", u8"shaderGroupStart" } , {u8"vec2s*", u8"pixelBuffer"} });

		pipeline->DeclareStruct(visibilityHandle, u8"InstanceData", { { u8"mat4x3f", u8"ModelMatrix" }, { u8"uint32", u8"vertexBufferOffset" }, { u8"uint32", u8"indexBufferOffset" }, { u8"uint32", u8"shaderGroupIndex" }, { u8"uint32", u8"padding" } });

		{ //visibility pass
			visibilityPass = pipeline->DeclareScope(shader_generation_data.Scopes.back(), u8"VisibilityPass");

			pushConstantBlockHandle = pipeline->DeclareScope(visibilityPass, u8"pushConstantBlock"); //todo: make handles per stage
			pipeline->DeclareVariable(pushConstantBlockHandle, { u8"globalData*", u8"global" });
			pipeline->DeclareVariable(pushConstantBlockHandle, { u8"cameraData*", u8"camera" });
			pipeline->DeclareVariable(pushConstantBlockHandle, { u8"renderPassData*", u8"renderPass" });
			pipeline->DeclareVariable(pushConstantBlockHandle, { u8"LightingData*", u8"lightingData" });
			pipeline->DeclareVariable(pushConstantBlockHandle, { u8"InstanceData*", u8"instances" });

			pipeline->DeclareRawFunction(visibilityHandle, u8"mat4f", u8"GetInstancePosition", {}, u8"return mat4(pushConstantBlock.instances[gl_InstanceIndex].ModelMatrix);");
			pipeline->DeclareRawFunction(visibilityHandle, u8"uint32", u8"GetVertexIndex", {}, u8"return gl_VertexIndex;");

			auto vertexBlock = pipeline->DeclareScope(visibilityPass, u8"vertex");
			pipeline->DeclareVariable(vertexBlock, { u8"vec3f", u8"POSITION" });
			pipeline->DeclareVariable(vertexBlock, { u8"vec3f", u8"NORMAL" });
			pipeline->DeclareVariable(vertexBlock, { u8"vec3f", u8"TANGENT" });
			pipeline->DeclareVariable(vertexBlock, { u8"vec3f", u8"BITANGENT" });
			pipeline->DeclareVariable(vertexBlock, { u8"vec2f", u8"TEXTURE_COORDINATES" });
		}

		{ //count pixels pass
			countPixelsPass = pipeline->DeclareScope(shader_generation_data.Scopes.back(), u8"CountPixelsPass");

			auto pushConstantBlockHandle = pipeline->DeclareScope(countPixelsPass, u8"pushConstantBlock"); //todo: make handles per stage
			pipeline->DeclareVariable(pushConstantBlockHandle, { u8"globalData*", u8"global" });
			pipeline->DeclareVariable(pushConstantBlockHandle, { u8"renderPassData*", u8"renderPass" });
			auto instancesPointerHandle = pipeline->DeclareVariable(pushConstantBlockHandle, { u8"instanceData*", u8"instances" });
			auto visibilityDataHandle = pipeline->DeclareVariable(pushConstantBlockHandle, { u8"VisibilityData*", u8"visibility" });

			pipeline->AddMemberDeductionGuide(countPixelsPass, u8"visibility", { pushConstantBlockHandle, visibilityDataHandle, });
			pipeline->AddMemberDeductionGuide(countPixelsPass, u8"instances", { pushConstantBlockHandle, instancesPointerHandle });
		}

		{ //prefix sum pass
			prefixSumPass = pipeline->DeclareScope(shader_generation_data.Scopes.back(), u8"PrefixSumPass");

			auto pushConstantBlockHandle = pipeline->DeclareScope(prefixSumPass, u8"pushConstantBlock"); //todo: make handles per stage
			pipeline->DeclareVariable(pushConstantBlockHandle, { u8"globalData*", u8"global" });
			pipeline->DeclareVariable(pushConstantBlockHandle, { u8"VisibilityData*", u8"visibility" });

			//todo: declare deduction guides
		}

		{ //select pixels pass
			selectPixelsPass = pipeline->DeclareScope(shader_generation_data.Scopes.back(), u8"SelectPixelsPass");

			auto pushConstantBlockHandle = pipeline->DeclareScope(selectPixelsPass, u8"pushConstantBlock"); //todo: make handles per stage
			pipeline->DeclareVariable(pushConstantBlockHandle, { u8"globalData*", u8"global" });
			pipeline->DeclareVariable(pushConstantBlockHandle, { u8"renderPassData*", u8"renderPass" });
			pipeline->DeclareVariable(pushConstantBlockHandle, { u8"VisibilityData*", u8"visibility" });

			//todo: declare deduction guides
		}

		{ //paint pass
			paintPass = pipeline->DeclareScope(shader_generation_data.Scopes.back(), u8"PaintPass");

			auto pushConstantBlockHandle = pipeline->DeclareScope(paintPass, u8"pushConstantBlock"); //todo: make handles per stage
			pipeline->DeclareVariable(pushConstantBlockHandle, { u8"globalData*", u8"global" });
			pipeline->DeclareVariable(pushConstantBlockHandle, { u8"renderPassData*", u8"renderPass" });
			pipeline->DeclareVariable(pushConstantBlockHandle, { u8"VisibilityData*", u8"visibility" });

			//compute shader surface shader scope
			pipeline->DeclareRawFunction(paintPass, u8"vec4f", u8"GetVertexPosition", {}, u8"instanceData* instance = pushConstantBlock.instances[gl_InstanceIndex]; u16vec3 indices = index*(pushConstantBlock.renderPass.indexBuffer + instance.indexBufferOffset)[gl_PrimitiveID].indexTri; PositionVertex* vertices = pushConstantBlock.visibility.positionStream + instance.vertexBufferOffset; vec3f barycenter = GetVertexBarycenter(); return vec4f(vertices[indices[0]].xyz * barycenter.x + vertices[indices[1]].xyz * barycenter.y + vertices[indices[2]].xyz * barycenter.z, 1);");

			pipeline->DeclareFunction(paintPass, u8"vec4f", u8"RandomColorFromUint", { { u8"uint32", u8"index" } }, u8"vec3f table[8] = vec3f[8](vec3f(0, 0.9, 0.4), vec3f(0, 0.2, 0.9), vec3f(1, 0.3, 1), vec3f(0.1, 0, 0.9), vec3f(1, 0.5, 0.1), vec3f(0.5, 0.4, 0.4), vec3f(1, 1, 0), vec3f(1, 0, 0)); return vec4f(table[index % 8], 1);");
		}

		CommonPermutation* common_permutation = Find<CommonPermutation>(u8"CommonPermutation", shader_generation_data.Hierarchy);

		if (common_permutation) {
			pipeline->DeclareFunction(visibilityHandle, u8"vec3f", u8"GetCameraPosition", {}, u8"return vec3f(pushConstantBlock.camera.worldPosition);");

			auto vertexSurfaceInterface = pipeline->DeclareScope(visibilityHandle, u8"vertexSurfaceInterface");
			pipeline->DeclareVariable(vertexSurfaceInterface, { u8"uint32", u8"instanceIndex" });
			pipeline->DeclareVariable(vertexSurfaceInterface, { u8"uint32", u8"triangleIndex" });

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

		auto shaderScope = pipeline->DeclareShader(visibilityHandle, shader_json[u8"name"]);
		auto mainFunctionHandle = pipeline->DeclareFunction(shaderScope, u8"void", u8"main");

		{ //add deduction guides for reaching shader parameters
			auto shaderParametersStructHandle = pipeline->DeclareStruct(shaderScope, u8"shaderParametersData", shaderParameters);

			for (auto& e : shaderParameters) {
				pipeline->AddMemberDeductionGuide(shaderScope, e.Name, { pushConstantBlockHandle, shaderParametersHandle, pipeline->GetElementHandle(shaderParametersStructHandle, e.Name) });
			}

		}

		auto& main = pipeline->GetFunction({ shaderScope }, u8"main");

		switch (Hash(shader_group_json[u8"domain"])) {
		case GTSL::Hash(u8"World"): {
			auto& batch = batches.EmplaceBack();

			batch.Tags = GetTagList();
			batch.Scopes.EmplaceBack(GPipeline::ElementHandle());

			CommonPermutation* common_permutation = Find<CommonPermutation>(u8"CommonPermutation", hierarchy);
			batch.Scopes.EmplaceBack(common_permutation->commonScope);
			batch.Scopes.EmplaceBack(visibilityHandle);
			batch.Scopes.EmplaceBack(visibilityPass);

			switch (Hash(shader_json[u8"class"])) {
			case GTSL::Hash(u8"Vertex"): {
				batch.TargetSemantics = GAL::ShaderType::VERTEX;
				batch.Scopes.EmplaceBack(common_permutation->vertexShaderScope);
				batch.Scopes.EmplaceBack(shaderScope);

				tokenizeCode(u8"vertexSurfaceInterface.instanceIndex = gl_InstanceIndex; vertexSurfaceInterface.triangleIndex = gl_VertexIndex;", main.Tokens);
				tokenizeCode(u8"vertexSurfaceInterface.worldSpacePosition = vec3f(GetInstancePosition() * GetVertexPosition()); vertexSurfaceInterface.worldSpaceNormal = vec3f(GetInstancePosition() * GetVertexNormal());", main.Tokens, GetPersistentAllocator());
				tokenizeCode(shader_json[u8"code"], main.Tokens, GetPersistentAllocator());
				break;
			}
			case GTSL::Hash(u8"Surface"): {
				batch.TargetSemantics = GAL::ShaderType::COMPUTE;
				batch.Scopes.EmplaceBack(common_permutation->computeShaderScope);
				batch.Scopes.EmplaceBack(shaderScope);
				
				tokenizeCode(u8"float32 surfaceRoughness = 1.0f; vec4f surfaceNormal = vec4f(0, 0, -1, 0); vec4f surfaceColor = vec4f(0);", main.Tokens, GetPersistentAllocator());
				tokenizeCode(u8"vec4u pixel = SampleUint(pushConstantBlock.renderPass.Visibility, GetPixelPosition()); uint32 instanceIndex = pixel.r; uint32 triangleIndex = pixel.g;", main.Tokens);
				//tokenizeCode(shader_json[u8"code"], main.Tokens, GetPersistentAllocator());
				//tokenizeCode(u8"vec4f BE_COLOR_0 = surfaceColor; surfaceColor = vec4f(0); for(uint32 i = 0; i < pushConstantBlock.lightingData.pointLightsLength; ++i) { PointLightData l = pushConstantBlock.lightingData.pointLights[i]; surfaceColor += vec4f(light(l.position, GetCameraPosition(), //GetSurfaceWorldSpacePosition(), GetSurfaceWorldSpaceNormal(), vec3f(1) * l.radius, normalize(GetCameraPosition() - GetSurfaceWorldSpacePosition()), vec3f(BE_COLOR_0), vec3f(0.04f), surfaceRoughness), 0.1); }", main.Tokens, GetPersistentAllocator());
				tokenizeCode(u8"Write(pushConstantBlock.renderPass.Color, pushConstant.visibility.pixelBuffer[GetGlobalIndex().x].hw, RandomColorFromUint(triangleIndex));", main.Tokens);
				//tokenizeCode(u8"Write(pushConstantBlock.renderPass.Color, GetPixelPosition(), surfaceColor);", main.Tokens);

				batches.PopBack();

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
		case GTSL::Hash(u8"CountPixels"): { break; }
		case GTSL::Hash(u8"PrefixPass"): { break; }
		case GTSL::Hash(u8"SelectPixels"): { break; }
		}
	}

	void ProcessVisibility(GPipeline* pipeline, GTSL::JSONMember shader_group_json, GTSL::JSONMember shader_json, GTSL::StaticVector<PermutationManager*, 16> hierarchy, GTSL::StaticVector<Result, 8>& batches) {
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

		auto shaderScope = pipeline->DeclareShader(visibilityHandle, shader_json[u8"name"]);
		auto mainFunctionHandle = pipeline->DeclareFunction(shaderScope, u8"void", u8"main");
		auto& main = pipeline->GetFunction({ shaderScope }, u8"main");

		{ //add deduction guides for reaching shader parameters
			auto shaderParametersStructHandle = pipeline->DeclareStruct(shaderScope, u8"shaderParametersData", shaderParameters);

			for (auto& e : shaderParameters) {
				pipeline->AddMemberDeductionGuide(shaderScope, e.Name, { pushConstantBlockHandle, shaderParametersHandle, pipeline->GetElementHandle(shaderParametersStructHandle, e.Name) });
			}

		}

		auto& batch = batches.EmplaceBack();

		batch.Tags = GetTagList();
		batch.Scopes.EmplaceBack(GPipeline::ElementHandle());

		CommonPermutation* common_permutation = Find<CommonPermutation>(u8"CommonPermutation", hierarchy);
		batch.Scopes.EmplaceBack(common_permutation->commonScope);
		batch.Scopes.EmplaceBack(visibilityHandle);
		batch.Scopes.EmplaceBack(visibilityPass);

		switch (Hash(shader_json[u8"class"])) {
		case GTSL::Hash(u8"Vertex"): {
			batch.TargetSemantics = GAL::ShaderType::VERTEX;
			batch.Scopes.EmplaceBack(common_permutation->vertexShaderScope);
			batch.Scopes.EmplaceBack(shaderScope);

			tokenizeCode(u8"vertexSurfaceInterface.instanceIndex = gl_InstanceIndex; vertexSurfaceInterface.triangleIndex = gl_VertexIndex;", main.Tokens);
			tokenizeCode(u8"vertexSurfaceInterface.worldSpacePosition = vec3f(GetInstancePosition() * GetVertexPosition()); vertexSurfaceInterface.worldSpaceNormal = vec3f(GetInstancePosition() * GetVertexNormal());", main.Tokens, GetPersistentAllocator());
			tokenizeCode(shader_json[u8"code"], main.Tokens, GetPersistentAllocator());

			break;
		}
		case GTSL::Hash(u8"Surface"): {
			batch.TargetSemantics = GAL::ShaderType::FRAGMENT;
			batch.Scopes.EmplaceBack(common_permutation->fragmentShaderScope);
			batch.Scopes.EmplaceBack(shaderScope);

			tokenizeCode(shader_json[u8"code"], main.Tokens, GetPersistentAllocator());

			batches.PopBack();

			break;
		}
		default: {
			batches.PopBack(); //remove added batch as no shader was created
			BE_LOG_ERROR(u8"Can't utilize this shader class in this domain.")
		}
		}
	}

	GPipeline::ElementHandle visibilityHandle, pushConstantBlockHandle, shaderParametersHandle;
	GPipeline::ElementHandle visibilityPass, countPixelsPass, prefixSumPass, selectPixelsPass, paintPass;
};