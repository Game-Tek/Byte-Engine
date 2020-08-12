#pragma once

#include "ByteEngine/Game/World.h"
#include "ByteEngine/Render/RenderSystem.h"
#include "ByteEngine/Resources/AudioResourceManager.h"

class MenuWorld : public World
{
public:
	void InitializeWorld(const InitializeInfo& initializeInfo) override
	{
		World::InitializeWorld(initializeInfo);

		BE_LOG_MESSAGE("Initilized world!");
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