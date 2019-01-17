#pragma once

#include "Core.h"

#include "Vector2.h"
#include "Vector3.h"
#include "Quat.h"
#include "Matrix4.h"

GS_CLASS GSM
{
public:
	//Mixes A and B by the specified values, Where Alpha 0 returns A and Alpha 1 returns B.
	static const float Lerp(float A, float B, float Alpha);

	//Interpolates from Current to Target, returns Current + an amount determined by the InterpSpeed.
	static const float FInterp(float Target, float Current, float DT, float InterpSpeed);

	static const float MapToRange(float A, float AMin, float AMax, float RangeMin, float RangeMax);

	static const float SquareRoot(float A);
	
	static const float Abs(float A);

	static const int Round(float A);

	static const float DegreesToRadians(float Degrees);

	static const float RadiansToDegrees(float Radians);

	//////////////////////////////////////////////////////////////
	//						VECTOR MATH							//
	//////////////////////////////////////////////////////////////

	//Calculates the length of a 2D vector.
	static const float VectorLength(const Vector2 &Vec1);

	//Calculates the length of a 3D vector.
	static const float VectorLength(const Vector3 & Vec1);

	//Calculates the squared length of a 2D vector (CHEAPER).
	static const float VectorLengthSquared(const Vector2 & Vec1);

	//Calculates the squared length of a 3D vector (CHEAPER).
	static const float VectorLengthSquared(const Vector3 & Vec1);

	//Returns a unit-length 2D vector based on the input.
	static const Vector2 Normalize(const Vector2 & Vec1);

	//Returns a unit-length 3D vector based on the input.
	static const Vector3 Normalize(const Vector3 & Vec1);

	//Calculates the dot product of two 2D vectors.
	static const float Dot(const Vector2 & Vec1, const Vector2 & Vec2);

	//Calculates the dot product of two 3D vectors.
	static const float Dot(const Vector3 & Vec1, const Vector3 & Vec2);

	//Calculates the cross product of two 3D vectors.
	static const Vector3 Cross(const Vector3 & Vec1, const Vector3 & Vec2);

	static const Vector2 AbsVector(const Vector2 & Vec1);

	static const Vector3 AbsVector(const Vector3 & Vec1);

	//////////////////////////////////////////////////////////////
	//						LOGIC								//
	//////////////////////////////////////////////////////////////

	static const bool IsNearlyEqual(float A, float Target, float Tolerance);

	static const bool IsInRange(float A, float Min, float Max);

	static const bool IsVectorEqual(const Vector2 & A, const Vector2 & B);

	static const bool IsVectorEqual(const Vector3 & A, const Vector3 & B);

	static const bool IsVectorNearlyEqual(const Vector2 & A, const Vector2 & Target, float Tolerance);

	static const bool IsVectorNearlyEqual(const Vector3 & A, const Vector3 & Target, float Tolerance);

	//Returns true if all of Vector A's components are bigger than B's.
	static const bool AreVectorComponentsGreater(const Vector3 & A, const Vector3 & B);

	//////////////////////////////////////////////////////////////
	//						MATRIX MATH							//
	//////////////////////////////////////////////////////////////

	static const Matrix4 Translate(const Vector3 & Vector);

	static const Matrix4 Rotate(const Quat & A);
};