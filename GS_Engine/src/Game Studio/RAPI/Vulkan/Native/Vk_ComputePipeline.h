#pragma once

#include "Core.h"

#include "RAPI/Vulkan/VulkanBase.h"

MAKE_VK_HANDLE(VkPipeline)

GS_CLASS Vk_ComputePipeline final : public VulkanObject
{
	VkPipeline ComputePipeline = nullptr;
public:
	Vk_ComputePipeline(const Vk_Device& _Device);
	~Vk_ComputePipeline();

	INLINE VkPipeline GetVkPipeline() const { return ComputePipeline; }

	INLINE operator VkPipeline() const { return ComputePipeline; }
};