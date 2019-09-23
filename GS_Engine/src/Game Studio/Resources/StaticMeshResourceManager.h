#pragma once

#include "Core.h"

#include "ResourceManager.h"

#include "StaticMesh.h"

#include <unordered_map>

class GS_API StaticMeshResourceManager : public ResourceManager<StaticMesh>
{
	std::unordered_map<StaticMesh*, StaticMesh*> ResourceMap;

	static StaticMeshResourceManager SMRM;
public:
	static StaticMeshResourceManager& Get() { return SMRM; }

	const char* GetName() const override { return "StaticMeshResourceManager"; }

	StaticMesh* GetResource(const FString& _Name) override
	{
		//auto HashedName = Id(_Name);
		//
		//auto Loc = ResourceMap.find(HashedName.GetID());
		//
		//if(Loc != ResourceMap.cend())
		//{
		//	return StaticMeshResourceHandle(&ResourceMap[HashedName.GetID()]);
		//}

		auto NewObject = new StaticMesh(GetBaseResourcePath() + "static meshes/" + _Name + ".obj");
		NewObject->IncrementReferences();
		return ResourceMap.emplace(NewObject, NewObject).first->second;

		//return nullptr;
	}

	void ReleaseResource(StaticMesh* _Resource) override
	{
		_Resource->DecrementReferences();

		if(_Resource->GetReferenceCount() == 0)
		{
			delete ResourceMap[_Resource];
		}
	}
};

StaticMeshResourceManager StaticMeshResourceManager::SMRM;