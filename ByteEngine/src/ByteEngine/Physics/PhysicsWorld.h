#pragma once

#include <GTSL/KeepVector.h>
#include <GTSL/Math/Vector3.h>
#include <GTSL/Math/Vector4.h>



#include "HitResult.h"
#include "ByteEngine/Game/System.h"
#include "ByteEngine/Handle.hpp"
#include "ByteEngine/Game/GameInstance.h"
#include "ByteEngine/Resources/StaticMeshResourceManager.h"

class StaticMeshResourceManager;
MAKE_HANDLE(uint32, PhysicsObject);

class PhysicsWorld : public System
{
public:

	void Initialize(const InitializeInfo& initializeInfo) override
	{
		physicsObjects.Initialize(32, GetPersistentAllocator()); updatedObjects.Initialize(32, GetPersistentAllocator());
		initializeInfo.GameInstance->AddTask("onUpdate", Task<>::Create<PhysicsWorld, &PhysicsWorld::onUpdate>(this), {}, "FrameUpdate", "RenderStart");

		onStaticMeshInfoLoadedHandle = initializeInfo.GameInstance->StoreDynamicTask("onStaticMeshInfoLoad", Task<StaticMeshResourceManager*, StaticMeshResourceManager::StaticMeshInfo, uint32>::Create<PhysicsWorld, &PhysicsWorld::onStaticMeshInfoLoaded>(this), {});
		onStaticMeshLoadedHandle = initializeInfo.GameInstance->StoreDynamicTask("onStaticMeshLoad", Task<StaticMeshResourceManager*, StaticMeshResourceManager::StaticMeshInfo, uint32>::Create<PhysicsWorld, &PhysicsWorld::onStaticMeshLoaded>(this), {});

		boundlessForces.EmplaceBack(0, -10, 0, 0);
	}
	
	void Shutdown(const ShutdownInfo& shutdownInfo) override;

	PhysicsObjectHandle AddPhysicsObject(GameInstance* gameInstance, Id meshName, StaticMeshResourceManager* staticMeshResourceManager);

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
	float32 dampFactor = 0.001;

	/**
	 * \brief Defines the number of substeps used for simulation. Default is 0, which mean only one iteration will run each frame.
	 */
	uint16 simSubSteps = 0;

	GTSL::Vector<PhysicsObjectHandle, BE::PAR> updatedObjects;
	
	void doBroadPhase();
	void doNarrowPhase();
	void solveDynamicObjects(double _UpdateTime);

	void onUpdate(TaskInfo taskInfo);

	void onStaticMeshInfoLoaded(TaskInfo taskInfo, StaticMeshResourceManager* staticMeshResourceManager, StaticMeshResourceManager::StaticMeshInfo staticMeshInfo, uint32);
	void onStaticMeshLoaded(TaskInfo taskInfo, StaticMeshResourceManager* staticMeshResourceManager, StaticMeshResourceManager::StaticMeshInfo staticMeshInfo, uint32);
	
	struct PhysicsObject
	{
		GTSL::Buffer<BE::PAR> Buffer;
		GTSL::Vector4 Velocity, Acceleration, Position;
	};
	GTSL::KeepVector<PhysicsObject, BE::PAR> physicsObjects;

	GTSL::Array<GTSL::Vector4, 8> boundlessForces;
	
	DynamicTaskHandle<StaticMeshResourceManager*, StaticMeshResourceManager::StaticMeshInfo, uint32> onStaticMeshInfoLoadedHandle;
	DynamicTaskHandle<StaticMeshResourceManager*, StaticMeshResourceManager::StaticMeshInfo, uint32> onStaticMeshLoadedHandle;
};
