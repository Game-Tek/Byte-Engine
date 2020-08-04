#include "GameInstance.h"

#include "ByteEngine/Game/World.h"
#include "ByteEngine/Game/System.h"
#include "ByteEngine/Game/ComponentCollection.h"

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

using namespace GTSL;

GameInstance::GameInstance() : Object("GameInstance"), worlds(4, GetPersistentAllocator()), systems(8, GetPersistentAllocator()), componentCollections(64, GetPersistentAllocator()),
goals(16, GetPersistentAllocator()), goalNames(8, GetPersistentAllocator()), objectNames(64, GetPersistentAllocator()),
dynamicGoals(32, GetPersistentAllocator()),
dynamicTasksInfo(32, GetTransientAllocator()), task_sorter(64, GetPersistentAllocator())
{
}

GameInstance::~GameInstance()
{
	{
		System::ShutdownInfo shutdown_info;
		shutdown_info.GameInstance = this;
		ForEach(systems, [&](SmartPointer<System, BE::PersistentAllocatorReference>& system) { system->Shutdown(shutdown_info); });
	}
		
	World::DestroyInfo destroy_info;
	destroy_info.GameInstance = this;
	for (auto& world : worlds) { world->DestroyWorld(destroy_info); }
}

void GameInstance::OnUpdate(BE::Application* application)
{
	PROFILE;

	Vector<Goal<TaskType, BE::TAR>, BE::TAR> recurring_goals(64, GetTransientAllocator());
	Vector<Goal<DynamicTaskFunctionType, BE::TAR>, BE::TAR> dynamic_goals(64, GetTransientAllocator());
	Vector<Vector<Semaphore, BE::TAR>, BE::TAR> semaphores(64, GetTransientAllocator());
	
	{
		ReadLock lock(goalsMutex);
		ReadLock lock_2(dynamicTasksMutex);
		
		for (uint32 i = 0; i < goals.GetLength(); ++i)
		{
			recurring_goals.EmplaceBack(goals[i], GetTransientAllocator());
			dynamic_goals.EmplaceBack(dynamicGoals[i], GetTransientAllocator());
			semaphores.EmplaceBack(128, GetTransientAllocator());
		}
	}
	
	TaskInfo task_info;
	task_info.GameInstance = this;

	using task_sorter_t = decltype(task_sorter);
	using on_done_args = Tuple<Array<uint16, 64>, Array<AccessType, 64>, task_sorter_t*>;
	
	auto on_done = [](const Array<uint16, 64>& objects, const Array<AccessType, 64>& accesses, task_sorter_t* taskSorter) -> void
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

		BE::Application::Get()->GetLogger()->PrintBasicLog(BE::Logger::VerbosityLevel::SUCCESS, log);
	};

	const auto on_done_del = Delegate<void(const Array<uint16, 64>&, const Array<AccessType, 64>&, task_sorter_t*)>::Create(on_done);

	const uint32 goal_count = recurring_goals.GetLength();

	for(uint32 goal = 0; goal < goal_count; ++goal)
	{		
		uint16 recurring_goal_number_of_tasks = recurring_goals[goal].GetNumberOfTasks();
		uint16 dynamic_goal_number_of_tasks = dynamic_goals[goal].GetNumberOfTasks();

		uint16 recurring_goal_task = recurring_goal_number_of_tasks;
		uint16 dynamic_goal_task = dynamic_goal_number_of_tasks;
		
		for (auto& semaphore : semaphores[goal]) { semaphore.Wait(); }
		
		while(recurring_goal_number_of_tasks + dynamic_goal_number_of_tasks > 0)
		{
			--recurring_goal_task;
			
			while (recurring_goal_number_of_tasks > 0) //try recurring goals
			{	
				Ranger<const uint16> accessed_objects = recurring_goals[goal].GetTaskAccessedObjects(recurring_goal_task);
				Ranger<const AccessType> access_types = recurring_goals[goal].GetTaskAccessTypes(recurring_goal_task);

				if (task_sorter.CanRunTask(accessed_objects, access_types))
				{
					on_done_args done_args(accessed_objects, access_types, &task_sorter);

					const uint16 target_goal = recurring_goals[goal].GetTaskGoalIndex(recurring_goal_task);
					
					application->GetThreadPool()->EnqueueTask(recurring_goals[goal].GetTask(recurring_goal_task), on_done_del,
						&semaphores[target_goal][semaphores[target_goal].EmplaceBack()], MoveRef(done_args), task_info);

					GTSL::StaticString<1024> log;

					log += "Dispatched recurring task ";
					log += recurring_goal_task;
					log += " of ";
					log += recurring_goal_number_of_tasks;
					log += '\n';
					log += " Goal: ";
					log += goal;
					log += '\n';
					log += "With accesses: \n";
					for (const auto& e : access_types) { log += AccessTypeToString(e); log += ", "; }
					log += '\n';
					log += "Accessed objects: \n";
					for (const auto& e : accessed_objects) { log += e; log += ", "; }

					BE_LOG_WARNING(log);
					
					
					--recurring_goal_number_of_tasks;
					--recurring_goal_task;
				}
				else
				{
					++recurring_goal_task;
					break;
				}
			}
			
			--dynamic_goal_task;
			
			while (dynamic_goal_number_of_tasks > 0) //try dynamic goals
			{
				Ranger<const uint16> accessed_objects = dynamic_goals[goal].GetTaskAccessedObjects(dynamic_goal_task);
				Ranger<const AccessType> access_types = dynamic_goals[goal].GetTaskAccessTypes(dynamic_goal_task);

				if (task_sorter.CanRunTask(accessed_objects, access_types))
				{
					on_done_args done_args(accessed_objects, access_types, &task_sorter);
					const uint16 target_goal = dynamic_goals[goal].GetTaskGoalIndex(dynamic_goal_task);
					const auto semaphore_index = semaphores[target_goal].EmplaceBack();
					
					application->GetThreadPool()->EnqueueTask(dynamic_goals[goal].GetTask(dynamic_goal_task), on_done_del, &semaphores[target_goal][semaphore_index],
						MoveRef(done_args), this, GTSL::ForwardRef<uint16>(dynamic_goal_task));

					GTSL::StaticString<1024> log;
					
					log += "Dispatched dynamic task ";
					log += dynamic_goal_task;
					log += " of ";
					log += dynamic_goal_number_of_tasks;
					log += '\n';
					log += " Goal: ";
					log += goal;
					log += '\n';
					log += "With accesses: \n";
					for (const auto& e : access_types) { log += AccessTypeToString(e); log += ", "; }
					log += '\n';
					log += "Accessed objects: \n";
					for (const auto& e : accessed_objects) { log += e; log += ", "; }

					BE_LOG_WARNING(log);
					
					--dynamic_goal_number_of_tasks;
					--dynamic_goal_task;
				}
				else
				{
					++dynamic_goal_task;
					break;
				}
			}
		}

		{
			WriteLock lock(dynamicTasksMutex);
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

void GameInstance::AddTask(const Id64 name, const Delegate<void(TaskInfo)> function, const Ranger<const TaskDependency> actsOn, const Id64 startsOn, const Id64 doneFor)
{
	Array<uint16, 32> objects; Array<AccessType, 32> accesses;

	uint16 goal_index, target_goal_index;

	{
		ReadLock lock(goalNamesMutex);
		decomposeTaskDescriptor(actsOn, objects, accesses);
		goal_index = getGoalIndex(startsOn);
		target_goal_index = getGoalIndex(doneFor);
	}

	{
		WriteLock lock(goalsMutex);
		goals[goal_index].AddTask(name, function, objects, accesses, target_goal_index, GetPersistentAllocator());
	}
}

void GameInstance::RemoveTask(const Id64 name, const Id64 doneFor)
{
	uint16 i = 0;
	{
		ReadLock lock(goalNamesMutex);
		i = getGoalIndex(name);
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
		dynamicGoals.EmplaceBack(16, GetPersistentAllocator());
	}

	{
		WriteLock lock(goalNamesMutex);

		uint16 i = 0; for (auto goal_name : goalNames) { if (goal_name == name) break; ++i; }
		BE_ASSERT(i == goalNames.GetLength(), "There is already a goal with that name!")
		
		goalNames.EmplaceBack(name);
	}
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