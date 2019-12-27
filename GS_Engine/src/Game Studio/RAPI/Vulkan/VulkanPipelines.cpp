#include "Vulkan.h"

#include "Native/VKDevice.h"

#include "VulkanPipelines.h"

#include "RAPI/Vulkan/Native/VKShaderModule.h"

#include "VulkanRenderPass.h"
#include "VulkanUniformLayout.h"

#include "RAPI/Window.h"

VKGraphicsPipelineCreator VulkanGraphicsPipeline::CreateVk_GraphicsPipelineCreator(VKDevice* _Device, const GraphicsPipelineCreateInfo& _GPCI, VkPipeline _OldPipeline)
{
	//  VERTEX INPUT STATE

	Array<VkVertexInputBindingDescription, 4> BindingDescriptions(1);
	BindingDescriptions[0].binding = 0;
	BindingDescriptions[0].stride = _GPCI.VDescriptor->GetSize();
	BindingDescriptions[0].inputRate = VK_VERTEX_INPUT_RATE_VERTEX;

	Array<VkVertexInputAttributeDescription, 8>	VertexElements(_GPCI.VDescriptor->GetAttributeCount());
	for (uint8 i = 0; i < VertexElements.getLength(); ++i)
	{
		VertexElements[i].binding = 0;
		VertexElements[i].location = i;
		VertexElements[i].format = ShaderDataTypesToVkFormat(_GPCI.VDescriptor->GetAttribute(i));
		VertexElements[i].offset = _GPCI.VDescriptor->GetOffsetToMember(i);
	}

	VkPipelineVertexInputStateCreateInfo VertexInputState = { VK_STRUCTURE_TYPE_PIPELINE_VERTEX_INPUT_STATE_CREATE_INFO };
	VertexInputState.vertexBindingDescriptionCount = BindingDescriptions.getLength();
	VertexInputState.pVertexBindingDescriptions = BindingDescriptions.getData();
	VertexInputState.vertexAttributeDescriptionCount = VertexElements.getLength();
	VertexInputState.pVertexAttributeDescriptions = VertexElements.getData();


	//  INPUT ASSEMBLY STATE
	VkPipelineInputAssemblyStateCreateInfo InputAssemblyState = { VK_STRUCTURE_TYPE_PIPELINE_INPUT_ASSEMBLY_STATE_CREATE_INFO };

	InputAssemblyState.topology = VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST;
	InputAssemblyState.primitiveRestartEnable = VK_FALSE;



	//  TESSELLATION STATE
	VkPipelineTessellationStateCreateInfo TessellationState = { VK_STRUCTURE_TYPE_PIPELINE_TESSELLATION_STATE_CREATE_INFO };


	//  VIEWPORT STATE
	VkViewport Viewport;
	Viewport.x = 0;
	Viewport.y = 0;
	auto window_extent = _GPCI.ActiveWindow->GetWindowExtent();
	Viewport.width = window_extent.Width;
	Viewport.height = window_extent.Height;
	Viewport.minDepth = 0.0f;
	Viewport.maxDepth = 1.0f;

	VkRect2D Scissor = { { 0, 0 }, { window_extent.Width, window_extent.Height } };

	VkPipelineViewportStateCreateInfo ViewportState = { VK_STRUCTURE_TYPE_PIPELINE_VIEWPORT_STATE_CREATE_INFO };
	ViewportState.viewportCount = 1;
	ViewportState.pViewports = &Viewport;
	ViewportState.scissorCount = 1;
	ViewportState.pScissors = &Scissor;

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



	//  MULTISAMPLE STATE
	VkPipelineMultisampleStateCreateInfo MultisampleState = { VK_STRUCTURE_TYPE_PIPELINE_MULTISAMPLE_STATE_CREATE_INFO };
	MultisampleState.sampleShadingEnable = VK_FALSE;
	MultisampleState.rasterizationSamples = VK_SAMPLE_COUNT_1_BIT;
	MultisampleState.minSampleShading = 1.0f; // Optional
	MultisampleState.pSampleMask = nullptr; // Optional
	MultisampleState.alphaToCoverageEnable = VK_FALSE; // Optional
	MultisampleState.alphaToOneEnable = VK_FALSE; // Optional



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



	//  COLOR BLEND STATE
	VkPipelineColorBlendAttachmentState ColorBlendAttachment = {};
	ColorBlendAttachment.colorWriteMask = VK_COLOR_COMPONENT_R_BIT | VK_COLOR_COMPONENT_G_BIT | VK_COLOR_COMPONENT_B_BIT | VK_COLOR_COMPONENT_A_BIT;
	ColorBlendAttachment.blendEnable = VK_FALSE;//_GPCI.PipelineDescriptor.BlendEnable;
	ColorBlendAttachment.srcColorBlendFactor = VK_BLEND_FACTOR_ONE;
	ColorBlendAttachment.dstColorBlendFactor = VK_BLEND_FACTOR_ZERO;
	ColorBlendAttachment.colorBlendOp = VK_BLEND_OP_ADD;
	ColorBlendAttachment.srcAlphaBlendFactor = VK_BLEND_FACTOR_ONE;
	ColorBlendAttachment.dstAlphaBlendFactor = VK_BLEND_FACTOR_ZERO;
	ColorBlendAttachment.alphaBlendOp = VK_BLEND_OP_ADD;

	VkPipelineColorBlendStateCreateInfo ColorBlendState = { VK_STRUCTURE_TYPE_PIPELINE_COLOR_BLEND_STATE_CREATE_INFO };
	ColorBlendState.logicOpEnable = VK_FALSE;
	ColorBlendState.logicOp = VK_LOGIC_OP_COPY; // Optional
	ColorBlendState.attachmentCount = 1;
	ColorBlendState.pAttachments = &ColorBlendAttachment;
	ColorBlendState.blendConstants[0] = 0.0f; // Optional
	ColorBlendState.blendConstants[1] = 0.0f; // Optional
	ColorBlendState.blendConstants[2] = 0.0f; // Optional
	ColorBlendState.blendConstants[3] = 0.0f; // Optional



	//  DYNAMIC STATE

	VkPipelineDynamicStateCreateInfo DynamicState = { VK_STRUCTURE_TYPE_PIPELINE_DYNAMIC_STATE_CREATE_INFO };
	Array<VkDynamicState, 1> DynamicStates = { VK_DYNAMIC_STATE_VIEWPORT };
	DynamicState.dynamicStateCount = DynamicStates.getCapacity();
	DynamicState.pDynamicStates = DynamicStates.getData();


	///////////////////////////////////////////////////////////////////////////////////////////////////////////

	Array<VkPipelineShaderStageCreateInfo, 8> PSSCI(_GPCI.PipelineDescriptor.Stages.getLength());
	Array<VkShaderModuleCreateInfo, 8> VSMCI(_GPCI.PipelineDescriptor.Stages.getLength());
	DArray<DArray<uint32, uint32>> SPIRV(_GPCI.PipelineDescriptor.Stages.getLength());
	DArray<VkShaderModule> SMS(_GPCI.PipelineDescriptor.Stages.getLength());

	for (uint8 i = 0; i < _GPCI.PipelineDescriptor.Stages.getLength(); ++i)
	{
		PSSCI[i].sType = VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO;
		PSSCI[i].pNext = nullptr;
		PSSCI[i].flags = 0;
		PSSCI[i].stage = ShaderTypeToVkShaderStageFlagBits(_GPCI.PipelineDescriptor.Stages[i].Type);
		PSSCI[i].pName = "main";

		//TODO: ask for shader name from outside
		auto TT = VKShaderModule::CompileGLSLToSpirV(*_GPCI.PipelineDescriptor.Stages[i].ShaderCode, FString("Vertex Shader"), PSSCI[i].stage);

		VSMCI[i].sType = VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO;
		VSMCI[i].pNext = nullptr;
		VSMCI[i].flags = 0;
		VSMCI[i].codeSize = TT.getLengthSize();
		VSMCI[i].pCode = TT.getData();
		
		auto T = vkCreateShaderModule(_Device->GetVkDevice(), &VSMCI[i], ALLOCATOR, &SMS[i]);

		PSSCI[i].module = SMS[i];
		PSSCI[i].pSpecializationInfo = nullptr;
	}

	//////////////////////////////////////////////////////////////////////////////////////////////////////////////

	VkGraphicsPipelineCreateInfo CreateInfo = { VK_STRUCTURE_TYPE_GRAPHICS_PIPELINE_CREATE_INFO };

	CreateInfo.stageCount = PSSCI.getLength();
	CreateInfo.pStages = PSSCI.getData();
	CreateInfo.pVertexInputState = &VertexInputState;
	CreateInfo.pInputAssemblyState = &InputAssemblyState;
	CreateInfo.pTessellationState = &TessellationState;
	CreateInfo.pViewportState = &ViewportState;
	CreateInfo.pRasterizationState = &RasterizationState;
	CreateInfo.pMultisampleState = &MultisampleState;
	CreateInfo.pDepthStencilState = nullptr;//&DepthStencilState;
	CreateInfo.pColorBlendState = &ColorBlendState;
	CreateInfo.pDynamicState = &DynamicState;
	CreateInfo.layout = SCAST(VulkanUniformLayout*, _GPCI.UniformLayout)->GetVKPipelineLayout().GetHandle();
	CreateInfo.renderPass = SCAST(VulkanRenderPass*, _GPCI.RenderPass)->GetVKRenderPass().GetHandle();
	CreateInfo.subpass = 0;
	CreateInfo.basePipelineHandle = _OldPipeline; // Optional
	CreateInfo.basePipelineIndex = _OldPipeline ? 0 : -1;

	return VKGraphicsPipelineCreator(_Device, &CreateInfo);
}

VulkanGraphicsPipeline::VulkanGraphicsPipeline(VKDevice* _Device, const GraphicsPipelineCreateInfo& _GPCI) :
	Pipeline(CreateVk_GraphicsPipelineCreator(_Device, _GPCI))
{
}
