#pragma once

#include "ByteEngine/Game/System.h"

#include <GTSL/Array.hpp>
#include <GTSL/FlatHashMap.h>

#include "ByteEngine/Id.h"
#include <GTSL/Vector.hpp>

#include "BindingsManager.hpp"
#include "MaterialSystem.h"
#include "RenderTypes.h"
#include "ByteEngine/Game/Tasks.h"

class MaterialSystem;
class RenderSystem;
class RenderGroup;
struct TaskInfo;

class RenderManager : public System
{
public:
	virtual void GetSetupAccesses(GTSL::Array<TaskDependency, 16>& dependencies) = 0;

	struct SetupInfo
	{
		GameInstance* GameInstance;
		RenderSystem* RenderSystem;
		MaterialSystem* MaterialSystem;
		GTSL::Matrix4 ViewMatrix, ProjectionMatrix;
	};
	virtual void Setup(const SetupInfo& info) = 0;
};

class StaticMeshRenderManager : public RenderManager
{
	void Initialize(const InitializeInfo& initializeInfo) override;
	void Shutdown(const ShutdownInfo& shutdownInfo) override {}
	
	void GetSetupAccesses(GTSL::Array<TaskDependency, 16>& dependencies) override;

	void Setup(const SetupInfo& info) override;

private:
	uint64 matrixUniformBufferMemberHandle;
};

class UIRenderManager : public RenderManager
{
	void Initialize(const InitializeInfo& initializeInfo) override;
	void Shutdown(const ShutdownInfo& shutdownInfo) override {}
	
	void GetSetupAccesses(GTSL::Array<TaskDependency, 16>& dependencies) override;

	void Setup(const SetupInfo& info) override;

private:
	ComponentReference square;
};

class RenderOrchestrator : public System
{
public:
	void Initialize(const InitializeInfo& initializeInfo) override;
	void Shutdown(const ShutdownInfo& shutdownInfo) override;
	
	void Setup(TaskInfo taskInfo);
	void Render(TaskInfo taskInfo);

	void AddRenderManager(GameInstance* gameInstance, const Id renderManager, const uint16 systemReference);
	void RemoveRenderManager(GameInstance* gameInstance, const Id renderManager, const uint16 systemReference);
private:
	inline static const Id RENDER_TASK_NAME{ "RenderRenderGroups" };
	inline static const Id SETUP_TASK_NAME{ "SetupRenderGroups" };
	inline static const Id CLASS_NAME{ "RenderOrchestrator" };
	
	GTSL::Vector<Id, BE::PersistentAllocatorReference> systems;
	GTSL::Vector<GTSL::Array<TaskDependency, 32>, BE::PersistentAllocatorReference> setupSystemsAccesses;
	GTSL::Vector<GTSL::Array<TaskDependency, 32>, BE::PersistentAllocatorReference> renderSystemsAccesses;
	
	GTSL::FlatHashMap<uint16, BE::PersistentAllocatorReference> renderManagers;
};