#include "ApplicationManager.h"

#include "ByteEngine/Game/World.h"
#include "ByteEngine/Game/System.hpp"

#include "ByteEngine/Debug/FunctionTimer.h"

#include "ByteEngine/Application/ThreadPool.h"
#include "ByteEngine/Application/Application.h"

#include <GTSL/Semaphore.h>

ApplicationManager::ApplicationManager() : Object(u8"ApplicationManager"), worlds(4, GetPersistentAllocator()), systems(8, GetPersistentAllocator()), systemNames(16, GetPersistentAllocator()),
systemsMap(16, GetPersistentAllocator()), systemsIndirectionTable(64, GetPersistentAllocator()), events(32, GetPersistentAllocator()), tasks(128, GetPersistentAllocator()), stagesNames(8, GetPersistentAllocator()), taskSorter(128, GetPersistentAllocator()), systemsData(16, GetPersistentAllocator()), functionToTaskMap(128, GetPersistentAllocator()), enqueuedTasks(128, GetPersistentAllocator()), tasksInFlight(0u)
{
}

ApplicationManager::~ApplicationManager() {
	{
		//Call shutdown in reverse order since systems initialized last during application start
		//may depend on those created before them also for shutdown
		auto shutdownSystem = [&](GTSL::SmartPointer<BE::System, BE::PAR>& system) -> void {
			system.TryFree();
		};
		
		GTSL::ReverseForEach(systems, shutdownSystem);
	}
		
	World::DestroyInfo destroy_info;
	destroy_info.GameInstance = this;
	for (auto& world : worlds) { world->DestroyWorld(destroy_info); }
}

void ApplicationManager::OnUpdate(BE::Application* application) {
	using TaskStackType = GTSL::Vector<TypeErasedTaskHandle, BE::TAR>;
	TaskStackType freeTaskStack(64, GetTransientAllocator());
	GTSL::StaticVector<TaskStackType, 16> perStageTasks; // Holds all tasks which are to be executed

	TaskStackType executedTasks(64, GetTransientAllocator());

	GTSL::Vector<uint32, BE::TAR> perStageCounter(32, GetTransientAllocator()); // Maintains the count of how many tasks were executed for each stage. It's used to know when an stage can advance.

	for(uint32 i = 0; i < stages; ++i) { // Loads all recurrent task onto the stack
		perStageTasks.EmplaceBack(16, GetTransientAllocator());

		for(uint32 j = 0; j < stages[i]; ++j) {
			perStageTasks.back().EmplaceBack(stages[i][j]);
		}

		perStageCounter.EmplaceBack(0);
	}

	{
		for (uint32 i = 0; i < enqueuedTasks; ++i) {
			freeTaskStack.EmplaceBack(enqueuedTasks[i]);
		}

		enqueuedTasks.Resize(0); // Clear enqueued tasks list after processing it
	}

	// Mutex used to wait until resource availability changes.
	GTSL::Mutex waitWhenNoChange;

	// Round robin counter to ensure all tasks run.
	uint32 rr = 0;

	uint16 stageIndex = 0;

	auto tryDispatchTask = [&](TaskStackType& stack) -> bool {
		const uint32 taskIndex = rr++ % stack.GetLength();
		auto taskHandle = stack[taskIndex];
		auto& task = tasks[taskHandle()];

		if(!task.Instances) { stack.Pop(taskIndex); return false; } //todo: instead cull queue and eliminate duplicate entries

		if (const auto result = taskSorter.CanRunTask(task.Access)) {
			uint32 i = 0;

			while (i < task.Instances) {
				auto& instance = task.Instances[i];

				if(instance.InstanceIndex != 0xFFFFFFFF) { // Is executable instance
					auto& s = systemsData[instance.SystemId];
					auto& t = s.RegisteredTypes[instance.EntityId];
					auto& entt = t.Entities[instance.InstanceIndex];

					if (auto r = t.SetupSteps.LookFor([&](const SystemData::TypeData::DependencyData& d) { return taskHandle == d.TaskHandle; }); !r || r.Get() != entt.ResourceCounter) { // If this task can, at this point, execute for this entity type
						++i; continue;
					}
				}

				if(task.Pre != 0xFFFFFFFF) {
					if(!executedTasks.Find(TypeErasedTaskHandle(task.Pre))) { // If task which which we depend on executing hasn't yet executed, don't schedule instance.
						++i; continue;
					}
				}

				taskSorter.AddInstance(result.Get(), instance.TaskInfo); // Append task instance to the task sorter's task dispatch packet
				task.Instances.Pop(i); // Remove tasks instances which where successfully scheduled for execution.
			}

			if (!taskSorter.GetValidInstances(result.Get())) {
				taskSorter.ReleaseResources(result.Get()); return false;
			} // Don't schedule dispatcher execution if no instance was up for execution

			application->GetThreadPool()->EnqueueTask(task.TaskDispatcher, this, GTSL::MoveRef(result.Get()), GTSL::MoveRef(taskHandle)); // Add task dispatcher to thread pool

			++tasksInFlight;

			if(task.IsDependedOn) {
				executedTasks.EmplaceBack(taskHandle);
			}

			const uint16 targetStageIndex = task.EndStageIndex;

			if (targetStageIndex != 0xFFFF) {
				semaphores[targetStageIndex].Add();
				++perStageCounter[targetStageIndex];
			}

			stack.Pop(taskIndex); // If task was executed remove from stack.

			return true;
		}

		return false;
	};

	while(freeTaskStack || (stageIndex < perStageTasks.GetLength()) && perStageTasks[stageIndex]) { // While there are elements to be processed
		while (stageIndex < perStageTasks.GetLength() && perStageTasks[stageIndex]) {
			semaphores[stageIndex].Wait();

			if(!tryDispatchTask(perStageTasks[stageIndex])) {
				break;
			}
		}

		if (stageIndex < perStageTasks.GetLength() && !perStageTasks[stageIndex]) { // If stage can be changed
			++stageIndex;
			//getLogger()->InstantEvent(GTSL::StringView(stagesNames[stageIndex]), application->GetClock()->GetCurrentMicroseconds().GetCount()); //TODO: USE LOCK ON STAGE NAME					
		}

		while (freeTaskStack) {
			if (!tryDispatchTask(freeTaskStack)) {
				break;
			}
		}

		if (tasksInFlight) { // If there are task enqueued on the thread pool wait until a change in resource availability occurs to continue trying to dispatch tasks. Don't wait without checking if there are tasks left, because that will leave the thread waiting indefinitely since there are no tasks to signal the condition.
			resourcesUpdated.Wait(waitWhenNoChange);
		}
	}

	++frameNumber;
}

