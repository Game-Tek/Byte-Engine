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

GameInstance::GameInstance() : Object("GameInstance"), worlds(4, GetPersistentAllocator()), systems(8, GetPersistentAllocator()), systemsMap(16, GetPersistentAllocator()),
recurringGoals(16, GetPersistentAllocator()), goalNames(8, GetPersistentAllocator()), objectNames(64, GetPersistentAllocator()),
dynamicGoals(32, GetPersistentAllocator()),
dynamicTasksInfo(32, GetTransientAllocator()), task_sorter(64, GetPersistentAllocator())
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

	GTSL::Vector<Goal<TaskType, BE::TAR>, BE::TAR> localRecurringGoals(64, GetTransientAllocator());
	GTSL::Vector<Goal<DynamicTaskFunctionType, BE::TAR>, BE::TAR> localDynamicGoals(64, GetTransientAllocator());
	GTSL::Vector<GTSL::Vector<GTSL::Semaphore, BE::TAR>, BE::TAR> semaphores(64, GetTransientAllocator());

	uint32 goalCount;
	
	{
		GTSL::ReadLock lock(goalNamesMutex); //use goalNames vector to get length from since it has much less contention
		goalCount = goalNames.GetLength();
		for (uint32 i = 0; i < goalNames.GetLength(); ++i)
		{
			semaphores.EmplaceBack(128, GetTransientAllocator());
		}
	}
	
	TaskInfo task_info;
	task_info.GameInstance = this;

	using task_sorter_t = decltype(task_sorter);
	using on_done_args = GTSL::Tuple<GTSL::Array<uint16, 64>, GTSL::Array<AccessType, 64>, task_sorter_t*>;
	
	auto on_done = [](const GTSL::Array<uint16, 64>& objects, const GTSL::Array<AccessType, 64>& accesses, task_sorter_t* taskSorter) -> void
	{
		taskSorter->ReleaseResources(objects, accesses);
		GTSL::StaticString<1024> log;

		log += "Task finished";
		log += '\n';
		log += "With accesses: \n";
		for (const auto& e : accesses) { log += AccessTypeToString(e); log += ", "; }
		log += '\n';
		log += "Accessed objects: \n";
		for (const auto& e : objects) { log += e; log += ", "; }

		//BE::Application::Get()->GetLogger()->PrintBasicLog(BE::Logger::VerbosityLevel::SUCCESS, log);
	};

	const auto onDoneDel = GTSL::Delegate<void(const GTSL::Array<uint16, 64>&, const GTSL::Array<AccessType, 64>&, task_sorter_t*)>::Create(on_done);
	
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
		
		auto& locRecGoal = localRecurringGoals[goal];
		auto& locDynGoal = localDynamicGoals[goal];
		auto& recGoal = recurringGoals[goal];
		auto& dynGoal = dynamicGoals[goal];
		
		while(recurringGoalNumberOfTasks + dynamicGoalNumberOfTasks > 0)
		{
			--recurringGoalTask;
			
			while (recurringGoalNumberOfTasks > 0) //try recurring goals
			{
				GTSL::Ranger<const uint16> accessed_objects = localRecurringGoals[goal].GetTaskAccessedObjects(recurringGoalTask);
				GTSL::Ranger<const AccessType> access_types = localRecurringGoals[goal].GetTaskAccessTypes(recurringGoalTask);

				if (task_sorter.CanRunTask(accessed_objects, access_types))
				{
					on_done_args done_args(accessed_objects, access_types, &task_sorter);

					const uint16 target_goal = localRecurringGoals[goal].GetTaskGoalIndex(recurringGoalTask);
					
					application->GetThreadPool()->EnqueueTask(localRecurringGoals[goal].GetTask(recurringGoalTask), onDoneDel,
						&semaphores[target_goal][semaphores[target_goal].EmplaceBack()], MoveRef(done_args), task_info);

					//GTSL::StaticString<1024> log;

					//log += "Dispatched recurring task ";
					//log += recurringGoalTask;
					//log += " of ";
					//log += recurringGoalNumberOfTasks;
					//log += '\n';
					//log += " Goal: ";
					//log += goal;
					//log += '\n';
					//log += "With accesses: \n";
					//for (const auto& e : access_types) { log += AccessTypeToString(e); log += ", "; }
					//log += '\n';
					//log += "Accessed objects: \n";
					//for (const auto& e : accessed_objects) { log += e; log += ", "; }

					//BE_LOG_WARNING(log);
					
					
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

				if (task_sorter.CanRunTask(accessed_objects, access_types))
				{
					on_done_args done_args(accessed_objects, access_types, &task_sorter);
					const uint16 target_goal = localDynamicGoals[goal].GetTaskGoalIndex(dynamicGoalTask);
					const auto semaphore_index = semaphores[target_goal].EmplaceBack();
					
					application->GetThreadPool()->EnqueueTask(localDynamicGoals[goal].GetTask(dynamicGoalTask), onDoneDel, &semaphores[target_goal][semaphore_index],
						MoveRef(done_args), this, GTSL::ForwardRef<uint16>(dynamicGoalTask));

					//GTSL::StaticString<1024> log;
					
					//log += "Dispatched dynamic task ";
					//log += dynamicGoalTask;
					//log += " of ";
					//log += dynamicGoalNumberOfTasks;
					//log += '\n';
					//log += " Goal: ";
					//log += goal;
					//log += '\n';
					//log += "With accesses: \n";
					//for (const auto& e : access_types) { log += AccessTypeToString(e); log += ", "; }
					//log += '\n';
					//log += "Accessed objects: \n";
					//for (const auto& e : accessed_objects) { log += e; log += ", "; }

					//BE_LOG_WARNING(log);
					
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

void GameInstance::AddTask(const GTSL::Id64 name, const GTSL::Delegate<void(TaskInfo)> function, const GTSL::Ranger<const TaskDependency> actsOn, const GTSL::Id64 startsOn, const GTSL::Id64 doneFor)
{
	GTSL::Array<uint16, 32> objects; GTSL::Array<AccessType, 32> accesses;

	uint16 goal_index, target_goal_index;

	{
		GTSL::ReadLock lock(goalNamesMutex);
		decomposeTaskDescriptor(actsOn, objects, accesses);
		goal_index = getGoalIndex(startsOn);
		target_goal_index = getGoalIndex(doneFor);
	}

	{
		GTSL::WriteLock lock(goalsMutex);
		recurringGoals[goal_index].AddTask(name, function, objects, accesses, target_goal_index, GetPersistentAllocator());
	}
}

void GameInstance::RemoveTask(const GTSL::Id64 name, const GTSL::Id64 doneFor)
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

void GameInstance::AddGoal(GTSL::Id64 name)
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