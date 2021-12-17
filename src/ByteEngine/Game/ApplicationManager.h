#pragma once

#include <GTSL/Delegate.hpp>
#include <GTSL/HashMap.hpp>
#include <GTSL/Mutex.h>
#include <GTSL/Vector.hpp>
#include <GTSL/Algorithm.hpp>
#include <GTSL/Allocator.h>
#include <GTSL/Semaphore.h>

#include "System.h"
#include "Tasks.h"
#include "ByteEngine/Id.h"

#include "ByteEngine/Debug/Assert.h"

#include "ByteEngine/Handle.hpp"
#include "ByteEngine/Debug/FunctionTimer.h"

#include "ByteEngine/Application/Handle.hpp"

class World;
class ComponentCollection;
class System;

namespace BE {
	class Application;
}

template<typename... ARGS>
using Task = GTSL::Delegate<void(TaskInfo, ARGS...)>;

inline const char8_t* AccessTypeToString(const AccessType access) {
	switch (static_cast<uint8>(access)) {
	case static_cast<uint8>(AccessTypes::READ): return u8"READ";
	case static_cast<uint8>(AccessTypes::READ_WRITE): return u8"READ_WRITE";
	}
}

template<class T>
struct TypedDependency {
	TypedDependency(Id name) : Name(name) {}
	TypedDependency(Id name, AccessType at) : Name(name), Access(at) {}

	using type = T;
	Id Name; AccessType Access = AccessTypes::READ_WRITE;
};

template<class... C>
struct DependencyBlock {
	DependencyBlock(C... tds) : Names{ {}, tds.Name... }, AccessTypes{ {}, tds.Access... } {}

	Id Names[1 + sizeof...(C)];
	AccessType AccessTypes[1 + sizeof...(C)];
	uint64 Length = sizeof...(C);
};

template<typename... ACCESSES>
struct Resources{};

template<typename... ARGS>
struct DynamicTaskHandle {
	DynamicTaskHandle() = default;
	DynamicTaskHandle(uint32 reference) : Reference(reference) {}
	
	uint32 Reference = ~0U;

	operator bool() const { return Reference != ~0U; }
};

template<typename... ARGS>
struct EventHandle {
	EventHandle(const Id name) : Name(name) {}
	Id Name;
};

MAKE_HANDLE(uint32, System)

//#define Q(x) #x
//#define MAKE_TASK(am, className, functionName, startStage, endStage, dependencies, ...) am->AddTask(this, u8 Q(functionName), &className::functionName, dependencies, startStage, endStage, __VA_ARGS__)

#include "ByteEngine/Application/Application.h"

class ApplicationManager : public Object {
	using FunctionType = GTSL::Delegate<void(ApplicationManager*, DispatchedTaskHandle, void*)>;
public:
	ApplicationManager();
	~ApplicationManager();
	
	void OnUpdate(BE::Application* application);
	
	using WorldReference = uint8;
	
	struct CreateNewWorldInfo {};
	template<typename T>
	WorldReference CreateNewWorld(const CreateNewWorldInfo& createNewWorldInfo)
	{
		auto index = worlds.GetLength();
		worlds.EmplaceBack(GetPersistentAllocator());
		initWorld(index); return index;
	}

	void UnloadWorld(WorldReference worldId);

	void DestroyEntity(const BE::Handle handle) {
		//systems[handle.Identifier.SystemId]->Destroy(handle.Handle);
	}

	template<class T>
	T* GetSystem(const Id systemName) {
		GTSL::Lock lock(systemsMutex);
		return static_cast<T*>(systemsMap.At(systemName));
	}
	
	template<class T>
	T* GetSystem(const SystemHandle systemReference) {
		GTSL::Lock lock(systemsMutex);
		return static_cast<T*>(systems[systemReference()].GetData());
	}
	
	SystemHandle GetSystemReference(const Id systemName) {
		GTSL::Lock lock(systemsMutex);
		return SystemHandle(systemsIndirectionTable.At(systemName));
	}

