#pragma once

#include <GTSL/Math/Vector2.h>
#include <GTSL/Math/Vector3.h>
#include <GTSL/TextureCoordinates.h>
#include "Physics/ForceGenerators.h"

using Index = uint16;

struct Vertex2D
{
	GTSL::Vector2 Position;
	GTSL::TextureCoordinates2D TextureCoordinates;
};

struct Vertex
{
	GTSL::Vector3 Position;
	GTSL::Vector3 Normal;
	GTSL::TextureCoordinates2D TextCoord;
	GTSL::Vector3 Tangent;
	GTSL::Vector3 BiTangent;
};

