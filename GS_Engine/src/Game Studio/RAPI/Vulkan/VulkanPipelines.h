#pragma once

#include "Core.h"

#include "RAPI/Pipelines.h"
#include "Extent.h"
#include "Native/Vk_PipelineLayout.h"
#include "Native/Vk_GraphicsPipeline.h"
#include "Native/Vk_ComputePipeline.h"
#include "RAPI/Mesh.h"

class Vk_RenderPass;
class RenderPass;

MAKE_VK_HANDLE(VkPipelineLayout)

GS_CLASS VulkanGraphicsPipeline final : public GraphicsPipeline
{
	Vk_PipelineLayout Layout;
	Vk_GraphicsPipeline Pipeline;

	static Vk_GraphicsPipelineCreator CreateVk_GraphicsPipelineCreator(const Vk_Device& _Device, const Vk_PipelineLayout& _PL, const Vk_RenderPass& _RP, const Extent2D& _Extent, const VertexDescriptor& _VD, const PipelineDescriptor& _Stages, VkPipeline _OldPipeline = VK_NULL_HANDLE);

public:
	VulkanGraphicsPipeline(const Vk_Device& _Device, RenderPass* _RP, Extent2D _SwapchainSize, const PipelineDescriptor& _PD, const VertexDescriptor& _VD);
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