#pragma once

#include "PermutationManager.hpp"
#include "ByteEngine/Render/ShaderGenerator.h"

class UIPermutation : public PermutationManager {
public:
	UIPermutation(const GTSL::StringView instance_name) : PermutationManager(instance_name, u8"UIPermutation") {
		AddSupportedDomain(u8"UI");
	}

	void Initialize(GPipeline* pipeline, ShaderGenerationData& shader_generation_data) override {
		uiScope = pipeline->DeclareScope(GPipeline::GLOBAL_SCOPE, u8"UIPermutation");

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
		AddVertexSurfaceInterfaceBlockDeclaration(pipeline, uiScope, { {u8"vec2f", u8"vertexPos"}, {u8"vec2f", u8"vertexUV"}, {u8"uint32", u8"instanceIndex"} });
		uiRenderPassScopeHandle = pipeline->DeclareStruct(uiScope, u8"RenderPassData", { { u8"TextureReference", u8"Color" } });

		{
			auto fragmentOutputBlockHandle = pipeline->DeclareScope(uiScope, u8"fragmentOutputBlock");
			auto outColorHandle = pipeline->DeclareVariable(fragmentOutputBlockHandle, { u8"vec4f", u8"out_Color" });
			pipeline->AddMemberDeductionGuide(uiScope, u8"surfaceColor", { outColorHandle });
		}

		auto closestPointOnLineSegmentToPointHandle = pipeline->DeclareFunction(uiScope, u8"vec2f", u8"ClosestPointOnLineSegmentToPoint", { { u8"vec2f", u8"a" }, { u8"vec2f", u8"b" }, { u8"vec2f", u8"p" } }, u8"const vec2f AB = b - a; float32 t = dot(AB, p - a) / dot(AB, AB); return a + AB * clamp(t, 0.0f, 1.0f);");

		auto testPointToLineSideFunctionHandle = pipeline->DeclareFunction(uiScope, u8"float32", u8"TestPointToLineSide", { { u8"vec2f", u8"a" }, { u8"vec2f", u8"b" }, { u8"vec2f", u8"p" } }, u8"return ((a.x - b.x) * (p.y - b.y) - (a.y - b.y) * (p.x - b.x));");

		auto solveLinearSegmentFunctionHandle = pipeline->DeclareFunction(uiScope, u8"UIRes", u8"SolveLinearSegment", { { u8"float32", u8"distance" }, { u8"LinearSegment", u8"segment" }, { u8"vec2f", u8"a" }, { u8"vec2f", u8"b" }, { u8"vec2f", u8"point" } }, u8"vec2f p = ClosestPointOnLineSegmentToPoint(segment.segments[0], segment.segments[1], point); float32 l = length(p - point); if(l < distance) { return UIRes(l, segment.segments[0], segment.segments[1]); } return UIRes(distance, a, b);");

		auto solveQuadraticSegmentFunctionHandle = pipeline->DeclareFunction(uiScope, u8"UIRes", u8"SolveQuadraticSegment", { { u8"float32", u8"distance" }, { u8"QuadraticSegment", u8"segment" }, { u8"vec2f", u8"a" }, { u8"vec2f", u8"b" }, { u8"vec2f", u8"point" } });

		pipeline->AddCodeToFunction(solveQuadraticSegmentFunctionHandle, u8"vec2f abl[2] = vec2f[2](vec2f(0), vec2f(0)); float32 dist = 100.0f; const uint32 LOOPS = 32; float32 bounds[2] = float32[](0.0f, 1.0f); uint32 sideToAdjust = 0; for (uint32 l = 0; l < LOOPS; ++l) { for (uint32 i = 0, ni = 1; i < 2; ++i, --ni) { float32 t = mix(bounds[0], bounds[1], float32(i) / 1.0f); vec2f ab = mix(segment.segments[0], segment.segments[1], t); vec2f bc = mix(segment.segments[1], segment.segments[2], t); vec2f pos = mix(ab, bc, t); abl[0] = ab; abl[1] = bc; float32 newDist = length(pos - point); if (newDist < dist) { sideToAdjust = ni; dist = newDist; } } bounds[sideToAdjust] = (bounds[0] + bounds[1]) / 2.0f; } if(dist < distance) { return UIRes(dist, abl[0], abl[1]); } return UIRes(distance, a, b);");
	}

private:
	GPipeline::ElementHandle uiScope, uiRenderPassScopeHandle;
};