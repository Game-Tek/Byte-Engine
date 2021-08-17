#pragma once

#include "GAL/Pipelines.h"

#define VK_ENABLE_BETA_EXTENSIONS
#include <shaderc/shaderc.h>
#include <shaderc/shaderc.hpp>

#include "Vulkan.h"
#include "VulkanBindings.h"
#include "VulkanRenderPass.h"

namespace GTSL {
	class BufferInterface;
}

namespace GAL
{
	//static bool glslLangInitialized = false;

	//bool GAL::VulkanShader::CompileShader(GTSL::Range<const GTSL::char8_t> code, GTSL::Range<const GTSL::char8_t> shaderName, ShaderType shaderType, ShaderLanguage shaderLanguage, GTSL::Buffer& result, GTSL::Buffer& stringError)
	//{
	//	EShLanguage shaderc_stage;
	//	
	//	switch (ShaderTypeToVkShaderStageFlagBits(shaderType))
	//	{
	//	case VK_SHADER_STAGE_VERTEX_BIT: shaderc_stage = EShLangVertex;	break;
	//	case VK_SHADER_STAGE_TESSELLATION_CONTROL_BIT: shaderc_stage = EShLangTessControl;	break;
	//	case VK_SHADER_STAGE_TESSELLATION_EVALUATION_BIT: shaderc_stage = EShLangTessEvaluation; break;
	//	case VK_SHADER_STAGE_GEOMETRY_BIT: shaderc_stage = EShLangGeometry;	break;
	//	case VK_SHADER_STAGE_FRAGMENT_BIT: shaderc_stage = EShLangFragment;	break;
	//	case VK_SHADER_STAGE_COMPUTE_BIT: shaderc_stage = EShLangCompute; break;
	//	case VK_SHADER_STAGE_RAYGEN_BIT_KHR: shaderc_stage = EShLangRayGen; break;
	//	case VK_SHADER_STAGE_CLOSEST_HIT_BIT_KHR: shaderc_stage = EShLangClosestHit; break;
	//	case VK_SHADER_STAGE_ANY_HIT_BIT_KHR: shaderc_stage = EShLangAnyHit; break;
	//	case VK_SHADER_STAGE_MISS_BIT_KHR: shaderc_stage = EShLangMiss; break;
	//	case VK_SHADER_STAGE_INTERSECTION_BIT_KHR: shaderc_stage = EShLangIntersect; break;
	//	case VK_SHADER_STAGE_CALLABLE_BIT_KHR: shaderc_stage = EShLangCallable; break;
	//	default: GAL_DEBUG_BREAK;
	//	}
	//
	//	const TBuiltInResource DefaultTBuiltInResource{};
	//
	//	if(glslLangInitialized)
	//	{
	//		glslang::InitializeProcess();
	//		glslLangInitialized = true;
	//	}
	//
	//	glslang::TShader shader(shaderc_stage);
	//	GTSL::int32 length = code.ElementCount();
	//	auto* string = code.begin();
	//	shader.setStringsWithLengths(&string, &length, 1);
	//
	//	int ClientInputSemanticsVersion = 460; // maps to, say, #define VULKAN 100
	//	glslang::EShTargetClientVersion VulkanClientVersion = glslang::EShTargetVulkan_1_2;
	//	glslang::EShTargetLanguageVersion TargetVersion = glslang::EShTargetSpv_1_5;
	//
	//	shader.setEnvInput(glslang::EShSourceGlsl, shaderc_stage, glslang::EShClientVulkan, ClientInputSemanticsVersion);
	//	shader.setEnvClient(glslang::EShClientVulkan, VulkanClientVersion);
	//	shader.setEnvTarget(glslang::EShTargetSpv, TargetVersion);
	//
	//	TBuiltInResource resources;
	//	resources = DefaultTBuiltInResource;
	//	EShMessages messages = (EShMessages)(EShMsgSpvRules | EShMsgVulkanRules);
	//
	//	const GTSL::uint32 defaultVersion = 460;
	//
	//	if (!shader.parse(&resources, defaultVersion, false, messages))
	//	{
	//		const char* textBegin = "GLSL Parsing Failed for: ";
	//		stringError.WriteBytes(26, reinterpret_cast<const GTSL::byte*>(textBegin));
	//		stringError.WriteBytes(shaderName.ElementCount(), reinterpret_cast<const GTSL::byte*>(shaderName.begin()));
	//		stringError.WriteBytes(GTSL::StringLength(shader.getInfoLog()), reinterpret_cast<const GTSL::byte*>(shader.getInfoLog()));
	//		stringError.WriteBytes(GTSL::StringLength(shader.getInfoDebugLog()), reinterpret_cast<const GTSL::byte*>(shader.getInfoDebugLog()));
	//
	//		return false;
	//	}
	//
	//	glslang::TProgram Program;
	//	Program.addShader(&shader);
	//
	//	if (!Program.link(messages))
	//	{
	//		const char* textBegin = "GLSL Linking Failed for: ";
	//		stringError.WriteBytes(26, reinterpret_cast<const GTSL::byte*>(textBegin));
	//		stringError.WriteBytes(shaderName.ElementCount(), reinterpret_cast<const GTSL::byte*>(shaderName.begin()));
	//		stringError.WriteBytes(GTSL::StringLength(shader.getInfoLog()), reinterpret_cast<const GTSL::byte*>(shader.getInfoLog()));
	//		stringError.WriteBytes(GTSL::StringLength(shader.getInfoDebugLog()), reinterpret_cast<const GTSL::byte*>(shader.getInfoDebugLog()));
	//
	//		return false;
	//	}
	//
	//	std::vector<GTSL::uint32> spirv;
	//	spv::SpvBuildLogger logger;
	//	glslang::SpvOptions spvOptions;
	//	glslang::GlslangToSpv(*Program.getIntermediate(shaderc_stage), spirv, &logger, &spvOptions);
	//
	//	result.WriteBytes(spirv.size() * sizeof(GTSL::uint32), reinterpret_cast<const GTSL::byte*>(spirv.data()));
	//
	//	return true;
	//}
	
