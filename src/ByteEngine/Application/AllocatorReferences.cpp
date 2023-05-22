#include "AllocatorReferences.h"
#include "Application.h"

void BE::SystemAllocatorReference::Allocate(GTSL::uint64 size, GTSL::uint64 alignment, void** memory,
	GTSL::uint64* allocatedSize) const
{
	allocatedSize = &size;
	BE::Application::Get()->GetSystemAllocator()->Allocate(size, alignment, memory);
}

void BE::SystemAllocatorReference::Deallocate(GTSL::uint64 size, GTSL::uint64 alignment, void* memory) const
{
	BE::Application::Get()->GetSystemAllocator()->Deallocate(size, alignment, memory);
}

void BE::TransientAllocatorReference::Allocate(GTSL::uint64 size, GTSL::uint64 alignment, void** memory,
	GTSL::uint64* allocatedSize) const
{
	BE::Application::Get()->GetTransientAllocator()->Allocate(size,alignment,memory,allocatedSize,Name);
}

void BE::TransientAllocatorReference::Deallocate(GTSL::uint64 size, GTSL::uint64 alignment, void* memory) const
{
	BE::Application::Get()->GetTransientAllocator()->Deallocate(size, alignment, memory, Name);
}

void BE::PersistentAllocatorReference::Allocate(GTSL::uint64 size, GTSL::uint64 alignment, void** memory,
	GTSL::uint64* allocatedSize) const
{
	BE::Application::Get()->GetPersistantAllocator()->Allocate(size, alignment, memory, allocatedSize, Name);
}

void BE::PersistentAllocatorReference::Deallocate(GTSL::uint64 size, GTSL::uint64 alignment, void* memory) const
{
	BE::Application::Get()->GetPersistantAllocator()->Deallocate(size, alignment, memory, Name);
}
