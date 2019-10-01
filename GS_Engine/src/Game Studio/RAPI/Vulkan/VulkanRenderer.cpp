#include "Vulkan.h"
#include "VulkanRenderer.h"

#include "VulkanRenderContext.h"
#include "VulkanPipelines.h"
#include "VulkanRenderPass.h"
#include "VulkanMesh.h"
#include "VulkanImage.h"
#include "VulkanUniformBuffer.h"
#include "VulkanUniformLayout.h"

//  VULKAN RAPI

VKCommandPoolCreator VulkanRenderDevice::CreateCommandPool()
{
	VkCommandPoolCreateInfo CommandPoolCreateInfo = { VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO };
	CommandPoolCreateInfo.flags = VK_COMMAND_POOL_CREATE_TRANSIENT_BIT;

	return VKCommandPoolCreator(&Device, &CommandPoolCreateInfo);
}

VulkanRenderDevice::VulkanRenderDevice() : Instance("Game Studio"),
	PhysicalDevice(Instance),
	Device(Instance, PhysicalDevice),
	TransientCommandPool(CreateCommandPool())
{
}

VulkanRenderDevice::~VulkanRenderDevice()
{
}

Mesh* VulkanRenderDevice::CreateMesh(const MeshCreateInfo& _MCI)
{
	return new VulkanMesh(&Device, TransientCommandPool, _MCI.VertexData, _MCI.VertexCount * _MCI.VertexLayout->GetSize(), _MCI.IndexData, _MCI.IndexCount);
}

UniformBuffer* VulkanRenderDevice::CreateUniformBuffer(const UniformBufferCreateInfo& _BCI)
{
	return new VulkanUniformBuffer(&Device, _BCI);
}

UniformLayout* VulkanRenderDevice::CreateUniformLayout(const UniformLayoutCreateInfo& _ULCI)
{
	return new VulkanUniformLayout(&Device, _ULCI);
}

Image* VulkanRenderDevice::CreateImage(const ImageCreateInfo& _ICI)
{
	return new VulkanImage(&Device, _ICI.Extent, _ICI.ImageFormat, _ICI.Dimensions, _ICI.Type, _ICI.Use);
}

GraphicsPipeline* VulkanRenderDevice::CreateGraphicsPipeline(const GraphicsPipelineCreateInfo& _GPCI)
{
	return new VulkanGraphicsPipeline(&Device, _GPCI);
}

RenderPass* VulkanRenderDevice::CreateRenderPass(const RenderPassCreateInfo& _RPCI)
{
	return new VulkanRenderPass(&Device, _RPCI.Descriptor);
}

ComputePipeline* VulkanRenderDevice::CreateComputePipeline(const ComputePipelineCreateInfo& _CPCI)
{
	return new ComputePipeline();
}

Framebuffer* VulkanRenderDevice::CreateFramebuffer(const FramebufferCreateInfo& _FCI)
{
	return new VulkanFramebuffer(&Device, SCAST(VulkanRenderPass*, _FCI.RenderPass), _FCI.Extent, _FCI.Images);
}

RenderContext* VulkanRenderDevice::CreateRenderContext(const RenderContextCreateInfo& _RCCI)
{
	return new VulkanRenderContext(&Device, &Instance, PhysicalDevice, _RCCI.Window);
}