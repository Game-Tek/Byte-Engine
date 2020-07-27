#include "GameInstance.h"

#include "ComponentCollection.h"

#include "System.h"
#include "ByteEngine/Debug/Assert.h"
#include "ByteEngine/Debug/FunctionTimer.h"

#include <GTSL/Semaphore.h>

#include "ByteEngine/Application/Application.h"
#include "ByteEngine/Application/ThreadPool.h"

using namespace GTSL;

GameInstance::GameInstance() : worlds(4, GetPersistentAllocator()), systems(8, GetPersistentAllocator()), componentCollections(64, GetPersistentAllocator()),
goals(16, GetPersistentAllocator()), goalNames(8, GetPersistentAllocator())
{
}

GameInstance::~GameInstance()
{
	ForEach(systems, [&](SmartPointer<System, BE::PersistentAllocatorReference>& system) { system->Shutdown(); });

	World::DestroyInfo destroy_info;
	destroy_info.GameInstance = this;
	for (auto& world : worlds) { world->DestroyWorld(destroy_info); }
}

void GameInstance::OnUpdate(BE::Application* application)
{
	PROFILE;
	
	TaskSorter<BE::TAR> task_sorter(32, GetTransientAllocator());
	
	Vector<Goal<TaskType, BE::TAR>, BE::TAR> recurring_goals(32, GetTransientAllocator());
	{
		for(uint32 i = 0; i < recurring_goals.GetLength(); ++i)
		{
			recurring_goals.EmplaceBack(64, GetTransientAllocator());
		}
	}

	Vector<Goal<DynamicTaskFunctionType, BE::TAR>, BE::TAR> dynamic_goals(32, GetTransientAllocator());
	{
		for(uint32 i = 0; i < dynamic_goals.GetLength(); ++i)
		{
			dynamic_goals.EmplaceBack(64, GetTransientAllocator());
		}
	}
	
	Vector<Vector<Semaphore, BE::TAR>, BE::TAR> semaphores(64, GetTransientAllocator());
	{
		for (auto& e : semaphores)
		{
			e.Initialize(128, GetTransientAllocator());
			for (uint8 i = 0; i < 128; ++i) { e.EmplaceBack(); }
		}
	}
	
	TaskInfo task_info; task_info.GameInstance = this;

	uint16 recurring_goal_number_of_tasks, dynamic_goal_number_of_tasks;

	auto on_done = [](const Array<uint16, 64>& objects, const Array<AccessType, 64>& accesses, decltype(task_sorter)* taskSorter) -> void
	{
		taskSorter->ReleaseResources(objects, accesses);
	};

	auto on_done_del = Delegate<void(const Array<uint16, 64>&, const Array<AccessType, 64>&, decltype(task_sorter)*)>::Create(on_done);
	
	for(uint32 goal = 0; goal < recurring_goals.GetLength(); ++goal)
	{
		uint16 task_n = 0;
		
		recurring_goals[goal].GetNumberOfTasks(recurring_goal_number_of_tasks);
		dynamic_goals[goal].GetNumberOfTasks(dynamic_goal_number_of_tasks);
		
		for (auto& semaphore : semaphores[goal]) { semaphore.Wait(); }
		
		while(recurring_goal_number_of_tasks + dynamic_goal_number_of_tasks != 0)
		{
			bool can_run = false;

			//try recurring goals
			while (true)
			{
				Ranger<const uint16> accessed_objects; Ranger<const AccessType> access_types;
				recurring_goals[goal].GetTaskAccessedObjects(recurring_goal_number_of_tasks, accessed_objects); recurring_goals[goal].GetTaskAccessTypes(recurring_goal_number_of_tasks, access_types);

				task_sorter.CanRunTask(can_run, accessed_objects, access_types);

				if (can_run)
				{
					Tuple<Array<uint16, 64>, Array<AccessType, 64>, decltype(task_sorter)*> test_args(accessed_objects, access_types, &task_sorter);

					uint16 target_goal; Id64 goal_name;
					recurring_goals[goal].GetTaskGoal(recurring_goal_number_of_tasks, goal_name);
					getGoalIndex(goal_name, target_goal);
					
					TaskType task; recurring_goals[goal].GetTask(task, recurring_goal_number_of_tasks);
					application->GetThreadPool()->EnqueueTask(task, on_done_del, &semaphores[target_goal][task_n], MakeTransferReference(test_args), task_info);

					--recurring_goal_number_of_tasks;
				}
				else
				{
					break;
				}
			}
			
			//try dynamic goals
			while (true)
			{
				Ranger<const uint16> accessed_objects; Ranger<const AccessType> access_types;
				dynamic_goals[goal].GetTaskAccessedObjects(dynamic_goal_number_of_tasks, accessed_objects); dynamic_goals[goal].GetTaskAccessTypes(dynamic_goal_number_of_tasks, access_types);

				task_sorter.CanRunTask(can_run, accessed_objects, access_types);

				if (can_run)
				{
					Tuple<Array<uint16, 64>, Array<AccessType, 64>, decltype(task_sorter)*> test_args(accessed_objects, access_types, &task_sorter);
					
					uint16 target_goal; Id64 goal_name;
					dynamic_goals[goal].GetTaskGoal(dynamic_goal_number_of_tasks, goal_name);
					getGoalIndex(goal_name, target_goal);
					
					DynamicTaskFunctionType task; dynamic_goals[goal].GetTask(task, dynamic_goal_number_of_tasks);
					application->GetThreadPool()->EnqueueTask(task, on_done_del, &semaphores[target_goal][task_n], MakeTransferReference(test_args), task_info);

					--dynamic_goal_number_of_tasks;
				}
				else
				{
					break;
				}
			}
			
			++task_n;
		}
		
	} //goals

}

