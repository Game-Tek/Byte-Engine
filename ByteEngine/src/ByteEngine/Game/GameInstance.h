#pragma once

#include <GTSL/Delegate.hpp>
#include <GTSL/FlatHashMap.h>
#include <GTSL/Id.h>
#include <GTSL/Mutex.h>
#include <GTSL/Vector.hpp>
#include <GTSL/Algorithm.h>
#include <GTSL/Allocator.h>

#include "Tasks.h"

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
	T* AddSystem(const GTSL::Id64 systemName)
	{
		T* ret = static_cast<T*>(systems.Emplace(systemName, GTSL::SmartPointer<System, BE::PersistentAllocatorReference>::Create<T>(GetPersistentAllocator())).GetData());
		objectNames.EmplaceBack(systemName);
		initSystem(static_cast<System*>(ret), systemName);
		return ret;
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
	T* GetSystem(const GTSL::Id64 systemName) { return static_cast<T*>(systems.At(systemName).GetData()); }
	
	void AddTask(GTSL::Id64 name, GTSL::Delegate<void(TaskInfo)> function, GTSL::Ranger<const TaskDependency> actsOn, GTSL::Id64 startsOn, GTSL::Id64 doneFor);
	void RemoveTask(GTSL::Id64 name, GTSL::Id64 doneFor);

	template<typename... ARGS>
	void AddDynamicTask(const GTSL::Id64 name, const GTSL::Delegate<void(TaskInfo, ARGS...)>& function, const GTSL::Ranger<const TaskDependency> dependencies,
	                    const GTSL::Id64 startOn, const GTSL::Id64 doneFor, ARGS&&... args)
	{
		void* task_info;
		GTSL::New<DynamicTaskInfo<TaskInfo, ARGS...>>(&task_info, GetTransientAllocator(), function, TaskInfo(), GTSL::ForwardRef<ARGS>(args)...);
		
		auto task = [](GameInstance* gameInstance, const uint32 i) -> void
		{
			{
				GTSL::ReadLock lock(gameInstance->dynamicTasksMutex);
				DynamicTaskInfo<TaskInfo, ARGS...>* info = static_cast<DynamicTaskInfo<TaskInfo, ARGS...>*>(gameInstance->dynamicTasksInfo[i]);
				GTSL::Get<0>(info->Arguments).GameInstance = gameInstance;
				GTSL::Call(info->Delegate, info->Arguments);
				GTSL::Delete<DynamicTaskInfo<TaskInfo, ARGS...>>(gameInstance->dynamicTasksInfo[i], gameInstance->GetTransientAllocator());
			}

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
			dynamicGoals[start_on_goal_index].AddTask(name, GTSL::Delegate<void(GameInstance*, uint32)>::Create(task), objects, accesses, task_objective_index, GetPersistentAllocator());
			dynamicTasksInfo.EmplaceBack(task_info);
		}
	}
	
	void AddGoal(GTSL::Id64 name);
	
private:
	GTSL::Vector<GTSL::SmartPointer<World, BE::PersistentAllocatorReference>, BE::PersistentAllocatorReference> worlds;
	GTSL::FlatHashMap<GTSL::SmartPointer<System, BE::PersistentAllocatorReference>, BE::PersistentAllocatorReference> systems;

	GTSL::Vector<GTSL::Id64, BE::PersistentAllocatorReference> objectNames;
	
	template<typename... ARGS>
	struct DynamicTaskInfo
	{
		DynamicTaskInfo(const GTSL::Delegate<void(ARGS...)>& delegate, ARGS&&... args) : Delegate(delegate), Arguments(GTSL::ForwardRef<ARGS>(args)...)
		{
		}

		GTSL::Delegate<void(ARGS...)> Delegate;
		GTSL::Tuple<ARGS...> Arguments;
	};
	
	using TaskType = GTSL::Delegate<void(TaskInfo)>;
	
	GTSL::ReadWriteMutex goalsMutex;
	GTSL::Vector<Goal<TaskType, BE::PersistentAllocatorReference>, BE::PersistentAllocatorReference> goals;

	GTSL::ReadWriteMutex goalNamesMutex;
	GTSL::Vector<GTSL::Id64, BE::PersistentAllocatorReference> goalNames;

	using DynamicTaskFunctionType = GTSL::Delegate<void(GameInstance*, uint32 i)>;
	
	GTSL::ReadWriteMutex dynamicTasksMutex;
	GTSL::Vector<void*, BE::TAR> dynamicTasksInfo;
	GTSL::Vector<Goal<DynamicTaskFunctionType, BE::PersistentAllocatorReference>, BE::PersistentAllocatorReference> dynamicGoals;

	TaskSorter<BE::PersistentAllocatorReference> task_sorter;

	void initWorld(uint8 worldId);
	void initSystem(System* system, GTSL::Id64 name);

	uint16 getGoalIndex(const GTSL::Id64 name)
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
