#pragma once

#include "Byte Engine/Core.h"

class PowerOf2PoolAllocator
{
	GTSL::AllocatorReference* allocatorReference{ nullptr };
	
public:
	PowerOf2PoolAllocator(GTSL::AllocatorReference* allocatorReference) : allocatorReference(allocatorReference)
	{
	}
	
	void Allocate(uint64 size, uint64 alignment, void** memory, uint64* allocatedSize, const char* name)
	{
	}

	void Deallocate(uint64 size, uint64 alignment, void* memory, const char* name)
	{
	}

	
};
