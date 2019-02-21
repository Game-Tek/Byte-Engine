#pragma once

#include "Core.h"

#include "Object.h"
#include "WorldPrimitive.h"

#include "Transform3.h"
#include "RenderProxy.h"
#include "Application.h"

class GameInstance;

GS_CLASS WorldObject : public Object, public WorldPrimitive
{
public:
	WorldObject() = default;
	explicit WorldObject(const Transform3 & Transform);
	virtual ~WorldObject();

	GameInstance * GetGameInstance() { return GS::Application::Get()->GetGameInstanceInstance(); }

	RenderProxy * GetRenderProxy() const { return RenderProxy; }

protected:
	RenderProxy * RenderProxy = nullptr;
};