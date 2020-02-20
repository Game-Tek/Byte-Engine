#pragma once

#include "Core.h"

#include "RenderCore.h"
#include "Utility/Extent.h"

namespace RAPI
{

	struct TextureCreateInfo
	{
		void* ImageData = nullptr;
		size_t ImageDataSize = 0;
		ImageLayout Layout = ImageLayout::COLOR_ATTACHMENT;
		ImageFormat ImageFormat = ImageFormat::RGBA_I8;
		Extent2D Extent = { 1280, 720 };

		uint8 Anisotropy = 0;
	};

	//Represents a resource utilized by the rendering API for storing and referencing textures. Which are images which hold some information loaded from memory.
	class Texture
	{
		ImageLayout layout;
	public:

		explicit Texture(const TextureCreateInfo& textureCreateInfo) : layout(textureCreateInfo.Layout)
		{
		}

		[[nodiscard]] ImageLayout GetImageLayout() const { return layout; }
	};

}