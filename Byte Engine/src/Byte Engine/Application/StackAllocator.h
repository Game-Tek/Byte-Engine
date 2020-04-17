#pragma once

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

		void AllocateInBlock(uint64 size, const uint64 alignment, void** data)
		{
			at = static_cast<byte*>(std::align(alignment, end - at, *data, size)); //take alignment
		}
		
		bool TryAllocateInBlock(uint64 size, const uint64 alignment, void** data)
		{
			const auto p = static_cast<byte*>(std::align(alignment, end - at, *data, size)); //take alignment
			if (p) { AllocateInBlock(size, alignment, data); return true; }
			return false;
		}

		void Clear() { at = start; }

		[[nodiscard]] bool FitsInBlock(const uint64 size, uint64 alignment) const { return at + size < end; }

		[[nodiscard]] uint64 GetBlockSize() const { return end - start; }
	};

	static void clearBlock(Block& block)
	{
		block.at = block.start;
	}

	const uint64 blockSize{ 0 };
	std::atomic<uint32> stackIndex{ 0 };
	const uint8 maxStacks{ 8 };
	GTSL::Vector<GTSL::Vector<Block>> stacks;
	GTSL::Vector<GTSL::Mutex> stacksMutexes;
	GTSL::AllocatorReference* allocatorReference{ nullptr };
	
public:
	explicit StackAllocator(GTSL::AllocatorReference* allocatorReference, const uint8 stackCount = 8, const uint8 defaultBlocksPerStackCount = 2, const uint64 blockSizes = 64) : blockSize(blockSizes), maxStacks(stackCount), stacks(stackCount, allocatorReference), allocatorReference(allocatorReference)
	{
		for(uint8 i = 0; i < stackCount; ++i)
		{
			stacks.EmplaceBack(defaultBlocksPerStackCount, defaultBlocksPerStackCount, allocatorReference); //construct stack i's block vector
			
			for (auto& block : stacks[i]) //for every block in constructed vector
			{
				stacks[i].EmplaceBack(); //construct a default block
				block.AllocateBlock(blockSizes, allocatorReference); //allocate constructed block, which is also current block
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
	
	void Clear()
	{
		for(auto& stack : stacks)
		{
			for (auto& block : stack)
			{
				block.Clear();
			}
		}
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
	}

	void Allocate(const uint64 size, const uint64 alignment, void** memory, uint64* allocatedSize, const char* name)
	{
		uint64 n{ 0 };
		const auto i{ stackIndex % maxStacks };

		stacksMutexes[i].Lock();
		for(auto& block : stacks[i])
		{
			if(block.TryAllocateInBlock(size, alignment, memory))
			{
				++stackIndex;
				return;
			}
			++n;
		}

		stacks[i].EmplaceBack();
		stacks[i][n].AllocateBlock(blockSize, allocatorReference);
		stacks[i][n].AllocateInBlock(size, alignment, memory);
		stacksMutexes[i].Unlock();
		
		++stackIndex;
	}

	void Deallocate(uint64 size, uint64 alignment, void* memory, const char* name)
	{
	}
};
