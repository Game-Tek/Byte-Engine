#pragma once

#include "Core.h"

#include "RAPI/GraphicsPipeline.h"
#include "RAPI/ComputePipeline.h"

#include "Vulkan.h"

class VulkanShaders
{
public:
	//takes an unitialized fvector
	static void CompileShader(const FString& code, const FString& shaderName, uint32 shaderStage, FVector<uint32>& result);
};

class VulkanGraphicsPipeline final : public GraphicsPipeline
{
	VkPipelineLayout vkPipelineLayout = nullptr;
	VkPipeline vkPipeline = nullptr;

public:
	VulkanGraphicsPipeline(class VulkanRenderDevice* vulkanRenderDevice, const GraphicsPipelineCreateInfo& _GPCI);
	~VulkanGraphicsPipeline() = default;

	void Destroy(RenderDevice* renderDevice) override;

	INLINE VkPipeline GetVkGraphicsPipeline() const { return vkPipeline; }
	INLINE VkPipelineLayout GetVkPipelineLayout() const { return vkPipelineLayout; }
};

class VulkanComputePipeline final : public ComputePipeline
{
	VkPipeline vkPipeline = nullptr;

public:
	VulkanComputePipeline(class VulkanRenderDevice* vulkanRenderDevice, const ComputePipelineCreateInfo& computePipelineCreateInfo);
	~VulkanComputePipeline() = default;

	[[nodiscard]] VkPipeline GetVkPipeline() const { return vkPipeline; }
};
