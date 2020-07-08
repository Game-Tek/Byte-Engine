#pragma once

#include "ByteEngine/Game/World.h"
#include "ByteEngine/Render/RenderStaticMeshCollection.h"
#include "ByteEngine/Render/RenderSystem.h"
#include "ByteEngine/Render/StaticMeshRenderGroup.h"

class MenuWorld : public World
{
public:
	void InitializeWorld(const InitializeInfo& initializeInfo) override
	{
		World::InitializeWorld(initializeInfo);

		BE_LOG_MESSAGE("Initilized world!");

		//auto collection = static_cast<RenderStaticMeshCollection*>(initializeInfo.GameInstance->GetComponentCollection("RenderStaticMeshCollection"));
		
		ComponentCollection::CreateInstanceInfo create_instance_info;
		//auto component = collection->CreateInstance(create_instance_info);
		//collection->SetMesh(component, "plane");

		//auto static_mesh_renderer = static_cast<StaticMeshRenderGroup*>(initializeInfo.GameInstance->GetSystem("StaticMeshRenderGroup"));
		//static_mesh_renderer->AddStaticMesh(component, collection);
	}
	
	void DestroyWorld(const DestroyInfo& destroyInfo) override
	{
		//destroyInfo.GameInstance->DestroyComponentCollection(testComponentCollectionReference);
	}
	
private:
	uint64 testComponentCollectionReference{ 0 };
};

class SandboxWorld
{
	
};