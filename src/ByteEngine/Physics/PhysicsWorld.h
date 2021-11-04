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

class PhysicsWorld : public System
{
public:
	PhysicsWorld(const InitializeInfo& initialize_info) : System(initialize_info, u8"PhysicsWorld"), updatedObjects(32, GetPersistentAllocator()),
		physicsObjects(32, GetPersistentAllocator())
	{
		initialize_info.GameInstance->AddTask(u8"onUpdate", Task<>::Create<PhysicsWorld, &PhysicsWorld::onUpdate>(this), {}, u8"FrameUpdate", u8"RenderStart");

		onStaticMeshInfoLoadedHandle = initialize_info.GameInstance->StoreDynamicTask(u8"onStaticMeshInfoLoad", Task<StaticMeshResourceManager*, StaticMeshResourceManager::StaticMeshInfo, uint32>::Create<PhysicsWorld, &PhysicsWorld::onStaticMeshInfoLoaded>(this), {});
		onStaticMeshLoadedHandle = initialize_info.GameInstance->StoreDynamicTask(u8"onStaticMeshLoad", Task<StaticMeshResourceManager*, StaticMeshResourceManager::StaticMeshInfo, uint32>::Create<PhysicsWorld, &PhysicsWorld::onStaticMeshLoaded>(this), {});

		boundlessForces.EmplaceBack(0, -10, 0, 0);
	}
	
	void Shutdown(const ShutdownInfo& shutdownInfo) override;

	PhysicsObjectHandle AddPhysicsObject(ApplicationManager* gameInstance, Id meshName, StaticMeshResourceManager* staticMeshResourceManager);

	//void SetGravity(const GTSL::Vector3 newGravity) { gravity = newGravity; }
	void SetDampFactor(const float32 newDampFactor) { dampFactor = newDampFactor; }

	//[[nodiscard]] auto GetGravity() const { return gravity; }
	[[nodiscard]] auto GetAirDensity() const { return dampFactor; }

	HitResult TraceRay(const GTSL::Vector3 start, const GTSL::Vector3 end);

private:
	/**
	 * \brief Specifies how much speed the to remove from entities.\n
	 * Default value is 0.0001.
	 */
	float32 dampFactor = 0.001f;

	/**
	 * \brief Defines the number of substeps used for simulation. Default is 0, which mean only one iteration will run each frame.
	 */
	uint16 simSubSteps = 0;

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
	
	struct PhysicsObject
	{
		//GTSL::Buffer<BE::PAR> Buffer;
		GTSL::Vector4 Velocity, Acceleration, Position;
	};
	GTSL::FixedVector<PhysicsObject, BE::PAR> physicsObjects;

	GTSL::StaticVector<GTSL::Vector4, 8> boundlessForces;
	
	DynamicTaskHandle<StaticMeshResourceManager*, StaticMeshResourceManager::StaticMeshInfo, uint32> onStaticMeshInfoLoadedHandle;
	DynamicTaskHandle<StaticMeshResourceManager*, StaticMeshResourceManager::StaticMeshInfo, uint32> onStaticMeshLoadedHandle;
};
