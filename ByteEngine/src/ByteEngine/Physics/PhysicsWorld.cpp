#include "PhysicsWorld.h"


#include "ByteEngine/Application/Application.h"
#include "ByteEngine/Resources/StaticMeshResourceManager.h"

PhysicsObjectHandle PhysicsWorld::AddPhysicsObject(GameInstance* gameInstance, Id meshName, StaticMeshResourceManager* staticMeshResourceManager)
{
	//staticMeshResourceManager->LoadStaticMesh()
	return PhysicsObjectHandle(0);
}

void PhysicsWorld::onUpdate(TaskInfo taskInfo)
{
	auto deltaMicroseconds = BE::Application::Get()->GetClock()->GetDeltaTime();

	auto deltaSeconds = deltaMicroseconds.As<float32, GTSL::Seconds>();
	
	for(auto& e : physicsObjects)
	{
		e.Position = e.Velocity * deltaSeconds;
	}

	updatedObjects.ResizeDown(0);
}

void PhysicsWorld::onStaticMeshInfoLoaded(TaskInfo taskInfo, StaticMeshResourceManager* staticMeshResourceManager, StaticMeshResourceManager::StaticMeshInfo staticMeshInfo)
{
	staticMeshInfo.BoundingBox;
	staticMeshInfo.BoundingRadius;
}
