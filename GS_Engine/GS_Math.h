#pragma once

#include "GS_DataTypes.h"
#include "math.h"

namespace GSMath
{
	public:

	//Mixes A and B by the specified values, Where Alpha 0 returns A and Alpha 1 returns B.
	float Lerp(float A, float B, float Alpha);

	//Interpolates from Current to Target, returns Current + an amount determined by the InterpSpeed.
	float FInterp(float Target, float Current, float DT, float InterpSpeed);

	float MapToRange(float A, float AMin, float AMax, float RangeMin, float RangeMax);

	//////////////////////////////////////////////////////////////
	//						VECTOR MATH							//
	//////////////////////////////////////////////////////////////

	//Adds two 2D vectors together.
	Vector2 Add(const Vector2 &Vec1, const Vector2 &Vec2);

	//Adds two 3D vectors together.
	Vector3 Add(const Vector3 &Vec1, const Vector3 &Vec2);

	//Subtracts two 2D vectors.
	Vector2 Subtract(const Vector2 &Vec1, const Vector2 &Vec2);

	//Subtracts two 3D vectors.
	Vector3 Subtract(const Vector3 &Vec1, const Vector3 &Vec2);

	//Multiplies a 2D vector by a scalar value.
	Vector2 Multiply(const Vector2 &Vec1, float B);

	//Multiplies two 2D vectors.
	Vector2 Multiply(const Vector2 &Vec1, const Vector2 &Vec2);

	//Multiplies a 3D vector by a scalar value.
	Vector3 Multiply(const Vector3 &Vec1, float B);

	//Multiplies two 3D vectors.
	Vector3 Multiply(const Vector3 &Vec1, const Vector3 &Vec2);

	//Calculates the length of a 2D vector.
	float VectorLength(const Vector2 &Vec1);

	//Calculates the length of a 3D vector.
	float VectorLength(const Vector3 &Vec1);

	//Calculates the squared length of a 2D vector (CHEAPER).
	float VectorLengthSquared(const Vector2 &Vec1);

	//Calculates the squared length of a 3D vector (CHEAPER).
	float VectorLengthSquared(const Vector3 &Vec1);

	//Returns a unit-length 2D vector based on the input.
	Vector2 Normalize(const Vector2 &Vec1);

	//Returns a unit-length 3D vector based on the input.
	Vector3 Normalize(const Vector3 &Vec1);

	//Calculates the dot product of two 2D vectors.
	float Dot(const Vector2 &Vec1, const Vector2 &Vec2);

	//Calculates the dot product of two 3D vectors.
	float Dot(const Vector3 &Vec1, const Vector3 &Vec2);

	//Calculates the cross product of two 3D vectors.
	Vector3 Cross(const Vector3 &Vec1, const Vector3 &Vec2);

	Vector2 AbsVector(const Vector2 & Vec1);

	Vector3 AbsVector(const Vector3 & Vec1);

	//////////////////////////////////////////////////////////////
	//						LOGIC								//
	//////////////////////////////////////////////////////////////

	bool IsNearlyEqual(float A, float Target, float Tolerance);

	bool IsInRange(float A, float Min, float Max);

	bool IsVectorEqual(const Vector2 & A, const Vector2 & B);

	bool IsVectorEqual(const Vector3 & A, const Vector3 & B);

	bool IsVectorNearlyEqual(const Vector2 & A, const Vector2 & Target, float Tolerance);

	bool IsVectorNearlyEqual(const Vector3 & A, const Vector3 & Target, float Tolerance);

	//Returns true if all of Vector A's components are bigger than B's.
	bool AreVectorComponentsGreater(const Vector3 & A, const Vector3 & B)
};