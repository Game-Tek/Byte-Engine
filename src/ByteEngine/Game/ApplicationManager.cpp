#include "ApplicationManager.h"

#include "ByteEngine/Game/World.h"
#include "ByteEngine/Game/System.h"

#include "ByteEngine/Debug/FunctionTimer.h"

#include "ByteEngine/Application/ThreadPool.h"
#include "ByteEngine/Application/Application.h"

#include <GTSL/Semaphore.h>

ApplicationManager::ApplicationManager() : Object(u8"ApplicationManager"), worlds(4, GetPersistentAllocator()), systems(8, GetPersistentAllocator()), systemNames(16, GetPersistentAllocator()),
systemsMap(16, GetPersistentAllocator()), systemsIndirectionTable(64, GetPersistentAllocator()), storedDynamicTasks(16, GetPersistentAllocator()),
events(32, GetPersistentAllocator()),
recurringTasksPerStage(16, GetPersistentAllocator()),
dynamicTasksPerStage(32, GetPersistentAllocator()), asyncTasks(32, GetPersistentAllocator()),
stagesNames(8, GetPersistentAllocator()), recurringTasksInfo(32, GetPersistentAllocator()), taskSorter(128, GetPersistentAllocator())
{
}

ApplicationManager::~ApplicationManager() {
	{
		//Call shutdown in reverse order since systems initialized last during application start
		//may depend on those created before them also for shutdown
		auto shutdownSystem = [&](GTSL::SmartPointer<System, BE::PAR>& system) -> void {
			system.TryFree();
		};
		
		GTSL::ReverseForEach(systems, shutdownSystem);
	}
		
	World::DestroyInfo destroy_info;
	destroy_info.GameInstance = this;
	for (auto& world : worlds) { world->DestroyWorld(destroy_info); }
}

void ApplicationManager::OnUpdate(BE::Application* application) {
	GTSL::Vector<Stage<FunctionType, BE::TAR>, BE::TAR> localRecurringTasksPerStage(64, GetTransientAllocator());
	GTSL::Vector<Stage<FunctionType, BE::TAR>, BE::TAR> localDynamicTasksPerStage(64, GetTransientAllocator());
	
	asyncTasksMutex.WriteLock();
	Stage localAsyncTasks(asyncTasks, GetTransientAllocator());
	asyncTasks.Clear();
	asyncTasksMutex.WriteUnlock();
	
	uint32 stageCount;
	
	{
		GTSL::ReadLock lock(stagesNamesMutex); //use stagesNames vector to get length from since it has much less contention
		stageCount = stagesNames.GetLength();
	}
	
	{
		GTSL::ReadLock lock(recurringTasksMutex);
		for (uint32 i = 0; i < stageCount; ++i)
		{
			localRecurringTasksPerStage.EmplaceBack(recurringTasksPerStage[i], GetTransientAllocator());
		}
	}
	
	GTSL::Mutex waitWhenNoChange;

	auto tryDispatchTask = [&](uint16 goalIndex, Stage<FunctionType, BE::TAR>&stage, uint16& taskIndex, bool& t) {
		for(; taskIndex < stage.GetNumberOfTasks(); ++taskIndex) {
			auto result = taskSorter.CanRunTask(stage.GetTaskAccesses(taskIndex));
			if (result.State()) {
				const uint16 targetGoalIndex = stage.GetTaskGoalIndex(taskIndex);
				application->GetThreadPool()->EnqueueTask(stage.GetTask(taskIndex), this, GTSL::MoveRef(result.Get()), stage.GetTaskInfo(taskIndex));

				if (targetGoalIndex != 0xFFFF) {
					semaphores[targetGoalIndex].Add();
				}

				continue;
			}

			return;
		}

		t = true;
	};
	
	uint16 asyncTasksIndex = 0;
	
	for(uint32 stageIndex = 0; stageIndex < stageCount; ++stageIndex)
	{
		{
			GTSL::WriteLock lock(dynamicTasksPerStageMutex);
			localDynamicTasksPerStage.EmplaceBack(dynamicTasksPerStage[stageIndex], GetTransientAllocator());
			dynamicTasksPerStage[stageIndex].Clear();
		}

		getLogger()->InstantEvent(GTSL::StringView(stagesNames[stageIndex]), application->GetClock()->GetCurrentMicroseconds().GetCount()); //TODO: USE LOCK ON STAGE NAME

		uint16 recurringTasksIndex = 0, dynamicTasksIndex = 0;

		bool r = false, d = false, a = false;
		
		while (!(r && d && a)) {
			tryDispatchTask(stageIndex, localRecurringTasksPerStage[stageIndex], recurringTasksIndex, r);
			tryDispatchTask(stageIndex, localDynamicTasksPerStage[stageIndex], dynamicTasksIndex, d);
			tryDispatchTask(stageIndex, localAsyncTasks, asyncTasksIndex, a);
		}

		//waitWhenNoChange.Lock();
		//resourcesUpdated.Wait(waitWhenNoChange);

		semaphores[stageIndex].Wait();
	} //goals

	++frameNumber;
}

void ApplicationManager::UnloadWorld(const WorldReference worldId)
{
	World::DestroyInfo destroy_info;
	destroy_info.GameInstance = this;
	worlds[worldId]->DestroyWorld(destroy_info);
	worlds.Pop(worldId);
}

void ApplicationManager::RemoveTask(const Id taskName, const Id startOn) {
	uint16 i = 0;

	if constexpr (_DEBUG) {
		GTSL::ReadLock lock(stagesNamesMutex);
		GTSL::WriteLock lock2(recurringTasksMutex);
		
		if(!stagesNames.Find(startOn).State()) {
			BE_LOG_ERROR(u8"Tried to remove task ", GTSL::StringView(taskName), u8" from stage ", GTSL::StringView(startOn), u8" which doesn't exist. Resolve this issue as it leads to undefined behavior in release builds!")
			return;
		}

		i = getStageIndex(startOn);
		
		if(!recurringTasksPerStage[i].DoesTaskExist(taskName)) {
			BE_LOG_ERROR(u8"Tried to remove task ", GTSL::StringView(taskName), u8" which doesn't exist from stage ", GTSL::StringView(startOn), u8". Resolve this issue as it leads to undefined behavior in release builds!")
			return;
		}
	}
	
	{
		GTSL::ReadLock lock(stagesNamesMutex);
		i = getStageIndex(startOn);
	}

	{
		GTSL::WriteLock lock(recurringTasksMutex);
		recurringTasksPerStage[i].RemoveTask(taskName);
	}

	BE_LOG_MESSAGE(u8"Removed recurring task ", GTSL::StringView(taskName), u8" from stage ", GTSL::StringView(startOn))
}

void ApplicationManager::AddStage(Id stageName)
{
	if constexpr (_DEBUG) {
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
	
	{
		GTSL::WriteLock lock(recurringTasksMutex);
		recurringTasksPerStage.EmplaceBack(16, GetPersistentAllocator());
	}

	{
		GTSL::WriteLock lock(dynamicTasksPerStageMutex);
		dynamicTasksPerStage.EmplaceBack(16, GetPersistentAllocator());
	}

	{
		GTSL::WriteLock lock(recurringTasksInfoMutex);
		recurringTasksInfo.EmplaceBack(64, GetPersistentAllocator());
	}

	BE_LOG_MESSAGE(u8"Added stage ", GTSL::StringView(stageName))
}

void ApplicationManager::initWorld(const uint8 worldId)
{
	World::InitializeInfo initializeInfo;
	initializeInfo.GameInstance = this;
	worlds[worldId]->InitializeWorld(initializeInfo);
}