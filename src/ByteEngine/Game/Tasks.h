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

namespace AccessTypes {
	static constexpr AccessType READ(1), READ_WRITE(4);
}

struct TaskInfo
{
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
struct Stage
{
	Stage() = default;

	Stage(uint32 num, const ALLOCATOR& allocatorReference) :
	taskAccessedObjects(num, allocatorReference),
	taskAccessTypes(num, allocatorReference),
	taskGoalIndex(num, allocatorReference),
	taskNames(num, allocatorReference),
	tasksInfos(num, allocatorReference),
	tasks(num, allocatorReference)
	{
	}

	template<class OALLOC>
	Stage(const Stage<TASK, OALLOC>& other, const ALLOCATOR& allocatorReference) :
	taskAccessedObjects(other.taskAccessedObjects.GetLength(), allocatorReference),
	taskAccessTypes(other.taskAccessTypes.GetLength(), allocatorReference),
	taskGoalIndex(other.taskGoalIndex, allocatorReference),
	taskNames(other.taskNames, allocatorReference),
	tasksInfos(other.tasksInfos, allocatorReference),
	tasks(other.tasks, allocatorReference)
	{
		for(uint32 i = 0; i < other.taskAccessedObjects.GetLength(); ++i) {
			taskAccessedObjects.EmplaceBack(other.taskAccessedObjects[i], allocatorReference);
			taskAccessTypes.EmplaceBack(other.taskAccessTypes[i], allocatorReference);
		}
	}
	
	void AddTask(Id name, TASK task, GTSL::Range<const uint16*> offsets, const GTSL::Range<const AccessType*> accessTypes, uint16 goalIndex, void* taskInfo, const ALLOCATOR& allocator)
	{
		auto task_n = taskAccessedObjects.EmplaceBack(16, allocator);
		taskAccessTypes.EmplaceBack(16, allocator);
		
		taskAccessedObjects.back().PushBack(offsets);
		taskAccessTypes.back().PushBack(accessTypes);
		
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
		
		taskAccessedObjects.Pop(i);
		taskAccessTypes.Pop(i);
		taskGoalIndex.Pop(i);
		tasksInfos.Pop(i);
		taskNames.Pop(i);
		tasks.Pop(i);
	}

	[[nodiscard]] TASK GetTask(const uint32 i) const { return tasks[i]; }
	[[nodiscard]] void* GetTaskInfo(const uint16 task) const { return tasksInfos[task]; }

	[[nodiscard]] GTSL::Range<const uint16*> GetTaskAccessedObjects(uint16 task) const { return taskAccessedObjects[task]; }

	[[nodiscard]] GTSL::Range<const AccessType*> GetTaskAccessTypes(uint16 task) const { return taskAccessTypes[task]; }

	[[nodiscard]] Id GetTaskName(const uint16 task) const { return taskNames[task]; }

	[[nodiscard]] uint16 GetNumberOfTasks() const { return static_cast<uint16>(tasks.GetLength()); }

	[[nodiscard]] uint16 GetTaskGoalIndex(const uint16 task) const { return taskGoalIndex[task]; }

	void Clear()
	{
		for (auto& e : taskAccessedObjects) { e.Resize(0); }
		taskAccessedObjects.Resize(0);
		for (auto& e : taskAccessTypes) { e.Resize(0); }
		taskAccessTypes.Resize(0);

		taskGoalIndex.Resize(0);

		tasksInfos.Resize(0);
		taskNames.Resize(0);
		tasks.Resize(0);
	}

	bool DoesTaskExist(const Id id) const { return taskNames.Find(id).State(); }
	
	void Pop(const uint32 from, const uint32 range)
	{
		taskAccessedObjects.Pop(from, range);
		taskAccessTypes.Pop(from, range);
		taskGoalIndex.Pop(from, range);
		tasksInfos.Pop(from, range);
		taskNames.Pop(from, range);
		tasks.Pop(from, range);
	}

private:
	GTSL::Vector<GTSL::Vector<uint16, ALLOCATOR>, ALLOCATOR> taskAccessedObjects;
	GTSL::Vector<GTSL::Vector<AccessType, ALLOCATOR>, ALLOCATOR> taskAccessTypes;
	
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
	ongoingTasksAccesses(num, allocator), ongoingTasksObjects(num, allocator), objectNames(num, allocator)
	{
	}

	GTSL::Result<uint32> CanRunTask(const GTSL::Range<const uint16*> objects, const GTSL::Range<const AccessType*> accesses)
	{
		BE_ASSERT(objects.ElementCount() == accesses.ElementCount(), "Bad data, shold be equal");

		const auto elementCount = objects.ElementCount();
		
		{
			GTSL::WriteLock lock(mutex);
			
			for (uint32 i = 0; i < elementCount; ++i)
			{
				if (currentObjectAccessState[objects[i]] == AccessTypes::READ_WRITE) { return GTSL::Result<uint32>(false); }
				if (currentObjectAccessState[objects[i]] == AccessTypes::READ && accesses[i] == AccessTypes::READ_WRITE) { return GTSL::Result<uint32>(false); }
			}
			
			for (uint32 i = 0; i < elementCount; ++i)
			{
				currentObjectAccessState[objects[i]] = accesses[i];
				++currentObjectAccessCount[objects[i]];
			}
			
			auto i = ongoingTasksAccesses.Emplace(accesses);
			auto j = ongoingTasksObjects.Emplace(objects);


			BE_ASSERT(i == j, "Error")
			return GTSL::Result<uint32>(GTSL::MoveRef(i), true);
		}
	}

	void ReleaseResources(const uint32 taskIndex)
	{
		GTSL::WriteLock lock(mutex);

		const auto count = ongoingTasksAccesses[taskIndex].GetLength();
		auto& objects = ongoingTasksObjects[taskIndex];
		auto& accesses = ongoingTasksAccesses[taskIndex];
		
		for (uint32 i = 0; i < count; ++i)
		{
			BE_ASSERT(currentObjectAccessCount[objects[i]] != 0, "Oops :/");
			BE_ASSERT(accesses[i] == AccessTypes::READ || accesses[i] == AccessTypes::READ_WRITE, "Unexpected value");
			if (--currentObjectAccessCount[objects[i]] == 0) { //if object is no longer accessed
				currentObjectAccessState[objects[i]] = AccessType();
			}

		}
		
		ongoingTasksAccesses.Pop(taskIndex); ongoingTasksObjects.Pop(taskIndex);
	}

	void AddSystem(Id objectName)
	{
		GTSL::WriteLock lock(mutex);
		objectNames.Emplace(objectName);
		currentObjectAccessState.Emplace(0);
		currentObjectAccessCount.Emplace(0);
	}

private:
	GTSL::FixedVector<AccessType, ALLOCATOR> currentObjectAccessState;
	GTSL::FixedVector<uint16, ALLOCATOR> currentObjectAccessCount;

	GTSL::FixedVector<GTSL::StaticVector<AccessType, 64>, ALLOCATOR> ongoingTasksAccesses;
	GTSL::FixedVector<GTSL::StaticVector<uint16, 64>, ALLOCATOR> ongoingTasksObjects;

	GTSL::FixedVector<Id, ALLOCATOR> objectNames;

	GTSL::ReadWriteMutex mutex;
};