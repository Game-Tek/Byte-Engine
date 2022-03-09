#pragma once

#include "PermutationManager.hpp"

class UIPermutation : public PermutationManager {
public:
	GTSL::Vector<Result1, BE::TAR> MakeShaderGroups(GPipeline* pipeline, GTSL::Range<const PermutationManager**> hierarchy) override {
		auto* commonPermutation = Find<CommonPermutation>(u8"CommonPermutation", hierarchy);
		//tokenizeCode(u8"float32 x = float32(((uint32(gl_VertexID) + 2) / 3) % 2); float32 y = float32(((uint32(gl_VertexID) + 1) / 3) % 2); vertexTextureCoordinates = vec2f(x, y); vertexPosition = vec4f(-1.0f + x * 2.0f, -1.0f + y * 2.0f, 0.0f, 1.0f);")
	}
private:
};