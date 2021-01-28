#pragma once

#include <GTSL/Delegate.hpp>
#include <GTSL/FlatHashMap.h>
#include <GTSL/Id.h>
#include <GTSL/Mutex.h>
#include <GTSL/Vector.hpp>
#include <GTSL/Algorithm.h>
#include <GTSL/Allocator.h>
#include <GTSL/Array.hpp>
#include <GTSL/Pair.h>
#include <GTSL/Semaphore.h>

#include "Tasks.h"
#include "ByteEngine/Id.h"

#include "ByteEngine/Debug/Assert.h"

#include "ByteEngine/Handle.hpp"

class World;
class ComponentCollection;
class System;

namespace BE {
	class Application;
}

template<typename... ARGS>
using Task = GTSL::Delegate<void(TaskInfo, ARGS...)>;

inline const char* AccessTypeToString(const AccessType access)
{
	switch (access)
	{
	case AccessType::READ: return "READ";
	case AccessType::READ_WRITE: return "READ_WRITE";
	}
}

template<typename... ARGS>
struct DynamicTaskHandle
{
	DynamicTaskHandle(uint32 reference) : Reference(reference) {}
	
	uint32 Reference;
};

class GameInstance : public Object
{
	using FunctionType = GTSL::Delegate<void(GameInstance*, uint32, uint32, void*)>;
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
	T* GetSystem(const Id systemName)
	{
		GTSL::ReadLock lock(systemsMutex);
		return static_cast<T*>(systemsMap.At(systemName()));
	}
	
	template<class T>
	T* GetSystem(const uint16 systemReference)
	{
		GTSL::ReadLock lock(systemsMutex);
		return static_cast<T*>(systems[systemReference].GetData());
	}
	
	uint16 GetSystemReference(const Id systemName)
	{
		GTSL::ReadLock lock(systemsMutex);
		return static_cast<uint16>(systemsIndirectionTable.At(systemName()));
	}

	template<typename... ARGS>
	void AddTask(const Id name, const GTSL::Delegate<void(TaskInfo, ARGS...)>& function, const GTSL::Range<const TaskDependency*> dependencies, const Id startOn, const Id doneFor, ARGS&&... args)
	{
		if constexpr (_DEBUG) { if (assertTask(name, startOn, doneFor, dependencies)) { return; } }
		
		auto taskInfo = GTSL::SmartPointer<void*, BE::PersistentAllocatorReference>::Create<DispatchTaskInfo<TaskInfo, ARGS...>>(GetPersistentAllocator(), function, TaskInfo(), GTSL::ForwardRef<ARGS>(args)...);

		auto task = [](GameInstance* gameInstance, const uint32 goal, const uint32 dynamicTaskIndex, void* data) -> void
		{			
			{
				DispatchTaskInfo<TaskInfo, ARGS...>* info = static_cast<DispatchTaskInfo<TaskInfo, ARGS...>*>(data);
				
				GTSL::Get<0>(info->Arguments).GameInstance = gameInstance;
				GTSL::Call(info->Delegate, GTSL::MoveRef(info->Arguments));
			}

			gameInstance->resourcesUpdated.NotifyAll();
			gameInstance->semaphores[goal].Post();
			gameInstance->taskSorter.ReleaseResources(dynamicTaskIndex);
		};

		GTSL::Array<uint16, 32> objects; GTSL::Array<AccessType, 32> accesses;

		uint16 startOnGoalIndex, taskObjectiveIndex;

		{
			GTSL::ReadLock lock(stagesNamesMutex);
			decomposeTaskDescriptor(dependencies, objects, accesses);
			startOnGoalIndex = getStageIndex(startOn);
			taskObjectiveIndex = getStageIndex(doneFor);
		}

		{
			GTSL::WriteLock lock(recurringTasksInfoMutex);
			GTSL::WriteLock lock2(recurringTasksMutex);
			recurringTasksPerStage[startOnGoalIndex].AddTask(name, FunctionType::Create(task), objects, accesses, taskObjectiveIndex, static_cast<void*>(taskInfo.GetData()), GetPersistentAllocator());
			recurringTasksInfo[startOnGoalIndex].EmplaceBack(GTSL::MoveRef(taskInfo));
		}

		BE_LOG_MESSAGE("Added recurring task ", name.GetString(), " to goal ", startOn.GetString(), " to be done before ", doneFor.GetString())
	}
	
	void RemoveTask(Id name, Id startOn);

