#include "PoolAllocator.h"

#include <GTSL/Bitscan.h>
#include <GTSL/Math/Math.hpp>
#include <new>
#include <GTSL/Memory.h>

#include "Application.h"
#include "Byte Engine/Debug/Assert.h"

uint32 PoolAllocator::Pool::Block::slotIndexFromPointer(void* p, const uint16 slotsCount, const uint32 slotsSize) const
{
	BE_ASSERT(p > blockDataEnd(slotsCount, slotsSize) || p < blockData(slotsCount), "p does not belong to block!");
	return static_cast<uint32>((blockDataEnd(slotsCount, slotsSize) - static_cast<byte*>(p))) / slotsSize;
}

bool PoolAllocator::Pool::Block::DoesAllocationBelongToBlock(void* data, const uint16 slotsCount, const uint32 slotsSize) const
{
	return data > blockData(slotsCount) && data < blockDataEnd(slotsCount, slotsSize);
}

PoolAllocator::Pool::Block::Block(const uint16 slotsCount, const uint32 slotsSize, uint64& allocatedSize, GTSL::AllocatorReference* allocatorReference) : freeSlotsCount(slotsCount)
{
	const auto free_indeces_stack_space = slotsCount * sizeof(uint32);
	const auto block_data_space = slotsSize * static_cast<uint64>(slotsCount);

	allocatorReference->Allocate(free_indeces_stack_space + block_data_space, alignof(uint64), reinterpret_cast<void**>(&data), &allocatedSize);

	for (uint32 i = 0; i < slotsCount; ++i) { freeSlotsIndices()[i] = i; }
}

void PoolAllocator::Pool::Block::FreeBlock(const uint16 slotsCount, const uint32 slotsSize, uint64& freedSpace, GTSL::AllocatorReference* allocatorReference)
{
	freedSpace = slotsSize * static_cast<uint64>(slotsCount) + slotsCount * sizeof(uint32);
	allocatorReference->Deallocate(freedSpace, alignof(uint32), data);
	//mutex.Lock();
	data = nullptr;
	//mutex.Unlock();
}

void PoolAllocator::Pool::Block::AllocateInBlock(const uint64 alignment, void** data, uint64& allocatedSize, const uint16 slotsCount, const uint32 slotsSize)
{
	uint32 free_slot{ 0 };
	//mutex.Lock();
	popFreeSlot(free_slot);
	//mutex.Unlock();
	*data = GTSL::Memory::AlignedPointer(alignment, blockData(slotsCount) + free_slot * static_cast<uint64>(slotsSize));
	allocatedSize = slotsSize - ((blockData(slotsCount) + (static_cast<uint64>(free_slot) + 1ull) * static_cast<uint64>(slotsSize)) - static_cast<byte*>(*data));
}

bool PoolAllocator::Pool::Block::TryAllocateInBlock(const uint64 alignment, void** data, uint64& allocatedSize, const uint16 slotsCount, const uint32 slotsSize)
{
	uint32 free_slot{ 0 };
	//mutex.Lock();
	if (freeSlot()) [[likely]]
	{
		popFreeSlot(free_slot);
		//mutex.Unlock();
		*data = GTSL::Memory::AlignedPointer(alignment, blockData(slotsCount) + free_slot * static_cast<uint64>(slotsSize));
		allocatedSize = slotsSize - ((blockData(slotsCount) + (static_cast<uint64>(free_slot) + 1ull) * static_cast<uint64>(slotsSize)) - static_cast<byte*>(*data));
		return true;
	}
	//mutex.Unlock();
	return false;
}

void PoolAllocator::Pool::Block::DeallocateInBlock(uint64 alignment, void* data, const uint16 slotsCount, const uint32 slotsSize)
{
	placeFreeSlot(slotIndexFromPointer(data, slotsCount, slotsSize));
}

uint32 PoolAllocator::Pool::allocateAndAddNewBlock(GTSL::AllocatorReference* allocatorReference)
{
	uint64 allocated_size{ 0 };
	void* new_data{ nullptr };
	allocatorReference->Allocate(sizeof(Block) * blockCapacity * 2, alignof(Block), &new_data, &allocated_size);
	GTSL::Memory::CopyMemory(blockCount * sizeof(Block), blocks, new_data);
	allocatorReference->Deallocate(blockCapacity * sizeof(Block), alignof(Block), blocks);
	//blockCapacity.store(allocated_size / sizeof(Block), std::memory_order::memory_order_seq_cst);
	//blocks.store(static_cast<Block*>(new_data), std::memory_order::memory_order_seq_cst);
	//
	blockCapacity = allocated_size / sizeof(Block);
	blocks = static_cast<Block*>(new_data);
	return ++blockCount;
}

