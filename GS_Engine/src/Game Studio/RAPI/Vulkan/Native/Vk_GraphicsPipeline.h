#pragma once

#include "RAPI/Vulkan/VulkanBase.h"
#include "Containers/FVector.hpp"

struct ShaderStages;
class VertexDescriptor;
struct PipelineState;
class Vk_PipelineLayout;
class Vk_RenderPass;

MAKE_VK_HANDLE(VkPipeline)

struct VkExtent2D;

MAKE_VK_HANDLE(VkShaderModule)

GS_CLASS Vk_GraphicsPipeline final : public VulkanObject
{
	VkPipeline GraphicsPipeline = nullptr;

public:
	Vk_GraphicsPipeline(const Vk_Device& _Device, const Vk_RenderPass& _RP, VkExtent2D _SwapchainSize, const Vk_PipelineLayout& _PL, const ShaderStages& _SI, const VertexDescriptor& _VD);
	~Vk_GraphicsPipeline();

	INLINE VkPipeline GetVkGraphicsPipeline() const { return GraphicsPipeline; }

	INLINE operator VkPipeline() const { return GraphicsPipeline; }
};