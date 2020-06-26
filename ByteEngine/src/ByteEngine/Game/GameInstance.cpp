#include "GameInstance.h"

#include <GTSL/FixedVector.hpp>


#include "ByteEngine/Application/Application.h"

#include "System.h"
#include "ByteEngine/Debug/FunctionTimer.h"

static BE::PersistentAllocatorReference persistent_allocator("Game Instance");

GameInstance::GameInstance() : worlds(4, &persistent_allocator), systems(8, GetPersistentAllocator()), componentCollections(64, GetPersistentAllocator()), schedulerSystems(8, GetPersistentAllocator())
{
}

GameInstance::~GameInstance()
{
	for(auto& e : worlds) { GTSL::Delete(e, GetPersistentAllocator()); }
	GTSL::ForEach(systems, [&](const GTSL::Allocation<System>& system) { GTSL::Delete(system, GetPersistentAllocator()); });
	systems.Free(GetPersistentAllocator());
	GTSL::ForEach(componentCollections, [&](const GTSL::Allocation<ComponentCollection>& componentCollection) { GTSL::Delete(componentCollection, GetPersistentAllocator()); });
	componentCollections.Free(GetPersistentAllocator());
	schedulerSystems.Free(GetPersistentAllocator());
}

void GameInstance::OnUpdate()
{
	PROFILE;
	
	GTSL::Array<World*, 64> world_pointers(worlds.GetLength());
	for(auto& e : worlds) { world_pointers[&e - worlds.begin()] = e.Data; }

	ForEach(systems, [&](GTSL::Allocation<System>& system) { system->Process(world_pointers); });
	//GTSL::ForEach(componentCollections, [&](ComponentCollection*& collection) { collection->Process(worlds_range); });

	TaskInfo task_info;
	
	GTSL::ForEach(schedulerSystems, [&](const SchedulerSystem& schedulerSystem)
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
		auto& scheduler_system = schedulerSystems[system]; auto& goal = scheduler_system.goals[i];

		if (scheduler_system.nextNeedsNewStack) { goal.AddNewTaskStack(); scheduler_system.nextNeedsNewStack = accessType == AccessType::READ_WRITE; }
		
		goal.AddTask(function);
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
	ForEach(schedulerSystems, [&](SchedulerSystem& schedulerSystem) { schedulerSystem.goals.EmplaceBack(); });
	goalNames.EmplaceBack(name);
}

GameInstance::SchedulerSystem::Goal::Goal() : ParallelTasks(8, &persistent_allocator)
{
	for (auto& e : ParallelTasks) { e.Initialize(8, &persistent_allocator); }
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
