#pragma once

#include "Core.h"

namespace GTSL
{
#undef CopyMemory
#undef Allocate
#undef Allocate
	
	class Memory
	{
	public:
		static void Allocate(uint64 size, void** data);
		static void Deallocate(uint64 size, void* data);
		static void CopyMemory(uint64 size, const void* from, void* to);
		static void SetZero(uint64 size, void* data);
	};
}