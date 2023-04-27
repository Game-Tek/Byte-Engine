#pragma once

#include <unordered_map>

#include "ByteEngine/Core.h"

#include <GTSL/Mutex.h>
#include <GTSL/Range.hpp>
#include <GTSL/Vector.hpp>

#include "ByteEngine/Game/System.hpp"

class PoolAllocator {
public:
	PoolAllocator() = default;
	PoolAllocator(BE::SystemAllocatorReference* allocatorReference);

	~PoolAllocator() = default;

	void initialize();

	void Allocate(uint64 size, uint64 alignment, void** memory, uint64* allocatedSize, GTSL::Range<const char8_t*> name) const;
	void Deallocate(uint64 size, uint64 alignment, void* memory, const GTSL::Range<const char8_t*> name) const;

	void free();

	class Pool {
	public:
		Pool() = default;

		// Intialize pool with slotsCount slots of size slotsSize
		void initialize(uint32 slotsCount, uint32 slotsSize, uint64& allocatedSize, BE::SystemAllocatorReference* allocatorReference);

		// Allocate memory from pool
		void allocate(uint64 size, uint64 alignment, void** data, uint64* allocatedSize) const;

		// Deallocate memory from pool
		void deallocate(uint64 size, uint64 alignment, void* memory, BE::SystemAllocatorReference* allocatorReference) const;

		// Free pool
		void free(uint64& freedBytes, BE::SystemAllocatorReference* allocatorReference);

	private:
		using free_slots_type = uint64;
		
		free_slots_type* freeSlots = nullptr;
		
#if BE_DEBUG
		free_slots_type* freeSlotsBitTrack2{ nullptr };
		uint8* allocCounter{ nullptr };
#endif
		
		byte* slotsData = nullptr;
		
		uint32 SLOTS_SIZE = 0;
		uint32 MAX_SLOTS_COUNT = 0;

		//mutable GTSL::Mutex poolLock;
		
		uint32 bitNums = 0;
		
		[[nodiscard]] byte* getSlotAddress(const uint32 slotIndex) const { return slotsData + (slotIndex * SLOTS_SIZE); }
		uint32 getSlotIndexFromPointer(void* pointer) const { return static_cast<uint32>((static_cast<byte*>(pointer) - slotsData) / SLOTS_SIZE); }

		[[nodiscard]] uint64 slotsDataAllocationSize() const { return static_cast<uint64>(MAX_SLOTS_COUNT) * SLOTS_SIZE; }
		static uint64 slotsDataAllocationAlignment() { return alignof(uint64); }
	};

	static constexpr bool USE_MALLOC = false;
	static constexpr bool MEMORY_PATTERN = false;
	static constexpr bool DEALLOC_COUNT = false;

private:
	uint64 minimumPoolSize = 16ull; // Minumum default size of a pool, 16 bytes
	uint64 maximumPoolSize = 1024ull * 1024ull * 4ull; // Maximum default size of a pool, 4 MB

	uint64 minimumPoolSizeBits = 0ull;
	uint64 maximumPoolSizeBits = 0ull;

	//GTSL::StaticVector<Pool, 16> pools;

	uint32 poolCount = 0;
	Pool pools[32];

	BE::SystemAllocatorReference* systemAllocatorReference{ nullptr };

	// mutable GTSL::Mutex debugLock;

	// mutable std::unordered_map<void*, uint32> allocMap;

	// [[nodiscard]] GTSL::Range<Pool*> pools() const { return GTSL::Range<Pool*>(poolCount, pools); }
};
