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
	mutable std::unordered_map<Id::HashType, Resource*> ResourceMap;

	static FString GetBaseResourcePath() { return FString("resources/"); }
	void SaveFile(const FString& _ResourceName, FString& fileName, ResourceData& ResourceData_);

	void LoadResource(const FString& _ResourceName, Resource* _Resource);

	std::unordered_map<Id::HashType, SubResourceManager*> resourceManagers;
	
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

	ResourceReference GetResource(const FString& name, const Id& type);

	template <class T>
	void CreateResource(const FString& _Name, ResourceData& ResourceData_)
	{
		Resource* resource = new T();
		resource->makeFromData(ResourceData_);
		//auto path = _Name + "." + resource->getResourceTypeExtension();
		//SaveFile(_Name, path, ResourceData_);
		//LoadResource(_Name, resource);
	}

	void ReleaseResource(Resource* _Resource) const;
	void ReleaseResource(const ResourceReference& resourceReference) const;
	void ReleaseResource(const Id& resourceType, const Id& resourceName);

	void* CreateFile();

	template<class T>
	void CreateSubResourceManager()
	{
		auto new_resource_manager = static_cast<SubResourceManager*>(new T());
		
		resourceManagers.insert({ Id(new_resource_manager->GetResourceType()), new_resource_manager });
	}

	[[nodiscard]] const char* GetName() const override { return "Resource Manager"; }
};
