#pragma once

#include <GTSL/Array.hpp>

#include "ByteEngine/Game/System.h"

#include "ByteEngine/Id.h"
#include <GTSL/Vector.hpp>

#include "ByteEngine/Game/Tasks.h"

class RenderGroup;
struct TaskInfo;

class RenderOrchestrator : public System
{
public:
	void Initialize(const InitializeInfo& initializeInfo) override;
	
	void Render(TaskInfo taskInfo);

	void AddRenderGroup(GameInstance* gameInstance, Id renderGroupName, RenderGroup* renderGroup);
	void RemoveRenderGroup(GameInstance* gameInstance, Id renderGroupName);
private:
	static constexpr Id RENDER_TASK_NAME{ "RenderRenderGroups" };
	static constexpr Id CLASS_NAME{ "RenderOrchestrator" };
	
	GTSL::Vector<Id, BE::PersistentAllocatorReference> systems;
	GTSL::Vector<GTSL::Array<TaskDependency, 64>, BE::PersistentAllocatorReference> systemsAccesses;
};

