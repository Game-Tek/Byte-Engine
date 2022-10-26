#pragma once

#include <GTSL/Delegate.hpp>
#include <GTSL/HashMap.hpp>
#include <GTSL/Mutex.h>
#include <GTSL/Vector.hpp>
#include <GTSL/Algorithm.hpp>
#include <GTSL/Allocator.h>
#include <GTSL/SmartPointer.hpp>
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
	TypedDependency(GTSL::StringView name) : Name(name) {}
	TypedDependency(GTSL::StringView name, AccessType at) : Name(name), Access(at) {}

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
	EventHandle(const Id hashed_name) : Name(hashed_name) {}
	EventHandle(const GTSL::StringView name) : Name(name) {}
	Id Name;
};

MAKE_HANDLE(uint32, System)

namespace BE {
	struct TypeIdentifier {
		TypeIdentifier() = delete;
		TypeIdentifier(const uint16 sysid, const uint16 tid) : SystemId(sysid), TypeId(tid) {}

		bool operator==(const TypeIdentifier&) const = default;

		uint16 SystemId = 0xFFFF, TypeId = 0xFFFF;

		uint32 operator()() const { return (SystemId | TypeId << 16); }
	};

	template <size_t L>
	struct fixed_string {
		constexpr fixed_string(const char8_t (&s)[L + 1]) {
			for(uint32 i = 0; i < L; ++i) {
				_chars[i] = s[i];
			}
		}

		const char8_t _chars[L+1] = {}; // +1 for null terminator
	};

	template <size_t N>
	fixed_string(const char8_t (&arr)[N]) -> fixed_string<N-1>;  // Drop the null terminator

	struct BaseHandle {
	protected:
		friend ApplicationManager;
		BaseHandle(uint32 a, uint32 handle) : InstanceIndex(a), EntityIndex(handle) {}
		
		uint32 InstanceIndex = 0xFFFFFFFF, EntityIndex = 0xFFFFFFFF;

	public:
		BaseHandle() = default;

		uint32 operator()() const { return EntityIndex; }

		explicit operator uint64() const { return EntityIndex; }
		explicit operator bool() const { return EntityIndex != 0xFFFFFFFF; }
	};

	template<typename T>
	struct Handle : public BaseHandle {
		Handle() = default;
		Handle(const Handle&) = default;
		//Handle& operator=(const Handle& other) {
		//	Identifier.SystemId = other.Identifier.SystemId;
		//	Identifier.TypeId = other.Identifier.TypeId;
		//	EntityIndex = other.EntityIndex;
		//}
	private:
		friend ApplicationManager;
		Handle(uint32 a, uint32 handle) : BaseHandle(a, handle) {}
	};
	//static_assert(sizeof(Handle<struct RRRR {}> ) <= 8);
}

namespace GTSL {
	template<>
	struct Hash<BE::BaseHandle> {
		uint64 value = 0;
		Hash(const BE::BaseHandle handle) : value(uint64(handle)) {}
		operator uint64() const { return value; }
	};

	Hash(BE::BaseHandle) -> GTSL::Hash<BE::BaseHandle>;
}

#define LSTR(x) u8 ## x

#define MAKE_BE_HANDLE(name)\
	using name##Handle = BE::Handle<struct name##_tag>;

#define BE_RESOURCES(...) __VA_ARGS__
#define DECLARE_BE_TASK(name, res, ...) private: TaskHandle<__VA_ARGS__> name##TaskHandle; using name##Dependencies = DependencyBlock<res>; public: auto Get##name##TaskHandle() const { return name##TaskHandle; }
#define DECLARE_BE_TYPE(name) MAKE_BE_HANDLE(name); private: BE::TypeIdentifier name##TypeIndentifier; public: BE::TypeIdentifier Get##name##TypeIdentifier() const { return name##TypeIndentifier; }
#define DECLARE_BE_EVENT(name, ...) private: EventHandle<__VA_ARGS__> name##EventHandle; public: static auto Get##name##EventHandle() { return EventHandle<__VA_ARGS__>(Id(LSTR(#name))); }

template <class, template <class> class>
struct is_instance : public std::false_type {};

template <class T, template <class> class U>
struct is_instance<U<T>, U> : public std::true_type {};



class ApplicationManager : public Object {
	MAKE_HANDLE(uint32, TypeErasedTask)
	using TypeErasedHandleHandle = BE::BaseHandle;
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
		auto& typeData = systemsData[handle.Identifier.SystemId].Types[handle.Identifier.TypeId];

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
	T* GetSystem(const GTSL::StringView systemName) {
		GTSL::Lock lock(systemsMutex);
		return static_cast<T*>(systemsMap.At(systemName));
	}

