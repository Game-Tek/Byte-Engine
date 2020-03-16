#pragma once

#include "RenderableTypeManager.h"

#include "Containers/FVector.hpp"
#include "RAPI/Bindings.h"

#include "Containers/Id.h"

class RenderComponent;

class StaticMeshRenderableManager final : public RenderableTypeManager
{
	
public:
	struct StaticMeshRenderableManagerCreateInfo : RenderableTypeManagerCreateInfo
	{
	};

	explicit StaticMeshRenderableManager(const StaticMeshRenderableManagerCreateInfo& staticMeshRenderableManagerCreateInfo);
	
	[[nodiscard]] const char* GetName() const override { return "StaticMeshRenderableManager"; }
	
	void DrawObjects(const DrawObjectsInfo& drawObjectsInfo) override;
	Id64 GetRenderableTypeName() const override { return Id64("Static Mesh"); }
	uint32 RegisterComponent(Renderer* renderer, RenderComponent* renderComponent) override;
};
