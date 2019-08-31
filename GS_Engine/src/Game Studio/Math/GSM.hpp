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
	0.01745241, 0.0348995,  0.05233596, 0.06975647, 0.08715574, 0.10452846, 0.12186934, 0.1391731,  0.15643447, 0.17364818,
	0.190809,   0.20791169, 0.22495105, 0.2419219,  0.25881905, 0.27563736, 0.2923717,  0.30901699, 0.32556815, 0.34202014,
	0.35836795, 0.37460659, 0.39073113, 0.40673664, 0.42261826, 0.43837115, 0.4539905,  0.46947156, 0.48480962, 0.5,
	0.51503807, 0.52991926, 0.54463904, 0.5591929,  0.57357644, 0.58778525, 0.60181502, 0.61566148, 0.62932039, 0.64278761,
	0.65605903, 0.66913061, 0.68199836, 0.69465837, 0.70710678, 0.7193398,  0.7313537,  0.74314483, 0.75470958, 0.76604444,
	0.77714596, 0.78801075, 0.79863551, 0.80901699, 0.81915204, 0.82903757, 0.83867057, 0.8480481,  0.8571673,  0.8660254,
	0.87461971, 0.88294759, 0.89100652, 0.89879405, 0.90630779, 0.91354546, 0.92050485, 0.92718385, 0.93358043, 0.93969262,
	0.94551858, 0.95105652, 0.95630476, 0.9612617,  0.96592583, 0.97029573, 0.97437006, 0.9781476,  0.98162718, 0.98480775,
	0.98768834, 0.99026807, 0.99254615, 0.9945219,  0.9961947,  0.99756405, 0.99862953, 0.99939083, 0.9998477,  1.00000,
	0.9998477,  0.99939083, 0.99862953, 0.99756405, 0.9961947,  0.9945219,  0.99254615, 0.99026807, 0.98768834, 0.98480775,
	0.98162718, 0.9781476,  0.97437006, 0.97029573, 0.96592583, 0.9612617,  0.95630476, 0.95105652, 0.94551858, 0.93969262,
	0.93358043, 0.92718385, 0.92050485, 0.91354546, 0.90630779, 0.89879405, 0.89100652, 0.88294759, 0.87461971, 0.8660254,
	0.8571673,  0.8480481,  0.83867057, 0.82903757, 0.81915204, 0.80901699, 0.79863551, 0.78801075, 0.77714596, 0.76604444,
	0.75470958, 0.74314483, 0.7313537,  0.7193398,  0.70710678, 0.69465837, 0.68199836, 0.66913061, 0.65605903, 0.64278761,
	0.62932039, 0.61566148, 0.60181502, 0.58778525, 0.57357644, 0.5591929,  0.54463904, 0.52991926, 0.51503807, 0.5,
	0.48480962, 0.46947156, 0.4539905,  0.43837115, 0.42261826, 0.40673664, 0.39073113, 0.37460659, 0.35836795, 0.34202014,
	0.32556815, 0.30901699, 0.2923717,  0.27563736, 0.25881905, 0.2419219,  0.22495105, 0.20791169, 0.190809,   0.17364818,
	0.15643447, 0.1391731,  0.12186934, 0.10452846, 0.08715574, 0.06975647, 0.05233596, 0.0348995,  0.01745241 };

	//Increments by 1
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

	//Increments by 0.05
	static constexpr float ArcSinTable[] = {
	0.00000,
	2.86598398, 5.73917048,  8.62692656,  11.53695903, 14.47751219, 17.45760312, 20.48731511, 23.57817848, 26.74368395, 30.00000,
	33.36701297, 36.86989765, 40.54160187, 44.427004,   48.59037789, 53.13010235, 58.21166938, 64.15806724, 71.80512766,
	90.00000,
	};

	//Increments by 0.1
	static constexpr float AtanTable[] = {

	0.00000,
	5.71059314,  11.30993247, 16.69924423, 21.80140949, 26.56505118,
	30.96375653, 34.9920202,  38.65980825, 41.9872125,  45.00000,
	47.72631099, 50.19442891, 52.43140797, 54.46232221, 56.30993247,
	57.99461679, 59.53445508, 60.9453959,  62.2414594,  63.43494882,
	64.53665494, 65.55604522, 66.50143432, 67.38013505, 68.19859051,
	68.96248897, 69.67686317, 70.34617594, 70.97439396, 71.56505118,
	72.1213034,  72.64597536, 73.14160123, 73.61045967, 74.0546041,
	74.475889,   74.87599269, 75.25643716, 75.61860541, 75.96375653,
	76.293039,   76.60750225, 76.90810694, 77.19573393, 77.47119229,
	77.73522627, 77.98852161, 78.23171107, 78.46537935, 78.69006753,
	78.90627699, 79.11447295, 79.3150876,  79.50852299, 79.69515353,
	79.87532834, 80.04937331, 80.21759297, 80.3802722,  80.53767779, //ArcTan of 6
	80.69005983, 80.83765295, 80.98067757, 81.11934085, 81.25383774,
	81.38435182, 81.51105612, 81.63411388, 81.75367919, 81.86989765,
	81.98290693, 82.0928373,  82.19981212, 82.30394828, 82.40535663,
	82.50414236, 82.60040534, 82.69424047, 82.78573796, 82.87498365, // ArcTan of 8
	82.96205924, 83.04704253, 83.13000769, 83.21102543, 83.29016319,
	83.36748538, 83.4430535,  83.51692631, 83.58915998, 83.65980825,
	83.72892255, 83.7965521,  83.86274405, 83.92754359, 83.99099404,
	84.05313695, 84.11401217, 84.17365797, 84.2321111,  84.28940686,
	84.34557918, 84.40066066, 84.45468269, 84.50767544, 84.55966797,
	84.61068824, 84.6607632,  84.70991879, 84.75818005, 84.80557109,
	84.85211518, 84.89783475, 84.94275147, 84.98688624, 85.03025927,
	85.07289005, 85.11479743, 85.15599962, 85.19651424, 85.23635831 // ArcTan of 12
	};

	inline static uint8 RandUseCount = 0;
	static constexpr uint32 RandTable[] = { 542909189, 241292975, 485392319, 280587594, 22564577, 131346666, 540115444, 163133756, 7684350, 906455780 };
	inline static uint8 FloatRandUseCount = 0;
	static constexpr float FloatRandTable[] = { 0.7406606394, 0.8370865161, 0.3390759540, 0.4997499184, 0.0598975500, 0.1089056913, 0.3401726208, 0.2333399466, 0.3234475486, 0.2359271793 };

	INLINE static float Sin(const float Degrees)
	{
		const uint8 a = Floor(Degrees);

		return Lerp(SinTable[a], SinTable[a + 1], Degrees - a);
	}

	INLINE static float Tan(const float Degrees)
	{
		const uint8 a = Floor(Degrees);

		return Lerp(TanTable[a], TanTable[a + 1], Degrees - a);
	}

	INLINE static float ASin(float Degrees)
	{
		Degrees *= 20.0f;

		const uint8 a = Floor(Degrees);

		return Lerp(ArcSinTable[a], ArcSinTable[a + 1], Degrees - a);
	}

	INLINE static float ACos(float Degrees)
	{
		Degrees *= 20.0f;

		const uint8 a = Floor(Degrees);

		return Lerp(ArcSinTable[a], ArcSinTable[a + 1], Degrees - a);
	}

	INLINE static float ATan(float Degrees)
	{
		Degrees *= 10.0f;

		const uint8 a = Floor(Degrees);

		return Lerp(AtanTable[a], AtanTable[a + 1], Degrees - a);
	}

	INLINE static float StraightRaise(const float A, const uint8 Times)
	{
		float Result = A;

		for (uint8 i = 0; i < Times - 1; i++)
		{
			Result *= A;
		}

		return Result;
	}

