#include "PhysicsWorld.h"

#include "ByteEngine/Resources/StaticMeshResourceManager.h"

PhysicsObjectHandle PhysicsWorld::AddPhysicsObject(GameInstance* gameInstance, Id meshName, StaticMeshResourceManager* staticMeshResourceManager)
{
	//staticMeshResourceManager->LoadStaticMesh()
	return PhysicsObjectHandle(0);
}

void PhysicsWorld::onUpdate(TaskInfo taskInfo)
{
	
}
