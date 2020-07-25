#pragma once

#include "ByteEngine/Core.h"
#include <GTSL/Id.h>

#include "ByteEngine/Debug/Assert.h"

enum class AccessType : uint8 { READ = 1, READ_WRITE = 4 };

struct TaskInfo
{
};

struct TaskDependency
{
	GTSL::Id64 AccessedObject;
	AccessType Access;
};

template<typename TASK, class ALLOCATOR>
struct Goal
{
	Goal() = default;

	Goal(uint32 num, const ALLOCATOR& allocatorReference) : taskAccessedObjects(num, allocatorReference), taskAccessTypes(num, allocatorReference),
	taskGoals(num, allocatorReference), taskNames(num, allocatorReference)
	{
	}

	void AddTask(GTSL::Id64 name, TASK task, GTSL::Ranger<const TaskDependency> dependencies, GTSL::Id64 doneFor, const ALLOCATOR& allocator)
	{
		auto task_n = taskAccessedObjects.EmplaceBack(16, allocator);
		taskAccessTypes.EmplaceBack(16, allocator);
		
		for(auto e : dependencies)
		{
			taskAccessedObjects[task_n].EmplaceBack(e.AccessedObject);
			taskAccessTypes[task_n].EmplaceBack(e.Access);
		}
		
		taskNames.EmplaceBack(name);
		taskGoals.EmplaceBack(doneFor);
		tasks.EmplaceBack(task);
	}

	void RemoveTask(const GTSL::Id64 name)
	{
		auto res = taskNames.Find(name);
		BE_ASSERT(res != taskNames.end(), "No task by that name");

		uint32 i = res - taskNames.begin();
		
		taskAccessedObjects.Pop(i);
		taskAccessTypes.Pop(i);
		taskGoals.Pop(i);
		taskNames.Pop(i);
		tasks.Pop(i);
	}

private:
	GTSL::Vector<GTSL::Vector<GTSL::Id64, ALLOCATOR>, ALLOCATOR> taskAccessedObjects;
	GTSL::Vector<GTSL::Vector<AccessType, ALLOCATOR>, ALLOCATOR> taskAccessTypes;
	GTSL::Vector<GTSL::Vector<GTSL::Id64, ALLOCATOR>, ALLOCATOR> taskGoals;
	GTSL::Vector<GTSL::Id64, ALLOCATOR> taskNames;
	GTSL::Vector<TASK, ALLOCATOR> tasks;
};

template<class ALLOCATOR>
struct ParallelTasks
{
	explicit ParallelTasks(const BE::PersistentAllocatorReference& allocatorReference) : names(8, allocatorReference), taskDependencies(8, allocatorReference),
		tasks(8, allocatorReference)
	{
	}

	void AddTask(GTSL::Id64 name, const GTSL::Ranger<const TaskDependency> taskDescriptors, TaskType delegate)
	{
		names.EmplaceBack(name); taskDependencies.PushBack(taskDescriptors); tasks.EmplaceBack(delegate);
	}

	void RemoveTask(const uint32 i)
	{
		taskDependencies.Pop(i); tasks.Pop(i); names.Pop(i);
	}

	TaskType& operator[](const uint32 i) { return tasks[i]; }

	[[nodiscard]] GTSL::Ranger<TaskType> GetTasks() const { return tasks; }
	[[nodiscard]] GTSL::Ranger<GTSL::Id64> GetTaskNames() const { return names; }
	[[nodiscard]] GTSL::Ranger<TaskDependency> GetTaskDescriptors() const { return taskDependencies; }

	[[nodiscard]] const TaskType* begin() const { return tasks.begin(); }
	[[nodiscard]] const TaskType* end() const { return tasks.end(); }

private:
	GTSL::Vector<TaskDependency, BE::PersistentAllocatorReference> taskDependencies;
};