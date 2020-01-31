#pragma once

#include "Core.h"

#include "Utility/Extent.h"
#include "RenderCore.h"

#include "Containers/DArray.hpp"
#include "Containers/FVector.hpp"
#include "Utility/RGBA.h"

namespace RAPI
{
	class RenderPass;
	class Image;

	struct FramebufferAttachments
	{
		Format ColorAttachmentsFormat[8] = {};
		uint8 ColorAttachmentsCount = 0;

		Format DepthStencilFormat = Format::DEPTH16_STENCIL8;

		Image* Images = nullptr;
	};

	struct FramebufferCreateInfo
	{
		RenderPass* RenderPass = nullptr;
		Extent2D Extent = { 1280, 720 };
		DArray<Image*> Images;
		FVector<RGBA> ClearValues;
	};

	class Framebuffer
	{
	protected:
		Extent2D Extent;
		uint8 attachmentCount = 0;
	public:
		explicit Framebuffer(const FramebufferCreateInfo& framebufferCreateInfo) :
			Extent(framebufferCreateInfo.Extent)
		{
		}

		virtual ~Framebuffer() = default;

		[[nodiscard]] const Extent2D& GetExtent() const { return Extent; }
		[[nodiscard]] uint8 GetAttachmentCount() const { return attachmentCount; };
	};
}
