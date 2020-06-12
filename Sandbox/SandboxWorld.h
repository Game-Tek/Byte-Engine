#pragma once

#include "ByteEngine/Game/World.h"

class MenuWorld : public World
{
public:
	void InitializeWorld(const InitializeInfo& initializeInfo) override
	{
		World::InitializeWorld(initializeInfo);

		BE_LOG_MESSAGE("Initilized world!");

		initializeInfo.GameInstance->GetComponentCollection("TestComponents", testComponentCollectionReference);
		initializeInfo.GameInstance->GetSystem("Renderer");
	}

private:
	uint64 testComponentCollectionReference{ 0 };
};

class SandboxWorld
{
	
};