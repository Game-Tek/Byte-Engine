#include "World.h"

#include "Byte Engine/Application/Application.h"

World::World()
{
}

void World::InitializeWorld(const InitializeInfo& initializeInfo)
{
	
}

void World::DestroyWorld(const DestroyInfo& destroyInfo)
{
	for (auto& e : entitiesManager)
	{
		TypeManager::DestroyInstancesInfo destroy_instances_info;
		e.second->DestroyInstances(destroy_instances_info);
	}
}

void World::Pause()
{
	worldTimeMultiplier = 0;
}