	template<typename DTI, typename T, bool doDelete, typename... ACC>
	static void task(ApplicationManager* gameInstance, const DispatchedTaskHandle dispatched_task_handle, void* data) {
		DTI* info = static_cast<DTI*>(data);
		
		auto startTime = BE::Application::Get()->GetClock()->GetCurrentMicroseconds();
		call<T, typename ACC::type...>(static_cast<T*>(info->Callee), TaskInfo(gameInstance), info);
		GTSL::StaticString<512> args(u8"\"Start stage\":{ "); args += u8"\"Name\":\""; ToString(args, info->StartStage); args += u8"\", \"Index\":"; ToString(args, info->startStageIndex); args += u8" },";
		args += u8"\"End stage\":{ "; args += u8"\"Name\":\""; ToString(args, info->EndStage); args += u8"\", \"Index\":"; ToString(args, info->endStageIndex); args += u8" },";
		args += u8"\"Accesses\":[ ";
		for(auto&[name, access] : info->Accesses) {
			args += u8"\"System\":{ "; args += u8"\"Name\":\""; args += name; args += u8"\", \"Access type\":\""; args += AccessTypeToString(access); args += u8"\" }";
		}
		args += u8" ]";
		BE::Application::Get()->GetLogger()->logFunction(info->Name, startTime, BE::Application::Get()->GetClock()->GetCurrentMicroseconds(), args);
		
		if(info->endStageIndex != 0xFFFF) { gameInstance->semaphores[info->endStageIndex].Post(); }
		if constexpr (doDelete) { GTSL::Delete<DTI>(&info, gameInstance->GetPersistentAllocator()); }
		gameInstance->resourcesUpdated.NotifyAll();
		gameInstance->taskSorter.ReleaseResources(dispatched_task_handle);
	}

	template<class T, typename F, typename... ACC, typename... FARGS>
	void AddTask(T* caller, const Id name, F delegate, DependencyBlock<ACC...> dependencies, const Id startStage, const Id endStage, FARGS&&... args) {
		dependencies.Names[0] = caller->instanceName; dependencies.AccessTypes[0] = AccessTypes::READ_WRITE;

		if constexpr (_DEBUG) { if (assertTask(name, startStage, endStage, dependencies.Length + 1, dependencies.Names, dependencies.AccessTypes)) { return; } }

		GTSL::StaticVector<TaskAccess, 32> accesses;

		uint16 startStageIndex = 0xFFFF, endStageIndex = 0xFFFF;

		{
			GTSL::ReadLock lock(stagesNamesMutex);
			decomposeTaskDescriptor(dependencies.Length + 1, dependencies.Names, dependencies.AccessTypes, accesses);
			startStageIndex = getStageIndex(startStage); endStageIndex = getStageIndex(endStage);
		}

		[&] <typename... ARGS>(void(T::*function)(TaskInfo, typename ACC::type*..., ARGS...)) {
			using DTI = DispatchTaskInfo<ARGS...>;
			auto taskInfo = GTSL::SmartPointer<DTI, BE::PAR>(GetPersistentAllocator(), function, dependencies.Length, GTSL::ForwardRef<ARGS>(args)...);
			taskInfo->Callee = caller;
			taskInfo->startStageIndex = startStageIndex; taskInfo->endStageIndex = endStageIndex;

			if constexpr (BE_DEBUG) {
				GTSL::ShortString<128> n(caller->GetName()); n += u8"::"; n += GTSL::StringView(name);
				taskInfo->Name = n;
				taskInfo->StartStage = GTSL::StringView(startStage); taskInfo->EndStage = GTSL::StringView(endStage);
				for (uint32 i = 0; i < dependencies.Length + 1; ++i) {
					taskInfo->Accesses.EmplaceBack(GTSL::StringView(dependencies.Names[i]), dependencies.AccessTypes[i]);
				}
			}

			for(uint32 i = 0; i < dependencies.Length; ++i) {
				taskInfo->SetResource(i, systemsMap[dependencies.Names[1 + i]]);
			}

			{
				GTSL::WriteLock lock(recurringTasksInfoMutex);
				GTSL::WriteLock lock2(recurringTasksMutex);
				recurringTasksPerStage[startStageIndex].AddTask(name, FunctionType::Create<&ApplicationManager::task<DTI, T, false, ACC...>>(), accesses, endStageIndex, static_cast<void*>(taskInfo.GetData()), GetPersistentAllocator());
				recurringTasksInfo[startStageIndex].EmplaceBack(GTSL::MoveRef(taskInfo));
			}
		}(delegate);
	}
	
