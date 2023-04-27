#include "AllocatorReferences.h"

#include "Application.h"

void BE::SystemAllocatorReference::Allocate(const uint64 size, const uint64 alignment, void** memory, uint64* allocatedSize) const {
	(*allocatedSize) = size;
	BE::Application::Get()->GetSystemAllocator()->Allocate(size, alignment, memory);
}

void BE::SystemAllocatorReference::Deallocate(const uint64 size, const uint64 alignment, void* memory) const {
	BE::Application::Get()->GetSystemAllocator()->Deallocate(size, alignment, memory);
}

void BE::TransientAllocatorReference::Allocate(const uint64 size, const uint64 alignment, void** memory, uint64* allocatedSize) const { BE::Application::Get()->GetTransientAllocator()->Allocate(size, alignment, memory, allocatedSize, Name); }

void BE::TransientAllocatorReference::Deallocate(const uint64 size, const uint64 alignment, void* memory) const { BE::Application::Get()->GetTransientAllocator()->Deallocate(size, alignment, memory, Name); }

void BE::PersistentAllocatorReference::Allocate(const uint64 size, const uint64 alignment, void** memory, uint64* allocatedSize) const { Application::Get()->GetPersistantAllocator()->Allocate(size, alignment, memory, allocatedSize, Name); }

void BE::PersistentAllocatorReference::Deallocate(const uint64 size, const uint64 alignment, void* memory) const { Application::Get()->GetPersistantAllocator()->Deallocate(size, alignment, memory, Name); }