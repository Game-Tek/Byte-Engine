#include "RenderResourcesManager.h"

#include "RAPI/RenderDevice.h"

#include "RAPI/GraphicsPipeline.h"

RenderResourcesManager::~RenderResourcesManager()
{
	for (auto const& x : Materials)
	{
		delete x.second.Second;
	}
}

Mesh* RenderResourcesManager::CreateMesh(StaticMesh* _SM)
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
