#pragma once

#include <GTSL/Delegate.hpp>
#include <GTSL/HashMap.hpp>
#include <GTSL/Mutex.h>
#include <GTSL/Vector.hpp>
#include <GTSL/Algorithm.hpp>
#include <GTSL/Allocator.h>
#include <GTSL/Semaphore.h>
#include <GTSL/Atomic.hpp>

#include "Tasks.h"
#include "ByteEngine/Id.h"

#include "ByteEngine/Debug/Assert.h"

#include "ByteEngine/Handle.hpp"
#include "ByteEngine/Debug/FunctionTimer.h"

#include "ByteEngine/Application/Handle.hpp"

#include "ByteEngine/Game/System.hpp"

class World;
class ComponentCollection;

namespace BE {
	class Application;
	class System;
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
struct Resources {};

template<typename... ARGS>
struct TaskHandle {
	TaskHandle() = default;
	TaskHandle(uint32 reference) : Reference(reference) {}

	uint32 Reference = ~0U;

	operator bool() const { return Reference != ~0U; }

	uint32 operator()() const { return Reference; }
};

template<typename... ARGS>
struct EventHandle {
	EventHandle(const Id name) : Name(name) {}
	Id Name;
};

MAKE_HANDLE(uint32, System)

namespace BE {
	struct TypeIdentifier {
		TypeIdentifier() = delete;
		TypeIdentifier(const uint16 sysid, const uint16 tid) : SystemId(sysid), TypeId(tid) {}

		uint16 SystemId = 0xFFFF, TypeId = 0xFFFF;

		uint32 operator()() const { return (SystemId | TypeId << 16); }
	};

	template<typename T>
	struct Handle {
		Handle() : Identifier(0xFFFF, 0xFFFF), EntityIndex(0xFFFFFFFF) {}
		Handle(TypeIdentifier type_identifier, uint32 handle) : Identifier(type_identifier), EntityIndex(handle) {}
		Handle(const Handle&) = default;
		//Handle& operator=(const Handle& other) {
		//	Identifier.SystemId = other.Identifier.SystemId;
		//	Identifier.TypeId = other.Identifier.TypeId;
		//	EntityIndex = other.EntityIndex;
		//}

		uint32 operator()() const { return EntityIndex; }

		explicit operator uint64() const { return EntityIndex; }
		explicit operator bool() const { return EntityIndex != 0xFFFFFFFF; }

		TypeIdentifier Identifier;
		uint32 EntityIndex = 0xFFFFFFFF;
	};

	//static_assert(sizeof(Handle<struct RRRR {}> ) <= 8);

#define MAKE_BE_HANDLE(name)\
	using name##Handle = BE::Handle<struct name##_tag>;
}

#define BE_RESOURCES(...) __VA_ARGS__
#define DECLARE_BE_TASK(name, res, ...) private: TaskHandle<__VA_ARGS__> name##TaskHandle; public: auto Get##name##TaskHandle() const { return name##TaskHandle; }
#define DECLARE_BE_TYPE(name) MAKE_BE_HANDLE(name); private: BE::TypeIdentifier name##TypeIndentifier; public: BE::TypeIdentifier Get##name##TypeIdentifier() const { return name##TypeIndentifier; }

template <class, template <class> class>
struct is_instance : public std::false_type {};

template <class T, template <class> class U>
struct is_instance<U<T>, U> : public std::true_type {};

class ApplicationManager : public Object {
	MAKE_HANDLE(uint32, TypeErasedTask)
		using FunctionType = GTSL::Delegate<void(ApplicationManager*, const DispatchedTaskHandle, TypeErasedTaskHandle)>;
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

