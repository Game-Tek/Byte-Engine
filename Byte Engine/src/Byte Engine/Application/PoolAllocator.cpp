#include "PoolAllocator.h"

#include <GTSL/Bitscan.h>
#include <GTSL/Math/Math.hpp>
#include <new>
#include <GTSL/Memory.h>

PoolAllocator::Pool::Block::Block(const uint16 slotsCount, const uint32 slotsSize, uint64& allocatedSize, GTSL::AllocatorReference* allocatorReference)
{
	const uint64 free_indeces_stack_space = slotsCount * sizeof(uint32);
	const uint64 block_data_space = slotsSize * slotsCount;

	allocatorReference->Allocate(free_indeces_stack_space + block_data_space, alignof(uint32), reinterpret_cast<void**>(&data), &allocatedSize);

	for (uint32 i = 0; i < slotsCount; ++i)
	{
		freeSlotsIndices()[i] = i;
	}
}

void PoolAllocator::Pool::Block::FreeBlock(const uint16 slotsCount, const uint32 slotsSize, uint64& freedSpace, GTSL::AllocatorReference* allocatorReference)
{
	freedSpace = slotsSize * slotsCount + slotsCount * sizeof(uint32);
	allocatorReference->Deallocate(freedSpace, alignof(uint32), data);
	data = nullptr;
}

void PoolAllocator::Pool::Block::AllocateInBlock(const uint64 alignment, void** data, uint64& allocatedSize, const uint16 slotsCount, const uint32 slotsSize)
{
	uint32 free_slot{ 0 };
	popFreeSlot(free_slot);
	*data = GTSL::Memory::AlignedPointer(alignment, blockData(slotsCount) + free_slot * slotsSize);
	allocatedSize = slotsSize - ((blockData(slotsCount) + (free_slot + 1) * slotsSize) - static_cast<byte*>(*data));
}

bool PoolAllocator::Pool::Block::TryAllocateInBlock(const uint64 alignment, void** data, uint64& allocatedSize, const uint16 slotsCount, const uint32 slotsSize)
{
	uint32 free_slot{ 0 };
	if (freeSlot())
	{
		popFreeSlot(free_slot);
		*data = GTSL::Memory::AlignedPointer(alignment, blockData(slotsCount) + free_slot * slotsSize);
		allocatedSize = slotsSize - ((blockData(slotsCount) + (free_slot + 1) * slotsSize) - static_cast<byte*>(*data));
		return true;
	}
	return false;
}

uint32 PoolAllocator::Pool::allocateAndAddNewBlock(GTSL::AllocatorReference* allocatorReference)
{
	uint64 allocated_size{ 0 };
	void* new_data{ nullptr };
	allocatorReference->Allocate(sizeof(Block) * blockCapacity * 2, alignof(Block), &new_data, &allocated_size);
	GTSL::Memory::CopyMemory(blockCount * sizeof(Block), blocks, new_data);
	allocatorReference->Deallocate(blockCapacity * sizeof(Block), alignof(Block), blocks);
	blockCapacity = allocated_size / sizeof(Block);
	blocks = static_cast<Block*>(new_data);
	return ++blockCount;
}

PoolAllocator::Pool::Pool(const uint16 slotsCount, const uint32 slotsSize, const uint8 blockCount, uint64& allocatedSize, GTSL::AllocatorReference* allocatorReference)
{
	for (uint8 i = 0; i < blockCount; ++i) { ::new(static_cast<void*>(blocks + blockCount)) Block(slotsCount, slotsSize, allocatedSize, allocatorReference); }
}

void PoolAllocator::Pool::Free(uint64& freedBytes, GTSL::AllocatorReference* allocatorReference)
{
	for (auto& block : blocksRange()) { block.FreeBlock(slotsCount, slotsSize, freedBytes, allocatorReference); }
}

