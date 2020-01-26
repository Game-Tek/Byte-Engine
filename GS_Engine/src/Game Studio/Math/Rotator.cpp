#include "Rotator.h"

#include "GSM.hpp"

//there seems to be some inaccuracy in the X field, CHECK, but works fairly well for now

Rotator::Rotator(const Vector3& vector) : X(GSM::ArcTan2(vector.Z, vector.Y)), Y(GSM::ArcTan2(vector.Z, vector.X)), Z(0)
{
}
