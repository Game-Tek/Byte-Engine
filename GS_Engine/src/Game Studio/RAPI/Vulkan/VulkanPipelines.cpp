#include "Vulkan.h"

#include "VulkanPipelines.h"

#include "RAPI/RenderPass.h"
#include "RAPI/Vulkan/Native/Vk_ShaderModule.h"

#include "VulkanRenderPass.h"

VulkanGraphicsPipeline::VulkanGraphicsPipeline(const Vk_Device& _Device, RenderPass* _RP, Extent2D _SwapchainSize, const ShaderStages& _SI, const VertexDescriptor& _VD) :
	Layout(_Device),
	Pipeline(_Device, SCAST(VulkanRenderPass*, _RP)->GetVk_RenderPass(), Extent2DToVkExtent2D(_SwapchainSize), Layout, _SI, _VD)
{
}

VulkanComputePipeline::VulkanComputePipeline(const Vk_Device& _Device) : ComputePipeline(_Device)
{
}