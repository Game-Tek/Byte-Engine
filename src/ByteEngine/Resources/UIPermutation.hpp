#pragma once

#include "PermutationManager.hpp"
#include "ByteEngine/Render/ShaderGenerator.h"

inline std::tuple<GPipeline::ElementHandle, GPipeline::ElementHandle> DeclareShader(GPipeline* pipeline, GTSL::StaticVector<PermutationManager::ShaderPermutation, 8>& s, GTSL::StringView name, GTSL::Range<const PermutationManager::ShaderTag*> tags, GAL::ShaderType target_semantics) {
	auto shaderHandle = pipeline->DeclareShader({}, name);

	PermutationManager::ShaderPermutation& perm = s.EmplaceBack();
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

		pipeline->DeclareStruct(uiScope, u8"TextData", UI_TEXT_DATA);
		pipeline->DeclareStruct(uiScope, u8"LinearSegment", UI_LINEAR_SEGMENT);
		pipeline->DeclareStruct(uiScope, u8"QuadraticSegment", UI_QUADRATIC_SEGMENT);
		pipeline->DeclareStruct(uiScope, u8"GlyphContourData", UI_GLYPH_CONTOUR_DATA);
		pipeline->DeclareStruct(uiScope, u8"GlyphData", UI_GLYPH_DATA);
		pipeline->DeclareStruct(uiScope, u8"FontData", UI_FONT_DATA);
		pipeline->DeclareStruct(uiScope, u8"UIData", UI_DATA);
		pipeline->DeclareStruct(uiScope, u8"UIInstanceData", UI_INSTANCE_DATA);
		pipeline->DeclareStruct(uiScope, u8"UIRes", UI_RES);

		// No vertex declaration as we have no incoming data
		//AddVertexSurfaceInterfaceBlockDeclaration(pipeline, uiScope, { {u8"vec2f", u8"vertexPos"}, {u8"vec2f", u8"vertexUV"}, { u8"int", u8"instanceIndex" }, { u8"float32", u8"r" } });
		AddVertexSurfaceInterfaceBlockDeclaration(pipeline, uiScope, { {u8"vec2f", u8"vertexPos"}, {u8"vec2f", u8"vertexUV"}, {u8"uint32", u8"instanceIndex"} });
		//AddVertexSurfaceInterfaceBlockDeclaration1(pipeline, uiScope, {  });
		AddPushConstantDeclaration(pipeline, uiScope, { { u8"GlobalData*", u8"global" }, { u8"RenderPassData*", u8"renderPass" }, { u8"UIData*", u8"ui"}, { u8"UIInstanceData*", u8"uiInstances" }});
		uiRenderPassScopeHandle = AddRenderPassDeclaration(pipeline, u8"UIRenderPass", { { u8"TextureReference", u8"Color" } });

		shader_generation_data.Scopes.EmplaceBack(uiRenderPassScopeHandle);

		auto shaderHandle = pipeline->DeclareShader(uiScope, u8"UIVertex");
		auto mainFunctionHandle = pipeline->DeclareFunction(shaderHandle, u8"void", u8"main");
		//pipeline->AddCodeToFunction(mainFunctionHandle, u8"UIInstanceData* instance = pushConstantBlock.uiInstances[gl_InstanceIndex];");
		//pipeline->AddCodeToFunction(mainFunctionHandle, u8"float32 x = float32(((uint32(gl_VertexIndex) + 2) / 3) % 2); float32 y = float32(((uint32(gl_VertexIndex) + 1) / 3) % 2);");
		//pipeline->AddCodeToFunction(mainFunctionHandle, u8"vertexSurfaceInterface.vertexUV = vec2f(-1.0f + x * 2.0f, -1.0f + y * 2.0f); vertexSurfaceInterface.vertexPos = vec2f(-1.0f + x * 2.0f, -1.0f + y * 2.0f); vertexPosition = pushConstantBlock.ui.projection * vec4f(instance.transform * vec4f(vertexSurfaceInterface.vertexPos, 0.0f, 1.0f), 1.0f);");
		//pipeline->AddCodeToFunction(mainFunctionHandle, u8"vertexSurfaceInterface.instanceIndex = uint32(gl_InstanceIndex);");

		pipeline->AddCodeToFunction(mainFunctionHandle, u8"UIInstanceData* instance = pushConstantBlock.uiInstances[gl_InstanceIndex];");
		pipeline->AddCodeToFunction(mainFunctionHandle, u8"float32 x = float32(((uint32(gl_VertexIndex) + 1) / 3) % 2); float32 y = float32(((uint32(gl_VertexIndex) + 2) / 3) % 2);");
		//pipeline->AddCodeToFunction(mainFunctionHandle, u8"float32 x = float32(((uint32(gl_VertexIndex) + 2) / 3) % 2); float32 y = float32(((uint32(gl_VertexIndex) + 1) / 3) % 2);");
		pipeline->AddCodeToFunction(mainFunctionHandle, u8"vertexUV = vec2f(-1.0f + x * 2.0f, -1.0f + y * 2.0f); vertexPos = vec2f(-1.0f + x * 2.0f, -1.0f + y * 2.0f); vertexPosition = pushConstantBlock.ui.projection * vec4f(instance.transform * vec4f(vertexPos, 0.0f, 1.0f), 1.0f);");
		pipeline->AddCodeToFunction(mainFunctionHandle, u8"instanceIndex = uint32(gl_InstanceIndex);");

		vertexShaderHandle = shaderHandle;

		auto* commonPermutation = Find<CommonPermutation>(u8"CommonPermutation", shader_generation_data.Hierarchy);

		AddScope(GPipeline::GLOBAL_SCOPE);
		AddScope(uiScope);
		AddScope(commonPermutation->commonScope);
		AddScope(uiRenderPassScopeHandle);
	}

	GTSL::Vector<ShaderGroupDescriptor, BE::TAR> MakeShaderGroups(GPipeline* pipeline, GTSL::Range<const PermutationManager**> hierarchy) override {
		GTSL::Vector<ShaderGroupDescriptor, BE::TAR> results(4, GetTransientAllocator());

		auto* commonPermutation = Find<CommonPermutation>(u8"CommonPermutation", hierarchy);

		auto& basicUIShaderGroup = results.EmplaceBack();

		basicUIShaderGroup.ShaderGroupJSON = u8"{ \"name\":\"UI\", \"instances\":[{ \"name\":\"UI\", \"parameters\":[] }], \"domain\":\"Screen\", \"tags\":[{ \"name\":\"RenderPass\", \"value\":\"UIRenderPass\" }, { \"name\":\"Transparency\", \"value\":\"true\" }] }";

		{
			auto& vsp = basicUIShaderGroup.Shaders.EmplaceBack();
			auto& vs = vsp.EmplaceBack();
			vs.Tags.EmplaceBack(u8"Domain", u8"UI");
			vs.Scopes.EmplaceBack(commonPermutation->vertexShaderScope);
			vs.Scopes.EmplaceBack(vertexShaderHandle);
			vs.TargetSemantics = GAL::ShaderType::VERTEX;

		}

		{
			auto fragmentShaderHandle = pipeline->DeclareShader(uiScope, u8"UIFragment"); auto mainFunctionHandle = pipeline->DeclareFunction(fragmentShaderHandle, u8"void", u8"main");

			AddSurfaceShaderOutDeclaration(pipeline, fragmentShaderHandle, { { u8"vec4f", u8"surfaceColor" } });

			//pipeline->AddCodeToFunction(mainFunctionHandle, u8"UIInstanceData* instance = pushConstantBlock.uiInstances[vertexSurfaceInterface1.instanceIndex];");
			//pipeline->AddCodeToFunction(mainFunctionHandle, u8"float roundness = instance.roundness; float aspectRatio = instance.transform[1][1] / instance.transform[0][0];");
			//pipeline->AddCodeToFunction(mainFunctionHandle, u8"vec2 d = abs(vertexSurfaceInterface.vertexUV) - vec2((1.0f - roundness * aspectRatio), 1.0f - roundness); d.x /= aspectRatio; float distance = length(max(d,0.0f)) + min(max(d.x,d.y),0.0f) - roundness; float32 alpha = clamp(-distance / fwidth(distance), 0.0f, 1.0f); surfaceColor = vec4f(vec3(instance.color), instance.color.a * alpha);");

			pipeline->AddCodeToFunction(mainFunctionHandle, u8"UIInstanceData* instance = pushConstantBlock.uiInstances[instanceIndex];");
			pipeline->AddCodeToFunction(mainFunctionHandle, u8"float roundness = instance.roundness; float aspectRatio = instance.transform[1][1] / instance.transform[0][0];");
			pipeline->AddCodeToFunction(mainFunctionHandle, u8"vec2 d = abs(vertexUV) - vec2((1.0f - roundness * aspectRatio), 1.0f - roundness); d.x /= aspectRatio; float distance = length(max(d,0.0f)) + min(max(d.x,d.y),0.0f) - roundness; float32 alpha = clamp(-distance / fwidth(distance), 0.0f, 1.0f); surfaceColor = vec4f(vec3(instance.color), instance.color.a * alpha);");

			auto& fsp = basicUIShaderGroup.Shaders.EmplaceBack();
			auto& fs = fsp.EmplaceBack();
			fs.Tags.EmplaceBack(u8"Domain", u8"UI"); fs.TargetSemantics = GAL::ShaderType::FRAGMENT;
			fs.Scopes.EmplaceBack(commonPermutation->fragmentShaderScope);
			fs.Scopes.EmplaceBack(fragmentShaderHandle);
		}

		{
			auto closestPointOnLineSegmentToPointHandle = pipeline->DeclareFunction(uiScope, u8"vec2f", u8"ClosestPointOnLineSegmentToPoint", { { u8"vec2f", u8"a" }, { u8"vec2f", u8"b" }, { u8"vec2f", u8"p" } }, u8"const vec2f AB = b - a; float32 t = dot(AB, p - a) / dot(AB, AB); return a + AB * clamp(t, 0.0f, 1.0f);");

			auto testPointToLineSideFunctionHandle = pipeline->DeclareFunction(uiScope, u8"float32", u8"TestPointToLineSide", { { u8"vec2f", u8"a" }, { u8"vec2f", u8"b" }, { u8"vec2f", u8"p" } }, u8"return ((a.x - b.x) * (p.y - b.y) - (a.y - b.y) * (p.x - b.x));");

			auto solveLinearSegmentFunctionHandle = pipeline->DeclareFunction(uiScope, u8"UIRes", u8"SolveLinearSegment", { { u8"float32", u8"distance" }, { u8"LinearSegment", u8"segment" }, { u8"vec2f", u8"a" }, { u8"vec2f", u8"b" }, { u8"vec2f", u8"point" } }, u8"vec2f p = ClosestPointOnLineSegmentToPoint(segment.segments[0], segment.segments[1], point); float32 l = length(p - point); if(l < distance) { return UIRes(l, segment.segments[0], segment.segments[1]); } return UIRes(distance, a, b);");

			auto solveQuadraticSegmentFunctionHandle = pipeline->DeclareFunction(uiScope, u8"UIRes", u8"SolveQuadraticSegment", { { u8"float32", u8"distance" }, { u8"QuadraticSegment", u8"segment" }, { u8"vec2f", u8"a" }, { u8"vec2f", u8"b" }, { u8"vec2f", u8"point" } });

			pipeline->AddCodeToFunction(solveQuadraticSegmentFunctionHandle, u8"vec2f abl[2] = vec2f[2](vec2f(0), vec2f(0)); float32 dist = 100.0f; const uint32 LOOPS = 32; float32 bounds[2] = float32[](0.0f, 1.0f); uint32 sideToAdjust = 0; for (uint32 l = 0; l < LOOPS; ++l) { for (uint32 i = 0, ni = 1; i < 2; ++i, --ni) { float32 t = mix(bounds[0], bounds[1], float32(i) / 1.0f); vec2f ab = mix(segment.segments[0], segment.segments[1], t); vec2f bc = mix(segment.segments[1], segment.segments[2], t); vec2f pos = mix(ab, bc, t); abl[0] = ab; abl[1] = bc; float32 newDist = length(pos - point); if (newDist < dist) { sideToAdjust = ni; dist = newDist; } } bounds[sideToAdjust] = (bounds[0] + bounds[1]) / 2.0f; } if(dist < distance) { return UIRes(dist, abl[0], abl[1]); } return UIRes(distance, a, b);");

			auto fontRenderingFragmentShader = pipeline->DeclareShader(uiScope, u8"fontRendering");
			auto mainFunctionHandle = pipeline->DeclareFunction(fontRenderingFragmentShader, u8"void", u8"main");

			pipeline->AddCodeToFunction(mainFunctionHandle, u8"const uint32 BE_INSTANCE_INDEX = instanceIndex; const vec2f BE_UV = vertexUV;");

			pipeline->AddCodeToFunction(mainFunctionHandle, u8"FontData* font = pushConstantBlock.ui.fontData[pushConstantBlock.ui.textData[pushConstantBlock.uiInstances[BE_INSTANCE_INDEX].derivedTypeIndex[0]].fontIndex];");
			pipeline->AddCodeToFunction(mainFunctionHandle, u8"GlyphData* glyph = font.glyphs[pushConstantBlock.uiInstances[BE_INSTANCE_INDEX].derivedTypeIndex[1]];");
			pipeline->AddCodeToFunction(mainFunctionHandle, u8"float32 shortestDistance = 1000000.0f; vec2f point = BE_UV * 0.5f + 0.5f, a = point, b = point;");

			//pipeline->AddCodeToFunction(mainFunctionHandle, u8"for (uint32 c = 0; c < 2; ++c) {");
			pipeline->AddCodeToFunction(mainFunctionHandle, u8"for (uint32 c = 0; c < glyph.contourCount; ++c) {");
			pipeline->AddCodeToFunction(mainFunctionHandle, u8"GlyphContourData contour = glyph.contours[c];");
			
			//pipeline->AddCodeToFunction(mainFunctionHandle, u8"for (uint32 i = 0; i < 2; ++i) {");
			pipeline->AddCodeToFunction(mainFunctionHandle, u8"for (uint32 i = 0; i < glyph.contours[c].linearSegmentCount; ++i) {");
			pipeline->AddCodeToFunction(mainFunctionHandle, u8"UIRes res = SolveLinearSegment(shortestDistance, glyph.contours[c].linearSegments[i], a, b, point);");
			pipeline->AddCodeToFunction(mainFunctionHandle, u8"shortestDistance = res.bestDistance; a = res.a; b = res.b;");
			pipeline->AddCodeToFunction(mainFunctionHandle, u8"}");
			
			//pipeline->AddCodeToFunction(mainFunctionHandle, u8"for (uint32 i = 0; i < 33; ++i) {");
			pipeline->AddCodeToFunction(mainFunctionHandle, u8"for (uint32 i = 0; i < glyph.contours[c].quadraticSegmentCount; ++i) {");
			pipeline->AddCodeToFunction(mainFunctionHandle, u8"UIRes res = SolveQuadraticSegment(shortestDistance, glyph.contours[c].quadraticSegments[i], a, b, point);");
			pipeline->AddCodeToFunction(mainFunctionHandle, u8"shortestDistance = res.bestDistance; a = res.a; b = res.b;");
			pipeline->AddCodeToFunction(mainFunctionHandle, u8"}");
			pipeline->AddCodeToFunction(mainFunctionHandle, u8"}");
			
			//pipeline->AddCodeToFunction(mainFunctionHandle, u8"surfaceColor = vec4f(vec3f(TestPointToLineSide(a, b, point)), 1.0f);");
			pipeline->AddCodeToFunction(mainFunctionHandle, u8"surfaceColor = vec4f(TestPointToLineSide(a, b, point) >= 0.0f ? 1.0f : 0.0f);");
			//pipeline->AddCodeToFunction(mainFunctionHandle, u8"surfaceColor = vec4f(point, 0.0f, 0.9f);");
			//pipeline->AddCodeToFunction(mainFunctionHandle, u8"surfaceColor = vec4f(glyph.contourCount / 255.f, glyph.contours[0].linearSegmentCount / 255.f, glyph.contours[0].quadraticSegmentCount / 255.f, 

			AddSurfaceShaderOutDeclaration(pipeline, fontRenderingFragmentShader, { { u8"vec4f", u8"surfaceColor" } });

			auto& textShaderGroup = results.EmplaceBack();
			textShaderGroup.ShaderGroupJSON = u8"{ \"name\":\"UIText\", \"instances\":[{ \"name\":\"UIText\", \"parameters\":[] }], \"domain\":\"Screen\", \"tags\":[{ \"name\":\"RenderPass\", \"value\":\"UIRenderPass\" }, { \"name\":\"Transparency\", \"value\":\"true\" }] }";
			auto& vs = textShaderGroup.Shaders.EmplaceBack(); auto& vss = vs.EmplaceBack();
			vss.Scopes.EmplaceBack(commonPermutation->vertexShaderScope);
			vss.Scopes.EmplaceBack(vertexShaderHandle);
			vss.Tags.EmplaceBack(u8"Domain", u8"UI");
			vss.TargetSemantics = GAL::ShaderType::VERTEX;

			auto& fs = textShaderGroup.Shaders.EmplaceBack(); auto& fss = fs.EmplaceBack();
			fss.Scopes.EmplaceBack(commonPermutation->fragmentShaderScope);
			fss.Scopes.EmplaceBack(fontRenderingFragmentShader);
			fss.Tags.EmplaceBack(u8"Domain", u8"UI");
			fss.TargetSemantics = GAL::ShaderType::FRAGMENT;

			//float DistToLine(vec2 pt1, vec2 pt2, vec2 testPt)
//{
//  vec2 lineDir = pt2 - pt1;
//  vec2 perpDir = vec2(lineDir.y, -lineDir.x);
//  vec2 dirToPt1 = pt1 - testPt;
//  return abs(dot(normalize(perpDir), dirToPt1));
//}
		}

		return results;
	}

	void ProcessShader(GPipeline* pipeline, GTSL::JSONMember shader_group_json, GTSL::JSONMember shader_json, const GTSL::Range<const PermutationManager**> hierarchy, GTSL::StaticVector<ShaderPermutation, 8>& batches) override {
		auto [shaderHandle, mainFunctionHandle] = DeclareShader(pipeline, batches, shader_json[u8"name"], {}, GAL::ShaderType::FRAGMENT);
		pipeline->AddCodeToFunction(mainFunctionHandle, shader_json[u8"code"]);
	}

private:
	GPipeline::ElementHandle uiScope, vertexShaderHandle, uiRenderPassScopeHandle;
};