#include "SystemAllocator.h"

void SystemAllocator::Allocate(const uint64 size, const uint64 alignment, void** data)
{
	const uint64 allocated_size{ GTSL::Math::PowerOf2RoundUp(size, static_cast<uint32>(alignment)) };
	void* raw_data_alloc{nullptr};

	allocatorMutex.Lock();
	GTSL::Allocate(size, &raw_data_alloc);
	allocatorMutex.Unlock();

	//*data = GTSL::Memory::AlignedPointer(alignment, raw_data_alloc);
	//::new(static_cast<byte*>(*data)) (void*)(raw_data_alloc);

	*data = raw_data_alloc;

	BE_DEBUG_ONLY(GTSL::Lock<GTSL::Mutex> lock(debugDataMutex))
	BE_DEBUG_ONLY(allocatedBytes += size)
	BE_DEBUG_ONLY(totalAllocatedBytes += size)
	BE_DEBUG_ONLY(++allocationCount)
	BE_DEBUG_ONLY(++totalAllocationCount)
}

void SystemAllocator::Deallocate(const uint64 size, const uint64 alignment, void* data)
{
	const uint64 deallocated_size{GTSL::Math::PowerOf2RoundUp(size, static_cast<uint32>(alignment))};

	allocatorMutex.Lock();
	GTSL::Deallocate(size, data);
	allocatorMutex.Unlock();

	BE_DEBUG_ONLY(GTSL::Lock<GTSL::Mutex> lock(debugDataMutex))
	BE_DEBUG_ONLY(deallocatedBytes += size)
	BE_DEBUG_ONLY(totalDeallocatedBytes += size)
	BE_DEBUG_ONLY(++deallocationCount)
	BE_DEBUG_ONLY(++totalDeallocationCount)
}
