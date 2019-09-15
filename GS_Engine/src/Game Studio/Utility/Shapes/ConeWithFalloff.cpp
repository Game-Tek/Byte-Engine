#include "ConeWithFalloff.h"

#include "Math\GSM.hpp"

ConeWithFalloff::ConeWithFalloff(const float Radius, const float Length) : Cone(Radius, Length)
{
}

ConeWithFalloff::ConeWithFalloff(const float Radius, const float Length, const float ExtraRadius) : Cone(Radius, Length), ExtraRadius(ExtraRadius)
{
}	

float ConeWithFalloff::GetOuterConeInnerRadius() const
{
	return GSM::ArcTangent((Radius + ExtraRadius) / Length);
}