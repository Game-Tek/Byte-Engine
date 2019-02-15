#include "World.h"

World::World()
{
}


World::~World()
{
	for (uint32 i = 0; i < EntityList.length(); i++)
	{
		delete EntityList[i];
	}
}

void World::SpawnObject(WorldObject * NewObject, const Vector3 & Position)
{
	NewObject->SetPosition(Position);
	EntityList.push_back(NewObject);
}


void World::OnUpdate()
{
	for (uint32 i = 0; i < EntityList.length(); i++)
	{
		EntityList[i]->OnUpdate();
	}
}
