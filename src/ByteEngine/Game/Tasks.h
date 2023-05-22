#pragma once

#include "ByteEngine/Core.h"
#include <GTSL/Vector.hpp>
#include <GTSL/Flags.h>
#include <GTSL/FixedVector.hpp>
#include <GTSL/Result.h>
#include "ByteEngine/Id.h"
#include "ByteEngine/Debug/Assert.h"
#include "ByteEngine/Handle.hpp"

class ApplicationManager;
using AccessType = GTSL::Flags<GTSL::uint8, struct AccessTypeTag>;
using TaskAccess = GTSL::Pair<GTSL::uint16, AccessType>;

namespace AccessTypes {
	static constexpr AccessType READ(1), READ_WRITE(4);
}

struct TaskInfo {
	TaskInfo(ApplicationManager* application_manager) : AppManager(application_manager) {}

	ApplicationManager* AppManager = nullptr;
	GTSL::uint8 InvocationID = 0;
};

struct TaskDependency
{
	TaskDependency() = default;
	TaskDependency(const GTSL::StringView object, const AccessType access) : AccessedObject(object), Access(access) {}
	TaskDependency(const Id object, const AccessType access) : AccessedObject(object), Access(access) {}

	Id AccessedObject;
	AccessType Access;
};

MAKE_HANDLE(GTSL::uint32, DispatchedTask);

template<class ALLOCATOR>
struct TaskSorter {
	explicit TaskSorter(const GTSL::uint32 num, const ALLOCATOR& allocator) :
		currentObjectAccessState(num, allocator), currentObjectAccessCount(num, allocator),
		ongoingTasksAccesses(num, allocator), instances(num, allocator)
	{
	}

	GTSL::Result<DispatchedTaskHandle> CanRunTask(const GTSL::Range<const TaskAccess*> accesses) {
		const auto elementCount = accesses.ElementCount();

		GTSL::uint32 res = 0;

		{
			GTSL::WriteLock<GTSL::ReadWriteMutex> lock(mutex);

			for (GTSL::uint32 i = 0; i < elementCount; ++i) {
				if (currentObjectAccessState[accesses[i].First] == AccessTypes::READ_WRITE) { return GTSL::Result<DispatchedTaskHandle>(false); }
				if (currentObjectAccessState[accesses[i].First] == AccessTypes::READ && accesses[i].Second == AccessTypes::READ_WRITE) { return GTSL::Result<DispatchedTaskHandle>(false); }
			}

			for (GTSL::uint32 i = 0; i < elementCount; ++i) {
				currentObjectAccessState[accesses[i].First] = accesses[i].Second;
				++currentObjectAccessCount[accesses[i].First];
			}

			auto insPos = instances.Emplace();

			res = ongoingTasksAccesses.Emplace(accesses);

			BE_ASSERT(insPos == res, u8"");
		}

		return GTSL::Result(DispatchedTaskHandle(res), true);
	}

	void ReleaseResources(const DispatchedTaskHandle taskIndex) {
		GTSL::WriteLock<GTSL::ReadWriteMutex> lock(mutex);

		const auto count = ongoingTasksAccesses[taskIndex()].GetLength();
		auto& accesses = ongoingTasksAccesses[taskIndex()];

		for (GTSL::uint32 i = 0; i < count; ++i) {
			BE_ASSERT(currentObjectAccessCount[accesses[i].First] != 0, "Oops :/");
			if (--currentObjectAccessCount[accesses[i].First] == 0) { //if object is no longer accessed
				currentObjectAccessState[accesses[i].First] = AccessType();
			}
		}

		ongoingTasksAccesses.Pop(taskIndex());
		instances.Pop(taskIndex());
	}

	void AddSystem(Id objectName) {
		auto lock = GTSL::WriteLock<GTSL::ReadWriteMutex>(mutex);
		currentObjectAccessState.Emplace(0);
		currentObjectAccessCount.Emplace(0);
	}

	void AddInstance(const DispatchedTaskHandle dispatched_task_handle, void* instance) {
		instances[dispatched_task_handle()].EmplaceBack(instance);
	}

	auto GetValidInstances(DispatchedTaskHandle dispatched_task_handle) {
		return instances[dispatched_task_handle()];
	}

private:
	GTSL::FixedVector<AccessType, ALLOCATOR> currentObjectAccessState;
	GTSL::FixedVector<GTSL::uint16, ALLOCATOR> currentObjectAccessCount;

	GTSL::FixedVector<GTSL::StaticVector<TaskAccess, 32>, ALLOCATOR> ongoingTasksAccesses;

	GTSL::FixedVector<GTSL::StaticVector<void*, 16>, ALLOCATOR> instances;

	GTSL::ReadWriteMutex mutex;
};