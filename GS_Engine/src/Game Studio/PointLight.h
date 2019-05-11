#pragma once

#include "Core.h"

#include "Light.h"

class PointLightRenderProxy;

GS_CLASS PointLight : public Light
{
public:
	PointLight();
	~PointLight();

	RenderProxy * GetRenderProxy() override { return (RenderProxy *)LightRenderProxy; }

protected:
	PointLightRenderProxy * LightRenderProxy = nullptr;
};

