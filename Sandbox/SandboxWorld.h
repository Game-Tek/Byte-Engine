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