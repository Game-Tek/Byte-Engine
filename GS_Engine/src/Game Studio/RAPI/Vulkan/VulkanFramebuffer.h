#pragma once

#include "Core.h"

#include "../Framebuffer.h"
#include "VulkanBase.h"

#include "Extent.h"

class VulkanImage;
class RenderPass;

MAKE_VK_HANDLE(VkFramebuffer)
MAKE_VK_HANDLE(VkRenderPass)
MAKE_VK_HANDLE(VkImageView)

struct VkExtent2D;

GS_CLASS Vk_Framebuffer final : public VulkanObject
{
	VkFramebuffer Framebuffer = nullptr;

public:
	Vk_Framebuffer(VkDevice _Device, VkRenderPass _RP, VkExtent2D _Extent, VulkanImage* _ImageViews, uint8 _AttachmentCount);
	~Vk_Framebuffer();

	INLINE VkFramebuffer GetVkFramebuffer() const { return Framebuffer; }
};

GS_CLASS VulkanFramebuffer final : public Framebuffer
{
	Vk_Framebuffer m_Framebuffer;

	
public:
	VulkanFramebuffer(VkDevice _Device, RenderPass* _RP, Extent2D _Extent, Image* _Images, uint8 _ImagesCount);
	~VulkanFramebuffer();

	INLINE const Vk_Framebuffer& GetVk_Framebuffer() const { return m_Framebuffer; }
};