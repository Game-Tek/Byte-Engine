#pragma once

#include "Box.h"

struct BoxWithFalloff : public Box
{
	float falloffDistance = 0;
};
