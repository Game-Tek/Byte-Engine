#pragma once

#include "Core.h"

#include "WorldObject.h"

//Actor is the base class for all objects that can be possesed by a controller.
GS_CLASS Actor : public WorldObject
{
public:
	Actor();
	~Actor();
};

