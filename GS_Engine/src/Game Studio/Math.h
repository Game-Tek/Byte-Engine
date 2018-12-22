#pragma once

#include "DataTypes.h"
#include "Matrix4x4.h"

namespace GS
{
	//Mixes A and B by the specified values, Where Alpha 0 returns A and Alpha 1 returns B.
	static float Lerp(float A, float B, float Alpha);

	//Interpolates from Current to Target, returns Current + an amount determined by the InterpSpeed.
	static float FInterp(float Target, float Current, float DT, float InterpSpeed);

	static float MapToRange(float A, float AMin, float AMax, float RangeMin, float RangeMax);

	//////////////////////////////////////////////////////////////
	//						VECTOR MATH							//
	//////////////////////////////////////////////////////////////

	//Calculates the length of a 2D vector.
	static float VectorLength(const Vector2 &Vec1);

	//Calculates the length of a 3D vector.
	static float VectorLength(const Vector3 &Vec1);

	//Calculates the squared length of a 2D vector (CHEAPER).
	static float VectorLengthSquared(const Vector2 &Vec1);

	//Calculates the squared length of a 3D vector (CHEAPER).
	static float VectorLengthSquared(const Vector3 &Vec1);

	//Returns a unit-length 2D vector based on the input.
	static Vector2 Normalize(const Vector2 &Vec1);

	//Returns a unit-length 3D vector based on the input.
	static Vector3 Normalize(const Vector3 &Vec1);

	//Calculates the dot product of two 2D vectors.
	static float Dot(const Vector2 &Vec1, const Vector2 &Vec2);

	//Calculates the dot product of two 3D vectors.
	static float Dot(const Vector3 &Vec1, const Vector3 &Vec2);

	//Calculates the cross product of two 3D vectors.
	static Vector3 Cross(const Vector3 &Vec1, const Vector3 &Vec2);

	static Vector2 AbsVector(const Vector2 & Vec1);

	static Vector3 AbsVector(const Vector3 & Vec1);

	//////////////////////////////////////////////////////////////
	//						LOGIC								//
	//////////////////////////////////////////////////////////////

	static bool IsNearlyEqual(float A, float Target, float Tolerance);

	static bool IsInRange(float A, float Min, float Max);

	static bool IsVectorEqual(const Vector2 & A, const Vector2 & B);

	static bool IsVectorEqual(const Vector3 & A, const Vector3 & B);

	static bool IsVectorNearlyEqual(const Vector2 & A, const Vector2 & Target, float Tolerance);

	static bool IsVectorNearlyEqual(const Vector3 & A, const Vector3 & Target, float Tolerance);

	//Returns true if all of Vector A's components are bigger than B's.
	static bool AreVectorComponentsGreater(const Vector3 & A, const Vector3 & B);

	//////////////////////////////////////////////////////////////
	//						MATRIX MATH							//
	//////////////////////////////////////////////////////////////

	Matrix4x4 Translate(const Vector3 & Vector);
	Matrix4x4 Rotate(const Quat & A);
};

