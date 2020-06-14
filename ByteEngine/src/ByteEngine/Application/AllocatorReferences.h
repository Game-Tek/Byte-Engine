#pragma once

#include "ByteEngine/Core.h"
#include <GTSL/Allocator.h>

namespace BE
{
	struct BEAllocatorReference : GTSL::AllocatorReference
	{
		const char* Name{ nullptr };
		bool IsDebugAllocation = false;

		explicit BEAllocatorReference(const decltype(allocate)& allocateFunc, const decltype(deallocate)& deallocateFunc, const char* name, const bool isDebugAllocation = false) : AllocatorReference(allocateFunc, deallocateFunc), Name(name), IsDebugAllocation(isDebugAllocation) {}
	};

	struct SystemAllocatorReference : BEAllocatorReference
	{
	protected:
		void allocateFunc(uint64 size, uint64 alignment, void** memory, uint64* allocatedSize) const;

		void deallocateFunc(uint64 size, uint64 alignment, void* memory) const;

	public:
		SystemAllocatorReference(const char* name, const bool isDebugAllocation = false) :
			BEAllocatorReference(reinterpret_cast<decltype(allocate)>(&SystemAllocatorReference::allocateFunc), reinterpret_cast<decltype(deallocate)>(&SystemAllocatorReference::deallocateFunc),
				name, isDebugAllocation)
		{
		}

	};

	struct TransientAllocatorReference : BEAllocatorReference
	{
	protected:
		void allocateFunc(uint64 size, uint64 alignment, void** memory, uint64* allocatedSize) const;

		void deallocateFunc(uint64 size, uint64 alignment, void* memory) const;

	public:
		TransientAllocatorReference(const char* name, const bool isDebugAllocation = false) :
			BEAllocatorReference(reinterpret_cast<decltype(allocate)>(&TransientAllocatorReference::allocateFunc), reinterpret_cast<decltype(deallocate)>(&TransientAllocatorReference::deallocateFunc),
				name, isDebugAllocation)
		{
		}
	};

	struct PersistentAllocatorReference : BEAllocatorReference
	{
	protected:
		void allocateFunc(uint64 size, uint64 alignment, void** memory, uint64* allocatedSize) const;

		void deallocateFunc(uint64 size, uint64 alignment, void* memory) const;

	public:
		PersistentAllocatorReference(const char* name, const bool isDebugAllocation = false) :
			BEAllocatorReference(reinterpret_cast<decltype(allocate)>(&PersistentAllocatorReference::allocateFunc), reinterpret_cast<decltype(deallocate)>(&PersistentAllocatorReference::deallocateFunc),
				name, isDebugAllocation)
		{
		}
	};
}