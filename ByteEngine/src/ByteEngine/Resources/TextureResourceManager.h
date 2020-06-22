#pragma once

#include "SubResourceManager.h"
#include <GTSL/Extent.h>
#include "ResourceData.h"
#include <GTSL/Id.h>
#include <GAL/RenderCore.h>
#include <GTSL/Delegate.hpp>

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
	
	struct TextureLoadInfo
	{
		GTSL::Ranger<byte> TextureDataBuffer;
		GTSL::Delegate<void(OnTextureLoadInfo)> OnTextureLoadInfo;
		GTSL::Extent3D TextureExtent;
		float32 LODPercentage{ 0.0f };
	};
};