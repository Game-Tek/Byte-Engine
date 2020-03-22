#include "Plane.h"

#include "BEM.hpp"

Plane::Plane(const Vector3& _A, const Vector3& _B, const Vector3& _C) :
	Normal(BEM::Normalized(BEM::Cross(_B - _A, _C - _A))), D(BEM::DotProduct(Normal, _A))
{
}
