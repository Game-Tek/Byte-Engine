#pragma once

#include "SubResourceManager.h"
#include <GTSL/Extent.h>
#include "ResourceData.h"
#include <GAL/RenderCore.h>
#include <GTSL/Delegate.hpp>
#include <GTSL/FlatHashMap.h>
#include <GTSL/StaticString.hpp>

struct TextureResourceData final : ResourceHandle
{
};

class TextureResourceManager final : public SubResourceManager
{
public:
	TextureResourceManager() : SubResourceManager("Texture")
	{
	}

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
	//GTSL::FlatHashMap<
};