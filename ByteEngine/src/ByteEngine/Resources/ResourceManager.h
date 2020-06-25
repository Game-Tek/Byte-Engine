#pragma once

#include "ByteEngine/Object.h"

#include <GTSL/FlatHashMap.h>
#include <GTSL/Id.h>
#include <GTSL/StaticString.hpp>


#include "SubResourceManager.h"

/**
 * \brief Manages a set of named sub-resource managers which can be added dynamically.
 */
class ResourceManager : public Object
{
public:
	ResourceManager();
	
	virtual ~ResourceManager()
	{
		ForEach(resourceManagers, [&](GTSL::Allocation<SubResourceManager>& allocation){ Delete(allocation, GetPersistentAllocator()); });
		resourceManagers.Free(GetPersistentAllocator());
	}

	SubResourceManager* GetSubResourceManager(const GTSL::Id64 name)
	{
		return resourceManagers.At(name).Data;
		//BE_ASSERT(resource_manager == resourceManagers.end(), "A resource manager for the specified resource type could not be found! Remember to register all needed resource managers on startup.")
	}

	template<class T>
	T* CreateSubResourceManager(const GTSL::Id64 name)
	{
		return static_cast<T*>(resourceManagers.Emplace(GetPersistentAllocator(), name, GTSL::Allocation<SubResourceManager>::Create<T>(GetPersistentAllocator()))->Data);
	}

	[[nodiscard]] const char* GetName() const override { return "Resource Manager"; }

	[[nodiscard]] GTSL::StaticString<256> GetResourcePath() const;
	
protected:
	GTSL::FlatHashMap<GTSL::Allocation<SubResourceManager>> resourceManagers;
};