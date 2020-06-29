#include "World.h"

#include "ByteEngine/Application/Application.h"

World::World()
{
}

void World::InitializeWorld(const InitializeInfo& initializeInfo)
{
	
}

void World::DestroyWorld(const DestroyInfo& destroyInfo)
{
}

void World::Pause()
{
	worldTimeMultiplier = 0;
}
