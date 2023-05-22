#include "StackAllocator.h"

#include <GTSL/Math/Math.hpp>
#include "ByteEngine/Debug/Assert.h"

void StackAllocator::Block::initialize(const GTSL::uint64 minimumSize, BE::SystemAllocatorReference allocatorReference, GTSL::uint64& allocatedSize)
{
	GTSL::uint64 allocated_size{ 0 };

	allocatorReference.Allocate(minimumSize, alignof(GTSL::uint8), reinterpret_cast<void**>(&start), &allocated_size);

	allocatedSize = allocated_size;

	at = start;
	end = start + allocated_size;
}

void StackAllocator::Block::deinitialize(BE::SystemAllocatorReference allocatorReference, GTSL::uint64& deallocatedBytes) const
{
	allocatorReference.Deallocate(end - start, alignof(GTSL::uint8), start);
	deallocatedBytes += end - start;
}

void StackAllocator::Block::AllocateInBlock(const GTSL::uint64 size, const GTSL::uint64 alignment, void** data, GTSL::uint64& allocatedSize)
{
	allocatedSize = GTSL::Math::RoundUpByPowerOf2(size, alignment);
	*data = GTSL::AlignPointer(alignment, at); at += allocatedSize;
}

bool StackAllocator::Block::TryAllocateInBlock(const GTSL::uint64 size, const GTSL::uint64 alignment, void** data, GTSL::uint64& allocatedSize)
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

void StackAllocator::Block::clear() { at = start; }