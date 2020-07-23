#pragma once

#include "ByteEngine/Core.h"

#include <unordered_map>
#include <GTSL/Id.h>
#include <GTSL/Mutex.h>
#include <GTSL/Atomic.hpp>
#include <GTSL/StaticString.hpp>
#include <GTSL/Vector.hpp>
#include "AllocatorReferences.h"

class StackAllocator
{
public:
	struct DebugData
	{
		explicit DebugData(BE::SystemAllocatorReference* allocatorReference)
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

		operator GTSL::StaticString<1024>() const
		{
#define ADD_FIELD(string, var) string += #var; (string) += ": "; (string) += (var); (string) += '\n';
			
			GTSL::StaticString<1024> result;
			ADD_FIELD(result, BytesAllocated)
			ADD_FIELD(result, TotalBytesAllocated)
			ADD_FIELD(result, TotalAllocatorAllocatedBytes)
			ADD_FIELD(result, TotalAllocatorDeallocatedBytes)

#undef ADD_FIELD
			return result;
		}
	};

	StackAllocator() = default;
	explicit StackAllocator(BE::SystemAllocatorReference* allocatorReference, uint8 stackCount = 8, uint8 defaultBlocksPerStackCount = 2, uint64 blockSizes = 512);

	~StackAllocator();

#if BE_DEBUG
	void GetDebugData(DebugData& debugData);
#endif

	void Clear();

	void LockedClear();

	void Allocate(uint64 size, uint64 alignment, void** memory, uint64* allocatedSize, const char* name);

	void Deallocate(uint64 size, uint64 alignment, void* memory, const char* name);

	void Free();

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
	GTSL::Vector<GTSL::Mutex, BE::SystemAllocatorReference> stacksMutexes;
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
