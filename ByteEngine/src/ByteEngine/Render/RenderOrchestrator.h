#pragma once

#include <GTSL/Array.hpp>
#include <GTSL/FlatHashMap.h>


#include "ByteEngine/Game/System.h"

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

class RenderOrchestrator : public System
{
public:
	void Initialize(const InitializeInfo& initializeInfo) override;
	void Shutdown(const ShutdownInfo& shutdownInfo) override;
	
	void Setup(TaskInfo taskInfo);
	void Render(TaskInfo taskInfo);

	void AddRenderGroup(GameInstance* gameInstance, Id renderGroupName, RenderGroup* renderGroup);
	void RemoveRenderGroup(GameInstance* gameInstance, Id renderGroupName);

	struct RenderManager
	{
		struct RenderInfo
		{
			GameInstance* GameInstance;
			CommandBuffer* CommandBuffer;
			uint8 CurrentFrame;
			RenderSystem* RenderSystem;
			MaterialSystem* MaterialSystem;
			BindingsManager<BE::TAR>* BindingsManager;
			uint8 RenderPass, SubPass;
		};
		virtual void Render(const RenderInfo& renderInfo) = 0;

		struct SetupInfo
		{
			GameInstance* GameInstance;
			RenderSystem* RenderSystem;
			MaterialSystem* MaterialSystem;
			GTSL::Matrix4 ViewMatrix, ProjectionMatrix;
		};
		virtual void Setup(const SetupInfo& info) = 0;
	};
	
private:
	inline static const Id RENDER_TASK_NAME{ "RenderRenderGroups" };
	inline static const Id SETUP_TASK_NAME{ "SetupRenderGroups" };
	inline static const Id CLASS_NAME{ "RenderOrchestrator" };
	
	GTSL::Vector<Id, BE::PersistentAllocatorReference> systems;
	GTSL::Vector<GTSL::Array<TaskDependency, 64>, BE::PersistentAllocatorReference> systemsAccesses;
	
	GTSL::FlatHashMap<RenderManager*, BE::PersistentAllocatorReference> renderManagers;
};

