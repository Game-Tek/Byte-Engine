#pragma once

#include "Core.h"

#include "RAPI/Vulkan/VulkanBase.h"

MAKE_VK_HANDLE(VkPipelineLayout)

struct VkPipelineLayoutCreateInfo;

GS_STRUCT VKPipelineLayoutCreator final : VKObjectCreator<VkPipelineLayout>
{
	VKPipelineLayoutCreator(const VKDevice & _Device, const VkPipelineLayoutCreateInfo * _VkPLCI);
};


GS_CLASS VKPipelineLayout final : public VKObject<VkPipelineLayout>
{
public:
	VKPipelineLayout(const VKPipelineLayoutCreator& _VKPLC) : VKObject<VkPipelineLayout>(_VKPLC)
	{
	}

	~VKPipelineLayout();
};
