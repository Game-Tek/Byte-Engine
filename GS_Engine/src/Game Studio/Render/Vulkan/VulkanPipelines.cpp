#include "Vulkan.h"

#include "VulkanPipelines.h"

#include "..\RenderPass.h"

#include "VulkanRenderPass.h"
#include "VulkanShader.h"



VulkanStageInfo StageInfoToVulkanStageInfo(const StageInfo& _SI)
{
	VulkanStageInfo Result;

	for (uint8 i = 0; i < _SI.ShaderCount; i++)
	{
		Result.Shaders[i] = SCAST(VulkanShader*, _SI.Shader[i])->GetVk_Shader().GetVkShaderModule();
		Result.ShaderTypes[i] = ShaderTypeToVkShaderStageFlagBits(_SI.Shader[i]->GetShaderType());
	}

	Result.ShaderCount = _SI.ShaderCount;

	return Result;
}

VulkanGraphicsPipeline::VulkanGraphicsPipeline(VkDevice _Device, RenderPass * _RP, Extent2D _SwapchainSize, const StageInfo& _SI) :
	Layout(_Device),
	Pipeline(_Device, SCAST(VulkanRenderPass*, _RP)->GetVk_RenderPass(), Extent2DToVkExtent2D(_SwapchainSize), Layout, StageInfoToVulkanStageInfo(_SI))
{
}

VulkanComputePipeline::VulkanComputePipeline(VkDevice _Device) : ComputePipeline(_Device)
{
}


//  VK_GRAPHICS PIPELINE

VkPipelineVertexInputStateCreateInfo Vk_GraphicsPipeline::CreateVertexInputState()
{
	VkPipelineVertexInputStateCreateInfo VertexInputState = { VK_STRUCTURE_TYPE_PIPELINE_VERTEX_INPUT_STATE_CREATE_INFO };

	VertexInputState.vertexBindingDescriptionCount = 0;
	VertexInputState.pVertexBindingDescriptions = nullptr; // Optional
	VertexInputState.vertexAttributeDescriptionCount = 0;
	VertexInputState.pVertexAttributeDescriptions = nullptr; // Optional

	return VertexInputState;
}

VkPipelineInputAssemblyStateCreateInfo Vk_GraphicsPipeline::CreateInputAssemblyState()
{
	VkPipelineInputAssemblyStateCreateInfo InputAssemblyState = { VK_STRUCTURE_TYPE_PIPELINE_INPUT_ASSEMBLY_STATE_CREATE_INFO };

	InputAssemblyState.topology = VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST;
	InputAssemblyState.primitiveRestartEnable = VK_FALSE;

	return InputAssemblyState;
}

VkPipelineTessellationStateCreateInfo Vk_GraphicsPipeline::CreateTessellationState()
{
	VkPipelineTessellationStateCreateInfo TessellationState = { VK_STRUCTURE_TYPE_PIPELINE_TESSELLATION_STATE_CREATE_INFO };

	return TessellationState;
}

VkPipelineViewportStateCreateInfo Vk_GraphicsPipeline::CreateViewportState(VkExtent2D _SwapchainSize)
{
	VkViewport Viewport = {};
	Viewport.x = 0;
	Viewport.y = 0;
	Viewport.width = _SwapchainSize.width;
	Viewport.height = _SwapchainSize.height;
	Viewport.minDepth = 0.0f;
	Viewport.maxDepth = 1.0f;

	VkRect2D Scissor = { { 0, 0 }, { _SwapchainSize } };

	VkPipelineViewportStateCreateInfo ViewportState = { VK_STRUCTURE_TYPE_PIPELINE_VIEWPORT_STATE_CREATE_INFO };
	ViewportState.viewportCount = 1;
	ViewportState.pViewports = &Viewport;
	ViewportState.scissorCount = 1;
	ViewportState.pScissors = &Scissor;

	return ViewportState;
}