void GameInstance::AddTask(const Id64 name, const Delegate<void(TaskInfo)> function, const Ranger<const TaskDependency> actsOn, const Id64 startsOn, const Id64 doneFor)
{
	Array<uint16, 32> objects; Array<AccessType, 32> accesses;

	uint16 goal_index = 0;

	{
		ReadLock lock(goalNamesMutex);
		decomposeTaskDescriptor(actsOn, objects, accesses);
		getGoalIndex(startsOn, goal_index);
	}

	goalsMutex.WriteLock();
	goals[goal_index].AddTask(name, function, objects, accesses, doneFor, GetPersistentAllocator());
	goalsMutex.WriteUnlock();
}

void GameInstance::RemoveTask(const Id64 name, const Id64 doneFor)
{
	uint16 i = 0;
	{
		ReadLock lock(goalNamesMutex);
		getGoalIndex(name, i);
	}
	
	goalsMutex.WriteLock();
	goals[i].RemoveTask(name);
	goalsMutex.WriteUnlock();

	BE_ASSERT(false, "No task under specified name!")
}

void GameInstance::AddGoal(Id64 name)
{
	{
		WriteLock lock(goalsMutex);
		goals.EmplaceBack(16, GetPersistentAllocator());
	}

	{
		WriteLock lock(goalNamesMutex);
		goalNames.EmplaceBack(name);
	}
}

void GameInstance::popDynamicTask(DynamicTaskFunctionType& dynamicTaskFunction, uint32& i)
{
	WriteLock lock(newDynamicTasks);
	i = dynamicGoals.GetLength() - 1;
	dynamicGoals.back().GetTask(dynamicTaskFunction, i);
	dynamicGoals.PopBack();
}

void GameInstance::initWorld(const uint8 worldId)
{
	World::InitializeInfo initialize_info;
	initialize_info.GameInstance = this;
	worlds[worldId]->InitializeWorld(initialize_info);
}

void GameInstance::initCollection(ComponentCollection* collection)
{
	//collection->Initialize();
}

void GameInstance::initSystem(System* system, const Id64 name)
{
	System::InitializeInfo initialize_info;
	initialize_info.GameInstance = this;
	system->Initialize(initialize_info);
}