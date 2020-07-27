#include "GameInstance.h"

#include "ByteEngine/Game/World.h"
#include "ByteEngine/Game/System.h"
#include "ByteEngine/Game/ComponentCollection.h"

#include "ByteEngine/Debug/Assert.h"
#include "ByteEngine/Debug/FunctionTimer.h"

#include "ByteEngine/Application/ThreadPool.h"
#include "ByteEngine/Application/Application.h"

#include <GTSL/Semaphore.h>

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
	
	TaskSorter<BE::TAR> task_sorter(64, GetTransientAllocator());
	
	Vector<Goal<TaskType, BE::TAR>, BE::TAR> recurring_goals(64, GetTransientAllocator());
	Vector<Goal<DynamicTaskFunctionType, BE::TAR>, BE::TAR> dynamic_goals(64, GetTransientAllocator());
	Vector<Vector<Semaphore, BE::TAR>, BE::TAR> semaphores(64, GetTransientAllocator());
	{
		for(uint32 i = 0; i < recurring_goals.GetCapacity(); ++i)
		{
			recurring_goals.EmplaceBack(128, GetTransientAllocator());
			dynamic_goals.EmplaceBack(128, GetTransientAllocator());
			semaphores[i].Initialize(128, GetTransientAllocator());
		}
	}
		
	const uint32 goal_count = recurring_goals.GetLength();
	
	TaskInfo task_info;
	task_info.GameInstance = this;

	uint16 recurring_goal_number_of_tasks, dynamic_goal_number_of_tasks;

	using task_sorter_t = decltype(task_sorter);
	using on_done_args = Tuple<Array<uint16, 64>, Array<AccessType, 64>, task_sorter_t*>;
	
	auto on_done = [](const Array<uint16, 64>& objects, const Array<AccessType, 64>& accesses, task_sorter_t* taskSorter) -> void
	{
		taskSorter->ReleaseResources(objects, accesses);
	};

	auto on_done_del = Delegate<void(const Array<uint16, 64>&, const Array<AccessType, 64>&, task_sorter_t*)>::Create(on_done);
	
	for(uint32 goal = 0; goal < goal_count; ++goal)
	{		
		recurring_goals[goal].GetNumberOfTasks(recurring_goal_number_of_tasks);
		dynamic_goals[goal].GetNumberOfTasks(dynamic_goal_number_of_tasks);
		
		for (auto& semaphore : semaphores[goal]) { semaphore.Wait(); }
		
		while(recurring_goal_number_of_tasks + dynamic_goal_number_of_tasks != 0)
		{
			bool can_run = false;

			//try recurring goals
			while (recurring_goal_number_of_tasks != 0)
			{
				Ranger<const uint16> accessed_objects;
				recurring_goals[goal].GetTaskAccessedObjects(recurring_goal_number_of_tasks, accessed_objects);
				
				Ranger<const AccessType> access_types;
				recurring_goals[goal].GetTaskAccessTypes(recurring_goal_number_of_tasks, access_types);

				task_sorter.CanRunTask(can_run, accessed_objects, access_types);

				if (can_run)
				{
					on_done_args done_args(accessed_objects, access_types, &task_sorter);

					uint16 target_goal;
					recurring_goals[goal].GetTaskGoalIndex(recurring_goal_number_of_tasks, target_goal);

					auto semaphore_index = semaphores[target_goal].EmplaceBack();
					
					TaskType task;
					recurring_goals[goal].GetTask(task, recurring_goal_number_of_tasks);
					application->GetThreadPool()->EnqueueTask(task, on_done_del, &semaphores[target_goal][semaphore_index], MakeTransferReference(done_args), task_info);

					--recurring_goal_number_of_tasks;
				}
				else
				{
					break;
				}
			}
			
			//try dynamic goals
			while (dynamic_goal_number_of_tasks != 0)
			{
				Ranger<const uint16> accessed_objects; Ranger<const AccessType> access_types;
				dynamic_goals[goal].GetTaskAccessedObjects(dynamic_goal_number_of_tasks, accessed_objects); dynamic_goals[goal].GetTaskAccessTypes(dynamic_goal_number_of_tasks, access_types);

				task_sorter.CanRunTask(can_run, accessed_objects, access_types);

				if (can_run)
				{
					on_done_args done_args(accessed_objects, access_types, &task_sorter);
					
					uint16 target_goal;
					dynamic_goals[goal].GetTaskGoalIndex(dynamic_goal_number_of_tasks, target_goal);

					auto semaphore_index = semaphores[target_goal].EmplaceBack();
					
					DynamicTaskFunctionType task;
					dynamic_goals[goal].GetTask(task, dynamic_goal_number_of_tasks);
					application->GetThreadPool()->EnqueueTask(task, on_done_del, &semaphores[target_goal][semaphore_index], MakeTransferReference(done_args), this, dynamic_goal_number_of_tasks);

					--dynamic_goal_number_of_tasks;
				}
				else
				{
					break;
				}
			}
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

void GameInstance::AddTask(const Id64 name, const Delegate<void(TaskInfo)> function, const Ranger<const TaskDependency> actsOn, const Id64 startsOn, const Id64 doneFor)
{
	Array<uint16, 32> objects; Array<AccessType, 32> accesses;

	uint16 goal_index = 0, target_goal_index = 0;

	{
		ReadLock lock(goalNamesMutex);
		decomposeTaskDescriptor(actsOn, objects, accesses);
		getGoalIndex(startsOn, goal_index);
		getGoalIndex(doneFor, target_goal_index);
	}

	{
		WriteLock lock(goalsMutex);
		goals[goal_index].AddTask(name, function, objects, accesses, doneFor, target_goal_index, GetPersistentAllocator());
	}
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