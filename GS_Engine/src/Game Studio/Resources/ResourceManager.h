#pragma once

#include "Core.h"

#include "Object.h"

#include "Containers/FString.h"
#include "Containers/Id.h"

#include <unordered_map>

#include "Debug/Logger.h"

#include "Resource.h"

class ResourceManager : public Object
{
	std::unordered_map<Resource*, Resource*> ResourceMap;

	static FString GetBaseResourcePath() { return FString("resources/"); }
public:
	template<class T>
	T* GetResource(const FString& _ResourceName)
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
			GS_LOG_SUCCESS("Loaded resource %s succesfully!", _ResourceName.c_str())
		}
		else
		{
			GS_LOG_ERROR("Failed to load %s resource of type %s!\nLoaded default resource.", _ResourceName.c_str(), resource->GetName())
			resource->LoadFallbackResource(FullPath);
		}

		resource->IncrementReferences();
		return ResourceMap.emplace(resource, resource).first->second;

		//return nullptr;
		return resource;
	}

	template<class T>
	void CreateResource(const FString& _Path, void (*f)(OutStream& _OS))
	{
		SaveFile(_Path, f);
	}

	void ReleaseResource(Resource* _Resource);

	void SaveFile(const FileDescriptor& _FD);
	void SaveFile(const FString& _Path, void (*f)(OutStream& _OS));

	[[nodiscard]] const char* GetName() const override { return "Resource Manager"; }
};
