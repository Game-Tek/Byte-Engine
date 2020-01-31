#pragma once

#include "Core.h"

#include "Vector2.h"
#include "Vector3.h"
#include "Vector4.h"

#include "Quaternion.h"
#include "Matrix4.h"

#include "Transform3.h"
#include "Plane.h"
#include "Rotator.h"

class GSM
{
	// +  float
	// += float
	// +  type
	// += type
	// -  float
	// -= float
	// -  type
	// -= type
	// *  float
	// *= float
	// *  type
	// *= type
	// /  float
	// /= float
	// /  type
	// /= type

	inline static uint8 RandUseCount = 0;
	static constexpr uint32 RandTable[] = {
		542909189, 241292975, 485392319, 280587594, 22564577, 131346666, 540115444, 163133756, 7684350, 906455780
	};
	inline static uint8 FloatRandUseCount = 0;
	static constexpr double FloatRandTable[] = {
		0.7406606394, 0.8370865161, 0.3390759540, 0.4997499184, 0.0598975500, 0.1089056913, 0.3401726208, 0.2333399466,
		0.3234475486, 0.2359271793
	};

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
	static constexpr double PI = 3.141592653589793238462643383279502884197169399375105820974944592307816406286;
	static constexpr double e = 2.718281828459045235360287471352662497757247093699959574966967627724076630353;

	INLINE static int64 Random()
	{
		int64 ret = RandTable[RandUseCount];

		ret = RandUseCount % 2 == 1 ? ret * -1 : ret;

		RandUseCount = (RandUseCount + 1) % 10;

		return ret;
	}

	INLINE static int64 Random(int64 _Min, int64 _Max)
	{
		return Random() % (_Max - _Min + 1) + _Min;
	}

	INLINE static double fRandom()
	{
		auto ret = FloatRandTable[FloatRandUseCount];

		ret = FloatRandUseCount % 2 == 1 ? ret * -1 : ret;

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
		//return (C - Floor(C)) * B;
		return (C - static_cast<int>(C)) * B;
	}

	//INLINE static float Power(const float Base, const int32 Exp)
	//{
	//	if (Exp < 0)
	//	{
	//		if (Base == 0)
	//		{
	//			return -0; // Error!!
	//		}
	//
	//		return 1 / (Base * Power(Base, (-Exp) - 1));
	//	}
	//
	//	if (Exp == 0)
	//	{
	//		return 1;
	//	}
	//
	//	if (Exp == 1)
	//	{
	//		return Base;
	//	}
	//
	//	return Base * Power(Base, Exp - 1);
	//}

	INLINE static uint32 Fact(const int8 A)
	{
		uint8 Result = 1;

		for (uint8 i = 1; i < A + 1; i++)
		{
			Result *= (i + 1);
		}

		return Result;
	}

	static float Power(float x, float y);

	static float Log10(float x);
	
	//Returns the sine of an angle.
	static float Sine(float Degrees);

	//Returns the sine of an angle.
	static double Sine(double Degrees);

	//Returns the cosine of an angle.
	static float Cosine(float Degrees);

	//Returns the cosine of an angle.
	static double Cosine(double Degrees);

	//Returns the tangent of an angle. INPUT DEGREES MUST BE BETWEEN 0 AND 90.
	static float Tangent(float Degrees);

	//Returns the tangent of an angle. INPUT DEGREES MUST BE BETWEEN 0 AND 90.
	static double Tangent(double Degrees);

	//Returns the ArcSine. INPUT DEGREES MUST BE BETWEEN 0 AND 1.
	static float ArcSine(float A);

	static float ArcCosine(float A);

	//Returns the arctangent of the number. INPUT DEGREES MUST BE BETWEEN 0 AND 12.
	static float ArcTangent(float A);

	static float ArcTan2(float X, float Y);

	//////////////////////////////////////////////////////////////
	//						SCALAR MATH							//
	//////////////////////////////////////////////////////////////

