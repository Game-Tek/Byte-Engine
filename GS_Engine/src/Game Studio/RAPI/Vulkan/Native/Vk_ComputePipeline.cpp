#include "Vk_ComputePipeline.h"

#include "RAPI/Vulkan/Vulkan.h"

#include "Vk_Device.h"

Vk_ComputePipeline::Vk_ComputePipeline(const Vk_Device& _Device) : VulkanObject(_Device)
{
	VkComputePipelineCreateInfo CreateInfo = { VK_STRUCTURE_TYPE_COMPUTE_PIPELINE_CREATE_INFO };
	CreateInfo.stage;
	CreateInfo.basePipelineHandle = VK_NULL_HANDLE;
	CreateInfo.basePipelineIndex = -1;

	GS_VK_CHECK(vkCreateComputePipelines(m_Device, VK_NULL_HANDLE, 1, &CreateInfo, ALLOCATOR, &ComputePipeline), "Failed to create Compute Pipeline!")
}

Vk_ComputePipeline::~Vk_ComputePipeline()
{
	vkDestroyPipeline(m_Device, ComputePipeline, ALLOCATOR);
}