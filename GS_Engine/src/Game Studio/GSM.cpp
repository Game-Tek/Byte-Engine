#include "GSM.hpp"

#define TERMS 6

const static float PI = 3.1415926535f;

//INLINE STATIC	


float Mod(float A, float B)
{
	//https://www.geeksforgeeks.org/modulus-two-float-double-numbers/

	// Handling negative values 
	if (A < 0)
		A = -A;
	if (B < 0)
		B = -B;

	// Finding mod by repeated subtraction 
	float Mod = A;
	while (Mod >= B)
	{
		Mod = Mod - B;
	}

	// Sign of result typically depends 
	// on sign of a. 
	if (A < 0)
	{
		return -Mod;
	}

	return Mod;
}

float Power(float Base, int Exp)
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

int Fact(int A)
{
	return A <= 0 ? 1 : A * Fact(A - 1);
}

float Sine(float Degrees)
{
	Mod(Degrees, 360); // make it less than 360
	float rad = Degrees * PI / 180;
	float sin = 0;

	int i;
	for (i = 0; i < TERMS; i++)
	{ // That's Taylor series!!
		sin += Power(-1, i) * Power(rad, 2 * i + 1) / Fact(2 * i + 1);
	}
	return sin;
}

float Cosine(float Degrees) {
	Mod(Degrees, 360); // make it less than 360
	float rad = Degrees * PI / 180;
	float cos = 0;

	int i;
	for (i = 0; i < TERMS; i++) { // That's also Taylor series!!
		cos += Power(-1, i) * Power(rad, 2 * i) / Fact(2 * i);
	}
	return cos;
}


//////////////////////////////////////////////////////////////
//						SCALAR MATH							//
//////////////////////////////////////////////////////////////

const float GSM::Lerp(float A, float B, float Alpha)
{
	return A + Alpha * (B - A);
}

const float GSM::FInterp(float Target, float Current, float DT, float InterpSpeed)
{
	return (((Target - Current) * DT) * InterpSpeed) + Current;
}

const float GSM::MapToRange(float A, float InMin, float InMax, float OutMin, float OutMax)
{
	return InMin + ((OutMax - OutMin) / (InMax - InMin)) * (A - InMin);
}

const float GSM::SquareRoot(float A)
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

const float GSM::Abs(float A)
{
	return A > 0 ? A : -A;
}

const int GSM::Round(float A)
{
	int Truncated = (int)A;

	if ((A - Truncated) > 0.5f)
	{
		return Truncated + 1;
	}

	else
	{
		return Truncated;
	}
}

const float GSM::DegreesToRadians(float Degrees)
{
	return Degrees * PI / 180;
}

const float GSM::RadiansToDegrees(float Radians)
{
	return Radians * 180 / PI;
}

//////////////////////////////////////////////////////////////
//						VECTOR MATH							//
//////////////////////////////////////////////////////////////

const float GSM::VectorLength (const Vector2 & Vec1)
{
	return SquareRoot(Vec1.X * Vec1.X + Vec1.Y * Vec1.Y);
}

const float GSM::VectorLength (const Vector3 & Vec1)
{
	return SquareRoot(Vec1.X * Vec1.X + Vec1.Y * Vec1.Y + Vec1.Z * Vec1.Z);	
}

const float GSM::VectorLengthSquared(const Vector2 & Vec1)
{
	return Vec1.X * Vec1.X + Vec1.Y * Vec1.Y;
}

const float GSM::VectorLengthSquared(const Vector3 & Vec1)
{
	return Vec1.X * Vec1.X + Vec1.Y * Vec1.Y + Vec1.Z * Vec1.Z;
}

const Vector2 GSM::Normalize(const Vector2 & Vec1)
{
	float Length = VectorLength(Vec1);
	return { Vec1.X / Length, Vec1.Y / Length };
}

const Vector3 GSM::Normalize(const Vector3 & Vec1)
{
	float Length = VectorLength(Vec1);
	return { Vec1.X / Length, Vec1.Y / Length, Vec1.Z / Length };
}

const float GSM::Dot(const Vector2 & Vec1, const Vector2 & Vec2)
{
	return Vec1.X * Vec2.X + Vec1.Y * Vec2.Y;
}

const float GSM::Dot(const Vector3 & Vec1, const Vector3 & Vec2)
{
	return Vec1.X * Vec2.X + Vec1.Y * Vec2.Y + Vec1.Z * Vec2.Z;
}

const Vector3 GSM::Cross(const Vector3 & Vec1, const Vector3 & Vec2)
{
	return { Vec1.Y * Vec2.Z - Vec1.Z * Vec2.Y, Vec1.Z * Vec2.X - Vec1.X * Vec2.Z, Vec1.X * Vec2.Y - Vec1.Y * Vec2.X };
}

const Vector2 GSM::AbsVector(const Vector2 & Vec1)
{
	return { Abs(Vec1.X), Abs(Vec1.Y) };
}

const Vector3 GSM::AbsVector(const Vector3 & Vec1)
{
	return { Abs(Vec1.X), Abs(Vec1.Y), Abs(Vec1.Z) };
}


//////////////////////////////////////////////////////////////
//						ROTATOR MATH						//
//////////////////////////////////////////////////////////////







//////////////////////////////////////////////////////////////
//						LOGIC								//
//////////////////////////////////////////////////////////////

const bool GSM::IsNearlyEqual(float A, float Target, float Tolerance)
{
	return A > Target - Tolerance && A < Target + Tolerance;
}

const bool GSM::IsInRange(float A, float Min, float Max)
{
	return A > Min && A < Max;
}

const bool GSM::IsVectorEqual(const Vector2 & A, const Vector2 & B)
{
	return A.X == B.X && A.Y == B.Y;
}

const bool GSM::IsVectorEqual(const Vector3 & A, const Vector3 & B)
{
	return A.X == B.X && A.Y == B.Y && A.Z == B.Z;
}

const bool GSM::IsVectorNearlyEqual(const Vector2 & A, const Vector2 & Target, float Tolerance)
{
	return IsNearlyEqual(A.X, Target.X, Tolerance) && IsNearlyEqual(A.Y, Target.Y, Tolerance);
}

const bool GSM::IsVectorNearlyEqual(const Vector3 & A, const Vector3 & Target, float Tolerance)
{
	return IsNearlyEqual(A.X, Target.X, Tolerance) && IsNearlyEqual(A.Y, Target.Y, Tolerance) && IsNearlyEqual(A.Z, Target.Z, Tolerance);
}

const bool GSM::AreVectorComponentsGreater(const Vector3 & A, const Vector3 & B)
{
	return A.X > B.X && A.Y > B.Y && A.Z > B.Z;
}

//////////////////////////////////////////////////////////////
//						MATRIX MATH							//
//////////////////////////////////////////////////////////////

const Matrix4 GSM::Translate(const Vector3 & Vector)
{
	Matrix4 Result;

	Result[0 + 3 * 4] = Vector.X;
	Result[1 + 3 * 4] = Vector.Y;
	Result[2 + 3 * 4] = Vector.Z;

	return Result;
}

const Matrix4 GSM::Rotate(const Quat & A)
{
	Matrix4 Result;
	Result.Identity();

	float r = DegreesToRadians(A.Q);
	float cos = Cosine(r);
	float sin = Sine(r);
	float omc = 1.0f - cos;

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
