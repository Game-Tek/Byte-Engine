#pragma once

#include "Core.h"

#include "RenderProxy.h"

GS_CLASS LightRenderProxy : public RenderProxy
{
public:
	LightRenderProxy() = default;
	LightRenderProxy(WorldObject * Owner);
	virtual ~LightRenderProxy();
};

