#pragma once

#include <GTSL/Bitfield.h>
#include <GTSL/FixedVector.hpp>
#include <GTSL/Math/Vectors.h>

#include "HitResult.h"
#include "ByteEngine/Game/System.h"
#include "ByteEngine/Handle.hpp"
#include "ByteEngine/Game/ApplicationManager.h"
#include "ByteEngine/Resources/StaticMeshResourceManager.h"

class StaticMeshResourceManager;
MAKE_HANDLE(uint32, PhysicsObject);

class PhysicsWorld : public System {
public:
	PhysicsWorld(const InitializeInfo& initialize_info) : System(initialize_info, u8"PhysicsWorld"), updatedObjects(32, GetPersistentAllocator()),
		physicsObjects(32, GetPersistentAllocator())
	{
		//initialize_info.GameInstance->AddTask(u8"onUpdate", &PhysicsWorld::onUpdate, {}, u8"FrameUpdate", u8"RenderStart");

		//onStaticMeshInfoLoadedHandle = initialize_info.GameInstance->StoreDynamicTask(u8"onStaticMeshInfoLoad", Task<StaticMeshResourceManager*, StaticMeshResourceManager::StaticMeshInfo, uint32>::Create<PhysicsWorld, &PhysicsWorld::onStaticMeshInfoLoaded>(this), {});
		//onStaticMeshLoadedHandle = initialize_info.GameInstance->StoreDynamicTask(u8"onStaticMeshLoad", Task<StaticMeshResourceManager*, StaticMeshResourceManager::StaticMeshInfo, uint32>::Create<PhysicsWorld, &PhysicsWorld::onStaticMeshLoaded>(this), {});

		boundlessForces.EmplaceBack(0, -10, 0, 0);
	}

	PhysicsObjectHandle AddPhysicsObject(ApplicationManager* gameInstance, Id meshName, StaticMeshResourceManager* staticMeshResourceManager);

	GTSL::Vector4 GetPosition(const PhysicsObjectHandle physics_object_handle) const { return physicsObjects[physics_object_handle()].position; }

	void SetMass(const PhysicsObjectHandle physics_object_handle, float32 massKg) { physicsObjects[physics_object_handle()].inverseMass = 1.f / massKg; }

	void SetDampFactor(const float32 newDampFactor) { dampFactor = newDampFactor; }

	[[nodiscard]] auto GetAirDensity() const { return dampFactor; }

	HitResult TraceRay(const GTSL::Vector3 start, const GTSL::Vector3 end);

private:
	/**
	 * \brief Specifies how much speed the to remove from entities.\n
	 * Default value is 0.0001.
	 */
	float32 dampFactor = 0.001f;

	/**
	 * \brief Defines the number of substeps used for simulation. Default is 1, which mean only one iteration will run each frame.
	 */
	uint16 simSubSteps = 1;

	GTSL::Vector<PhysicsObjectHandle, BE::PAR> updatedObjects;
	
	void doBroadPhase();
	void doNarrowPhase();
	void solveDynamicObjects(double _UpdateTime);

	void insertObject() {
		GTSL::Vector3 aabb, pos;

		GTSL::Bitfield<3> bitfield;

		bitfield[0] = pos.X() > 0.0f; bitfield[1] = pos.Y() > 0.0f; bitfield[2] = pos.Z() > 0.0f;
	}
	
	void onUpdate(TaskInfo taskInfo);

	void onStaticMeshInfoLoaded(TaskInfo taskInfo, StaticMeshResourceManager* staticMeshResourceManager, StaticMeshResourceManager::StaticMeshInfo staticMeshInfo, uint32);
	void onStaticMeshLoaded(TaskInfo taskInfo, StaticMeshResourceManager* staticMeshResourceManager, StaticMeshResourceManager::StaticMeshInfo staticMeshInfo, uint32);
	
	struct PhysicsObject {
		GTSL::Vector4 velocity, angularVelocity, acceleration, position, centerOfMass;
		GTSL::Quaternion orientation;

		//kg
		float32 mass = 1.0f, inverseMass = 1.0f;
		float32 restitutionFactor = 0.5f;

		//shape
		float32 radius = 1.0f;
	};
	GTSL::FixedVector<PhysicsObject, BE::PAR> physicsObjects;

	GTSL::StaticVector<GTSL::Vector4, 8> boundlessForces;
	
	DynamicTaskHandle<StaticMeshResourceManager*, StaticMeshResourceManager::StaticMeshInfo, uint32> onStaticMeshInfoLoadedHandle;
	DynamicTaskHandle<StaticMeshResourceManager*, StaticMeshResourceManager::StaticMeshInfo, uint32> onStaticMeshLoadedHandle;

	void applyImpulseLinear(PhysicsObject* a, const GTSL::Vector4 impulse) {
		a->velocity += impulse * a->inverseMass;
	}

	void applyImpulseAngular(PhysicsObject* a, const GTSL::Vector4 impulse) {
		a->angularVelocity += getInverseWorldSpaceInertiaTensor(*a) * impulse;

		constexpr auto MAX_ANGULAR_SPEED = 30.f;

		if(GTSL::Math::LengthSquared(a->angularVelocity) > MAX_ANGULAR_SPEED * MAX_ANGULAR_SPEED) {
			GTSL::Math::Normalize(a->angularVelocity);
			a->angularVelocity *= MAX_ANGULAR_SPEED;
		}
	}

	HitResult intersect(PhysicsObject& a, PhysicsObject& b) {
		auto ab = b.position - a.position;
		auto abRadius = a.radius + b.radius;

		HitResult hit;
		hit.WasHit = GTSL::Math::LengthSquared(ab) <= abRadius * abRadius;
		//hit.Normal = GTSL::Math::Normalized(ab);
		//hit.Position = a.position + hit.Normal * a.radius;

		return hit;
	}

	GTSL::Matrix4 getInertiaTensor(const PhysicsObject& a) {
		return GTSL::Matrix4{ 2 * a.radius * a.radius / 5.f };
	}

	GTSL::Matrix4 getInverseWorldSpaceInertiaTensor(const PhysicsObject& a) {
		auto inertiaTensor = getInertiaTensor(a);
		auto inverted = GTSL::Math::Inverse(inertiaTensor) * a.inverseMass;
		auto orientation = GTSL::Matrix4(a.orientation);
		//inverted = orientation * inverted * orientation.Transpose();
		return inverted;
	}
};
