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

GTSL::StaticString<1024> genTaskLog(const char* from, Id taskName, Id goalName, const GTSL::Ranger<const AccessType> accesses, const GTSL::Ranger<const uint16> objects, const GTSL::Ranger<const Id> objectNames)
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
	
	log += '\n';
	
	log += "Accessed objects: \n	";
	for (const auto& e : objects)
	{
		log += objectNames[e].GetString(); log += "{ "; log += e; log += " } "; log += ", ";
	}

	return log;
}

GameInstance::GameInstance() : Object("GameInstance"), worlds(4, GetPersistentAllocator()), systems(8, GetPersistentAllocator()), systemsMap(16, GetPersistentAllocator()),
recurringGoals(16, GetPersistentAllocator()), goalNames(8, GetPersistentAllocator()), objectNames(64, GetPersistentAllocator()),
dynamicGoals(32, GetPersistentAllocator()),
dynamicTasksInfo(32, GetTransientAllocator()), taskSorter(64, GetPersistentAllocator())
{
}

GameInstance::~GameInstance()
{
	{
		System::ShutdownInfo shutdownInfo;
		shutdownInfo.GameInstance = this;

		//Call shutdown in reverse order since systems initialized last during application start
		//may depend on those created before them also for shutdown
		
		for(auto* end = systems.end() - 1; end > systems.begin() - 1; --end) { (*end)->Shutdown(shutdownInfo); }
	}
		
	World::DestroyInfo destroy_info;
	destroy_info.GameInstance = this;
	for (auto& world : worlds) { world->DestroyWorld(destroy_info); }
}

void GameInstance::OnUpdate(BE::Application* application)
{
	PROFILE;

	GTSL::Vector<Goal<DispatchFunctionType, BE::TAR>, BE::TAR> localRecurringGoals(64, GetTransientAllocator());
	GTSL::Vector<Goal<DispatchFunctionType, BE::TAR>, BE::TAR> localDynamicGoals(64, GetTransientAllocator());
	GTSL::Vector<GTSL::Vector<GTSL::Semaphore, BE::TAR>, BE::TAR> semaphores(64, GetTransientAllocator());

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
			GTSL::ReadLock lock(goalsMutex);
			localRecurringGoals.EmplaceBack(recurringGoals[goal], GetTransientAllocator());
		}
		
		uint16 recurringGoalNumberOfTasks = localRecurringGoals[goal].GetNumberOfTasks();

		{
			GTSL::ReadLock lock(dynamicTasksMutex);
			localDynamicGoals.EmplaceBack(dynamicGoals[goal], GetTransientAllocator());
		}
		
		uint16 dynamicGoalNumberOfTasks = localDynamicGoals[goal].GetNumberOfTasks();

		uint16 recurringGoalTask = recurringGoalNumberOfTasks;
		uint16 dynamicGoalTask = dynamicGoalNumberOfTasks;
		
		for (auto& semaphore : semaphores[goal]) { semaphore.Wait(); }
		
		while(recurringGoalNumberOfTasks + dynamicGoalNumberOfTasks > 0)
		{
			--recurringGoalTask;
			
			while (recurringGoalNumberOfTasks > 0) //try recurring goals
			{
				GTSL::Ranger<const uint16> accessed_objects = localRecurringGoals[goal].GetTaskAccessedObjects(recurringGoalTask);
				GTSL::Ranger<const AccessType> access_types = localRecurringGoals[goal].GetTaskAccessTypes(recurringGoalTask);

				if (auto res = taskSorter.CanRunTask(accessed_objects, access_types))
				{
					const uint16 targetGoalIndex = localRecurringGoals[goal].GetTaskGoalIndex(recurringGoalTask);
					const auto semaphoreIndex = semaphores[targetGoalIndex].EmplaceBack();
					
					application->GetThreadPool()->EnqueueTask(localRecurringGoals[goal].GetTask(recurringGoalTask), &semaphores[targetGoalIndex][semaphoreIndex], this, GTSL::ForwardRef<uint16>(recurringGoalTask), res.Get());

					BE_LOG_WARNING(genTaskLog("Dispatched recurring task ", localRecurringGoals[goal].GetTaskName(recurringGoalTask), goalNames[goal], access_types, accessed_objects, objectNames));
					
					
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
				GTSL::Ranger<const uint16> accessed_objects = localDynamicGoals[goal].GetTaskAccessedObjects(dynamicGoalTask);
				GTSL::Ranger<const AccessType> access_types = localDynamicGoals[goal].GetTaskAccessTypes(dynamicGoalTask);

				if (auto res = taskSorter.CanRunTask(accessed_objects, access_types))
				{
					const uint16 targetGoalIndex = localDynamicGoals[goal].GetTaskGoalIndex(dynamicGoalTask);
					
					const auto semaphoreIndex = semaphores[targetGoalIndex].EmplaceBack();
					
					application->GetThreadPool()->EnqueueTask(localDynamicGoals[goal].GetTask(dynamicGoalTask), &semaphores[targetGoalIndex][semaphoreIndex], this, GTSL::ForwardRef<uint16>(dynamicGoalTask), res.Get());

					BE_LOG_WARNING(genTaskLog("Dispatched dynamic task ", localDynamicGoals[goal].GetTaskName(dynamicGoalTask), goalNames[goal], access_types, accessed_objects, objectNames));
					
					--dynamicGoalNumberOfTasks;
					--dynamicGoalTask;
				}
				else
				{
					++dynamicGoalTask;
					break;
				}
			}

			{
				GTSL::ReadLock lock(goalsMutex);
				localRecurringGoals[goal].AddTask(recurringGoals[goal], localRecurringGoals[goal].GetNumberOfTasks(), recurringGoals[goal].GetNumberOfTasks(), GetTransientAllocator());
				recurringGoalNumberOfTasks += localRecurringGoals[goal].GetNumberOfTasks() - recurringGoals[goal].GetNumberOfTasks();
			}
			
			{
				GTSL::ReadLock lock(dynamicTasksMutex);
				localDynamicGoals[goal].AddTask(dynamicGoals[goal], localDynamicGoals[goal].GetNumberOfTasks(), dynamicGoals[goal].GetNumberOfTasks(), GetTransientAllocator());
				dynamicGoalNumberOfTasks += localDynamicGoals[goal].GetNumberOfTasks() - dynamicGoals[goal].GetNumberOfTasks();
			}
		}

		{
			GTSL::WriteLock lock(dynamicTasksMutex);
			dynamicGoals[goal].Clear();
		}
	} //goals
}

