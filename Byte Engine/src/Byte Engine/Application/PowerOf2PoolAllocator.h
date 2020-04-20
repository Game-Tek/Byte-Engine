#pragma once

#include "Byte Engine/Core.h"

#include <GTSL/KeepVector.h>
#include <GTSL/Math/Math.hpp>
#include <GTSL/Bitscan.h>

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
				freeSlot = freeSlotsIndices()[0];
				--freeSlotsCount;
				GTSL::Memory::CopyMemory(sizeof(uint32) * freeSlotsCount, freeSlotsIndices() + 1, freeSlotsIndices());
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
			Block(const uint16 slotsCount, const uint32 slotsSize, uint64& allocatedSize, GTSL::AllocatorReference* allocatorReference)
			{				
				const uint64 free_indeces_stack_space = slotsCount * sizeof(uint32);
				const uint64 block_data_space = slotsSize * slotsCount;
				
				allocatorReference->Allocate(free_indeces_stack_space + block_data_space, alignof(uint32), reinterpret_cast<void**>(&data), &allocatedSize);
				
				for(uint32 i = 0; i < slotsCount; ++i)
				{
					freeSlotsIndices()[i] = i;
				}
			}

			void FreeBlock(const uint32 slotsSize, const uint16 slotsCount, uint64& freedSpace, GTSL::AllocatorReference* allocatorReference)
			{
				freedSpace = slotsSize * slotsCount + slotsCount * sizeof(uint32);
				allocatorReference->Deallocate(freedSpace, alignof(uint32), data);
				data = nullptr;
			}
			
			bool DoesAllocationBelongToBlock(void* p, const uint16 slotsCount, const uint32 slotsSize) const { return p > blockData(slotsCount) && p < blockDataEnd(slotsCount, slotsSize); }

			void AllocateInBlock(const uint64 alignment, void** data, uint64& allocatedSize, const uint16 slotsCount, const uint32 slotsSize)
			{
				uint32 free_slot{ 0 };
				mutex.Lock();
				popFreeSlot(free_slot);
				mutex.Unlock();
				*data = GTSL::Memory::AlignedPointer(alignment, blockData(slotsCount) + free_slot * slotsSize);
				allocatedSize = slotsSize - ((blockData(slotsCount) + (free_slot + 1) * slotsSize) - reinterpret_cast<byte*>(*data));
			}

			bool TryAllocateInBlock(const uint64 alignment, void** data, uint64& allocatedSize, const uint16 slotsCount, const uint32 slotsSize)
			{
				uint32 free_slot{ 0 };
				mutex.Lock();
				if (freeSlot())
				{
					popFreeSlot(free_slot);
					mutex.Unlock();
					*data = GTSL::Memory::AlignedPointer(alignment, blockData(slotsCount) + free_slot * slotsSize);
					allocatedSize = slotsSize - ((blockData(slotsCount) + (free_slot + 1) * slotsSize) - reinterpret_cast<byte*>(*data));
					return true;
				}
				mutex.Unlock();
				return false;
			}

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
		Pool(uint16 slotsCount, uint32 slotsSize, const uint8 blockCount, uint64& allocatedSize, GTSL::AllocatorReference* allocatorReference) : blocks(blockCount, allocatorReference)
		{	
			for(uint8 i = 0; i < blockCount; ++i)
			{
				blocks.EmplaceBack(slotsCount, slotsSize, allocatedSize, allocatorReference);
			}
		}
		
		void Allocate(const uint64 size, const uint64 alignment, void** data, uint64* allocatedSize, GTSL::AllocatorReference* allocatorReference)
		{
			BE_ASSERT(size > slotsSize, "Allocation size greater than pool's slot size")
			BE_ASSERT(GTSL::Math::AlignedNumber(alignment, size) > slotsSize, "Aligned allocation size greater than pool's slot size")
			
			blocksMutex.ReadLock();
			const auto i{ index % blocks.GetLength() };
			blocksMutex.ReadUnlock();
			++index;

			blocksMutex.ReadLock();
			for(uint32 j = 0; j < blocks.GetLength(); ++j)
			{
				if (blocks[(i + j) % blocks.GetLength()].TryAllocateInBlock(alignment, data, *allocatedSize,slotsCount, slotsSize)) { blocksMutex.ReadUnlock(); return; }
			}
			blocksMutex.ReadUnlock();
			
			blocksMutex.WriteLock();
			blocks[blocks.EmplaceBack(slotsSize, slotsCount, allocatorReference)].AllocateInBlock(alignment, data, *allocatedSize, slotsCount, slotsSize);
			blocksMutex.WriteUnlock();
		}

		void Deallocate(uint64 size, const uint64 alignment, void* memory, GTSL::AllocatorReference* allocatorReference)
		{
			blocksMutex.ReadLock();
			for(auto& e : blocks)
			{
				if(e.DoesAllocationBelongToBlock(memory, slotsCount, slotsSize))
				{
					e.DeallocateInBlock(alignment, memory, slotsCount, slotsSize);
					blocksMutex.ReadUnlock();
					return;
				}
			}
			blocksMutex.ReadUnlock();

			BE_ASSERT(true, "Allocation couldn't be freed from this pool, pointer does not belong to any allocation in this pool!")
		}
	};
	
	GTSL::Vector<Pool> pools;
public:
	PowerOf2PoolAllocator(GTSL::AllocatorReference* allocatorReference) : allocatorReference(allocatorReference)
	{
		const auto max_power_of_two_allocatable = 10;

		uint64 allocated_size{ 0 }; //debug
		
		for(uint32 i = max_power_of_two_allocatable; i > 0; --i)
		{
			pools.EmplaceBack(i * max_power_of_two_allocatable, 1 << i, i, allocated_size, allocatorReference);
		}
	}
	
	void Allocate(const uint64 size, const uint64 alignment, void** memory, uint64* allocatedSize, const char* name)
	{
		BE_ASSERT((alignment & (alignment - 1)) != 0, "Alignment is not power of two!")

		uint64 allocation_min_size{ 0 };
		GTSL::NextPowerOfTwo(size, allocation_min_size);
		
		BE_ASSERT((allocation_min_size& (allocation_min_size - 1)) != 0, "allocation_min_size is not power of two!")
		
		uint8 set_bit{ 0 };
		GTSL::BitScanForward(allocation_min_size, set_bit);

		pools[set_bit].Allocate(size, alignment, memory, allocatedSize, allocatorReference);
	}

	void Deallocate(const uint64 size, const uint64 alignment, void* memory, const char* name)
	{
		BE_ASSERT((alignment & (alignment - 1)) != 0, "Alignment is not power of two!")

		const uint64 allocation_min_size{ GTSL::Math::AlignedNumber(size, alignment) };
		BE_ASSERT((allocation_min_size& (allocation_min_size - 1)) != 0, "allocation_min_size is not power of two!")
		
		uint8 set_bit{ 0 };
		GTSL::BitScanForward(allocation_min_size, set_bit);

		pools[set_bit].Deallocate(size, alignment, memory, allocatorReference);
	}

	
};
