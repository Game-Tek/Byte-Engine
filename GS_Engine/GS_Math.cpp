#include "GS_Math.h"
#include <math.h>

//////////////////////////////////////////////////////////////
//						SCALAR MATH							//
//////////////////////////////////////////////////////////////

float GSMath::Lerp(float A, float B, float Alpha)
{
	return A + Alpha * (B - A);
}

float GSMath::FInterp(float Target, float Current, float DT, float InterpSpeed)
{
	return (((Target - Current) * DT) * InterpSpeed) + Current;
}

float GSMath::MapToRange(float A, float InMin, float InMax, float OutMin, float OutMax)
{
	return InMin + ((OutMax - OutMin) / (InMax - InMin)) * (A - InMin);
}

int Round(float A)
{
	int Truncated = (int)A

	if ((A - Truncated) > 0.5f)
	{
		return Truncated + 1;
	}

	else
	{
		return Truncated;
	}
}

//////////////////////////////////////////////////////////////
//						VECTOR MATH							//
//////////////////////////////////////////////////////////////

Vector2 GSMath::Add(const Vector2 & Vec1, const Vector2 & Vec2)
{
	return { Vec1.X + Vec2.X, Vec1.Y + Vec2.Y };
}

Vector2 GSMath::operator+(const Vector2 & Vec1, const Vector2 & Vec2)
{
	return { Vec1.X + Vec2.X, Vec1.Y + Vec2.Y };
}

Vector3 GSMath::Add(const Vector3 & Vec1, const Vector3 & Vec2)
{
	return { Vec1.X + Vec2.X, Vec1.Y + Vec2.Y, Vec1.Z + Vec2.Z };
}

Vector3 GSMath::operator+(const Vector3 & Vec1, const Vector3 & Vec2)
{
	return { Vec1.X + Vec2.X, Vec1.Y + Vec2.Y, Vec1.Z + Vec2.Z };
}

Vector2 GSMath::Subtract(const Vector2 & Vec1, const Vector2 & Vec2)
{
	return { Vec1.X - Vec2.X, Vec1.Y - Vec2.Y };
}

Vector3 GSMath::Subtract(const Vector3 & Vec1, const Vector3 & Vec2)
{
	return { Vec1.X - Vec2.X, Vec1.Y - Vec2.Y, Vec1.Z - Vec2.Z };
}

Vector3 GSMath::operator-(const Vector3 & Vec1, const Vector3 & Vec2)
{
	return { Vec1.X - Vec2.X, Vec1.Y - Vec2.Y, Vec1.Z - Vec2.Z };
}

Vector2 GSMath::Multiply(const Vector2 & Vec1, float B)
{
	return { Vec1.X * B, Vec1.Y * B };
}

Vector2 GSMath::operator*(const Vector2 & Vec1, float B)
{
	return { Vec1.X * B, Vec1.Y * B };
}

Vector2 GSMath::Multiply(const Vector2 & Vec1, const Vector2 & Vec2)
{
	return { Vec1.X * Vec2.X, Vec1.Y * Vec2.Y };
}

Vector2 GSMath::operator*(const Vector2 & Vec1, const Vector2 & Vec2)
{
	return { Vec1.X * Vec2.X, Vec1.Y * Vec2.Y };
}

Vector3 GSMath::Multiply(const Vector3 & Vec1, float B)
{
	return { Vec1.X * B, Vec1.Y * B, Vec1.Z * B };
}

Vector3 GSMath::operator*(const Vector3 & Vec1, float B)
{
	return { Vec1.X * B, Vec1.Y * B, Vec1.Z * B };
}

Vector3 GSMath::Multiply(const Vector3 & Vec1, const Vector3 & Vec2)
{
	return { Vec1.X * Vec2.X, Vec1.Y * Vec2.Y, Vec1.Z * Vec2.Z };
}

Vector3 GSMath::operator*(const Vector3 & Vec1, const Vector3 & Vec2)
{
	return { Vec1.X * Vec2.X, Vec1.Y * Vec2.Y, Vec1.Z * Vec2.Z };
}

