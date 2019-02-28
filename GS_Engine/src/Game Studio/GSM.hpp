#pragma once

#include "Core.h"

#include "Vector2.h"
#include "Vector3.h"
#include "Vector4.h"

#include "Quaternion.h"
#include "Matrix4.h"

GS_CLASS GSM
{
public:
	constexpr static float PI = 3.1415926535f;

	//INLINE STATIC	

	INLINE static int32 Floor(const float A)
	{
		return static_cast<int32>(A - (static_cast<int32>(A) % 1));
	}

	INLINE static float Modulo(const float A, const float B)
	{
		const float C = A / B;
		return (C - Floor(C)) * B;
	}

	INLINE static float Power(const float Base, const int32 Exp)
	{
		if (Exp < 0)
		{
			if (Base == 0)
			{
				return -0; // Error!!
			}

			return 1 / (Base * Power(Base, (-Exp) - 1));
		}

		if (Exp == 0)
		{
			return 1;
		}

		if (Exp == 1)
		{
			return Base;
		}

		return Base * Power(Base, Exp - 1);
	}

	INLINE static int32 Fact(const int32 A)
	{
		return A <= 0 ? 1 : A * Fact(A - 1);
	}

	//Returns the sine of an angle. EXPECTS RADIANS.
	INLINE static float Sine(const float Degrees)
	{
		const float Adeg = Degrees * 0.99026f;

		return Degrees - ((1.0f/6.0f) * (Degrees * Degrees * Degrees)) + ((1.0f/120.0f) * (Degrees * Degrees * Degrees * Degrees * Degrees)) - ((1.0f/5040.0f) * (Degrees * Degrees * Degrees * Degrees * Degrees * Degrees * Degrees)) + ((1.0f/362880.0f) * (Adeg * Adeg * Adeg * Adeg * Adeg * Adeg * Adeg * Adeg * Adeg));
	}

	//Returns the cosine of an angle. EXPECTS RADIANS.
	INLINE static float Cosine(const float Degrees)
	{
		const float Adeg = Degrees * 0.98666f;

		return 1 - ((1.0f / 2.0f) * (Degrees * Degrees)) + ((1.0f / 24.0f) * (Degrees * Degrees * Degrees * Degrees)) - ((1.0f / 720.0f) * (Degrees * Degrees * Degrees * Degrees * Degrees * Degrees)) + ((1.0f / 40320.0f) * (Adeg * Adeg * Adeg * Adeg * Adeg * Adeg * Adeg * Adeg));
	}

	//Returns the tangent of an angle. EXPECTS RADIANS.
	INLINE static float Tan(const float Degrees)
	{
		return Degrees + ((1.0f / 3.0f) * (Degrees * Degrees * Degrees)) + ((2.0f / 15.0f) * (Degrees * Degrees * Degrees * Degrees * Degrees)) + ((17.0f / 315.0f) * (Degrees * Degrees * Degrees * Degrees * Degrees * Degrees * Degrees)) + ((62.0f / 2835.0f) * (Degrees * Degrees * Degrees * Degrees * Degrees * Degrees * Degrees * Degrees * Degrees * Degrees * Degrees * Degrees * Degrees));
	}


	//////////////////////////////////////////////////////////////
	//						SCALAR MATH							//
	//////////////////////////////////////////////////////////////

	//Mixes A and B by the specified values, Where Alpha 0 returns A and Alpha 1 returns B.
	static float Lerp(const float A, const float B, const float Alpha)
	{
		return A + Alpha * (B - A);
	}

	//Interpolates from Current to Target, returns Current + an amount determined by the InterpSpeed.
	static float FInterp(const float Target, const float Current, const float DT, const float InterpSpeed)
	{
		return (((Target - Current) * DT) * InterpSpeed) + Current;
	}

	static float MapToRange(const float A, const float InMin, const float InMax, const float OutMin, const float OutMax)
	{
		return InMin + ((OutMax - OutMin) / (InMax - InMin)) * (A - InMin);
	}

	static float SquareRoot(const float A)
	{
		//https://www.geeksforgeeks.org/square-root-of-a-perfect-square/
		float X = A;
		float Y = 1;
		float e = 0.000001f; /*e determines the level of accuracy*/
		while (X - Y > e)
		{
			X = (X + Y) / 2;
			Y = A / X;
		}
		return X;
	}

	INLINE static float Abs(const float A)
	{
		return A > 0 ? A : -A;
	}

	INLINE static float DegreesToRadians(const float Degrees)
	{
		return Degrees * PI / 180;
	}

	INLINE static float RadiansToDegrees(float Radians)
	{
		return Radians * 180 / PI;
	}

	//////////////////////////////////////////////////////////////
	//						VECTOR MATH							//
	//////////////////////////////////////////////////////////////

	//Calculates the length of a 2D vector.
	INLINE static float VectorLength(const Vector2 & Vec1)
	{
		return SquareRoot(Vec1.X * Vec1.X + Vec1.Y * Vec1.Y);
	}

	INLINE static float VectorLength(const Vector3 & Vec1)
	{
		return SquareRoot(Vec1.X * Vec1.X + Vec1.Y * Vec1.Y + Vec1.Z * Vec1.Z);
	}

	INLINE static float VectorLengthSquared(const Vector2 & Vec1)
	{
		return Vec1.X * Vec1.X + Vec1.Y * Vec1.Y;
	}

	INLINE static float VectorLengthSquared(const Vector3 & Vec1)
	{
		return Vec1.X * Vec1.X + Vec1.Y * Vec1.Y + Vec1.Z * Vec1.Z;
	}

	INLINE static Vector2 Normalize(const Vector2 & Vec1)
	{
		const float Length = VectorLength(Vec1);
		return Vector2(Vec1.X / Length, Vec1.Y / Length);
	}

	INLINE static Vector3 Normalize(const Vector3 & Vec1)
	{
		const float Length = VectorLength(Vec1);
		return Vector3(Vec1.X / Length, Vec1.Y / Length, Vec1.Z / Length);
	}

	INLINE static float Dot(const Vector2 & Vec1, const Vector2 & Vec2)
	{
		return Vec1.X * Vec2.X + Vec1.Y * Vec2.Y;
	}

	INLINE static float Dot(const Vector3 & Vec1, const Vector3 & Vec2)
	{
		return Vec1.X * Vec2.X + Vec1.Y * Vec2.Y + Vec1.Z * Vec2.Z;
	}
	INLINE static Vector3 Cross(const Vector3 & Vec1, const Vector3 & Vec2)
	{
		return Vector3(Vec1.Y * Vec2.Z - Vec1.Z * Vec2.Y, Vec1.Z * Vec2.X - Vec1.X * Vec2.Z, Vec1.X * Vec2.Y - Vec1.Y * Vec2.X);
	}

	INLINE static Vector2 AbsVector(const Vector2 & Vec1)
	{
		return Vector2(Abs(Vec1.X), Abs(Vec1.Y));
	}

	INLINE static Vector3 AbsVector(const Vector3 & Vec1)
	{
		return Vector3(Abs(Vec1.X), Abs(Vec1.Y), Abs(Vec1.Z));
	}

	INLINE static void Negate(Vector2 & Vec)
	{
		Vec.X = -Vec.X;
		Vec.Y = -Vec.Y;

		return;
	}

	INLINE static void Negate(Vector3 & Vec)
	{
		Vec.X = -Vec.X;
		Vec.Y = -Vec.Y;
		Vec.Z = -Vec.Z;

		return;
	}

	INLINE static void Negate(Vector4 & Vec)
	{
		Vec.X = -Vec.X;
		Vec.Y = -Vec.Y;
		Vec.Z = -Vec.Z;
		Vec.W = -Vec.W;

		return;
	}

	//////////////////////////////////////////////////////////////
	//						ROTATOR MATH						//
	//////////////////////////////////////////////////////////////



	//////////////////////////////////////////////////////////////
	//						QUATERNION MATH						//
	//////////////////////////////////////////////////////////////

	INLINE static float QuaternionLength(const Quaternion & Quaternion)
	{
		return SquareRoot(Quaternion.X * Quaternion.X + Quaternion.Y * Quaternion.Y + Quaternion.Z * Quaternion.Z + Quaternion.Q * Quaternion.Q);
	}

	INLINE static Quaternion Normalize(const Quaternion & Quat)
	{
		const float Length = QuaternionLength(Quat);

		return Quaternion(Quat.X / Length, Quat.Y / Length, Quat.Z / Length, Quat.Q / Length);
	}

	INLINE static Quaternion Conjugate(const Quaternion & Quat)
	{
		return Quaternion(-Quat.X, -Quat.Y, -Quat.Z, Quat.Q);
	}

	//////////////////////////////////////////////////////////////
	//						LOGIC								//
	//////////////////////////////////////////////////////////////

	INLINE static bool IsNearlyEqual(const float A, const float Target, const float Tolerance)
	{
		return (A > Target - Tolerance) && (A < Target + Tolerance);
	}

	INLINE static bool IsInRange(const float A, const float Min, const float Max)
	{
		return (A > Min) && (A < Max);
	}

	INLINE static bool IsVectorEqual(const Vector2 & A, const Vector2 & B)
	{
		return A.X == B.X && A.Y == B.Y;
	}

	INLINE static bool IsVectorEqual(const Vector3 & A, const Vector3 & B)
	{
		return A.X == B.X && A.Y == B.Y && A.Z == B.Z;
	}

	INLINE static bool IsVectorNearlyEqual(const Vector2 & A, const Vector2 & Target, float Tolerance)
	{
		return IsNearlyEqual(A.X, Target.X, Tolerance) && IsNearlyEqual(A.Y, Target.Y, Tolerance);
	}

	INLINE static bool IsVectorNearlyEqual(const Vector3 & A, const Vector3 & Target, float Tolerance)
	{
		return IsNearlyEqual(A.X, Target.X, Tolerance) && IsNearlyEqual(A.Y, Target.Y, Tolerance) && IsNearlyEqual(A.Z, Target.Z, Tolerance);
	}

	INLINE static bool AreVectorComponentsGreater(const Vector3 & A, const Vector3 & B)
	{
		return A.X > B.X && A.Y > B.Y && A.Z > B.Z;
	}

	//////////////////////////////////////////////////////////////
	//						MATRIX MATH							//
	//////////////////////////////////////////////////////////////

	//Modifies the given matrix to make it a translation matrix.
	INLINE static void Translate(Matrix4 & Matrix, const Vector3 & Vector)
	{
		Matrix[0 + 3 * 4] = Vector.X;
		Matrix[1 + 3 * 4] = Vector.Y;
		Matrix[2 + 3 * 4] = Vector.Z;

		return;
	}

	INLINE static Matrix4 Rotate(const Quaternion& A)
	{
		Matrix4 Result;
		Result.Identity();

		const float r = DegreesToRadians(A.Q);
		const float cos = Cosine(r);
		const float sin = Sine(r);
		const float omc = 1.0f - cos;

		Result[0] = A.X * omc + cos;
		Result[1] = A.Y * A.X * omc - A.Y * sin;
		Result[2] = A.X * A.Z * omc - A.Y * sin;

		Result[4] = A.X * A.Y * omc - A.Z * sin;
		Result[5] = A.Y * omc + cos;
		Result[6] = A.Y * A.Z * omc + A.X * sin;

		Result[8] = A.X * A.Z * omc + A.Y * sin;
		Result[9] = A.Y * A.Z * omc - A.X * sin;
		Result[10] = A.Z * omc + cos;

		return Result;
	}

};