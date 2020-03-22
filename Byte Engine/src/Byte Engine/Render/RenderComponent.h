#pragma once

#include "Core.h"

#include "Game/Component.h"
#include "Containers/Id.h"

struct RenderComponentCreateInfo : ComponentCreateInfo
{
};

class RenderComponent : public Component
{
protected:
	//Determines whether this object will be drawn on the current update. DOES NOT DEPEND ON IsDynamic.
	bool ShouldRender = true;
	
public:
	//Defines whether this render component updates it's properties during it's lifetime or if the settings found on creation are the ones that will be used for all it's lifetime.
	//All other properties won't be updated during runtime if this flag is set to true, unless stated otherwise.
	[[nodiscard]] virtual bool IsDynamic() const { return false; }

	//Returns whether this render component should be rendered on the current update.
	[[nodiscard]] bool GetShouldRender() const { return ShouldRender; }

	[[nodiscard]] virtual Id64 GetRenderableType() const = 0;
};