public:
	static constexpr float PI = 3.1415926535f;
	static constexpr float e = 2.718281828459045235360;

	INLINE static int64 Random()
	{
		int64 ret = RandTable[RandUseCount];

		ret = RandUseCount % 1 == 1 ? ret * -1 : ret;

		RandUseCount = (RandUseCount + 1) % 10;

		return ret;
	}

	INLINE static int64 Random(int64 _Min, int64 _Max)
	{
		return (Random() % _Min) % _Max;
	}

	INLINE static float fRandom()
	{
		float ret = FloatRandTable[FloatRandUseCount];

		ret = FloatRandUseCount % 1 == 1 ? ret * -1 : ret;

		RandUseCount = (RandUseCount + 1) % 10;

		return ret;
	}

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

	INLINE static uint32 Fact(const int8 A)
	{
		uint8 Result = 1;

		for (uint8 i = 1; i < A + 1; i++)
		{
			Result *= (i + 1);
		}

		return Result;
	}

	//Returns the sine of an angle.
	INLINE static float Sine(const float Degrees)
	{
		float abs = Abs(Degrees);

		float Result = 0.0f;

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

	//Returns the ArcSine. MUST BE BETWEEN 0 AND 1.
	INLINE static float ArcSine(const float A)
	{
		if (A > 0.0f)
		{
			return ASin(A);
		}
		else
		{
			return -ASin(Abs(A));
		}
	}

	INLINE static float ArcCosine(const float A)
	{
		if (A > 0.0f)
		{
			return 90.0f - ASin(1.0f - A);
		}
		else
		{
			return 90.0f + ASin(Abs(A));
		}
	}

	//Returns the arctangent of the number. MUST BE BETWEEN 0 AND 12.
	INLINE static float ArcTangent(const float A)
	{
		if (A > 0.0f)
		{
			return ATan(A);
		}
		else
		{
			return -ATan(Abs(A));
		}
	}

	INLINE static float ArcTan2(const float X, const float Y)
	{
		return ArcTangent(Y / X);
	}

	INLINE static float Power(const float A, const float Times)
	{
		const float Timesplus = StraightRaise(A, Floor(Times));

		return Lerp(Timesplus, Timesplus * Times, Times - Floor(Times));
	}

	//////////////////////////////////////////////////////////////
	//						SCALAR MATH							//
	//////////////////////////////////////////////////////////////

		//Returns 1 if A is bigger than 0, 0 if A is equal to 0, and -1 if A is less than 0.
	INLINE static int8 Sign(const int32 A)
	{
		if (A > 0)
		{
			return 1;
		}
		else if (A < 0)
		{
			return -1;
		}
		else
		{
			return 0;
		}
	}

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
		float Y = 1.0f;
		float e = 0.000001f; /*e determines the level of accuracy*/
		
		while (X - Y > e)
		{
			X = (X + Y) / 2.0f;
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

	INLINE static int32 Min(const int32 A, const int32 B)
	{
		return (A < B) ? A : B;
	}

	INLINE static int32 Max(const int32 A, const int32 B)
	{
		return (A > B) ? A : B;
	}

	INLINE static float Min(const float A, const float B)
	{
		return (A < B) ? A : B;
	}

	INLINE static float Max(const float A, const float B)
	{
		return (A > B) ? A : B;
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

	INLINE static float RadiansToDegrees(const float Radians)
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

	INLINE static float VectorLength(const Vector4 & Vec1)
	{
		return SquareRoot(Vec1.X * Vec1.X + Vec1.Y * Vec1.Y + Vec1.Z * Vec1.Z + Vec1.W * Vec1.W);
	}

	INLINE static float VectorLengthSquared(const Vector2 & Vec1)
	{
		return Vec1.X * Vec1.X + Vec1.Y * Vec1.Y;
	}

	INLINE static float VectorLengthSquared(const Vector3 & Vec1)
	{
		return Vec1.X * Vec1.X + Vec1.Y * Vec1.Y + Vec1.Z * Vec1.Z;
	}

	INLINE static Vector2 Normalized(const Vector2 & Vec1)
	{
		const float Length = VectorLength(Vec1);
		return Vector2(Vec1.X / Length, Vec1.Y / Length);
	}

	INLINE static void Normalize(Vector2 & Vec1)
	{
		const float Length = VectorLength(Vec1);

		Vec1.X = Vec1.X / Length;
		Vec1.Y = Vec1.Y / Length;
	}

	INLINE static Vector3 Normalized(const Vector3 & Vec1)
	{
		const float Length = VectorLength(Vec1);
		return Vector3(Vec1.X / Length, Vec1.Y / Length, Vec1.Z / Length);
	}

	INLINE static void Normalize(Vector3 & Vec1)
	{
		const float Length = VectorLength(Vec1);

		Vec1.X = Vec1.X / Length;
		Vec1.Y = Vec1.Y / Length;
		Vec1.Z = Vec1.Z / Length;
	}

	INLINE static Vector4 Normalized(const Vector4 & Vec1)
	{
		const float Length = VectorLength(Vec1);
		return Vector4(Vec1.X / Length, Vec1.Y / Length, Vec1.Z / Length, Vec1.W / Length);
	}

	INLINE static void Normalize(Vector4 & Vec1)
	{
		const float Length = VectorLength(Vec1);

		Vec1.X = Vec1.X / Length;
		Vec1.Y = Vec1.Y / Length;
		Vec1.Z = Vec1.Z / Length;
		Vec1.W = Vec1.W / Length;
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

	INLINE static Vector2 Negated(const Vector2 & Vec)
	{
		Vector2 Result;

		Result.X = -Vec.X;
		Result.Y = -Vec.Y;

		return Result;
	}

	INLINE static void Negate(Vector2 & Vec)
	{
		Vec.X = -Vec.X;
		Vec.Y = -Vec.Y;

		return;
	}

	INLINE static Vector3 Negated(const Vector3 & Vec)
	{
		Vector3 Result;

		Result.X = -Vec.X;
		Result.Y = -Vec.Y;
		Result.Z = -Vec.Z;

		return Result;
	}

	INLINE static void Negate(Vector3 & Vec)
	{
		Vec.X = -Vec.X;
		Vec.Y = -Vec.Y;
		Vec.Z = -Vec.Z;

		return;
	}

	INLINE static Vector4 Negated(const Vector4 & Vec)
	{
		Vector4 Result;

		Result.X = -Vec.X;
		Result.Y = -Vec.Y;
		Result.Z = -Vec.Z;
		Result.W = -Vec.W;

		return Result;
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

	INLINE static Quaternion Normalized(const Quaternion & Quat)
	{
		const float Length = QuaternionLength(Quat);

		return Quaternion(Quat.X / Length, Quat.Y / Length, Quat.Z / Length, Quat.Q / Length);
	}

	INLINE static void Normalize(Quaternion & Quat)
	{
		const float Length = QuaternionLength(Quat);

		Quat.X = Quat.X / Length;
		Quat.Y = Quat.Y / Length;
		Quat.Z = Quat.Z / Length;
		Quat.Q = Quat.Q / Length;
	}

	INLINE static Quaternion Conjugated(const Quaternion & Quat)
	{
		return Quaternion(-Quat.X, -Quat.Y, -Quat.Z, Quat.Q);
	}

	INLINE static void Conjugate(Quaternion & Quat)
	{
		Quat.X = -Quat.X;
		Quat.Y = -Quat.Y;
		Quat.Z = -Quat.Z;

		return;
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

	INLINE static bool IsVectorNearlyEqual(const Vector2 & A, const Vector2 & Target, const float Tolerance)
	{
		return IsNearlyEqual(A.X, Target.X, Tolerance) && IsNearlyEqual(A.Y, Target.Y, Tolerance);
	}

	INLINE static bool IsVectorNearlyEqual(const Vector3 & A, const Vector3 & Target, const float Tolerance)
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

	INLINE static void Scale(Matrix4 & A, const Vector3 & B)
	{
		A[0] = B.X;
		A[5] = B.Y;
		A[10] = B.Z;

		return;
	}

	INLINE static Matrix4 Scaling(const Vector3 & A)
	{
		Matrix4 Result;

		Result[0] = A.X;
		Result[5] = A.Y;
		Result[10] = A.Z;

		return Result;
	}

};