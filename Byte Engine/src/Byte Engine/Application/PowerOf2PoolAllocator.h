#pragma once

#include "Byte Engine/Core.h"

#include <atomic>
#include <GTSL/Mutex.h>
#include <GTSL/Allocator.h>
#include <GTSL/Vector.hpp>

class PowerOf2PoolAllocator
{
	GTSL::AllocatorReference* allocatorReference{ nullptr };

	class Pool
	{
		struct Block
		{
		protected:
			GTSL::Mutex mutex;
			void* data{ nullptr };

			uint16 freeSlotsCount{ 0 };

			[[nodiscard]] uint32* freeSlotsIndices() const { return reinterpret_cast<uint32*>(reinterpret_cast<byte*>(data)); }
			[[nodiscard]] byte* blockData(const uint16 slotsCount) const { return reinterpret_cast<byte*>(freeSlotsIndices()) + sizeof(uint32) * slotsCount; }
			[[nodiscard]] byte* blockDataEnd(const uint16 slotsCount, const uint32 slotsSize) const { return blockData(slotsCount) + slotsCount * slotsSize; }

			void popFreeSlot(uint32& freeSlot)
			{
				freeSlot = freeSlotsIndices()[freeSlotsCount];
				--freeSlotsCount;
			}

			void placeFreeSlot(const uint32 freeSlot)
			{
				++freeSlotsCount;
				freeSlotsIndices()[freeSlotsCount] = freeSlot;
			}
			
			[[nodiscard]] bool freeSlot() const { return freeSlotsCount != 0; }

			uint32 slotIndexFromPointer(void* p, const uint16 slotsCount, const uint32 slotsSize) const
			{
				BE_ASSERT(p > blockDataEnd(slotsCount, slotsSize) || p < blockData(slotsCount), "p does not belong to block!");
				return (blockDataEnd(slotsCount, slotsSize) - static_cast<byte*>(p)) / slotsSize;
			}
			
		public:
			Block(uint16 slotsCount, uint32 slotsSize, uint64& allocatedSize, GTSL::AllocatorReference* allocatorReference);

			void FreeBlock(uint16 slotsCount, uint32 slotsSize, uint64& freedSpace, GTSL::AllocatorReference* allocatorReference);

			bool DoesAllocationBelongToBlock(void* p, uint16 slotsCount, const uint32 slotsSize) const { return p > blockData(slotsCount) && p < blockDataEnd(slotsCount, slotsSize); }

			void AllocateInBlock(uint64 alignment, void** data, uint64& allocatedSize, uint16 slotsCount, uint32 slotsSize);

			bool TryAllocateInBlock(uint64 alignment, void** data, uint64& allocatedSize, uint16 slotsCount, uint32 slotsSize);

			void DeallocateInBlock(uint64 alignment, void* data, const uint16 slotsCount, const uint32 slotsSize)
			{
				mutex.Lock();
				placeFreeSlot(slotIndexFromPointer(data, slotsCount, slotsSize));
				mutex.Unlock();
			}
		};

		GTSL::Vector<Block> blocks;
		GTSL::ReadWriteMutex blocksMutex;
		std::atomic<uint32> index{ 0 };
		const uint16 slotsCount{ 0 };
		const uint32 slotsSize{ 0 };
		
	public:
		Pool(uint16 slotsCount, uint32 slotsSize, uint8 blockCount, uint64& allocatedSize, GTSL::AllocatorReference* allocatorReference);

		void Free(uint64& freedBytes, GTSL::AllocatorReference* allocatorReference);

		void Allocate(uint64 size, uint64 alignment, void** data, uint64* allocatedSize, uint64& allocatorAllocatedBytes, GTSL::AllocatorReference* allocatorReference);

		void Deallocate(uint64 size, uint64 alignment, void* memory, GTSL::AllocatorReference* allocatorReference);
	};
	
	GTSL::Vector<Pool> pools;
public:
	PowerOf2PoolAllocator(GTSL::AllocatorReference* allocatorReference);

	~PowerOf2PoolAllocator()
	{
		Free();
	}

	void Free();

	void Allocate(uint64 size, uint64 alignment, void** memory, uint64* allocatedSize, const char* name);

	void Deallocate(uint64 size, uint64 alignment, void* memory, const char* name);
};
