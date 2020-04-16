#pragma once

#include "Core.h"
#include <GTSL/Array.hpp>

class BigAllocator
{
public:
	void Allocate(uint64 size, uint64 alignment, void** memory, uint64* allocatedSize, const GTSL::Array<char, 255>& name)
	{
	}

	void Deallocate(uint64 size, uint64 alignment, void* memory, const GTSL::Array<char, 255>& name)
	{
	}

	
};
