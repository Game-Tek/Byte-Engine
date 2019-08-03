#pragma once

#include "Core.h"

#include "RAPI/Pipelines.h"
#include "Extent.h"
#include "Native/Vk_PipelineLayout.h"
#include "Native/Vk_GraphicsPipeline.h"
#include "Native/Vk_ComputePipeline.h"
#include "RAPI/Mesh.h"

class RenderPass;

MAKE_VK_HANDLE(VkPipelineLayout)

GS_CLASS VulkanGraphicsPipeline final : public GraphicsPipeline
{
	Vk_PipelineLayout Layout;
	Vk_GraphicsPipeline Pipeline;

	static FVector<VkPipelineShaderStageCreateInfo> StageInfoToVulkanStageInfo(const ShaderStages& _SI, const Vk_Device& _Device);
	static PipelineState CreatePipelineState(const Extent2D& _Extent, const ShaderStages& _SI, const VertexDescriptor& _VD);
	static VkPipelineVertexInputStateCreateInfo CreateVertexInputState(const VertexDescriptor& _VD);
	static VkPipelineInputAssemblyStateCreateInfo CreateInputAssemblyState();
	static VkPipelineTessellationStateCreateInfo CreateTessellationState();
	static VkPipelineViewportStateCreateInfo CreateViewportState(VkExtent2D _SwapchainSize);
	static VkPipelineRasterizationStateCreateInfo CreateRasterizationState();
	static VkPipelineMultisampleStateCreateInfo CreateMultisampleState();
	static VkPipelineDepthStencilStateCreateInfo CreateDepthStencilState();
	static VkPipelineColorBlendStateCreateInfo CreateColorBlendState();
	static VkPipelineDynamicStateCreateInfo CreateDynamicState();
public:
	VulkanGraphicsPipeline(const Vk_Device& _Device, RenderPass* _RP, Extent2D _SwapchainSize, const ShaderStages& _SI, const VertexDescriptor& _VD);
	~VulkanGraphicsPipeline() = default;

	INLINE const Vk_GraphicsPipeline& GetVk_GraphicsPipeline() const { return Pipeline; }
};

GS_CLASS VulkanComputePipeline final : public ComputePipeline
{
	Vk_ComputePipeline ComputePipeline;

public:
	VulkanComputePipeline(const Vk_Device& _Device);
	~VulkanComputePipeline() = default;

	INLINE const Vk_ComputePipeline& GetVk_ComputePipeline() const { return ComputePipeline; }
};