#pragma once

#include "Core.h"

#include "Native/VKPipelineLayout.h"
#include "Native/VKGraphicsPipeline.h"
#include "Native/VKComputePipeline.h"
#include "RAPI/GraphicsPipeline.h"
#include "RAPI/ComputePipeline.h"

class VKRenderPass;
class RenderPass;

MAKE_VK_HANDLE(VkPipelineLayout)
MAKE_VK_HANDLE(VkPipeline)

class VulkanGraphicsPipeline final : public GraphicsPipeline
{
	VkPipelineLayout vkPipelineLayout = nullptr;
	VkPipeline vkPipeline = nullptr;

	static VKGraphicsPipelineCreator CreateVk_GraphicsPipelineCreator(VKDevice* _Device,
	                                                                  const GraphicsPipelineCreateInfo& _GPCI,
	                                                                  VkPipeline _OldPipeline = VK_NULL_HANDLE);
public:
	VulkanGraphicsPipeline(const GraphicsPipelineCreateInfo& _GPCI);
	~VulkanGraphicsPipeline() = default;

	INLINE VkPipeline GetVkGraphicsPipeline() const { return vkPipeline; }
	INLINE VkPipelineLayout GetVkPipelineLayout() const { return vkPipelineLayout; }
};

class VulkanComputePipeline final : public ComputePipeline
{
	VKComputePipeline ComputePipeline;

public:
	VulkanComputePipeline(VKDevice* _Device);
	~VulkanComputePipeline() = default;

	INLINE const VKComputePipeline& GetVk_ComputePipeline() const { return ComputePipeline; }
};
