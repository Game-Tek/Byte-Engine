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
	allocatedSize = GTSL::Math::PowerOf2RoundUp(size, alignment);
	*data = GTSL::AlignPointer(alignment, at); at += allocatedSize;
}

bool StackAllocator::Block::TryAllocateInBlock(const uint64 size, const uint64 alignment, void** data, uint64& allocatedSize)
{
	allocatedSize = GTSL::Math::PowerOf2RoundUp(size, alignment);
	if (at + allocatedSize < end)
	{
		*data = GTSL::AlignPointer(alignment, at);
		at += allocatedSize;
		return true;
	}
	return false;
}

void StackAllocator::Block::Clear() { at = start; }

StackAllocator::StackAllocator(BE::SystemAllocatorReference* allocatorReference, const uint8 stackCount, const uint8 defaultBlocksPerStackCount, const uint64 blockSizes) :
	blockSize(blockSizes), stacks(stackCount, *allocatorReference), stacksMutexes(stackCount, *allocatorReference), allocatorReference(allocatorReference), MAX_STACKS(stackCount)
{
	uint64 allocated_size = 0;

	for (uint8 stack = 0; stack < stackCount; ++stack)
	{
		stacks.EmplaceBack(defaultBlocksPerStackCount, *allocatorReference);

		for (uint32 block = 0; block < defaultBlocksPerStackCount; ++block)
		{
			stacks[stack].EmplaceBack(); //construct a default block

			stacks[stack][block].AllocateBlock(blockSizes, allocatorReference, allocated_size);

			if constexpr (BE_DEBUG)
			{
				GTSL::Lock<GTSL::Mutex> lock(debugDataMutex);
				++allocatorAllocationsCount;
				++totalAllocatorAllocationsCount;
				allocatorAllocatedBytes += allocated_size;
				totalAllocatorAllocatedBytes += allocated_size;
			}
		}

		stacksMutexes.EmplaceBack();
	}
}

StackAllocator::~StackAllocator()
{
}

#if BE_DEBUG
void StackAllocator::GetDebugData(DebugData& debugData)
{
	GTSL::Lock<GTSL::Mutex> lock(debugDataMutex);

	debugData.BlockMisses = blockMisses;

	debugData.PerNameAllocationsData = perNameData;

	debugData.AllocationsCount = allocationsCount;
	debugData.TotalAllocationsCount = totalAllocationsCount;

	debugData.DeallocationsCount = deallocationsCount;
	debugData.TotalDeallocationsCount = totalDeallocationsCount;

	debugData.BytesAllocated = bytesAllocated;
	debugData.TotalBytesAllocated = totalBytesAllocated;

	debugData.BytesDeallocated = bytesDeallocated;
	debugData.TotalBytesDeallocated = totalBytesDeallocated;

	debugData.AllocatorAllocationsCount = allocatorAllocationsCount;
	debugData.TotalAllocatorAllocationsCount = totalAllocatorAllocationsCount;

	debugData.AllocatorDeallocationsCount = allocatorDeallocationsCount;
	debugData.TotalAllocatorDeallocationsCount = totalAllocatorDeallocationsCount;

	debugData.AllocatorAllocatedBytes = allocatorAllocatedBytes;
	debugData.TotalAllocatorAllocatedBytes = totalAllocatorAllocatedBytes;

	debugData.AllocatorDeallocatedBytes = allocatorDeallocatedBytes;
	debugData.TotalAllocatorDeallocatedBytes = totalAllocatorDeallocatedBytes;

	for (auto& e : perNameData)
	{
		e.second.DeallocationCount = 0;
		e.second.AllocationCount = 0;
		e.second.BytesAllocated = 0;
		e.second.BytesDeallocated = 0;
	}

	blockMisses = 0;

	bytesAllocated = 0;
	bytesDeallocated = 0;

	allocationsCount = 0;
	deallocationsCount = 0;

	allocatorAllocationsCount = 0;
	allocatorDeallocationsCount = 0;

	allocatorAllocatedBytes = 0;
	allocatorDeallocatedBytes = 0;
}
#endif

void StackAllocator::Clear()
{
	for (auto& stack : stacks)
	{
		for (auto& block : stack)
		{
			block.Clear();
		}
	}
}

