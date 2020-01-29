#pragma once

#include "Core.h"

#include "Vector2.h"

//Used to specify a transform in 2D space with floating point precision.
class GS_API Transform2
{
public:
	Vector2 Position;
	float Rotation = 0.0f;
	Vector2 Scale;
};
