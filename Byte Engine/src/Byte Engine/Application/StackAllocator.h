#pragma once

#include <GTSL/Array.hpp>

class StackAllocator
{
public:
	void Clear()
	{
	}
	
	void Allocate(uint64 size, uint64 alignment, void** memory, uint64* allocatedSize, const char* name)
	{
	}

	void Deallocate(uint64 size, uint64 alignment, void* memory, const char* name)
	{
	}
};
