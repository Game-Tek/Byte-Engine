#include "ConeWithFalloff.h"

#include <GTSL/Math/Math.hpp>

ConeWithFalloff::ConeWithFalloff(const float Radius, const float Length) : Cone(Radius, Length)
{
}

ConeWithFalloff::ConeWithFalloff(const float Radius, const float Length, const float ExtraRadius) : Cone(Radius, Length), ExtraRadius(ExtraRadius)
{
}

float ConeWithFalloff::GetOuterConeInnerRadius() const
{
	return GTSL::Math::ArcTangent((Radius + ExtraRadius) / Length);
}
