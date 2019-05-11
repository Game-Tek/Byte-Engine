#include "PointLight.h"

#include "PointLightRenderProxy.h"

PointLight::PointLight()
{
	LightRenderProxy = new PointLightRenderProxy(this);
}

PointLight::~PointLight()
{
	delete LightRenderProxy;
}