	template<typename... ARGS>
	void AddDynamicTask(const Id name, const GTSL::Delegate<void(TaskInfo, ARGS...)>& function, const GTSL::Range<const TaskDependency*> dependencies, const Id startOn, const Id doneFor, ARGS&&... args)
	{
		auto* taskInfo = GTSL::New<DispatchTaskInfo<TaskInfo, ARGS...>>(GetPersistentAllocator(), function, TaskInfo(), GTSL::ForwardRef<ARGS>(args)...);

		GTSL::Array<uint16, 32> objects; GTSL::Array<AccessType, 32> accesses;

		uint16 startOnGoalIndex, taskObjectiveIndex;

		auto task = [](GameInstance* gameInstance, const uint32 goal, const uint32 dynamicTaskIndex, void* data) -> void
		{
			{
				DispatchTaskInfo<TaskInfo, ARGS...>* info = static_cast<DispatchTaskInfo<TaskInfo, ARGS...>*>(data);

				GTSL::Get<0>(info->Arguments).GameInstance = gameInstance;
				GTSL::Call(info->Delegate, GTSL::MoveRef(info->Arguments));

				gameInstance->resourcesUpdated.NotifyAll();
				gameInstance->semaphores[goal].Post();
				GTSL::Delete<DispatchTaskInfo<TaskInfo, ARGS...>>(info, gameInstance->GetPersistentAllocator());
			}

			gameInstance->taskSorter.ReleaseResources(dynamicTaskIndex);
		};

		{
			GTSL::ReadLock lock(stagesNamesMutex);
			decomposeTaskDescriptor(dependencies, objects, accesses);
			startOnGoalIndex = getStageIndex(startOn);
			taskObjectiveIndex = getStageIndex(doneFor);
		}
		
		{
			GTSL::WriteLock lock2(dynamicTasksPerStageMutex);
			dynamicTasksPerStage[startOnGoalIndex].AddTask(name, FunctionType::Create(task), objects, accesses, taskObjectiveIndex, static_cast<void*>(taskInfo), GetPersistentAllocator());
		}

		BE_LOG_MESSAGE("Added dynamic task ", name.GetString(), " to goal ", startOn.GetString(), " to be done before ", doneFor.GetString())
	}

	template<typename... ARGS>
	void AddDynamicTask(const Id name, const GTSL::Delegate<void(TaskInfo, ARGS...)>& function, const GTSL::Range<const TaskDependency*> dependencies, ARGS&&... args)
	{
		auto task = [](GameInstance* gameInstance, const uint32 goal, const uint32 asyncTasksIndex, void* data) -> void
		{
			{
				auto* info = static_cast<DispatchTaskInfo<TaskInfo, ARGS...>*>(data);

				GTSL::Get<0>(info->Arguments).GameInstance = gameInstance;
				GTSL::Call(info->Delegate, GTSL::MoveRef(info->Arguments));
				GTSL::Delete<DispatchTaskInfo<TaskInfo, ARGS...>>(info, gameInstance->GetPersistentAllocator());
			}

			gameInstance->resourcesUpdated.NotifyAll();
			gameInstance->taskSorter.ReleaseResources(asyncTasksIndex);
		};

		GTSL::Array<uint16, 32> objects; GTSL::Array<AccessType, 32> accesses;

		{
			GTSL::ReadLock lock(stagesNamesMutex);
			decomposeTaskDescriptor(dependencies, objects, accesses);
		}

		{
			GTSL::WriteLock lock(asyncTasksMutex);
			auto* taskInfo = GTSL::New<DispatchTaskInfo<TaskInfo, ARGS...>>(GetPersistentAllocator(), function, TaskInfo(), GTSL::ForwardRef<ARGS>(args)...);
			asyncTasks.AddTask(name, FunctionType::Create(task), objects, accesses, 0xFFFFFFFF, static_cast<void*>(taskInfo), GetPersistentAllocator());
		}

		BE_LOG_MESSAGE("Added async task ", name.GetString())
	}

	template<typename... ARGS>
	DynamicTaskHandle<ARGS...> StoreDynamicTask(const Id name, const GTSL::Delegate<void(TaskInfo, ARGS...)>& function, const GTSL::Range<const TaskDependency*> dependencies)
	{
		GTSL::Array<uint16, 32> objects; GTSL::Array<AccessType, 32> accesses;

		auto task = [](GameInstance* gameInstance, const uint32 goal, const uint32 dynamicTaskIndex, void* data) -> void
		{
			{
				DispatchTaskInfo<TaskInfo, ARGS...>* info = static_cast<DispatchTaskInfo<TaskInfo, ARGS...>*>(data);
				GTSL::Get<0>(info->Arguments).GameInstance = gameInstance;
				GTSL::Call(info->Delegate, GTSL::MoveRef(info->Arguments));
				GTSL::Delete<DispatchTaskInfo<TaskInfo, ARGS...>>(info, gameInstance->GetPersistentAllocator());
			}

			gameInstance->resourcesUpdated.NotifyAll();
			gameInstance->taskSorter.ReleaseResources(dynamicTaskIndex);
		};

		uint32 index;

		auto* taskInfo = GTSL::New<DispatchTaskInfo<TaskInfo, ARGS...>>(GetPersistentAllocator(), function);

		{
			GTSL::ReadLock lock(stagesNamesMutex);
			decomposeTaskDescriptor(dependencies, objects, accesses);
		}
		
		{
			GTSL::WriteLock lock(storedDynamicTasksMutex);
			index = storedDynamicTasks.Emplace(StoredDynamicTaskData{ name, objects, accesses, FunctionType::Create(task), static_cast<void*>(taskInfo) });
		}

		return DynamicTaskHandle<ARGS...>(index);
	}
	
