#include "Rotator.h"

#include "BEM.hpp"

//there seems to be some inaccuracy in the X field, CHECK, but works fairly well for now

Rotator::Rotator(const Vector3& vector) : X(BEM::ArcSine(vector.Y)), Y(BEM::ArcSine(vector.X / BEM::Cosine(X))), Z(0)
{
}
