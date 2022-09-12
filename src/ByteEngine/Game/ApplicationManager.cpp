#include "ApplicationManager.h"

#include "ByteEngine/Game/World.h"
#include "ByteEngine/Game/System.hpp"

#include "ByteEngine/Debug/FunctionTimer.h"

#include "ByteEngine/Application/ThreadPool.h"
#include "ByteEngine/Application/Application.h"

#include <GTSL/Semaphore.h>

ApplicationManager::ApplicationManager() : Object(u8"ApplicationManager"), worlds(4, GetPersistentAllocator()), systems(8, GetPersistentAllocator()), systemNames(16, GetPersistentAllocator()),
systemsMap(16, GetPersistentAllocator()), systemsIndirectionTable(64, GetPersistentAllocator()), events(32, GetPersistentAllocator()), tasks(128, GetPersistentAllocator()), functionToTaskMap(128, GetPersistentAllocator()), enqueuedTasks(128, GetPersistentAllocator()), tasksInFlight(0u), stagesNames(8, GetPersistentAllocator()), taskSorter(128, GetPersistentAllocator()), systemsData(16, GetPersistentAllocator()), liveInstances(256, GetPersistentAllocator())
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
	struct DDD {
		TypeErasedTaskHandle TaskHandle;
		uint8 RunAttempts = 0;
	};

	using TaskStackType = GTSL::Vector<DDD, BE::TAR>;

	TaskStackType freeTaskStack(GetTransientAllocator());
	GTSL::StaticVector<TaskStackType, 16> perStageTasks; // Holds all tasks which are to be executed

	GTSL::Vector<TypeErasedTaskHandle, BE::TAR> executedTasks(GetTransientAllocator());

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

	// Stores tasks which were not dispatched during this cycle, so must be added back into the enqued tasks list for them to have another shot at running the next cycle. Note, that when loading the free tasks stack the en-queued tasks list is cleared completely.
	GTSL::Vector<TypeErasedTaskHandle, BE::TAR> nonDispatchedTasks(32, GetTransientAllocator());

	// Mutex used to wait until resource availability changes.
	GTSL::Mutex waitWhenNoChange;

	// Round robin counter to ensure all tasks run.
	uint32 rr = 0;

	uint16 stageIndex = 0;

	auto tryDispatchTask = [&](TaskStackType& stack) -> bool {
		const uint32 taskIndex = rr++ % stack.GetLength();
		auto& ddd = stack[taskIndex];
		auto taskHandle = ddd.TaskHandle;
		auto& task = tasks[taskHandle()];

		if(!task.Instances) { stack.Pop(taskIndex); return false; } //todo: instead cull queue and eliminate duplicate entries

		if (const auto result = taskSorter.CanRunTask(task.Access)) {
			uint32 i = 0;

			while (i < task.Instances) {
				auto& taskInstance = task.Instances[i];

				if(taskInstance.Signals) {
					auto& s = systemsData[taskInstance.SystemId];

					uint8 val = 0;

					uint32 l = 0;

					{
						GTSL::ReadLock lock{ liveInstancesMutex };
						auto& e = liveInstances[taskInstance.InstanceHandle.InstanceIndex];
						val = e.Counter;
						l = e.SystemID;
						l |= e.ComponentID << 16;
					}

					auto& t = s.Types[l];

					if (auto r = t.SetupSteps.LookFor([&](const SystemData::TypeData::DependencyData& d) { return taskHandle == d.TaskHandle; }); !r || val < r.Get()) { // If this task can, at this point, execute for this entity type
						++ddd.RunAttempts;
						++taskInstance.DispatchAttempts;

						if(taskInstance.DispatchAttempts > 3) {
							BE_LOG_WARNING(u8"Failed to dispatch ", task.Name, u8", instance: ", taskInstance.TaskNumber, u8". Requires level: ", r.Get(), u8", but has: ", val)
							BE_LOG_WARNING(u8"Task: ", tasks[t.SetupSteps[val].TaskHandle()].Name, u8" is required")
						}

						++i; continue;
					}
				}

				if(task.Pre != 0xFFFFFFFF) {
					if(!executedTasks.Find(TypeErasedTaskHandle(task.Pre))) { // If task which which we depend on executing hasn't yet executed, don't schedule instance.
						++ddd.RunAttempts;
						++taskInstance.DispatchAttempts;

						if(taskInstance.DispatchAttempts > 3) {
							BE_LOG_WARNING(u8"Failed to dispatch ", task.Name, u8", instance: ", taskInstance.TaskNumber)
						}

						++i; continue;
					}
				}

				taskSorter.AddInstance(result.Get(), taskInstance.TaskInfo); // Append task instance to the task sorter's task dispatch packet

				if (!task.Scheduled) {
					task.Instances.Pop(i); // Remove tasks instances which where successfully scheduled for execution.
				} else {
					++i;
				}
			}

			if (!taskSorter.GetValidInstances(result.Get())) {
				taskSorter.ReleaseResources(result.Get());
				if(ddd.RunAttempts > 3) {
					BE_LOG_WARNING(u8"Task: ", task.Name, u8", has failed to run multiple times, removing from stack.");

					nonDispatchedTasks.EmplaceBack(taskHandle);
					stack.Pop(taskIndex); // Remove task from the stack for this cycle, since multiple fails to run can stall the whole pipeline
				}
				return false;
			} // Don't schedule dispatcher execution if no instance was up for execution

			application->GetThreadPool()->EnqueueTask(task.TaskDispatcher, this, GTSL::MoveRef(result.Get()), GTSL::MoveRef(taskHandle)); // Add task dispatcher to thread pool

			++tasksInFlight;
			resourcesUpdated.Add();

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

		if(ddd.RunAttempts > 3) {
			BE_LOG_WARNING(u8"Task: ", task.Name, u8", has failed to run multiple times, removing from stack.");
			nonDispatchedTasks.EmplaceBack(taskHandle);
			stack.Pop(taskIndex); // Remove task from the stack for this cycle, since multiple fails to run can stall the whole pipeline
		}

		return false;
	};

	while(freeTaskStack || stageIndex < perStageTasks.GetLength()) { // While there are elements to be processed
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
			//resourcesUpdated.Wait(waitWhenNoChange);
			resourcesUpdated.Wait();
		}
	}

	for(auto e : nonDispatchedTasks) {
		enqueuedTasks.EmplaceBack(e);
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

BE::TypeIdentifier ApplicationManager::RegisterType(const BE::System* system, const GTSL::StringView type_name) {
	uint16 id = system->systemId;
	uint16 typeId = systemsData[id].TypeCount++;

	systemsData[id].Types.Emplace(BE::TypeIdentifier(id, typeId)(), GetPersistentAllocator());

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

void ApplicationManager::AddStage(GTSL::StringView stageName)
{
	auto hashedName = Id(stageName);

	if constexpr (BE_DEBUG) {
		GTSL::WriteLock lock(stagesNamesMutex);
		if (stagesNames.Find(hashedName).State()) {
			BE_LOG_ERROR(u8"Tried to add stage ", stageName, u8" which already exists. Resolve this issue as it leads to undefined behavior in release builds!")
			return;
		}
	}

	{
		GTSL::WriteLock lock(stagesNamesMutex);
		stagesNames.EmplaceBack(hashedName);
	}

	stages.EmplaceBack();

	BE_LOG_MESSAGE(u8"Added stage ", stageName)
}

void ApplicationManager::initWorld(const uint8 worldId)
{
	World::InitializeInfo initializeInfo;
	initializeInfo.GameInstance = this;
	worlds[worldId]->InitializeWorld(initializeInfo);
}