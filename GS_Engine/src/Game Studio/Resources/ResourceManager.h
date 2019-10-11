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
public:
	class ResourcePush
	{
		FVector<SaveResourceElementDescriptor> FileElements;

	public:
		ResourcePush& operator+=(const SaveResourceElementDescriptor& _FED)
		{
			FileElements.push_back(_FED);
			return *this;
		}

		const SaveResourceElementDescriptor& operator[](uint64 _I) const { return FileElements[_I]; }

		[[nodiscard]] uint64 GetElementCount() const { return FileElements.length(); }
	};

private:
	mutable std::unordered_map<Resource*, Resource*> ResourceMap;

	static FString GetBaseResourcePath() { return FString("resources/"); }
	void SaveFile(const FString& _Path, void (*f)(ResourcePush& _OS));
public:
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
		const auto FullPath = _ResourceName;
		const auto Result = resource->LoadResource(_ResourceName);

		if (Result)
		{
			//GS_LOG_SUCCESS("Loaded resource %s succesfully!", _ResourceName.c_str())
		}
		else
		{
			//GS_LOG_ERROR("Failed to load %s resource of type %s!\nLoaded default resource.", _ResourceName.c_str(), resource->GetName())
			resource->LoadFallbackResource(FullPath);
		}

		resource->IncrementReferences();
		ResourceMap.emplace(resource, resource);

		//return nullptr;
		return SCAST(T*, resource);
	}

	template<class T>
	void CreateResource(const FString& _Path, void (*f)(ResourcePush& _OS))
	{
		SaveFile(_Path, f);
		GetResource<T>(_Path);
	}

	void ReleaseResource(Resource* _Resource) const;

	[[nodiscard]] const char* GetName() const override { return "Resource Manager"; }
};
