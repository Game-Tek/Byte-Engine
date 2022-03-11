#pragma once

#include <GTSL/Delegate.hpp>
#include <GTSL/HashMap.hpp>
#include <GTSL/Mutex.h>
#include <GTSL/Vector.hpp>
#include <GTSL/Algorithm.hpp>
#include <GTSL/Allocator.h>
#include <GTSL/Semaphore.h>

#include "Tasks.h"
#include "ByteEngine/Id.h"

#include "ByteEngine/Debug/Assert.h"

#include "ByteEngine/Handle.hpp"
#include "ByteEngine/Debug/FunctionTimer.h"

#include "ByteEngine/Application/Handle.hpp"

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
struct Resources{};

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

//#define Q(x) #x
//#define MAKE_TASK(am, className, functionName, startStage, endStage, dependencies, ...) am->AddTask(this, u8 Q(functionName), &className::functionName, dependencies, startStage, endStage, __VA_ARGS__)

#include "ByteEngine/Application/Application.h"

namespace BE {
	struct TypeIdentifer {
		uint16 SystemId = 0xFFFF, TypeId = 0xFFFF;
	};

	template<typename T>
	struct Handle {
		Handle() = default;
		Handle(TypeIdentifer type_identifier, uint32 handle) : Identifier(type_identifier), EntityIndex(handle) {}
		Handle(const Handle&) = default;
		//Handle& operator=(const Handle& other) {
		//	Identifier.SystemId = other.Identifier.SystemId;
		//	Identifier.TypeId = other.Identifier.TypeId;
		//	EntityIndex = other.EntityIndex;
		//}

		uint32 operator()() const { return EntityIndex; }

		explicit operator uint64() const { return EntityIndex; }
		explicit operator bool() const { return EntityIndex != 0xFFFFFFFF; }

		TypeIdentifer Identifier;
		uint32 EntityIndex = 0xFFFFFFFF;
	};

	//static_assert(sizeof(Handle<struct RRRR {}> ) <= 8);

#define MAKE_BE_HANDLE(name)\
	using name##Handle = BE::Handle<struct name##_tag>;
}