	void RemoveTask(Id taskName, Id startOn);

	template<class T, typename F, typename... ACC, typename... FARGS>
	void AddDynamicTask(T* caller, const Id name, DependencyBlock<ACC...> dependencies, F function, const Id startStage, const Id endStage, FARGS&&... args) {

		[&]<typename... ARGS>(void(T::* d)(TaskInfo, typename ACC::type*..., ARGS...)) {
			using DTI = DispatchTaskInfo<ARGS...>;

			static_assert((GTSL::IsSame<FARGS, ARGS>() && ...), "Provided parameter types for task are not compatible with those required.");

			auto* taskInfo = GTSL::New<DTI>(GetPersistentAllocator(), function, dependencies.Length, GTSL::ForwardRef<FARGS>(args)...);
			taskInfo->Callee = caller;

			dependencies.Names[0] = caller->instanceName; dependencies.AccessTypes[0] = AccessTypes::READ_WRITE;

			uint16 startStageIndex = 0xFFFF, endStageIndex = 0xFFFF;

			if constexpr (BE_DEBUG) {
				taskInfo->Name = GTSL::StringView(name);
				taskInfo->StartStage = GTSL::StringView(startStage); taskInfo->EndStage = GTSL::StringView(endStage);
				for (uint32 i = 0; i < dependencies.Length + 1; ++i) {
					taskInfo->Accesses.EmplaceBack(GTSL::StringView(dependencies.Names[i]), dependencies.AccessTypes[i]);
				}
			}

			for (uint32 i = 0; i < dependencies.Length; ++i) {
				taskInfo->SetResource(i, systemsMap[dependencies.Names[1 + i]]);
			}

			GTSL::StaticVector<TaskAccess, 32> accesses;

			if (startStage && endStage) {
				{
					GTSL::ReadLock lock(stagesNamesMutex);
					decomposeTaskDescriptor(dependencies.Length + 1, dependencies.Names, dependencies.AccessTypes, accesses);
					startStageIndex = getStageIndex(startStage);
					endStageIndex = getStageIndex(endStage);
				}

				{
					GTSL::WriteLock lock2(dynamicTasksPerStageMutex);
					dynamicTasksPerStage[startStageIndex].AddTask(name, FunctionType::Create<&ApplicationManager::task<DTI, T, true, ACC...>>(), accesses, endStageIndex, static_cast<void*>(taskInfo), GetPersistentAllocator());
				}
			}
			else { //no dependecies
				{
					GTSL::ReadLock lock(stagesNamesMutex);
					decomposeTaskDescriptor(dependencies.Length + 1, dependencies.Names, dependencies.AccessTypes, accesses);
				}

				{
					GTSL::WriteLock lock(asyncTasksMutex);
					asyncTasks.AddTask(name, FunctionType::Create<&ApplicationManager::task<DTI, T, true, ACC...>>(), accesses, 0xFFFFFFFF, static_cast<void*>(taskInfo), GetPersistentAllocator());
				}
			}

			taskInfo->startStageIndex = startStageIndex; taskInfo->endStageIndex = endStageIndex;
		}(function);
	}

