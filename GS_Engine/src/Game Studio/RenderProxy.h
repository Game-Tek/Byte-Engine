#pragma once

#include "Core.h"

class WorldObject;

GS_CLASS RenderProxy
{
public:
	RenderProxy(WorldObject * Owner);
	virtual ~RenderProxy() = default;

protected:
	WorldObject * Owner;
};