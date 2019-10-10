#include "Quaternion.h"

#include "SIMD/float4.h"

Quaternion Quaternion::operator*(const Quaternion& _Other) const
{
	float4 thi(&X);
	float4 other(&_Other.X);

	float4 wzyx(thi.Shuffle(thi, 0, 1, 2, 3));
	float4 baba(other.Shuffle(other, 0, 1, 0, 1));
	float4 dcdc(other.Shuffle(other, 2, 3, 2, 3));

	float4 ZnXWY = float4(thi * baba).HorizontalSub(wzyx * dcdc);

	float4 XZYnW = float4(thi * dcdc).HorizontalAdd(wzyx * baba);

	float4 XZWY(XZYnW.Shuffle(ZnXWY, 3, 2, 1, 0));
	XZWY = XZWY.Add13Sub02(ZnXWY.Shuffle(XZYnW, 2, 3, 0, 1));

	float4 res(XZWY.Shuffle(XZWY, 2, 1, 3, 0));

	alignas(16) Quaternion result;
	res.CopyToAlignedData(&result.X);

	return result;
}

Quaternion& Quaternion::operator*=(const Quaternion& _Other)
{
	float4 thi(&X);
	float4 other(&_Other.X);

	float4 wzyx(thi.Shuffle(thi, 0, 1, 2, 3));
	float4 baba(other.Shuffle(other, 0, 1, 0, 1));
	float4 dcdc(other.Shuffle(other, 2, 3, 2, 3));

	float4 ZnXWY = float4(thi * baba).HorizontalSub(wzyx * dcdc);

	float4 XZYnW = float4(thi * dcdc).HorizontalAdd(wzyx * baba);

	float4 XZWY(XZYnW.Shuffle(ZnXWY, 3, 2, 1, 0));
	XZWY = XZWY.Add13Sub02(ZnXWY.Shuffle(XZYnW, 2, 3, 0, 1));

	float4 res(XZWY.Shuffle(XZWY, 2, 1, 3, 0));

	res.CopyToUnalignedData(&X);

	return *this;
}
