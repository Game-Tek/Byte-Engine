#include "Vector3.h"

#include "GSM.hpp"

Vector3::Vector3(const Rotator& rotator) : X(GSM::Cosine(rotator.X) * GSM::Sine(rotator.Y)), Y(-GSM::Sine(rotator.X)), Z(GSM::Cosine(rotator.Y) * GSM::Cosine(rotator.X))
{
	//CODE IS CORRECT
}
