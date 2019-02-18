#pragma once

#include "Core.h"

#include "Object.h"
#include "WorldPrimitive.h"

#include "Transform3.h"
#include "RenderProxy.h"

GS_CLASS WorldObject : public Object, public WorldPrimitive
{
public:
	WorldObject();
	WorldObject(const Transform3 & Transform);
	virtual ~WorldObject();

	RenderProxy * GetRenderProxy() const { return RenderProxy; }

protected:
	RenderProxy * RenderProxy = nullptr;
};