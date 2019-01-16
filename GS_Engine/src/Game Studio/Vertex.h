#pragma once

#include "Core.h"

#include "Vector3.h"
#include "RGB.h"
#include "TextureCoordinates.h"

GS_STRUCT Vertex
{
	Vector3				Position;
	RGB					Color;
	TextureCoordinates	TextCoord;
};
