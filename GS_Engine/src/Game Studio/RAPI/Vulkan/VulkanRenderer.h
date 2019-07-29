#pragma once

#include "Core.h"

#include "RAPI/Renderer.h"

#include "VulkanBase.h"

#include "VulkanFramebuffer.h"
#include "Native/Vk_Queue.h"
#include "VulkanRenderContext.h"
#include "Native/Vk_Instance.h"
#include "Native/Vk_Device.h"
#include "Native/Vk_CommandPool.h"

MAKE_VK_HANDLE(VkPhysicalDevice)
struct VkDeviceQueueCreateInfo;
struct QueueInfo;
enum VkPhysicalDeviceType;

GS_CLASS VulkanRenderer final : public Renderer
{
	Vk_Instance Instance;
	Vk_Device Device;

	Vk_CommandPool TransientCommandPool;
public:
	VulkanRenderer();
	~VulkanRenderer();

	Shader* CreateShader(const ShaderCreateInfo& _SI) final override;
	Mesh* CreateMesh(const MeshCreateInfo& _MCI) final override;
	GraphicsPipeline* CreateGraphicsPipeline(const GraphicsPipelineCreateInfo& _GPCI) final override;
	RenderPass* CreateRenderPass(const RenderPassCreateInfo& _RPCI) final override;
	ComputePipeline* CreateComputePipeline(const ComputePipelineCreateInfo& _CPCI) final override;
	Framebuffer* CreateFramebuffer(const FramebufferCreateInfo& _FCI) final override;
	RenderContext* CreateRenderContext(const RenderContextCreateInfo& _RCCI) final override;

	INLINE const Vk_Device& GetVulkanDevice() const { return Device; }
};

#define VKRAPI SCAST(VulkanRenderer*, Renderer::GetRenderer())