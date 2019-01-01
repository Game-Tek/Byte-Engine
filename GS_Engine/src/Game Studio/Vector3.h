#pragma once

#include "Core.h"

//Used to specify a location in 3D space with floating point precision.
GS_CLASS Vector3
{
public:
	float X;
	float Y;
	float Z;

	Vector3 operator+ (float Other)
	{
		return { X + Other, Y + Other, Z + Other };
	}

	Vector3 operator+ (const Vector3 & Other)
	{
		return { X + Other.X, Y + Other.Y, Z + Other.Z };
	}

	Vector3 operator- (float Other)
	{
		return { X - Other, Y - Other, Z - Other };
	}

	Vector3 operator- (const Vector3 & Other)
	{
		return { X - Other.X, Y - Other.Y, Z - Other.Z };
	}

	Vector3 operator* (float Other)
	{
		return { X * Other, Y * Other, Z * Other };
	}

	Vector3 operator/ (float Other)
	{
		return { X / Other, Y / Other, Z / Other };
	}
};