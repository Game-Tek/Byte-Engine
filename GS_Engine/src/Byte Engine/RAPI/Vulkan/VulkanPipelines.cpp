#include "VulkanPipelines.h"
#include "VulkanRenderPass.h"
#include "RAPI/Window.h"
#include "VulkanRenderDevice.h"
#include "VulkanBindings.h"

#include <shaderc/shaderc.hpp>

void VulkanShaders::CompileShader(const FString& code, const FString& shaderName, uint32 shaderStage, FVector<uint32>& result)
{
	shaderc_shader_kind shaderc_stage;

	switch (shaderStage)
	{
		case VK_SHADER_STAGE_VERTEX_BIT: shaderc_stage = shaderc_vertex_shader;	break;
		case VK_SHADER_STAGE_TESSELLATION_CONTROL_BIT: shaderc_stage = shaderc_tess_control_shader;	break;
		case VK_SHADER_STAGE_TESSELLATION_EVALUATION_BIT: shaderc_stage = shaderc_tess_evaluation_shader; break;
		case VK_SHADER_STAGE_GEOMETRY_BIT: shaderc_stage = shaderc_geometry_shader;	break;
		case VK_SHADER_STAGE_FRAGMENT_BIT: shaderc_stage = shaderc_fragment_shader;	break;
		case VK_SHADER_STAGE_COMPUTE_BIT: shaderc_stage = shaderc_compute_shader; break;
		default: shaderc_stage = shaderc_spirv_assembly; break;
	}

	const shaderc::Compiler shaderc_compiler;
	shaderc::CompileOptions shaderc_compile_options;
	shaderc_compile_options.SetTargetSpirv(shaderc_spirv_version_1_1);
	shaderc_compile_options.SetTargetEnvironment(shaderc_target_env_vulkan, shaderc_env_version_vulkan_1_1);
	shaderc_compile_options.SetSourceLanguage(shaderc_source_language_glsl);
	shaderc_compile_options.SetOptimizationLevel(shaderc_optimization_level_performance);
	const auto shaderc_module = shaderc_compiler.CompileGlslToSpv(code.c_str(), shaderc_stage, shaderName.c_str(), shaderc_compile_options);

	if (shaderc_module.GetCompilationStatus() != shaderc_compilation_status_success)
	{
		BE_BASIC_LOG_ERROR("Failed to compile shader: %s. Errors: %s", shaderName.c_str(), shaderc_module.GetErrorMessage().c_str())
	}

	result.init(shaderc_module.end() - shaderc_module.begin(), shaderc_module.begin());
}

