#include "PhysicsWorld.h"


#include "ByteEngine/Application/Application.h"
#include "ByteEngine/Resources/StaticMeshResourceManager.h"

PhysicsObjectHandle PhysicsWorld::AddPhysicsObject(ApplicationManager* gameInstance, Id meshName, StaticMeshResourceManager* staticMeshResourceManager)
{
	auto objectIndex = physicsObjects.Emplace();
	
	//staticMeshResourceManager->LoadStaticMeshInfo(gameInstance, meshName, onStaticMeshInfoLoadedHandle, GTSL::MoveRef(objectIndex));
	
	return PhysicsObjectHandle(objectIndex);
}

void PhysicsWorld::onUpdate(TaskInfo taskInfo)
{
	auto deltaMicroseconds = BE::Application::Get()->GetClock()->GetDeltaTime();

	auto deltaSeconds = deltaMicroseconds.As<float32, GTSL::Seconds>();

	GTSL::Vector4 accumulatedUnboundedForces;
	for (auto f : boundlessForces) { accumulatedUnboundedForces += f; }

	//for(auto& a : physicsObjects) {
	//	for(auto& b : physicsObjects) {
	//		if (auto hit = intersect(a, b); hit.WasHit) {
	//			const auto totalInverseMass = a.inverseMass + b.inverseMass;
	//			const auto totalElasticity = a.restitutionFactor * b.restitutionFactor;
	//
	//			const auto vAB = a.velocity - b.velocity;
	//			const auto impulseJ = -(1.0f + totalElasticity) * GTSL::Math::DotProduct(vAB, hit.Normal) / totalInverseMass;
	//			const auto vecImpulse = hit.Normal * impulseJ;
	//
	//			applyImpulseLinear(&a, vecImpulse);
	//			applyImpulseLinear(&b, -vecImpulse);
	//
	//			const auto tA = a.inverseMass / totalInverseMass;
	//			const auto tB = b.inverseMass / totalInverseMass;
	//
	//			const auto ds = hit.PointOnB - hit.PointOnA;
	//			a.position += ds * tA;
	//			b.position -= ds * tB;
	//		}
	//	}
	//}

	for(auto& e : physicsObjects) { //semi implicit euler
		e.velocity += accumulatedUnboundedForces * deltaSeconds;
		e.position += e.velocity * deltaSeconds;
	}

	updatedObjects.Resize(0);
}

void PhysicsWorld::onStaticMeshInfoLoaded(TaskInfo taskInfo, StaticMeshResourceManager* staticMeshResourceManager, StaticMeshResourceManager::StaticMeshInfo staticMeshInfo, uint32 buffer)
{
	staticMeshInfo.BoundingBox;
	staticMeshInfo.BoundingRadius;

	//physicsObjects[buffer].Buffer.Allocate(staticMeshInfo.GetVerticesSize() + staticMeshInfo.GetIndicesSize(), 16, GetPersistentAllocator());
	
	//staticMeshResourceManager->LoadStaticMesh(taskInfo.ApplicationManager, staticMeshInfo, 16, physicsObjects[buffer].Buffer, onStaticMeshLoadedHandle, GTSL::MoveRef(buffer));
}

void PhysicsWorld::onStaticMeshLoaded(TaskInfo taskInfo, StaticMeshResourceManager* staticMeshResourceManager, StaticMeshResourceManager::StaticMeshInfo staticMeshInfo, uint32)
{
	
}
