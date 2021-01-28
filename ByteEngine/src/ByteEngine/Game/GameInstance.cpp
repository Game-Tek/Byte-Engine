#include "GameInstance.h"

#include "ByteEngine/Game/World.h"
#include "ByteEngine/Game/System.h"

#include "ByteEngine/Debug/Assert.h"
#include "ByteEngine/Debug/FunctionTimer.h"

#include "ByteEngine/Application/ThreadPool.h"
#include "ByteEngine/Application/Application.h"

#include <GTSL/Semaphore.h>

GameInstance::GameInstance() : Object("GameInstance"), worlds(4, GetPersistentAllocator()), systems(8, GetPersistentAllocator()), systemsMap(16, GetPersistentAllocator()),
recurringTasksPerStage(16, GetPersistentAllocator()), stagesNames(8, GetPersistentAllocator()), systemsIndirectionTable(64, GetPersistentAllocator()),
dynamicTasksPerStage(32, GetPersistentAllocator()),
taskSorter(64, GetPersistentAllocator()),
recurringTasksInfo(32, GetPersistentAllocator()),
asyncTasks(32, GetPersistentAllocator()), semaphores(16, GetPersistentAllocator()), systemNames(16, GetPersistentAllocator()), storedDynamicTasks(16, GetPersistentAllocator())
{
}

GameInstance::~GameInstance()
{
	{
		System::ShutdownInfo shutdownInfo;
		shutdownInfo.GameInstance = this;

		//Call shutdown in reverse order since systems initialized last during application start
		//may depend on those created before them also for shutdown
		auto shutdownSystem = [&](System* system) -> void
		{
			system->Shutdown(shutdownInfo);
		};
		
		GTSL::ReverseForEach(systems, shutdownSystem);
	}
		
	World::DestroyInfo destroy_info;
	destroy_info.GameInstance = this;
	for (auto& world : worlds) { world->DestroyWorld(destroy_info); }
}

void GameInstance::OnUpdate(BE::Application* application)
{
	PROFILE;

	GTSL::Vector<Stage<FunctionType, BE::TAR>, BE::TAR> localRecurringTasksPerStage(64, GetTransientAllocator());
	GTSL::Vector<Stage<FunctionType, BE::TAR>, BE::TAR> localDynamicTasksPerStage(64, GetTransientAllocator());

	asyncTasksMutex.ReadLock();
	Stage<FunctionType, BE::TAR> localAsyncTasks(asyncTasks, GetTransientAllocator());
	asyncTasks.Clear();
	asyncTasksMutex.ReadUnlock();
	
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

	TaskInfo task_info;
	task_info.GameInstance = this;
	
	uint16 asyncTasksIndex = localAsyncTasks.GetNumberOfTasks();

	auto tryDispatchGoalTask = [&](uint16 goalIndex, Stage<GameInstance::FunctionType, BE::TAR>&stage, uint16& taskIndex)
	{
		if (taskIndex)
		{
			auto index = taskIndex - 1;
			auto result = taskSorter.CanRunTask(stage.GetTaskAccessedObjects(index), stage.GetTaskAccessTypes(index));
			if (result.State())
			{
				const uint16 targetGoalIndex = stage.GetTaskGoalIndex(index);
				application->GetThreadPool()->EnqueueTask(stage.GetTask(index), this, GTSL::MoveRef(targetGoalIndex), GTSL::MoveRef(result.Get()), stage.GetTaskInfo(index));
				//BE_LOG_MESSAGE(genTaskLog("Dispatched recurring task ", localRecurringGoals[stage].GetTaskName(recurringGoalTask), stagesNames[stage], localRecurringGoals[stage].GetTaskAccessTypes(recurringGoalTask), localRecurringGoals[stage].GetTaskAccessedObjects(recurringGoalTask)));
				--taskIndex;
				semaphores[targetGoalIndex].Add();
			}
		}
	};

	auto tryDispatchTask = [&](Stage<GameInstance::FunctionType, BE::TAR>&stage, uint16& taskIndex)
	{
		if (taskIndex)
		{
			auto index = taskIndex - 1;
			auto result = taskSorter.CanRunTask(stage.GetTaskAccessedObjects(index), stage.GetTaskAccessTypes(index));
			if (result.State())
			{
				application->GetThreadPool()->EnqueueTask(stage.GetTask(index), this, 0xFFFF, GTSL::MoveRef(result.Get()), stage.GetTaskInfo(index));
				//BE_LOG_MESSAGE(genTaskLog("Dispatched recurring task ", localRecurringGoals[stage].GetTaskName(recurringGoalTask), stagesNames[stage], localRecurringGoals[stage].GetTaskAccessTypes(recurringGoalTask), localRecurringGoals[stage].GetTaskAccessedObjects(recurringGoalTask)));
				--taskIndex;
			}
		}
	};
	
	for(uint32 stageIndex = 0; stageIndex < stageCount; ++stageIndex)
	{		
		uint16 recurringTasksIndex = localRecurringTasksPerStage[stageIndex].GetNumberOfTasks();

		{
			GTSL::ReadLock lock(dynamicTasksPerStageMutex);
			localDynamicTasksPerStage.EmplaceBack(dynamicTasksPerStage[stageIndex], GetTransientAllocator());
			dynamicTasksPerStage[stageIndex].Clear();
		}
		
		uint16 dynamicTasksIndex = localDynamicTasksPerStage[stageIndex].GetNumberOfTasks();
		
		while(recurringTasksIndex + dynamicTasksIndex + asyncTasksIndex > 0)
		{
			tryDispatchGoalTask(stageIndex, localRecurringTasksPerStage[stageIndex], recurringTasksIndex);
			tryDispatchGoalTask(stageIndex, localDynamicTasksPerStage[stageIndex], dynamicTasksIndex);
			tryDispatchTask(localAsyncTasks, asyncTasksIndex);

			//waitWhenNoChange.Lock();
			//resourcesUpdated.Wait(waitWhenNoChange);
		}

		semaphores[stageIndex].Wait();
	} //goals

	++frameNumber;
}

