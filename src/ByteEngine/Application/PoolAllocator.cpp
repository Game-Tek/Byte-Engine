#include "PoolAllocator.h"

#include <new>

#include <GTSL/Bitman.h>
#include <GTSL/Math/Math.hpp>
#include <GTSL/Memory.h>
#include <GTSL/BitTracker.h>
#include <GTSL/Assert.h>

#include "ByteEngine/Debug/Assert.h"

PoolAllocator::PoolAllocator(BE::SystemAllocatorReference* allocatorReference) : systemAllocatorReference(allocatorReference) {}

void PoolAllocator::Pool::initialize(const uint32 slotsCount, const uint32 slotsSize, uint64& allocatedSize, BE::SystemAllocatorReference* allocatorReference)
{
	SLOTS_SIZE = slotsSize; MAX_SLOTS_COUNT = GTSL::Math::RoundUpByPowerOf2(slotsCount, 8);

	uint64 pool_allocated_size{ 0 };

	// Allocate memory for slots, this memory is where allocations will be placed
	allocatorReference->Allocate(slotsDataAllocationSize(), slotsDataAllocationAlignment(), reinterpret_cast<void**>(&slotsData), &pool_allocated_size);
	allocatedSize += pool_allocated_size;
	
	// Allocate memory to track free slots
	allocatorReference->Allocate(GTSL::GetAllocationSize<free_slots_type>(MAX_SLOTS_COUNT), alignof(free_slots_type), reinterpret_cast<void**>(&freeSlots), &pool_allocated_size);
	allocatedSize += pool_allocated_size;

	bitNums = MAX_SLOTS_COUNT / GTSL::Bits<free_slots_type>() + 1;

#if BE_DEBUG
	if constexpr (DEALLOC_COUNT) {
		allocatorReference->Allocate(GTSL::GetAllocationSize<free_slots_type>(MAX_SLOTS_COUNT), alignof(free_slots_type), reinterpret_cast<void**>(&freeSlotsBitTrack2), &pool_allocated_size);
		allocatedSize += pool_allocated_size;

		GTSL::InitializeBits(GTSL::Range<free_slots_type*>(bitNums, freeSlotsBitTrack2));
		
		allocatorReference->Allocate(MAX_SLOTS_COUNT * sizeof(uint8), alignof(uint8), reinterpret_cast<void**>(&allocCounter), &pool_allocated_size);
		allocatedSize += pool_allocated_size;

		for (uint32 i = 0; i < MAX_SLOTS_COUNT; ++i) {
			allocCounter[i] = 0;
		}
	}
#endif

	// Initialize free slots
	GTSL::InitializeBits(GTSL::Range<free_slots_type*>(bitNums, freeSlots));
	
	if constexpr (MEMORY_PATTERN) {
		for (uint32 i = 0; i < SLOTS_SIZE * MAX_SLOTS_COUNT; ++i) {
			slotsData[i] = 0xCA;
		}
	}
}

void PoolAllocator::initialize() {
	BE_ASSERT(maximumPoolSize != 0 || minimumPoolSize != 0, "Minimum and maximum pool size must be set.");
	BE_ASSERT(minimumPoolSize <= maximumPoolSize, "Minimum pool size must be smaller than maximum pool size.");
	BE_ASSERT(GTSL::Math::IsPowerOfTwo(minimumPoolSize), "Minimum pool size must be a power of 2.");
	BE_ASSERT(GTSL::Math::IsPowerOfTwo(maximumPoolSize), "Maximum pool size must be a power of 2.");

	uint64 allocator_allocated_size{ 0 }; // Accumulated size of all allocations of underlying pools

	minimumPoolSizeBits = GTSL::FindFirstSetBit(minimumPoolSize).Get();
	maximumPoolSizeBits = GTSL::FindFirstSetBit(maximumPoolSize).Get();

	uint64 poolCount = maximumPoolSizeBits - minimumPoolSizeBits + 1;

	for(uint64 i = 0, poolSizeBits = minimumPoolSizeBits; i < poolCount; ++poolSizeBits, ++i) {
		const auto slot_count = (poolCount - i) * 60; //pools with smaller slot sizes get more slots
		const auto slot_size = 1 << poolSizeBits; // 2^poolSize, all pools have power of 2 slot sizes

		//auto& pool = pools.EmplaceBack();
		auto& pool = pools[i];
		pool.initialize(slot_count, slot_size, allocator_allocated_size, systemAllocatorReference);
	}

	this->poolCount = poolCount;
}

// ALLOCATE //

void PoolAllocator::Allocate(const uint64 size, const uint64 alignment, void** memory, uint64* allocatedSize, GTSL::Range<const char8_t*> name) const
{
	// Find the smallest pool size that can fit the allocation
	uint64 allocationMinSize = GTSL::NextPowerOfTwo(GTSL::Math::RoundUpByPowerOf2(size, alignment));
	allocationMinSize = GTSL::Math::Max(allocationMinSize, minimumPoolSize); //

	BE_ASSERT(allocationMinSize <= maximumPoolSize, "Allocation is too big!");
	BE_ASSERT((alignment & (alignment - 1)) == 0, "Alignment is not power of two!");
	
	if constexpr (USE_MALLOC) {
		*memory = malloc(size);
		*allocatedSize = size;
	} else {
		auto set_bit = GTSL::FindFirstSetBit(allocationMinSize);
		uint64 poolIndex = set_bit.Get() - minimumPoolSizeBits;
		pools[poolIndex].allocate(size, alignment, memory, allocatedSize);
	}

	if constexpr (DEALLOC_COUNT) {
		//GTSL::Lock lock(debugLock);
		//allocMap.emplace(*memory, allocationMinSize);
	}
}

