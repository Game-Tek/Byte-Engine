#include "Cone.h"

#include "Math\BEM.hpp"

Cone::Cone(const float Radius, const float Length) : Radius(Radius), Length(Length)
{
}

float Cone::GetInnerAngle() const
{
	return BEM::ArcTangent(Radius / Length);
}
