#pragma once

#include "ByteEngine/Core.h"

#include <atomic>
#include <GTSL/Allocator.h>
#include <GTSL/Mutex.h>
#include <GTSL/Ranger.h>

class PoolAllocator
{
public:
	PoolAllocator(GTSL::AllocatorReference* allocatorReference);

	~PoolAllocator();

	void Allocate(uint64 size, uint64 alignment, void** memory, uint64* allocatedSize, const char* name) const;

	void Deallocate(uint64 size, uint64 alignment, void* memory, const char* name) const;

	void Free() const;

	class Pool
	{
	public:
		Pool(uint16 slotsCount, uint32 slotsSize, uint8 blockCount, uint64& allocatedSize, GTSL::AllocatorReference* allocatorReference);

		void Allocate(uint64 size, uint64 alignment, void** data, uint64* allocatedSize, uint64& allocatorAllocatedBytes, GTSL::AllocatorReference* allocatorReference);

		void Deallocate(uint64 size, uint64 alignment, void* memory, GTSL::AllocatorReference* allocatorReference) const;

		void Free(uint64& freedBytes, GTSL::AllocatorReference* allocatorReference) const;
		
		class Block
		{
		public:
			Block(uint16 slotsCount, uint32 slotsSize, uint64& allocatedSize, GTSL::AllocatorReference* allocatorReference);
			
			void Allocate(uint64 alignment, void** data, uint64& allocatedSize, uint16 slotsCount, uint32 slotsSize);

			bool AllocateIfFreeSlot(uint64 alignment, void** data, uint64& allocatedSize, uint16 slotsCount, uint32 slotsSize);

			void Deallocate(uint64 alignment, void* data, uint16 slotsCount, uint32 slotsSize);
			
			bool DoesAllocationBelongToBlock(void* data, uint16 slotsCount, uint32 slotsSize) const;
			
			void Free(uint16 slotsCount, uint32 slotsSize, uint64& freedSpace, GTSL::AllocatorReference* allocatorReference);

		private:
			void* dataPointer{ nullptr };
			std::atomic<uint16> freeSlotsCount{ 0 };
			//uint16 freeSlotsCount{ 0 };
			//GTSL::Mutex mutex;
			
			GTSL::Ranger<uint32> freeSlots(const uint16 slotsCount) const { return GTSL::Ranger<uint32>(slotsCount, static_cast<uint32*>(dataPointer)); }
			GTSL::Ranger<byte> slotsData(const uint16 slotsCount, const uint32 slotsSize) const { return GTSL::Ranger<byte>(static_cast<uint64>(slotsCount) * slotsSize, reinterpret_cast<byte*>(freeSlots(slotsCount).end())); }
			
			void popFreeSlot(uint32& freeSlot, const uint16 slotsCount) { freeSlot = freeSlots(slotsCount)[--freeSlotsCount]; }

			void insertFreeSlot(const uint32 freeSlot, const uint16 slotsCount) { freeSlots(slotsCount)[freeSlotsCount++] = freeSlot; }
			
			[[nodiscard]] bool freeSlot() const { return freeSlotsCount; }

			uint32 slotIndexFromPointer(void* p, uint16 slotsCount, uint32 slotsSize) const;
		};

	private:
		//std::atomic<Block*> blocks{ nullptr };
		std::atomic<uint32> blockCount{ 0 };
		std::atomic<uint32> blockCapacity{ 0 };
		std::atomic<uint32> index{ 0 };
		
		Block* blocksData{ nullptr };
		//uint32 blockCount{ 0 };
		//uint32 blockCapacity{ 0 };
		//uint32 index{ 0 };
		GTSL::ReadWriteMutex mutex;
		const uint32 slotsSize{ 0 };
		const uint16 slotsCount{ 0 };

		uint32 allocateAndAddNewBlock(GTSL::AllocatorReference* allocatorReference);
		[[nodiscard]] GTSL::Ranger<Block> blocks() const { return GTSL::Ranger(blockCount, blocksData); }
	};

private:
	Pool* poolsData{ nullptr };
	const uint32 poolCount{ 0 };
	GTSL::AllocatorReference* systemAllocatorReference{ nullptr };

	[[nodiscard]] GTSL::Ranger<Pool> pools() const { return GTSL::Ranger<Pool>(poolCount, poolsData); }
};
