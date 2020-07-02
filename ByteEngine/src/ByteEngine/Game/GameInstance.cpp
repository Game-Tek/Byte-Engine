#include "GameInstance.h"

#include <GTSL/FixedVector.hpp>

#include "ByteEngine/Application/Application.h"

#include "ComponentCollection.h"

#include "System.h"
#include "ByteEngine/Debug/FunctionTimer.h"

GameInstance::GameInstance() : worlds(4, GetPersistentAllocator()), systems(8, GetPersistentAllocator()), componentCollections(64, GetPersistentAllocator()), schedulerSystems(8, GetPersistentAllocator()), goalNames(8, GetPersistentAllocator())
{
}

GameInstance::~GameInstance()
{
	goalNames.Free(GetPersistentAllocator());
	for(auto& e : worlds) { Delete(e, GetPersistentAllocator()); }
	ForEach(systems, [&](const GTSL::Allocation<System>& system) { system->Shutdown(); Delete(system, GetPersistentAllocator()); });
	systems.Free(GetPersistentAllocator());
	ForEach(componentCollections, [&](const GTSL::Allocation<ComponentCollection>& componentCollection) { Delete(componentCollection, GetPersistentAllocator()); });
	componentCollections.Free(GetPersistentAllocator());
	schedulerSystems.Free(GetPersistentAllocator());
	worlds.Free(GetPersistentAllocator());
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
			if (!goal.IsLastStackEmpty()) { goal.AddNewTaskStack(GetPersistentAllocator()); }
			goal.AddTask(function, GetPersistentAllocator());
			goal.AddNewTaskStack(GetPersistentAllocator());
		}
		else
		{
			goal.AddTask(function, GetPersistentAllocator());
		}
	}
}

void GameInstance::AddGoal(const GTSL::Id64 name, const GTSL::Id64 dependsOn)
{
	uint32 i = 0;
	
	for (auto goal_name : goalNames) { if (goal_name == dependsOn) { break; } ++i;  } ++i;
	
	ForEach(schedulerSystems, [&](SchedulerSystem& schedulerSystem) { schedulerSystem.goals.Insert(GetPersistentAllocator(), i, GetPersistentAllocator()); });
	goalNames.Insert(GetPersistentAllocator(), i, name);
}

void GameInstance::AddGoal(GTSL::Id64 name)
{
	goalNames.EmplaceBack(GetPersistentAllocator(), name);
	ForEach(schedulerSystems, [&](SchedulerSystem& schedulerSystem) { schedulerSystem.goals.EmplaceBack(GetPersistentAllocator()); });
}

GameInstance::SchedulerSystem::Goal::Goal(const GTSL::AllocatorReference& allocatorReference) : ParallelTasks(8, allocatorReference)
{
	AddNewTaskStack(allocatorReference);
}

void GameInstance::SchedulerSystem::Goal::AddTask(const GTSL::Delegate<void(const TaskInfo&)> function, const GTSL::AllocatorReference& allocatorReference)
{
	ParallelTasks.back().EmplaceBack(allocatorReference, function);
}

void GameInstance::SchedulerSystem::Goal::AddNewTaskStack(const GTSL::AllocatorReference& allocatorReference)
{
	ParallelTasks.EmplaceBack(allocatorReference, 8, allocatorReference);
}

GameInstance::SchedulerSystem::SchedulerSystem(const GTSL::AllocatorReference& allocatorReference) : goals(8, allocatorReference)
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
	schedulerSystems.Emplace(GetPersistentAllocator(), name, GetPersistentAllocator());
	
	//Add all existing goals to the newly added system since thay also need to take into account all existing goal
	//and since goals are per system they need to be added like this
	auto& sys = schedulerSystems.At(name);
	for (uint32 i = 0; i < goalNames.GetLength(); ++i) { sys.goals.EmplaceBack(GetPersistentAllocator(), GetPersistentAllocator()); }
	
	System::InitializeInfo initialize_info;
	initialize_info.GameInstance = this;
	system->Initialize(initialize_info);
}
