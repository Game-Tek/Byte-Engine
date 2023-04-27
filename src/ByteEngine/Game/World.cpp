#include "World.h"

#include "GTSL/JSON.hpp"

World::World() : Object(u8"World")
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
