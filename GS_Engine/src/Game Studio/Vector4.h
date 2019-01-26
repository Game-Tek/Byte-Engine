#pragma once

#include "Core.h"

GS_CLASS Vector4
{
public:
	//X component of this vector.
	float X = 0;

	//Y component of this vector.
	float Y = 0;

	//Z component of this vector.
	float Z = 0;

	//W component of this vector.
	float W = 0;

	Vector4()
	{
	}

	Vector4(float X, float Y, float Z, float W) : X(X), Y(Y), Z(Z), W(W)
	{
	}

	~Vector4()
	{
	}

	Vector4 operator+ (float Other) const
	{
		return { X + Other, Y + Other, Z + Other, W + Other };
	}

	Vector4 operator+ (const Vector4 & Other) const
	{
		return { X + Other.X, Y + Other.Y, Z + Other.Z, W + Other.W };
	}

	Vector4 & operator+= (float Other)
	{
		X += Other;
		Y += Other;
		Z += Other;
		W += Other;

		return *this;
	}

	Vector4 & operator+= (const Vector4 & Other)
	{
		X += Other.X;
		Y += Other.Y;
		Z += Other.Z;
		W += Other.W;

		return *this;
	}

	Vector4 operator- (float Other) const
	{
		return { X - Other, Y - Other, Z - Other, W - Other };
	}

	Vector4 operator- (const Vector4 & Other) const
	{
		return { X - Other.X, Y - Other.Y, Z - Other.Z, W - Other.W };
	}

	Vector4 & operator-= (float Other)
	{
		X -= Other;
		Y -= Other;
		Z -= Other;
		W -= Other;

		return *this;
	}

	Vector4 & operator-= (const Vector4 & Other)
	{
		X -= Other.X;
		Y -= Other.Y;
		Z -= Other.Z;
		W -= Other.W;

		return *this;
	}

	Vector4 operator* (float Other) const
	{
		return { X * Other, Y * Other, Z * Other, W * Other };
	}

	Vector4 & operator*= (float Other)
	{
		X *= Other;
		Y *= Other;
		Z *= Other;
		W *= Other;

		return *this;
	}

	Vector4 operator/ (float Other) const
	{
		return { X / Other, Y / Other, Z / Other, W / Other };
	}

	Vector4 & operator/= (float Other)
	{
		X /= Other;
		Y /= Other;
		Z /= Other;
		W /= Other;

		return *this;
	}

	inline bool operator== (const Vector4 & Other)
	{
		return X == Other.X && Y == Other.Y && Z == Other.Z && W == Other.W;
	}

	inline bool operator!= (const Vector4 & Other)
	{
		return X != Other.X || Y != Other.Y || Z != Other.Z || W != Other.W;
	}
};