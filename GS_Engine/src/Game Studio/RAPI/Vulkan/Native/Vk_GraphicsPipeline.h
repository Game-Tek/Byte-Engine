#pragma once

#include "RAPI/Vulkan/VulkanBase.h"
#include "Containers/FVector.hpp"

class Vk_PipelineLayout;
class Vk_RenderPass;
MAKE_VK_HANDLE(VkPipeline)

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

struct VkPipelineShaderStageCreateInfo;

MAKE_VK_HANDLE(VkShaderModule)

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
	Vk_GraphicsPipeline(const Vk_Device& _Device, const Vk_RenderPass& _RP, VkExtent2D _SwapchainSize, const Vk_PipelineLayout& _PL, const FVector<VkPipelineShaderStageCreateInfo>& _SI);
	~Vk_GraphicsPipeline();

	INLINE VkPipeline GetVkGraphicsPipeline() const { return GraphicsPipeline; }

	INLINE operator VkPipeline() const { return GraphicsPipeline; }
};