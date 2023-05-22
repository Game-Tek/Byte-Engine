#pragma once

#include "ByteEngine/Core.h"
#include <GTSL/Mutex.h>

/**
 * \brief Allocates memory directly from the OS. Useful for all other allocators.
 */
class SystemAllocator
{
public:
	struct DebugData
	{
		GTSL::uint64 AllocatedBytes{ 0 };
		GTSL::uint64 DeallocatedBytes{ 0 };
		GTSL::uint64 TotalAllocatedBytes{ 0 };
		GTSL::uint64 TotalDeallocatedBytes{ 0 };
		GTSL::uint64 AllocationCount{ 0 };
		GTSL::uint64 TotalAllocationCount{ 0 };
		GTSL::uint64 DeallocationCount{ 0 };
		GTSL::uint64 TotalDeallocationCount{ 0 };
	};
protected:
	// GTSL::Mutex allocatorMutex;
	
#if BE_DEBUG
	// GTSL::Mutex debugDataMutex;
	GTSL::uint64 allocatedBytes{ 0 };
	GTSL::uint64 deallocatedBytes{ 0 };
	GTSL::uint64 totalAllocatedBytes{ 0 };
	GTSL::uint64 totalDeallocatedBytes{ 0 };
	GTSL::uint64 allocationCount{ 0 };
	GTSL::uint64 deallocationCount{ 0 };
	GTSL::uint64 totalAllocationCount{ 0 };
	GTSL::uint64 totalDeallocationCount{ 0 };
#endif
	
public:
	SystemAllocator() = default;

#if BE_DEBUG
	void GetDebugData(DebugData& debugData)
	{
		// GTSL::Lock<GTSL::Mutex> lock(debugDataMutex);
		debugData.AllocationCount = allocationCount;
		debugData.AllocatedBytes = allocatedBytes;
		debugData.DeallocatedBytes = deallocatedBytes;
		debugData.TotalAllocatedBytes = totalAllocatedBytes;
		debugData.TotalDeallocatedBytes = totalDeallocatedBytes;
		debugData.TotalAllocationCount = totalAllocationCount;

		allocationCount = 0;
		allocatedBytes = 0;
		deallocatedBytes = 0;
	}
#endif

	void Allocate(const GTSL::uint64 size, const GTSL::uint64 alignment, void** data);

	void Deallocate(const GTSL::uint64 size, const GTSL::uint64 alignment, void* data);
};
