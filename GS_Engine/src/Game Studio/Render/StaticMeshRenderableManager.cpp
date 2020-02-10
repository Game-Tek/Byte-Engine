#include "StaticMeshRenderableManager.h"

#include "Containers/FString.h"

#include "RAPI/CommandBuffer.h"

#include "Renderer.h"

#include "RAPI/RenderDevice.h"
#include "StaticMeshRenderComponent.h"

StaticMeshRenderableManager::StaticMeshRenderableManager(const StaticMeshRenderableManagerCreateInfo& staticMeshRenderableManagerCreateInfo)
{

	
	RAPI::BindingsPoolCreateInfo bindings_pool_create_info;
	bindings_pool_create_info.BindingsSetCount = staticMeshRenderableManagerCreateInfo.MaxFramesInFlight;
	bindings_pool_create_info.BindingsSetLayout;
	bindings_pool_create_info.RenderDevice = staticMeshRenderableManagerCreateInfo.RenderDevice;
	staticMeshesTransformBindings.First = staticMeshRenderableManagerCreateInfo.RenderDevice->CreateBindingsPool(bindings_pool_create_info);
	
	RAPI::BindingsSetCreateInfo bindings_set_create_info;
	bindings_set_create_info.BindingsSetCount = staticMeshRenderableManagerCreateInfo.MaxFramesInFlight;
	bindings_set_create_info.BindingsSetLayout;
	bindings_set_create_info.BindingsPool = staticMeshesTransformBindings.First;
	bindings_set_create_info.RenderDevice = staticMeshRenderableManagerCreateInfo.RenderDevice;
	staticMeshesTransformBindings.Second = staticMeshRenderableManagerCreateInfo.RenderDevice->CreateBindingsSet(bindings_set_create_info);
}

void StaticMeshRenderableManager::DrawObjects(const DrawObjectsInfo& drawObjectsInfo)
{
	drawObjectsInfo.CommandBuffer;
	drawObjectsInfo.ViewProjectionMatrix;
}

void StaticMeshRenderableManager::GetRenderableTypeName(FString& name) { name = "StaticMesh"; }

uint32 StaticMeshRenderableManager::RegisterComponent(Renderer* renderer, RenderComponent* renderComponent)
{
	auto component = static_cast<StaticMeshRenderComponent*>(renderComponent);

	renderer->CreateMesh(component->GetStaticMesh());
	renderer->CreateMaterial(component->GetStaticMesh()->GetMaterial());
}
