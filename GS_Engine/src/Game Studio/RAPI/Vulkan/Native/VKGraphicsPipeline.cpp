#include "VKGraphicsPipeline.h"

#include "RAPI/Vulkan/Vulkan.h"

#include "VKDevice.h"

VKGraphicsPipelineCreator::
VKGraphicsPipelineCreator(VKDevice* _Device, const VkGraphicsPipelineCreateInfo* _VGPCI) : VKObjectCreator(_Device)
{
	GS_VK_CHECK(vkCreateGraphicsPipelines(m_Device->GetVkDevice(), VK_NULL_HANDLE, 1, _VGPCI, ALLOCATOR, &Handle),
	            "Failed to create Graphics Pipeline!")
}

VKGraphicsPipeline::~VKGraphicsPipeline()
{
	vkDestroyPipeline(m_Device->GetVkDevice(), Handle, ALLOCATOR);
}
