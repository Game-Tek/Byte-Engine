#include "SystemAllocator.h"

#include <GTSL/Memory.h>
#include <GTSL/Math/Math.hpp>

void SystemAllocator::Allocate(const uint64 size, const uint64 alignment, void** data)
{
	const uint64 allocated_size{ GTSL::Math::RoundUpByPowerOf2(size, alignment) };

	allocatorMutex.Lock();
	GTSL::Allocate(allocated_size, data);
	allocatorMutex.Unlock();

	//*data = GTSL::AlignPointer(alignment, data);

	BE_DEBUG_ONLY(GTSL::Lock<GTSL::Mutex> lock(debugDataMutex))
	BE_DEBUG_ONLY(allocatedBytes += allocated_size)
	BE_DEBUG_ONLY(totalAllocatedBytes += allocated_size)
	BE_DEBUG_ONLY(++allocationCount)
	BE_DEBUG_ONLY(++totalAllocationCount)
}

void SystemAllocator::Deallocate(const uint64 size, const uint64 alignment, void* data)
{
	const uint64 allocation_size{GTSL::Math::RoundUpByPowerOf2(size, alignment)};

	//byte* dealigned_pointer = static_cast<byte*>(data) - (allocation_size - size);
	byte* dealigned_pointer = static_cast<byte*>(data);
	
	allocatorMutex.Lock();
	GTSL::Deallocate(allocation_size, dealigned_pointer);
	allocatorMutex.Unlock();

	BE_DEBUG_ONLY(GTSL::Lock<GTSL::Mutex> lock(debugDataMutex))
	BE_DEBUG_ONLY(deallocatedBytes += allocation_size)
	BE_DEBUG_ONLY(totalDeallocatedBytes += allocation_size)
	BE_DEBUG_ONLY(++deallocationCount)
	BE_DEBUG_ONLY(++totalDeallocationCount)
}
