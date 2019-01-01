#pragma once

#include "Core.h"

#include "Vector3.h"
#include "Rotator.h"

//Used to specify a transform in 3D space with floating point precision.
GS_CLASS Transform3
{
public:
	Vector3 Position;
	Rotator Rotation;
	Vector3 Scale;
};
