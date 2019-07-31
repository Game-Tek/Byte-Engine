#include "Vulkan.h"

#include "VulkanPipelines.h"

#include "RAPI/RenderPass.h"

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

VulkanStageInfo StageInfoToVulkanStageInfo(const StageInfo& _SI)
{
	VulkanStageInfo Result;

	for (uint8 i = 0; i < _SI.ShaderCount; i++)
	{
		//Result.Shaders[i] = SCAST(VulkanShader*, _SI.Shader[i])->GetVk_ShaderModule();
		Result.ShaderTypes[i] = ShaderTypeToVkShaderStageFlagBits(_SI.Shader[i]->GetShaderType());
	}

	Result.ShaderCount = _SI.ShaderCount;

	return Result;
}

VulkanGraphicsPipeline::VulkanGraphicsPipeline(const Vk_Device& _Device, RenderPass * _RP, Extent2D _SwapchainSize, const StageInfo& _SI) :
	Layout(_Device),
	Pipeline(_Device, SCAST(VulkanRenderPass*, _RP)->GetVk_RenderPass(), Extent2DToVkExtent2D(_SwapchainSize), Layout, StageInfoToVulkanStageInfo(_SI))
{
}

VulkanComputePipeline::VulkanComputePipeline(const Vk_Device& _Device) : ComputePipeline(_Device)
{
}