	template<typename T>
	void DestroyEntity(const T handle) {
		auto& typeData = systemsData[handle.Identifier.SystemId].RegisteredTypes[handle.Identifier.TypeId];

		auto& ent = typeData.Entities[handle.EntityIndex];

		if (!(--ent.Uses)) {
			if (typeData.DeletionTaskHandle) { //if we have a valid deletion handle
				//EnqueueTask(TaskHandle<T>(typeData.DeletionTaskHandle()), GTSL::MoveRef(handle));
				//enqueue and then call task
			}
			else {
				BE_LOG_WARNING(u8"No deletion task available.");
			}
		}
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

	BE::TypeIdentifier RegisterType(const BE::System* system, const GTSL::StringView typeName);

	template<typename... ARGS>
	void BindTaskToType(const BE::TypeIdentifier type_identifier, const TaskHandle<ARGS...> handle) {
		systemsData[type_identifier.SystemId].RegisteredTypes[type_identifier()].Target += 1;
	}

	template<typename T>
	void BindDeletionTaskToType(const BE::TypeIdentifier handle, const TaskHandle<T> deletion_task_handle) {
		systemsData[handle.SystemId].RegisteredTypes[handle()].DeletionTaskHandle = TypeErasedTaskHandle(deletion_task_handle.Reference);
	}

	template<typename... ARGS1, typename... ARGS2>
	void SpecifyTaskCoDependency(const TaskHandle<ARGS1...> a, const TaskHandle<ARGS2...> b) {
		TaskData& taskA = tasks[a()];
		taskA.IsDependedOn = true;
		TaskData& taskB = tasks[b()];
		taskB.Pre = a();
	}

	template<typename... ARGS>
	void AddTypeSetupDependency(BE::System* system_pointer, BE::TypeIdentifier type_identifier, TaskHandle<ARGS...> dynamic_task_handle, bool is_required = true) {
		auto& system = systemsData[system_pointer->GetSystemId()];

		auto& s = systemsData[type_identifier.SystemId];

		if (auto r = system.RegisteredTypes.TryEmplace(type_identifier(), GetPersistentAllocator())) {
			auto& type = r.Get();
			type.IsOwn = false;

			GTSL::IndexedForEach(s.RegisteredTypes[type_identifier()].Entities, [&](uint32 index, SystemData::TypeData::EntityData&) { if (!system.RegisteredTypes[type_identifier()].Entities.IsSlotOccupied(index)) { system.RegisteredTypes[type_identifier()].Entities.EmplaceAt(index); } });
		}

		auto& type = system.RegisteredTypes[type_identifier()];
		type.SetupSteps.EmplaceBack(TypeErasedTaskHandle(dynamic_task_handle()), is_required);
		++type.Target;

		auto& vs = s.RegisteredTypes[type_identifier()].VisitingSystems;

		if (!vs.Find(system_pointer->GetSystemId())) { // If system is not already added
			vs.EmplaceBack(system_pointer->GetSystemId()); // Add system requesting to listen as visiting system to the listened type's information, to know which system to update.
		}
	}

	template<typename... ARGS>
	void AddTypeSetupDependency(BE::TypeIdentifier type_identifer, TaskHandle<ARGS...> dynamic_task_handle, bool is_required = true) {
		auto& system = systemsData[type_identifer.SystemId]; auto& type = system.RegisteredTypes[type_identifer()];
		type.SetupSteps.EmplaceBack(TypeErasedTaskHandle(dynamic_task_handle()), is_required);
		++type.Target;
	}

	template<typename T, typename DTI, typename... ACC>
	static void task(ApplicationManager* gameInstance, const DispatchedTaskHandle dispatched_task_handle, TypeErasedTaskHandle task_handle) {
		const TaskData& task = gameInstance->tasks[task_handle()];

		auto instances = gameInstance->taskSorter.GetValidInstances(dispatched_task_handle);

		for (auto e : instances) {
			DTI* info = static_cast<DTI*>(e);

			auto startTime = BE::Application::Get()->GetClock()->GetCurrentMicroseconds();

			call<T, typename ACC::type...>(TaskInfo(gameInstance), &task, info);

			GTSL::StaticString<512> args(u8"\"Start stage\":{ "); args += u8"\"Name\":\""; ToString(args, task.StartStage); args += u8"\", \"Index\":"; ToString(args, task.StartStageIndex); args += u8" },";
			args += u8"\"End stage\":{ "; args += u8"\"Name\":\""; ToString(args, task.EndStage); args += u8"\", \"Index\":"; ToString(args, task.EndStageIndex); args += u8" },";
			args += u8"\"Accesses\":[ ";
			for (auto& [name, access] : task.Access) {
				args += u8"\"System\":{ "; args += u8"\"Name\":\""; args += name; args += u8"\", \"Access type\":\""; args += AccessTypeToString(access); args += u8"\" }";
			}
			args += u8" ]";

			BE::Application::Get()->GetLogger()->logFunction(task.Name, startTime, BE::Application::Get()->GetClock()->GetCurrentMicroseconds(), args);

			if (task.EndStageIndex != 0xFFFF) { gameInstance->semaphores[task.EndStageIndex].Post(); }
			if (info->InstanceIndex != 0xFFFFFFFF) { ++gameInstance->systemsData[info->SystemId].RegisteredTypes[info->TTID()].Entities[info->InstanceIndex].ResourceCounter; }

			++info->D_CallCount;

			if (!task.Scheduled) { GTSL::Delete<DTI>(&info, gameInstance->GetPersistentAllocator()); } // KEEP LAST AS THIS ERASES DATA
		}

		--gameInstance->tasksInFlight;
		gameInstance->resourcesUpdated.NotifyAll();
		gameInstance->taskSorter.ReleaseResources(dispatched_task_handle);
	}

	/**
	 * \brief Registers a task with the application manager.
	 * \tparam T Callee type.
	 * \tparam ACC Typed dependencies types.
	 * \tparam FARGS Task function parameter types.
	 * \param caller Pointer to the system registering the function.
	 * \param taskName Task name.
	 * \param dependencies Dependencies list indicating which system will be accessed during this task and in what way(READ, READ_WRITE).
	 * \param delegate Pointer to the function which will be called.
	 * \param start_stage Stage at which the task can begin executing. Can be null to specify any moment in time is valid.
	 * \param end_stage Stage for which the task must be done executing. Can be null to specify any moment in time is valid.
	 * \return TaskHandle which identifies the task.
	 */
	template<typename T, typename... ACC, typename... FARGS>
	[[nodiscard]] auto RegisterTask(T* caller, const Id taskName, DependencyBlock<ACC...> dependencies, void(T::* delegate)(TaskInfo, FARGS...), const Id start_stage = Id(), const Id end_stage = Id()) {
		//const uint64 fPointer = *reinterpret_cast<uint64*>(&delegate);

		//if (const auto r = functionToTaskMap.TryGet(fPointer)) {
		//	return[&]<typename... ARGS>(void(T:: * d)(TaskInfo, typename ACC::type*..., ARGS...)) { return TaskHandle<ARGS...>(r.Get()()); }(delegate);
		//}
	   
		GTSL::StaticVector<TaskAccess, 16> accesses;

		dependencies.Names[0] = caller->instanceName; dependencies.AccessTypes[0] = AccessTypes::READ_WRITE; // Add a default access to the caller system since we also have to sync access to the caller and we don't expect the user to do so, access is assumed to be read_write

		//assertTask(taskName, {}, )

		{
			GTSL::ReadLock lock(stagesNamesMutex);
			decomposeTaskDescriptor(dependencies.Length + 1, dependencies.Names, dependencies.AccessTypes, accesses);
		}

		uint32 taskIndex = tasks.GetLength(); // Task index in tasks vector

		TaskData& task = tasks.EmplaceBack();

		uint16 startStageIndex = 0xFFFF, endStageIndex = 0xFFFF;

		if (start_stage) { // Store start stage indices if a start stage is specified
			startStageIndex = stagesNames.Find(start_stage).Get();
		}

		if (end_stage) { // Store end stage indices if an end stage is specified
			endStageIndex = stagesNames.Find(end_stage).Get();
		}

		//static_assert(GTSL::TypeAt<0, ARG)

		return[&]<typename... ARGS>(void(T:: * d)(TaskInfo, typename ACC::type*..., ARGS...)) {
			//static_assert((GTSL::IsSame<GTSL::TypeAt<sizeof...(ACC), FARGS>, ARGS>() && ...), "Provided parameter types for task are not compatible with those required.");

			using TDI = TaskDispatchInfo<ARGS...>;

			{
				//TODO: LOCKS!!
				task.Name = GTSL::StringView(taskName); // Store task name, for debugging purposes
				task.StartStageIndex = startStageIndex; // Store start stage index to correctly synchronize task execution, value may be 0xFFFF which indicates that there is no dependency on a stage.
				task.StartStage = static_cast<GTSL::StringView>(start_stage); // Store start stage name for debugging purposes
				task.EndStageIndex = endStageIndex; // Store end stage index to correctly synchronize task execution, value may be 0xFFFF which indicates that there is no dependency on a stage.
				task.EndStage = static_cast<GTSL::StringView>(end_stage); // Store end stage name for debugging purposes
				task.Callee = caller; // Store pointer to system instance which will be called when this task is executed
				task.TaskDispatcher = FunctionType::Create<&ApplicationManager::task<T, TDI, ACC...>>(); // Store dispatcher, an application manager function which manages task state and calls the client delegate.
				task.Access = accesses; // Store task accesses for synchronization and debugging purposes.
				task.SetDelegate(d); // Set client function to be called
			}


			return TaskHandle<ARGS...>(taskIndex);
		} (delegate);
	}

	/**
	 * \brief Schedules a task to run on a periodic basis.
	 * \tparam ARGS
	 */
	template<typename... ARGS>
	auto EnqueueScheduledTask(TaskHandle<ARGS...> task_handle, ARGS&&... args) -> void {
		using TDI = TaskDispatchInfo<ARGS...>;

		TaskData& task = tasks[task_handle()]; //TODO: locks
		task.Scheduled = true;
		allocateTaskDispatchInfo(task, 0, BE::TypeIdentifier(0xFFFF, 0xFFFF), 0xFFFFFFFF, GTSL::ForwardRef<ARGS>(args)...);

		stages[task.StartStageIndex].EmplaceBack(TypeErasedTaskHandle(task_handle()));
	}

	template <int I, class... Ts>
	static decltype(auto) get(Ts&&... ts) {
		return std::get<I>(std::forward_as_tuple(ts...));
	}

	template<typename... ARGS>
	void EnqueueTask(const TaskHandle<ARGS...> task_handle, ARGS&&... args) {
		TaskData& task = tasks[task_handle()]; //TODO: locks
		task.Scheduled = false;

		if constexpr (sizeof...(ARGS)) {
			if constexpr (is_instance<typename GTSL::TypeAt<0, ARGS...>::type, BE::Handle>{}) {
				auto handle = get<0>(args...);
				auto* taskDispatchInfo = allocateTaskDispatchInfo(task, task.Access.front().First, handle.Identifier, handle.EntityIndex, GTSL::ForwardRef<ARGS>(args)...);
			} else {
				allocateTaskDispatchInfo(task, 0, BE::TypeIdentifier(0xFFFF, 0xFFFF), 0xFFFFFFFF, GTSL::ForwardRef<ARGS>(args)...);
			}
		}

		enqueuedTasks.EmplaceBack(TypeErasedTaskHandle(task_handle()));
	}

	void RemoveTask(Id taskName, Id startOn);

	template<typename... ARGS>
	void AddEvent(const Id caller, const EventHandle<ARGS...> eventHandle, bool priority = false) {
		GTSL::WriteLock lock(eventsMutex);
		if constexpr (BE_DEBUG) { if (events.Find(eventHandle.Name)) { BE_LOG_ERROR(u8"An event by the name ", GTSL::StringView(eventHandle.Name), u8" already exists, skipping adition. ", BE::FIX_OR_CRASH_STRING); return; } }
		Event& eventData = events.Emplace(eventHandle.Name, GetPersistentAllocator());

		if (priority) {
			eventData.priorityEntry = 0;
		}
	}

	template<typename... ARGS>
	void SubscribeToEvent(const Id caller, const EventHandle<ARGS...> eventHandle, TaskHandle<ARGS...> taskHandle) {
		GTSL::WriteLock lock(eventsMutex);
		if constexpr (BE_DEBUG) { if (!events.Find(eventHandle.Name)) { BE_LOG_ERROR(u8"No event found by that name, skipping subscription. ", BE::FIX_OR_CRASH_STRING); return; } }
		auto& vector = events.At(eventHandle.Name).Functions;
		vector.EmplaceBack(taskHandle.Reference);
	}

	template<typename... ARGS>
	void DispatchEvent(const Id caller, const EventHandle<ARGS...> eventHandle, ARGS&&... args) {
		GTSL::ReadLock lock(eventsMutex);
		if constexpr (BE_DEBUG) { if (!events.Find(eventHandle.Name)) { BE_LOG_ERROR(u8"No event found by that name, skipping dispatch. ", BE::FIX_OR_CRASH_STRING); return; } }

		Event& eventData = events.At(eventHandle.Name);

		if (eventData.priorityEntry != ~0U) {
			EnqueueTask(TaskHandle<ARGS...>(eventData.Functions[eventData.priorityEntry]()), GTSL::ForwardRef<ARGS>(args)...);
		}
		else {
			auto& functionList = eventData.Functions;
			for (auto e : functionList) { EnqueueTask(TaskHandle<ARGS...>(e()), GTSL::ForwardRef<ARGS>(args)...); }
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

	template<typename T>
	T MakeHandle(BE::TypeIdentifier type_identifier, uint32 index) {
		auto& s = systemsData[type_identifier.SystemId];
		s.RegisteredTypes[type_identifier()].Entities.EmplaceAt(index);
		auto& ent = s.RegisteredTypes[type_identifier()].Entities[index];
		++ent.Uses;

		for (auto& e : s.RegisteredTypes[type_identifier()].VisitingSystems) {
			systemsData[e].RegisteredTypes[type_identifier()].Entities.EmplaceAt(index);
		}

		return T(type_identifier, index);
	}

private:
	GTSL::Vector<GTSL::SmartPointer<World, BE::PersistentAllocatorReference>, BE::PersistentAllocatorReference> worlds;

	mutable GTSL::Mutex systemsMutex;
	GTSL::FixedVector<GTSL::SmartPointer<BE::System, BE::PersistentAllocatorReference>, BE::PersistentAllocatorReference> systems;
	GTSL::FixedVector<Id, BE::PersistentAllocatorReference> systemNames;
	GTSL::HashMap<Id, BE::System*, BE::PersistentAllocatorReference> systemsMap;
	GTSL::HashMap<Id, uint32, BE::PersistentAllocatorReference> systemsIndirectionTable;

	/**
	 * \brief Stores all data necessary to invoke a dispatch.
	 * Resource parameters are stored separately from data parameters because it simplifies accessing DispatchTaskInfo through type erased pointers since we don't need to know what resources the task requires only the data it uses.
	 * Such a use case can be seen with stored tasks, only StoreDynamicTask() can see the tasks full signature but can't allocate a DTI
	 * since every task needs it's own DTI instance which will be allocated when innvoking an stored dynamic task, but since AddStoredDynamicTask doesn't know the full
	 * signature it's easier to have DTIs use just the data parameters since that information is known thanks to the DynamicTaskHandle<ARGS...>.
	 * \tparam ARGS Types of the non resource parameters for a task.
	 */
	template<typename... ARGS>
	struct TaskDispatchInfo {
		TaskDispatchInfo() : TTID(0xFFFF, 0xFFFF), Arguments{ 0 } {}

		template<typename T, typename... FULL_ARGS>
		TaskDispatchInfo(void(T::* function)(TaskInfo, FULL_ARGS...), uint32 sysCount) : ResourceCount(sysCount) {
			static_assert(sizeof(decltype(function)) == 8);
			WriteDelegate<T>(function);
		}

		template<typename T, typename... FULL_ARGS>
		TaskDispatchInfo(void(T::* function)(TaskInfo, FULL_ARGS...), uint32 sysCount, ARGS&&... args) requires static_cast<bool>(sizeof...(ARGS)) : ResourceCount(sysCount) {
			static_assert(sizeof(decltype(function)) == 8);
			WriteDelegate<T>(function);
			UpdateArguments(GTSL::ForwardRef<ARGS>(args)...);
		}

		TaskDispatchInfo(const TaskDispatchInfo&) = delete;
		TaskDispatchInfo(TaskDispatchInfo&&) = delete;

		~TaskDispatchInfo() {
			[&] <uint64... I>(GTSL::Indices<I...>) { (GetPointer<I>()->~GTSL::template GetTypeAt<I, ARGS...>::type(), ...); } (GTSL::BuildIndices<sizeof...(ARGS)>{});

#if BE_DEBUG
			Callee = nullptr;
#endif
		}

		//byte Delegate[8];
		void* Callee;
		uint32 ResourceCount = 0;
		uint16 SystemId;
		BE::TypeIdentifier TTID;
		uint32 InstanceIndex = 0xFFFFFFFF;
		byte Arguments[sizeof(BE::System*) * 8 + GTSL::PackSize<ARGS...>()];

		uint32 D_CallCount = 0;

		//template<class T, typename... FULL_ARGS>
		//void WriteDelegate(void(T::*d)(TaskInfo, FULL_ARGS...)) {
		//	union F {
		//		void(T::* Delegate)(TaskInfo, FULL_ARGS...);
		//	};
		//
		//	reinterpret_cast<F*>(Delegate)->Delegate = d;
		//}

		//void WriteDelegateVoid(byte* buffer) {
		//	for (uint64 i = 0; i < 8; ++i) { Delegate[i] = buffer[i]; }
		//}

		void SetResource(const uint64 pos, BE::System* pointer) { *reinterpret_cast<BE::System**>(Arguments + pos * 8) = pointer; }

		template<uint64 POS, typename T>
		T* GetResource() { return *reinterpret_cast<T**>(Arguments + POS * 8); }

		template<uint64 POS>
		auto GetPointer() { return reinterpret_cast<typename GTSL::TypeAt<POS, ARGS...>::type*>(Arguments + ResourceCount * 8 + GTSL::PackSizeAt<POS, ARGS...>()); }

		template<uint64 POS>
		auto& GetArgument() { return *GetPointer<POS>(); }

		void UpdateArguments(ARGS&&... args) {
			[&] <uint64... I>(GTSL::Indices<I...>) {
				(::new(GetPointer<I>()) ARGS(GTSL::ForwardRef<ARGS>(args)), ...);
			} (GTSL::BuildIndices<sizeof...(ARGS)>{});
		}
	};

	mutable GTSL::ReadWriteMutex eventsMutex;

	struct Event {
		Event(const BE::PAR& allocator) : Functions(allocator) {}

		uint32 priorityEntry = ~0U;
		GTSL::Vector<TypeErasedTaskHandle, BE::PAR> Functions;
	};
	GTSL::HashMap<Id, Event, BE::PersistentAllocatorReference> events;

	// TASKS
	mutable GTSL::ReadWriteMutex tasksMutex;
	struct TaskData {
		/**
		 * \brief Hold a function pointer to a dispatcher function with the signature of the task.
		 */
		FunctionType TaskDispatcher;

		GTSL::StaticVector<TaskAccess, 16> Access;
		void* Callee;
		byte TaskFunction[8];

		uint16 StartStageIndex = 0xFFFF, EndStageIndex = 0xFFFF;

#if BE_DEBUG
		GTSL::StaticString<64> Name, StartStage, EndStage;
#endif

		//Task that has to be called before this
		uint32 Pre = 0xFFFFFFFF;

		bool IsDependedOn = false;

		bool Scheduled = false;

		struct InstanceData {
			uint16 SystemId;
			BE::TypeIdentifier TTID; uint32 InstanceIndex = 0xFFFFFFFF;
			void* TaskInfo;
		};
		GTSL::StaticVector<InstanceData, 8> Instances;

		template<typename F>
		void SetDelegate(F delegate) {
			auto* d = reinterpret_cast<byte*>(&delegate);
			for (uint64 i = 0; i < 8; ++i) {
				TaskFunction[i] = d[i];
			}
		}

		template<class T, typename... ARGS>
		auto GetDelegate() const {
			union F {
				void(T::* Delegate)(TaskInfo, ARGS...);
			};

			return reinterpret_cast<const F*>(TaskFunction)->Delegate;
		}
	};
	GTSL::Vector<TaskData, BE::PAR> tasks;

	GTSL::HashMap<uint64, TypeErasedTaskHandle, BE::PAR> functionToTaskMap;

	GTSL::StaticVector<GTSL::StaticVector<TypeErasedTaskHandle, 16>, 16> stages;

	GTSL::Vector<TypeErasedTaskHandle, BE::PAR> enqueuedTasks;

	template<typename T, typename... RESOURCES, typename... ARGS>
	static void call(TaskInfo task_info, const TaskData* task_data, TaskDispatchInfo<ARGS...>* dispatch_task_info) {
		[&] <uint64... RI, uint64... AI>(GTSL::Indices<RI...>, GTSL::Indices<AI...>) {
			(static_cast<T*>(dispatch_task_info->Callee)->*task_data->GetDelegate<T, RESOURCES*..., ARGS...>())(task_info, dispatch_task_info->GetResource<RI, RESOURCES>()..., GTSL::MoveRef(dispatch_task_info->GetArgument<AI>())...);
		} (GTSL::BuildIndices<sizeof...(RESOURCES)>{}, GTSL::BuildIndices<sizeof...(ARGS)>{});
	}

	// TASKS

	GTSL::ConditionVariable resourcesUpdated;
	GTSL::Atomic<uint32> tasksInFlight;

	mutable GTSL::ReadWriteMutex stagesNamesMutex;
	GTSL::Vector<Id, BE::PersistentAllocatorReference> stagesNames;

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

		//{
		//	GTSL::ReadLock lock(recurringTasksMutex);
		//	
		//	if (recurringTasksPerStage[getStageIndex(startGoal)].DoesTaskExist(taskName)) {
		//		BE_LOG_ERROR(u8"Tried to add task ", GTSL::StringView(taskName), u8" which already exists to stage ", GTSL::StringView(startGoal), u8". Resolve this issue as it leads to undefined behavior in release builds!")
		//		return true;
		//	}
		//}

		{
			GTSL::Lock lock(systemsMutex);

			for (auto i = 0ull; i < len; ++i) {
				if (!doesSystemExist(names[i])) {
					BE_LOG_ERROR(u8"Tried to add task ", GTSL::StringView(taskName), u8" to stage ", GTSL::StringView(startGoal), u8" with a dependency on ", GTSL::StringView(names[i]), u8" which doesn't exist. Resolve this issue as it leads to undefined behavior in release builds!")
						return true;
				}
			}
		}

		return false;
	}

	template<typename... ARGS>
	auto allocateTaskDispatchInfo(TaskData& task, uint16 system_id, BE::TypeIdentifier type_identifier, uint32 instance_index, ARGS&&... args) {
		TaskDispatchInfo<ARGS...>* dispatchTaskInfo = GTSL::New<TaskDispatchInfo<ARGS...>>(GetPersistentAllocator());

		dispatchTaskInfo->ResourceCount = task.Access.GetLength() - 1;
		dispatchTaskInfo->UpdateArguments(GTSL::ForwardRef<ARGS>(args)...);

		for (uint32 i = 1; i < task.Access; ++i) { // Skip first resource because it's the system being called for which we don't send a pointer for
			dispatchTaskInfo->SetResource(i - 1, systems[task.Access[i].First]);
		}

		dispatchTaskInfo->Callee = task.Callee;

		task.Instances.EmplaceBack(system_id, type_identifier, instance_index, dispatchTaskInfo);

		dispatchTaskInfo->SystemId = system_id; dispatchTaskInfo->TTID = type_identifier; dispatchTaskInfo->InstanceIndex = instance_index;

		return dispatchTaskInfo;
	}

	void initWorld(uint8 worldId);

	uint32 getSystemIndex(Id systemName) {
		return systemsIndirectionTable[systemName];
	}

	bool doesSystemExist(const Id systemName) const {
		return systemsIndirectionTable.Find(systemName);
	}

	//struct EntityClassData {
	//	
	//};
	//GTSL::Vector<EntityClassData, BE::PAR> entityClasses;

	struct SystemData {
		SystemData(const BE::PAR& allocator) : RegisteredTypes(allocator) {}

		struct TypeData {
			uint32 Target = 0;
			TypeErasedTaskHandle DeletionTaskHandle;

			bool IsOwn = true;

			struct DependencyData {
				DependencyData(TypeErasedTaskHandle task_handle, bool is_required) : TaskHandle(task_handle), IsReq(is_required) {}

				TypeErasedTaskHandle TaskHandle;
				bool IsReq = true;
			};
			GTSL::StaticVector<DependencyData, 4> SetupSteps;

			struct EntityData {
				uint32 Uses = 0, ResourceCounter = 0;
			};
			GTSL::FixedVector<EntityData, BE::PAR> Entities;

			GTSL::StaticVector<uint16, 8> VisitingSystems;


			TypeData(const BE::PAR& allocator) : Entities(32, allocator) {}
		};
		GTSL::HashMap<uint32, TypeData, BE::PAR> RegisteredTypes;

		uint32 TypeCount = 0; // Owned types count
	};
	GTSL::Vector<SystemData, BE::PAR> systemsData;

public:
	/**
	 * \brief Create a system instance.
	 * \tparam T Class of system.
	 * \param systemName Identifying name for the system instance.
	 * \return A pointer to the created system.
	 */
	template<typename T>
	T* AddSystem(const Id systemName) {
		if constexpr (BE_DEBUG) {
			if (doesSystemExist(systemName)) {
				BE_LOG_ERROR(u8"System by that name already exists! Returning existing instance.", BE::FIX_OR_CRASH_STRING);
				return reinterpret_cast<T*>(systemsMap.At(systemName));
			}
		}

		T* systemPointer = nullptr; uint16 systemIndex = 0xFFFF;

		{
			BE::System::InitializeInfo initializeInfo;
			initializeInfo.ApplicationManager = this;
			initializeInfo.ScalingFactor = scalingFactor;
			initializeInfo.InstanceName = systemName;

			GTSL::Lock lock(systemsMutex);

			//if (!systemsMap.Find(systemName)) {
			systemIndex = systemNames.Emplace(systemName);
			initializeInfo.SystemId = systemIndex;
			systemsIndirectionTable.Emplace(systemName, systemIndex);
			systemsData.EmplaceBack(GetPersistentAllocator());

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
