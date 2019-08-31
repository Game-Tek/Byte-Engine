#pragma once

#include "Core.h"

#include "RAPI/Pipelines.h"
#include "Extent.h"
#include "Native/VKPipelineLayout.h"
#include "Native/VKGraphicsPipeline.h"
#include "Native/VKComputePipeline.h"
#include "RAPI/Mesh.h"

class VKRenderPass;
class RenderPass;

MAKE_VK_HANDLE(VkPipelineLayout)

GS_CLASS VulkanGraphicsPipeline final : public GraphicsPipeline
{
	VKPipelineLayout Layout;
	VKGraphicsPipeline Pipeline;

	static VKGraphicsPipelineCreator CreateVk_GraphicsPipelineCreator(VKDevice* _Device, const VKPipelineLayout& _PL, const VKRenderPass& _RP, const Extent2D& _Extent, const VertexDescriptor& _VD, const PipelineDescriptor& _Stages, VkPipeline _OldPipeline = VK_NULL_HANDLE);
	static VKPipelineLayoutCreator CreatePipelineLayout(VKDevice* _Device);
public:
	VulkanGraphicsPipeline(VKDevice* _Device, RenderPass* _RP, Extent2D _SwapchainSize, const PipelineDescriptor& _PD, const VertexDescriptor& _VD);
	~VulkanGraphicsPipeline() = default;

	INLINE const VKGraphicsPipeline& GetVk_GraphicsPipeline() const { return Pipeline; }
};

GS_CLASS VulkanComputePipeline final : public ComputePipeline
{
	VKComputePipeline ComputePipeline;

public:
	VulkanComputePipeline(VKDevice* _Device);
	~VulkanComputePipeline() = default;

	INLINE const VKComputePipeline& GetVk_ComputePipeline() const { return ComputePipeline; }
};