	//Returns 1 if A is bigger than 0. 0 if A is equal to 0. and -1 if A is less than 0.
	INLINE static int8 Sign(const int_64 _A)
	{
		if (_A > 0)
		{
			return 1;
		}
		if (_A < 0)
		{
			return -1;
		}

		return 0;
	}

	//Returns 1 if A is bigger than 0. 0 if A is equal to 0. and -1 if A is less than 0.
	INLINE static int8 Sign(const float A)
	{
		if (A > 0.0)
		{
			return 1;
		}
		if (A < 0.0)
		{
			return -1;
		}

		return 0;
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

	INLINE static float MapToRange(const float A, const float InMin, const float InMax, const float OutMin,
	                               const float OutMax)
	{
		return InMin + ((OutMax - OutMin) / (InMax - InMin)) * (A - InMin);
	}

	INLINE static float obMapToRange(const float A, const float InMax, const float OutMax)
	{
		return A / (InMax / OutMax);
	}

	INLINE static float SquareRoot(float _A)
	{
		constexpr auto error = 0.00001; //define the precision of your result
		double s = _A;

		while (s - _A / s > error) //loop until precision satisfied 
		{
			s = (s + _A / s) / 2.0;
		}

		return static_cast<float>(s);
	}

	INLINE static double SquareRoot(double _A)
	{
		constexpr auto error = 0.00001; //define the precision of your result
		double s = _A;

		while (s - _A / s > error) //loop until precision satisfied 
		{
			s = (s + _A / s) / 2.0;
		}

		return s;
	}

	INLINE static float Root(const float _A, const float _Root)
	{
		return Power(_A, 1.0f / _Root);
	}

	INLINE static uint32 Abs(const int32 A)
	{
		return A > 0 ? A : -A;
	}

	INLINE static float Abs(const float _A)
	{
		return _A > 0.0f ? _A : -_A;
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

	template <typename T>
	INLINE static T Min(const T& A, const T& B)
	{
		return (A < B) ? A : B;
	}

	template <typename T>
	INLINE static T Max(const T& A, const T& B)
	{
		return (A > B) ? A : B;
	}

	INLINE static float DegreesToRadians(const float Degrees)
	{
		return Degrees * static_cast<float>(PI / 180.0);
	}

	INLINE static double DegreesToRadians(const double Degrees)
	{
		return Degrees * (PI / 180.0);
	}

	INLINE static float RadiansToDegrees(const float Radians)
	{
		return Radians * static_cast<float>(180.0 / PI);
	}

	INLINE static double RadiansToDegrees(const double Radians)
	{
		return Radians * (180.0 / PI);
	}

	//////////////////////////////////////////////////////////////
	//						VECTOR MATH							//
	//////////////////////////////////////////////////////////////

	//Calculates the length of a 2D vector.
	INLINE static float Length(const Vector2& _A)
	{
		return SquareRoot(LengthSquared(_A));
	}

	INLINE static float Length(const Vector2& _A, const Vector2& _B)
	{
		return SquareRoot(LengthSquared(_A - _B));
	}

	INLINE static float Length(const Vector3& _A)
	{
		return SquareRoot(LengthSquared(_A));
	}

	INLINE static float Length(const Vector3& _A, const Vector3& _B)
	{
		return SquareRoot(LengthSquared(_A - _B));
	}

	INLINE static float Length(const Vector4& _A)
	{
		return SquareRoot(LengthSquared(_A));
	}

	INLINE static float Length(const Vector4& _A, const Vector4& _B)
	{
		return SquareRoot(LengthSquared(_A - _B));
	}

	static float LengthSquared(const Vector2& _A);
	//{
	//	return Vec1.X * Vec1.X + Vec1.Y * Vec1.Y;
	//}

	static float LengthSquared(const Vector3& _A);
	//{
	//	return Vec1.X * Vec1.X + Vec1.Y * Vec1.Y + Vec1.Z * Vec1.Z;
	//}

	static float LengthSquared(const Vector4& _A);

	static Vector2 Normalized(const Vector2& _A);
	//{
	//	const float Length = VectorLength(Vec1);
	//	return Vector2(Vec1.X / Length, Vec1.Y / Length);
	//}

	static void Normalize(Vector2& _A);
	//{
	//	const float Length = VectorLength(Vec1);
	//
	//	Vec1.X = Vec1.X / Length;
	//	Vec1.Y = Vec1.Y / Length;
	//}

	static Vector3 Normalized(const Vector3& _A);
	//{
	//	const float Length = VectorLength(Vec1);
	//	return Vector3(Vec1.X / Length, Vec1.Y / Length, Vec1.Z / Length);
	//}

	static void Normalize(Vector3& _A);
	//{
	//	const float Length = VectorLength(Vec1);
	//
	//	Vec1.X = Vec1.X / Length;
	//	Vec1.Y = Vec1.Y / Length;
	//	Vec1.Z = Vec1.Z / Length;
	//}

	static Vector4 Normalized(const Vector4& _A);
	//{
	//	const float Length = VectorLength(Vec1);
	//	return Vector4(Vec1.X / Length, Vec1.Y / Length, Vec1.Z / Length, Vec1.W / Length);
	//}

	static void Normalize(Vector4& _A);
	//{
	//	const float Length = VectorLength(Vec1);
	//
	//	Vec1.X = Vec1.X / Length;
	//	Vec1.Y = Vec1.Y / Length;
	//	Vec1.Z = Vec1.Z / Length;
	//	Vec1.W = Vec1.W / Length;
	//}

	static float DotProduct(const Vector2& _A, const Vector2& _B);
	//{
	//	return Vec1.X * Vec2.X + Vec1.Y * Vec2.Y;
	//}

	static float DotProduct(const Vector3& _A, const Vector3& _B);
	//{
	//	return Vec1.X * Vec2.X + Vec1.Y * Vec2.Y + Vec1.Z * Vec2.Z;
	//}

	static float DotProduct(const Vector4& _A, const Vector4& _B);

	static Vector3 Cross(const Vector3& _A, const Vector3& _B);
	//{
	//	return Vector3(Vec1.Y * Vec2.Z - Vec1.Z * Vec2.Y, Vec1.Z * Vec2.X - Vec1.X * Vec2.Z, Vec1.X * Vec2.Y - Vec1.Y * Vec2.X);
	//}

	INLINE static Vector2 Abs(const Vector2& Vec1)
	{
		return Vector2(Abs(Vec1.X), Abs(Vec1.Y));
	}

	INLINE static Vector3 Abs(const Vector3& Vec1)
	{
		return Vector3(Abs(Vec1.X), Abs(Vec1.Y), Abs(Vec1.Z));
	}

	INLINE static Vector4 Abs(const Vector4& _A)
	{
		return Vector4(Abs(_A.X), Abs(_A.Y), Abs(_A.Z), Abs(_A.Z));
	}

	INLINE static Vector2 Negated(const Vector2& Vec)
	{
		Vector2 Result;

		Result.X = -Vec.X;
		Result.Y = -Vec.Y;

		return Result;
	}

	INLINE static void Negate(Vector2& Vec)
	{
		Vec.X = -Vec.X;
		Vec.Y = -Vec.Y;

		return;
	}

	INLINE static Vector3 Negated(const Vector3& Vec)
	{
		Vector3 Result;

		Result.X = -Vec.X;
		Result.Y = -Vec.Y;
		Result.Z = -Vec.Z;

		return Result;
	}

	INLINE static void Negate(Vector3& Vec)
	{
		Vec.X = -Vec.X;
		Vec.Y = -Vec.Y;
		Vec.Z = -Vec.Z;

		return;
	}

	INLINE static Vector4 Negated(const Vector4& Vec)
	{
		Vector4 Result;

		Result.X = -Vec.X;
		Result.Y = -Vec.Y;
		Result.Z = -Vec.Z;
		Result.W = -Vec.W;

		return Result;
	}

	INLINE static void Negate(Vector4& Vec)
	{
		Vec.X = -Vec.X;
		Vec.Y = -Vec.Y;
		Vec.Z = -Vec.Z;
		Vec.W = -Vec.W;

		return;
	}

	//////////////////////////////////////////////////////////////
	//						QUATERNION MATH						//
	//////////////////////////////////////////////////////////////

	static float DotProduct(const Quaternion& _A, const Quaternion& _B);
	//{
	//    return _A.X * _B.X + _A.Y * _B.Y + _A.Z * _B.Z + _A.Q * _B.Q;
	//}

	static float LengthSquared(const Quaternion& _A);

	INLINE static float Length(const Quaternion& _A)
	{
		return SquareRoot(LengthSquared(_A));
	}

	static Quaternion Normalized(const Quaternion& _A);
	//{
	//	const float lLength = Length(Quat);
	//
	//	return Quaternion(Quat.X / lLength, Quat.Y / lLength, Quat.Z / lLength, Quat.Q / lLength);
	//}

	static void Normalize(Quaternion& _A);
	//{
	//	const float lLength = Length(Quat);
	//
	//	Quat.X = Quat.X / lLength;
	//	Quat.Y = Quat.Y / lLength;
	//	Quat.Z = Quat.Z / lLength;
	//	Quat.Q = Quat.Q / lLength;
	//}

	INLINE static Quaternion Conjugated(const Quaternion& Quat)
	{
		return Quaternion(-Quat.X, -Quat.Y, -Quat.Z, Quat.Q);
	}

	INLINE static void Conjugate(Quaternion& Quat)
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

	INLINE static bool IsVectorNearlyEqual(const Vector2& A, const Vector2& Target, const float Tolerance)
	{
		return IsNearlyEqual(A.X, Target.X, Tolerance) && IsNearlyEqual(A.Y, Target.Y, Tolerance);
	}

	INLINE static bool IsVectorNearlyEqual(const Vector3& A, const Vector3& Target, const float Tolerance)
	{
		if (IsNearlyEqual(A.X, Target.X, Tolerance))
		{
			if (IsNearlyEqual(A.Y, Target.Y, Tolerance))
			{
				if (IsNearlyEqual(A.Z, Target.Z, Tolerance))
				{
					return true;
				}
			}
		}

		return false;
	}

	INLINE static bool AreVectorComponentsGreater(const Vector3& A, const Vector3& B)
	{
		return A.X > B.X && A.Y > B.Y && A.Z > B.Z;
	}

	//////////////////////////////////////////////////////////////
	//						MATRIX MATH							//
	//////////////////////////////////////////////////////////////

	//Creates a translation matrix.
	INLINE static Matrix4 Translation(const Vector3& Vector)
	{
		Matrix4 Result(1);

		Result(0, 3) = Vector.X;
		Result(1, 3) = Vector.Y;
		Result(2, 3) = Vector.Z;

		return Result;
	}

	//Modifies the given matrix to make it a translation matrix.
	INLINE static void Translate(Matrix4& Matrix, const Vector3& Vector)
	{
		const auto translation = Translation(Vector);

		Matrix *= translation;

		return;
	}

	INLINE static Matrix4 NormalToRotation(Vector3 normal)
	{
		// Find a vector in the plane
		Vector3 tangent0 = Cross(normal, Vector3(1, 0, 0));
		if (DotProduct(tangent0, tangent0) < 0.001)
			tangent0 = Cross(normal, Vector3(0, 1, 0));
		Normalize(tangent0);
		// Find another vector in the plane
		Vector3 tangent1 = Normalized(Cross(normal, tangent0));
		return Matrix4(tangent0.X, tangent0.Y, tangent0.Z, 0.0f, tangent1.X, tangent1.Y, tangent1.Z, 0.0f, normal.X,
		               normal.Y, normal.Z, 0.0f, 0, 0, 0, 0);
	}

	INLINE static void Rotate(Matrix4& A, const Quaternion& Q)
	{
		const auto rotation = Rotation(Q);

		A *= rotation;

		return;
	}

	INLINE static Vector3 SphericalCoordinatesToCartesianCoordinates(const Vector2& sphericalCoordinates)
	{
		auto cy = Cosine(sphericalCoordinates.Y);

		return Vector3(cy * Sine(sphericalCoordinates.X), Sine(sphericalCoordinates.Y),
		               cy * Cosine(sphericalCoordinates.X));
	}

	INLINE static Vector3 RotatorToNormalVector(const Rotator& rotator)
	{
		auto x = Cosine(rotator.Y) * Cosine(rotator.X);
		auto y = Sine(rotator.Y) * Cosine(rotator.X);
		auto z = Sine(rotator.X);

		return Vector3(x, y, z);
	}

	INLINE static Quaternion RotatorToQuaternion(const Rotator& rotator)
	{
		// Abbreviations for the various angular functions
		const auto cy = Cosine(rotator.Y * 0.5f);
		const auto sy = Sine(rotator.Y   * 0.5f);
		const auto cp = Cosine(rotator.X * 0.5f);
		const auto sp = Sine(rotator.X   * 0.5f);
		const auto cr = Cosine(rotator.Z * 0.5f);
		const auto sr = Sine(rotator.Z   * 0.5f);

		Quaternion result;
		result.X = sy * cp * sr + cy * sp * cr;
		result.Y = sy * cp * cr - cy * sp * sr;
		result.Z = cy * cp * sr - sy * sp * cr;
		result.Q = cy * cp * cr + sy * sp * sr;

		return result;
	}

	INLINE static Matrix4 Rotation(const Quaternion& A)
	{
		Matrix4 result(1);

		auto xx = A.X * A.X;
		auto xy = A.X * A.Y;
		auto xz = A.X * A.Z;
		auto xw = A.X * A.Q;
		auto yy = A.Y * A.Y;
		auto yz = A.Y * A.Z;
		auto yw = A.Y * A.Q;
		auto zz = A.Z * A.Z;
		auto zw = A.Z * A.Q;

		result(0, 0) = 1 - 2 * (yy + zz);
		result(0, 1) = 2 * (xy - zw);
		result(0, 2) = 2 * (xz + yw);
		result(1, 0) = 2 * (xy + zw);
		result(1, 1) = 1 - 2 * (xx + zz);
		result(1, 2) = 2 * (yz - xw);
		result(2, 0) = 2 * (xz - yw);
		result(2, 1) = 2 * (yz + xw);
		result(2, 2) = 1 - 2 * (xx + yy);
		result(0, 3) = result(1, 3) = result(2, 3) = result(3, 0) = result(3, 1) = result(3, 2) = 0;
		result(3, 3) = 1;

		return result;
	}

	INLINE static Matrix4 Rotation(const Vector3& A, float angle)
	{
		Matrix4 result(1);

		float c = Cosine(angle); // cosine
		float s = Sine(angle); // sine
		float xx = A.X * A.X;
		float xy = A.X * A.Y;
		float xz = A.X * A.Z;
		float yy = A.Y * A.Y;
		float yz = A.Y * A.Z;
		float zz = A.Z * A.Z;

		// build rotation matrix
		Matrix4 m;
		m[0] = xx * (1 - c) + c;
		m[1] = xy * (1 - c) - A.Z * s;
		m[2] = xz * (1 - c) + A.Y * s;
		m[3] = 0;
		m[4] = xy * (1 - c) + A.Z * s;
		m[5] = yy * (1 - c) + c;
		m[6] = yz * (1 - c) - A.X * s;
		m[7] = 0;
		m[8] = xz * (1 - c) - A.Y * s;
		m[9] = yz * (1 - c) + A.X * s;
		m[10] = zz * (1 - c) + c;
		m[11] = 0;
		m[12] = 0;
		m[13] = 0;
		m[14] = 0;
		m[15] = 1;

		return result;
	}

	INLINE static void Scale(Matrix4& A, const Vector3& B)
	{
		const auto scaling = Scaling(B);

		A *= scaling;
	}

	INLINE static Matrix4 Scaling(const Vector3& A)
	{
		Matrix4 Result;

		Result[0] = A.X;
		Result[5] = A.Y;
		Result[10] = A.Z;

		return Result;
	}

	INLINE static Matrix4 Transformation(const Transform3& _A)
	{
		Matrix4 Return;
		Translate(Return, _A.Position);
		//Rotate(Return, _A.Rotation);
		Scale(Return, _A.Scale);
		return Return;
	}

	INLINE static void Transform(Matrix4& _A, Transform3& _B)
	{
		Translate(_A, _B.Position);
		//Rotate(_A, _B.Rotation);
		Scale(_A, _B.Scale);
	}

	INLINE static float Clamp(float _A, float _Min, float _Max)
	{
		return _A > _Max ? _Max : _A < _Min ? _Min : _A;
	}

	INLINE static Vector3 ClosestPointOnPlane(const Vector3& _Point, const Plane& _Plane)
	{
		const float T = (DotProduct(_Plane.Normal, _Point) - _Plane.D) / DotProduct(_Plane.Normal, _Plane.Normal);
		return _Point - _Plane.Normal * T;
	}

	INLINE static double DistanceFromPointToPlane(const Vector3& _Point, const Plane& _Plane)
	{
		// return Dot(q, p.n) - p.d; if plane equation normalized (||p.n||==1)
		return (DotProduct(_Plane.Normal, _Point) - _Plane.D) / DotProduct(_Plane.Normal, _Plane.Normal);
	}

	INLINE static void ClosestPointOnLineSegmentToPoint(const Vector3& _C, const Vector3& _A, const Vector3& _B,
	                                                    float& _T, Vector3& _D)
	{
		const Vector3 AB = _B - _A;
		// Project c onto ab, computing parameterized position d(t) = a + t*(b – a)
		_T = DotProduct(_C - _A, AB) / DotProduct(AB, AB);
		// If outside segment, clamp t (and therefore d) to the closest endpoint
		if (_T < 0.0) _T = 0.0;
		if (_T > 1.0) _T = 1.0;
		// Compute projected position from the clamped t
		_D = _A + AB * _T;
	}

	INLINE static double SquaredDistancePointToSegment(const Vector3& _A, const Vector3& _B, const Vector3& _C)
	{
		const Vector3 AB = _B - _A;
		const Vector3 AC = _C - _A;
		const Vector3 BC = _C - _B;
		float E = DotProduct(AC, AB);
		// Handle cases where c projects outside ab
		if (E <= 0.0f) return DotProduct(AC, AC);
		float f = DotProduct(AB, AB);
		if (E >= f) return DotProduct(BC, BC);
		// Handle cases where c projects onto ab
		return DotProduct(AC, AC) - E * E / f;
	}

	INLINE static Vector3 ClosestPointOnTriangleToPoint(const Vector3& _A, const Vector3& _P1, const Vector3& _P2,
	                                                    const Vector3& _P3)
	{
		// Check if P in vertex region outside A
		const Vector3 AP = _A - _P1;
		const Vector3 AB = _P2 - _P1;
		const Vector3 AC = _P3 - _P1;

		const float D1 = DotProduct(AB, AP);
		const float D2 = DotProduct(AC, AP);
		if (D1 <= 0.0f && D2 <= 0.0f) return _P1; // barycentric coordinates (1,0,0)

		// Check if P in vertex region outside B
		const Vector3 BP = _A - _P2;
		const float D3 = DotProduct(AB, BP);
		const float D4 = DotProduct(AC, BP);
		if (D3 >= 0.0f && D4 <= D3) return _P2; // barycentric coordinates (0,1,0)

		// Check if P in edge region of AB, if so return projection of P onto AB
		const float VC = D1 * D4 - D3 * D2;
		if (VC <= 0.0f && D1 >= 0.0f && D3 <= 0.0f)
		{
			const float V = D1 / (D1 - D3);
			return _P1 + AB * V; // barycentric coordinates (1-v,v,0)
		}

		// Check if P in vertex region outside C
		const Vector3 CP = _A - _P3;
		const float D5 = DotProduct(AB, CP);
		const float D6 = DotProduct(AC, CP);
		if (D6 >= 0.0f && D5 <= D6) return _P3; // barycentric coordinates (0,0,1)

		// Check if P in edge region of AC, if so return projection of P onto AC
		const float VB = D5 * D2 - D1 * D6;
		if (VB <= 0.0f && D2 >= 0.0f && D6 <= 0.0f)
		{
			const float W = D2 / (D2 - D6);
			return _P1 + AC * W; // barycentric coordinates (1-w,0,w)
		}

		// Check if P in edge region of BC, if so return projection of P onto BC
		float VA = D3 * D6 - D5 * D4;
		if (VA <= 0.0f && (D4 - D3) >= 0.0f && (D5 - D6) >= 0.0f)
		{
			const float W = (D4 - D3) / ((D4 - D3) + (D5 - D6));
			return _P2 + (_P3 - _P2) * W; // barycentric coordinates (0,1-w,w)
		}

		// P inside face region. Compute Q through its barycentric coordinates (u,v,w)
		const float Denom = 1.0f / (VA + VB + VC);
		const float V = VB * Denom;
		const float W = VC * Denom;
		return _P1 + AB * V + AC * W; // = u*a + v*b + w*c, u = va * denom = 1.0f - v - w
	}

	INLINE static bool PointOutsideOfPlane(const Vector3& p, const Vector3& a, const Vector3& b, const Vector3& c)
	{
		return DotProduct(p - a, Cross(b - a, c - a)) >= 0.0f; // [AP AB AC] >= 0
	}

	INLINE static bool PointOutsideOfPlane(const Vector3& p, const Vector3& a, const Vector3& b, const Vector3& c,
	                                       const Vector3& d)
	{
		const float signp = DotProduct(p - a, Cross(b - a, c - a)); // [AP AB AC]
		const float signd = DotProduct(d - a, Cross(b - a, c - a)); // [AD AB AC]
		// Points on opposite sides if expression signs are opposite
		return signp * signd < 0.0f;
	}

	INLINE static Vector3 ClosestPtPointTetrahedron(const Vector3& p, const Vector3& a, const Vector3& b,
	                                                const Vector3& c, const Vector3& d)
	{
		// Start out assuming point inside all halfspaces, so closest to itself
		Vector3 ClosestPoint = p;
		float BestSquaredDistance = 3.402823466e+38F;

		// If point outside face abc then compute closest point on abc
		if (PointOutsideOfPlane(p, a, b, c))
		{
			const Vector3 q = ClosestPointOnTriangleToPoint(p, a, b, c);
			const float sqDist = DotProduct(q - p, q - p);
			// Update best closest point if (squared) distance is less than current best
			if (sqDist < BestSquaredDistance) BestSquaredDistance = sqDist, ClosestPoint = q;
		}

		// Repeat test for face acd
		if (PointOutsideOfPlane(p, a, c, d))
		{
			const Vector3 q = ClosestPointOnTriangleToPoint(p, a, c, d);
			const float sqDist = DotProduct(q - p, q - p);
			if (sqDist < BestSquaredDistance) BestSquaredDistance = sqDist, ClosestPoint = q;
		}

		// Repeat test for face adb
		if (PointOutsideOfPlane(p, a, d, b))
		{
			const Vector3 q = ClosestPointOnTriangleToPoint(p, a, d, b);
			const float sqDist = DotProduct(q - p, q - p);
			if (sqDist < BestSquaredDistance) BestSquaredDistance = sqDist, ClosestPoint = q;
		}

		// Repeat test for face bdc
		if (PointOutsideOfPlane(p, b, d, c))
		{
			const Vector3 q = ClosestPointOnTriangleToPoint(p, b, d, c);
			const float sqDist = DotProduct(q - p, q - p);
			if (sqDist < BestSquaredDistance) BestSquaredDistance = sqDist, ClosestPoint = q;
		}

		return ClosestPoint;
	}

	INLINE static void SinCos(float* sp, float* cp, float degrees)
	{
		*sp = Sine(degrees);
		*cp = Cosine(degrees);
	}
};
