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

MAKE_VK_HANDLE(VkPipelineLayout)

class VulkanShader;
struct VulkanStageInfo;

GS_CLASS VulkanGraphicsPipeline final : public GraphicsPipeline
{
	Vk_GraphicsPipeline Pipeline;
	Vk_PipelineLayout Layout;
public:
	VulkanGraphicsPipeline(VkDevice _Device, RenderPass * _RP, Extent2D _SwapchainSize, const StageInfo& Stages);
	~VulkanGraphicsPipeline();

	INLINE const Vk_GraphicsPipeline& GetVk_GraphicsPipeline() const { return Pipeline; }
};

GS_CLASS VulkanComputePipeline final : public ComputePipeline
{
	Vk_ComputePipeline ComputePipeline;
public:
	VulkanComputePipeline(VkDevice _Device);
	~VulkanComputePipeline();

	INLINE const Vk_ComputePipeline& GetVk_ComputePipeline() const { return ComputePipeline; }

};


GS_CLASS Vk_GraphicsPipeline : public VulkanObject
{
	VkPipeline GraphicsPipeline = nullptr;

	static void CreateVertexInputState(VkPipelineVertexInputStateCreateInfo& _PVISCI);
	static void CreateInputAssemblyState(VkPipelineInputAssemblyStateCreateInfo& _PIASCI);
	static void CreateTessellationState(VkPipelineTessellationStateCreateInfo& _PTSCI);
	static void CreateViewportState(VkPipelineViewportStateCreateInfo& _PVSCI, VkExtent2D _SwapchainSize);
	static void CreateRasterizationState(VkPipelineRasterizationStateCreateInfo& _PRSCI);
	static void CreateMultisampleState(VkPipelineMultisampleStateCreateInfo& _PMSCI);
	static void CreateDepthStencilState(VkPipelineDepthStencilStateCreateInfo& _PDSSCI);
	static void CreateColorBlendState(VkPipelineColorBlendStateCreateInfo& _PCBSCI);
	static void CreateDynamicState(VkPipelineDynamicStateCreateInfo& _PDSCI);
public:
	Vk_GraphicsPipeline(VkDevice _Device, VkRenderPass _RP, VkExtent2D _SwapchainSize, VkPipelineLayout _PL, const VulkanStageInfo& _SI);
	~Vk_GraphicsPipeline();

	INLINE VkPipeline GetVkGraphicsPipeline() const { return GraphicsPipeline; }
};

GS_CLASS Vk_ComputePipeline : public VulkanObject
{
	VkPipeline ComputePipeline = nullptr;
public:
	Vk_ComputePipeline();
	~Vk_ComputePipeline();

	INLINE VkPipeline GetVkPipeline() const { return ComputePipeline; }
};

GS_CLASS Vk_PipelineLayout : public VulkanObject
{
	VkPipelineLayout Layout = nullptr;
public:
	Vk_PipelineLayout(VkDevice _Device);
	~Vk_PipelineLayout();

	INLINE VkPipelineLayout GetVkPipelineLayout() { return Layout; }
};