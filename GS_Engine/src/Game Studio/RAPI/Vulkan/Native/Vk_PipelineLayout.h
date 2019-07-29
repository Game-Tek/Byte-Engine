#pragma once

#include "Core.h"

#include "RAPI/Vulkan/VulkanBase.h"

MAKE_VK_HANDLE(VkPipelineLayout)

GS_CLASS Vk_PipelineLayout final : public VulkanObject
{
	VkPipelineLayout Layout = nullptr;
public:
	Vk_PipelineLayout(const Vk_Device& _Device);
	~Vk_PipelineLayout();

	INLINE VkPipelineLayout GetVkPipelineLayout() { return Layout; }

	INLINE operator VkPipelineLayout() const { return Layout; }
};