void GameInstance::UnloadWorld(const WorldReference worldId)
{
	World::DestroyInfo destroy_info;
	destroy_info.GameInstance = this;
	worlds[worldId]->DestroyWorld(destroy_info);
	worlds.Pop(worldId);
}

void GameInstance::RemoveTask(const Id name, const Id startOn)
{
	uint16 i = 0;

	if constexpr (_DEBUG) {
		GTSL::ReadLock lock(stagesNamesMutex);
		GTSL::WriteLock lock2(recurringTasksMutex);
		
		if(stagesNames.Find(startOn) == stagesNames.end()) {
			BE_LOG_WARNING("Tried to remove task ", name.GetString(), " from stage ", startOn.GetString(), " which doesn't exist. Resolve this issue as it leads to undefined behavior in release builds!")
			return;
		}

		i = getStageIndex(startOn);
		
		if(!recurringTasksPerStage[i].DoesTaskExist(name)) {
			BE_LOG_WARNING("Tried to remove task ", name.GetString(), " which doesn't exist from stage ", startOn.GetString(), ". Resolve this issue as it leads to undefined behavior in release builds!")
			return;
		}
	}
	
	{
		GTSL::ReadLock lock(stagesNamesMutex);
		i = getStageIndex(startOn);
	}

	{
		GTSL::WriteLock lock(recurringTasksMutex);
		recurringTasksPerStage[i].RemoveTask(name);
	}

	BE_LOG_MESSAGE("Removed recurring task ", name.GetString(), " from stage ", startOn.GetString())
}

void GameInstance::AddStage(Id name)
{
	if constexpr (_DEBUG) {
		GTSL::WriteLock lock(stagesNamesMutex);
		if (stagesNames.Find(name) != stagesNames.end()) {
			BE_LOG_WARNING("Tried to add stage ", name.GetString(), " which already exists. Resolve this issue as it leads to undefined behavior in release builds!")
			return;
		}
	}

	{
		GTSL::WriteLock lock(stagesNamesMutex);
		stagesNames.EmplaceBack(name);
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

	semaphores.EmplaceBack();

	BE_LOG_MESSAGE("Added stage ", name.GetString())
}

void GameInstance::initWorld(const uint8 worldId)
{
	World::InitializeInfo initializeInfo;
	initializeInfo.GameInstance = this;
	worlds[worldId]->InitializeWorld(initializeInfo);
}

void GameInstance::initSystem(System* system, const GTSL::Id64 name, const uint16 id)
{
	System::InitializeInfo initializeInfo;
	system->systemId = id;
	initializeInfo.GameInstance = this;
	initializeInfo.ScalingFactor = scalingFactor;
	system->Initialize(initializeInfo);
}