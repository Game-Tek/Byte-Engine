#include "GSM.hpp"

#include "SIMD/float4.h"

float GSM::LengthSquared(const Vector2& _A)
{
	float4 a(_A.X, _A.Y, 0.0f, 0.0f);
	return float4::DotProduct(a, a).GetX();
}

float GSM::LengthSquared(const Vector3& _A)
{
	float4 a(_A.X, _A.Y, _A.Z, 0.0f);
	return float4::DotProduct(a, a).GetX();
}

float GSM::LengthSquared(const Vector4& _A)
{
	float4 a(_A.X, _A.Y, _A.Z, _A.W);
	return float4::DotProduct(a, a).GetX();
}

Vector2 GSM::Normalized(const Vector2& _A)
{
	float4 a(_A.X, _A.Y, 0.0f, 0.0f);
	const float4 length(Length(_A));
	a /= length;
	alignas(16) float vector[4];
	a.CopyToAlignedData(vector);

	return Vector2(vector[0], vector[1]);
}

void GSM::Normalize(Vector2& _A)
{
	float4 a(_A.X, _A.Y,0.0f, 0.0f);
	const float4 length(Length(_A));
	a /= length;
	alignas(16) float vector[4];
	a.CopyToAlignedData(vector);
	_A.X = vector[0];
	_A.Y = vector[1];
}

Vector3 GSM::Normalized(const Vector3& _A)
{
	float4 a(_A.X, _A.Y, _A.Z, 0.0f);
	const float4 length(Length(_A));
	a /= length;
	alignas(16) float vector[4];
	a.CopyToAlignedData(vector);

	return Vector3(vector[0], vector[1], vector[2]);
}

void GSM::Normalize(Vector3& _A)
{
	float4 a(_A.X, _A.Y, _A.Z, 0.0f);
	const float4 length(Length(_A));
	a /= length;
	alignas(16) float Vector[4];
	a.CopyToAlignedData(Vector);
	_A.X = Vector[0];
	_A.Y = Vector[1];
	_A.Z = Vector[2];
}

Vector4 GSM::Normalized(const Vector4& _A)
{
	alignas(16) Vector4 result;
	float4 a(&_A.X);
	const float4 length(Length(_A));
	a /= length;
	a.CopyToAlignedData(&result.X);
	return result;
}

void GSM::Normalize(Vector4& _A)
{
	float4 a(&_A.X);
	const float4 length(Length(_A));
	a /= length;
	a.CopyToUnalignedData(&_A.X);
}

float GSM::Dot(const Vector2& _A, const Vector2& _B)
{
	return float4::DotProduct(float4(_A.X, _A.Y, 0.0f, 0.0f), float4(_B.X, _B.Y, 0.0f, 0.0f)).GetX();
}

float GSM::Dot(const Vector3& _A, const Vector3& _B)
{
	return float4::DotProduct(float4(_A.X, _A.Y, _A.Z, 0.0f), float4(_B.X, _B.Y, _B.Z, 0.0f)).GetX();
}

float GSM::Dot(const Vector4& _A, const Vector4& _B)
{
	return float4::DotProduct(float4(_A.X, _A.Y, _A.Z, _A.W), float4(_B.X, _B.Y, _B.Z, _A.W)).GetX();
}

Vector3 GSM::Cross(const Vector3& _A, const Vector3& _B)
{
	alignas(16) float vector[4];

	float4 a(_A.X, _A.Y, _A.Z, 0.0f);
	float4 b(_B.X, _B.Y, _B.Z, 0.0f);

	float4 res = float4::Shuffle<3, 0, 2, 1>(a, a) * float4::Shuffle<3, 1, 0, 2>(b, b) - float4::Shuffle<3, 0, 2, 1>(a, a) * float4::Shuffle<3, 0, 2, 1>(b, b);
	res.CopyToAlignedData(vector);

	return Vector3(vector[0], vector[1], vector[2]);
}

real GSM::Dot(const Quaternion& _A, const Quaternion& _B)
{
	return float4::DotProduct(float4(_A.X, _A.Y, _A.Z, _A.Q), float4(_B.X, _B.Y, _B.Z, _A.Q)).GetX();
}

Quaternion GSM::Normalized(const Quaternion& _A)
{
	alignas(16) Quaternion result;
	float4 a(&_A.X);
	const float4 length(Length(_A));
	a /= length;
	a.CopyToAlignedData(&result.X);
	return result;
}

void GSM::Normalize(Quaternion& _A)
{
	float4 a(&_A.X);
	const float4 length(Length(_A));
	a /= length;
	a.CopyToUnalignedData(&_A.X);
}
