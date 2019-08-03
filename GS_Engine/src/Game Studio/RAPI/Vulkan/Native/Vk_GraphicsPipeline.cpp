#include "Vk_GraphicsPipeline.h"

#include "RAPI/Vulkan/Vulkan.h"

#include "Vk_Device.h"

#include "Vk_PipelineLayout.h"
#include "Vk_RenderPass.h"


Vk_GraphicsPipeline::Vk_GraphicsPipeline(const Vk_Device& _Device, const Vk_RenderPass& _RP, VkExtent2D _SwapchainSize, const Vk_PipelineLayout& _PL, const FVector<VkPipelineShaderStageCreateInfo>& _SI, const PipelineState& _PS) : VulkanObject(_Device)
{
	VkGraphicsPipelineCreateInfo CreateInfo = { VK_STRUCTURE_TYPE_GRAPHICS_PIPELINE_CREATE_INFO };
	CreateInfo.stageCount = _SI.length();
	//pStages is an array of size stageCount structures of type VkPipelineShaderStageCreateInfo
	//describing the set of the shader stages to be included in the graphics pipeline.
	CreateInfo.pStages = _SI.data();
	//pVertexInputState is a pointer to an instance of the VkPipelineVertexInputStateCreateInfo structure.
	CreateInfo.pVertexInputState = _PS.PipelineVertexInputState;
	//pInputAssemblyState is a pointer to an instance of the VkPipelineInputAssemblyStateCreateInfo structure which determines input assembly behavior, as described in Drawing Commands.
	CreateInfo.pInputAssemblyState = _PS.PipelineInputAssemblyState;
	//pTessellationState is a pointer to an instance of the VkPipelineTessellationStateCreateInfo structure, and is ignored if the pipeline does not include a tessellation control shader stage and tessellation evaluation shader stage.
	CreateInfo.pTessellationState = _PS.PipelineTessellationState;
	//pViewportState is a pointer to an instance of the VkPipelineViewportStateCreateInfo structure, and is ignored if the pipeline has rasterization disabled.
	CreateInfo.pViewportState = _PS.PipelineViewportState;
	//pRasterizationState is a pointer to an instance of the VkPipelineRasterizationStateCreateInfo structure.
	CreateInfo.pRasterizationState = _PS.PipelineRasterizationState;
	//pMultisampleState is a pointer to an instance of the VkPipelineMultisampleStateCreateInfo, and is ignored if the pipeline has rasterization disabled.
	CreateInfo.pMultisampleState = _PS.PipelineMultisampleState;
	//pDepthStencilState is a pointer to an instance of the VkPipelineDepthStencilStateCreateInfo structure, and is ignored if the pipeline has rasterization disabled or if the subpass of the render pass the pipeline is created against does not use a depth / stencil attachment.
	CreateInfo.pDepthStencilState = _PS.PipelineDepthStencilState; // Optional
	//pColorBlendState is a pointer to an instance of the VkPipelineColorBlendStateCreateInfo structure, and is ignored if the pipeline has rasterization disabled or if the subpass of the render pass the pipeline is created against does not use any color attachments.
	CreateInfo.pColorBlendState = _PS.PipelineColorBlendState;
	//pDynamicState is a pointer to VkPipelineDynamicStateCreateInfo and is used to indicate which properties of the pipeline state object are dynamic and can be changed independently of the pipeline state.This can be NULL, which means no state in the pipeline is considered dynamic.
	CreateInfo.pDynamicState = _PS.PipelineDynamicState; // Optional
	//layout is the description of binding locations used by both the pipeline and descriptor sets used with the pipeline.
	CreateInfo.layout = _PL;
	CreateInfo.renderPass = _RP;
	//subpass is the index of the subpass in the render pass where this pipeline will be used.
	CreateInfo.subpass = 0;
	//basePipelineHandle is a pipeline to derive from.
	CreateInfo.basePipelineHandle = VK_NULL_HANDLE; // Optional
	//basePipelineIndex is an index into the pCreateInfos parameter to use as a pipeline to derive from.
	CreateInfo.basePipelineIndex = -1; // Optional

	GS_VK_CHECK(vkCreateGraphicsPipelines(m_Device, VK_NULL_HANDLE, 1, &CreateInfo, ALLOCATOR, &GraphicsPipeline), "Failed to create Graphics Pipeline!")
}

Vk_GraphicsPipeline::~Vk_GraphicsPipeline()
{
	vkDestroyPipeline(m_Device, GraphicsPipeline, ALLOCATOR);
}