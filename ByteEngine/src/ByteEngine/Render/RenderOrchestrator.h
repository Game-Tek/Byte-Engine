#pragma once

#include <GTSL/Array.hpp>
#include <GTSL/FlatHashMap.h>


#include "ByteEngine/Game/System.h"

#include "ByteEngine/Id.h"
#include <GTSL/Vector.hpp>


#include "MaterialSystem.h"
#include "RenderTypes.h"
#include "ByteEngine/Game/Tasks.h"

class MaterialSystem;
class RenderSystem;
class RenderGroup;
struct TaskInfo;

class RenderOrchestrator : public System
{
public:
	void Initialize(const InitializeInfo& initializeInfo) override;
	void Shutdown(const ShutdownInfo& shutdownInfo) override;
	
	void Render(TaskInfo taskInfo);

	void AddRenderGroup(GameInstance* gameInstance, Id renderGroupName, RenderGroup* renderGroup);
	void RemoveRenderGroup(GameInstance* gameInstance, Id renderGroupName);

	struct RenderManager
	{
		struct RenderInfo
		{
			GTSL::Matrix4 ViewMatrix, ProjectionMatrix;
			MaterialSystem::RenderGroupData* RenderGroupData;
			GameInstance* GameInstance;
			CommandBuffer* CommandBuffer;
			uint8 CurrentFrame;
			RenderSystem* RenderSystem;
			MaterialSystem* MaterialSystem;
		};
		virtual void Render(const RenderInfo& renderInfo) = 0;
	};
	
private:
	inline static const Id RENDER_TASK_NAME{ "RenderRenderGroups" };
	inline static const Id CLASS_NAME{ "RenderOrchestrator" };
	
	GTSL::Vector<Id, BE::PersistentAllocatorReference> systems;
	GTSL::Vector<GTSL::Array<TaskDependency, 64>, BE::PersistentAllocatorReference> systemsAccesses;
	
	GTSL::FlatHashMap<RenderManager*, BE::PersistentAllocatorReference> renderManagers;
};

