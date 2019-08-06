#include "Vulkan.h"

#include "VulkanPipelines.h"

#include "RAPI/RenderPass.h"
#include "RAPI/Vulkan/Native/Vk_ShaderModule.h"

#include "VulkanRenderPass.h"

#include "Containers/Tuple.h"
#include <vector>
#include <fstream>

Tuple<std::vector<char>, size_t> GetShaderCode(const FString& _Name)
{
	Tuple<std::vector<char>, size_t> Result;

	std::ifstream file(_Name.c_str(), std::ios::ate | std::ios::binary);

	if (!file.is_open())
	{
		throw std::runtime_error("failed to open file!");
	}

	const size_t fileSize = size_t(file.tellg());
	std::vector<char> buffer(fileSize);

	file.seekg(0);
	file.read(buffer.data(), fileSize);

	file.close();

	Result.First = buffer;
	Result.Second = fileSize;

	return Result;
}

FVector<VkPipelineShaderStageCreateInfo> VulkanGraphicsPipeline::StageInfoToVulkanStageInfo(const ShaderStages& _SI, const Vk_Device& _Device)
{
	FVector<VkPipelineShaderStageCreateInfo> Result (2);

	if(_SI.VertexShader)
	{
		VkPipelineShaderStageCreateInfo VS = { VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO };
		VS.stage = ShaderTypeToVkShaderStageFlagBits(_SI.VertexShader->Type);
		VS.module = Vk_ShaderModule(_Device, _SI.VertexShader->ShaderCode, VS.stage);
		VS.pName = "main";

		Result.push_back(VS);
	}

	if(_SI.TessellationShader)
	{
		VkPipelineShaderStageCreateInfo TS = { VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO };
		TS.stage = ShaderTypeToVkShaderStageFlagBits(_SI.TessellationShader->Type);
		TS.module = Vk_ShaderModule(_Device, _SI.TessellationShader->ShaderCode, TS.stage);
		TS.pName = "main";

		Result.push_back(TS);
	}

	if (_SI.GeometryShader)
	{
		VkPipelineShaderStageCreateInfo GS = { VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO };
		GS.stage = ShaderTypeToVkShaderStageFlagBits(_SI.GeometryShader->Type);
		GS.module = Vk_ShaderModule(_Device, _SI.GeometryShader->ShaderCode, GS.stage);
		GS.pName = "main";

		Result.push_back(GS);
	}

	if (_SI.FragmentShader)
	{
		VkPipelineShaderStageCreateInfo FS = { VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO };
		FS.stage = ShaderTypeToVkShaderStageFlagBits(_SI.FragmentShader->Type);
		FS.module = Vk_ShaderModule(_Device, _SI.FragmentShader->ShaderCode, FS.stage);
		FS.pName = "main";

		Result.push_back(FS);
	}

	return Result;
}

