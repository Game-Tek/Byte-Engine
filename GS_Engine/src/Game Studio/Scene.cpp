#include "Scene.h"

void Scene::AddWorldObject(WorldObject * Object)
{
	ObjectList.push_back(Object);

	return;
}

void Scene::RemoveWorldObject(WorldObject * Object)
{
	ObjectList.eraseObject(Object);

	return;
}
