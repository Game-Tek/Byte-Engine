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

	template <class T>
	T* GetResource(const FString& _ResourceName)
	{
		auto HashedName = Id(_ResourceName);

		auto Loc = ResourceMap.find(HashedName.GetID());

		if (Loc != ResourceMap.cend())
		{
			Loc->second->incrementReferences();
			return static_cast<T*>(Loc->second);
		}

		Resource* resource = new T();

		LoadResource(_ResourceName, resource);

		ResourceMap.insert({Id(_ResourceName).GetID(), resource});

		return SCAST(T*, resource);
	}

	SubResourceManager::OnResourceLoadInfo GetResource(const FString& name, const Id& type);
	

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

	void* CreateFile();

	template<class T>
	void CreateSubResourceManager()
	{
		auto new_resource_manager = static_cast<SubResourceManager*>(new T());
		
		resourceManagers.insert({ Id(new_resource_manager->GetResourceTypeName()), new_resource_manager });
	}

	[[nodiscard]] const char* GetName() const override { return "Resource Manager"; }
};