PipelineState VulkanGraphicsPipeline::CreatePipelineState(const Extent2D& _Extent, const ShaderStages& _SI,	const VertexDescriptor& _VD)
{
	PipelineState State;

	//  VERTEX INPUT STATE
	VkPipelineVertexInputStateCreateInfo VertexInputState = { VK_STRUCTURE_TYPE_PIPELINE_VERTEX_INPUT_STATE_CREATE_INFO };

	VkVertexInputBindingDescription BindingDescription = {};
	BindingDescription.binding = 0;
	BindingDescription.stride = _VD.GetSize();
	BindingDescription.inputRate = VK_VERTEX_INPUT_RATE_VERTEX;

	FVector<VkVertexInputAttributeDescription> VertexElements(_VD.GetAttributeCount());
	for (uint8 i = 0; i < VertexElements.length(); i++)
	{
		VertexElements[i].binding = 0;
		VertexElements[i].location = i;
		VertexElements[i].format = ShaderDataTypesToVkFormat(_VD.GetAttribute(i));
		VertexElements[i].offset = _VD.GetOffsetToMember(i);
	}

	VertexInputState.vertexBindingDescriptionCount = 1;
	VertexInputState.pVertexBindingDescriptions = &BindingDescription;
	VertexInputState.vertexAttributeDescriptionCount = 1;
	VertexInputState.pVertexAttributeDescriptions = VertexElements.data();

	State.PipelineVertexInputState = &VertexInputState;

	//  INPUT ASSEMBLY STATE
	VkPipelineInputAssemblyStateCreateInfo InputAssemblyState = { VK_STRUCTURE_TYPE_PIPELINE_INPUT_ASSEMBLY_STATE_CREATE_INFO };

	InputAssemblyState.topology = VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST;
	InputAssemblyState.primitiveRestartEnable = VK_FALSE;

	State.PipelineInputAssemblyState = &InputAssemblyState;


	//  TESSELLATION STATE
	VkPipelineTessellationStateCreateInfo TessellationState = { VK_STRUCTURE_TYPE_PIPELINE_TESSELLATION_STATE_CREATE_INFO };

	State.PipelineTessellationState = &TessellationState;

	//  VIEWPORT STATE
	VkViewport Viewport = {};
	Viewport.x = 0;
	Viewport.y = 0;
	Viewport.width = _Extent.Width;
	Viewport.height = _Extent.Height;
	Viewport.minDepth = 0.0f;
	Viewport.maxDepth = 1.0f;

	VkRect2D Scissor = { { 0, 0 }, { Extent2DToVkExtent2D(_Extent) } };

	VkPipelineViewportStateCreateInfo ViewportState = { VK_STRUCTURE_TYPE_PIPELINE_VIEWPORT_STATE_CREATE_INFO };
	ViewportState.viewportCount = 1;
	ViewportState.pViewports = &Viewport;
	ViewportState.scissorCount = 1;
	ViewportState.pScissors = &Scissor;

	State.PipelineViewportState = &ViewportState;

	//  RASTERIZATION STATE
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

	State.PipelineRasterizationState = &RasterizationState;

	//  MULTISAMPLE STATE
	VkPipelineMultisampleStateCreateInfo MultisampleState = { VK_STRUCTURE_TYPE_PIPELINE_MULTISAMPLE_STATE_CREATE_INFO };
	MultisampleState.sampleShadingEnable = VK_FALSE;
	MultisampleState.rasterizationSamples = VK_SAMPLE_COUNT_1_BIT;
	MultisampleState.minSampleShading = 1.0f; // Optional
	MultisampleState.pSampleMask = nullptr; // Optional
	MultisampleState.alphaToCoverageEnable = VK_FALSE; // Optional
	MultisampleState.alphaToOneEnable = VK_FALSE; // Optional

	State.PipelineMultisampleState = &MultisampleState;

	//  DEPTH STENCIL STATE
	VkPipelineDepthStencilStateCreateInfo DepthStencilState = { VK_STRUCTURE_TYPE_PIPELINE_DEPTH_STENCIL_STATE_CREATE_INFO };

	State.PipelineDepthStencilState = &DepthStencilState;

	//  COLOR BLEND STATE
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
	ColorBlendState.logicOpEnable = VK_FALSE;
	ColorBlendState.logicOp = VK_LOGIC_OP_COPY; // Optional
	ColorBlendState.attachmentCount = 1;
	ColorBlendState.pAttachments = &ColorBlendAttachment;
	ColorBlendState.blendConstants[0] = 0.0f; // Optional
	ColorBlendState.blendConstants[1] = 0.0f; // Optional
	ColorBlendState.blendConstants[2] = 0.0f; // Optional
	ColorBlendState.blendConstants[3] = 0.0f; // Optional

	State.PipelineColorBlendState = &ColorBlendState;

	//  DYNAMIC STATE
	VkDynamicState DynamicStates[] = {
	VK_DYNAMIC_STATE_VIEWPORT,
	VK_DYNAMIC_STATE_LINE_WIDTH
	};

	VkPipelineDynamicStateCreateInfo DynamicState = { VK_STRUCTURE_TYPE_PIPELINE_DYNAMIC_STATE_CREATE_INFO };
	DynamicState.dynamicStateCount = 2;
	DynamicState.pDynamicStates = DynamicStates;

	State.PipelineDynamicState = &DynamicState;

	return State;
}

VulkanGraphicsPipeline::VulkanGraphicsPipeline(const Vk_Device& _Device, RenderPass* _RP, Extent2D _SwapchainSize, const ShaderStages& _SI, const VertexDescriptor& _VD) :
	Layout(_Device),
	Pipeline(_Device, SCAST(VulkanRenderPass*, _RP)->GetVk_RenderPass(), Extent2DToVkExtent2D(_SwapchainSize), Layout, StageInfoToVulkanStageInfo(_SI, _Device), CreatePipelineState(_SwapchainSize, _SI, _VD))
{
}

VulkanComputePipeline::VulkanComputePipeline(const Vk_Device& _Device) : ComputePipeline(_Device)
{
}