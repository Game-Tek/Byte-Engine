#pragma once

#include "RenderableTypeManager.h"

#include "Containers/FVector.hpp"
#include "RAPI/Bindings.h"

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
	void GetRenderableTypeName(FString& name) override;
	uint32 RegisterComponent(Renderer* renderer, RenderComponent* renderComponent) override;
};
