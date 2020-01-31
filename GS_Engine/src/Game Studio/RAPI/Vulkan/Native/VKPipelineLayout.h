#pragma once

#include "Core.h"

#include "RAPI/Vulkan/VulkanBase.h"

MAKE_VK_HANDLE(VkPipelineLayout)

struct VkPipelineLayoutCreateInfo;

struct VKPipelineLayoutCreator final : VKObjectCreator<VkPipelineLayout>
{
	VKPipelineLayoutCreator(VKDevice* _Device, const VkPipelineLayoutCreateInfo* _VkPLCI);
};


class VKPipelineLayout final : public VKObject<VkPipelineLayout>
{
public:
	VKPipelineLayout(const VKPipelineLayoutCreator& _VKPLC) : VKObject<VkPipelineLayout>(_VKPLC)
	{
	}

	~VKPipelineLayout();
};
