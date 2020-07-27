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
	
	void GetTaskGoal(const uint16 task, GTSL::Id64& goal) { goal = taskGoals[task]; }
	
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
	currentObjectAccessState(num, allocator), currentObjectAccessCount(num, allocator)
	{
	}

	void CanRunTask(bool& can, const GTSL::Ranger<const uint16>& objects, const GTSL::Ranger<const AccessType>& accesses)
	{
		{
			GTSL::ReadLock lock(mutex);
			
			for (uint32 i = 0; i < objects.ElementCount(); ++i)
			{
				if (currentObjectAccessState[objects[i]] == static_cast<uint8>(AccessType::READ_WRITE)) { can = false; return; }
			}
		}

		{
			GTSL::WriteLock lock(mutex);
			
			for (uint32 i = 0; i < objects.ElementCount(); ++i)
			{
				currentObjectAccessState[objects[i]] = accesses[i];
				++currentObjectAccessCount[i];
			}
		}

		can = true;
	}

	void ReleaseResources(const GTSL::Ranger<const uint16> objects, const GTSL::Ranger<const AccessType> accesses)
	{
		GTSL::WriteLock lock(mutex);
		
		for (uint32 i = 0; i < objects.ElementCount(); ++i)
		{
			BE_ASSERT(currentObjectAccessCount[i] != 0, "Oops :/")
			if (--currentObjectAccessCount[i] == 0) { currentObjectAccessState[i] = 0; }
		}
	}
	
private:
	GTSL::Vector<uint8, ALLOCATOR> currentObjectAccessState;
	GTSL::Vector<uint16, ALLOCATOR> currentObjectAccessCount;

	GTSL::ReadWriteMutex mutex;
};