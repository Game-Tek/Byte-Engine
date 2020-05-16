#include "StackAllocator.h"


#include <GTSL/String.hpp>
#include <GTSL/Math/Math.hpp>
#include "ByteEngine/Debug/Assert.h"

void StackAllocator::Block::AllocateBlock(const uint64 minimumSize, GTSL::AllocatorReference* allocatorReference,
                                          uint64& allocatedSize)
{
	uint64 allocated_size{0};

	allocatorReference->Allocate(minimumSize, alignof(byte), reinterpret_cast<void**>(&start), &allocated_size);

	allocatedSize = allocated_size;

	at = start;
	end = start + allocated_size;
}

void StackAllocator::Block::DeallocateBlock(GTSL::AllocatorReference* allocatorReference,
                                            uint64& deallocatedBytes) const
{
	allocatorReference->Deallocate(end - start, alignof(byte), start);
	deallocatedBytes = end - start;
}

void StackAllocator::Block::AllocateInBlock(const uint64 size, const uint64 alignment, void** data,
                                            uint64& allocatedSize)
{
	*data = (at += (allocatedSize = GTSL::Math::AlignedNumber(size, alignment)));
}

bool StackAllocator::Block::TryAllocateInBlock(const uint64 size, const uint64 alignment, void** data, uint64& allocatedSize)
{
	auto* const new_at = at + (allocatedSize = GTSL::Math::AlignedNumber(size, alignment));
	if (new_at < end)
	{
		*data = new_at;
		at = new_at;
		return true;
	}
	return false;
}

StackAllocator::StackAllocator(GTSL::AllocatorReference* allocatorReference, const uint8 stackCount, const uint8 defaultBlocksPerStackCount, const uint64 blockSizes) :
	blockSize(blockSizes), stacks(stackCount, allocatorReference), stacksMutexes(stackCount, allocatorReference), allocatorReference(allocatorReference), maxStacks(stackCount)
{
	uint64 allocated_size = 0;

	for (uint8 i = 0; i < stackCount; ++i)
	{
		stacks.EmplaceBack(defaultBlocksPerStackCount, allocatorReference); //construct stack [i]s

		for (uint32 j = 0; j < defaultBlocksPerStackCount; ++j) //for every block in constructed vector
		{
			stacks[i].EmplaceBack(); //construct a default block

			stacks[i][j].AllocateBlock(blockSizes, allocatorReference, allocated_size);
			//allocate constructed block, which is also current block

			BE_DEBUG_ONLY(GTSL::Lock<GTSL::Mutex> lock(debugDataMutex))

			BE_DEBUG_ONLY(++allocatorAllocationsCount)
			BE_DEBUG_ONLY(++totalAllocatorAllocationsCount)

			BE_DEBUG_ONLY(allocatorAllocatedBytes += allocated_size)
			BE_DEBUG_ONLY(totalAllocatorAllocatedBytes += allocated_size)
		}

		stacksMutexes.EmplaceBack();
	}
}

StackAllocator::~StackAllocator()
{
	uint64 deallocatedBytes{0};

	for (auto& stack : stacks)
	{
		for (auto& block : stack)
		{
			block.DeallocateBlock(allocatorReference, deallocatedBytes);

			BE_DEBUG_ONLY(++allocatorDeallocationsCount)
			BE_DEBUG_ONLY(++totalAllocatorDeallocationsCount)

			BE_DEBUG_ONLY(allocatorDeallocatedBytes += deallocatedBytes)
			BE_DEBUG_ONLY(totalAllocatorDeallocatedBytes += deallocatedBytes)
		}
	}
}

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

void StackAllocator::Clear()
{
	for (auto& stack : stacks)
	{
		for (auto& block : stack)
		{
			block.Clear();
		}
	}

	stackIndex = 0;
}

void StackAllocator::LockedClear()
{
	uint64 n{0};
	for (auto& stack : stacks)
	{
		for (auto& block : stack)
		{
			stacksMutexes[n].Lock();
			block.Clear();
			stacksMutexes[n].Unlock();
		}
		++n;
	}

	stackIndex = 0;
}