VkPipelineRasterizationStateCreateInfo Vk_GraphicsPipeline::CreateRasterizationState()
{
	VkPipelineRasterizationStateCreateInfo RasterizationState = { VK_STRUCTURE_TYPE_PIPELINE_RASTERIZATION_STATE_CREATE_INFO };

	RasterizationState.sType = VK_STRUCTURE_TYPE_PIPELINE_RASTERIZATION_STATE_CREATE_INFO;
	RasterizationState.depthClampEnable = VK_FALSE;
	RasterizationState.rasterizerDiscardEnable = VK_FALSE;
	RasterizationState.polygonMode = VK_POLYGON_MODE_FILL;
	RasterizationState.lineWidth = 1.0f;
	RasterizationState.cullMode = VK_CULL_MODE_BACK_BIT;
	RasterizationState.frontFace = VK_FRONT_FACE_CLOCKWISE;
	RasterizationState.depthBiasEnable = VK_FALSE;
	RasterizationState.depthBiasConstantFactor = 0.0f; // Optional
	RasterizationState.depthBiasClamp = 0.0f; // Optional
	RasterizationState.depthBiasSlopeFactor = 0.0f; // Optional

	return RasterizationState;
}

VkPipelineMultisampleStateCreateInfo Vk_GraphicsPipeline::CreateMultisampleState()
{
	VkPipelineMultisampleStateCreateInfo MultisampleState = { VK_STRUCTURE_TYPE_PIPELINE_MULTISAMPLE_STATE_CREATE_INFO };
	MultisampleState.sampleShadingEnable = VK_FALSE;
	MultisampleState.rasterizationSamples = VK_SAMPLE_COUNT_1_BIT;
	MultisampleState.minSampleShading = 1.0f; // Optional
	MultisampleState.pSampleMask = nullptr; // Optional
	MultisampleState.alphaToCoverageEnable = VK_FALSE; // Optional
	MultisampleState.alphaToOneEnable = VK_FALSE; // Optional

	return MultisampleState;
}

VkPipelineDepthStencilStateCreateInfo Vk_GraphicsPipeline::CreateDepthStencilState()
{
	VkPipelineDepthStencilStateCreateInfo DepthStencilState = { VK_STRUCTURE_TYPE_PIPELINE_DEPTH_STENCIL_STATE_CREATE_INFO };
	return DepthStencilState;
}

VkPipelineColorBlendStateCreateInfo Vk_GraphicsPipeline::CreateColorBlendState()
{
	VkPipelineColorBlendAttachmentState ColorBlendAttachment = {};
	ColorBlendAttachment.colorWriteMask = VK_COLOR_COMPONENT_R_BIT | VK_COLOR_COMPONENT_G_BIT | VK_COLOR_COMPONENT_B_BIT | VK_COLOR_COMPONENT_A_BIT;
	ColorBlendAttachment.blendEnable = VK_FALSE;
	ColorBlendAttachment.srcColorBlendFactor = VK_BLEND_FACTOR_ONE; // Optional
	ColorBlendAttachment.dstColorBlendFactor = VK_BLEND_FACTOR_ZERO; // Optional
	ColorBlendAttachment.colorBlendOp = VK_BLEND_OP_ADD; // Optional
	ColorBlendAttachment.srcAlphaBlendFactor = VK_BLEND_FACTOR_ONE; // Optional
	ColorBlendAttachment.dstAlphaBlendFactor = VK_BLEND_FACTOR_ZERO; // Optional
	ColorBlendAttachment.alphaBlendOp = VK_BLEND_OP_ADD; // Optional

	VkPipelineColorBlendStateCreateInfo ColorBlendState = { VK_STRUCTURE_TYPE_PIPELINE_COLOR_BLEND_STATE_CREATE_INFO };
	ColorBlendState.sType = ;
	ColorBlendState.logicOpEnable = VK_FALSE;
	ColorBlendState.logicOp = VK_LOGIC_OP_COPY; // Optional
	ColorBlendState.attachmentCount = 1;
	ColorBlendState.pAttachments = &ColorBlendAttachment;
	ColorBlendState.blendConstants[0] = 0.0f; // Optional
	ColorBlendState.blendConstants[1] = 0.0f; // Optional
	ColorBlendState.blendConstants[2] = 0.0f; // Optional
	ColorBlendState.blendConstants[3] = 0.0f; // Optional

	return ColorBlendState;
}

VkPipelineDynamicStateCreateInfo Vk_GraphicsPipeline::CreateDynamicState()
{
	VkDynamicState DynamicStates[] = {
	VK_DYNAMIC_STATE_VIEWPORT,
	VK_DYNAMIC_STATE_LINE_WIDTH
	};

	VkPipelineDynamicStateCreateInfo DynamicState = { VK_STRUCTURE_TYPE_PIPELINE_DYNAMIC_STATE_CREATE_INFO };
	DynamicState.dynamicStateCount = 2;
	DynamicState.pDynamicStates = DynamicStates;

	return DynamicState;
}

