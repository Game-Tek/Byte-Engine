#pragma once


#include <atomic>

#include "Byte Engine/Core.h"

#include <unordered_map>
#include <GTSL/Id.h>
#include <GTSL/Mutex.h>
#include <GTSL/Math/Math.hpp>

class StackAllocator
{
	struct Block
	{
		byte* start{ nullptr };
		byte* at{ nullptr };
		byte* end{ nullptr };

		void AllocateBlock(const uint64 minimumSize, GTSL::AllocatorReference* allocatorReference, uint64& allocatedSize)
		{
			uint64 allocated_size{0};
			
			allocatorReference->Allocate(minimumSize, alignof(byte), reinterpret_cast<void**>(&start), &allocated_size);
			
			allocatedSize = allocated_size;
			
			at = start;
			end = start + allocated_size;
		}

		void DeallocateBlock(GTSL::AllocatorReference* allocatorReference, uint64& deallocatedBytes) const
		{
			allocatorReference->Deallocate(end - start, alignof(byte), start);
			deallocatedBytes = end - start;
		}

		void AllocateInBlock(const uint64 size, const uint64 alignment, void** data, uint64& allocatedSize)
		{
			*data = (at += (allocatedSize = GTSL::Math::AlignedNumber(size, alignment)));
		}
		
		bool TryAllocateInBlock(const uint64 size, const uint64 alignment, void** data, uint64& allocatedSize)
		{
			auto* const new_at = at + (allocatedSize = GTSL::Math::AlignedNumber(size, alignment));
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
		explicit DebugData(GTSL::AllocatorReference* allocatorReference)
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
	
	const uint8 maxStacks{ 8 };
	
public:
	explicit StackAllocator(GTSL::AllocatorReference* allocatorReference, uint8 stackCount = 8, uint8 defaultBlocksPerStackCount = 2, uint64 blockSizes = 512);

	~StackAllocator();

#if BE_DEBUG
	void GetDebugData(DebugData& debugData);
#endif

	void Clear();

	void LockedClear();

	void Allocate(uint64 size, uint64 alignment, void** memory, uint64* allocatedSize, const char* name);

	void Deallocate(uint64 size, uint64 alignment, void* memory, const char* name);
};
