#pragma once

#include "World.h"
#include "ByteEngine/Application/ThreadPool.h"

#include <GTSL/Delegate.hpp>
#include <GTSL/FlatHashMap.h>
#include <GTSL/Id.h>
#include <GTSL/Mutex.h>
#include <GTSL/Vector.hpp>
#include <GTSL/Algorithm.h>
#include <GTSL/Allocator.h>

#include "Tasks.h"
#include "ByteEngine/Debug/Assert.h"

class GameInstance : public Object
{
public:
	GameInstance();
	virtual ~GameInstance();
	
	virtual void OnUpdate();
	ThreadPool* GetThreadPool() { return &threadPool; }

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
	
	void AddTask(GTSL::Id64 name, GTSL::Delegate<void(TaskInfo)> function, GTSL::Ranger<const TaskDependency> actsOn, GTSL::Id64 doneFor);
	void RemoveTask(GTSL::Id64 name, GTSL::Id64 doneFor);

	template<typename... ARGS>
	void AddDynamicTask(GTSL::Id64 name, const GTSL::Delegate<void(TaskInfo, ARGS...)>& function, GTSL::Ranger<const TaskDependency> actsOn,
	                    const GTSL::Id64 doneFor, GTSL::Id64 dependsOn, ARGS&&... args)
	{
		auto task_info = GTSL::SmartPointer<DynamicTaskInfo<TaskInfo>, BE::TAR>::Create<DynamicTaskInfo<TaskInfo, ARGS...>>(GetTransientAllocator(), function, TaskInfo(), GTSL::MakeForwardReference<ARGS>(args)...);
		
		auto task = [](GameInstance* gameInstance, const uint32 i) -> void
		{
			GTSL::SmartPointer<DynamicTaskInfo<TaskInfo, ARGS...>, BE::TAR>& info = reinterpret_cast<GTSL::SmartPointer<DynamicTaskInfo<TaskInfo, ARGS...>, BE::TAR>&>(gameInstance->dynamicTasksInfo[i]);
			GTSL::Call<void, TaskInfo, ARGS...>(info->Delegate, info->Arguments);
			info.Free<DynamicTaskInfo<TaskInfo, ARGS...>>();
			gameInstance->dynamicTasksInfo.Pop(i);
		};

		{
			GTSL::WriteLock lock(newDynamicTasks);
			dynamicTasks.EmplaceBack(GTSL::Delegate<void(GameInstance*, uint32)>::Create(task));
			dynamicTasksInfo.EmplaceBack(task_info);
		}

		uint32 i = 0;

		{
			GTSL::ReadLock lock(goalNamesMutex);
			
			for (auto goal_name : goalNames) { if (goal_name == doneFor) break; ++i; }
			//BE_ASSERT(i != goalNames.GetLength(), "No goal found with that name!");
		}
		
		//dynamicGoalsMutex.ReadLock();
		//auto& goal = dynamicGoals->At(i);
		//
		//i = 0;
		//
		//for (const auto& parallel_task : goal)
		//{
		//	if (canInsert(parallel_task, actsOn))
		//	{
		//		dynamicGoalsMutex.ReadUnlock(); dynamicGoalsMutex.WriteLock();
		//		goal[i].AddTask(name, actsOn, function);
		//		dynamicGoalsMutex.WriteUnlock();
		//		return;
		//	}
		//
		//	++i;
		//}
		//
		//dynamicGoalsMutex.ReadUnlock(); dynamicGoalsMutex.WriteLock();
		//i = goal.AddNewTaskStack(GetPersistentAllocator());
		//goal[i].AddTask(name, actsOn, function);
		//dynamicGoalsMutex.WriteUnlock();
	}
	
	void AddGoal(GTSL::Id64 name, GTSL::Id64 dependsOn); void AddGoal(GTSL::Id64 name);
	
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
	
	ThreadPool threadPool;
	
	struct ParallelTasks
	{
		explicit ParallelTasks(const BE::PersistentAllocatorReference& allocatorReference) : names(8, allocatorReference), taskDependencies(8, allocatorReference),
		                                                                                     tasks(8, allocatorReference)
		{
		}