Vk_GraphicsPipeline::Vk_GraphicsPipeline(VkDevice _Device, VkRenderPass _RP, VkExtent2D _SwapchainSize, VkPipelineLayout _PL, const VulkanStageInfo& _VSI) : VulkanObject(_Device)
{
	auto PipelineVertexInputStateCreateInfo		= CreateVertexInputState();
	auto PipelineInputAssemblyStateCreateInfo	= CreateInputAssemblyState();
	auto PipelineTessellationStateCreateInfo	= CreateTessellationState();
	auto PipelineViewportStateCreateInfo		= CreateViewportState(_SwapchainSize);
	auto PipelineRasterizationStateCreateInfo	= CreateRasterizationState();
	auto PipelineMultisampleStateCreateInfo		= CreateMultisampleState();
	auto PipelineDepthStencilStateCreateInfo	= CreateDepthStencilState();
	auto PipelineColorBlendStateCreateInfo		= CreateColorBlendState();
	auto PipelineDynamicStateCreateInfo			= CreateDynamicState();


	VkPipelineShaderStageCreateInfo ShaderStages[MAX_SHADER_STAGES];
	for (uint8 i = 0; i < _VSI.ShaderCount; i++)
	{
		ShaderStages[i].sType = VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO;
		ShaderStages[i].stage = _VSI.ShaderTypes[i];
		ShaderStages[i].module = _VSI.Shaders[i];
		ShaderStages[i].pName = "main";
	}

	VkGraphicsPipelineCreateInfo CreateInfo = { VK_STRUCTURE_TYPE_GRAPHICS_PIPELINE_CREATE_INFO };
	CreateInfo.stageCount = _VSI.ShaderCount;
	//pStages is an array of size stageCount structures of type VkPipelineShaderStageCreateInfo
	//describing the set of the shader stages to be included in the graphics pipeline.
	CreateInfo.pStages = ShaderStages;
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
	//pDynamicState is a pointer to VkPipelineDynamicStateCreateInfoand is used to indicate which properties of the pipeline state object are dynamic and can be changed independently of the pipeline state.This can be NULL, which means no state in the pipeline is considered dynamic.
	CreateInfo.pDynamicState = &PipelineDynamicStateCreateInfo; // Optional
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

Vk_ComputePipeline::Vk_ComputePipeline(VkDevice _Device) : VulkanObject(_Device)
{
	VkComputePipelineCreateInfo CreateInfo = { VK_STRUCTURE_TYPE_COMPUTE_PIPELINE_CREATE_INFO };
	CreateInfo.stage;
	CreateInfo.basePipelineHandle = VK_NULL_HANDLE;
	CreateInfo.basePipelineIndex = -1;

	GS_VK_CHECK(vkCreateComputePipelines(m_Device, VK_NULL_HANDLE, 1, &CreateInfo, ALLOCATOR, &ComputePipeline), "Failed to create Compute Pipeline!")
}

Vk_ComputePipeline::~Vk_ComputePipeline()
{
	vkDestroyPipeline(m_Device, ComputePipeline, ALLOCATOR);
}


//  VK PIPELINE LAYOUT

Vk_PipelineLayout::Vk_PipelineLayout(VkDevice _Device) : VulkanObject(_Device)
{
	VkPipelineLayoutCreateInfo PipelineLayoutCreateInfo = { VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO };
	PipelineLayoutCreateInfo.setLayoutCount = 0; // Optional
	PipelineLayoutCreateInfo.pSetLayouts = nullptr; // Optional
	PipelineLayoutCreateInfo.pushConstantRangeCount = 0; // Optional
	PipelineLayoutCreateInfo.pPushConstantRanges = nullptr; // Optional

	GS_VK_CHECK(vkCreatePipelineLayout(m_Device, &PipelineLayoutCreateInfo, ALLOCATOR, &Layout), "Failed to create Pieline Layout!")
}

Vk_PipelineLayout::~Vk_PipelineLayout()
{
	vkDestroyPipelineLayout(m_Device, Layout, ALLOCATOR);
}
