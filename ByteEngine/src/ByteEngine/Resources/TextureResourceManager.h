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
		uint8 Format = 0;
	};
	
	struct OnTextureLoadInfo : OnResourceLoad
	{
		GAL::ImageFormat TextureFormat;
		float32 LODPercentage{ 0.0f };
	};
	
	struct TextureLoadInfo : ResourceLoadInfo
	{
		GTSL::Delegate<void(OnTextureLoadInfo)> OnTextureLoadInfo;
		GTSL::Extent3D TextureExtent;
		float32 LODPercentage{ 0.0f };
	};

	void LoadTexture(const TextureLoadInfo& textureLoadInfo);

private:
	GTSL::File packageFile, indexFile;
	GTSL::FlatHashMap<TextureInfo> textureInfos;
	GTSL::FlatHashMap<TextureInfo> textureAssets;
	
};

void Insert(const TextureResourceManager::TextureInfo& textureInfo, GTSL::Buffer& buffer, const GTSL::AllocatorReference& allocatorReference);
void Extract(TextureResourceManager::TextureInfo& textureInfo, GTSL::Buffer& buffer, const GTSL::AllocatorReference& allocatorReference);