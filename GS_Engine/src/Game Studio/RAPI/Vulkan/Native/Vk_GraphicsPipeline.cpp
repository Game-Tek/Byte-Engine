#include "Vk_GraphicsPipeline.h"

#include "RAPI/Vulkan/Vulkan.h"

#include "Vk_Device.h"

Vk_GraphicsPipelineCreator::Vk_GraphicsPipelineCreator(const Vk_Device& _Device, const VkGraphicsPipelineCreateInfo* _VGPCI) : VulkanObjectCreateInfo(_Device)
{
	GS_VK_CHECK(vkCreateGraphicsPipelines(m_Device, VK_NULL_HANDLE, 1, _VGPCI, ALLOCATOR, &GraphicsPipeline), "Failed to create Graphics Pipeline!")
}

Vk_GraphicsPipeline::Vk_GraphicsPipeline(const Vk_GraphicsPipelineCreator& _Vk_GPC) : VulkanObject(_Vk_GPC.m_Device), GraphicsPipeline(_Vk_GPC.GraphicsPipeline)
{
}

Vk_GraphicsPipeline::~Vk_GraphicsPipeline()
{
	vkDestroyPipeline(m_Device, GraphicsPipeline, ALLOCATOR);
}
