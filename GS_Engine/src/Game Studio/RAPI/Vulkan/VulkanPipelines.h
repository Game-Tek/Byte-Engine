#pragma once

#include "Core.h"

#include "RAPI/Pipelines.h"
#include "VulkanBase.h"

#include "Extent.h"
#include "Native/Vk_GraphicsPipeline.h"
#include "Native/Vk_ComputePipeline.h"
#include "Native/Vk_PipelineLayout.h"

MAKE_VK_HANDLE(VkPipeline)
MAKE_VK_HANDLE(VkRenderPass)
MAKE_VK_HANDLE(VkShaderModule)

class RenderPass;

MAKE_VK_HANDLE(VkPipelineLayout)

GS_CLASS VulkanGraphicsPipeline final : public GraphicsPipeline
{
	Vk_PipelineLayout Layout;
	Vk_GraphicsPipeline Pipeline;

public:
	VulkanGraphicsPipeline(VkDevice _Device, RenderPass * _RP, Extent2D _SwapchainSize, const StageInfo& Stages);
	~VulkanGraphicsPipeline() = default;

	INLINE const Vk_GraphicsPipeline& GetVk_GraphicsPipeline() const { return Pipeline; }
};

GS_CLASS VulkanComputePipeline final : public ComputePipeline
{
	Vk_ComputePipeline ComputePipeline;

public:
	VulkanComputePipeline(VkDevice _Device);
	~VulkanComputePipeline() = default;

	INLINE const Vk_ComputePipeline& GetVk_ComputePipeline() const { return ComputePipeline; }
};