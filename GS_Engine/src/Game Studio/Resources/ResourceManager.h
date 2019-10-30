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
	//mutable std::unordered_map<Resource*, Resource*> ResourceMap;
	mutable FVector<Resource*> R;

	static FString GetBaseResourcePath() { return FString("resources/"); }
	void SaveFile(FString& _ResourceName, FString& _ResourcePath, void (*f)(OutStream& _OS));

	void GetResourceInternal(const FString& _ResourceName, Resource* _Resource) const;

public:
	ResourceManager() : R(50)
	{
	}

	template<class T>
	T* GetResource(const FString& _ResourceName) const
	{
		//auto HashedName = Id(_Name);
		//
		//auto Loc = ResourceMap.find(HashedName.GetID());
		//
		//if(Loc != ResourceMap.cend())
		//{
		//	return StaticMeshResourceHandle(&ResourceMap[HashedName.GetID()]);
		//}
		
		//auto Path = GetBaseResourcePath() + "static meshes/" + _Name + ".obj";

		Resource* resource = new T();

		GetResourceInternal(_ResourceName, resource);

		//return nullptr;
		return SCAST(T*, resource);
	}

	template<class T>
	void CreateResource(const FString& _Name, void (*f)(OutStream& _OS))
	{
		Resource* resource = new T();
		FString path = _Name + resource->GetResourceTypeExtension();
		SaveFile(const_cast<FString&>(_Name), path, f);
		GetResourceInternal(_Name, resource);
	}

	void ReleaseResource(Resource* _Resource) const;

	[[nodiscard]] const char* GetName() const override { return "Resource Manager"; }
};
