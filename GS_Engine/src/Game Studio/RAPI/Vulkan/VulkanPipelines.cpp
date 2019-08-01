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

VulkanGraphicsPipeline::VulkanGraphicsPipeline(const Vk_Device& _Device, RenderPass* _RP, Extent2D _SwapchainSize, const ShaderStages& _SI) :
	Layout(_Device),
	Pipeline(_Device, SCAST(VulkanRenderPass*, _RP)->GetVk_RenderPass(), Extent2DToVkExtent2D(_SwapchainSize), Layout, StageInfoToVulkanStageInfo(_SI, _Device))
{
}

VulkanComputePipeline::VulkanComputePipeline(const Vk_Device& _Device) : ComputePipeline(_Device)
{
}