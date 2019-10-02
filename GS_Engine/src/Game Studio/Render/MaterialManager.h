#pragma once

#include <map>

#include "Containers/Id.h"

#include "Material.h"

class MaterialManager
{
	std::map<Id, Material*> Materials;
	FVector<Material*> MaterialList;
public:
	~MaterialManager()
	{
		for (auto const& x : Materials)
		{
			delete x.second;
		}
	}

	template<class T>
	Material* AddMaterial()
	{
		Material* NewMaterial = new T();
		if (!Materials.try_emplace(Id(NewMaterial->GetMaterialName()), NewMaterial).second)
		{
			delete NewMaterial;
			NewMaterial = nullptr;
		}
		MaterialList.emplace_back(NewMaterial);
		return NewMaterial;
	}

	Material* GetMaterial(const char* _MaterialName)
	{
		return Materials[Id(_MaterialName)];
	}

	[[nodiscard]] uint32 GetMaterialCount() const { return Materials.size(); }
	[[nodiscard]] const FVector<Material*>& GetMaterialList() const { return MaterialList; }
};
