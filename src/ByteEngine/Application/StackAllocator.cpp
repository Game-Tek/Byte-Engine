#include "StackAllocator.h"

#include <GTSL/Math/Math.hpp>
#include "ByteEngine/Debug/Assert.h"

void StackAllocator::Block::AllocateBlock(const uint64 minimumSize, BE::SystemAllocatorReference* allocatorReference, uint64& allocatedSize)
{
	uint64 allocated_size{ 0 };

	allocatorReference->Allocate(minimumSize, alignof(byte), reinterpret_cast<void**>(&start), &allocated_size);

	allocatedSize = allocated_size;

	at = start;
	end = start + allocated_size;
}

void StackAllocator::Block::DeallocateBlock(BE::SystemAllocatorReference* allocatorReference, uint64& deallocatedBytes) const
{
	allocatorReference->Deallocate(end - start, alignof(byte), start);
	deallocatedBytes += end - start;
}

void StackAllocator::Block::AllocateInBlock(const uint64 size, const uint64 alignment, void** data, uint64& allocatedSize)
{
	allocatedSize = GTSL::Math::RoundUpByPowerOf2(size, alignment);
	*data = GTSL::AlignPointer(alignment, at); at += allocatedSize;
}

bool StackAllocator::Block::TryAllocateInBlock(const uint64 size, const uint64 alignment, void** data, uint64& allocatedSize)
{
	allocatedSize = GTSL::Math::RoundUpByPowerOf2(size, alignment);
	if (at + allocatedSize < end)
	{
		//*data = GTSL::AlignPointer(alignment, at);
		*data = at;
		at += allocatedSize;
		return true;
	}
	return false;
}

void StackAllocator::Block::Clear() { at = start; }