class ApplicationManager : public Object {
	using FunctionType = GTSL::Delegate<void(ApplicationManager*, DispatchedTaskHandle, void*)>;
	MAKE_HANDLE(uint32, TypeErasedTask)
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
			if (typeData.DeletionTaskHandle != ~0U) { //if we have a valid deletion handle
				//AddStoredDynamicTask(DynamicTaskHandle<GTSL::Range<const T*>>(typeData.DeletionTaskHandle));
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

	BE::TypeIdentifer RegisterType(const BE::System* system, const GTSL::StringView typeName);

	template<typename... ARGS>
	void BindTaskToType(const BE::TypeIdentifer type_identifer, const TaskHandle<ARGS...> handle) {
		systemsData[type_identifer.SystemId].RegisteredTypes[type_identifer.TypeId].Target += 1;
	}

	template<typename T>
	void BindDeletionTaskToType(const BE::TypeIdentifer handle, const TaskHandle<GTSL::Range<const T*>> deletion_task_handle) {
		systemsData[handle.SystemId].RegisteredTypes[handle.TypeId].DeletionTaskHandle = deletion_task_handle.Reference;
	}

	template<typename... ARGS1, typename... ARGS2>
	void SpecifyTaskCoDependency(const TaskHandle<ARGS1...> a, const TaskHandle<ARGS2...> b) {
		TaskData& taskB = tasks[b()];
		taskB.Pre = a();
	}

	template<typename... ARGS>
	void AddTypeSetupDependency(BE::TypeIdentifer type_identifer, TaskHandle<ARGS...> dynamic_task_handle) {
		++systemsData[type_identifer.SystemId].RegisteredTypes[type_identifer.TypeId].Target;
	}

	template<typename DTI, typename T, typename... ACC>
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
		if (info->InstanceIndex != 0xFFFFFFFF) { ++gameInstance->systemsData[info->SystemId].RegisteredTypes[info->EntityId].Entities[info->InstanceIndex].ResourceCounter; }
		if (info->Scheduled) { GTSL::Delete<DTI>(&info, gameInstance->GetPersistentAllocator()); }
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
		const uint64 fPointer = *reinterpret_cast<uint64*>(&delegate);

		if (const auto r = functionToTaskMap.TryGet(fPointer)) {
			return [&]<typename... ARGS>(void(T::*d)(TaskInfo, typename ACC::type*..., ARGS...)) { return TaskHandle<ARGS...>(r.Get()()); }(delegate);
		}

		GTSL::StaticVector<TaskAccess, 32> accesses;

		dependencies.Names[0] = caller->instanceName; dependencies.AccessTypes[0] = AccessTypes::READ_WRITE; // Add a default access to the caller system since we also have to sync access to the caller and we don't expect the user to do so, access is assumed to be read_write

		//assertTask(taskName, {}, )

		{
			GTSL::ReadLock lock(stagesNamesMutex);
			decomposeTaskDescriptor(dependencies.Length + 1, dependencies.Names, dependencies.AccessTypes, accesses);
		}

		uint32 taskIndex = tasks.GetLength(); // Task index in tasks vector

		TaskData& task = tasks.EmplaceBack();

		uint16 startStageIndex = 0xFFFF, endStageIndex = 0xFFFF;

		if(start_stage) { // Store start stage indices if a start stage is specified
			startStageIndex = stagesNames.Find(start_stage).Get();
		}

		if(end_stage) { // Store end stage indices if an end stage is specified
			endStageIndex = stagesNames.Find(end_stage).Get();
		}

		return [&]<typename... ARGS>(void(T::*d)(TaskInfo, typename ACC::type*..., ARGS...)) {
			//static_assert((GTSL::IsSame<typename ACC::type*, FARGS>() && ...), "Provided parameter types for task are not compatible with those required.");

			using TDI = TaskDispatchInfo<ARGS...>;

			{
				//TODO: LOCKS!!
				task.Name = taskName; // Store task name, for debugging purposes
				task.StartStageIndex = startStageIndex; // Store start stage index to correctly synchronize task execution, value may be 0xFFFF which indicates that there is no dependency on a stage.
				task.StartStage = static_cast<GTSL::StringView>(start_stage); // Store start stage name for debugging purposes
				task.EndStageIndex = endStageIndex; // Store end stage index to correctly synchronize task execution, value may be 0xFFFF which indicates that there is no dependency on a stage.
				task.EndStage = static_cast<GTSL::StringView>(end_stage); // Store end stage name for debugging purposes
				task.Callee = caller; // Store pointer to system instance which will be called when this task is executed
				task.TaskDispatcher = FunctionType::Create<&ApplicationManager::task<TDI, T, ACC...>>(); // Store dispatcher, an application manager function which manages task state and calls the client delegate.
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

		TDI* dispatchTaskInfo = GTSL::New<TDI>(GetPersistentAllocator());

		dispatchTaskInfo->ResourceCount = task.Access.GetLength();
		dispatchTaskInfo->UpdateArguments(GTSL::ForwardRef<ARGS>(args)...);
		dispatchTaskInfo->WriteDelegateVoid(task.TaskFunction);

		uint16 startStageIndex = 0xFFFF, endStageIndex = 0xFFFF;

		if constexpr (BE_DEBUG) {
			dispatchTaskInfo->Name = GTSL::StringView(task.Name);
			dispatchTaskInfo->StartStage = GTSL::StringView(task.StartStage); dispatchTaskInfo->EndStage = GTSL::StringView(task.EndStage);
			for (uint32 i = 0; i < task.Access; ++i) { dispatchTaskInfo->Accesses.EmplaceBack(GTSL::StringView(systemNames[task.Access[i].First]), task.Access[i].Second); }
		}

		for (uint32 i = 1; i < task.Access; ++i) { // Skip first resource because it's the system being called for which we don't send a pointer for
			dispatchTaskInfo->SetResource(i, systems[task.Access[i].First]);
		}

		dispatchTaskInfo->Callee = task.Callee;
		dispatchTaskInfo->startStageIndex = startStageIndex;
		dispatchTaskInfo->endStageIndex = endStageIndex;

		task.Scheduled = true;
	}
	
	void RemoveTask(Id taskName, Id startOn);

	template<typename T, typename... ARGS>
	void CallTaskOnEntity(const TaskHandle<BE::Handle<T>, ARGS...> taskHandle, const BE::Handle<T> handle, ARGS&&... args) {
		
	}

	template<typename... ARGS>
	void EnqueueTask(const TaskHandle<ARGS...> task_handle, ARGS&&... args) {
		//TODO: enqueu task
	}

	template<typename... ARGS>
	void AddEvent(const Id caller, const EventHandle<ARGS...> eventHandle, bool priority = false) {
		GTSL::WriteLock lock(eventsMutex);
		if constexpr (BE_DEBUG) { if (events.Find(eventHandle.Name)) { BE_LOG_ERROR(u8"An event by the name ", GTSL::StringView(eventHandle.Name), u8" already exists, skipping adition. ", BE::FIX_OR_CRASH_STRING); return; } }
		Event& eventData = events.Emplace(eventHandle.Name, GetPersistentAllocator());

		if(priority) {
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

		if(eventData.priorityEntry != ~0U) {
			EnqueueTask(TaskHandle<ARGS...>(eventData.Functions[eventData.priorityEntry]()), GTSL::ForwardRef<ARGS>(args)...);
		} else {
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
	T MakeHandle(BE::TypeIdentifer type_identifer, uint32 index) {
		auto& s = systemsData[type_identifer.SystemId];
		auto entI = s.RegisteredTypes[type_identifer.TypeId].Entities.Emplace();
		auto& ent = s.RegisteredTypes[type_identifer.TypeId].Entities[entI];
		++ent.Uses;
		return T(type_identifer, index);
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
		TaskDispatchInfo() : Arguments{ 0 } {}

		template<typename T, typename... FULL_ARGS>
		TaskDispatchInfo(void(T::*function)(TaskInfo, FULL_ARGS...), uint32 sysCount) : ResourceCount(sysCount) {
			static_assert(sizeof(decltype(function)) == 8);
			WriteDelegate<T>(function);
		}

		template<typename T, typename... FULL_ARGS>
		TaskDispatchInfo(void(T::*function)(TaskInfo, FULL_ARGS...), uint32 sysCount, ARGS&&... args) requires static_cast<bool>(sizeof...(ARGS)) : ResourceCount(sysCount) {
			static_assert(sizeof(decltype(function)) == 8);
			WriteDelegate<T>(function);
			UpdateArguments(GTSL::ForwardRef<ARGS>(args)...);
		}

		TaskDispatchInfo(const TaskDispatchInfo&) = delete;
		TaskDispatchInfo(TaskDispatchInfo&&) = delete;

		~TaskDispatchInfo() {
			[&]<uint64... I>(GTSL::Indices<I...>) { (GetPointer<I>()->~GTSL::template GetTypeAt<I, ARGS...>::type(), ...); } (GTSL::BuildIndices<sizeof...(ARGS)>{});

#if BE_DEBUG
				Name = u8"deleted";
				Callee = nullptr;
#endif
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
		byte Arguments[sizeof(BE::System*) * 8 + GTSL::PackSize<ARGS...>()];

		bool Scheduled; // Whether this task is scheduled
		uint16 SystemId, EntityId; uint32 InstanceIndex;

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

		void SetResource(const uint64 pos, BE::System* pointer) { *reinterpret_cast<BE::System**>(Arguments + pos * 8) = pointer; }

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
	static void call(T* whoToCall, TaskInfo task_info, TaskDispatchInfo<ARGS...>* dispatch_task_info) {
		[&] <uint64... RI, uint64... AI>(GTSL::Indices<RI...>, GTSL::Indices<AI...>) {
			(whoToCall->*dispatch_task_info->GetDelegate<T, RS...>())(task_info, dispatch_task_info->GetResource<RI, RS>()..., GTSL::MoveRef(dispatch_task_info->GetArgument<AI>())...);
		} (GTSL::BuildIndices<sizeof...(RS)>{}, GTSL::BuildIndices<sizeof...(ARGS)>{});
	}

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

		Id Name;
		GTSL::StaticVector<TaskAccess, 32> Access;
		void* Callee;
		byte TaskFunction[8];

		uint16 StartStageIndex = 0xFFFF, EndStageIndex = 0xFFFF;
		GTSL::StaticString<64> StartStage, EndStage;

		//Task that has to be called before this
		uint32 Pre = 0xFFFFFFFF;

		bool Scheduled = false;

		template<typename F>
		void SetDelegate(F delegate) {
			auto* d = reinterpret_cast<byte*>(&delegate);
			for (uint64 i = 0; i < 8; ++i) {
				TaskFunction[i] = d[i];
			}
		}
	};
	GTSL::Vector<TaskData, BE::PAR> tasks;

	GTSL::HashMap<uint64, TypeErasedTaskHandle, BE::PAR> functionToTaskMap;

	// TASKS

	GTSL::ConditionVariable resourcesUpdated;
	
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

	//struct EntityClassData {
	//	
	//};
	//GTSL::Vector<EntityClassData, BE::PAR> entityClasses;

	struct SystemData {
		struct TypeData {
			uint32 Target = 0;
			uint32 DeletionTaskHandle = ~0U;

			struct EntityData {
				uint32 Uses = 0, ResourceCounter = 0;
			};
			GTSL::FixedVector<EntityData, BE::PAR> Entities;

			TypeData(const BE::PAR& allocator) : Entities(32, allocator) {}
		};
		GTSL::StaticVector<TypeData, 32> RegisteredTypes;
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
				systemsData.EmplaceBack();

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