	template<typename... ARGS>
	void AddStoredDynamicTask(const DynamicTaskHandle<ARGS...> taskHandle, ARGS&&... args)
	{
		StoredDynamicTaskData storedDynamicTask;
		
		{
			GTSL::WriteLock lock(storedDynamicTasksMutex);
			storedDynamicTask = storedDynamicTasks[taskHandle.Reference];
			storedDynamicTasks.Pop(taskHandle.Reference);
		}

		DispatchTaskInfo<TaskInfo, ARGS...>* data = static_cast<DispatchTaskInfo<TaskInfo, ARGS...>*>(storedDynamicTask.Data);
		::new(&data->Arguments) GTSL::Tuple<TaskInfo, ARGS...>(TaskInfo(), GTSL::ForwardRef<ARGS>(args)...);
		
		{
			GTSL::WriteLock lock(asyncTasksMutex);
			asyncTasks.AddTask(storedDynamicTask.Name, storedDynamicTask.Function, storedDynamicTask.Objects, storedDynamicTask.Access, 0xFFFFFFFF, storedDynamicTask.Data, GetPersistentAllocator());
		}
	}

	void AddStage(Id name);

private:
	GTSL::Vector<GTSL::SmartPointer<World, BE::PersistentAllocatorReference>, BE::PersistentAllocatorReference> worlds;
	
	mutable GTSL::ReadWriteMutex systemsMutex;
	GTSL::KeepVector<GTSL::SmartPointer<System, BE::PersistentAllocatorReference>, BE::PersistentAllocatorReference> systems;
	GTSL::KeepVector<Id, BE::PersistentAllocatorReference> systemNames;
	GTSL::FlatHashMap<System*, BE::PersistentAllocatorReference> systemsMap;
	GTSL::FlatHashMap<uint32, BE::PersistentAllocatorReference> systemsIndirectionTable;
	
	template<typename... ARGS>
	struct DispatchTaskInfo
	{
		DispatchTaskInfo(const GTSL::Delegate<void(ARGS...)>& delegate) : Delegate(delegate)
		{
		}

		DispatchTaskInfo(const GTSL::Delegate<void(ARGS...)>& delegate, ARGS&&... args) : Delegate(delegate), Arguments(GTSL::ForwardRef<ARGS>(args)...)
		{
		}

		uint32 TaskIndex;
		GTSL::Delegate<void(ARGS...)> Delegate;
		GTSL::Tuple<ARGS...> Arguments;
	};
	
	mutable GTSL::ReadWriteMutex storedDynamicTasksMutex;
	struct StoredDynamicTaskData
	{
		Id Name; GTSL::Array<uint16, 16> Objects;  GTSL::Array<AccessType, 16> Access; FunctionType Function; void* Data;
	};
	GTSL::KeepVector<StoredDynamicTaskData, BE::PersistentAllocatorReference> storedDynamicTasks;

	mutable GTSL::ReadWriteMutex recurringTasksMutex;
	GTSL::Vector<Stage<FunctionType, BE::PersistentAllocatorReference>, BE::PersistentAllocatorReference> recurringTasksPerStage;
	mutable GTSL::ReadWriteMutex dynamicTasksPerStageMutex;
	GTSL::Vector<Stage<FunctionType, BE::PersistentAllocatorReference>, BE::PersistentAllocatorReference> dynamicTasksPerStage;
	
	mutable GTSL::ReadWriteMutex asyncTasksMutex;
	Stage<FunctionType, BE::PersistentAllocatorReference> asyncTasks;

	GTSL::ConditionVariable resourcesUpdated;
	
	mutable GTSL::ReadWriteMutex stagesNamesMutex;
	GTSL::Vector<Id, BE::PersistentAllocatorReference> stagesNames;

	mutable GTSL::ReadWriteMutex recurringTasksInfoMutex;
	GTSL::Vector<GTSL::Vector<GTSL::SmartPointer<void*, BE::PersistentAllocatorReference>, BE::PersistentAllocatorReference>, BE::PersistentAllocatorReference> recurringTasksInfo;

	TaskSorter<BE::PersistentAllocatorReference> taskSorter;
	
	GTSL::Vector<GTSL::Semaphore, BE::PAR> semaphores;

	uint32 scalingFactor = 16;

