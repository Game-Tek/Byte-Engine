#include "SubResourceManager.h"

#include "Byte Engine/Application/Application.h"

void ResourceManagerBigAllocatorReference::allocateFunc(const uint64 size, uint64 alignment, void** memory,
	uint64* allocatedSize) const
{
	BE::Application::Get()->GetNormalAllocator()->Allocate(size, alignment, memory, allocatedSize, name);
}

void ResourceManagerBigAllocatorReference::deallocateFunc(const uint64 size, uint64 alignment, void* memory) const
{
	BE::Application::Get()->GetNormalAllocator()->Deallocate(size, alignment, memory, name);
}

void ResourceManagerTransientAllocatorReference::allocateFunc(const uint64 size, uint64 alignment, void** memory,
	uint64* allocatedSize) const
{
	BE::Application::Get()->GetTransientAllocator()->Allocate(size, alignment, memory, allocatedSize, name);
}

void ResourceManagerTransientAllocatorReference::deallocateFunc(const uint64 size, uint64 alignment, void* memory) const
{
	BE::Application::Get()->GetTransientAllocator()->Deallocate(size, alignment, memory, name);
}
