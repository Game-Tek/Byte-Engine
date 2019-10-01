#pragma once

#include "Core.h"

#include "RAPI/RenderDevice.h"

#include "VulkanFramebuffer.h"
#include "VulkanRenderContext.h"
#include "Native/VKInstance.h"
#include "Native/VKDevice.h"
#include "Native/VKCommandPool.h"
#include "Native/vkPhysicalDevice.h"

struct VkDeviceQueueCreateInfo;
struct QueueInfo;
enum VkPhysicalDeviceType;

class GS_API VulkanRenderDevice final : public RenderDevice
{
	VKInstance Instance;
	vkPhysicalDevice PhysicalDevice;
	VKDevice Device;

	VKCommandPool TransientCommandPool;

	VKCommandPoolCreator CreateCommandPool();
public:
	VulkanRenderDevice();
	~VulkanRenderDevice();

	Mesh* CreateMesh(const MeshCreateInfo& _MCI) final override;
	UniformBuffer* CreateUniformBuffer(const UniformBufferCreateInfo& _BCI) final override;
	UniformLayout* CreateUniformLayout(const UniformLayoutCreateInfo& _ULCI) final override;
	Image* CreateImage(const ImageCreateInfo& _ICI) final override;
	GraphicsPipeline* CreateGraphicsPipeline(const GraphicsPipelineCreateInfo& _GPCI) final override;
	RenderPass* CreateRenderPass(const RenderPassCreateInfo& _RPCI) final override;
	ComputePipeline* CreateComputePipeline(const ComputePipelineCreateInfo& _CPCI) final override;
	Framebuffer* CreateFramebuffer(const FramebufferCreateInfo& _FCI) final override;
	RenderContext* CreateRenderContext(const RenderContextCreateInfo& _RCCI) final override;

	INLINE const VKDevice& GetVKDevice() const { return Device; }
};