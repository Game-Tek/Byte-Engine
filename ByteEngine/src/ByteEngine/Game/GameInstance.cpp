#include "GameInstance.h"

#include <GTSL/FixedVector.hpp>

#include "ByteEngine/Application/Application.h"

#include "System.h"
#include "ByteEngine/Debug/FunctionTimer.h"

static BE::PersistentAllocatorReference persistent_allocator("Game Instance");

GameInstance::GameInstance() : worlds(4, &persistent_allocator), systems(8, GetPersistentAllocator()), componentCollections(64, GetPersistentAllocator()), schedulerSystems(8, GetPersistentAllocator()), goalNames(8, &persistent_allocator)
{
}

GameInstance::~GameInstance()
{
	for(auto& e : worlds) { GTSL::Delete(e, GetPersistentAllocator()); }
	GTSL::ForEach(systems, [&](const GTSL::Allocation<System>& system) { system->Shutdown(); Delete(system, GetPersistentAllocator()); });
	systems.Free(GetPersistentAllocator());
	GTSL::ForEach(componentCollections, [&](const GTSL::Allocation<ComponentCollection>& componentCollection) { Delete(componentCollection, GetPersistentAllocator()); });
	componentCollections.Free(GetPersistentAllocator());
	schedulerSystems.Free(GetPersistentAllocator());
}

void GameInstance::OnUpdate()
{
	PROFILE;

	TaskInfo task_info;
	
	ForEach(schedulerSystems, [&](const SchedulerSystem& schedulerSystem)
	{
		for(const auto& goal : schedulerSystem.goals)
		{
			for(const auto& parallel_tasks : goal.ParallelTasks)
			{
				for (auto task : parallel_tasks) { task(task_info); }
			}
		}
	});
}

void GameInstance::AddTask(const GTSL::Id64 name, const AccessType accessType, const GTSL::Delegate<void(const TaskInfo&)> function, const GTSL::Ranger<GTSL::Id64> actsOn, const GTSL::Id64 doneFor)
{
	uint32 i = 0;
	for (auto goal_name : goalNames) { if (goal_name == doneFor) break; ++i; }
	
	for(auto system : actsOn)
	{
		auto& scheduler_system = schedulerSystems.At(system); auto& goal = scheduler_system.goals[i];
		
		if (accessType == AccessType::READ_WRITE)
		{
			if (goal.ParallelTasks[goal.ParallelTasks.GetLength() - 1].GetLength() != 0) { goal.AddNewTaskStack(); }
			goal.AddTask(function);
			goal.AddNewTaskStack();
		}
		else
		{
			goal.AddTask(function);
		}
		
	}
}

void GameInstance::AddGoal(const GTSL::Id64 name, const GTSL::Id64 dependsOn)
{
	uint32 i = 0;
	
	for(auto goal_name : goalNames) { if(goal_name == dependsOn) { break; } } ++i;
	
	ForEach(schedulerSystems, [&](SchedulerSystem& schedulerSystem) { schedulerSystem.goals.Insert(i, SchedulerSystem::Goal()); });
	goalNames.Insert(i, name);
}

void GameInstance::AddGoal(GTSL::Id64 name)
{
	goalNames.EmplaceBack(name);
	ForEach(schedulerSystems, [&](SchedulerSystem& schedulerSystem) { schedulerSystem.goals.EmplaceBack(); });
}

GameInstance::SchedulerSystem::Goal::Goal() : ParallelTasks(8, &persistent_allocator)
{
	AddNewTaskStack();
}

void GameInstance::SchedulerSystem::Goal::AddTask(const GTSL::Delegate<void(const TaskInfo&)> function)
{
	ParallelTasks[ParallelTasks.GetLength() - 1].EmplaceBack(function);
}

void GameInstance::SchedulerSystem::Goal::AddNewTaskStack()
{
	ParallelTasks.EmplaceBack(8, &persistent_allocator);
}

GameInstance::SchedulerSystem::SchedulerSystem() : goals(8, &persistent_allocator)
{
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
	//Add all existing goals to the newly added system since thay also need to take into account all existing goal
	//and since goals are per system thay need to be added like this
	auto& sys = schedulerSystems.At(name);
	for (uint32 i = 0; i < goalNames.GetLength(); ++i) { sys.goals.EmplaceBack(); }
	
	System::InitializeInfo initialize_info;
	initialize_info.GameInstance = this;
	system->Initialize(initialize_info);
}
