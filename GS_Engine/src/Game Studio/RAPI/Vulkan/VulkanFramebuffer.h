#pragma once

#include "Core.h"

#include "RAPI/Framebuffer.h"

#include "Extent.h"
#include "Native/Vk_Framebuffer.h"

class VulkanRenderPass;
class VulkanImage;

struct VkExtent2D;

struct VkAttachmentDescription;
struct VkSubpassDescription;

GS_CLASS VulkanFramebuffer final : public Framebuffer
{
	Vk_Framebuffer m_Framebuffer;

public:
	VulkanFramebuffer(const Vk_Device& _Device, VulkanRenderPass* _RP, Extent2D _Extent, Image* _Images, uint8 _ImagesCount);
	~VulkanFramebuffer();

	INLINE const Vk_Framebuffer& GetVk_Framebuffer() const { return m_Framebuffer; }
};