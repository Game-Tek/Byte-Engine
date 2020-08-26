#pragma once

#include <GTSL/Delegate.hpp>
#include <GTSL/FlatHashMap.h>
#include <GTSL/Id.h>
#include <GTSL/Mutex.h>
#include <GTSL/Vector.hpp>
#include <GTSL/Algorithm.h>
#include <GTSL/Allocator.h>
#include <GTSL/Array.hpp>


#include "Tasks.h"
#include "ByteEngine/Id.h"

#include "ByteEngine/Debug/Assert.h"

class World;
class ComponentCollection;
class System;

namespace BE {
	class Application;
}

class GameInstance : public Object
{
public:
	GameInstance();
	~GameInstance();
	
	void OnUpdate(BE::Application* application);

	using WorldReference = uint8;

	template<typename T>
	T* AddSystem(const Id systemName)
	{
		GTSL::WriteLock lock(systemsMutex);
		
		auto l = systems.EmplaceBack(GTSL::SmartPointer<System, BE::PersistentAllocatorReference>::Create<T>(GetPersistentAllocator()));
		systemsMap.Emplace(systemName, systems[l]);
		objectNames.EmplaceBack(systemName);
		initSystem(systems[l], systemName);
		return static_cast<T*>(systems[l].GetData());
	}
	
	struct CreateNewWorldInfo
	{
	};
	template<typename T>
	WorldReference CreateNewWorld(const CreateNewWorldInfo& createNewWorldInfo)
	{
		auto index = worlds.EmplaceBack(GTSL::SmartPointer<World, BE::PersistentAllocatorReference>::Create<T>(GetPersistentAllocator()));
		initWorld(index); return index;
	}

	void UnloadWorld(WorldReference worldId);
	
	template<class T>
	T* GetSystem(const Id systemName) { return static_cast<T*>(systemsMap.At(systemName)); }

	template<typename... ARGS>
	void AddTask(const Id name, const GTSL::Delegate<void(TaskInfo, ARGS...)>& function, const GTSL::Ranger<const TaskDependency> dependencies, const Id startOn, const Id doneFor, ARGS&&... args)
	{
		auto* taskInfo = GTSL::New<DispatchTaskInfo<TaskInfo, ARGS...>>(GetTransientAllocator(), function, TaskInfo(), GTSL::ForwardRef<ARGS>(args)...);

		auto task = [](GameInstance* gameInstance, const uint32 i, const uint32 taskIndex) -> void
		{
			{
				GTSL::ReadLock lock(gameInstance->recurringTasksMutex);
				DispatchTaskInfo<TaskInfo, ARGS...>* info = static_cast<DispatchTaskInfo<TaskInfo, ARGS...>*>(gameInstance->recurringTasksInfo[i]);
				GTSL::Get<0>(info->Arguments).GameInstance = gameInstance;
				GTSL::Call(info->Delegate, info->Arguments);
				GTSL::Delete(info, gameInstance->GetTransientAllocator());
			}

			gameInstance->taskSorter.ReleaseResources(taskIndex);

			{
				GTSL::WriteLock lock(gameInstance->recurringTasksMutex);
				gameInstance->recurringTasksInfo.Pop(i); //TODO: CHECK WHERE TO POP
			}
		};

		GTSL::Array<uint16, 32> objects; GTSL::Array<AccessType, 32> accesses;

		uint16 start_on_goal_index, task_objective_index;

		{
			GTSL::ReadLock lock(goalNamesMutex);
			decomposeTaskDescriptor(dependencies, objects, accesses);
			start_on_goal_index = getGoalIndex(startOn);
			task_objective_index = getGoalIndex(doneFor);
		}

		{
			GTSL::WriteLock lock(recurringTasksMutex);
			recurringGoals[start_on_goal_index].AddTask(name, GTSL::Delegate<void(GameInstance*, uint32, uint32)>::Create(task), objects, accesses, task_objective_index, GetPersistentAllocator());
			recurringTasksInfo.EmplaceBack(taskInfo);
		}
	}
	
	void RemoveTask(Id name, Id doneFor);

	template<typename... ARGS>
	void AddDynamicTask(const Id name, const GTSL::Delegate<void(TaskInfo, ARGS...)>& function, const GTSL::Ranger<const TaskDependency> dependencies, const Id startOn, const Id doneFor, ARGS&&... args)
	{
		auto* task_info = GTSL::New<DispatchTaskInfo<TaskInfo, ARGS...>>(GetTransientAllocator(), function, TaskInfo(), GTSL::ForwardRef<ARGS>(args)...);
		
		auto task = [](GameInstance* gameInstance, const uint32 i, const uint32 taskIndex) -> void
		{
			{
				GTSL::ReadLock lock(gameInstance->dynamicTasksMutex);
				DispatchTaskInfo<TaskInfo, ARGS...>* info = static_cast<DispatchTaskInfo<TaskInfo, ARGS...>*>(gameInstance->dynamicTasksInfo[i]);
				GTSL::Get<0>(info->Arguments).GameInstance = gameInstance;
				GTSL::Call(info->Delegate, info->Arguments);
				GTSL::Delete(info, gameInstance->GetTransientAllocator());
			}

			gameInstance->taskSorter.ReleaseResources(taskIndex);
			
			{
				GTSL::WriteLock lock(gameInstance->dynamicTasksMutex);
				gameInstance->dynamicTasksInfo.Pop(i); //TODO: CHECK WHERE TO POP
			}
		};

		GTSL::Array<uint16, 32> objects; GTSL::Array<AccessType, 32> accesses;

		uint16 start_on_goal_index, task_objective_index;
		
		{
			GTSL::ReadLock lock(goalNamesMutex);
			decomposeTaskDescriptor(dependencies, objects, accesses);
			start_on_goal_index = getGoalIndex(startOn);
			task_objective_index = getGoalIndex(doneFor);
		}
		
		{
			GTSL::WriteLock lock(dynamicTasksMutex);
			dynamicGoals[start_on_goal_index].AddTask(name, GTSL::Delegate<void(GameInstance*, uint32, uint32)>::Create(task), objects, accesses, task_objective_index, GetPersistentAllocator());
			dynamicTasksInfo.EmplaceBack(task_info);
		}
	}

