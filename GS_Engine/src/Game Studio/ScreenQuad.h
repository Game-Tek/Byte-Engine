#pragma once

#include "Core.h"

#include "Vertex.h"

struct GS_API ScreenQuad
{
	ScreenQuad() = default;

	Vertex2D Vertices[4] = { { {-1.0f, -1.0f}, { 1.0f, 1.0f } }, { {1.0f, -1.0f}, { 1.0f, 0.0f } }, { {1.0f, 1.0f}, { 0.0f, 0.0f } }, { {-1.0f, 1.0f}, { 0.0f, 1.0f } } };

	Index Indices[6] = { 0, 1, 2, 2, 3, 0 };

	uint8 VertexCount = 4;
	uint8 IndexCount = 6;
};