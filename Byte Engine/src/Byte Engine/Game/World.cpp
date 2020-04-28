#include "World.h"

#include "Byte Engine/Application/Application.h"

World::World()
{
}

World::~World()
{
	for(auto& e : types)
	{
		delete e.second;
	}
}

void World::OnUpdate()
{
	for(auto& e : types)
	{
		TypeManager::UpdateInstancesInfo update_instances_info;
		e.second->UpdateInstances(update_instances_info);
	}
}

void World::Pause()
{
	worldTimeMultiplier = 0;
}
