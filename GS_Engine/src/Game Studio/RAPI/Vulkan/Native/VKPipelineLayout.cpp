#include "VKPipelineLayout.h"

#include "RAPI/Vulkan/Vulkan.h"

#include "VKDevice.h"

VKPipelineLayoutCreator::VKPipelineLayoutCreator(VKDevice* _Device, const VkPipelineLayoutCreateInfo* _VkPLCI) : VKObjectCreator<VkPipelineLayout>(_Device)
{
	GS_VK_CHECK(vkCreatePipelineLayout(m_Device->GetVkDevice(), _VkPLCI, ALLOCATOR, &Handle), "Failed to create Pieline Layout!")
}

VKPipelineLayout::~VKPipelineLayout()
{
	vkDestroyPipelineLayout(m_Device->GetVkDevice(), Handle, ALLOCATOR);
}