PoolAllocator::Pool::Pool(const uint16 slotsCount, const uint32 slotsSize, const uint8 blockCount, uint64& allocatedSize, GTSL::AllocatorReference* allocatorReference) : blockCount(blockCount), blockCapacity(blockCount), slotsSize(slotsSize), slotsCount(slotsCount)
{
	allocatorReference->Allocate(sizeof(Block) * blockCount, alignof(Block), reinterpret_cast<void**>(&blocks), &allocatedSize);

	uint64 block_allocation_size{ 0 };
	for (uint32 i = 0; i < blockCount; ++i)
	{
		::new(blocks + i) Block(slotsCount, slotsSize, block_allocation_size, allocatorReference);
		allocatedSize += block_allocation_size;
	}
}

void PoolAllocator::Pool::Allocate(const uint64 size, const uint64 alignment, void** data, uint64* allocatedSize, uint64& allocatorAllocatedBytes, GTSL::AllocatorReference* allocatorReference)
{
	BE_ASSERT(size > slotsSize, "Allocation size greater than pool's slot size")
	BE_ASSERT(GTSL::Math::AlignedNumber(alignment, size) > slotsSize, "Aligned allocation size greater than pool's slot size")

	const auto i{ index % blockCount };
	
	++index;

	mutex.ReadLock();
	for (uint32 j = 0; j < blockCount; ++j)
	{
		if (blocks[(i + j) % blockCount].TryAllocateInBlock(alignment, data, *allocatedSize, slotsCount, slotsSize))
		{
			mutex.ReadUnlock();
			return;
		}
	}
	mutex.ReadUnlock();

	mutex.WriteLock();
	const auto new_block_index{ allocateAndAddNewBlock(allocatorReference) };
	mutex.WriteUnlock();

	mutex.ReadLock();
	blocks[new_block_index].AllocateInBlock(alignment, data, *allocatedSize, slotsCount, slotsSize);
	mutex.ReadUnlock();
}

void PoolAllocator::Pool::Deallocate(uint64 size, const uint64 alignment, void* memory, GTSL::AllocatorReference* allocatorReference) const
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

void PoolAllocator::Pool::Free(uint64& freedBytes, GTSL::AllocatorReference* allocatorReference) const
{
	for (auto& block : blocksRange()) { block.FreeBlock(slotsCount, slotsSize, freedBytes, allocatorReference); }
}

PoolAllocator::PoolAllocator(GTSL::AllocatorReference* allocatorReference): systemAllocatorReference(allocatorReference), poolCount(15)
{
	uint64 allocated_size{ 0 }; //debug

	uint64 alloc_size{ 0 };
	allocatorReference->Allocate(sizeof(Pool) * poolCount, alignof(Pool), reinterpret_cast<void**>(&pools), &alloc_size);
	
	for (uint32 i = 0, j = poolCount; i < poolCount; ++i, --j)
	{
		const auto slot_count = j * poolCount; //pools with smaller slot sizes get more slots
		const auto slot_size = 1 << i;
		
		::new(pools + i) Pool(slot_count, slot_size, j/*block count, pools of smaller sizes get more blocks*/, alloc_size, allocatorReference);
		allocated_size += alloc_size;
	}
}

void PoolAllocator::Allocate(const uint64 size, const uint64 alignment, void** memory, uint64* allocatedSize, const char* name) const
{
	BE_ASSERT((alignment & (alignment - 1)) != 0, "Alignment is not power of two!")
	
	uint64 allocation_min_size{ 0 }; GTSL::NextPowerOfTwo(size, allocation_min_size);
	
	BE_ASSERT((allocation_min_size& (allocation_min_size - 1)) != 0, "allocation_min_size is not power of two!")
	
	uint8 set_bit{ 0 }; GTSL::BitScanForward(allocation_min_size, set_bit);
	
	uint64 allocator_bytes{ 0 };
	pools[set_bit].Allocate(size, alignment, memory, allocatedSize, allocator_bytes, systemAllocatorReference);
}

void PoolAllocator::Deallocate(const uint64 size, const uint64 alignment, void* memory, const char* name) const
{
	BE_ASSERT((alignment & (alignment - 1)) != 0, "Alignment is not power of two!")

	uint64 allocation_min_size{0}; GTSL::NextPowerOfTwo(size, allocation_min_size);

	BE_ASSERT((allocation_min_size& (allocation_min_size - 1)) != 0, "allocation_min_size is not power of two!")

	uint8 set_bit{0}; GTSL::BitScanForward(allocation_min_size, set_bit);

	pools[set_bit].Deallocate(size, alignment, memory, systemAllocatorReference);
}

void PoolAllocator::Free() const
{
	uint64 freed_bytes{ 0 };

	for (auto& pool : GTSL::Ranger(poolCount, pools))
	{
		pool.Free(freed_bytes, systemAllocatorReference);
	}
}
