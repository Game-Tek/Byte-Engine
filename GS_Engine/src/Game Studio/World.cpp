#include "World.h"
#include "Application.h"

#include "StaticMesh.h"
#include "PointLight.h"

World::World() : EntityList(10)
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
}

void World::SpawnObject(StaticMesh * NewStaticMesh, const Vector3 & Position)
{
	//Set position.
	NewStaticMesh->SetPosition(Position);
	//Add it to the entity list array.
	EntityList.push_back(reinterpret_cast<WorldObject *>(NewStaticMesh));

	GS::Application::Get()->GetRendererInstance()->GetScene()->AddObject(NewStaticMesh->GetRenderProxy());
}

void World::SpawnObject(PointLight * NewPointLight, const Vector3 & Position)
{
	//Set position.
	NewPointLight->SetPosition(Position);
	//Add it to the entity list array.
	EntityList.push_back(reinterpret_cast<WorldObject *>(NewPointLight));

	GS::Application::Get()->GetRendererInstance()->GetScene()->AddLight((PointLightRenderProxy *)NewPointLight->GetRenderProxy());
}

void World::SetActiveCamera(Camera * Camera) const
{
	GS::Application::Get()->GetRendererInstance()->GetScene()->SetCamera(Camera);
}

void World::OnUpdate()
{
	for (uint32 i = 0; i < EntityList.length(); i++)
	{
		EntityList[i]->OnUpdate();
	}
}
