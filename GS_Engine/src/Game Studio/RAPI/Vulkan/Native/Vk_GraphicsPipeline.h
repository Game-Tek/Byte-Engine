#pragma once

#include "RAPI/Vulkan/VulkanBase.h"
#include "Containers/FVector.hpp"

MAKE_VK_HANDLE(VkPipeline)

MAKE_VK_HANDLE(VkShaderModule)

struct VkGraphicsPipelineCreateInfo;

GS_STRUCT Vk_GraphicsPipelineCreator : VulkanObjectCreateInfo
{
	Vk_GraphicsPipelineCreator(const Vk_Device & _Device, const VkGraphicsPipelineCreateInfo * _VGPCI);

	VkPipeline GraphicsPipeline = VK_NULL_HANDLE;
};

GS_CLASS Vk_GraphicsPipeline final : public VulkanObject
{
	VkPipeline GraphicsPipeline = nullptr;

public:
	explicit Vk_GraphicsPipeline(const Vk_GraphicsPipelineCreator& _Vk_GPC);
	~Vk_GraphicsPipeline();

	INLINE VkPipeline GetVkGraphicsPipeline() const { return GraphicsPipeline; }

	INLINE operator VkPipeline() const { return GraphicsPipeline; }
};