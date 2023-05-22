#pragma once

#include "ByteEngine/Core.h"
#include <GTSL/Allocator.hpp>
#include <GTSL/ShortString.hpp>

namespace BE
{
	class Application;

	struct BEAllocatorReference : GTSL::AllocatorReference
	{
		GTSL::ShortString<16> Name = { u8"-" };
		bool IsDebugAllocation = false;

		BEAllocatorReference() = default;
		BEAllocatorReference(const BEAllocatorReference& ref) = default;
		BEAllocatorReference(BEAllocatorReference&& ref) = default;

		BEAllocatorReference& operator=(const BEAllocatorReference& other)
		{
			Name = other.Name;
			IsDebugAllocation = other.IsDebugAllocation;
			return *this;
		}

		explicit BEAllocatorReference(const GTSL::ShortString<16>& name, const bool isDebug = false) : Name(name), IsDebugAllocation(isDebug) {}
		explicit BEAllocatorReference(const char8_t* name, bool isDebug = false) : Name(name), IsDebugAllocation(isDebug) {}
	};

	struct SystemAllocatorReference : BEAllocatorReference
	{
		void Allocate(GTSL::uint64 size, GTSL::uint64 alignment, void** memory, GTSL::uint64* allocatedSize) const;
		void Deallocate(GTSL::uint64 size, GTSL::uint64 alignment, void* memory) const;

		SystemAllocatorReference() = default;
		SystemAllocatorReference(const char8_t* name, const bool isDebugAllocation = false) : BEAllocatorReference(name, isDebugAllocation) {}
		SystemAllocatorReference(const GTSL::Range<const char8_t*> name, const bool isDebugAllocation = false) : BEAllocatorReference(name,isDebugAllocation) {}
	};

	struct TransientAllocatorReference : BEAllocatorReference
	{
		void Allocate(GTSL::uint64 size, GTSL::uint64 alignment, void** memory, GTSL::uint64* allocatedSize) const;
		void Deallocate(GTSL::uint64 size, GTSL::uint64 alignment, void* memory) const;

		TransientAllocatorReference() = default;
		TransientAllocatorReference(const TransientAllocatorReference& reference) : BEAllocatorReference(reference.Name, reference.IsDebugAllocation) {}
		TransientAllocatorReference(TransientAllocatorReference&& reference) = default;

		TransientAllocatorReference(const char8_t* name, const bool isDebugAllocation = false) : BEAllocatorReference(name, isDebugAllocation) {}
		TransientAllocatorReference(const GTSL::Range<const char8_t*> name, const bool isDebugAllocation = false) : BEAllocatorReference(name, isDebugAllocation) {}
	};

	struct PersistentAllocatorReference : BEAllocatorReference
	{
		void Allocate(GTSL::uint64 size, GTSL::uint64 alignment, void** memory, GTSL::uint64* allocatedSize) const;
		void Deallocate(GTSL::uint64 size, GTSL::uint64 alignment, void* memory) const;

		PersistentAllocatorReference() = default;
		PersistentAllocatorReference(const PersistentAllocatorReference& reference) : BEAllocatorReference(reference.Name, reference.IsDebugAllocation) {}
		PersistentAllocatorReference(PersistentAllocatorReference&& reference) = default;

		PersistentAllocatorReference(const char8_t* name, const bool isDebugAllocation = false) : BEAllocatorReference(name, isDebugAllocation) {}
		PersistentAllocatorReference(const GTSL::Range<const char8_t*> name, const bool isDebugAllocation = false) : BEAllocatorReference(name, isDebugAllocation) {}
	};

	using TAR = TransientAllocatorReference;
	using PAR = PersistentAllocatorReference;
}