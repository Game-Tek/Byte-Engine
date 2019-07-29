#include "Vk_RenderPass.h"

#include "RAPI/Vulkan/Vulkan.h"

Vk_RenderPass::Vk_RenderPass(const Vk_Device& _Device, const FVector<VkAttachmentDescription>& _Attachments, const FVector<VkSubpassDescription>& _Subpasses) : VulkanObject(_Device)
{
	VkRenderPassCreateInfo RenderPassCreateInfo = { VK_STRUCTURE_TYPE_RENDER_PASS_CREATE_INFO };
	RenderPassCreateInfo.attachmentCount = _Attachments.length();
	RenderPassCreateInfo.pAttachments = _Attachments.data();
	RenderPassCreateInfo.subpassCount = _Subpasses.length();
	RenderPassCreateInfo.pSubpasses = _Subpasses.data();

	GS_VK_CHECK(vkCreateRenderPass(m_Device, &RenderPassCreateInfo, ALLOCATOR, &RenderPass), "Failed to create RenderPass!")
}

Vk_RenderPass::~Vk_RenderPass()
{
	vkDestroyRenderPass(m_Device, RenderPass, ALLOCATOR);
}
