#pragma once

#include "SubResourceManager.h"
#include <GTSL/Extent.h>
#include "ResourceData.h"
#include <GAL/RenderCore.h>
#include <GTSL/Delegate.hpp>
#include <GTSL/File.h>
#include <GTSL/FlatHashMap.h>

struct TextureResourceData final : ResourceHandle
{
};

class TextureResourceManager final : public SubResourceManager
{
public:
	TextureResourceManager();
	~TextureResourceManager();
	
	const char* GetName() const override { return "Texture Resource Manager"; }
	
	struct TextureInfo
	{
		uint32 ByteOffset = 0;
		uint32 ImageSize = 0;
		uint8 Format = 0;
	};
	
	struct OnTextureLoadInfo
	{
		GTSL::Ranger<byte> TextureDataBuffer;
		GAL::ImageFormat TextureFormat;
		float32 LODPercentage{ 0.0f };
	};
	
	struct TextureLoadInfo : ResourceLoadInfo
	{
		GTSL::Ranger<byte> TextureDataBuffer;
		GTSL::Delegate<void(OnTextureLoadInfo)> OnTextureLoadInfo;
		GTSL::Extent3D TextureExtent;
		float32 LODPercentage{ 0.0f };
	};

	void LoadTexture(const TextureLoadInfo& textureLoadInfo);

private:
	GTSL::File packageFile;
	GTSL::File indexFile;
	GTSL::FlatHashMap<TextureInfo> textureInfos;
	GTSL::FlatHashMap<TextureInfo> textureAssets;
	
};