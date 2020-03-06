#pragma once

#include "Core.h"

#include "RAPI/GraphicsPipeline.h"
#include "RAPI/ComputePipeline.h"

class VKRenderPass;
class RenderPass;

class VulkanGraphicsPipeline final : public GraphicsPipeline
{
	VkPipelineLayout vkPipelineLayout = nullptr;
	VkPipeline vkPipeline = nullptr;

public:
	VulkanGraphicsPipeline(const GraphicsPipelineCreateInfo& _GPCI);
	~VulkanGraphicsPipeline() = default;

	INLINE VkPipeline GetVkGraphicsPipeline() const { return vkPipeline; }
	INLINE VkPipelineLayout GetVkPipelineLayout() const { return vkPipelineLayout; }
};

class VulkanComputePipeline final : public ComputePipeline
{
	VkPipeline vkPipeline = nullptr;

public:
	explicit VulkanComputePipeline(VKDevice* _Device);
	~VulkanComputePipeline() = default;

	[[nodiscard]] VkPipeline GetVkPipeline() const { return vkPipeline; }
};
