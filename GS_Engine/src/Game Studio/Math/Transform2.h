#pragma once

#include "Core.h"

#include "Vector2.h"

//Used to specify a transform in 2D space with floating point precision.
GS_CLASS Transform2
{
public:
	Vector2 Position;
	float Rotation;
	Vector2 Scale;
};