void ApplicationManager::UnloadWorld(const WorldReference worldId)
{
	World::DestroyInfo destroy_info;
	destroy_info.GameInstance = this;
	worlds[worldId]->DestroyWorld(destroy_info);
	worlds.Pop(worldId);
}

BE::TypeIdentifer ApplicationManager::RegisterType(const BE::System* system, const GTSL::StringView type_name) {
	uint16 id = system->systemId;
	uint16 typeId = systemsData[id].RegisteredTypes.GetLength();

	systemsData[id].RegisteredTypes.EmplaceBack(GetPersistentAllocator());

	return { id, typeId };
}

void ApplicationManager::RemoveTask(const Id taskName, const Id startOn) {
	uint16 i = 0;

	if constexpr (BE_DEBUG) {
		GTSL::ReadLock lock(stagesNamesMutex);
		
		if(!stagesNames.Find(startOn).State()) {
			BE_LOG_ERROR(u8"Tried to remove task ", GTSL::StringView(taskName), u8" from stage ", GTSL::StringView(startOn), u8" which doesn't exist. Resolve this issue as it leads to undefined behavior in release builds!")
			return;
		}

		i = getStageIndex(startOn);
	}
	
	{
		GTSL::ReadLock lock(stagesNamesMutex);
		i = getStageIndex(startOn);
	}

	BE_LOG_MESSAGE(u8"Removed recurring task ", GTSL::StringView(taskName), u8" from stage ", GTSL::StringView(startOn))
}

void ApplicationManager::AddStage(Id stageName)
{
	if constexpr (BE_DEBUG) {
		GTSL::WriteLock lock(stagesNamesMutex);
		if (stagesNames.Find(stageName).State()) {
			BE_LOG_ERROR(u8"Tried to add stage ", GTSL::StringView(stageName), u8" which already exists. Resolve this issue as it leads to undefined behavior in release builds!")
			return;
		}
	}

	{
		GTSL::WriteLock lock(stagesNamesMutex);
		stagesNames.EmplaceBack(stageName);
	}

	stages.EmplaceBack();

	BE_LOG_MESSAGE(u8"Added stage ", GTSL::StringView(stageName))
}

void ApplicationManager::initWorld(const uint8 worldId)
{
	World::InitializeInfo initializeInfo;
	initializeInfo.GameInstance = this;
	worlds[worldId]->InitializeWorld(initializeInfo);
}