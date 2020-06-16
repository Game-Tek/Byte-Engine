#include "World.h"

#include "ByteEngine/Application/Application.h"

void EntitiesManager::AddType(const GTSL::Ranger<UTF8>& name, ComponentCollection* typeManager)
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
		ComponentCollection::DestroyInstanceInfo destroy_instance_info;
		e->DestroyInstance(destroy_instance_info);
	}
}

void World::Pause()
{
	worldTimeMultiplier = 0;
}
