#pragma once

#include "ByteEngine/Core.h"

#include <GTSL/Allocator.h>
#include <GTSL/Atomic.hpp>
#include <GTSL/Ranger.h>

#include "ByteEngine/Game/System.h"

class PoolAllocator
{
public:
	PoolAllocator() = default;
	PoolAllocator(BE::SystemAllocatorReference* allocatorReference);

	~PoolAllocator() = default;

	void Allocate(uint64 size, uint64 alignment, void** memory, uint64* allocatedSize, const char* name) const;

	void Deallocate(uint64 size, uint64 alignment, void* memory, const char* name) const;

	void Free() const;

	class Pool
	{
	public:
		Pool() = default;
		
		Pool(uint16 slotsCount, uint32 slotsSize, uint64& allocatedSize, BE::SystemAllocatorReference* allocatorReference);

		void Allocate(uint64 size, uint64 alignment, void** data, uint64* allocatedSize);

		void Deallocate(uint64 size, uint64 alignment, void* memory, BE::SystemAllocatorReference* allocatorReference);

		void Free(uint64& freedBytes, BE::SystemAllocatorReference* allocatorReference) const;

	private:
		using free_slots_type = uint32;
		
		free_slots_type* freeSlotsStack{ nullptr };
		byte* slotsData{ nullptr };
		
		const uint32 SLOTS_SIZE{ 0 };
		const uint32 MAX_SLOTS_COUNT{ 0 };
		GTSL::Atomic<free_slots_type> slotsCount{ 0 };

		[[nodiscard]] byte* getSlotAddress(const uint32 slotIndex) const { return &slotsData[slotIndex * SLOTS_SIZE]; }
		uint64 getSlotIndexFromPointer(void* pointer) const { return (static_cast<byte*>(pointer) - slotsData) / SLOTS_SIZE; }

		[[nodiscard]] uint64 slotsDataAllocationSize() const { return static_cast<uint64>(MAX_SLOTS_COUNT) * SLOTS_SIZE; }
		static uint64 slotsDataAllocationAlignment() { return alignof(uint64); }

		[[nodiscard]] uint64 freeSlotsStackSize() const { return MAX_SLOTS_COUNT * sizeof(free_slots_type); }
		static uint64 freeSlotsStackAlignment() { return alignof(free_slots_type); }
	};

private:
	Pool* poolsData{ nullptr };
	const uint32 POOL_COUNT{ 0 };
	BE::SystemAllocatorReference* systemAllocatorReference{ nullptr };

	[[nodiscard]] GTSL::Ranger<Pool> pools() const { return GTSL::Ranger<Pool>(POOL_COUNT, poolsData); }
};