void StackAllocator::LockedClear()
{
	for (auto& stack : stacks)
	{
		for (auto& block : stack)
		{
			const auto i = static_cast<uint32>(&stack - stacks.begin());
			stacksMutexes[i].Lock();
			block.Clear();
			stacksMutexes[i].Unlock();
		}
	}
}

void StackAllocator::Allocate(const uint64 size, const uint64 alignment, void** memory, uint64* allocatedSize, const char* name)
{
	const auto i{ stackIndex % MAX_STACKS }; ++stackIndex;

	BE_ASSERT((alignment & (alignment - 1)) == 0, "Alignment is not power of two!")
	BE_ASSERT(size <= blockSize, "Single allocation is larger than block sizes! An allocation larger than block size can't happen.")

	uint64 allocated_size{0};

	if constexpr (BE_DEBUG)
	{
		GTSL::Lock<GTSL::Mutex> lock(debugDataMutex);
		perNameData.try_emplace(GTSL::Id64(name)).first->second.Name = name;
	}

	stacksMutexes[i].Lock();
	for (auto& block : stacks[i])
	{
		if (block.TryAllocateInBlock(size, alignment, memory, allocated_size))
		{
			stacksMutexes[i].Unlock();
			*allocatedSize = allocated_size;

			if constexpr (BE_DEBUG)
			{
				GTSL::Lock<GTSL::Mutex> lock(debugDataMutex);
				perNameData[GTSL::Id64(name)].BytesAllocated += allocated_size;
				perNameData[GTSL::Id64(name)].AllocationCount += 1;
				bytesAllocated += allocated_size;
				totalBytesAllocated += allocated_size;
				++allocationsCount;
				++totalAllocationsCount;
			}

			return;
		}

		if constexpr (BE_DEBUG)
		{
			debugDataMutex.Lock();
			++blockMisses;
			debugDataMutex.Unlock();
		}
	}

	const auto last_block = stacks[i].EmplaceBack();
	stacks[i][last_block].AllocateBlock(blockSize, allocatorReference, allocated_size);
	stacks[i][last_block].AllocateInBlock(size, alignment, memory, allocated_size);
	stacksMutexes[i].Unlock();
	
	*allocatedSize = allocated_size;

	if constexpr (BE_DEBUG)
	{
		GTSL::Lock<GTSL::Mutex> lock(debugDataMutex);
		perNameData[GTSL::Id64(name)].BytesAllocated += allocated_size;
		perNameData[GTSL::Id64(name)].AllocationCount += 1;
		bytesAllocated += allocated_size;
		totalBytesAllocated += allocated_size;
		allocatorAllocatedBytes += allocated_size;
		totalAllocatorAllocatedBytes += allocated_size;
		++allocatorAllocationsCount;
		++totalAllocatorAllocationsCount;
		++allocationsCount;
		++totalAllocationsCount;
	}
}

void StackAllocator::Deallocate(const uint64 size, const uint64 alignment, void* memory, const char* name)
{
	BE_ASSERT((alignment & (alignment - 1)) == 0, "Alignment is not power of two!")
	BE_ASSERT(size <= blockSize, "Deallocation size is larger than block size! An allocation larger than block size can't happen. Trying to deallocate more bytes than allocated!")

	if constexpr (BE_DEBUG)
	{
		const auto bytes_deallocated{ GTSL::Math::PowerOf2RoundUp(size, alignment) };

		GTSL::Lock<GTSL::Mutex> lock(debugDataMutex);
		perNameData[GTSL::Id64(name)].BytesDeallocated += bytes_deallocated;
		perNameData[GTSL::Id64(name)].DeallocationCount += 1;
		bytesDeallocated += bytes_deallocated;
		totalBytesDeallocated += bytes_deallocated;
		++deallocationsCount;
		++totalDeallocationsCount;
	}
}

void StackAllocator::Free()
{
	uint64 freed_bytes{ 0 };
	
	for(auto& stack : stacks)
	{
		for(auto& block : stack)
		{
			block.DeallocateBlock(allocatorReference, freed_bytes);
			if constexpr (BE_DEBUG)
			{
				++allocatorDeallocationsCount;
				++totalAllocatorDeallocationsCount;
			}
		}
	}
	
	if constexpr (BE_DEBUG)
	{
		allocatorDeallocatedBytes += freed_bytes;
		totalAllocatorDeallocatedBytes += freed_bytes;
	}
}
