#pragma once

#include "Core.h"

#include "Object.h"

#include "Resource.h"

#include <unordered_map>
#include "Containers/FString.h"
#include "Containers/Id.h"

#include "Debug/Logger.h"

class ResourceManager : public Object
{
	mutable std::unordered_map<Id::HashType, Resource*> ResourceMap;

	static FString GetBaseResourcePath() { return FString("resources/"); }
	void SaveFile(const FString& _ResourceName, FString& fileName, ResourceData& ResourceData_);

	void LoadResource(const FString& _ResourceName, Resource* _Resource);

public:
	ResourceManager()
	{
		for (auto& element : ResourceMap)
		{
			delete element.second;
		}
	}

	template<class T>
	T* GetResource(const FString& _ResourceName)
	{
		auto HashedName = Id(_ResourceName);
		
		auto Loc = ResourceMap.find(HashedName.GetID());
		
		if(Loc != ResourceMap.cend())
		{
			Loc->second->IncrementReferences();
			return static_cast<T*>(Loc->second);
		}

		Resource* resource = new T();

		LoadResource(_ResourceName, resource);

		ResourceMap.insert({ Id(_ResourceName).GetID(), resource});
		
		return SCAST(T*, resource);
	}

	template<class T>
	void CreateResource(const FString& _Name, ResourceData& ResourceData_)
	{
		Resource* resource = new T();
		auto path = _Name + "." + resource->GetResourceTypeExtension();
		SaveFile(_Name, path, ResourceData_);
		LoadResource(_Name, resource);
	}

	void ReleaseResource(Resource* _Resource) const;

	void* CreateFile();
	
	[[nodiscard]] const char* GetName() const override { return "Resource Manager"; }
};
