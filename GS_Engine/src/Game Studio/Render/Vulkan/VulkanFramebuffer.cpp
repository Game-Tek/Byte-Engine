#include "VulkanFramebuffer.h"

#include "Vulkan.h"

VulkanFramebuffer::VulkanFramebuffer(VkDevice _Device) : VulkanObject(_Device)
{
	VkFramebufferCreateInfo CreateInfo = { VK_STRUCTURE_TYPE_FRAMEBUFFER_CREATE_INFO };
	CreateInfo.renderPass = renderPass;
	CreateInfo.attachmentCount = 1;
	CreateInfo.pAttachments = attachments;
	CreateInfo.width = swapChainExtent.width;
	CreateInfo.height = swapChainExtent.height;
	CreateInfo.layers = 1;

	GS_VK_CHECK(vkCreateFramebuffer(m_Device, &CreateInfo, ALLOCATOR, &Framebuffer), "Failed to create Frambuffer!")
}

VulkanFramebuffer::~VulkanFramebuffer()
{
	vkDestroyFramebuffer(m_Device, Framebuffer, ALLOCATOR);
}