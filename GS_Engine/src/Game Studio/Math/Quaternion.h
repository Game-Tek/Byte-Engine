#pragma once

#include "Core.h"

//Used to specify a rotation in 3D space with floating point precision.
class GS_API Quaternion
{
public:
	//X component of this quaternion.
	float X = 0.0f;

	//Y component of this quaternion.
	float Y = 0.0f;
	
	//Z component of this quaternion.
	float Z = 0.0f;

	//Q component of this quaternion.
	float Q = 0.0f;

	Quaternion() = default;

	Quaternion(float X, float Y, float Z, float Q) : X(X), Y(Y), Z(Z), Q(Q)
	{
	}

	~Quaternion() = default;

	Quaternion operator+(const float Other) const
	{
		return Quaternion(X + Other, Y + Other, Z + Other, Q + Other);
	}

	Quaternion operator+(const Quaternion & Other) const
	{
		return Quaternion(X + Other.X, Y + Other.Y, Z + Other.Y, Q + Other.Z);
	}

	Quaternion & operator+=(const float Other)
	{
		X += Other;
		Y += Other;
		Z += Other;
		Q += Other;

		return *this;
	}

	Quaternion operator+=(const Quaternion & Other)
	{
		X += Other.X;
		Y += Other.Y;
		Z += Other.Z;
		Q += Other.Q;

		return *this;
	}

	Quaternion operator-(void) const
	{
		return Quaternion(-X, -Y, -Z, -Q);
	}

	Quaternion operator-(const float Other) const
	{
		return Quaternion(X - Other, Y - Other, Z - Other, Q - Other);
	}

	Quaternion operator-(const Quaternion & Other) const
	{
		return Quaternion(X - Other.X, Y - Other.Y, Z - Other.Y, Q - Other.Z);
	}

	Quaternion & operator-=(const float Other)
	{
		X -= Other;
		Y -= Other;
		Z -= Other;
		Q -= Other;

		return *this;
	}

	Quaternion operator-=(const Quaternion & Other)
	{
		X -= Other.X;
		Y -= Other.Y;
		Z -= Other.Z;
		Q -= Other.Q;

		return *this;
	}

	Quaternion operator*(const float Other)
	{
		return Quaternion(X * Other, Y * Other, Z * Other, Q * Other);
	}

	Quaternion operator*(const Quaternion & Other)
	{
		Quaternion Result;

		Result.X = Q * Other.X + X * Other.Q + Y * Other.Z - Z * Other.Y;
		Result.Y = Q * Other.Y - X * Other.Z + Y * Other.Q + Z * Other.X;
		Result.Z = Q * Other.Z + X * Other.Y - Y * Other.X + Z * Other.Q;
		Result.Q = Q * Other.Q - X * Other.X - Y * Other.Y - Z * Other.Z;

		return Result;
	}

	Quaternion & operator*=(const float Other)
	{
		X *= Other;
		Y *= Other;
		Z *= Other;
		Q *= Other;

		return *this;
	}

	Quaternion & operator*=(const Quaternion & Other)
	{
		X = Q * Other.X + X * Other.Q + Y * Other.Z - Z * Other.Y;
		Y = Q * Other.Y - X * Other.Z + Y * Other.Q + Z * Other.X;
		Z = Q * Other.Z + X * Other.Y - Y * Other.X + Z * Other.Q;
		Q = Q * Other.Q - X * Other.X - Y * Other.Y - Z * Other.Z;

		return *this;
	}

	Quaternion operator/(const float Other)
	{
		return Quaternion(X / Other, Y / Other, Z / Other, Q / Other);
	}

	Quaternion operator/(const Quaternion & Other)
	{
		return Quaternion(X / Other.X, Y / Other.X, Z / Other.Z, Q / Other.Q);
	}

	Quaternion & operator/=(const float Other)
	{
		X /= Other;
		Y /= Other;
		Z /= Other;
		Q /= Other;

		return *this;
	}

	Quaternion & operator/=(const Quaternion & Other)
	{
		X /= Other.X;
		Y /= Other.Y;
		Z /= Other.Z;
		Q /= Other.Q;

		return *this;
	}
};