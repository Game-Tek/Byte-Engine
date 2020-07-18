#include "GameInstance.h"

#include <GTSL/FixedVector.hpp>

#include "ByteEngine/Application/Application.h"

#include "ComponentCollection.h"

#include "System.h"
#include "ByteEngine/Debug/Assert.h"
#include "ByteEngine/Debug/FunctionTimer.h"

GameInstance::GameInstance() : worlds(4, GetPersistentAllocator()), systems(8, GetPersistentAllocator()), componentCollections(64, GetPersistentAllocator()),
goals(16, GetPersistentAllocator()), goalNames(8, GetPersistentAllocator())
{
}

GameInstance::~GameInstance()
{
	ForEach(systems, [&](GTSL::SmartPointer<System, BE::PersistentAllocatorReference>& system) { system->Shutdown(); });

	World::DestroyInfo destroy_info;
	destroy_info.GameInstance = this;
	for (auto& world : worlds) { world->DestroyWorld(destroy_info); }
}

void GameInstance::OnUpdate()
{
	PROFILE;

	goalsMutex.ReadLock(); GTSL::Vector<Goal, BE::TransientAllocatorReference> dynamic_goals(goals, GetTransientAllocator()); goalsMutex.ReadUnlock();

	for(auto& e : dynamic_goals) { ::new(&e) Goal(GetPersistentAllocator()); }
	
	dynamicGoals = &dynamic_goals;
	
	GTSL::Vector<GTSL::Semaphore, BE::TransientAllocatorReference> semaphores(256, GetTransientAllocator());

	uint32 task_n = 0;
	
	const TaskInfo task_info;
	
	dynamicGoalsMutex.ReadLock();
	for(auto& goal : dynamic_goals)
	{
		dynamicGoalsMutex.ReadUnlock();

		dynamicGoalsMutex.ReadLock();
		for(const auto& parallel_tasks : goal.GetParallelTasks())
		{
			dynamicGoalsMutex.ReadUnlock();
			
			dynamicGoalsMutex.ReadLock();
			for (const auto& task : parallel_tasks)
			{
				threadPool.EnqueueTask(task, &semaphores[task_n], task_info);
				semaphores.EmplaceBack();
				++task_n;
			}
			dynamicGoalsMutex.ReadUnlock();

			semaphores.Resize(task_n);
			for (auto& e : semaphores) { e.Wait(); }
				
			task_n = 0;
			semaphores.ResizeDown(0);
			
			dynamicGoalsMutex.ReadLock();
		}
		
		dynamicGoalsMutex.ReadUnlock();
		dynamicGoalsMutex.ReadLock();
	}
	dynamicGoalsMutex.ReadUnlock();
	
	dynamicGoals = nullptr;
}

void GameInstance::AddTask(GTSL::Id64 name, GTSL::Delegate<void(const TaskInfo&)> function, GTSL::Ranger<TaskDescriptor> actsOn, const GTSL::Id64 doneFor)
{	
	uint32 i = 0;
	goalNamesMutex.ReadLock();
	for (auto goal_name : goalNames) { if (goal_name == doneFor) break; ++i; }
	BE_ASSERT(i != goalNames.GetLength(), "No goal found with that name!")
	goalNamesMutex.ReadUnlock();
	
	goalsMutex.ReadLock();
	auto& goal = goals[i];

	i = 0;
	
	for(const auto& parallel_task : goal.GetParallelTasks())
	{
		if (canInsert(parallel_task, actsOn))
		{
			goalsMutex.ReadUnlock(); goalsMutex.WriteLock();
			goal[i].AddTask(name, actsOn, function);
			goalsMutex.WriteUnlock();
			return;
		}

		++i;
	}

	goalsMutex.ReadUnlock(); goalsMutex.WriteLock();
	i = goal.AddNewTaskStack(GetPersistentAllocator());
	goal[i].AddTask(name, actsOn, function);
	goalsMutex.WriteUnlock();
}

void GameInstance::RemoveTask(const GTSL::Id64 name, const GTSL::Id64 doneFor)
{
	uint32 i = 0;
	goalNamesMutex.ReadLock();
	for (auto goal_name : goalNames)
	{
		if (goal_name == doneFor) { break; } { ++i;	}
	}
	BE_ASSERT(i != goalNames.GetLength(), "No goal found with that name!")
	goalNamesMutex.ReadUnlock();
	
	goalsMutex.ReadLock(); auto& goal = goals[i]; goalsMutex.ReadUnlock();

	i = 0;
	
	goalsMutex.ReadLock();
	for(auto& parallel_task : goal)
	{
		for (auto task_name : parallel_task.GetTaskNames())
		{
			if (task_name == name)
			{
				goalsMutex.ReadUnlock();

				goalsMutex.WriteLock();
				parallel_task.RemoveTask(i);
				goalsMutex.WriteUnlock();
				return;
			}
		}
	}
	goalsMutex.ReadUnlock();

	BE_ASSERT(false, "No task under specified name!")
}

void GameInstance::AddDynamicTask(GTSL::Id64 name, const GTSL::Delegate<void(const TaskInfo&)>& function, const GTSL::Ranger<TaskDescriptor> actsOn, const GTSL::Id64 doneFor)
{
	uint32 i = 0;
	goalNamesMutex.ReadLock();
	for (auto goal_name : goalNames) { if (goal_name == doneFor) break; ++i; }
	BE_ASSERT(i != goalNames.GetLength(), "No goal found with that name!")
	goalNamesMutex.ReadUnlock();
	
	dynamicGoalsMutex.ReadLock();
	auto& goal = dynamicGoals->At(i);

	i = 0;
	
	for (const auto& parallel_task : goal)
	{
		if (canInsert(parallel_task, actsOn))
		{
			dynamicGoalsMutex.ReadUnlock(); dynamicGoalsMutex.WriteLock();
			goal[i].AddTask(name, actsOn, function);
			dynamicGoalsMutex.WriteUnlock();
			return;
		}

		++i;
	}

	dynamicGoalsMutex.ReadUnlock(); dynamicGoalsMutex.WriteLock();
	i = goal.AddNewTaskStack(GetPersistentAllocator());
	goal[i].AddTask(name, actsOn, function);
	dynamicGoalsMutex.WriteUnlock();	
}

void GameInstance::AddGoal(const GTSL::Id64 name, const GTSL::Id64 dependsOn)
{
	uint32 i = 0;

	goalNamesMutex.ReadLock();
	for (auto goal_name : goalNames) { if (goal_name == dependsOn) { break; } ++i;  } ++i;
	goalNamesMutex.ReadUnlock();

	goalsMutex.WriteLock();
	goals.EmplaceBack(GetPersistentAllocator());
	goalsMutex.WriteUnlock();
	
	goalNamesMutex.WriteLock();
	goalNames.Insert(i, name);
	goalNamesMutex.WriteUnlock();
}

void GameInstance::AddGoal(GTSL::Id64 name)
{
	goalsMutex.WriteLock();
	goals.EmplaceBack(GetPersistentAllocator());
	goalsMutex.WriteUnlock();

	goalNamesMutex.WriteLock();
	goalNames.EmplaceBack(name);
	goalNamesMutex.WriteUnlock();
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

void GameInstance::initSystem(System* system, const GTSL::Id64 name)
{
	System::InitializeInfo initialize_info;
	initialize_info.GameInstance = this;
	system->Initialize(initialize_info);
}