	class VulkanPipelineCache : public PipelineCache
	{
	public:
		VulkanPipelineCache() = default;

		void Initialize(const VulkanRenderDevice* renderDevice, bool externallySync, GTSL::Range<const GTSL::byte*> data) {

			VkPipelineCacheCreateInfo vkPipelineCacheCreateInfo{ VK_STRUCTURE_TYPE_PIPELINE_CACHE_CREATE_INFO };
			GTSL::SetBitAs(0, externallySync, vkPipelineCacheCreateInfo.flags);
			vkPipelineCacheCreateInfo.initialDataSize = data.Bytes();
			vkPipelineCacheCreateInfo.pInitialData = data.begin();

			renderDevice->VkCreatePipelineCache(renderDevice->GetVkDevice(), &vkPipelineCacheCreateInfo, renderDevice->GetVkAllocationCallbacks(), &pipelineCache);
		}
		
		void Initialize(const VulkanRenderDevice* renderDevice, GTSL::Range<const VulkanPipelineCache*> caches) {

			VkPipelineCacheCreateInfo vkPipelineCacheCreateInfo{ VK_STRUCTURE_TYPE_PIPELINE_CACHE_CREATE_INFO };

			renderDevice->VkCreatePipelineCache(renderDevice->GetVkDevice(), &vkPipelineCacheCreateInfo, renderDevice->GetVkAllocationCallbacks(), &pipelineCache);

			renderDevice->VkMergePipelineCaches(renderDevice->GetVkDevice(), pipelineCache, static_cast<GTSL::uint32>(caches.ElementCount()), reinterpret_cast<const VkPipelineCache*>(caches.begin()));
		}

		void Destroy(const VulkanRenderDevice* renderDevice) {
			renderDevice->VkDestroyPipelineCache(renderDevice->GetVkDevice(), pipelineCache, renderDevice->GetVkAllocationCallbacks());
			debugClear(pipelineCache);
		}

		[[nodiscard]] VkPipelineCache GetVkPipelineCache() const { return pipelineCache; }

		void GetCacheSize(const VulkanRenderDevice* renderDevice, GTSL::uint32& size) const {
			size_t data_size = 0;
			renderDevice->VkGetPipelineCacheData(renderDevice->GetVkDevice(), pipelineCache, &data_size, nullptr);
			size = static_cast<GTSL::uint32>(data_size);
		}

