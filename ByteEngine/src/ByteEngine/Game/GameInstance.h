#pragma once

#include <GTSL/Delegate.hpp>
#include <GTSL/FlatHashMap.h>
#include <GTSL/Id.h>
#include <GTSL/Mutex.h>
#include <GTSL/Vector.hpp>
#include <GTSL/Algorithm.h>
#include <GTSL/Allocator.h>

#include "Tasks.h"
#include "World.h"

#include "ByteEngine/Debug/Assert.h"

namespace BE {
	class Application;
}

class GameInstance : public Object
{
public:
	GameInstance();
	virtual ~GameInstance();
	
	void OnUpdate(BE::Application* application);

	using WorldReference = uint8;

	template<typename T>
	T* AddSystem(const GTSL::Id64 systemName)
	{
		T* ret = static_cast<T*>(systems.Emplace(systemName, GTSL::SmartPointer<System, BE::PersistentAllocatorReference>::Create<T>(GetPersistentAllocator())).GetData());
		initSystem(ret, systemName); return ret;
	}

	template<typename T>
	T* AddComponentCollection(const GTSL::Id64 componentCollectionName)
	{
		T* pointer = static_cast<T*>(componentCollections.Emplace(componentCollectionName, GTSL::SmartPointer<ComponentCollection, BE::PersistentAllocatorReference>::Create<T>(GetPersistentAllocator())).GetData());
		initCollection(pointer); return pointer;
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

	void UnloadWorld(const WorldReference worldId)
	{
		World::DestroyInfo destroy_info;
		destroy_info.GameInstance = this;
		worlds[worldId]->DestroyWorld(destroy_info);
		worlds.Pop(worldId);
	}

	class ComponentCollection* GetComponentCollection(const GTSL::Id64 collectionName) { return componentCollections.At(collectionName); }
	class System* GetSystem(const GTSL::Id64 systemName) { return systems.At(systemName); }
	
	void AddTask(GTSL::Id64 name, GTSL::Delegate<void(TaskInfo)> function, GTSL::Ranger<const TaskDependency> actsOn, GTSL::Id64 startsOn, GTSL::Id64 doneFor);
	void RemoveTask(GTSL::Id64 name, GTSL::Id64 doneFor);

	template<typename... ARGS>
	void AddDynamicTask(const GTSL::Id64 name, const GTSL::Delegate<void(TaskInfo, ARGS...)>& function, const GTSL::Ranger<const TaskDependency> dependencies,
	                    const GTSL::Id64 startOn, const GTSL::Id64 doneFor, ARGS&&... args)
	{
		auto task_info = GTSL::SmartPointer<DynamicTaskInfo<TaskInfo>, BE::TAR>::Create<DynamicTaskInfo<TaskInfo, ARGS...>>(GetTransientAllocator(), function, TaskInfo(), GTSL::MakeForwardReference<ARGS>(args)...);
		
		auto task = [](GameInstance* gameInstance, const uint32 i) -> void
		{
			GTSL::SmartPointer<DynamicTaskInfo<TaskInfo, ARGS...>, BE::TAR>& info = reinterpret_cast<GTSL::SmartPointer<DynamicTaskInfo<TaskInfo, ARGS...>, BE::TAR>&>(gameInstance->dynamicTasksInfo[i]);
			GTSL::Call<void, TaskInfo, ARGS...>(info->Delegate, info->Arguments);
			info.Free<DynamicTaskInfo<TaskInfo, ARGS...>>();
			gameInstance->dynamicTasksInfo.Pop(i); //TODO: CHECK WHERE TO POP
		};

		GTSL::Array<uint16, 32> objects; GTSL::Array<AccessType, 32> accesses;

		uint16 i = 0;
		
		{
			GTSL::ReadLock lock(goalNamesMutex);
			decomposeTaskDescriptor(dependencies, objects, accesses);
			getGoalIndex(startOn, i);
		}
		
		{
			GTSL::WriteLock lock(newDynamicTasks);
			dynamicGoals[i].AddTask(name, GTSL::Delegate<void(GameInstance*, uint32)>::Create(task), objects, accesses, doneFor, GetPersistentAllocator());
			dynamicTasksInfo.EmplaceBack(task_info);
		}
	}
	
	void AddGoal(GTSL::Id64 name);
	
private:
	GTSL::Vector<GTSL::SmartPointer<World, BE::PersistentAllocatorReference>, BE::PersistentAllocatorReference> worlds;
	GTSL::FlatHashMap<GTSL::SmartPointer<System, BE::PersistentAllocatorReference>, BE::PersistentAllocatorReference> systems;
	GTSL::FlatHashMap<GTSL::SmartPointer<ComponentCollection, BE::PersistentAllocatorReference>, BE::PersistentAllocatorReference> componentCollections;
	
	template<typename... ARGS>
	struct DynamicTaskInfo
	{
		DynamicTaskInfo(const GTSL::Delegate<void(ARGS...)>& delegate, ARGS&&... args) : Delegate(delegate), Arguments(GTSL::MakeForwardReference<ARGS>(args)...)
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
	
	GTSL::ReadWriteMutex newDynamicTasks;
	GTSL::Vector<GTSL::SmartPointer<DynamicTaskInfo<TaskInfo>, BE::TAR>, BE::TAR> dynamicTasksInfo;
	GTSL::Vector<Goal<DynamicTaskFunctionType, BE::PersistentAllocatorReference>, BE::PersistentAllocatorReference> dynamicGoals;

	void popDynamicTask(DynamicTaskFunctionType& dynamicTaskFunction, uint32& i);

	void initWorld(uint8 worldId);
	void initCollection(ComponentCollection* collection);
	void initSystem(System* system, GTSL::Id64 name);

	void getGoalIndex(const GTSL::Id64 name, uint16& goal)
	{
		uint16 i = 0; for (auto goal_name : goalNames) { if (goal_name == name) break; ++i; }
		BE_ASSERT(i != goalNames.GetLength(), "No goal found with that name!")
		goal = i;
	}
	
	template<uint32 N>
	void decomposeTaskDescriptor(GTSL::Ranger<const TaskDependency> taskDependencies, GTSL::Array<uint16, N>& object, GTSL::Array<AccessType, N>& access)
	{
		object.Resize(taskDependencies.ElementCount()); access.Resize(taskDependencies.ElementCount());
		
		for (uint16 i = 0; i < static_cast<uint16>(taskDependencies.ElementCount()); ++i)
		{
			getGoalIndex((taskDependencies + i)->AccessedObject, object[i]);
			access[i] = (taskDependencies + i)->Access;
		}
	}
};
