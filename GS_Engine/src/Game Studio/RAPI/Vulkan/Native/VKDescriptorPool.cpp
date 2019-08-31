#include "VKDescriptorPool.h"

#include "VKDevice.h"

#include "RAPI/Vulkan/Vulkan.h"

VKDescriptorPoolCreator::VKDescriptorPoolCreator(VKDevice* _Device, const VkDescriptorPoolCreateInfo* _VkDPCI) : VKObjectCreator<VkDescriptorPool>(_Device)
{
	vkCreateDescriptorPool(m_Device->GetVkDevice(), _VkDPCI, ALLOCATOR, &Handle);
}

VKDescriptorPool::~VKDescriptorPool()
{
	vkDestroyDescriptorPool(m_Device->GetVkDevice(), Handle, ALLOCATOR);
}
