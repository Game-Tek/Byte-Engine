#include "Vk_RenderPass.h"

#include "RAPI/Vulkan/Vulkan.h"

#include "Vk_Device.h"

Vk_RenderPassCreateInfo Vk_RenderPass::CreateVk_RenderPassCreateInfo(const Vk_Device& _Device, const VkRenderPassCreateInfo* _VkRPCI)
{
	VkRenderPass RenderPass = VK_NULL_HANDLE;
	GS_VK_CHECK(vkCreateRenderPass(_Device, _VkRPCI, ALLOCATOR, &RenderPass), "Failed to create RenderPass!");
	return { _Device, RenderPass };
}

Vk_RenderPass::Vk_RenderPass(const Vk_RenderPassCreateInfo& _Vk_RenderPassCreateInfo) : VulkanObject(_Vk_RenderPassCreateInfo.m_Device), RenderPass(_Vk_RenderPassCreateInfo.RenderPass)
{
}

Vk_RenderPass::~Vk_RenderPass()
{
	vkDestroyRenderPass(m_Device, RenderPass, ALLOCATOR);
}