float GSMath::VectorLength(const Vector2 & Vec1)
{
	return sqrt(Vec1.X * Vec1.X + Vec1.Y * Vec1.Y);
}

float GSMath::VectorLength(const Vector3 & Vec1)
{
	return sqrt(Vec1.X * Vec1.X + Vec1.Y * Vec1.Y + Vec1.Z * Vec1.Z);
}

float GSMath::VectorLengthSquared(const Vector2 & Vec1)
{
	return Vec1.X * Vec1.X + Vec1.Y * Vec1.Y;
}

float GSMath::VectorLengthSquared(const Vector3 & Vec1)
{
	return Vec1.X * Vec1.X + Vec1.Y * Vec1.Y + Vec1.Z * Vec1.Z;
}

Vector2 GSMath::Normalize(const Vector2 & Vec1)
{
	float length = VectorLength(Vec1);
	return { Vec1.X / length, Vec1.Y / length };
}

Vector3 GSMath::Normalize(const Vector3 & Vec1)
{
	float length = VectorLength(Vec1);
	return { Vec1.X / length, Vec1.Y / length, Vec1.Z / length };
}

float GSMath::Dot(const Vector2 & Vec1, const Vector2 & Vec2)
{
	return Vec1.X * Vec2.X + Vec1.Y * Vec2.Y;
}

float GSMath::Dot(const Vector3 & Vec1, const Vector3 & Vec2)
{
	return Vec1.X * Vec2.X + Vec1.Y * Vec2.Y + Vec1.Z * Vec2.Z;
}

Vector3 GSMath::Cross(const Vector3 & Vec1, const Vector3 & Vec2)
{
	return { Vec1.Y * Vec2.Z - Vec1.Z * Vec2.Y, Vec1.Z * Vec2.X - Vec1.X * Vec2.Z, Vec1.X * Vec2.Y - Vec1.Y * Vec2.X };
}

Vector2 GSMath::AbsVector(const Vector2 & Vec1)
{
	return { abs(Vec1.X), abs(Vec1.Y) };
}

Vector3 GSMath::AbsVector(const Vector3 & Vec1)
{
	return { abs(Vec1.X), abs(Vec1.Y), abs(Vec1.Z) };
}







//////////////////////////////////////////////////////////////
//						ROTATOR MATH						//
//////////////////////////////////////////////////////////////







//////////////////////////////////////////////////////////////
//						LOGIC								//
//////////////////////////////////////////////////////////////

bool GSMath::IsNearlyEqual(float A, float Target, float Tolerance)
{
	return A > Target - Tolerance && A < Target + Tolerance;
}

bool GSMath::IsInRange(float A, float Min, float Max)
{
	return A > Min && A < Max;
}

bool GSMath::IsVectorEqual(const Vector2 & A, const Vector2 & B)
{
	return A.X == B.X && A.Y == B.Y;
}

bool operator=(const Vector2 & A, const Vector2 & B)
{
	return A.X == B.X && A.Y == B.Y;
}

bool GSMath::IsVectorEqual(const Vector3 & A, const Vector3 & B)
{
	return A.X == B.X && A.Y == B.Y && A.Z == B.Z;
}

bool operator=(const Vector3 & A, const Vector3 & B)
{
	return A.X == B.X && A.Y == B.Y && A.Z == B.Z;
}

bool GSMath::IsVectorNearlyEqual(const Vector2 & A, const Vector2 & Target, float Tolerance)
{
	return IsNearlyEqual(A.X, Target.X, Tolerance) && IsNearlyEqual(A.Y, Target.Y, Tolerance);
}

bool GSMath::IsVectorNearlyEqual(const Vector3 & A, const Vector3 & Target, float Tolerance)
{
	return IsNearlyEqual(A.X, Target.X, Tolerance) && IsNearlyEqual(A.Y, Target.Y, Tolerance) && IsNearlyEqual(A.Z, Target.Z, Tolerance);
}

bool GSMath::AreVectorComponentsGreater(const Vector3 & A, const Vector3 & B)
{
	return A.X > B.X && A.Y > B.Y && A.Z > B.Z;
}