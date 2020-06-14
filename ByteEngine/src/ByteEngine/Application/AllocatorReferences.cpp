#include "AllocatorReferences.h"

#include "Application.h"

void BE::SystemAllocatorReference::allocateFunc(const uint64 size, const uint64 alignment, void** memory, uint64* allocatedSize) const { (*allocatedSize) = size;	BE::Application::Get()->GetSystemAllocator()->Allocate(size, alignment, memory); }

void BE::SystemAllocatorReference::deallocateFunc(const uint64 size, const uint64 alignment, void* memory) const { BE::Application::Get()->GetSystemAllocator()->Deallocate(size, alignment, memory); }

void BE::TransientAllocatorReference::allocateFunc(const uint64 size, const uint64 alignment, void** memory, uint64* allocatedSize) const { BE::Application::Get()->GetTransientAllocator()->Allocate(size, alignment, memory, allocatedSize, Name); }

void BE::TransientAllocatorReference::deallocateFunc(const uint64 size, const uint64 alignment, void* memory) const { BE::Application::Get()->GetTransientAllocator()->Deallocate(size, alignment, memory, Name); }

void BE::PersistentAllocatorReference::allocateFunc(const uint64 size, const uint64 alignment, void** memory, uint64* allocatedSize) const { Application::Get()->GetNormalAllocator()->Allocate(size, alignment, memory, allocatedSize, Name); }

void BE::PersistentAllocatorReference::deallocateFunc(const uint64 size, const uint64 alignment, void* memory) const { Application::Get()->GetNormalAllocator()->Deallocate(size, alignment, memory, Name); }