#include "Vk_Framebuffer.h"

#include "RAPI/Vulkan/Vulkan.h"

Vk_Framebuffer::Vk_Framebuffer(const Vk_Device& _Device, Extent2D _Extent, const Vk_RenderPass& _RP, const FVector<Vk_ImageView>& _Images) : VulkanObject(_Device)
{
	VkFramebufferCreateInfo FramebufferCreateInfo = { VK_STRUCTURE_TYPE_FRAMEBUFFER_CREATE_INFO };
	FramebufferCreateInfo.renderPass = _RP;
	FramebufferCreateInfo.attachmentCount = _Images.length();
	FramebufferCreateInfo.pAttachments = _Images.data();
	FramebufferCreateInfo.width = _Extent.Width;
	FramebufferCreateInfo.height = _Extent.Height;
	FramebufferCreateInfo.layers = 1;

	GS_VK_CHECK(vkCreateFramebuffer(m_Device, &FramebufferCreateInfo, ALLOCATOR, &Framebuffer), "Failed to create Framebuffer!")
}

Vk_Framebuffer::~Vk_Framebuffer()
{
	vkDestroyFramebuffer(m_Device, Framebuffer, ALLOCATOR);
}