#pragma once

#include "Core.h"

#include "Object.h"

#include <unordered_map>
#include <GTSL/String.hpp>
#include <GTSL/Id.h>
#include "SubResourceManager.h"

class ResourceManager : public Object
{
	//void SaveFile(const GTSL::String& _ResourceName, GTSL::String& fileName, ResourceData& ResourceData_);

	std::unordered_map<GTSL::Id64::HashType, SubResourceManager*> resourceManagers;
	AllocatorReference* allocatorReference{ nullptr };
	
public:
	ResourceManager()
	{
		for (auto& resource_manager : resourceManagers)
		{
			delete resource_manager.second;
		}
	}

	template<class T>
	T* GetSubResourceManager()
	{
		const auto resource_manager = resourceManagers.find(T::type);
		BE_ASSERT(resource_manager == resourceManagers.end(), "A resource manager for the specified resource type could not be found! Remember to register all needed resource managers on startup.")
		return static_cast<T*>(resource_manager->second);
	}

	void* CreateFile();

	template<class T>
	void CreateSubResourceManager()
	{
		const auto new_resource_manager = static_cast<SubResourceManager*>(new T());
		resourceManagers.insert({ T::type, new_resource_manager });
	}

	[[nodiscard]] const char* GetName() const override { return "Resource Manager"; }
};
