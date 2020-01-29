#pragma once

#include "WorldObject.h"

//Actor is the base class for all objects that can be possessed by a controller.
class Actor : public WorldObject
{
public:
	Actor();
	~Actor();
};
