#include "World.h"
#include "Application.h"

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
	//Set position.
	NewObject->SetPosition(Position);
	//Add it to the entity list array.
	EntityList.push_back(NewObject);

	GS::Application::Get()->GetRendererInstance()->GetScene().AddWorldObject(NewObject);
}


void World::OnUpdate()
{
	for (uint32 i = 0; i < EntityList.length(); i++)
	{
		EntityList[i]->OnUpdate();
	}
}
