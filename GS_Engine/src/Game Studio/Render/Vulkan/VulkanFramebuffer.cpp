#include "VulkanFramebuffer.h"

#include "Vulkan.h"

#include "VulkanRenderPass.h"

VulkanFramebuffer::VulkanFramebuffer(VkDevice _Device, RenderPass* _RP, Extent2D _Extent) : VulkanObject(_Device)
{
	VkFramebufferCreateInfo FramebufferCreateInfo = { VK_STRUCTURE_TYPE_FRAMEBUFFER_CREATE_INFO };
	FramebufferCreateInfo.renderPass = SCAST(VulkanRenderPass*, _RP)->GetVk_RenderPass().GetVkRenderPass();
	FramebufferCreateInfo.attachmentCount = 1;
	FramebufferCreateInfo.pAttachments = attachments;
	FramebufferCreateInfo.width = _Extent.Width;
	FramebufferCreateInfo.height = _Extent.Height;
	FramebufferCreateInfo.layers = 1;

	GS_VK_CHECK(vkCreateFramebuffer(m_Device, &FramebufferCreateInfo, ALLOCATOR, &Framebuffer), "Failed to create Framebuffer!")
}

VulkanFramebuffer::~VulkanFramebuffer()
{
	vkDestroyFramebuffer(m_Device, Framebuffer, ALLOCATOR);
}