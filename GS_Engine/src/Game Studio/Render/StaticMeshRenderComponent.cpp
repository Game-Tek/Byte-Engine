#include "StaticMeshRenderComponent.h"

#include "RenderableInstructions.h"

#include "Renderer.h"
#include "MeshRenderResource.h"

RenderableInstructions StaticMeshRenderComponent::StaticMeshRenderInstructions = {
	decltype(RenderableInstructions::CreateInstanceResources)::Create<&CreateInstanceResources>(),
	decltype(RenderableInstructions::BuildTypeInstanceSortData)::Create<&BuildTypeInstanceSortData>(),
	decltype(RenderableInstructions::BindTypeResources)::Create<&BindTypeResources>(),
	decltype(RenderableInstructions::DrawInstance)::Create<&DrawInstance>()
};

void StaticMeshRenderComponent::CreateInstanceResources(CreateInstanceResourcesInfo& _CIRI)
{
	const auto component = SCAST(StaticMeshRenderComponent*, _CIRI.RenderComponent);
	const auto create_info = SCAST(StaticMeshRenderComponentCreateInfo*, _CIRI.RenderComponentCreateInfo);

	component->staticMesh = create_info->StaticMesh;
	component->renderMaterial = _CIRI.Scene->CreateMaterial(create_info->StaticMesh->GetMaterial());
	component->renderMesh = _CIRI.Scene->CreateMesh(create_info->StaticMesh);
}

void StaticMeshRenderComponent::BuildTypeInstanceSortData(BuildTypeInstanceSortDataInfo& _BTISDI)
{
	for (auto& e : _BTISDI.InstancesVector)
	{
		e.Material = SCAST(StaticMeshRenderComponent*, e.RenderComponent)->staticMesh->GetMaterial();
	}
}

void StaticMeshRenderComponent::BindTypeResources(BindTypeResourcesInfo& _BTRI)
{
}

void StaticMeshRenderComponent::DrawInstance(DrawInstanceInfo& _DII)
{
	DrawInfo DI;
	DI.IndexCount = SCAST(StaticMeshRenderComponent*, _DII.RenderComponent)->staticMesh->GetModel().IndexCount;
	DI.InstanceCount = 1;
	_DII.Scene->DrawMesh(DI, SCAST(StaticMeshRenderComponent*, _DII.RenderComponent)->renderMesh);
}
