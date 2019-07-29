#include "Vulkan.h"
#include "VulkanRenderer.h"

#include "VulkanShader.h"
#include "VulkanRenderContext.h"
#include "VulkanPipelines.h"
#include "VulkanRenderPass.h"
#include "VulkanMesh.h"


//  VULKAN RENDERER

VulkanRenderer::VulkanRenderer() : Instance("Game Studio"), Device(Instance.GetVkInstance()),
                                   TransientCommandPool(Device, Device.GetTransferQueue().GetQueueIndex(),
                                                        VK_COMMAND_POOL_CREATE_TRANSIENT_BIT)
{
}

VulkanRenderer::~VulkanRenderer()
{
}

Shader* VulkanRenderer::CreateShader(const ShaderCreateInfo& _SI)
{
	return new VulkanShader(Device, _SI.ShaderName, _SI.Type);
}

Mesh* VulkanRenderer::CreateMesh(const MeshCreateInfo& _MCI)
{
	return new VulkanMesh(Device);
}

GraphicsPipeline* VulkanRenderer::CreateGraphicsPipeline(const GraphicsPipelineCreateInfo& _GPCI)
{
	return new VulkanGraphicsPipeline(Device, _GPCI.RenderPass, _GPCI.SwapchainSize, _GPCI.StagesInfo);
}

RenderPass* VulkanRenderer::CreateRenderPass(const RenderPassCreateInfo& _RPCI)
{
	return new VulkanRenderPass(Device, _RPCI.RPDescriptor);
}

ComputePipeline* VulkanRenderer::CreateComputePipeline(const ComputePipelineCreateInfo& _CPCI)
{
	return new ComputePipeline();
}

Framebuffer* VulkanRenderer::CreateFramebuffer(const FramebufferCreateInfo& _FCI)
{
	return new VulkanFramebuffer(Device, _FCI.RenderPass, _FCI.Extent, _FCI.Images, _FCI.ImagesCount);
}

RenderContext* VulkanRenderer::CreateRenderContext(const RenderContextCreateInfo& _RCCI)
{
	return new VulkanRenderContext(Device, Instance.GetVkInstance(), Device.GetVkPhysicalDevice(), _RCCI.Window);
}