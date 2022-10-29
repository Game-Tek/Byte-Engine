#pragma once

#include "ByteEngine/Core.h"

#include <unordered_map>
#include <GTSL/Id.h>
#include <GTSL/Mutex.h>
#include <GTSL/Atomic.hpp>
#include <GTSL/String.hpp>
#include <GTSL/Vector.hpp>
#include "AllocatorReferences.h"
#include "ByteEngine/Debug/Assert.h"

class StackAllocator
{
public:
	struct DebugData
	{
		explicit DebugData(BE::SystemAllocatorReference*)
		{
		}
		
		struct PerNameData
		{
			GTSL::ShortString<128> Name;
			uint64 AllocationCount{ 0 };
			uint64 DeallocationCount{ 0 };
			uint64 BytesAllocated{ 0 };
			uint64 BytesDeallocated{ 0 };
		};
		
		std::unordered_map<GTSL::Id64::HashType, PerNameData> PerNameAllocationsData;
		
		/**
		 * \brief Number of times it was tried to allocate on different blocks to no avail.
		 * To improve this number(lower it) try to make the blocks bigger.
		 * Don't make it so big that in the event that a new block has to be allocated it takes up too much space.
		 * Reset to 0 on every call to GetDebugInfo()
		 */
		uint64 BlockMisses{ 0 };

		uint64 BytesAllocated{ 0 };
		uint64 TotalBytesAllocated{ 0 };
		
		uint64 BytesDeallocated{ 0 };
		uint64 TotalBytesDeallocated{ 0 };
		
		uint64 AllocatorAllocatedBytes{ 0 };
		uint64 TotalAllocatorAllocatedBytes{ 0 };

		uint64 AllocatorDeallocatedBytes{ 0 };
		uint64 TotalAllocatorDeallocatedBytes{ 0 };
		
		uint64 AllocationsCount{ 0 };
		uint64 TotalAllocationsCount{ 0 };
		
		uint64 DeallocationsCount{ 0 };
		uint64 TotalDeallocationsCount{ 0 };
		
		uint64 AllocatorAllocationsCount{ 0 };
		uint64 TotalAllocatorAllocationsCount{ 0 };
		
		uint64 AllocatorDeallocationsCount{ 0 };
		uint64 TotalAllocatorDeallocationsCount{ 0 };

		operator GTSL::StaticString<1024>() const
		{
#define ADD_FIELD(string, var) string += reinterpret_cast<const char8_t*>(#var); (string) += u8": "; ToString(string, var); (string) += u8'\n';
			
			GTSL::StaticString<1024> result;
			ADD_FIELD(result, BytesAllocated)
			ADD_FIELD(result, TotalBytesAllocated)
			ADD_FIELD(result, TotalAllocatorAllocatedBytes)
			ADD_FIELD(result, TotalAllocatorDeallocatedBytes)

#undef ADD_FIELD
			return result;
		}
	};

	explicit StackAllocator(BE::SystemAllocatorReference* allocatorReference, const uint8 stackCount = 8, const uint8 defaultBlocksPerStackCount = 2, const uint64 blockSizes = 512) :
		blockSize(blockSizes), stacks(stackCount, *allocatorReference), allocatorReference(allocatorReference), MAX_STACKS(stackCount)
	{
		uint64 allocated_size = 0;

		for (uint8 stack = 0; stack < stackCount; ++stack)
		{
			stacks.EmplaceBack(defaultBlocksPerStackCount, *allocatorReference);

			for (uint32 block = 0; block < defaultBlocksPerStackCount; ++block)
			{
				stacks[stack].EmplaceBack(); //construct a default block

				stacks[stack][block].AllocateBlock(blockSizes, allocatorReference, allocated_size);

#if BE_DEBUG
					GTSL::Lock<GTSL::Mutex> lock(debugDataMutex);
					++allocatorAllocationsCount;
					++totalAllocatorAllocationsCount;
					allocatorAllocatedBytes += allocated_size;
					totalAllocatorAllocatedBytes += allocated_size;
#endif
			}
		}
	}
	
	~StackAllocator()
	{
		
	}

#if BE_DEBUG
	void GetDebugData(DebugData& debugData)
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

