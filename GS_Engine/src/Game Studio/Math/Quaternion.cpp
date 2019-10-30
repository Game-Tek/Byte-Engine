#include "Quaternion.h"

#include "SIMD/float4.h"

Quaternion Quaternion::operator*(const Quaternion& _Other) const
{
	auto thi = float4::MakeFromUnaligned(&X);
	auto other = float4::MakeFromUnaligned(&_Other.X);

	float4 wzyx(float4::Shuffle<0, 1, 2, 3>(thi, thi));
	float4 baba(float4::Shuffle<0, 1, 0, 1>(other, other));
	float4 dcdc(float4::Shuffle<2, 3, 2, 3>(other, other));

	auto ZnXWY = float4::HorizontalSub(thi * baba, wzyx * dcdc);

	auto XZYnW = float4::HorizontalAdd(thi * dcdc, wzyx * baba);

	float4 XZWY(float4::Shuffle<3, 2, 1, 0>(XZYnW, ZnXWY));
	XZWY = float4::Add13Sub02(XZWY, float4::Shuffle<2, 3, 0, 1>(ZnXWY, XZYnW));

	float4 res(float4::Shuffle<2, 1, 3, 0>(XZWY, XZWY));

	alignas(16) Quaternion result;
	res.CopyToAlignedData(&result.X);

	return result;
}

Quaternion& Quaternion::operator*=(const Quaternion& _Other)
{
	auto thi = float4::MakeFromUnaligned(&X);
	auto other = float4::MakeFromUnaligned(&_Other.X);
	
	float4 wzyx(float4::Shuffle<0, 1, 2, 3>(thi, thi));
	float4 baba(float4::Shuffle<0, 1, 0, 1>(other, other));
	float4 dcdc(float4::Shuffle<2, 3, 2, 3>(other, other));
	
	float4 ZnXWY = float4::HorizontalSub(thi * baba,wzyx * dcdc);
	
	float4 XZYnW = float4::HorizontalAdd(thi * dcdc, wzyx * baba);
	
	float4 XZWY(float4::Shuffle<3, 2, 1, 0>(XZYnW, ZnXWY));
	XZWY = float4::Add13Sub02(XZWY, float4::Shuffle<2, 3, 0, 1>(ZnXWY, XZYnW));
	
	float4 res(float4::Shuffle<2, 1, 3, 0>(XZWY, XZWY));
	
	res.CopyToUnalignedData(&X);

	return *this;
}
