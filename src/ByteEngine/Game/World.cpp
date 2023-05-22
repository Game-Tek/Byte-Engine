#include "World.h"

World::World()
	: Object(u8"World")
{
}

void World::InitializeWorld(const InitializeInfo& info)
{
}

void World::DestroyWorld(const DestroyInfo& destroyInfo)
{
}

void World::Pause()
{
	m_worldTimeMult = 0;
}

