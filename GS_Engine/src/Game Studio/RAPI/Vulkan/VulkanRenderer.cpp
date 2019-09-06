#include "Vulkan.h"
#include "VulkanRenderer.h"

#include "VulkanRenderContext.h"
#include "VulkanPipelines.h"
#include "VulkanRenderPass.h"
#include "VulkanMesh.h"
#include "VulkanImage.h"
#include "VulkanUniformBuffer.h"
#include "VulkanUniformLayout.h"

//  VULKAN RENDERER

VKCommandPoolCreator VulkanRenderer::CreateCommandPool()
{
	VkCommandPoolCreateInfo CommandPoolCreateInfo = { VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO };
	CommandPoolCreateInfo.flags = VK_COMMAND_POOL_CREATE_TRANSIENT_BIT;

	return VKCommandPoolCreator(&Device, &CommandPoolCreateInfo);
}

VulkanRenderer::VulkanRenderer() : Instance("Game Studio"),
	PhysicalDevice(Instance),
	Device(Instance, PhysicalDevice),
	TransientCommandPool(CreateCommandPool())
{
}

VulkanRenderer::~VulkanRenderer()
{
}

Mesh* VulkanRenderer::CreateMesh(const MeshCreateInfo& _MCI)
{
	return new VulkanMesh(&Device, TransientCommandPool, _MCI.VertexData, _MCI.VertexCount * _MCI.VertexLayout->GetSize(), _MCI.IndexData, _MCI.IndexCount);
}

UniformBuffer* VulkanRenderer::CreateUniformBuffer(const UniformBufferCreateInfo& _BCI)
{
	return new VulkanUniformBuffer(&Device, _BCI);
}

UniformLayout* VulkanRenderer::CreateUniformLayout(const UniformLayoutCreateInfo& _ULCI)
{
	return new VulkanUniformLayout(&Device, _ULCI);
}

Image* VulkanRenderer::CreateImage(const ImageCreateInfo& _ICI)
{
	return new VulkanImage(&Device, _ICI.Extent, _ICI.ImageFormat, _ICI.Dimensions, _ICI.Type, _ICI.Use);
}

GraphicsPipeline* VulkanRenderer::CreateGraphicsPipeline(const GraphicsPipelineCreateInfo& _GPCI)
{
	return new VulkanGraphicsPipeline(&Device, _GPCI);
}

RenderPass* VulkanRenderer::CreateRenderPass(const RenderPassCreateInfo& _RPCI)
{
	return new VulkanRenderPass(&Device, _RPCI.Descriptor);
}

ComputePipeline* VulkanRenderer::CreateComputePipeline(const ComputePipelineCreateInfo& _CPCI)
{
	return new ComputePipeline();
}

Framebuffer* VulkanRenderer::CreateFramebuffer(const FramebufferCreateInfo& _FCI)
{
	return new VulkanFramebuffer(&Device, SCAST(VulkanRenderPass*, _FCI.RenderPass), _FCI.Extent, _FCI.Images);
}

RenderContext* VulkanRenderer::CreateRenderContext(const RenderContextCreateInfo& _RCCI)
{
	return new VulkanRenderContext(&Device, Instance, PhysicalDevice, *_RCCI.Window);
}