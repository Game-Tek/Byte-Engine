#include "StaticMeshRenderableManager.h"

#include "Containers/FString.h"

#include "RAPI/CommandBuffer.h"

#include "Renderer.h"

#include "RAPI/RenderDevice.h"
#include "StaticMeshRenderComponent.h"

StaticMeshRenderableManager::StaticMeshRenderableManager(const StaticMeshRenderableManagerCreateInfo& staticMeshRenderableManagerCreateInfo)
{
}

void StaticMeshRenderableManager::DrawObjects(const DrawObjectsInfo& drawObjectsInfo)
{
}

uint32 StaticMeshRenderableManager::RegisterComponent(Renderer* renderer, RenderComponent* renderComponent)
{
	auto component = static_cast<StaticMeshRenderComponent*>(renderComponent);

	renderer->CreateMesh(component->GetStaticMesh());
	//renderer->CreateMaterial(component->GetStaticMesh()->GetMaterial());
	return 0;
}
