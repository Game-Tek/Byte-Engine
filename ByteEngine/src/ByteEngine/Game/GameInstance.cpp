#include "GameInstance.h"

#include "ByteEngine/Game/World.h"
#include "ByteEngine/Game/System.h"

#include "ByteEngine/Debug/Assert.h"
#include "ByteEngine/Debug/FunctionTimer.h"

#include "ByteEngine/Application/ThreadPool.h"
#include "ByteEngine/Application/Application.h"

#include <GTSL/Semaphore.h>

const char* AccessTypeToString(const AccessType access)
{
	switch (access)
	{
	case AccessType::READ: return "READ";
	case AccessType::READ_WRITE: return "READ_WRITE";
	}
}

GTSL::StaticString<1024> genTaskLog(const char* from, Id taskName, Id goalName, const GTSL::Range<const AccessType*> accesses, const GTSL::Range<const uint16*> objects)
{
	GTSL::StaticString<1024> log;
	
	log += from;
	log += taskName.GetString();
	
	log += '\n';
	
	log += " Goal: ";
	log += goalName.GetString();
	
	log += '\n';
	
	log += "With accesses: \n	";
	for (const auto& e : accesses) { log += AccessTypeToString(e); log += ", "; }
	
	//log += '\n';
	//
	//log += "Accessed objects: \n	";
	//for (const auto& e : objects)
	//{
	//	log += objectNames[e].GetString(); log += "{ "; log += e; log += " } "; log += ", ";
	//}

	return log;
}

