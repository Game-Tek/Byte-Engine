#pragma once

#include "Core.h"

#include "Vertex.h"

struct ScreenQuad
{
	ScreenQuad() = default;

	inline static Vertex2D Vertices[4] = {
		{{-1.0f, -1.0f}, {1.0f, 1.0f}}, {{1.0f, -1.0f}, {1.0f, 0.0f}}, {{1.0f, 1.0f}, {0.0f, 0.0f}},
		{{-1.0f, 1.0f}, {0.0f, 1.0f}}
	};

	inline static Index Indices[6] = {0, 1, 2, 2, 3, 0};

	inline static uint8 VertexCount = 4;
	inline static uint8 IndexCount = 6;

	inline static const DArray<ShaderDataTypes> Elements = {ShaderDataTypes::FLOAT2, ShaderDataTypes::FLOAT2};
	inline static VertexDescriptor VD{Elements};
};
