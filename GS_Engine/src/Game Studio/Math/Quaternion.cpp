#include "Quaternion.h"

#include "SIMD/float4.h"
#include "GSM.hpp"

//CODE IS CORRECT
Quaternion::Quaternion(const Rotator& rotator)
{
	// Abbreviations for the various angular functions
	const auto cy = GSM::Cosine(rotator.Y * 0.5f);
	const auto sy = GSM::Sine(rotator.Y   * 0.5f);
	const auto cp = GSM::Cosine(rotator.X * 0.5f);
	const auto sp = GSM::Sine(rotator.X   * 0.5f);
	const auto cr = GSM::Cosine(rotator.Z * 0.5f);
	const auto sr = GSM::Sine(rotator.Z   * 0.5f);

	X = sy * cp * cr - cy * sp * sr;
	Y = sy * cp * sr + cy * sp * cr;
	Z = cy * cp * sr - sy * sp * cr;
	Q = cy * cp * cr + sy * sp * sr;
}

Quaternion Quaternion::operator*(const Quaternion& other) const
{
	//auto thi = float4::MakeFromUnaligned(&X);
	//auto other = float4::MakeFromUnaligned(&other.X);
	//
	//float4 wzyx(float4::Shuffle<0, 1, 2, 3>(thi, thi));
	//float4 baba(float4::Shuffle<0, 1, 0, 1>(other, other));
	//float4 dcdc(float4::Shuffle<2, 3, 2, 3>(other, other));
	//
	//auto ZnXWY = float4::HorizontalSub(thi * baba, wzyx * dcdc);
	//
	//auto XZYnW = float4::HorizontalAdd(thi * dcdc, wzyx * baba);
	//
	//float4 XZWY(float4::Shuffle<3, 2, 1, 0>(XZYnW, ZnXWY));
	//XZWY = float4::Add13Sub02(XZWY, float4::Shuffle<2, 3, 0, 1>(ZnXWY, XZYnW));
	//
	//float4 res(float4::Shuffle<2, 1, 3, 0>(XZWY, XZWY));
	//
	//alignas(16) Quaternion result;
	//res.CopyToAlignedData(&result.X);
	//
	//return result;
	//
	Quaternion result;

	result.X = Q * other.X + X * other.Q + Y * other.Z - Z * other.Y;
	result.Y = Q * other.Y + Y * other.Q + Z * other.X - X * other.Z;
	result.Z = Q * other.Z + Z * other.Q + X * other.Y - Y * other.X;
	result.Q = Q * other.Q - X * other.X - Y * other.Y - Z * other.Z;

	return result;
}

Quaternion& Quaternion::operator*=(const Quaternion& other)
{
	//auto thi = float4::MakeFromUnaligned(&X);
	//auto other = float4::MakeFromUnaligned(&other.X);
	//
	//float4 wzyx(float4::Shuffle<0, 1, 2, 3>(thi, thi));
	//float4 baba(float4::Shuffle<0, 1, 0, 1>(other, other));
	//float4 dcdc(float4::Shuffle<2, 3, 2, 3>(other, other));
	//
	//float4 ZnXWY = float4::HorizontalSub(thi * baba,wzyx * dcdc);
	//
	//float4 XZYnW = float4::HorizontalAdd(thi * dcdc, wzyx * baba);
	//
	//float4 XZWY(float4::Shuffle<3, 2, 1, 0>(XZYnW, ZnXWY));
	//XZWY = float4::Add13Sub02(XZWY, float4::Shuffle<2, 3, 0, 1>(ZnXWY, XZYnW));
	//
	//float4 res(float4::Shuffle<2, 1, 3, 0>(XZWY, XZWY));
	//
	//res.CopyToUnalignedData(&X);
	//
	//return *this;
	//
	//X = X * other.Q + Y * other.Z - Z * other.Y + Q * other.X;
	//Y = -X * other.Z + Y * other.Q + Z * other.X + Q * other.Y;
	//Z = X * other.Y - Y * other.X + Z * other.Q + Q * other.Z;
	//Q = -X * other.X - Y * other.Y - Z * other.Z + Q * other.Q;

	//((lhs.w * rhs.x) + (lhs.x * rhs.w) + (lhs.y * rhs.z) - (lhs.z * rhs.y),
	//(lhs.w * rhs.y) + (lhs.y * rhs.w) + (lhs.z * rhs.x) - (lhs.x * rhs.z),
	//(lhs.w * rhs.z) + (lhs.z * rhs.w) + (lhs.x * rhs.y) - (lhs.y * rhs.x),
	//(lhs.w * rhs.w) - (lhs.x * rhs.x) - (lhs.y * rhs.y) - (lhs.z * rhs.z));

	//X = Q * other.X + X * other.Q + Y * other.Z - Z * other.Y;
	//Y = Q * other.Y + Y * other.Q + Z * other.X - X * other.Z;
	//Z = Q * other.Z + Z * other.Q + X * other.Y - Y * other.X;
	//Q = Q * other.Q - X * other.X - Y * other.Y - Z * other.Z;

	*this = *this * other;
	return *this;
}
