#pragma once

#include "ResourceManager.h"

#include <GTSL/Extent.h>
#include <GAL/RenderCore.h>
#include <GTSL/Delegate.hpp>
#include <GTSL/File.h>
#include <GTSL/FlatHashMap.h>

class TextureResourceManager final : public ResourceManager
{
public:
	TextureResourceManager();
	~TextureResourceManager();
	
	struct TextureInfo
	{
		uint32 ByteOffset = 0;
		uint32 ImageSize = 0;
		GAL::Dimension Dimensions;
		GTSL::Extent3D Extent;
		uint8 Format = 0;
	};
	
	struct OnTextureLoadInfo : OnResourceLoad
	{
		GAL::TextureFormat TextureFormat;
		GTSL::Extent3D Extent;
		GAL::Dimension Dimensions;
		float32 LODPercentage{ 0.0f };
	};
	
	void GetTextureSizeFormatExtent(const GTSL::Id64 name, uint32* size, GAL::TextureFormat* format, GTSL::Extent3D* extent)
	{
		auto& e = textureInfos.At(name);
		*size = e.ImageSize;
		*format = static_cast<GAL::TextureFormat>(e.Format);
		*extent = e.Extent;
	}
	
	struct TextureLoadInfo : ResourceLoadInfo
	{
		GTSL::Delegate<void(TaskInfo, OnTextureLoadInfo)> OnTextureLoadInfo;
		GTSL::Extent3D TextureExtent;
		float32 LODPercentage{ 0.0f };
	};
	void LoadTexture(const TextureLoadInfo& textureLoadInfo);

private:
	GTSL::File packageFile, indexFile;
	GTSL::FlatHashMap<TextureInfo, BE::PersistentAllocatorReference> textureInfos;
	
};

void Insert(const TextureResourceManager::TextureInfo& textureInfo, GTSL::Buffer& buffer);
void Extract(TextureResourceManager::TextureInfo& textureInfo, GTSL::Buffer& buffer);