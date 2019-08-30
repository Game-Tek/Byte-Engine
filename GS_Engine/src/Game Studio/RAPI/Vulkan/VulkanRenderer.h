#pragma once

#include "Core.h"

#include "RAPI/Renderer.h"

#include "VulkanBase.h"

#include "VulkanFramebuffer.h"
#include "VulkanRenderContext.h"
#include "Native/VKInstance.h"
#include "Native/VKDevice.h"
#include "Native/VKCommandPool.h"
#include "Native/vkPhysicalDevice.h"

MAKE_VK_HANDLE(VkPhysicalDevice)
struct VkDeviceQueueCreateInfo;
struct QueueInfo;
enum VkPhysicalDeviceType;

GS_CLASS VulkanRenderer final : public Renderer
{
	VKInstance Instance;
	vkPhysicalDevice PhysicalDevice;
	VKDevice Device;

	VKCommandPool TransientCommandPool;
public:
	VulkanRenderer();
	~VulkanRenderer();

	Mesh* CreateMesh(const MeshCreateInfo& _MCI) final override;
	Image* CreateImage(const ImageCreateInfo& _ICI) override;
	GraphicsPipeline* CreateGraphicsPipeline(const GraphicsPipelineCreateInfo& _GPCI) final override;
	RenderPass* CreateRenderPass(const RenderPassCreateInfo& _RPCI) final override;
	ComputePipeline* CreateComputePipeline(const ComputePipelineCreateInfo& _CPCI) final override;
	Framebuffer* CreateFramebuffer(const FramebufferCreateInfo& _FCI) final override;
	RenderContext* CreateRenderContext(const RenderContextCreateInfo& _RCCI) final override;

	INLINE const VKDevice& GetVulkanDevice() const { return Device; }
};

#define VKRAPI SCAST(VulkanRenderer*, Renderer::GetRenderer())