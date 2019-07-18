#include "Cone.h"

#include "Math\GSM.hpp"

Cone::Cone(const float Radius, const float Length) : Radius(Radius), Length(Length)
{
}

float Cone::GetInnerAngle() const
{
	return GSM::ArcTangent(Radius / Length);
}