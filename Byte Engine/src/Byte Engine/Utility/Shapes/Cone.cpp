#include "Cone.h"

#include <GTM/GTM.hpp>

Cone::Cone(const float Radius, const float Length) : Radius(Radius), Length(Length)
{
}

float Cone::GetInnerAngle() const
{
	return GTM::ArcTangent(Radius / Length);
}
