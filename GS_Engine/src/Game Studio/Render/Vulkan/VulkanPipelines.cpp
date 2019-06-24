#include "VulkanPipelines.h"

#include "Vulkan.h"

#include "..\RenderPass.h"

#include "VulkanRenderPass.h"

VulkanGraphicsPipeline::VulkanGraphicsPipeline(VkDevice _Device, RenderPass * _RP) : VulkanObject(_Device)
{
	VkPipelineVertexInputStateCreateInfo PipelineVertexInputStateCreateInfo;
	VkPipelineInputAssemblyStateCreateInfo PipelineInputAssemblyStateCreateInfo;
	VkPipelineTessellationStateCreateInfo PipelineTessellationStateCreateInfo;
	VkPipelineViewportStateCreateInfo PipelineViewportStateCreateInfo;
	VkPipelineRasterizationStateCreateInfo PipelineRasterizationStateCreateInfo;
	VkPipelineMultisampleStateCreateInfo PipelineMultisampleStateCreateInfo;
	VkPipelineDepthStencilStateCreateInfo PipelineDepthStencilStateCreateInfo;
	VkPipelineColorBlendStateCreateInfo PipelineColorBlendStateCreateInfo;
	VkPipelineDynamicStateCreateInfo PipelineDynamicStateCreateInfo;

	CreateVertexInputState(PipelineVertexInputStateCreateInfo);
	CreateInputAssemblyState(PipelineInputAssemblyStateCreateInfo);
	CreateTessellationState(PipelineTessellationStateCreateInfo);
	CreateViewportState(PipelineViewportStateCreateInfo);
	CreateRasterizationState(PipelineRasterizationStateCreateInfo);
	CreateMultisampleState(PipelineMultisampleStateCreateInfo);
	CreateDepthStencilState(PipelineDepthStencilStateCreateInfo);
	CreateColorBlendState(PipelineColorBlendStateCreateInfo);
	CreateDynamicState(PipelineDynamicStateCreateInfo);

	VkGraphicsPipelineCreateInfo CreateInfo = { VK_STRUCTURE_TYPE_GRAPHICS_PIPELINE_CREATE_INFO };
	CreateInfo.stageCount = 2;
	//pStages is an array of size stageCount structures of type VkPipelineShaderStageCreateInfo
	//describing the set of the shader stages to be included in the graphics pipeline.
	CreateInfo.pStages = shaderStages;
	//pVertexInputState is a pointer to an instance of the VkPipelineVertexInputStateCreateInfo structure.
	CreateInfo.pVertexInputState = &PipelineVertexInputStateCreateInfo;
	//pInputAssemblyState is a pointer to an instance of the VkPipelineInputAssemblyStateCreateInfo structure which determines input assembly behavior, as described in Drawing Commands.
	CreateInfo.pInputAssemblyState = &PipelineInputAssemblyStateCreateInfo;
	//pTessellationState is a pointer to an instance of the VkPipelineTessellationStateCreateInfo structure, and is ignored if the pipeline does not include a tessellation control shader stage and tessellation evaluation shader stage.
	CreateInfo.pTessellationState = nullptr;
	//pViewportState is a pointer to an instance of the VkPipelineViewportStateCreateInfo structure, and is ignored if the pipeline has rasterization disabled.
	CreateInfo.pViewportState = &PipelineViewportStateCreateInfo;
	//pRasterizationState is a pointer to an instance of the VkPipelineRasterizationStateCreateInfo structure.
	CreateInfo.pRasterizationState = &PipelineRasterizationStateCreateInfo;
	//pMultisampleState is a pointer to an instance of the VkPipelineMultisampleStateCreateInfo, and is ignored if the pipeline has rasterization disabled.
	CreateInfo.pMultisampleState = &PipelineMultisampleStateCreateInfo;
	//pDepthStencilState is a pointer to an instance of the VkPipelineDepthStencilStateCreateInfo structure, and is ignored if the pipeline has rasterization disabled or if the subpass of the render pass the pipeline is created against does not use a depth / stencil attachment.
	CreateInfo.pDepthStencilState = &PipelineDepthStencilStateCreateInfo; // Optional
	//pColorBlendState is a pointer to an instance of the VkPipelineColorBlendStateCreateInfo structure, and is ignored if the pipeline has rasterization disabled or if the subpass of the render pass the pipeline is created against does not use any color attachments.
	CreateInfo.pColorBlendState = &PipelineColorBlendStateCreateInfo;
	//pDynamicState is a pointer to VkPipelineDynamicStateCreateInfoand is used to indicate which properties of the pipeline state object are dynamicand can be changed independently of the pipeline state.This can be NULL, which means no state in the pipeline is considered dynamic.
	CreateInfo.pDynamicState = &PipelineDynamicStateCreateInfo; // Optional
	//layout is the description of binding locations used by both the pipeline and descriptor sets used with the pipeline.
	CreateInfo.layout = pipelineLayout;

	CreateInfo.renderPass = DCAST(_RP, VulkanRenderPass)->GetVkRenderPass();
	//subpass is the index of the subpass in the render pass where this pipeline will be used.
	CreateInfo.subpass = 0;
	//basePipelineHandle is a pipeline to derive from.
	CreateInfo.basePipelineHandle = VK_NULL_HANDLE; // Optional
	//basePipelineIndex is an index into the pCreateInfos parameter to use as a pipeline to derive from.
	CreateInfo.basePipelineIndex = -1; // Optional

	GS_VK_CHECK(vkCreateGraphicsPipelines(m_Device, VK_NULL_HANDLE, 1, &CreateInfo, ALLOCATOR, &GraphicsPipeline), "Failed to create Graphics Pipeline!")
}

VulkanGraphicsPipeline::~VulkanGraphicsPipeline()
{
	vkDestroyPipeline(m_Device, GraphicsPipeline, ALLOCATOR);
}

VulkanComputePipeline::VulkanComputePipeline(VkDevice _Device) : VulkanObject(_Device)
{
	VkComputePipelineCreateInfo CreateInfo = { VK_STRUCTURE_TYPE_COMPUTE_PIPELINE_CREATE_INFO };
	CreateInfo.stage; //TODO
	CreateInfo.basePipelineHandle = VK_NULL_HANDLE;
	CreateInfo.basePipelineIndex = -1;

	GS_VK_CHECK(vkCreateComputePipelines(m_Device, VK_NULL_HANDLE, 1, &CreateInfo, ALLOCATOR, &ComputePipeline), "Failed to create Compute Pipeline!")
}

VulkanComputePipeline::~VulkanComputePipeline()
{
	vkDestroyPipeline(m_Device, ComputePipeline, ALLOCATOR);
}