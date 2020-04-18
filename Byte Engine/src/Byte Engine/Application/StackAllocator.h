#pragma once
#include <GTSL/Math/Math.hpp>
#include <GTSL/FixedVector.hpp>

class StackAllocator
{
	struct Block
	{
		byte* start{ nullptr };
		byte* at{ nullptr };
		byte* end{ nullptr };

		void AllocateBlock(const uint64 minimumSize, GTSL::AllocatorReference* allocatorReference)
		{
			uint64 allocatedSize{0};
			allocatorReference->Allocate(minimumSize, alignof(byte), reinterpret_cast<void**>(&start), &allocatedSize);
			at = start;
			end = start + allocatedSize;
		}

		void DeallocateBlock(GTSL::AllocatorReference* allocatorReference) const
		{
			allocatorReference->Deallocate(end - start, alignof(byte), start);
		}

		void AllocateInBlock(const uint64 size, const uint64 alignment, void** data, uint64& allocatedSize)
		{
			*data = (at += (allocatedSize = GTSL::Math::AlignedNumber(size, alignment)));
		}
		
		bool TryAllocateInBlock(const uint64 size, const uint64 alignment, void** data, uint64& allocatedSize)
		{
			const auto new_at = at + (allocatedSize = GTSL::Math::AlignedNumber(size, alignment));
			if (new_at < end) { *data = new_at; at = new_at; return true; }
			return false;
		}

		void Clear() { at = start; }

		[[nodiscard]] bool FitsInBlock(const uint64 size, uint64 alignment) const { return at + size < end; }

		[[nodiscard]] uint64 GetBlockSize() const { return end - start; }
		[[nodiscard]] uint64 GetRemainingSize() const { return end - at; }
	};

public:
	struct DebugData
	{
		DebugData(GTSL::AllocatorReference* allocatorReference)
		{
		}
		
		struct PerNameData
		{
			const char* Name{ nullptr };
			uint64 AllocationCount{ 0 };
			uint64 DeallocationCount{ 0 };
			uint64 BytesAllocated{ 0 };
			uint64 BytesDeallocated{ 0 };
		};
		
		std::unordered_map<GTSL::Id64::HashType, PerNameData> PerNameAllocationsData;
		
		/**
		 * \brief Number of times it was tried to allocate on a block to no avail.
		 * To improve this number(lower it) try to make the blocks bigger.
		 * Don't make it so big that in the event that a new block has to be allocated it takes up too much space.
		 * Reset to 0 on every call to GetDebugInfo()
		 * LOWER BETTER; IDEAL 0.
		 */
		uint64 BlockMisses{ 0 };

		uint64 BytesAllocated{ 0 };
		uint64 BytesDeallocated{ 0 };
		uint64 TotalBytesAllocated{ 0 };
		uint64 TotalBytesDeallocated{ 0 };
		uint64 MemoryUsage{ 0 };
	};
protected:
	const uint64 blockSize{ 0 };
	std::atomic<uint32> stackIndex{ 0 };
	GTSL::Vector<GTSL::Vector<Block>> stacks;
	GTSL::Vector<GTSL::Mutex> stacksMutexes;
	GTSL::AllocatorReference* allocatorReference{ nullptr };

#if BE_DEBUG
	uint64 blockMisses{ 0 };
	std::unordered_map<GTSL::Id64::HashType, DebugData::PerNameData> perNameData;
	GTSL::Mutex debugDataMutex;
	uint64 bytesAllocated{ 0 };
	uint64 bytesDeallocated{ 0 };
	uint64 totalBytesAllocated{ 0 };
	uint64 totalBytesDeallocated{ 0 };
	uint64 memoryUsage{ 0 };
#endif
	
	const uint8 maxStacks{ 8 };
	
public:
	explicit StackAllocator(GTSL::AllocatorReference* allocatorReference, const uint8 stackCount = 8, const uint8 defaultBlocksPerStackCount = 2, const uint64 blockSizes = 512) : blockSize(blockSizes), stacks(stackCount, allocatorReference), stacksMutexes(stackCount, allocatorReference), allocatorReference(allocatorReference), maxStacks(stackCount)
	{
		for(uint8 i = 0; i < stackCount; ++i)
		{
			stacks.EmplaceBack(defaultBlocksPerStackCount, defaultBlocksPerStackCount, allocatorReference); //construct stack i's block vector
			
			for (auto& blocks : stacks) //for every block in constructed vector
			{
				//stacks[i].EmplaceBack(); //construct a default block
				for (auto& block : blocks)
				{
					block.AllocateBlock(blockSizes, allocatorReference); //allocate constructed block, which is also current block
				}
			}
			
			stacksMutexes.EmplaceBack();
		}
	}

