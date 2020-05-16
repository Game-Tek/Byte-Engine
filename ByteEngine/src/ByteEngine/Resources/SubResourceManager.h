#pragma once

#include "ByteEngine/Core.h"

#include <GTSL/Mutex.h>
#include <GTSL/Allocator.h>

struct ResourceManagerBigAllocatorReference final : GTSL::AllocatorReference
{
	explicit ResourceManagerBigAllocatorReference(const char* name) : name(name)
	{
	}
	
	~ResourceManagerBigAllocatorReference() = default;
	
protected:
	void allocateFunc(const uint64 size, uint64 alignment, void** memory, uint64* allocatedSize) const;

	void deallocateFunc(const uint64 size, uint64 alignment, void* memory) const;
	
	const char* name{ nullptr };
};

struct ResourceManagerTransientAllocatorReference final : GTSL::AllocatorReference
{
	explicit ResourceManagerTransientAllocatorReference(const char* name) : name(name)
	{
	}
	
	~ResourceManagerTransientAllocatorReference() = default;
	
protected:
	void allocateFunc(const uint64 size, uint64 alignment, void** memory, uint64* allocatedSize) const;

	void deallocateFunc(const uint64 size, uint64 alignment, void* memory) const;
	
	const char* name{ nullptr };
};

/**
 * \brief Used to specify a type of resource loader. When inherited it's functions implementation should load resources as per request
 * from the ResourceManager.
 *
 * This class will be instanced sometime during the application's lifetime to allow loading of some type of resource made possible by extension of this class.
 * 
 * Every extension will allow for loading of 1 type of resource specified with a pretty name by the GetResourceType() function. Users will request loading of
 * some type of resource by asking for a resource of this "pretty" name type.
 */
class SubResourceManager
{
protected:
	ResourceManagerBigAllocatorReference bigAllocator;
	ResourceManagerTransientAllocatorReference transientAllocator;

	GTSL::ReadWriteMutex resourceMapMutex;
public:
	explicit SubResourceManager(const char* resourceType) : bigAllocator(resourceType), transientAllocator(resourceType)
	{	
	}
	
	~SubResourceManager() = default;
};