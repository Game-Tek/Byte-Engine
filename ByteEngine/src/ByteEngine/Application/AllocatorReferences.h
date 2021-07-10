#pragma once

#include "ByteEngine/Core.h"
#include <GTSL/Allocator.h>
#include <GTSL/ShortString.hpp>

namespace BE
{
	struct BEAllocatorReference : GTSL::AllocatorReference
	{
		GTSL::ShortString<128> Name;
		bool IsDebugAllocation = false;

		BEAllocatorReference() = default;
		BEAllocatorReference(const BEAllocatorReference& allocatorReference) = default;
		BEAllocatorReference(BEAllocatorReference&& allocatorReference) = default;

		BEAllocatorReference& operator=(const BEAllocatorReference& other)
		{
			Name = other.Name; IsDebugAllocation = other.IsDebugAllocation; return *this;
		}
		
		explicit BEAllocatorReference(const GTSL::ShortString<128>& name, const bool isDebugAllocation = false) : Name(name), IsDebugAllocation(isDebugAllocation) {}
		explicit BEAllocatorReference(const utf8* name, const bool isDebugAllocation = false) : Name(name), IsDebugAllocation(isDebugAllocation) {}
	};

	struct SystemAllocatorReference : BEAllocatorReference
	{
		void Allocate(uint64 size, uint64 alignment, void** memory, uint64* allocatedSize) const;

		void Deallocate(uint64 size, uint64 alignment, void* memory) const;

		SystemAllocatorReference() = default;
		
		SystemAllocatorReference(const utf8* name, const bool isDebugAllocation = false) : BEAllocatorReference(name, isDebugAllocation)
		{
		}

	};

	struct TransientAllocatorReference : BEAllocatorReference
	{
		void Allocate(uint64 size, uint64 alignment, void** memory, uint64* allocatedSize) const;

#if(_DEBUG)
		void Deallocate(uint64 size, uint64 alignment, void* memory) const;
#else
		void Deallocate(uint64 size, uint64 alignment, void* memory) const {}
#endif

		TransientAllocatorReference() = default;

		TransientAllocatorReference(const TransientAllocatorReference& reference) : BEAllocatorReference(reference.Name, reference.IsDebugAllocation) {}
		TransientAllocatorReference(TransientAllocatorReference&& reference) = default;

		TransientAllocatorReference& operator=(const TransientAllocatorReference& allocatorReference) = default;
		
		TransientAllocatorReference(const GTSL::Range<const char8_t*> name, const bool isDebugAllocation = false) : BEAllocatorReference(name, isDebugAllocation)
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

		PersistentAllocatorReference& operator=(const PersistentAllocatorReference&) = default;
		
		PersistentAllocatorReference(PersistentAllocatorReference&& persistentAllocatorReference) = default;

		explicit PersistentAllocatorReference(const GTSL::Range<const char8_t*> name, const bool isDebugAllocation = false) : BEAllocatorReference(name, isDebugAllocation)
		{
		}
	};

	using TAR = TransientAllocatorReference;
	using PAR = PersistentAllocatorReference;
}
