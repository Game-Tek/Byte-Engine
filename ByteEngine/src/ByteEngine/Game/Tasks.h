#pragma once

#include "ByteEngine/Core.h"

#include <GTSL/Id.h>
#include <GTSL/Vector.hpp>
#include <GTSL/Flags.h>


#include "ByteEngine/Debug/Assert.h"

//enum class AccessType : uint8 { READ = 1, READ_WRITE = 4 };

struct AccessType : GTSL::Flags<uint8>
{	
	static constexpr value_type READ = 1;
	static constexpr value_type READ_WRITE = 4;
};

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

	Goal(uint32 num, const ALLOCATOR& allocatorReference) :
	taskAccessedObjects(num, allocatorReference),
	taskAccessTypes(num, allocatorReference),
	taskGoalIndex(num, allocatorReference),
	taskNames(num, allocatorReference),
	tasks(num, allocatorReference)
	{
	}

	template<class OALLOC>
	Goal(const Goal<TASK, OALLOC>& other, const ALLOCATOR& allocatorReference) :
	taskAccessedObjects(other.taskAccessedObjects.GetCapacity(), allocatorReference),
	taskAccessTypes(other.taskAccessTypes.GetCapacity(), allocatorReference),
	taskGoalIndex(other.taskGoalIndex, allocatorReference),
	taskNames(other.taskNames, allocatorReference),
	tasks(other.tasks, allocatorReference)
	{
		for(uint32 i = 0; i< other.taskAccessedObjects.GetLength(); ++i)
		{
			taskAccessedObjects.EmplaceBack(other.taskAccessedObjects[i], allocatorReference);
			taskAccessTypes.EmplaceBack(other.taskAccessTypes[i], allocatorReference);
		}
	}

	template<class OALLOC>
	Goal& operator=(const Goal<TASK, OALLOC>& other)
	{
		taskAccessedObjects = other.taskAccessedObjects;
		taskAccessTypes = other.taskAccessTypes;
		taskGoalIndex = other.taskGoalIndex;
		taskNames = other.taskNames;
		tasks = other.tasks;
		return *this;
	}
	
	void AddTask(GTSL::Id64 name, TASK task, GTSL::Ranger<const uint16> offsets, const GTSL::Ranger<const AccessType> accessTypes, uint16 goalIndex, const ALLOCATOR& allocator)
	{
		auto task_n = taskAccessedObjects.EmplaceBack(16, allocator);
		taskAccessTypes.EmplaceBack(16, allocator);
		
		taskAccessedObjects.back().PushBack(offsets);
		taskAccessTypes.back().PushBack(accessTypes);
		
		taskNames.EmplaceBack(name);
		taskGoalIndex.EmplaceBack(goalIndex);
		tasks.EmplaceBack(task);
	}

	void RemoveTask(const GTSL::Id64 name)
	{
		auto res = taskNames.Find(name);
		BE_ASSERT(res != taskNames.end(), "No task by that name");

		uint32 i = static_cast<uint32>(res - taskNames.begin());
		
		taskAccessedObjects.Pop(i);
		taskAccessTypes.Pop(i);
		taskGoalIndex.Pop(i);
		taskNames.Pop(i);
		tasks.Pop(i);
	}

	TASK GetTask(const uint32 i) { return tasks[i]; }

	GTSL::Ranger<const uint16> GetTaskAccessedObjects(uint16 task) { return taskAccessedObjects[task]; }

	GTSL::Ranger<const AccessType> GetTaskAccessTypes(uint16 task) { return taskAccessTypes[task]; }
	
	uint16 GetNumberOfTasks() { return (uint16)tasks.GetLength(); }
	
	uint16 GetTaskGoalIndex(const uint16 task) { return taskGoalIndex[task]; }

	void Clear()
	{
		for (auto& e : taskAccessedObjects) { e.ResizeDown(0); }
		taskAccessedObjects.ResizeDown(0);
		for (auto& e : taskAccessTypes) { e.ResizeDown(0); }
		taskAccessTypes.ResizeDown(0);

		taskGoalIndex.ResizeDown(0);

		taskNames.ResizeDown(0);
		tasks.ResizeDown(0);
	}
	
private:
	GTSL::Vector<GTSL::Vector<uint16, ALLOCATOR>, ALLOCATOR> taskAccessedObjects;
	GTSL::Vector<GTSL::Vector<AccessType, ALLOCATOR>, ALLOCATOR> taskAccessTypes;
	
	GTSL::Vector<uint16, ALLOCATOR> taskGoalIndex;
	
	GTSL::Vector<GTSL::Id64, ALLOCATOR> taskNames;
	GTSL::Vector<TASK, ALLOCATOR> tasks;

	friend struct Goal;
};

template<class ALLOCATOR>
struct TaskSorter
{
	explicit TaskSorter(const uint32 num, const ALLOCATOR& allocator) :
	currentObjectAccessState(num, allocator), currentObjectAccessCount(num, allocator)
	{
		currentObjectAccessState.Resize(num);
		for (auto& e : currentObjectAccessState) { e = 0; }
		currentObjectAccessCount.Resize(num);
		for (auto& e : currentObjectAccessCount) { e = 0; }
	}

	bool CanRunTask(const GTSL::Ranger<const uint16>& objects, const GTSL::Ranger<const AccessType>& accesses)
	{
		{
			GTSL::ReadLock lock(mutex);
			
			for (uint32 i = 0; i < objects.ElementCount(); ++i)
			{
				if (currentObjectAccessState[objects[i]] == AccessType::READ_WRITE) { return false; }
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

		return true;
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
	GTSL::Vector<AccessType::value_type, ALLOCATOR> currentObjectAccessState;
	GTSL::Vector<uint16, ALLOCATOR> currentObjectAccessCount;

	GTSL::ReadWriteMutex mutex;
};