#pragma once

#include "Core.h"

#include "WorldObject.h"

//Actor is the base class for all objects that can be possessed by a controller.
class GS_API Actor : public WorldObject
{
public:
	Actor();
	~Actor();
};

