#include "VKComputePipeline.h"

#include "RAPI/Vulkan/Vulkan.h"

#include "VKDevice.h"

VKComputePipelineCreator::VKComputePipelineCreator(VKDevice* _Device, const VkComputePipelineCreateInfo* _VkCPCI) : VKObjectCreator(_Device)
{
	GS_VK_CHECK(vkCreateComputePipelines(m_Device->GetVkDevice(), VK_NULL_HANDLE, 1, _VkCPCI, ALLOCATOR, &Handle), "Failed to create Compute Pipeline!")
}

VKComputePipeline::~VKComputePipeline()
{
	vkDestroyPipeline(m_Device->GetVkDevice(), Handle, ALLOCATOR);
}
