#include "StaticMeshRenderComponent.h"

#include "RenderableInstructions.h"

#include "Scene.h"

void StaticMeshRenderComponent::CreateInstanceResources(CreateInstanceResourcesInfo& _CIRI)
{
	_CIRI.StaticMesh = SCAST(StaticMeshRenderComponent*, _CIRI.RenderComponent)->staticMesh;
	_CIRI.Material = SCAST(StaticMeshRenderComponent*, _CIRI.RenderComponent)->staticMesh->GetMaterial();
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
	_DII.Scene->DrawMesh(DI);
}

RenderableInstructions StaticMeshRenderComponent::GetRenderableInstructions() const
{
	RenderableInstructions SMRCRI;
	SMRCRI.RenderableTypeName = "StaticMesh";
	SMRCRI.CreateInstanceResources = decltype(SMRCRI.CreateInstanceResources)::Create<&CreateInstanceResources>();
	SMRCRI.BuildTypeInstanceSortData = decltype(SMRCRI.BuildTypeInstanceSortData)::Create<&BuildTypeInstanceSortData>();
	SMRCRI.BindTypeResources = decltype(SMRCRI.BindTypeResources)::Create<&BindTypeResources>();
	SMRCRI.DrawInstance = decltype(SMRCRI.DrawInstance)::Create<&DrawInstance>();
	return SMRCRI;
}