void PoolAllocator::Pool::Allocate(const uint64 size, const uint64 alignment, void** data, uint64* allocatedSize, uint64& allocatorAllocatedBytes, GTSL::AllocatorReference* allocatorReference)
{
	BE_ASSERT(size > slotsSize, "Allocation size greater than pool's slot size")
	BE_ASSERT(GTSL::Math::AlignedNumber(alignment, size) > slotsSize, "Aligned allocation size greater than pool's slot size")

	const auto i{ index % blockCount };

	++index;

	for (uint32 j = 0; j < blockCount; ++j)
	{
		if (blocks[(i + j) % blockCount].TryAllocateInBlock(alignment, data, *allocatedSize, slotsCount, slotsSize)) return;
	}
	
	blocks[allocateAndAddNewBlock(allocatorReference)].AllocateInBlock(alignment, data, *allocatedSize, slotsCount, slotsSize);
}

void PoolAllocator::Pool::Deallocate(uint64 size, const uint64 alignment, void* memory, GTSL::AllocatorReference* allocatorReference)
{;
	for (auto& e : blocksRange())
	{
		if (e.DoesAllocationBelongToBlock(memory, slotsCount, slotsSize))
		{
			e.DeallocateInBlock(alignment, memory, slotsCount, slotsSize);
			return;
		}
	}

	BE_ASSERT(true, "Allocation couldn't be freed from this pool, pointer does not belong to any allocation in this pool!")
}

PoolAllocator::PoolAllocator(GTSL::AllocatorReference* allocatorReference): systemAllocatorReference(allocatorReference)
{
	const auto max_power_of_two_allocatable = 10;

	uint64 allocated_size{ 0 }; //debug

	void* data{ nullptr };
	uint64 alloc_size{ 0 };
	allocatorReference->Allocate(sizeof(Pool) * max_power_of_two_allocatable, alignof(Pool), &data, &alloc_size);
	
	for (uint32 i = max_power_of_two_allocatable; i > 0; --i)
	{
		::new(static_cast<void*>(pools + 1)) Pool(i * max_power_of_two_allocatable, 1 << i, i, alloc_size, allocatorReference);
		allocated_size += alloc_size;
	}

	pools = static_cast<Pool*>(data);
	poolCount = max_power_of_two_allocatable;
}

void PoolAllocator::Free()
{
	uint64 freed_bytes{ 0 };

	for (auto& pool : Ranger(pools, pools + poolCount))
	{
		pool.Free(freed_bytes, systemAllocatorReference);
	}
}

void PoolAllocator::Allocate(const uint64 size, const uint64 alignment, void** memory, uint64* allocatedSize, const char* name)
{
	BE_ASSERT((alignment & (alignment - 1)) != 0, "Alignment is not power of two!")
	
	uint64 allocation_min_size{ 0 };
	GTSL::NextPowerOfTwo(size, allocation_min_size);
	
	BE_ASSERT((allocation_min_size& (allocation_min_size - 1)) != 0, "allocation_min_size is not power of two!")
	
	uint8 set_bit{ 0 };
	GTSL::BitScanForward(allocation_min_size, set_bit);
	
	uint64 allocator_bytes{ 0 };
	
	pools[set_bit].Allocate(size, alignment, memory, allocatedSize, allocator_bytes, systemAllocatorReference);
}

void PoolAllocator::Deallocate(const uint64 size, const uint64 alignment, void* memory, const char* name)
{
	BE_ASSERT((alignment & (alignment - 1)) != 0, "Alignment is not power of two!")

	uint64 allocation_min_size{0};
	GTSL::NextPowerOfTwo(size, allocation_min_size);

	BE_ASSERT((allocation_min_size& (allocation_min_size - 1)) != 0, "allocation_min_size is not power of two!")

	uint8 set_bit{0};
	GTSL::BitScanForward(allocation_min_size, set_bit);

	pools[set_bit].Deallocate(size, alignment, memory, systemAllocatorReference);
}
