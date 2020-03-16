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
	mutable std::unordered_map<Id64::HashType, Resource*> ResourceMap;

	static FString GetBaseResourcePath() { return FString("resources/"); }
	void SaveFile(const FString& _ResourceName, FString& fileName, ResourceData& ResourceData_);

	void LoadResource(const FString& _ResourceName, Resource* _Resource);

	std::unordered_map<Id64::HashType, SubResourceManager*> resourceManagers;
	
public:
	
	ResourceManager()
	{
		for (auto& resource_manager : resourceManagers)
		{
			delete resource_manager.second;
		}
		
		for (auto& element : ResourceMap)
		{
			delete element.second;
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