		template<class B>
		void GetCache(const VulkanRenderDevice* renderDevice, B& buffer) const {
			GTSL::uint64 data_size;
			renderDevice->VkGetPipelineCacheData(renderDevice->GetVkDevice(), pipelineCache, &data_size, buffer.begin());
			buffer.Resize(data_size);
		}
		
	private:
		VkPipelineCache pipelineCache = nullptr;
	};

	class VulkanShader final : public Shader
	{
	public:
		VulkanShader() = default;
		void Initialize(const VulkanRenderDevice* renderDevice, GTSL::Range<const GTSL::byte*> blob) {
			VkShaderModuleCreateInfo shaderModuleCreateInfo{ VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO };
			shaderModuleCreateInfo.codeSize = blob.Bytes();
			shaderModuleCreateInfo.pCode = reinterpret_cast<const GTSL::uint32*>(blob.begin());
			renderDevice->VkCreateShaderModule(renderDevice->GetVkDevice(), &shaderModuleCreateInfo, renderDevice->GetVkAllocationCallbacks(), &vkShaderModule);
		}
		
		void Destroy(const VulkanRenderDevice* renderDevice) {
			renderDevice->VkDestroyShaderModule(renderDevice->GetVkDevice(), vkShaderModule, renderDevice->GetVkAllocationCallbacks());
			debugClear(vkShaderModule);
		}

		VkShaderModule GetVkShaderModule() const { return vkShaderModule; }
	
	private:
		VkShaderModule vkShaderModule;
	};

	class VulkanPipelineLayout final
	{
	public:
		VulkanPipelineLayout() = default;
		
		void Initialize(const VulkanRenderDevice* renderDevice, const PushConstant* pushConstant, const GTSL::Range<const VulkanBindingsSetLayout*> bindingsSetLayouts) {
			VkPipelineLayoutCreateInfo vkPipelineLayoutCreateInfo{ VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO };

			GTSL::StaticVector<VkDescriptorSetLayout, 16> vkDescriptorSetLayouts;
			for (auto& e : bindingsSetLayouts) { vkDescriptorSetLayouts.EmplaceBack(e.GetVkDescriptorSetLayout()); }
			
			VkPushConstantRange vkPushConstantRange;
			if (pushConstant) {
				vkPushConstantRange.size = pushConstant->NumberOf4ByteSlots * 4;
				vkPushConstantRange.offset = 0;
				vkPushConstantRange.stageFlags = ToVulkan(pushConstant->Stage);

				vkPipelineLayoutCreateInfo.pushConstantRangeCount = 1;
				vkPipelineLayoutCreateInfo.pPushConstantRanges = &vkPushConstantRange;
			} else {
				vkPipelineLayoutCreateInfo.pushConstantRangeCount = 0;
				vkPipelineLayoutCreateInfo.pPushConstantRanges = nullptr;
			}

			vkPipelineLayoutCreateInfo.setLayoutCount = vkDescriptorSetLayouts.GetLength();
			vkPipelineLayoutCreateInfo.pSetLayouts = vkDescriptorSetLayouts.begin();

			renderDevice->VkCreatePipelineLayout(renderDevice->GetVkDevice(), &vkPipelineLayoutCreateInfo, renderDevice->GetVkAllocationCallbacks(), &pipelineLayout);
			//setName(createInfo.RenderDevice, pipelineLayout, VK_OBJECT_TYPE_PIPELINE_LAYOUT, createInfo.Name);
		}
		
		void Destroy(const VulkanRenderDevice* renderDevice) {
			renderDevice->VkDestroyPipelineLayout(renderDevice->GetVkDevice(), pipelineLayout, renderDevice->GetVkAllocationCallbacks());
			debugClear(pipelineLayout);
		}

		[[nodiscard]] VkPipelineLayout GetVkPipelineLayout() const { return pipelineLayout; }
	private:
		VkPipelineLayout pipelineLayout = nullptr;
	};
	
	class VulkanPipeline : public Pipeline
	{
	public:
		struct ShaderInfo
		{
			VulkanShader Shader; ShaderType Type;
			GTSL::Range<const GTSL::byte*> Blob;
		};