		for (auto& e : perNameData) {
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

	void Clear()
	{
		for (auto& stack : stacks)
		{
			for (auto& block : stack)
			{
				block.Clear();
			}
		}
	}

	void LockedClear()
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

	void Allocate(uint64 size, uint64 alignment, void** memory, uint64* allocatedSize, const GTSL::Range<const char8_t*> name)
	{
		const auto i{ stackIndex % MAX_STACKS }; ++stackIndex;

		BE_ASSERT((alignment & (alignment - 1)) == 0, "Alignment is not power of two!");
		BE_ASSERT(size <= blockSize, "Single allocation is larger than block sizes! An allocation larger than block size can't happen.");

		uint64 allocated_size{ 0 };

#if BE_DEBUG
		{
			GTSL::Lock lock(debugDataMutex);
			perNameData.try_emplace(GTSL::Id64(name)()).first->second.Name = name;
		}
#endif

		stacksMutexes[i].Lock();
		for (auto& block : stacks[i])
		{
			if (block.TryAllocateInBlock(size, alignment, memory, allocated_size))
			{
				stacksMutexes[i].Unlock();
				*allocatedSize = allocated_size;

#if BE_DEBUG
				{
					GTSL::Lock<GTSL::Mutex> lock(debugDataMutex);
					perNameData[GTSL::Id64(name)()].BytesAllocated += allocated_size;
					perNameData[GTSL::Id64(name)()].AllocationCount += 1;
					bytesAllocated += allocated_size;
					totalBytesAllocated += allocated_size;
					++allocationsCount;
					++totalAllocationsCount;
				}
#endif

				return;
			}

#if BE_DEBUG
			debugDataMutex.Lock();
			++blockMisses;
			debugDataMutex.Unlock();
#endif
		}

		auto& lastBlock = stacks[i].EmplaceBack();
		lastBlock.AllocateBlock(blockSize, allocatorReference, allocated_size);
		lastBlock.AllocateInBlock(size, alignment, memory, allocated_size);
		stacksMutexes[i].Unlock();

		*allocatedSize = allocated_size;

#if BE_DEBUG
		{
			GTSL::Lock lock(debugDataMutex);
			perNameData[GTSL::Id64(name)()].BytesAllocated += allocated_size;
			perNameData[GTSL::Id64(name)()].AllocationCount += 1;
			bytesAllocated += allocated_size;
			totalBytesAllocated += allocated_size;
			allocatorAllocatedBytes += allocated_size;
			totalAllocatorAllocatedBytes += allocated_size;
			++allocatorAllocationsCount;
			++totalAllocatorAllocationsCount;
			++allocationsCount;
			++totalAllocationsCount;
		}
#endif
	}

	void Deallocate(uint64 size, uint64 alignment, void*, const GTSL::Range<const char8_t*> name)
	{
		BE_ASSERT((alignment & (alignment - 1)) == 0, "Alignment is not power of two!");
		BE_ASSERT(size <= blockSize, "Deallocation size is larger than block size! An allocation larger than block size can't happen. Trying to deallocate more bytes than allocated!");

#if BE_DEBUG
			const auto bytes_deallocated{ GTSL::Math::RoundUpByPowerOf2(size, alignment) };

			GTSL::Lock lock(debugDataMutex);
			perNameData[GTSL::Id64(name)()].BytesDeallocated += bytes_deallocated;
			perNameData[GTSL::Id64(name)()].DeallocationCount += 1;
			bytesDeallocated += bytes_deallocated;
			totalBytesDeallocated += bytes_deallocated;
			++deallocationsCount;
			++totalDeallocationsCount;
#endif
	}

	void Free()
	{
		uint64 freed_bytes{ 0 };

		for (auto& stack : stacks)
		{
			for (auto& block : stack)
			{
				block.DeallocateBlock(allocatorReference, freed_bytes);
#if BE_DEBUG
					++allocatorDeallocationsCount;
					++totalAllocatorDeallocationsCount;
#endif
			}
		}

#if BE_DEBUG
			allocatorDeallocatedBytes += freed_bytes;
			totalAllocatorDeallocatedBytes += freed_bytes;
#endif
	}


protected:
	struct Block
	{
		Block() = default;

		byte* start{ nullptr };
		byte* at{ nullptr };
		byte* end{ nullptr };

		void AllocateBlock(uint64 minimumSize, BE::SystemAllocatorReference* allocatorReference, uint64& allocatedSize);

		void DeallocateBlock(BE::SystemAllocatorReference* allocatorReference, uint64& deallocatedBytes) const;

		void AllocateInBlock(uint64 size, uint64 alignment, void** data, uint64& allocatedSize);

		bool TryAllocateInBlock(uint64 size, uint64 alignment, void** data, uint64& allocatedSize);

		void Clear();

		[[nodiscard]] uint64 GetBlockSize() const { return end - start; }
		[[nodiscard]] uint64 GetRemainingSize() const { return end - at; }
	};

	const uint64 blockSize{ 0 };
	GTSL::Atomic<uint32> stackIndex{ 0 };
	GTSL::Vector<GTSL::Vector<Block, BE::SystemAllocatorReference>, BE::SystemAllocatorReference> stacks;
	GTSL::Mutex stacksMutexes[32];
	BE::SystemAllocatorReference* allocatorReference{ nullptr };

#if BE_DEBUG
	uint64 blockMisses{ 0 };
	std::unordered_map<GTSL::Id64::HashType, DebugData::PerNameData> perNameData;
	GTSL::Mutex debugDataMutex;

	uint64 bytesAllocated{ 0 };
	uint64 bytesDeallocated{ 0 };

	uint64 totalAllocatorAllocatedBytes{ 0 };
	uint64 totalAllocatorDeallocatedBytes{ 0 };

	uint64 allocationsCount{ 0 };
	uint64 deallocationsCount{ 0 };

	uint64 allocatorAllocationsCount{ 0 };
	uint64 allocatorDeallocationsCount{ 0 };

	uint64 allocatorAllocatedBytes{ 0 };
	uint64 allocatorDeallocatedBytes{ 0 };

	uint64 totalBytesAllocated{ 0 };
	uint64 totalBytesDeallocated{ 0 };

	uint64 totalAllocationsCount{ 0 };
	uint64 totalDeallocationsCount{ 0 };

	uint64 totalAllocatorAllocationsCount{ 0 };
	uint64 totalAllocatorDeallocationsCount{ 0 };
#endif

	const uint8 MAX_STACKS{ 8 };

};
