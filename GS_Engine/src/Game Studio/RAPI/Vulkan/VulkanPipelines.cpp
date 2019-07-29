#include "Vulkan.h"

#include "VulkanPipelines.h"

#include "RAPI/RenderPass.h"

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