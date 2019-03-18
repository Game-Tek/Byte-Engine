#pragma once

#include "Core.h"

#include "Vector2.h"
#include "Vector3.h"
#include "Vector4.h"

#include "Quaternion.h"
#include "Matrix4.h"

GS_CLASS GSM
{
private:
	static constexpr float SinTable[] = {	0.00000,
	0.01745, 0.03490, 0.05234, 0.06976, 0.08716, 0.10453, 0.12187, 0.13917, 0.15643, 0.17365,
	0.19081, 0.20791, 0.22495, 0.24192, 0.25882, 0.27564, 0.29237, 0.30902, 0.32557, 0.34202,
	0.35837, 0.37461, 0.39073, 0.40674, 0.42262, 0.43837, 0.45399, 0.46947, 0.48481, 0.5,
	0.51504, 0.52992, 0.54464, 0.55919, 0.57358, 0.58779, 0.60182, 0.61566, 0.62932, 0.64279,
	0.65606, 0.66913, 0.68200, 0.69466, 0.70711, 0.71934, 0.73135, 0.74314, 0.75471, 0.76604,
	0.77715, 0.78801, 0.79864, 0.80902, 0.81915, 0.82904, 0.83867, 0.84805, 0.85717, 0.86603,
	0.87462, 0.88295, 0.89101, 0.89879, 0.90631, 0.91355, 0.92050, 0.92718, 0.93358, 0.93969,
	0.94552, 0.95106, 0.95630, 0.96126, 0.96593, 0.97030, 0.97437, 0.97815, 0.98163, 0.98481,
	0.98769, 0.99027, 0.99255, 0.99452, 0.99619, 0.99756, 0.99863, 0.99939, 0.99985, 1.00000,
	0.99985, 0.99939, 0.99863, 0.99756, 0.99619, 0.99452, 0.99255, 0.99027, 0.98769, 0.98481,
	0.98163, 0.97815, 0.97437, 0.97030, 0.96593, 0.96126, 0.95630, 0.95106, 0.94552, 0.93969,
	0.93358, 0.92718, 0.92050, 0.91355, 0.90631, 0.89879, 0.89101, 0.88295, 0.87462, 0.86603,
	0.85717, 0.84805, 0.83867, 0.82904, 0.81915, 0.80902, 0.79864, 0.78801, 0.77715, 0.76604,
	0.75471, 0.74314, 0.73135, 0.71934, 0.70711, 0.69466, 0.68200, 0.66913, 0.65606, 0.64279,
	0.62932, 0.61566, 0.60182, 0.58779, 0.57358, 0.55919, 0.54464, 0.52992, 0.51504, 0.50000,
	0.48481, 0.46947, 0.45399, 0.43837, 0.42262, 0.40674, 0.39073, 0.37461, 0.35837, 0.34202,
	0.32557, 0.30902, 0.29237, 0.27564, 0.25882, 0.24192, 0.22495, 0.20791, 0.19081, 0.17365,
	0.15643, 0.13917, 0.12187, 0.10453, 0.08716, 0.06976, 0.05234, 0.03490, 0.01745, };

	static constexpr float TanTable[] = {  0.00000,
	0.01745506492, 0.03492076949, 0.05240777928, 0.06992681194, 0.08748866352,
	0.10510423526, 0.1227845609,  0.1405408347,  0.15838444032, 0.1763269807,
	0.19438030913, 0.21255656167, 0.23086819112, 0.24932800284, 0.26794919243,
	0.28674538575, 0.30573068145, 0.32491969623, 0.34432761329, 0.36397023426,
	0.38386403503, 0.40402622583, 0.42447481621, 0.4452286853,  0.46630765815,
	0.48773258856, 0.50952544949, 0.53170943166, 0.55430905145, 0.57735026919,
	0.60086061902, 0.6248693519,  0.64940759319, 0.67450851684, 0.70020753821,
	0.726542528,   0.7535540501,  0.7812856265,  0.80978403319, 0.83909963117,
	0.86928673781, 0.90040404429, 0.93251508613, 0.9656887748,  1.00000,
	1.03553031379, 1.07236871002, 1.11061251483, 1.15036840722, 1.19175359259,
	1.23489715654, 1.27994163219, 1.32704482162, 1.37638192047, 1.42814800674,
	1.48256096851, 1.53986496381, 1.60033452904, 1.66427948235, 1.73205080757,
	1.80404775527, 1.88072646535, 1.96261050551, 2.05030384158, 2.14450692051,
	2.2460367739,  2.35585236582, 2.47508685342, 2.60508906469, 2.74747741945,
	2.90421087768, 3.07768353718, 3.27085261848, 3.48741444384, 3.73205080757,
	4.01078093354, 4.33147587428, 4.70463010948, 5.14455401597, 5.67128181962,
	6.31375151468, 6.31375151468, 8.14434642797, 9.51436445422, 11.4300523028,
	14.3006662567, 19.0811366877, 28.6362532829, 57.2899616308, 1000.00000 };

	INLINE static float Sin(const float Degrees)
	{
		int a = Floor(Degrees);
		int b = a + 1;

		return Lerp(SinTable[a], SinTable[b], Degrees - a);
	}

	INLINE static float Tan(const float Degrees)
	{
		int a = Floor(Degrees);
		int b = a + 1;

		return Lerp(TanTable[a], TanTable[b], Degrees - a);
	}

	INLINE static double CoTangent(const float Degrees)
	{
		return 1.0 / Tangent(Degrees);
	}

	INLINE static float StraightRaise(float A, uint8 Times)
	{
		for (uint8 i = 0; i < Times - 1; i++)
		{
			A *= A;
		}

		return A;
	}

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

	//Returns the sine of an angle.
	INLINE static float Sine(const float Degrees)
	{
		float abs = Abs(Degrees);

		float Result;

		if (Modulo(abs, 360.0f) > 180.0f)
		{
			Result = -Sin(Modulo(abs, 180.0f));
		}
		else
		{
			Result = Sin(Modulo(abs, 180.0f));
		}

		return (Degrees > 0.0f) ? Result : -Result;
	}

	//Returns the cosine of an angle.
	INLINE static float Cosine(const float Degrees)
	{
		return Sine(Degrees + 90.0f);
	}

	//Returns the tangent of an angle.
	INLINE static float Tangent(const float Degrees)
	{
		if (Degrees > 0.0f)
		{
			return Tan(Degrees);
		}
		else
		{
			return -Tan(Abs(Degrees));
		}
	}

	INLINE static float ArcTangent(const float Degrees)
	{
		return CoTangent(1.0 / Degrees);
	}

	INLINE static float Power(const float A, const float Times)
	{
		const float Timesplus = StraightRaise(A, Floor(Times));

		return Lerp(Timesplus, Timesplus * Timesplus, Times - Floor(Times));
	}

	//////////////////////////////////////////////////////////////
	//						SCALAR MATH							//
	//////////////////////////////////////////////////////////////

	//Returns 1 if A is bigger than 0, 0 if A is equal to 0, and -1 if A is less than 0.
	INLINE static int32 Sign(const float A)
	{
		if (A > 0.0f)
		{
			return 1;
		}
		else if (A < 0.0f)
		{
			return -1;
		}
		else
		{
			return 0;
		}
	}

	//Mixes A and B by the specified values, Where Alpha 0 returns A and Alpha 1 returns B.
	INLINE static float Lerp(const float A, const float B, const float Alpha)
	{
		return A + Alpha * (B - A);
	}

	//Interpolates from Current to Target, returns Current + an amount determined by the InterpSpeed.
	INLINE static float FInterp(const float Target, const float Current, const float DT, const float InterpSpeed)
	{
		return (((Target - Current) * DT) * InterpSpeed) + Current;
	}

	INLINE static float MapToRange(const float A, const float InMin, const float InMax, const float OutMin, const float OutMax)
	{
		return InMin + ((OutMax - OutMin) / (InMax - InMin)) * (A - InMin);
	}

	INLINE static float obMapToRange(const float A, const float InMax, const float OutMax)
	{
		return A / (InMax / OutMax);
	}

	INLINE static float SquareRoot(const float A)
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

	INLINE static uint32 Abs(const int32 A)
	{
		return A > 0 ? A : -A;
	}

	INLINE static float Abs(const float A)
	{
		return A > 0.0f ? A : -A;
	}

	template<typename T>
	INLINE static T Min(const T & A, const T & B)
	{
		return (A < B) ? A : B;
	}

	template<typename T>
	INLINE static T Max(const T & A, const T & B)
	{
		return (A > B) ? A : B;
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

	//Creates a translation matrix.
	INLINE static Matrix4 Translation(const Vector3 & Vector)
	{
		Matrix4 Result;

		Result[0 + 3 * 4] = Vector.X;
		Result[1 + 3 * 4] = Vector.Y;
		Result[2 + 3 * 4] = Vector.Z;

		return Result;
	}

	//Modifies the given matrix to make it a translation matrix.
	INLINE static void Translate(Matrix4 & Matrix, const Vector3 & Vector)
	{
		Matrix[0 + 3 * 4] = Vector.X;
		Matrix[1 + 3 * 4] = Vector.Y;
		Matrix[2 + 3 * 4] = Vector.Z;

		return;
	}

	INLINE static void Rotate(Matrix4 & A, const Quaternion & Q)
	{
		const float cos = Cosine(Q.Q);
		const float sin = Sine(Q.Q);
		const float omc = 1.0f - cos;

		A[0] = Q.X * omc + cos;
		A[1] = Q.Y * Q.X * omc - Q.Y * sin;
		A[2] = Q.X * Q.Z * omc - Q.Y * sin;

		A[4] = Q.X * Q.Y * omc - Q.Z * sin;
		A[5] = Q.Y * omc + cos;
		A[6] = Q.Y * Q.Z * omc + Q.X * sin;

		A[8] = Q.X * Q.Z * omc + Q.Y * sin;
		A[9] = Q.Y * Q.Z * omc - Q.X * sin;
		A[10] = Q.Z * omc + cos;
	}

	INLINE static Matrix4 Rotation(const Quaternion & A)
	{
		Matrix4 Result;

		const float cos = Cosine(A.Q);
		const float sin = Sine(A.Q);
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