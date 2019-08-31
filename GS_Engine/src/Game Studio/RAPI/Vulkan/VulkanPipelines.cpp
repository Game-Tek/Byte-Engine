#include "Vulkan.h"

#include "VulkanPipelines.h"

#include "RAPI/RenderPass.h"
#include "RAPI/Vulkan/Native/VKShaderModule.h"

#include "VulkanRenderPass.h"

VKGraphicsPipelineCreator VulkanGraphicsPipeline::CreateVk_GraphicsPipelineCreator(VKDevice* _Device, const VKPipelineLayout& _PL, const VKRenderPass& _RP, const Extent2D& _Extent, const VertexDescriptor& _VD, const PipelineDescriptor& _PD, VkPipeline _OldPipeline)
{
	//  VERTEX INPUT STATE

	Array<VkVertexInputBindingDescription, 4> BindingDescriptions(1);
	BindingDescriptions[0].binding = 0;
	BindingDescriptions[0].stride = _VD.GetSize();
	BindingDescriptions[0].inputRate = VK_VERTEX_INPUT_RATE_VERTEX;

	Array<VkVertexInputAttributeDescription, 8>	VertexElements(_VD.GetAttributeCount());
	for (uint8 i = 0; i < VertexElements.length(); ++i)
	{
		VertexElements[i].binding = 0;
		VertexElements[i].location = i;
		VertexElements[i].format = ShaderDataTypesToVkFormat(_VD.GetAttribute(i));
		VertexElements[i].offset = _VD.GetOffsetToMember(i);
	}

	VkPipelineVertexInputStateCreateInfo VertexInputState = { VK_STRUCTURE_TYPE_PIPELINE_VERTEX_INPUT_STATE_CREATE_INFO };
	VertexInputState.vertexBindingDescriptionCount = BindingDescriptions.length();
	VertexInputState.pVertexBindingDescriptions = BindingDescriptions.data();
	VertexInputState.vertexAttributeDescriptionCount = VertexElements.length();
	VertexInputState.pVertexAttributeDescriptions = VertexElements.data();


	//  INPUT ASSEMBLY STATE
	VkPipelineInputAssemblyStateCreateInfo InputAssemblyState = { VK_STRUCTURE_TYPE_PIPELINE_INPUT_ASSEMBLY_STATE_CREATE_INFO };

	InputAssemblyState.topology = VK_PRIMITIVE_TOPOLOGY_TRIANGLE_STRIP;
	InputAssemblyState.primitiveRestartEnable = VK_FALSE;



	//  TESSELLATION STATE
	VkPipelineTessellationStateCreateInfo TessellationState = { VK_STRUCTURE_TYPE_PIPELINE_TESSELLATION_STATE_CREATE_INFO };



	//  VIEWPORT STATE
	VkViewport Viewport;
	Viewport.x = 0.0f;
	Viewport.y = 0.0f;
	Viewport.width = _Extent.Width;
	Viewport.height = _Extent.Height;
	Viewport.minDepth = 0.0f;
	Viewport.maxDepth = 1.0f;

	VkRect2D Scissor = { { 0, 0 }, Extent2DToVkExtent2D(_Extent) };

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
	VkPipelineColorBlendAttachmentState ColorBlendAttachment;
	ColorBlendAttachment.colorWriteMask = VK_COLOR_COMPONENT_R_BIT | VK_COLOR_COMPONENT_G_BIT | VK_COLOR_COMPONENT_B_BIT | VK_COLOR_COMPONENT_A_BIT;
	ColorBlendAttachment.blendEnable = VK_FALSE;
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
	DynamicState.dynamicStateCount = DynamicStates.capacity();
	DynamicState.pDynamicStates = DynamicStates.data();


	///////////////////////////////////////////////////////////////////////////////////////////////////////////

	Array<VkPipelineShaderStageCreateInfo, 8> PSSCI;

	//if (_PD.Stages.VertexShader)
	//{
	VkPipelineShaderStageCreateInfo VS = { VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO };
	VS.stage = ShaderTypeToVkShaderStageFlagBits(_PD.Stages.VertexShader->Type);

	auto VertexShaderCode = VKShaderModule::CompileGLSLToSpirV(_PD.Stages.VertexShader->ShaderCode, FString("Vertex Shader"), VK_SHADER_STAGE_VERTEX_BIT);

	VkShaderModuleCreateInfo VkVertexShaderModuleCreateInfo = { VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO };
	VkVertexShaderModuleCreateInfo.codeSize = VertexShaderCode.size();
	VkVertexShaderModuleCreateInfo.pCode = VertexShaderCode.data();

	auto vs = VKShaderModule(VKShaderModuleCreator(_Device, &VkVertexShaderModuleCreateInfo));

	VS.module = vs.GetHandle();
	VS.pName = "main";

	PSSCI.push_back(VS);
	//}

	//if (_PD.Stages.FragmentShader)
	//{
		VkPipelineShaderStageCreateInfo FS = { VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO };
		FS.stage = ShaderTypeToVkShaderStageFlagBits(_PD.Stages.FragmentShader->Type);

		auto FragmentShaderCode = VKShaderModule::CompileGLSLToSpirV(_PD.Stages.FragmentShader->ShaderCode, FString("Fragment Shader"), VK_SHADER_STAGE_FRAGMENT_BIT);

		VkShaderModuleCreateInfo VkFragmentShaderModuleCreateInfo = { VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO };
		VkFragmentShaderModuleCreateInfo.codeSize = FragmentShaderCode.size();
		VkFragmentShaderModuleCreateInfo.pCode = FragmentShaderCode.data();

		auto fs = VKShaderModule(VKShaderModuleCreator(_Device, &VkFragmentShaderModuleCreateInfo));

		FS.module = fs.GetHandle();
		FS.pName = "main";

		PSSCI.push_back(FS);
	//}

	//////////////////////////////////////////////////////////////////////////////////////////////////////////////

	VkGraphicsPipelineCreateInfo CreateInfo = { VK_STRUCTURE_TYPE_GRAPHICS_PIPELINE_CREATE_INFO };

	CreateInfo.stageCount = PSSCI.length();
	CreateInfo.pStages = PSSCI.data();
	CreateInfo.pVertexInputState = &VertexInputState;
	CreateInfo.pInputAssemblyState = &InputAssemblyState;
	CreateInfo.pTessellationState = &TessellationState;
	CreateInfo.pViewportState = &ViewportState;
	CreateInfo.pRasterizationState = &RasterizationState;
	CreateInfo.pMultisampleState = &MultisampleState;
	CreateInfo.pDepthStencilState = &DepthStencilState;
	CreateInfo.pColorBlendState = &ColorBlendState;
	CreateInfo.pDynamicState = nullptr;//&DynamicState;
	CreateInfo.layout = _PL.GetHandle();
	CreateInfo.renderPass = _RP.GetHandle();
	CreateInfo.subpass = 0;
	CreateInfo.basePipelineHandle = _OldPipeline; // Optional
	CreateInfo.basePipelineIndex = _OldPipeline ? 0 : -1;

	return VKGraphicsPipelineCreator(_Device, &CreateInfo);
}

VKPipelineLayoutCreator VulkanGraphicsPipeline::CreatePipelineLayout(VKDevice* _Device)
{
	VkPipelineLayoutCreateInfo PipelineLayoutCreateInfo = { VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO };
	return VKPipelineLayoutCreator(_Device, &PipelineLayoutCreateInfo);
}

VulkanGraphicsPipeline::VulkanGraphicsPipeline(VKDevice* _Device, RenderPass* _RP, Extent2D _SwapchainSize, const PipelineDescriptor& _PD, const VertexDescriptor& _VD) :
	Layout(CreatePipelineLayout(_Device)),
	Pipeline(CreateVk_GraphicsPipelineCreator(_Device, Layout, SCAST(VulkanRenderPass*, _RP)->GetVk_RenderPass(), _SwapchainSize, _VD, _PD))
{
}