	template<typename T, typename... ACC, typename... FARGS>
	[[nodiscard]] auto StoreDynamicTask(T* caller, const Id taskName, DependencyBlock<ACC...> dependencies, void(T::*delegate)(TaskInfo, FARGS...)) {
		uint32 index;

		GTSL::StaticVector<TaskAccess, 32> accesses;

		dependencies.Names[0] = caller->instanceName; dependencies.AccessTypes[0] = AccessTypes::READ_WRITE;

		//assertTask(taskName, {}, )

		{
			GTSL::ReadLock lock(stagesNamesMutex);
			decomposeTaskDescriptor(dependencies.Length + 1, dependencies.Names, dependencies.AccessTypes, accesses);
		}

		return[&]<typename... ARGS>(void(T:: * d)(TaskInfo, typename ACC::type*..., ARGS...)) {
			using DTI = DispatchTaskInfo<ARGS...>;

			{
				GTSL::WriteLock lock(storedDynamicTasksMutex);
				index = storedDynamicTasks.Emplace(StoredDynamicTaskData{ taskName, accesses, FunctionType::Create<&ApplicationManager::task<DTI, T, true, ACC...>>(), caller });
				auto& sdt = storedDynamicTasks[index];
				sdt.SetDelegate(d);
			}

			return DynamicTaskHandle<ARGS...>(index);
		} (delegate);
	}

	template<typename... ARGS>
	void AddStoredDynamicTask(const DynamicTaskHandle<ARGS...> taskHandle, ARGS&&... args) {
		StoredDynamicTaskData storedDynamicTask;
		
		{
			GTSL::WriteLock lock(storedDynamicTasksMutex);
			storedDynamicTask = storedDynamicTasks[taskHandle.Reference];
		}

		using DTI = DispatchTaskInfo<ARGS...>;

		DTI* taskInfo = GTSL::New<DTI>(GetPersistentAllocator());
		taskInfo->Name = GTSL::StringView(storedDynamicTask.Name);
		taskInfo->Callee = storedDynamicTask.Callee;
		taskInfo->ResourceCount = storedDynamicTask.Access.GetLength() - 1;
		taskInfo->WriteDelegateVoid(storedDynamicTask.TaskFunction);

		taskInfo->startStageIndex = storedDynamicTask.StartStageIndex; taskInfo->endStageIndex = storedDynamicTask.EndStageIndex;

		if constexpr (BE_DEBUG) {
			taskInfo->Name = GTSL::StringView(storedDynamicTask.Name);
			taskInfo->StartStage = storedDynamicTask.StartStage; taskInfo->EndStage = storedDynamicTask.EndStage;
			for (uint32 i = 0; i < storedDynamicTask.Access.GetLength(); ++i) {
				taskInfo->Accesses.EmplaceBack(GTSL::StringView(systemNames[storedDynamicTask.Access[i].First]), storedDynamicTask.Access[i].Second);
			}
		}

		for (uint32 i = 0; i < storedDynamicTask.Access.GetLength() - 1; ++i) {
			taskInfo->SetResource(i, systems[storedDynamicTask.Access[i + 1].First].GetData());
		}

		taskInfo->UpdateArguments(GTSL::ForwardRef<ARGS>(args)...);

		{
			GTSL::WriteLock lock(asyncTasksMutex);
			asyncTasks.AddTask(storedDynamicTask.Name, storedDynamicTask.GameInstanceFunction,storedDynamicTask.Access, 0xFFFFFFFF, taskInfo, GetPersistentAllocator());
		}
	}

	template<typename... ARGS>
	void AddEvent(const Id caller, const EventHandle<ARGS...> eventHandle, bool priority = false) {
		GTSL::WriteLock lock(eventsMutex);
		if constexpr (_DEBUG) { if (events.Find(eventHandle.Name)) { BE_LOG_ERROR(u8"An event by the name ", GTSL::StringView(eventHandle.Name), u8" already exists, skipping adition. ", BE::FIX_OR_CRASH_STRING); return; } }
		Event& eventData = events.Emplace(eventHandle.Name, GetPersistentAllocator());

		if(priority) {
			eventData.priorityEntry = 0;
		}
	}

	template<typename... ARGS>
	void SubscribeToEvent(const Id caller, const EventHandle<ARGS...> eventHandle, DynamicTaskHandle<ARGS...> taskHandle) {
		GTSL::WriteLock lock(eventsMutex);
		if constexpr (_DEBUG) { if (!events.Find(eventHandle.Name)) { BE_LOG_ERROR(u8"No event found by that name, skipping subscription. ", BE::FIX_OR_CRASH_STRING); return; } }
		auto& vector = events.At(eventHandle.Name).Functions;
		vector.EmplaceBack(taskHandle.Reference);
	}
	
