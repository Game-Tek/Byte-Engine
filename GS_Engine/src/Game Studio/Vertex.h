#pragma once

#include "Core.h"

#include "Vector3.h"
#include "TextureCoordinates.h"

GS_STRUCT Vertex
{
	Vector3 Position;
	TextureCoordinates TextCoord;
};
