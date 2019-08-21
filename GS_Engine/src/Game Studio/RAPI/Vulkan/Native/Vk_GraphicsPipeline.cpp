#include "Vk_GraphicsPipeline.h"

#include "RAPI/Vulkan/Vulkan.h"

#include "Vk_Device.h"

#include "Vk_PipelineLayout.h"
#include "Vk_RenderPass.h"
#include "Vk_ShaderModule.h"
#include "RAPI/Pipelines.h"


Vk_GraphicsPipeline::Vk_GraphicsPipeline(const Vk_Device& _Device, const Vk_RenderPass& _RP, VkExtent2D _SwapchainSize, const Vk_PipelineLayout& _PL, const ShaderStages& _SI, const VertexDescriptor& _VD) : VulkanObject(_Device)
{
	VkPipelineVertexInputStateCreateInfo		* PipelineVertexInputState = nullptr;
	VkPipelineInputAssemblyStateCreateInfo		* PipelineInputAssemblyState = nullptr;
	VkPipelineTessellationStateCreateInfo		* PipelineTessellationState = nullptr;
	VkPipelineViewportStateCreateInfo			* PipelineViewportState = nullptr;
	VkPipelineRasterizationStateCreateInfo		* PipelineRasterizationState = nullptr;
	VkPipelineMultisampleStateCreateInfo		* PipelineMultisampleState = nullptr;
	VkPipelineDepthStencilStateCreateInfo		* PipelineDepthStencilState = nullptr;
	VkPipelineColorBlendStateCreateInfo			* PipelineColorBlendState = nullptr;
	VkPipelineDynamicStateCreateInfo			* PipelineDynamicState = nullptr;

	//  VERTEX INPUT STATE

	VkPipelineVertexInputStateCreateInfo VertexInputState = { VK_STRUCTURE_TYPE_PIPELINE_VERTEX_INPUT_STATE_CREATE_INFO };

	FVector<VkVertexInputBindingDescription> BindingDescriptions(1);
	BindingDescriptions[0].binding = 0;
	BindingDescriptions[0].stride = _VD.GetSize();
	BindingDescriptions[0].inputRate = VK_VERTEX_INPUT_RATE_VERTEX;

	FVector<VkVertexInputAttributeDescription>	VertexElements(_VD.GetAttributeCount());
	for (uint8 i = 0; i < VertexElements.capacity(); i++)
	{
		VertexElements[i].binding = 0;
		VertexElements[i].location = i;
		VertexElements[i].format = ShaderDataTypesToVkFormat(_VD.GetAttribute(i));
		VertexElements[i].offset = _VD.GetOffsetToMember(i);
	}

	VertexInputState.vertexBindingDescriptionCount = BindingDescriptions.capacity();
	VertexInputState.pVertexBindingDescriptions = BindingDescriptions.data();
	VertexInputState.vertexAttributeDescriptionCount = VertexElements.capacity();
	VertexInputState.pVertexAttributeDescriptions = VertexElements.data();

	PipelineVertexInputState = &VertexInputState;

	//  INPUT ASSEMBLY STATE
	VkPipelineInputAssemblyStateCreateInfo InputAssemblyState = { VK_STRUCTURE_TYPE_PIPELINE_INPUT_ASSEMBLY_STATE_CREATE_INFO };

	InputAssemblyState.topology = VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST;
	InputAssemblyState.primitiveRestartEnable = VK_FALSE;

	PipelineInputAssemblyState = &InputAssemblyState;


	//  TESSELLATION STATE
	VkPipelineTessellationStateCreateInfo TessellationState = { VK_STRUCTURE_TYPE_PIPELINE_TESSELLATION_STATE_CREATE_INFO };

	PipelineTessellationState = nullptr;//&TessellationState;


	//  VIEWPORT STATE
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

	PipelineViewportState = &ViewportState;


	//  RASTERIZATION STATE
	VkPipelineRasterizationStateCreateInfo RasterizationState = { VK_STRUCTURE_TYPE_PIPELINE_RASTERIZATION_STATE_CREATE_INFO };
	RasterizationState.depthClampEnable = VK_FALSE;
	RasterizationState.rasterizerDiscardEnable = VK_FALSE;
	RasterizationState.polygonMode = VK_POLYGON_MODE_FILL;
	RasterizationState.lineWidth = 1.0f;
	RasterizationState.frontFace = VK_FRONT_FACE_CLOCKWISE;
	RasterizationState.cullMode = VK_CULL_MODE_NONE;
	RasterizationState.depthBiasEnable = VK_FALSE;
	RasterizationState.depthBiasConstantFactor = 0.0f; // Optional
	RasterizationState.depthBiasClamp = 0.0f; // Optional
	RasterizationState.depthBiasSlopeFactor = 0.0f; // Optional

	PipelineRasterizationState = &RasterizationState;


	//  MULTISAMPLE STATE
	VkPipelineMultisampleStateCreateInfo MultisampleState = { VK_STRUCTURE_TYPE_PIPELINE_MULTISAMPLE_STATE_CREATE_INFO };
	MultisampleState.sampleShadingEnable = VK_FALSE;
	MultisampleState.rasterizationSamples = VK_SAMPLE_COUNT_1_BIT;
	MultisampleState.minSampleShading = 1.0f; // Optional
	MultisampleState.pSampleMask = nullptr; // Optional
	MultisampleState.alphaToCoverageEnable = VK_FALSE; // Optional
	MultisampleState.alphaToOneEnable = VK_FALSE; // Optional

	PipelineMultisampleState = &MultisampleState;


	//  DEPTH STENCIL STATE
	VkPipelineDepthStencilStateCreateInfo DepthStencilState = { VK_STRUCTURE_TYPE_PIPELINE_DEPTH_STENCIL_STATE_CREATE_INFO };
	DepthStencilState.depthTestEnable = VK_FALSE;
	DepthStencilState.depthWriteEnable = VK_TRUE;
	DepthStencilState.depthCompareOp = VK_COMPARE_OP_NEVER;
	DepthStencilState.depthBoundsTestEnable = VK_FALSE;
	DepthStencilState.minDepthBounds = 0.0f; // Optional
	DepthStencilState.maxDepthBounds = 1.0f; // Optional
	DepthStencilState.stencilTestEnable = VK_FALSE;
	DepthStencilState.front = {}; // Optional
	DepthStencilState.back = {}; // Optional

	PipelineDepthStencilState = &DepthStencilState;


	//  COLOR BLEND STATE
	VkPipelineColorBlendAttachmentState ColorBlendAttachment = { VK_STRUCTURE_TYPE_PIPELINE_COLOR_BLEND_STATE_CREATE_INFO };
	ColorBlendAttachment.colorWriteMask = VK_COLOR_COMPONENT_R_BIT | VK_COLOR_COMPONENT_G_BIT | VK_COLOR_COMPONENT_B_BIT | VK_COLOR_COMPONENT_A_BIT;
	ColorBlendAttachment.blendEnable = VK_FALSE;
	ColorBlendAttachment.srcColorBlendFactor = VK_BLEND_FACTOR_ONE; // Optional
	ColorBlendAttachment.dstColorBlendFactor = VK_BLEND_FACTOR_ZERO; // Optional
	ColorBlendAttachment.colorBlendOp = VK_BLEND_OP_ADD; // Optional
	ColorBlendAttachment.srcAlphaBlendFactor = VK_BLEND_FACTOR_ONE; // Optional
	ColorBlendAttachment.dstAlphaBlendFactor = VK_BLEND_FACTOR_ZERO; // Optional
	ColorBlendAttachment.alphaBlendOp = VK_BLEND_OP_ADD; // Optional

	VkPipelineColorBlendStateCreateInfo ColorBlendState = { VK_STRUCTURE_TYPE_PIPELINE_COLOR_BLEND_STATE_CREATE_INFO };
	ColorBlendState.logicOpEnable = VK_FALSE;
	ColorBlendState.logicOp = VK_LOGIC_OP_COPY; // Optional
	ColorBlendState.attachmentCount = 1;
	ColorBlendState.pAttachments = &ColorBlendAttachment;
	ColorBlendState.blendConstants[0] = 0.0f; // Optional
	ColorBlendState.blendConstants[1] = 0.0f; // Optional
	ColorBlendState.blendConstants[2] = 0.0f; // Optional
	ColorBlendState.blendConstants[3] = 0.0f; // Optional

	PipelineColorBlendState = &ColorBlendState;


	//  DYNAMIC STATE

	PipelineDynamicState = nullptr;//&DynamicState;

	///////////////////////////////////////////////////////////////////////////////////////////////////////////

	FVector<VkPipelineShaderStageCreateInfo> PSSCI(2);

	//if (_SI.VertexShader)
	//{
		VkPipelineShaderStageCreateInfo VS = { VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO };
		VS.stage = ShaderTypeToVkShaderStageFlagBits(_SI.VertexShader->Type);
		Vk_ShaderModule VSSM(_Device, _SI.VertexShader->ShaderCode, VS.stage);
		VS.module = VSSM;
		VS.pName = "main";

		PSSCI.push_back(VS);
	//}

	if (_SI.TessellationShader)
	{
		VkPipelineShaderStageCreateInfo TS = { VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO };
		TS.stage = ShaderTypeToVkShaderStageFlagBits(_SI.TessellationShader->Type);
		TS.module = Vk_ShaderModule(_Device, _SI.TessellationShader->ShaderCode, TS.stage);
		TS.pName = "main";

		PSSCI.push_back(TS);
	}

	if (_SI.GeometryShader)
	{
		VkPipelineShaderStageCreateInfo GS = { VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO };
		GS.stage = ShaderTypeToVkShaderStageFlagBits(_SI.GeometryShader->Type);
		GS.module = Vk_ShaderModule(_Device, _SI.GeometryShader->ShaderCode, GS.stage);
		GS.pName = "main";

		PSSCI.push_back(GS);
	}

	//if (_SI.FragmentShader)
	//{
		VkPipelineShaderStageCreateInfo FS = { VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO };
		FS.stage = ShaderTypeToVkShaderStageFlagBits(_SI.FragmentShader->Type);
		Vk_ShaderModule FSSM(_Device, _SI.FragmentShader->ShaderCode, FS.stage);
		FS.module = FSSM;
		FS.pName = "main";

		PSSCI.push_back(FS);
	//}

	//////////////////////////////////////////////////////////////////////////////////////////////////////////////

	VkGraphicsPipelineCreateInfo CreateInfo = { VK_STRUCTURE_TYPE_GRAPHICS_PIPELINE_CREATE_INFO };
	CreateInfo.stageCount = PSSCI.length();
	//pStages is an array of size stageCount structures of type VkPipelineShaderStageCreateInfo
	//describing the set of the shader stages to be included in the graphics pipeline.
	CreateInfo.pStages = PSSCI.data();
	//pVertexInputState is a pointer to an instance of the VkPipelineVertexInputStateCreateInfo structure.
	CreateInfo.pVertexInputState = PipelineVertexInputState;
	//pInputAssemblyState is a pointer to an instance of the VkPipelineInputAssemblyStateCreateInfo structure which determines input assembly behavior, as described in Drawing Commands.
	CreateInfo.pInputAssemblyState = PipelineInputAssemblyState;
	//pTessellationState is a pointer to an instance of the VkPipelineTessellationStateCreateInfo structure, and is ignored if the pipeline does not include a tessellation control shader stage and tessellation evaluation shader stage.
	CreateInfo.pTessellationState = PipelineTessellationState;
	//pViewportState is a pointer to an instance of the VkPipelineViewportStateCreateInfo structure, and is ignored if the pipeline has rasterization disabled.
	CreateInfo.pViewportState = PipelineViewportState;
	//pRasterizationState is a pointer to an instance of the VkPipelineRasterizationStateCreateInfo structure.
	CreateInfo.pRasterizationState = PipelineRasterizationState;
	//pMultisampleState is a pointer to an instance of the VkPipelineMultisampleStateCreateInfo, and is ignored if the pipeline has rasterization disabled.
	CreateInfo.pMultisampleState = PipelineMultisampleState;
	//pDepthStencilState is a pointer to an instance of the VkPipelineDepthStencilStateCreateInfo structure, and is ignored if the pipeline has rasterization disabled or if the subpass of the render pass the pipeline is created against does not use a depth / stencil attachment.
	CreateInfo.pDepthStencilState = PipelineDepthStencilState; // Optional
	//pColorBlendState is a pointer to an instance of the VkPipelineColorBlendStateCreateInfo structure, and is ignored if the pipeline has rasterization disabled or if the subpass of the render pass the pipeline is created against does not use any color attachments.
	CreateInfo.pColorBlendState = PipelineColorBlendState;
	//pDynamicState is a pointer to VkPipelineDynamicStateCreateInfo and is used to indicate which properties of the pipeline state object are dynamic and can be changed independently of the pipeline state.This can be NULL, which means no state in the pipeline is considered dynamic.
	CreateInfo.pDynamicState = PipelineDynamicState; // Optional
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