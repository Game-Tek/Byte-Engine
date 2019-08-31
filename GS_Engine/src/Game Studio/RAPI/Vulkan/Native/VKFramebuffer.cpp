#include "VKFramebuffer.h"

#include "RAPI/Vulkan/Vulkan.h"

#include "VKDevice.h"

VKFramebufferCreator::VKFramebufferCreator(VKDevice* _Device, const VkFramebufferCreateInfo* _VkFCI) : VKObjectCreator<VkFramebuffer>(_Device)
{
	GS_VK_CHECK(vkCreateFramebuffer(m_Device->GetVkDevice(), _VkFCI, ALLOCATOR, &Handle), "Failed to create Framebuffer!")
}

VKFramebuffer::~VKFramebuffer()
{
	vkDestroyFramebuffer(m_Device->GetVkDevice(), Handle, ALLOCATOR);
}
