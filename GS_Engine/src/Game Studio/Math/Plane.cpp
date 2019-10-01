#include "Plane.h"

#include "GSM.hpp"

Plane::Plane(const Vector3& _A, const Vector3& _B, const Vector3& _C) : Normal(GSM::Normalized(GSM::Cross(_B - _A, _C  - _A))), D(GSM::Dot(Normal, _A))
{
}
