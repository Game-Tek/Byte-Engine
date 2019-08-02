#include "Vulkan.h"
#include "VulkanRenderer.h"

#include "VulkanRenderContext.h"
#include "VulkanPipelines.h"
#include "VulkanRenderPass.h"
#include "VulkanMesh.h"


//  VULKAN RENDERER

VulkanRenderer::VulkanRenderer() : Instance("Game Studio"),
	PhysicalDevice(Instance),
	Device(Instance, PhysicalDevice),
	TransientCommandPool(Device, Device.GetTransferQueue(),VK_COMMAND_POOL_CREATE_TRANSIENT_BIT)
{
}

VulkanRenderer::~VulkanRenderer()
{
}

Mesh* VulkanRenderer::CreateMesh(const MeshCreateInfo& _MCI)
{
	return new VulkanMesh(Device, TransientCommandPool, _MCI.VertexData, _MCI.VertexCount * _MCI.VertexLayout.GetSize(), _MCI.IndexData, _MCI.IndexCount);
}

GraphicsPipeline* VulkanRenderer::CreateGraphicsPipeline(const GraphicsPipelineCreateInfo& _GPCI)
{
	return new VulkanGraphicsPipeline(Device, _GPCI.RenderPass, _GPCI.SwapchainSize, _GPCI.Stages);
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
	return new VulkanFramebuffer(Device, SCAST(VulkanRenderPass*, _FCI.RenderPass), _FCI.Extent, _FCI.Images, _FCI.ImagesCount);
}

RenderContext* VulkanRenderer::CreateRenderContext(const RenderContextCreateInfo& _RCCI)
{
	return new VulkanRenderContext(Device, Instance, PhysicalDevice, *_RCCI.Window);
}