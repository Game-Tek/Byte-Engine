#pragma once

#include "Core.h"

#include "RAPI/Vulkan/VulkanBase.h"

MAKE_VK_HANDLE(VkPipeline)

struct VkComputePipelineCreateInfo;

struct GS_API VKComputePipelineCreator final : VKObjectCreator<VkPipeline>
{
	VKComputePipelineCreator(VKDevice* _Device, const VkComputePipelineCreateInfo* _VkCPCI);
};

class GS_API VKComputePipeline final : public VKObject<VkPipeline>
{
public:
	explicit VKComputePipeline(const VKComputePipelineCreator& _VKCPC) : VKObject(_VKCPC)
	{
	}

	~VKComputePipeline();
};