		void InitializeRasterPipeline(const VulkanRenderDevice* renderDevice, const GTSL::Range<const PipelineStateBlock*> pipelineStates, GTSL::Range<const ShaderInfo*> stages, const VulkanPipelineLayout pipelineLayout, const VulkanPipelineCache pipelineCache) {
			VkPipelineMultisampleStateCreateInfo vkPipelineMultisampleStateCreateInfo;
			vkPipelineMultisampleStateCreateInfo.sType = VK_STRUCTURE_TYPE_PIPELINE_MULTISAMPLE_STATE_CREATE_INFO;
			vkPipelineMultisampleStateCreateInfo.pNext = 0;
			vkPipelineMultisampleStateCreateInfo.alphaToCoverageEnable = false;
			vkPipelineMultisampleStateCreateInfo.alphaToOneEnable = false;
			vkPipelineMultisampleStateCreateInfo.flags = 0;
			vkPipelineMultisampleStateCreateInfo.minSampleShading = 0;
			vkPipelineMultisampleStateCreateInfo.pSampleMask = nullptr;
			vkPipelineMultisampleStateCreateInfo.rasterizationSamples = VK_SAMPLE_COUNT_1_BIT;
			vkPipelineMultisampleStateCreateInfo.sampleShadingEnable = false;

			VkGraphicsPipelineCreateInfo vkGraphicsPipelineCreateInfo{ VK_STRUCTURE_TYPE_GRAPHICS_PIPELINE_CREATE_INFO };
			vkGraphicsPipelineCreateInfo.pTessellationState = nullptr; vkGraphicsPipelineCreateInfo.pColorBlendState = nullptr;
			vkGraphicsPipelineCreateInfo.pVertexInputState = nullptr; vkGraphicsPipelineCreateInfo.pInputAssemblyState = nullptr;
			vkGraphicsPipelineCreateInfo.pViewportState = nullptr; vkGraphicsPipelineCreateInfo.pRasterizationState = nullptr;
			vkGraphicsPipelineCreateInfo.pDepthStencilState = nullptr; vkGraphicsPipelineCreateInfo.pMultisampleState = &vkPipelineMultisampleStateCreateInfo;
			vkGraphicsPipelineCreateInfo.layout = pipelineLayout.GetVkPipelineLayout();

			GTSL::Buffer<GTSL::StaticAllocator<8192>> buffer(8192, 8);

			for (GTSL::uint8 ps = 0; ps < static_cast<GTSL::uint8>(pipelineStates.ElementCount()); ++ps)
			{
				switch (const auto& pipelineState = pipelineStates[ps]; pipelineState.Type)
				{
				case PipelineStateBlock::StateType::VIEWPORT_STATE: {
					auto* vkViewport = buffer.AllocateStructure<VkViewport>();
					vkViewport->x = 0;
					vkViewport->y = 0;
					vkViewport->width = 1.0f;
					vkViewport->height = 1.0f;
					vkViewport->minDepth = 0.0f;
					vkViewport->maxDepth = 1.0f;

					auto* vkScissor = buffer.AllocateStructure<VkRect2D>();
					*vkScissor = { { 0, 0 }, { 1, 1 } };

					auto* pointer = buffer.AllocateStructure<VkPipelineViewportStateCreateInfo>();

					VkPipelineViewportStateCreateInfo& vkPipelineViewportStateCreateInfo = *pointer;
					vkPipelineViewportStateCreateInfo.sType = VK_STRUCTURE_TYPE_PIPELINE_VIEWPORT_STATE_CREATE_INFO;
					vkPipelineViewportStateCreateInfo.pNext = nullptr;
					vkPipelineViewportStateCreateInfo.viewportCount = pipelineState.Viewport.ViewportCount;
					vkPipelineViewportStateCreateInfo.pViewports = vkViewport;
					vkPipelineViewportStateCreateInfo.scissorCount = 1;
					vkPipelineViewportStateCreateInfo.pScissors = vkScissor;

					vkGraphicsPipelineCreateInfo.pViewportState = pointer;

					break;
				}
				case PipelineStateBlock::StateType::RASTER_STATE: {
					auto* pointer = buffer.AllocateStructure<VkPipelineRasterizationStateCreateInfo>();

					VkPipelineRasterizationStateCreateInfo& vkPipelineRasterizationStateCreateInfo = *pointer;
					vkPipelineRasterizationStateCreateInfo.sType = VK_STRUCTURE_TYPE_PIPELINE_RASTERIZATION_STATE_CREATE_INFO;
					vkPipelineRasterizationStateCreateInfo.pNext = nullptr;
					vkPipelineRasterizationStateCreateInfo.depthClampEnable = VK_FALSE;
					vkPipelineRasterizationStateCreateInfo.rasterizerDiscardEnable = VK_FALSE;
					vkPipelineRasterizationStateCreateInfo.polygonMode = VK_POLYGON_MODE_FILL;
					vkPipelineRasterizationStateCreateInfo.lineWidth = 1.0f;
					vkPipelineRasterizationStateCreateInfo.frontFace = ToVulkan(pipelineState.Raster.WindingOrder);
					vkPipelineRasterizationStateCreateInfo.cullMode = ToVulkan(pipelineState.Raster.CullMode);
					vkPipelineRasterizationStateCreateInfo.depthBiasEnable = VK_FALSE;
					vkPipelineRasterizationStateCreateInfo.depthBiasConstantFactor = 0.0f; // Optional
					vkPipelineRasterizationStateCreateInfo.depthBiasClamp = 0.0f; // Optional
					vkPipelineRasterizationStateCreateInfo.depthBiasSlopeFactor = 0.0f; // Optional

					vkGraphicsPipelineCreateInfo.pRasterizationState = pointer;

					break;
				}
				case PipelineStateBlock::StateType::DEPTH_STATE:
				{
					auto* pointer = buffer.AllocateStructure<VkPipelineDepthStencilStateCreateInfo>();

					auto& vkPipelineDepthStencilStateCreateInfo = *pointer;
					vkPipelineDepthStencilStateCreateInfo.sType = VK_STRUCTURE_TYPE_PIPELINE_DEPTH_STENCIL_STATE_CREATE_INFO;
					vkPipelineDepthStencilStateCreateInfo.pNext = nullptr;
					vkPipelineDepthStencilStateCreateInfo.depthTestEnable = true;
					vkPipelineDepthStencilStateCreateInfo.depthWriteEnable = true;
					vkPipelineDepthStencilStateCreateInfo.depthCompareOp = ToVulkan(pipelineState.Depth.CompareOperation);
					vkPipelineDepthStencilStateCreateInfo.depthBoundsTestEnable = VK_FALSE;
					vkPipelineDepthStencilStateCreateInfo.minDepthBounds = 0.0f; // Optional
					vkPipelineDepthStencilStateCreateInfo.maxDepthBounds = 1.0f; // Optional
					vkPipelineDepthStencilStateCreateInfo.stencilTestEnable = false;

					vkGraphicsPipelineCreateInfo.pDepthStencilState = pointer;

					break;
				}
				case PipelineStateBlock::StateType::COLOR_BLEND_STATE: {
					auto* pointer = buffer.AllocateStructure<VkPipelineColorBlendStateCreateInfo>();

					auto& vkPipelineColorblendStateCreateInfo = *pointer;
					vkPipelineColorblendStateCreateInfo.sType = VK_STRUCTURE_TYPE_PIPELINE_COLOR_BLEND_STATE_CREATE_INFO;
					vkPipelineColorblendStateCreateInfo.pNext = nullptr;
					vkPipelineColorblendStateCreateInfo.logicOpEnable = VK_FALSE;
					vkPipelineColorblendStateCreateInfo.logicOp = VK_LOGIC_OP_COPY; // Optional
					vkPipelineColorblendStateCreateInfo.pAttachments = reinterpret_cast<const VkPipelineColorBlendAttachmentState*>(buffer.GetData() + buffer.GetLength());

					GTSL::uint8 attachmentCount = 0;
					for (GTSL::uint8 i = 0; i < static_cast<GTSL::uint8>(pipelineState.Context.Attachments.ElementCount()); ++i) {
						if (pipelineState.Context.Attachments[i].FormatDescriptor.Type == TextureType::COLOR) {
							auto* state = buffer.AllocateStructure<VkPipelineColorBlendAttachmentState>();
							state->blendEnable = pipelineState.Context.Attachments[i].BlendEnable;
							state->colorWriteMask = VK_COLOR_COMPONENT_R_BIT | VK_COLOR_COMPONENT_G_BIT | VK_COLOR_COMPONENT_B_BIT | VK_COLOR_COMPONENT_A_BIT;
							state->srcColorBlendFactor = VK_BLEND_FACTOR_ONE; state->dstColorBlendFactor = VK_BLEND_FACTOR_ZERO;
							state->colorBlendOp = VK_BLEND_OP_ADD; state->alphaBlendOp = VK_BLEND_OP_ADD;
							state->srcAlphaBlendFactor = VK_BLEND_FACTOR_ONE; state->dstAlphaBlendFactor = VK_BLEND_FACTOR_ZERO;
							++attachmentCount;
						}
					}

					vkPipelineColorblendStateCreateInfo.attachmentCount = attachmentCount;
					vkPipelineColorblendStateCreateInfo.blendConstants[0] = 0.0f; // Optional
					vkPipelineColorblendStateCreateInfo.blendConstants[1] = 0.0f; // Optional
					vkPipelineColorblendStateCreateInfo.blendConstants[2] = 0.0f; // Optional
					vkPipelineColorblendStateCreateInfo.blendConstants[3] = 0.0f; // Optional

					vkGraphicsPipelineCreateInfo.pColorBlendState = pointer;

					vkGraphicsPipelineCreateInfo.renderPass = static_cast<const VulkanRenderPass*>(pipelineState.Context.RenderPass)->GetVkRenderPass();
					vkGraphicsPipelineCreateInfo.subpass = pipelineState.Context.SubPassIndex;

					break;
				}
				case PipelineStateBlock::StateType::VERTEX_STATE: {
					auto* pointer = buffer.AllocateStructure<VkPipelineVertexInputStateCreateInfo>();
					auto* binding = buffer.AllocateStructure<VkVertexInputBindingDescription>();

					binding->binding = 0; binding->inputRate = VK_VERTEX_INPUT_RATE_VERTEX; binding->stride = 0;

					pointer->sType = VK_STRUCTURE_TYPE_PIPELINE_VERTEX_INPUT_STATE_CREATE_INFO; pointer->pNext = nullptr;
					pointer->vertexBindingDescriptionCount = 1;
					pointer->pVertexBindingDescriptions = binding;
					pointer->vertexAttributeDescriptionCount = 0;
					pointer->pVertexAttributeDescriptions = reinterpret_cast<const VkVertexInputAttributeDescription*>(buffer.GetData() + buffer.GetLength());;

					GTSL::uint16 offset = 0;

					for (GTSL::uint8 i = 0; i < static_cast<GTSL::uint8>(pipelineState.Vertex.VertexDescriptor.ElementCount()); ++i) {
						auto size = ShaderDataTypesSize(pipelineState.Vertex.VertexDescriptor[i].Type);

						auto& vertex = *buffer.AllocateStructure<VkVertexInputAttributeDescription>();
						vertex.binding = 0; vertex.location = i; vertex.format = ToVulkan(pipelineState.Vertex.VertexDescriptor[i].Type);
						vertex.offset = offset;
						offset += size;
						binding->stride += size;
						++pointer->vertexAttributeDescriptionCount;
					}

					vkGraphicsPipelineCreateInfo.pVertexInputState = pointer;

					auto* inputAssemblyState = buffer.AllocateStructure<VkPipelineInputAssemblyStateCreateInfo>();
					inputAssemblyState->sType = VK_STRUCTURE_TYPE_PIPELINE_INPUT_ASSEMBLY_STATE_CREATE_INFO; inputAssemblyState->pNext = nullptr;
					inputAssemblyState->flags = 0; inputAssemblyState->primitiveRestartEnable = false; inputAssemblyState->topology = VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST;
					vkGraphicsPipelineCreateInfo.pInputAssemblyState = inputAssemblyState;

					break;
				}
				default:;
				}
			}

			VkPipelineDynamicStateCreateInfo vkPipelineDynamicStateCreateInfo{ VK_STRUCTURE_TYPE_PIPELINE_DYNAMIC_STATE_CREATE_INFO };
			GTSL::StaticVector<VkDynamicState, 4> vkDynamicStates = { VK_DYNAMIC_STATE_VIEWPORT, VK_DYNAMIC_STATE_SCISSOR };
			vkPipelineDynamicStateCreateInfo.dynamicStateCount = vkDynamicStates.GetLength();
			vkPipelineDynamicStateCreateInfo.pDynamicStates = vkDynamicStates.begin();

			GTSL::StaticVector<VkPipelineShaderStageCreateInfo, MAX_SHADER_STAGES> vkPipelineShaderStageCreateInfos;

			for (GTSL::uint8 i = 0; i < static_cast<GTSL::uint8>(stages.ElementCount()); ++i) {
				auto& stage = vkPipelineShaderStageCreateInfos.EmplaceBack();

				stage.sType = VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO;
				stage.pNext = nullptr;
				stage.flags = 0;
				stage.stage = ToVulkan(stages[i].Type);
				stage.pName = "main";
				stage.module = stages[i].Shader.GetVkShaderModule();
				stage.pSpecializationInfo = nullptr;
			}

			vkGraphicsPipelineCreateInfo.stageCount = vkPipelineShaderStageCreateInfos.GetLength();
			vkGraphicsPipelineCreateInfo.pStages = vkPipelineShaderStageCreateInfos.begin();
			vkGraphicsPipelineCreateInfo.pDynamicState = &vkPipelineDynamicStateCreateInfo;
			vkGraphicsPipelineCreateInfo.basePipelineIndex = -1;
			vkGraphicsPipelineCreateInfo.basePipelineHandle = nullptr;

			renderDevice->VkCreateGraphicsPipelines(renderDevice->GetVkDevice(), pipelineCache.GetVkPipelineCache(), 1, &vkGraphicsPipelineCreateInfo, renderDevice->GetVkAllocationCallbacks(), &pipeline);
			//SET_NAME(pipeline, VK_OBJECT_TYPE_PIPELINE, createInfo);
		}
		
