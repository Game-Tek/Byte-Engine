#include "VKRenderPass.h"

#include "RAPI/Vulkan/Vulkan.h"

#include "VKDevice.h"

VKRenderPassCreator::VKRenderPassCreator(VKDevice* _Device, const VkRenderPassCreateInfo* _VkRPCI) : VKObjectCreator(_Device)
{
	GS_VK_CHECK(vkCreateRenderPass(m_Device->GetVkDevice(), _VkRPCI, ALLOCATOR, &Handle), "Failed to create RenderPass!");
}

VKRenderPass::~VKRenderPass()
{
	vkDestroyRenderPass(m_Device->GetVkDevice(), Handle, ALLOCATOR);
}
