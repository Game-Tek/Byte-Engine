#pragma once

#include "Core.h"

#include "Math/Vector2.h"
#include "Math/Vector3.h"
#include "TextureCoordinates.h"
#include "RAPI/Mesh.h"

using Index = uint16;

GS_STRUCT Vertex2D
{
	Vector2 Position;
	TextureCoordinates TextureCoordinates;

	static VertexDescriptor Descriptor;
};

GS_STRUCT Vertex
{
	Vector3				Position;
	Vector3				Normal;
	TextureCoordinates	TextCoord;
	Vector3				Tangent;
	Vector3				BiTangent;
};