GameInstance::GameInstance() : Object("GameInstance"), worlds(4, GetPersistentAllocator()), systems(8, GetPersistentAllocator()), systemsMap(16, GetPersistentAllocator()),
recurringGoals(16, GetPersistentAllocator()), goalNames(8, GetPersistentAllocator()), systemsIndirectionTable(64, GetPersistentAllocator()),
dynamicGoals(32, GetPersistentAllocator()),
taskSorter(64, GetPersistentAllocator()),
recurringTasksInfo(32, GetPersistentAllocator()),
dynamicTasksInfo(32, GetPersistentAllocator()),
asyncTasks(32, GetPersistentAllocator()), asyncTasksInfo(32, GetPersistentAllocator()),
freeDynamicTasks(32, GetPersistentAllocator()), freeDynamicTasksInfo(32, GetPersistentAllocator())
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
	GTSL::Vector<GTSL::Vector<GTSL::Semaphore, BE::TAR>, BE::TAR> semaphores(64, GetTransientAllocator());

	GTSL::KeepVector<decltype(freeDynamicTasks)::type, BE::TAR> localFreeDynamicTasks(64, GetTransientAllocator());
	GTSL::KeepVector<decltype(freeDynamicTasksInfo)::type, BE::TAR> localFreeDynamicTasksInfo(64, GetTransientAllocator());
	GTSL::KeepVector<decltype(asyncTasks)::type, BE::TAR> localAsyncTasks(64, GetTransientAllocator());
	GTSL::KeepVector<decltype(asyncTasksInfo)::type, BE::TAR> localAsyncTasksInfo(64, GetTransientAllocator());
	
	GTSL::Vector<uint32, BE::TAR> localDynamicGoalsNumberOfTasks(64, GetTransientAllocator());
	uint32 goalCount;
	
	{
		GTSL::ReadLock lock(goalNamesMutex); //use goalNames vector to get length from since it has much less contention
		goalCount = goalNames.GetLength();
		for (uint32 i = 0; i < goalNames.GetLength(); ++i) //beware of semaphores vector resizing invalidating pointers!
		{
			semaphores.EmplaceBack(128, GetTransientAllocator());
		}
	}
	
	TaskInfo task_info;
	task_info.GameInstance = this;
	
	for(uint32 goal = 0; goal < goalCount; ++goal)
	{
		{
			GTSL::ReadLock lock(recurringGoalsMutex);
			localRecurringGoals.EmplaceBack(recurringGoals[goal], GetTransientAllocator());
		}
		
		uint16 recurringGoalNumberOfTasks = localRecurringGoals[goal].GetNumberOfTasks();

		{
			GTSL::ReadLock lock(dynamicGoalsMutex);
			localDynamicGoals.EmplaceBack(dynamicGoals[goal], GetTransientAllocator());
		}
		
		uint16 dynamicGoalNumberOfTasks = localDynamicGoals[goal].GetNumberOfTasks();
		localDynamicGoalsNumberOfTasks.EmplaceBack(dynamicGoalNumberOfTasks);

		uint16 freeDynamicNumberOfTasks = 0;
		uint16 freeDynamicTask = 0;
		
		uint16 recurringGoalTask = recurringGoalNumberOfTasks;
		uint16 dynamicGoalTask = dynamicGoalNumberOfTasks;
		
		for (auto& semaphore : semaphores[goal]) { semaphore.Wait(); }
		
		while(recurringGoalNumberOfTasks + dynamicGoalNumberOfTasks > 0)
		{
			--recurringGoalTask;
			
			while (recurringGoalNumberOfTasks > 0) //try recurring goals
			{
				auto res = taskSorter.CanRunTask(localRecurringGoals[goal].GetTaskAccessedObjects(recurringGoalTask), localRecurringGoals[goal].GetTaskAccessTypes(recurringGoalTask));
				
				if (res.State())
				{
					const uint16 targetGoalIndex = localRecurringGoals[goal].GetTaskGoalIndex(recurringGoalTask);
					const auto semaphoreIndex = semaphores[targetGoalIndex].EmplaceBack();
					
					application->GetThreadPool()->EnqueueTask(localRecurringGoals[goal].GetTask(recurringGoalTask), &semaphores[targetGoalIndex][semaphoreIndex], this, GTSL::MoveRef(goal), GTSL::MoveRef(recurringGoalTask), GTSL::MoveRef(res.Get()));
					
					//BE_LOG_WARNING(genTaskLog("Dispatched recurring task ", localRecurringGoals[goal].GetTaskName(recurringGoalTask), goalNames[goal], localRecurringGoals[goal].GetTaskAccessTypes(recurringGoalTask), localRecurringGoals[goal].GetTaskAccessedObjects(recurringGoalTask)));
					
					--recurringGoalNumberOfTasks;
					--recurringGoalTask;
				}
				else
				{
					++recurringGoalTask;
					break;
				}
			}
			
			--dynamicGoalTask;
			
			while (dynamicGoalNumberOfTasks > 0) //try dynamic goals
			{
				auto res = taskSorter.CanRunTask(localDynamicGoals[goal].GetTaskAccessedObjects(dynamicGoalTask), localDynamicGoals[goal].GetTaskAccessTypes(dynamicGoalTask));
				
				if (res.State())
				{
					const uint16 targetGoalIndex = localDynamicGoals[goal].GetTaskGoalIndex(dynamicGoalTask);
					const auto semaphoreIndex = semaphores[targetGoalIndex].EmplaceBack();
					
					application->GetThreadPool()->EnqueueTask(localDynamicGoals[goal].GetTask(dynamicGoalTask), &semaphores[targetGoalIndex][semaphoreIndex], this, GTSL::MoveRef(goal), GTSL::MoveRef(dynamicGoalTask), GTSL::MoveRef(res.Get()));

					//BE_LOG_WARNING(genTaskLog("Dispatched dynamic task ", localDynamicGoals[goal].GetTaskName(dynamicGoalTask), goalNames[goal], localDynamicGoals[goal].GetTaskAccessTypes(dynamicGoalTask), localDynamicGoals[goal].GetTaskAccessedObjects(dynamicGoalTask)));
					
					--dynamicGoalNumberOfTasks;
					--dynamicGoalTask;
				}
				else
				{
					//BE_LOG_WARNING(genTaskLog("Tried to dispatch dynamic task ", localDynamicGoals[goal].GetTaskName(dynamicGoalTask), goalNames[goal], localDynamicGoals[goal].GetTaskAccessTypes(dynamicGoalTask), localDynamicGoals[goal].GetTaskAccessedObjects(dynamicGoalTask)));
					++dynamicGoalTask;
					break;
				}
			}

			--freeDynamicTask;
			
			{
				{
					GTSL::ReadLock lock(asyncTasksMutex);
					GTSL::ReadLock lock2(asyncTasksInfoMutex);
					localAsyncTasks.Copy(asyncTasks);
					localAsyncTasksInfo.Copy(asyncTasksInfo);
				}

				auto dispatch = [&](const uint32 i, AsyncFunctionType task)
				{
					void* data;

					{
						GTSL::WriteLock lock(asyncTasksMutex);
						GTSL::WriteLock lock0(asyncTasksInfoMutex);
						asyncTasks.Pop(i);
						data = asyncTasksInfo[i];
						asyncTasksInfo.Pop(i);
					}
					
					application->GetThreadPool()->EnqueueTask(task, &dummy, this, GTSL::MoveRef(data));
					
				};

				ForEachIndexed(localAsyncTasks, dispatch);

				localAsyncTasks.Clear();
				localAsyncTasksInfo.Clear();
			}

			{
				{
					GTSL::ReadLock lock(freeDynamicTasksMutex);
					GTSL::ReadLock lock2(freeDynamicTasksInfoMutex);
					localFreeDynamicTasks.Copy(freeDynamicTasks);
					localFreeDynamicTasksInfo.Copy(freeDynamicTasksInfo);
				}

				auto dispatchFree = [&](const uint32 index, decltype(*freeDynamicTasks.begin()) function)
				{
					auto res = taskSorter.CanRunTask(GTSL::Get<1>(freeDynamicTasks[index]), GTSL::Get<2>(freeDynamicTasks[index]));

					if (res.State())
					{
						void* data;

						{
							GTSL::ReadLock lock(freeDynamicTasksMutex);
							GTSL::ReadLock lock2(freeDynamicTasksInfoMutex);
							freeDynamicTasks.Pop(index);
							data = freeDynamicTasksInfo[index];
							freeDynamicTasksInfo.Pop(index);
						}

						application->GetThreadPool()->EnqueueTask(GTSL::Get<0>(function), &dummy, this, GTSL::MoveRef(data), GTSL::MoveRef(res.Get()));
					}
				};

				GTSL::ForEachIndexed(localFreeDynamicTasks, dispatchFree);

				localFreeDynamicTasks.Clear();
				localFreeDynamicTasksInfo.Clear();
			}
		}
	} //goals

	{
		GTSL::WriteLock lock(dynamicGoalsMutex);
		GTSL::WriteLock lock2(dynamicTasksInfoMutex);
		
		for (uint32 i = 0; i < goalCount; ++i)
		{
			dynamicGoals[i].Pop(0, localDynamicGoalsNumberOfTasks[i]);
			dynamicTasksInfo[i].Pop(0, localDynamicGoalsNumberOfTasks[i]);
		}

		//TODO keep count of done tasks as with other tasks
	}
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

	{
		GTSL::WriteLock lock(dynamicTasksInfoMutex);
		dynamicTasksInfo.EmplaceBack(64, GetPersistentAllocator());
	}

	BE_LOG_MESSAGE("Added goal ", name.GetString())
}

void GameInstance::initWorld(const uint8 worldId)
{
	World::InitializeInfo initializeInfo;
	initializeInfo.GameInstance = this;
	worlds[worldId]->InitializeWorld(initializeInfo);
}

void GameInstance::initSystem(System* system, const GTSL::Id64 name)
{
	System::InitializeInfo initializeInfo;
	initializeInfo.GameInstance = this;
	initializeInfo.ScalingFactor = scalingFactor;
	system->Initialize(initializeInfo);
}