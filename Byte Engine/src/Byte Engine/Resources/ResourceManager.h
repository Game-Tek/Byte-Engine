#pragma once

#include "Core.h"

#include "Object.h"

#include "Resource.h"

#include <unordered_map>
#include "Containers/FString.h"
#include "Containers/Id.h"
#include "SubResourceManager.h"

class ResourceManager : public Object
{
	static FString GetBaseResourcePath() { return FString("resources/"); }
	void SaveFile(const FString& _ResourceName, FString& fileName, ResourceData& ResourceData_);

	std::unordered_map<Id64::HashType, SubResourceManager*> resourceManagers;
	
public:
	
	ResourceManager()
	{
		for (auto& resource_manager : resourceManagers)
		{
			delete resource_manager.second;
		}
	}

	ResourceReference TryGetResource(const FString& name, const Id64& type);
	ResourceData* GetResource(const ResourceReference& resourceReference);
	
	void ReleaseResource(const ResourceReference& resourceReference) const;
	void ReleaseResource(const Id64& resourceType, const Id64& resourceName);

	void* CreateFile();

	template<class T>
	void CreateSubResourceManager()
	{
		auto new_resource_manager = static_cast<SubResourceManager*>(new T());
		
		resourceManagers.insert({ Id64(new_resource_manager->GetResourceType()), new_resource_manager });
	}

	[[nodiscard]] const char* GetName() const override { return "Resource Manager"; }
};
