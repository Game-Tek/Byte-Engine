#pragma once

#include "Core.h"

#include "GSM.hpp"

//Used to specify a location in 3D space with floating point precision.
GS_CLASS Vector3
{
public:
	//X component of this vector.
	float X = 0.0f;

	//Y component of this vector.
	float Y = 0.0f;

	//Z component of this vector.
	float Z = 0.0f;

	Vector3()
	{
	}

	Vector3(float X, float Y, float Z) : X(X), Y(Y), Z(Z)
	{
	}

	Vector3(const Vector3 & Other) : X(Other.X), Y(Other.Y), Z(Other.Z)
	{
	}

	void Negate()
	{
		X = -X;
		Y = -Y;
		Z = -Z;

		return;
	}

	void Normalize()
	{
		*this = GSM::Normalize(*this);

		return;
	}

	inline float Length() const { return GSM::SquareRoot(LengthSquared()); }

	inline float LengthSquared() const
	{
		return X * X + Y * Y + Z * Z;
	}

	void operator= (const Vector3 & Other)
	{
		X = Other.X;
		Y = Other.Y;
		Z = Other.Z;

		return;
	}

	Vector3 operator+ (float Other) const
	{
		return { X + Other, Y + Other, Z + Other };
	}

	Vector3 operator+ (const Vector3 & Other) const
	{
		return { X + Other.X, Y + Other.Y, Z + Other.Z };
	}

	void operator+= (float Other)
	{
		X += Other;
		Y += Other;
		Z += Other;

		return;
	}

	void operator+= (const Vector3 & Other)
	{
		X += Other.X;
		Y += Other.Y;
		Z += Other.Z;

		return;
	}

	Vector3 operator- (float Other) const
	{
		return { X - Other, Y - Other, Z - Other };
	}

	Vector3 operator- (const Vector3 & Other) const
	{
		return { X - Other.X, Y - Other.Y, Z - Other.Z };
	}

	void operator-= (float Other)
	{
		X -= Other;
		Y -= Other;
		Z -= Other;

		return;
	}

	void operator-= (const Vector3 & Other)
	{
		X -= Other.X;
		Y -= Other.Y;
		Z -= Other.Z;

		return;
	}

	Vector3 operator* (float Other) const
	{
		return { X * Other, Y * Other, Z * Other };
	}

	void operator*= (float Other)
	{
		X *= Other;
		Y *= Other;
		Z *= Other;

		return;
	}

	Vector3 operator/ (float Other) const
	{
		return { X / Other, Y / Other, Z / Other };
	}

	void operator/= (float Other)
	{
		X /= Other;
		Y /= Other;
		Z /= Other;

		return;
	}
};