#pragma once

#include "Core.h"

#include "RAPI/Vulkan/VulkanBase.h"

MAKE_VK_HANDLE(VkPipeline)

struct VkComputePipelineCreateInfo;

GS_STRUCT VKComputePipelineCreator final : VKObjectCreator<VkPipeline>
{
	VKComputePipelineCreator(const VKDevice& _Device, const VkComputePipelineCreateInfo* _VkCPCI);
};

GS_CLASS VKComputePipeline final : public VKObject
{
	VkPipeline ComputePipeline = nullptr;

public:
	explicit VKComputePipeline(const VKComputePipelineCreator& _VKCPC) : VKObject(_VKCPC.m_Device), ComputePipeline(_VKCPC.Handle)
	{
	}

	~VKComputePipeline();

	INLINE VkPipeline GetVkPipeline() const { return ComputePipeline; }

	INLINE operator VkPipeline() const { return ComputePipeline; }
};