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
	using FunctionType = GTSL::Delegate<void(GameInstance*, uint32, uint32, uint32)>;
public:
	GameInstance();
	~GameInstance();
	
	void OnUpdate(BE::Application* application);

	using WorldReference = uint8;
	
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
		if constexpr (_DEBUG) { if (assertTask(name, startOn, doneFor, dependencies)) { return; } }
		
		auto taskInfo = GTSL::SmartPointer<void*, BE::PersistentAllocatorReference>::Create<DispatchTaskInfo<TaskInfo, ARGS...>>(GetPersistentAllocator(), function, TaskInfo(), GTSL::ForwardRef<ARGS>(args)...);

		auto task = [](GameInstance* gameInstance, const uint32 goal, const uint32 goalTaskIndex, const uint32 dynamicTaskIndex) -> void
		{			
			{
				GTSL::ReadLock lock(gameInstance->recurringTasksInfoMutex);
				DispatchTaskInfo<TaskInfo, ARGS...>* info = reinterpret_cast<DispatchTaskInfo<TaskInfo, ARGS...>*>(gameInstance->recurringTasksInfo[goal][goalTaskIndex].GetData());
				GTSL::Get<0>(info->Arguments).GameInstance = gameInstance;
				GTSL::Call(info->Delegate, info->Arguments);
			}

			gameInstance->taskSorter.ReleaseResources(dynamicTaskIndex);
		};

		GTSL::Array<uint16, 32> objects; GTSL::Array<AccessType, 32> accesses;

		uint16 startOnGoalIndex, taskObjectiveIndex;

		{
			GTSL::ReadLock lock(goalNamesMutex);
			decomposeTaskDescriptor(dependencies, objects, accesses);
			startOnGoalIndex = getGoalIndex(startOn);
			taskObjectiveIndex = getGoalIndex(doneFor);
		}

		{
			GTSL::WriteLock lock(recurringTasksInfoMutex);
			GTSL::WriteLock lock2(recurringGoalsMutex);
			recurringGoals[startOnGoalIndex].AddTask(name, FunctionType::Create(task), objects, accesses, taskObjectiveIndex, GetPersistentAllocator());
			recurringTasksInfo[startOnGoalIndex].EmplaceBack(GTSL::MoveRef(taskInfo));
		}
	}
	
	void RemoveTask(Id name, Id startOn);

	template<typename... ARGS>
	void AddDynamicTask(const Id name, const GTSL::Delegate<void(TaskInfo, ARGS...)>& function, const GTSL::Ranger<const TaskDependency> dependencies, const Id startOn, const Id doneFor, ARGS&&... args)
	{
		auto* taskInfo = GTSL::New<DispatchTaskInfo<TaskInfo, ARGS...>>(GetTransientAllocator(), function, TaskInfo(), GTSL::ForwardRef<ARGS>(args)...);
		
		auto task = [](GameInstance* gameInstance, const uint32 goal, const uint32 goalTaskIndex, const uint32 dynamicTaskIndex) -> void
		{
			{				
				GTSL::ReadLock lock(gameInstance->dynamicTasksInfoMutex);
				DispatchTaskInfo<TaskInfo, ARGS...>* info = static_cast<DispatchTaskInfo<TaskInfo, ARGS...>*>(gameInstance->dynamicTasksInfo[goal][goalTaskIndex]);
				GTSL::Get<0>(info->Arguments).GameInstance = gameInstance;
				GTSL::Call(info->Delegate, info->Arguments);
				GTSL::Delete<DispatchTaskInfo<TaskInfo, ARGS...>>(info, gameInstance->GetTransientAllocator());
			}

			gameInstance->taskSorter.ReleaseResources(dynamicTaskIndex);
		};

		GTSL::Array<uint16, 32> objects; GTSL::Array<AccessType, 32> accesses;

		uint16 startOnGoalIndex, taskObjectiveIndex;
		
		{
			GTSL::ReadLock lock(goalNamesMutex);
			decomposeTaskDescriptor(dependencies, objects, accesses);
			startOnGoalIndex = getGoalIndex(startOn);
			taskObjectiveIndex = getGoalIndex(doneFor);
		}
		
		{
			GTSL::WriteLock lock(dynamicTasksInfoMutex);
			GTSL::WriteLock lock2(dynamicGoalsMutex);
			dynamicGoals[startOnGoalIndex].AddTask(name, FunctionType::Create(task), objects, accesses, taskObjectiveIndex, GetPersistentAllocator());
			dynamicTasksInfo[startOnGoalIndex].EmplaceBack(taskInfo);
		}
	}

	template<typename... ARGS>
	void AddDynamicTask(const Id name, const GTSL::Delegate<void(TaskInfo, ARGS...)>& function, const GTSL::Ranger<const TaskDependency> dependencies, ARGS&&... args)
	{
		auto* taskInfo = GTSL::New<DispatchTaskInfo<TaskInfo, ARGS...>>(GetTransientAllocator(), function, TaskInfo(), GTSL::ForwardRef<ARGS>(args)...);

		auto task = [](GameInstance* gameInstance, const uint32 goal, const uint32 goalTaskIndex, const uint32 dynamicTaskIndex) -> void
		{
			{
				GTSL::ReadLock lock(gameInstance->dynamicTasksInfoMutex);
				DispatchTaskInfo<TaskInfo, ARGS...>* info = static_cast<DispatchTaskInfo<TaskInfo, ARGS...>*>(gameInstance->dynamicTasksInfo[goal][goalTaskIndex]);
				GTSL::Get<0>(info->Arguments).GameInstance = gameInstance;
				GTSL::Call(info->Delegate, info->Arguments);
				GTSL::Delete<DispatchTaskInfo<TaskInfo, ARGS...>>(info, gameInstance->GetTransientAllocator());
			}

			gameInstance->taskSorter.ReleaseResources(dynamicTaskIndex);
		};

		GTSL::Array<uint16, 32> objects; GTSL::Array<AccessType, 32> accesses;

		{
			GTSL::ReadLock lock(goalNamesMutex);
			decomposeTaskDescriptor(dependencies, objects, accesses);
		}
		
		{
			GTSL::WriteLock lock(dynamicTasksInfoMutex);
			GTSL::WriteLock lock2(dynamicGoalsMutex);
			dynamicGoals[0].AddTask(name, FunctionType::Create(task), objects, accesses, 1, GetPersistentAllocator());
			dynamicTasksInfo[0].EmplaceBack(taskInfo);
		}
	}

	void AddGoal(Id name);
	
