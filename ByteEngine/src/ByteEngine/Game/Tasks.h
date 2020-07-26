#pragma once

#include "ByteEngine/Core.h"
#include <GTSL/Id.h>
#include <GTSL/Pair.h>
#include <GTSL/Vector.hpp>

#include "ByteEngine/Debug/Assert.h"

enum class AccessType : uint8 { READ = 1, READ_WRITE = 4 };

struct TaskInfo
{
	const class GameInstance* GameInstance = nullptr;
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

	void AddTask(GTSL::Id64 name, TASK task, GTSL::Ranger<const uint16> offsets, const GTSL::Ranger<const AccessType> accessTypes, GTSL::Id64 doneFor, const ALLOCATOR& allocator)
	{
		auto task_n = taskAccessedObjects.EmplaceBack(16, allocator);
		taskAccessTypes.EmplaceBack(16, allocator);
		
		taskAccessedObjects.back().PushBack(offsets);
		taskAccessTypes.back().PushBack(accessTypes);
		
		taskNames.EmplaceBack(name);
		taskGoals.EmplaceBack(doneFor);
		tasks.EmplaceBack(task);
	}

	void RemoveTask(const GTSL::Id64 name)
	{
		auto res = taskNames.Find(name);
		BE_ASSERT(res != taskNames.end(), "No task by that name");

		uint32 i = static_cast<uint32>(res - taskNames.begin());
		
		taskAccessedObjects.Pop(i);
		taskAccessTypes.Pop(i);
		taskGoals.Pop(i);
		taskNames.Pop(i);
		tasks.Pop(i);
	}

	void GetTask(TASK& task, const uint32 i) { task = tasks[i]; }

	void GetTaskAccessedObjects(uint16 task, GTSL::Ranger<const uint16>& accessedObjects) { accessedObjects = taskAccessedObjects[task]; }

	void GetTaskAccessTypes(uint16 task, GTSL::Ranger<const AccessType>& accesses) { accesses = taskAccessTypes[task]; }
	
	void GetNumberOfTasks(uint16& numberOfStacks) { numberOfStacks = tasks.GetLength(); }

private:
	GTSL::Vector<GTSL::Vector<uint16, ALLOCATOR>, ALLOCATOR> taskAccessedObjects;
	GTSL::Vector<GTSL::Vector<AccessType, ALLOCATOR>, ALLOCATOR> taskAccessTypes;
	
	GTSL::Vector<GTSL::Id64, ALLOCATOR> taskGoals;
	GTSL::Vector<GTSL::Id64, ALLOCATOR> taskNames;
	GTSL::Vector<TASK, ALLOCATOR> tasks;
};

template<class ALLOCATOR>
struct TaskSorter
{
	explicit TaskSorter(const uint32 num, const ALLOCATOR& allocator) :
	tasksAccessedObjects(num, allocator), tasksAccessTypes(num, allocator)
	{
	}

	/**
	 * \brief Adds task to be scheduled.
	 * \param num Number of preallocated task properties.
	 * \param allocator Allocator reference for creation of new lists.
	 * \param taskAccessedObjects Indices of the task accessed objects.
	 * \param taskAccessTypes Access types of the tasks.
	 */
	void AddTask(uint32 num, const ALLOCATOR& allocator, 
		const GTSL::Ranger<const uint16> taskAccessedObjects,
		const GTSL::Ranger<const AccessType> taskAccessTypes, uint16 stack)
	{
		tasksAccessedObjects.EmplaceBack(num, allocator);
		tasksAccessTypes.EmplaceBack(num, allocator);
		
		tasksAccessedObjects.back().PushBack(taskAccessedObjects);
		tasksAccessTypes.back().PushBack(taskAccessTypes);

		pendingTasks.Resize(pendingTasks.GetLength() + taskAccessedObjects.ElementCount());
		uint32 i = pendingTasks.GetLength();
		for (auto& e : pendingTasks) { e = i; ++i; }
	}

	void AddObjects(const uint32 i) { currentObjectAccessState.Resize(currentObjectAccessState.GetLength() + i); }
	
	void RemoveTask(const uint32 i)
	{
	}

	void CanRunTask(bool& can, const GTSL::Ranger<const uint16>& objects, const GTSL::Ranger<const AccessType>& accesses)
	{
		for (uint32 i = 0; i < objects.ElementCount(); ++i)
		{
			if (currentObjectAccessState[objects[i]] == AccessType::READ_WRITE) { can = false; return; }
		}
	}
	
	void PopTask(uint32& list, uint32& index)
	{
		uint32 task = pendingTasks.back();

		GTSL::Ranger<const uint16> accessed_objects = tasksAccessedObjects[task];
		GTSL::Ranger<const AccessType> access_types = tasksAccessTypes[task];

		for(uint32 i = 0; i < accessed_objects.ElementCount(); ++i)
		{
			if(access_types[i] == AccessType::READ_WRITE || currentObjectAccessState[accessed_objects[i]] == AccessType::READ_WRITE)
			{
				//goto next task, and put this pending
			}
		}
	}
	
private:
	GTSL::Vector<GTSL::Vector<uint16, ALLOCATOR>, ALLOCATOR> tasksAccessedObjects;
	GTSL::Vector<GTSL::Vector<AccessType, ALLOCATOR>, ALLOCATOR> tasksAccessTypes;

	GTSL::Vector<uint8, ALLOCATOR> currentObjectAccessState;

	GTSL::Vector<uint32, ALLOCATOR> pendingTasks;

	GTSL::Vector<GTSL::Pair<uint16, uint16>, ALLOCATOR> tasks;
};