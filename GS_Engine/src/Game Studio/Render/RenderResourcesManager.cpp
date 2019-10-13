#include "RenderResourcesManager.h"

#include "RAPI/RenderDevice.h"

#include "RAPI/GraphicsPipeline.h"

RenderResourcesManager::~RenderResourcesManager()
{
	for (auto const& x : Pipelines)
	{
		delete x.second;
	}
}

Mesh* RenderResourcesManager::RegisterMesh(StaticMesh* _SM)
{
	Mesh* NewMesh = nullptr;

	if(!Meshes.find(_SM)->second)
	{
		MeshCreateInfo MCI;
		NewMesh = RenderDevice::Get()->CreateMesh(MCI);
	}
	else
	{
		NewMesh = Meshes[_SM];
	}

	return NewMesh;
}

GraphicsPipeline* RenderResourcesManager::CreatePipelineFromMaterial(Material* _Mat)
{
	GraphicsPipelineCreateInfo GPCI;

	ShaderInfo VSI;
	ShaderInfo FSI;
	_Mat->GetRenderingCode(VSI.ShaderCode, FSI.ShaderCode);

	GPCI.PipelineDescriptor.Stages.push_back(VSI);
	GPCI.PipelineDescriptor.Stages.push_back(FSI);
	GPCI.PipelineDescriptor.BlendEnable = _Mat->GetHasTransparency();
	GPCI.PipelineDescriptor.ColorBlendOperation = BlendOperation::ADD;
	GPCI.PipelineDescriptor.CullMode = _Mat->GetIsTwoSided() ? CullMode::CULL_NONE : CullMode::CULL_BACK;
	GPCI.PipelineDescriptor.DepthCompareOperation = CompareOperation::GREATER;

	return RenderDevice::Get()->CreateGraphicsPipeline(GPCI);
}

GraphicsPipeline* RenderResourcesManager::RegisterMaterial(Material* _Mat)
{
	auto Res = Pipelines.find(Id(_Mat->GetMaterialName()).GetID());
	if (Res != Pipelines.end())
	{
		return Pipelines[Res->first];
	}

	auto NP = CreatePipelineFromMaterial(_Mat);
	Pipelines.emplace(Res->first, NP);
	return NP;
}
