#pragma once

#include "PermutationManager.hpp"
#include "ByteEngine/Render/Culling.h"

struct VisibilityRenderPassPermutation : PermutationManager {
	VisibilityRenderPassPermutation(const GTSL::StringView instance_name) : PermutationManager(instance_name, u8"VisibilityRenderPassPermutation") {
		AddTag(u8"RenderTechnique", u8"Visibility");

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
		pipeline->SetMakeStruct(pipeline->DeclareStruct(visibilityHandle, u8"Derivatives", { { u8"vec3f", u8"db_dx" }, { u8"vec3f", u8"db_dy" } }));
		pipeline->DeclareFunction(visibilityHandle, u8"BarycentricDeriv", u8"CalcFullBary", { { u8"vec4f", u8"pt0" }, { u8"vec4f", u8"pt1" }, { u8"vec4f", u8"pt2" }, { u8"vec2f", u8"pixelNdc" }, { u8"vec2f", u8"winSize" } }, u8"BarycentricDeriv ret; vec3f invW = vec3f(1) / vec3f(pt0.w, pt1.w, pt2.w); vec2f ndc0 = pt0.xy * invW.x; vec2f ndc1 = pt1.xy * invW.y; vec2f ndc2 = pt2.xy * invW.z; float32 invDet = 1.0f / determinant(mat2f(ndc2 - ndc1, ndc0 - ndc1)); ret.m_ddx = vec3f(ndc1.y - ndc2.y, ndc2.y - ndc0.y, ndc0.y - ndc1.y) * invDet; ret.m_ddy = vec3f(ndc2.x - ndc1.x, ndc0.x - ndc2.x, ndc1.x - ndc0.x) * invDet; vec2f deltaVec = pixelNdc - ndc0; float32 interpInvW = (invW.x + deltaVec.x * dot(invW, ret.m_ddx) + deltaVec.y * dot(invW, ret.m_ddy)); float32 interpW = 1.0f / interpInvW; ret.m_lambda.x = interpW * (invW[0] + deltaVec.x * ret.m_ddx.x * invW[0] + deltaVec.y * ret.m_ddy.x * invW[0]); ret.m_lambda.y = interpW * (0.0f + deltaVec.x * ret.m_ddx.y * invW[1] + deltaVec.y * ret.m_ddy.y * invW[1]); ret.m_lambda.z = interpW * (0.0f + deltaVec.x * ret.m_ddx.z * invW[2] + deltaVec.y * ret.m_ddy.z * invW[2]); ret.m_ddx *= (2.0f / winSize.x); ret.m_ddy *= (2.0f / winSize.y); ret.m_ddy *= -1.0f; return ret;");
		pipeline->DeclareFunction(visibilityHandle, u8"vec3f", u8"InterpolateWithDeriv", { { u8"BarycentricDeriv", u8"deriv" }, { u8"vec3f", u8"mergedV" } }, u8"vec3f ret; ret.x = dot(deriv.m_lambda, mergedV); ret.y = dot(deriv.m_ddx * mergedV, vec3f(1, 1, 1)); ret.z = dot(deriv.m_ddy * mergedV, vec3f(1, 1, 1)); return ret;");

		pipeline->DeclareFunction(visibilityHandle, u8"Derivatives", u8"ComputePartialDerivatives", { { u8"vec2f[3]", u8"v" } }, u8"Derivatives result; float32 d = 1.0f / determinant(mat2f(v[2] - v[1], v[0] - v[1])); result.db_dx = vec3f(v[1].y - v[2].y, v[2].y - v[0].y, v[0].y - v[1].y) * d; result.db_dy = vec3f(v[2].x - v[1].x, v[0].x - v[2].x, v[1].x - v[0].x) * d; return result;");

		pipeline->DeclareFunction(visibilityHandle, u8"float32", u8"InterpolateAttribute", { { u8"vec3f", u8"attributes" }, { u8"vec3f", u8"db_dx" }, { u8"vec3f", u8"db_dy" }, { u8"vec2f", u8"d" }	}, u8"float attribute_x = dot(attributes, db_dx); float attribute_y = dot(attributes, db_dy); float attribute_s = attributes[0]; return (attribute_s + d.x * attribute_x + d.y * attribute_y);");

		pipeline->DeclareFunction(visibilityHandle, u8"vec3f", u8"InterpolateAttribute", { { u8"mat3f", u8"attributes" }, { u8"vec3f", u8"db_dx" }, { u8"vec3f", u8"db_dy" }, { u8"vec2f", u8"d" }	}, u8"vec3f attribute_x = db_dx * attributes; vec3f attribute_y = db_dy * attributes; vec3f attribute_s = attributes[0]; return (attribute_s + d.x * attribute_x + d.y * attribute_y);");

		pipeline->SetMakeStruct(pipeline->DeclareStruct(visibilityHandle, u8"PointLightData", { { u8"vec3f", u8"position" }, {u8"float32", u8"radius"} }));
		pipeline->DeclareStruct(visibilityHandle, u8"LightingData", { {u8"uint32", u8"pointLightsLength"},  {u8"PointLightData[4]", u8"pointLights"} });

		pipeline->DeclareStruct(visibilityHandle, u8"VisibilityData", { { u8"vec3f*", u8"positionStream" }, { u8"vec3f*", u8"normalStream" }, { u8"vec3f*", u8"tangentStream" }, { u8"vec3f*", u8"bitangentStream" }, { u8"vec2f*", u8"textureCoordinatesStream" }, {u8"uint32", u8"shaderGroupLength"},  {u8"uint32[256]", u8"shaderGroupUseCount"}, {u8"uint32[256]", u8"shaderGroupStart" } , { u8"IndirectDispatchCommand[256]", u8"indirectBuffer" }, {u8"vec2s*", u8"pixelBuffer"}});

		simplePushConstant = pipeline->DeclareScope(visibilityHandle, u8"pushConstantBlock");
		pipeline->DeclareVariable(simplePushConstant, { u8"GlobalData*", u8"global" });
		pipeline->DeclareVariable(simplePushConstant, { u8"renderPassData*", u8"renderPass" });
		auto instancesPointerHandle = pipeline->DeclareVariable(simplePushConstant, { u8"InstanceData*", u8"instances" });
		auto visibilityDataHandle = pipeline->DeclareVariable(simplePushConstant, { u8"VisibilityData*", u8"visibility" });

		{ //visibility pass
			visibilityPass = pipeline->DeclareScope(shader_generation_data.Scopes.back(), u8"VisibilityPass");

			pipeline->DeclareFunction(visibilityHandle, u8"mat4f", u8"GetInstancePosition", {}, u8"return mat4(pushConstantBlock.instances[gl_InstanceIndex].ModelMatrix);");
			pipeline->DeclareFunction(visibilityHandle, u8"uint32", u8"GetVertexIndex", {}, u8"return gl_VertexIndex;");

			auto vertexBlock = pipeline->DeclareScope(visibilityPass, u8"vertex");
			pipeline->DeclareVariable(vertexBlock, { u8"vec3f", u8"POSITION" });
			pipeline->DeclareVariable(vertexBlock, { u8"vec3f", u8"NORMAL" });
			pipeline->DeclareVariable(vertexBlock, { u8"vec3f", u8"TANGENT" });
			pipeline->DeclareVariable(vertexBlock, { u8"vec3f", u8"BITANGENT" });
			pipeline->DeclareVariable(vertexBlock, { u8"vec2f", u8"TEXTURE_COORDINATES" });
		}

		{ //count pixels pass
			countShaderGroupsShader = pipeline->DeclareShader(shader_generation_data.Scopes.back(), u8"CountShaderGroups");

			pipeline->AddMemberDeductionGuide(countShaderGroupsShader, u8"visibility", { simplePushConstant, visibilityDataHandle, });
			pipeline->AddMemberDeductionGuide(countShaderGroupsShader, u8"instances", { simplePushConstant, instancesPointerHandle });

			// Count how many pixels contain each shader group
			pipeline->DeclareFunction(countShaderGroupsShader, u8"void", u8"main", {}, u8"uint32 shaderGroupIndex = instances[SampleUint(pushConstantBlock.renderPass.visibility).r].shaderGroupIndex; atomicAdd(visibility.shaderGroupUseCount[shaderGroupIndex].a, 1);");
			//execution = windowExtent
		}

		{ //prefix sum pass
			prefixSumShader = pipeline->DeclareShader(shader_generation_data.Scopes.back(), u8"PrefixSum");

			pipeline->DeclareFunction(prefixSumShader, u8"void", u8"main", {}, u8"uint32 sum = 0; for(uint32 i = 0; i < pushConstantBlock.visibility.shaderGroupLength; ++i) { pushConstantBlock.visibility.shaderGroupStart[i].a = sum; sum += pushConstantBlock.visibility.shaderGroupUseCount[i].a; pushConstantBlock.visibility.indirectBuffer[i].width = pushConstantBlock.visibility.shaderGroupUseCount[i].a; }");
		}

		{ //select pixels pass
			buildPixelBufferShader = pipeline->DeclareShader(shader_generation_data.Scopes.back(), u8"BuildPixelBuffer");
			
			pipeline->AddMemberDeductionGuide(buildPixelBufferShader, u8"visibility", { simplePushConstant, visibilityDataHandle, });

			// For every pixel on the screen determine which shader group is visible and append the current pixel coordinate to a per shader group list of pixels that need to be shaded
			pipeline->DeclareFunction(buildPixelBufferShader, u8"void", u8"main", {}, u8"visibility.pixelBuffer[atomicAdd(visibility.shaderGroupStart[pushConstantBlock.instances[SampleUint(pushConstantBlock.renderPass.visibility).r].shaderGroupIndex].a, 1)] = vec2s(GetGlobalIndex());");
			//execution = windowExtent
		}

		{ //paint pass
			paintPass = pipeline->DeclareScope(shader_generation_data.Scopes.back(), u8"PaintPass");

			paintPushConstant = pipeline->DeclareScope(paintPass, u8"pushConstantBlock");
			pipeline->DeclareVariable(paintPushConstant, { u8"GlobalData*", u8"global" });
			pipeline->DeclareVariable(paintPushConstant, { u8"CameraData*", u8"camera" });
			pipeline->DeclareVariable(paintPushConstant, { u8"renderPassData*", u8"renderPass" });
			pipeline->DeclareVariable(paintPushConstant, { u8"LightingData*", u8"lightingData" });
			pipeline->DeclareVariable(paintPushConstant, { u8"InstanceData*", u8"instances" });

			pipeline->DeclareFunction(paintPass, u8"vec4f", u8"RandomColorFromUint", { { u8"uint32", u8"index" } }, u8"vec3f table[8] = vec3f[8](vec3f(0, 0.9, 0.4), vec3f(0, 0.2, 0.9), vec3f(1, 0.3, 1), vec3f(0.1, 0, 0.9), vec3f(1, 0.5, 0.1), vec3f(0.5, 0.4, 0.4), vec3f(1, 1, 0), vec3f(1, 0, 0)); return vec4f(table[index % 8], 1);");
		}

		const CommonPermutation* common_permutation = Find<CommonPermutation>(u8"CommonPermutation", shader_generation_data.Hierarchy);

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

	GTSL::Vector<ShaderGroupDescriptor, BE::TAR> MakeShaderGroups(GPipeline* pipeline, GTSL::Range<const PermutationManager**> hierarchy) override {
		GTSL::Vector<ShaderGroupDescriptor, BE::TAR> results(8, GetTransientAllocator());

		{ //visibility
			auto& sg = results.EmplaceBack();
			sg.ShaderGroupJSON = 
u8R"({
    "name":"VisibilityShaderGroup",
    "instances":[{"name":"Visibility", "parameters":[]}],
    "domain":"Visibility"
})";
		}

