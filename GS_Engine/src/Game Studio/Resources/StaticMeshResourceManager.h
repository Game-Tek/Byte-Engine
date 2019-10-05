#pragma once

#include "Core.h"

#include "ResourceManager.h"

#include "StaticMeshResource.h"

#include <unordered_map>
#include "Debug/Logger.h"

class GS_API StaticMeshResourceManager : public ResourceManager<StaticMeshResource>
{
	std::unordered_map<StaticMeshResource*, StaticMeshResource*> ResourceMap;

	static StaticMeshResourceManager SMRM;
public:
	static StaticMeshResourceManager& Get() { return SMRM; }

	const char* GetName() const override { return "StaticMeshResourceManager"; }

	StaticMeshResource* GetResource(const FString& _Name) override
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

		auto NewObject = new StaticMeshResource(_Name);
		auto Result = NewObject->LoadResource();

		if (Result)
		{
			GS_LOG_SUCCESS("Loaded resource %s succesfully!", _Name.c_str())
		}
		else
		{
			GS_LOG_ERROR("Failed to load %s resource!", _Name.c_str())
		}

		NewObject->IncrementReferences();
		return ResourceMap.emplace(NewObject, NewObject).first->second;

		//return nullptr;
	}

	void ReleaseResource(StaticMeshResource* _Resource) override
	{
		_Resource->DecrementReferences();

		if(_Resource->GetReferenceCount() == 0)
		{
			delete ResourceMap[_Resource];
		}
	}
};

StaticMeshResourceManager StaticMeshResourceManager::SMRM;