VulkanGraphicsPipeline::VulkanGraphicsPipeline(VulkanRenderDevice* vulkanRenderDevice, const GraphicsPipelineCreateInfo& _GPCI)
{
	//  VERTEX INPUT STATE

	Array<VkVertexInputBindingDescription, 4> vk_vertex_input_binding_descriptions(1);
	vk_vertex_input_binding_descriptions[0].binding = 0;
	vk_vertex_input_binding_descriptions[0].stride = _GPCI.VDescriptor->GetSize();
	vk_vertex_input_binding_descriptions[0].inputRate = VK_VERTEX_INPUT_RATE_VERTEX;

	Array<VkVertexInputAttributeDescription, 8> vk_vertex_input_attribute_descriptions(_GPCI.VDescriptor->GetAttributeCount());
	for (uint8 i = 0; i < vk_vertex_input_attribute_descriptions.getLength(); ++i)
	{
		vk_vertex_input_attribute_descriptions[i].binding = 0;
		vk_vertex_input_attribute_descriptions[i].location = i;
		vk_vertex_input_attribute_descriptions[i].format = ShaderDataTypesToVkFormat(_GPCI.VDescriptor->GetAttribute(i));
		vk_vertex_input_attribute_descriptions[i].offset = _GPCI.VDescriptor->GetOffsetToMember(i);
	}

	VkPipelineVertexInputStateCreateInfo vk_pipeline_vertex_input_state_create_info{ VK_STRUCTURE_TYPE_PIPELINE_VERTEX_INPUT_STATE_CREATE_INFO };
	vk_pipeline_vertex_input_state_create_info.vertexBindingDescriptionCount = vk_vertex_input_binding_descriptions.getLength();
	vk_pipeline_vertex_input_state_create_info.pVertexBindingDescriptions = vk_vertex_input_binding_descriptions.getData();
	vk_pipeline_vertex_input_state_create_info.vertexAttributeDescriptionCount = vk_vertex_input_attribute_descriptions.getLength();
	vk_pipeline_vertex_input_state_create_info.pVertexAttributeDescriptions = vk_vertex_input_attribute_descriptions.getData();

	//  INPUT ASSEMBLY STATE
	VkPipelineInputAssemblyStateCreateInfo vk_pipeline_input_assembly_state_create_info{ VK_STRUCTURE_TYPE_PIPELINE_INPUT_ASSEMBLY_STATE_CREATE_INFO };
	vk_pipeline_input_assembly_state_create_info.topology = VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST;
	vk_pipeline_input_assembly_state_create_info.primitiveRestartEnable = VK_FALSE;

	//  TESSELLATION STATE
	VkPipelineTessellationStateCreateInfo vk_pipeline_tessellation_state_create_info{ VK_STRUCTURE_TYPE_PIPELINE_TESSELLATION_STATE_CREATE_INFO	};

	//  VIEWPORT STATE
	VkViewport vk_viewport;
	vk_viewport.x = 0;
	vk_viewport.y = 0;
	auto window_extent = _GPCI.ActiveWindow->GetWindowExtent();
	vk_viewport.width = window_extent.Width;
	vk_viewport.height = window_extent.Height;
	vk_viewport.minDepth = 0.0f;
	vk_viewport.maxDepth = 1.0f;

	VkRect2D vk_scissor = { { 0, 0 }, { window_extent.Width, window_extent.Height } };

	VkPipelineViewportStateCreateInfo vk_pipeline_viewport_state_create_info{ VK_STRUCTURE_TYPE_PIPELINE_VIEWPORT_STATE_CREATE_INFO };
	vk_pipeline_viewport_state_create_info.viewportCount = 1;
	vk_pipeline_viewport_state_create_info.pViewports = &vk_viewport;
	vk_pipeline_viewport_state_create_info.scissorCount = 1;
	vk_pipeline_viewport_state_create_info.pScissors = &vk_scissor;

	//  RASTERIZATION STATE
	VkPipelineRasterizationStateCreateInfo vk_pipeline_rasterization_state_create_info{ VK_STRUCTURE_TYPE_PIPELINE_RASTERIZATION_STATE_CREATE_INFO };
	vk_pipeline_rasterization_state_create_info.depthClampEnable = VK_FALSE;
	vk_pipeline_rasterization_state_create_info.rasterizerDiscardEnable = VK_FALSE;
	vk_pipeline_rasterization_state_create_info.polygonMode = VK_POLYGON_MODE_FILL;
	vk_pipeline_rasterization_state_create_info.lineWidth = 1.0f;
	vk_pipeline_rasterization_state_create_info.frontFace = VK_FRONT_FACE_COUNTER_CLOCKWISE;
	vk_pipeline_rasterization_state_create_info.cullMode = CullModeToVkCullModeFlagBits(_GPCI.PipelineDescriptor.CullMode);
	vk_pipeline_rasterization_state_create_info.depthBiasEnable = VK_FALSE;
	vk_pipeline_rasterization_state_create_info.depthBiasConstantFactor = 0.0f; // Optional
	vk_pipeline_rasterization_state_create_info.depthBiasClamp = 0.0f; // Optional
	vk_pipeline_rasterization_state_create_info.depthBiasSlopeFactor = 0.0f; // Optional

	//  MULTISAMPLE STATE
	VkPipelineMultisampleStateCreateInfo vk_pipeline_multisample_state_create_info{ VK_STRUCTURE_TYPE_PIPELINE_MULTISAMPLE_STATE_CREATE_INFO };
	vk_pipeline_multisample_state_create_info.sampleShadingEnable = VK_FALSE;
	vk_pipeline_multisample_state_create_info.rasterizationSamples = VK_SAMPLE_COUNT_1_BIT;
	vk_pipeline_multisample_state_create_info.minSampleShading = 1.0f; // Optional
	vk_pipeline_multisample_state_create_info.pSampleMask = nullptr; // Optional
	vk_pipeline_multisample_state_create_info.alphaToCoverageEnable = VK_FALSE; // Optional
	vk_pipeline_multisample_state_create_info.alphaToOneEnable = VK_FALSE; // Optional

	//  DEPTH STENCIL STATE
	VkPipelineDepthStencilStateCreateInfo vk_pipeline_depthstencil_state_create_info{ VK_STRUCTURE_TYPE_PIPELINE_DEPTH_STENCIL_STATE_CREATE_INFO };
	vk_pipeline_depthstencil_state_create_info.depthTestEnable = VK_TRUE;
	vk_pipeline_depthstencil_state_create_info.depthWriteEnable = VK_TRUE;
	vk_pipeline_depthstencil_state_create_info.depthCompareOp = CompareOperationToVkCompareOp(_GPCI.PipelineDescriptor.DepthCompareOperation);
	vk_pipeline_depthstencil_state_create_info.depthBoundsTestEnable = VK_FALSE;
	vk_pipeline_depthstencil_state_create_info.minDepthBounds = 0.0f; // Optional
	vk_pipeline_depthstencil_state_create_info.maxDepthBounds = 1.0f; // Optional
	vk_pipeline_depthstencil_state_create_info.stencilTestEnable = VK_FALSE;
	vk_pipeline_depthstencil_state_create_info.front = {}; // Optional
	vk_pipeline_depthstencil_state_create_info.back = {}; // Optional

	//  COLOR BLEND STATE
	VkPipelineColorBlendAttachmentState vk_pipeline_colorblend_attachment_state{};
	vk_pipeline_colorblend_attachment_state.colorWriteMask = VK_COLOR_COMPONENT_R_BIT | VK_COLOR_COMPONENT_G_BIT | VK_COLOR_COMPONENT_B_BIT	| VK_COLOR_COMPONENT_A_BIT;
	vk_pipeline_colorblend_attachment_state.blendEnable = _GPCI.PipelineDescriptor.BlendEnable;
	vk_pipeline_colorblend_attachment_state.srcColorBlendFactor = VK_BLEND_FACTOR_ONE;
	vk_pipeline_colorblend_attachment_state.dstColorBlendFactor = VK_BLEND_FACTOR_ZERO;
	vk_pipeline_colorblend_attachment_state.colorBlendOp = VK_BLEND_OP_ADD;
	vk_pipeline_colorblend_attachment_state.srcAlphaBlendFactor = VK_BLEND_FACTOR_ONE;
	vk_pipeline_colorblend_attachment_state.dstAlphaBlendFactor = VK_BLEND_FACTOR_ZERO;
	vk_pipeline_colorblend_attachment_state.alphaBlendOp = VK_BLEND_OP_ADD;

	VkPipelineColorBlendStateCreateInfo vk_pipeline_colorblend_state_create_info{ VK_STRUCTURE_TYPE_PIPELINE_COLOR_BLEND_STATE_CREATE_INFO };
	vk_pipeline_colorblend_state_create_info.logicOpEnable = VK_FALSE;
	vk_pipeline_colorblend_state_create_info.logicOp = VK_LOGIC_OP_COPY; // Optional
	vk_pipeline_colorblend_state_create_info.attachmentCount = 1;
	vk_pipeline_colorblend_state_create_info.pAttachments = &vk_pipeline_colorblend_attachment_state;
	vk_pipeline_colorblend_state_create_info.blendConstants[0] = 0.0f; // Optional
	vk_pipeline_colorblend_state_create_info.blendConstants[1] = 0.0f; // Optional
	vk_pipeline_colorblend_state_create_info.blendConstants[2] = 0.0f; // Optional
	vk_pipeline_colorblend_state_create_info.blendConstants[3] = 0.0f; // Optional

	//  DYNAMIC STATE
	VkPipelineDynamicStateCreateInfo vk_pipeline_dynamic_state_create_info{ VK_STRUCTURE_TYPE_PIPELINE_DYNAMIC_STATE_CREATE_INFO };
	Array<VkDynamicState, 1> vk_dynamic_states = { VK_DYNAMIC_STATE_VIEWPORT };
	vk_pipeline_dynamic_state_create_info.dynamicStateCount = vk_dynamic_states.getCapacity();
	vk_pipeline_dynamic_state_create_info.pDynamicStates = vk_dynamic_states.getData();

	///////////////////////////////////////////////////////////////////////////////////////////////////////////

	Array<VkPipelineShaderStageCreateInfo, MAX_SHADER_STAGES, uint8> vk_pipeline_shader_stage_create_infos(_GPCI.PipelineDescriptor.Stages.getLength());
	Array<VkShaderModuleCreateInfo, MAX_SHADER_STAGES, uint8> vk_shader_module_create_infos(_GPCI.PipelineDescriptor.Stages.getLength());
	Array<FVector<uint32>, MAX_SHADER_STAGES, uint8> SPIRV(_GPCI.PipelineDescriptor.Stages.getLength());
	Array<VkShaderModule, MAX_SHADER_STAGES, uint8> vk_shader_modules(_GPCI.PipelineDescriptor.Stages.getLength());

	for (uint8 i = 0; i < _GPCI.PipelineDescriptor.Stages.getLength(); ++i)
	{
		vk_pipeline_shader_stage_create_infos[i].sType = VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO;
		vk_pipeline_shader_stage_create_infos[i].pNext = nullptr;
		vk_pipeline_shader_stage_create_infos[i].flags = 0;
		vk_pipeline_shader_stage_create_infos[i].stage = ShaderTypeToVkShaderStageFlagBits(_GPCI.PipelineDescriptor.Stages[i].Type);
		vk_pipeline_shader_stage_create_infos[i].pName = "main";

		//TODO: ask for shader name from outside
		VulkanShaders::CompileShader(*_GPCI.PipelineDescriptor.Stages[i].ShaderCode, FString("Shader"), vk_pipeline_shader_stage_create_infos[i].stage, SPIRV[i]);

		vk_shader_module_create_infos[i].sType = VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO;
		vk_shader_module_create_infos[i].pNext = nullptr;
		vk_shader_module_create_infos[i].flags = 0;
		vk_shader_module_create_infos[i].codeSize = SPIRV[i].getLengthSize();
		vk_shader_module_create_infos[i].pCode = SPIRV[i].getData();

		vkCreateShaderModule(static_cast<VulkanRenderDevice*>(_GPCI.RenderDevice)->GetVkDevice(), &vk_shader_module_create_infos[i], vulkanRenderDevice->GetVkAllocationCallbacks(), &vk_shader_modules[i]);

		vk_pipeline_shader_stage_create_infos[i].module = vk_shader_modules[i];
		vk_pipeline_shader_stage_create_infos[i].pSpecializationInfo = nullptr;
	}

	//////////////////////////////////////////////////////////////////////////////////////////////////////////////

	VkPipelineLayoutCreateInfo vk_pipeline_layout_create_info{ VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO };

	VkPushConstantRange vk_push_constant_range{};
	if (_GPCI.PushConstant)
	{
		vk_push_constant_range.size = static_cast<uint32>(_GPCI.PushConstant->Size);
		vk_push_constant_range.offset = 0;
		vk_push_constant_range.stageFlags = ShaderTypeToVkShaderStageFlagBits(_GPCI.PushConstant->Stage);

		vk_pipeline_layout_create_info.pushConstantRangeCount = 1;
		vk_pipeline_layout_create_info.pPushConstantRanges = &vk_push_constant_range;
	}
	else
	{
		vk_pipeline_layout_create_info.pushConstantRangeCount = 0;
		vk_pipeline_layout_create_info.pPushConstantRanges = nullptr;
	}

	Array<VkDescriptorSetLayout, 16> vk_descriptor_set_layouts(_GPCI.BindingsSets.getLength());
	{
		uint8 i = 0;
		for (auto& e : vk_descriptor_set_layouts)
		{
			e = static_cast<VulkanBindingsSet*>(_GPCI.BindingsSets[i])->GetVkDescriptorSetLayout();

			++i;
		}
	}

	vk_pipeline_layout_create_info.setLayoutCount = vk_descriptor_set_layouts.getLength();
	//What sets this pipeline layout uses.
	vk_pipeline_layout_create_info.pSetLayouts = vk_descriptor_set_layouts.getData();

	vkCreatePipelineLayout(static_cast<VulkanRenderDevice*>(_GPCI.RenderDevice)->GetVkDevice(), &vk_pipeline_layout_create_info, vulkanRenderDevice->GetVkAllocationCallbacks(), &vkPipelineLayout);

	VkGraphicsPipelineCreateInfo vk_graphics_pipeline_create_info{ VK_STRUCTURE_TYPE_GRAPHICS_PIPELINE_CREATE_INFO };
	vk_graphics_pipeline_create_info.stageCount = vk_pipeline_shader_stage_create_infos.getLength();
	vk_graphics_pipeline_create_info.pStages = vk_pipeline_shader_stage_create_infos.getData();
	vk_graphics_pipeline_create_info.pVertexInputState = &vk_pipeline_vertex_input_state_create_info;
	vk_graphics_pipeline_create_info.pInputAssemblyState = &vk_pipeline_input_assembly_state_create_info;
	vk_graphics_pipeline_create_info.pTessellationState = &vk_pipeline_tessellation_state_create_info;
	vk_graphics_pipeline_create_info.pViewportState = &vk_pipeline_viewport_state_create_info;
	vk_graphics_pipeline_create_info.pRasterizationState = &vk_pipeline_rasterization_state_create_info;
	vk_graphics_pipeline_create_info.pMultisampleState = &vk_pipeline_multisample_state_create_info;
	vk_graphics_pipeline_create_info.pDepthStencilState = &vk_pipeline_depthstencil_state_create_info;
	vk_graphics_pipeline_create_info.pColorBlendState = &vk_pipeline_colorblend_state_create_info;
	vk_graphics_pipeline_create_info.pDynamicState = &vk_pipeline_dynamic_state_create_info;
	vk_graphics_pipeline_create_info.layout = vkPipelineLayout;
	vk_graphics_pipeline_create_info.renderPass = static_cast<VulkanRenderPass*>(_GPCI.RenderPass)->GetVkRenderPass();
	vk_graphics_pipeline_create_info.subpass = 0;
	vk_graphics_pipeline_create_info.basePipelineHandle = _GPCI.ParentPipeline ? static_cast<VulkanGraphicsPipeline*>(_GPCI.ParentPipeline)->vkPipeline	: nullptr; // Optional
	vk_graphics_pipeline_create_info.basePipelineIndex = _GPCI.ParentPipeline ? 0 : -1;

	vkCreateGraphicsPipelines(static_cast<VulkanRenderDevice*>(_GPCI.RenderDevice)->GetVkDevice(), nullptr, 1, &vk_graphics_pipeline_create_info, vulkanRenderDevice->GetVkAllocationCallbacks(), &vkPipeline);
}

void VulkanGraphicsPipeline::Destroy(RenderDevice* renderDevice)
{
	auto vk_render_device = static_cast<VulkanRenderDevice*>(renderDevice);
	vkDestroyPipeline(vk_render_device->GetVkDevice(), vkPipeline, vk_render_device->GetVkAllocationCallbacks());
	vkDestroyPipelineLayout(vk_render_device->GetVkDevice(), vkPipelineLayout, vk_render_device->GetVkAllocationCallbacks());
}

