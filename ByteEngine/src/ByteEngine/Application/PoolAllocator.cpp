#include "PoolAllocator.h"

#include <GTSL/Bitman.h>
#include <GTSL/Math/Math.hpp>
#include <new>
#include <GTSL/Memory.h>


#include "ByteEngine/Debug/Assert.h"

PoolAllocator::PoolAllocator(BE::SystemAllocatorReference* allocatorReference) : POOL_COUNT(23), systemAllocatorReference(allocatorReference)
{
	uint64 allocator_allocated_size{ 0 }; //debug
	
	allocatorReference->Allocate(sizeof(Pool) * POOL_COUNT, alignof(Pool), reinterpret_cast<void**>(&poolsData), &allocator_allocated_size);

	for (uint8 i = 0, j = POOL_COUNT; i < POOL_COUNT; ++i, --j)
	{	
		const auto slot_count = j * POOL_COUNT / 4; //pools with smaller slot sizes get more slots
		const auto slot_size = 1 << i;

		::new(poolsData + i) Pool(slot_count, slot_size, allocator_allocated_size, allocatorReference);
	}
}

PoolAllocator::Pool::Pool(const uint16 slotsCount, const uint32 slotsSize, uint64& allocatedSize, BE::SystemAllocatorReference* allocatorReference) : SLOTS_SIZE(slotsSize), MAX_SLOTS_COUNT(slotsCount), slotsCount(MAX_SLOTS_COUNT)
{
	uint64 pool_allocated_size{ 0 };
	
	allocatorReference->Allocate(slotsDataAllocationSize(), slotsDataAllocationAlignment(), reinterpret_cast<void**>(&slotsData), &pool_allocated_size);
	allocatedSize += pool_allocated_size;
	allocatorReference->Allocate(freeSlotsStackSize(), freeSlotsStackAlignment(), reinterpret_cast<void**>(&freeSlotsStack), &pool_allocated_size);
	allocatedSize += pool_allocated_size;

	BE_DEBUG_ONLY(GTSL::SetMemory(slotsDataAllocationSize(), slotsData));
	
	for(uint32 i = 0; i < MAX_SLOTS_COUNT; ++i)
	{
		freeSlotsStack[i] = i;
	}
}

// ALLOCATE //

void PoolAllocator::Allocate(const uint64 size, const uint64 alignment, void** memory, uint64* allocatedSize, const char* name) const
{
	GTSL::Lock lock(globalLock);
	BE_ASSERT((alignment & (alignment - 1)) == 0, "Alignment is not power of two!");
	uint64 allocation_min_size{ 0 }; GTSL::NextPowerOfTwo(GTSL::Math::PowerOf2RoundUp(size, alignment), allocation_min_size);

	uint8 set_bit { 0 }; GTSL::FindFirstSetBit(allocation_min_size, set_bit);
	BE_ASSERT(POOL_COUNT > set_bit, "No pool big enough!");	

	poolsData[set_bit].Allocate(size, alignment, memory, allocatedSize);
}

void PoolAllocator::Pool::Allocate(const uint64 size, const uint64 alignment, void** data, uint64* allocatedSize)
{
	BE_ASSERT(slotsCount != 0, "No more free slots remaining!")
	const uint32 slot = freeSlotsStack[--slotsCount];
	byte* const slot_address = getSlotAddress(slot);
	*data = GTSL::AlignPointer(alignment, slot_address);
	*allocatedSize = (slot_address + SLOTS_SIZE) - static_cast<byte*>(*data);

	BE_ASSERT(*data >= slotsData && *data <= slotsData + slotsDataAllocationSize(), "Allocation does not belong to pool!")
}

// DEALLOCATE //

void PoolAllocator::Deallocate(const uint64 size, const uint64 alignment, void* memory, const char* name) const
{
	GTSL::Lock lock(globalLock);
	BE_ASSERT((alignment & (alignment - 1)) == 0, "Alignment is not power of two!");
	uint64 allocation_min_size { 0 }; GTSL::NextPowerOfTwo(size, allocation_min_size);
	uint8 set_bit{ 0 }; GTSL::FindFirstSetBit(allocation_min_size, set_bit);
	poolsData[set_bit].Deallocate(size, alignment, memory, systemAllocatorReference);
}

void PoolAllocator::Pool::Deallocate(uint64 size, const uint64 alignment, void* memory, BE::SystemAllocatorReference* allocatorReference)
{
	BE_ASSERT(memory >= slotsData && memory <= slotsData + slotsDataAllocationSize(), "Allocation does not belong to pool!")

	const auto index = getSlotIndexFromPointer(memory);
	freeSlotsStack[slotsCount++] = index;

	BE_ASSERT(slotsCount <= MAX_SLOTS_COUNT, "More allocations have been freed than there are slots!")
}

// FREE //

void PoolAllocator::Free() const
{
	uint64 freed_bytes{ 0 };

	for (auto& pool : pools()) { pool.Free(freed_bytes, systemAllocatorReference); }
}

void PoolAllocator::Pool::Free(uint64& freedBytes, BE::SystemAllocatorReference* allocatorReference) const
{
	allocatorReference->Deallocate(slotsDataAllocationSize(), slotsDataAllocationAlignment(), slotsData);
	allocatorReference->Deallocate(freeSlotsStackSize(), freeSlotsStackAlignment(), freeSlotsStack);

	freedBytes += slotsDataAllocationSize();
	freedBytes += freeSlotsStackSize();
}