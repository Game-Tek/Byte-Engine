#pragma once

#include "Core.h"

class WorldObject;

GS_CLASS RenderProxy
{
public:
	RenderProxy() = default;
	explicit RenderProxy(WorldObject * Owner);
	virtual ~RenderProxy() = default;

	virtual void Draw() {};

	const WorldObject * GetOwner() const { return WorldObjectOwner; }

protected:
	WorldObject * WorldObjectOwner = nullptr;
};