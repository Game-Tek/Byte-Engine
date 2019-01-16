#pragma once

#include "Core.h"

//Used to specify a rotation in 3D space with floating point precision.
GS_CLASS Quat
{
public:
	//X component of this quaternion.
	float X = 0;

	//Y component of this quaternion.
	float Y = 0;
	
	//Z component of this quaternion.
	float Z = 0;

	//Q component of this quaternion.
	float Q = 0;

	Quat()
	{
	}

	Quat(float X, float Y, float Z, float Q) : X(X), Y(Y), Z(Z), Q(Q)
	{
	}
};