	template<typename... ARGS>
	void AddDynamicTask(const Id name, const GTSL::Delegate<void(TaskInfo, ARGS...)>& function, const GTSL::Ranger<const TaskDependency> dependencies, ARGS&&... args)
	{
		auto* task_info = GTSL::New<DispatchTaskInfo<TaskInfo, ARGS...>>(GetTransientAllocator(), function, TaskInfo(), GTSL::ForwardRef<ARGS>(args)...);

		auto task = [](GameInstance* gameInstance, const uint32 i, const uint32 taskIndex) -> void
		{
			{
				GTSL::ReadLock lock(gameInstance->dynamicTasksMutex);
				DispatchTaskInfo<TaskInfo, ARGS...>* info = static_cast<DispatchTaskInfo<TaskInfo, ARGS...>*>(gameInstance->dynamicTasksInfo[i]);
				GTSL::Get<0>(info->Arguments).GameInstance = gameInstance;
				GTSL::Call(info->Delegate, info->Arguments);
				GTSL::Delete(info, gameInstance->GetTransientAllocator());
			}

			gameInstance->taskSorter.ReleaseResources(taskIndex);
			
			{
				GTSL::WriteLock lock(gameInstance->dynamicTasksMutex);
				gameInstance->dynamicTasksInfo.Pop(i); //TODO: CHECK WHERE TO POP
			}
		};

		GTSL::Array<uint16, 32> objects; GTSL::Array<AccessType, 32> accesses;

		{
			GTSL::ReadLock lock(goalNamesMutex);
			decomposeTaskDescriptor(dependencies, objects, accesses);
		}

		{
			GTSL::WriteLock lock(dynamicTasksMutex);
			dynamicGoals[0].AddTask(name, GTSL::Delegate<void(GameInstance*, uint32, uint32)>::Create(task), objects, accesses, 1, GetPersistentAllocator());
			dynamicTasksInfo.EmplaceBack(task_info);
		}
	}

	void AddGoal(Id name);
	
private:
	GTSL::ReadWriteMutex systemsMutex;
	GTSL::Vector<GTSL::SmartPointer<World, BE::PersistentAllocatorReference>, BE::PersistentAllocatorReference> worlds;
	GTSL::Vector<GTSL::SmartPointer<System, BE::PersistentAllocatorReference>, BE::PersistentAllocatorReference> systems;
	GTSL::FlatHashMap<System*, BE::PersistentAllocatorReference> systemsMap;

	GTSL::Vector<Id, BE::PersistentAllocatorReference> objectNames;
	
	template<typename... ARGS>
	struct DispatchTaskInfo
	{
		DispatchTaskInfo(const GTSL::Delegate<void(ARGS...)>& delegate, ARGS&&... args) : Delegate(delegate), Arguments(GTSL::ForwardRef<ARGS>(args)...)
		{
		}

		GTSL::Delegate<void(ARGS...)> Delegate;
		GTSL::Tuple<ARGS...> Arguments;
	};
	
	using DispatchFunctionType = GTSL::Delegate<void(GameInstance*, uint32, uint32)>;
	
	GTSL::ReadWriteMutex goalsMutex;
	GTSL::Vector<Goal<DispatchFunctionType, BE::PersistentAllocatorReference>, BE::PersistentAllocatorReference> recurringGoals;

	GTSL::ReadWriteMutex goalNamesMutex;
	GTSL::Vector<Id, BE::PersistentAllocatorReference> goalNames;

	GTSL::ReadWriteMutex recurringTasksMutex;
	GTSL::Vector<void*, BE::TAR> recurringTasksInfo;
	
	GTSL::ReadWriteMutex dynamicTasksMutex;
	GTSL::Vector<void*, BE::TAR> dynamicTasksInfo;
	GTSL::Vector<Goal<DispatchFunctionType, BE::PersistentAllocatorReference>, BE::PersistentAllocatorReference> dynamicGoals;

	TaskSorter<BE::PersistentAllocatorReference> taskSorter;

	uint32 scalingFactor = 16;
	
	void initWorld(uint8 worldId);
	void initSystem(System* system, GTSL::Id64 name);

	uint16 getGoalIndex(const Id name)
	{
		uint16 i = 0; for (auto goal_name : goalNames) { if (goal_name == name) break; ++i; }
		BE_ASSERT(i != goalNames.GetLength(), "No goal found with that name!")
		return i;
	}
	
	template<uint32 N>
	void decomposeTaskDescriptor(GTSL::Ranger<const TaskDependency> taskDependencies, GTSL::Array<uint16, N>& object, GTSL::Array<AccessType, N>& access)
	{
		object.Resize(taskDependencies.ElementCount()); access.Resize(taskDependencies.ElementCount());
		
		for (uint16 i = 0; i < static_cast<uint16>(taskDependencies.ElementCount()); ++i) //for each dependency
		{
			object[i] = 0;
			for (auto object_name : objectNames) { if (object_name == (taskDependencies + i)->AccessedObject) break; ++object[i]; }
			BE_ASSERT(object[i] != objectNames.GetLength(), "No object found with that name!")
			access[i] = (taskDependencies + i)->Access;
		}
	}
};
