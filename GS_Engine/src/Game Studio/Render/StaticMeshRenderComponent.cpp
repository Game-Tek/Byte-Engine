#include "StaticMeshRenderComponent.h"

#include "RenderableInstructions.h"

#include "Scene.h"

RenderableInstructions StaticMeshRenderComponent::StaticMeshRenderInstructions = { decltype(RenderableInstructions::CreateInstanceResources)::Create<&CreateInstanceResources>(), decltype(RenderableInstructions::BuildTypeInstanceSortData)::Create<&BuildTypeInstanceSortData>(), decltype(RenderableInstructions::BindTypeResources)::Create<&BindTypeResources>(), decltype(RenderableInstructions::DrawInstance)::Create<&DrawInstance>() };

void StaticMeshRenderComponent::CreateInstanceResources(CreateInstanceResourcesInfo& _CIRI)
{
	_CIRI.Material = SCAST(StaticMeshRenderComponentCreateInfo*, _CIRI.RenderComponentCreateInfo)->StaticMesh->GetMaterial();

	SCAST(StaticMeshRenderComponent*, _CIRI.RenderComponent)->staticMesh = SCAST(StaticMeshRenderComponentCreateInfo*, _CIRI.RenderComponentCreateInfo)->StaticMesh;
	SCAST(StaticMeshRenderComponent*, _CIRI.RenderComponent)->renderMesh = _CIRI.Scene->RegisterMesh(SCAST(StaticMeshRenderComponentCreateInfo*, _CIRI.RenderComponentCreateInfo)->StaticMesh);
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