	template<class T>
	T* GetSystem(const SystemHandle systemReference) {
		GTSL::Lock lock(systemsMutex);
		return static_cast<T*>(systems[systemReference()].GetData());
	}

	SystemHandle GetSystemReference(const GTSL::StringView systemName) {
		GTSL::Lock lock(systemsMutex);
		return SystemHandle(systemsIndirectionTable.At(systemName));
	}

	BE::TypeIdentifier RegisterType(const BE::System* system, const GTSL::StringView typeName);

	template<typename T>
	void BindDeletionTaskToType(const BE::TypeIdentifier handle, const TaskHandle<T> deletion_task_handle) {
		systemsData[handle.SystemId].Types[handle()].DeletionTaskHandle = TypeErasedTaskHandle(deletion_task_handle.Reference);
	}

	void SpecifyTaskCoDependency(const TypeErasedTaskHandle a, const TypeErasedTaskHandle b) {
		TaskData& taskA = tasks[a()];
		taskA.IsDependedOn = true;
		TaskData& taskB = tasks[b()];
		taskB.Pre = a();
	}

	template<typename... ARGS>
	void AddTypeSetupDependency(BE::System* system_pointer, BE::TypeIdentifier type_identifier, TaskHandle<ARGS...> dynamic_task_handle, bool is_required = true) {
		auto& registeringSystem = systemsData[system_pointer->GetSystemId()];

		if(type_identifier.SystemId != system_pointer->systemId) { // If type is not own, add to observed types
			if(!registeringSystem.ObservedTypes.Find(type_identifier)) {
				registeringSystem.ObservedTypes.EmplaceBack(type_identifier);
			}
		}

		auto& observedSystem = systemsData[type_identifier.SystemId];

		if(!observedSystem.ObservingSystems.Find(SystemHandle(system_pointer->GetSystemId()))) {
			observedSystem.ObservingSystems.EmplaceBack(SystemHandle(system_pointer->GetSystemId()));
		}

		auto& type = registeringSystem.Types.TryEmplace(type_identifier(), GetPersistentAllocator()).Get();

		if(type.SetupSteps) {
			SpecifyTaskCoDependency(type.SetupSteps.back().TaskHandle, TypeErasedTaskHandle(dynamic_task_handle()));
		}

		type.SetupSteps.EmplaceBack(TypeErasedTaskHandle(dynamic_task_handle()), is_required);
		++type.Target;

		TaskData& task = tasks[dynamic_task_handle()];
		task.AssociatedType = type_identifier;
	}

	MAKE_HANDLE(uint32, Resource);

	ResourceHandle AddResource(BE::System* system_pointer, BE::TypeIdentifier type_identifier) {
		auto& system = systemsData[system_pointer->GetSystemId()];
		auto& type = system.Types[type_identifier()];
		type.SetupSteps.EmplaceBack(TypeErasedTaskHandle(), true);
		++type.Target;
		return ResourceHandle(0);
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
			for (uint32 i = 0; i < task.Access; ++i) {
				auto& [name, access] = task.Access[i];
				if(i) { args+=u8", "; }
				args += u8"{ \"System\":\""; args += GTSL::StringView(gameInstance->systemNames[name]); args += u8"\", \"Access type\":\""; args += AccessTypeToString(access); args += u8"\" }";
			}
			args += u8" ]";

			BE::Application::Get()->GetLogger()->logFunction(task.Name, startTime, BE::Application::Get()->GetClock()->GetCurrentMicroseconds(), args);

			if (task.EndStageIndex != 0xFFFF) { gameInstance->semaphores[task.EndStageIndex].Post(); }

			if (info->Signals) {
				GTSL::WriteLock lock(gameInstance->liveInstancesMutex);
				++gameInstance->liveInstances[info->InstanceHandle.InstanceIndex].Counter;
			}

			++info->D_CallCount;

			if (!task.Scheduled) { GTSL::Delete<DTI>(&info, gameInstance->GetPersistentAllocator()); } // KEEP LAST AS THIS ERASES DATA
		}

