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

	void Allocate(GTSL::uint64 size, GTSL::uint64 alignment, void** memory, GTSL::uint64* allocatedSize, GTSL::Range<const char8_t*> name) const;
	void Deallocate(GTSL::uint64 size, GTSL::uint64 alignment, void* memory, const GTSL::Range<const char8_t*> name) const;

	void free();

	class Pool {
	public:
		Pool() = default;

		// Intialize pool with slotsCount slots of size slotsSize
		void initialize(GTSL::uint32 slotsCount, GTSL::uint32 slotsSize, GTSL::uint64& allocatedSize, BE::SystemAllocatorReference* allocatorReference);

		// Allocate memory from pool
		void allocate(GTSL::uint64 size, GTSL::uint64 alignment, void** data, GTSL::uint64* allocatedSize) const;

		// Deallocate memory from pool
		void deallocate(GTSL::uint64 size, GTSL::uint64 alignment, void* memory, BE::SystemAllocatorReference* allocatorReference) const;

		// Free pool
		void free(GTSL::uint64& freedBytes, BE::SystemAllocatorReference* allocatorReference);

	private:
		using free_slots_type = GTSL::uint64;
		
		free_slots_type* freeSlots = nullptr;
		
#if BE_DEBUG
		free_slots_type* freeSlotsBitTrack2{ nullptr };
		GTSL::uint8* allocCounter{ nullptr };
#endif
		
		GTSL::uint8* slotsData = nullptr;
		
		GTSL::uint32 SLOTS_SIZE = 0;
		GTSL::uint32 MAX_SLOTS_COUNT = 0;

		//mutable GTSL::Mutex poolLock;
		
		GTSL::uint32 bitNums = 0;
		
		[[nodiscard]] GTSL::uint8* getSlotAddress(const GTSL::uint32 slotIndex) const { return slotsData + (slotIndex * SLOTS_SIZE); }
		GTSL::uint32 getSlotIndexFromPointer(void* pointer) const { return static_cast<GTSL::uint32>((static_cast<GTSL::uint8*>(pointer) - slotsData) / SLOTS_SIZE); }

		[[nodiscard]] GTSL::uint64 slotsDataAllocationSize() const { return static_cast<GTSL::uint64>(MAX_SLOTS_COUNT) * SLOTS_SIZE; }
		static GTSL::uint64 slotsDataAllocationAlignment() { return alignof(GTSL::uint64); }
	};

	static constexpr bool USE_MALLOC = false;
	static constexpr bool MEMORY_PATTERN = false;
	static constexpr bool DEALLOC_COUNT = false;

private:
	GTSL::uint64 minimumPoolSize = 16ull; // Minumum default size of a pool, 16 bytes
	GTSL::uint64 maximumPoolSize = 1024ull * 1024ull * 4ull; // Maximum default size of a pool, 4 MB

	GTSL::uint64 minimumPoolSizeBits = 0ull;
	GTSL::uint64 maximumPoolSizeBits = 0ull;

	//GTSL::StaticVector<Pool, 16> pools;

	GTSL::uint32 poolCount = 0;
	Pool pools[32];

	BE::SystemAllocatorReference* systemAllocatorReference{ nullptr };

	// mutable GTSL::Mutex debugLock;

	// mutable std::unordered_map<void*, GTSL::uint32> allocMap;

	// [[nodiscard]] GTSL::Range<Pool*> pools() const { return GTSL::Range<Pool*>(poolCount, pools); }
};
