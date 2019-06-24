#pragma once

#include "Core.h"

#include "..\Pipelines.h"
#include "VulkanBase.h"

MAKE_VK_HANDLE(VkPipeline)

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

GS_CLASS VulkanGraphicsPipeline final : public GraphicsPipeline, public VulkanObject
{
	VkPipeline GraphicsPipeline = nullptr;

	static void CreateVertexInputState(VkPipelineVertexInputStateCreateInfo & _PVISCI);
	static void CreateInputAssemblyState(VkPipelineInputAssemblyStateCreateInfo& _PIASCI);
	static void CreateTessellationState(VkPipelineTessellationStateCreateInfo& _PTSCI);
	static void CreateViewportState(VkPipelineViewportStateCreateInfo& _PVSCI);
	static void CreateRasterizationState(VkPipelineRasterizationStateCreateInfo& _PRSCI);
	static void CreateMultisampleState(VkPipelineMultisampleStateCreateInfo& _PMSCI);
	static void CreateDepthStencilState(VkPipelineDepthStencilStateCreateInfo& _PDSSCI);
	static void CreateColorBlendState(VkPipelineColorBlendStateCreateInfo& _PCBSCI);
	static void CreateDynamicState(VkPipelineDynamicStateCreateInfo& _PDSCI);
public:
	VulkanGraphicsPipeline(VkDevice _Device, RenderPass * _RP);
	~VulkanGraphicsPipeline();

	INLINE VkPipeline GetVkGraphicsPipeline() const { return GraphicsPipeline; }
};

GS_CLASS VulkanComputePipeline final : public ComputePipeline, public VulkanObject
{
	VkPipeline ComputePipeline = nullptr;
public:
	VulkanComputePipeline(VkDevice _Device);
	~VulkanComputePipeline();

	INLINE VkPipeline GetVkComputePipeline() const { return ComputePipeline; }

};