void GameInstance::UnloadWorld(const WorldReference worldId)
{
	World::DestroyInfo destroy_info;
	destroy_info.GameInstance = this;
	worlds[worldId]->DestroyWorld(destroy_info);
	worlds.Pop(worldId);
}

void GameInstance::RemoveTask(const Id name, const Id doneFor)
{
	uint16 i = 0;
	{
		GTSL::ReadLock lock(goalNamesMutex);
		i = getGoalIndex(name);
	}
	
	goalsMutex.WriteLock();
	recurringGoals[i].RemoveTask(name);
	goalsMutex.WriteUnlock();

	BE_ASSERT(false, "No task under specified name!")
}

void GameInstance::AddGoal(Id name)
{
	{	
		GTSL::WriteLock lock(goalsMutex);
		
		recurringGoals.EmplaceBack(16, GetPersistentAllocator());
		dynamicGoals.EmplaceBack(16, GetPersistentAllocator());
	}

	{
		GTSL::WriteLock lock(goalNamesMutex);

		uint16 i = 0; for (auto goal_name : goalNames) { if (goal_name == name) break; ++i; }
		BE_ASSERT(i == goalNames.GetLength(), "There is already a goal with that name!")
		
		goalNames.EmplaceBack(name);
	}
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

//using task_sorter_t = decltype(taskSorter);
//using on_done_args = GTSL::Tuple<GTSL::Array<uint16, 64>, GTSL::Array<AccessType, 64>, task_sorter_t*>;

//auto on_done = [](const GTSL::Array<uint16, 64>& objects, const GTSL::Array<AccessType, 64>& accesses, task_sorter_t* taskSorter) -> void
//{
//	taskSorter->ReleaseResources(objects, accesses);
//	GTSL::StaticString<1024> log;
//
//	log += "Task finished";
//	log += '\n';
//	log += "With accesses: \n	";
//	for (const auto& e : accesses) { log += AccessTypeToString(e); log += ", "; }
//	log += '\n';
//	log += "Accessed objects: \n	";
//	for (const auto& e : objects) { log += e; log += ", "; }
//
//	BE::Application::Get()->GetLogger()->PrintBasicLog(BE::Logger::VerbosityLevel::SUCCESS, log);
//};

//const auto onDoneDel = GTSL::Delegate<void(const GTSL::Array<uint16, 64>&, const GTSL::Array<AccessType, 64>&, task_sorter_t*)>::Create(on_done);