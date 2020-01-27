#include "Rotator.h"

#include "GSM.hpp"

//there seems to be some inaccuracy in the X field, CHECK, but works fairly well for now

Rotator::Rotator(const Vector3& vector) : X(GSM::ArcSine(vector.Y)), Y(GSM::ArcSine(vector.X / GSM::Cosine(X))), Z(0)
{
}