	uint64 frameNumber = 0;

	GTSL::StaticString<1024> genTaskLog(const char* from, Id taskName, const GTSL::Range<const AccessType*> accesses, const GTSL::Range<const uint16*> objects)
	{
		GTSL::StaticString<1024> log;

		log += from;
		log += taskName.GetString();

		log += '\n';
		
		log += "Accessed objects: \n	";
		for (uint16 i = 0; i < objects.ElementCount(); ++i)
		{
			log += "Obj: "; log += systemNames[objects[i]].GetString(); log += ". Access: "; log += AccessTypeToString(accesses[i]); log += "\n	";
		}

		return log;
	}
	
	GTSL::StaticString<1024> genTaskLog(const char* from, Id taskName, Id goalName, const GTSL::Range<const AccessType*> accesses, const GTSL::Range<const uint16*> objects)
	{
		GTSL::StaticString<1024> log;

		log += from;
		log += taskName.GetString();

		log += '\n';

		log += " Stage: ";
		log += goalName.GetString();

		log += '\n';
		
		log += "Accessed objects: \n	";
		for (uint16 i = 0; i < objects.ElementCount(); ++i)
		{
			log += "Obj: "; log += systemNames[objects[i]].GetString(); log += ". Access: "; log += AccessTypeToString(accesses[i]); log += "\n	";
		}

		return log;
	}

	uint16 getStageIndex(const Id name) const
	{
		uint16 i = 0; for (auto goal_name : stagesNames) { if (goal_name == name) break; ++i; }
		BE_ASSERT(i != stagesNames.GetLength(), "No stage found with that name!")
		return i;
	}
	
	template<uint32 N>
	void decomposeTaskDescriptor(GTSL::Range<const TaskDependency*> taskDependencies, GTSL::Array<uint16, N>& object, GTSL::Array<AccessType, N>& access)
	{
		object.Resize(taskDependencies.ElementCount()); access.Resize(taskDependencies.ElementCount());
		
		for (uint16 i = 0; i < static_cast<uint16>(taskDependencies.ElementCount()); ++i) //for each dependency
		{
			object[i] = systemsIndirectionTable.At(taskDependencies[i].AccessedObject());
			access[i] = (taskDependencies.begin() + i)->Access;
		}
	}

	[[nodiscard]] bool assertTask(const Id name, const Id startGoal, const Id endGoal, const GTSL::Range<const TaskDependency*> dependencies) const
	{
		{
			GTSL::ReadLock lock(stagesNamesMutex);
			
			if (stagesNames.Find(startGoal) == stagesNames.end())
			{
				BE_LOG_WARNING("Tried to add task ", name.GetString(), " to stage ", startGoal.GetString(), " which doesn't exist. Resolve this issue as it leads to undefined behavior in release builds!")
				return true;
			}

			//assert done for exists
			if (stagesNames.Find(endGoal) == stagesNames.end())
			{
				BE_LOG_WARNING("Tried to add task ", name.GetString(), " ending for stage ", endGoal.GetString(), " which doesn't exist. Resolve this issue as it leads to undefined behavior in release builds!")
				return true;
			}
		}

		{
			GTSL::ReadLock lock(recurringTasksMutex);
			
			if (recurringTasksPerStage[getStageIndex(startGoal)].DoesTaskExist(name))
			{
				BE_LOG_WARNING("Tried to add task ", name.GetString(), " which already exists to stage ", startGoal.GetString(), ". Resolve this issue as it leads to undefined behavior in release builds!")
				return true;
			}
		}

		{
			GTSL::ReadLock lock(systemsMutex);

			for(auto e : dependencies)
			{
				if (!systemsMap.Find(e.AccessedObject())) {
					BE_LOG_ERROR("Tried to add task ", name.GetString(), " to stage ", startGoal.GetString(), " with a dependency on ", e.AccessedObject.GetString(), " which doesn't exist. Resolve this issue as it leads to undefined behavior in release builds!")
					return true;
				}
			}
		}

		return false;
	}

	void initWorld(uint8 worldId);
	void initSystem(System* system, GTSL::Id64 name, const uint16 id);
	
public:
	template<typename T>
	T* AddSystem(const Id systemName)
	{
		System* system;

		uint32 l;
		
		{
			GTSL::WriteLock lock(systemsMutex);
			l = systems.Emplace(GTSL::SmartPointer<System, BE::PersistentAllocatorReference>::Create<T>(GetPersistentAllocator()));
			systemsMap.Emplace(systemName(), systems[l]);
			systemsIndirectionTable.Emplace(systemName(), l);
			systemNames.Emplace(systemName());
			system = systems[l];
		}

		initSystem(system, systemName, static_cast<uint16>(l));

		taskSorter.AddSystem(systemName);
		
		return static_cast<T*>(system);
	}
};
