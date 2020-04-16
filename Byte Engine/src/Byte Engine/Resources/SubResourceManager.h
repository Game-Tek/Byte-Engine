#pragma once

#include <GTSL/String.hpp>
#include <GTSL/Array.hpp>
#include <GTSL/Mutex.h>
#include <GTSL/Id.h>

struct ResourceManagerBigAllocatorReference final : AllocatorReference
{
	explicit ResourceManagerBigAllocatorReference(const char* name) : name(GTSL::String::StringLength(name), name)
	{
	}
	
	virtual ~ResourceManagerBigAllocatorReference() = default;
	void Allocate(uint64 size, uint64 alignment, void** memory, uint64* allocatedSize) const override;
	void Deallocate(uint64 size, uint64 alignment, void* memory) const override;
	
protected:
	GTSL::Array<char, 255> name;
};

struct ResourceManagerTransientAllocatorReference final : AllocatorReference
{
	explicit ResourceManagerTransientAllocatorReference(const char* name) : name(GTSL::String::StringLength(name), name)
	{
	}
	
	virtual ~ResourceManagerTransientAllocatorReference() = default;
	void Allocate(uint64 size, uint64 alignment, void** memory, uint64* allocatedSize) const override;
	void Deallocate(uint64 size, uint64 alignment, void* memory) const override;
	
protected:
	GTSL::Array<char, 255> name;
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

	ReadWriteMutex resourceMapMutex;
public:
	explicit SubResourceManager(const char* resourceType) : bigAllocator(resourceType), transientAllocator(resourceType)
	{	
	}
	
	~SubResourceManager() = default;
};