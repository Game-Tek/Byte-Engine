#pragma once

#include "TestCollection.h"
#include "ByteEngine/Game/World.h"

class MenuWorld : public World
{
public:
	void InitializeWorld(const InitializeInfo& initializeInfo) override
	{
		World::InitializeWorld(initializeInfo);

		BE_LOG_MESSAGE("Initilized world!");

		//testComponentCollectionReference = initializeInfo.GameInstance->AddComponentCollection<TestCollection>("TestCollection");
		
		auto collection = initializeInfo.GameInstance->AddComponentCollection<TestCollection>("TestCollection");
		ComponentCollection::CreateInstancesInfo create_instances_info;
		create_instances_info.Count = 3;
		collection->CreateInstances(create_instances_info);

		for(auto& e : collection->GetNumbers())
		{
			BE_LOG_SUCCESS(e)
		}
	}

	void DestroyWorld(const DestroyInfo& destroyInfo) override
	{
		destroyInfo.GameInstance->DestroyComponentCollection(testComponentCollectionReference);
	}
	
private:
	uint64 testComponentCollectionReference{ 0 };
};

class SandboxWorld
{
	
};