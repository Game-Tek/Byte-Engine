#pragma once

#include "Core.h"

#include "Object.h"

//Used to specify a location in 3D space with floating point precision.
GS_STRUCT Vector3
{
	//X component of this vector.
	float X = 0.0f;

	//Y component of this vector.
	float Y = 0.0f;

	//Z component of this vector.
	float Z = 0.0f;

	Vector3() = default;

	Vector3(float X, float Y, float Z) : X(X), Y(Y), Z(Z)
	{
	}

	Vector3(const Vector3& _Other) = default;

	~Vector3() = default;

	Vector3 operator+ (float Other) const
	{
		return { X + Other, Y + Other, Z + Other };
	}

	Vector3 operator+ (const Vector3 & Other) const
	{
		return { X + Other.X, Y + Other.Y, Z + Other.Z };
	}

	Vector3 & operator+= (float Other)
	{
		X += Other;
		Y += Other;
		Z += Other;

		return *this;
	}

	Vector3 & operator+= (const Vector3 & Other)
	{
		X += Other.X;
		Y += Other.Y;
		Z += Other.Z;

		return *this;
	}

	Vector3 operator- (float Other) const
	{
		return { X - Other, Y - Other, Z - Other };
	}

	Vector3 operator- (const Vector3 & Other) const
	{
		return { X - Other.X, Y - Other.Y, Z - Other.Z };
	}

	Vector3 & operator-= (float Other)
	{
		X -= Other;
		Y -= Other;
		Z -= Other;

		return *this;
	}

	Vector3 & operator-= (const Vector3 & Other)
	{
		X -= Other.X;
		Y -= Other.Y;
		Z -= Other.Z;

		return *this;
	}

	Vector3 operator* (float Other) const
	{
		return { X * Other, Y * Other, Z * Other };
	}

	Vector3 & operator*= (float Other)
	{
		X *= Other;
		Y *= Other;
		Z *= Other;

		return *this;
	}

	Vector3 operator/ (float Other) const
	{
		return { X / Other, Y / Other, Z / Other };
	}

	Vector3 & operator/= (float Other)
	{
		X /= Other;
		Y /= Other;
		Z /= Other;

		return *this;
	}

	inline bool operator== (const Vector3 & Other)
	{
		return X == Other.X && Y == Other.Y && Z == Other.Z;
	}

	inline bool operator!= (const Vector3 & Other)
	{
		return X != Other.X || Y != Other.Y || Z != Other.Z;
	}
};