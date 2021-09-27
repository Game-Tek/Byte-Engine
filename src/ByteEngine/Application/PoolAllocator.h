#pragma once

#include <unordered_map>

#include "ByteEngine/Core.h"

#include <GTSL/Mutex.h>
#include <GTSL/Range.hpp>

#include "ByteEngine/Game/System.h"

class PoolAllocator
{
public:
	PoolAllocator() = default;
	PoolAllocator(BE::SystemAllocatorReference* allocatorReference);

	~PoolAllocator() = default;

	void Allocate(uint64 size, uint64 alignment, void** memory, uint64* allocatedSize, GTSL::Range<const char8_t*> name) const;

	void Deallocate(uint64 size, uint64 alignment, void* memory, const GTSL::Range<const char8_t*> name) const;

	void Free() const;

	class Pool
	{
	public:
		Pool() = default;
		
		Pool(uint32 slotsCount, uint32 slotsSize, uint64& allocatedSize, BE::SystemAllocatorReference* allocatorReference);

		void Allocate(uint64 size, uint64 alignment, void** data, uint64* allocatedSize);

		void Deallocate(uint64 size, uint64 alignment, void* memory, BE::SystemAllocatorReference* allocatorReference);

		void Free(uint64& freedBytes, BE::SystemAllocatorReference* allocatorReference) const;

	private:
		using free_slots_type = uint64;
		
		free_slots_type* freeSlotsBitTrack{ nullptr };
		
#ifdef _DEBUG
		free_slots_type* freeSlotsBitTrack2{ nullptr };
		uint8* allocCounter{ nullptr };
#endif
		
		byte* slotsData{ nullptr };
		
		const uint32 SLOTS_SIZE{ 0 };
		const uint32 MAX_SLOTS_COUNT{ 0 };
		mutable GTSL::Mutex poolLock;
		uint32 bitNums = 0;
		
		[[nodiscard]] byte* getSlotAddress(const uint32 slotIndex) const { return slotsData + (slotIndex * SLOTS_SIZE); }
		uint32 getSlotIndexFromPointer(void* pointer) const { return static_cast<uint32>((static_cast<byte*>(pointer) - slotsData) / SLOTS_SIZE); }

		[[nodiscard]] uint64 slotsDataAllocationSize() const { return static_cast<uint64>(MAX_SLOTS_COUNT) * SLOTS_SIZE; }
		static uint64 slotsDataAllocationAlignment() { return alignof(uint64); }
	};


	static constexpr bool USE_MALLOC = true;
	static constexpr bool MEMORY_PATTERN = false;
	static constexpr bool DEALLOC_COUNT = false;

private:
	Pool* poolsData{ nullptr };
	const uint32 POOL_COUNT{ 0 };
	BE::SystemAllocatorReference* systemAllocatorReference{ nullptr };

	mutable GTSL::Mutex debugLock;
	mutable std::unordered_map<void*, uint32> allocMap;	

	[[nodiscard]] GTSL::Range<Pool*> pools() const { return GTSL::Range<Pool*>(POOL_COUNT, poolsData); }
};
