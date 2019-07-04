#pragma once

#include "Core.h"

#include "..\Framebuffer.h"
#include "VulkanBase.h"

class RenderPass;

MAKE_VK_HANDLE(VkFramebuffer)

GS_CLASS VulkanFramebuffer final : public Framebuffer, public VulkanObject
{
	VkFramebuffer Framebuffer = nullptr;
public:
	VulkanFramebuffer(VkDevice _Device, RenderPass* _RP, Extent2D _Extent);
	~VulkanFramebuffer();

	INLINE VkFramebuffer GetVkFramebuffer() const { return Framebuffer; }
};