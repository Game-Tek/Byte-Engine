#include "VKDescriptorSetLayout.h"

#include "RAPI/Vulkan/Vulkan.h"

#include "VKDevice.h"

VKDescriptorSetLayoutCreator::VKDescriptorSetLayoutCreator(VKDevice* _Device,
                                                           const VkDescriptorSetLayoutCreateInfo* _VkDSLCI) :
	VKObjectCreator<VkDescriptorSetLayout>(_Device)
{
	vkCreateDescriptorSetLayout(_Device->GetVkDevice(), _VkDSLCI, ALLOCATOR, &Handle);
}

VKDescriptorSetLayout::~VKDescriptorSetLayout()
{
	vkDestroyDescriptorSetLayout(m_Device->GetVkDevice(), Handle, ALLOCATOR);
}
