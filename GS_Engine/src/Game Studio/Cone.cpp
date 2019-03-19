#include "Cone.h"

#include "GSM.hpp"

float Cone::GetInnerAngle() const
{
	return GSM::ArcTangent(Radius / Length);
}