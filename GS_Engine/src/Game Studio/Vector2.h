#pragma once

#include "Core.h"

//Used to specify a location in 2D space with floating point precision.
GS_CLASS Vector2
{
public:
	float X;
	float Y;

	Vector2 operator+ (const Vector2 & Other)
	{
		return { X + Other.X, Y + Other.Y };
	}

	Vector2 operator- (const Vector2 & Other)
	{
		return { X - Other.X, Y - Other.Y };
	}

	Vector2 operator* (float Other)
	{
		return { X * Other, Y * Other };
	}

	Vector2 operator/ (float Other)
	{
		return { X / Other, Y / Other };
	}
};
