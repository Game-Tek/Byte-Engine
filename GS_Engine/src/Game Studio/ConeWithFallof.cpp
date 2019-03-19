#include "ConeWithFallof.h"

#include "GSM.hpp"

float ConeWithFallof::GetOuterConeInnerRadius() const
{
	return GSM::ArcTangent((Radius + ExtraRadius) / Length);
}