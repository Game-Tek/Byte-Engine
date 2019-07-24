#pragma once

#include "Core.h"

#include "..\Pipelines.h"
#include "VulkanBase.h"

#include "Extent.h"

MAKE_VK_HANDLE(VkPipeline)
MAKE_VK_HANDLE(VkRenderPass)
MAKE_VK_HANDLE(VkShaderModule)

class RenderPass;

struct VkPipelineVertexInputStateCreateInfo;
struct VkPipelineInputAssemblyStateCreateInfo;
struct VkPipelineTessellationStateCreateInfo;
struct VkPipelineViewportStateCreateInfo;
struct VkPipelineRasterizationStateCreateInfo;
struct VkPipelineMultisampleStateCreateInfo;
struct VkPipelineDepthStencilStateCreateInfo;
struct VkPipelineColorBlendStateCreateInfo;
struct VkPipelineDynamicStateCreateInfo;
struct VkExtent2D;

enum VkShaderStageFlagBits;

MAKE_VK_HANDLE(VkPipelineLayout)

GS_STRUCT VulkanStageInfo
{
	VkShaderModule Shaders[MAX_SHADER_STAGES];
	VkShaderStageFlagBits ShaderTypes[MAX_SHADER_STAGES];
	uint8 ShaderCount = 2;
};

GS_CLASS Vk_PipelineLayout final : public VulkanObject
{
	VkPipelineLayout Layout = nullptr;
public:
	Vk_PipelineLayout(VkDevice _Device);
	~Vk_PipelineLayout();

	INLINE VkPipelineLayout GetVkPipelineLayout() { return Layout; }

	INLINE operator VkPipelineLayout() const { return Layout; }
};

GS_CLASS Vk_GraphicsPipeline final : public VulkanObject
{
	VkPipeline GraphicsPipeline = nullptr;

	static VkPipelineVertexInputStateCreateInfo CreateVertexInputState();
	static VkPipelineInputAssemblyStateCreateInfo CreateInputAssemblyState();
	static VkPipelineTessellationStateCreateInfo CreateTessellationState();
	static VkPipelineViewportStateCreateInfo CreateViewportState(VkExtent2D _SwapchainSize);
	static VkPipelineRasterizationStateCreateInfo CreateRasterizationState();
	static VkPipelineMultisampleStateCreateInfo CreateMultisampleState();
	static VkPipelineDepthStencilStateCreateInfo CreateDepthStencilState();
	static VkPipelineColorBlendStateCreateInfo CreateColorBlendState();
	static VkPipelineDynamicStateCreateInfo CreateDynamicState();
public:
	Vk_GraphicsPipeline(VkDevice _Device, VkRenderPass _RP, VkExtent2D _SwapchainSize, VkPipelineLayout _PL, const VulkanStageInfo& _SI);
	~Vk_GraphicsPipeline();

	INLINE VkPipeline GetVkGraphicsPipeline() const { return GraphicsPipeline; }

	INLINE operator VkPipeline() const { return GraphicsPipeline; }
};

GS_CLASS Vk_ComputePipeline final : public VulkanObject
{
	VkPipeline ComputePipeline = nullptr;
public:
	Vk_ComputePipeline(VkDevice _Device);
	~Vk_ComputePipeline();

	INLINE VkPipeline GetVkPipeline() const { return ComputePipeline; }
};

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