	template<typename... ARGS>
	void DispatchEvent(const Id caller, const EventHandle<ARGS...> eventHandle, ARGS&&... args) {
		GTSL::ReadLock lock(eventsMutex);
		if constexpr (_DEBUG) { if (!events.Find(eventHandle.Name)) { BE_LOG_ERROR(u8"No event found by that name, skipping dispatch. ", BE::FIX_OR_CRASH_STRING); return; } }

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

	void AddStage(Id stageName);

private:
	GTSL::Vector<GTSL::SmartPointer<World, BE::PersistentAllocatorReference>, BE::PersistentAllocatorReference> worlds;
	
	mutable GTSL::Mutex systemsMutex;
	GTSL::FixedVector<GTSL::SmartPointer<System, BE::PersistentAllocatorReference>, BE::PersistentAllocatorReference> systems;
	GTSL::FixedVector<Id, BE::PersistentAllocatorReference> systemNames;
	GTSL::HashMap<Id, System*, BE::PersistentAllocatorReference> systemsMap;
	GTSL::HashMap<Id, uint32, BE::PersistentAllocatorReference> systemsIndirectionTable;
	
	/**
	 * \brief Stores all data necessary to invoke a task.
	 * Resource parameters are stored separately from data parameters because it simplifies accessing DispatchTaskInfo through type erased pointers since we don't need to know what resources the task requires only the data it uses.
	 * Such a use case can be seen with stored tasks, only StoreDynamicTask() can see the tasks full signature but can't allocate a DTI
	 * since every task needs it's own DTI instance which will be allocated when innvoking an stored dynamic task, but since AddStoredDynamicTask doesn't know the full
	 * signature it's easier to have DTIs use just the data parameters since that information is known thanks to the DynamicTaskHandle<ARGS...>.
	 * \tparam ARGS Types of the non resource parameters for a task.
	 */
	template<typename... ARGS>
	struct DispatchTaskInfo {
		DispatchTaskInfo() : Arguments{ 0 } {}

		template<typename T, typename... FULL_ARGS>
		DispatchTaskInfo(void(T::*function)(TaskInfo, FULL_ARGS...), uint32 sysCount) : ResourceCount(sysCount) {
			static_assert(sizeof(decltype(function)) == 8);
			WriteDelegate<T>(function);
		}

		template<typename T, typename... FULL_ARGS>
		DispatchTaskInfo(void(T::*function)(TaskInfo, FULL_ARGS...), uint32 sysCount, ARGS&&... args) requires static_cast<bool>(sizeof...(ARGS)) : ResourceCount(sysCount) {
			static_assert(sizeof(decltype(function)) == 8);
			WriteDelegate<T>(function);
			UpdateArguments(GTSL::ForwardRef<ARGS>(args)...);
		}

		DispatchTaskInfo(const DispatchTaskInfo&) = delete;
		DispatchTaskInfo(DispatchTaskInfo&&) = delete;

		~DispatchTaskInfo() {
			[&]<uint64... I>(GTSL::Indices<I...>) { (GetPointer<I>()->~ARGS(),...); } (GTSL::BuildIndices<sizeof...(ARGS)>{});

			if constexpr (BE_DEBUG) {
				Name = u8"deleted";
				Callee = nullptr;
			}
		}

		uint32 TaskIndex = 0;
		uint16 startStageIndex = 0xFFFF, endStageIndex = 0xFFFF;

#if BE_DEBUG
		GTSL::StaticString<64> Name = u8"null", StartStage, EndStage;
		GTSL::StaticVector<GTSL::Pair<GTSL::ShortString<32>, AccessType>, 8> Accesses;
#endif

		byte Delegate[8];
		void* Callee;
		uint32 ResourceCount = 0;
		byte Arguments[sizeof(System*) * 8 + GTSL::PackSize<ARGS...>()];

		template<class T, typename... RS>
		auto GetDelegate() -> void(T::*)(TaskInfo, RS*..., ARGS...) {
			union F {
				void(T::* Delegate)(TaskInfo, RS*..., ARGS...);
			};

			return reinterpret_cast<F*>(Delegate)->Delegate;
		}

		template<class T, typename... FULL_ARGS>
		void WriteDelegate(void(T::*d)(TaskInfo, FULL_ARGS...)) {
			union F {
				void(T::* Delegate)(TaskInfo, FULL_ARGS...);
			};

			reinterpret_cast<F*>(Delegate)->Delegate = d;
		}

		void WriteDelegateVoid(byte* buffer) {
			for (uint64 i = 0; i < 8; ++i) { Delegate[i] = buffer[i]; }
		}

		void SetResource(const uint64 pos, System* pointer) { *reinterpret_cast<System**>(Arguments + pos * 8) = pointer; }

		template<uint64 POS, typename T>
		T* GetResource() { return *reinterpret_cast<T**>(Arguments + POS * 8); }

		template<uint64 POS>
		auto GetPointer() { return reinterpret_cast<typename GTSL::TypeAt<POS, ARGS...>::type*>(Arguments + ResourceCount * 8 + GTSL::PackSizeAt<POS, ARGS...>()); }

		template<uint64 POS>
		auto& GetArgument() { return *GetPointer<POS>(); }

		void UpdateArguments(ARGS&&... args) {
			[&]<uint64... I>(GTSL::Indices<I...>){
				(::new(GetPointer<I>()) ARGS(GTSL::ForwardRef<ARGS>(args)), ...);
			} (GTSL::BuildIndices<sizeof...(ARGS)>{});
		}
	};

	template<typename T, typename... RS, typename... ARGS>
	static void call(T* whoToCall, TaskInfo task_info, DispatchTaskInfo<ARGS...>* dispatch_task_info) {
		[&] <uint64... RI, uint64... AI>(GTSL::Indices<RI...>, GTSL::Indices<AI...>) {
			(whoToCall->*dispatch_task_info->GetDelegate<T, RS...>())(task_info, dispatch_task_info->GetResource<RI, RS>()..., GTSL::MoveRef(dispatch_task_info->GetArgument<AI>())...);
		} (GTSL::BuildIndices<sizeof...(RS)>{}, GTSL::BuildIndices<sizeof...(ARGS)>{});
	}

	mutable GTSL::ReadWriteMutex storedDynamicTasksMutex;
	struct StoredDynamicTaskData {
		Id Name;
		GTSL::StaticVector<TaskAccess, 32> Access;
		FunctionType GameInstanceFunction;
		void* Callee;
		byte TaskFunction[8];

		uint16 StartStageIndex = 0xFFFF, EndStageIndex = 0xFFFF;
		GTSL::StaticString<64> StartStage, EndStage;

		template<typename F>
		void SetDelegate(F delegate) {
			auto* d = reinterpret_cast<byte*>(&delegate);
			for(uint64 i = 0; i < 8; ++i) {
				TaskFunction[i] = d[i];
			}
		}
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
	GTSL::Vector<GTSL::Vector<GTSL::SmartPointer<DispatchTaskInfo<uint32>, BE::PersistentAllocatorReference>, BE::PersistentAllocatorReference>, BE::PersistentAllocatorReference> recurringTasksInfo;

	TaskSorter<BE::PersistentAllocatorReference> taskSorter;
	
	GTSL::Semaphore semaphores[64];

	uint32 scalingFactor = 16;

	uint64 frameNumber = 0;

	uint16 getStageIndex(const Id stageName) const {
		auto findRes = GTSL::Find(stagesNames, [&](const Id& goal_name) { return goal_name == stageName; });
		BE_ASSERT(findRes, "No stage found with that name!")
		return findRes.Get() - stagesNames.begin();
	}
	
	template<typename U>
	void decomposeTaskDescriptor(uint64 len, const Id* names, const AccessType* accessTypes, U& access) {
		for (uint16 i = 0; i < len; ++i) { //for each dependency
			access.EmplaceBack(getSystemIndex(names[i]), accessTypes[i]);
		}
	}

	[[nodiscard]] bool assertTask(const Id taskName, const Id startGoal, const Id endGoal, const uint64 len, const Id* names, const AccessType* access) const {
		{
			GTSL::ReadLock lock(stagesNamesMutex);
			
			if (!stagesNames.Find(startGoal).State()) {
				BE_LOG_ERROR(u8"Tried to add task ", GTSL::StringView(taskName), u8" to stage ", GTSL::StringView(startGoal), u8" which doesn't exist. Resolve this issue as it leads to undefined behavior in release builds!")
				return true;
			}

			//assert done for exists
			if (!stagesNames.Find(endGoal).State()) {
				BE_LOG_ERROR(u8"Tried to add task ", GTSL::StringView(taskName), u8" ending on stage ", GTSL::StringView(endGoal), u8" which doesn't exist. Resolve this issue as it leads to undefined behavior in release builds!")
				return true;
			}
		}

		{
			GTSL::ReadLock lock(recurringTasksMutex);
			
			if (recurringTasksPerStage[getStageIndex(startGoal)].DoesTaskExist(taskName)) {
				BE_LOG_ERROR(u8"Tried to add task ", GTSL::StringView(taskName), u8" which already exists to stage ", GTSL::StringView(startGoal), u8". Resolve this issue as it leads to undefined behavior in release builds!")
				return true;
			}
		}

		{
			GTSL::Lock lock(systemsMutex);
		
			for(auto i = 0ull; i < len; ++i) {
				if (!doesSystemExist(names[i])) {
					BE_LOG_ERROR(u8"Tried to add task ", GTSL::StringView(taskName), u8" to stage ", GTSL::StringView(startGoal), u8" with a dependency on ", GTSL::StringView(names[i]), u8" which doesn't exist. Resolve this issue as it leads to undefined behavior in release builds!")
					return true;
				}
			}
		}

		return false;
	}

	void initWorld(uint8 worldId);

	uint32 getSystemIndex(Id systemName) {
		return systemsIndirectionTable[systemName];
	}

	bool doesSystemExist(const Id systemName) const {
		return systemsIndirectionTable.Find(systemName);
	}

public:
	/**
	 * \brief Create a system instance.
	 * \tparam T Class of system.
	 * \param systemName Identifying name for the system instance.
	 * \return A pointer to the created system.
	 */
	template<typename T>
	T* AddSystem(const Id systemName) {
		if constexpr (_DEBUG) {
			if (doesSystemExist(systemName)) {
				BE_LOG_ERROR(u8"System by that name already exists! Returning existing instance.", BE::FIX_OR_CRASH_STRING);
				return reinterpret_cast<T*>(systemsMap.At(systemName));
			}
		}

		T* systemPointer = nullptr; uint16 systemIndex = 0xFFFF;
		
		{
			System::InitializeInfo initializeInfo;
			initializeInfo.ApplicationManager = this;
			initializeInfo.ScalingFactor = scalingFactor;
			initializeInfo.InstanceName = systemName;

			GTSL::Lock lock(systemsMutex);

			//if (!systemsMap.Find(systemName)) {
				systemIndex = systemNames.Emplace(systemName);
				initializeInfo.SystemId = systemIndex;
				systemsIndirectionTable.Emplace(systemName, systemIndex);

				auto systemAllocation = GTSL::SmartPointer<T, BE::PAR>(GetPersistentAllocator(), initializeInfo);
				systemPointer = systemAllocation.GetData();

				systems.Emplace(GTSL::MoveRef(systemAllocation));
				taskSorter.AddSystem(systemName);
				systemsMap.Emplace(systemName, systemPointer);
			//} else {
			//	if (!systemsMap[systemName]) {
			//		initializeInfo.SystemId = systemIndex;
			//
			//		auto systemAllocation = GTSL::SmartPointer<T, BE::PAR>(GetPersistentAllocator(), initializeInfo);
			//		systemPointer = systemAllocation.GetData();
			//
			//		systems.Pop(systemsIndirectionTable[systemName]);
			//		systems.EmplaceAt(systemsIndirectionTable[systemName], GTSL::MoveRef(systemAllocation));
			//		systemsMap[systemName] = systemPointer;
			//	}
			//}

		}

		systemPointer->systemId = systemIndex;
		systemPointer->instanceName = systemName;
		
		return systemPointer;
	}
};
