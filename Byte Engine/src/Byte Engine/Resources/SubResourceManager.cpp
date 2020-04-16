#include "SubResourceManager.h"

#include "Application/Application.h"

void ResourceManagerBigAllocatorReference::Allocate(const uint64 size, const uint64 alignment, void** memory, uint64* allocatedSize) const
{
	BE::Application::Get()->GetBigAllocator()->Allocate(size, alignment, memory, allocatedSize, name);
}

void ResourceManagerBigAllocatorReference::Deallocate(const uint64 size, const uint64 alignment, void* memory) const
{
	BE::Application::Get()->GetBigAllocator()->Deallocate(size, alignment, memory, name);
}

void ResourceManagerTransientAllocatorReference::Allocate(const uint64 size, const uint64 alignment, void** memory, uint64* allocatedSize) const
{
	BE::Application::Get()->GetTransientAllocator()->Allocate(size, alignment, memory, allocatedSize, name);
}

void ResourceManagerTransientAllocatorReference::Deallocate(const uint64 size, const uint64 alignment, void* memory) const
{
	BE::Application::Get()->GetTransientAllocator()->Deallocate(size, alignment, memory, name);
}