private:
	mutable GTSL::ReadWriteMutex systemsMutex;
	GTSL::Vector<GTSL::SmartPointer<World, BE::PersistentAllocatorReference>, BE::PersistentAllocatorReference> worlds;
	GTSL::KeepVector<GTSL::SmartPointer<System, BE::PersistentAllocatorReference>, BE::PersistentAllocatorReference> systems;
	GTSL::FlatHashMap<System*, BE::PersistentAllocatorReference> systemsMap;

	GTSL::FlatHashMap<uint32, BE::PersistentAllocatorReference> systemsIndirectionTable;
	
	template<typename... ARGS>
	struct DispatchTaskInfo
	{
		DispatchTaskInfo(const GTSL::Delegate<void(ARGS...)>& delegate, ARGS&&... args) : Delegate(delegate), Arguments(GTSL::ForwardRef<ARGS>(args)...)
		{
		}

		GTSL::Delegate<void(ARGS...)> Delegate;
		GTSL::Tuple<ARGS...> Arguments;
	};
	
	mutable GTSL::ReadWriteMutex recurringGoalsMutex;
	GTSL::Vector<Goal<FunctionType, BE::PersistentAllocatorReference>, BE::PersistentAllocatorReference> recurringGoals;
	mutable GTSL::ReadWriteMutex dynamicGoalsMutex;
	GTSL::Vector<Goal<FunctionType, BE::PersistentAllocatorReference>, BE::PersistentAllocatorReference> dynamicGoals;

	mutable GTSL::ReadWriteMutex goalNamesMutex;
	GTSL::Vector<Id, BE::PersistentAllocatorReference> goalNames;

	mutable GTSL::ReadWriteMutex recurringTasksInfoMutex;
	GTSL::Vector<GTSL::Vector<GTSL::SmartPointer<void*, BE::PersistentAllocatorReference>, BE::PersistentAllocatorReference>, BE::PersistentAllocatorReference> recurringTasksInfo;
	
	mutable GTSL::ReadWriteMutex dynamicTasksInfoMutex;
	GTSL::Vector<GTSL::Vector<void*, BE::PersistentAllocatorReference>, BE::PersistentAllocatorReference> dynamicTasksInfo;

	TaskSorter<BE::PersistentAllocatorReference> taskSorter;

	uint32 scalingFactor = 16;
	
	void initWorld(uint8 worldId);
	void initSystem(System* system, GTSL::Id64 name);

	uint16 getGoalIndex(const Id name) const
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
			object[i] = systemsIndirectionTable.At(taskDependencies[i].AccessedObject);
			access[i] = (taskDependencies + i)->Access;
		}
	}

	[[nodiscard]] bool assertTask(const Id name, const Id startGoal, const Id endGoal, const GTSL::Ranger<const TaskDependency> dependencies) const
	{
		{
			GTSL::ReadLock lock(goalNamesMutex);
			
			if (goalNames.Find(startGoal) == goalNames.end())
			{
				BE_LOG_WARNING("Tried to add task ", name.GetString(), " to goal ", startGoal.GetString(), " which doesn't exist. Resolve this issue as it leads to undefined behavior in release builds!")
				return true;
			}

			//assert done for exists
			if (goalNames.Find(endGoal) == goalNames.end())
			{
				BE_LOG_WARNING("Tried to add task ", name.GetString(), " ending for goal ", endGoal.GetString(), " which doesn't exist. Resolve this issue as it leads to undefined behavior in release builds!")
				return true;
			}
		}

		{
			GTSL::ReadLock lock(recurringGoalsMutex);
			
			if (recurringGoals[getGoalIndex(startGoal)].DoesTaskExist(name))
			{
				BE_LOG_WARNING("Tried to add task ", name.GetString(), " which already exists to goal ", startGoal.GetString(), ". Resolve this issue as it leads to undefined behavior in release builds!")
				return true;
			}
		}

		{
			GTSL::ReadLock lock(systemsMutex);

			for(auto e : dependencies)
			{
				if (!systemsMap.Find(e.AccessedObject)) {
					BE_LOG_WARNING("Tried to add task ", name.GetString(), " to goal ", startGoal.GetString(), " with a dependency on ", e.AccessedObject.GetString(), " which doesn't exist. Resolve this issue as it leads to undefined behavior in release builds!")
					return true;
				}
			}
		}

		return false;
	}

public:
	template<typename T>
	T* AddSystem(const Id systemName)
	{
		System* system;

		{
			GTSL::WriteLock lock(systemsMutex);
			auto l = systems.Emplace(GTSL::SmartPointer<System, BE::PersistentAllocatorReference>::Create<T>(GetPersistentAllocator()));
			systemsMap.Emplace(systemName, systems[l]);
			systemsIndirectionTable.Emplace(systemName, l);
			system = systems[l];
		}

		initSystem(system, systemName);

		taskSorter.AddSystem();
		
		return static_cast<T*>(system);
	}
};