		void AddTask(GTSL::Id64 name, const GTSL::Ranger<const TaskDependency> taskDescriptors, TaskType delegate)
		{
			names.EmplaceBack(name); taskDependencies.PushBack(taskDescriptors); tasks.EmplaceBack(delegate);
		}
		
		void RemoveTask(const uint32 i)
		{
			taskDependencies.Pop(i); tasks.Pop(i); names.Pop(i);
		}

		TaskType& operator[](const uint32 i) { return tasks[i]; }

		[[nodiscard]] GTSL::Ranger<TaskType> GetTasks() const { return tasks; }
		[[nodiscard]] GTSL::Ranger<GTSL::Id64> GetTaskNames() const { return names; }
		[[nodiscard]] GTSL::Ranger<TaskDependency> GetTaskDescriptors() const { return taskDependencies; }

		[[nodiscard]] const TaskType* begin() const { return tasks.begin(); }
		[[nodiscard]] const TaskType* end() const { return tasks.end(); }
		
	private:
		GTSL::Vector<GTSL::Id64, BE::PersistentAllocatorReference> names;
		GTSL::Vector<TaskDependency, BE::PersistentAllocatorReference> taskDependencies;
		GTSL::Vector<TaskType, BE::PersistentAllocatorReference> tasks;
	};

	struct Goal
	{
		Goal() = default;
		
		Goal(const BE::PersistentAllocatorReference& allocatorReference) : parallelTasks(16, allocatorReference)
		{
		}
		
		uint32 AddNewTaskStack(const BE::PersistentAllocatorReference& allocatorReference)
		{
			return parallelTasks.EmplaceBack(allocatorReference);
		}

		ParallelTasks& operator[](const uint8 i) { return parallelTasks[i]; }

		[[nodiscard]] GTSL::Ranger<ParallelTasks> GetParallelTasks() const { return parallelTasks; }

		ParallelTasks* begin() { return parallelTasks.begin(); }
		ParallelTasks* end() { return parallelTasks.end(); }

		[[nodiscard]] const ParallelTasks* begin() const { return parallelTasks.begin(); }
		[[nodiscard]] const ParallelTasks* end() const { return parallelTasks.end(); }
		
	private:
		GTSL::Vector<ParallelTasks, BE::PersistentAllocatorReference> parallelTasks;
	};
	
	GTSL::ReadWriteMutex goalsMutex;
	GTSL::Vector<Goal, BE::PersistentAllocatorReference> goals;

	GTSL::ReadWriteMutex goalNamesMutex;
	GTSL::Vector<GTSL::Id64, BE::PersistentAllocatorReference> goalNames;

	using DynamicTaskFunctionType = GTSL::Delegate<void(GameInstance*, uint32 i)>;
	
	GTSL::ReadWriteMutex newDynamicTasks;
	GTSL::Vector<GTSL::SmartPointer<DynamicTaskInfo<TaskInfo>, BE::TAR>, BE::TAR> dynamicTasksInfo;
	GTSL::Vector<DynamicTaskFunctionType, BE::TAR> dynamicTasks;

	void popDynamicTask(DynamicTaskFunctionType& dynamicTaskFunction, uint32& i)
	{
		GTSL::WriteLock lock(newDynamicTasks);
		i = dynamicTasks.GetLength() - 1;
		dynamicTaskFunction = dynamicTasks.back();
		dynamicTasks.PopBack();
	}
	
	void initWorld(uint8 worldId);
	void initCollection(ComponentCollection* collection);
	void initSystem(System* system, GTSL::Id64 name);

	static bool canInsert(const ParallelTasks& parallelTasks, GTSL::Ranger<const TaskDependency> actsOn)
	{
		for (const auto& task_descriptor : parallelTasks.GetTaskDescriptors())
		{
			for (const auto& e : actsOn)
			{
				if (task_descriptor.System == e.System && (task_descriptor.Access == AccessType::READ_WRITE || e.Access == AccessType::READ_WRITE)) { return false; }
			}
		}

		return true;
	};
};