		--gameInstance->tasksInFlight;
		//gameInstance->resourcesUpdated.NotifyAll();
		gameInstance->resourcesUpdated.Post();
		gameInstance->taskSorter.ReleaseResources(dispatched_task_handle);
	}

	static constexpr uint32 MAX_SYSTEMS = 8u;

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
	[[nodiscard]] auto RegisterTask(T* caller, const GTSL::StringView taskName, DependencyBlock<ACC...> dependencies, void(T::* delegate)(TaskInfo, FARGS...), const GTSL::StringView start_stage = GTSL::StringView(), const GTSL::StringView end_stage = GTSL::StringView()) {
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

		if (start_stage != GTSL::StringView()) { // Store start stage indices if a start stage is specified
			startStageIndex = stagesNames.Find(Id(start_stage)).Get();
		}

		if (end_stage != GTSL::StringView()) { // Store end stage indices if an end stage is specified
			endStageIndex = stagesNames.Find(Id(end_stage)).Get();
		}

		systemsData[caller->GetSystemId()].Tasks.EmplaceBack(taskIndex); // Add task handle to list of system owned tasks

		static_assert(sizeof...(ACC) <= MAX_SYSTEMS);

		return [&]<typename... ARGS>(void(T:: * d)(TaskInfo, typename ACC::type*..., ARGS...)) {
			//static_assert((GTSL::IsSame<GTSL::TypeAt<sizeof...(ACC), FARGS>, ARGS>() && ...), "Provided parameter types for task are not compatible with those required.");

			using TDI = TaskDispatchInfo<ARGS...>;

			if constexpr (sizeof...(ARGS)) {
				if constexpr (is_instance<typename GTSL::TypeAt<0, ARGS...>::type, BE::Handle>{}) {
					task.Signals = true;
				}
			}

			{
				//TODO: LOCKS!!
				task.Name = GTSL::StringView(taskName); // Store task name, for debugging purposes
				task.StartStageIndex = startStageIndex; // Store start stage index to correctly synchronize task execution, value may be 0xFFFF which indicates that there is no dependency on a stage.
				task.StartStage = start_stage; // Store start stage name for debugging purposes
				task.EndStageIndex = endStageIndex; // Store end stage index to correctly synchronize task execution, value may be 0xFFFF which indicates that there is no dependency on a stage.
				task.EndStage = end_stage; // Store end stage name for debugging purposes
				task.Callee = caller; // Store pointer to system instance which will be called when this task is executed
				task.TaskDispatcher = FunctionType::Create<&ApplicationManager::task<T, TDI, ACC...>>(); // Store dispatcher, an application manager function which manages task state and calls the client delegate.
				task.Access = accesses; // Store task accesses for synchronization and debugging purposes.
				task.SetDelegate(d); // Set client function to be called
			}

			return TaskHandle<ARGS...>(taskIndex);
		} (delegate);
	}

	template<typename... ARGS>
	void SetTaskReceiveOnlyLast(const TaskHandle<ARGS...> task_handle) {
		TaskData& task = tasks[task_handle()];
		task.OnlyLast = true;
	}

	/**
	 * \brief Schedules a task to run on a periodic basis.
	 * \tparam ARGS
	 */
	template<typename... ARGS>
	auto EnqueueScheduledTask(TaskHandle<ARGS...> task_handle, ARGS&&... args) -> void {
		using TDI = TaskDispatchInfo<ARGS...>;
		TaskData& task = tasks[task_handle()]; //TODO: locks

		if (stages[task.StartStageIndex].Find(TypeErasedTaskHandle(task_handle()))) {
			BE_LOG_WARNING(u8"Task: ", task.Name, u8" is already scheduled"); return;
		}

		task.Scheduled = true;
		allocateTaskDispatchInfo(task, task.Access.front().First, TypeErasedHandleHandle(), GTSL::ForwardRef<ARGS>(args)...);

		stages[task.StartStageIndex].EmplaceBack(TypeErasedTaskHandle(task_handle()));
	}

	template<typename... ARGS>
	void RemoveScheduledTask(TaskHandle<ARGS...> task_handle) {
		TaskData& task = tasks[task_handle()]; //TODO: locks

		if (!stages[task.StartStageIndex].Find(TypeErasedTaskHandle(task_handle()))) {
			BE_LOG_WARNING(u8"Task: ", task.Name, u8" couldnt't be found"); return;
		}

		task.Instances.PopBack();

		stages[task.StartStageIndex].Pop(stages[task.StartStageIndex].Find(TypeErasedTaskHandle(task_handle())).Get());
	}

	template <int I, class... Ts>
	static decltype(auto) get(Ts&&... ts) {
		return std::get<I>(std::forward_as_tuple(ts...));
	}

	BE::TypeIdentifier getTypeIdentifier(const TypeErasedHandleHandle type_erased_handle_handle) const {
		GTSL::ReadLock mutex{ liveInstancesMutex };
		const auto& e = liveInstances[type_erased_handle_handle.InstanceIndex];
		return { e.SystemID, e.ComponentID };
	}

	template<typename... ARGS>
	void EnqueueTask(const TaskHandle<ARGS...> task_handle, ARGS&&... args) {
		if(!task_handle) { BE_LOG_ERROR(u8"Tried to dispatch task but handle was invalid."); return; }

		TaskData& task = tasks[task_handle()]; //TODO: locks
		task.Scheduled = false;

		if constexpr (sizeof...(ARGS)) {
			if(task.Signals) {
				if constexpr (is_instance<typename GTSL::TypeAt<0, ARGS...>::type, BE::Handle>{}) {
					auto& invokedSystem = systemsData[task.Access.front().First];

					auto typedHandle = get<0>(args...);

					auto handle = makeTypeErasedHandle(typedHandle);

					if(invokedSystem.Types.Find(getTypeIdentifier(typedHandle)())) { // Check if entity identifier is associated with receiving task
					} else {
					}

					allocateTaskDispatchInfo(task, task.Access.front().First, handle, GTSL::ForwardRef<ARGS>(args)...);
				} else {
					allocateTaskDispatchInfo(task, task.Access.front().First, TypeErasedHandleHandle(), GTSL::ForwardRef<ARGS>(args)...);
				}
			} else {
				allocateTaskDispatchInfo(task, task.Access.front().First, TypeErasedHandleHandle(), GTSL::ForwardRef<ARGS>(args)...);
			}
		} else {
			allocateTaskDispatchInfo(task, task.Access.front().First, TypeErasedHandleHandle(), GTSL::ForwardRef<ARGS>(args)...);
		}

		enqueuedTasks.EmplaceBack(TypeErasedTaskHandle(task_handle()));
	}

	void RemoveTask(Id taskName, Id startOn);

	template<typename... ARGS>
	EventHandle<ARGS...> RegisterEvent(const BE::System* caller, const GTSL::StringView event_name, bool priority = false) {
		GTSL::WriteLock lock(eventsMutex);
		if constexpr (BE_DEBUG) { if (events.Find(Id(event_name))) { BE_LOG_ERROR(u8"An event by the name ", event_name, u8" already exists, skipping adition. ", BE::FIX_OR_CRASH_STRING); return EventHandle<ARGS...>(u8""); } }
		Event& eventData = events.Emplace(Id(event_name), GetPersistentAllocator());

		if (priority) {
			eventData.priorityEntry = 0;
		}

		return EventHandle<ARGS...>(event_name);
	}

	template<typename... ARGS>
	void AddEvent(const GTSL::StringView caller, const EventHandle<ARGS...> eventHandle, bool priority = false) {
		GTSL::WriteLock lock(eventsMutex);
		if constexpr (BE_DEBUG) { if (events.Find(eventHandle.Name)) { BE_LOG_ERROR(u8"An event by the name ", GTSL::StringView(eventHandle.Name), u8" already exists, skipping adition. ", BE::FIX_OR_CRASH_STRING); return; } }
		Event& eventData = events.Emplace(eventHandle.Name, GetPersistentAllocator());

		if (priority) {
			eventData.priorityEntry = 0;
		}
	}

	template<typename... ARGS>
	void SubscribeToEvent(const GTSL::StringView caller, const EventHandle<ARGS...> eventHandle, TaskHandle<ARGS...> taskHandle) {
		GTSL::WriteLock lock(eventsMutex);
		if constexpr (BE_DEBUG) { if (!events.Find(eventHandle.Name)) { BE_LOG_ERROR(u8"No event found by that name, skipping subscription. ", BE::FIX_OR_CRASH_STRING); return; } }
		auto& vector = events.At(eventHandle.Name).Functions;
		vector.EmplaceBack(taskHandle.Reference);
	}

	template<typename... ARGS>
	void DispatchEvent(const BE::System* caller, const EventHandle<ARGS...> eventHandle, ARGS&&... args) {
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

	void AddStage(GTSL::StringView stageName);

	template<typename T>
	T MakeHandle(BE::TypeIdentifier type_identifier, uint32 index) {
		GTSL::WriteLock lock(liveInstancesMutex);

		auto instanceIndex = liveInstances.Emplace(type_identifier.SystemId, type_identifier.TypeId);

		auto& s = systemsData[type_identifier.SystemId];
		//s.Types[type_identifier()].Entities.EmplaceAt(instanceIndex, TypeErasedHandleHandle(index, instanceIndex));

		{
			for(auto osh : s.ObservingSystems) {
				auto in = liveInstances.Emplace(type_identifier.SystemId, type_identifier.TypeId);
			}
		}

		return T(instanceIndex, index);
	}

private:
	GTSL::Vector<GTSL::SmartPointer<World, BE::PersistentAllocatorReference>, BE::PersistentAllocatorReference> worlds;

	mutable GTSL::Mutex systemsMutex;
	GTSL::FixedVector<GTSL::SmartPointer<BE::System, BE::PersistentAllocatorReference>, BE::PersistentAllocatorReference> systems;
	GTSL::FixedVector<Id, BE::PersistentAllocatorReference> systemNames;
	GTSL::HashMap<GTSL::StringView, BE::System*, BE::PersistentAllocatorReference> systemsMap;
	GTSL::HashMap<GTSL::StringView, uint32, BE::PersistentAllocatorReference> systemsIndirectionTable;

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
		TaskDispatchInfo() : Arguments{ 0 } {}

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
		
		void* Callee;
		uint32 ResourceCount = 0;
		uint16 SystemId;

		// Index of the entity to signal as being updated by the task.
		bool Signals = false;
		byte Arguments[sizeof(BE::System*) * MAX_SYSTEMS + GTSL::PackSize<ARGS...>()];

		uint32 D_CallCount = 0;

		TypeErasedHandleHandle InstanceHandle;

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

	GTSL::Atomic<uint32> taskCounter{ 0u };

	// TASKS
	mutable GTSL::ReadWriteMutex tasksMutex;
	struct TaskData {
		//TaskData() : TargetType(0xFFFF, 0xFFFF) {}
		TaskData() {}

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
		bool Signals = false;

		TypeErasedTaskHandle Next;
		BE::TypeIdentifier AssociatedType{ 0xFFFF, 0xFFFF };

		struct InstanceData {
			uint32 TaskNumber = 0;

			// System index which the entity to update corresponds to.
			uint16 SystemId;

			// Index of the entity to signal as being updated by the task.
			TypeErasedHandleHandle InstanceHandle;
			void* TaskInfo;

			bool Signals = false;

			uint16 DispatchAttempts = 0;
		};
		GTSL::StaticVector<InstanceData, 16> Instances;

		bool OnlyLast = false;

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

	GTSL::Semaphore resourcesUpdated;
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
	auto allocateTaskDispatchInfo(TaskData& task, uint16 task_owner_system_id, TypeErasedHandleHandle instance_handle, ARGS&&... args) {
		TaskDispatchInfo<ARGS...>* dispatchTaskInfo = GTSL::New<TaskDispatchInfo<ARGS...>>(GetPersistentAllocator());

		dispatchTaskInfo->ResourceCount = task.Access.GetLength() - 1;
		dispatchTaskInfo->UpdateArguments(GTSL::ForwardRef<ARGS>(args)...);

		for (uint32 i = 1; i < task.Access; ++i) { // Skip first resource because it's the system being called for which we don't send a pointer for
			dispatchTaskInfo->SetResource(i - 1, systems[task.Access[i].First]);
		}

		dispatchTaskInfo->Callee = task.Callee;

		const bool signals = task.Signals;

		const uint32 taskNumber = taskCounter++;

		auto h = instance_handle.InstanceIndex;

		if(task.OnlyLast) {
			//TODO: match system index, and everything
			auto taskForSameInstanceExists = task.Instances.LookFor([&](const TaskData::InstanceData& instance_data){ return instance_data.InstanceHandle.InstanceIndex == h; });
			if(taskForSameInstanceExists) {
				TaskData::InstanceData& instance = task.Instances[taskForSameInstanceExists.Get()];
				GTSL::Delete(reinterpret_cast<TaskDispatchInfo<ARGS...>**>(&instance.TaskInfo), GetPersistentAllocator()); // Free overwritten task data
				instance.TaskInfo = dispatchTaskInfo;
				instance.TaskNumber = taskNumber;
			} else {
				if(task_owner_system_id == 0xFFFF) {
					task.Instances.EmplaceBack(taskNumber, task_owner_system_id, instance_handle, dispatchTaskInfo, signals);
				} else {
					task.Instances.EmplaceBack(taskNumber, task_owner_system_id, instance_handle, dispatchTaskInfo, signals);
				}
			}
		} else {
			if(task_owner_system_id == 0xFFFF) {
				task.Instances.EmplaceBack(taskNumber, task_owner_system_id, instance_handle, dispatchTaskInfo, signals);
			} else {
				task.Instances.EmplaceBack(taskNumber, task_owner_system_id, instance_handle, dispatchTaskInfo, signals);
			}
		}

		dispatchTaskInfo->SystemId = task_owner_system_id; dispatchTaskInfo->Signals = signals;
		dispatchTaskInfo->InstanceHandle = instance_handle;

		return dispatchTaskInfo;
	}

	void initWorld(uint8 worldId);

	uint32 getSystemIndex(Id systemName) {
		return systemsIndirectionTable[GTSL::StringView(systemName)];
	}

	bool doesSystemExist(const Id systemName) const {
		return systemsIndirectionTable.Find(GTSL::StringView(systemName));
	}

	//struct EntityClassData {
	//	
	//};
	//GTSL::Vector<EntityClassData, BE::PAR> entityClasses;

	struct SystemData {
		SystemData(const BE::PAR& allocator) : Types(allocator) {}

		struct TypeData {
			uint32 Target = 0;
			TypeErasedTaskHandle DeletionTaskHandle;

			struct DependencyData {
				DependencyData(TypeErasedTaskHandle task_handle, bool is_required) : TaskHandle(task_handle), IsReq(is_required) {}

				TypeErasedTaskHandle TaskHandle;
				bool IsReq = true;
			};
			GTSL::StaticVector<DependencyData, 4> SetupSteps;
			//GTSL::FixedVector<TypeErasedHandleHandle, BE::PAR> Entities;

			TypeData(const BE::PAR& allocator) {}
		};
		GTSL::HashMap<uint32, TypeData, BE::PAR> Types;

		GTSL::StaticVector<BE::TypeIdentifier, 4> ObservedTypes;
		GTSL::StaticVector<SystemHandle, 4> ObservingSystems;

		GTSL::StaticVector<TypeErasedTaskHandle, 32> Tasks;

		uint32 TypeCount = 0; // Owned types count

		Id Name;
	};
	GTSL::Vector<SystemData, BE::PAR> systemsData;

	mutable GTSL::ReadWriteMutex liveInstancesMutex;

	struct InstanceData {
		uint16 SystemID, ComponentID;
		uint32 Counter = 0;
	};
	GTSL::FixedVector<InstanceData, BE::PAR> liveInstances;

	template<typename T>
	static TypeErasedHandleHandle makeTypeErasedHandle(const BE::Handle<T> a) {
		return TypeErasedHandleHandle(a.InstanceIndex, a.EntityIndex);
	}

public:
	/**
	 * \brief Create a system instance.
	 * \tparam T Class of system.
	 * \param systemName Identifying name for the system instance.
	 * \return A pointer to the created system.
	 */
	template<typename T>
	T* AddSystem(const GTSL::StringView systemName) {
		if constexpr (BE_DEBUG) {
			if (doesSystemExist(Id(systemName))) {
				BE_LOG_ERROR(u8"System by that name already exists! Returning existing instance.", BE::FIX_OR_CRASH_STRING);
				return reinterpret_cast<T*>(systemsMap.At(systemName));
			}
		}

		T* systemPointer = nullptr; uint16 systemIndex = 0xFFFF;

		{
			BE::System::InitializeInfo initializeInfo;
			initializeInfo.ApplicationManager = this;
			initializeInfo.ScalingFactor = scalingFactor;
			initializeInfo.InstanceName = Id(systemName);

			{
				GTSL::Lock lock(systemsMutex);

				systemIndex = systemNames.Emplace(systemName);
				initializeInfo.SystemId = systemIndex;
				systemsIndirectionTable.Emplace(systemName, systemIndex);
				auto& systemData = systemsData.EmplaceBack(GetPersistentAllocator());
				systemData.Name = Id(systemName);
			}

			auto systemAllocation = GTSL::SmartPointer<T, BE::PAR>(GetPersistentAllocator(), initializeInfo);
			systemPointer = systemAllocation.GetData();

			{
				GTSL::Lock lock(systemsMutex);

				systems.EmplaceAt(systemIndex, GTSL::MoveRef(systemAllocation));
				taskSorter.AddSystem(Id(systemName));
				systemsMap.Emplace(systemName, systemPointer);
			}
		}

		systemPointer->systemId = systemIndex;
		systemPointer->instanceName = Id(systemName);

		return systemPointer;
	}
};
