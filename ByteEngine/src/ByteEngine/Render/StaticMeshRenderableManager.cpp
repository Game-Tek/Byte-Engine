#include "StaticMeshRenderableManager.h"

#include <GTSL/String.hpp>

#include "GAL/CommandBuffer.h"

#include "Renderer.h"

#include "GAL/RenderDevice.h"
#include "StaticMeshRenderComponent.h"

StaticMeshRenderableManager::StaticMeshRenderableManager(const StaticMeshRenderableManagerCreateInfo& staticMeshRenderableManagerCreateInfo)
{
}

void StaticMeshRenderableManager::DrawObjects(const DrawObjectsInfo& drawObjectsInfo)
{
}

uint32 StaticMeshRenderableManager::RegisterComponent(Renderer* renderer, RenderComponent* renderComponent)
{
	return 0;
}
