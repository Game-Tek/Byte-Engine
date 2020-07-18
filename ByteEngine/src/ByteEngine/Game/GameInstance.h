#pragma once

#include "World.h"
#include "ByteEngine/Application/ThreadPool.h"

#include <GTSL/Delegate.hpp>
#include <GTSL/FlatHashMap.h>
#include <GTSL/Id.h>
#include <GTSL/Mutex.h>
#include <GTSL/Vector.hpp>

class GameInstance : public Object
{
public:
	GameInstance();
	virtual ~GameInstance();
	
	virtual void OnUpdate();
	
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

	template<typename T>
	void UnloadWorld(const WorldReference worldId)
	{
		World::DestroyInfo destroy_info;
		destroy_info.GameInstance = this;
		worlds[worldId]->DestroyWorld(destroy_info);
		worlds.Pop(worldId);
	}

	class ComponentCollection* GetComponentCollection(const GTSL::Id64 collectionName) { return componentCollections.At(collectionName); }
	class System* GetSystem(const GTSL::Id64 systemName) { return systems.At(systemName); }

	struct TaskInfo
	{
	};
	
	enum class AccessType : uint8 { READ, READ_WRITE };

	struct TaskDescriptor { GTSL::Id64 System; AccessType Access; };
	
	void AddTask(GTSL::Id64 name, GTSL::Delegate<void(const TaskInfo&)> function, GTSL::Ranger<TaskDescriptor> actsOn, GTSL::Id64 doneFor);
	void RemoveTask(GTSL::Id64 name, GTSL::Id64 doneFor);
	void AddDynamicTask(GTSL::Id64 name, const GTSL::Delegate<void(const TaskInfo&)>& function, GTSL::Ranger<TaskDescriptor> actsOn, GTSL::Id64 doneFor);
	void AddGoal(GTSL::Id64 name, GTSL::Id64 dependsOn); void AddGoal(GTSL::Id64 name);
private:
	GTSL::Vector<GTSL::SmartPointer<World, BE::PersistentAllocatorReference>, BE::PersistentAllocatorReference> worlds;
	GTSL::FlatHashMap<GTSL::SmartPointer<System, BE::PersistentAllocatorReference>, BE::PersistentAllocatorReference> systems;
	GTSL::FlatHashMap<GTSL::SmartPointer<ComponentCollection, BE::PersistentAllocatorReference>, BE::PersistentAllocatorReference> componentCollections;

	using TaskType = GTSL::Delegate<void(const TaskInfo&)>;
	
	ThreadPool threadPool;
	
	struct ParallelTasks
	{
		ParallelTasks(const BE::PersistentAllocatorReference& allocatorReference) : descriptors(8, allocatorReference), tasks(8, allocatorReference), names(8, allocatorReference)
		{
		}

		void AddTask(GTSL::Id64 name, GTSL::Ranger<TaskDescriptor> taskDescriptors, GTSL::Delegate<void(const TaskInfo&)> delegate)
		{
			names.EmplaceBack(name); descriptors.PushBack(taskDescriptors); tasks.EmplaceBack(delegate);
		}
		
		void RemoveTask(const uint32 i)
		{
			descriptors.Pop(i); tasks.Pop(i); names.Pop(i);
		}

		TaskType& operator[](const uint32 i) { return tasks[i]; }

		[[nodiscard]] GTSL::Ranger<TaskType> GetTasks() const { return tasks; }
		[[nodiscard]] GTSL::Ranger<GTSL::Id64> GetTaskNames() const { return names; }
		[[nodiscard]] GTSL::Ranger<TaskDescriptor> GetTaskDescriptors() const { return descriptors; }

		[[nodiscard]] const TaskType* begin() const { return tasks.begin(); }
		[[nodiscard]] const TaskType* end() const { return tasks.end(); }
		
	private:
		GTSL::Vector<GTSL::Id64, BE::PersistentAllocatorReference> names;
		GTSL::Vector<TaskDescriptor, BE::PersistentAllocatorReference> descriptors;
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

	GTSL::ReadWriteMutex dynamicGoalsMutex;
	GTSL::Vector<Goal, BE::TransientAllocatorReference>* dynamicGoals = nullptr;
	
	void initWorld(uint8 worldId);
	void initCollection(ComponentCollection* collection);
	void initSystem(System* system, GTSL::Id64 name);

	static bool canInsert(const ParallelTasks& parallelTasks, GTSL::Ranger<TaskDescriptor> actsOn)
	{
		for (const auto& task_descriptor : parallelTasks.GetTaskDescriptors())
		{
			for (auto& e : actsOn)
			{
				if (task_descriptor.System == e.System && (task_descriptor.Access == AccessType::READ_WRITE || e.Access == AccessType::READ_WRITE)) { return false; }
			}
		}

		return true;
	};
};
