#pragma once

#include "Core.h"

#include "Math/Vector2.h"
#include "Math/Vector3.h"
#include "Utility/TextureCoordinates.h"
#include "RAPI/RenderMesh.h"

using Index = uint16;

struct GS_API Vertex2D
{
	Vector2 Position;
	TextureCoordinates TextureCoordinates;
};

struct GS_API Vertex
{
	Vector3 Position;
	Vector3 Normal;
	TextureCoordinates TextCoord;
	Vector3 Tangent;
	Vector3 BiTangent;
};