		{ //count shader groups
			auto& result = results.EmplaceBack();
			result.ShaderGroupJSON =
				u8R"({
    "name":"CountShaderGroups",
    "instances":[{"name":"Count", "parameters":[]}],
    "domain":"Visibility"
})";
		}

		{ //prefix sum
			auto& result = results.EmplaceBack();
			result.ShaderGroupJSON =
				u8R"({
    "name":"PrefixSum",
    "instances":[{"name":"PrefixSum", "parameters":[]}],
    "domain":"Visibility"
})";
		}

		{ //build pixel buffer
			auto& result = results.EmplaceBack();
			result.ShaderGroupJSON =
				u8R"({
    "name":"BuildPixelBuffer",
    "instances":[{"name":"BuildPixelBuffer", "parameters":[]}],
    "domain":"Visibility"
})";
		}

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

		auto shaderScope = pipeline->DeclareShader(visibilityHandle, shader_json[u8"name"]);
		auto mainFunctionHandle = pipeline->DeclareFunction(shaderScope, u8"void", u8"main");

		{ //add deduction guides for reaching shader parameters
			auto shaderParametersStructHandle = pipeline->DeclareStruct(shaderScope, u8"shaderParametersData", shaderParameters);

			for (auto& e : shaderParameters) {
				//															simplePushConstant will work under all passes, but it's nn ot the correct way to do this
				pipeline->AddMemberDeductionGuide(shaderScope, e.Name, { simplePushConstant, shaderParametersHandle, pipeline->GetElementHandle(shaderParametersStructHandle, e.Name) });
			}

		}

		auto& main = pipeline->GetFunction({ shaderScope }, u8"main");

		switch (Hash(shader_group_json[u8"domain"])) {
		case GTSL::Hash(u8"World"): {
			auto& batch = batches.EmplaceBack();

			batch.Tags = GetTagList();
			batch.Scopes.EmplaceBack(GPipeline::GLOBAL_SCOPE);

			const CommonPermutation* common_permutation = Find<CommonPermutation>(u8"CommonPermutation", hierarchy);
			batch.Scopes.EmplaceBack(common_permutation->commonScope);
			batch.Scopes.EmplaceBack(visibilityHandle);
			batch.Scopes.EmplaceBack(visibilityPass);

			switch (Hash(shader_json[u8"class"])) {
			case GTSL::Hash(u8"Vertex"): {
				batch.TargetSemantics = GAL::ShaderType::VERTEX;
				batch.Scopes.EmplaceBack(common_permutation->vertexShaderScope);
				batch.Scopes.EmplaceBack(shaderScope);

				tokenizeCode(u8"instanceIndex = gl_InstanceIndex; triangleIndex = gl_VertexIndex / 3;", main.Tokens);
				tokenizeCode(u8"worldSpacePosition = vec3f(GetInstancePosition() * GetVertexPosition()); worldSpaceNormal = vec3f(GetInstancePosition() * GetVertexNormal());", main.Tokens, GetPersistentAllocator());
				tokenizeCode(shader_json[u8"code"], main.Tokens, GetPersistentAllocator());
				break;
			}
			case GTSL::Hash(u8"Surface"): {
				batch.TargetSemantics = GAL::ShaderType::COMPUTE;
				batch.Scopes.EmplaceBack(common_permutation->computeShaderScope);
				batch.Scopes.EmplaceBack(shaderScope);
				
				tokenizeCode(u8"float32 surfaceRoughness = 1.0f; vec4f surfaceNormal = vec4f(0, 0, -1, 0); vec4f surfaceColor = vec4f(0);", main.Tokens, GetPersistentAllocator());
				tokenizeCode(u8"vec4u pixel = SampleUint(pushConstantBlock.renderPass.Visibility, GetPixelPosition()); uint32 instanceIndex = pixel.r; uint32 triangleIndex = pixel.g;", main.Tokens);

				tokenizeCode(u8"instanceData* instance = pushConstantBlock.instances[instanceIndex];", main.Tokens);
				tokenizeCode(u8"u16vec3 indices = index*(pushConstantBlock.visibility.indexBuffer + instance.indexBufferOffset)[triangleIndex].indexTri; vec3f* vertices = pushConstantBlock.visibility.positionStream + instance.vertexBufferOffset; vec3f pos[3] = vec3f[3](vertices[indices[0]].xyz, vertices[indices[1]].xyz, vertices[indices[2]].xyz);", main.Tokens);
				tokenizeCode(u8"mat4f mvp = instance.matrix * pushConstantBlock.camera.vp;", main.Tokens); // Calculate MVP matrix
				tokenizeCode(u8"vec4f positions[3] = vec4f[3](mvp * float4(pos[0], 1.0f), mvp * float4(pos[1], 1.0f), mvp * float4(pos[2], 1.0f));", main.Tokens); // Transform positions to clip space
				tokenizeCode(u8"vec3f oneOverW = vec3f(1.0f) / vec3f(positions[0].w, positions[1].w, positions[2].w);", main.Tokens); // Calculate the inverse of w, since it's going to be used several times
				tokenizeCode(u8"positions[0] *= oneOverW[0]; positions[1] *= oneOverW[1]; positions[2] *= oneOverW[2];", main.Tokens); // Project vertex positions to calculate 2D post-perspective positions
				tokenizeCode(u8"vec2f screenPosition[3] = vec2f[3](positions[0].xy, positions[1].xy, positions[2].xy);", main.Tokens);
				tokenizeCode(u8"Derivatives derivativesOut = ComputePartialDerivatives(screenPosition);", main.Tokens); // Compute partial derivatives. This is necessary to interpolate triangle attributes per pixel.
				tokenizeCode(u8"vec2f d = vec2f(GetNormalizedGlobalIndex()) + -screenPosition[0];", main.Tokens); // Calculate delta vector (d) that points from the projected vertex 0 to the current screen point
				tokenizeCode(u8"float32 w = 1.0f / InterpolateAttribute(oneOverW, derivativesOut.db_dx, derivativesOut.db_dy, d);", main.Tokens); // Interpolate the 1/w (one_over_w) for all three vertices of the triangle using the barycentric coordinates and the delta vector
				tokenizeCode(u8"float z = w * getElem(Get(transform)[VIEW_CAMERA].projection, 2, 2) + getElem(Get(transform)[VIEW_CAMERA].projection, 3, 2);", main.Tokens); // Reconstruct the Z value at this screen point performing only the necessary matrix * vector multiplication operations that involve computing Z

				//tokenizeCode(shader_json[u8"code"], main.Tokens, GetPersistentAllocator());
				//tokenizeCode(u8"vec4f BE_COLOR_0 = surfaceColor; surfaceColor = vec4f(0); for(uint32 i = 0; i < pushConstantBlock.lightingData.pointLightsLength; ++i) { PointLightData l = pushConstantBlock.lightingData.pointLights[i]; surfaceColor += vec4f(light(l.position, GetCameraPosition(), //GetSurfaceWorldSpacePosition(), GetSurfaceWorldSpaceNormal(), vec3f(1) * l.radius, normalize(GetCameraPosition() - GetSurfaceWorldSpacePosition()), vec3f(BE_COLOR_0), vec3f(0.04f), surfaceRoughness), 0.1); }", main.Tokens, GetPersistentAllocator());
				tokenizeCode(u8"Write(pushConstantBlock.renderPass.Color, pushConstantBlock.visibility.pixelBuffer[GetGlobalIndex().x].hw, RandomColorFromUint(triangleIndex));", main.Tokens);
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

	void ProcessVisibility(GPipeline* pipeline, GTSL::JSONMember shader_group_json, GTSL::JSONMember shader_json, GTSL::Range<const PermutationManager**> hierarchy, GTSL::StaticVector<ShaderPermutation, 8>& batches) {
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
				pipeline->AddMemberDeductionGuide(shaderScope, e.Name, { simplePushConstant, shaderParametersHandle, pipeline->GetElementHandle(shaderParametersStructHandle, e.Name) });
			}

		}

		auto& batch = batches.EmplaceBack();

		batch.Tags = GetTagList();
		batch.Scopes.EmplaceBack(GPipeline::GLOBAL_SCOPE);

		const CommonPermutation* common_permutation = Find<CommonPermutation>(u8"CommonPermutation", hierarchy);
		batch.Scopes.EmplaceBack(common_permutation->commonScope);
		batch.Scopes.EmplaceBack(visibilityHandle);
		batch.Scopes.EmplaceBack(visibilityPass);

		switch (Hash(shader_json[u8"class"])) {
		case GTSL::Hash(u8"Vertex"): {
			batch.TargetSemantics = GAL::ShaderType::VERTEX;
			batch.Scopes.EmplaceBack(common_permutation->vertexShaderScope);
			batch.Scopes.EmplaceBack(shaderScope);

			tokenizeCode(u8"instanceIndex = gl_InstanceIndex; triangleIndex = gl_VertexIndex;", main.Tokens);
			tokenizeCode(u8"worldSpacePosition = vec3f(GetInstancePosition() * GetVertexPosition()); worldSpaceNormal = vec3f(GetInstancePosition() * GetVertexNormal());", main.Tokens, GetPersistentAllocator());
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

	GPipeline::ElementHandle visibilityHandle, simplePushConstant, paintPushConstant, shaderParametersHandle;
	GPipeline::ElementHandle visibilityPass, countShaderGroupsShader, prefixSumShader, buildPixelBufferShader, paintPass;
};

inline AABB2 Make2DAABBForAABB(AABB aabb, GTSL::Matrix4& mat) {
	GTSL::Vector3 vertices[8]{ aabb };

	//back plane
	vertices[0][0] *= 1;

	vertices[1][1] *= -1;

	vertices[2][0] *= -1;
	vertices[2][1] *= -1;

	vertices[3][0] *= -1;

	//front plane
	vertices[0][0] *= 1;

	vertices[1][1] *= -1;

	vertices[2][0] *= -1;
	vertices[2][1] *= -1;

	vertices[3][0] *= -1;

	vertices[4][2] *= -1;
	vertices[5][2] *= -1;
	vertices[6][2] *= -1;
	vertices[7][2] *= -1;

	float32 maxMagnitude = 0.0f;
	GTSL::Vector3 res;

	for(uint32 i = 0; i < 8; ++i) {
		auto vec = mat * aabb;

		if (auto mag = GTSL::Math::Length(vec); mag > maxMagnitude) {
			maxMagnitude = mag;
			res = vec;
		}
	}

	return { res[0], res[1] };
}