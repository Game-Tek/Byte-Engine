#pragma once

#include "Core.h"

#include "Vertex.h"

GS_STRUCT ScreenQuad
{
	inline static Vertex2D Vertices[] = { { {  1.0f,  1.0f }, { 1.0f, 1.0f } },
										  { {  1.0f, -1.0f }, { 1.0f, 0.0f } },
										  { { -1.0f, -1.0f }, { 0.0f, 0.0f } },
										  { { -1.0f,  1.0f }, { 0.0f, 1.0f } } };

	inline static Index Indices[] = { 0, 1, 2, 2, 3, 0 };

	inline static uint8 VertexCount = 4;
	inline static uint8 IndexCount = 6;
};