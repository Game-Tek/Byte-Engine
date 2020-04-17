#include "Cone.h"

#include <GTSL/Math/Math.hpp>

Cone::Cone(const float Radius, const float Length) : Radius(Radius), Length(Length)
{
}

float Cone::GetInnerAngle() const
{
	return GTSL::Math::ArcTangent(Radius / Length);
}
