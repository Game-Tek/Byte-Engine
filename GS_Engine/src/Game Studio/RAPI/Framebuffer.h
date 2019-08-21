#pragma once

#include "Core.h"

#include "Extent.h"
#include "RenderCore.h"

#include "Containers/DArray.hpp"

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
	DArray<Image*> Images;
};

GS_CLASS Framebuffer
{
	Extent2D Extent;
public:
	explicit Framebuffer(Extent2D _Extent) :
		Extent(_Extent)
	{
	}

	virtual ~Framebuffer() = default;

	[[nodiscard]] const Extent2D& GetExtent() const { return Extent; }
};