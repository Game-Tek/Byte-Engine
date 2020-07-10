#pragma once

#include "ByteEngine/Core.h"

#include <atomic>
#include <GTSL/Allocator.h>
#include <GTSL/Ranger.h>

class PoolAllocator
{
public:
	PoolAllocator() = default;
	PoolAllocator(GTSL::AllocatorReference* allocatorReference);

	~PoolAllocator();

	void Allocate(uint64 size, uint64 alignment, void** memory, uint64* allocatedSize, const char* name) const;

	void Deallocate(uint64 size, uint64 alignment, void* memory, const char* name) const;

	void Free() const;

	class Pool
	{
	public:
		Pool() = default;
		
		Pool(uint16 slotsCount, uint32 slotsSize, uint64& allocatedSize, GTSL::AllocatorReference* allocatorReference);

		void Allocate(uint64 size, uint64 alignment, void** data, uint64* allocatedSize);

		void Deallocate(uint64 size, uint64 alignment, void* memory, GTSL::AllocatorReference* allocatorReference);

		void Free(uint64& freedBytes, GTSL::AllocatorReference* allocatorReference) const;

	private:
		using free_slots_type = uint32;
		
		free_slots_type* freeSlotsStack{ nullptr };
		byte* slotsData{ nullptr };
		
		const uint32 SLOTS_SIZE{ 0 };
		const uint32 MAX_SLOTS_COUNT{ 0 };
		
		std::atomic<free_slots_type> slotsCount{ 0 };

		byte* getSlotAddress(const uint32 slotIndex) const { return &slotsData[slotIndex * SLOTS_SIZE]; }
		uint64 getSlotIndexFromPointer(void* pointer) const { return (static_cast<byte*>(pointer) - slotsData) / SLOTS_SIZE; }

		uint64 slotsDataAllocationSize() const { return static_cast<uint64>(MAX_SLOTS_COUNT) * SLOTS_SIZE; }
		static uint64 slotsDataAllocationAlignment() { return alignof(uint64); }

		uint64 freeSlotsStackSize() const { return MAX_SLOTS_COUNT * sizeof(free_slots_type); }
		static uint64 freeSlotsStackAlignment() { return alignof(free_slots_type); }
	};

private:
	Pool* poolsData{ nullptr };
	const uint32 POOL_COUNT{ 0 };
	GTSL::AllocatorReference* systemAllocatorReference{ nullptr };

	[[nodiscard]] GTSL::Ranger<Pool> pools() const { return GTSL::Ranger<Pool>(POOL_COUNT, poolsData); }
};
