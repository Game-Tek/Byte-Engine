#pragma once

#include "Core.h"

//Used to specify a rotation with floating point precision.
class GS_API Rotator
{
public:
	//Yaw(Y) component of this rotator.
	float X = 0;

	//Pitch(X) component of this rotator.
	float Y = 0;

	//Roll(Z) component of this rotator.
	float Z = 0;

	Rotator()
	{
	}

	Rotator(float x, float y, float z) : X(x), Y(y), Z(z)
	{
	}

	Rotator(const Rotator & Other) : X(Other.X), Y(Other.Y), Z(Other.Z)
	{
	}

	explicit Rotator(const class Vector3& vector);

	Rotator operator+ (const Rotator & Other)
	{
		return { X + Other.X, Y + Other.Y, Z + Other.Z };
	}

	Rotator operator- (const Rotator & Other)
	{
		return { X - Other.X, Y - Other.Y, Z - Other.Z };
	}

	Rotator operator* (float Other)
	{
		return { X * Other, Y * Other, Z * Other };
	}

	Rotator operator/ (float Other)
	{
		return { X / Other, Y / Other, Z / Other };
	}

	Rotator& operator+=(const Rotator& rotator)
	{
		X += rotator.X;
		Y += rotator.Y;
		Z += rotator.Z;
		return *this;
	}
};