#include "World.h"

#include "ByteEngine/Application/Application.h"

void EntitiesManager::AddType(const GTSL::Ranger<char>& name, TypeManager* typeManager)
{
	hashes.PushBack(GTSL::Id64(name));
	managers.PushBack(typeManager);
}

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
		e->DestroyInstances(destroy_instances_info);
	}
}

void World::Pause()
{
	worldTimeMultiplier = 0;
}
