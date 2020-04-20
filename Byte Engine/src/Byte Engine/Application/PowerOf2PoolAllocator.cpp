#include "PowerOf2PoolAllocator.h"

#include <GTSL/Bitscan.h>
#include <GTSL/Math/Math.hpp>

PowerOf2PoolAllocator::Pool::Block::Block(const uint16 slotsCount, const uint32 slotsSize, uint64& allocatedSize, GTSL::AllocatorReference* allocatorReference)
{
	const uint64 free_indeces_stack_space = slotsCount * sizeof(uint32);
	const uint64 block_data_space = slotsSize * slotsCount;

	allocatorReference->Allocate(free_indeces_stack_space + block_data_space, alignof(uint32), reinterpret_cast<void**>(&data), &allocatedSize);

	for (uint32 i = 0; i < slotsCount; ++i)
	{
		freeSlotsIndices()[i] = i;
	}
}

void PowerOf2PoolAllocator::Pool::Block::FreeBlock(const uint16 slotsCount, const uint32 slotsSize, uint64& freedSpace, GTSL::AllocatorReference* allocatorReference)
{
	freedSpace = slotsSize * slotsCount + slotsCount * sizeof(uint32);
	allocatorReference->Deallocate(freedSpace, alignof(uint32), data);
	data = nullptr;
}

void PowerOf2PoolAllocator::Pool::Block::AllocateInBlock(const uint64 alignment, void** data, uint64& allocatedSize, const uint16 slotsCount, const uint32 slotsSize)
{
	uint32 free_slot{0};
	mutex.Lock();
	popFreeSlot(free_slot);
	mutex.Unlock();
	*data = GTSL::Memory::AlignedPointer(alignment, blockData(slotsCount) + free_slot * slotsSize);
	allocatedSize = slotsSize - ((blockData(slotsCount) + (free_slot + 1) * slotsSize) - reinterpret_cast<byte*>(*data));
}

bool PowerOf2PoolAllocator::Pool::Block::TryAllocateInBlock(const uint64 alignment, void** data, uint64& allocatedSize, const uint16 slotsCount, const uint32 slotsSize)
{
	uint32 free_slot{0};
	mutex.Lock();
	if (freeSlot())
	{
		popFreeSlot(free_slot);
		mutex.Unlock();
		*data = GTSL::Memory::AlignedPointer(alignment, blockData(slotsCount) + free_slot * slotsSize);
		allocatedSize = slotsSize - ((blockData(slotsCount) + (free_slot + 1) * slotsSize) - reinterpret_cast<byte*>(*
			data));
		return true;
	}
	mutex.Unlock();
	return false;
}

PowerOf2PoolAllocator::Pool::Pool(uint16 slotsCount, uint32 slotsSize, const uint8 blockCount, uint64& allocatedSize, GTSL::AllocatorReference* allocatorReference): blocks(blockCount, allocatorReference)
{
	for (uint8 i = 0; i < blockCount; ++i)
	{
		blocks.EmplaceBack(slotsCount, slotsSize, allocatedSize, allocatorReference);
	}
}

void PowerOf2PoolAllocator::Pool::Free(uint64& freedBytes, GTSL::AllocatorReference* allocatorReference)
{
	for (auto& block : blocks)
	{
		block.FreeBlock(slotsCount, slotsSize, freedBytes, allocatorReference);
	}
}

void PowerOf2PoolAllocator::Pool::Allocate(const uint64 size, const uint64 alignment, void** data, uint64* allocatedSize, uint64& allocatorAllocatedBytes, GTSL::AllocatorReference* allocatorReference)
{
	BE_ASSERT(size > slotsSize, "Allocation size greater than pool's slot size")
	BE_ASSERT(GTSL::Math::AlignedNumber(alignment, size) > slotsSize, "Aligned allocation size greater than pool's slot size")

	blocksMutex.ReadLock();
	const auto i{index % blocks.GetLength()};

	++index;

	for (uint32 j = 0; j < blocks.GetLength(); ++j)
	{
		if (blocks[(i + j) % blocks.GetLength()].TryAllocateInBlock(alignment, data, *allocatedSize, slotsCount, slotsSize))
		{
			blocksMutex.ReadUnlock();
			return;
		}
	}
	blocksMutex.ReadUnlock();

	blocksMutex.WriteLock();
	blocks[blocks.EmplaceBack(slotsSize, slotsCount, allocatorAllocatedBytes, allocatorReference)].AllocateInBlock(alignment, data, *allocatedSize, slotsCount, slotsSize);
	blocksMutex.WriteUnlock();
}

void PowerOf2PoolAllocator::Pool::Deallocate(uint64 size, const uint64 alignment, void* memory, GTSL::AllocatorReference* allocatorReference)
{
	blocksMutex.ReadLock();
	for (auto& e : blocks)
	{
		if (e.DoesAllocationBelongToBlock(memory, slotsCount, slotsSize))
		{
			e.DeallocateInBlock(alignment, memory, slotsCount, slotsSize);
			blocksMutex.ReadUnlock();
			return;
		}
	}
	blocksMutex.ReadUnlock();

	BE_ASSERT(true, "Allocation couldn't be freed from this pool, pointer does not belong to any allocation in this pool!")
}

PowerOf2PoolAllocator::PowerOf2PoolAllocator(GTSL::AllocatorReference* allocatorReference): allocatorReference(allocatorReference)
{
	const auto max_power_of_two_allocatable = 10;

	uint64 allocated_size{0}; //debug

	for (uint32 i = max_power_of_two_allocatable; i > 0; --i)
	{
		pools.EmplaceBack(i * max_power_of_two_allocatable, 1 << i, i, allocated_size, allocatorReference);
	}
}

void PowerOf2PoolAllocator::Free()
{
	uint64 freed_bytes{0};

	for (auto& pool : pools)
	{
		pool.Free(freed_bytes, allocatorReference);
	}
}

void PowerOf2PoolAllocator::Allocate(const uint64 size, const uint64 alignment, void** memory, uint64* allocatedSize, const char* name)
{
	BE_ASSERT((alignment & (alignment - 1)) != 0, "Alignment is not power of two!")

	uint64 allocation_min_size{0};
	GTSL::NextPowerOfTwo(size, allocation_min_size);

	BE_ASSERT((allocation_min_size& (allocation_min_size - 1)) != 0, "allocation_min_size is not power of two!")

	uint8 set_bit{0};
	GTSL::BitScanForward(allocation_min_size, set_bit);

	uint64 allocator_bytes{0};

	pools[set_bit].Allocate(size, alignment, memory, allocatedSize, allocator_bytes, allocatorReference);
}

void PowerOf2PoolAllocator::Deallocate(const uint64 size, const uint64 alignment, void* memory, const char* name)
{
	BE_ASSERT((alignment & (alignment - 1)) != 0, "Alignment is not power of two!")

	uint64 allocation_min_size{0};
	GTSL::NextPowerOfTwo(size, allocation_min_size);

	BE_ASSERT((allocation_min_size& (allocation_min_size - 1)) != 0, "allocation_min_size is not power of two!")

	uint8 set_bit{0};
	GTSL::BitScanForward(allocation_min_size, set_bit);

	pools[set_bit].Deallocate(size, alignment, memory, allocatorReference);
}
