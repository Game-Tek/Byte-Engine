#include "GameInstance.h"

#include "ByteEngine/Game/World.h"
#include "ByteEngine/Game/System.h"

#include "ByteEngine/Debug/Assert.h"
#include "ByteEngine/Debug/FunctionTimer.h"

#include "ByteEngine/Application/ThreadPool.h"
#include "ByteEngine/Application/Application.h"

#include <GTSL/Semaphore.h>

GameInstance::GameInstance() : Object("GameInstance"), worlds(4, GetPersistentAllocator()), systems(8, GetPersistentAllocator()), systemsMap(16, GetPersistentAllocator()),
recurringGoals(16, GetPersistentAllocator()), goalNames(8, GetPersistentAllocator()), systemsIndirectionTable(64, GetPersistentAllocator()),
dynamicGoals(32, GetPersistentAllocator()),
taskSorter(64, GetPersistentAllocator()),
recurringTasksInfo(32, GetPersistentAllocator()),
asyncTasks(32, GetPersistentAllocator()), semaphores(16, GetPersistentAllocator()), systemNames(16, GetPersistentAllocator())
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

	GTSL::Vector<Goal<FunctionType, BE::TAR>, BE::TAR> localRecurringGoals(64, GetTransientAllocator());
	GTSL::Vector<Goal<FunctionType, BE::TAR>, BE::TAR> localDynamicGoals(64, GetTransientAllocator());

	asyncTasksMutex.ReadLock();
	Goal<FunctionType, BE::TAR> localAsyncTasks(asyncTasks, GetTransientAllocator());
	asyncTasks.Clear();
	asyncTasksMutex.ReadUnlock();
	
	uint32 goalCount;
	
	{
		GTSL::ReadLock lock(goalNamesMutex); //use goalNames vector to get length from since it has much less contention
		goalCount = goalNames.GetLength();
	}
	
	GTSL::Mutex waitWhenNoChange;

	TaskInfo task_info;
	task_info.GameInstance = this;
	
	uint16 asyncTasksIndex = localAsyncTasks.GetNumberOfTasks();

	for(uint32 goal = 0; goal < goalCount; ++goal)
	{
		{
			GTSL::ReadLock lock(recurringGoalsMutex);
			localRecurringGoals.EmplaceBack(recurringGoals[goal], GetTransientAllocator());
		}
		
		uint16 recurringTasksIndex = localRecurringGoals[goal].GetNumberOfTasks();

		{
			GTSL::ReadLock lock(dynamicGoalsMutex);
			localDynamicGoals.EmplaceBack(dynamicGoals[goal], GetTransientAllocator());
			dynamicGoals[goal].Clear();
		}
		
		uint16 dynamicTasksIndex = localDynamicGoals[goal].GetNumberOfTasks();
		
		while(recurringTasksIndex + dynamicTasksIndex + asyncTasksIndex > 0)
		{
			if(recurringTasksIndex)
			{
				auto index = recurringTasksIndex - 1;
				auto res = taskSorter.CanRunTask(localRecurringGoals[goal].GetTaskAccessedObjects(index), localRecurringGoals[goal].GetTaskAccessTypes(index));
				
				if (res.State())
				{
					const uint16 targetGoalIndex = localRecurringGoals[goal].GetTaskGoalIndex(index);
					
					application->GetThreadPool()->EnqueueTask(localRecurringGoals[goal].GetTask(index), this, GTSL::MoveRef(goal), GTSL::MoveRef(res.Get()), localRecurringGoals[goal].GetTaskInfo(index));
					
					//BE_LOG_MESSAGE(genTaskLog("Dispatched recurring task ", localRecurringGoals[goal].GetTaskName(recurringGoalTask), goalNames[goal], localRecurringGoals[goal].GetTaskAccessTypes(recurringGoalTask), localRecurringGoals[goal].GetTaskAccessedObjects(recurringGoalTask)));
					
					--recurringTasksIndex;
					semaphores[goal].Add();
				}
			}

			if(dynamicTasksIndex)
			{
				auto index = dynamicTasksIndex - 1;
				auto res = taskSorter.CanRunTask(localDynamicGoals[goal].GetTaskAccessedObjects(index), localDynamicGoals[goal].GetTaskAccessTypes(index));
				
				if (res.State())
				{
					const uint16 targetGoalIndex = localDynamicGoals[goal].GetTaskGoalIndex(index);
					
					application->GetThreadPool()->EnqueueTask(localDynamicGoals[goal].GetTask(index), this, GTSL::MoveRef(goal), GTSL::MoveRef(res.Get()), localDynamicGoals[goal].GetTaskInfo(index));

					//BE_LOG_MESSAGE(genTaskLog("Dispatched dynamic task ", localDynamicGoals[goal].GetTaskName(dynamicGoalTask), goalNames[goal], localDynamicGoals[goal].GetTaskAccessTypes(dynamicGoalTask), localDynamicGoals[goal].GetTaskAccessedObjects(dynamicGoalTask)));
					
					--dynamicTasksIndex;
					semaphores[goal].Add();
				}
			}

			if(asyncTasksIndex)
			{
				auto index = asyncTasksIndex - 1;
				auto res = taskSorter.CanRunTask(localAsyncTasks.GetTaskAccessedObjects(index), localAsyncTasks.GetTaskAccessTypes(index));

				if (res.State())
				{					
					application->GetThreadPool()->EnqueueTask(localAsyncTasks.GetTask(index), this, GTSL::MoveRef(goal), GTSL::MoveRef(res.Get()), localAsyncTasks.GetTaskInfo(index));

					//BE_LOG_MESSAGE(genTaskLog("Dispatched async task ", localAsyncTasks.GetTaskName(index), localAsyncTasks.GetTaskAccessTypes(index), localAsyncTasks.GetTaskAccessedObjects(index)), "frame: ", frameNumber);

					--asyncTasksIndex;
				}
			}

			//waitWhenNoChange.Lock();
			//resourcesUpdated.Wait(waitWhenNoChange);
		}

		semaphores[goal].Wait();
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
		GTSL::ReadLock lock(goalNamesMutex);
		GTSL::WriteLock lock2(recurringGoalsMutex);
		
		if(goalNames.Find(startOn) == goalNames.end()) {
			BE_LOG_WARNING("Tried to remove task ", name.GetString(), " from goal ", startOn.GetString(), " which doesn't exist. Resolve this issue as it leads to undefined behavior in release builds!")
			return;
		}

		i = getGoalIndex(startOn);
		
		if(!recurringGoals[i].DoesTaskExist(name)) {
			BE_LOG_WARNING("Tried to remove task ", name.GetString(), " which doesn't exist from goal ", startOn.GetString(), ". Resolve this issue as it leads to undefined behavior in release builds!")
			return;
		}
	}
	
	{
		GTSL::ReadLock lock(goalNamesMutex);
		i = getGoalIndex(startOn);
	}

	{
		GTSL::WriteLock lock(recurringGoalsMutex);
		recurringGoals[i].RemoveTask(name);
	}

	BE_LOG_MESSAGE("Removed recurring task ", name.GetString(), " from goal ", startOn.GetString())
}

void GameInstance::AddGoal(Id name)
{
	if constexpr (_DEBUG) {
		GTSL::WriteLock lock(goalNamesMutex);
		if (goalNames.Find(name) != goalNames.end()) {
			BE_LOG_WARNING("Tried to add goal ", name.GetString(), " which already exists. Resolve this issue as it leads to undefined behavior in release builds!")
			return;
		}
	}

	{
		GTSL::WriteLock lock(goalNamesMutex);
		goalNames.EmplaceBack(name);
	}
	
	{
		GTSL::WriteLock lock(recurringGoalsMutex);
		recurringGoals.EmplaceBack(16, GetPersistentAllocator());
	}

	{
		GTSL::WriteLock lock(dynamicGoalsMutex);
		dynamicGoals.EmplaceBack(16, GetPersistentAllocator());
	}

	{
		GTSL::WriteLock lock(recurringTasksInfoMutex);
		recurringTasksInfo.EmplaceBack(64, GetPersistentAllocator());
	}

	semaphores.EmplaceBack();

	BE_LOG_MESSAGE("Added goal ", name.GetString())
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