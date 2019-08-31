#pragma once

#include "Core.h"

#include "RAPI/Vulkan/VulkanBase.h"

MAKE_VK_HANDLE(VkPipeline)

struct VkComputePipelineCreateInfo;

GS_STRUCT VKComputePipelineCreator final : VKObjectCreator<VkPipeline>
{
	VKComputePipelineCreator(VKDevice* _Device, const VkComputePipelineCreateInfo* _VkCPCI);
};

GS_CLASS VKComputePipeline final : public VKObject<VkPipeline>
{
public:
	explicit VKComputePipeline(const VKComputePipelineCreator& _VKCPC) : VKObject(_VKCPC)
	{
	}

	~VKComputePipeline();
};