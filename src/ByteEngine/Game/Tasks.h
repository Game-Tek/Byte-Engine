#pragma once

#include "ByteEngine/Core.h"

#include <GTSL/Vector.hpp>
#include <GTSL/Flags.h>
#include <GTSL/FixedVector.hpp>
#include <GTSL/Result.h>
#include <GTSL/Vector.hpp>

#include "ByteEngine/Id.h"
#include "ByteEngine/Debug/Assert.h"

//enum class AccessType : uint8 { READ = 1, READ_WRITE = 4 };

using AccessType = GTSL::Flags<uint8, struct AccessTypeTag>;
using TaskAccess = GTSL::Pair<uint16, AccessType>;

namespace AccessTypes {
	static constexpr AccessType READ(1), READ_WRITE(4);
}

struct TaskInfo
{
	TaskInfo(ApplicationManager* application_manager) : ApplicationManager(application_manager) {}

	class ApplicationManager* ApplicationManager = nullptr;
	uint8 InvocationID = 0;
};

struct TaskDependency
{
	TaskDependency() = default;
	TaskDependency(const Id object, const AccessType access) : AccessedObject(object), Access(access) {}

	Id AccessedObject;
	AccessType Access;
};

template<typename TASK, class ALLOCATOR>
struct Stage {
	Stage() = default;

	Stage(uint32 num, const ALLOCATOR& allocatorReference) :
	taskAccesses(num, allocatorReference),
	taskGoalIndex(num, allocatorReference),
	taskNames(num, allocatorReference),
	tasksInfos(num, allocatorReference),
	tasks(num, allocatorReference)
	{
	}

	template<class OALLOC>
	Stage(const Stage<TASK, OALLOC>& other, const ALLOCATOR& allocatorReference) :
	taskAccesses(other.taskAccesses.GetLength(), allocatorReference),
	taskGoalIndex(other.taskGoalIndex, allocatorReference),
	taskNames(other.taskNames, allocatorReference),
	tasksInfos(other.tasksInfos, allocatorReference),
	tasks(other.tasks, allocatorReference)
	{
		for(uint32 i = 0; i < other.taskAccesses.GetLength(); ++i) {
			taskAccesses.EmplaceBack(other.taskAccesses[i], allocatorReference);
		}
	}
	
	void AddTask(Id name, TASK task, GTSL::Range<const TaskAccess*> accesses, uint16 goalIndex, void* taskInfo, const ALLOCATOR& allocator)
	{
		auto task_n = taskAccesses.EmplaceBack(16, allocator);
		
		taskAccesses.back().PushBack(accesses);
		
		taskNames.EmplaceBack(name);
		taskGoalIndex.EmplaceBack(goalIndex);
		tasksInfos.EmplaceBack(taskInfo);
		tasks.EmplaceBack(task);
	}

	void RemoveTask(const Id name)
	{
		auto res = taskNames.Find(name);
		BE_ASSERT(res.State(), "No task by that name");

		uint32 i = res.Get();
		
		taskAccesses.Pop(i);
		taskGoalIndex.Pop(i);
		tasksInfos.Pop(i);
		taskNames.Pop(i);
		tasks.Pop(i);
	}

	[[nodiscard]] TASK GetTask(const uint32 i) const { return tasks[i]; }
	[[nodiscard]] void* GetTaskInfo(const uint16 task) const { return tasksInfos[task]; }

	[[nodiscard]] GTSL::Range<const TaskAccess*> GetTaskAccesses(uint16 task) const { return taskAccesses[task]; }

	[[nodiscard]] Id GetTaskName(const uint16 task) const { return taskNames[task]; }

	[[nodiscard]] uint16 GetNumberOfTasks() const { return static_cast<uint16>(tasks.GetLength()); }

	[[nodiscard]] uint16 GetTaskGoalIndex(const uint16 task) const { return taskGoalIndex[task]; }

	void Clear()
	{
		for (auto& e : taskAccesses) { e.Resize(0); }
		taskAccesses.Resize(0);

		taskGoalIndex.Resize(0);

		tasksInfos.Resize(0);
		taskNames.Resize(0);
		tasks.Resize(0);
	}

	bool DoesTaskExist(const Id id) const { return taskNames.Find(id).State(); }
	
	void Pop(const uint32 from, const uint32 range)
	{
		taskAccesses.Pop(from, range);
		taskGoalIndex.Pop(from, range);
		tasksInfos.Pop(from, range);
		taskNames.Pop(from, range);
		tasks.Pop(from, range);
	}

private:
	GTSL::Vector<GTSL::Vector<TaskAccess, ALLOCATOR>, ALLOCATOR> taskAccesses;
	
	GTSL::Vector<uint16, ALLOCATOR> taskGoalIndex;
	
	GTSL::Vector<Id, ALLOCATOR> taskNames;
	GTSL::Vector<void*, ALLOCATOR> tasksInfos;
	GTSL::Vector<TASK, ALLOCATOR> tasks;

	friend struct Stage;
};

template<class ALLOCATOR>
struct TaskSorter
{
	explicit TaskSorter(const uint32 num, const ALLOCATOR& allocator) :
	currentObjectAccessState(num, allocator), currentObjectAccessCount(num, allocator),
	ongoingTasksAccesses(num, allocator), objectNames(num, allocator)
	{
	}

	GTSL::Result<uint32> CanRunTask(const GTSL::Range<const TaskAccess*> accesses) {
		const auto elementCount = accesses.ElementCount();

		uint32 res = 0;

		{
			GTSL::WriteLock lock(mutex);
			
			for (uint32 i = 0; i < elementCount; ++i) {
				if (currentObjectAccessState[accesses[i].First] == AccessTypes::READ_WRITE) { return GTSL::Result<uint32>(false); }
				if (currentObjectAccessState[accesses[i].First] == AccessTypes::READ && accesses[i].Second == AccessTypes::READ_WRITE) { return GTSL::Result<uint32>(false); }
			}
			
			for (uint32 i = 0; i < elementCount; ++i) {
				currentObjectAccessState[accesses[i].First] = accesses[i].Second;
				++currentObjectAccessCount[accesses[i].First];
			}
			
			res = ongoingTasksAccesses.Emplace(accesses);
		}

		return GTSL::Result(GTSL::MoveRef(res), true);
	}

	void ReleaseResources(const uint32 taskIndex) {
		GTSL::WriteLock lock(mutex);

		const auto count = ongoingTasksAccesses[taskIndex].GetLength();
		auto& accesses = ongoingTasksAccesses[taskIndex];
		
		for (uint32 i = 0; i < count; ++i)
		{
			BE_ASSERT(currentObjectAccessCount[accesses[i].First] != 0, "Oops :/");
			if (--currentObjectAccessCount[accesses[i].First] == 0) { //if object is no longer accessed
				currentObjectAccessState[accesses[i].First] = AccessType();
			}
		}
		
		ongoingTasksAccesses.Pop(taskIndex);
	}

	void AddSystem(Id objectName) {
		GTSL::WriteLock lock(mutex);
		objectNames.Emplace(objectName);
		currentObjectAccessState.Emplace(0);
		currentObjectAccessCount.Emplace(0);
	}

private:
	GTSL::FixedVector<AccessType, ALLOCATOR> currentObjectAccessState;
	GTSL::FixedVector<uint16, ALLOCATOR> currentObjectAccessCount;

	GTSL::FixedVector<GTSL::StaticVector<TaskAccess, 64>, ALLOCATOR> ongoingTasksAccesses;

	GTSL::FixedVector<Id, ALLOCATOR> objectNames;

	GTSL::ReadWriteMutex mutex;
};