void PoolAllocator::Pool::allocate(const uint64 size, const uint64 alignment, void** data, uint64* allocatedSize) const
{
	//GTSL::Lock lock(poolLock);
	
	auto slot = GTSL::OccupyFirstFreeSlot(GTSL::Range<free_slots_type*>(bitNums, freeSlots));

	byte* const slot_address = getSlotAddress(slot.Get());
	
	if constexpr (MEMORY_PATTERN) {
		bool isCorrect = true;

		for (uint32 i = 0; i < SLOTS_SIZE; ++i) {
			if (slot_address[i] != 0xCA) { isCorrect = false; break; }
		}

		BE_ASSERT(isCorrect, u8"Memory was written to after deallocation.");
	}

#if BE_DEBUG
	if constexpr (DEALLOC_COUNT) {
		auto slot2 = GTSL::OccupyFirstFreeSlot(GTSL::Range<free_slots_type*>(bitNums, freeSlotsBitTrack2));
		BE_ASSERT(slot.Get() == slot2.Get(), u8"");
		BE_ASSERT(allocCounter[slot.Get()] == 0, u8"");
		++allocCounter[slot.Get()];
	}
#endif
	
	BE_ASSERT(slot.State(), "No more free slots!")
	
	//*data = GTSL::AlignPointer(alignment, slot_address);
	*data = slot_address;
	*allocatedSize = (slot_address + SLOTS_SIZE) - static_cast<byte*>(*data);
	
	BE_ASSERT(*data >= slotsData && *data <= slotsData + slotsDataAllocationSize(), "Allocation does not belong to pool!")
}

// DEALLOCATE //

void PoolAllocator::Deallocate(const uint64 size, const uint64 alignment, void* memory, GTSL::Range<const char8_t*> name) const
{
	uint64 allocationMinSize = GTSL::NextPowerOfTwo(size);
	allocationMinSize = GTSL::Math::Max(allocationMinSize, minimumPoolSize); //

	BE_ASSERT(allocationMinSize <= maximumPoolSize, "Allocation is too big!");
	BE_ASSERT((alignment & (alignment - 1)) == 0, "Alignment is not power of two!");
	
	if constexpr (USE_MALLOC) {
		::free(memory);
	} else {
		auto set_bit = GTSL::FindFirstSetBit(allocationMinSize);
		uint64 poolIndex = set_bit.Get() - minimumPoolSizeBits;
		pools[poolIndex].deallocate(size, alignment, memory, systemAllocatorReference);
	}

	if constexpr (DEALLOC_COUNT) {
		//GTSL::Lock lock(debugLock);
		//BE_ASSERT(allocMap.at(memory) == allocationMinSize, u8"");
		//allocMap.erase(memory);
	}
}

void PoolAllocator::Pool::deallocate(uint64 size, const uint64 alignment, void* memory, BE::SystemAllocatorReference* allocatorReference) const
{
	//GTSL::Lock lock(poolLock);
	
	BE_ASSERT(memory >= slotsData && memory <= slotsData + slotsDataAllocationSize(), "Allocation does not belong to pool!")

	const auto index = getSlotIndexFromPointer(memory);

	if constexpr (MEMORY_PATTERN) {
		for (uint32 i = 0; i < SLOTS_SIZE; ++i) {
			static_cast<byte*>(memory)[i] = 0xCA;
		}
	}

#if BE_DEBUG
	if constexpr (DEALLOC_COUNT) {
		GTSL::SetAsFree(GTSL::Range<free_slots_type*>(bitNums, freeSlotsBitTrack2), index);
		BE_ASSERT(allocCounter[index] == 1, u8"");
		--allocCounter[index];
	}
#endif
	
	GTSL::SetAsFree(GTSL::Range<free_slots_type*>(bitNums, freeSlots), index);
}

// FREE //

void PoolAllocator::free() {
	uint64 freed_bytes{ 0 };

	for (auto& pool : pools) { pool.free(freed_bytes, systemAllocatorReference); }
}

void PoolAllocator::Pool::free(uint64& freedBytes, BE::SystemAllocatorReference* allocatorReference) {
	if (slotsData) allocatorReference->Deallocate(slotsDataAllocationSize(), slotsDataAllocationAlignment(), slotsData);	
	if(freeSlots) allocatorReference->Deallocate(GTSL::GetAllocationSize<free_slots_type>(MAX_SLOTS_COUNT), alignof(free_slots_type), freeSlots);

#if BE_DEBUG
	if constexpr (DEALLOC_COUNT) {
		allocatorReference->Deallocate(GTSL::GetAllocationSize<free_slots_type>(MAX_SLOTS_COUNT), alignof(free_slots_type), freeSlotsBitTrack2);
		freedBytes += GTSL::GetAllocationSize<free_slots_type>(MAX_SLOTS_COUNT);

		allocatorReference->Deallocate(MAX_SLOTS_COUNT * sizeof(uint8), alignof(uint32), allocCounter);
		freedBytes += MAX_SLOTS_COUNT * sizeof(uint8);
	}
#endif
}