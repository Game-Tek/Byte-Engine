#include "AxisAngle.h"

#include "GSM.hpp"

#include "Vector3.h"
#include "Quaternion.h"
#include <Math\SIMD\float4.h>

constexpr AxisAngle::AxisAngle(const Vector3& vector, const float angle) :
	X(vector.X), Y(vector.Y), Z(vector.Z), Angle(angle)
{
}

AxisAngle::AxisAngle(const Quaternion& quaternion) : Angle(2.0f * GSM::ArcCosine(quaternion.Q))
{
	auto components = float4::MakeFromUnaligned(reinterpret_cast<float*>(&const_cast<Quaternion&>(quaternion)));
	float4 sqrt(1 - quaternion.Q * quaternion.Q);
	components /= sqrt;
	alignas(16) float data[4];
	components.CopyToAlignedData(data);
	X = data[0];
	Y = data[1];
	Z = data[2];
}
