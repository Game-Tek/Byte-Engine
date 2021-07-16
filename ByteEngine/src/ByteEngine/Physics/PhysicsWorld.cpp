#include "PhysicsWorld.h"


#include "ByteEngine/Application/Application.h"
#include "ByteEngine/Resources/StaticMeshResourceManager.h"

PhysicsObjectHandle PhysicsWorld::AddPhysicsObject(GameInstance* gameInstance, Id meshName, StaticMeshResourceManager* staticMeshResourceManager)
{
	auto objectIndex = physicsObjects.Emplace();
	
	staticMeshResourceManager->LoadStaticMeshInfo(gameInstance, meshName, onStaticMeshInfoLoadedHandle, GTSL::MoveRef(objectIndex));
	
	return PhysicsObjectHandle(objectIndex);
}

void PhysicsWorld::onUpdate(TaskInfo taskInfo)
{
	auto deltaMicroseconds = BE::Application::Get()->GetClock()->GetDeltaTime();

	auto deltaSeconds = deltaMicroseconds.As<float32, GTSL::Seconds>();

	GTSL::Vector4 accumulatedUnboundedForces;
	for (auto f : boundlessForces) { accumulatedUnboundedForces += f; }
	
	for(auto& e : physicsObjects) { //semi implicit euler
		e.Velocity += e.Acceleration * deltaSeconds;
		e.Position += e.Velocity * deltaSeconds;
	}

	updatedObjects.Resize(0);
}

void PhysicsWorld::onStaticMeshInfoLoaded(TaskInfo taskInfo, StaticMeshResourceManager* staticMeshResourceManager, StaticMeshResourceManager::StaticMeshInfo staticMeshInfo, uint32 buffer)
{
	staticMeshInfo.BoundingBox;
	staticMeshInfo.BoundingRadius;

	//physicsObjects[buffer].Buffer.Allocate(staticMeshInfo.GetVerticesSize() + staticMeshInfo.GetIndicesSize(), 16, GetPersistentAllocator());
	
	//staticMeshResourceManager->LoadStaticMesh(taskInfo.GameInstance, staticMeshInfo, 16, physicsObjects[buffer].Buffer, onStaticMeshLoadedHandle, GTSL::MoveRef(buffer));
}

void PhysicsWorld::onStaticMeshLoaded(TaskInfo taskInfo, StaticMeshResourceManager* staticMeshResourceManager, StaticMeshResourceManager::StaticMeshInfo staticMeshInfo, uint32)
{
	
}