		void InitializeComputePipeline(const VulkanRenderDevice* renderDevice, const GTSL::Range<const PipelineStateBlock*> pipelineStates, GTSL::Range<const ShaderInfo*> stages, const VulkanPipelineLayout pipelineLayout, const VulkanPipelineCache pipelineCache) {
			VkComputePipelineCreateInfo computePipelineCreateInfo{ VK_STRUCTURE_TYPE_COMPUTE_PIPELINE_CREATE_INFO };
			computePipelineCreateInfo.basePipelineIndex = -1;
			computePipelineCreateInfo.layout = pipelineLayout.GetVkPipelineLayout();
			computePipelineCreateInfo.stage.sType = VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO;
			computePipelineCreateInfo.stage.stage = VK_SHADER_STAGE_COMPUTE_BIT;
			computePipelineCreateInfo.stage.pName = "main";
			computePipelineCreateInfo.stage.module = stages[0].Shader.GetVkShaderModule();

			renderDevice->VkCreateComputePipelines(renderDevice->GetVkDevice(), pipelineCache.GetVkPipelineCache(), 1, &computePipelineCreateInfo, renderDevice->GetVkAllocationCallbacks(), &pipeline);
		}
		
		void InitializeRayTracePipeline(const VulkanRenderDevice* renderDevice, const GTSL::Range<const PipelineStateBlock*> pipelineStates, GTSL::Range<const ShaderInfo*> stages, const VulkanPipelineLayout pipelineLayout, const VulkanPipelineCache pipelineCache) {
			GTSL::StaticVector<VkRayTracingShaderGroupCreateInfoKHR, 16> vkRayTracingShaderGroupCreateInfoKhrs;

			VkRayTracingPipelineCreateInfoKHR vkRayTracingPipelineCreateInfo{ VK_STRUCTURE_TYPE_RAY_TRACING_PIPELINE_CREATE_INFO_KHR };
			vkRayTracingPipelineCreateInfo.basePipelineIndex = -1;
			vkRayTracingPipelineCreateInfo.maxPipelineRayRecursionDepth = 0;

			for (GTSL::uint32 i = 0; i < static_cast<GTSL::uint32>(pipelineStates.ElementCount()); ++i)
			{
				auto& pipelineState = pipelineStates[i];

				switch (pipelineState.Type)
				{
				case PipelineStateBlock::StateType::RAY_TRACE_GROUPS:
				{
					for (const auto& e : pipelineState.RayTracing.Groups) {
						auto& p = vkRayTracingShaderGroupCreateInfoKhrs.EmplaceBack();
						p.sType = VK_STRUCTURE_TYPE_RAY_TRACING_SHADER_GROUP_CREATE_INFO_KHR;
						p.pNext = nullptr;

						p.anyHitShader = e.AnyHitShader == RayTraceGroup::SHADER_UNUSED ? VK_SHADER_UNUSED_KHR : e.AnyHitShader;
						p.closestHitShader = e.ClosestHitShader == RayTraceGroup::SHADER_UNUSED ? VK_SHADER_UNUSED_KHR : e.ClosestHitShader;
						p.generalShader = e.GeneralShader == RayTraceGroup::SHADER_UNUSED ? VK_SHADER_UNUSED_KHR : e.GeneralShader;
						p.intersectionShader = e.IntersectionShader == RayTraceGroup::SHADER_UNUSED ? VK_SHADER_UNUSED_KHR : e.IntersectionShader;

						p.type = ToVulkan(e.ShaderGroup);

						p.pShaderGroupCaptureReplayHandle = nullptr;
					}

					vkRayTracingPipelineCreateInfo.maxPipelineRayRecursionDepth = pipelineState.RayTracing.MaxRecursionDepth;

					break;
				}
				default: break;
				}
			}

			GTSL::StaticVector<VkPipelineShaderStageCreateInfo, 32> vkPipelineShaderStageCreateInfos;

			for (GTSL::uint32 i = 0; i < static_cast<GTSL::uint32>(stages.ElementCount()); ++i)
			{
				auto& stageCreateInfo = vkPipelineShaderStageCreateInfos.EmplaceBack();
				stageCreateInfo.sType = VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO;
				stageCreateInfo.pNext = nullptr;
				stageCreateInfo.flags = 0;
				stageCreateInfo.stage = static_cast<VkShaderStageFlagBits>(stages[i].Type);
				stageCreateInfo.pName = "main";
				stageCreateInfo.pSpecializationInfo = nullptr;
				stageCreateInfo.module = stages[i].Shader.GetVkShaderModule();
			}

			vkRayTracingPipelineCreateInfo.stageCount = vkPipelineShaderStageCreateInfos.GetLength();
			vkRayTracingPipelineCreateInfo.pStages = vkPipelineShaderStageCreateInfos.begin();

			vkRayTracingPipelineCreateInfo.layout = pipelineLayout.GetVkPipelineLayout();

			vkRayTracingPipelineCreateInfo.groupCount = vkRayTracingShaderGroupCreateInfoKhrs.GetLength();
			vkRayTracingPipelineCreateInfo.pGroups = vkRayTracingShaderGroupCreateInfoKhrs.begin();

			renderDevice->vkCreateRayTracingPipelinesKHR(renderDevice->GetVkDevice(), nullptr, pipelineCache.GetVkPipelineCache(), 1, &vkRayTracingPipelineCreateInfo, renderDevice->GetVkAllocationCallbacks(), &pipeline);
			//SET_NAME(pipeline, VK_OBJECT_TYPE_PIPELINE, createInfo)
		}
		
		void Destroy(const VulkanRenderDevice* renderDevice) {
			renderDevice->VkDestroyPipeline(renderDevice->GetVkDevice(), pipeline, renderDevice->GetVkAllocationCallbacks());
			debugClear(pipeline);
		}
		
		[[nodiscard]] VkPipeline GetVkPipeline() const { return pipeline; }
		[[nodiscard]] GTSL::uint64 GetHandle() const { return reinterpret_cast<uint64_t>(pipeline); }

		template<class ALLOCATOR>
		void GetShaderGroupHandles(VulkanRenderDevice* renderDevice, GTSL::uint32 firstGroup, GTSL::uint32 groupCount, GTSL::Vector<ShaderHandle, ALLOCATOR>& vector) {
			vector.SetLength(groupCount);
			renderDevice->vkGetRayTracingShaderGroupHandlesKHR(renderDevice->GetVkDevice(), pipeline, firstGroup, groupCount, groupCount * 32u, vector.begin());
		}
	protected:
		VkPipeline pipeline = nullptr;
	};
}
