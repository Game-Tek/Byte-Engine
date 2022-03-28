#pragma once

#include "PermutationManager.hpp"

inline std::tuple<GPipeline::ElementHandle, GPipeline::ElementHandle> DeclareShader(GPipeline* pipeline, GTSL::StaticVector<PermutationManager::ShaderPermutation, 8>& s, GTSL::StringView name, GTSL::Range<const PermutationManager::ShaderTag*> tags, GAL::ShaderType target_semantics) {
	auto shaderHandle = pipeline->DeclareShader({}, name);

	PermutationManager::ShaderPermutation& perm = s.EmplaceBack();
	perm.Name = name;
	perm.Tags = tags;
	perm.TargetSemantics = target_semantics;
	perm.Scopes.EmplaceBack(shaderHandle);

	return { shaderHandle, pipeline->DeclareFunction(shaderHandle, u8"void", u8"main") };
}

class UIPermutation : public PermutationManager {
public:
	UIPermutation(const GTSL::StringView instance_name) : PermutationManager(instance_name, u8"UIPermutation") {
		//AddTag(u8"RenderTechnique", u8"Forward");

		//AddSupportedDomain(u8"UI");
	}

	void Initialize(GPipeline* pipeline, ShaderGenerationData& shader_generation_data) override {
		uiScope = pipeline->DeclareScope(GPipeline::GLOBAL_SCOPE, u8"UI");

		// No vertex declaration as we have no incoming data
		AddVertexSurfaceInterfaceBlockDeclaration(pipeline, uiScope, { { u8"vec2f", u8"vertexPos" }, { u8"vec2f", u8"vertexUV" } });
		AddPushConstantDeclaration(pipeline, uiScope, { { u8"GlobalData*", u8"global" }, { u8"RenderPassData*", u8"renderPass" } });
		pipeline->DeclareStruct(uiScope, u8"RenderPassData", { { u8"TextureReference", u8"color" } });

		auto shaderHandle = pipeline->DeclareShader(uiScope, u8"UIVertex");
		auto mainFunctionHandle = pipeline->DeclareFunction(shaderHandle, u8"void", u8"main");
		pipeline->AddCodeToFunction(mainFunctionHandle, u8"float32 x = float32(((uint32(gl_VertexIndex) + 2) / 3) % 2); float32 y = float32(((uint32(gl_VertexIndex) + 1) / 3) % 2); vertexSurfaceInterface.vertexUV = vec2f(x, y); vertexSurfaceInterface.vertexPos = vec2f(-1.0f + x * 2.0f, -1.0f + y * 2.0f); vertexPosition = vec4f(vertexSurfaceInterface.vertexPos, 0, 1);");
		vertexShaderHandle = shaderHandle;
	}

	GTSL::Vector<ShaderGroupDescriptor, BE::TAR> MakeShaderGroups(GPipeline* pipeline, GTSL::Range<const PermutationManager**> hierarchy) override {
		GTSL::Vector<ShaderGroupDescriptor, BE::TAR> results(4, GetTransientAllocator());

		auto* commonPermutation = Find<CommonPermutation>(u8"CommonPermutation", hierarchy);

		auto& res = results.EmplaceBack();
		
		res.ShaderGroupJSON = u8"{ \"name\":\"UI\", \"instances\":[{ \"name\":\"unique\", \"parameters\":[] }], \"domain\":\"Screen\" }";
		auto& vsp = res.Shaders.EmplaceBack();
		auto& vs = vsp.EmplaceBack();
		vs.Name = u8"UIVertex"; vs.Tags.EmplaceBack(u8"Domain", u8"UI"); vs.TargetSemantics = GAL::ShaderType::VERTEX; vs.Scopes.EmplaceBack(GPipeline::GLOBAL_SCOPE); vs.Scopes.EmplaceBack(uiScope); vs.Scopes.EmplaceBack(commonPermutation->commonScope); vs.Scopes.EmplaceBack(vertexShaderHandle); vs.Scopes.EmplaceBack(commonPermutation->vertexShaderScope);

		{
			auto fragmentShaderHandle = pipeline->DeclareShader(uiScope, u8"UIFragment"); auto mainFunctionHandle = pipeline->DeclareFunction(fragmentShaderHandle, u8"void", u8"main");

			pipeline->AddCodeToFunction(mainFunctionHandle, u8"surfaceColor = vec4f(1);");

			auto& fsp = res.Shaders.EmplaceBack();
			auto& fs = fsp.EmplaceBack();
			fs.Name = u8"UIFragment"; fs.Tags.EmplaceBack(u8"Domain", u8"UI"); fs.TargetSemantics = GAL::ShaderType::FRAGMENT; fs.Scopes.EmplaceBack(GPipeline::GLOBAL_SCOPE); fs.Scopes.EmplaceBack(uiScope); fs.Scopes.EmplaceBack(commonPermutation->commonScope); fs.Scopes.EmplaceBack(fragmentShaderHandle); fs.Scopes.EmplaceBack(commonPermutation->fragmentShaderScope);
		}

		return results;
	}

	void ProcessShader(GPipeline* pipeline, GTSL::JSONMember shader_group_json, GTSL::JSONMember shader_json, const GTSL::Range<const PermutationManager**> hierarchy, GTSL::StaticVector<ShaderPermutation, 8>& batches) override {
		auto [shaderHandle, mainFunctionHandle] = DeclareShader(pipeline, batches, shader_json[u8"name"], {}, GAL::ShaderType::FRAGMENT);
		pipeline->AddCodeToFunction(mainFunctionHandle, shader_json[u8"code"]);
	}

private:
	GPipeline::ElementHandle uiScope, vertexShaderHandle;
};