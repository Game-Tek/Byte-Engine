#pragma once

#include <GTSL/Delegate.hpp>
#include <GTSL/HashMap.hpp>
#include <GTSL/Mutex.h>
#include <GTSL/Vector.hpp>
#include <GTSL/Algorithm.h>
#include <GTSL/Allocator.h>
#include <GTSL/Semaphore.h>

#include "System.h"
#include "Tasks.h"
#include "ByteEngine/Id.h"

#include "ByteEngine/Debug/Assert.h"

#include "ByteEngine/Handle.hpp"
#include "ByteEngine/Debug/FunctionTimer.h"

class World;
class ComponentCollection;
class System;

namespace BE {
	class Application;
}

template<typename... ARGS>
using Task = GTSL::Delegate<void(TaskInfo, ARGS...)>;

inline const char8_t* AccessTypeToString(const AccessType access)
{
	switch (static_cast<uint8>(access))
	{
	case static_cast<uint8>(AccessTypes::READ): return u8"READ";
	case static_cast<uint8>(AccessTypes::READ_WRITE): return u8"READ_WRITE";
	}
}

template<typename... ARGS>
struct DynamicTaskHandle
{
	DynamicTaskHandle() = default;
	DynamicTaskHandle(uint32 reference) : Reference(reference) {}
	
	uint32 Reference = ~0U;

	operator bool() const { return Reference != ~0U; }
};

template<typename... ARGS>
struct EventHandle
{
	EventHandle(const Id name) : Name(name) {}
	Id Name;
};

MAKE_HANDLE(uint32, System)

class ApplicationManager : public Object
{
	using FunctionType = GTSL::Delegate<void(ApplicationManager*, uint32, uint32, void*)>;
public:
	ApplicationManager();
	~ApplicationManager();
	
	void OnUpdate(BE::Application* application);

	
	using WorldReference = uint8;
	
	struct CreateNewWorldInfo
	{
	};
	template<typename T>
	WorldReference CreateNewWorld(const CreateNewWorldInfo& createNewWorldInfo)
	{
		auto index = worlds.GetLength();
		worlds.EmplaceBack(GetPersistentAllocator());
		initWorld(index); return index;
	}

	void UnloadWorld(WorldReference worldId);
	
	template<class T>
	T* GetSystem(const Id systemName)
	{
		GTSL::Lock lock(systemsMutex);
		return static_cast<T*>(systemsMap.At(systemName));
	}
	
	template<class T>
	T* GetSystem(const SystemHandle systemReference)
	{
		GTSL::Lock lock(systemsMutex);
		return static_cast<T*>(systems[systemReference()].GetData());
	}
	
	SystemHandle GetSystemReference(const Id systemName)
	{
		GTSL::Lock lock(systemsMutex);
		return SystemHandle(systemsIndirectionTable.At(systemName));
	}

	template<typename... ARGS>
	void AddTask(const Id name, const GTSL::Delegate<void(TaskInfo, ARGS...)>& function, const GTSL::Range<const TaskDependency*> dependencies, const Id startOn, const Id doneFor, ARGS&&... args) {
		if constexpr (_DEBUG) { if (assertTask(name, startOn, doneFor, dependencies)) { return; } }
		
		auto taskInfo = GTSL::SmartPointer<DispatchTaskInfo<TaskInfo, ARGS...>, BE::PAR>(GetPersistentAllocator(), function, TaskInfo(), GTSL::ForwardRef<ARGS>(args)...);

		taskInfo->Name = name.GetString();
		
		auto task = [](ApplicationManager* gameInstance, const uint32 goal, const uint32 dynamicTaskIndex, void* data) -> void {			
			DispatchTaskInfo<TaskInfo, ARGS...>* info = static_cast<DispatchTaskInfo<TaskInfo, ARGS...>*>(data);

			BE_ASSERT(info->Counter == 0, "")
			
			++info->Counter;
			
			GTSL::Get<0>(info->Arguments).ApplicationManager = gameInstance;
			
			{
				FunctionTimer f(info->Name);
				GTSL::Call(info->Delegate, GTSL::MoveRef(info->Arguments));
			}
			
			--info->Counter;

			gameInstance->resourcesUpdated.NotifyAll();
			gameInstance->semaphores[goal].Post();
			gameInstance->taskSorter.ReleaseResources(dynamicTaskIndex);
		};

		GTSL::StaticVector<uint16, 32> objects; GTSL::StaticVector<AccessType, 32> accesses;

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
	}
	
