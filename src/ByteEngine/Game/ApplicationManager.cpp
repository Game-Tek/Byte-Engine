#include "ApplicationManager.h"

#include "ByteEngine/Game/World.h"
#include "ByteEngine/Game/System.hpp"

#include "ByteEngine/Debug/FunctionTimer.h"

#include "ByteEngine/Application/ThreadPool.h"
#include "ByteEngine/Application/Application.h"

#include <GTSL/Semaphore.h>

ApplicationManager::ApplicationManager() : Object(u8"ApplicationManager"), worlds(4, GetPersistentAllocator()), systems(8, GetPersistentAllocator()), systemNames(16, GetPersistentAllocator()),
systemsMap(16, GetPersistentAllocator()), systemsIndirectionTable(64, GetPersistentAllocator()), events(32, GetPersistentAllocator()), tasks(128, GetPersistentAllocator()), stagesNames(8, GetPersistentAllocator()), taskSorter(128, GetPersistentAllocator()), systemsData(16, GetPersistentAllocator()), functionToTaskMap(128, GetPersistentAllocator())
{
}

ApplicationManager::~ApplicationManager() {
	{
		//Call shutdown in reverse order since systems initialized last during application start
		//may depend on those created before them also for shutdown
		auto shutdownSystem = [&](GTSL::SmartPointer<BE::System, BE::PAR>& system) -> void {
			system.TryFree();
		};
		
		GTSL::ReverseForEach(systems, shutdownSystem);
	}
		
	World::DestroyInfo destroy_info;
	destroy_info.GameInstance = this;
	for (auto& world : worlds) { world->DestroyWorld(destroy_info); }
}

void ApplicationManager::OnUpdate(BE::Application* application) {
	GTSL::Vector<TypeErasedTaskHandle, BE::TAR> taskStack(64, GetTransientAllocator()); // Holds all tasks which are to be executed

	GTSL::Vector<uint32, BE::TAR> perStageCounter(32, GetTransientAllocator()); // Maintains the count of how many tasks were executed for each stage. It's used to know when an stage can advance.

	for(uint32 si = 0; si < stagesNames; ++si) { // Loads all recurrent task onto the stack
		
	}

	GTSL::Mutex waitWhenNoChange;

	auto tryDispatchTask = [&](uint16 goalIndex, Stage<FunctionType, BE::TAR>&stage, uint16& taskIndex, bool& t) {
		for(; taskIndex < stage.GetNumberOfTasks(); ++taskIndex) {
			auto result = taskSorter.CanRunTask(stage.GetTaskAccesses(taskIndex));
			if (result.State()) {
				const uint16 targetStageIndex = stage.GetTaskGoalIndex(taskIndex);
				application->GetThreadPool()->EnqueueTask(stage.GetTask(taskIndex), this, GTSL::MoveRef(result.Get()), stage.GetTaskInfo(taskIndex));

				if (targetStageIndex != 0xFFFF) {
					semaphores[targetStageIndex].Add();
					++perStageCounter[targetStageIndex];
				}

				continue;
			}

			return;
		}

		t = true;
	};

	uint16 stageIndex = 0;

	while(taskStack) { // While there are elements to be processed
		getLogger()->InstantEvent(GTSL::StringView(stagesNames[stageIndex]), application->GetClock()->GetCurrentMicroseconds().GetCount()); //TODO: USE LOCK ON STAGE NAME
		
		semaphores[stageIndex].Wait();
	}

	++frameNumber;
}

void ApplicationManager::UnloadWorld(const WorldReference worldId)
{
	World::DestroyInfo destroy_info;
	destroy_info.GameInstance = this;
	worlds[worldId]->DestroyWorld(destroy_info);
	worlds.Pop(worldId);
}

BE::TypeIdentifer ApplicationManager::RegisterType(const BE::System* system, const GTSL::StringView type_name) {
	uint16 id = system->systemId;
	uint16 typeId = systemsData[id].RegisteredTypes.GetLength();

	systemsData[id].RegisteredTypes.EmplaceBack(GetPersistentAllocator());

	return { id, typeId };
}

void ApplicationManager::RemoveTask(const Id taskName, const Id startOn) {
	uint16 i = 0;

	if constexpr (BE_DEBUG) {
		GTSL::ReadLock lock(stagesNamesMutex);
		
		if(!stagesNames.Find(startOn).State()) {
			BE_LOG_ERROR(u8"Tried to remove task ", GTSL::StringView(taskName), u8" from stage ", GTSL::StringView(startOn), u8" which doesn't exist. Resolve this issue as it leads to undefined behavior in release builds!")
			return;
		}

		i = getStageIndex(startOn);
	}
	
	{
		GTSL::ReadLock lock(stagesNamesMutex);
		i = getStageIndex(startOn);
	}

	BE_LOG_MESSAGE(u8"Removed recurring task ", GTSL::StringView(taskName), u8" from stage ", GTSL::StringView(startOn))
}

void ApplicationManager::AddStage(Id stageName)
{
	if constexpr (BE_DEBUG) {
		GTSL::WriteLock lock(stagesNamesMutex);
		if (stagesNames.Find(stageName).State()) {
			BE_LOG_ERROR(u8"Tried to add stage ", GTSL::StringView(stageName), u8" which already exists. Resolve this issue as it leads to undefined behavior in release builds!")
			return;
		}
	}

	{
		GTSL::WriteLock lock(stagesNamesMutex);
		stagesNames.EmplaceBack(stageName);
	}

	BE_LOG_MESSAGE(u8"Added stage ", GTSL::StringView(stageName))
}

void ApplicationManager::initWorld(const uint8 worldId)
{
	World::InitializeInfo initializeInfo;
	initializeInfo.GameInstance = this;
	worlds[worldId]->InitializeWorld(initializeInfo);
}