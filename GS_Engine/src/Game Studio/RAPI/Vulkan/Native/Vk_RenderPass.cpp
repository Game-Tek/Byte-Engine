#include "Vk_RenderPass.h"

#include "RAPI/Vulkan/Vulkan.h"

#include "Vk_Device.h"

Vk_RenderPass::Vk_RenderPass(const Vk_Device& _Device, const Tuple<FVector<VkAttachmentDescription>, FVector<VkSubpassDescription>>& _Info) : VulkanObject(_Device)
{
	VkRenderPassCreateInfo RenderPassCreateInfo = { VK_STRUCTURE_TYPE_RENDER_PASS_CREATE_INFO };
	RenderPassCreateInfo.attachmentCount = _Info.First.length();
	RenderPassCreateInfo.pAttachments = _Info.First.data();
	RenderPassCreateInfo.subpassCount = _Info.Second.length();
	RenderPassCreateInfo.pSubpasses = _Info.Second.data();

	GS_VK_CHECK(vkCreateRenderPass(m_Device, &RenderPassCreateInfo, ALLOCATOR, &RenderPass), "Failed to create RenderPass!")
}

Vk_RenderPass::~Vk_RenderPass()
{
	vkDestroyRenderPass(m_Device, RenderPass, ALLOCATOR);
}
