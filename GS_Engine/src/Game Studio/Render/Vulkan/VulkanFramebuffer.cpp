#include "Vulkan.h"

#include "VulkanFramebuffer.h"

#include "VulkanRenderPass.h"

VulkanFramebuffer::VulkanFramebuffer(VkDevice _Device, RenderPass* _RP, Extent2D _Extent, const FramebufferAttachments& _FA) : Framebuffer(_Extent),
	m_Framebuffer(_Device, SCAST(VulkanRenderPass*, _RP)->GetVk_RenderPass().GetVkRenderPass(), Extent2DToVkExtent2D(_Extent), _FA.Images,_FA.ColorAttachmentsCount)
{

}

VulkanFramebuffer::~VulkanFramebuffer()
{
}

Vk_Framebuffer::Vk_Framebuffer(VkDevice _Device, VkRenderPass _RP, VkExtent2D _Extent, VkImageView* _ImageViews, uint8 _AttachmentCount) : VulkanObject(_Device)
{
	VkFramebufferCreateInfo FramebufferCreateInfo = { VK_STRUCTURE_TYPE_FRAMEBUFFER_CREATE_INFO };
	FramebufferCreateInfo.renderPass = _RP;
	FramebufferCreateInfo.attachmentCount =_AttachmentCount;
	FramebufferCreateInfo.pAttachments = _ImageViews;
	FramebufferCreateInfo.width = _Extent.width;
	FramebufferCreateInfo.height = _Extent.height;
	FramebufferCreateInfo.layers = 1;

	GS_VK_CHECK(vkCreateFramebuffer(m_Device, &FramebufferCreateInfo, ALLOCATOR, &Framebuffer), "Failed to create Framebuffer!")
}

Vk_Framebuffer::~Vk_Framebuffer()
{
	vkDestroyFramebuffer(m_Device, Framebuffer, ALLOCATOR);
}