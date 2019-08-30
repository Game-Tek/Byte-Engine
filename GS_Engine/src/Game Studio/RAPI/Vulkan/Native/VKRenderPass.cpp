#include "VKRenderPass.h"

#include "RAPI/Vulkan/Vulkan.h"

#include "VKDevice.h"

VKRenderPassCreator::VKRenderPassCreator(const VKDevice& _Device, const VkRenderPassCreateInfo* _VkRPCI) : VKObjectCreator(_Device)
{
	GS_VK_CHECK(vkCreateRenderPass(m_Device, _VkRPCI, ALLOCATOR, &Handle), "Failed to create RenderPass!");
}

VKRenderPass::~VKRenderPass()
{
	vkDestroyRenderPass(m_Device, Handle, ALLOCATOR);
}
