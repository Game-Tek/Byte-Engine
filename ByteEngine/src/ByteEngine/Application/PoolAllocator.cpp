#include "PoolAllocator.h"

#include <GTSL/Bitman.h>
#include <GTSL/Math/Math.hpp>
#include <new>
#include <GTSL/Memory.h>

#include <GTSL/BitTracker.h>

#include "ByteEngine/Debug/Assert.h"

PoolAllocator::PoolAllocator(BE::SystemAllocatorReference* allocatorReference) : POOL_COUNT(23), systemAllocatorReference(allocatorReference)
{
	uint64 allocator_allocated_size{ 0 }; //debug
	
	allocatorReference->Allocate(sizeof(Pool) * POOL_COUNT, alignof(Pool), reinterpret_cast<void**>(&poolsData), &allocator_allocated_size);

	for (uint8 i = 0, j = POOL_COUNT; i < POOL_COUNT; ++i, --j)
	{	
		const auto slot_count = j * POOL_COUNT * 1.5; //pools with smaller slot sizes get more slots
		const auto slot_size = 1 << i;

		::new(poolsData + i) Pool(slot_count, slot_size, allocator_allocated_size, allocatorReference);
	}
}

PoolAllocator::Pool::Pool(const uint32 slotsCount, const uint32 slotsSize, uint64& allocatedSize, BE::SystemAllocatorReference* allocatorReference) : SLOTS_SIZE(slotsSize), MAX_SLOTS_COUNT(slotsCount)
{
	uint64 pool_allocated_size{ 0 };
	
	allocatorReference->Allocate(slotsDataAllocationSize(), slotsDataAllocationAlignment(), reinterpret_cast<void**>(&slotsData), &pool_allocated_size);
	allocatedSize += pool_allocated_size;
	
	allocatorReference->Allocate(GTSL::GetAllocationSize<free_slots_type>(MAX_SLOTS_COUNT), alignof(free_slots_type), reinterpret_cast<void**>(&freeSlotsBitTrack), &pool_allocated_size);
	allocatedSize += pool_allocated_size;

	bitNums = MAX_SLOTS_COUNT / GTSL::Bits<free_slots_type>() + 1;

	if constexpr (STRONG_CHECK) {
		for (uint32 i = 0; i < SLOTS_SIZE * MAX_SLOTS_COUNT; ++i) {
			slotsData[i] = 0xCA;
		}
	}
	
	GTSL::InitializeBits(GTSL::Range<free_slots_type*>(bitNums, freeSlotsBitTrack));
}

// ALLOCATE //

void PoolAllocator::Allocate(const uint64 size, const uint64 alignment, void** memory, uint64* allocatedSize, const char* name) const
{
	GTSL::Lock lock(globalLock);
	
	if constexpr (USE_MALLOC) {
		*memory = malloc(size);
		*allocatedSize = size;
	} else {		
		BE_ASSERT((alignment & (alignment - 1)) == 0, "Alignment is not power of two!");
		uint64 allocation_min_size{ GTSL::NextPowerOfTwo(GTSL::Math::RoundUpByPowerOf2(size, alignment)) };

		auto set_bit = GTSL::FindFirstSetBit(allocation_min_size);
		BE_ASSERT(POOL_COUNT > set_bit.Get(), "No pool big enough!");

		poolsData[set_bit.Get()].Allocate(size, alignment, memory, allocatedSize);
	}
}

void PoolAllocator::Pool::Allocate(const uint64 size, const uint64 alignment, void** data, uint64* allocatedSize)
{
	auto slot = GTSL::OccupyFirstFreeSlot(GTSL::Range<free_slots_type*>(bitNums, freeSlotsBitTrack), MAX_SLOTS_COUNT);

	byte* const slot_address = getSlotAddress(slot.Get());
	
	if constexpr (STRONG_CHECK) {
		bool isCorrect = true;

		for(uint32 i = 0; i < SLOTS_SIZE; ++i) {
			if (slot_address[i] != 0xCA) { isCorrect = false; break; }
		}

		BE_ASSERT(isCorrect);
	}
	
	BE_ASSERT(slot.State(), "No more free slots!")
	
	//*data = GTSL::AlignPointer(alignment, slot_address);
	*data = slot_address;
	*allocatedSize = (slot_address + SLOTS_SIZE) - static_cast<byte*>(*data);
	
	BE_ASSERT(*data >= slotsData && *data <= slotsData + slotsDataAllocationSize(), "Allocation does not belong to pool!")
}

// DEALLOCATE //

void PoolAllocator::Deallocate(const uint64 size, const uint64 alignment, void* memory, const char* name) const
{
	GTSL::Lock lock(globalLock);

	if constexpr (USE_MALLOC) {
		free(memory);
	} else {
		BE_ASSERT((alignment & (alignment - 1)) == 0, "Alignment is not power of two!");
		uint64 allocation_min_size{ GTSL::NextPowerOfTwo(size) };
		auto set_bit = GTSL::FindFirstSetBit(allocation_min_size);
		poolsData[set_bit.Get()].Deallocate(size, alignment, memory, systemAllocatorReference);
	}
}

void PoolAllocator::Pool::Deallocate(uint64 size, const uint64 alignment, void* memory, BE::SystemAllocatorReference* allocatorReference)
{
	BE_ASSERT(memory >= slotsData && memory <= slotsData + slotsDataAllocationSize(), "Allocation does not belong to pool!")

	const auto index = getSlotIndexFromPointer(memory);

	if constexpr (STRONG_CHECK) {
		for (uint32 i = 0; i < SLOTS_SIZE; ++i) {
			static_cast<byte*>(memory)[i] = 0xCA;
		}
	}
	
	GTSL::SetAsFree(GTSL::Range<free_slots_type*>(bitNums, freeSlotsBitTrack), index);
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
	allocatorReference->Deallocate(GTSL::GetAllocationSize<free_slots_type>(MAX_SLOTS_COUNT), alignof(free_slots_type), freeSlotsBitTrack);

	if constexpr (_DEBUG)
	{
		freedBytes += slotsDataAllocationSize();
		freedBytes += GTSL::GetAllocationSize<free_slots_type>(MAX_SLOTS_COUNT);
	}
}