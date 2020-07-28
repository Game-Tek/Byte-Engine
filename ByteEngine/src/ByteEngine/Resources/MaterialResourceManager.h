#pragma once

#include <GTSL/Delegate.hpp>
#include <GTSL/File.h>
#include <GTSL/FlatHashMap.h>
#include "ResourceManager.h"

class MaterialResourceManager final : public ResourceManager
{
public:
	MaterialResourceManager();
	~MaterialResourceManager();
	
	struct MaterialInfo
	{
		uint32 VertexShaderOffset = 0, VertexShaderSize = 0, FragmentShaderSize = 0;
		GTSL::Array<uint8, 20> VertexElements;
		GTSL::Array<GTSL::Array<uint8, 12>, 12> BindingSets;
		friend void Insert(const MaterialInfo& materialInfo, GTSL::Buffer& buffer);
		friend void Extract(MaterialInfo& materialInfo, GTSL::Buffer& buffer);
	};

	struct MaterialCreateInfo
	{
		GTSL::StaticString<128> ShaderName;
		GTSL::Ranger<const uint8> VertexFormat;
		GTSL::Ranger<const GTSL::Ranger<const uint8>> BindingSets;
	};
	void CreateMaterial(const MaterialCreateInfo& materialCreateInfo);

	void GetMaterialSize(GTSL::Id64 name, uint32& size);
	
	struct OnMaterialLoadInfo : OnResourceLoad
	{
	};
	
	struct MaterialLoadInfo : ResourceLoadInfo
	{
		GTSL::Delegate<void(TaskInfo, OnMaterialLoadInfo)> OnMaterialLoad;
	};
	void LoadMaterial(const MaterialLoadInfo& loadInfo);
	
private:
	GTSL::File package, index;
	GTSL::FlatHashMap<MaterialInfo, BE::PersistentAllocatorReference> materialInfos;
	GTSL::ReadWriteMutex mutex;
};