	~StackAllocator()
	{
		for (auto& stack : stacks)
		{
			for (auto& block : stack)
			{
				block.DeallocateBlock(allocatorReference);
			}
		}
	}

#if BE_DEBUG
	void GetDebugData(DebugData& debugData)
	{
		GTSL::Lock<GTSL::Mutex> lock(debugDataMutex);
		
		for (auto& stack : stacks)
		{
			for (auto& block : stack)
			{
				debugData.MemoryUsage = block.GetBlockSize();
			}
		}
		
		debugData.BlockMisses = blockMisses;
		debugData.PerNameAllocationsData = perNameData;
		debugData.BytesAllocated = bytesAllocated;
		debugData.BytesDeallocated = bytesDeallocated;
		debugData.TotalBytesAllocated = totalBytesAllocated;
		debugData.TotalBytesDeallocated = totalBytesDeallocated;
		
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
	}
#endif
	
	void Clear()
	{
		for(auto& stack : stacks)
		{
			for (auto& block : stack)
			{
				block.Clear();
			}
		}
		
		stackIndex = 0;
	}

	void LockedClear()
	{
		uint64 n{ 0 };
		for(auto& stack : stacks)
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

	void Allocate(const uint64 size, const uint64 alignment, void** memory, uint64* allocatedSize, const char* name)
	{
		uint64 n{ 0 };
		const auto i{ stackIndex % maxStacks };

		++stackIndex;
		
		BE_ASSERT((alignment & (alignment - 1)) != 0, "Alignment is not power of two!")
		BE_ASSERT(size > blockSize, "Single allocation is larger than block sizes! An allocation larger than block size can't happen.")
		
		uint64 allocated_size{ 0 };

		{
			BE_DEBUG_ONLY(GTSL::Lock<GTSL::Mutex> lock(debugDataMutex));
			BE_DEBUG_ONLY(perNameData.try_emplace(GTSL::Id64(name)).first->second.Name = name)
		}
			
		stacksMutexes[i].Lock();
		for(uint32 j = 0; j < stacks[i].GetLength(); ++j)
		{
			if (stacks[j][n].TryAllocateInBlock(size, alignment, memory, allocated_size))
			{
				stacksMutexes[i].Unlock();
				*allocatedSize = allocated_size;
				BE_DEBUG_ONLY(GTSL::Lock<GTSL::Mutex> lock(debugDataMutex));
				BE_DEBUG_ONLY(perNameData[GTSL::Id64(name)].BytesAllocated += allocated_size)
				BE_DEBUG_ONLY(perNameData[GTSL::Id64(name)].AllocationCount += 1)
				BE_DEBUG_ONLY(bytesAllocated += allocated_size)
				BE_DEBUG_ONLY(totalBytesAllocated += allocated_size)
				return;
			}

			++n;
			BE_DEBUG_ONLY(debugDataMutex.Lock())
			BE_DEBUG_ONLY(++blockMisses)
			BE_DEBUG_ONLY(debugDataMutex.Unlock())
		}

		//stacks[i].EmplaceBack();
		stacks[i][n].AllocateBlock(blockSize, allocatorReference);
		stacks[i][n].AllocateInBlock(size, alignment, memory, allocated_size);
		stacksMutexes[i].Unlock();
		*allocatedSize = allocated_size;

		BE_DEBUG_ONLY(GTSL::Lock<GTSL::Mutex> lock(debugDataMutex));
		BE_DEBUG_ONLY(perNameData[GTSL::Id64(name)].BytesAllocated += allocated_size)
		BE_DEBUG_ONLY(perNameData[GTSL::Id64(name)].AllocationCount += 1)
		BE_DEBUG_ONLY(bytesAllocated += allocated_size)
		BE_DEBUG_ONLY(totalBytesAllocated += allocated_size)
	}

	void Deallocate(const uint64 size, const uint64 alignment, void* memory, const char* name)
	{
		BE_ASSERT((alignment & (alignment - 1)) != 0, "Alignment is not power of two!")
		BE_ASSERT(size > blockSize, "Deallocation size is larger than block size! An allocation larger than block size can't happen. Trying to deallocate more bytes than allocated!")
		
		BE_DEBUG_ONLY(const auto bytes_deallocated{ GTSL::Math::AlignedNumber(size, alignment) })
		
		BE_DEBUG_ONLY(GTSL::Lock<GTSL::Mutex> lock(debugDataMutex));
		BE_DEBUG_ONLY(perNameData[GTSL::Id64(name)].BytesDeallocated += bytes_deallocated)
		BE_DEBUG_ONLY(perNameData[GTSL::Id64(name)].DeallocationCount += 1)
		BE_DEBUG_ONLY(bytesDeallocated += bytes_deallocated)
		BE_DEBUG_ONLY(totalBytesDeallocated += bytes_deallocated)
	}
};
