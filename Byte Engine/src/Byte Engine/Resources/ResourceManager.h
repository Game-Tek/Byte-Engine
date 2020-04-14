#pragma once

#include "Core.h"

#include "Object.h"

#include <unordered_map>
#include <GTSL/String.hpp>
#include <GTSL/Id.h>
#include "SubResourceManager.h"

class ResourceManager : public Object
{
	void SaveFile(const GTSL::String& _ResourceName, GTSL::String& fileName, ResourceData& ResourceData_);

	std::unordered_map<GTSL::Id64::HashType, SubResourceManager*> resourceManagers;
	
public:
	
	ResourceManager()
	{
		for (auto& resource_manager : resourceManagers)
		{
			delete resource_manager.second;
		}
	}

	ResourceReference TryGetResource(const GTSL::String& name, const GTSL::Id64& type);
	ResourceData* GetResource(const ResourceReference& resourceReference);
	
	void ReleaseResource(const ResourceReference& resourceReference) const;
	void ReleaseResource(const GTSL::Id64& resourceType, const GTSL::Id64& resourceName);

	void* CreateFile();

	template<class T>
	void CreateSubResourceManager()
	{
		auto new_resource_manager = static_cast<SubResourceManager*>(new T());
		
		resourceManagers.insert({ GTSL::Id64(new_resource_manager->GetResourceType()), new_resource_manager });
	}

	[[nodiscard]] const char* GetName() const override { return "Resource Manager"; }
};
