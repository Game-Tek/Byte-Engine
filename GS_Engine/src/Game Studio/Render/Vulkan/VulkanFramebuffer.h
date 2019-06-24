#pragma once

#include "Core.h"

#include "..\Framebuffer.h"
#include "VulkanBase.h"

MAKE_VK_HANDLE(VkFramebuffer)

GS_CLASS VulkanFramebuffer final : public Framebuffer, public VulkanObject
{
	VkFramebuffer Framebuffer = nullptr;
public:
	VulkanFramebuffer(VkDevice _Device);
	~VulkanFramebuffer();
};