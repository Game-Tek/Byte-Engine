#include "Vk_PipelineLayout.h"

#include "RAPI/Vulkan/Vulkan.h"

#include "Vk_Device.h"

Vk_PipelineLayout::Vk_PipelineLayout(const Vk_Device& _Device) : VulkanObject(_Device)
{
	VkPipelineLayoutCreateInfo PipelineLayoutCreateInfo = { VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO };
	PipelineLayoutCreateInfo.setLayoutCount = 0; // Optional
	PipelineLayoutCreateInfo.pSetLayouts = nullptr; // Optional
	PipelineLayoutCreateInfo.pushConstantRangeCount = 0; // Optional
	PipelineLayoutCreateInfo.pPushConstantRanges = nullptr; // Optional

	GS_VK_CHECK(vkCreatePipelineLayout(m_Device, &PipelineLayoutCreateInfo, ALLOCATOR, &Layout), "Failed to create Pieline Layout!")
}

Vk_PipelineLayout::~Vk_PipelineLayout()
{
	vkDestroyPipelineLayout(m_Device, Layout, ALLOCATOR);
}