void StackAllocator::Allocate(const uint64 size, const uint64 alignment, void** memory, uint64* allocatedSize, const char* name)
{
	uint64 n{0};
	const auto i{ stackIndex % maxStacks };

	BE_DEBUG_ONLY(GTSL::Ranger<GTSL::UTF8> range(GTSL::String::StringLength(name), const_cast<char*>(name)))
	
	++stackIndex;

	BE_ASSERT((alignment & (alignment - 1)) != 0, "Alignment is not power of two!")
	BE_ASSERT(size > blockSize, "Single allocation is larger than block sizes! An allocation larger than block size can't happen.")

	uint64 allocated_size{0};

	{
		BE_DEBUG_ONLY(GTSL::Lock<GTSL::Mutex> lock(debugDataMutex));
		BE_DEBUG_ONLY(perNameData.try_emplace(GTSL::Id64(range)).first->second.Name = name)
	}

	stacksMutexes[i].Lock();
	for (uint32 j = 0; j < stacks[i].GetLength(); ++j)
	{
		if (stacks[j][n].TryAllocateInBlock(size, alignment, memory, allocated_size))
		{
			stacksMutexes[i].Unlock();
			*allocatedSize = allocated_size;

			BE_DEBUG_ONLY(GTSL::Lock<GTSL::Mutex> lock(debugDataMutex));

			BE_DEBUG_ONLY(perNameData[GTSL::Id64(range)].BytesAllocated += allocated_size)
			BE_DEBUG_ONLY(perNameData[GTSL::Id64(range)].AllocationCount += 1)

			BE_DEBUG_ONLY(bytesAllocated += allocated_size)
			BE_DEBUG_ONLY(totalBytesAllocated += allocated_size)

			BE_DEBUG_ONLY(++allocationsCount)
			BE_DEBUG_ONLY(++totalAllocationsCount)

			return;
		}

		++n;
		BE_DEBUG_ONLY(debugDataMutex.Lock())
		BE_DEBUG_ONLY(++blockMisses)
		BE_DEBUG_ONLY(debugDataMutex.Unlock())
	}

	//stacks[i].EmplaceBack();
	stacks[i][n].AllocateBlock(blockSize, allocatorReference, allocated_size);
	stacks[i][n].AllocateInBlock(size, alignment, memory, allocated_size);
	stacksMutexes[i].Unlock();
	*allocatedSize = allocated_size;

	BE_DEBUG_ONLY(GTSL::Lock<GTSL::Mutex> lock(debugDataMutex));

	BE_DEBUG_ONLY(perNameData[GTSL::Id64(range)].BytesAllocated += allocated_size)
	BE_DEBUG_ONLY(perNameData[GTSL::Id64(range)].AllocationCount += 1)

	BE_DEBUG_ONLY(bytesAllocated += allocated_size)
	BE_DEBUG_ONLY(totalBytesAllocated += allocated_size)

	BE_DEBUG_ONLY(allocatorAllocatedBytes += allocated_size)
	BE_DEBUG_ONLY(totalAllocatorAllocatedBytes += allocated_size)

	BE_DEBUG_ONLY(++allocatorAllocationsCount)
	BE_DEBUG_ONLY(++totalAllocatorAllocationsCount)

	BE_DEBUG_ONLY(++allocationsCount)
	BE_DEBUG_ONLY(++totalAllocationsCount)
}

void StackAllocator::Deallocate(const uint64 size, const uint64 alignment, void* memory, const char* name)
{
	BE_ASSERT((alignment & (alignment - 1)) != 0, "Alignment is not power of two!")
	BE_ASSERT(size > blockSize, "Deallocation size is larger than block size! An allocation larger than block size can't happen. Trying to deallocate more bytes than allocated!")

	BE_DEBUG_ONLY(const auto bytes_deallocated{ GTSL::Math::AlignedNumber(size, alignment) })

	BE_DEBUG_ONLY(GTSL::Ranger<GTSL::UTF8> range(GTSL::String::StringLength(name), const_cast<char*>(name)))
	
	BE_DEBUG_ONLY(GTSL::Lock<GTSL::Mutex> lock(debugDataMutex));
	
	BE_DEBUG_ONLY(perNameData[GTSL::Id64(range)].BytesDeallocated += bytes_deallocated)
	BE_DEBUG_ONLY(perNameData[GTSL::Id64(range)].DeallocationCount += 1)

	BE_DEBUG_ONLY(bytesDeallocated += bytes_deallocated)
	BE_DEBUG_ONLY(totalBytesDeallocated += bytes_deallocated)

	BE_DEBUG_ONLY(++deallocationsCount)
	BE_DEBUG_ONLY(++totalDeallocationsCount)
}