	void RemoveTask(Id name, Id startOn);

	template<typename... ARGS>
	void AddDynamicTask(const Id name, const GTSL::Delegate<void(TaskInfo, ARGS...)>& function, const GTSL::Range<const TaskDependency*> dependencies, const Id startOn, const Id doneFor, ARGS&&... args) {
		auto* taskInfo = GTSL::New<DispatchTaskInfo<TaskInfo, ARGS...>>(GetPersistentAllocator(), function, TaskInfo(), GTSL::ForwardRef<ARGS>(args)...);
		taskInfo->Name = name.GetString();
		
		GTSL::StaticVector<uint16, 32> objects; GTSL::StaticVector<AccessType, 32> accesses;

		uint16 startOnGoalIndex, taskObjectiveIndex;

		auto task = [](ApplicationManager* gameInstance, const uint32 goal, const uint32 dynamicTaskIndex, void* data) -> void {
			DispatchTaskInfo<TaskInfo, ARGS...>* info = static_cast<DispatchTaskInfo<TaskInfo, ARGS...>*>(data);

			GTSL::Get<0>(info->Arguments).ApplicationManager = gameInstance;
			
			{
				FunctionTimer f(info->Name);
				GTSL::Call(info->Delegate, GTSL::MoveRef(info->Arguments));
			}

			gameInstance->resourcesUpdated.NotifyAll();
			gameInstance->semaphores[goal].Post();
			GTSL::Delete<DispatchTaskInfo<TaskInfo, ARGS...>>(&info, gameInstance->GetPersistentAllocator());			

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
	}

	template<typename... ARGS>
	void AddDynamicTask(const Id name, const GTSL::Delegate<void(TaskInfo, ARGS...)>& function, const GTSL::Range<const TaskDependency*> dependencies, ARGS&&... args) {
		auto task = [](ApplicationManager* gameInstance, const uint32 goal, const uint32 asyncTasksIndex, void* data) -> void {
			auto* info = static_cast<DispatchTaskInfo<TaskInfo, ARGS...>*>(data);
			
			GTSL::Get<0>(info->Arguments).ApplicationManager = gameInstance;

			{
				FunctionTimer f(info->Name);
				GTSL::Call(info->Delegate, GTSL::MoveRef(info->Arguments));
			}
			
			GTSL::Delete<DispatchTaskInfo<TaskInfo, ARGS...>>(&info, gameInstance->GetPersistentAllocator());			

			gameInstance->resourcesUpdated.NotifyAll();
			gameInstance->taskSorter.ReleaseResources(asyncTasksIndex);
		};

		GTSL::StaticVector<uint16, 32> objects; GTSL::StaticVector<AccessType, 32> accesses;

		{
			GTSL::ReadLock lock(stagesNamesMutex);
			decomposeTaskDescriptor(dependencies, objects, accesses);
		}

		{
			GTSL::WriteLock lock(asyncTasksMutex);
			auto* taskInfo = GTSL::New<DispatchTaskInfo<TaskInfo, ARGS...>>(GetPersistentAllocator(), function, TaskInfo(), GTSL::ForwardRef<ARGS>(args)...);
			taskInfo->Name = name.GetString();
			asyncTasks.AddTask(name, FunctionType::Create(task), objects, accesses, 0xFFFFFFFF, static_cast<void*>(taskInfo), GetPersistentAllocator());
		}
	}

	template<typename... ARGS>
	[[nodiscard]] DynamicTaskHandle<ARGS...> StoreDynamicTask(const Id name, const GTSL::Delegate<void(TaskInfo, ARGS...)>& function, const GTSL::Range<const TaskDependency*> dependencies) {
		GTSL::StaticVector<uint16, 32> objects; GTSL::StaticVector<AccessType, 32> accesses;

		auto task = [](ApplicationManager* gameInstance, const uint32 goal, const uint32 dynamicTaskIndex, void* data) -> void {			
			DispatchTaskInfo<TaskInfo, ARGS...>* info = static_cast<DispatchTaskInfo<TaskInfo, ARGS...>*>(data);
			
			GTSL::Get<0>(info->Arguments).ApplicationManager = gameInstance;

			{
				FunctionTimer f(info->Name);
				GTSL::Call(info->Delegate, GTSL::MoveRef(info->Arguments));
			}
			
			GTSL::Delete<DispatchTaskInfo<TaskInfo, ARGS...>>(&info, gameInstance->GetPersistentAllocator());

			gameInstance->resourcesUpdated.NotifyAll();
			gameInstance->taskSorter.ReleaseResources(dynamicTaskIndex);
		};

		uint32 index;

		{
			GTSL::ReadLock lock(stagesNamesMutex);
			decomposeTaskDescriptor(dependencies, objects, accesses);
		}
		
		{
			GTSL::WriteLock lock(storedDynamicTasksMutex);
			index = storedDynamicTasks.Emplace(StoredDynamicTaskData{ name, objects, accesses, FunctionType::Create(task), function });
		}

		return DynamicTaskHandle<ARGS...>(index);
	}
	
	template<typename... ARGS>
	void AddStoredDynamicTask(const DynamicTaskHandle<ARGS...> taskHandle, ARGS&&... args) {
		StoredDynamicTaskData storedDynamicTask;
		
		{
			GTSL::WriteLock lock(storedDynamicTasksMutex);
			storedDynamicTask = storedDynamicTasks[taskHandle.Reference];
		}

		auto* taskInfo = GTSL::New<DispatchTaskInfo<TaskInfo, ARGS...>>(GetPersistentAllocator(), GTSL::Delegate<void(TaskInfo, ARGS...)>(storedDynamicTask.AnonymousFunction), TaskInfo(), GTSL::ForwardRef<ARGS>(args)...);
		taskInfo->Name = storedDynamicTask.Name.GetString();
		
		{
			GTSL::WriteLock lock(asyncTasksMutex);
			asyncTasks.AddTask(storedDynamicTask.Name, storedDynamicTask.GameInstanceFunction, storedDynamicTask.Objects, storedDynamicTask.Access, 0xFFFFFFFF, (void*)taskInfo, GetPersistentAllocator());
		}
	}

	template<typename... ARGS>
	void AddEvent(const Id caller, const EventHandle<ARGS...> eventHandle, bool priority = false) {
		GTSL::WriteLock lock(eventsMutex);
		if constexpr (_DEBUG) { if (events.Find(eventHandle.Name)) { BE_LOG_ERROR("An event by the name ", eventHandle.Name.GetString(), " already exists, skipping adition. ", BE::FIX_OR_CRASH_STRING); return; } }
		Event& eventData = events.Emplace(eventHandle.Name, GetPersistentAllocator());

		if(priority) {
			eventData.priorityEntry = 0;
		}
	}

	template<typename... ARGS>
	void SubscribeToEvent(const Id caller, const EventHandle<ARGS...> eventHandle, DynamicTaskHandle<ARGS...> taskHandle) {
		GTSL::WriteLock lock(eventsMutex);
		if constexpr (_DEBUG) { if (!events.Find(eventHandle.Name)) { BE_LOG_ERROR("No event found by that name, skipping subscription. ", BE::FIX_OR_CRASH_STRING); return; } }
		auto& vector = events.At(eventHandle.Name).Functions;
		vector.EmplaceBack(taskHandle.Reference);
	}
	
	template<typename... ARGS>
	void DispatchEvent(const Id caller, const EventHandle<ARGS...> eventHandle, ARGS&&... args) {
		GTSL::ReadLock lock(eventsMutex);
		if constexpr (_DEBUG) { if (!events.Find(eventHandle.Name)) { BE_LOG_ERROR("No event found by that name, skipping dispatch. ", BE::FIX_OR_CRASH_STRING); return; } }

		Event& eventData = events.At(eventHandle.Name);

		if(eventData.priorityEntry != ~0U) {
			AddStoredDynamicTask(DynamicTaskHandle<ARGS...>(eventData.Functions[eventData.priorityEntry]), GTSL::ForwardRef<ARGS>(args)...);
		} else {
			auto& functionList = eventData.Functions;
			for (auto e : functionList) { AddStoredDynamicTask(DynamicTaskHandle<ARGS...>(e), GTSL::ForwardRef<ARGS>(args)...); }
		}
	}

	template<typename... ARGS>
	void SetEventPrioritizedSubscriber(const EventHandle<ARGS...> eventHandle, const uint32 prioritized) { //todo: make event subscription handle
		events[eventHandle.Name].priorityEntry = prioritized;
	}

	template<typename... ARGS>
	void SetEventPriority(const EventHandle<ARGS...> eventHandle, const bool priority) { //todo: make event subscription handle
		events[eventHandle.Name].priorityEntry = priority ? 0 : ~0U;
	}

	void AddStage(Id name);

private:
	GTSL::Vector<GTSL::SmartPointer<World, BE::PersistentAllocatorReference>, BE::PersistentAllocatorReference> worlds;
	
	mutable GTSL::Mutex systemsMutex;
	GTSL::FixedVector<GTSL::SmartPointer<System, BE::PersistentAllocatorReference>, BE::PersistentAllocatorReference> systems;
	GTSL::FixedVector<Id, BE::PersistentAllocatorReference> systemNames;
	GTSL::HashMap<Id, System*, BE::PersistentAllocatorReference> systemsMap;
	GTSL::HashMap<Id, uint32, BE::PersistentAllocatorReference> systemsIndirectionTable;
	
	template<typename... ARGS>
	struct DispatchTaskInfo
	{
		DispatchTaskInfo(const GTSL::Delegate<void(ARGS...)>& delegate) : Delegate(delegate)
		{
		}

		DispatchTaskInfo(const GTSL::Delegate<void(ARGS...)>& delegate, ARGS&&... args) : Delegate(delegate), Arguments(GTSL::ForwardRef<ARGS>(args)...)
		{
		}

		GTSL::StaticString<64> Name;
		uint32 TaskIndex, Counter = 0;
		GTSL::Delegate<void(ARGS...)> Delegate;
		GTSL::Tuple<ARGS...> Arguments;
	};
	
	mutable GTSL::ReadWriteMutex storedDynamicTasksMutex;
	struct StoredDynamicTaskData
	{
		Id Name; GTSL::StaticVector<uint16, 32> Objects;  GTSL::StaticVector<AccessType, 32> Access; FunctionType GameInstanceFunction; GTSL::Delegate<void()> AnonymousFunction;
	};
	GTSL::FixedVector<StoredDynamicTaskData, BE::PersistentAllocatorReference> storedDynamicTasks;

	mutable GTSL::ReadWriteMutex eventsMutex;

	struct Event {
		Event(const BE::PAR& allocator) : Functions(allocator) {}

		uint32 priorityEntry = ~0U;
		GTSL::Vector<uint32, BE::PAR> Functions;
	};
	GTSL::HashMap<Id, Event, BE::PersistentAllocatorReference> events;

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
	GTSL::Vector<GTSL::Vector<GTSL::SmartPointer<DispatchTaskInfo<TaskInfo>, BE::PersistentAllocatorReference>, BE::PersistentAllocatorReference>, BE::PersistentAllocatorReference> recurringTasksInfo;

	TaskSorter<BE::PersistentAllocatorReference> taskSorter;
	
	GTSL::Vector<GTSL::Semaphore, BE::PAR> semaphores;

	uint32 scalingFactor = 16;

	uint64 frameNumber = 0;

	GTSL::StaticString<1024> genTaskLog(const char8_t* from, Id taskName, const GTSL::Range<const AccessType*> accesses, const GTSL::Range<const uint16*> objects)
	{
		GTSL::StaticString<1024> log;

		log += from;
		log += taskName.GetString();

		log += u8'\n';
		
		log += u8"Accessed objects: \n	";
		for (uint16 i = 0; i < objects.ElementCount(); ++i)
		{
			log += u8"Obj: "; log += systemNames[objects[i]].GetString(); log += u8". Access: "; log += AccessTypeToString(accesses[i]); log += u8"\n	";
		}

		return log;
	}
	
	GTSL::StaticString<1024> genTaskLog(const char8_t* from, Id taskName, Id goalName, const GTSL::Range<const AccessType*> accesses, const GTSL::Range<const uint16*> objects)
	{
		GTSL::StaticString<1024> log;

		log += from;
		log += taskName.GetString();

		log += u8'\n';

		log += u8" Stage: ";
		log += goalName.GetString();

		log += u8'\n';
		
		log += u8"Accessed objects: \n	";
		for (uint16 i = 0; i < objects.ElementCount(); ++i)
		{
			log += u8"Obj: "; log += systemNames[objects[i]].GetString(); log += u8". Access: "; log += AccessTypeToString(accesses[i]); log += u8"\n	";
		}

		return log;
	}

	uint16 getStageIndex(const Id name) const
	{
		uint16 i = 0; for (auto goal_name : stagesNames) { if (goal_name == name) break; ++i; }
		BE_ASSERT(i != stagesNames.GetLength(), "No stage found with that name!")
		return i;
	}
	
	template<typename T, typename U>
	void decomposeTaskDescriptor(GTSL::Range<const TaskDependency*> taskDependencies, T& object, U& access)
	{		
		for (uint16 i = 0; i < static_cast<uint16>(taskDependencies.ElementCount()); ++i) { //for each dependency
			object.EmplaceBack(systemsIndirectionTable.At(taskDependencies[i].AccessedObject));
			access.EmplaceBack(taskDependencies[i].Access);
		}
	}

	[[nodiscard]] bool assertTask(const Id name, const Id startGoal, const Id endGoal, const GTSL::Range<const TaskDependency*> dependencies) const
	{
		{
			GTSL::ReadLock lock(stagesNamesMutex);
			
			if (!stagesNames.Find(startGoal).State())
			{
				BE_LOG_WARNING("Tried to add task ", name.GetString(), " to stage ", startGoal.GetString(), " which doesn't exist. Resolve this issue as it leads to undefined behavior in release builds!")
				return true;
			}

			//assert done for exists
			if (!stagesNames.Find(endGoal).State())
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
			GTSL::Lock lock(systemsMutex);

			for(auto e : dependencies)
			{
				if (!systemsMap.Find(e.AccessedObject)) {
					BE_LOG_ERROR("Tried to add task ", name.GetString(), " to stage ", startGoal.GetString(), " with a dependency on ", e.AccessedObject.GetString(), " which doesn't exist. Resolve this issue as it leads to undefined behavior in release builds!")
					return true;
				}
			}
		}

		return false;
	}

	void initWorld(uint8 worldId);
	
public:
	template<typename T>
	T* AddSystem(const Id systemName)
	{
		if constexpr (_DEBUG) {
			if (systemsMap.Find(systemName)) {
				BE_LOG_ERROR("System by that name already exists! Returning existing instance.", BE::FIX_OR_CRASH_STRING);
				return reinterpret_cast<T*>(systemsMap.At(systemName));
			}
		}

		T* systemPointer = nullptr;
		
		taskSorter.AddSystem(systemName);
		
		{
			System::InitializeInfo initializeInfo;
			initializeInfo.GameInstance = this;
			initializeInfo.ScalingFactor = scalingFactor;
			
			GTSL::Lock lock(systemsMutex);
			auto systemIndex = systemNames.Emplace(systemName);
			auto& t = systemsMap.Emplace(systemName, reinterpret_cast<System*>(systemPointer));
			systemsIndirectionTable.Emplace(systemName, systemIndex);
			
			systems.Emplace(GTSL::SmartPointer<T, BE::PAR>(GetPersistentAllocator(), initializeInfo));
			systemPointer = reinterpret_cast<T*>(systems[systemIndex].GetData());
			t = reinterpret_cast<System*>(systemPointer);
		}

		
		return systemPointer;
	}
};
