#pragma once

#include "Core.h"

#include "Extent.h"
#include "RenderCore.h"

class Image;
class RenderPass;

GS_STRUCT FramebufferAttachments
{
	Format ColorAttachmentsFormat[8] = {};
	uint8 ColorAttachmentsCount = 0;

	Format DepthStencilFormat = Format::DEPTH16_STENCIL8;

	Image* Images = nullptr;
};

GS_STRUCT FramebufferCreateInfo
{
	RenderPass* RenderPass = nullptr;
	Extent2D Extent = { 1280, 720 };
	Image* Images = nullptr;
	uint8 ImagesCount = 0;
};

GS_CLASS Framebuffer
{
	Extent2D Extent;
public:
	Framebuffer(Extent2D _Extent) :
		Extent(_Extent)
	{
	}

	virtual ~Framebuffer() {};

	[[nodiscard]] const Extent2D& GetExtent() const { return Extent; }
};