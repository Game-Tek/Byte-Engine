#pragma once

#include "Core.h"
#include "Image.h"

struct GS_API TextureCreateInfo
{
	void* ImageData = nullptr;
	size_t ImageDataSize = 0;
	ImageUse Use = ImageUse::INPUT_ATTACHMENT;
	Format ImageFormat = Format::RGBA_I8;
	Extent2D Extent = { 1280, 720 };
};

//Represents a resource utilized by the rendering API for storing and referencing textures. Which are images which hold some information loaded from memory.
class GS_API Texture
{
public:

};