#pragma once

#include "Core.h"

class WorldObject;

GS_CLASS RenderProxy
{
public:
	RenderProxy() = default;
	explicit RenderProxy(WorldObject * Owner);
	virtual ~RenderProxy() = default;

	virtual void Draw() = 0;

protected:
	WorldObject * Owner = nullptr;
};