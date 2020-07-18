#pragma once

#include "ByteEngine/Core.h"
#include <GTSL/Allocator.h>

namespace BE
{
	struct BEAllocatorReference : GTSL::AllocatorReference
	{
		const char* Name{ nullptr };
		bool IsDebugAllocation = false;

		BEAllocatorReference() = default;
		BEAllocatorReference(const BEAllocatorReference& allocatorReference) = default;
		BEAllocatorReference(BEAllocatorReference&& allocatorReference) = default;
		
		explicit BEAllocatorReference(const char* name, const bool isDebugAllocation = false) : Name(name), IsDebugAllocation(isDebugAllocation) {}
	};

	struct SystemAllocatorReference : BEAllocatorReference
	{
		void Allocate(uint64 size, uint64 alignment, void** memory, uint64* allocatedSize) const;

		void Deallocate(uint64 size, uint64 alignment, void* memory) const;

		SystemAllocatorReference(const char* name, const bool isDebugAllocation = false) : BEAllocatorReference(name, isDebugAllocation)
		{
		}

	};

	struct TransientAllocatorReference : BEAllocatorReference
	{
		void Allocate(uint64 size, uint64 alignment, void** memory, uint64* allocatedSize) const;

		void Deallocate(uint64 size, uint64 alignment, void* memory) const;

		TransientAllocatorReference(const char* name, const bool isDebugAllocation = false) : BEAllocatorReference(name, isDebugAllocation)
		{
		}
	};

	struct PersistentAllocatorReference : BEAllocatorReference
	{
		void Allocate(uint64 size, uint64 alignment, void** memory, uint64* allocatedSize) const;

		void Deallocate(uint64 size, uint64 alignment, void* memory) const;

		PersistentAllocatorReference() = default;
		
		PersistentAllocatorReference(const PersistentAllocatorReference& allocatorReference) : BEAllocatorReference(allocatorReference.Name, allocatorReference.IsDebugAllocation)
		{
		}
		
		PersistentAllocatorReference(PersistentAllocatorReference&& persistentAllocatorReference) = default;

		explicit PersistentAllocatorReference(const char* name, const bool isDebugAllocation = false) : BEAllocatorReference(name, isDebugAllocation)
		{
		}
	};
}